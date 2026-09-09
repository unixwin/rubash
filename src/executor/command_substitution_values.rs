use super::*;

impl Executor {
    pub(in crate::executor) fn command_substitution_pipeline_filter(
        &self,
        words: &[String],
        input: &str,
    ) -> Option<(String, i32)> {
        match words.first().map(String::as_str)? {
            "sort" => {
                let unique = words[1..].iter().any(|word| self.expand_word(word) == "-u");
                let mut lines = input.lines().map(str::to_string).collect::<Vec<_>>();
                lines.sort();
                if unique {
                    lines.dedup();
                }
                let mut output = lines.join("\n");
                if !output.is_empty() {
                    output.push('\n');
                }
                Some((output, 0))
            }
            "sed" => {
                let args = words[1..]
                    .iter()
                    .map(|word| {
                        self.expand_word(word)
                            .replace('\x15', "\\")
                            .replace('\x1f', "$")
                            .replace('\x11', "")
                    })
                    .collect::<Vec<_>>();
                apply_simple_sed_args(input, &args).map(|output| (output, 0))
            }
            "tr" => {
                let args = words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                if args.len() != 2 {
                    return None;
                }
                if matches!(args[0].as_str(), "\\n" | "\n") {
                    Some((input.replace('\n', &args[1]), 0))
                } else if crate::executor::pipeline_exec::inline_expand_tr_set(&args[0]).is_some()
                    && crate::executor::pipeline_exec::inline_expand_tr_set(&args[1]).is_some()
                {
                    Some((
                        crate::executor::pipeline_exec::translate_tr(input, &args[0], &args[1]),
                        0,
                    ))
                } else {
                    // Syntax the inline fast path cannot represent must run
                    // the real external `tr`.
                    None
                }
            }
            "head" => {
                let args = words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                let count = crate::executor::pipeline_exec::head_line_count(&args).unwrap_or(10);
                Some((input.split_inclusive('\n').take(count).collect(), 0))
            }
            "grep" => {
                let pattern = words.get(1).map(|word| self.expand_word(word))?;
                let mut output = String::new();
                let mut matched = false;
                for line in input.split_inclusive('\n') {
                    let comparable = line.strip_suffix('\n').unwrap_or(line);
                    if crate::executor::simple_grep_pattern_matches(comparable, &pattern) {
                        matched = true;
                        output.push_str(line);
                        if !line.ends_with('\n') {
                            output.push('\n');
                        }
                    }
                }
                Some((output, i32::from(!matched)))
            }
            "wc" => {
                let option = words.get(1).map(String::as_str).unwrap_or("-l");
                let value = match option {
                    "-c" => input.as_bytes().len(),
                    "-l" => input.bytes().filter(|byte| *byte == b'\n').count(),
                    _ => return None,
                };
                Some((format!("{value}\n"), 0))
            }
            "tail" => {
                let args = words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                let count = crate::executor::pipeline_exec::head_line_count(&args).unwrap_or(10);
                let lines = input.split_inclusive('\n').collect::<Vec<_>>();
                let start = lines.len().saturating_sub(count);
                Some((lines[start..].concat(), 0))
            }
            "uniq" => {
                let mut output = String::new();
                let mut previous = None;
                for line in input.split_inclusive('\n') {
                    let comparable = line.strip_suffix('\n').unwrap_or(line);
                    if previous != Some(comparable) {
                        output.push_str(line);
                    }
                    previous = Some(comparable);
                }
                Some((output, 0))
            }
            _ => {
                let cmd_name = self.expand_word(&words[0]);
                let expanded_args: Vec<String> =
                    words[1..].iter().map(|w| self.expand_word(w)).collect();
                use std::io::Write;
                use std::process::Stdio;
                let program = find_user_command(&cmd_name, &self.env_vars)?;
                let (mut process, _) = external_command_for_named_program(
                    &program,
                    Some(&cmd_name),
                    &expanded_args,
                    &self.env_vars,
                );
                self.apply_child_environment(&mut process);
                let mut child = process
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .ok()?;
                child
                    .stdin
                    .as_mut()?
                    .write_all(
                        &crate::executor::substitution_metadata::shell_text_to_raw_bytes(input),
                    )
                    .ok()?;
                let output = child.wait_with_output().ok()?;
                Some((
                    crate::executor::substitution_metadata::bytes_to_shell_text(&output.stdout)
                        .trim_end_matches('\n')
                        .to_string(),
                    output.status.code().unwrap_or(1),
                ))
            }
        }
    }

