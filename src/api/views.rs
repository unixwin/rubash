//! State views: the §5.3 layer of the Session API, plus the owned
//! snapshots the debugger's inspection surface renders (WP-6).
//!
//! Two shapes share this file, deliberately:
//!
//! - **Live views** ([`StateViews`] and the views it hands out) borrow
//!   the executor and operate on live state: this is the surface a
//!   host drives between runs — the designed form of the calls the
//!   API ledger's census found niubash making most (`get_env` ×84,
//!   `set_env` ×49, option setters, `last_exit_code` ×11). Every
//!   operation delegates to the engine's existing machinery; the view
//!   introduces no second write path (§7.6's rule for the debugger
//!   applies to hosts too: a state write goes through the machinery
//!   every other write uses).
//! - **Snapshots** ([`VariableSnapshot`], [`FrameSnapshot`],
//!   [`InspectionSnapshot`]) are owned data captured at a moment —
//!   at a debug stop, the consult fills the session's inspection
//!   slot from these types — and rendered by the debugger's WP-6
//!   rendering (`crate::debugger::inspect`). A snapshot's
//!   `declaration` text is rendered at capture time by the one
//!   declaration printer (`builtins::declare`), so snapshot text and
//!   `declare -p` output are the same bytes by construction.

use std::path::{Path, PathBuf};

use crate::executor::Executor;

/// The ten declare-family attributes of a variable, as the API
/// reports them. Mirrors the attribute word GNU keeps on the
/// SHELL_VAR (variables.c) and the typed store + mark sets rubash
/// maintains; capture merges those sources (see
/// `executor::state_views`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VariableAttrs {
    pub exported: bool,
    pub readonly: bool,
    pub integer: bool,
    pub nameref: bool,
    pub trace: bool,
    pub uppercase: bool,
    pub lowercase: bool,
    pub capcase: bool,
    pub array: bool,
    pub assoc: bool,
}

impl VariableAttrs {
    /// No attribute set (a plain scalar binding).
    pub fn is_empty(&self) -> bool {
        *self == VariableAttrs::default()
    }
}

/// A variable's value in typed-cell form (§5.3: array access goes
/// through the typed-cell view so hosts never touch the mirror
/// encoding). Indexed entries carry their real indices — a sparse
/// array snapshots sparse, never renumbered (debugger design §7.3).
/// Associative entries are sorted by key for snapshot determinism;
/// the shell's own hash order remains a presentation detail of the
/// mirror format functions, not of this view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariableValue {
    /// Declared but unset: distinct from unset-entirely (no snapshot
    /// is produced for an unbound name) and from empty.
    Unset,
    Scalar(String),
    Indexed(Vec<(i64, String)>),
    Associative(Vec<(String, String)>),
}

impl VariableValue {
    /// The scalar text, when the value is a scalar.
    pub fn as_scalar(&self) -> Option<&str> {
        match self {
            VariableValue::Scalar(text) => Some(text),
            _ => None,
        }
    }

    /// Element count for array values; None for scalar/unset.
    pub fn element_count(&self) -> Option<usize> {
        match self {
            VariableValue::Indexed(entries) => Some(entries.len()),
            VariableValue::Associative(entries) => Some(entries.len()),
            _ => None,
        }
    }
}

/// One variable, captured as the debuggee itself resolves it
/// (debugger design §7.2): innermost `local` scope outward to the
/// globals, through the engine's own stores — never a raw map index
/// under a different resolution rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableSnapshot {
    pub name: String,
    pub value: VariableValue,
    pub attrs: VariableAttrs,
    /// True when the binding is a dynamic parameter (RANDOM,
    /// PIPESTATUS, the stack arrays, …): its value is materialized
    /// by a getter at read time, so the snapshot is one read of a
    /// moving value — for RANDOM/SRANDOM-family names a second read
    /// would draw again, exactly as `$RANDOM` does.
    pub dynamic: bool,
    /// The local scope this binding was found in, counted outward
    /// from the innermost live scope (0 = innermost); None for a
    /// global binding.
    pub scope_depth: Option<usize>,
    /// The `declare -p NAME` line for this binding, rendered at
    /// capture time by the shared declaration printer; None when no
    /// printer form exists (a dynamic name whose declaration view
    /// does not render, e.g. FUNCNAME outside the named-lookup seed).
    pub declaration: Option<String>,
}

