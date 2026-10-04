use super::*;
use crate::executor::markers::{PARSE_ERROR_FIELD_SEP, STORAGE_WORD_PREFIX};

impl Executor {
    /// Execute an AST
    pub fn execute_command(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
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
            if self.debug_trap_running && self.shell_state.function_depth > 0 {
                if let Some(line) = self.debug_trap_function_line {
                    self.set_current_line_value(line);
                } else {
                    self.set_current_line(cmd);
                }
            } else {
                self.set_current_line(cmd);
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
                return Err(ExecuteError::ExitCode(self.exit_code));
            }
            return result;
        }

        if cmd.words.is_empty() {
            let result = self.execute_empty_words_command(cmd);
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
            self.mark_parse_error();
            eprintln!(
                "{}syntax error in command substitution",
                self.parser_diagnostic_prefix()
            );
            self.exit_code = 2;
            self.last_command_substitution_status.set(None);
            return Err(ExecuteError::ExitCode(2));
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
                return self.execute_empty_words_command(cmd);
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
                return Err(ExecuteError::ExitCode(self.exit_code));
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
            return self.execute_empty_words_command(&cmd);
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
        if cmd.words.first().map(String::as_str) != Some("exec")
            && self.apply_dynamic_fd_var_redirects(&cmd, true)?
        {
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
                writeln!(
                    &mut stderr,
                    "{}{}",
                    self.diagnostic_prefix(),
                    crate::posix_errors::message(&error)
                )?;
                self.write_redirect_diagnostic_routed(&cmd, &stderr)?;
                self.exit_code = 1;
                if self.errexit_enabled() && self.errexit_is_active() {
                    return Err(ExecuteError::ExitCode(1));
                }
                Ok(())
            }
            other => other,
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
            eprintln!(
                "{}syntax error near {message}",
                self.parser_diagnostic_prefix()
            );
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
                        self.parser_diagnostic_prefix(),
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
            eprintln!("{}{}", self.parser_diagnostic_prefix(), message);
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
                    self.diagnostic_prefix_for_line(warn_line)
                );
            }
            eprintln!(
                "{}syntax error: unexpected end of file from `{compound_name}' command on line {open_line}",
                self.parser_diagnostic_prefix_for_line(eof_line)
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
                eprintln!(
                    "{}{}",
                    self.parser_diagnostic_prefix_for_line(line),
                    message
                );
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
                    self.parser_diagnostic_prefix_for_line(aux_b)
                );
                self.errexit_active_at_diagnostic();
            } else {
                eprintln!(
                    "{}syntax error near `{aux_a}'",
                    self.parser_diagnostic_prefix_for_line(aux_b)
                );
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
                            self.parser_diagnostic_prefix_for_line(aux_b),
                            parse_error_source_display(source)
                        );
                    }
                }
            }
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }

        if let Some(spec) = cmd.get_assignment("__RUBASH_PARSE_ERROR_NEAR__") {
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
                eprintln!(
                    "{}warning: here-document at line {at_line} delimited by end-of-file (wanted `{delimiter}')",
                    self.diagnostic_prefix_for_line(warn_line)
                );
            }
            eprintln!(
                "{}syntax error near unexpected token `{token}'",
                self.parser_diagnostic_prefix_for_line(line)
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
                    eprintln!(
                        "{}`{}'",
                        self.parser_diagnostic_prefix_for_line(line),
                        source
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
            eprintln!(
                "{}syntax error: unexpected end of file",
                self.parser_diagnostic_prefix_for_line(eof_line)
            );
            // Single-line parser_error: GNU error.c:324-327 still
            // exit_shell(2)s right after it under live errexit — record
            // that so eval containment propagates the exit (rubash#306).
            self.errexit_active_at_diagnostic();
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }

        if cmd.has_assignment("__RUBASH_PARSE_ERROR__") {
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
            eprintln!(
                "{}syntax error near unexpected token `('",
                self.parser_diagnostic_prefix()
            );
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }

        // Bash must parse extglob syntax while the extglob option is
        // enabled.  A pathname such as `@(name)` in a simple command is
        // therefore a syntax error when the option is off; treating it as a
        // literal silently accepts malformed scripts.  Conditional RHS
        // patterns are handled separately and intentionally remain eligible
        // for Bash's conditional-pattern semantics.
        if !crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "extglob")
            && !cmd.extglob_patterns.is_empty()
            && cmd.conditional_command.is_none()
            && cmd.case_command.is_none()
        {
            self.mark_parse_error();
            eprintln!(
                "{}syntax error near unexpected token `('",
                self.parser_diagnostic_prefix()
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
        // Find the run of leading array-style assignment words.
        let mut prefix_end = 0usize;
        for index in 0..cmd.words.len() {
            if !command_word_is_array_element_assignment(cmd, index) {
                break;
            }
            prefix_end += 1;
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
            return std::borrow::Cow::Borrowed(cmd);
        }
        // Report the diagnostic for each invalid identifier, mirroring
        // GNU assign_in_env -> sh_invalidid -> builtin_error. The name is
        // everything before `=` (with `+` stripped for append), which for
        // array-style words like `var[0]=X` is `var[0]` -- not a valid
        // identifier because `[` and `]` are not legal variable characters.
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
                let _ = std::io::stderr().write_all(line.as_bytes());
            }
        }
        let mut stripped = cmd.clone();
        stripped.words = cmd.words[prefix_end..].to_vec();
        if cmd.word_metadata.len() > prefix_end {
            stripped.word_metadata = cmd.word_metadata[prefix_end..].to_vec();
        }
        std::borrow::Cow::Owned(stripped)
    }
}

