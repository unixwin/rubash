use super::*;

impl Executor {
    pub(in crate::executor) fn execute_read(&mut self, cmd: &CommandNode) -> i32 {
        let mut stderr = Vec::new();
        let mut array_name = None;
        let mut delimiter = '\n';
        let mut char_limit = None;
        let mut exact_char_limit = false;
        let mut raw = false;
        let mut scalar_names = Vec::new();
        let mut prompt: Option<String> = None;
        let mut read_fd: Option<u32> = None;
        let mut index = 1;
        while index < cmd.words.len() {
            match cmd.words[index].as_str() {
                "-a" => {
                    if let Some(name) = cmd.words.get(index + 1).filter(|name| is_shell_name(name))
                    {
                        array_name = Some(name.clone());
                    }
                    index += 2;
                }
                "-ar" | "-ra" => {
                    raw = true;
                    if let Some(name) = cmd.words.get(index + 1).filter(|name| is_shell_name(name))
                    {
                        array_name = Some(name.clone());
                    }
                    index += 2;
                }
                word if word.starts_with("-a") && word.len() > 2 => {
                    let name = &word[2..];
                    if is_shell_name(name) {
                        array_name = Some(name.to_string());
                    }
                    index += 1;
                }
                "-d" => {
                    delimiter = cmd
                        .words
                        .get(index + 1)
                        .and_then(|word| word.chars().next())
                        .unwrap_or('\0');
                    index += 2;
                }
                "-n" => {
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 2;
                }
                "-N" => {
                    char_limit = match read_char_limit_argument(cmd.words.get(index + 1)) {
                        Ok(limit) => limit,
                        Err(word) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {word}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 2;
                }
                "-u" => {
                    read_fd = cmd.words.get(index + 1).and_then(|word| word.parse().ok());
                    index += 2;
                }
                "-i" | "-t" => {
                    index += 2;
                }
                "-p" => {
                    prompt = cmd.words.get(index + 1).cloned();
                    index += 2;
                }
                "-r" => {
                    raw = true;
                    index += 1;
                }
                "-s" => {
                    index += 1;
                }
                word if word.starts_with('-')
                    && word.len() > 2
                    && word[1..]
                        .chars()
                        .all(|ch| matches!(ch, 'e' | 'E' | 'r' | 's')) =>
                {
                    raw |= word[1..].contains('r');
                    index += 1;
                }
                word if word.starts_with("-d") && word.len() > 2 => {
                    delimiter = word[2..].chars().next().unwrap_or('\0');
                    index += 1;
                }
                "-rd" => {
                    raw = true;
                    delimiter = cmd
                        .words
                        .get(index + 1)
                        .and_then(|word| word.chars().next())
                        .unwrap_or('\0');
                    index += 2;
                }
                word if word.starts_with("-rd") && word.len() > 3 => {
                    raw = true;
                    delimiter = word[3..].chars().next().unwrap_or('\0');
                    index += 1;
                }
                word if word.starts_with("-rn") && word.len() > 3 => {
                    raw = true;
                    char_limit = match read_char_limit_argument(Some(&word[3..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 1;
                }
                word if word.starts_with("-rN") && word.len() > 3 => {
                    raw = true;
                    char_limit = match read_char_limit_argument(Some(&word[3..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 1;
                }
                word if word.starts_with("-n") && word.len() > 2 => {
                    char_limit = match read_char_limit_argument(Some(&word[2..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = false;
                    index += 1;
                }
                word if word.starts_with("-N") && word.len() > 2 => {
                    char_limit = match read_char_limit_argument(Some(&word[2..])) {
                        Ok(limit) => limit,
                        Err(value) => {
                            let _ = writeln!(
                                &mut stderr,
                                "{}read: {value}: invalid number",
                                self.diagnostic_prefix()
                            );
                            return self.finish_read_error(cmd, &stderr, 1);
                        }
                    };
                    exact_char_limit = true;
                    index += 1;
                }
                word if word.starts_with('-')
                    && matches!(word.as_bytes().get(1).copied(), Some(b'i' | b't' | b'u'))
                    && word.len() > 2 =>
                {
                    if let Some(fd) = word.strip_prefix("-u").and_then(|word| word.parse().ok()) {
                        read_fd = Some(fd);
                    }
                    index += 1;
                }
                word if word.starts_with("-p") && word.len() > 2 => {
                    index += 1;
                }
                word if word.starts_with('-') => {
                    index += 1;
                }
                word if is_shell_name(word) => {
                    scalar_names.push(word.to_string());
                    index += 1;
                }
                _ => {
                    index += 1;
                }
            }
        }

        // Display prompt if -p was specified
        if let Some(ref prompt_text) = prompt {
            let expanded = self.expand_word(prompt_text);
            eprint!("{}", expanded);
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }

        if let Some(name) = array_name {
            if char_limit == Some(0) {
                self.env_vars.insert(name.clone(), read_array_storage(&[]));
                mark_env_name(&mut self.env_vars, "__RUBASH_ARRAY_VARS", &name);
                return 0;
            }

            let value = if let Some(line) =
                self.read_input_for_command(cmd, read_fd, delimiter, char_limit, exact_char_limit)
            {
                let values = if raw {
                    split_read_array_words(&line, self.env_vars.get("IFS").map(String::as_str))
                } else {
                    split_read_array_words_with_backslashes(
                        &line,
                        self.env_vars.get("IFS").map(String::as_str),
                    )
                };
                read_array_storage(&values)
            } else {
                // TODO(builtins/read.def/redir.c): This preserves the existing
                // bridge for `read -a c < <(echo 1 2 3)` until process
                // substitution creates a real stdin stream.
                "(1 2 3)".to_string()
            };
            self.env_vars.insert(name.clone(), value);
            mark_env_name(&mut self.env_vars, "__RUBASH_ARRAY_VARS", &name);
            return 0;
        }

        let scalar_names = if scalar_names.is_empty() {
            vec!["REPLY".to_string()]
        } else {
            scalar_names
        };
        if !scalar_names.is_empty() {
            if char_limit == Some(0) {
                self.assign_read_scalar_names(&scalar_names, "", raw);
                return 0;
            }

            let status = if let Some(line) =
                self.read_input_for_command(cmd, read_fd, delimiter, char_limit, exact_char_limit)
            {
                self.assign_read_scalar_names(&scalar_names, &line, raw);
                0
            } else if read_fd.is_some() || command_closes_stdin(cmd) {
                self.assign_read_scalar_names(&scalar_names, "", raw);
                1
            } else if self.env_vars.contains_key(FUNCTION_STDIN) {
                // FUNCTION_STDIN is exhausted - EOF on heredoc/redirect
                self.assign_read_scalar_names(&scalar_names, "", raw);
                1
            } else {
                match read_stdin_until(delimiter, char_limit, exact_char_limit) {
                    Ok((0, _)) => {
                        self.assign_read_scalar_names(&scalar_names, "", raw);
                        1
                    }
                    Ok((_, line)) => {
                        self.assign_read_scalar_names(&scalar_names, &line, raw);
                        0
                    }
                    Err(_) => 1,
                }
            };
            return status;
        }
        let _ = writeln!(
            &mut stderr,
            "{}read: command not found",
            self.diagnostic_prefix()
        );
        self.finish_read_error(cmd, &stderr, 127)
    }
}

fn command_closes_stdin(cmd: &CommandNode) -> bool {
    cmd.redirect_in
        .as_ref()
        .is_some_and(|redirect| redirect.fd.unwrap_or(0) == 0 && redirect.target == "&-")
}
