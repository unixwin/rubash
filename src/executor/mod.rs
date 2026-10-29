//! Executor Module - Bash Command Executor
//!
//! Executes parsed AST commands.

pub(crate) mod arithmetic;
pub(crate) mod glob;
pub(crate) mod path;
pub(crate) mod types;
pub(crate) mod wait_status;
use arithmetic::{
    arithmetic_division_by_zero_token, arithmetic_unbound_variable, eval_conditional_arith_value,
    eval_conditional_arith_value_categorized, eval_conditional_arith_value_categorized_with_writes,
    eval_conditional_arith_value_categorized_with_writes_and_state,
    eval_conditional_arith_value_with_writes,
};
pub(crate) use types::*;

mod arrays;
use arrays::*;
mod alias_arithmetic_for;
mod alias_case;
mod alias_loop_match;
mod alias_loops;
mod alias_reparse;
mod alias_select;
mod alias_set_builtins;
mod arithmetic_aliases;
mod array_assignment_exec;
mod assignment_dispatch;
mod assignment_expansion;
mod bind_builtins;
mod builtin_direct_command;
mod builtin_redirects;
mod command_dispatch;
mod command_dispatch_late;
mod command_dispatch_primary;
mod command_execute;
mod command_input_scope;
mod command_no_alias;
mod command_no_alias_late;
mod command_prepare;
mod command_substitution;
mod command_substitution_pipelines;
mod command_substitution_values;
mod command_words;
mod compound_exec;
mod copy_source;
mod history_exec;
mod inherited_stdin;
pub(crate) mod substitution_metadata;

/// Decode raw input bytes (script source read from stdin, command output,
/// embedded data) into rubash shell text. Invalid UTF-8 bytes are preserved
/// as raw-byte marker code points so lexer/executor paths round-trip them
/// byte-exactly instead of Latin-1 widening them with `byte as char`.
/// Embedders feeding bytes from `Read` sources should use this rather than
/// `String::from_utf8_lossy`, which would corrupt non-UTF-8 input.
pub fn bytes_to_shell_text(bytes: &[u8]) -> String {
    substitution_metadata::bytes_to_shell_text(bytes)
}

/// Unbuffered read of up to `count` bytes from the process's real stdin
/// handle (GNU input.c/zread semantics: no BufReader prefetch, so a child
/// sharing the descriptor continues at the exact kernel offset the shell
/// consumed). Returns an empty vec at EOF.
pub fn read_process_stdin_bytes(count: usize) -> std::io::Result<Vec<u8>> {
    crate::fd::read_some(crate::fd::process_std_handle(0), count)
}

use command_words::raw_word_has_unquoted_parameter_expansion;
use compound_exec::*;
mod declare_local;
mod dev_fd_operands;
mod dynamic_arrays;
mod exec_profile;
pub(in crate::executor) use dynamic_arrays::dense_view_element;
pub(in crate::executor) use dynamic_arrays::env_derived_dynamic_parameter_value;
pub(crate) mod embedded_mutations;
mod embedded_parameters;
mod expand_braced_indices;
mod expand_braced_ops;
mod expand_braced_patterns;
mod expand_braced_replacement;
mod expand_braced_special;
mod expand_word;
mod export_builtin;
mod external_file_builtins;
mod external_finish;
mod external_inner;
mod external_redirects;
mod external_setup;
pub(crate) mod fd_table;
mod function_calls;
mod function_locals;
mod getopts_enable;
pub mod identity;
mod init;

pub use init::RESPAWNED_CHILD;
pub mod profiler;
// rubash#331: the binary crate renders its self-produced path values
// (BASH, argv[0]-derived $0) through the same POSIX-form display the
// executor uses for $PWD.
pub use path::shell_pwd_display_path;
mod job_builtins;
mod limit_builtins;
mod lookup_paths;
mod loop_select;
mod mapfile_builtin;
mod mapfile_helpers;
pub(crate) mod markers;
mod option_builtins;
mod parameter_core;
mod parameter_errors;
mod parameter_patterns;
mod parameter_transforms;
mod parameter_words;
mod printf_path_builtins;
mod prompt_command;
mod prompt_expansion;
mod public_accessors;
mod pwd_loop_builtins;
mod read_builtin;
mod read_io;
mod read_redirected_fd;
mod readonly_functions;
mod shell_options;
#[cfg(windows)]
mod sudo_builtin;
// Windows-only stand-in (see the module header): unix builds never see it,
// so no cross-target dead-code surface.
#[cfg(windows)]
mod tput_emulation;

pub(crate) use shell_options::{write_stderr_bytes, write_stdout_bytes, GlobalStdout};

/// Writer that sends output through the Win32 `WriteFile` stdout path,
/// matching the executor's `write_default_stdout` sink. Used by builtins
/// that need the same immediate, unbuffered write semantics as the main
/// output path when invoked without redirection.
pub(crate) struct WriteFileStdout;
impl std::io::Write for WriteFileStdout {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        write_stdout_bytes(buf).map(|()| buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Writer that sends output through the Win32 `WriteFile` stderr path,
/// matching the executor's `write_default_stderr` sink. Used by builtins
/// that need the same immediate, unbuffered write semantics as the main
/// diagnostic path when invoked without redirection.
pub(crate) struct WriteFileStderr;
impl std::io::Write for WriteFileStderr {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        write_stderr_bytes(buf).map(|()| buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

mod shift_echo_builtins;
mod source_type_state;
mod state_views;
pub(crate) mod subscript_expansion;
pub(in crate::executor) use subscript_expansion::{
    is_plain_literal_subscript, IndexedSubscript, OperandSubscriptMode, SubscriptSource,
};
mod temporary_assignments;
pub(crate) mod timed_input;
mod trap_exec;
mod trap_stack_builtins;
mod type_builtin;
mod type_describe;
mod type_functions;
mod unset_arrays;
mod variable_state;
mod windows_drive;

mod alias_helpers;
pub(crate) mod assignment_helpers;
mod ast_exec;
mod builtin_names;
mod command_subst_helpers;
mod command_text;
mod env_helpers;
mod execution_misc;
mod function_env;
pub(crate) mod local_helpers;
mod parameter_case;
mod parameter_decode;
mod parameter_ops;
mod parameter_replace;
mod parse_helpers;
mod pipeline_exec;
mod pipeline_stages;
mod print_comsub;

mod read_helpers;
mod read_split;
mod redirect_inherit;
mod redirection;
mod select_exec;
mod support_names;

use crate::shell::state::ShellState;
use alias_helpers::*;
use assignment_helpers::*;
// Shared with builtins::declare for `declare -p` assoc rendering.
pub(crate) use assignment_helpers::{assoc_nbuckets, bash_assoc_order};
// Shared with builtins::declare for `declare -p` $'...' value rendering
// (strtrans.c ansic_shouldquote/ansic_quote over the raw byte stream).
pub(crate) use arrays::{ansic_quote, ansic_shouldquote};
// rubash#616 (1a): typed-cell writers shared with builtins::declare and
// builtins::printf, including the drop helper the declare sparse-cell fix
// needs to preserve the refuse-to-shadow-mirror rule.
pub(crate) use arrays::{
    drop_typed_indexed_array, replace_typed_indexed_array, sync_typed_indexed_storage,
};
use builtin_names::*;
use command_subst_helpers::*;
use command_text::*;
pub(crate) use embedded_mutations::COMPOUND_EXPANSION_WS_TAG;
use env_helpers::*;
pub(crate) use execution_misc::RandomGen;
use execution_misc::*;
use external_setup::{
    command_needs_process_substitution_materialization, ProcessSubstitutionFiles,
};
use fd_table::{FdReadEndpoint, FdTable, FdWriteEndpoint, FileFd, MaterializedRead};
use function_env::*;
use local_helpers::*;
use parameter_case::*;
use parameter_decode::*;
use parameter_ops::*;
use parameter_replace::*;
use parse_helpers::*;
use read_helpers::*;
pub(crate) use read_split::*;
use redirect_inherit::*;
use substitution_metadata::*;
use support_names::*;

pub(crate) mod conditional;
use conditional::{
    case_pattern_matches, case_pattern_matches_nocase, case_pattern_matches_nocase_with_word_chars,
    case_pattern_matches_with_word_chars, simple_grep_pattern_matches,
};

use crate::builtins::alias::Alias;
use crate::expand::tilde::tilde as tilde_expand;
use crate::lexer::TokenKind;
use crate::parser::{
    AndOrListCommand, ArithmeticCommand, ArithmeticExpressionMetadata, ArithmeticForCommand, Ast,
    BackgroundCommand, CaseClause, CaseCommand, CaseTerminator, CommandBodyKind, CommandNode,
    ConditionalCommand, ForCommand, FunctionBodyKind, FunctionCommand, IfCommand, InvertedCommand,
    LoopCommand, PipelineCommand, Redirect, SelectCommand, SubshellCommand, TimeCommand,
    WordMetadata,
};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;

pub struct HostExternalCommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: i32,
}

struct HostExternalCommandHandler(
    Box<dyn FnMut(&[String], &HashMap<String, String>) -> Option<HostExternalCommandOutput>>,
);

impl std::fmt::Debug for HostExternalCommandHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostExternalCommandHandler(..)")
    }
}

#[cfg(windows)]
pub use crate::builtins::sudo::SudoMode;

#[cfg(windows)]
#[derive(Debug, Clone)]
pub struct ElevationRequest {
    pub command: Vec<String>,
    pub resolved_program: Option<PathBuf>,
    pub environment: HashMap<String, String>,
    pub current_dir: PathBuf,
    pub preserve_environment: bool,
    pub mode: SudoMode,
}

#[cfg(windows)]
pub struct ElevationOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: i32,
}

#[cfg(windows)]
struct ElevationHandler(Box<dyn FnMut(ElevationRequest) -> Result<ElevationOutput, String>>);

#[cfg(windows)]
impl std::fmt::Debug for ElevationHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ElevationHandler(..)")
    }
}
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use self::path::{
    adopt_orphan_console, apply_required_windows_child_environment,
    external_command_for_named_program, find_shell, find_user_command,
    find_user_command_without_hash, shell_path_to_process, shell_path_to_windows, standard_path,
    suppress_orphan_console,
};

// version.c:90 show_shell_version(): the --version banner prints the same
// configure-time MACHTYPE triple the executor binds to $MACHTYPE; the bin
// crate reuses it (main.rs --version, builtins/help.rs help listing).
pub use self::support_names::bash_version_value;
pub use self::support_names::machtype_value;

