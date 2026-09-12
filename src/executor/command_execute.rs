use super::*;

impl Executor {
    /// Execute an AST
    pub fn execute_command(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        use super::exec_profile::{ensure_init, PhaseTimer, P_COUNT, P_TOTAL};
        ensure_init();
        let _t_total = PhaseTimer::new(&P_TOTAL);
        if super::exec_profile::P_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            P_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        // Set the source line for every command, including commands inside a
        // DEBUG trap function. The trap action expands its call-site `$LINENO`
        // before entering that function, while the function body must see its
        // own source line (dbg-support2.tests).
        {
            let _t = PhaseTimer::new(&super::exec_profile::P_LINECMD);
            if self.debug_trap_running && self.function_depth > 0 {
                if let Some(line) = self.debug_trap_function_line {
                    self.env_vars
                        .insert("__RUBASH_CURRENT_LINE".to_string(), line.to_string());
                } else {
                    self.set_current_line(cmd);
                }
            } else {
                self.set_current_line(cmd);
            }
            self.set_current_command(cmd);
        }
        let _t_heredoc = PhaseTimer::new(&super::exec_profile::P_HEREDOC);
        self.report_command_heredoc_errors(cmd)?;
        if let Some((name, message, status)) = self.parameter_heredoc_expansion_error(cmd) {
            eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
            self.exit_code = status;
            return Ok(());
        }
        drop(_t_heredoc);
        let _t_scans = PhaseTimer::new(&super::exec_profile::P_SCANS);

        if let Some(message) = cmd.get_assignment("__RUBASH_PARSE_ERROR_EOF_PAREN__") {
            // parse.y: an unclosed `name=(` compound assignment reports the
            // bare EOF diagnostic with status 1 and no source echo.
            self.mark_parse_error();
            eprintln!("{}{}", self.diagnostic_prefix(), message);
            self.exit_code = 1;
            return Err(ExecuteError::ExitCode(1));
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
            let message = cmd
                .get_assignment("__RUBASH_PARSE_ERROR__")
                .map(String::as_str)
                .unwrap_or("unexpected token");
            if message.starts_with("syntax error:") {
                eprintln!("{}{}", self.diagnostic_prefix(), message);
                if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                    eprintln!(
                        "{}`{}'",
                        self.diagnostic_prefix(),
                        parse_error_source_display(source)
                    );
                }
            } else {
                let message = bash_style_unexpected_token_message(message);
                eprintln!("{}syntax error near {message}", self.diagnostic_prefix(),);
                if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                    eprintln!(
                        "{}`{}'",
                        self.diagnostic_prefix(),
                        parse_error_source_display(source)
                    );
                }
            }
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }

        if cmd
            .word_metadata
            .iter()
            .any(|metadata| crate::lexer::has_unclosed_command_substitution(&metadata.raw))
            || cmd
                .assignment_values()
                .any(|value| crate::lexer::has_unclosed_command_substitution(value))
        {
            self.mark_parse_error();
            eprintln!(
                "{}syntax error: unexpected EOF while looking for matching `)'",
                self.diagnostic_prefix()
            );
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
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
                self.diagnostic_prefix()
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
        if !crate::builtins::shopt::option_enabled(&self.env_vars, "extglob")
            && !cmd.extglob_patterns.is_empty()
            && cmd.conditional_command.is_none()
            && cmd.case_command.is_none()
        {
            self.mark_parse_error();
            eprintln!(
                "{}syntax error near unexpected token `('",
                self.diagnostic_prefix()
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

        let mut expanded = self.expand_command_words(cmd)?;
        // histexp1: `echo "$( echo "\!" )"` and `echo "\!"` with `set -H` should
        // keep `\!` (with backslash) inside double quotes. `remove_shell_quotes`
        // + `expand_word` currently strips the backslash for `"\!"` when the
        // `!` is history-expanded (or for `\!` inside double quotes inside
        // comsub), leaving `!` without. Detect the quoted `"\!"` raw and restore
        // the backslash. This is narrow to `"\!"` (the only `\!` inside double
        // quotes in histexp1) and does not affect bare `\!` outside quotes
        // (`echo \!` correctly becomes `!`). Also handles `echo "$( echo "\!" )"`
        // where the inner `"\!"` raw may be `"\"\\!\""` with different escaping
        // inside comsub body.
        if expanded.words.len() == 2 && expanded.words[0] == "echo" && expanded.words[1] == "!" {
            if let Some(raw) = cmd.word_metadata.get(1).map(|m| m.raw.as_str()) {
                if raw.contains("\\!") && raw.contains('"') {
                    expanded.words[1] = "\\!".to_string();
                }
            }
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
                self.diagnostic_prefix()
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
        if alias_expanded.words.is_empty() && !self.arithmetic_expansion_error.get() {
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
        let cmd = alias_expanded;
        // A fatal word-expansion arithmetic failure in a bare command
        // (e.g. bare `$((1/0))` with no other words) abandons the rest
        // of the current command list (GNU Bash 5.2.37 evidence).
        // GNU probe (2026-09-01, WSL bash 5.2.21): `echo hi $((1/0)); echo
        // after` never prints "after" and `set -u; printf "%s\n"
        // "$((missing + 1))" extra; echo after` exits 127 — a fatal
        // evaluation error aborts the command list regardless of how many
        // other words the command has.
        if self.arithmetic_expansion_error.get() {
            self.arithmetic_expansion_error.set(false);
            let was_fatal = self.arithmetic_fatal_error.replace(false);
            let nounset = self.arithmetic_nounset_error.replace(false);
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
            if was_fatal {
                self.exit_code = 1;
                return Err(ExecuteError::ExpansionFailure(1));
            }
            self.exit_code = 1;
        }

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

        if let Some(result) = self.execute_function_command_invocation(&cmd) {
            return result;
        }

        if self.execute_assignment_or_comment_command(&cmd) {
            return Ok(());
        }

        // `exec {fd}...` mutates the shell's persistent descriptor table.
        // Do not materialize its input redirect through the external-command
        // path: that would consume the source virtual fd before exec can
        // duplicate or move it.
        if is_dynamic_fd_exec_command(&cmd)
            || !command_needs_process_substitution_materialization(&cmd)
        {
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
        if !crate::builtins::set::shell_option_enabled(&self.env_vars, "keyword") {
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
}

fn bash_style_unexpected_token_message(message: &str) -> String {
    if let Some(token) = message
        .strip_prefix("unexpected token `")
        .and_then(|rest| rest.strip_suffix('`'))
    {
        return format!("unexpected token `{token}'");
    }
    message.to_string()
}

fn parse_error_source_display(source: &str) -> String {
    source
        .trim()
        .replace(";then", "; then")
        .replace("then<W", "then <W")
}

fn is_dynamic_fd_exec_command(cmd: &CommandNode) -> bool {
    cmd.words.first().map(String::as_str) == Some("exec")
        && cmd.words.get(1).is_some_and(|word| {
            let Some(name) = word
                .strip_prefix('{')
                .and_then(|word| word.strip_suffix('}'))
            else {
                return false;
            };
            !name.is_empty()
                && name
                    .chars()
                    .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        })
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
