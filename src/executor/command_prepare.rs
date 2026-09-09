use super::glob::{pathname_expand_word, PathnameExpansion};
use super::*;

fn materialize_expanded_command_word(word: &str) -> String {
    decode_command_substitution_payload(&restore_pathname_escape_markers(
        &word.replace('\x15', "\\").replace('\x14', "\\"),
    ))
}

#[allow(dead_code)]
/// GNU subst.c::param_expand reaches `bad_substitution:` for a whitespace-led
/// `${ command; }` word and reports the FULL word text under expansion
/// (subst.c:10278 `report_error (_("%s: bad substitution"), string)`), not
/// just the braced span. parameter_errors.rs reports the span only, so
/// whole-word occurrences (`AA${ cmd; }BB`, `--${ }--`) pre-empt it here with
/// the exact GNU text; the expansion failure abandons the current command
/// with status 1 while the script continues. `${| ... }` stays a Rubash
/// current-shell extension and never takes this path.
fn whitespace_led_bad_substitution_word(word: &str) -> Option<String> {
    let word = word
        .strip_prefix('\x1b')
        .or_else(|| word.strip_prefix('\x1d'))
        .unwrap_or(word);
    if word.contains("${|") {
        return None;
    }
    let mut rest = word;
    while let Some(start) = rest.find("${") {
        let after_start = &rest[start + 2..];
        let Some(end) = matching_parameter_brace(after_start) else {
            return None;
        };
        if after_start.chars().next().is_some_and(char::is_whitespace) {
            return Some(word.to_string());
        }
        rest = &after_start[end + 1..];
    }
    None
}

impl Executor {
    /// Shared whole-word bad-substitution pre-check; see
    /// whitespace_led_bad_substitution_word for the GNU source mapping.
    /// Dead since the 5.3 funsub flip; kept for the line-number notes.
    #[allow(dead_code)]
    pub(in crate::executor) fn report_whitespace_led_bad_substitution(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        for word in cmd.words.iter().chain(cmd.assignment_values()) {
            let Some(bad_word) = whitespace_led_bad_substitution_word(word) else {
                continue;
            };
            // GNU parse.y stamps each simple command with the line number
            // current when a word token completes (read_token_word,
            // parse.y:5849-5851) and execute_cmd.c reports that value
            // (SET_LINE_NUMBER at execute_cmd.c:936). Because the lexer has
            // crossed into later physical lines while scanning a multi-line
            // word, the reported line advances by the newlines the offending
            // word spans (comsub2.tests:23-24 reports line 24, :25-27 reports
            // line 27). The one exception observed in the suites is a word
            // with an EMBEDDED `${` spanning exactly one newline
            // (`echo blank --${ \n}--`, comsub.tests:84-85), which reports
            // the command's starting line, so the span is not added there.
            let start_line = cmd.line.unwrap_or(1);
            let word_newlines = bad_word.matches('\n').count();
            let embedded_single_newline =
                !bad_word.starts_with("${") && word_newlines == 1;
            let diag_line = if embedded_single_newline {
                start_line
            } else {
                start_line + word_newlines
            };
            // One write per diagnostic, mirroring GNU bash's report_error
            // (error.c): the message must not be split across write calls or
            // a redirected stderr can interleave other output inside it.
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "{}{}: bad substitution",
                self.diagnostic_prefix_for_line(diag_line),
                bad_word
            )?;
            self.write_default_stderr(&stderr)?;
            self.exit_code = 1;
            return Err(ExecuteError::ExpansionFailure(1));
        }
        Ok(())
    }
}