// Identity persona (rubash#154): persona-aware OSTYPE/MACHTYPE/uname
// values behind the RUBASH_IDENTITY switch; main.rs --identity, the
// --help disclosure and the uname/arch builtins consume these via
// `crate::executor::identity`.

// NOTE: The executor's shared constants (env-var markers, fd-table key
// prefixes, etc.) live in `types.rs` and are re-exported via
// `pub(crate) use types::*;` above. Do not redeclare them here.

// G-01: the process-state lock. Once held for whole execute_ast
// runs; now taken only at process-global touch points through
// local_helpers::acquire_process_state (env mirror writes, trap
// disposition reconcile, birth snapshots, Drop restore, fresh-shell
// env swap).
static EXECUTION_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    // Run-nesting marker for execute_ast / call_function_owned: at
    // depth 0 the caller is a fresh reader-level run and the
    // evalerror latches reset. (Historically this tracked the
    // EXECUTION_LOCK hold; the lock no longer spans runs.)
    static EXECUTION_LOCK_DEPTH: Cell<usize> = const { Cell::new(0) };
}

enum NamerefResolution {
    Target(String),
    Circular,
    /// GNU variables.c:2022 find_variable_nameref: the chain exceeded
    /// NAMEREF_MAX (8) levels without resolving — reported as
    /// "maximum nameref depth (8) exceeded", not "circular".
    MaxDepth,
    /// GNU variables.c:2023-2026 find_variable_nameref: a marked nameref
    /// whose cell is missing, empty, or not a resolvable name resolves to
    /// nothing — the variable reads as unset for value lookups, while
    /// name-resolution contexts (assignment, unset) keep the variable
    /// itself (find_variable_nameref_for_assignment returns the nameref).
    Unresolved,
    NotNameref,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeDescribeMode {
    Verbose,
    Reusable,
    TypeOnly,
    PathOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopControlKind {
    Break,
    Continue,
}

pub(crate) type FunctionBody = Rc<Ast>;

/// Print/roundtrip metadata for a defined function that the body AST alone
/// does not carry. GNU keeps the whole FUNCTION_DEF command for
/// `declare -f`/`type NAME` rendering (print_cmd.c named_function_string /
/// print_function_def): the body kind decides whether the stored body is
/// wrapped in `( )` (cm_subshell body), and redirections attached to the
/// function definition itself (`f () { ... } 1>&2`) print after the closing
/// brace and travel in the exportstr so subshell children re-import them.
#[derive(Clone, Debug, Default)]
pub(crate) struct FunctionDefInfo {
    pub body_kind: Option<crate::parser::FunctionBodyKind>,
    pub def_redirects: Vec<crate::parser::Redirect>,
}

#[derive(Clone, Debug)]
pub(crate) struct FunctionDefinitionLocation {
    line: usize,
    source: String,
    /// GNU reports the body group's line for DEBUG fires inside a traced
    /// function (trap.tests `func2[43] debug`: `func2()` on 42, `{` on 43),
    /// so the plain-call path needs this in addition to the definition line.
    body_open_line: Option<usize>,
}

impl LoopControlKind {
    fn name(self) -> &'static str {
        match self {
            LoopControlKind::Break => "break",
            LoopControlKind::Continue => "continue",
        }
    }
}

/// Execution error
#[derive(Debug)]
pub enum ExecuteError {
    CommandNotFound(String),
    /// A direct host-side function dispatch requested a function that is not
    /// defined in the executor.
    FunctionNotFound(String),
    IoError(std::io::Error),
    ExitCode(i32),
    /// A word-expansion failure that aborts only the current command
    /// list: function frames absorb it as an early return carrying this
    /// status, `( )` frames end just the subshell, and a top-level list
    /// ends the noninteractive run (GNU probes f3/f4, 2026-08-24).
    ExpansionFailure(i32),
    /// A fatal function-definition error (GNU execute_cmd.c
    /// execute_intern_function with posixly_correct: last_command_exit_value
    /// = EX_BADUSAGE and jump_to_top_level(ERREXIT)). Under POSIX mode an
    /// invalid function name aborts the current subshell (parent continues)
    /// or, at script top level, ends the noninteractive run.
    FatalFunctionError(i32),
    Break(usize),
    Continue(usize),
    Return(i32),
    UnknownBuiltin(String),
    /// `exit N` called from a lastpipe stage (shopt -s lastpipe) runs in
    /// the current shell, so it must exit the current shell — not just set
    /// the pipeline's exit status.  This variant propagates through
    /// `execute_simple_pipeline` / `execute_pipeline_command` without being
    /// caught by the pipeline-status match in `execute_ast_inner`, and the
    /// top-level caller converts it back to `ExitCode(N)`.  GNU reference:
    /// execute_cmd.c:2758 `execute_command_internal(cmd, ...)` runs the
    /// lastpipe stage in the current shell; `exit` longjmps to top level.
    LastpipeExit(i32),
    /// niubash#345: a shell-side pipeline producer wrote to the next stage's
    /// OS pipe after the downstream consumer closed its read end. This is the
    /// in-process analogue of SIGPIPE (execute_cmd.c:2695/redir.c): the
    /// producing stage terminates with the synthesized status 141. The stage
    /// helpers (`execute_compound_pipeline_stage`,
    /// `execute_builtin_pipeline_stage`, `execute_function_pipeline_stage`)
    /// convert it to that status; it never escapes a pipeline element.
    BrokenPipe,
    /// rubash#1639: an untrapped SIGINT killed the foreground job of an
    /// interactive reader session — GNU jobs.c waitchld's
    /// job-control/foreground/SIGINT block does ADDINTERRUPT and the
    /// executing list throws to the top-level reader
    /// (throw_to_top_level), discarding the rest of the current
    /// command line in every frame with $? recorded as 130. (For the
    /// in-process virtual child — the builtin sleep — the killing
    /// delivery is the shell's own kernel-caused SIGINT receipt.)
    /// The wait
    /// sites only MARK the event (Executor::fg_sigint_unwind); the list
    /// executor converts it to this error at the next command boundary
    /// so frames cannot absorb it as a plain 130 status. Frames must
    /// propagate it untouched (like LastpipeExit); drivers treat it as
    /// the end of the current input unit with exit_code already 130.
    SigintUnwind,
}

impl std::fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecuteError::CommandNotFound(cmd) => write!(f, "rubash: {}: command not found", cmd),
            ExecuteError::FunctionNotFound(name) => {
                write!(f, "rubash: {}: function not found", name)
            }
            ExecuteError::IoError(e) => write!(f, "rubash: {}", crate::posix_errors::message(e)),
            ExecuteError::ExitCode(code) => write!(f, "exit code: {}", code),
            ExecuteError::ExpansionFailure(code) => write!(f, "exit code: {}", code),
            ExecuteError::FatalFunctionError(code) => write!(f, "exit code: {}", code),
            ExecuteError::Break(level) => write!(f, "break {}", level),
            ExecuteError::Continue(level) => write!(f, "continue {}", level),
            ExecuteError::Return(status) => write!(f, "return {}", status),
            ExecuteError::UnknownBuiltin(name) => {
                write!(f, "rubash: {}: builtin command not found", name)
            }
            ExecuteError::LastpipeExit(code) => write!(f, "exit code: {}", code),
            ExecuteError::BrokenPipe => write!(f, "broken pipe"),
            ExecuteError::SigintUnwind => write!(f, "interrupt: discarding current command line"),
        }
    }
}

impl std::error::Error for ExecuteError {}

impl From<std::io::Error> for ExecuteError {
    fn from(e: std::io::Error) -> Self {
        ExecuteError::IoError(e)
    }
}

/// The ten declare-family SHELL_VAR attributes (GNU variables.h:124-133
/// att_* port). Defined in `shell::var_table` beside the structured
/// storage; re-exported here for the executor call sites that captured it
/// under `use super::*`.
pub(crate) use crate::shell::var_table::VarAttrs;

#[derive(Debug)]
struct SavedGlobalDeclareLocal {
    name: String,
    scope_index: usize,
    local_value: Option<String>,
    local_attrs: VarAttrs,
    local_typed: Option<crate::shell::Variable>,
}

