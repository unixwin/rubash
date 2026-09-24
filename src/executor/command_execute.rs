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
                    self.shell_state
                        .env_vars
                        .insert("__RUBASH_CURRENT_LINE".to_string(), line.to_string());
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
            self.shell_state.stdin_redir.set(
                cmd.redirects.iter().any(redirect_updates_stdin_redir),
            );
        }
        let _t_heredoc = PhaseTimer::new(&super::exec_profile::P_HEREDOC);
        self.report_command_heredoc_errors(cmd)?;
        if let Some((name, message, status)) = self.parameter_heredoc_expansion_error(cmd) {
            let line = format!("{}{}: {}\n", self.diagnostic_prefix(), name, message);
            self.write_default_stderr(line.as_bytes())?;
            self.exit_code = status;
            return Ok(());
        }
        drop(_t_heredoc);
        let _t_scans = PhaseTimer::new(&super::exec_profile::P_SCANS);

        if let Some(message) = cmd.get_assignment("__RUBASH_COMPOUND_SYNTAX_ERROR__") {
            let message = bash_style_unexpected_token_message(message);
            eprintln!(
                "{}syntax error near {message}",
                self.parser_diagnostic_prefix()
            );
            if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                eprintln!(
                    "{}`{}'",
                    self.parser_diagnostic_prefix(),
                    parse_error_source_display(source)
                );
            }
            self.exit_code = 1;
            return Ok(());
        }

        if let Some(message) = cmd.get_assignment("__RUBASH_PARSE_ERROR_EOF_PAREN__") {
            // parse.y: an unclosed `name=(` compound assignment reports the
            // bare EOF diagnostic with status 1 and no source echo.
            self.mark_parse_error();
            eprintln!("{}{}", self.parser_diagnostic_prefix(), message);
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
            }
            if shape == "eof" {
                eprintln!(
                    "{}syntax error: unexpected end of file from `[[' command on line {aux_a}",
                    self.parser_diagnostic_prefix_for_line(aux_b)
                );
            } else {
                eprintln!(
                    "{}syntax error near `{aux_a}'",
                    self.parser_diagnostic_prefix_for_line(aux_b)
                );
                if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                    eprintln!(
                        "{}`{}'",
                        self.parser_diagnostic_prefix_for_line(aux_b),
                        parse_error_source_display(source)
                    );
                }
            }
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
                        return self.execute_ast(&ast);
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
                if crate::lexer::has_unclosed_command_substitution(raw) {
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
                .any(|metadata| unterminated_extglob(&metadata.raw))
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

        drop(_t_scans);
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
            return self.execute_empty_words_command(cmd);
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
            self.exit_code = status;
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
            self.expand_command_words(cmd)?
        };
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
        let original_raws: Vec<Option<&str>> = cmd
            .word_metadata
            .iter()
            .map(|metadata| Some(metadata.raw.as_str()))
            .collect();
        let original_words_had_command_substitution = original_raws
            .iter()
            .flatten()
            .any(|raw| raw.contains("$(") || raw.contains('`'));
        let pre_alias_words = expanded.words.clone();
        let alias_expanded =
            self.apply_alias_expansion_after_word_expansion(expanded, &original_raws);
        let alias_expansion_changed_words = alias_expanded.words != pre_alias_words;
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
            }
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

        if let Some(result) = self.execute_function_command_invocation(&cmd) {
            return result;
        }

        if self.execute_assignment_or_comment_command(&cmd) {
            return Ok(());
        }

        if !command_needs_process_substitution_materialization(&cmd) {
            return self.execute_materialized_command(&cmd, ProcessSubstitutionFiles::default());
        }

        let (materialized_cmd, process_substitution_files) =
            self.command_with_process_substitution_files(&cmd)?;
        self.execute_materialized_command(&materialized_cmd, process_substitution_files)
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
    fn strip_invalid_env_assignment_prefixes(&mut self, cmd: &CommandNode) -> CommandNode {
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
        if prefix_end == 0 || prefix_end >= cmd.words.len() {
            return cmd.clone();
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
        stripped
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
                // GNU expr.c expr_streval: an unbound variable under `set -u`
                // raises FORCE_EOF and terminates the noninteractive shell.
                // This mirrors the plain-parameter nounset path
                // (command_prepare.rs), which also exits the script; other
                // arithmetic evaluation errors keep the nonfatal
                // ExpansionFailure line-skip semantics (GNU probe d2:
                // `echo $((1/0)); echo after` still prints "after").
                self.exit_code = 127;
                return Err(ExecuteError::ExitCode(127));
            }
            // GNU subst.c:10881-10888: an expok==0 result from $((...)) word
            // expansion is expand_wdesc_fatal when posixly_correct &&
            // !interactive_shell, which subst.c:4296 turns into
            // exp_jump_to_top_level(FORCE_EOF). shell.c:1471 run_one_command
            // maps FORCE_EOF to status 127 for `-c`; eval.c:104-109
            // (reader_loop) sets EOF_Reached so a script-mode shell exits
            // with last_command_exit_value (EXECUTION_FAILURE=1).
            if self.posix_mode_enabled()
                && self
                    .shell_state
                    .env_vars
                    .get("__RUBASH_INTERACTIVE")
                    .map(String::as_str)
                    != Some("1")
            {
                let code = if self.shell_state.env_vars.get("__RUBASH_IS_C").is_some() {
                    127
                } else {
                    1
                };
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
        // exits with last_command_exit_value (EXECUTION_FAILURE=1).
        if self.shell_state.parameter_bad_substitution.replace(false) {
            if self.posix_mode_enabled()
                && self
                    .shell_state
                    .env_vars
                    .get("__RUBASH_INTERACTIVE")
                    .map(String::as_str)
                    != Some("1")
            {
                let code = if self.shell_state.env_vars.get("__RUBASH_IS_C").is_some() {
                    127
                } else {
                    1
                };
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
    source
        .trim()
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
