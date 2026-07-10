use super::*;

impl Executor {
    pub(in crate::executor) fn command_substitution_pipeline_filter(
        &self,
        words: &[String],
        input: &str,
    ) -> Option<String> {
        match words.first().map(String::as_str)? {
            "sed" => {
                let script = strip_matching_quotes(sed_script_arg(&words[1..])?);
                apply_simple_sed_substitution(input, script)
            }
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
                Some(output)
            }
            _ => {
                // Generic external command filter - run command with stdin
                let cmd_name = self.expand_word(&words[0]);
                let expanded_args: Vec<String> =
                    words[1..].iter().map(|w| self.expand_word(w)).collect();
                use std::io::Write;
                use std::process::{Command, Stdio};
                let child = Command::new(&cmd_name)
                    .args(&expanded_args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .ok()?;
                child.stdin.as_ref()?.write_all(input.as_bytes()).ok()?;
                let output = child.wait_with_output().ok()?;
                Some(
                    String::from_utf8_lossy(&output.stdout)
                        .trim_end_matches('\n')
                        .to_string(),
                )
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
        vec![strip_matching_quotes(&self.expand_word(word)).to_string()]
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
            match self.describe_name_with_io(&name, mode, use_standard_path, false, &mut stdout) {
                Ok(true) => {}
                Ok(false) => status = 1,
                Err(_) => status = 1,
            }
        }
        self.last_command_substitution_status.set(Some(status));
        Some(
            String::from_utf8_lossy(&stdout)
                .trim_end_matches('\n')
                .to_string(),
        )
    }

    pub(in crate::executor) fn quoted_positional_at_word_values(
        &self,
        word: &str,
        kind: Option<&TokenKind>,
    ) -> Option<Vec<String>> {
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
            if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
                if var_name == "@" {
                    return Some(positional_parameter_substring(
                        &self.positional_params,
                        offset,
                        length,
                    ));
                }
            }
        }
        None
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
        if expression.ends_with("[*]") {
            let separator = self
                .env_vars
                .get("IFS")
                .and_then(|ifs| ifs.chars().next())
                .unwrap_or(' ');
            return values.join(&separator.to_string());
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
        if words
            .iter()
            .any(|word| matches!(word.as_str(), "|" | ">" | ">>" | "<" | "2>" | "2>>" | "&>"))
        {
            return None;
        }

        let expanded_words: Vec<String> = words
            .iter()
            .map(|word| strip_matching_quotes(&self.expand_word(word)).to_string())
            .collect();
        let Some(program) = find_user_command(&expanded_words[0], &self.env_vars) else {
            self.last_command_substitution_status.set(Some(127));
            return Some(String::new());
        };
        let mut process = if should_run_with_shell(&program) {
            if let Some(shell) = find_shell(&self.env_vars) {
                let mut command = Command::new(shell);
                command.arg(&program);
                command.args(&expanded_words[1..]);
                command
            } else {
                Command::new(&program)
            }
        } else {
            let mut command = Command::new(&program);
            command.args(&expanded_words[1..]);
            command
        };

        self.apply_child_environment(&mut process);
        let output = process.output().ok()?;
        let status = output.status.code().unwrap_or(1);
        self.last_command_substitution_status.set(Some(status));
        Some(
            String::from_utf8_lossy(&output.stdout)
                .trim_end_matches('\n')
                .to_string(),
        )
    }

    pub(in crate::executor) fn expand_backtick_substitution(&self, word: &str) -> Option<String> {
        // TODO(subst.c): Backquote command substitution should invoke the
        // parser and run a subshell. This reuses the same in-process command
        // substitution bridge as `$()`.
        if !backtick_substitution_spans_whole_word(word) {
            return None;
        }
        let source = word.strip_prefix('`')?.strip_suffix('`')?;
        Some(self.expand_command_substitution(source))
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
