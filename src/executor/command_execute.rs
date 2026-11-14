use super::*;
use crate::executor::markers::{PARSE_ERROR_FIELD_SEP, STORAGE_WORD_PREFIX};

impl Executor {
    /// Execute an AST
    pub fn execute_command(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        // rubash#2652: a command dispatched outside suppression owns
        // the terminal status from here — clear any stale
        // suppressed-terminal record from an earlier command in the
        // unit, so it cannot exempt this command's own failure (the
        // `false || false` shape: the first `false` records the
        // exemption, the last `false` must clear it before its own
        // gate fires). A command dispatched under suppression keeps
        // the record: its `with_errexit_suppressed` wrapper
        // re-records it from the outcome. A pass-through compound
        // clears here too, then re-records inside its body if its
        // terminal failure was suppressed.
        if self.suppress_errexit == 0 {
            self.errexit_suppressed_terminal.set(false);
        }
        // rubash#545 (cell 2): the command owns every input-procsub fd its
        // words materialize — expansion registers them into the scoped vec
        // and the close below runs at the command's very end, giving the
        // `/dev/fd/N` word form GNU's per-command lifetime (GNU closes a
        // command's process-substitution fds when the command finishes;
        // `a=(<(echo p9)); cat "${a[0]}"` then reports `No such file`).
        let procsub_fd_base = self.command_scoped_procsub_input_fds.len();
        // rubash#1646: watermark for this command's varredir_close
        // records — drained at the command's end below, when GNU's
        // undo_redirections would close them.
        let varredir_close_base = self.deferred_varredir_closes.len();
        let result = self.execute_command_with_procsub_scope(cmd, procsub_fd_base);
        self.close_command_scoped_procsub_input_fds_from(procsub_fd_base);
        self.drain_deferred_varredir_closes(varredir_close_base);
        // rubash#3107: the dynamic associative variables' owner tables
        // (aliases, hashed_filenames) mutate without a mirror write, so
        // a materialized typed cell goes stale at this boundary;
        // reconcile it from the owner before the verifier below (and
        // before any later read trusts the cell). See
        // Executor::sync_dynamic_assoc_cells.
        self.sync_dynamic_assoc_cells();
        // Debug-only systemic regression net for the typed-cell ↔ mirror
        // invariant: after every command boundary every fresh (undirty)
        // structured array cell must still equal its mirror projection.
        // Cells carry a per-name freshness proof (rubash#1510) so an
        // untouched array is not re-proven element by element at every
        // boundary. Release builds compile this out.
        #[cfg(debug_assertions)]
        self.verify_typed_cells();
        result
    }

    fn execute_command_with_procsub_scope(
        &mut self,
        cmd: &CommandNode,
        procsub_fd_base: usize,
    ) -> Result<(), ExecuteError> {
        let _ = procsub_fd_base;
        use super::exec_profile::{ensure_init, PhaseTimer, P_COUNT, P_TOTAL};
        ensure_init();
        let _t_total = PhaseTimer::new(&P_TOTAL);
        if super::exec_profile::P_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            P_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        // GNU redir.c do_redirections expands each redirect word once per
        // command execution; drop the previous command's target memo so a
        // re-executed node (loop body, function body) expands fresh.
        self.redirect_target_memo.borrow_mut().clear();
        self.redirect_target_single_glob_match.borrow_mut().clear();
        self.redirect_target_transport_spellings
            .borrow_mut()
            .clear();
        self.redirect_target_node_spellings.borrow_mut().clear();
        self.assignment_expansion_memo.borrow_mut().clear();
        // Commit any deferred arithmetic writes queued by `&self` redirect
        // target expansion during the previous command (GNU redirection_expand
        // evaluates in the live environment; the queue exists only because
        // `&self` expansions cannot write env_vars directly).
        self.apply_pending_subscript_writes();
        // Set the source line for every command, including commands inside a
        // DEBUG trap function. The trap action expands its call-site `$LINENO`
        // before entering that function, while the function body must see its
        // own source line (dbg-support2.tests).
        {
            let _t = PhaseTimer::new(&super::exec_profile::P_LINECMD);
            // #682 stamped-by-caller contract: the ast_exec loop stamps this
            // node's line pair immediately before the funnel call, and the
            // funnel's own stamp recomputes the identical value from the same
            // node and the unchanged ambient (GNU stamps line_number ONCE per
            // command — execute_cmd.c SET_LINE_NUMBER; rubash's reader-loop
            // stamp is that stamp, the funnel copy is redundant). Skip only
            // that copy: the flag is consumed on entry (nested execute_command
            // callers always stamp), and it is not honored when the trap-scope
            // branch below would stamp debug_trap_function_line instead — an
            // in-action command's funnel stamp differs from the loop's plain
            // stamp. A DEBUG fire / debugger seam between the loop stamp and
            // the funnel suppresses the flag at the setter (ast_exec), so the
            // post-trap restamp (GNU's :936 behavior after the action) runs.
            let pre_stamped = self.pre_stamped_line.replace(false);
            let trap_line_scope = self.debug_trap_running && self.shell_state.function_depth > 0;
            if !pre_stamped || trap_line_scope {
                if trap_line_scope {
                    if let Some(line) = self.debug_trap_function_line {
                        self.set_current_line_value(line);
                    } else {
                        self.set_current_line(cmd);
                    }
                } else {
                    self.set_current_line(cmd);
                }
            }
            self.set_current_command(cmd);
        }
        // GNU execute_cmd.c:826-828 (execute_command_internal): a shell
        // control structure carrying redirections records in the global
        // stdin_redir whether fd 0 is among them; the flag is sticky for the
        // structure's whole dynamic extent and eval.c:181 resets it per
        // reader command. Async `cmd &` consults it (execute_cmd.c:2837).
        if command_is_shell_control_structure(cmd) && !cmd.redirects.is_empty() {
            self.shell_state
                .stdin_redir
                .set(cmd.redirects.iter().any(redirect_updates_stdin_redir));
        }
        let _t_heredoc = PhaseTimer::new(&super::exec_profile::P_HEREDOC);
        self.report_command_heredoc_errors(cmd)?;
        if let Some((name, message, status)) = self.parameter_heredoc_expansion_error(cmd) {
            let line = format!("{}{}: {}\n", self.diagnostic_prefix(), name, message);
            self.write_default_stderr(line.as_bytes())?;
            if let Err(error) = self.finish_heredoc_expansion_error(cmd, status) {
                return Err(error);
            }
            return Ok(());
        }
        drop(_t_heredoc);
        if self.command_parse_diagnostics(cmd)? {
            return Ok(());
        }

        // unixwin/niubash#481: the host REPL's bare-drive normalizer runs
        // only on its fast path; the grouped reader (`D:; pwd`), the
        // engine's own piped `-i` stdin driver (niubash#483), and nested
        // contexts the host AST pass does not descend into all reach this
        // dispatch with `D:` still a bare command word. Rewrite it to the
        // same `cd D:/` node the host pass emits — one semantic owner at
        // the funnel every route shares.
        if let Some(drive_root) = windows_drive::bare_windows_drive_command_root(cmd) {
            return self.execute_bare_windows_drive_command(cmd, drive_root);
        }

        let _t_dispatch = PhaseTimer::new(&super::exec_profile::P_DISPATCH);
        if let Some(result) = self.execute_initial_command_node(cmd) {
            // Compound commands run through execute_initial_command_node and
            // bypass the errexit check in execute_materialized_command. A
            // failing subshell must still honor `set -e`: `(exit 17)` exits
            // the script (set-e1.sub). &&/||/! contexts already suppressed
            // errexit at the ast_exec call site.
            if cmd.subshell_command.is_some()
                && result.is_ok()
                && self.errexit_enabled()
                && self.errexit_is_active()
                && self.exit_code != 0
            {
                // rubash#720: GNU's user-subshell wait path adjudicates the
                // ERR trap BEFORE the errexit jump — execute_cmd.c:769-778
                // runs run_error_trap and :777-795 then jumps ERREXIT (that
                // site runs no ERR fire of its own), so `set -E; set -e;
                // trap 'echo E' ERR; (false)` prints E and exits 1. Under
                // the rubash#720 fork-collapse this frame performs no
                // wait-side adjudication at all (execute_cmd.c:662-663
                // skips wait_for) — the failure is observed one frame up.
                // rubash#1611: the timed-dispatch depth gate is the
                // same single-adjudication rule as in
                // execute_materialized_command's tail.
                // rubash#2482 face b: a pipeline element's root `( )`
                // gets no wait-side adjudication either — the depth
                // marker the stage installed names this frame (a
                // nested `( )` inside the body sits deeper, its
                // subshell_depth restored above the marker, and keeps
                // its fire).
                let element_root = self.subshell_element_root_depth.get()
                    == Some(self.shell_state.subshell_depth.get());
                if !self.subshell_fork_collapse.get()
                    && self.timed_dispatch_depth.get() == 0
                    && !element_root
                {
                    self.maybe_run_error_trap(cmd)?;
                }
                // rubash#1611: conversion of a completed subshell's
                // status, not a jump out of it — mark for
                // execute_time_ast_command's report adjudication.
                self.errexit_status_converted.set(true);
                // DE-06 (design 9.3; PLAN row 12): the errexit
                // decision — park a selected `set -e` filter here,
                // before the exit path. The gate matches the ERR
                // fire above exactly: under fork collapse the
                // death is adjudicated at the region close (whose
                // promotion parks, after the wait-side ERR fire),
                // and a timed death is decided by the time branch.
                if !self.subshell_fork_collapse.get()
                    && self.timed_dispatch_depth.get() == 0
                    && !element_root
                {
                    self.debugger_note_exception_site(
                        crate::debugger::ExceptionKind::Errexit,
                        self.exit_code,
                        None,
                    )?;
                }
                return Err(self.errexit_exit(self.exit_code));
            }
            // rubash#2659: GNU execute_cmd.c:854-883 — a compound
            // command's OWN do_redirections failure is an ordinary
            // command failure: run_error_trap under was_error_trap,
            // then the ERREXIT jump (the simple-command twin sits at
            // execute_cmd.c:993-1010). The compound wrappers propagate
            // that failure as an IoError with the
            // compound_redirect_failed mark set (prepare pass /
            // with_command_input_redirects); unconverted, it bubbles
            // to the ast_exec loop, whose IoError arm only reports it
            // and whose Ok-path adjudication treats these compounds'
            // status as body-produced — no ERR trap, no errexit.
            // Convert it here, at the funnel that dispatched the
            // compound: report the diagnostic once, record status 1,
            // fire the ERR trap, and take the errexit exit when
            // errexit is active. The fire call's own gates keep
            // inverted, and-or and suppressed contexts exempt, and
            // its fired-at-dispatch record keeps a caller's own look
            // (the loop's, for the pass-through forms) from firing a
            // second time. Unmarked IoErrors are body-stage failures
            // and keep the passthrough below.
            // rubash#1611 + PR #3269 review R1: inside a timed
            // dispatch this conversion must NOT intercept at all.
            // err_trap_fired_at_dispatch means "the ERR trap already
            // fired at dispatch" (the #2117 suppression gate in
            // maybe_run_error_trap) — setting it without firing, and
            // converting the failure into an Ok completion, blocks
            // the ast_exec time branch's own adjudication look, so
            // the timed compound's redirect failure lost its ERR
            // trap entirely (the timed branch of this conversion did
            // exactly that when first written). Let the marked
            // IoError propagate in its unconverted shape instead:
            // the time branch reports it once (report_io_error_routed
            // records status 1) and adjudicates ERR / errexit
            // against the final status, exactly as the pre-#2659
            // base did.
            if self.timed_dispatch_depth.get() == 0 && self.compound_redirect_failed.get() {
                if let Err(ExecuteError::IoError(error)) = &result {
                    self.report_io_error_routed(cmd, error)?;
                    self.exit_code = 1;
                    self.maybe_run_error_trap(cmd)?;
                    if self.errexit_enabled() && self.errexit_is_active() {
                        return Err(self.errexit_exit(1));
                    }
                    return Ok(());
                }
            }
            return result;
        }

        // rubash#1805: parse.y's timespec production is recursive
        // (pipeline_command: timespec pipeline_command), so a
        // TimeCommand wrapper can sit under another timed frame and
        // reach this funnel directly through the outer frame's inner
        // dispatch, without passing the execute_ast list loop that
        // normally dispatches timed commands. Execute it here — the
        // funnel every route shares — instead of letting the
        // empty-words arm below mistake the wrapper (which carries no
        // words of its own) for a null command and silently drop the
        // whole timed command.
        if let Some(time_command) = &cmd.time_command {
            return self.execute_time_ast_command(time_command);
        }

        if cmd.words.is_empty() {
            let result = self.execute_empty_words_command(cmd, None);
            // GNU execute_cmd.c:4625-4640: with WORDS==0 the null command
            // path binds `$_` to the null string (4638 bind_lastarg((char
            // *)NULL)) after the command (including its assignment
            // expansions) runs — in the current shell, so `x=$(: sub)`
            // leaves `$_`="" in the parent.
            if result.is_ok() {
                self.bind_underscore("");
            }
            return result;
        }

        if let Some((name, message, status)) = self.parameter_heredoc_expansion_error(cmd) {
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "{}{}: {}",
                self.diagnostic_prefix(),
                name,
                message
            )?;
            self.write_default_stderr(&stderr)?;
            if let Err(error) = self.finish_heredoc_expansion_error(cmd, status) {
                return Err(error);
            }
            return Ok(());
        }