/// One call frame, zipped from the engine's dynamic stack views
/// (debugger design §7.1): `function`/`source` are FUNCNAME[i] /
/// BASH_SOURCE[i], and `line` is the line in `source` this frame is
/// currently executing — for frame 0 the live line stamp, for frame
/// i > 0 the BASH_LINENO[i-1] call line. That is exactly the triple
/// `caller i-1` prints for this frame, which is what keeps `where`
/// and `caller` from ever disagreeing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameSnapshot {
    pub function: String,
    pub source: String,
    pub line: String,
    /// The frame's positional parameters, in call order, from the
    /// BASH_ARGC/BASH_ARGV views. Empty when the arg arrays have not
    /// materialized (GNU pushes call frames only in debugging mode —
    /// execute_cmd.c push_args — so this is a state fact, not a
    /// snapshot gap).
    pub args: Vec<String>,
}

/// Which option family an [`OptionState`] belongs to: `set -o`
/// shell options or `shopt` options (§5.3's OptionView covers both,
/// with validity checked and reported).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionFamily {
    Shell,
    Shopt,
}

/// One option's current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionState {
    pub name: String,
    pub family: OptionFamily,
    pub enabled: bool,
}

/// The dynamic state a stop inspection reports alongside frames and
/// variables (debugger design §7.3): PIPESTATUS is a first-class
/// inspection target, and `$?` / `$BASH_COMMAND` render through the
/// same dynamic views the expander uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicStateSnapshot {
    /// `$?` at capture time.
    pub last_status: i32,
    /// The live PIPESTATUS cell's entries, with real (possibly
    /// sparse) indices; empty unless the cell is a live array.
    pub pipestatus: Vec<(i64, i32)>,
    /// `$BASH_COMMAND` — at a stop, the about-to-run command (the
    /// §4.2 ordering sets it before the consult).
    pub bash_command: Option<String>,
    /// The live line stamp (`$LINENO`'s source) at capture time.
    pub current_line: Option<u32>,
}

/// Everything a frontend renders at a stop: frames, the visible
/// variables, and the dynamic state — one capture, one moment, from
/// the engine's own views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionSnapshot {
    pub frames: Vec<FrameSnapshot>,
    pub variables: Vec<VariableSnapshot>,
    pub dynamic: DynamicStateSnapshot,
}

impl InspectionSnapshot {
    /// One variable by name, as captured.
    pub fn variable(&self, name: &str) -> Option<&VariableSnapshot> {
        self.variables.iter().find(|var| var.name == name)
    }
}

/// The §5.3 state views over one executor. Constructed per use from
/// [`Executor::state_views`]; the borrow is the session's
/// single-thread contract made literal (API design §11 Q5).
pub struct StateViews<'a> {
    executor: &'a mut Executor,
}

