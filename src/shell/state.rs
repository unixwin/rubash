//! Shell semantic state — the single isolation boundary for subshells.
//!
//! GNU reference: `execute_cmd.c:1576 execute_in_subshell` runs `( list )`
//! and `subst.c:7143 command_substitute` runs `$( )` in forked children, so
//! every mutable shell datum — variables, aliases, functions, positional
//! parameters, local scopes, completion specs, the history list, job
//! bookkeeping — is isolated by the fork copy and discarded on child exit.
//! Rubash executes flat subshells and command substitutions in-process, so
//! the same boundary is expressed as `ShellState::clone`: cloning produces
//! the child's state, and assigning the parent's saved clone back restores
//! it. New semantic shell state MUST be added here — never as an Executor
//! field — so isolation stays automatic instead of depending on
//! hand-maintained save/restore lists that drift (the alias/function leak
//! that motivated this structure came from exactly such a list).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::builtins::alias::Alias;
use crate::builtins::complete::CompletionRegistry;
use crate::executor::{FunctionBody, FunctionDefInfo, FunctionDefinitionLocation, RandomGen};
use crate::history::SessionHistory;
use crate::parser::CommandNode;
use crate::shell::Variable;

use super::var_table::{VarAttrs, VarTable};
use super::variables::VariableStore;

