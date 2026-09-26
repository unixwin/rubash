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
use crate::executor::{
    FunctionBody, FunctionDefInfo, FunctionDefinitionLocation, RandomGen, VarAttrs,
};
use crate::history::SessionHistory;
use crate::parser::CommandNode;
use crate::shell::Variable;

use super::variables::VariableStore;

#[derive(Debug)]
pub struct ShellState {
    /// variables.c shell_variables — typed store carrying attributes,
    /// arrays, and assoc maps.
    pub(crate) variables: VariableStore,
    /// variables.c shell_variables flat mirror — env/option carrier map.
    /// Kept in sync with `variables` at mutation sites during the ongoing
    /// typed-store migration.
    pub(crate) env_vars: HashMap<String, String>,
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
    /// execute_cmd.c/jobs.c pipeline status vector ($PIPESTATUS).
    pub(crate) pipestatus: Vec<i32>,
    /// variables.c FUNCNAME stack.
    pub(crate) function_name_stack: Vec<String>,
    /// variables.c BASH_ARGC/BASH_ARGV stacks.
    pub(crate) bash_argc_stack: Vec<String>,
    pub(crate) bash_argv_stack: Vec<String>,
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
    /// variables.c funcnest / function call depth.
    pub(crate) function_depth: usize,
    /// variables.c dollar_vars_changed bookkeeping for `set` scope.
    pub(crate) dollar_vars_changed_by_set: bool,
    /// lib/sh/random.c RANDOM/SRANDOM generator state.
    pub(crate) random_state: RandomGen,
    /// execute_cmd.c subshell_level.
    pub(crate) subshell_depth: Cell<usize>,
    /// execute_cmd.c subshell_environment & SUBSHELL_COMSUB — set while a
    /// `$( )`/backtick body runs (shared-executor and cloned-executor
    /// paths). jobs.c:3837-3843 start_job refuses fg/bg inside it.
    pub(crate) in_command_substitution: Cell<bool>,
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
    /// bashhist.c per-session history list. Cloned deeply (not the Rc) so a
    /// subshell's `history` mutations cannot reach the parent — GNU gets
    /// the same isolation from the fork copy.
    pub(crate) session_history: Option<Rc<RefCell<SessionHistory>>>,
    /// Arithmetic expansion error flags (subshell boundary).
    /// GNU expr.c raises FORCE_EOF on unbound variable under `set -u`;
    /// these flags isolate errors in command substitutions from the outer shell.
    /// TODO: Move to ShellState::clone() for automatic isolation (requires &mut self API change)
    pub(crate) arithmetic_expansion_error: Cell<bool>,
    pub(crate) arithmetic_nonfatal_error: Cell<bool>,
    pub(crate) arithmetic_fatal_error: Cell<bool>,
    pub(crate) arithmetic_nounset_error: Cell<bool>,
    pub(crate) arithmetic_last_error_category:
        Cell<Option<crate::executor::arithmetic::ArithmeticErrorCategory>>,
    /// Bad substitution flag (subshell boundary).
    /// GNU subst.c:10277 - a mid-expansion bad substitution kills the
    /// substitution's command list, never the enclosing word's command.
    /// TODO: Move to ShellState::clone() for automatic isolation (requires &mut self API change)
    pub(crate) parameter_bad_substitution: Cell<bool>,
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
}

/// Buffered contents of one materialized `<(cmd)` output — see
/// `ShellState::procsub_streams`.
#[derive(Debug)]
pub(crate) struct ProcSubStream {
    pub(crate) data: Vec<u8>,
    pub(crate) offset: usize,
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
    arithmetic_expansion_error: bool,
    arithmetic_nonfatal_error: bool,
    arithmetic_fatal_error: bool,
    arithmetic_nounset_error: bool,
    arithmetic_last_error_category: Option<crate::executor::arithmetic::ArithmeticErrorCategory>,
    parameter_bad_substitution: bool,
    debug_trap_command: Option<String>,
    xtrace_fd: i32,
    xtrace_fd_source: String,
}

impl InteriorSnapshot {
    pub(crate) fn subshell_depth(&self) -> usize {
        self.subshell_depth
    }
}