impl<'a> StateViews<'a> {
    /// The variables view.
    pub fn variables(&mut self) -> VariablesView<'_> {
        VariablesView {
            executor: self.executor,
        }
    }

    /// The options view.
    pub fn options(&mut self) -> OptionsView<'_> {
        OptionsView {
            executor: self.executor,
        }
    }

    /// `$?`, named plainly (§5.3: the ×11 call in the census).
    pub fn last_status(&self) -> i32 {
        self.executor.last_exit_code()
    }

    /// Set `$?` (the host-side counterpart niubash uses today as
    /// `set_last_exit_code`).
    pub fn set_last_status(&mut self, status: i32) {
        self.executor.set_last_exit_code(status);
    }

    /// The session's current working directory (G-02: a session
    /// field, never the process cwd).
    pub fn cwd(&self) -> &Path {
        self.executor.session_cwd()
    }

    /// Move the session's working directory through the `cd`
    /// machinery itself (bindpwd semantics: PWD/OLDPWD and the cwd
    /// field move together, or not at all). False when the move
    /// failed, with the session left where it was.
    pub fn set_cwd(&mut self, path: &Path) -> bool {
        self.executor.view_set_cwd(path)
    }

    /// The session's positional parameters (`$@` as a list).
    pub fn positional(&self) -> Vec<String> {
        self.executor.positional_params()
    }

    /// Replace the positional parameters (the `set --` path).
    pub fn set_positional(&mut self, args: Vec<String>) {
        self.executor.set_positional_params(args);
    }

    /// Alias snapshot: (name, value) pairs, sorted by name.
    pub fn aliases(&self) -> Vec<(String, String)> {
        let mut aliases: Vec<(String, String)> =
            self.executor.aliases_snapshot().into_iter().collect();
        aliases.sort();
        aliases
    }

    /// Define or replace an alias, through the same store the
    /// `alias` builtin writes (BASH_ALIASES re-syncs from it).
    pub fn define_alias(&mut self, name: &str, value: &str) {
        self.executor.view_define_alias(name, value);
    }

    /// Remove an alias; false when it did not exist.
    pub fn undefine_alias(&mut self, name: &str) -> bool {
        self.executor.view_undefine_alias(name)
    }

    /// Function names currently defined, sorted.
    pub fn functions(&self) -> Vec<String> {
        self.executor.functions_snapshot()
    }

    /// Whether a function of this name is defined.
    pub fn has_function(&self, name: &str) -> bool {
        self.executor.has_function(name)
    }

    /// Remove a function definition; false when it did not exist.
    /// Defining a function is running its definition — code goes
    /// through the run path (§5.2), not through a state view.
    pub fn undefine_function(&mut self, name: &str) -> bool {
        self.executor.view_undefine_function(name)
    }

    /// What children will inherit (§5.3's environment view): the
    /// exported scalar variables with their values — the same
    /// exportability filter the spawn paths apply
    /// (`is_exportable_env_name`: array/assoc cells never cross).
    /// Sorted by name.
    pub fn environment(&self) -> Vec<(String, String)> {
        self.executor.view_environment()
    }

    /// Capture the full inspection snapshot of the current state
    /// (frames + visible variables + dynamic state). At a debug
    /// stop the consult captures this into the session; hosts may
    /// capture it between runs.
    pub fn inspection(&self) -> InspectionSnapshot {
        self.executor.capture_inspection()
    }
}

/// The §5.3 variables view: get / set / export / unset / attributes,
/// with array access through the typed-cell view.
pub struct VariablesView<'a> {
    executor: &'a mut Executor,
}