    pub(in crate::executor) fn expand_command_substitution_arg_values(
        &self,
        word: &str,
    ) -> Vec<String> {
        if let Some(values) = self.quoted_positional_at_word_values(word, None) {
            return values;
        }
        if let Some(values) = self.array_at_word_values(word) {
            return values;
        }
        let expanded = strip_matching_quotes(&restore_command_substitution_output(
            &self.expand_word(word),
        ))
        .to_string();
        // The specialized substitution paths emulate GNU's expand_words for
        // the substitution body (subst.c): an unquoted parameter/command
        // substitution word is field-split on $IFS exactly like a plain
        // command's word list (nquote5.tests `$(echo $a)` with IFS=$'\001'
        // must hand echo three args whose space-joined output re-splits to
        // one field, not the literal \001 bytes).
        if for_word_has_unquoted_expansion(word, None) {
            return self.field_split_values(&expanded);
        }
        vec![expanded]
    }

    pub(in crate::executor) fn command_describe_substitution_output(
        &self,
        words: &[String],
    ) -> Option<String> {
        if words.first().map(String::as_str) != Some("command") {
            return None;
        }
        if words
            .iter()
            .any(|word| matches!(word.as_str(), "|" | ">" | ">>" | "<" | "2>" | "2>>" | "&>"))
        {
            return None;
        }
        let Some((mode, use_standard_path, first_name)) = parse_command_describe_args(&words[1..])
        else {
            return None;
        };

        let mut stdout = Vec::new();
        let mut status = 0;
        for name in &words[1 + first_name..] {
            let name = self.expand_word(name);
            match self.describe_name_with_io(&name, mode, use_standard_path, false, false, &mut stdout) {
                Ok(true) => {}
                Ok(false) => status = 1,
                Err(_) => status = 1,
            }
        }
        self.last_command_substitution_status.set(Some(status));
        Some(
            crate::executor::substitution_metadata::bytes_to_shell_text(&stdout)
                .trim_end_matches('\n')
                .to_string(),
        )
    }

