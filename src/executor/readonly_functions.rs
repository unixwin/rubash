use super::*;

impl Executor {
    pub(in crate::executor) fn execute_readonly(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if readonly_args_request_functions(&cmd.words[1..]) {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let status =
                self.execute_readonly_functions(&cmd.words[1..], &mut stdout, &mut stderr)?;
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(status);
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::setattr::readonly_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::setattr::readonly_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return Ok(crate::builtins::setattr::readonly_with_io(
                    cmd.words[1..].iter().map(String::as_str),
                    &mut self.env_vars,
                    &mut std::io::stdout().lock(),
                    &mut std::io::sink(),
                )?);
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::setattr::readonly_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::setattr::readonly_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        Ok(crate::builtins::setattr::readonly(
            &cmd.words[1..],
            &mut self.env_vars,
        )?)
    }

    pub(in crate::executor) fn execute_readonly_functions<W, E>(
        &mut self,
        args: &[String],
        stdout: &mut W,
        stderr: &mut E,
    ) -> io::Result<i32>
    where
        W: Write,
        E: Write,
    {
        let mut print = false;
        let mut index = 0;
        while let Some(arg) = args.get(index) {
            if arg == "--" {
                index += 1;
                break;
            }
            if !arg.starts_with('-') || arg == "-" {
                break;
            }
            for option in arg[1..].chars() {
                match option {
                    'f' => {}
                    'p' => print = true,
                    'a' | 'A' => {}
                    other => {
                        writeln!(
                            stderr,
                            "{}readonly: -{other}: invalid option",
                            self.diagnostic_prefix()
                        )?;
                        writeln!(
                            stderr,
                            "readonly: usage: readonly [-aAf] [name[=value] ...] or readonly -p"
                        )?;
                        return Ok(2);
                    }
                }
            }
            index += 1;
        }

        if print && index >= args.len() {
            let mut names = marked_env_names(&self.env_vars, READONLY_FUNCTIONS);
            names.sort();
            for name in names {
                if let Some(body) = self.functions.get(&name) {
                    self.write_function_definition(&name, body, false, stdout)?;
                    writeln!(stdout, "declare -fr {name}")?;
                }
            }
            return Ok(0);
        }

        let mut status = 0;
        for name in &args[index..] {
            let Some(body) = self.functions.get(name) else {
                writeln!(
                    stderr,
                    "{}readonly: {name}: not a function",
                    self.diagnostic_prefix()
                )?;
                status = 1;
                continue;
            };
            if print {
                self.write_function_definition(name, body, false, stdout)?;
                writeln!(stdout, "declare -fr {name}")?;
            }
            mark_env_name(&mut self.env_vars, READONLY_FUNCTIONS, name);
        }

        Ok(status)
    }

    pub(in crate::executor) fn write_function_definition<W>(
        &self,
        name: &str,
        body: &[CommandNode],
        exported: bool,
        stdout: &mut W,
    ) -> io::Result<()>
    where
        W: Write,
    {
        if exported {
            writeln!(stdout, "declare -fx {name}")?;
        }
        writeln!(stdout, "{name} () ")?;
        writeln!(stdout, "{{ ")?;
        let printable_commands = body
            .iter()
            .filter(|command| !command.words.is_empty())
            .collect::<Vec<_>>();
        let last_index = printable_commands.len().saturating_sub(1);
        let mut indent_level = 1usize;
        for (index, command) in printable_commands.iter().enumerate() {
            if command.words.is_empty() {
                continue;
            }
            if function_definition_command_closes_block(command) {
                indent_level = indent_level.saturating_sub(1).max(1);
            }
            let indent = "    ".repeat(indent_level);
            let terminator =
                if function_definition_command_omits_terminator(command) || index == last_index {
                    ""
                } else {
                    ";"
                };
            if let Some(here_string) = &command.here_string {
                writeln!(
                    stdout,
                    "{indent}{} <<< {}{}",
                    command.words.join(" "),
                    function_here_string_text(here_string, printable_commands.len() > 1),
                    terminator
                )?;
            } else if command.words == ["time"] {
                writeln!(stdout, "{indent}time {terminator}")?;
            } else if command.heredoc.is_some() {
                let line = self
                    .function_command_description_line(command, false)
                    .unwrap_or_else(|| command.words.join(" "));
                writeln!(stdout, "{indent}{line}")?;
                write_function_definition_heredoc_body(command, stdout)?;
            } else {
                writeln!(stdout, "{indent}{}{terminator}", command.words.join(" "))?;
            }
            if function_definition_command_opens_nested_body(command) {
                indent_level += 1;
            }
        }
        writeln!(stdout, "}}")
    }

    pub(in crate::executor) fn apply_exported_functions_to_child(&self, process: &mut Command) {
        for name in marked_env_names(&self.env_vars, EXPORTED_FUNCTIONS) {
            let Some(body) = self.functions.get(&name) else {
                continue;
            };
            process.env(
                exported_function_env_name(&name),
                exported_function_env_value(body),
            );
        }
    }

    pub(in crate::executor) fn apply_child_environment(&self, process: &mut Command) {
        process.env_clear();
        for name in marked_env_names(&self.env_vars, EXPORTED_VARS) {
            if let Some(value) = self.env_vars.get(&name) {
                if is_valid_process_env(&name, value) {
                    process.env(&name, self.child_env_value(&name, value));
                }
            }
        }
        for (name, value) in local_export_env_values(&self.env_vars) {
            if is_valid_process_env(&name, &value) {
                process.env(&name, self.child_env_value(&name, &value));
            }
        }
        self.apply_exported_functions_to_child(process);
    }

    pub(in crate::executor) fn child_env_value(&self, name: &str, value: &str) -> String {
        if cfg!(windows) && name == "TMPDIR" {
            return shell_display_path(
                &shell_path_to_windows(value, &self.env_vars)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
        value.to_string()
    }
}