        self.validate_command_parameter_expansions(cmd)?;

        if self.execute_parser_level_alias(cmd)? {
            return Ok(());
        }

        // GNU subst.c:12494-12535 (expand_word_list) with `set -k`
        // (flags.c `place_keywords_in_env`): once the leading assignment run
        // is split off, every remaining `name=value` word is moved onto the
        // assignment list, so it reaches the temporary environment instead of
        // the command's argument list. When the harvest leaves no command word
        // behind the command is assignment-only and the assignments persist in
        // the shell (varenv.tests lines 42-49 and 70-88: `set -k` then
        // `a=5 b=6 $CHMOD c=7 $MODE d=8 $FN e=9`).
        let keep_temporary_cmd = self.command_with_keep_temporary_assignments(cmd);
        let cmd = match keep_temporary_cmd {
            Some(ref materialized) => materialized,
            None => cmd,
        };

        // GNU execute_cmd.c execute_arith_command: the `(( ... ))`
        // expression is expanded once (expand_arith_string, parameter and
        // command substitutions only — no word splitting/globbing) and the
        // expanded text is handed to evalexp. Rubash's evaluator performs
        // that expansion itself on the raw captured expression
        // (eval_arithmetic_command_value), so running the generic word
        // expansion on cmd.words here would expand `$RANDOM`/`$(...)`
        // TWICE — the first draws discarded, the second evaluated
        // (arith3.sub `(( dice[$RANDOM...]++ ))` lost its dice[6]/dice[7]
        // hits to doubled draws).
        let mut expanded = if cmd.arithmetic_command.is_some() {
            cmd.clone()
        } else {
            match self.expand_command_words(cmd) {
                Ok(expanded) => expanded,
                // GNU expr.c expr_streval: an unbound variable under `set -u`
                // raises FORCE_EOF from ANY expansion position — the word
                // expansion's DISCARD classification (ExpansionFailure) must
                // not downgrade it — EXCEPT in the interactive reader, where
                // expr.c:1208-1216 takes `jump_to_top_level (DISCARD)` (the
                // command is abandoned with status 1 and the session keeps
                // reading). `$((missing+1))` as a command argument abandons
                // the command list and exits the noninteractive shell (127
                // under `-c` via shell.c:1471, 1 in script mode; probe
                // 2026-09-27).
                Err(ExecuteError::ExpansionFailure(_))
                    if self.shell_state.arithmetic_nounset_error.replace(false) =>
                {
                    self.shell_state.arithmetic_expansion_error.set(false);
                    if self.expansion_error_is_interactive_discard() {
                        self.note_unit_expansion_error_discard();
                        self.exit_code = 1;
                        return Err(ExecuteError::ExpansionFailure(1));
                    }
                    let code = self.expansion_fatal_status();
                    self.exit_code = code;
                    return Err(ExecuteError::ExitCode(code));
                }
                Err(error) => return Err(error),
            }
        };
        // Same FORCE_EOF mapping when word expansion completed without an
        // error result but latched the nounset flag. The `(( ))` command
        // form keeps its own check in command_dispatch_late.
        if self.shell_state.arithmetic_nounset_error.replace(false) {
            self.shell_state.arithmetic_expansion_error.set(false);
            if self.expansion_error_is_interactive_discard() {
                self.note_unit_expansion_error_discard();
                self.exit_code = 1;
                return Err(ExecuteError::ExpansionFailure(1));
            }
            let code = self.expansion_fatal_status();
            self.exit_code = code;
            return Err(ExecuteError::ExitCode(code));
        }
        if let Some(code) = self.current_shell_substitution_exit.take() {
            // A `${ ...; exit N; }` body aborts the enclosing (sub)shell with
            // N (GNU subst.c nofork exit propagation; comsub26.sub line 32).
            self.exit_code = code;
            return Err(ExecuteError::ExitCode(code));
        }
        if self.last_command_substitution_status.get() == Some(2)
            && self.last_command_substitution_parse_error.get()
        {
            // A BACKTICK body's parse error killed only the substituted
            // child (GNU subst.c string_locale -> parse_and_execute: the
            // child reports `command substitution: line N: ...' at its own
            // parse and exits 2). The substitution is empty, status 2
            // becomes the command's status (an assignment keeps it in $?,
            // GNU `x=\`case x in\`` -> $?=2), and the script continues —
            // no `syntax error in command substitution' escalation, which
            // is the read-time `$(`-body failure shape (issue #515).
            if self
                .last_command_substitution_parse_error_backtick
                .replace(false)
            {
                self.last_command_substitution_parse_error.set(false);
            } else {
                self.mark_parse_error();
                // rubash#2751: when the body site already printed the
                // diagnostic in GNU's read-time shape, that report is
                // the whole story — GNU has no second escalation line,
                // at any nesting depth. The latch paths that never
                // reached a reporting site keep the escalation.
                if self.last_command_substitution_error_reported.replace(false) {
                    self.comsub_error_reported_out.set(true);
                } else {
                    eprintln!(
                        "{}syntax error in command substitution",
                        self.parser_error_prefix()
                    );
                }
                // rubash#2787: the command's exit status stays the
                // substitution's 2 only for contained routes; the
                // abort out of a top-level `-c` string carries the
                // run_one_command FORCE_EOF remap to 127
                // (shell.c:1471-1472) via comsub_parse_error_abort_status.
                let abort_status = self.comsub_parse_error_abort_status();
                self.exit_code = abort_status;
                self.last_command_substitution_status.set(None);
                return Err(ExecuteError::ExitCode(abort_status));
            }
        }
        let original_words_had_command_substitution = cmd
            .word_metadata
            .iter()
            .any(|metadata| metadata.raw.contains("$(") || metadata.raw.contains('`'));
        // perf17: GNU consults the alias table itself (parse.y:3249
        // alias_expand_token -> find_alias, a hash lookup) — with an empty
        // table no word expands, so "did alias expansion change the words"
        // is answered by that one emptiness test. The deep Vec<String>
        // clone of every command's expanded words (and the raw-word Vec
        // that feeds the expansion) existed only to answer the question
        // after the fact; with an empty table
        // apply_alias_expansion_after_word_expansion is an identity move
        // (its own aliases.is_empty() fast path), so the comparison is
        // constant-false and the clone is dead weight.
        let aliases_empty = self.shell_state.aliases.is_empty();
        let original_raws: Vec<Option<&str>> = if aliases_empty {
            Vec::new()
        } else {
            cmd.word_metadata
                .iter()
                .map(|metadata| Some(metadata.raw.as_str()))
                .collect()
        };
        let pre_alias_words = (!aliases_empty).then(|| expanded.words.clone());
        let alias_expanded =
            self.apply_alias_expansion_after_word_expansion(expanded, &original_raws);
        let alias_expansion_changed_words = match pre_alias_words {
            Some(pre) => alias_expanded.words != pre,
            None => false,
        };
        if alias_expanded.words.is_empty() && !self.shell_state.arithmetic_expansion_error.get() {
            // GNU execute_simple_command (execute_cmd.c): a simple command
            // whose words all expand to nothing is still executed as a null
            // command - its redirections run and create/truncate their
            // targets with status 0 (redir.tests: `$EXIT > $TMPDIR/file`
            // must create the file, and `exit 3 | $EXIT > file` reports
            // status 0 for the null side, not command-not-found).
            if !cmd.assignments.is_empty()
                || command_has_redirect(&cmd)
                || !cmd.redirects.is_empty()
                || cmd.heredoc.is_some()
                || cmd.here_string.is_some()
            {
                // rubash#1661: the vanished words' substitution status
                // is this null command's status seed (consumed here,
                // where the bare arm below reads the same channel). The
                // channel is only attributable when this command's own
                // words carried a substitution — otherwise it may hold
                // a previous command's leftover, and the seed stays
                // None (the pre-#1661 behavior for those shapes).
                let vanished_status = if original_words_had_command_substitution {
                    self.last_command_substitution_status.take()
                } else {
                    None
                };
                // rubash#2761: GNU's words==0 check
                // (execute_cmd.c:4626-4640) runs on the EXPANDED word
                // list, which is empty here — but `cmd` still carries
                // the raw words, and the null-command path decides
                // on them (the heredoc fork classifier reads the
                // effective word). Hand it the node as GNU sees it:
                // wordless, so `$EMPTY 3<<EOF` classifies under
                // execute_null_command's force-fork rule (no fd-0
                // redirect -> the body expands in the shell and
                // `:=` persists) instead of as an external
                // command's forked expansion.
                let mut vanished_cmd = cmd.clone();
                vanished_cmd.words.clear();
                return self.execute_empty_words_command(&vanished_cmd, vanished_status);
            }
            if let Some(status) = self.last_command_substitution_status.get() {
                self.exit_code = status;
                self.last_command_substitution_status.set(None);
            } else {
                // GNU execute_cmd.c:4625-4640 execute_simple_command: with
                // WORDS == 0 the command dispatches to execute_null_command
                // (execute_cmd.c:4203), which performs no command lookup and
                // returns 0 — the status does NOT keep the previous
                // command's value (`false; "${arr[@]}"` is status 0,
                // rubash#134). Only a command substitution inside the
                // vanished words can still impose its own status.
                self.exit_code = 0;
            }
            // GNU execute_cmd.c:4638 bind_lastarg((char *)NULL): a null
            // command binds `$_` to the null string.
            self.bind_underscore("");
            if self.errexit_enabled() && self.errexit_is_active() && self.exit_code != 0 {
                // rubash#1661: GNU adjudicates the ERR trap for a failed
                // simple command before the ERREXIT jump (execute_cmd.c
                // run_error_trap ahead of the errexit exit) — a null
                // command carrying a failed substitution's status is a
                // failed simple command, so `set -e; trap 'echo T' ERR;
                // $(false)` prints T and exits 1. The assignment/
                // redirect forms convert at execute_empty_words_command's
                // tail (rubash#1171); this bare arm converts here.
                // maybe_run_error_trap's own gates keep the exempt
                // shapes (inverted, and-or, suppressed) silent.
                // rubash#1611: inside a timed dispatch the fire is
                // deferred to the ast_exec time branch, which performs
                // the single trap adjudication for the completed timed
                // command — an unguarded fire here double-fired
                // `time $(false)` (ERRFIRED x2, GNU x1). The conversion
                // mark below is what lets execute_time_ast_command
                // treat this ExitCode as a completion, exactly like
                // the dispatch-level conversion sites.
                if self.timed_dispatch_depth.get() == 0 {
                    self.maybe_run_error_trap(cmd)?;
                }
                // rubash#1611: conversion of a completed command's
                // status — mark for execute_time_ast_command.
                self.errexit_status_converted.set(true);
                // DE-06 (design 9.3; PLAN row 12): the errexit
                // decision — park a selected `set -e` filter here,
                // before the exit path (timed deaths park at the
                // time branch's promotion instead).
                if self.timed_dispatch_depth.get() == 0 {
                    self.debugger_note_exception_site(
                        crate::debugger::ExceptionKind::Errexit,
                        self.exit_code,
                        None,
                    )?;
                }
                return Err(self.errexit_exit(self.exit_code));
            }
            return Ok(());
        }
        let mut cmd = alias_expanded;
        self.abort_on_expansion_errors()?;
        // GNU redir.c do_redirections: here-document bodies and the
        // here-string word are expanded after the command words and before
        // the command runs; an expansion error there aborts the command
        // with the same expand_word_error classification. Expand once here
        // and mark the results so the stdin paths return them verbatim
        // instead of re-running embedded substitutions.
        self.preexpand_command_stdin(&mut cmd);
        self.abort_on_expansion_errors()?;

