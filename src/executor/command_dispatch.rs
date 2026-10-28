use super::*;

impl Executor {
    pub(in crate::executor) fn execute_materialized_command(
        &mut self,
        cmd: &CommandNode,
        // DBG
        process_substitution_files: ProcessSubstitutionFiles,
    ) -> Result<(), ExecuteError> {
        let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_MATCMD);
        let standalone_assignments = cmd.words.is_empty() && !cmd.assignments.is_empty();
        let keep_temporary_assignments = self.keeps_temporary_assignments(cmd);
        // GNU execute_cmd.c: this_command_name is the command word for the
        // duration of the command — including its leading tempenv
        // assignments — so assignment diagnostics carry the command segment
        // (`FOO=x getopts ...` reports under `getopts:`). A command with no
        // words is a bare assignment list and reports no command segment.
        let previous_command_name = std::mem::replace(
            &mut self.assignment_command_name,
            cmd.words.first().cloned(),
        );
        if self.posix_function_declare_prefix_assignments_are_local(cmd) {
            self.save_assignment_local_names(&cmd.assignments);
        }
        // GNU redir.c:298 redirection_expand uses expand_words_no_vars:
        // redirect words expand with the temp environment IGNORED
        // (`a=9 echo >&$(echo $a)` sees the outer a, not 9). Expanding here —
        // before apply_temporary_assignments — seeds the redirect-target memo
        // with the correct pre-binding values; the apply sites reuse them.
        let redirect_rejection = self.reject_invalid_redirects(cmd)?;
        let mut skip_command = redirect_rejection.is_some();
        if redirect_rejection == Some(super::redirection::RedirectRejection::FatalNounset) {
            // rubash#535: an unbound variable in a redirect word is a fatal
            // word expansion error classified by where do_redirections runs
            // (finish_heredoc_expansion_error — the same main-shell/child
            // split the heredoc bodies use): a builtin/function target ends
            // the noninteractive script, an external target contains the
            // jump in the forked child. The diagnostic and status 1 are
            // already in place from the walker.
            self.finish_heredoc_expansion_error(cmd, Self::FATAL_PARAMETER_EXPANSION_STATUS)?;
            skip_command = true;
        }
        let redirect_rejected = skip_command;
        let assignment_raws = (cmd.assignment_raws.len() == cmd.assignments.len())
            .then(|| cmd.assignment_raws.as_slice());
        let temporary_assignments = if standalone_assignments {
            // GNU subst.c expand_word_error + execute_cmd.c: a failed
            // assignment with no command word aborts the command list in
            // every mode (the DISCARD jump); POSIX mode additionally EXITS
            // the non-interactive shell (subst.c:13177-13182
            // expand_word_list_internal's is_nullcmd arm sets
            // last_command_exit_value = EXECUTION_FAILURE and calls
            // posix_variable_assignment_error(0), subst.c:11181-11196 — the
            // non-interactive posix branch jumps FORCE_EOF;
            // `ro=2` where ro is readonly).
            let applied = self.apply_permanent_assignments(&cmd.assignments, assignment_raws);
            if !applied && self.posix_assignment_failure_is_fatal() {
                // The FORCE_EOF status is context-owned (shell.c:1471-1472
                // run_one_command `case FORCE_EOF: return
                // last_command_exit_value = 127` under -c; eval.c:104-109
                // reader_loop keeps last_command_exit_value = 1 for scripts;
                // execute_cmd.c:1811 contains the jump at 1 inside subshell
                // children — WSL 5.3.0: `(ro=2)` leaves $? = 1 and the
                // parent runs).
                let code = self.expansion_fatal_status();
                self.exit_code = code;
                return Err(ExecuteError::ExitCode(code));
            }
            Vec::new()
        } else {
            // rubash#1692: GNU binds a simple command's tempenv only
            // for builtin/function commands; for a disk command the
            // assignments become child-env entries and never bind in
            // the shell (execute_cmd.c assign_in_env), so a bad
            // tempenv locale warns only in the binding classes. The
            // class is known from the command word up front; defer the
            // locale attempts until just after the apply, then flush
            // (binding class — GNU warns at bind time, before the
            // command runs) or drop (disk class).
            let tempenv_binds = cmd.words.first().is_some_and(|word| {
                is_shell_builtin_name(word) || self.function_name_for_command_word(word).is_some()
            });
            self.defer_locale_assignment_warnings.set(true);
            let (applied, assignments_failed) =
                self.apply_temporary_assignments(&cmd.assignments, assignment_raws);
            self.defer_locale_assignment_warnings.set(false);
            if tempenv_binds {
                self.flush_deferred_locale_assignment_binds();
            } else {
                self.drop_deferred_locale_assignment_binds();
            }
            // POSIX-mode notes item 36, GNU 5.3 verified cell-by-cell with
            // unbuffered per-process probes (the early "every command class
            // exits" reading was a stdout pipe-buffering artifact — GNU's
            // jump-exit does not flush it): a failed readonly tempenv
            // assignment before a SPECIAL builtin exits the shell (status 1,
            // `ro=2 :` — no further statement runs), while before an
            // ordinary builtin, external or function the command is SKIPPED
            // and the rest of the command list is abandoned with status 1
            // (ksh93 emulation — GNU errors7.sub: `x=8 echo` runs nothing,
            // `after non-special builtin: 1`; subshell forms exit the
            // subshell with 1 and the parent continues). ExpansionFailure(1)
            // is exactly that skip-and-abandon contract; the default mode
            // keeps the historical apply-and-run.
            if assignments_failed && self.posix_assignment_failure_is_fatal() {
                self.exit_code = 1;
                let special = cmd
                    .words
                    .first()
                    .is_some_and(|word| is_posix_special_builtin(word));
                // Restore the partial tempenv binds before unwinding: the
                // skip-and-abandon path keeps the shell running, and an
                // unbalanced tempenv mark would corrupt every later restore.
                self.restore_temporary_assignments(applied);
                if special || self.evalerror_pending.get() {
                    // GNU subst.c:13186-13195 (the tempenv arm):
                    // bash_variable_assignment_error(is_special_builtin)
                    // jumps FORCE_EOF for a special builtin
                    // (subst.c:11169-11180); shell.c:1471-1472
                    // run_one_command remaps the jump to 127 under -c while
                    // script mode exits 1 (eval.c:104-109) and subshell
                    // children contain it at 1 (execute_cmd.c:1811).
                    // A DISCARD-class expansion error (evalerror pending)
                    // is expand_wdesc_fatal under posixly_correct
                    // (subst.c:10881-10888): FORCE_EOF for every command
                    // class, special or not (rubash#1226).
                    let code = self.expansion_fatal_status();
                    self.exit_code = code;
                    return Err(ExecuteError::ExitCode(code));
                }
                return Err(ExecuteError::ExpansionFailure(1));
            }
            // rubash#1226: a DISCARD-class expansion error raised inside the
            // prefix expansion (arith evalerror, subscript trap) armed the
            // reader-level abort — GNU's jump fires inside expand_words
            // (subst.c:13101-13105), so the command is SKIPPED in every
            // mode and the rest of the list is abandoned. The failing
            // bind never landed; restore the partial tempenv first.
            if assignments_failed && self.evalerror_pending.get() {
                self.exit_code = 1;
                self.restore_temporary_assignments(applied);
                return Err(ExecuteError::ExpansionFailure(1));
            }
            applied
        };
        // rubash#1342: a builtin pipeline element re-dispatched here by
        // execute_builtin_pipeline_stage was already traced once by the
        // pipeline driver; consume that mark so the element prints
        // exactly one line, like GNU's forked execute_simple_command.
        let stage_pre_traced = self.pipeline_stage_pre_traced.replace(false);
        if self.xtrace_enabled() && cmd.arithmetic_command.is_none() && !stage_pre_traced {
            // GNU dispatches `(( ))` to execute_arith_command, whose own
            // xtrace (execute_cmd.c:3940) prints the raw between-parens text;
            // the generic simple-command trace (execute_cmd.c:4480) never
            // fires for it. rubash's words ["((", expr, "))"] would print a
            // second, normalized line (issue: gnu-compat set-x G16).
            let prefix = self.xtrace_prefix();
            let mut xtrace_output = Vec::new();
            for line in self.xtrace_command_lines(cmd) {
                writeln!(xtrace_output, "{prefix}{line}").ok();
            }
            // GNU bash emits xtrace after applying the command's redirects
            // (execute_cmd.c:4480+), so the trace goes to the redirected
            // stderr. Rubash emits xtrace before redirect application, so
            // check redirect_err_append for an inherited 2>&1 (fd-reference
            // target) and route the trace to that fd's endpoint. This
            // preserves the `2>&1` semantics for nested same-shell scripts.
            if let Some(redirect) = &cmd.redirect_err_append {
                let target = self.expand_redirect_target(redirect);
                if self.has_output_fd_target(&target) {
                    let _ = self.write_output_fd_redirect(&target, &xtrace_output);
                } else {
                    self.xtrace_write(&xtrace_output);
                }
            } else {
                self.xtrace_write(&xtrace_output);
            }
        }