impl Executor {
    pub(in crate::executor) fn report_command_heredoc_errors(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if cmd.subshell && command_has_unterminated_heredoc(cmd) {
            self.mark_parse_error();
            self.report_unterminated_subshell_heredoc(cmd);
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }
        if command_has_unterminated_heredoc(cmd) {
            self.report_unterminated_heredoc(cmd);
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_initial_command_node(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<Result<(), ExecuteError>> {
        if command_is_time_prefixed_compound(cmd) {
            return Some(self.execute_time_prefixed_compound_command(cmd));
        }
        if let Some(for_command) = &cmd.for_command {
            return Some(self.execute_for_command_with_redirects(for_command, cmd));
        }
        if let Some(if_command) = &cmd.if_command {
            return Some(self.execute_if_command_with_redirects(cmd, if_command));
        }
        if let Some(loop_command) = &cmd.loop_command {
            return Some(self.execute_loop_command_with_redirects(cmd, loop_command));
        }
        if let Some(subshell_command) = &cmd.subshell_command {
            return Some(self.execute_subshell_command_with_redirects(cmd, subshell_command));
        }
        if let Some(select_command) = &cmd.select_command {
            return Some(self.execute_select_command(cmd, select_command));
        }
        if let Some(case_command) = &cmd.case_command {
            return Some(self.execute_case_command_with_redirects(cmd, case_command));
        }
        if let Some(coproc_cmd) = &cmd.coproc_command {
            return Some(self.execute_coproc_command(cmd, coproc_cmd));
        }
        if let Some(conditional_command) = &cmd.conditional_command {
            return Some(self.execute_conditional_command_with_redirects(cmd, conditional_command));
        }
        if let Some(function_command) = &cmd.function_command {
            return Some(self.define_function(cmd, function_command));
        }
        None
    }

    fn execute_conditional_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        conditional_command: &ConditionalCommand,
    ) -> Result<(), ExecuteError> {
        self.apply_no_output_builtin_redirects(cmd)?;
        self.exit_code = self.execute_conditional_command(conditional_command);
        Ok(())
    }

    pub(in crate::executor) fn execute_empty_words_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if command_has_no_effect(cmd) {
            return Ok(());
        }
        if let Some((name, message)) = self.parameter_assignment_error(cmd) {
            eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
            self.exit_code = 1;
            // GNU Bash 5.2 subst.c:10404-10410: `${special=word}` on a
            // special/positional param reports "$N: cannot assign in this
            // way" and returns &expand_wdesc_error (NON-fatal); err_readonly
            // for `${ro=word}` likewise leaves the shell alive. Both reach
            // call_expand_word_internal (subst.c:4288-4296) as
            // expand_word_error -> exp_jump_to_top_level(DISCARD): abandon the
            // current command list, keep running the script. ExitCode(1) here
            // bubbled to the top and terminated the whole script.
            return Err(ExecuteError::ExpansionFailure(1));
        }
        // Bash 5.3 (parser.h FUNSUB_CHAR) executes whitespace-led
        // `${ command; }` as a foreground current-shell command
        // substitution, so the 5.2-era whole-word `bad substitution`
        // pre-check no longer applies.
        if let Some((name, message, status)) = self.parameter_expansion_error(cmd) {
            eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
            self.exit_code = status;
            if status == 1 {
                // GNU Bash 5.2: bad substitution and `substring expression
                // < 0` are non-fatal word-expansion errors - they abandon
                // the current command list with status 1, but the next
                // line still runs (probe 2026-09-02: `echo ${#:}; echo
                // after` prints `after`; cf. t25 failglob). ExitCode(1)
                // here made them abort the whole script at top level.
                return Err(ExecuteError::ExpansionFailure(status));
            }
            return Err(ExecuteError::ExitCode(status));
        }
        if self.xtrace_enabled() {
            let prefix = self.xtrace_prefix();
            let text = self.xtrace_command_text(cmd);
            eprintln!("{prefix}{text}");
        }
        let mut status = 0;
        for (name, value) in &cmd.assignments {
            let assignment_result = self.expand_assignment_value_result(value);
            let expanded_value = assignment_result.value;
            let substitution_status = assignment_result.substitution_status;
            if assignment_result.arithmetic_error && !assignment_result.arithmetic_nonfatal_error {
                // A fatal arithmetic expansion error in an assignment word
                // aborts the current command list in Bash. Do not install a
                // partial assignment or let the AST walker skip only the next
                // command as if this were an ordinary word expansion.
                // `set -u` unbound is script-fatal (GNU expr.c expr_streval
                // FORCE_EOF; probe a1: `set -u; x=$((b)); echo after` never
                // prints "after"), matching the plain-parameter nounset path;
                // other fatal categories abandon only the command list
                // (probe a6: `x=$((1/0)); echo after` prints "after").
                if self.arithmetic_nounset_error.replace(false) {
                    self.exit_code = 127;
                    return Err(ExecuteError::ExitCode(127));
                }
                self.exit_code = 1;
                return Err(ExecuteError::ExpansionFailure(1));
            }
            if assignment_result.arithmetic_error {
                // Nonfatal arithmetic errors (e.g. "attempted assignment to
                // non-variable" inside a ternary branch) report the diagnostic
                // on stderr and continue without aborting the command list
                // (GNU Bash 5.2: `y=$((1 ? 20 : x+=2))` continues).
                self.exit_code = 1;
            }
            if let Some(substitution_status) = substitution_status {
                status = substitution_status;
            }
            self.pending_scalar_assignment =
                expanded_value.starts_with('\x1d') && value.contains(['$', '`']);
            if !self.apply_shell_assignment(name, expanded_value) {
                status = 1;
                let (base_name, _) = assignment_name_and_append(name);
                if is_marked_var(&self.env_vars, READONLY_VARS, base_name)
                    && !special_readonly_assignment_is_recoverable(base_name)
                {
                    self.exit_code = 1;
                    let script_mode_nonfatal = self.env_vars.contains_key("__RUBASH_SCRIPT_NAME")
                        && (!self.errexit_enabled() || !self.errexit_is_active());
                    if !script_mode_nonfatal {
                        return Err(ExecuteError::ExitCode(1));
                    }
                }
            }
        }
        match self.apply_no_output_builtin_redirects_with_status(cmd) {
            Ok(redirect_failed) => {
                if redirect_failed {
                    status = 1;
                }
            }
            Err(ExecuteError::IoError(error)) => {
                let mut stderr = Vec::new();
                let redirect_target = [
                    cmd.redirect_in.as_ref(),
                    cmd.redirect_out.as_ref(),
                    cmd.append.as_ref(),
                    cmd.redirect_err.as_ref(),
                    cmd.redirect_err_append.as_ref(),
                ]
                .into_iter()
                .flatten()
                .map(|redirect| self.expand_word(&redirect.target))
                .find(|target| {
                    cfg!(windows)
                        && (target.contains('\\')
                            || contains_windows_forbidden_posix_filename_char(target))
                });
                if let Some(target) = redirect_target {
                    writeln!(
                        &mut stderr,
                        "{}{}: No such file or directory",
                        self.diagnostic_prefix(),
                        target
                    )?;
                } else {
                    writeln!(
                        &mut stderr,
                        "{}{}",
                        self.diagnostic_prefix(),
                        crate::posix_errors::message(&error)
                    )?;
                }
                self.write_default_stderr(&stderr)?;
                status = 1;
            }
            Err(error) => return Err(error),
        }
        self.exit_code = status;
        if self.errexit_enabled() && self.errexit_is_active() && self.exit_code != 0 {
            return Err(ExecuteError::ExitCode(self.exit_code));
        }
        Ok(())
    }

    pub(in crate::executor) fn validate_command_parameter_expansions(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if let Some((name, message)) = self.parameter_assignment_error(cmd) {
            eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
            self.exit_code = 1;
            // Same GNU DISCARD class as execute_empty_words_command above:
            // subst.c:10404-10410 expand_wdesc_error (non-fatal) for
            // `${special=word}` / `${ro=word}` readonly; the script keeps
            // running after the diagnostic.
            return Err(ExecuteError::ExpansionFailure(1));
        }
        // Bash 5.3 (parser.h FUNSUB_CHAR) executes whitespace-led
        // `${ command; }` as a foreground current-shell command
        // substitution, so the 5.2-era whole-word `bad substitution`
        // pre-check no longer applies.
        if let Some((name, message, status)) = self.parameter_expansion_error(cmd) {
            eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
            self.exit_code = status;
            if status == 1 {
                // GNU 5.2 non-fatal word-expansion errors (bad substitution,
                // substring < 0): abandon this command list with status 1;
                // the next line still runs (same semantics as t25 failglob).
                return Err(ExecuteError::ExpansionFailure(status));
            }
            return Err(ExecuteError::ExitCode(status));
        }
        Ok(())
    }

    pub(in crate::executor) fn expand_command_words(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<CommandNode, ExecuteError> {
        let preserve_word_metadata = cmd.conditional_command.is_some()
            || cmd.words.first().is_some_and(|word| word == "[[")
            || !cmd.process_substitutions.is_empty()
            || cmd.word_metadata.iter().any(|metadata| {
                !metadata.process_substitutions.is_empty()
                    || metadata.raw.contains("<(")
                    || metadata.raw.contains(">(")
            });
        let mut variable_expanded = CommandNode {
            words: Vec::new(),
            word_metadata: if preserve_word_metadata {
                cmd.word_metadata.clone()
            } else {
                Vec::new()
            },
            word_kinds: Vec::new(),
            assignments: cmd.assignments.clone(),
            compound_assignments: cmd.compound_assignments.clone(),
            array_element_assignments: cmd.array_element_assignments.clone(),
            process_substitutions: cmd.process_substitutions.clone(),
            redirects: cmd.redirects.clone(),
            redirect_in: cmd.redirect_in.clone(),
            redirect_out: cmd.redirect_out.clone(),
            append: cmd.append.clone(),
            redirect_err: cmd.redirect_err.clone(),
            redirect_err_append: cmd.redirect_err_append.clone(),
            heredoc: cmd.heredoc.clone(),
            heredoc_delimiter: cmd.heredoc_delimiter.clone(),
            heredoc_redirects: cmd.heredoc_redirects.clone(),
            here_string: cmd.here_string.clone(),
            pipe: cmd.pipe,
            background: cmd.background,
            and_or: cmd.and_or,
            inverted: cmd.inverted,
            arithmetic_command: cmd.arithmetic_command.clone(),
            conditional_command: cmd.conditional_command.clone(),
            brace_group: cmd.brace_group.clone(),
            line: cmd.line,
            ..CommandNode::new()
        };
        let expanded_words = cmd
            .words
            .iter()
            .enumerate()
            .flat_map(|(index, word)| {
                let metadata = cmd.word_metadata.get(index);
                let raw = metadata.map(|metadata| metadata.raw.as_str());
                let suppress_glob = assignment_builtin_receives_assignment_word(cmd, index, word)
                    || word.starts_with('\x1b')
                    || word.starts_with('\x1d')
                    || raw_word_suppresses_pathname_expansion(raw, metadata);
                self.expand_command_word(cmd, index, word, raw)
                    .into_iter()
                    .map(move |word| (word, suppress_glob))
            })
            .collect::<Vec<_>>();
        variable_expanded.words = expanded_words
            .iter()
            .map(|(word, _)| materialize_expanded_command_word(word))
            .collect();
        variable_expanded.word_kinds = Vec::new();

        let is_test_cmd = cmd.words.first().is_some_and(|w| w == "[[" || w == "[");
        if !is_test_cmd {
            let mut words = Vec::new();
            for (word, suppress_glob) in expanded_words {
                if suppress_glob {
                    words.push(materialize_expanded_command_word(&word).replace('\x17', "'"));
                } else {
                    match pathname_expand_word(&word, &self.env_vars) {
                        PathnameExpansion::Matches(matches) => words
                            .extend(matches.into_iter().map(|value| value.replace('\x17', "'"))),
                        PathnameExpansion::NoMatch => words
                            .push(materialize_expanded_command_word(&word).replace('\x17', "'")),
                        PathnameExpansion::Fail(pattern) => {
                            self.report_failglob(&pattern);
                            // GNU failglob is a fatal word-expansion error:
                            // it abandons the rest of the current command
                            // list with status 1 but the next line still runs
                            // (probe 2026-09-02: `echo f[12] nope*; echo x`
                            // skips x, the next line prints). ExitCode(1) made
                            // it abort the whole script at top level
                            // (glob.tests line 67 truncated later output).
                            return Err(ExecuteError::ExpansionFailure(1));
                        }
                    }
                }
            }
            variable_expanded.words = words;
        }
        Ok(variable_expanded)
    }

    fn expand_unquoted_parameter_transform_word(&self, word: &str) -> Option<String> {
        let start = word.find("${")?;
        let end = word[start..].find('}')? + start;
        let inner = &word[start + 2..end];
        if !(inner.ends_with("@Q") || inner.ends_with("@K")) {
            return None;
        }
        let value = self.expand_braced_transform_parameter(inner)?;
        let prefix = self.expand_word(&word[..start]);
        let suffix = self.expand_word(&word[end + 1..]);
        Some(format!("{prefix}{value}{suffix}"))
    }

    fn expand_simple_substitution_fragments(
        &mut self,
        cmd: &CommandNode,
        index: usize,
        word: &str,
        raw: &str,
    ) -> Option<Vec<String>> {
        let raw_fragments = split_raw_word_fragments(raw);
        if raw_fragments.len() < 3
            || !raw_fragments.iter().any(|fragment| fragment.substitution)
            || !raw_fragments
                .iter()
                .any(|fragment| !fragment.substitution && !fragment.text.is_empty())
        {
            return None;
        }
        let substitution_start = raw_fragments
            .iter()
            .position(|fragment| fragment.substitution)?;
        if raw_fragments[..substitution_start]
            .iter()
            .any(|fragment| fragment.text.contains('='))
        {
            return None;
        }
        if raw_fragments
            .iter()
            .any(|fragment| !fragment.substitution && fragment.text.contains(['$', '`', '{', '}']))
        {
            return None;
        }
        let mut expanded_fragments = Vec::new();
        for fragment in raw_fragments {
            if fragment.substitution {
                let context = fragment
                    .context
                    .unwrap_or(SubstitutionQuoteContext::Unquoted);
                let source = fragment.text.strip_prefix("$(")?.strip_suffix(')')?;
                let output =
                    self.expand_command_substitution_mut_typed_with_context(source, context);
                expanded_fragments.push(output.into_fragment());
            } else {
                let literal = crate::lexer::remove_shell_quotes(&fragment.text);
                if !literal.is_empty() {
                    expanded_fragments.push(ExpandedFragment::literal(&literal, false));
                }
            }
        }
        let policy = if self.splits_unquoted_expanded_word(cmd, index, word) {
            SubstitutionSplitPolicy::Split
        } else {
            SubstitutionSplitPolicy::NoSplit
        };
        Some(split_expanded_fragments(
            &expanded_fragments,
            self.env_vars.get("IFS").map(String::as_str),
            policy,
        ))
    }

    pub(in crate::executor) fn expand_command_word(
        &mut self,
        cmd: &CommandNode,
        index: usize,
        word: &str,
        raw: Option<&str>,
    ) -> Vec<String> {
        // Assignment operators inside parameter expansions take effect at
        // the point where their word is expanded. Applying them to every
        // command word up front changes Bash's left-to-right semantics.
        self.apply_parameter_assignment_expansions_in_word(word);
        // GNU subst.c materializes a complete quoted arithmetic expansion before
        // word splitting. Keep it out of the legacy `$()` materializer, which
        // otherwise treats the inner `+` as a command word.
        if let Some(expression) = raw.and_then(|raw| {
            raw.strip_prefix("\"$((")
                .and_then(|rest| rest.strip_suffix("))\""))
        }) {
            if let Some(value) = self.eval_arithmetic_expansion_value(expression) {
                return vec![value.to_string()];
            }
            self.arithmetic_expansion_error.set(true);
            // GNU expr.c raises evalerror from the actual evaluation;
            // classify from the recorded real-environment category.
            let actual_fatal = self.arithmetic_last_error_category.take().is_some();
            if actual_fatal
                || crate::executor::arithmetic::arithmetic_expansion_is_fatal(expression)
            {
                self.arithmetic_fatal_error.set(true);
            } else {
                self.arithmetic_nonfatal_error.set(true);
            }
            return Vec::new();
        }
        if let Some(source) = raw.and_then(|raw| {
            raw.strip_prefix("\"$(")
                .and_then(|rest| rest.strip_suffix(")\""))
                // GNU subst.c expands each $() span inside the quoted word on
                // its own ("$(a)$(b)" is two substitutions, not one command
                // whose text runs from the first $( to the last )). Strip the
                // fast path only when the quoted content is exactly one
                // $() group; multi-span words fall through to the fragment
                // and embedded-parameter expanders (issue niubash#71).
                .filter(|_| {
                    raw.get(1..raw.len() - 1)
                        .is_some_and(command_substitution_spans_whole_word)
                })
        }) {
            return vec![self.expand_command_substitution_with_context(
                source,
                SubstitutionQuoteContext::DoubleQuoted,
            )];
        }
        if let Some(raw) = raw {
            if let Some(values) = self.expand_simple_substitution_fragments(cmd, index, word, raw) {
                return values;
            }
        }
        if let Some(raw_substitution) = raw
            .filter(|raw| raw.starts_with("$(") && raw.ends_with(')'))
            .filter(|raw| {
                raw[2..raw.len() - 1]
                    .split_whitespace()
                    .next()
                    .is_some_and(|name| self.functions.contains_key(name))
            })
        {
            let context = scan_substitution_spans(raw_substitution)
                .first()
                .map(|span| span.context)
                .unwrap_or(SubstitutionQuoteContext::Unquoted);
            let expanded =
                self.expand_embedded_parameters_mut_with_context(raw_substitution, context);
            if self.splits_unquoted_expanded_word(cmd, index, &expanded) {
                return field_split_escaped_ifs(
                    &expanded,
                    self.env_vars.get("IFS").map(String::as_str),
                );
            }
            return vec![expanded];
        }
        if cmd
            .process_substitutions
            .iter()
            .any(|process| process.word_index == Some(index) && process.target == word)
        {
            return vec![word.to_string()];
        }
        // A fully single-quoted raw word has already had its outer quotes
        // removed by the lexer. Any double quotes left in its value are
        // literal data, including inside parameter alternate words. The
        // leading \x1b quoted-tilde marker (word.rs) has already served its
        // purpose here: tilde expansion never runs on this literal path and
        // the caller read it for suppress_glob, so it must not leak into
        // builtin arguments (dstack2/tilde `printf %q '~'`).
        if raw_word_is_fully_single_quoted(raw) {
            let word = word.strip_prefix('\x1b').unwrap_or(word);
            return vec![word.replace('\x1f', "$")];
        }
        if !word.starts_with('\x1d') {
            if let Some(values) = self.braced_alternate_word_values(word, raw) {
                return values;
            }
        }
        if let Some(values) = self.array_at_word_values(word) {
            if word_is_unquoted_array_list_expansion(word) {
                return field_split_array_values_with_ifs(
                    values,
                    self.env_vars.get("IFS").map(String::as_str),
                );
            }
            return values;
        }
        // Unquoted `$@` expands to one word per positional parameter
        // (quoted `"$@"` is handled by quoted_positional_at_word_values_with_raw).
        if word == "$@" && !raw_word_is_quoted(raw) {
            return field_split_positional_values_with_ifs(
                self.positional_params.clone(),
                self.env_vars.get("IFS").map(String::as_str),
            );
        }
        // Unquoted `$*` with a set-empty IFS: Posix interp 888 dispatches
        // to the dollar_at word list (subst.c string_list_pos_params:3047-
        // 3048). The W_SPLITSPACE join at subst.c:10664-10668 runs over
        // list_quote_escapes-protected elements, so each element's internal
        // spaces survive the re-split: the net result is one word per
        // positional parameter, preserved verbatim (exp10.sub `${*}` with
        // `set -- ' A ' ' B '`).
        if word == "${*}" && !raw_word_is_quoted(raw)
            && self.env_vars.get("IFS").is_some_and(|ifs| ifs.is_empty())
        {
            return self.positional_params.clone();
        }
        if word == "$*" && !raw_word_is_quoted(raw)
            && self.env_vars.get("IFS").is_some_and(|ifs| ifs.is_empty())
        {
            return self.positional_params.clone();
        }
        if let Some(values) =
            self.quoted_positional_at_word_values_with_raw(word, raw, cmd.word_kinds.get(index))
        {
            if self.word_is_unquoted_positional_modified_list_expansion(word)
                || self.word_is_unquoted_positional_list_expansion(word)
            {
                return field_split_positional_values_with_ifs(
                    values,
                    self.env_vars.get("IFS").map(String::as_str),
                );
            }
            return values;
        }
        // A fully double-quoted `${op...}` word whose alternate embeds `$@`
        // (e.g. `"${1+  $@  }"`) is skipped by braced_alternate_word_values
        // above (its \x1d marker), but GNU still expands the alternate with
        // the quoted-$@ word-boundary semantics: affixes attach to the
        // first/last positional word, one word per parameter (subst.c
        // param_expand carries `quoted` into the alternate word).
        if word.starts_with('\x1d') {
            if let Some(values) = self.quoted_braced_alternate_positional_at_values(word) {
                return values;
            }
        }
        // GNU bash runs brace expansion once, on the original word, before
        // any other expansion, and never re-expands expansion results. raw
        // is None only for already-expanded fragments (the brace-result loop
        // below), so their literal braces must not expand again here.
        // A leading '\x1d' marks a fully double-quoted word (word.rs), and a
        // fully quoted word never brace-expands in GNU. The raw-based
        // expander below rebuilds its results from raw, which never carries
        // the marker, so letting it run here drops the marker and the
        // recursive call (raw = None) re-expands the parameter as unquoted:
        // the quotes inside a double-quoted alternate become real single
        // quotes and $key stops expanding (posixexp2 cases 24/38).
        // A double-quoted $@ or $* is excluded: those reach the per-positional
        // split through the raw-based path, and skipping it here collapsed
        // "${1+  $@  }" to one word (exp suite).
        let quoted_whole_word = word.starts_with('\x1d')
            && !word.contains("$@")
            && !word.contains("$*");
        if !quoted_whole_word && raw.is_some() && self.is_brace_expand_enabled()
        // GNU runs brace expansion before parameter expansion, so a
        // dollar-brace in the word does not suppress it: the dollar-brace
        // body is skipped by the brace scanner (foo{bar,${var.} splits).
        // Quoted raws are safe too: the brace scanner is quote-aware, so
        // "{a,b}" stays literal while {a,b}'q' still splits.
        {
            let braced = expand_braces_with_optional_raw(word, raw);
            if braced.len() > 1 || (braced.len() == 1 && braced[0] != word) {
                let mut values = Vec::new();
                for expanded in braced {
                    values.extend(self.expand_command_word(cmd, index, &expanded, None));
                }
                return values;
            }
        }
        if raw_word_is_quoted(raw) {
            if let Some(output) = self.expand_backtick_substitution_typed(word, true) {
                let expanded = output.text_lossy();
                if expanded.is_empty() && self.removes_unquoted_null_word(cmd, index) {
                    return Vec::new();
                }
                return vec![expanded];
            }
        }
        // scan_substitution_spans only sees $()/backtick spans, so a plain
        // double-quoted "${...}" word reports Unquoted. Restore the lexer's
        //  quote marker for the whole-word braced form so the operator
        // expander can tell "${v:-~}" (no tilde) from ${v:-~} (tilde).
        let word = if raw.is_some_and(|raw| raw.starts_with('"') && raw.ends_with('"'))
            && !word.starts_with('')
            && word.starts_with("${")
            && word.ends_with('}')
        {
            format!("{word}")
        } else {
            word.to_string()
        };
        let word = word.as_str();
        // scan_substitution_spans only sees $()/backtick spans, so a
        // plain double-quoted braced word reports Unquoted. The \x1d marker
        // is authoritative instead: a fully double-quoted word keeps
        // inner quotes literal, which is what makes ${IFS+'$key'}
        // expanded inside double quotes, yield 'value' rather than
        // the literal $key (posixexp2 cases 24/38).
        let context = if word.starts_with('\x1d') {
            SubstitutionQuoteContext::DoubleQuoted
        } else {
            raw
                .map(scan_substitution_spans)
                .filter(|spans| spans.len() == 1)
                .and_then(|spans| spans.first().map(|span| span.context))
                .unwrap_or(SubstitutionQuoteContext::Unquoted)
        };
        let expanded = self.expand_word_mut_with_context(word, context);
        // GNU does not apply quote removal to parameter-expansion results:
        // quotes in an expanded value are literal data, not syntax. Calling
        // remove_shell_quotes here dropped a trailing quote such as the x'
        // produced by a braced pattern substitution (foo%*'a'* with foo=x'a'y)
        // under set -o posix. Only the original lexer tokens get quote removal.
        if cmd
            .array_element_assignments
            .iter()
            .any(|assignment| assignment.word_index == Some(index))
        {
            if let Some(raw_value) = raw
                .and_then(|raw| raw.split_once('=').map(|(_, value)| value))
                .and_then(|value| {
                    value
                        .strip_prefix('\"')
                        .and_then(|value| value.strip_suffix('\"'))
                })
            {
                if let Some((left, _)) = expanded.split_once('=') {
                    return vec![format!(
                        "{left}={}",
                        self.expand_quoted_parameter_word(raw_value)
                    )];
                }
            }
        }
        if assignment_builtin_receives_assignment_word(cmd, index, word) {
            return vec![strip_assignment_builtin_command_subst_quotes(
                &expanded, raw,
            )];
        }
        if let Some(formatted) = self.expand_unquoted_parameter_transform_word(word) {
            return vec![formatted];
        }
        if expanded.is_empty() && self.removes_unquoted_null_word(cmd, index) {
            Vec::new()
        } else if raw_word_contains_process_substitution(raw)
            && expanded_word_has_process_substitution(&expanded)
        {
            vec![expanded]
        } else if let Some(values) = self.field_split_word_with_quoted_empty_suffix(raw, &expanded)
        {
            values
        } else if self.splits_unquoted_expanded_word(
            cmd,
            index,
            &decode_command_substitution_payload(&expanded),
        ) {
            // Command-substitution output arrives payload-protected (control
            // bytes such as \n are encoded as __RUBASH_CSB1_ markers). Field
            // splitting must see the real IFS bytes — GNU subst.c splits the
            // expanded word before any of this protection is undone — so
            // decode here for the split decision and the split itself. The
            // no-split path keeps the protected form and decodes later.
            let decoded = decode_command_substitution_payload(&expanded);
            // GNU field-splits the expanded value without interpreting its
            // backslashes: a value like a\ b keeps the backslash in the
            // field and still splits on the space (a\ + b), whereas
            // field_split_escaped_ifs read it as an escaped separator and
            // stripped the backslash (a + b). Escaping belongs to the lexer,
            // not to parameter-expansion results.
            field_split_values_with_ifs(&decoded, self.env_vars.get("IFS").map(String::as_str))
        } else {
            vec![expanded]
        }
    }

    fn field_split_word_with_quoted_empty_suffix(
        &self,
        raw: Option<&str>,
        expanded: &str,
    ) -> Option<Vec<String>> {
        let raw = raw?;
        if !raw_has_quoted_empty_suffix(raw) || !expanded_ends_with_ifs_separator(expanded, self) {
            return None;
        }

        let mut values = self.field_split_values(expanded);
        values.push(String::new());
        Some(values)
    }

    fn braced_alternate_word_values(&mut self, word: &str, raw: Option<&str>) -> Option<Vec<String>> {
        let name = word.strip_prefix("${")?.strip_suffix('}')?;
        if !braced_parameter_spans_whole_word(word) {
            return None;
        }

        // `+`/`:+` use the word when the parameter is set (non-empty for
        // `:+`); `-`/`:-` use the word when it is unset (or empty for `:-`).
        // `$@`/`$*` inside the word must expand to their own field boundaries
        // (subst.c param_expand), which the String-based path cannot express,
        // so only those fragments are routed through the re-expansion path.
        let (var_name, alternate, use_when_set, require_non_empty, narrow) =
            if let Some((var_name, alternate)) = name.split_once(":+") {
                (var_name, alternate, true, true, false)
            } else if let Some((var_name, alternate)) = name.split_once('+') {
                (var_name, alternate, true, false, false)
            } else if let Some((var_name, alternate)) = name.split_once(":-") {
                (var_name, alternate, false, true, true)
            } else if let Some((var_name, alternate)) = name.split_once('-') {
                (var_name, alternate, false, false, true)
            } else {
                return None;
            };

        if narrow
            && !alternate.contains("$@")
            && !alternate.contains("$*")
            && !alternate.contains('"')
            && !alternate.contains('\'')
            && !parameter_word_has_escaped_whitespace(alternate)
        {
            return None;
        }

        let value = self.parameter_operator_value(var_name);
        let word_used = if use_when_set {
            value.is_some() && (!require_non_empty || !value.unwrap_or_default().is_empty())
        } else {
            value.is_none() || (require_non_empty && value.unwrap_or_default().is_empty())
        };
        if !word_used {
            return None;
        }

        // A double-quoted outer word quotes its alternate text: GNU expands
        // `${1+  $@  }` with the surrounding double-quote context intact
        // (subst.c param_expand passes `quoted` down to the alternate word),
        // so the affix spaces are quote-protected and the $@ still produces
        // one word per parameter. Re-lex the fragment inside synthetic double
        // quotes so the raw-level quoted-$@ path can split it. Fragments that
        // already carry their own quotes keep today's path.
        let outer_double_quoted = raw
            .filter(|raw| raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"'))
            .is_some();
        let fragment_quoted = alternate.starts_with('"') && alternate.ends_with('"');
        if outer_double_quoted
            && !fragment_quoted
            && (alternate.contains("$@") || alternate.contains("${@}"))
        {
            return Some(self.expand_alternate_word_fragment(&format!(
                "\"{alternate}\""
            )));
        }

        // Posix interp 888 (subst.c string_list_pos_params:3047-3072): the
        // word of ${param-OPword} expands $@/$* in assignment-RHS style.
        // An unquoted $* with a set-empty IFS joins with a space and splits
        // on the join spaces (W_SPLITSPACE, subst.c:10202-10207) — the
        // dollar_star path below concatenates the positionals instead
        // (posixexp3 `${var-$*}` IFS=). An unquoted $@ with a non-null IFS
        // joins with spaces (string_list_dollar_at:2951-2955; param_expand
        // sets PF_ASSIGNRHS for special parameters, subst.c:7840-7860) and
        // the joined value then field-splits per the ambient IFS, so
        // ${var-$@} under IFS=: stays one word (posixexp4's "inconsistent"
        // line). A null IFS leaves $@ on the per-parameter path below,
        // which already matches GNU.
        if !outer_double_quoted && matches!(alternate, "$@" | "${@}" | "$*" | "${*}") {
            if self.positional_params.is_empty() {
                return Some(Vec::new());
            }
            match self.env_vars.get("IFS").map(String::as_str) {
                Some("") => {
                    if alternate == "$*" || alternate == "${*}" {
                        // subst.c param_expand routes the operator word
                        // through expand_string_for_rhs -> W_SPLITSPACE over
                        // escaped elements: one preserved word per positional
                        // (exp9.sub `${var-$*}` with `set -- abc 'def ghi' jkl`).
                        return Some(self.positional_params.clone());
                    }
                }
                Some(ifs) => {
                    if alternate == "$@" || alternate == "${@}" {
                        let joined = self.positional_params.join(" ");
                        return Some(field_split_values_with_ifs(&joined, Some(ifs)));
                    }
                }
                None => {}
            }
        }
        let positional_at = alternate.contains("$@")
            || alternate.contains("${@}")
            || alternate.contains("$*");
        let posix_literal_quotes = self.posix_mode_enabled()
            && alternate.starts_with("\"")
            && alternate.ends_with("\"")
            && alternate.len() >= 2;
        // Fragments without $@/$* do not need the re-parse path, and the
        // lexer strips quotes while building word tokens, so a re-parsed
        // fragment loses the quoted/unquoted whitespace distinction
        // (posixexp2 29: the IFS alternate must yield fields `abx{ {{`
        // and `{}b`). The String path decodes quotes first and marks
        // quoted whitespace, keeping the boundary intact. It is
        // restricted to whitespace IFS: the whitespace sentinel only
        // protects space/tab/newline, and a non-whitespace IFS character
        // inside a quoted region must not split (GNU keeps a quoted
        // `a:b` whole under IFS=:), so those fragments stay on the
        // quote-aware re-parse path.
        let ifs_all_whitespace = self
            .env_vars
            .get("IFS")
            .map(|ifs| ifs.chars().all(|ch| matches!(ch, ' ' | '\t' | '\n')))
            .unwrap_or(true);
        if !positional_at && !posix_literal_quotes && ifs_all_whitespace {
            let expanded = self.expand_alternate_parameter_word(alternate);
            // A fully quoted empty alternate (foo:-quote-quote) is a
            // quoted null: GNU yields one empty field, not zero
            // (subst.c W_HASQUOTEDNULL).
            if expanded.is_empty() && alternate.contains(['"', '\'']) {
                return Some(vec![String::new()]);
            }
            return Some(self.field_split_values(&expanded));
        }
        Some(self.expand_alternate_word_fragment(alternate))
    }

    // Fully double-quoted `${op...$@...}` words (e.g. `"${1+  $@  }"`) carry
    // the \x1d marker, so braced_alternate_word_values skips them, but GNU
    // still expands their alternate with quoted-$@ word-boundary semantics
    // (subst.c param_expand keeps `quoted` set for the alternate word). The
    // alternate is expanded here as the body of a double-quoted span: affix
    // text attaches to the first/last positional word, one word per
    // parameter. Returning None leaves every other form on its existing path.
    fn quoted_braced_alternate_positional_at_values(
        &mut self,
        word: &str,
    ) -> Option<Vec<String>> {
        let braced = word.strip_prefix('\x1d')?;
        if !braced.starts_with("${") || !braced.ends_with('}') {
            return None;
        }
        if !braced_parameter_spans_whole_word(braced) {
            return None;
        }
        let inner = &braced[2..braced.len() - 1];
        if !inner.contains("$@")
            && !inner.contains("${@}")
            && !inner.contains("$*")
            && !inner.contains("${*}")
        {
            return None;
        }
        let (var_name, alternate, use_when_set, require_non_empty) =
            if let Some((var_name, alternate)) = inner.split_once(":+") {
                (var_name, alternate, true, true)
            } else if let Some((var_name, alternate)) = inner.split_once('+') {
                (var_name, alternate, true, false)
            } else if let Some((var_name, alternate)) = inner.split_once(":-") {
                (var_name, alternate, false, true)
            } else if let Some((var_name, alternate)) = inner.split_once('-') {
                (var_name, alternate, false, false)
            } else {
                return None;
            };

        let value = self.parameter_operator_value(var_name);
        let word_used = if use_when_set {
            value.is_some() && (!require_non_empty || !value.unwrap_or_default().is_empty())
        } else {
            value.is_none() || (require_non_empty && value.unwrap_or_default().is_empty())
        };
        if !word_used {
            // Alternate unused: quoted-empty/quoted-null handling belongs to
            // the existing paths, which already match GNU for those forms.
            return None;
        }

        // GNU string_list_pos_params (subst.c:3035-3040): a quoted `$*` joins
        // the positionals with IFS[0] into ONE word (concatenation when IFS
        // is set empty). The outer double quotes carry into the operator
        // word, so `"${var-$*}"` never field-splits (exp9.sub `${var-$*}`
        // under IFS=':' stays `abc:def ghi:jkl`).
        if alternate == "$*" || alternate == "${*}" {
            return Some(vec![
                self.positional_params
                    .join(&self.ifs_first_char_separator()),
            ]);
        }

        let synthetic_raw = format!("\"{alternate}\"");
        self.quoted_positional_at_word_values_with_raw(alternate, Some(&synthetic_raw), None)
    }

    fn expand_alternate_word_fragment(&mut self, fragment: &str) -> Vec<String> {
        // In POSIX mode, quotes inside a double-quoted parameter expansion
        // word are literal for quote-state purposes. Preserve that context
        // when expanding an alternate fragment instead of reparsing it as an
        // independent command (which incorrectly suppresses $name in '...').
        if self.posix_mode_enabled()
            && fragment.starts_with('\"')
            && fragment.ends_with('\"')
            && fragment.len() >= 2
        {
            let inner = &fragment[1..fragment.len() - 1];
            return vec![self.expand_embedded_parameters_mut_with_context(
                inner,
                SubstitutionQuoteContext::DoubleQuoted,
            )];
        }
        let source = format!("__rubash_parameter_alternate__ {fragment}");
        let tokens = crate::lexer::tokenize(&source);
        let ast = crate::parser::parse(&tokens);
        let Some(cmd) = ast.commands.first() else {
            return Vec::new();
        };

        let mut values = Vec::new();
        for index in 1..cmd.words.len() {
            let raw = cmd
                .word_metadata
                .get(index)
                .map(|metadata| metadata.raw.as_str());
            values.extend(self.expand_command_word(cmd, index, &cmd.words[index], raw));
        }
        values
    }

    pub(in crate::executor) fn apply_alias_expansion_after_word_expansion(
        &mut self,
        mut variable_expanded: CommandNode,
        original_raws: &[Option<&str>],
    ) -> CommandNode {
        if self.aliases.is_empty() {
            return variable_expanded;
        }

        // Honour quote state: `'hi'` / `"hi"` never expand as aliases.
        // expand_command_words drops word metadata, so quote state must come
        // from the original command's raw words (they line up 1:1 because
        // expansion does not reorder leading words).
        variable_expanded.words =
            self.expand_aliases_with_raw(&variable_expanded.words, original_raws);
        variable_expanded
    }

    pub(in crate::executor) fn execute_function_command_invocation(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<Result<(), ExecuteError>> {
        // GNU execute_cmd.c:4657-4676 (execute_simple_command): Posix.2 says
        // special builtins are found before functions. In posix mode a special
        // builtin precludes function lookup entirely (find_function is never
        // called); in default mode functions shadow all builtins (func5.sub
        // testfunc: posix 'break' runs the builtin, default runs the function).
        if self.posix_mode_enabled()
            && cmd
                .words
                .first()
                .is_some_and(|word| is_posix_special_builtin(word))
        {
            return None;
        }
        let function_name = cmd
            .words
            .first()
            .and_then(|word| self.function_name_for_command_word(word))?;
        let (materialized_cmd, process_substitution_files) =
            match self.command_with_process_substitution_files(cmd) {
                Ok(materialized) => materialized,
                Err(error) => return Some(Err(error)),
            };
        let temporary_assignments = self.apply_temporary_assignments(&materialized_cmd.assignments);
        let applied_assignment_values =
            self.applied_temporary_assignment_values(&materialized_cmd.assignments);
        let old_posix_export_touched = self.env_vars.remove(POSIX_FUNCTION_EXPORT_TOUCHED);
        let result = self.execute_function(
            &function_name,
            &materialized_cmd.words[1..],
            &materialized_cmd,
        );
        let finish_result = self.finish_process_substitutions(process_substitution_files);
        let assignment_finish_result =
            self.finish_assignment_output_process_substitutions_for_command(&materialized_cmd);
        if self.posix_mode_enabled() {
            self.restore_function_temporary_assignments(
                temporary_assignments,
                applied_assignment_values,
            );
        } else {
            self.restore_temporary_assignments(temporary_assignments);
        }
        restore_optional_env_var(
            &mut self.env_vars,
            POSIX_FUNCTION_EXPORT_TOUCHED,
            old_posix_export_touched,
        );
        Some(result.and(finish_result).and(assignment_finish_result))
    }

    pub(in crate::executor) fn execute_assignment_or_comment_command(
        &mut self,
        cmd: &CommandNode,
    ) -> bool {
        if self.execute_integer_assignment_suffix(cmd) || self.execute_assignment_words(cmd) {
            return true;
        }
        if self.execute_array_element_assignment(cmd) {
            return true;
        }
        if cmd.words.first().is_some_and(|word| word.starts_with('#')) {
            self.exit_code = 0;
            return true;
        }
        false
    }

    pub(in crate::executor) fn report_failglob(&mut self, pattern: &str) {
        eprintln!("{}no match: {pattern}", self.diagnostic_prefix());
        self.exit_code = 1;
    }
}

fn special_readonly_assignment_is_recoverable(name: &str) -> bool {
    matches!(
        name,
        "BASHOPTS" | "BASH_VERSINFO" | "EUID" | "PPID" | "SHELLOPTS" | "UID"
    )
}

fn assignment_builtin_receives_assignment_word(
    cmd: &CommandNode,
    index: usize,
    word: &str,
) -> bool {
    if index == 0 || split_assignment_word(word).is_none() {
        return false;
    }
    matches!(
        cmd.words.first().map(String::as_str),
        Some("export" | "readonly" | "declare" | "typeset" | "local")
    )
}

fn raw_word_contains_process_substitution(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if !single && !double && matches!(ch, '<' | '>') && chars.get(index + 1) == Some(&'(') {
            return true;
        }
        index += 1;
    }
    false
}

fn strip_assignment_builtin_command_subst_quotes(expanded: &str, raw: Option<&str>) -> String {
    let Some(raw_value) = raw.and_then(|raw| raw.split_once('=').map(|(_, value)| value)) else {
        return expanded.to_string();
    };
    let raw_value = raw_value.trim();
    if !(raw_value.starts_with('"')
        && raw_value.ends_with('"')
        && (raw_value.contains("$(") || raw_value.contains('`')))
    {
        return expanded.to_string();
    }
    let Some((name, value)) = expanded.split_once('=') else {
        return expanded.to_string();
    };
    let Some(value) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return expanded.to_string();
    };
    format!("{}={}", name, value)
}

fn expanded_word_has_process_substitution(word: &str) -> bool {
    !WordMetadata::new(0, word.to_string(), word.to_string())
        .process_substitutions
        .is_empty()
}

pub(in crate::executor) fn expand_braces_with_optional_raw(
    word: &str,
    raw: Option<&str>,
) -> Vec<String> {
    if let Some(raw) = raw {
        if raw != word {
            let braced = crate::expand::braces::expand_braces(raw);
            if braced.len() > 1 || word_contains_brace_group(raw) {
                // The raw-based expansion kept backslash escapes that the
                // lexer would otherwise strip. Strip them so escaped chars
                // render literally, matching GNU bash, while real brace
                // expansion still yields multiple words.
                let mut stripped: Vec<String> = Vec::new();
                for item in braced {
                    stripped.push(crate::lexer::remove_shell_quotes(&item));
                }
                return stripped;
            }
            // When raw has escaped braces that prevent expansion,
            // but word has lost the escapes and would expand, preserve the
            // raw result to avoid incorrect brace expansion.
            if crate::expand::braces::expand_braces(word).len() > 1 {
                return vec![word.to_string()];
            }
        }
    }

    let braced = crate::expand::braces::expand_braces(word);
    // GNU bash runs brace expansion on the raw word (backslash escapes
    // intact) and applies quote removal to each result word afterwards
    // (subst.c brace expansion runs before the quote-removal pass on the
    // expanded word list). BraceExpand tokens skip lexer-level quote
    // removal, so strip the escapes here: {a,b\,c} must yield b,c and
    // \{a,b} must yield {a,b} once no expansion is possible.
    if braced.len() > 1 {
        braced
            .into_iter()
            .map(|item| crate::lexer::remove_shell_quotes(&item))
            .collect()
    } else if word_contains_brace_group(word) {
        // A brace group can be syntactically present without producing
        // multiple words; quote removal still applies to its escaped text.
        braced
            .into_iter()
            .map(|item| crate::lexer::remove_shell_quotes(&item))
            .collect()
    } else {
        braced
    }
}

fn word_contains_brace_group(word: &str) -> bool {
    let mut escaped = false;
    let mut open = false;
    for ch in word.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
        } else if ch == '{' {
            open = true;
        } else if ch == '}' && open {
            return true;
        }
    }
    false
}