        if alias_expansion_changed_words
            && !original_words_had_command_substitution
            && self.execute_alias_expanded_syntax(&cmd)?
        {
            return Ok(());
        }
        // Unquoted command substitutions can disappear during word
        // expansion. A command that started as `name=$(...)` may therefore
        // become assignment-only and must still apply the assignment and its
        // redirections.
        if cmd.words.is_empty() {
            // rubash#1661: same attributable-seed rule as the
            // expansion-time vanished-words arm above — this site is
            // reached by commands that started as `name=$(...)` and
            // became assignment-only, whose substitution status word
            // expansion left in the channel.
            let vanished_status = if original_words_had_command_substitution {
                self.last_command_substitution_status.take()
            } else {
                None
            };
            return self.execute_empty_words_command(&cmd, vanished_status);
        }

        // GNU redir.c do_redirections → redir_varassign (redir.c:1133-1166):
        // a `{var}` redirection allocates a fresh descriptor and assigns
        // its number to var for every simple command — builtins, functions
        // and external commands alike — before the command runs. Apply
        // them once here, ahead of dispatch; a failed redirect aborts the
        // command (do_redirections returns on the first error). `exec`
        // owns its fd_var redirects itself (execute_stdio_only_exec_redirect
        // applies them in list order with persistent semantics), so it is
        // excluded here.
        let fd_var_walk_undoable = self.redirect_walk_is_undoable(&cmd);
        if cmd.words.first().map(String::as_str) != Some("exec")
            && self.apply_dynamic_fd_var_redirects(&cmd, true, fd_var_walk_undoable)?
        {
            // rubash#2125: the applier rendered the diagnostic and set
            // status 1, but the failure never becomes an IoError at the
            // #2082 conversion arm below — apply_dynamic_fd_var_redirects
            // swallows it into its bool. GNU reaches the same failure
            // through redir_varassign inside do_redirections, whose
            // EX_REDIRFAIL return (execute_cmd.c:5613) takes the
            // execute_cmd.c:4865-4875 jump for a special builtin in a
            // posixly_correct non-interactive shell: convert here, at
            // the applier's call site, under the #2082 gate. The jump
            // is an Err, so tested contexts do not contain it and the
            // ERR trap never runs — exactly the plain-redirect channel.
            if self.posix_special_redirect_failure_exits(&cmd) {
                return Err(ExecuteError::ExitCode(1));
            }
            return Ok(());
        }

        // GNU execute_cmd.c execute_simple_command: array-style assignment
        // prefixes like `var[0]=X` are recognized as assignment words by
        // assignment() (general.c:480) and separated from command words at
        // parse time. assign_in_env (variables.c:3536) then calls
        // valid_identifier() on the extracted name (e.g. `var[0]`), which
        // fails because `[` and `]` are not legal variable characters.
        // sh_invalidid (builtins/common.c:208) reports
        // `` `var[0]': not a valid identifier `` but execution continues
        // with the remaining command word(s). Rubash's parser does not
        // separate array-style assignment words from command words, so they
        // remain in cmd.words; strip them here, report the diagnostic, and
        // continue with the remaining command.
        let cmd = self.strip_invalid_env_assignment_prefixes(&cmd);