        // GNU execute_cmd.c:4480 resets special_builtin_failed before each
        // simple command; builtins that return EX_USAGE/EX_UTILERROR/etc.
        // (> EX_SHERRBASE) set it during dispatch.
        self.special_builtin_failed.set(false);
        // rubash#1525: record whether the dispatch reaches
        // execute_external. GNU's completion write
        // (set_pipestatus_from_exit, execute_cmd.c:4904) targets the
        // cell visible when the command returns: for a builtin the
        // tempenv binding is still installed (variables.c
        // push_temp_var surrounds the call), so a tempenv PIPESTATUS
        // absorbs the write; for a disk command the tempenv was
        // consumed into the child's environment
        // (maybe_make_export_env), so the write lands in the real
        // cell after the tempenv restore. The marker is saved and
        // restored around the dispatch so a nested command's class
        // cannot leak into this command's verdict.
        let saved_external_marker = self.external_dispatch_ran.replace(false);
        let result = if redirect_rejected {
            Ok(())
        } else {
            self.execute_prepared_command(cmd)
        };
        let external_ran = self.external_dispatch_ran.get();
        self.external_dispatch_ran.set(saved_external_marker);
        // rubash#1525: with a tempenv PIPESTATUS and an in-process
        // dispatch, perform the completion write NOW, against the
        // still-bound tempenv cell (scalar: frozen; array: absorbed
        // into the tempenv vector, discarded with it at restore),
        // and mark it spent so the ast_exec loop does not write a
        // second time against the restored cell.
        let pipestatus_write_absorbed = !external_ran
            && temporary_assignments
                .iter()
                .any(|(name, ..)| name == crate::shell::state::PIPESTATUS_STATE_SNAPSHOT_KEY);
        if pipestatus_write_absorbed {
            let status = self.exit_code;
            self.set_pipestatus([status]);
        }
        self.finish_process_substitutions(process_substitution_files)?;
        self.finish_assignment_output_process_substitutions_for_command(cmd)?;
        if cmd.background && result.is_ok() {
            self.shell_state.last_background_pid = Some(std::process::id());
            self.exit_code = 0;
        }
        // rubash#1567: the saved pre-tempenv entries for the merge in
        // merge_converted_tempenv_bindings (declare.def:1045-1083
        // absorption / posix merge rebind through the underlying
        // variable). Built before the restore below consumes the
        // vector; only kept commands can convert a binding.
        let tempenv_saved: std::collections::HashMap<
            String,
            (
                Option<String>,
                Option<crate::shell::Variable>,
                Option<VarAttrs>,
            ),
        > = if keep_temporary_assignments {
            temporary_assignments
                .iter()
                .map(|(name, env, typed, attrs)| {
                    (name.clone(), (env.clone(), typed.clone(), *attrs))
                })
                .collect()
        } else {
            std::collections::HashMap::new()
        };
        if !keep_temporary_assignments {
            self.restore_temporary_assignments(temporary_assignments);
        } else if Self::attribute_builtin_absorbs_named_tempenv(cmd) {
            // GNU setattr.def:590-614 set_var_attribute ("only called by
            // readonly/export") + declare.def:1105: an attribute builtin
            // absorbs from the temporary environment ONLY the variables its
            // operands name — `FOO=val export BAR` leaves FOO unset while
            // `FOO=val export FOO` persists; every other tempenv binding
            // dies with the temporary environment (rubash#538). `export -f`
            // operands name functions, not variables: nothing absorbs.
            let keep = self.attribute_builtin_operand_names(cmd);
            // GNU's absorption inherits the tempvar's attributes
            // (setattr.def:604 `var->attributes |= tv->attributes &
            // ~att_tempvar`) — assign_in_env marks every tempenv binding
            // exported, so an ABSORBED assignment name keeps att_exported.
            // The __RUBASH_EXPORTED_VARS marker snapshot therefore must not
            // be restored (it predates the tempenv's export marks); the
            // absorbed names are re-marked after the restore instead. Every
            // other real variable entry not named by an operand is restored.
            let absorbed: std::collections::HashSet<String> = cmd
                .assignments
                .iter()
                .filter_map(|(name, _)| {
                    let base = assignment_name_and_append(name).0.to_string();
                    keep.contains(&base).then_some(base)
                })
                .collect();
            let survivors: Vec<_> = temporary_assignments
                .into_iter()
                .filter(|(name, ..)| {
                    name != "__RUBASH_EXPORTED_VARS"
                        && !keep.contains(name)
                        // rubash#1525: an absorbed PIPESTATUS keeps
                        // its tempenv shape — the (shape, vector)
                        // snapshot must be dropped with its cell
                        // entry, not restored over the live binding.
                        && !(name == crate::shell::state::PIPESTATUS_STATE_SNAPSHOT_KEY
                            && keep.contains("PIPESTATUS"))
                })
                .collect();
            if keep.contains("PIPESTATUS") {
                self.tempenv_pipestatus_state = None;
            }
            self.restore_temporary_assignments(survivors);
            for name in &absorbed {
                self.mark_exported(name);
            }
            // GNU declare.def:1045-1083: the absorbed tempvar is
            // rebound through the underlying variable, so a conversion
            // attribute there recomputes the window value and the
            // surviving attribute word is saved | tempvar's — merge
            // the shadow back (rubash#1567).
            self.merge_converted_tempenv_bindings(&absorbed, &tempenv_saved);
            // rubash#1110 item 2: at compat <= 44 the absorbed tempvar is
            // converted in the GLOBAL context even inside a function (the
            // pre-4.4 set_var_attribute behavior bash 4.4 gated behind
            // compat44): `v=inner export v` on a frame local promotes the
            // binding out of the frame — later writes in the function go
            // to the global, and the frame pop restores nothing for the
            // name. Drop the name from the nearest local scope snapshot
            // so restore_function_locals leaves the live binding in place.
            if crate::builtins::shopt::compatibility_level(&self.shell_state.env_vars) <= 44
                && self.shell_state.function_depth > 0
            {
                for name in &absorbed {
                    if let Some(index) = self.visible_local_scope_index(name) {
                        self.shell_state.local_var_scopes[index].remove(name);
                        if let Some(scope) = self.shell_state.local_attr_scopes.get_mut(index) {
                            scope.remove(name);
                        }
                        if let Some(scope) = self.shell_state.local_typed_scopes.get_mut(index) {
                            scope.remove(name);
                        }
                    }
                }
            }
        } else if self.posix_mode_enabled() && self.shell_state.function_depth > 0 {
            // GNU execute_cmd.c:4892-4894 → variables.c:4662
            // merge_temporary_env → variables.c:4485 push_posix_temp_var:
            // a posix special builtin's tempenv binds at the current
            // variable context; a non-local context binding (context>0 &&
            // !local_p) gains att_propagate and descends into the caller's
            // context when the frame pops — surviving the function-call
            // tempenv restore (`var=inner export var` inside `var=func f`).
            // A binding that lands on a local dies with its frame instead
            // (func2's `var=global :` on `local var`).
            // The merge binds through bind_variable (variables.c:4501),
            // so conversion attributes recompute the value before the
            // propagation capture below reads the cell (rubash#1567).
            let merge_names: std::collections::HashSet<String> = cmd
                .assignments
                .iter()
                .map(|(name, _)| assignment_name_and_append(name).0.to_string())
                .collect();
            self.merge_converted_tempenv_bindings(&merge_names, &tempenv_saved);
            for (name, _) in &cmd.assignments {
                let (base, _) = assignment_name_and_append(name);
                let is_local = self
                    .shell_state
                    .local_var_scopes
                    .iter()
                    .any(|scope| scope.contains_key(base));
                let is_function_tempenv = self
                    .function_tempenv_names
                    .iter()
                    .any(|frame| frame.iter().any(|n| n == base));
                if !is_local
                    && is_function_tempenv
                    && !self
                        .tempenv_propagated_names
                        .iter()
                        .any(|(propagated, _)| propagated == base)
                {
                    self.tempenv_propagated_names.push((
                        base.to_string(),
                        capture_var_attrs(&self.shell_state.env_vars, base),
                    ));
                }
            }
        } else {
            // Remaining keep forms (posix special builtin at top level,
            // the eval append keep): the bindings stay as they lie, but
            // a conversion attribute on an underlying variable still
            // rebinds the window value in GNU (merge_temporary_env ->
            // bind_variable), so merge those names back (rubash#1567).
            let merge_names: std::collections::HashSet<String> = cmd
                .assignments
                .iter()
                .map(|(name, _)| assignment_name_and_append(name).0.to_string())
                .collect();
            self.merge_converted_tempenv_bindings(&merge_names, &tempenv_saved);
        }
        // rubash#1525: the tempenv completion write above is this
        // command's one PIPESTATUS write; tell the ast_exec loop
        // not to write again against the restored cell.
        if pipestatus_write_absorbed {
            self.pipestatus_write_absorbed.set(true);
        }
        self.update_underscore_parameter(cmd);
        // GNU execute_cmd.c:1004-1017: in POSIX mode, a non-interactive shell
        // exits when a special builtin returned an error status (> EX_SHERRBASE).
        // execute_cmd.c:4657-4666: `builtin_is_special` is only set on the
        // dispatch path where `(cmdflags & CMD_NO_FUNCTIONS) == 0' — the
        // `command' builtin's re-dispatched argument (check_command_builtin
        // sets CMD_NO_FUNCTIONS, execute_cmd.c:4723) never qualifies, so a
        // special builtin executed as `command return ...' cannot mark the
        // failure ("we don't want to exit the shell if a special builtin
        // executed with `command builtin' fails"). The depth gate is the
        // same state GNU tracks as executing_command_builtin
        // (execute_cmd.c:4731-4735; errors8.sub ok 5/ok 6).
        // GNU order (rubash#344): the ERR trap for a failing simple command
        // fires inside execute_simple_command (run_error_trap callers,
        // execute_cmd.c:1000/1166) — DEEPER than the ERREXIT jump in
        // execute_command_internal (execute_cmd.c:624) — so `set -e; trap
        // 'echo E' ERR; false` prints E and exits 1. This dispatch-level
        // errexit conversion is the shell's jump site; fire the trap here
        // when it is about to kill the command, before converting. The
        // non-errexit path keeps firing at the ast_exec loop sites (a fire
        // here would double it once the Ok result reaches them), and
        // maybe_run_error_trap's own gates (inverted / &&-|| context /
        // errtrace function depth) match the errexit conditions.
        let outcome = if result.is_ok()
            && self.special_builtin_failed.get()
            && self.command_builtin_depth == 0
            && self.posix_mode_enabled()
            && self
                .shell_state
                .env_vars
                .get("__RUBASH_INTERACTIVE")
                .map(String::as_str)
                != Some("1")
        {
            // rubash#1611: inside a timed dispatch the ERR trap is
            // adjudicated once, by the ast_exec time branch, when the
            // timed command completes (rubash#1277) — firing here too
            // would double it.
            if self.timed_dispatch_depth.get() == 0 {
                self.maybe_run_error_trap(cmd)?;
            }
            // rubash#1611: this is a conversion of a COMPLETED command's
            // status, not a jump out of it — mark it so an enclosing
            // execute_time_ast_command can still print its report (GNU
            // adjudicates this exit in execute_time_command's caller,
            // after print_time_info).
            self.errexit_status_converted.set(true);
            Err(ExecuteError::ExitCode(self.exit_code))
        } else if matches!(result, Err(ExecuteError::Return(_)))
            && (self.shell_state.function_depth > 0 || self.source_nest_depth > 0)
        {
            // rubash#720 form 4: a `return N` unwinding through dispatch is
            // the builtin's own completion, not an errexit trigger at THIS
            // gate. GNU's return_builtin (return.def:57-76) resolves N via
            // get_exitstat (builtins/common.c) and sh_longjmps the
            // return_catch (:71-72) — only the invalid-argument arm errors
            // out (:67-69 posix / :73-75 outside a function). An in-range
            // return unwinds via execute_function's return_catch, the
            // function boundary stamps last_command_exit_value = N, and the
            // CALL's own gate then adjudicates. Firing here would attribute
            // the loop condition's stale status (`until false` leaves
            // exit_code = 1 from the failing test) to the return and lose N:
            // GNU prints ERR:7 and exits 7 for `until false; do f; done`
            // with `f(){ return 7; }`, not ERR:1 / exit 1. The unwind passes
            // through to the function (or source) boundary untouched.
            result
        } else if self.errexit_enabled() && self.errexit_is_active() && self.exit_code != 0 {
            // rubash#1611: inside a timed dispatch the ERR trap is
            // adjudicated once, by the ast_exec time branch, when the
            // timed command completes (rubash#1277) — firing here too
            // would double it.
            if self.timed_dispatch_depth.get() == 0 {
                self.maybe_run_error_trap(cmd)?;
            }
            // rubash#1611: mark ONLY the conversion of an Ok completion —
            // an Err already in flight (the exit builtin's own unwind)
            // stays unmarked: it takes the print-and-propagate arm in
            // execute_time_ast_command instead of the completion flow.
            if result.is_ok() {
                self.errexit_status_converted.set(true);
            }
            // DE-06 (design 9.3; PLAN row 12): this conversion is
            // the errexit decision — a selected `set -e` filter
            // reports the site and parks the session before the
            // exit path runs. Inside a timed dispatch the death
            // is decided later, by the ast_exec time branch,
            // which parks at its own promotion instead. Only the
            // frame that ORIGINATES the conversion parks: an Err
            // already in flight was adjudicated at the deeper
            // frame where its command completed, and the frames
            // it unwinds through (a sourced file's `.` call, a
            // function call) re-enter this arm with the same
            // status — parking there too would report one death
            // as two exception stops.
            let site_result = if self.timed_dispatch_depth.get() == 0 && result.is_ok() {
                self.debugger_note_exception_site(
                    crate::debugger::ExceptionKind::Errexit,
                    self.exit_code,
                    None,
                )
            } else {
                Ok(())
            };
            match site_result {
                Ok(()) => Err(ExecuteError::ExitCode(self.exit_code)),
                Err(error) => Err(error),
            }
        } else {
            result
        };
        self.assignment_command_name = previous_command_name;
        outcome
    }

    fn execute_prepared_command(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        let Some(word) = cmd.words.first() else {
            return Ok(());
        };
        // rubash#815 profiler: the simple-command dispatch point (GNU
        // execute_cmd.c:624 execute_command_internal). The frame is armed as
        // TENTATIVE here — the real dispatcher classifies it: a function or
        // external dispatch cancels the frame (the classifier is the
        // dispatch that actually happened, no second admission list,
        // rubash#117), and any other return confirms it as a builtin on
        // drop. Off path: one relaxed AtomicBool load.
        let _profiler_command = super::profiler::enter_prepared_command(word);
        // GNU execute_cmd.c:4750-4757: a simple command whose first word
        // starts with `%` is a job spec — dispatch to fg (bg when async)
        // before builtin/function/PATH lookup, with the `%` word itself as
        // the jobspec operand (fg_bg.def get_job_spec sees it in word[0]).
        // The fg builtin itself reports "no job control" when the monitor
        // option is off.
        if word.starts_with('%') {
            let mut fg_cmd = cmd.clone();
            fg_cmd.words.insert(0, "fg".to_string());
            self.exit_code =
                self.execute_fg_bg(&fg_cmd, crate::builtins::fg_bg::JobControlBuiltin::Fg)?;
            return Ok(());
        }
        if let Some(message) = self.restricted_command_error(cmd, word) {
            self.write_default_stderr(message.as_bytes())?;
            self.exit_code = 1;
            return Ok(());
        }
        if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, word) {
            return self.execute_external(cmd);
        }
        if let Some(result) = self.execute_primary_builtin_command(cmd, word)? {
            return result;
        }
        self.execute_late_builtin_command(cmd, word)
    }
}