#[derive(Debug)]
pub struct ShellState {
    /// variables.c shell_variables — typed store carrying attributes,
    /// arrays, and assoc maps.
    pub(crate) variables: VariableStore,
    /// variables.c shell_variables flat mirror — env/option carrier map,
    /// plus the structured attribute map (variables.h:124-133 att_* port;
    /// see `var_table`). Kept in sync with `variables` at mutation sites
    /// during the ongoing typed-store migration.
    pub(crate) env_vars: VarTable,
    /// The session's current working directory, in physical (native)
    /// form. GNU keeps the cwd in the process (builtins/cd.def cd_builtin
    /// -> chdir), so a forked child inherits a COPY of it — ShellState
    /// Clone is this port's fork boundary, and this field rides the
    /// clone. The PROCESS cwd is not session state (multi-session
    /// stop-bleed G-02): `cd`/`pushd`/`popd` move only this field, every
    /// spawn passes it as `Command::current_dir`, and engine-internal
    /// relative path resolution anchors here. `$PWD` in `env_vars` is
    /// the logical display twin maintained by the cd family; this field
    /// is the physical truth the filesystem sees.
    pub(crate) cwd: std::path::PathBuf,
    /// rubash#618/#616: element writes append one storage segment in O(1)
    /// instead of re-rendering every key, so the storage string accumulates
    /// duplicate segments until the next canonical render folds them. This is
    /// how many appends each array is still allowed before that render runs;
    /// it is seeded from the array's own entry count, so a compaction costs
    /// O(entries) once per O(entries) appends (amortized O(1) per write) and
    /// the string never exceeds twice its canonical length, keeping read-side
    /// parses as cheap as before. A scheduling hint only: a stale budget moves
    /// the compaction, never the resulting value.
    pub(crate) array_compact_budget: HashMap<String, u32>,
    /// alias.c aliases.
    pub(crate) aliases: HashMap<String, Alias>,
    /// variables.c shell_functions.
    pub(crate) functions: HashMap<String, FunctionBody>,
    /// Redirect words parsed at function definition time, applied at call.
    pub(crate) function_definition_redirects: HashMap<String, CommandNode>,
    /// Definition-shape metadata (body kind, def redirects).
    pub(crate) function_def_infos: HashMap<String, FunctionDefInfo>,
    /// Definition source locations for DEBUG trap reporting.
    pub(crate) function_definition_locations: HashMap<String, FunctionDefinitionLocation>,
    /// variables.c positional_params ($1..$#, $*, $@).
    pub(crate) positional_params: Vec<String>,
    /// Offset-window base for `shift` (variables.c `loptend` port): `$1`
    /// is `positional_params[positional_offset]`, `$#` is
    /// `len - offset`. `shift n` bumps the offset instead of
    /// clone+drain; `set --` replaces the storage and resets it to 0.
    /// Invariant: `offset <= len` (empty window is `offset == len`).
    pub(crate) positional_offset: usize,
    /// execute_cmd.c/jobs.c pipeline status vector ($PIPESTATUS).
    /// The PIPESTATUS array object as sparse (index, status) entries
    /// in ascending index order with unique indices — the live ARRAY
    /// of variables.c:6291 set_pipestatus_array, whose writer
    /// preserves existing indices (update in place / insert at
    /// position) and only rebuilds densely when the element count
    /// shrinks past the status count. Only meaningful while
    /// `pipestatus_shape` is `LiveArray`; reads of element 0 (the
    /// scalar `$PIPESTATUS`, arithmetic contexts) resolve by INDEX,
    /// never by position in this list (rubash#1364 R3).
    pub(crate) pipestatus: Vec<(i64, i32)>,
    /// variables.c:6291 set_pipestatus_array PIPESTATUS cell shape
    /// (rubash#1363/#1364): the status writer stores straight into the
    /// ARRAY object and never consults att_readonly — it is a no-op
    /// unless the PIPESTATUS cell currently holds an array object.
    /// A valueless declaration (`readonly`/`declare`/`export` naming
    /// PIPESTATUS before any command materialized it) creates an
    /// att_invisible cell with NO array object (array_insert(NULL)
    /// fails silently), and a scalar assignment creates a scalar cell;
    /// both shapes freeze the vector until an array-shaped bind
    /// (compound/element assignment, or `declare -a` over a scalar)
    /// installs an array object.
    pub(crate) pipestatus_shape: PipestatusShape,
    /// variables.c FUNCNAME stack.
    pub(crate) function_name_stack: Vec<String>,
    /// variables.c BASH_ARGC/BASH_ARGV stacks.
    pub(crate) bash_argc_stack: Vec<String>,
    pub(crate) bash_argv_stack: Vec<String>,
    /// The BASH_ARGV bottom frame (the shell's own positional parameters,
    /// reversed into display order) installed by init_bash_argv the first
    /// time the arg arrays materialize (rubash#1051). A RefCell because
    /// GNU's materialization can happen inside the variables.c getter
    /// (get_bashargcv) during a read, where the executor is shared.
    /// None while the arrays have never been initialized.
    pub(crate) bash_arg_bottom: std::cell::RefCell<Option<Vec<String>>>,
    /// variables.c BASH_LINENO/BASH_SOURCE stacks.
    pub(crate) bash_lineno_stack: Vec<String>,
    pub(crate) bash_source_stack: Vec<String>,
    /// variables.c variable_context — local scope stacks.
    pub(crate) local_var_scopes: Vec<HashMap<String, Option<String>>>,
    pub(crate) local_attr_scopes: Vec<HashMap<String, VarAttrs>>,
    pub(crate) local_typed_scopes: Vec<HashMap<String, Option<Variable>>>,
    /// alias.c expansion recursion guard stack.
    pub(crate) expanding_aliases: Vec<String>,
    /// execute_cmd.c loop_level — break/continue scope boundary.
    pub(crate) loop_depth: usize,
    /// Loop depth inherited across a fork boundary (command substitution
    /// body, simple pipeline/async element): GNU's fork copies loop_level
    /// into the child, whose parse_and_execute is not wrapped by
    /// execute_in_subshell, so break/continue there are meaningful and
    /// silently end the child's own command list (builtins/break.def
    /// check_loop_level sees loop_level > 0). Compound elements and `( )`
    /// subshells DO pass execute_in_subshell, which resets loop_level to 0
    /// (execute_cmd.c:1642, Posix interp 842), so those boundaries leave
    /// this at 0 (rubash#1064, rubash#1065).
    pub(crate) inherited_loop_depth: usize,
    /// rubash#817 read-unit identity for the executor-level alias same-read
    /// guard (arithmetic_aliases.rs alias_defined_on_current_line). GNU
    /// expands an alias use only when the definition was consumed by an
    /// earlier read of the SAME input stream (parse.y read-time expansion);
    /// `read_generation` is the stream identity of the commands executing
    /// now, and `read_generation_ceiling` is the monotonic high-water every
    /// nested read unit draws a fresh value from, so sibling units never
    /// share an identity. enter_read_unit/exit_read_unit push/pop at the
    /// evalfile.c:253-257 push_stream boundaries (builtins/source/
    /// execution.rs for `source`, executor/trap_exec.rs execute_eval_source
    /// for `eval`). Cloned across the fork boundary like every C global —
    /// a subshell body is the same read unit as its parent.
    pub(crate) read_generation: usize,
    pub(crate) read_generation_ceiling: usize,
    /// shell.c:183 `current_command_number` (init 1): the reader_loop
    /// increments it per command list read, before execution (eval.c:178).
    /// `\#` in prompt expansion reads it (parse.y:6568-6574; in a script
    /// `${var@P}` sees decoding_prompt == ps0_prompt == NULL, so the
    /// ps0/ps1/ps2 compensation does NOT apply and the raw counter shows).
    /// Copied across the fork boundary like every C global.
    pub(crate) command_number: usize,
    /// variables.c funcnest / function call depth.
    pub(crate) function_depth: usize,
    /// variables.c dollar_vars_changed bookkeeping for `set` scope.
    pub(crate) dollar_vars_changed_by_set: bool,
    /// lib/sh/random.c RANDOM/SRANDOM generator state.
    pub(crate) random_state: RandomGen,
    /// execute_cmd.c subshell_level.
    pub(crate) subshell_depth: Cell<usize>,
    /// print_cmd.c make_command_string of the innermost enclosing
    /// `( )` subshell, rendered at subshell entry (executor
    /// execute_subshell_command_with_redirects / the ast_exec region
    /// open). GNU's waitchld death notice names the WHOLE job command
    /// (jobs.c:4625), so an `exec`ed command dying by signal inside
    /// the parens is reported against this text, not its own
    /// last-member text (rubash#1493). Set only after the entry
    /// clone_for_child_save, so the wholesale boundary restore
    /// reverts it to the parent's value like every other field here.
    pub(crate) subshell_source_text: Option<String>,
    /// execute_cmd.c subshell_environment & SUBSHELL_COMSUB — set while a
    /// `$( )`/backtick body runs (shared-executor and cloned-executor
    /// paths). jobs.c:3837-3843 start_job refuses fg/bg inside it.
    pub(crate) in_command_substitution: Cell<bool>,
    /// print_cmd.c:445 indirection_level_string / shell.c:186
    /// indirection_level: how many parse_and_execute-style command lists
    /// are stacked ABOVE the shell's base reader (eval.c:63-70). The xtrace
    /// prefix repeats PS4's first character once per level, so a trap
    /// action (trap.c:496 parse_and_execute) traces as `++...` under
    /// `set -x`. rubash's base is 0 (indistinguishable from GNU's 1: both
    /// render the PS4 unchanged); each nested trap action adds one.
    pub(crate) xtrace_indirection_level: Cell<usize>,
    /// print_cmd.c:445 indirection_level_string suspends xtrace while it
    /// expands PS4 (`change_flag ('x', FLAG_OFF)` around
    /// decode_prompt_string, restored afterwards). While suspended,
    /// `xtrace_enabled()` reads false so command substitutions inside PS4
    /// run untraced (rubash#1040), and `shell_option_flags()` omits `x`
    /// so `$-` expanded in PS4 context has no `x` (rubash#1042) — both
    /// match the C flag flip. A depth counter (not the env var itself)
    /// so an inner `set -x` cannot re-enable tracing mid-expansion and
    /// recurse unboundedly (a `PS4='$(set -x; ...)'` body hangs GNU 5.3
    /// the same way; the gate terminates instead).
    pub(crate) xtrace_suspended: Cell<usize>,
    /// execute_cmd.c:199 stdin_redir — the global recording whether the
    /// currently executing control structure redirected fd 0
    /// (execute_cmd.c:828 sets it from stdin_redirects, redir.c:1435).
    /// eval.c:181 clears it before each top-level reader command. An
    /// async `cmd &` consults it (execute_cmd.c:2837): with job control
    /// off or inside a subshell, a cleared flag sends the child's stdin to
    /// /dev/null; a set flag inherits the redirected stream instead.
    pub(crate) stdin_redir: Cell<bool>,
    /// jobs.c bookkeeping — the job registry (jobs.c `jobs` array /
    /// `job_table`): bash-observable job identity, pipeline pids, states,
    /// and notification bits. Process handles are Executor resources and
    /// never cloned. GNU jobs.c last_made_pid ($!).
    pub(crate) job_table: crate::jobs::table::JobTable,
    pub(crate) last_background_pid: Option<u32>,
    /// pids of coproc processes → bound array variable name (coproc.c
    /// coproc_setvars keeps c_name so coproc_unsetvars can unbind even on
    /// bind failure).
    pub(crate) coproc_names: HashMap<u32, String>,
    /// pcomplete.c completion spec registry.
    pub(crate) completion_specs: CompletionRegistry,
    /// readline binding registry (`bind` builtin's data plane):
    /// function/macro bindings and the `bind -x` command map
    /// (bashline.c:4771 init_unix_command_map). Unlike session_history,
    /// the Rc is SHARED across clones: readline keymaps are process-global
    /// C globals in GNU, so a `bind` inside a subshell must reach the
    /// parent's world (there is no fork to isolate it).
    pub(crate) bind_registry: crate::shell::bind_registry::SharedBindRegistry,
    /// bashhist.c per-session history list. Cloned deeply (not the Rc) so a
    /// subshell's `history` mutations cannot reach the parent — GNU gets
    /// the same isolation from the fork copy.
    pub(crate) session_history: Option<Rc<RefCell<SessionHistory>>>,
    /// histexpand.c process statics (search_string, search_match, subst_lhs,
    /// subst_rhs) that persist across history expansions within one shell —
    /// GNU keeps them as globals for the process lifetime. Carried here so
    /// the interactive pre_process_line path
    /// (script_driver::pre_process_interactive_line) keeps them across
    /// calls even when a host provider owns the list.
    pub(crate) interactive_hist_engine: crate::history_expand::HistEngineState,
    /// Arithmetic expansion error flags (subshell boundary).
    /// GNU expr.c raises FORCE_EOF on unbound variable under `set -u`;
    /// these flags isolate errors in command substitutions from the outer shell.
    /// TODO: Move to ShellState::clone() for automatic isolation (requires &mut self API change)
    pub(crate) arithmetic_expansion_error: Cell<bool>,
    pub(crate) arithmetic_nonfatal_error: Cell<bool>,
    pub(crate) arithmetic_fatal_error: Cell<bool>,
    pub(crate) arithmetic_nounset_error: Cell<bool>,
    /// rubash#1238: the readonly diagnostic for a failed arithmetic bind
    /// was already printed by report_arithmetic_readonly_error; the
    /// command-layer reporters must not follow it with a synthesized
    /// syntax error (GNU prints exactly one line for `((ro++))`).
    pub(crate) arithmetic_readonly_error: Cell<bool>,
    /// Latch for a nounset fatal raised while expanding a `[[ =~ ]]`
    /// operand (rubash#1177): the diagnostic prints at detection inside
    /// conditional evaluation (which returns only an i32 status), and
    /// execute_conditional_command_with_redirects converts the latch to
    /// the FORCE_EOF exit, mirroring arithmetic_nounset_error's flow
    /// through execute_bare_arithmetic_command.
    pub(crate) conditional_nounset_error: Cell<bool>,
    pub(crate) arithmetic_last_error_category:
        Cell<Option<crate::executor::arithmetic::ArithmeticErrorCategory>>,
    /// Bad substitution flag (subshell boundary).
    /// GNU subst.c:10277 - a mid-expansion bad substitution kills the
    /// substitution's command list, never the enclosing word's command.
    /// TODO: Move to ShellState::clone() for automatic isolation (requires &mut self API change)
    pub(crate) parameter_bad_substitution: Cell<bool>,
    /// A redirect-target pathname expansion failed under failglob (GNU
    /// subst.c:12668 report_error + exp_jump_to_top_level(DISCARD) inside
    /// redirection_expand -> expand_words_no_vars). The diagnostic prints
    /// at detection (expand_word.rs expand_redirect_operand_fields); the
    /// first redirect-expansion site of the command (the simple-command
    /// redirect gate, the compound redirect appliers) consumes the flag
    /// and aborts the command list with status 1, like the argument-word
    /// failglob port.
    pub(crate) redirect_failglob_error: Cell<bool>,
    /// Non-consumed latch for the same event: the stage-subshell boundary
    /// (pipeline_stages.rs execute_compound_pipeline_stage) reads it to
    /// propagate the abort OUT of the stage instead of letting the parent
    /// routing phase re-expand and open the failed target (GNU performs one
    /// do_redirections in the forked child and the parent never applies the
    /// element's redirections at all).
    pub(crate) redirect_failglob_seen: Cell<bool>,
    /// variables.c this_command_name / BASH_COMMAND source — the command
    /// text the DEBUG trap reports. A command substitution evaluates it
    /// against the substitution's own source; the clone boundary restores
    /// the outer command automatically.
    pub(crate) debug_trap_command: RefCell<Option<String>>,
    /// print_cmd.c xtrace_fd — resolved descriptor xtrace writes to
    /// (-1 = stderr). variables.c sv_xtracefd resolves BASH_XTRACEFD on
    /// each assignment to it.
    pub(crate) xtrace_fd: Cell<i32>,
    /// The BASH_XTRACEFD value `xtrace_fd` was last resolved against, so
    /// xtrace writes can realign when the variable changed through a path
    /// that skipped the resolve hook (read/local/unset/direct store).
    pub(crate) xtrace_fd_source: RefCell<String>,
    /// subst.c:7143 command_substitute — a `<(cmd)` word names a
    /// pipe-like stream: every open shares one offset that drains on
    /// read (procsub.tests count_lines: repeated `wc -l < $1` ->
    /// 1,0,0,0,0). rubash writes the capture to a temp file so real
    /// external argv consumers get a path; in-shell opens of a
    /// registered path serve the remaining bytes and advance the shared
    /// offset. The `Rc` keeps the stream shared across `ShellState`
    /// clones, matching fork's shared open file description.
    pub(crate) procsub_streams: RefCell<HashMap<PathBuf, Rc<RefCell<ProcSubStream>>>>,
    /// rubash#1691: ad-hoc carrier temps written when a registered
    /// stream (or a text/fd endpoint) is served to a consumer that
    /// needs a real `File` — `open_input_redirect`'s consume branch and
    /// `open_fd_read_endpoint` write the remaining bytes to a FRESH
    /// temp, hand out the opened File, and park the path here; the
    /// per-command process-substitution finish removes the files.
    /// Before this list existed the paths were dropped at the write
    /// site and the carriers leaked in $TMPDIR for the shell's whole
    /// lifetime. A clone starts empty: a fork child may only drain
    /// carriers it wrote itself, never the parent's pending ones.
    pub(crate) procsub_adhoc_carriers: RefCell<Vec<PathBuf>>,
    /// rubash#1907: the PROCESS umask recorded at the fork-save point
    /// (`clone_for_child_save`). The `umask` builtin applies the mask
    /// to the process (builtins/umask.rs), and a flat `( )` region has
    /// no fork to discard the child's mask the way GNU's
    /// execute_in_subshell (execute_cmd.c:1576) does — so the save
    /// point snapshots the process mask into the saved state and
    /// `restore_flat_subshell` re-applies it when the region ends.
    /// Only the SAVED snapshot's copy is ever read; the live state's
    /// own copy is inert. None where there is no process umask
    /// (Windows) or when the state was never a fork snapshot.
    pub(crate) fork_saved_process_umask: Option<u32>,
}