        let dispatch_result = if let Some(result) = self.execute_function_command_invocation(&cmd) {
            result
        } else if self.execute_assignment_or_comment_command(&cmd) {
            Ok(())
        } else if !command_needs_process_substitution_materialization(&cmd) {
            self.execute_materialized_command(&cmd, ProcessSubstitutionFiles::default())
        } else {
            match self.command_with_process_substitution_files(&cmd) {
                Ok((materialized_cmd, process_substitution_files)) => {
                    self.execute_materialized_command(&materialized_cmd, process_substitution_files)
                }
                Err(error) => Err(error),
            }
        };
        match dispatch_result {
            // GNU redir.c:135 redirection_error (from redir.c:260
            // do_redirections) reports a failed redirect open through the fd
            // 2 binding the command's earlier redirections already applied —
            // not the shell's default stderr — so `cat 2>/dev/null < /missing`
            // stays silent in default, posix and comsub contexts alike
            // (issue #250). Closed-output errors keep propagating for the
            // pipeline machinery to observe (is_closed_output_io_error).
            Err(ExecuteError::IoError(error))
                if !super::ast_exec::is_closed_output_io_error(&error) =>
            {
                let mut stderr = Vec::new();
                // GNU redir.c:135 redirection_error is internal_error
                // (error.c:149-176) via error_prolog: the prefix honors
                // gnu_errfmt (rubash#1038). The LINE in the prolog is the
                // one do_redirections sees (execute_cmd.c:854): the
                // ambient line for a compound in the ambient class, the
                // completion line for cm_arith / cm_cond — see
                // Executor::compound_redirect_error_prefix (rubash#3183).
                // When the special prefix's identification fails the
                // funnel-stamped composition below runs unchanged.
                let special_prefix = self.compound_redirect_error_prefix(&cmd);
                let mut special_handled = false;
                if special_prefix != self.error_prefix() {
                    writeln!(
                        &mut stderr,
                        "{}{}",
                        special_prefix,
                        crate::posix_errors::message(&error)
                    )?;
                    if self
                        .failing_open_redirect_index_with_prefix(&cmd, &stderr, &special_prefix)
                        .is_some()
                    {
                        let _ = self.write_redirect_diagnostic_routed_with_prefix(
                            &cmd,
                            &stderr,
                            &special_prefix,
                        );
                        special_handled = true;
                    } else {
                        stderr.clear();
                    }
                }
                if !special_handled {
                    writeln!(
                        &mut stderr,
                        "{}{}",
                        self.error_prefix(),
                        crate::posix_errors::message(&error)
                    )?;
                    self.write_redirect_diagnostic_routed(&cmd, &stderr)?;
                }
                self.exit_code = 1;
                // rubash#2082: GNU execute_cmd.c:4865-4875 — the failure
                // above is execute_builtin_or_function's do_redirections
                // returning EX_REDIRFAIL (execute_cmd.c:5613), and for a
                // special builtin in a posixly_correct non-interactive
                // shell the conversion is an immediate
                // jump_to_top_level (ERREXIT) at status
                // EXECUTION_FAILURE. The jump fires inside
                // execute_simple_command, so tested contexts (||, if
                // conditions, `!`) do not contain it and the caller's
                // ERR-trap check (execute_cmd.c:994) is never reached:
                // exit WITHOUT the ERR trap the errexit conversion
                // below would fire. The diagnostic printed above is the
                // redirection_error report GNU emits before the jump
                // (redir.c:135).
                if self.posix_special_redirect_failure_exits(&cmd) {
                    return Err(ExecuteError::ExitCode(1));
                }
                if self.errexit_enabled() && self.errexit_is_active() {
                    // rubash#1171 (same class): a redirect-failed simple
                    // command is a failed command — GNU fires the ERR
                    // trap before the ERREXIT jump; the Ok path below
                    // lets the ast_exec loop fire instead.
                    // rubash#1611: inside a timed dispatch the ERR trap
                    // is adjudicated once by the ast_exec time branch
                    // (rubash#1277) — firing here too would double it.
                    if self.timed_dispatch_depth.get() == 0 {
                        self.maybe_run_error_trap(&cmd)?;
                    }
                    // rubash#1611: the command completed (with a
                    // redirection failure status) — mark the conversion
                    // for execute_time_ast_command.
                    self.errexit_status_converted.set(true);
                    // DE-06 (design 9.3; PLAN row 12): the errexit
                    // decision — park a selected `set -e` filter
                    // here, before the exit path (timed deaths park
                    // at the time branch's promotion instead).
                    if self.timed_dispatch_depth.get() == 0 {
                        self.debugger_note_exception_site(
                            crate::debugger::ExceptionKind::Errexit,
                            1,
                            None,
                        )?;
                    }
                    return Err(self.errexit_exit(1));
                }
                Ok(())
            }
            other => other,
        }
    }

    /// rubash#2082: the gate behind the EX_REDIRFAIL conversions —
    /// execute_command's IoError arm for the apply-layer channel and
    /// execute_materialized_command's reject-gate channel
    /// (command_dispatch.rs). GNU's condition is execute_cmd.c:4865-4875:
    /// `posixly_correct && builtin_is_special && interactive_shell
    /// == 0`. The `command` builtin's re-dispatch never carries
    /// builtin_is_special — the same depth gate the
    /// special_builtin_failed adjudication in
    /// execute_materialized_command applies (command_dispatch.rs;
    /// errors8.sub) — and a name the `enable` builtin disabled
    /// dispatches to the filesystem instead, where GNU's disk-command
    /// redirection failure stays a plain status 1 (execute_disk_command
    /// applies the redirections in the forked child).
    pub(in crate::executor) fn posix_special_redirect_failure_exits(
        &self,
        cmd: &CommandNode,
    ) -> bool {
        self.posix_mode_enabled()
            && self.command_builtin_depth == 0
            && !self.invocation_interactive()
            && cmd.words.first().is_some_and(|word| {
                is_posix_special_builtin(word)
                    && !crate::builtins::enable::is_disabled(&self.shell_state.env_vars, word)
            })
    }

    /// GNU builtins/evalstring.c:588-596 (parse_and_execute's parse-
    /// failure else-branch): when the parser reports EX_BADSYNTAX out of
    /// an evalstring, the shell jumps to top level with
    /// last_command_exit_value = EX_BADUSAGE (2) iff
    /// `interactive_shell == 0 && this_shell_builtin` is the source or
    /// eval builtin `&& posixly_correct && executing_command_builtin
    /// == 0`. report_syntax_error produces EX_BADSYNTAX only while
    /// `executing_builtin && parse_and_execute_level` (parse.y:6842),
    /// i.e. only for a string being parsed by eval/source — the tracker
    /// bit is what decides whether the CURRENT string's owner is the
    /// most recent dispatch (see the field's doc in shell/state.rs).
    /// executing_command_builtin is the caller-context bit: the
    /// `command` re-dispatch runs with it set, mirrored here by
    /// command_builtin_depth, exactly as for the #2082 gate.
    pub(crate) fn evalstring_syntax_error_exits(&self) -> bool {
        self.posix_mode_enabled()
            && self.command_builtin_depth == 0
            && !self.invocation_interactive()
            && self.shell_state.last_dispatch_was_eval_or_source
    }

    /// execute_cmd.c:4813 (`this_shell_builtin = ...`) reduced to the
    /// #2126 tracker bit: a simple-command dispatch of `eval`, `source`
    /// or `.` arms it, every other simple-command dispatch (builtin,
    /// function, disk command — including an eval/source name that is
    /// disabled and therefore resolves externally) disarms it. GNU
    /// assigns on dispatch and never restores on completion; the note
    /// points are the dispatch funnels (execute_prepared_command, the
    /// function fast path's commit point, and the `command`/`builtin`
    /// re-dispatch tables).
    pub(in crate::executor) fn note_simple_command_dispatch(&mut self, word: &str) {
        self.shell_state.last_dispatch_was_eval_or_source = matches!(word, "eval" | "source" | ".")
            && !crate::builtins::enable::is_disabled(&self.shell_state.env_vars, word);
    }

    /// execute_cmd.c:4810-4813 (`run_builtin`): `last_shell_builtin =
    /// this_shell_builtin; this_shell_builtin = builtin;` — the roll
    /// happens when a simple command RESOLVES to a builtin (this =
    /// that builtin) or to a function (this = NULL, projected here as
    /// `Other`); a disk command never reaches run_builtin and never
    /// rolls. The note points are the resolution-confirmed dispatch
    /// sites: the primary/late funnel entries, the function fast path,
    /// the `%` jobspec fg/bg dispatch, and the exit/logout/jobs arms
    /// of the `command`/`builtin` re-dispatch tables (whose outer
    /// dispatch already rolled `Other`, so only the classes the exit
    /// check distinguishes need an inner roll there). Read solely by
    /// the interactive exit/logout job check (builtins/exit.def
    /// `exit_or_logout`, rubash#2397).
    pub(in crate::executor) fn note_builtin_dispatch(
        &mut self,
        class: crate::shell::BuiltinDispatchClass,
    ) {
        self.shell_state.last_builtin_class = self.shell_state.this_builtin_class;
        self.shell_state.this_builtin_class = class;
    }

    /// The exit.def-distinguished class of a builtin name; every other
    /// builtin (and every function call) projects to `Other`.
    pub(in crate::executor) fn builtin_dispatch_class(
        word: &str,
    ) -> crate::shell::BuiltinDispatchClass {
        match word {
            "exit" => crate::shell::BuiltinDispatchClass::Exit,
            "logout" => crate::shell::BuiltinDispatchClass::Logout,
            "jobs" => crate::shell::BuiltinDispatchClass::Jobs,
            _ => crate::shell::BuiltinDispatchClass::Other,
        }
    }

    /// GNU shell.c reader_loop under `-n`/`set -n` (noexec): the reader
    /// still parses every command, so all parse-time diagnostics —
    /// `syntax error near ...`, unclosed-compound/cond `unexpected end of
    /// file`, unterminated comsub/extglob — fire exactly as the
    /// execute_command preamble reports them; only execution is skipped.
    /// Returns Ok(true) when a diagnostic already reported (and set the
    /// status or re-executed a repaired parse); Ok(false) lets the caller
    /// continue to the real dispatch.
    pub(in crate::executor) fn command_parse_diagnostics(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        let _t_scans = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_SCANS);

        // GNU parse.y:3249 alias_expand_token expands aliases in the reader
        // — inside a function body too — so an alias supplying a compound
        // opener (`alias forever='while :;'` → `while :; do`) parses
        // structurally. The token-level body parse is alias-blind; when it
        // failed, the body text is parked on
        // FunctionCommand::unparsed_body_source for one alias-aware retry
        // before the diagnostic fires (modernish `forever do` inside
        // sourced .mm module bodies).
        if !self.noexec_enabled() {
            if let Some(function) = &cmd.function_command {
                if let Some((source, base_line)) = function.unparsed_body_source.clone() {
                    if let Some(body) = self.retry_function_body_with_aliases(&source, base_line) {
                        let mut fixed_cmd = cmd.clone();
                        fixed_cmd
                            .assignments
                            .retain(|(name, _)| !name.starts_with("__RUBASH_PARSE"));
                        let mut fixed = function.as_ref().clone();
                        fixed.body = body;
                        // rubash#3209: same definition-line restamp
                        // as the parser path (bare `name()` only).
                        if !fixed.keyword && fixed.has_parentheses {
                            if let Some(definition_line) = cmd.line {
                                crate::parser::restamp_function_body_first_command(
                                    &mut fixed.body,
                                    definition_line,
                                );
                            }
                        }
                        fixed.unparsed_body_source = None;
                        fixed_cmd.function_command = Some(Box::new(fixed));
                        let fixed_function =
                            fixed_cmd.function_command.as_deref().expect("set above");
                        self.define_function(&fixed_cmd, fixed_function)?;
                        return Ok(true);
                    }
                }
            }
        }

        // GNU alias_expand_token during parse (parse.y): a reserved-word
        // alias such as `f=fi` can close a compound the first parse missed.
        // Nodes that parked their whole source span get one alias-expanded
        // retry before their diagnostic fires.
        if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE_SPAN__") {
            if !source.contains("<<") {
                if let Some(reparsed) = self.reparse_reserved_word_aliases(source) {
                    let tokens = crate::lexer::tokenize(&reparsed);
                    let ast = crate::parser::parse(&tokens);
                    return self.execute_ast(&ast).map(|_| true);
                }
            }
        }

        if let Some(message) = cmd.get_assignment("__RUBASH_COMPOUND_SYNTAX_ERROR__") {
            let message = bash_style_unexpected_token_message(message);
            eprintln!("{}syntax error near {message}", self.parser_error_prefix());
            self.record_run_diagnostic(format!("syntax error near {message}"), cmd.line);
            // GNU error.c:324-327: parser_error under live errexit prints
            // only the first line and exit_shell(2)s (rubash#306).
            if self.errexit_active_at_diagnostic() {
                self.exit_code = 2;
                return Err(ExecuteError::ExitCode(2));
            }
            if self.offending_line_echo_enabled() {
                if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                    eprintln!(
                        "{}`{}'",
                        self.parser_error_prefix(),
                        parse_error_source_display(source)
                    );
                }
            }
            self.exit_code = 1;
            return Ok(true);
        }

        if let Some(message) = cmd.get_assignment("__RUBASH_PARSE_ERROR_EOF_PAREN__") {
            // parse.y: an unclosed `name=(` compound assignment reports the
            // bare EOF diagnostic with status 1 and no source echo — but
            // GNU error.c:324-327 forces exit status 2 when errexit is live
            // at the parser_error (probe: `set -e; bad=(' exits 2 while
            // plain `bad=(' exits 1).
            self.mark_parse_error();
            eprintln!("{}{}", self.parser_error_prefix(), message);
            self.record_run_diagnostic(message.clone(), cmd.line);
            if self.errexit_active_at_diagnostic() {
                self.exit_code = 2;
                return Err(ExecuteError::ExitCode(2));
            }
            self.exit_code = 1;
            return Err(ExecuteError::ExitCode(1));
        }

        if let Some(spec) = cmd
            .get_assignment("__RUBASH_PARSE_ERROR_EOF_SUBSHELL__")
            .map(|value| format!("({PARSE_ERROR_FIELD_SEP}{value}"))
            .or_else(|| {
                cmd.get_assignment("__RUBASH_PARSE_ERROR_EOF_COMPOUND__")
                    .cloned()
            })
        {
            // GNU parse.y:6890-6901 (yyerror EOF path): an unclosed compound
            // reports "unexpected end of file from `X' command on line N"
            // naming the innermost open compound (compoundcmd_lineno stack
            // top). Heredocs still pending inside the region issued their
            // gather warnings during the parse (make_cmd.c:626), so emit
            // them first.
            self.mark_parse_error();
            let mut fields = spec.split(crate::executor::markers::PARSE_ERROR_FIELD_SEP);
            let compound_name = fields.next().unwrap_or("(");
            let open_line = fields
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1);
            let eof_line = fields
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1);
            for index in 0usize.. {
                let key = format!("__RUBASH_PARSE_ERROR_HD_WARN_{index}__");
                let Some(warn) = cmd.get_assignment(&key) else {
                    break;
                };
                let mut parts = warn.split(crate::executor::markers::PARSE_ERROR_FIELD_SEP);
                let delimiter = parts.next().unwrap_or("");
                let at_line = parts
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(1);
                let warn_line = parts
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(1);
                eprintln!(
                    "{}warning: here-document at line {at_line} delimited by end-of-file (wanted `{delimiter}')",
                    self.error_prefix_for_line(warn_line)
                );
            }
            eprintln!(
                "{}syntax error: unexpected end of file from `{compound_name}' command on line {open_line}",
                self.parser_error_prefix_for_line(eof_line)
            );
            self.record_run_diagnostic(
                format!(
                    "syntax error: unexpected end of file from `{compound_name}' command on line {open_line}"
                ),
                Some(eof_line),
            );
            // GNU error.c:324-327: parser_error + live errexit exits the
            // shell immediately (rubash#306).
            self.errexit_active_at_diagnostic();
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }

        if let Some(spec) = cmd.get_assignment("__RUBASH_PARSE_ERROR_COND__") {
            // GNU parse.y conditional diagnostics: each parser_error message
            // first, then report_syntax_error — either `syntax error near
            // `X'` plus the offending source line, or at EOF the
            // `unexpected end of file from `[[' command on line N' tail.
            self.mark_parse_error();
            let mut fields = spec.split(crate::executor::markers::PARSE_ERROR_FIELD_SEP);
            let shape = fields.next().unwrap_or("near");
            let aux_a = fields.next().unwrap_or_default();
            let aux_b = fields
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1);
            for field in fields {
                let (line, message) = field
                    .split_once('\u{1}')
                    .map(|(line, msg)| (line.parse::<usize>().unwrap_or(1), msg))
                    .unwrap_or((1, field));
                eprintln!("{}{}", self.parser_error_prefix_for_line(line), message);
                self.record_run_diagnostic(message.to_string(), Some(line));
                // GNU error.c:324-327: every one of these lines is a
                // parser_error; with live errexit the first one already
                // exit_shell(2)ed (rubash#306).
                if self.errexit_active_at_diagnostic() {
                    self.exit_code = 2;
                    return Err(ExecuteError::ExitCode(2));
                }
            }
            if shape == "eof" {
                eprintln!(
                    "{}syntax error: unexpected end of file from `[[' command on line {aux_a}",
                    self.parser_error_prefix_for_line(aux_b)
                );
                self.record_run_diagnostic(
                    format!(
                        "syntax error: unexpected end of file from `[[' command on line {aux_a}"
                    ),
                    Some(aux_b),
                );
                self.errexit_active_at_diagnostic();
            } else if shape == "bare" {
                // PST_NOERROR shape (parse.y:3914 + 6724-6726): the messages
                // above are the whole diagnostic — yyerror's tail is
                // suppressed and nothing echoes.
            } else {
                eprintln!(
                    "{}syntax error near `{aux_a}'",
                    self.parser_error_prefix_for_line(aux_b)
                );
                self.record_run_diagnostic(format!("syntax error near `{aux_a}'"), Some(aux_b));
                if self.errexit_active_at_diagnostic() {
                    self.exit_code = 2;
                    return Err(ExecuteError::ExitCode(2));
                }
                // parse.y:6865: no offending-line echo in an interactive
                // shell.
                if self.offending_line_echo_enabled() {
                    if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                        eprintln!(
                            "{}`{}'",
                            self.parser_error_prefix_for_line(aux_b),
                            parse_error_source_display(source)
                        );
                    }
                }
            }
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }

        if let Some(spec) = cmd.get_assignment("__RUBASH_PARSE_ERROR_NEAR__") {
            // rubash#1093: GNU reads and executes one line at a time, so a
            // runtime `shopt -s extglob` executed on an earlier line (via
            // a function, eval, or an if-block — none of which the lexer's
            // literal-shopt line scan can foresee) opens the parse gate
            // for this line. Rubash parsed the whole script up front with
            // the gate closed and stored a parse-error node; when the
            // gate is open NOW and was opened by an earlier line, reparse
            // the stored line with the live gate and execute that, which
            // is exactly the cadence GNU had. A same-line enable keeps
            // the error (GNU parses a full line before executing it).
            {
                let mut fields = spec.split(crate::executor::markers::PARSE_ERROR_FIELD_SEP);
                let token = fields.next().unwrap_or_default();
                let line = fields
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(1);
                let enabled_line = self
                    .shell_state
                    .env_vars
                    .get("__RUBASH_EXTGLOB_ENABLED_LINE")
                    .and_then(|value| value.parse::<usize>().ok());
                if token == "("
                    && crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "extglob")
                    && enabled_line.is_some_and(|enabled| enabled < line)
                {
                    if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                        if !source.contains("<<") {
                            let tokens = crate::lexer::tokenize(source);
                            let ast = crate::parser::parse(&tokens);
                            let still_errored = ast.commands.iter().any(|command| {
                                command.has_assignment("__RUBASH_PARSE_ERROR__")
                                    || command.has_assignment("__RUBASH_PARSE_ERROR_NEAR__")
                                    || command.has_assignment("__RUBASH_PARSE_ERROR_EOF__")
                            });
                            if !still_errored && !ast.commands.is_empty() {
                                return self.execute_ast(&ast).map(|_| true);
                            }
                        }
                    }
                }
            }
            // GNU parse.y yyerror: `syntax error near unexpected token 'X'`
            // reported at the line of the offending token, with that input
            // line echoed (mismatched closer inside a compound command).
            self.mark_parse_error();
            let mut fields = spec.split(crate::executor::markers::PARSE_ERROR_FIELD_SEP);
            let token = fields.next().unwrap_or_default();
            let line = fields
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1);
            // rubash#305: a pending here document that hit EOF warned during
            // gathering (make_cmd.c:626), i.e. BEFORE read_token returned
            // the NEWLINE whose rejection printed this error — emit any
            // carried warnings first, at the input-stream prefix GNU's
            // parser_error used for them (script name, not the parser's
            // `eval'/`-c' stream tag).
            for warn_index in 0usize.. {
                let key = format!("__RUBASH_PARSE_ERROR_HD_WARN_{warn_index}__");
                let Some(warn) = cmd.get_assignment(&key) else {
                    break;
                };
                let mut parts = warn.split(crate::executor::markers::PARSE_ERROR_FIELD_SEP);
                let delimiter = parts.next().unwrap_or("");
                let at_line = parts
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(1);
                let warn_line = parts
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(1);
                self.write_diagnostic_fd2(
                    format!(
                        "{}warning: here-document at line {at_line} delimited by end-of-file (wanted `{delimiter}')\n",
                        self.error_prefix_for_line(warn_line)
                    )
                    .as_bytes(),
                );
            }
            self.write_diagnostic_fd2(
                format!(
                    "{}syntax error near unexpected token `{token}'\n",
                    self.parser_error_prefix_for_line(line)
                )
                .as_bytes(),
            );
            self.record_run_diagnostic(
                format!("syntax error near unexpected token `{token}'"),
                Some(line),
            );
            // GNU error.c:324-327: with exit_immediately_on_error live, the
            // offending-line echo (parse.y:6814 print_offending_line) never
            // runs — the first parser_error line already exit_shell(2)ed
            // (rubash#306).
            if self.errexit_active_at_diagnostic() {
                self.exit_code = 2;
                return Err(ExecuteError::ExitCode(2));
            }
            // The producer stores the verbatim physical line (parse.y
            // y.error echoes it as read); parse_error_source_display would
            // trim GNU's leading whitespace.
            // parse.y:6865: no offending-line echo in an interactive shell.
            if self.offending_line_echo_enabled() {
                if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                    self.write_diagnostic_fd2(
                        format!("{}`{}'\n", self.parser_error_prefix_for_line(line), source)
                            .as_bytes(),
                    );
                }
            }
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }

        if cmd.has_assignment("__RUBASH_PARSE_ERROR_EOF__") {
            // GNU parse.y yyerror: input ended where a command was
            // expected (`x |` EOF) — "syntax error: unexpected end of
            // file" at the end-of-input line, with no token echo.
            self.mark_parse_error();
            let eof_line = cmd.line.map(|line| line + 1).unwrap_or(1);
            self.write_diagnostic_fd2(
                format!(
                    "{}syntax error: unexpected end of file\n",
                    self.parser_error_prefix_for_line(eof_line)
                )
                .as_bytes(),
            );
            self.record_run_diagnostic(
                "syntax error: unexpected end of file".to_string(),
                Some(eof_line),
            );
            // Single-line parser_error: GNU error.c:324-327 still
            // exit_shell(2)s right after it under live errexit — record
            // that so eval containment propagates the exit (rubash#306).
            self.errexit_active_at_diagnostic();
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }

        if cmd.has_assignment("__RUBASH_PARSE_ERROR__") {
            // rubash#1093: same runtime-extglob recovery as the NEAR arm
            // below, for producers that store the offending line.
            if cmd
                .get_assignment("__RUBASH_PARSE_ERROR__")
                .is_some_and(|message| message.contains("unexpected token `('"))
                && crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "extglob")
                && self
                    .shell_state
                    .env_vars
                    .get("__RUBASH_EXTGLOB_ENABLED_LINE")
                    .and_then(|value| value.parse::<usize>().ok())
                    .is_some_and(|enabled| enabled < cmd.line.unwrap_or(1))
            {
                if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                    if !source.contains("<<") {
                        let tokens = crate::lexer::tokenize(source);
                        let ast = crate::parser::parse(&tokens);
                        let still_errored = ast.commands.iter().any(|command| {
                            command.has_assignment("__RUBASH_PARSE_ERROR__")
                                || command.has_assignment("__RUBASH_PARSE_ERROR_NEAR__")
                                || command.has_assignment("__RUBASH_PARSE_ERROR_EOF__")
                        });
                        if !still_errored && !ast.commands.is_empty() {
                            return self.execute_ast(&ast).map(|_| true);
                        }
                    }
                }
            }
            self.mark_parse_error();
            if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                if !source.contains("<<") {
                    if let Some(reparsed) = self.reparse_reserved_word_aliases(source) {
                        let tokens = crate::lexer::tokenize(&reparsed);
                        let ast = crate::parser::parse(&tokens);
                        return self.execute_ast(&ast).map(|_| true);
                    }
                }
            }
            self.report_command_parse_error(cmd);
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }

        {
            // GNU make_cmd.c gather_here_documents + parse.y:6883: warn on
            // any heredoc header the unclosed comsub swallowed, then report
            // `unexpected EOF` at the line after the last input line — not
            // the command's start line.
            let mut base_line = cmd.line.unwrap_or(1);
            let mut reported = false;
            for raw in cmd
                .assignment_values()
                .map(|v| v.as_str())
                .chain(cmd.word_metadata.iter().map(|m| m.raw.as_str()))
            {
                // Same composition as the driver's completeness gate
                // (lexer/mod.rs has_unclosed_input_syntax_posix): the
                // corrected skip::command_substitutions_balanced overrides
                // false positives from has_unclosed_command_substitution —
                // e.g. `$(cat <<< hi)` inside a double-quoted word, where
                // the here-string operator previously read as a `<<` heredoc
                // and swallowed the closing `)` (rubash#168). Without the
                // secondary check here the executor re-killed scripts the
                // driver had already accepted.
                // Admission whitelist (not a symptom blacklist): every
                // residual has_unclosed_command_substitution can report is
                // introduced by `$(`, `$'`, `${` or a backtick
                // (lexer/continuation.rs comsub_residuals), so a raw with
                // no `$` and no backtick is provably clean and the
                // char-by-char DFA never needs to run. A false admission
                // is impossible; a miss only costs the scan.
                if (raw.contains('$') || raw.contains('`'))
                    && crate::lexer::has_unclosed_command_substitution(raw)
                    && !crate::lexer::command_substitutions_balanced(raw)
                {
                    // parse.y:3105 push_heredoc: when the comsub body's own
                    // tokenize parked the HEREDOC_MAX overflow, GNU's nested
                    // parse died with the fatal before any of the body's own
                    // EOF diagnostics — this unclosed-comsub sweep and its
                    // per-body gather replay model exactly that dead parse.
                    // Report only the fatal, at the offending line.
                    if let Some(line) = crate::lexer::heredoc_overflow_line() {
                        self.mark_parse_error();
                        self.write_diagnostic_fd2(
                            format!(
                                "{}maximum here-document count exceeded\n",
                                self.parser_error_prefix_for_line(line)
                            )
                            .as_bytes(),
                        );
                        self.record_run_diagnostic(
                            "maximum here-document count exceeded".to_string(),
                            Some(line),
                        );
                        self.exit_code = 2;
                        return Err(ExecuteError::ExitCode(2));
                    }
                    self.mark_parse_error();
                    self.report_unclosed_comsub_eof(raw, base_line);
                    reported = true;
                    break;
                }
                base_line += raw.matches('\n').count();
            }
            if reported {
                self.exit_code = 2;
                return Err(ExecuteError::ExitCode(2));
            }
        }

        if cmd.function_command.is_none()
            && cmd
                .word_metadata
                .iter()
                // Same admission shape: unterminated_extglob can only
                // leave depth > 0 when an extglob operator is followed by
                // `(`, so a raw without `(` is provably clean.
                .any(|metadata| metadata.raw.contains('(') && unterminated_extglob(&metadata.raw))
        {
            self.mark_parse_error();
            self.write_diagnostic_fd2(
                format!(
                    "{}syntax error near unexpected token `('\n",
                    self.parser_error_prefix()
                )
                .as_bytes(),
            );
            self.record_run_diagnostic(
                "syntax error near unexpected token `('".to_string(),
                cmd.line,
            );
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }

        // GNU adjudicates extglob legality at READ time: parse.y:5466
        // read_token_word consumes a `(` group as pattern text only
        // while the read-time `extended_glob` is on, and a group met
        // with the gate off fails the parse itself. At execution the
        // pattern text is inert data — pathname expansion and strmatch
        // consult the LIVE flag (pathexp.c unquoted_glob_pattern_p;
        // pathexp.h FNMATCH_EXTFLAG), so a group that WAS consumed as
        // a pattern at read time degrades to literal text/matching
        // when a same-unit `shopt -u extglob` has already executed
        // (rubash#3224) and must not be vetoed here. The diagnostic
        // below therefore belongs only to patterns whose producing
        // pass did not consume them as patterns
        // (ExtglobPattern::read_gate_on == false): for those, treating
        // the word as a literal would silently accept a script GNU
        // rejects at read time. Conditional RHS patterns are handled
        // separately and intentionally remain eligible for Bash's
        // conditional-pattern semantics.
        if !crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "extglob")
            && cmd
                .extglob_patterns
                .iter()
                .any(|pattern| !pattern.read_gate_on)
            && cmd.conditional_command.is_none()
            && cmd.case_command.is_none()
        {
            self.mark_parse_error();
            self.write_diagnostic_fd2(
                format!(
                    "{}syntax error near unexpected token `('\n",
                    self.parser_error_prefix()
                )
                .as_bytes(),
            );
            self.record_run_diagnostic(
                "syntax error near unexpected token `('".to_string(),
                cmd.line,
            );
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }

        Ok(false)
    }

    /// `set -k` harvest, mirroring GNU subst.c:12479-12535. The parser only
    /// recognises assignment words while the command still has no words
    /// (token_actions.rs, matching GNU's leading-run `subst_assign_varlist`),
    /// so every trailing `name=value` word sits in `cmd.words`. Under
    /// `set -k` each of them is moved to the end of the assignment list and
    /// dropped from the word list; the caller then sees either an
    /// assignment-only command (all assignments become permanent) or a command
    /// whose arguments no longer contain the assignments (they become the
    /// temporary environment, GNU execute_cmd.c tempenv path).
    fn command_with_keep_temporary_assignments(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<CommandNode> {
        if !crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "keyword") {
            return None;
        }

        let mut harvested: Vec<(String, String)> = Vec::new();
        let mut indexes: Vec<usize> = Vec::new();
        for (index, word) in cmd.words.iter().enumerate() {
            // GNU takes the assignment verdict at READ time, on the
            // raw token: parse.y stamps W_ASSIGNMENT via assignment()
            // (general.c:480) over the token exactly as read — quotes
            // included — and the sweep moves exactly the stamped
            // words. The cooked word cannot carry the verdict: a
            // quoted `"v=$v"` cooks to the same text as the bare
            // token but was never stamped, so judging the cooked
            // word harvested quoted arguments out of the command
            // line (rubash#2959). Judge the raw text with the
            // parser's own predicate (the same one the leading-run
            // classification reads, parse_loop.rs); the cooked word
            // only supplies the (name, value) pair. A synthetic word
            // has no raw token and is never stamped.
            let raw = cmd
                .word_metadata
                .get(index)
                .map(|meta| meta.raw.as_str())
                .unwrap_or("");
            if !crate::lexer::is_assignment(raw) {
                continue;
            }
            let Some((name, value)) = split_assignment_word(word) else {
                continue;
            };
            // `name=(...)` keeps its own storage path (compound_assignments)
            // and GNU rejects it in this position as a syntax error, so it is
            // never harvested.
            if value.starts_with(COMPOUND_ASSIGNMENT_MARKER) {
                continue;
            }
            harvested.push((name.to_string(), value.to_string()));
            indexes.push(index);
        }
        if harvested.is_empty() {
            return None;
        }

        let mut materialized = cmd.clone();
        for index in indexes.into_iter().rev() {
            if index < materialized.words.len() {
                materialized.words.remove(index);
            }
            if index < materialized.word_metadata.len() {
                materialized.word_metadata.remove(index);
            }
            if index < materialized.word_kinds.len() {
                materialized.word_kinds.remove(index);
            }
        }
        materialized.assignments.extend(harvested);
        Some(materialized)
    }

    /// GNU execute_cmd.c execute_simple_command + variables.c assign_in_env:
    /// array-style assignment prefixes like `var[0]=X` are recognized as
    /// assignment words by assignment() (general.c:480) and separated from
    /// command words at parse time. assign_in_env (variables.c:3536-3566)
    /// extracts the name before `=` (e.g. `var[0]`), calls valid_identifier()
    /// which fails for array-style names, and sh_invalidid (builtins/common.c:208)
    /// reports `` `var[0]': not a valid identifier ``. The assignment is
    /// skipped but execution continues with the remaining command word(s).
    ///
    /// Rubash's parser does not separate array-style assignment words from
    /// command words (is_assignment in classification.rs only recognizes
    /// simple `name=value`), so they remain in cmd.words. This function
    /// detects leading array-style assignment words, reports the diagnostic
    /// for each, strips them, and returns the modified command so the
    /// remaining command word(s) reach function/builtin/external dispatch.
    fn strip_invalid_env_assignment_prefixes<'a>(
        &mut self,
        cmd: &'a CommandNode,
    ) -> std::borrow::Cow<'a, CommandNode> {
        let (stripped, diagnostics) = self.strip_invalid_env_assignment_prefixes_collecting(cmd);
        for line in &diagnostics {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        stripped
    }

    /// The strip core, with the diagnostics returned instead of
    /// written: pipeline stages route them through the ambient fd 2
    /// (write_default_stderr) because GNU reports them during the
    /// element's word expansion, before the element's own
    /// redirections apply (execute_cmd.c:4617) — the same routing as
    /// the stage expansion-error checks (rubash#1568).
    pub(in crate::executor) fn strip_invalid_env_assignment_prefixes_collecting<'a>(
        &mut self,
        cmd: &'a CommandNode,
    ) -> (std::borrow::Cow<'a, CommandNode>, Vec<String>) {
        // Find the run of leading array-style assignment words.
        let mut prefix_end = 0usize;
        for index in 0..cmd.words.len() {
            if !command_word_is_array_element_assignment(cmd, index) {
                break;
            }
            prefix_end += 1;
        }
        // rubash#1568: when EVERY word is an assignment word (the
        // element run plus a following plain-assignment run), there
        // is no command word at all — GNU's assignment()
        // (general.c:480) classifies them all as assignment words,
        // so the node is an assignment STATEMENT whose words all
        // bind permanently (execute_mixed_assignment_statement),
        // never an invalidid prefix in front of a command. Leave
        // the node untouched (and silent) for that arm.
        if prefix_end > 0 {
            // The continuation run uses the mixed-statement
            // admission's own classification (a plain word of any
            // value shape, or another element word) — NOT the
            // harvest's, which additionally refuses compound values
            // it cannot move.
            let mut rest_run = 0usize;
            for index in prefix_end..cmd.words.len() {
                let is_assignment_word = split_assignment_word(&cmd.words[index]).is_some()
                    || command_word_is_array_element_assignment(cmd, index);
                if !is_assignment_word {
                    break;
                }
                rest_run += 1;
            }
            if prefix_end + rest_run == cmd.words.len() {
                return (std::borrow::Cow::Borrowed(cmd), Vec::new());
            }
        }
        // Only strip when there is at least one remaining command word.
        // If all words are array-style assignments, the command is
        // assignment-only and execute_array_element_assignment handles it.
        //
        // GNU execute_simple_command walks the WORD_LIST by pointer
        // (execute_cmd.c:4550+; the assignment/word classification is flag
        // tests on existing nodes, no command copy) — the common nothing-to-
        // strip case must not pay a whole-CommandNode deep copy, so borrow
        // instead of cloning (Cow; the copy exists only when words are
        // actually removed).
        if prefix_end == 0 || prefix_end >= cmd.words.len() {
            return (std::borrow::Cow::Borrowed(cmd), Vec::new());
        }
        // Report the diagnostic for each invalid identifier, mirroring
        // GNU assign_in_env -> sh_invalidid -> builtin_error. The name is
        // everything before `=` (with `+` stripped for append), which for
        // array-style words like `var[0]=X` is `var[0]` -- not a valid
        // identifier because `[` and `]` are not legal variable characters.
        let mut diagnostics = Vec::new();
        for word in &cmd.words[..prefix_end] {
            let Some((left, _)) = word.split_once('=') else {
                continue;
            };
            let name = left.strip_suffix('+').unwrap_or(left);
            if !is_shell_name(name) {
                let line = format!(
                    "{}`{}': not a valid identifier
",
                    self.diagnostic_prefix(),
                    name
                );
                diagnostics.push(line);
            }
        }
        // rubash#1556: the parser's assignment-run classification
        // also stops at the first array-style word, so simple
        // assignment words FOLLOWING the stripped run are still
        // sitting in cmd.words (`a[0]=5 b=2 declare -p b` dispatched
        // `b=2` as the command name and reported it not found).
        // GNU's assignment() recognizes every leading assignment
        // word regardless of an invalid sibling: assign_in_env
        // skips the invalid word and the remaining words still bind
        // as the temporary environment. Move the following
        // simple-assignment run into the assignment list (the shape
        // the set -k harvest uses), always keeping one word as the
        // command name.
        let mut harvested: Vec<(String, String)> = Vec::new();
        let mut harvest_end = prefix_end;
        while harvest_end + 1 < cmd.words.len() {
            let Some((name, value)) = split_assignment_word(&cmd.words[harvest_end]) else {
                break;
            };
            if value.starts_with(COMPOUND_ASSIGNMENT_MARKER) {
                break;
            }
            harvested.push((name.to_string(), value.to_string()));
            harvest_end += 1;
        }
        let mut stripped = cmd.clone();
        stripped.words = cmd.words[harvest_end..].to_vec();
        // rubash#2237: word_metadata entries address words by key
        // (WordMetadata::word_index), exactly like the array-element
        // records shifted below (#1568) — drop the removed words'
        // entries and shift the survivors' keys. Slicing the vector by
        // count instead assumed it is dense; it is not (a
        // process-substitution word carries no entry), so the surviving
        // words kept entries keyed to their pre-strip positions and a
        // keyed consumer read the wrong word's metadata.
        stripped.word_metadata = cmd
            .word_metadata
            .iter()
            .filter_map(|metadata| {
                metadata.word_index.checked_sub(harvest_end).map(|shifted| {
                    let mut metadata = metadata.clone();
                    metadata.word_index = shifted;
                    metadata
                })
            })
            .collect();
        stripped.assignments.extend(harvested);
        // rubash#1568: the parser's array-element records address
        // words by index; dropping the leading run leaves those
        // indexes pointing at the wrong words (a later consumer
        // re-classified the new head word `declare` as an element
        // word from the stale record and stripped it too). Drop the
        // removed words' records and shift the survivors' indexes.
        stripped.array_element_assignments = cmd
            .array_element_assignments
            .iter()
            .filter_map(|assignment| match assignment.word_index {
                None => Some(assignment.clone()),
                Some(word_index) => word_index.checked_sub(harvest_end).map(|shifted| {
                    let mut assignment = assignment.clone();
                    assignment.word_index = Some(shifted);
                    assignment
                }),
            })
            .collect();
        (std::borrow::Cow::Owned(stripped), diagnostics)
    }
}

