//! Readline key-binding registry — the `bind` builtin's data plane.
//!
//! GNU bash keeps the binding state inside readline's keymaps: function
//! bindings and macros in the active keymap, and `bind -x` shell commands
//! in a parallel command map per editing mode (`bashline.c:4771
//! init_unix_command_map`: emacs_std_cmd_xmap, vi_insert_cmd_xmap,
//! vi_movement_cmd_xmap; `bashline.c:4850 bind_keyseq_to_unix_command`
//! stores the command there, `bashline.c:4593 bash_execute_unix_command`
//! runs it on the keypress). The rubash engine has no in-process line
//! editor — hosts own one (niubash's reedline REPL) — so the registry is
//! the shared data plane a host reads to mirror registered bindings into
//! its own keymaps, and through which the engine runs `bind -x` commands
//! with the GNU READLINE_LINE/POINT/MARK protocol.
//!
//! The registry lives in [`crate::shell::ShellState`] behind an
//! `Rc<RefCell<..>>` (the `session_history` pattern) because readline
//! keymaps are process-global C globals in GNU: a clone (flat subshell,
//! command substitution) shares the same registry, so a `bind` inside a
//! subshell is visible to the parent exactly as it is in GNU.

use std::cell::RefCell;
use std::rc::Rc;

/// What a key sequence is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindKind {
    /// `bind -x '"\C-r": command'` — the command runs on the keypress with
    /// the READLINE_* variables set and read back
    /// (bashline.c:4593-4707 bash_execute_unix_command).
    Execute { command: String },
    /// `bind '"\C-a": beginning-of-line'` — a readline function name
    /// (bind.def:324 passes the inputrc-form argument to
    /// rl_parse_and_bind; an unquoted value is a function name).
    Function { name: String },
    /// `bind '"\C-t": "text"'` — a macro (quoted value, bind.def inputrc
    /// form; GNU ignores an unquoted non-function value, see the wt100
    /// probe: `bind '"\C-t": ins-macro'` binds nothing and `bind -s`
    /// lists nothing).
    Macro { text: String },
}

/// One registered binding. `keyseq` keeps the user's written form
/// (`\C-g`, `\e[200~`, `\C-x\C-f`) — GNU's dumpers print the same
/// backslash form (`bind -X` probe: `"\C-g" "echo bound-c-g"`), and the
/// host translates it for its own key tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindEntry {
    pub keyseq: String,
    pub kind: BindKind,
    /// `-m keymap` target (bind.def:37-40): emacs, emacs-standard,
    /// emacs-meta, emacs-ctlx, vi, vi-move, vi-command, vi-insert. `None`
    /// means the current keymap, which is emacs-standard in the default
    /// editing mode.
    pub keymap: Option<String>,
}

/// Process-global binding registry (readline's keymap world).
#[derive(Debug, Clone, Default)]
pub struct BindRegistry {
    pub entries: Vec<BindEntry>,
    /// Bumped on every mutation so an embedding host can notice that its
    /// mirrored keymaps are stale and rebuild them between prompts.
    pub generation: u64,
    /// `set editing-mode vi|emacs` seen through `bind -f` inputrc files
    /// (or future `bind` set-variable support). Hosts that own an editor
    /// consult it at keymap-build time.
    pub editing_mode: Option<String>,
}

impl BindRegistry {
    /// Insert or replace the binding for `(keyseq, keymap)`. GNU's
    /// rl_generic_bind replaces any previous binding of the same sequence.
    pub fn register(&mut self, entry: BindEntry) {
        match self
            .entries
            .iter_mut()
            .find(|existing| existing.keyseq == entry.keyseq && existing.keymap == entry.keymap)
        {
            Some(existing) => existing.kind = entry.kind,
            None => self.entries.push(entry),
        }
        self.generation += 1;
    }

    /// Remove any binding for `keyseq` (bind -r; GNU's unbind_keyseq,
    /// bind.def:395-421, succeeds silently when nothing is bound).
    /// Returns true when an entry was removed.
    pub fn unbind_keyseq(&mut self, keyseq: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.keyseq != keyseq);
        let removed = self.entries.len() != before;
        if removed {
            self.generation += 1;
        }
        removed
    }

    /// Remove every binding for the named function (bind -u,
    /// bind.def:375-393 unbind_command → rl_unbind_function_in_map).
    /// Returns how many entries went away.
    pub fn unbind_function(&mut self, name: &str) -> usize {
        let before = self.entries.len();
        self.entries.retain(
            |entry| !matches!(&entry.kind, BindKind::Function { name: bound } if bound == name),
        );
        let removed = before - self.entries.len();
        if removed > 0 {
            self.generation += 1;
        }
        removed
    }

    /// Bindings restricted to one keymap (`None` keymap counts as
    /// emacs-standard, the default current keymap).
    pub fn entries_for_keymap<'a>(
        &'a self,
        keymap: Option<&'a str>,
    ) -> impl Iterator<Item = &'a BindEntry> + 'a {
        let wanted = normalize_keymap_name(keymap.unwrap_or("emacs-standard"));
        self.entries
            .iter()
            .filter(move |entry| match entry.keymap.as_deref() {
                None => wanted == "emacs-standard",
                Some(bound) => normalize_keymap_name(bound) == wanted,
            })
    }

    /// Remove any binding for `keyseq` within one keymap (`None` =
    /// emacs-standard). GNU's `bind -r` unbinds in the current keymap
    /// only (bind.def:395-421 unbind_keyseq), so a vi-insert binding for
    /// the same sequence survives an emacs-standard removal. Returns
    /// true when an entry went away.
    pub fn unbind_keyseq_for_keymap(&mut self, keyseq: &str, keymap: Option<&str>) -> bool {
        let wanted = normalize_keymap_name(keymap.unwrap_or("emacs-standard"));
        let before = self.entries.len();
        self.entries.retain(|entry| {
            if entry.keyseq != keyseq {
                return true;
            }
            let bound = normalize_keymap_name(entry.keymap.as_deref().unwrap_or("emacs-standard"));
            bound != wanted
        });
        let removed = self.entries.len() != before;
        if removed {
            self.generation += 1;
        }
        removed
    }

    pub fn snapshot(&self) -> Vec<BindEntry> {
        self.entries.clone()
    }
}