impl VariablesView<'_> {
    /// Capture one variable as the shell resolves it; None when the
    /// name is unbound.
    pub fn get(&self, name: &str) -> Option<VariableSnapshot> {
        self.executor.capture_variable(name)
    }

    /// The value a `$name` read produces (dynamic getters and
    /// nameref resolution included), as one scalar text; None when
    /// the read is unset.
    pub fn resolved_value(&self, name: &str) -> Option<String> {
        self.executor.view_resolved_value(name)
    }

    /// Bind a scalar through the host write path (the designed form
    /// of `set_env`: the mirror write that also syncs the typed
    /// scalar cell in place).
    pub fn set(&mut self, name: &str, value: &str) {
        self.executor.set_env(name, value);
    }

    /// Whole-value bind of an indexed array from typed cells,
    /// through the engine's own array-state store. Attribute-driven
    /// transforms (integer evaluation, case mods) are the assignment
    /// path's job, not the view's — semantic writes go through
    /// execution (§7.6's rule). False on a refused bind (negative
    /// index, or the name is an associative array).
    pub fn set_array(&mut self, name: &str, cells: &[(i64, String)]) -> bool {
        self.executor.view_set_indexed_array(name, cells)
    }

    /// Whole-value bind of an associative array from typed cells;
    /// same contract as [`VariablesView::set_array`].
    pub fn set_assoc(&mut self, name: &str, cells: &[(String, String)]) -> bool {
        self.executor.view_set_assoc_array(name, cells)
    }

    /// Set or clear the exported attribute, through the same mark
    /// machinery the `export` builtin uses.
    pub fn set_exported(&mut self, name: &str, exported: bool) {
        self.executor.view_set_exported(name, exported);
    }

    /// Unbind a variable with the unset builtin's own leaf sequence
    /// (dynamic unbind record, mirror + typed removal, PIPESTATUS
    /// cell reset). False when the name refuses unbinding (the
    /// att_nounset stack arrays) or was not bound.
    pub fn unset(&mut self, name: &str) -> bool {
        self.executor.view_unset_variable(name)
    }

    /// The attributes of one bound variable.
    pub fn attributes(&self, name: &str) -> Option<VariableAttrs> {
        self.executor.capture_variable(name).map(|snap| snap.attrs)
    }
}

/// The §5.3 options view: `set -o` and `shopt` options by name,
/// with validity checked and reported (an unknown name reads None
/// and refuses to set, the honesty `set_shopt_option` already had).
pub struct OptionsView<'a> {
    executor: &'a mut Executor,
}

impl OptionsView<'_> {
    /// One option's state by name (either family); None for a name
    /// neither family knows.
    pub fn get(&self, name: &str) -> Option<bool> {
        self.executor.view_option_enabled(name)
    }

    /// Set one option by name; false when the name is unknown to both
    /// families, or when the flip itself is refused (no state changes
    /// then). Flag-only shell options (the `restricted` of GNU
    /// flags.c shell_flags[]) go through the same `set_shell_option`
    /// machinery the `set -r` builtin branch uses (rubash#2040), with
    /// GNU's one-way gate intact: change_flag refuses FLAG_OFF while
    /// the option is on (flags.c:231-233).
    pub fn set(&mut self, name: &str, enabled: bool) -> bool {
        if self.executor.is_shell_option(name) {
            self.executor.set_shell_option(name, enabled);
            true
        } else if crate::builtins::set::is_flag_only_shell_option(name) {
            let armed = self.executor.view_option_enabled(name) == Some(true);
            if armed && !enabled {
                return false;
            }
            self.executor.set_shell_option(name, enabled);
            true
        } else {
            self.executor.set_shopt_option(name, enabled)
        }
    }

    /// Every option in both families with its current state, shell
    /// options first, each family in its table order.
    pub fn list(&self) -> Vec<OptionState> {
        self.executor.capture_options()
    }
}

impl Executor {
    /// The §5.3 state views over this executor (API design §5.3;
    /// PLAN row 6).
    pub fn state_views(&mut self) -> StateViews<'_> {
        StateViews { executor: self }
    }

    /// The §5.3 variables view directly over this executor —
    /// the one-hop form for owners that return the view itself
    /// (the Session API): a view borrowed through a temporary
    /// `StateViews` cannot outlive the call, so the constructor
    /// lives here, beside `state_views`.
    pub fn variables_view(&mut self) -> VariablesView<'_> {
        VariablesView { executor: self }
    }

    /// The §5.3 options view directly over this executor (the
    /// one-hop form of `state_views().options()`).
    pub fn options_view(&mut self) -> OptionsView<'_> {
        OptionsView { executor: self }
    }

    /// The session cwd as an owned path (convenience for hosts that
    /// cannot hold the borrow).
    pub fn state_cwd_owned(&self) -> PathBuf {
        self.session_cwd().to_path_buf()
    }
}