/// The shape of the PIPESTATUS variable cell (variables.c:6291
/// set_pipestatus_array's `array_p (v)` / `array_cell (v)` gate):
/// `LiveArray` cells hold a real ARRAY object the status writer
/// updates in place; `Scalar` cells hold a plain scalar value the
/// writer leaves alone; `Valueless` cells (att_invisible, from a
/// valueless `declare`/`readonly`/`export` operand) have neither a
/// value nor an array object, so the writer's inserts fail and the
/// variable reads as unset. `ValuelessArray` is the same frozen
/// cell created with att_array (`declare -a PIPESTATUS` before
/// materialization): still no ARRAY object for the writer, but
/// `declare -p` renders the `-a` flag. `Absent` is no cell at
/// all — the next status write materializes a fresh array
/// (make_new_array_variable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PipestatusShape {
    Absent,
    LiveArray,
    Scalar,
    Valueless,
    ValuelessArray,
}

impl PipestatusShape {
    pub(crate) fn snapshot_name(self) -> &'static str {
        match self {
            PipestatusShape::Absent => "Absent",
            PipestatusShape::LiveArray => "LiveArray",
            PipestatusShape::Scalar => "Scalar",
            PipestatusShape::Valueless => "Valueless",
            PipestatusShape::ValuelessArray => "ValuelessArray",
        }
    }

    pub(crate) fn from_snapshot_name(name: &str) -> Option<Self> {
        match name {
            "Absent" => Some(PipestatusShape::Absent),
            "LiveArray" => Some(PipestatusShape::LiveArray),
            "Scalar" => Some(PipestatusShape::Scalar),
            "Valueless" => Some(PipestatusShape::Valueless),
            "ValuelessArray" => Some(PipestatusShape::ValuelessArray),
            _ => None,
        }
    }
}