    pub(in crate::executor) fn quoted_positional_at_word_values(
        &self,
        word: &str,
        kind: Option<&TokenKind>,
    ) -> Option<Vec<String>> {
        let quoted_positional_word =
            (word.starts_with('"') && word.ends_with('"')) || word.starts_with('\x1d');
        let word = word
            .strip_prefix('"')
            .and_then(|word| word.strip_suffix('"'))
            .unwrap_or(word);
        let word = word.strip_prefix('\x1d').unwrap_or(word);
        if word == "${@}" {
            return Some(self.positional_params.clone());
        }
        if word == "$@" && kind.map_or(true, |kind| *kind == TokenKind::Word) {
            return Some(self.positional_params.clone());
        }
        if let Some(name) = word
            .strip_prefix("${")
            .and_then(|word| word.strip_suffix('}'))
        {
            // A quoted indirect reference whose target is @ or * expands with
            // the same word-boundary rules as "${@}" / "${*}" (GNU subst.c
            // parameter_brace_expand_indir re-expands the target name in the
            // caller's quote context): "${!foo}" with foo=@ yields one word
            // per positional parameter, foo=* one joined word. Unquoted
            // indirect references keep today's join-and-split path.
            if quoted_positional_word {
                if let Some(indirect) = name.strip_prefix('!') {
                    if indirect == "@" {
                        return Some(self.positional_params.clone());
                    }
                    if indirect == "*" {
                        return Some(vec![
                            self.positional_params.join(&self.ifs_first_char_separator()),
                        ]);
                    }
                    if is_shell_name(indirect) {
                        if let Some(target) = self.env_vars.get(indirect).map(String::as_str) {
                            if target == "@" {
                                return Some(self.positional_params.clone());
                            }
                            if target == "*" {
                                return Some(vec![
                                    self.positional_params.join(&self.ifs_first_char_separator()),
                                ]);
                            }
                        }
                    }
                }
            }
            if let Some(values) =
                self.positional_transform_word_values(name, quoted_positional_word)
            {
                return Some(values);
            }
            if let Some(values) = self.positional_modified_word_values(name, quoted_positional_word)
            {
                return Some(values);
            }
            if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
                if var_name == "@" {
                    return Some(positional_parameter_substring(
                        &self.positional_params,
                        offset,
                        length,
                    ));
                }
                if var_name == "*" {
                    let values =
                        positional_parameter_substring(&self.positional_params, offset, length);
                    if quoted_positional_word {
                        return Some(vec![values.join(&self.ifs_first_char_separator())]);
                    }
                    return Some(values);
                }
            }
        }
        None
    }

    pub(in crate::executor) fn word_is_unquoted_positional_modified_list_expansion(
        &self,
        word: &str,
    ) -> bool {
        if word.starts_with('"') || word.starts_with('\'') || word.starts_with('\x1d') {
            return false;
        }
        let Some(inner) = word
            .strip_prefix("${")
            .and_then(|word| word.strip_suffix('}'))
        else {
            return false;
        };
        self.positional_modified_base_name(inner)
            .or_else(|| parse_parameter_transform(inner).map(|(name, _)| name))
            .or_else(|| {
                self.parse_parameter_substring(inner)
                    .map(|(name, _, _)| name)
            })
            .is_some_and(|name| matches!(name, "@" | "*"))
    }

    pub(in crate::executor) fn word_is_unquoted_positional_list_expansion(
        &self,
        word: &str,
    ) -> bool {
        if word.starts_with('"') || word.starts_with('\'') || word.starts_with('\x1d') {
            return false;
        }
        let Some(inner) = word
            .strip_prefix("${")
            .and_then(|word| word.strip_suffix('}'))
        else {
            return false;
        };
        matches!(inner, "@" | "*")
    }

    fn positional_transform_word_values(&self, name: &str, quoted: bool) -> Option<Vec<String>> {
        let (var_name, transform) = parse_parameter_transform(name)?;
        if !matches!(var_name, "@" | "*") {
            return None;
        }

        let values = if transform == ParameterTransform::Assignment {
            let mut values = vec!["set".to_string(), "--".to_string()];
            values.extend(
                self.positional_params
                    .iter()
                    .map(|value| shell_single_quote_assignment_value(value)),
            );
            values
        } else {
            self.positional_params
                .iter()
                .map(|value| self.apply_parameter_transform_value(value, transform))
                .collect::<Vec<_>>()
        };

        if var_name == "*" && transform != ParameterTransform::Assignment {
            // GNU string_list_pos_params (subst.c:3030-3057): an unquoted `*`
            // uses dollar_star (IFS[0]) except when IFS is set empty, where
            // Posix interp 888 falls back to dollar_at (space separator); the
            // joined word is then field-split like any unquoted expansion
            // (W_SPLITSPACE). The split runs over list_quote_escapes-protected
            // elements (subst.c:3014), so each element survives verbatim: one
            // word per positional (exp10.sub `${*@Q}` with `set -- ' A ' ' B '`).
            let ifs_set_empty = self
                .env_vars
                .get("IFS")
                .is_some_and(|value| value.is_empty());
            if ifs_set_empty && !quoted {
                return Some(values);
            }
            let separator = if ifs_set_empty {
                " ".to_string()
            } else {
                self.ifs_first_char_separator()
            };
            return Some(vec![values.join(&separator)]);
        }
        if quoted && var_name == "*" {
            if transform == ParameterTransform::Assignment {
                let mut rendered = String::from("set -- ");
                rendered.push_str(
                    &values[2..]
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(&self.ifs_first_char_separator()),
                );
                return Some(vec![rendered]);
            }
            return Some(vec![values.join(&self.ifs_first_char_separator())]);
        }

        Some(values)
    }

    fn positional_modified_word_values(&self, name: &str, quoted: bool) -> Option<Vec<String>> {
        if let Some((var_name, pattern, operation)) = parse_indirect_pattern_removal(name) {
            let pattern = self.expand_parameter_pattern_word(pattern);
            return self.positional_modified_values(var_name, quoted, |value| {
                remove_parameter_pattern(value, &pattern, operation)
            });
        }

        if let Some((var_name, pattern, replacement, global)) = parse_parameter_replacement(name) {
            let pattern = self.expand_parameter_pattern_word(pattern);
            let replacement = self.expand_patsub_replacement_text(replacement);
            return self.positional_modified_values(var_name, quoted, |value| {
                self.replace_patsub_pattern(value, &pattern, &replacement, global)
            });
        }

        if let Some((var_name, operation, pattern)) = parse_parameter_case_mod(name) {
            let pattern = self.expand_embedded_parameters(pattern);
            return self.positional_modified_values(var_name, quoted, |value| {
                apply_parameter_case_mod(value, operation, &pattern)
            });
        }

        None
    }

    fn positional_modified_base_name<'a>(&self, name: &'a str) -> Option<&'a str> {
        parse_indirect_pattern_removal(name)
            .map(|(name, _, _)| name)
            .or_else(|| parse_parameter_replacement(name).map(|(name, _, _, _)| name))
            .or_else(|| parse_parameter_case_mod(name).map(|(name, _, _)| name))
    }

    fn positional_modified_values<F>(
        &self,
        name: &str,
        quoted: bool,
        modify: F,
    ) -> Option<Vec<String>>
    where
        F: Fn(&str) -> String,
    {
        if !matches!(name, "@" | "*") {
            return None;
        }
        let values = self
            .positional_params
            .iter()
            .map(|value| modify(value))
            .collect::<Vec<_>>();
        if quoted && name == "*" {
            return Some(vec![values.join(&self.ifs_first_char_separator())]);
        }
        Some(values)
    }

    pub(in crate::executor) fn quoted_positional_at_word_values_with_raw(
        &self,
        word: &str,
        raw: Option<&str>,
        kind: Option<&TokenKind>,
    ) -> Option<Vec<String>> {
        if !word.starts_with("${") {
            if let Some(values) = raw.and_then(|raw| {
                quoted_positional_at_segments(raw).map(|segments| {
                    expand_quoted_positional_at_segments(
                        &segments,
                        &self.positional_params,
                        |literal| self.expand_embedded_parameters(literal),
                    )
                })
            }) {
                return Some(values);
            }
        }

        self.quoted_positional_at_word_values(word, kind)
    }

    pub(in crate::executor) fn join_array_parameter_values(
        &self,
        value: &str,
        expression: &str,
    ) -> String {
        let values = array_values(value)
            .into_iter()
            .map(normalize_array_expanded_value)
            .collect::<Vec<_>>();
        self.join_expanded_array_values(values, expression)
    }

    pub(in crate::executor) fn join_expanded_array_values(
        &self,
        values: Vec<String>,
        expression: &str,
    ) -> String {
        if expression.ends_with("[*]") {
            // GNU dollar_star (subst.c string_list_dollar_star): IFS[0], the
            // empty string when IFS is set empty, a space when IFS is unset.
            return values.join(&self.ifs_first_char_separator());
        }
        values.join(" ")
    }

    pub(in crate::executor) fn report_command_substitution_heredoc_warning(
        &self,
        source: &str,
        command: &CommandNode,
    ) {
        let start_line = self
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .unwrap_or_else(|| command.line.unwrap_or(1));
        let warning_line = start_line + source.lines().count().saturating_sub(1);
        let delimiter = command.heredoc_delimiter.as_deref().unwrap_or("");
        eprintln!(
            "{}warning: here-document at line {start_line} delimited by end-of-file (wanted `{delimiter}')",
            self.diagnostic_prefix_for_line(warning_line)
        );
    }

    pub(in crate::executor) fn run_external_command_substitution(
        &self,
        words: &[String],
    ) -> Option<String> {
        words.first()?;
        let stdio = self.command_substitution_words_and_stdio(words)?;
        if matches!(
            stdio.expanded_words.first().map(String::as_str),
            Some("tty") | Some("/bin/tty") | Some("/usr/bin/tty")
        ) {
            let silent = stdio
                .expanded_words
                .iter()
                .skip(1)
                .any(|arg| arg == "-s" || arg == "--silent" || arg == "--quiet");
            self.last_command_substitution_status.set(Some(1));
            return Some(if silent {
                String::new()
            } else {
                "not a tty".to_string()
            });
        }
        // Don't route shell builtins through the external-command path.
        // On Windows, "fc" resolves to system32\fc.exe (file compare),
        // not the shell's "fc" builtin. Also respect "enable -n".
        let first_word = stdio.expanded_words.first().map(String::as_str).unwrap_or("");
        if is_shell_builtin_name(first_word)
            && !crate::builtins::enable::is_disabled(&self.env_vars, first_word)
        {
            return None;
        }
        let Some(program) = find_user_command(&stdio.expanded_words[0], &self.env_vars) else {
            if stdio.expanded_words.first().map(String::as_str) == Some("mktemp") {
                return None;
            }
            // Not an external command: let the full-execution fallback run
            // the source (functions, builtins, compound commands).
            return None;
        };
        let (mut process, _) = external_command_for_named_program(
            &program,
            Some(&stdio.expanded_words[0]),
            &stdio.expanded_words[1..],
            &self.env_vars,
        );

        self.apply_child_environment(&mut process);
        if let Some(stdin_path) = stdio.stdin_path {
            let file = File::open(stdin_path).ok()?;
            process.stdin(Stdio::from(file));
        }
        if let Some(redirect) = &stdio.stdout_redirect {
            let file = open_command_substitution_redirect(redirect).ok()?;
            process.stdout(Stdio::from(file));
        } else {
            process.stdout(Stdio::piped());
        }
        if let Some(redirect) = &stdio.stderr_redirect {
            let file = open_command_substitution_redirect(redirect).ok()?;
            process.stderr(Stdio::from(file));
        } else {
            process.stderr(Stdio::piped());
        }
        let output = process.spawn().ok()?.wait_with_output().ok()?;
        let status = output.status.code().unwrap_or(1);
        if stdio.expanded_words.first().map(String::as_str) == Some("mktemp")
            && status != 0
            && !stdio.had_redirect
        {
            return None;
        }
        self.last_command_substitution_status.set(Some(status));
        if stdio.stdout_redirect.is_some() {
            return Some(String::new());
        }
        Some(
            bytes_to_shell_text(&output.stdout)
                .trim_end_matches('\n')
                .to_string(),
        )
    }

    fn command_substitution_words_and_stdio(
        &self,
        words: &[String],
    ) -> Option<CommandSubstitutionStdio> {
        let mut stdio = CommandSubstitutionStdio::default();
        let mut index = 0;
        while index < words.len() {
            match words[index].as_str() {
                "|" => return None,
                "<" => {
                    stdio.stdin_path =
                        Some(self.command_substitution_redirect_path(words.get(index + 1)?)?);
                    stdio.had_redirect = true;
                    index += 2;
                }
                ">" | "1>" | ">|" | "1>|" => {
                    stdio.stdout_redirect = Some(CommandSubstitutionRedirect {
                        path: self.command_substitution_redirect_path(words.get(index + 1)?)?,
                        append: false,
                    });
                    stdio.had_redirect = true;
                    index += 2;
                }
                ">>" | "1>>" => {
                    stdio.stdout_redirect = Some(CommandSubstitutionRedirect {
                        path: self.command_substitution_redirect_path(words.get(index + 1)?)?,
                        append: true,
                    });
                    stdio.had_redirect = true;
                    index += 2;
                }
                "2>" | "2>|" => {
                    stdio.stderr_redirect = Some(CommandSubstitutionRedirect {
                        path: self.command_substitution_redirect_path(words.get(index + 1)?)?,
                        append: false,
                    });
                    stdio.had_redirect = true;
                    index += 2;
                }
                "2>>" => {
                    stdio.stderr_redirect = Some(CommandSubstitutionRedirect {
                        path: self.command_substitution_redirect_path(words.get(index + 1)?)?,
                        append: true,
                    });
                    stdio.had_redirect = true;
                    index += 2;
                }
                word => {
                    stdio
                        .expanded_words
                        .extend(self.expand_command_substitution_arg_values(word));
                    index += 1;
                }
            }
        }
        (!stdio.expanded_words.is_empty()).then_some(stdio)
    }

    fn command_substitution_redirect_path(&self, target: &str) -> Option<PathBuf> {
        let expanded = strip_matching_quotes(&self.expand_word(target)).to_string();
        Some(shell_path_to_windows(&expanded, &self.env_vars))
    }

    pub(in crate::executor) fn expand_backtick_substitution_typed(
        &mut self,
        word: &str,
        quoted: bool,
    ) -> Option<SubstitutionOutput> {
        if !backtick_substitution_spans_whole_word(word) {
            return None;
        }
        let source =
            decode_backtick_substitution_source(word.strip_prefix('`')?.strip_suffix('`')?);
        Some(self.expand_command_substitution_mut_typed_with_context(
            &source,
            if quoted {
                SubstitutionQuoteContext::DoubleQuoted
            } else {
                SubstitutionQuoteContext::Unquoted
            },
        ))
    }

    pub(in crate::executor) fn expand_backtick_substitution(&self, word: &str) -> Option<String> {
        // TODO(subst.c): Backquote command substitution should invoke the
        // parser and run a subshell. This reuses the same in-process command
        // substitution bridge as `$()`.
        if !backtick_substitution_spans_whole_word(word) {
            return None;
        }
        let source =
            decode_backtick_substitution_source(word.strip_prefix('`')?.strip_suffix('`')?);
        Some(
            self.expand_command_substitution_readback_with_context(
                &source,
                SubstitutionQuoteContext::Unquoted,
            )
            .text_lossy(),
        )
    }

    pub(in crate::executor) fn expand_dirstack_tilde(&self, word: &str) -> Option<String> {
        // TODO(subst.c/builtins/pushd.def): Bash performs directory-stack
        // tilde expansion during word expansion. This implements ~N and ~-N
        // for upstream dstack2.tests.
        let rest = word.strip_prefix('~')?;
        if rest.is_empty() || rest.starts_with('/') {
            return None;
        }

        let (from_right, digits) = if let Some(digits) = rest.strip_prefix('-') {
            (true, digits)
        } else {
            (false, rest)
        };
        if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }

        let value = digits.parse::<usize>().ok()?;
        let stack = crate::builtins::pushd::load_stack(&self.env_vars);
        let index = if from_right {
            if value < stack.len() {
                stack.len() - 1 - value
            } else {
                return Some(word.to_string());
            }
        } else {
            value
        };
        stack.get(index).cloned().or_else(|| Some(word.to_string()))
    }

    pub(in crate::executor) fn dirstack_subscript(&self, index: &str) -> Option<usize> {
        if let Ok(index) = index.parse::<usize>() {
            return Some(index);
        }

        if index == "NDIRS" {
            return self
                .env_vars
                .get("NDIRS")
                .and_then(|value| value.parse::<usize>().ok())
                .or_else(|| {
                    Some(
                        crate::builtins::pushd::load_stack(&self.env_vars)
                            .len()
                            .saturating_sub(1),
                    )
                });
        }

        let (name, rhs) = index.split_once('-')?;
        if name != "NDIRS" {
            return None;
        }
        let rhs = rhs.parse::<usize>().ok()?;
        let ndirs = self
            .env_vars
            .get("NDIRS")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_else(|| {
                crate::builtins::pushd::load_stack(&self.env_vars)
                    .len()
                    .saturating_sub(1)
            });
        ndirs.checked_sub(rhs)
    }
}