impl Executor {
    pub(in crate::executor) fn parameter_heredoc_expansion_error(
        &self,
        cmd: &CommandNode,
    ) -> Option<(String, String, i32)> {
        if let Some(body) = &cmd.heredoc {
            if let Some(error) = self.parameter_expansion_error_in_heredoc_body(body) {
                return Some(error);
            }
        }
        for redirect in &cmd.heredoc_redirects {
            if let Some(body) = &redirect.body {
                if let Some(error) = self.parameter_expansion_error_in_heredoc_body(body) {
                    return Some(error);
                }
            }
        }
        // The unnumbered `<<< word` form keeps its body in here_string (the
        // numbered/carrier form above already covers heredoc_redirects): the
        // body expands as a word when the here-string applies, so an
        // unbound variable reports and the command dies (rubash#535: `cat
        // <<< $nope` used to expand silently to an empty string). GNU
        // probes 2026-10-12: `<<< "$nope"` under set -u still REPORTS (the
        // double-quoted body expands parameters), while `<<< '$nope'` stays
        // the literal text with no diagnostic — so only the single-quoted
        // and ANSI-C forms skip the scan.
        if let Some(word) = &cmd.here_string {
            let trimmed = word.trim_start();
            let no_expansion = trimmed.starts_with('\'') || trimmed.starts_with("$'");
            if !no_expansion {
                if let Some(error) = self.parameter_expansion_error_in_word_isolated(word) {
                    return Some(error);
                }
            }
        }
        None
    }