impl Executor {
    fn parameter_heredoc_expansion_error(
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
    fn finish_heredoc_expansion_error(
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
            self.exit_code = 1;
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

    /// True when the command carrying a failing heredoc would dispatch to
    /// a builtin or function (main-shell redirects, execute_cmd.c:5606)
    /// rather than a forked external child (execute_cmd.c:5884+).
    fn heredoc_error_command_runs_in_main_shell(&mut self, cmd: &CommandNode) -> bool {
        let Some(first) = cmd.words.first() else {
            return false;
        };
        // Expansion of the first word can run substitutions with side
        // effects; only classify the literal-word forms where expansion is
        // inert.
        if first.contains('$') || first.contains('`') {
            return false;
        }
        let expanded = self.expand_word(first);
        crate::executor::builtin_names::is_shell_builtin_name(&expanded)
            || self.function_name_for_command_word(&expanded).is_some()
    }

    /// Latched expansion-error flags, checked after word expansion and
    /// again after the stdin-body pre-expansion (GNU do_redirections
    /// order). A fatal word-expansion arithmetic failure in a bare command
    /// (e.g. bare `$((1/0))` with no other words) abandons the rest of the
    /// current command list (GNU Bash 5.2.37 evidence).
    fn abort_on_expansion_errors(&mut self) -> Result<(), ExecuteError> {
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
                    self.exit_code = 1;
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
            if self.posix_mode_enabled()
                && self
                    .shell_state
                    .env_vars
                    .get("__RUBASH_INTERACTIVE")
                    .map(String::as_str)
                    != Some("1")
            {
                let code = self.expansion_fatal_status();
                self.exit_code = code;
                return Err(ExecuteError::ExitCode(code));
            }
            if was_fatal {
                self.exit_code = 1;
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
        if self.shell_state.parameter_bad_substitution.replace(false) {
            if self.posix_mode_enabled()
                && self
                    .shell_state
                    .env_vars
                    .get("__RUBASH_INTERACTIVE")
                    .map(String::as_str)
                    != Some("1")
            {
                let code = self.expansion_fatal_status();
                self.exit_code = code;
                return Err(ExecuteError::ExitCode(code));
            }
            self.exit_code = 1;
            return Err(ExecuteError::ExpansionFailure(1));
        }

        // GNU subst.c: a failing `${var:=word}`/`${var=word}` assignment is an
        // expand_word_error; noninteractive shells jump to top level with
        // DISCARD, abandoning the current command list while the script
        // continues.
        if self.parameter_assignment_failure.replace(false) {
            // GNU reports expansion failures with EX_BADUSAGE (2), matching
            // `${var?msg}` / bad-substitution status, not the builtin-failure
            // status 1.
            self.exit_code = 2;
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
    fn preexpand_command_stdin(&mut self, cmd: &mut CommandNode) {
        // The parser stores an unnumbered `<<EOF` body in BOTH `heredoc` and
        // `heredoc_redirects` (fd == None); it is one redirection, so expand
        // it once and share the marked result between the two fields —
        // otherwise embedded substitutions like `${ incr; }` would run
        // twice (comsub23.sub `after here-doc: 1`).
        let shared_raw = cmd.heredoc.clone();
        if let Some(body) = cmd.heredoc.take() {
            let carrier = if Self::stdin_body_needs_expansion(&body) {
                let expanded = self.expand_heredoc_body_mut(&body);
                crate::parser::StdinBody::Preexpanded(expanded)
            } else {
                crate::parser::StdinBody::NeedsExpansion(body)
            };
            cmd.heredoc = Some(carrier.to_string());
            cmd.heredoc_body = Some(carrier);
        }
        for redirect in &mut cmd.heredoc_redirects {
            if let Some(body) = redirect.body.take() {
                if redirect.fd.is_none() && shared_raw.as_deref() == Some(body.as_str()) {
                    redirect.body = cmd.heredoc.clone();
                    redirect.body_carrier = cmd.heredoc_body.clone();
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
