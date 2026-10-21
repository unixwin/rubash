use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// Global parser/mutation generation for the lazy BASH_COMMAND cache.
pub static BASH_COMMAND_SEQ: AtomicU64 = AtomicU64::new(1);

#[inline]
pub(crate) fn bump_bash_command_seq() {
    BASH_COMMAND_SEQ.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub(crate) fn current_bash_command_seq() -> u64 {
    BASH_COMMAND_SEQ.load(Ordering::Relaxed)
}

#[inline]
pub(crate) fn touch_bash_command_pending() {
    bump_bash_command_seq();
}

/// Lazy BASH_COMMAND write marker + read-side materialization cache.
///
/// Lives in `Executor` (interior-mutable) so `&self` readers can flush
/// without `&mut`. Subshell isolation: fork-like children get a fresh
/// default (their writes never reach the parent); the shared-executor
/// `$( )` fast path saves/restores it alongside `InteriorSnapshot`.
#[derive(Debug, Clone, Default)]
pub(crate) struct BashCommandLazyState {
    pub ptr: usize,
    pub seq: u64,
    pub words_len: usize,
    pub meta_len: usize,
    pub is_case_head: bool,
    pub dirty: bool,
    pub last_ptr: usize,
    pub last_seq: u64,
    pub last_words_len: usize,
    pub last_meta_len: usize,
    pub last_is_case_head: bool,
    pub last_text: Option<String>,
    pub last_materialized: Option<String>,
}

impl Executor {
    pub(in crate::executor) fn bash_command_value(&self) -> String {
        if let Some(frozen) = self.shell_state.debug_trap_command.borrow().clone() {
            return frozen;
        }
        {
            let lazy = self.bash_command_lazy.borrow();
            if !lazy.dirty {
                if let Some(text) = lazy.last_text.clone() {
                    return text;
                }
            }
        }
        let (ptr, seq, wl, ml, is_case) = {
            let lazy = self.bash_command_lazy.borrow();
            (
                lazy.ptr,
                lazy.seq,
                lazy.words_len,
                lazy.meta_len,
                lazy.is_case_head,
            )
        };
        if ptr != 0 {
            let hit = {
                let lazy = self.bash_command_lazy.borrow();
                lazy.dirty
                    && lazy.ptr == lazy.last_ptr
                    && lazy.seq == lazy.last_seq
                    && lazy.words_len == lazy.last_words_len
                    && lazy.meta_len == lazy.last_meta_len
                    && lazy.is_case_head == lazy.last_is_case_head
                    && {
                        let env_current = self
                            .shell_state
                            .env_vars
                            .get("__RUBASH_CURRENT_COMMAND")
                            .cloned();
                        env_current == lazy.last_materialized
                    }
                    && lazy.last_text.is_some()
            };
            if hit {
                let text = {
                    let mut lazy = self.bash_command_lazy.borrow_mut();
                    lazy.dirty = false;
                    lazy.last_text.clone().unwrap_or_default()
                };
                return text;
            }
            let rendered = Self::render_bash_command_ptr(ptr, is_case);
            {
                let mut lazy = self.bash_command_lazy.borrow_mut();
                lazy.last_ptr = ptr;
                lazy.last_seq = seq;
                lazy.last_words_len = wl;
                lazy.last_meta_len = ml;
                lazy.last_is_case_head = is_case;
                lazy.last_text = Some(rendered.clone());
                lazy.dirty = false;
            }
            return self
                .bash_command_lazy
                .borrow()
                .last_text
                .clone()
                .unwrap_or_default();
        }
        self.shell_state
            .env_vars
            .get("__RUBASH_CURRENT_COMMAND")
            .cloned()
            .unwrap_or_default()
    }

    fn render_bash_command_ptr(ptr: usize, is_case_head: bool) -> String {
        if ptr == 0 {
            return String::new();
        }
        let cmd: &crate::parser::CommandNode =
            unsafe { &*(ptr as *const crate::parser::CommandNode) };
        if is_case_head {
            if let Some(case_command) = &cmd.case_command {
                return crate::executor::command_text::case_command_head_source_text(case_command);
            }
        }
        crate::executor::command_text::bash_command_source_text(cmd)
    }

    pub(in crate::executor) fn materialize_bash_command_for_node(&mut self, cmd: &CommandNode) {
        let is_case = cmd.case_command.is_some();
        let ptr = cmd as *const _ as usize;
        let seq = current_bash_command_seq();
        let wl = cmd.words.len();
        let ml = cmd.word_metadata.len();
        let hit = {
            let lazy = self.bash_command_lazy.borrow();
            !lazy.dirty
                && lazy.last_ptr == ptr
                && lazy.last_seq == seq
                && lazy.last_words_len == wl
                && lazy.last_meta_len == ml
                && lazy.last_is_case_head == is_case
                && lazy.last_text.is_some()
        };
        let text = if hit {
            self.bash_command_lazy
                .borrow()
                .last_text
                .clone()
                .unwrap_or_default()
        } else if is_case {
            if let Some(case_command) = &cmd.case_command {
                crate::executor::command_text::case_command_head_source_text(case_command)
            } else {
                crate::executor::command_text::bash_command_source_text(cmd)
            }
        } else {
            crate::executor::command_text::bash_command_source_text(cmd)
        };
        self.shell_state
            .env_vars
            .insert("__RUBASH_LAST_COMMAND".to_string(), text.clone());
        self.shell_state
            .env_vars
            .insert("__RUBASH_CURRENT_COMMAND".to_string(), text.clone());
        let mut lazy = self.bash_command_lazy.borrow_mut();
        lazy.ptr = ptr;
        lazy.seq = seq;
        lazy.words_len = wl;
        lazy.meta_len = ml;
        lazy.is_case_head = is_case;
        lazy.dirty = false;
        lazy.last_ptr = ptr;
        lazy.last_seq = seq;
        lazy.last_words_len = wl;
        lazy.last_meta_len = ml;
        lazy.last_is_case_head = is_case;
        lazy.last_text = Some(text.clone());
        lazy.last_materialized = Some(text);
    }

    /// Save the lazy cache for a shared-executor `$( )` boundary
    /// (mirrors `ShellState::snapshot_interior`).
    pub(crate) fn snapshot_bash_command_lazy(&self) -> BashCommandLazyState {
        self.bash_command_lazy.borrow().clone()
    }

    /// Restore the lazy cache saved by `snapshot_bash_command_lazy`.
    pub(crate) fn restore_bash_command_lazy(&self, snapshot: &BashCommandLazyState) {
        *self.bash_command_lazy.borrow_mut() = snapshot.clone();
    }
}