    /// Map a heredoc-body expansion-error status into the exit status the
    /// command reports (diagnostic already printed by the caller).
    ///
    /// GNU anchor: redir.c:348 heredoc_expand() runs the body through
    /// expand_string_to_string (redir.c:374-377) inside the command's
    /// do_redirections. Where those redirects run decides the blast
    /// radius of the resulting FORCE_EOF jump
    /// (subst.c:4020-4032 call_expand_word_internal: `w->word = NULL;
    /// last_command_exit_value = EXECUTION_FAILURE;
    /// exp_jump_to_top_level(... FORCE_EOF)`):
    ///
    /// - External command: execute_cmd.c:5884+ (execute_disk_command
    ///   child) does do_redirections AFTER fork, in the child. The child
    ///   inherits the parent's top_level setjmp, so the jump exits the
    ///   CHILD with the parent-mode fatal status — script mode exits
    ///   EXECUTION_FAILURE (1) via last_command_exit_value, `-c` mode
    ///   exits 127 via shell.c:1471 run_one_command's FORCE_EOF arm.
    ///   The parent observes only the child's status and the script
    ///   continues (WSL 5.3.0 probes: `M=ERR; cat <<EOF; printf
    ///   'status=%s\n' "$?"` with body `${D?$M}` prints status=1 in
    ///   script mode, status=127 under -c, `after` in both, rc 0).
    /// - Builtin/function: execute_cmd.c:5606
    ///   (execute_builtin_or_function) applies do_redirections in the
    ///   MAIN shell, so the jump reaches the main shell's setjmp and the
    ///   noninteractive script dies (probes: `: <<EOF`/`read X <<EOF`
    ///   with the same body print nothing further, rc 1 script / 127 -c).
    ///
    /// Rubash runs the heredoc expansion before dispatch, so the class is
    /// decided here by the command's first word. A `$var` command word is
    /// expanded only at dispatch and predicts as external (contained):
    /// a false-contained only keeps a script running that GNU would have
    /// killed, while a false-fatal would kill one GNU keeps running.
    pub(in crate::executor) fn finish_heredoc_expansion_error(
        &mut self,
        cmd: &CommandNode,
        status: i32,
    ) -> Result<(), ExecuteError> {
        if status != Self::FATAL_PARAMETER_EXPANSION_STATUS {
            // Non-fatal statuses (bad substitution = 1, EOF = 2) already
            // carry their GNU DISCARD-style values.
            self.exit_code = status;
            return Ok(());
        }
        // subst.c:10416-10418 / 11032-11034: interactive_shell expands the
        // heredoc body in the MAIN shell and takes the error branch
        // (DISCARD) — the command is abandoned with status 1 and the
        // session keeps reading, whatever the target dispatches to.
        if self.expansion_error_is_interactive_discard() {
            self.note_unit_expansion_error_discard();
            self.exit_code = 1;
            // rubash#439: expansion error, not a failed command — no ERR trap.
            self.command_expansion_failed.set(true);
            return Err(ExecuteError::ExpansionFailure(1));
        }
        let code = self.expansion_fatal_status();
        self.exit_code = code;
        if self.heredoc_error_command_runs_in_main_shell(cmd) {
            // Builtin/function target: the FORCE_EOF jump lands in the
            // main shell (execute_cmd.c:5606) and the script exits.
            return Err(ExecuteError::ExitCode(code));
        }
        // External target: the forked child contains the jump; the
        // command reports the mode's fatal status and the script
        // continues.
        Ok(())
    }