/// Command executor
#[derive(Debug)]
pub struct Executor {
    /// All mutable shell semantics live here — the fork-copy boundary
    /// (execute_cmd.c:1576 execute_in_subshell / subst.c:7143
    /// command_substitute). Flat subshells and command substitutions clone
    /// it wholesale; fields below are process resources, host wiring, or
    /// per-command/expansion transients that must never be cloned.
    pub(crate) shell_state: ShellState,
    fd_table: FdTable,
    exit_code: i32,
    parse_error_occurred: bool,
    /// rubash#1662: set by the boundary signal dispatch when it aborts a
    /// unit for an untrapped signal in an interactive shell. GNU
    /// classifies that unwind as DISCARD (the reader prompts again with
    /// $? = 128+sig), not as a jump_to_top_level exit; run_source_impl
    /// consumes the mark instead of recording an exit jump.
    signal_discard_pending: bool,
    /// GNU exit.def:52 (sourced_logout): ~/.bash_logout runs at most once
    /// per shell process (bash_logout, exit.def:156-166).
    bash_logout_sourced: bool,
    shell_pid: u32,
    /// rubash#1095: the subshell level this process's own pid answers for
    /// $BASHPID. GNU variables.c:1511 get_bashpid is getpid() — every fork
    /// level is a fresh pid. Rubash shares one pid across in-process
    /// subshells and fakes distinctness with pid + depth, but a spawned
    /// async wrapper (compound_exec.rs) already IS GNU's single
    /// async/subshell fork (execute_cmd.c:701): its pid is the level the
    /// spawn realized, so BASHPID = pid + depth.saturating_sub(origin)
    /// with origin = 0 for ordinary shells.
    bashpid_origin_depth: usize,
    owns_signal_mailbox: bool,
    /// startup21: true when this Executor is the process's outermost one
    /// AND the process exits immediately after it drops (main.rs sets it
    /// for the run_main executor). The Drop-time process-environment
    /// restore (env diff plus one putenv per startup variable, ~0.45ms)
    /// models "an in-process child leaves its parent's process env
    /// untouched" (rubash#182, see the command_substitution.rs child
    /// executor) — at real process exit nothing can observe the process
    /// environment anymore, so the restore is skipped. Embedded/child
    /// executors keep the restore.
    is_process_exit_executor: bool,
    background_children: HashMap<u32, crate::fd::SpawnedChild>,
    /// rubash#1952: pids spawned as streamed input process
    /// substitutions (real children in background_children), mapped
    /// to whether the boundary reap has spent its one bounded grace
    /// wait for the pid (see refresh_background_jobs_impl).
    procsub_stream_children: HashMap<u32, bool>,
    /// Background-job completions the boundary reap has already
    /// observed (try_wait returned) but whose job-table visibility is
    /// deferred to the next GNU observation point — a foreground
    /// process wait, a new reader line, a new execute_ast run, or an
    /// explicit wait/fg (jobs.c waitchld lag; see
    /// refresh_background_jobs in job_builtins.rs). Entries are
    /// (pid, flattened status, killing signal).
    pending_job_completions: Vec<(u32, i32, Option<i32>)>,
    /// rubash#1528: deferred `time` reports for timed coproc commands.
    /// GNU times a coproc like any async job — the interval is the
    /// job's lifetime (spawn to completion) and the report prints when
    /// the completion is noticed (at `wait` or a later command
    /// boundary), not when the spawn returns. Entries are
    /// (pid, start stamp, posix_format), flushed by
    /// flush_timed_coproc_report when the job's completion is applied.
    pending_timed_coproc_reports: Vec<(u32, read_helpers::TimeCommandStart, bool)>,
    /// rubash#1528: the reap instant of a timed coproc, recorded when
    /// the boundary sweep's try_wait first observes the death (GNU
    /// waitchld stamps the job timeval at reap, not at notice). Keyed
    /// by pid, consumed by flush_timed_coproc_report.
    timed_coproc_death_instants: std::collections::HashMap<u32, std::time::Instant>,
    /// The (execute_generation, reader_command_line,
    /// foreground_wait_count) mark as of the previous
    /// refresh_background_jobs call; an observation point has occurred
    /// when the current mark differs in any component.
    reap_prev_mark: Cell<(u64, Option<usize>, u64)>,
    /// Bumped once per fresh reader-level execute_ast run.
    execute_generation: Cell<u64>,
    /// Count of foreground external processes this executor has
    /// blocked in wait() for (GNU wait_for reaps background children
    /// while it blocks, jobs.c:3064 — an observation point).
    foreground_wait_count: Cell<u64>,
    coproc_stderr_forwarders: HashMap<u32, std::thread::JoinHandle<Result<(), std::io::Error>>>,
    /// rubash#1632: pipe writers displaced from a coproc child's stdout
    /// by the coproc command's own `>file`/`>>file` redirection, held for
    /// the coproc's lifetime keyed by child pid. GNU applies such a
    /// redirect in the forked child, where the displaced pipe end
    /// survives in a redirect save slot — so the parent's read end sees
    /// no EOF while the coproc lives. On Unix the writer rides into the
    /// child itself (pre_exec dup to a free high fd) and this map stays
    /// empty; where no fd can be injected, the parent holds the writer
    /// and drops it when the coproc retires.
    coproc_displaced_stdout_writers: HashMap<u32, std::io::PipeWriter>,
    /// Assignment-RHS output process substitutions awaiting their
    /// finish, in materialization order (GNU forks left-to-right
    /// and the bodies' observable effects order the same way,
    /// rubash#1523). Keyed by the `/dev/fd/N` word form.
    assignment_output_process_substitutions: Vec<(String, String)>,
    /// rubash#1691: backing temp files of assignment-RHS INPUT process
    /// substitutions, in materialization order. The fd word's payload
    /// lives in the stream registry, but the on-disk carrier is only
    /// owned by this list — it is drained (files removed) at the same
    /// per-command finish as the output queue above, which is the
    /// lifetime GNU gives the substitution's fd (dead by the next
    /// command). Before this list existed the paths were discarded at
    /// the write site and the files leaked in $TMPDIR for the shell's
    /// whole lifetime.
    assignment_input_process_substitution_files: Vec<PathBuf>,
    /// rubash#1692: while a simple command's tempenv is being applied
    /// and its class (builtin/function vs disk command) is not yet
    /// known, locale-family binds are recorded in
    /// `deferred_locale_assignment_binds` instead of running GNU's
    /// setlocale attempt: GNU binds tempenv only for builtin/function
    /// commands (assign_in_env makes a child-env entry for disk
    /// commands, never a shell binding, so a bad tempenv locale warns
    /// only when the binding is restored afterwards). The dispatch
    /// flushes the deferred binds when the command turns out to bind,
    /// and drops them when it does not.
    defer_locale_assignment_warnings: Cell<bool>,
    deferred_locale_assignment_binds: Vec<String>,
    /// rubash#1525: set by execute_external so the simple-command
    /// funnel can tell which completion-write class a command fell
    /// into (GNU execute_cmd.c: the tempenv of a disk command is
    /// consumed into the child's environment, so the parent's
    /// set_pipestatus_from_exit write lands in the real cell;
    /// builtin/function/null commands write against the still-bound
    /// tempenv cell). Saved/restored around each dispatch so nested
    /// commands cannot leak their class into the enclosing one.
    external_dispatch_ran: Cell<bool>,
    /// rubash#1525: set by the simple-command funnel when it has
    /// already performed this command's PIPESTATUS completion write
    /// against the still-bound tempenv cell; the ast_exec loop's
    /// post-command write consumes (and clears) it instead of
    /// writing a second time against the restored cell.
    pipestatus_write_absorbed: Cell<bool>,
    /// rubash#1525: the (shape, vector) snapshot apply_temporary_
    /// assignments recorded for the innermost PIPESTATUS tempenv
    /// binding, mirrored for promote_tempenv_locals: a promoted
    /// tempenv PIPESTATUS becomes a frame local whose frame-restore
    /// snapshot must hold the pre-tempenv shape, not the tempenv
    /// shape. Cleared by the restore that consumes the snapshot.
    tempenv_pipestatus_state: Option<String>,
    /// rubash#545 (cell 2): `/dev/fd/N` input-procsub word fds created by
    /// the command CURRENTLY executing. GNU closes a process substitution's
    /// fd (and reaps its process) when the creating command finishes
    /// (subst.c), so an array element or variable carrying the `/dev/fd/N`
    /// STRING holds a dead path afterwards. execute_materialized_command
    /// snapshots this vec's length at entry and closes the delta at the
    /// command's end.
    command_scoped_procsub_input_fds: Vec<u32>,
    /// rubash#545 (cell 3): `>(...)` operands realized for a `[[ ]]`
    /// command. The body runs at the command's end, reading the temp
    /// carrier (the same finish the simple-command path gives output
    /// process substitutions).
    conditional_output_procsubs: Vec<(std::path::PathBuf, String, Option<usize>)>,
    pending_scalar_assignment: bool,
    suppress_errexit: usize,
    /// rubash#1563: depth of GNU's CMD_IGNORE_RETURN clearing frame for
    /// eval/source (execute_cmd.c:5004-5016): when the builtin is
    /// dispatched under CMD_IGNORE_RETURN outside a subshell environment,
    /// execute_builtin unwind-protects exit_immediately_on_error, sets it
    /// to 0 for the body's duration and raises builtin_ignoring_errexit
    /// (which flags.c:262 consults, so a `set -e` inside the body updates
    /// the option without re-arming the print-time exit). While this is
    /// nonzero, exit_immediately_on_error() reads false even with the
    /// errexit option on — the eval/source bodies in `eval '...' || true`
    /// therefore survive a DISCARD-class diagnostic GNU only prints.
    pub(crate) errexit_ignore_frame: usize,
    /// rubash#1563: the stack of live parse_and_execute frames (eval
    /// strings and sourced files), each entry the subshell_depth at
    /// which the frame was entered. GNU's parse_and_execute DISCARD
    /// catch (builtins/evalstring.c:423-450) folds the pre-set
    /// EX_BADUSAGE 2 to EXECUTION_FAILURE; the catch that receives a
    /// DISCARD is the innermost one whose environment is still current,
    /// and a `( )` region entered since the frame installs
    /// execute_in_subshell's preserving catch instead
    /// (execute_cmd.c:1821-1825) — the entry-depth snapshot models that
    /// barrier: inside such a region the current depth differs from the
    /// entry and the fold must not fire.
    pub(crate) parse_and_execute_frames: Vec<usize>,
    /// GNU execute_cmd.c:273 `executing_command_builtin`: set while a
    /// command runs as the argument of the `command` builtin
    /// (execute_cmd.c:4732-4736 wraps the CMD_COMMAND_BUILTIN re-dispatch,
    /// execute_cmd.c:5096 the `command` builtin itself). Read by the
    /// eval/source parse-error containment: evalstring.c:590 suppresses
    /// the posix ERREXIT for `command eval '...broken...'` — errors8.sub
    /// `command eval '( '` must not exit the shell.
    command_builtin_depth: usize,
    /// GNU builtins/builtin.def: the wrapped command runs under the
    /// `builtin` builtin's dispatch. A redirect-only `exec` reached
    /// through it is a one-shot redirection for that command only, not
    /// the persistent form plain `exec` performs (rubash#1412).
    /// execute_builtin_direct_command raises this around its `exec` and
    /// `command` arms; execute_exec snapshots and restores the fd state
    /// for its no-operand paths while it is non-zero.
    builtin_wrapper_depth: usize,
    /// GNU `command -p` (builtins/command.def CDESC_STDPATH): the
    /// standard path for the re-dispatched command's OWN lookup only.
    /// Set for the duration of the re-dispatch and consulted by the
    /// simple-command execution lookup; the shell's $PATH variable —
    /// and therefore child environments and nested builtins' PATH
    /// reads — is never mutated (rubash#1047).
    standard_path_scope: Option<String>,
    debug_trap_running: bool,
    /// rubash#375: parse-once cache for the DEBUG trap action. GNU parses a
    /// trap action once at `trap` invocation (trap.c read_trap_action) and
    /// runs the cached parse per firing; re-tokenizing + re-parsing the same
    /// action text on every firing (measured 12.5µs x 6002 firings in the
    /// bats_debug_trap lane) is dead work. Key = the alias-spliced action
    /// text (the exact bytes tokenize sees), value = the parsed AST. During
    /// a firing the AST is TAKEN out of the cell (the per-firing
    /// `command.line` stamp mutates it), executed, and put back — a stable
    /// action text pays zero clones and zero parses per firing. Any change
    /// to the spliced text (trap re-set, alias-table change feeding the
    /// splice) misses and re-parses.
    debug_trap_action_cache: std::cell::RefCell<Option<(String, crate::parser::Ast, i64)>>,
    /// WP-3 (1.5.0 debugger design 4.4): the attached debug
    /// session's driver, consulted through the DEBUG-trap funnel
    /// (`run_debug_trap`). `None` on every executor but the
    /// session's owner — child executors (command substitution,
    /// pipeline stages) never inherit it, so no stop can fire
    /// inside a synthesized frame (DE-12's frame model is a later
    /// work package).
    debug_driver: Option<crate::debugger::SessionDriver>,
    /// WP-8 (debugger design 5.1): the file identity the next
    /// `run_source_impl` unit belongs to, when that unit is backed
    /// by a real file. Set by the two file-backed group drivers
    /// (the grouped script reader and the `source` builtin's
    /// incomplete-group arm) immediately before they hand a unit
    /// over; `run_source_impl` takes it at entry and registers the
    /// parsed unit's syntax view with the attached session under
    /// exactly this identity — the same string the engine puts on
    /// the BASH_SOURCE stack for the unit. Synthetic units (eval
    /// strings, -c text, trap actions) never set it, so they never
    /// register a view.
    pub(crate) debug_syntax_unit_file: Option<String>,
    return_trap_running: bool,
    signal_trap_running: bool,
    /// The last value written to the OS environment's
    /// __RUBASH_CURRENT_LINE mirror (None = never written this process).
    /// The stamp path skips the Win32 set only when this equals the newly
    /// stamped value; direct map writers (ambient-line restores) never
    /// touch the OS env, so the comparison must be against what WE wrote,
    /// not against the map value (rubash perf12: the map-value gate
    /// produced stale alias/declare line numbers after an ambient restore).
    line_env_os_value: std::cell::RefCell<Option<String>>,
    /// The background direct-spawn eligibility gate (#809) expands the async
    /// command's words in the PARENT as a probe; GNU expands the forked
    /// child's words inside the child, so the parent never observes a
    /// failglob verdict. While this flag is set, `report_failglob` stays
    /// silent — the wrapper fallback re-expands in the child and reports
    /// exactly once, and the gate restores the expansion latches so the
    /// parent-side discard never swallows the rest of the launch line
    /// (`set -- /no/such/*.zzz & echo BG_after` lost its echo, 753.11).
    expansion_probe_active: std::cell::Cell<bool>,
    /// GNU trap.c SIG_INPROGRESS: _run_trap_internal marks a trap
    /// in-progress before executing its action, and run_error_trap refuses
    /// to re-enter while it is set — an ERR trap action that itself fails
    /// must not recursively re-trigger the ERR trap (niubash #121).
    error_trap_running: bool,
    /// GNU execute_cmd.c:1799-1806 (execute_in_subshell): a user-subshell
    /// child whose whole body is a single simple or subshell command marks
    /// that body CMD_NO_FORK; execute_cmd.c:662-663 then dispatches a
    /// subshell body IN PLACE — no make_child, no wait_for, and therefore
    /// no :769 run_error_trap in that frame. The forkless engine runs every
    /// `( )` body in place already, so the frame boundary that GNU erased
    /// is exactly the enclosing subshell whose body is one subshell
    /// command: while this flag is set, a failing subshell command's
    /// aggregate ERR adjudication is suppressed (rubash#720 form 1,
    /// `( (false) )` fires 2 not 3).
    subshell_fork_collapse: std::cell::Cell<bool>,
    /// GNU trap.c:124 trap_saved_exit_value — the shell status snapshot
    /// _run_trap_internal (trap.c:367) takes before running a trap action.
    /// get_exitstat (builtins/common.c:519, POSIX interp 1602) resolves a
    /// bare `exit` inside a trap action to this value when the exit runs at
    /// the trap's own function/source nesting level (the level recorded at
    /// fire time, trap.c:383) and the immediate trap is not DEBUG —
    /// commands inside the action cannot change the bare-exit status.
    /// None outside trap actions. rubash#524.
    trap_saved_exit_value: Option<i32>,
    /// GNU trap.c:383 trap_return_context — the function + source nesting
    /// depth at the moment the current trap action was fired.
    trap_saved_exit_context: usize,
    /// GNU builtins/exec.def exec_builtin + execute_cmd.c:6126 shell_execve:
    /// a successful `exec command` replaces the shell image and never takes
    /// the exit path, so the EXIT trap does not run (shell.c:985 exit_shell
    /// -> run_exit_trap is bypassed entirely). trap.c:1440
    /// reset_or_restore_signal_handlers clears SIG_TRAPPED for the exit trap
    /// (keeping the string) before shell_execve, so a post-reset execve
    /// failure also exits without the trap — only the pre-reset search
    /// failure (command == 0 -> sh_notfound -> failed_exec) keeps it.
    /// Rubash cannot execve, so it spawns the replacement and unwinds with
    /// its status; this latch tells the exit-trap choke point to skip the
    /// action on that unwind (rubash#1012).
    exec_replaced: bool,
    /// rubash#1461: an in-process subshell layer was killed by a signal
    /// (a `kill` to that layer's fake BASHPID, intercepted in
    /// execute_kill): (subshell_depth of the dead layer, signal). The
    /// subshell boundary that owns the depth consumes it — it skips the
    /// layer's EXIT trap (a signal-killed GNU process runs none) and
    /// queues the parent-side death notice.
    subshell_signal_death: Option<(usize, i32)>,
    /// rubash#1639: a foreground wait adjudicated an untrapped SIGINT
    /// job death for an interactive reader session (the wait sites in
    /// trap_exec/external_inner set this; exit_code already holds
    /// 130). The list executor converts it to
    /// ExecuteError::SigintUnwind at the next command boundary,
    /// discarding the rest of the current command line in every frame
    /// (GNU wait_for's throw_to_top_level). Transient per-command
    /// state: set only by a wait, consumed by the boundary that
    /// immediately follows it.
    fg_sigint_unwind: bool,
    /// rubash#1175: true only on the executor that IS this OS process's
    /// shell (constructed by Executor::new for a real process root).
    /// Fork-copy clones (command_substitution_executor) construct with
    /// false: they share the process with the shell that must survive
    /// them, so their `exec` keeps the spawn-and-wait simulation. Even a
    /// rooted executor only replaces the image when the host binary
    /// opted in (builtins::exec::enable_process_replacement) and the
    /// live context passes the dispatcher's gates.
    owns_process_image: bool,
    /// GNU execute_cmd.c:330 sourcenest — active `.`/source frames.
    source_nest_depth: usize,
    /// Child-death notifications that arrived while the SIGCHLD trap action
    /// was already running (a re-entrant reap drops them otherwise; bash
    /// re-runs the trap once per pending notification — trap.tests expects
    /// three "caught a child death" lines for three reaped background jobs).
    sigchld_notifications_pending: std::cell::Cell<usize>,
    /// Children killed by a signal, awaiting the reader-boundary stderr
    /// notice (rubash#229). GNU jobs.c:4625-4652 notify_of_job_status runs
    /// from eval.c:355 notify_and_cleanup after each top-level command:
    /// non-INT/TERM/PIPE untrapped signal deaths print
    /// `name: line N: %5ld <desc padded to 27> <cmd>` (JLIST_NONINTERACTIVE),
    /// TERM (config-top.h DONT_REPORT_SIGTERM) and trapped-signal deaths of
    /// FOREGROUND jobs print the bare `<desc padded to 27><cmd>` shape
    /// (print_pipeline JLIST_STANDARD), everything else is silent.
    /// (pid, signal, command text, foreground).
    pending_signal_notices: std::cell::RefCell<Vec<(u32, i32, String, bool)>>,
    /// Depth of `with_traps_suspended` scopes currently active. GNU has no
    /// notion of host housekeeping commands: when an embedding host runs
    /// internal commands through execute_ast (startup/shutdown hooks,
    /// rc cleanup, plugin probes), they are not script commands and must
    /// not fire the user's DEBUG/RETURN/ERR traps — the script's own
    /// commands already reached EOF (engine sinking list S2).
    pub(crate) host_internal_depth: std::cell::Cell<usize>,
    debug_trap_function_line: Option<usize>,
    /// Expanded form of the offset/length expression from the last failed
    /// substring arithmetic evaluation. `report_substring_arithmetic_error`
    /// uses this so the error token reflects the post-expansion text (GNU
    /// evalexp runs after parameter/command substitution).
    arithmetic_last_error_expression: std::cell::RefCell<String>,
    /// Expanded+normalized expression text from the most recent
    /// `(( ... ))` evaluation — the exact string `evalexp` saw. GNU
    /// execute_arith_command (execute_cmd.c:3937) runs
    /// expand_arith_string before evalexp, so diagnostics echo the
    /// post-expansion text (`$A` shows its value, not the literal).
    arithmetic_last_eval_input: std::cell::RefCell<String>,
    /// GNU error.c builtin_error -> error_prolog: diagnostics raised while a
    /// builtin runs carry `this_command_name` (`declare: ...`, `getopts: ...`).
    /// Assignment paths deep inside apply_shell_assignment consult this so a
    /// failed `builtin name=value` names the builtin, while a bare
    /// `name=value` command prints no command segment.
    assignment_command_name: Option<String>,
    /// GNU builtins emit assignment diagnostics through builtin_error, which
    /// honors the builtin's redirected stderr and ordering. Builtin callers
    /// (getopts OPTARG/name binds) set buffer_assignment_diagnostics and drain
    /// pending_assignment_diagnostics into their buffered stderr so a
    /// preceding buffered diagnostic (e.g. `illegal option -- h`) keeps its
    /// position relative to the bind error.
    buffer_assignment_diagnostics: bool,
    pending_assignment_diagnostics: Vec<u8>,
    /// GNU getopts.def dogetopts: binding the option character into an
    /// integer-attributed OPTIND (name == "OPTIND") whose value is not
    /// arithmetic (`?`/`:`) raises an evalexp error that abandons the
    /// rest of the call — the trailing OPTARG bind/unbind never runs —
    /// while the script continues with status 1 (rubash#1270). The name
    /// bind latches the fatality here; execute_getopts reads and clears
    /// it at the affected call sites.
    getopts_optind_arith_fatal: bool,
    /// GNU parse.y builds `time` [`!`] pipeline — the timing and the
    /// inversion bind to the WHOLE pipeline (execute_cmd.c
    /// execute_pipeline under CMD_TIMED). Rubash's compound parse parks
    /// the TimeCommand on pipeline element 0 instead; while the pipeline
    /// executor runs that element it raises this flag so the stage
    /// executor defers both effects to pipeline scope (rubash#1263).
    defer_stage_time_report: bool,
    /// A deferred time report's (start, posix_format), stashed by the
    /// stage executor for the pipeline executor (rubash#1263).
    pending_stage_time_report: Option<(read_helpers::TimeCommandStart, bool)>,
    /// rubash#1277: while the list loop executes a `time` command it
    /// raises this flag so execute_time_ast_command queues its report
    /// instead of printing it. GNU's ERR trap for the timed command fires
    /// inside the inner execute_command (execute_cmd.c
    /// execute_time_command -> execute_command -> run_error_trap),
    /// BEFORE execute_time_command prints the timing block; the list
    /// loop flushes the queue after its own maybe_run_error_trap and
    /// before the errexit jump, reproducing that order.
    defer_list_time_report: bool,
    /// Queued (start, posix_format) reports under
    /// defer_list_time_report, in completion order (innermost first).
    pending_list_time_reports: Vec<(read_helpers::TimeCommandStart, bool)>,
    /// GNU subst.c: an assignment performed by `${var:=word}`/`${var=word}`
    /// that fails (invalid nameref target value, readonly, ...) is a word
    /// expansion error — expand_word_error -> exp_jump_to_top_level(DISCARD)
    /// abandons the rest of the command list. The := applier runs inside
    /// word expansion where it cannot return an error, so it latches here
    /// and execute_prepared_command raises ExpansionFailure(1).
    parameter_assignment_failure: Cell<bool>,
    /// rubash#1542: `$?` as it stood when the latched
    /// `parameter_assignment_failure` bind was attempted — GNU prints the
    /// readonly diagnostic from inside bind_variable (variables.c
    /// err_readonly -> error.c report_error), BEFORE subst.c
    /// parameter_brace_expand_rhs writes EX_BADUSAGE into
    /// last_command_exit_value, and report_error's errexit branch
    /// exit_shells with that earlier value (0 remapped to
    /// EXECUTION_FAILURE). Captured by the := applier before the bind
    /// attempts, consumed beside `parameter_assignment_failure`.
    parameter_assignment_failure_prev_status: Cell<Option<i32>>,
    /// rubash#1109: an extract_dollar_brace_string failure (subst.c:1954
    /// span rules) latched by the expansion walkers — DISCARD semantics
    /// in every mode, consumed beside the bad-substitution checks.
    no_closing_brace_failure: Cell<bool>,
    /// GNU subst.c:10272-10288 (parameter_brace_expand): a `${...}` whose
    /// parameter name ends on a character that starts no operator — a quote
    /// at name position (`${'x'%'t'}`, `${x'y'}`) — hits the `bad
    /// substitution` default. The error surfaces mid-expansion (nested
    /// `${}` inside a pattern/alternate word reaches it only when that word
    /// is actually evaluated), so the expander latches it here and the
    /// command boundary raises DISCARD — or FORCE_EOF for a noninteractive
    /// POSIX shell (subst.c:10288). Stored in ShellState so the command-
    /// substitution boundary isolates it via the typed clone.
    /// GNU variables.c:3536 assign_in_env: names bound through `name=value
    /// cmd` temporary-environment assignments are live at the command's
    /// variable context while it runs — a function-local `declare -n r`
    /// sees a tempenv `r` as an existing variable (validating its value as
    /// the new cell), not as a fresh empty local. Tracked as a stack of the
    /// bound names so nested commands restore correctly.
    tempenv_names: Vec<String>,
    /// Frame marks pairing apply_temporary_assignments /
    /// restore_temporary_assignments so nested commands truncate the
    /// tempenv-name stack to the boundary recorded by the outer call.
    tempenv_marks: Vec<usize>,
    /// GNU variables.c:2604-2620 make_local_variable (was_tmpvar): a
    /// `declare`/`typeset`/`local` operand whose name is bound by this
    /// command's own `name=value` prefix promotes the tempenv binding to a
    /// frame local in place — `z=y typeset z` leaves a live exported local
    /// z=y for the rest of the frame instead of an empty local that pops
    /// with the command. Names listed here skip the command-end tempenv
    /// restore in restore_temporary_assignments.
    tempenv_promoted_names: Vec<String>,
    /// Pre-tempenv snapshots (env value, typed cell, attribute set) for the
    /// names bound by the innermost apply_temporary_assignments mark. The
    /// declare/local promotion path consults it so a promoted local's
    /// saved-previous is the value before the prefix applied, not the
    /// tempenv value itself (which would otherwise leak back into the
    /// global scope at frame restore).
    tempenv_previous: HashMap<String, (Option<String>, Option<crate::shell::Variable>, VarAttrs)>,
    /// GNU variables.c:4485-4525 (push_posix_temp_var): a posix special
    /// builtin's tempenv merged into a function context gets att_propagate
    /// when the bound cell is not a local, so the binding descends into the
    /// caller's context when the frame pops instead of dying with it. Names
    /// listed here skip function-call tempenv restores (like promoted
    /// names) until no live function tempenv covers them any more.
    tempenv_propagated_names: Vec<(String, VarAttrs)>,
    /// Base names bound by each active function call's own tempenv prefix —
    /// the function's variable context in GNU terms (variables.c
    /// push_context). Lets the posix merge path distinguish a function
    /// tempenv binding (propagates on frame pop) from a plain command
    /// tempenv binding (dies with its command scope).
    function_tempenv_names: Vec<Vec<String>>,
    /// GNU expr.c evalerror -> jump_to_top_level (DISCARD): an arithmetic
    /// evaluation failure — including a failed array-subscript evaluation —
    /// discards the rest of the command list that contained the failing
    /// command (`a[$x]=v; echo after` never prints `after`). Rubash models
    /// the discarded tail as the commands sharing the failing reader-level
    /// command's source line (`CommandNode::line`).
    pub(crate) evalerror_pending: Cell<bool>,
    /// rubash#639: a nounset (FORCE_EOF) subscript trap rides the same
    /// `evalerror_pending` abandon vehicle as DISCARD failures, but GNU
    /// exits 127 at the `-c` top level for FORCE_EOF (shell.c:1471) versus
    /// 1 for DISCARD (shell.c:1477). This marker carries the class from the
    /// trap site to the abandon point, which consumes it; every boundary
    /// that clears `evalerror_pending` clears this alongside, so a caught
    /// abort never poisons a later unrelated abandon with 127.
    pub(crate) nounset_force_eof: Cell<bool>,
    /// rubash#1130: a [[ ]] operand word hit a FATAL expansion error
    /// (nounset unbound) during cond_expand_word; the diagnostic and
    /// exit_code are already in place and
    /// execute_conditional_command_with_redirects converts the flag
    /// into the FORCE_EOF/DISCARD unwind, since the conditional
    /// evaluator itself only returns an i32 status.
    pub(crate) conditional_expansion_fatal: Cell<bool>,
    /// rubash#1201: the exit status expand_conditional_word computed for
    /// the fatal above. The wrapper must restore THIS status after
    /// execute_conditional_command overwrites exit_code with the
    /// conditional's own result: `[[ -z $UNSET ]]` under `set -u` evaluates
    /// -z of the empty substitution to 0, and GNU's FORCE_EOF unwind keeps
    /// the fatal status (1) instead of the predicate result.
    pub(crate) conditional_fatal_status: Cell<Option<i32>>,
    /// rubash#1422: a [[ ]] operand word hit a NON-fatal expansion
    /// error (substring expression < 0 / bad substitution, status 1)
    /// during cond_expand_word; the diagnostic and exit_code are
    /// already in place and execute_conditional_command_with_redirects
    /// converts the recorded status into the DISCARD unwind
    /// (ExpansionFailure), mirroring the simple-command handling of
    /// the same error class.
    pub(crate) conditional_expansion_error_status: Cell<Option<i32>>,
    /// rubash#1915: the process-substitution span targets the parser
    /// recorded for the operands of the `[[ ]]` command currently
    /// being evaluated, each paired with the exact operand text it was
    /// recorded for. `execute_conditional_command` installs the table
    /// (save/restore around the evaluation); the conditional nounset
    /// scan points mask exactly the spans recorded for the word under
    /// scan before judging it — GNU's parent word walk extracts a
    /// procsub span verbatim (subst.c:1311 extract_process_subst) and
    /// never expands its body (subst.c:6362 process_substitute owns
    /// it), the same structural criterion the simple-command pre-scan
    /// applies through `CommandNode::process_substitutions`. A
    /// look-alike span in a quoted sibling operand was never recorded
    /// for THAT word, so its `$` stays a live parent reference. Empty
    /// outside conditional evaluation.
    pub(crate) conditional_procsub_span_targets: Vec<(String, Vec<String>)>,
    /// rubash#637: marks a pending evalerror abort as bad-subscript class.
    /// The eval boundary absorbs exactly this class (eval returns failure,
    /// the caller continues — GNU evalstring.c:365,422-442 swallowing
    /// DISCARD) while arithmetic failures keep escaping. Armed alongside
    /// the abort at the bad-subscript sites; consumed at the abandon point
    /// and cleared at every pending-clearing boundary, same discipline as
    /// `nounset_force_eof`.
    pub(crate) evalerror_subscript_pending: Cell<bool>,
    /// rubash#753: marks a pending evalerror abort as the failglob /
    /// redirect-no-match word-expansion class. The eval and source
    /// boundaries absorb exactly this class (GNU evalstring.c:365,422-442
    /// and evalfile.c's parse_and_execute both install a local top_level
    /// catch: the discard ends only the eval'd string / sourced file and
    /// the CALLER's remaining commands run — probe `eval 'set -- nope*';
    /// echo K` prints K). Arithmetic and nounset subscript failures keep
    /// escaping those boundaries (rubash#637/#639), so the class needs its
    /// own marker beside `evalerror_subscript_pending`. Armed alongside
    /// `evalerror_pending` at every failglob site; consumed at the eval /
    /// source boundary and cleared at every pending-clearing boundary,
    /// same discipline as `evalerror_subscript_pending`.
    pub(crate) evalerror_failglob_pending: Cell<bool>,
    /// rubash#705: marks a pending evalerror abort as the `exit` too-many-
    /// arguments class (GNU builtins/common.c no_args, bash 5.3: prints the
    /// diagnostic, `set_exit_status (EX_BADUSAGE)` = 2, then
    /// `top_level_cleanup` + `jump_to_top_level (DISCARD)`). The status the
    /// discard leaves behind is 2 — not the plain DISCARD 1 — and the
    /// top_level_cleanup pops every parse_and_execute level, so the jump
    /// lands at the OUTERMOST reader: script/stdin continue with the next
    /// command ($?=2), while a `-c` string is abandoned entirely and
    /// shell.c:1477 run_one_command's DISCARD catch exits 1. Unlike the
    /// failglob/subscript classes this discard is NOT absorbed at the
    /// eval/source boundaries (their string streams were popped); the
    /// eval'd list just unwinds whole. Armed alongside `evalerror_pending`
    /// at the exit interception site; consumed at the reader abandon point
    /// and cleared at every pending-clearing boundary, same discipline as
    /// `evalerror_subscript_pending`.
    pub(crate) evalerror_exit_toomany_pending: Cell<bool>,
    /// rubash#1518: marks a pending evalerror abort as the ARITHMETIC
    /// evalerror class (GNU expr.c `evalerror` -> `jump_to_top_level
    /// (DISCARD)`). Unlike the bad-substitution / failglob classes —
    /// whose word-expansion errors a `set -e` shell treats as fatal
    /// (GNU 5.3.20 probes: `set -e; echo ${#:}; echo after` and the
    /// failglob twin both stop) — an arithmetic evalerror is a pure
    /// jump in every context: it bypasses the errexit epilogue exactly
    /// like it bypasses the ERR trap (probes: assignment, argument,
    /// subscript, offset, for-list, case-word and declare forms under
    /// `set -e` all discard the command and continue with rc 0). The
    /// reader's errexit promotion therefore skips this class. Armed at
    /// the conversion funnels that turn a recorded arithmetic failure
    /// into `ExpansionFailure` (abort_on_expansion_errors,
    /// expand_command_words, the assignment-word funnel); cleared at
    /// every pending-clearing boundary, same discipline as
    /// `evalerror_failglob_pending`.
    pub(crate) evalerror_arith_pending: Cell<bool>,
    /// rubash#637: the `evalerror_exec_depth` value at which the currently
    /// executing eval'd list runs (`None` outside `eval`). A nested abort
    /// unwinds silently everywhere EXCEPT the eval'd top list itself, where
    /// GNU's parse_and_execute loops per line — so a later line still runs
    /// and eval returns its status. Function/loop bodies inside eval run
    /// deeper and keep unwinding. Save/restored per eval nesting, exactly
    /// like `__RUBASH_EVAL_CONTEXT`.
    pub(crate) eval_list_depth: Cell<Option<usize>>,
    /// Source line of the reader-level command whose list tail is being
    /// discarded; set lazily when the reader-level loop first observes
    /// `evalerror_pending`.
    evalerror_line: Cell<Option<usize>>,
    /// rubash#439: a WORD/ASSIGNMENT expansion error aborted the current
    /// simple command (GNU subst.c expand_wdesc_error ->
    /// exp_jump_to_top_level(DISCARD)). GNU fires the ERR trap only for a
    /// simple command that EXECUTED and failed (execute_cmd.c run_error_trap
    /// callers sit below expansion); the DISCARD jump bypasses them, so the
    /// trap never fires for the expansion failure itself. Set by the word/
    /// assignment expansion error sites, reset at each execute_ast loop
    /// iteration, consulted by maybe_run_error_trap.
    pub(crate) command_expansion_failed: Cell<bool>,
    /// rubash#1164: GNU samples `was_error_trap` (execute_cmd.c:914/851 —
    /// signal_is_trapped(ERROR_TRAP) && !signal_is_ignored) BEFORE a
    /// command executes and adjudicates that command's failure against the
    /// sample, reading the action string at fire time. A trap INSTALLED by
    /// the command itself (a function that sets an ERR trap in its body,
    /// then fails via its action's `return`) therefore does not re-fire
    /// for the enclosing call. The ast_exec list loop refreshes this per
    /// command; execute_ast_inner saves/restores it across nested lists.
    /// None means no enclosing command is being adjudicated (direct
    /// dispatch paths) — the runner then falls back to live presence.
    pub(crate) err_trap_armed_sample: Cell<Option<bool>>,
    /// Nesting depth of `execute_ast_inner`: a nested list (function body,
    /// loop body, sourced text, ...) unwinds silently while an evalerror
    /// abort is pending; only the reader-level loop skips to the next
    /// source line.
    evalerror_exec_depth: Cell<usize>,
    /// rubash#395: the failure status a `break 0`/`continue 0`/
    /// `break -1` carries out of the loop (GNU break.def:86-88: sh_erange,
    /// `breaking = loop_level`, return EXECUTION_FAILURE — the loop still
    /// unwinds, with status 1). The Break unwind resets exit_code at every
    /// loop boundary, so the builtin's failure rides this Cell to the
    /// innermost consumer arm.
    break_failure_status: Cell<Option<i32>>,
    /// Source line of the command currently executing at reader level —
    /// becomes the abort boundary when `evalerror_pending` is observed.
    reader_command_line: Cell<Option<usize>>,
    /// rubash#394: `>(...)`-text -> carrier path for the compound command
    /// currently in scope (materialize_compound_output_process_substitutions
    /// -> expand_redirect_target; cleared at the compound's finish). The
    /// compound fd-table walk runs on the ORIGINAL command node whose
    /// ordered-list entries still carry the verbatim `>(` text.
    procsub_carrier_memo: std::cell::RefCell<HashMap<String, String>>,
    /// shell.c:183 current_command_number support: source line of the last
    /// reader-level list that incremented the counter (eval.c:178). A
    /// semicolon-joined command sequence on one source line is ONE GNU
    /// list (read_command gathers the whole line), so the counter steps
    /// only when the top-level node's line changes.
    reader_last_list_line: Cell<Option<usize>>,
    /// True while a `bash -c` command string runs: shell.c
    /// run_one_command bypasses reader_loop, so current_command_number
    /// never increments (WSL GNU 5.3.0: `bash -c 'x="\#"; echo ${x@P}'`
    /// prints 1).
    command_string_mode: Cell<bool>,
    /// rubash#1497: armed by the -c driver for the command string's
    /// final reader unit (GNU builtins/evalstring.c:126
    /// should_suppress_fork requires startup_state == 2 — a command
    /// string, never a script file); run_source_impl consumes it and
    /// raises exec_replace_final_frame for that unit's execute_ast only.
    pub(crate) command_string_final_pending: Cell<bool>,
    /// rubash#1497: true while run_source_impl executes the final -c
    /// unit's AST; the depth-1 list walk stamps that unit's last plain
    /// command (see exec_replace_dispatch_stamp).
    pub(crate) exec_replace_final_frame: Cell<bool>,
    /// rubash#1497: nonzero stamp of the -c tail dispatch currently in
    /// flight on this executor. execute_ast_inner zeroes it for every
    /// nested list walk (function bodies, trap actions, sourced/eval
    /// units, compound bodies), so the spawn-site exec gate can only
    /// ever fire for the tail command's own dispatch.
    pub(crate) exec_replace_dispatch_stamp: Cell<u64>,
    /// rubash#1497: monotonic source for exec_replace_dispatch_stamp.
    pub(crate) exec_replace_stamp_counter: Cell<u64>,
    /// rubash#1497: one-shot pass letting an and_or_list's last element
    /// keep the enclosing dispatch stamp across its execute_ast wrapper
    /// (GNU execute_cmd.c:2886 execute_connection walks operands in the
    /// caller's frame; this port wraps each operand in execute_ast).
    pub(crate) exec_replace_stamp_hold: Cell<u32>,
    /// rubash#1497: SHLVL as the tail dispatch began — captured when the
    /// dispatch is stamped and refreshed when an and_or_list hands the
    /// stamp to its last operand. GNU execute_cmd.c:5813
    /// execute_disk_command reads the CURRENT SHLVL variable for
    /// adjust_shell_level; the command's own prefix temp assignments
    /// are applied to the child environment only and never enter that
    /// value. This port commits prefix assignments into env_vars on
    /// the way to the spawn (the fork path's child env depends on it),
    /// so the spawn-site read must use this pre-commit snapshot, not
    /// the live variable.
    pub(crate) exec_replace_shlvl_snapshot: Cell<Option<i64>>,
    /// rubash#353: locale in effect when each SOURCE LINE was first reached
    /// at reader level — the locale GNU's parser would decode that line's
    /// `$'...'` backslash-u escapes under (GNU reads a newline-terminated
    /// list at a time, parse.y ansiexpand at read_token_word time). Keyed
    /// by the line of the reader-level command that opens the line.
    line_lex_locales: std::cell::RefCell<HashMap<usize, String>>,
    /// rubash#353: the locale this execution unit (file/-c string) was
    /// upfront-lexed under; a word whose metadata `lex_locale` equals it
    /// came from that upfront lex (not from a later eval-time lex) and is
    /// eligible for the read-time re-derivation.
    unit_lex_locale: std::cell::RefCell<Option<String>>,
    /// GNU `line_number` ambient: at top level it advances to each
    /// command's parse-end line; inside a pre-parsed body it freezes at
    /// the enclosing command's established line (execute_cmd.c — only
    /// simple/for/select/case/arith/cond/subshell commands stamp their
    /// own `->line`; while/until/if/group inherit the ambient).
    ambient_line: Cell<Option<usize>>,
    /// True while an if/elif condition list is executing: word-expansion
    /// failures must pierce function frames so the enclosing compound
    /// command can abandon itself entirely (GNU probe f4).
    pub(crate) inside_compound_condition: Cell<bool>,
    /// Pending `!` inversion for the next evaluated `[[ ]]` leaf term —
    /// GNU parse.y XORs CMD_INVERT_RETURN onto the term node and
    /// xtrace_print_cond_term (print_cmd.c:956) prints it as `! ` inside
    /// the trace. Consumed when the leaf term traces.
    pub(crate) conditional_invert_pending: Cell<bool>,
    /// True while a scalar assignment RHS is expanding: GNU param_expand
    /// carries PF_ASSIGNRHS into `${!arr[@]}` so the unquoted `@` key list
    /// takes the dollar_at path (elements quoted, space-joined, never
    /// field-split) instead of the dollar_star IFS[0] join used for plain
    /// command words (subst.c string_list_pos_params). Fresh Executor
    /// instances (command substitution, subshells) start false, matching
    /// GNU dropping PF_ASSIGNRHS across a nested substitution boundary.
    pub(crate) inside_assignment_rhs: Cell<bool>,
    last_command_substitution_status: Cell<Option<i32>>,
    /// subst.c command_substitute: after wait_for (pid) the parent records
    /// the substitution child's status in last_command_exit_value, so `$?`
    /// in a word to the RIGHT of a substitution in the SAME command's word
    /// list observes it (`echo $(false) $?` prints 1). This overlay holds
    /// that status for the in-flight word expansion only; it is cleared at
    /// each command-node boundary (execute_ast_inner) so the next command
    /// reads its own committed status again. None means no substitution
    /// completed during this expansion — $? falls back to exit_code.
    word_expansion_comsub_exit: Cell<Option<i32>>,
    /// GNU subst.c:7143 command_substitute forks sharing fd 0, so input the
    /// substitution body consumed must advance the caller's FUNCTION_STDIN
    /// cursor. The substitution runs on `&self`; stash the child's final
    /// cursor plus a fingerprint of the shared buffer and apply it lazily at
    /// the next `&mut self` stdin consumer.
    comsub_stdin_writeback: Cell<Option<(usize, u64)>>,
    /// GNU execute_cmd.c execute_pipeline gives the first pipeline element
    /// the shell's own fd 0: the shared open-file cursor advances only by
    /// what the element actually read. Stage helpers run on subshell clones
    /// or real child processes, so the measured consumption is reported back
    /// through this cell and folded into FUNCTION_STDIN_OFFSET by the
    /// pipeline driver (initial_pipeline_input must not eagerly drain).
    pipeline_stdin_consumed: Cell<Option<usize>>,
    /// Set by a sequential pipeline stage that wired the child's fd 1/fd 2
    /// DIRECTLY onto their ordered targets (both fds sharing one open file
    /// description — `>f 2>&1`, `>f |&`): the driver then skips
    /// route_pipeline_stage_streams for the stage, because the routing
    /// walk re-opens `>f` and would truncate the bytes the child already
    /// wrote live through the shared description (niubash#144).
    pipeline_stage_fds_pre_wired: Cell<bool>,
    /// rubash#1454: set by the sequential pipeline driver around the LAST
    /// stage's dispatch only. The stage spawner may then hand an external
    /// final member the shell's own stdout (Stdio::inherit) instead of a
    /// capture pipe when final_member_stdout_streams_to_host agrees —
    /// GNU execute_cmd.c:2620 leaves the rightmost element's fd 1 on the
    /// shell's stdout, so a pager there sees the terminal. Non-final
    /// stages never observe this flag: their fd 1 is the inter-member
    /// pipe whose payload the driver routes onward.
    pipeline_final_stage: Cell<bool>,
    /// rubash#1342: set on the throwaway subshell clone in
    /// execute_builtin_pipeline_stage, whose stage command the pipeline
    /// driver already traced (execute_pipeline_stage_inner prints every
    /// element's line once, execute_cmd.c:4649). The clone's re-dispatch
    /// through execute_command would print the same line a second time
    /// for every builtin routed there (set/export/declare/umask/...),
    /// while inline stage arms never re-dispatch and traced once. The
    /// dispatch trace site consumes the flag (replace(false)) so exactly
    /// one trace — the driver's — survives; nested commands inside the
    /// builtin (an eval body, say) trace normally.
    pipeline_stage_pre_traced: Cell<bool>,
    /// Set by `restricted_command_error` when a sequential pipeline stage is
    /// refused (#804): the driver then skips `route_pipeline_stage_streams`
    /// for the stage, because the routing walk would OPEN the refused
    /// redirect's target (`>& f`) and materialize the very file the
    /// restricted shell must never create — GNU refuses the member inside
    /// its subshell before do_redirections, so nothing opens.
    pipeline_stage_restricted_refused: Cell<bool>,
    /// Tracks the source of the last heredoc EOF warning emitted from
    /// command_substitution_heredoc_output_mut_typed, to avoid duplicate
    /// warnings when the same comsub is expanded through multiple paths
    /// (expand_assignment_value_inner → expand_command_substitution_mut_typed
    /// and expand_embedded_parameters_mut → expand_command_substitution_mut).
    last_heredoc_warning_source: RefCell<Option<String>>,
    /// Decline-only memo for the heredoc comsub shortcut: the
    /// (IFS_GLUE-stripped) source whose most recent admission attempt
    /// declined. The mut dispatcher tries the typed heredoc entry
    /// first and, on decline, falls through to routes that retry the
    /// readback entry for the same source; the memo lets that retry
    /// skip a second tokenize/parse + admission pass. It can never
    /// admit anything — a hit only routes the body to the real
    /// executor — so a stale record costs at most the shortcut, never
    /// semantics. An unrelated heredoc attempt clears it.
    heredoc_admission_declined_source: RefCell<Option<String>>,
    /// Leading newlines in the comsub source (before trimming). Used to
    /// adjust heredoc warning line numbers: when `$(` is at end of line,
    /// the comsub body starts on the next line.
    comsub_leading_newlines: Cell<usize>,
    /// A current-shell (`${ ...; }` / `${| ...; }`) body that ran `exit N`
    /// aborts the enclosing (sub)shell with status N (GNU subst.c: the
    /// nofork body shares the shell's exit path). The walker records the
    /// status here; command_execute converts it into ExecuteError::ExitCode
    /// after word expansion so the enclosing context unwinds.
    current_shell_substitution_exit: Cell<Option<i32>>,
    last_command_substitution_parse_error: Cell<bool>,
    /// The comsub body whose parse error is currently latched was an
    /// old-style backtick substitution. GNU parses a backtick body at
    /// execution time inside the substituted child (subst.c string_locale ->
    /// parse_and_execute), so its syntax error kills only the child: the
    /// substitution yields empty, status 2 becomes the command's status,
    /// and the script continues (issue #515; `echo \`case x in\`` prints
    /// the `command substitution: line N:` diagnostic and rc 0). The
    /// `$(`-body fatal (read-time failure of the enclosing command) checks
    /// this flag before its escalation/abort sites.
    last_command_substitution_parse_error_backtick: Cell<bool>,
    /// A backtick body is being expanded RIGHT NOW — its inner
    /// parse/diagnostic layer must render as GNU's parse_and_execute
    /// child: `command substitution: line N:` prefix
    /// (error.c get_name_for_error with yy_input_name()=="command
    /// substitution") and a newline-terminated body reader.
    backtick_body_diagnostic: Cell<bool>,
    /// GNU error.c:324-327 (parser_error): when exit_immediately_on_error
    /// is set at diagnostic time the shell prints only the FIRST line of a
    /// syntax-error report and immediately exit_shell(2)s — the offending
    /// -line echo (parse.y:6814 print_offending_line) never runs, and no
    /// enclosing eval/source containment applies (exit_shell is terminal).
    /// Errexit-active parse-error reports set this so the eval containment
    /// (execute_eval_source, rubash#306) propagates the ExitCode instead of
    /// swallowing it, and the two-line report collapses to one line.
    parser_error_errexited: Cell<bool>,
    /// GNU execute_cmd.c:626-652: `! CMD` sets CMD_INVERT_RETURN on the
    /// inner command, and with exit_immediately_on_error on it additionally
    /// gains CMD_IGNORE_RETURN (execute_cmd.c:652-656) — an inverted
    /// command's status never satisfies errexit. The grouped script drivers
    /// (run_script_with_history / the stdin loop) re-check
    /// `status != 0 && errexit` after each complete command and cannot see
    /// the node's flags, so the flat-command loop records here whether the
    /// last executed top-level command carried `!`; the drivers consult it
    /// before breaking (set-e1.sub `! true` under `set -o posix`).
    pub(crate) last_command_inverted: Cell<bool>,
    /// GNU execute_cmd.c:854-883: a compound command's OWN redirect failure
    /// abandons it with status 1 and returns BEFORE the :1204 invert step,
    /// so `! { :; } > bad` stays 1 (no flip) while `! true > bad` (simple
    /// command, flipped at :1204) is 0. Set at each compound's own
    /// do_redirections-failure adjudication (IoError from the apply stage
    /// only — never ExpansionFailure, never a body write error); simple
    /// commands never set it. The `!` flip sites (compound_exec
    /// execute_inverted_ast_command, ast_exec command dispatch) suppress
    /// the flip while it is set and leave it set: nested `! ! C` wrappers
    /// suppress in turn, and the innermost fork/call boundary that owns a
    /// flip of its own (subshell-region single-inverted-body at
    /// execute_cmd.c:1831, pipeline wait-side at :763) consumes it.
    /// Comsub / function / eval boundaries consume it without flipping
    /// (command_substitute has no :1831-equivalent); the flat-command loop
    /// clears it between top-level commands as a backstop. rubash#846.
    pub(crate) compound_redirect_failed: Cell<bool>,
    /// rubash#1025: expanded targets a compound command's prepare phase
    /// (apply_compound_*_arm / apply_brace_group_redirects) already
    /// created through the noclobber O_EXCL open in
    /// create_redirect_output_prepared. The bind phase
    /// (open_compound_output_redirects) opens the SAME redirect again to
    /// bind the fd table; without this record its O_EXCL open fails on
    /// the file the prepare phase just created, so
    /// `set -C; { :; } > fresh` wrongly reports "cannot overwrite
    /// existing file" for a file that did not exist before the command.
    /// The bind phase consumes (removes) the record, a genuinely
    /// pre-existing file still fails in the prepare phase itself, and
    /// simple commands never consult the set.
    pub(crate) noclobber_prepared_targets: std::cell::RefCell<std::collections::HashSet<String>>,
    /// The group's execute_ast ended by unwinding (`exit`, errexit, POSIX
    /// special-builtin failure, fatal expansion — GNU's
    /// jump_to_top_level(EXITPROG/ERREXIT/FORCE_EOF)). run_source converts
    /// the ExecuteError into a bare status; this flag lets the grouped
    /// drivers stop reading instead of treating it as an ordinary nonzero
    /// command status (builtins source5.sub: `. missing` under `set -o
    /// posix` exits the shell even without `set -e`).
    pub(crate) exit_jump_pending: Cell<bool>,
    /// Set by the dispatch-level errexit conversion sites when an
    /// Ok-completed command's failing status is converted into
    /// ExecuteError::ExitCode purely for errexit/POSIX-exit adjudication
    /// (execute_materialized_command's tail and its siblings in
    /// execute_command). GNU adjudicates errexit in execute_command's
    /// CALLER, so such a conversion is not a jump out of the command;
    /// execute_time_ast_command takes this mark to tell a converted
    /// status from a genuine unwind (exit builtin, FORCE_EOF), whose
    /// timing report GNU drops (rubash#1611). Cleared at every
    /// execute_time_ast_command entry and taken right after its inner
    /// dispatch, so a stale mark can never be consumed.
    pub(crate) errexit_status_converted: Cell<bool>,
    /// Depth of enclosing execute_time_ast_command inner dispatches.
    /// While > 0, the dispatch-level errexit conversion sites convert
    /// WITHOUT firing the ERR trap: the timed command completes through
    /// execute_time_ast_command and the ast_exec time branch performs
    /// the single trap adjudication for it (the rubash#1277 design —
    /// the same treatment function and pipeline inners already get).
    /// Outside a timed dispatch the sites fire the trap themselves, as
    /// the plain list loop never adjudicates an Err completion
    /// (rubash#1611).
    pub(crate) timed_dispatch_depth: Cell<u32>,
    /// GNU execute_cmd.c:4887-4888 sets `special_builtin_failed = 1` when a
    /// POSIX special builtin returns an error status (> EX_SHERRBASE = 256).
    /// After the command (execute_cmd.c:1004-1017), if `posixly_correct &&
    /// interactive == 0 && special_builtin_failed`, the shell exits with the
    /// converted status. Rubash sets this flag in each special builtin that
    /// returns EX_USAGE/EX_UTILERROR/etc., and checks it in
    /// `execute_materialized_command` to exit the noninteractive POSIX shell.
    special_builtin_failed: Cell<bool>,
    /// Set by `route_builtin_buffered_output` when a builtin's buffered
    /// stdout/stderr could not be delivered because the target fd is
    /// closed — the executor-side analogue of GNU `sh_chkwrite`
    /// (builtins/common.c:320) turning the builtin's result into
    /// EXECUTION_FAILURE. Status-returning builtin wrappers read it via
    /// `take_builtin_write_failed` so the caller's
    /// `self.exit_code = self.execute_x(cmd)?` does not overwrite the
    /// write-failure status with the builtin's nominal zero.
    last_builtin_write_failed: Cell<bool>,
    /// How many LEADING words of the last alias expansion came from alias
    /// VALUES (parse.y push_string): only that prefix is alias-introduced
    /// syntax; every word after it is original input whose post-expansion
    /// content is DATA. execute_alias_expanded_syntax re-parses only when
    /// the prefix itself carries operators/assignments, and shell-quotes
    /// the tail words when rebuilding the source so their spaces/`=' cannot
    /// re-tokenize (modernish's `alias let='let --'` + `let "a = b"`
    /// became `let -- -- a = b` through the unquoted join + re-expansion).
    alias_introduced_words: Cell<usize>,
    /// GNU redir.c:298 redirection_expand expands each redirect word once,
    /// inside do_redirections' single left-to-right pass. Rubash resolves
    /// redirect targets in several passes (ambiguity precheck, fd-scope
    /// binding, post-hoc pipeline routing); this memo lets the passes share
    /// one expansion per Redirect node so `$((n+=1))`/`$(cmd)` in a target
    /// runs once per command execution. Cleared at each execute_command
    /// entry; keyed by node address, valid only within that command's
    /// execution span.
    redirect_target_memo: RefCell<HashMap<String, String>>,
    /// The field-count fact the ambiguity gate cannot recover from the
    /// carrier String: keys (same key as redirect_target_memo) whose
    /// expansion resolved through pathname expansion to EXACTLY ONE
    /// match. GNU subst.c:12645-12676 (glob_expand_word_list) returns
    /// one field per match, and glob matches are never word-split, so a
    /// single match is one field even when its name contains whitespace
    /// (rubash#1769: `> only*.txt` with one file `only one.txt` opens
    /// it, it is not AMBIGUOUS_REDIRECT). The carrier String joins
    /// multi-field results with ' ', which the textual gate cannot
    /// distinguish from one space-containing field — this set carries
    /// the count the gate needs. Cleared at each execute_command entry
    /// like redirect_target_memo; only the single-match arm sets it,
    /// every other arm (multi-match join, no-match literal, brace
    /// branches, failglob pattern) leaves the gate's existing behavior
    /// untouched, so an unquoted `$var` that word-splits stays ambiguous.
    redirect_target_single_glob_match: RefCell<HashSet<String>>,
    /// GNU subst.c:3565 do_assignment_internal expands an assignment RHS
    /// exactly once and subst.c:3576 hands the COMPUTED value to
    /// xtrace_print_assignment — the trace never re-runs the expansion.
    /// apply_permanent_assignments/apply_temporary_assignments record each
    /// computed value here so the command's xtrace rendering (and any other
    /// consumer needing the bound value) reuses it instead of executing a
    /// `v=$(cmd)` RHS a second time (`args=$(echo hi) echo x` under `set -x`
    /// must run `echo hi` once, rubash#254). Keyed by (name, raw value);
    /// cleared at each execute_command entry like redirect_target_memo.
    assignment_expansion_memo: RefCell<HashMap<(String, String), String>>,
    /// GNU execute_cmd.c: external commands (and the forced fork of a
    /// wordless `{var}` command, execute_null_command:4203-4278) run
    /// do_redirections in the child — the `{var}` bind never reaches the
    /// parent environment. The generic fd_var application in
    /// execute_command records (resolved name, prior env_vars entry, prior
    /// typed-store entry) here so execute_external can undo the binding and
    /// release the descriptor after the child finishes.
    fd_var_external_undo: Vec<(
        String,
        Option<String>,
        Option<crate::shell::variables::Variable>,
    )>,
    /// rubash#1646: descriptors a `{var}` redirection opened under
    /// `shopt varredir_close` on a command-scoped application. GNU closes
    /// them when the owning command COMPLETES (redir.c undo_redirections
    /// at execute_cmd.c command end); closing at application time — the
    /// previous behavior — rejected the command's own `>&$var` writes
    /// with EBADF and silently dropped a `>(...)` consumer's whole
    /// stream. execute_dynamic_fd_var_redirect therefore only RECORDS
    /// the fd here; execute_command drains the records its command added
    /// (watermark discipline, so nested commands keep their own closes).
    deferred_varredir_closes: Vec<u32>,
    /// builtins/read.def read_timeout: absolute deadline for `read -t N`
    /// (and the TMOUT default). Set only while a `read` builtin call is
    /// active; consulted by the genuinely blocking endpoints (inherited
    /// process stdin, coproc pipes) — buffered endpoints never wait.
    pub(crate) read_deadline: Option<std::time::Instant>,
    /// A bounded read expired mid-line: GNU assigns the partial input and
    /// returns 128+SIGALRM=142 (read.def:539-562 `goto assign_vars`).
    pub(crate) read_timed_out: bool,
    /// rubash#1250: an untrapped SIGINT aborted a blocking `read` — the
    /// ForegroundBuiltinSigintGuard routed it to the pending-signal
    /// mailbox and the read loop's slice poll consumed it. GNU read.def:
    /// the interrupted read assigns NOTHING and returns 128+SIGINT.
    /// Reset by arm_read_deadline at every `read` entry.
    pub(crate) read_interrupted_signal: Option<i32>,
    /// True while a `read` input acquisition holds the foreground
    /// SIGINT guard (external_inner::ForegroundBuiltinSigintGuard), so
    /// the blocking read loops know their slice polls may consume an
    /// untrapped SIGINT from the mailbox.
    pub(crate) read_sigint_guard_armed: bool,
    /// GNU read.def:949 `retval = eof ? EXECUTION_FAILURE : SUCCESS`: the
    /// record read exhausted its input before the delimiter (`read -d ''`
    /// over a NUL-free stream, `read v` without a trailing newline). The
    /// partial record is still assigned; only the status becomes 1. Merged
    /// with the fd-table copy by `take_read_eof_no_delimiter`.
    pub(crate) read_eof_no_delimiter: bool,
    /// rubash#1141: production timing of the input currently staged on
    /// FUNCTION_STDIN (a pipeline stage's captured producer output), so
    /// `read`/`select` can judge an armed deadline against the stream's
    /// real arrival schedule instead of its materialization instant.
    /// Installed/restored by the sequential pipeline driver alongside
    /// the FUNCTION_STDIN pair; substitution clones inherit it (their
    /// reads draw on the same stream). See timed_input.rs.
    pub(crate) timed_stdin: Option<timed_input::SharedTimedInput>,
    stdout_capture: Option<Vec<u8>>,
    stderr_capture: Option<Vec<u8>>,
    /// niubash#345: while a shell-side pipeline producer (compound `while`/
    /// `for`/`until`, ...) streams its fd 1 into the NEXT stage's OS pipe
    /// instead of the in-memory capture. Set by
    /// `execute_shell_writer_pipeline` around the producing stage; inherited
    /// by the stage's subshell (command_substitution_executor). Default fd-1
    /// writes go to this raw handle; a broken pipe (the downstream consumer
    /// exited) terminates the producing stage with the SIGPIPE status (141)
    /// so an unbounded shell-side writer stops exactly like GNU's
    /// SIGPIPE-killed pipeline element, instead of buffering forever.
    pipeline_stdout_stream: Option<crate::fd::HANDLE>,
    /// rubash#761: while a shell-side pipeline CONSUMER (a builtin tail such
    /// as `true`/`:`/`read`/`echo`) reads the NEXT upstream external
    /// producer's stream from a real OS pipe instead of the in-memory
    /// FUNCTION_STDIN mirror. Set by
    /// `execute_external_prefix_streaming_tail` around the consuming stage;
    /// the stage's readers (`read`, ...) then pull fd 0 = this raw pipe
    /// handle, so an unbounded producer (`yes`, ...) is closed/terminated the
    /// moment the consumer stops reading, exactly like GNU's forked element
    /// whose read end closes when the rightmost command exits. Without it the
    /// sequential loop materialized the producer's whole stream and never
    /// reached the consumer (`yes | true` drained into RAM forever).
    /// Mirrors `pipeline_stdout_stream` for the consumer side.
    pipeline_stdin_stream: Option<crate::fd::HANDLE>,
    /// niubash#144: outcome of the external stdio plan from the last
    /// `apply_external_redirects` (ordered-walk shared handles), consumed
    /// by the immediately following `spawn_external_process`. `Some(..)`
    /// means the child already owns real handles for fd 1/2 and the legacy
    /// capture-replay routes MUST NOT touch the command again (an empty
    /// replay would re-open `>f` and truncate the child's live output);
    /// `CapturePipe` additionally carries the ONE shared pipe the plan
    /// bound the merged fds to (a command substitution's pipe, GNU
    /// subst.c:7143 command_substitute) for the post-exit drain into the
    /// active capture. `None` means the planner declined and the legacy
    /// split/capture routes own the command untouched.
    external_stdio_outcome: Option<external_redirects::ExternalStdioOutcome>,
    /// Set by the per-fd legacy branches of `apply_external_redirects` when
    /// they hand the child a `Stdio::piped()` stream for a shape the stdio
    /// planner declined — the parent must then drain that pipe (a full pipe
    /// deadlocks the child) and route the bytes to the resolved fd
    /// endpoint. Reset at the start of every `apply_external_redirects`.
    external_stdio_piped_fallback: bool,
    host_external_command_handler: Option<HostExternalCommandHandler>,
    #[cfg(windows)]
    elevation_handler: Option<ElevationHandler>,
    external_file_builtins_enabled: bool,
    process_env_snapshot: HashMap<String, String>,
    /// G-03: this lineage's process-env mirror-write journal (see
    /// local_helpers::EnvWriteJournal). Drop restores only journaled keys
    /// whose current value still equals the journal's final state.
    env_write_journal: EnvWriteJournal,
    history_provider: Option<crate::history::SharedHistoryProvider>,
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod splits_gate;