/// Scope-snapshot key under which the PIPESTATUS shape note and
/// status vector ride through the two binding save/restore
/// mechanisms that restore the variable CELL but historically not
/// its shape (rubash#1525): the tempenv previous-list of
/// apply/restore_temporary_assignments and the frame local scopes
/// popped by restore_function_locals. GNU keeps shape and value in
/// the one SHELL_VAR object (variables.c pop_var_context /
/// dispose_temporary_env reinstall the saved object whole), so the
/// pair encoded by `encode_pipestatus_state` travels with the cell
/// snapshot and is reinstalled with it.
pub(crate) const PIPESTATUS_STATE_SNAPSHOT_KEY: &str = "__RUBASH_PIPESSTATUS_STATE";

/// Encode a (shape, status vector) pair for a scope snapshot:
/// `Shape|i0:v0,i1:v1,...` over the sparse (index, status) vector
/// (the vector segment is empty for no vector). Indices ride along
/// because the live vector is sparse (rubash#1364 R3): a snapshot
/// that densified it would resurrect unset elements on restore.
pub(crate) fn encode_pipestatus_state(shape: PipestatusShape, vector: &[(i64, i32)]) -> String {
    let mut out = shape.snapshot_name().to_string();
    out.push('|');
    for (position, (index, status)) in vector.iter().enumerate() {
        if position > 0 {
            out.push(',');
        }
        out.push_str(&index.to_string());
        out.push(':');
        out.push_str(&status.to_string());
    }
    out
}