    /// The effective command word of a simple command, for
    /// heredoc-expansion context classification: leading assignment
    /// words are skipped (GNU general.c:480 assignment() — the parser
    /// deliberately leaves array-element assignment words in `words`),
    /// then `command` prefixes are peeled exactly the way
    /// execute_cmd.c's execute_simple_command loop peels them through
    /// check_command_builtin (execute_cmd.c:4704-4740): `command [-p]
    /// [--] name` continues at `name` (which may itself be `command`);
    /// any other option word (-v/-V included) or a missing name stops
    /// the peel, leaving `command` itself as the effective word — the
    /// builtin is what runs, with the command's redirections.
    fn effective_command_word(words: &[String]) -> Option<&str> {
        let mut rest = words;
        while let Some((first, tail)) = rest.split_first() {
            // The executor's own assignment() ports (general.c:480):
            // the scalar form and the array-element form.
            let is_assignment = split_assignment_word(first).is_some()
                || super::arrays::is_array_element_assignment_word(first);
            if !is_assignment {
                break;
            }
            rest = tail;
        }
        loop {
            let (first, _) = rest.split_first()?;
            if first != "command" {
                return Some(first);
            }
            let mut next = &rest[1..];
            if next.first().map(String::as_str) == Some("-p") {
                next = &next[1..];
            }
            match next.first() {
                Some(word) if word == "--" => next = &next[1..],
                Some(word) if word.starts_with('-') => return Some(first),
                _ => {}
            }
            if next.is_empty() {
                return Some(first);
            }
            rest = next;
        }
    }

    /// Whether an effective command word executes in the current shell
    /// — a shell function or an ENABLED builtin — so its redirections
    /// (and their heredoc expansion) run in the shell
    /// (execute_cmd.c:5606 execute_builtin_or_function), versus a disk
    /// command, whose redirections run after the fork in the child
    /// (execute_cmd.c:5522 execute_disk_command). A disabled builtin
    /// dispatches to disk in GNU, so it classifies as a child context
    /// here too.
    fn command_word_runs_in_current_shell(&self, name: &str) -> bool {
        if self.function_name_for_command_word(name).is_some() {
            return true;
        }
        // `env` is rubash's one extension builtin (docs/builtins.md
        // section 2): GNU has no env builtin — it is an external
        // program, and rubash's in-process implementation likewise
        // hands its command operand to a spawned program
        // (execute_env_command) — so for heredoc expansion the context
        // follows GNU's dispatch class (the forked child,
        // execute_cmd.c:5522), not the implementation technique. The
        // builtin registration itself (resolution, `type env`) is
        // untouched; this predicate only places the expansion.
        name != "env"
            && self.routes_as_builtin(name)
            && !crate::builtins::enable::is_disabled(&self.shell_state.env_vars, name)
    }

    /// Whether a null command's (no effective word) redirections force
    /// the fork in GNU's execute_null_command (execute_cmd.c): any
    /// redirect targeting fd 0 — every unnumbered heredoc / here-string
    /// / input redirect — or binding a `{fd}` varassign makes the null
    /// command run its do_redirections in a subshell; otherwise they
    /// apply in the current shell (rubash#535's `> $nope` shape: an
    /// fd-1-only null command stays in the shell).
    fn null_command_redirects_force_fork(cmd: &CommandNode) -> bool {
        cmd.heredoc.is_some()
            || cmd.here_string.is_some()
            || cmd.redirects.iter().any(|redirect| {
                redirect.fd_var.is_some()
                    || (matches!(
                        redirect.kind,
                        crate::parser::RedirectKind::HereDoc
                            | crate::parser::RedirectKind::HereString
                            | crate::parser::RedirectKind::Input
                    ) && matches!(redirect.fd, None | Some(0)))
            })
    }

    /// rubash#2615: whether THIS command's own do_redirections walk
    /// runs with RX_UNDOABLE, so its fixed entries' undo saves occupy
    /// descriptor slots while the walk evolves (redir.c:243). GNU's
    /// in-process dispatch (builtins, functions, exec —
    /// execute_builtin_or_function, execute_cmd.c:5606) and the
    /// compound prologue (execute_cmd.c:854) are undoable; the fork
    /// child's walks are RX_ACTIVE only (execute_disk_command,
    /// execute_cmd.c:5522; the subshell's own list,
    /// execute_cmd.c:1761). A subshell-headed node therefore walks
    /// active even though its body later runs in a child executor,
    /// and a compound node without a simple word walks undoable.
    pub(in crate::executor) fn redirect_walk_is_undoable(&mut self, cmd: &CommandNode) -> bool {
        if cmd.subshell_command.is_some() {
            return false;
        }
        if cmd.words.is_empty() {
            return true;
        }
        self.heredoc_error_command_runs_in_main_shell(cmd)
    }

    /// True when the command carrying a failing heredoc would dispatch to
    /// a builtin or function (main-shell redirects, execute_cmd.c:5606)
    /// rather than a forked external child (execute_cmd.c:5884+).
    fn heredoc_error_command_runs_in_main_shell(&mut self, cmd: &CommandNode) -> bool {
        // rubash#3429: a compound command's redirections run in the main
        // shell (GNU execute_cmd.c:854 compound prologue,
        // RX_ACTIVE|RX_UNDOABLE — cm_arith/cm_cond/cm_case/cm_while/
        // cm_until/cm_if/cm_for/cm_group share it, :479-495) — but the
        // node's synthetic words name no builtin (`((` for an arithmetic
        // command; no words at all for a brace group), so the word-based
        // classification below reads them as a forked-child (external)
        // target and contains the fatal jump: the walker has already
        // reported the diagnostic and set status 1, yet the script keeps
        // running (`set -u; (( x = 1 )) > $nope` prints the line and
        // continues). A backgrounded (`&`) or pipelined stage still
        // forks first (execute_cmd.c:2837 / the pipeline children), so
        // those keep the child containment; `!` does not exempt it
        // (GNU aborts). A command inside a subshell keeps the main-shell
        // reading: the Err unwinds to the subshell boundary, which folds
        // it into the subshell's status (compound_exec.rs, the
        // body_result match) — the parent continues, as GNU does.
        if (cmd.arithmetic_command.is_some() || cmd.brace_group.is_some())
            && !cmd.background
            && cmd.background_command.is_none()
            && cmd.pipe.is_none()
            && cmd.pipeline_command.is_none()
        {
            return true;
        }
        // A NULL command (`> $nope`, no words) applies its redirections in
        // the main shell (execute_cmd.c execute_null_command runs
        // do_redirections directly) — the fatal jump is not contained
        // (rubash#535: GNU `set -u; > $nope` ends the script) — unless a
        // redirect forces the null command's fork (fd-0 target or a
        // {fd} varassign; see null_command_redirects_force_fork).
        let Some(first) = cmd.words.first() else {
            return !Self::null_command_redirects_force_fork(cmd);
        };
        // Expansion of the first word can run substitutions with side
        // effects; only classify the literal-word forms where expansion is
        // inert.
        if first.contains('$') || first.contains('`') {
            return false;
        }
        // rubash#2681: classification follows the EFFECTIVE command
        // word — `command cat` forks (its heredoc error is contained,
        // the script continues) while `command :` and `command -v cat`
        // run the `command` builtin itself in the shell. Only the
        // effective word is expanded, and only when it is inert, for
        // the same reason as the first word above.
        let Some(effective) = Self::effective_command_word(&cmd.words) else {
            return !Self::null_command_redirects_force_fork(cmd);
        };
        if effective.contains('$') || effective.contains('`') {
            return false;
        }
        let expanded = self.expand_word(effective);
        // routes_as_builtin already covers the pure builtin table on both
        // platforms (lookup_paths.rs) — testing is_shell_builtin_name
        // separately is a subsumed disjunct (#813 item 4). The shared
        // resolver additionally honors `enable -n` (a disabled builtin
        // dispatches to disk, so its failure is contained).
        self.command_word_runs_in_current_shell(&expanded)
    }

    /// Latched expansion-error flags, checked after word expansion and
    /// again after the stdin-body pre-expansion (GNU do_redirections
    /// order). A fatal word-expansion arithmetic failure in a bare command
    /// (e.g. bare `$((1/0))` with no other words) abandons the rest of the
    /// current command list (GNU Bash 5.2.37 evidence).
    /// rubash#2761: the empty-words path calls this too (from
    /// execute_empty_words_command, right after its pre-expansion),
    /// so it is crate-visible rather than module-private.
    pub(in crate::executor) fn abort_on_expansion_errors(&mut self) -> Result<(), ExecuteError> {
        if self.shell_state.arithmetic_expansion_error.get() {
            self.shell_state.arithmetic_expansion_error.set(false);
            let was_fatal = self.shell_state.arithmetic_fatal_error.replace(false);
            let nounset = self.shell_state.arithmetic_nounset_error.replace(false);
            if nounset {
                // GNU expr.c:1208-1216 expr_streval: interactive_shell
                // takes `jump_to_top_level (DISCARD)` — the command is
                // abandoned with status 1 and the session keeps reading;
                // the FORCE_EOF branch below is noninteractive only.
                if self.expansion_error_is_interactive_discard() {
                    self.note_unit_expansion_error_discard();
                    self.exit_code = 1;
                    // rubash#439: expansion error, not a failed command.
                    self.command_expansion_failed.set(true);
                    return Err(ExecuteError::ExpansionFailure(1));
                }
                // GNU expr.c:1190-1216 expr_streval: an unbound variable
                // under `set -u` set_exit_status(EXECUTION_FAILURE) and
                // raises FORCE_EOF, terminating the noninteractive shell.
                // This mirrors the plain-parameter nounset path
                // (command_prepare.rs), which also exits the script; other
                // arithmetic evaluation errors keep the nonfatal
                // ExpansionFailure line-skip semantics (GNU probe d2:
                // `echo $((1/0)); echo after` still prints "after").
                // The FORCE_EOF status is context-owned: 127 only at the
                // `-c` top-level catch (shell.c:1471); a `( ... )` subshell
                // child contains the jump at execute_cmd.c:1811 and exits
                // 1 (niubash#163).
                let code = self.expansion_fatal_status();
                self.exit_code = code;
                return Err(ExecuteError::ExitCode(code));
            }
            // GNU subst.c:10881-10888: an expok==0 result from $((...)) word
            // expansion is expand_wdesc_fatal when posixly_correct &&
            // !interactive_shell, which subst.c:4296 turns into
            // exp_jump_to_top_level(FORCE_EOF). shell.c:1471 run_one_command
            // maps FORCE_EOF to status 127 for `-c`; eval.c:104-109
            // (reader_loop) sets EOF_Reached so a script-mode shell exits
            // with last_command_exit_value (EXECUTION_FAILURE=1). A subshell
            // child re-arms top_level (execute_cmd.c:1811) and contains the
            // jump to 1 (niubash#163), so the status is owned by
            // expansion_fatal_status.
            if self.posix_mode_enabled() && !self.invocation_interactive() {
                let code = self.expansion_fatal_status();
                self.exit_code = code;
                return Err(ExecuteError::ExitCode(code));
            }
            if was_fatal {
                self.exit_code = 1;
                // rubash#439: expansion error, not a failed command — no
                // ERR trap.
                self.command_expansion_failed.set(true);
                // rubash#1518: the arithmetic evalerror class — the
                // reader's errexit promotion skips it (GNU expr.c
                // evalerror is a pure DISCARD jump in every context).
                self.evalerror_arith_pending.set(true);
                return Err(ExecuteError::ExpansionFailure(1));
            }
            self.exit_code = 1;
        }

        // GNU subst.c:10277-10288: a `bad substitution` raised while
        // expanding a word (including a nested `${}` inside a pattern or
        // alternate word that was actually evaluated) is an
        // expand_word_error — DISCARD for ordinary noninteractive shells,
        // FORCE_EOF when posixly_correct. shell.c:1471 maps FORCE_EOF to
        // 127 under `-c`; eval.c:104-109 ends script input so the shell
        // exits with last_command_exit_value (EXECUTION_FAILURE=1). A
        // subshell child contains the jump at execute_cmd.c:1811 and exits
        // 1 (niubash#163), so expansion_fatal_status owns the mapping.
        // subst.c:10285-10286 gates the fatal arm on the compat level:
        // `if (shell_compatibility_level <= 43) return
        // &expand_wdesc_error;` — at compat31..43 a POSIX-mode bad
        // substitution is the plain error class (command fails with 1,
        // the script continues) even in POSIX mode (audit B4-F08).
        // rubash#1109: the extract_dollar_brace_string no-closing-brace
        // verdict is DISCARD in every mode (see no_closing_brace_verdict).
        if self.no_closing_brace_failure.replace(false) {
            self.exit_code = 1;
            self.command_expansion_failed.set(true);
            return Err(ExecuteError::ExpansionFailure(1));
        }
        if self.shell_state.parameter_bad_substitution.replace(false) {
            if self.posix_mode_enabled()
                && !self.invocation_interactive()
                && crate::builtins::shopt::compatibility_level(&self.shell_state.env_vars) > 43
            {
                let code = self.expansion_fatal_status();
                self.exit_code = code;
                return Err(ExecuteError::ExitCode(code));
            }
            self.exit_code = 1;
            // rubash#439: expansion error, not a failed command — no ERR trap.
            self.command_expansion_failed.set(true);
            return Err(ExecuteError::ExpansionFailure(1));
        }

        // GNU subst.c: a failing `${var:=word}`/`${var=word}` assignment is an
        // expand_word_error; noninteractive shells jump to top level with
        // DISCARD, abandoning the current command list while the script
        // continues.
        if self.parameter_assignment_failure.replace(false) {
            let prev_status = self.parameter_assignment_failure_prev_status.take();
            // rubash#1542: the readonly diagnostic printed from inside
            // bind_variable (variables.c err_readonly -> error.c
            // report_error), and report_error exit_shells on the spot
            // when exit_immediately_on_error is live — with the status
            // from BEFORE the bind (EXECUTION_FAILURE when it was 0),
            // never reaching the EX_BADUSAGE write or the DISCARD. The
            // check reads the DYNAMIC flag: errexit-exempt positions
            // (|| / && elements, `!`, if conditions, function bodies
            // called there) do NOT clear exit_immediately_on_error, so
            // they all die (GNU 5.3.20 probes: exit = prev $? in every
            // such position). The frames that DO clear it are the
            // eval/source CMD_IGNORE_RETURN frame (execute_cmd.c:
            // 5004-5016; rubash#1563 models it as errexit_ignore_frame,
            // read through exit_immediately_on_error) and the comsub
            // child (subst.c:7356-7359, whose option table rubash clears
            // the same way, so errexit_enabled is false there unless
            // inherit_errexit).
            if self.exit_immediately_on_error() {
                let code = self.param_assign_discard_status(2, prev_status.unwrap_or(0));
                self.exit_code = code;
                // rubash#439: expansion error, not a failed command — no
                // ERR trap (report_error exits before any trap site).
                self.command_expansion_failed.set(true);
                return Err(ExecuteError::ExitCode(code));
            }
            // GNU reports expansion failures with EX_BADUSAGE (2), matching
            // `${var?msg}` / bad-substitution status, not the builtin-failure
            // status 1. The pre-set 2 travels with the DISCARD: the catch
            // that receives it (reader, `( )` region, parse_and_execute)
            // decides the final status — folding here at the raise site
            // would hide the 2 from an intervening preserving catch
            // (rubash#1563: `( : ${v:=x} )` inside a comsub body keeps 2).
            self.exit_code = 2;
            // rubash#439: expansion error, not a failed command — no ERR trap.
            self.command_expansion_failed.set(true);
            return Err(ExecuteError::ExpansionFailure(2));
        }
        Ok(())
    }