impl ShellState {
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
            arithmetic_expansion_error: self.arithmetic_expansion_error.get(),
            arithmetic_nonfatal_error: self.arithmetic_nonfatal_error.get(),
            arithmetic_fatal_error: self.arithmetic_fatal_error.get(),
            arithmetic_nounset_error: self.arithmetic_nounset_error.get(),
            arithmetic_last_error_category: self.arithmetic_last_error_category.get(),
            parameter_bad_substitution: self.parameter_bad_substitution.get(),
            debug_trap_command: self.debug_trap_command.borrow().clone(),
            xtrace_fd: self.xtrace_fd.get(),
            xtrace_fd_source: self.xtrace_fd_source.borrow().clone(),
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
        saved
    }

    /// Restore a snapshot taken by `snapshot_interior`.
    pub(crate) fn restore_interior(&self, snapshot: &InteriorSnapshot) {
        self.subshell_depth.set(snapshot.subshell_depth);
        self.in_command_substitution
            .set(snapshot.in_command_substitution);
        self.stdin_redir.set(snapshot.stdin_redir);
        self.arithmetic_expansion_error
            .set(snapshot.arithmetic_expansion_error);
        self.arithmetic_nonfatal_error
            .set(snapshot.arithmetic_nonfatal_error);
        self.arithmetic_fatal_error
            .set(snapshot.arithmetic_fatal_error);
        self.arithmetic_nounset_error
            .set(snapshot.arithmetic_nounset_error);
        self.arithmetic_last_error_category
            .set(snapshot.arithmetic_last_error_category);
        self.parameter_bad_substitution
            .set(snapshot.parameter_bad_substitution);
        *self.debug_trap_command.borrow_mut() = snapshot.debug_trap_command.clone();
        self.xtrace_fd.set(snapshot.xtrace_fd);
        *self.xtrace_fd_source.borrow_mut() = snapshot.xtrace_fd_source.clone();
    }
}

impl Clone for ShellState {
    fn clone(&self) -> Self {
        Self {
            variables: self.variables.clone(),
            env_vars: self.env_vars.clone(),
            aliases: self.aliases.clone(),
            functions: self.functions.clone(),
            function_definition_redirects: self.function_definition_redirects.clone(),
            function_def_infos: self.function_def_infos.clone(),
            function_definition_locations: self.function_definition_locations.clone(),
            positional_params: self.positional_params.clone(),
            pipestatus: self.pipestatus.clone(),
            function_name_stack: self.function_name_stack.clone(),
            bash_argc_stack: self.bash_argc_stack.clone(),
            bash_argv_stack: self.bash_argv_stack.clone(),
            bash_lineno_stack: self.bash_lineno_stack.clone(),
            bash_source_stack: self.bash_source_stack.clone(),
            local_var_scopes: self.local_var_scopes.clone(),
            local_attr_scopes: self.local_attr_scopes.clone(),
            local_typed_scopes: self.local_typed_scopes.clone(),
            expanding_aliases: self.expanding_aliases.clone(),
            loop_depth: self.loop_depth,
            function_depth: self.function_depth,
            dollar_vars_changed_by_set: self.dollar_vars_changed_by_set,
            random_state: self.random_state.clone_state(),
            subshell_depth: Cell::new(self.subshell_depth.get()),
            in_command_substitution: Cell::new(self.in_command_substitution.get()),
            stdin_redir: Cell::new(self.stdin_redir.get()),
            job_table: self.job_table.clone(),
            last_background_pid: self.last_background_pid,
            coproc_names: self.coproc_names.clone(),
            completion_specs: self.completion_specs.clone(),
            session_history: self
                .session_history
                .as_ref()
                .map(|s| Rc::new(RefCell::new(s.borrow().clone()))),
            arithmetic_expansion_error: Cell::new(self.arithmetic_expansion_error.get()),
            arithmetic_nonfatal_error: Cell::new(self.arithmetic_nonfatal_error.get()),
            arithmetic_fatal_error: Cell::new(self.arithmetic_fatal_error.get()),
            arithmetic_nounset_error: Cell::new(self.arithmetic_nounset_error.get()),
            arithmetic_last_error_category: Cell::new(self.arithmetic_last_error_category.get()),
            parameter_bad_substitution: Cell::new(self.parameter_bad_substitution.get()),
            xtrace_fd: Cell::new(self.xtrace_fd.get()),
            xtrace_fd_source: RefCell::new(self.xtrace_fd_source.borrow().clone()),
            debug_trap_command: RefCell::new(self.debug_trap_command.borrow().clone()),
            // GNU fork shares the parent's open file descriptions, so a
            // process-substitution stream's read offset stays shared
            // across the clone boundary.
            procsub_streams: RefCell::new(self.procsub_streams.borrow().clone()),
        }
    }
}