pub(crate) fn decode_pipestatus_state(text: &str) -> Option<(PipestatusShape, Vec<(i64, i32)>)> {
    let (name, vector_text) = text.split_once('|')?;
    let shape = PipestatusShape::from_snapshot_name(name)?;
    let mut vector = Vec::new();
    if !vector_text.is_empty() {
        for part in vector_text.split(',') {
            let (index, status) = part.split_once(':')?;
            vector.push((index.parse::<i64>().ok()?, status.parse::<i32>().ok()?));
        }
    }
    Some((shape, vector))
}

/// Buffered contents of one materialized `<(cmd)` output — see
/// `ShellState::procsub_streams`.
#[derive(Debug)]
pub(crate) struct ProcSubStream {
    pub(crate) data: Vec<u8>,
    pub(crate) offset: usize,
    /// rubash#1141: production schedule of the substitution body, so
    /// in-shell consumers of the stream observe GNU's pipe arrival
    /// times for read/select deadlines (see executor::timed_input).
    pub(crate) timing: Option<crate::executor::timed_input::SharedTimedInput>,
}

/// Typed snapshot of the interior-mutable slice of ShellState.
///
/// Word expansion holds `&Executor`, so an in-place command substitution
/// can only mutate state behind `Cell`/`RefCell` — plain fields are frozen
/// under `&self`. The subshell boundary for a shared-executor `$( )`
/// therefore only needs to save/restore this slice; every NEW interior-
/// mutable field added to ShellState MUST join this snapshot (the fork
/// path via `command_substitution_executor` covers it automatically
/// through `Clone`, but this path does not).
///
/// Deliberate exemption: `procsub_streams` is NOT snapshotted. GNU fork
/// shares the parent's open file descriptions, so a `<(cmd)` stream's
/// read offset stays shared across the `$( )` boundary — draining in the
/// child must drain the parent's stream too. A stream registered inside
/// the child leaves a dead map entry in the parent after restore, which
/// is harmless: its temp path is unique and already deleted.
#[derive(Debug)]
pub(crate) struct InteriorSnapshot {
    subshell_depth: usize,
    in_command_substitution: bool,
    stdin_redir: bool,
    xtrace_suspended: usize,
    arithmetic_expansion_error: bool,
    arithmetic_nonfatal_error: bool,
    arithmetic_fatal_error: bool,
    arithmetic_nounset_error: bool,
    arithmetic_readonly_error: bool,
    conditional_nounset_error: bool,
    arithmetic_last_error_category: Option<crate::executor::arithmetic::ArithmeticErrorCategory>,
    parameter_bad_substitution: bool,
    redirect_failglob_error: bool,
    redirect_failglob_seen: bool,
    debug_trap_command: Option<String>,
    xtrace_fd: i32,
    xtrace_fd_source: String,
    /// rubash#1907: the PROCESS umask at the command-substitution
    /// boundary. The body runs in this same process, so a `umask`
    /// inside `$( )` re-masks the process itself; GNU's substitution
    /// child (subst.c:7143 command_substitute) discards it at exit.
    /// Snapshotted/restored with the interior state so the leak
    /// cannot reach the parent (the `( )` boundary's twin fix rides
    /// `fork_saved_process_umask` on the full-state snapshot).
    process_umask: Option<u32>,
}