impl Executor {
    /// GNU declare.def:1045-1083 + variables.c:4662 merge_temporary_env:
    /// when a tempenv binding is CONVERTED into a real binding — an
    /// attribute builtin absorbing its named operands (declare -x /
    /// export / readonly on a tempvar), or a posix-mode keep where the
    /// tempenv merges into the context — the window value is rebound
    /// through the UNDERLYING variable (bind_variable runs
    /// make_variable_value), so a conversion attribute on that variable
    /// (integer/case) recomputes the value and the surviving attribute
    /// word is saved | tempvar's. The tempenv shadow (rubash#1567,
    /// #1556) keeps the window value verbatim with fresh exported-only
    /// marks, so a converted name whose saved entry carries a
    /// conversion attribute must be merged back here, or
    /// `declare -i s=5; s=007 declare -x s` would persist
    /// `declare -x s="007"` instead of GNU's `declare -ix s="7"`.
    /// Array underlyings keep the shadow outcome (bind_variable's
    /// element-0 rule for an absorbed array tempvar is a separate
    /// surface); names without a saved binding (fresh tempvars)
    /// persist as the shadow they are.
    pub(in crate::executor) fn merge_converted_tempenv_bindings(
        &mut self,
        names: &std::collections::HashSet<String>,
        saved: &std::collections::HashMap<
            String,
            (
                Option<String>,
                Option<crate::shell::Variable>,
                Option<VarAttrs>,
            ),
        >,
    ) {
        for name in names {
            let Some((saved_env, saved_typed, saved_attrs)) = saved.get(name) else {
                continue;
            };
            let Some(saved_attrs) = saved_attrs else {
                continue;
            };
            if saved_env.is_none() && saved_typed.is_none() {
                continue;
            }
            if !(saved_attrs.integer || saved_attrs.uppercase || saved_attrs.lowercase) {
                continue;
            }
            if saved_attrs.array || saved_attrs.assoc {
                continue;
            }
            let Some(window_value) = self.shell_state.env_vars.get(name).cloned() else {
                continue;
            };
            if matches!(
                self.shell_state.variables.get(name).map(|v| &v.value),
                Some(
                    crate::shell::ShellValue::IndexedArray(_)
                        | crate::shell::ShellValue::AssociativeArray(_)
                )
            ) {
                continue;
            }
            let merged_value = if saved_attrs.integer {
                match self.eval_integer_assignment_checked(&window_value) {
                    Some(result) => result.to_string(),
                    None => {
                        // bind_variable failed (evalerror already
                        // reported): GNU leaves the underlying variable
                        // untouched — reinstall the saved binding
                        // wholesale instead of merging.
                        match saved_env {
                            Some(value) => {
                                self.shell_state
                                    .env_vars
                                    .insert(name.clone(), value.clone());
                                set_process_env(name, value);
                            }
                            None => {
                                self.shell_state.env_vars.remove(name);
                                crate::executor::local_helpers::unset_process_env(name);
                            }
                        }
                        self.restore_typed_temporary_value(name, saved_typed.clone());
                        set_var_attrs(&mut self.shell_state.env_vars, name, *saved_attrs);
                        continue;
                    }
                }
            } else if saved_attrs.uppercase {
                window_value.to_uppercase()
            } else {
                window_value.to_lowercase()
            };
            let current_attrs = capture_var_attrs(&self.shell_state.env_vars, name);
            let mut merged_attrs = *saved_attrs;
            merged_attrs.exported = saved_attrs.exported || current_attrs.exported;
            merged_attrs.readonly = saved_attrs.readonly || current_attrs.readonly;
            merged_attrs.declared_unset = false;
            self.shell_state
                .env_vars
                .insert(name.clone(), merged_value.clone());
            let _ = self.shell_state.variables.set(
                name.clone(),
                crate::shell::Variable::scalar(merged_value.clone()),
            );
            set_var_attrs(&mut self.shell_state.env_vars, name, merged_attrs);
            sync_shell_assignment_process_env(&self.shell_state.env_vars, name, &merged_value);
            // rubash#1692: the merge reinstalls the binding — GNU's
            // sv_locale watcher runs on the reinstall.
            self.note_locale_assignment(name);
        }
    }
}
