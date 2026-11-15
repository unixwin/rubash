use super::*;

/// Outcome of a builtin's pre-help prologue (rubash#3340):
/// `Proceed` continues to the long help (any prologue diagnostics
/// already sit in the gate's stderr sink); `Abort(status)` means
/// the prologue refused the builtin outright — emit its diagnostic
/// with this status and print no help.
enum PreHelpOutcome {
    Proceed,
    Abort(i32),
}

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
        // rubash#2718: a command with NO command word binds its
        // assignments permanently as part of word expansion, BEFORE
        // execute_null_command runs do_redirections over the redirect
        // list (execute_cmd.c:4626-4635 + :4268-4286) — so the
        // redirect words expand against the NEW bindings
        // (`a=newname > ${a}.txt` creates newname.txt). A command WITH
        // a word is the opposite shape: its assignments are tempenv and
        // redir.c:298 expands the redirect words with the temp
        // environment ignored. Apply the standalone bindings here,
        // ahead of the reject gate, so the gate's open probes and the
        // redirect-target memo it seeds both see the post-binding
        // values; the named-command path below is untouched and keeps
        // the pre-binding expansion.
        let assignment_raws_pre = (cmd.assignment_raws.len() == cmd.assignments.len())
            .then(|| cmd.assignment_raws.as_slice());
        if standalone_assignments {
            let applied = self.apply_permanent_assignments(&cmd.assignments, assignment_raws_pre);
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
        }
        // GNU redir.c:298 redirection_expand uses expand_words_no_vars:
        // redirect words expand with the temp environment IGNORED
        // (`a=9 echo >&$(echo $a)` sees the outer a, not 9). Expanding here —
        // before apply_temporary_assignments — seeds the redirect-target memo
        // with the correct pre-binding values; the apply sites reuse them.
        // rubash#2121: classify the apply funnel so the reject gate
        // settles superseded net output entries only where the funnel
        // itself would lose them.
        let settle_superseded_net_outputs =
            self.superseded_net_output_settlement_applies(cmd, false);
        let redirect_rejection =
            self.reject_invalid_redirects(cmd, settle_superseded_net_outputs)?;
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
            // rubash#2718: the standalone bindings were already applied
            // above, ahead of the reject gate (GNU binds a null
            // command's assignments during word expansion, before
            // do_redirections expands the redirect words). The failure
            // adjudication (subst.c:13177-13182's DISCARD jump, the
            // posix FORCE_EOF) rode that early apply.
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
                self.apply_temporary_assignments(&cmd.assignments, assignment_raws, &cmd.words);
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
        if redirect_rejected {
            // rubash#2279: the funnel's speculative `{var}` apply
            // (apply_dynamic_fd_var_redirects in execute_command_with_
            // procsub_scope) is paired with an undo owned by the
            // dispatch class: execute_external undoes for disk commands,
            // builtins/functions keep the binding (GNU redir_varassign
            // persists in the shell's own context — it survives even a
            // later redirect's failure, verified against GNU 5.3.20).
            // A rejected DISK command never reaches execute_external,
            // so without this the allocation and the variable leak into
            // the parent (`/bin/true {v}<seed < /missing` left v set);
            // GNU's varassign for a disk command happens only in the
            // forked child (execute_cmd.c do_redirections after fork)
            // and the child dies with it — rejected or not.
            let in_process = cmd.words.first().is_some_and(|word| {
                is_shell_builtin_name(word) || self.function_name_for_command_word(word).is_some()
            });
            if !in_process {
                self.undo_child_fd_var_redirects();
            }
        }
        // rubash#2280: for an in-process dispatch (builtin/function —
        // the same classification as the rejected arm above), perform
        // the command's recorded {var} closes in the table for the
        // duration of the dispatch; the snapshots are replayed right
        // after it, below. An external dispatch leaves the records
        // for its delivery (rubash#2281). A builtin name that turns
        // out external after all (disabled builtin) is safe too: the
        // delivery then finds the records consumed but the table
        // already closed, and execute_external's own undo cannot
        // see these local snapshots — the replay below restores
        // them exactly once.
        let in_process_dispatch = !redirect_rejected
            && cmd.words.first().is_some_and(|word| {
                is_shell_builtin_name(word) || self.function_name_for_command_word(word).is_some()
            });
        let in_process_close_snapshots =
            self.apply_fd_var_closes_for_in_process(in_process_dispatch);
        // rubash#2692: set when the builtin redirect pregate below
        // refuses the dispatch; adjudicated beside the #2082 gate
        // conversion further down.
        let mut builtin_pregate_failed = false;
        let result = if redirect_rejected {
            Ok(())
        } else {
            // rubash#2124: GNU redir.c do_redirections opens a winning
            // /dev/tcp|udp input redirect before the body runs, for
            // in-process classes too — the funnel's bind performs the
            // one connect and binds the socket for the dispatch; a
            // failed connect was already diagnosed there (status 1)
            // and skips the dispatch like a gate rejection.
            match self.bind_simple_command_net_inputs(cmd) {
                Ok(saved_net_binds) => {
                    // rubash#2692: GNU execute_builtin_or_function runs
                    // do_redirections (execute_cmd.c:5606-5614, RX_UNDOABLE)
                    // BEFORE the builtin body: a failed open undoes the
                    // partial redirects and returns EX_REDIRFAIL — the
                    // builtin never runs, so a state-mutating builtin
                    // (export/readonly/declare/typeset/local/umask/alias/
                    // trap/shopt/ulimit/set) leaves no trace when any of
                    // its redirections fails to open. rubash's per-arm
                    // redirect application gated only the arms that route
                    // through the no-output or buffered-output helpers;
                    // the mutating arms applied their effects first and
                    // surfaced the failure afterwards. This funnel is the
                    // one point every in-process builtin dispatch passes
                    // (including `command`/`builtin` re-dispatch, whose
                    // redirections sit on the outer command, and pipeline
                    // stage re-dispatch), so the ordered open walk — the
                    // same one the no-output builtins and `read` use —
                    // pregates here. Classification: a builtin command
                    // word only. Functions own their redirect scope (and
                    // already gate), externals gate in their child setup,
                    // and a builtin name that resolves external after all
                    // (disabled builtin) observes the same net effect
                    // GNU's child-side gate produces. The walk opens in
                    // list order and stops at the first failure, exactly
                    // like do_redirections; its opens are the idempotent
                    // probes the serving arms repeat (targets memoized,
                    // FIFO rendezvous carved out), and its diagnostic is
                    // routed through the bindings in effect at the
                    // failing entry. The pregate form of the walk
                    // undoes its own file creations on success —
                    // the dispatch applies the list again, and a
                    // probe-created file would corrupt that second
                    // application (see pregate_redirect_open_check).
                    let pregate_failed = cmd
                        .words
                        .first()
                        .is_some_and(|word| is_shell_builtin_name(word))
                        && self.pregate_redirect_open_check(cmd)?;
                    let result = if pregate_failed {
                        builtin_pregate_failed = true;
                        Ok(())
                    } else {
                        self.execute_prepared_command(cmd)
                    };
                    self.restore_simple_command_net_inputs(saved_net_binds);
                    result
                }
                Err(()) => Ok(()),
            }
        };
        // rubash#2280: the in-process {var} closes end with the
        // dispatch — restore the parent's bindings from the
        // snapshots taken above.
        if !in_process_close_snapshots.is_empty() {
            self.restore_fd_var_close_snapshots(in_process_close_snapshots);
        }
        // rubash#2476: fold a buffered-write failure the dispatch
        // left unconsumed into this command's status. GNU's output
        // builtins return sh_chkwrite's verdict (builtins/common.c:
        // 319): when flushing the builtin's stdout fails, the
        // builtin returns EXECUTION_FAILURE whatever its payload
        // logic computed. rubash's write machinery records the
        // failure on last_builtin_write_failed and prints the
        // diagnostic; the echo/printf/pwd dispatches fold the flag
        // themselves, but the listing dispatches (declare/typeset,
        // alias, set, export -p, umask, ...) never did, so their
        // status stayed 0 next to a correct diagnostic — in-session
        // (`declare -p x >&-`) and in a child born with fd 1 closed
        // alike. This funnel is the one completion point every
        // simple command passes, so the fold lives here instead of
        // in each dispatch: lift a success status to 1 when the
        // flag is set. A status the command already failed with is
        // never rewritten — a usage-error path writes no stdout
        // payload and never sets the flag. Compound commands are
        // excluded: their inner commands complete through this
        // same funnel one by one and fold there.
        if !super::pipeline_exec::command_is_compound_pipeline_stage(cmd)
            && self.take_builtin_write_failed()
            && self.exit_code == 0
        {
            self.exit_code = 1;
        }
        // rubash#2174: GNU flushes its stdio stdout after EVERY builtin
        // or function executed in the current shell context
        // (execute_cmd.c:5648 execute_builtin_or_function:
        // fflush (stdout), done "before undoing the effects of any
        // redirections" — hence before the process-substitution
        // teardown below). rubash's stdio analog is the Rust std
        // stdout handle's line buffer: builtin blocks routed to the
        // real process stdout stay pending there when they carry no
        // newline, while foreign writers on the shared fd (external
        // commands, pipeline stages, procsub consumers, background
        // children) write straight to the fd and overtake them
        // (`printf 'A'; /usr/bin/printf 'B'` printed BAC). This funnel
        // is the one completion point every simple command passes —
        // builtins, functions, externals, and pipeline-stage
        // re-dispatches alike — so flushing here keeps the buffer
        // empty at every command boundary by construction. It is the
        // RAW std handle flush, not an fd-table endpoint write: GNU's
        // `printf 'A'; exec >file` leaves the pre-redirect A on the
        // OLD stdout, which is exactly where the raw handle points.
        // Cost: an empty-buffer no-op check when nothing is pending;
        // after an external dispatch the buffer is empty by this same
        // invariant, so spawn-heavy paths pay no extra syscall.
        super::shell_options::flush_process_stdout();
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
        } else if Self::attribute_builtin_absorbs_named_tempenv(cmd)
            && !(self.posix_mode_enabled()
                && cmd
                    .words
                    .first()
                    .is_some_and(|name| super::support_names::is_posix_special_builtin(name)))
        {
            // The absorption arm is skipped for a POSIX-mode SPECIAL
            // builtin (export/readonly are both): GNU's setattr.def
            // absorption is the non-posix mechanism, while in POSIX mode
            // the prefix assignments of a special builtin bind in the
            // current context and persist (variables.c
            // push_posix_temp_var / merge_temporary_env; POSIX 2.9.1 —
            // `v=zz readonly` leaves v=zz). Those commands fall through
            // to the posix merge arms below, which keep the bindings and
            // handle function-frame propagation (audit B4-F10).
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
            // rubash#2720: an absorbed dynamic name converts to the
            // real dynamic bind (the hook runs on the kept value).
            self.convert_kept_tempenv_dynamic_binds(&absorbed);
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
            // rubash#2720: a kept dynamic name converts to the real
            // dynamic bind (the hook runs on the kept value).
            self.convert_kept_tempenv_dynamic_binds(&merge_names);
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
        // The :1004 check is itself gated on `ignore_return == 0 &&
        // invert == 0' (and the ERR-trap call at :993 shares that gate):
        // a special builtin failing inside a tested context — an if/while
        // condition, a non-final &&/|| member, a `!' inversion, or a
        // function body reached from such a call — reports its status and
        // execution continues. rubash's with_errexit_suppressed marks
        // exactly those contexts (the executor's CMD_IGNORE_RETURN model:
        // conditions, non-final list members and inverted commands are
        // dispatched under it, and it stays raised across a tested
        // function call), and the errexit branch at the bottom of this
        // function already consults the same predicate, so the fatality
        // channel reads it too. (RC audit phase-1 finding ④ / errors10:
        // before this, failures already wired into the channel — the
        // `set' family — died inside conditions where GNU continues.)
        //
        // rubash#2082: GNU's single do_redirections verdict reaches
        // rubash through two channels. The apply layer's failures
        // surface as IoError and are converted in execute_command's
        // arm; the reject gate's RedirectRejection::Command — the
        // dup / input-open / ambiguous class, redir.c:255-260's first
        // failing entry — is adjudicated here instead: for a special
        // builtin in a posixly_correct non-interactive shell it is the
        // same EX_REDIRFAIL conversion (execute_cmd.c:4865-4875), an
        // exit at EXECUTION_FAILURE that precedes the outcome chain
        // because GNU's jump is not gated on ignore_return. The gate
        // already rendered the diagnostic and set status 1; the
        // FatalNounset rejection keeps its own adjudication above and
        // is not this class.
        if redirect_rejected
            && matches!(
                redirect_rejection,
                Some(super::redirection::RedirectRejection::Command)
            )
            && self.posix_special_redirect_failure_exits(cmd)
        {
            return Err(ExecuteError::ExitCode(1));
        }
        // rubash#2692: the builtin pregate's refusal is the same
        // EX_REDIRFAIL verdict arriving through the third channel —
        // execute_builtin_or_function's do_redirections failure
        // (execute_cmd.c:5606-5614) converted by execute_simple_command
        // (execute_cmd.c:4865-4875): for a special builtin in a
        // posixly_correct non-interactive shell the shell exits at
        // EXECUTION_FAILURE. The pregate already rendered the
        // diagnostic and set status 1.
        if builtin_pregate_failed && self.posix_special_redirect_failure_exits(cmd) {
            return Err(ExecuteError::ExitCode(1));
        }
        let outcome = if result.is_ok()
            && self.special_builtin_failed.get()
            && self.command_builtin_depth == 0
            && self.errexit_is_active()
            && self.posix_mode_enabled()
            && !self.invocation_interactive()
        {
            // rubash#2117: GNU's order in execute_command_internal is
            // the ERR-trap call (execute_cmd.c:993-1001) BEFORE the
            // special_builtin_failed check (:1004-1017), and every
            // simple command resets the flag at entry
            // (execute_cmd.c:4480) — so the trap body's own commands
            // clear the flag the failing builtin set, and the check
            // reads the CLEARED flag: a benign ERR trap rescues the
            // shell from the POSIX exit, while a trap body that itself
            // ends in a failing special builtin re-arms the flag and
            // the exit still fires. Capture the completed command's
            // status first: GNU's exit carries exec_result
            // (last_command_exit_value = exec_result, :1010), not the
            // trap body's last status.
            let completed_status = self.exit_code;
            // rubash#1611: inside a timed dispatch the ERR trap is
            // adjudicated once, by the ast_exec time branch, when the
            // timed command completes (rubash#1277) — firing here too
            // would double it.
            if self.timed_dispatch_depth.get() == 0 {
                self.maybe_run_error_trap(cmd)?;
            }
            if self.special_builtin_failed.get() {
                // rubash#1611: this is a conversion of a COMPLETED command's
                // status, not a jump out of it — mark it so an enclosing
                // execute_time_ast_command can still print its report (GNU
                // adjudicates this exit in execute_time_command's caller,
                // after print_time_info).
                self.errexit_status_converted.set(true);
                // rubash#2117: this conversion is the POSIX
                // special-builtin death, not an errexit one — the time
                // branch's jump is gated on posix fatality, not `set -e`.
                self.posix_special_status_converted.set(true);
                // GAP-1: this jump is the fatal class (the
                // classification's ExitJump family, like
                // FatalFunctionError), so it deliberately carries
                // no errexit mark. Drain any standing mark
                // defensively: the mark's lifetime invariant (one
                // writer, `errexit_exit`; consumed by
                // run_source_impl and the time wrapper) already
                // guarantees none can stand here — the command
                // completed, so no errexit unwind is in flight —
                // and the drain keeps this jump's kind correct by
                // construction rather than by that argument alone
                // (Q6 arm B approximation (ii), resolved).
                let _ = self.take_errexit_unwind_pending();
                Err(ExecuteError::ExitCode(completed_status))
            } else {
                // Rescued by the ERR trap: the trap already fired for
                // this command at this dispatch — mark it so the list
                // loop's own ERR adjudication does not fire it again
                // (rubash#2117).
                self.err_trap_fired_at_dispatch.set(true);
                result
            }
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
        } else if self.errexit_enabled()
            && self.errexit_is_active()
            && self.exit_code != 0
            && !self.evalerror_arith_pending.get()
        {
            // rubash#3287: the arithmetic evalerror class is exempt
            // here exactly as at the list loop's promotion
            // (ast_exec.rs, rubash#1518): expr.c's evalerror jumps
            // DISCARD directly, bypassing the errexit epilogue in
            // every context — a fatal nested expansion inside a
            // `(( ))` discards the command and the script continues,
            // it does not exit under `set -e`. This funnel is the
            // loop promotion's twin for the materialized dispatch
            // (a `(( ))` carrying `$` in its words takes the generic
            // path through here, never the bare fast entry), and
            // without the exemption its ExpansionFailure was
            // converted to ExitCode(1) before any list arm could
            // absorb it. Other expansion failures (bad substitution,
            // failglob) set no such pending mark and keep exiting
            // under `set -e`, per the 2026-09-02 probe.
            // rubash#1611: inside a timed dispatch the ERR trap is
            // adjudicated once, by the ast_exec time branch, when the
            // timed command completes (rubash#1277) — firing here too
            // would double it.
            // rubash#2482 (t133 residual): fire only for a COMPLETED
            // command. GNU's run_error_trap call sites (execute_cmd.c:
            // 773/1000/1166/2909) only ever adjudicate a command that
            // returned a status; the `exit` builtin never returns —
            // builtins/exit.def ends in jump_to_top_level (EXITBLTIN),
            // so no frame the unwind crosses performs failure
            // adjudication for the exit-terminated chain (probes:
            // `set -eET; trap 'echo E' ERR; exit 1` and an EXIT action
            // whose function chain ends in `exit` print no E in GNU).
            // An in-flight Err here is exactly such an unwind — the
            // exit builtin's own result, or an inner frame's
            // already-adjudicated conversion arriving through an
            // eval/source wrapper (`eval 'false'` fired twice: once
            // at the inner completion, once again at this frame) —
            // and it propagates below unchanged, the same rule the
            // conversion mark two statements down already applies.
            if self.timed_dispatch_depth.get() == 0 && result.is_ok() {
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
                // Q6 arm B: only the genuine conversion (an Ok
                // completion turned into the errexit unwind) is
                // marked; an Err already in flight (the exit
                // builtin's own unwind crossing this frame)
                // keeps its kind, exactly as the conversion mark
                // above already discriminates.
                Ok(()) if result.is_ok() => Err(self.errexit_exit(self.exit_code)),
                Ok(()) => Err(ExecuteError::ExitCode(self.exit_code)),
                Err(error) => Err(error),
            }
        } else {
            result
        };
        self.assignment_command_name = previous_command_name;
        outcome
    }

    /// rubash#2753: the shared `--help` gate — GNU's CHECK_HELPOPT /
    /// CASE_HELPOPT (builtins/common.h:29-41), applied once here in
    /// the simple-command funnel instead of once per builtin parser.
    ///
    /// Placement is load-bearing and mirrors GNU's order in
    /// execute_cmd.c: functions have already been dispatched by the
    /// caller (a function named like a builtin shadows it, help word
    /// and all), a restricted refusal and the `enable -n` external
    /// fall-through above have already won, and a loadable
    /// (`enable -f`) builtin — which GNU's find_shell_builtin also
    /// prefers over the static table — has already run. What remains
    /// is a static builtin call, and for that call
    /// `help_opt_requested` replays the exact scan GNU's
    /// internal_getopt / CHECK_HELPOPT would have run inside the
    /// builtin: on a help word the long help goes to the command's
    /// stdout through the same buffered-output path every other
    /// builtin uses (so redirections and command substitution see
    /// it), the status is EX_USAGE (user-visible 2), and a SPECIAL
    /// builtin additionally takes the special_builtin_failed mark
    /// its own EX_USAGE return would have produced
    /// (execute_cmd.c:4886-4888; the loop-control arms set the same
    /// mark by hand for their identical first-word path).
    ///
    /// Handlers that already run the (row + BuiltinOpt + Diag)
    /// triple reach the same verdict through their own scan — the
    /// gate simply arrives first with the same text and status, so
    /// the two layers cannot disagree; handlers with no scan at all
    /// (the #2753 divergent set: cd, kill, trap, read, fc, bind,
    /// history, getopts, mapfile, pwd, the complete family, ...)
    /// are covered by this one site. The `--help` dispatch itself
    /// (including the pre-option-scan prologue replay, rubash#3340)
    /// lives in `apply_builtin_help` below, shared with the
    /// `builtin` re-dispatch.
    fn builtin_help_gate(&mut self, cmd: &CommandNode, word: &str) -> Result<bool, ExecuteError> {
        let Some(row) = super::builtin_table::builtin_table_row(word) else {
            return Ok(false);
        };
        if !super::builtin_opt::help_opt_requested(row, &cmd.words[1..]) {
            return Ok(false);
        }
        self.apply_builtin_help(cmd, row)?;
        Ok(true)
    }

    /// Apply a `--help` dispatch for `row` under `cmd`: replay the
    /// builtin's pre-option-scan prologue first, then either emit
    /// the prologue's fatal diagnostic with its status and no help,
    /// or the long help (status 2) with the prologue's stderr ahead
    /// of it. The caller returns without running the handler either
    /// way. Shared by the funnel gate above and the `builtin`
    /// re-dispatch (builtin_direct_command.rs), which never
    /// re-enters the funnel — the trap pipeline-stage site
    /// (pipeline_exec.rs) is trap-only and trap has no prologue,
    /// so it keeps its own inline decision.
    ///
    /// rubash#3340: GNU runs each builtin's prologue BEFORE its
    /// option scan (bind.def:122-131 warning + initialize_readline
    /// before CASE_HELPOPT at :191; hash.def:86-90 hashing-disabled
    /// refusal before the getopt loop), so a `--help` dispatch must
    /// replay the prologue before the long help — the gate used to
    /// return before the handler was ever entered and swallowed it.
    /// The prologue is dispatched on the row's handler identity
    /// (not the name) and runs exactly once on this path: the gate
    /// returns before the handler is entered, so the handler's own
    /// prologue never runs twice. A refusing prologue (hash with
    /// hashing disabled) aborts the dispatch: its diagnostic is
    /// emitted with status 1 and no help is printed. The survey
    /// in tests/issue3340_help_gate_leading_diagnostics.rs read
    /// every other builtin's `.def` prologue — only bind and hash
    /// have observable code before the help check.
    pub(in crate::executor) fn apply_builtin_help(
        &mut self,
        cmd: &CommandNode,
        row: &super::builtin_table::BuiltinRow,
    ) -> Result<(), ExecuteError> {
        let mut prologue_stderr = Vec::new();
        if let PreHelpOutcome::Abort(status) =
            self.run_pre_help_prologue(row, &mut prologue_stderr)?
        {
            self.write_buffered_builtin_output(cmd, &[], &prologue_stderr)?;
            self.exit_code = status;
            return Ok(());
        }
        let mut stdout = Vec::new();
        crate::builtins::help::print_builtin_help(row.name, &mut stdout)?;
        // GNU emits a builtin's diagnostics unbuffered while its
        // output flushes at builtin exit (the #1134 printf rule),
        // so the prologue's stderr precedes the help text on a
        // merged stream (`bind --help 2>&1` shows the warning
        // first). The ordered-chunk log below is the mechanism that
        // preserves that stderr-first order (union #3399 + #3385).
        let log = super::trap_stack_builtins::OrderedOutputLog::new();
        {
            use std::io::Write;
            let _ = log.stderr_writer().write_all(&prologue_stderr);
            let _ = log.stdout_writer().write_all(&stdout);
        }
        self.write_buffered_builtin_output_chunks(cmd, &log)?;
        self.exit_code = 2;
        if row
            .flags
            .contains(super::builtin_table::BuiltinFlags::SPECIAL)
        {
            self.special_builtin_failed.set(true);
        }
        Ok(())
    }

    /// Run a builtin's pre-option-scan prologue for a `--help`
    /// dispatch (rubash#3340); see `builtin_help_gate`.
    fn run_pre_help_prologue(
        &mut self,
        row: &super::builtin_table::BuiltinRow,
        stderr: &mut Vec<u8>,
    ) -> Result<PreHelpOutcome, ExecuteError> {
        use super::builtin_table::HandlerSlot;
        match row.handler {
            HandlerSlot::Bind => {
                let prefix = self.diagnostic_prefix();
                self.bind_pre_help_prologue(self.invocation_interactive(), &prefix, stderr);
                Ok(PreHelpOutcome::Proceed)
            }
            HandlerSlot::Hash => {
                let mut diag = super::builtin_diag::Diag::new(row, stderr);
                if crate::builtins::hash::check_hashing_enabled(
                    &self.shell_state.env_vars,
                    &mut diag,
                ) {
                    Ok(PreHelpOutcome::Proceed)
                } else {
                    diag.flush_with(|| self.diagnostic_prolog());
                    Ok(PreHelpOutcome::Abort(1))
                }
            }
            _ => Ok(PreHelpOutcome::Proceed),
        }
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
            // rubash#2397: execute_cmd.c:4753-4755 rolls this/last
            // with the fg builtin's address for the jobspec dispatch.
            self.note_builtin_dispatch(crate::shell::BuiltinDispatchClass::Other);
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
        // rubash#2126: this funnel is where a simple command commits to
        // dispatch (execute_cmd.c:4813's this_shell_builtin assignment
        // point) — a restricted refusal above never dispatches, so the
        // note belongs after it. The classification itself (eval /
        // source / `.` arm, everything else disarms, disabled names
        // resolve externally) lives in note_simple_command_dispatch.
        self.note_simple_command_dispatch(word);
        if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, word) {
            return self.execute_external(cmd);
        }
        // GNU find_shell_builtin sees dynamically loaded entries in
        // shell_builtins (enable.def:449-466), so a loadable builtin
        // shadows the compiled table exactly like a static one.
        if self.execute_dynamic_builtin_command(cmd, word)? {
            return Ok(());
        }
        if self.builtin_help_gate(cmd, word)? {
            return Ok(());
        }
        if let Some(result) = self.execute_primary_builtin_command(cmd, word)? {
            return result;
        }
        self.execute_late_builtin_command(cmd, word)
    }

    /// Run NAME if it was installed by `enable -f`.  The plugin receives
    /// GNU's `WORD_LIST` of arguments (no command word) and returns the
    /// builtin status directly (`sh_builtin_func_t`, general.h:240).
    pub(in crate::executor) fn execute_dynamic_builtin_command(
        &mut self,
        cmd: &CommandNode,
        word: &str,
    ) -> Result<bool, ExecuteError> {
        if !crate::builtins::dynamic::is_loaded(word) {
            return Ok(false);
        }
        // rubash#2397: execute_cmd.c:4810-4813 roll, same as the static
        // primary/late slots.
        self.note_builtin_dispatch(Self::builtin_dispatch_class(word));
        // GNU runs the builtin's redirections before its function and lets
        // it write to those descriptors.  The plugin writes through libc,
        // so its fd 1/2 bytes are captured and replayed through the same
        // buffered router every in-process builtin uses (redirects,
        // command substitution, pipeline stages).
        // execute_cmd.c:5097 passes `words->next` — arguments only.
        let args = cmd.words.get(1..).unwrap_or(&[]);
        let Some(output) = crate::builtins::dynamic::call_captured(word, args) else {
            self.exit_code = 1;
            return Ok(true);
        };
        // execute_cmd.c:4881 EX_DISKFALLBACK: the builtin declined and
        // asks the shell to run the disk command of the same name.
        if output.status == 262 {
            self.execute_external(cmd)?;
            return Ok(true);
        }
        self.exit_code = crate::builtins::dynamic::normalize_builtin_status(output.status);
        self.write_buffered_builtin_output(cmd, &output.stdout, &output.stderr)?;
        Ok(true)
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