impl InteriorSnapshot {
    pub(crate) fn subshell_depth(&self) -> usize {
        self.subshell_depth
    }
}

impl ShellState {
    /// Live positional window (`shift`-visible `$1..$#`): the storage tail
    /// starting at `positional_offset`. GNU keeps the whole `list` and
    /// moves `loptend` (bash variables.c); every reader must go through
    /// this instead of the raw field so a shifted prefix stays invisible.
    pub(crate) fn positional_window(&self) -> &[String] {
        self.positional_params
            .get(self.positional_offset..)
            .unwrap_or(&[])
    }

    /// `$#` — the live window length, not the backing storage length.
    pub(crate) fn positional_len(&self) -> usize {
        self.positional_params
            .len()
            .saturating_sub(self.positional_offset)
    }

    /// Owned copy of the live window (what `$@` clones).
    pub(crate) fn positional_window_cloned(&self) -> Vec<String> {
        self.positional_window().to_vec()
    }

    /// subst.c:7143 command_substitute / execute_cmd.c:1576
    /// execute_in_subshell: a `$( )` body runs in a forked child, so none
    /// of its state mutations may reach the parent. Shared-executor fast
    /// paths can only touch interior-mutable fields, so saving this slice
    /// reproduces the fork boundary for exactly what they can write.
    pub(crate) fn snapshot_interior(&self) -> InteriorSnapshot {
        InteriorSnapshot {
            subshell_depth: self.subshell_depth.get(),
            in_command_substitution: self.in_command_substitution.get(),
            stdin_redir: self.stdin_redir.get(),
            xtrace_suspended: self.xtrace_suspended.get(),
            arithmetic_expansion_error: self.arithmetic_expansion_error.get(),
            arithmetic_nonfatal_error: self.arithmetic_nonfatal_error.get(),
            arithmetic_fatal_error: self.arithmetic_fatal_error.get(),
            arithmetic_nounset_error: self.arithmetic_nounset_error.get(),
            arithmetic_readonly_error: self.arithmetic_readonly_error.get(),
            conditional_nounset_error: self.conditional_nounset_error.get(),
            arithmetic_last_error_category: self.arithmetic_last_error_category.get(),
            parameter_bad_substitution: self.parameter_bad_substitution.get(),
            redirect_failglob_error: self.redirect_failglob_error.get(),
            redirect_failglob_seen: self.redirect_failglob_seen.get(),
            debug_trap_command: self.debug_trap_command.borrow().clone(),
            xtrace_fd: self.xtrace_fd.get(),
            xtrace_fd_source: self.xtrace_fd_source.borrow().clone(),
            process_umask: crate::builtins::umask::snapshot_process_umask(),
        }
    }