pub(in crate::executor) fn restore_pathname_escape_markers(word: &str) -> String {
    let word = crate::expand::tilde::tilde::strip_assignment_quote_marker(word);
    let word = word
        .split_once('=')
        .and_then(|(name, value)| {
            value
                .strip_prefix(crate::expand::tilde::tilde::QUOTED_ASSIGNMENT_VALUE)
                .map(|value| format!("{name}={value}"))
        })
        .unwrap_or_else(|| word.to_string());
    word.replace('\x11', "")
}

pub(in crate::executor) fn raw_word_suppresses_pathname_expansion(
    raw: Option<&str>,
    metadata: Option<&WordMetadata>,
) -> bool {
    raw_word_is_quoted(raw)
        && metadata
            .map(|metadata| metadata.pathname_patterns.is_empty())
            .unwrap_or(true)
}

pub(in crate::executor) fn raw_word_is_fully_single_quoted(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'')
}

pub(in crate::executor) fn raw_word_is_quoted(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        match chars[index] {
            '\'' | '"' => return true,
            '$' if chars.get(index + 1) == Some(&'\'') || chars.get(index + 1) == Some(&'"') => {
                return true;
            }
            '$' if chars.get(index + 1) == Some(&'{') => {
                // Quotes inside a braced parameter expansion (e.g. the
                // double quote in a default/alternate word) are word syntax,
                // not outer quoting: bash only treats the whole word as quoted
                // when the braced expansion itself is quoted. Skip the braced
                // body so an unquoted word still word-splits its result.
                index = skip_raw_braced_parameter(&chars, index + 2);
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'(') => {
                index = skip_raw_command_substitution(&chars, index + 2);
                continue;
            }
            '`' => {
                index = skip_raw_backtick(&chars, index + 1);
                continue;
            }
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    false
}

fn raw_has_quoted_empty_suffix(raw: &str) -> bool {
    raw.ends_with("''") || raw.ends_with("\"\"")
}

fn field_split_escaped_ifs(value: &str, ifs: Option<&str>) -> Vec<String> {
    const PROTECTED_IFS: char = '\u{1e}';
    let ifs = ifs.unwrap_or(" \t\n");
    let mut protected = String::with_capacity(value.len());
    let chars = value.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\' && chars.get(index + 1).is_some_and(|next| ifs.contains(*next)) {
            protected.push(PROTECTED_IFS);
            protected.push(chars[index + 1]);
            index += 2;
        } else {
            protected.push(chars[index]);
            index += 1;
        }
    }

    field_split_values_with_ifs(&protected, Some(ifs))
        .into_iter()
        .map(|field| field.replace(PROTECTED_IFS, "").chars().collect::<String>())
        .collect()
}