fn decode_backtick_substitution_source(source: &str) -> String {
    decode_old_style_backtick_source(source)
        .replace('\x1a', "`")
        .replace('\x11', "")
        .replace('\x1f', "$")
        .replace('\x15', "\\")
}

#[derive(Default)]
struct CommandSubstitutionStdio {
    expanded_words: Vec<String>,
    stdin_path: Option<PathBuf>,
    stdout_redirect: Option<CommandSubstitutionRedirect>,
    stderr_redirect: Option<CommandSubstitutionRedirect>,
    had_redirect: bool,
}

struct CommandSubstitutionRedirect {
    path: PathBuf,
    append: bool,
}

fn open_command_substitution_redirect(redirect: &CommandSubstitutionRedirect) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    if redirect.append {
        options.append(true);
    } else {
        options.truncate(true);
    }
    options.open(&redirect.path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QuotedPositionalAtSegment {
    /// Literal text. `quoted` marks text that came from inside a double-quoted
    /// span (GNU subst.c add_quoted_char): its IFS characters are quote-
    /// protected data, so they attach to the adjacent word and never field-
    /// split. Text collected between quoted spans is `quoted == false` and
    /// keeps the historic unquoted-literal split behavior.
    Literal { text: String, quoted: bool },
    PositionalAt,
}

fn quoted_positional_at_segments(raw: &str) -> Option<Vec<QuotedPositionalAtSegment>> {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut segments = Vec::new();
    let mut literal_start = 0usize;
    let mut index = 0usize;
    let mut saw_positional_at = false;

    while index < chars.len() {
        match chars[index] {
            '"' => {
                let Some(end) = skip_double_quote(&chars, index + 1) else {
                    return None;
                };
                let body = &chars[index + 1..end];
                if body == "$@".chars().collect::<Vec<_>>().as_slice() {
                    push_quoted_positional_literal_segment(
                        &mut segments,
                        &chars[literal_start..index],
                        false,
                    )?;
                    segments.push(QuotedPositionalAtSegment::PositionalAt);
                    saw_positional_at = true;
                    index = end + 1;
                    literal_start = index;
                    continue;
                }
                // A quoted span whose body embeds `$@`/`${@}` around other
                // text splits into literal/positional segments: affixes attach
                // to the first/last positional word (GNU subst.c expands the
                // quoted span into a word list whose boundary words carry the
                // surrounding quoted text; expand_word_internal 11723-11808).
                if let Some(body_segments) = quoted_body_positional_at_segments(body) {
                    push_quoted_positional_literal_segment(
                        &mut segments,
                        &chars[literal_start..index],
                        false,
                    )?;
                    segments.extend(body_segments);
                    saw_positional_at = true;
                    index = end + 1;
                    literal_start = index;
                    continue;
                }
                index = end + 1;
                continue;
            }
            '\'' => {
                index = skip_single_quote(&chars, index + 1)?;
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'\'') => {
                index = skip_single_quote(&chars, index + 2)?;
                continue;
            }
            '\\' => {
                index += 2;
                continue;
            }
            _ => index += 1,
        }
    }

    if !saw_positional_at {
        return None;
    }

    push_quoted_positional_literal_segment(&mut segments, &chars[literal_start..], false)?;
    Some(segments)
}

/// Split the body of one double-quoted span around its unescaped `$@` /
/// `${@}` occurrences. Returns None when the body has no positional-at token
/// (the caller leaves the span as ordinary text) or when the body contains
/// anything this narrow path must not guess at: other `${...}` operator
/// forms, backticks, or backslash escapes.
fn quoted_body_positional_at_segments(body: &[char]) -> Option<Vec<QuotedPositionalAtSegment>> {
    let mut segments = Vec::new();
    let mut piece_start = 0usize;
    let mut index = 0usize;
    let mut saw_positional_at = false;

    while index < body.len() {
        match body[index] {
            '`' | '\\' => return None,
            '$' if body.get(index + 1) == Some(&'{') => {
                if body.get(index + 2) == Some(&'@') && body.get(index + 3) == Some(&'}') {
                    push_body_piece(&mut segments, &body[piece_start..index])?;
                    segments.push(QuotedPositionalAtSegment::PositionalAt);
                    saw_positional_at = true;
                    index += 4;
                    piece_start = index;
                } else {
                    return None;
                }
            }
            '$' if body.get(index + 1) == Some(&'@') => {
                push_body_piece(&mut segments, &body[piece_start..index])?;
                segments.push(QuotedPositionalAtSegment::PositionalAt);
                saw_positional_at = true;
                index += 2;
                piece_start = index;
            }
            _ => index += 1,
        }
    }

    if !saw_positional_at {
        return None;
    }
    push_body_piece(&mut segments, &body[piece_start..])?;
    Some(segments)
}

fn push_body_piece(
    segments: &mut Vec<QuotedPositionalAtSegment>,
    chars: &[char],
) -> Option<()> {
    if chars.is_empty() {
        return Some(());
    }
    if chars.contains(&'`') || chars.contains(&'\\') {
        return None;
    }
    let raw = chars.iter().collect::<String>();
    segments.push(QuotedPositionalAtSegment::Literal {
        text: crate::lexer::remove_shell_quotes(&raw),
        quoted: true,
    });
    Some(())
}

fn push_quoted_positional_literal_segment(
    segments: &mut Vec<QuotedPositionalAtSegment>,
    chars: &[char],
    quoted: bool,
) -> Option<()> {
    if chars.is_empty() {
        return Some(());
    }
    let raw = chars.iter().collect::<String>();
    if raw.contains(['`', '\\']) {
        return None;
    }
    segments.push(QuotedPositionalAtSegment::Literal {
        text: crate::lexer::remove_shell_quotes(&raw),
        quoted,
    });
    Some(())
}

fn expand_quoted_positional_at_segments<F>(
    segments: &[QuotedPositionalAtSegment],
    positional_params: &[String],
    expand_literal: F,
) -> Vec<String>
where
    F: Fn(&str) -> String,
{
    let mut words = Vec::new();
    let mut current = String::new();
    let mut current_present = false;
    let mut saw_positional_at = false;
    let mut saw_non_empty_expansion = false;

    for (segment_index, segment) in segments.iter().enumerate() {
        match segment {
            QuotedPositionalAtSegment::Literal { text, quoted } => {
                let expanded = expand_literal(text);
                if !expanded.is_empty() {
                    saw_non_empty_expansion = true;
                }
                // Only an unquoted literal directly after $@ emulates the
                // GNU unquoted-suffix rule where trailing IFS whitespace
                // terminates the field instead of joining the next one
                // (e.g. `"$@"$space`). Quoted-literal affixes are data and
                // attach verbatim (GNU: `"  $@  "` keeps its spaces on the
                // first/last positional word).
                if !quoted
                    && segment_index > 0
                    && matches!(
                        segments.get(segment_index - 1),
                        Some(QuotedPositionalAtSegment::PositionalAt)
                    )
                {
                    current.push_str(expanded.trim_end_matches([' ', '\t', '\n']));
                } else {
                    current.push_str(&expanded);
                }
                current_present = true;
            }
            QuotedPositionalAtSegment::PositionalAt => {
                saw_positional_at = true;
                if positional_params.is_empty() {
                    continue;
                }
                saw_non_empty_expansion = true;
                current.push_str(&positional_params[0]);
                current_present = true;

                if positional_params.len() > 1 {
                    words.push(std::mem::take(&mut current));
                    for value in &positional_params[1..positional_params.len() - 1] {
                        words.push(value.clone());
                    }
                    current.push_str(&positional_params[positional_params.len() - 1]);
                }
            }
        }
    }

    if current_present {
        // GNU expand_word_internal (subst.c 12026-12052): a word containing a
        // quoted $@ whose every part expands empty produces NO word at all
        // (quoted_dollar_at beats the quoted-null retention). Only when some
        // part actually expanded (or positional params exist) is the word kept.
        if saw_positional_at && positional_params.is_empty() && !saw_non_empty_expansion {
            return Vec::new();
        }
        words.push(current);
    }

    words
}

fn skip_double_quote(chars: &[char], mut index: usize) -> Option<usize> {
    while index < chars.len() {
        match chars[index] {
            '"' => return Some(index),
            '\\' => index += 2,
            _ => index += 1,
        }
    }
    None
}

fn skip_single_quote(chars: &[char], mut index: usize) -> Option<usize> {
    while index < chars.len() {
        if chars[index] == '\'' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}
