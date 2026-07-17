use super::*;

impl Executor {
    pub(in crate::executor) fn mktemp_command_substitution(
        &self,
        words: &[String],
    ) -> Option<String> {
        // TODO(subst.c/execute_cmd.c): command substitution should fork a
        // subshell and capture external command stdout. This covers common
        // script prologues like `tmp=$(mktemp -t name.XXXXXX) || exit`.
        if words.first().map(String::as_str) != Some("mktemp") {
            return None;
        }
        let mut directory = false;
        let mut template = "rubash-mktemp.XXXXXX";
        let mut index = 1;
        while index < words.len() {
            match words[index].as_str() {
                "-d" => {
                    directory = true;
                    index += 1;
                }
                "-t" => {
                    template = words.get(index + 1)?.as_str();
                    index += 2;
                }
                value if value.starts_with('-') => return None,
                value => {
                    template = value;
                    index += 1;
                }
            }
        }
        let dir = self
            .env_vars
            .get("TMPDIR")
            .filter(|value| !value.contains('\0'))
            .cloned()
            .unwrap_or_else(safe_temp_dir_string);
        let dir = shell_path_to_windows(&dir, &self.env_vars);
        std::fs::create_dir_all(&dir).ok()?;
        let mut path = None;
        for attempt in 0..32 {
            let unique = format!(
                "{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0),
                attempt
            );
            let filename = if template.contains("XXXXXX") {
                template.replace("XXXXXX", &unique)
            } else {
                format!("{template}.{unique}")
            };
            let candidate = dir.join(filename);
            let created = if directory {
                std::fs::create_dir_all(&candidate).is_ok()
            } else {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&candidate)
                    .is_ok()
            };
            if created {
                path = Some(candidate);
                break;
            }
        }
        let path = path?;
        self.last_command_substitution_status.set(Some(0));
        Some(shell_display_path(
            &path.to_string_lossy().replace('\\', "/"),
        ))
    }

    pub(in crate::executor) fn command_substitution_heredoc_output(
        &self,
        source: &str,
    ) -> Option<String> {
        if !source.contains("<<") {
            return None;
        }

        let closed_by_paren = source.contains('\x1c');
        let source = source.replace('\x1c', "");
        let tokens = crate::lexer::tokenize(&source);
        let ast = crate::parser::parse(&tokens);
        let first = ast.commands.first()?;
        let (first, piped_next) = if let Some(pipeline_command) = &first.pipeline_command {
            (
                pipeline_command.stages.first()?,
                pipeline_command.stages.get(1),
            )
        } else {
            (first, ast.commands.get(1))
        };
        if first.words.first().map(String::as_str) != Some("cat") {
            return None;
        }

        if closed_by_paren {
            self.report_command_substitution_heredoc_warning(&source, first);
        }

        if first.pipe.is_none() && ast.commands.len() > 1 {
            let mut output = String::new();
            for command in &ast.commands {
                if command.words.first().map(String::as_str) != Some("cat")
                    || command.pipe.is_some()
                {
                    return None;
                }
                output.push_str(&self.stdin_string_for_command(command)?);
            }
            return Some(output.trim_end_matches('\n').to_string());
        }

        let mut output = self.stdin_string_for_command(first)?;
        if first.pipe.is_some() {
            let next = piped_next?;
            match next.words.as_slice() {
                [cmd, option] if cmd == "sort" && option == "-u" => {
                    let mut lines = output.lines().map(str::to_string).collect::<Vec<_>>();
                    lines.sort();
                    lines.dedup();
                    output = lines.join("\n");
                    output.push('\n');
                }
                _ => return None,
            }
        }

        Some(output.trim_end_matches('\n').to_string())
    }

    pub(in crate::executor) fn command_substitution_pipeline_output(
        &self,
        words: &[String],
    ) -> Option<String> {
        if !words.iter().any(|word| word == "|") {
            return None;
        }

        let stages = split_pipeline_words(words)?;
        let mut output = self.command_substitution_pipeline_first_stage(stages.first()?)?;
        for stage in stages.iter().skip(1) {
            output = self.command_substitution_pipeline_filter(stage, &output)?;
        }
        Some(output.trim_end_matches('\n').to_string())
    }

    pub(in crate::executor) fn timed_command_substitution_output(
        &self,
        words: &[String],
    ) -> Option<String> {
        if words.first().map(String::as_str) != Some("time") {
            return None;
        }

        let mut index = 1;
        let mut inverted = false;
        while let Some(word) = words.get(index).map(String::as_str) {
            match word {
                "-p" | "--" | "time" => index += 1,
                "!" => {
                    inverted = !inverted;
                    index += 1;
                }
                _ => break,
            }
        }

        let output = self.timed_command_substitution_inner(&words[index..])?;
        print_time(
            &self.env_vars,
            words
                .iter()
                .skip(1)
                .take_while(|word| word.as_str() != "!")
                .any(|word| word == "-p"),
        );
        let status = self.last_command_substitution_status.get().unwrap_or(0);
        self.last_command_substitution_status.set(Some(if inverted {
            invert_exit_status(status)
        } else {
            status
        }));
        Some(output)
    }

    fn timed_command_substitution_inner(&self, words: &[String]) -> Option<String> {
        match words.first().map(String::as_str) {
            None | Some(":") | Some("true") => {
                self.last_command_substitution_status.set(Some(0));
                Some(String::new())
            }
            Some("false") => {
                self.last_command_substitution_status.set(Some(1));
                Some(String::new())
            }
            Some("echo") => {
                let args = words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                self.last_command_substitution_status.set(Some(0));
                Some(echo_command_substitution_output(&args))
            }
            Some("printf") => {
                let expanded_args = words[1..]
                    .iter()
                    .map(|word| strip_matching_quotes(&self.expand_word(word)).to_string())
                    .collect::<Vec<_>>();
                let mut env_vars = self.env_vars.clone();
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let status = crate::builtins::printf::execute_with_io(
                    expanded_args.iter().map(String::as_str),
                    &mut env_vars,
                    &mut stdout,
                    &mut stderr,
                )
                .unwrap_or(1);
                self.last_command_substitution_status.set(Some(status));
                Some(
                    String::from_utf8_lossy(&stdout)
                        .trim_end_matches('\n')
                        .to_string(),
                )
            }
            Some(_) if words.iter().any(|word| word == "|") => {
                let output = self.command_substitution_pipeline_output(words)?;
                self.last_command_substitution_status.set(Some(0));
                Some(output)
            }
            Some("cat") => {
                let mut output = String::new();
                for word in &words[1..] {
                    let path = self.expand_word(word);
                    if let Ok(value) =
                        fs::read_to_string(shell_path_to_windows(&path, &self.env_vars))
                    {
                        output.push_str(&value);
                    }
                }
                self.last_command_substitution_status.set(Some(0));
                Some(output.trim_end_matches('\n').to_string())
            }
            Some(_) => self.run_external_command_substitution(words),
        }
    }

    pub(in crate::executor) fn command_substitution_pipeline_first_stage(
        &self,
        words: &[String],
    ) -> Option<String> {
        match words.first().map(String::as_str)? {
            "echo" => {
                let args = words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                let mut output = echo_command_substitution_output(&args);
                output.push('\n');
                Some(output)
            }
            "printf" => {
                let expanded_args: Vec<String> = words[1..]
                    .iter()
                    .flat_map(|word| self.expand_command_substitution_arg_values(word))
                    .collect();
                let mut env_vars = self.env_vars.clone();
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let _ = crate::builtins::printf::execute_with_io(
                    expanded_args.iter().map(String::as_str),
                    &mut env_vars,
                    &mut stdout,
                    &mut stderr,
                );
                Some(String::from_utf8_lossy(&stdout).into_owned())
            }
            "cat" => {
                let mut output = String::new();
                for word in &words[1..] {
                    let path = self.expand_word(word);
                    if let Ok(value) =
                        fs::read_to_string(shell_path_to_windows(&path, &self.env_vars))
                    {
                        output.push_str(&value);
                    }
                }
                Some(output)
            }
            "command" => self
                .command_describe_substitution_output(words)
                .map(|mut output| {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output
                }),
            _ => {
                // Generic external command first stage
                let cmd_name = self.expand_word(&words[0]);
                let expanded_args: Vec<String> =
                    words[1..].iter().map(|w| self.expand_word(w)).collect();
                use std::process::{Command, Stdio};
                let output = Command::new(&cmd_name)
                    .args(&expanded_args)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .output()
                    .ok()?;
                Some(String::from_utf8_lossy(&output.stdout).into_owned())
            }
        }
    }
}