    /// Saved-state snapshot for an in-place forked-child boundary
    /// (execute_cmd.c:1576 execute_in_subshell): `Clone` deep-copies
    /// `session_history` so the child's list mutations are private, but
    /// the private copy belongs to the CHILD — the saved snapshot must
    /// keep the parent's live `Rc`. The script driver records into the
    /// same session object, so restoring the stale clone would split the
    /// history list: later records land on one object while `history`/`fc`
    /// read the other. Swap so `self` (the child view) holds the clone
    /// and the returned snapshot holds the original.
    pub(crate) fn clone_for_child_save(&mut self) -> Self {
        let mut saved = self.clone();
        std::mem::swap(&mut saved.session_history, &mut self.session_history);
        // rubash#1907: the fork boundary is also where the PROCESS
        // umask is snapshotted — the child region runs against this
        // same process, so only a snapshot taken NOW lets the restore
        // put the parent's mask back (see the field docs).
        saved.fork_saved_process_umask = crate::builtins::umask::snapshot_process_umask();
        saved
    }

    /// Restore a snapshot taken by `snapshot_interior`.
    pub(crate) fn restore_interior(&self, snapshot: &InteriorSnapshot) {
        self.subshell_depth.set(snapshot.subshell_depth);
        self.in_command_substitution
            .set(snapshot.in_command_substitution);
        self.stdin_redir.set(snapshot.stdin_redir);
        self.xtrace_suspended.set(snapshot.xtrace_suspended);
        self.arithmetic_expansion_error
            .set(snapshot.arithmetic_expansion_error);
        self.arithmetic_nonfatal_error
            .set(snapshot.arithmetic_nonfatal_error);
        self.arithmetic_fatal_error
            .set(snapshot.arithmetic_fatal_error);
        self.arithmetic_nounset_error
            .set(snapshot.arithmetic_nounset_error);
        self.arithmetic_readonly_error
            .set(snapshot.arithmetic_readonly_error);
        self.conditional_nounset_error
            .set(snapshot.conditional_nounset_error);
        self.arithmetic_last_error_category
            .set(snapshot.arithmetic_last_error_category);
        self.parameter_bad_substitution
            .set(snapshot.parameter_bad_substitution);
        self.redirect_failglob_error
            .set(snapshot.redirect_failglob_error);
        self.redirect_failglob_seen
            .set(snapshot.redirect_failglob_seen);
        *self.debug_trap_command.borrow_mut() = snapshot.debug_trap_command.clone();
        self.xtrace_fd.set(snapshot.xtrace_fd);
        *self.xtrace_fd_source.borrow_mut() = snapshot.xtrace_fd_source.clone();
        // rubash#1907: the substitution body ran against this same
        // PROCESS — re-apply the boundary's process umask (see the
        // InteriorSnapshot field docs).
        if let Some(mask) = snapshot.process_umask {
            crate::builtins::umask::apply_process_umask(mask);
        }
    }
}