fn expanded_ends_with_ifs_separator(expanded: &str, executor: &Executor) -> bool {
    let Some(last) = expanded.chars().last() else {
        return false;
    };
    executor
        .env_vars
        .get("IFS")
        .map(String::as_str)
        .unwrap_or(" \t\n")
        .contains(last)
}

fn skip_raw_command_substitution(chars: &[char], mut index: usize) -> usize {
    let mut depth = 1usize;
    while index < chars.len() {
        match chars[index] {
            '\'' => index = skip_raw_quote(chars, index + 1, '\''),
            '"' => index = skip_raw_quote(chars, index + 1, '"'),
            '`' => index = skip_raw_backtick(chars, index + 1),
            '$' if chars.get(index + 1) == Some(&'(') => {
                depth += 1;
                index += 2;
                continue;
            }
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index + 1;
                }
            }
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    index
}

fn skip_raw_braced_parameter(chars: &[char], mut index: usize) -> usize {
    let mut depth = 1usize;
    while index < chars.len() {
        match chars[index] {
            '\'' => index = skip_raw_quote(chars, index + 1, '\''),
            '"' => index = skip_raw_quote(chars, index + 1, '"'),
            '`' => index = skip_raw_backtick(chars, index + 1),
            '$' if chars.get(index + 1) == Some(&'{') => {
                depth += 1;
                index += 2;
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'(') => {
                index = skip_raw_command_substitution(chars, index + 2);
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'\'') => {
                index = skip_raw_quote(chars, index + 2, '\'');
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'"') => {
                index = skip_raw_quote(chars, index + 2, '"');
                continue;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index + 1;
                }
            }
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    index
}

fn skip_raw_quote(chars: &[char], mut index: usize, quote: char) -> usize {
    while index < chars.len() {
        if chars[index] == '\\' && quote != '\'' {
            index += 2;
            continue;
        }
        if chars[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    index
}

fn skip_raw_backtick(chars: &[char], mut index: usize) -> usize {
    while index < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if chars[index] == '`' {
            return index + 1;
        }
        index += 1;
    }
    index
}

#[cfg(test)]
mod command_word_materialization_tests {
    use super::materialize_expanded_command_word;

    #[test]
    fn materializes_legacy_payload_at_single_boundary() {
        assert_eq!(
            materialize_expanded_command_word("prefix__RUBASH_CSB1_41;suffix"),
            "prefixAsuffix"
        );
    }

    #[test]
    fn leaves_ordinary_command_word_bytes_unchanged() {
        assert_eq!(
            materialize_expanded_command_word("plain\x15word"),
            "plain\\word"
        );
    }

    #[test]
    fn materializes_pathname_marker_before_payload_decode() {
        assert_eq!(
            materialize_expanded_command_word("prefix\x15__RUBASH_CSB1_41;suffix"),
            "prefix\\Asuffix"
        );
    }
}
