use super::*;

impl Executor {
    pub(in crate::executor) fn expand_embedded_parameters_mut(&mut self, word: &str) -> String {
        self.apply_parameter_assignment_expansions_in_word(word);
        let word = self.expand_embedded_arithmetic_mut(word);
        let word = self.expand_embedded_command_substitutions_mut(&word);
        let expanded = self.expand_embedded_parameters(&word);
        let expanded = if word.contains("$(") || word.contains('`') {
            unescape_remaining_shell_escapes(&expanded)
                .replace("\\\\'", "'")
                .replace("\\'", "'")
        } else {
            expanded
        };
        restore_protected_replacement_quotes(&expanded)
    }

    pub(in crate::executor) fn expand_embedded_command_substitutions_mut(
        &mut self,
        word: &str,
    ) -> String {
        let mut output = String::new();
        let mut chars = word.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '$' && chars.peek().copied() == Some('(') {
                chars.next();
                if chars.peek().copied() == Some('(') {
                    output.push_str("$((");
                    chars.next();
                    continue;
                }

                let mut depth = 1usize;
                let mut source = String::new();
                let mut single = false;
                let mut double = false;
                let mut escaped = false;
                for source_ch in chars.by_ref() {
                    if escaped {
                        source.push(source_ch);
                        escaped = false;
                        continue;
                    }
                    if source_ch == '\\' && !single {
                        source.push(source_ch);
                        escaped = true;
                        continue;
                    }
                    match source_ch {
                        '\'' if !double => {
                            single = !single;
                            source.push(source_ch);
                        }
                        '"' if !single => {
                            double = !double;
                            source.push(source_ch);
                        }
                        '(' if !single && !double => {
                            depth += 1;
                            source.push(source_ch);
                        }
                        ')' if !single && !double => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                            source.push(source_ch);
                        }
                        _ => source.push(source_ch),
                    }
                }
                let source = unescape_storage_command_substitution_source(&source);
                output.push_str(&protect_command_substitution_output(
                    &self.expand_command_substitution_mut(&source),
                ));
                continue;
            }

            if ch == '`' {
                let mut source = String::new();
                let mut escaped = false;
                let mut closed = false;
                for source_ch in chars.by_ref() {
                    if escaped {
                        source.push(source_ch);
                        escaped = false;
                        continue;
                    }
                    if source_ch == '\\' {
                        escaped = true;
                        continue;
                    }
                    if source_ch == '`' {
                        closed = true;
                        break;
                    }
                    source.push(source_ch);
                }
                if closed {
                    output.push_str(&protect_command_substitution_output(
                        &self.expand_command_substitution_mut(&source),
                    ));
                } else {
                    output.push('`');
                    output.push_str(&source);
                }
                continue;
            }

            output.push(ch);
        }

        output
    }

    pub(in crate::executor) fn expand_command_substitution_mut(&mut self, source: &str) -> String {
        let source = source.trim();
        let words = self.expand_aliases(&split_shell_words(source));
        if let Some(output) = self.run_function_command_substitution(&words) {
            return output;
        }
        self.expand_command_substitution(source)
    }

    pub(in crate::executor) fn run_function_command_substitution(
        &mut self,
        words: &[String],
    ) -> Option<String> {
        let name = words.first()?;
        if !self.functions.contains_key(name) {
            return None;
        }

        let args = words[1..]
            .iter()
            .flat_map(|word| self.expand_command_substitution_arg_values(word))
            .collect::<Vec<_>>();
        let mut call = CommandNode::new();
        call.words = words.to_vec();

        let saved_env = self.env_vars.clone();
        let saved_exit_code = self.exit_code;
        let saved_capture = self.stdout_capture.take();
        self.stdout_capture = Some(Vec::new());
        let result = self.execute_function(name, &args, &call);
        let output = self.stdout_capture.take().unwrap_or_default();
        self.stdout_capture = saved_capture;
        let status = match result {
            Ok(()) => self.exit_code,
            Err(ExecuteError::Return(status)) => status,
            Err(ExecuteError::ExitCode(status)) => status,
            Err(_) => 1,
        };
        self.env_vars = saved_env;
        self.exit_code = saved_exit_code;
        self.last_command_substitution_status.set(Some(status));

        Some(
            String::from_utf8_lossy(&output)
                .trim_end_matches('\n')
                .to_string(),
        )
    }

    pub(in crate::executor) fn expand_embedded_arithmetic_mut(&mut self, word: &str) -> String {
        let chars: Vec<char> = word.chars().collect();
        let mut output = String::new();
        let mut index = 0;

        while index < chars.len() {
            if chars[index] == '$'
                && chars.get(index + 1) == Some(&'(')
                && chars.get(index + 2) == Some(&'(')
            {
                index += 3;
                let mut expression = String::new();
                let mut paren_depth: usize = 0;
                let mut matched = false;

                while index < chars.len() {
                    match chars[index] {
                        '(' => {
                            paren_depth += 1;
                            expression.push(chars[index]);
                            index += 1;
                        }
                        ')' if paren_depth == 0 && chars.get(index + 1) == Some(&')') => {
                            index += 2;
                            matched = true;
                            break;
                        }
                        ')' => {
                            paren_depth = paren_depth.saturating_sub(1);
                            expression.push(chars[index]);
                            index += 1;
                        }
                        ch => {
                            expression.push(ch);
                            index += 1;
                        }
                    }
                }

                if matched {
                    if let Some(value) = self.eval_arithmetic_command_value(&expression) {
                        output.push_str(&value.to_string());
                    }
                } else {
                    output.push_str("$((");
                    output.push_str(&expression);
                }
                continue;
            }

            output.push(chars[index]);
            index += 1;
        }

        output
    }
}