impl Clone for ShellState {
    fn clone(&self) -> Self {
        Self {
            variables: self.variables.clone(),
            env_vars: self.env_vars.clone(),
            cwd: self.cwd.clone(),
            array_compact_budget: self.array_compact_budget.clone(),
            aliases: self.aliases.clone(),
            functions: self.functions.clone(),
            function_definition_redirects: self.function_definition_redirects.clone(),
            function_def_infos: self.function_def_infos.clone(),
            function_definition_locations: self.function_definition_locations.clone(),
            positional_params: self.positional_params.clone(),
            positional_offset: self.positional_offset,
            pipestatus: self.pipestatus.clone(),
            pipestatus_shape: self.pipestatus_shape,
            function_name_stack: self.function_name_stack.clone(),
            bash_argc_stack: self.bash_argc_stack.clone(),
            bash_argv_stack: self.bash_argv_stack.clone(),
            bash_arg_bottom: self.bash_arg_bottom.clone(),
            bash_lineno_stack: self.bash_lineno_stack.clone(),
            bash_source_stack: self.bash_source_stack.clone(),
            local_var_scopes: self.local_var_scopes.clone(),
            local_attr_scopes: self.local_attr_scopes.clone(),
            local_typed_scopes: self.local_typed_scopes.clone(),
            expanding_aliases: self.expanding_aliases.clone(),
            loop_depth: self.loop_depth,
            inherited_loop_depth: self.inherited_loop_depth,
            read_generation: self.read_generation,
            read_generation_ceiling: self.read_generation_ceiling,
            command_number: self.command_number,
            function_depth: self.function_depth,
            dollar_vars_changed_by_set: self.dollar_vars_changed_by_set,
            random_state: self.random_state.clone_state(),
            subshell_depth: Cell::new(self.subshell_depth.get()),
            subshell_source_text: self.subshell_source_text.clone(),
            xtrace_indirection_level: Cell::new(self.xtrace_indirection_level.get()),
            xtrace_suspended: Cell::new(self.xtrace_suspended.get()),
            in_command_substitution: Cell::new(self.in_command_substitution.get()),
            stdin_redir: Cell::new(self.stdin_redir.get()),
            job_table: self.job_table.clone(),
            last_background_pid: self.last_background_pid,
            coproc_names: self.coproc_names.clone(),
            completion_specs: self.completion_specs.clone(),
            // Process-global (see field docs): the shared handle, not a copy.
            bind_registry: self.bind_registry.clone(),
            session_history: self
                .session_history
                .as_ref()
                .map(|s| Rc::new(RefCell::new(s.borrow().clone()))),
            interactive_hist_engine: self.interactive_hist_engine.clone(),
            arithmetic_expansion_error: Cell::new(self.arithmetic_expansion_error.get()),
            arithmetic_nonfatal_error: Cell::new(self.arithmetic_nonfatal_error.get()),
            arithmetic_fatal_error: Cell::new(self.arithmetic_fatal_error.get()),
            arithmetic_nounset_error: Cell::new(self.arithmetic_nounset_error.get()),
            arithmetic_readonly_error: Cell::new(self.arithmetic_readonly_error.get()),
            conditional_nounset_error: Cell::new(self.conditional_nounset_error.get()),
            arithmetic_last_error_category: Cell::new(self.arithmetic_last_error_category.get()),
            parameter_bad_substitution: Cell::new(self.parameter_bad_substitution.get()),
            redirect_failglob_error: Cell::new(self.redirect_failglob_error.get()),
            redirect_failglob_seen: Cell::new(self.redirect_failglob_seen.get()),
            xtrace_fd: Cell::new(self.xtrace_fd.get()),
            xtrace_fd_source: RefCell::new(self.xtrace_fd_source.borrow().clone()),
            debug_trap_command: RefCell::new(self.debug_trap_command.borrow().clone()),
            // GNU fork shares the parent's open file descriptions, so a
            // process-substitution stream's read offset stays shared
            // across the clone boundary.
            procsub_streams: RefCell::new(self.procsub_streams.borrow().clone()),
            // rubash#1691: see the field docs — a clone drains only
            // its own ad-hoc carriers.
            procsub_adhoc_carriers: RefCell::new(Vec::new()),
            fork_saved_process_umask: self.fork_saved_process_umask,
        }
    }
}