    /// Expand here-document bodies and the here-string word of a simple
    /// command at the GNU do_redirections point (after word expansion,
    /// before the command runs). The expanded text is stored back with the
    /// StdinBody::Preexpanded typed carrier so the stdin paths return it
    /// verbatim instead of expanding — and re-running embedded substitutions — a
    /// second time. Quoted-delimiter bodies and `\x1d` ANSI-C bodies are
    /// left untouched: they take their own verbatim/decode paths.
    /// rubash#2681: whether this command's heredoc / here-string bodies
    /// expand in a forked child context in GNU. redir.c:348
    /// heredoc_expand runs inside do_redirections, and where
    /// do_redirections runs is decided by the command's dispatch:
    /// execute_disk_command forks first and the CHILD applies the
    /// redirects (execute_cmd.c:5522), while
    /// execute_builtin_or_function applies them in the current shell
    /// (execute_cmd.c:5606). A null command follows
    /// execute_null_command's force-fork rule instead. The bodies'
    /// expansion effects (assignments, arithmetic, function calls via
    /// substitutions) therefore persist exactly for builtin / function
    /// targets and are discarded for external ones.
    fn heredoc_bodies_expand_in_fork_context(&self, cmd: &CommandNode) -> bool {
        if let Some(effective) = Self::effective_command_word(&cmd.words) {
            return !self.command_word_runs_in_current_shell(effective);
        }
        Self::null_command_redirects_force_fork(cmd)
    }

    /// rubash#2681: run `f` — a heredoc / here-string body expansion
    /// for a command GNU would fork — against a throwaway clone of the
    /// shell state: the in-process shape of the fork that precedes
    /// do_redirections in execute_disk_command (execute_cmd.c:5522).
    /// The child inherits the full shell state, expands the bodies
    /// there, and its mutations die with it; the boundary is the same
    /// restore_flat_subshell the `( )` regions and command-substitution
    /// children use (execute_cmd.c:1576 execute_in_subshell), so
    /// variables, functions, aliases, traps, cwd, PIPESTATUS and the
    /// process-env projection all revert together — this is a fork
    /// emulation, not a variable snapshot.
    ///
    /// Two things cross the boundary back, because the parent observes
    /// them from the child: the child's substitution/status bookkeeping
    /// is discarded with it (saved and restored here), but expansion
    /// FAILURE is how the parent learns the child's do_redirections
    /// failed — the arithmetic error latches live in ShellState and
    /// would be wiped by the restore, so they are captured before it
    /// and re-applied after it (abort_on_expansion_errors, immediately
    /// after the pre-expansion, reads them). The
    /// parameter_assignment_failure latch lives on the Executor, not in
    /// ShellState, and already survives the swap untouched.
    pub(in crate::executor) fn with_forked_heredoc_expansion<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let saved_state = self.shell_state.clone();
        let saved_exit_code = self.exit_code;
        let saved_substitution_status = self.last_command_substitution_status.get();
        let saved_substitution_parse_error = self.last_command_substitution_parse_error.get();
        let saved_substitution_parse_error_backtick =
            self.last_command_substitution_parse_error_backtick.get();
        let result = f(self);
        let arithmetic_error = self.shell_state.arithmetic_expansion_error.get();
        let arithmetic_fatal = self.shell_state.arithmetic_fatal_error.get();
        let arithmetic_nounset = self.shell_state.arithmetic_nounset_error.get();
        self.restore_flat_subshell(saved_state);
        self.shell_state
            .arithmetic_expansion_error
            .set(arithmetic_error);
        self.shell_state
            .arithmetic_fatal_error
            .set(arithmetic_fatal);
        self.shell_state
            .arithmetic_nounset_error
            .set(arithmetic_nounset);
        self.exit_code = saved_exit_code;
        self.last_command_substitution_status
            .set(saved_substitution_status);
        self.last_command_substitution_parse_error
            .set(saved_substitution_parse_error);
        self.last_command_substitution_parse_error_backtick
            .set(saved_substitution_parse_error_backtick);
        result
    }

    // rubash#2761: the empty-words path calls this too (from
    // execute_empty_words_command, after its assignment loop), so
    // the funnel is crate-visible rather than module-private.
    pub(in crate::executor) fn preexpand_command_stdin(&mut self, cmd: &mut CommandNode) {
        // rubash#2681: for a command GNU forks, the bodies expand in
        // the child — run the whole pre-expansion (every body of the
        // command, in redirect order, so later bodies still see
        // earlier bodies' effects exactly as the child's single
        // do_redirections pass produces them) inside the fork boundary.
        let has_bodies =
            cmd.heredoc.is_some() || !cmd.heredoc_redirects.is_empty() || cmd.here_string.is_some();
        if has_bodies && self.heredoc_bodies_expand_in_fork_context(cmd) {
            self.with_forked_heredoc_expansion(|executor| {
                executor.preexpand_command_stdin_bodies(cmd)
            });
            return;
        }
        self.preexpand_command_stdin_bodies(cmd);
    }

    fn preexpand_command_stdin_bodies(&mut self, cmd: &mut CommandNode) {
        // GNU do_redirections walks the redirect list in document
        // order, expanding each heredoc body as it is reached — an
        // earlier (even overridden) body's expansion effects are
        // visible to the bodies after it. The parser stores an
        // unnumbered `<<EOF` body in BOTH `heredoc` and
        // `heredoc_redirects` (fd == None); it is one redirection, so
        // expand it once, at its redirect-list position, and share the
        // marked result between the two fields — otherwise embedded
        // substitutions like `${ incr; }` would run twice
        // (comsub23.sub `after here-doc: 1`), and expanding the legacy
        // field ahead of the list would invert the bodies' order
        // (rubash#2681: in `cat <<C <<D`, C's `${a:=fromD}` must be
        // visible when D's body expands).
        let shared_raw = cmd.heredoc.clone();
        let mut shared_carrier: Option<crate::parser::StdinBody> = None;
        for redirect in &mut cmd.heredoc_redirects {
            if let Some(body) = redirect.body.take() {
                if redirect.fd.is_none() && shared_raw.as_deref() == Some(body.as_str()) {
                    let carrier = match &shared_carrier {
                        Some(carrier) => carrier.clone(),
                        None if Self::stdin_body_needs_expansion(&body) => {
                            crate::parser::StdinBody::Preexpanded(
                                self.expand_heredoc_body_mut(&body),
                            )
                        }
                        None => crate::parser::StdinBody::NeedsExpansion(body),
                    };
                    redirect.body = Some(carrier.to_string());
                    redirect.body_carrier = Some(carrier.clone());
                    shared_carrier = Some(carrier);
                    continue;
                }
                let carrier = if Self::stdin_body_needs_expansion(&body) {
                    let expanded = if redirect.here_string {
                        self.expand_here_string_mut(&body)
                    } else {
                        self.expand_heredoc_body_mut(&body)
                    };
                    crate::parser::StdinBody::Preexpanded(expanded)
                } else {
                    crate::parser::StdinBody::NeedsExpansion(body)
                };
                redirect.body = Some(carrier.to_string());
                redirect.body_carrier = Some(carrier);
            }
        }
        if let Some(body) = cmd.heredoc.take() {
            let carrier = match shared_carrier {
                Some(carrier) => carrier,
                None if Self::stdin_body_needs_expansion(&body) => {
                    crate::parser::StdinBody::Preexpanded(self.expand_heredoc_body_mut(&body))
                }
                None => crate::parser::StdinBody::NeedsExpansion(body),
            };
            cmd.heredoc = Some(carrier.to_string());
            cmd.heredoc_body = Some(carrier);
        }
        if let Some(word) = cmd.here_string.take() {
            let carrier = crate::parser::StdinBody::Preexpanded(self.expand_here_string_mut(&word));
            cmd.here_string = Some(carrier.to_string());
            cmd.here_string_carrier = Some(carrier);
        }
    }

    fn stdin_body_needs_expansion(body: &str) -> bool {
        !body.starts_with(crate::lexer::QUOTED_HEREDOC_MARKER)
            && !body.starts_with(STORAGE_WORD_PREFIX)
            && !body.starts_with(PREEXPANDED_STDIN_BODY)
    }
}

pub(in crate::executor) fn bash_style_unexpected_token_message(message: &str) -> String {
    if let Some(token) = message
        .strip_prefix("unexpected token `")
        .and_then(|rest| rest.strip_suffix('`'))
    {
        return format!("unexpected token `{token}'");
    }
    message.to_string()
}

pub(in crate::executor) fn parse_error_source_display(source: &str) -> String {
    // GNU parse.y:6867 print_offending_line echoes the physical input line
    // verbatim — leading whitespace included (`  echo )' keeps its indent)
    // — so do not trim here.
    source
        .replace(";then", "; then")
        .replace("then<W", "then <W")
}

fn unterminated_extglob(raw: &str) -> bool {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut extglob_depth = 0usize;
    let mut quote = None;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            index += 1;
            continue;
        }
        if ch == '\\' {
            index += 2;
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            index += 1;
            continue;
        }
        if matches!(ch, '@' | '*' | '+' | '?' | '!') && chars.get(index + 1) == Some(&'(') {
            extglob_depth += 1;
            index += 2;
            continue;
        }
        if ch == ')' && extglob_depth > 0 {
            extglob_depth -= 1;
        }
        index += 1;
    }
    if extglob_depth > 0 {
        return true;
    }
    false
}