/// Keymap aliases that are the same readline map (bind.def:37-40: vi,
/// vi-move and vi-command name the movement keymap; emacs and
/// emacs-standard the standard one).
pub fn normalize_keymap_name(name: &str) -> &str {
    match name {
        "emacs" => "emacs-standard",
        "vi" | "vi-move" | "vi-command" => "vi-movement",
        other => other,
    }
}

/// The `-m` names GNU accepts (bind.def:37-40). Anything else is
/// ``bind: `<name>': invalid keymap name`` (bind.def:253-257).
pub const KEYMAP_NAMES: &[&str] = &[
    "emacs",
    "emacs-standard",
    "emacs-meta",
    "emacs-ctlx",
    "vi",
    "vi-move",
    "vi-command",
    "vi-insert",
];

/// Process-shared registry handle. Shared across ShellState clones so a
/// `bind` inside a subshell mutates the same world as the parent — GNU's
/// readline state is process-global, not fork-copied.
pub type SharedBindRegistry = Rc<RefCell<BindRegistry>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn xentry(keyseq: &str, command: &str) -> BindEntry {
        BindEntry {
            keyseq: keyseq.to_string(),
            kind: BindKind::Execute {
                command: command.to_string(),
            },
            keymap: None,
        }
    }

    #[test]
    fn register_replaces_same_keyseq_and_bumps_generation() {
        let mut registry = BindRegistry::default();
        registry.register(xentry("\\C-g", "one"));
        let gen = registry.generation;
        registry.register(xentry("\\C-g", "two"));
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.generation, gen + 1);
        match &registry.entries[0].kind {
            BindKind::Execute { command } => assert_eq!(command, "two"),
            other => panic!("unexpected kind: {other:?}"),
        }
    }

    #[test]
    fn unbind_keyseq_removes_and_reports() {
        let mut registry = BindRegistry::default();
        registry.register(xentry("\\C-g", "one"));
        assert!(registry.unbind_keyseq("\\C-g"));
        assert!(!registry.unbind_keyseq("\\C-g"));
        assert!(registry.entries.is_empty());
    }

    #[test]
    fn unbind_function_only_touches_function_entries() {
        let mut registry = BindRegistry::default();
        registry.register(xentry("\\C-g", "cmd"));
        registry.register(BindEntry {
            keyseq: "\\C-a".to_string(),
            kind: BindKind::Function {
                name: "beginning-of-line".to_string(),
            },
            keymap: None,
        });
        assert_eq!(registry.unbind_function("beginning-of-line"), 1);
        assert_eq!(registry.entries.len(), 1);
    }

    #[test]
    fn entries_for_keymap_defaults_to_emacs_standard_and_folds_aliases() {
        let mut registry = BindRegistry::default();
        registry.register(xentry("\\C-g", "default"));
        registry.register(BindEntry {
            keyseq: "\\C-i".to_string(),
            kind: BindKind::Execute {
                command: "vi-insert-only".to_string(),
            },
            keymap: Some("vi-insert".to_string()),
        });
        let names: Vec<_> = registry
            .entries_for_keymap(None)
            .map(|entry| entry.keyseq.as_str())
            .collect();
        assert_eq!(names, vec!["\\C-g"]);
        let vi_names: Vec<_> = registry
            .entries_for_keymap(Some("vi"))
            .map(|entry| entry.keyseq.as_str())
            .collect();
        assert!(vi_names.is_empty(), "vi alias must not match vi-insert");
        let viins_names: Vec<_> = registry
            .entries_for_keymap(Some("vi-insert"))
            .map(|entry| entry.keyseq.as_str())
            .collect();
        assert_eq!(viins_names, vec!["\\C-i"]);
    }

    #[test]
    fn shared_handle_is_shared_across_clone() {
        let shared: SharedBindRegistry = Rc::new(RefCell::new(BindRegistry::default()));
        let cloned = shared.clone();
        shared.borrow_mut().register(xentry("\\C-g", "one"));
        assert_eq!(cloned.borrow().entries.len(), 1);
    }
}
