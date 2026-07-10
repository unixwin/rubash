use super::*;

impl Executor {
    pub(in crate::executor) fn execute_command_describe(&mut self, args: &[String]) -> bool {
        // TODO(builtins/command.def/type.def/findcmd.c): `command -v/-V`
        // shares Bash's command-description machinery with `type`. Keep this
        // executor-local bridge while functions and aliases live on Executor.
        let Some((mode, use_standard_path, first_name)) = parse_command_describe_args(args) else {
            return false;
        };
        let saved_path = self.use_standard_path_for_lookup(use_standard_path);
        let mut status = 0;
        for name in &args[first_name..] {
            if !self.describe_name(name, mode, false, false) {
                status = 1;
                if mode == TypeDescribeMode::Verbose {
                    eprintln!("{}command: {name}: not found", self.diagnostic_prefix());
                }
            }
        }
        self.restore_lookup_path(saved_path);
        self.exit_code = status;
        true
    }

    pub(in crate::executor) fn execute_command_describe_redirected(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if self.execute_command_describe_with_io(&cmd.words[1..], &mut stdout, &mut stderr)? {
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(true);
        }

        match crate::builtins::command::execute_with_io(
            cmd.words[1..].iter().map(String::as_str),
            &mut stdout,
            &mut stderr,
        )? {
            crate::builtins::command::CommandAction::Complete(status) => {
                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                self.exit_code = status;
                Ok(true)
            }
            crate::builtins::command::CommandAction::Execute { .. } => Ok(false),
        }
    }

    pub(in crate::executor) fn execute_command_describe_with_io<W, E>(
        &mut self,
        args: &[String],
        stdout: &mut W,
        stderr: &mut E,
    ) -> Result<bool, ExecuteError>
    where
        W: Write,
        E: Write,
    {
        let Some((mode, use_standard_path, first_name)) = parse_command_describe_args(args) else {
            return Ok(false);
        };
        let saved_path = self.use_standard_path_for_lookup(use_standard_path);
        let mut status = 0;
        for name in &args[first_name..] {
            if !self.describe_name_with_io(name, mode, false, false, stdout)? {
                status = 1;
                if mode == TypeDescribeMode::Verbose {
                    writeln!(
                        stderr,
                        "{}command: {name}: not found",
                        self.diagnostic_prefix()
                    )?;
                }
            }
        }
        self.restore_lookup_path(saved_path);
        self.exit_code = status;
        Ok(true)
    }

    pub(in crate::executor) fn use_standard_path_for_lookup(
        &mut self,
        enabled: bool,
    ) -> Option<Option<String>> {
        if !enabled {
            return None;
        }

        let saved_path = self.env_vars.get("PATH").cloned();
        self.env_vars
            .insert("PATH".to_string(), standard_path(&self.env_vars));
        Some(saved_path)
    }

    pub(in crate::executor) fn restore_lookup_path(&mut self, saved_path: Option<Option<String>>) {
        let Some(saved_path) = saved_path else {
            return;
        };

        match saved_path {
            Some(path) => {
                self.env_vars.insert("PATH".to_string(), path);
            }
            None => {
                self.env_vars.remove("PATH");
            }
        }
    }

    pub(in crate::executor) fn execute_type(&mut self, args: &[String]) -> i32 {
        // TODO(builtins/type.def): Port Bash's `describe_command` and `type`
        // option parser completely. This context-aware implementation covers
        // upstream type.tests' function/alias/keyword/builtin/hash cases.
        let mut mode = TypeDescribeMode::Verbose;
        let mut all = false;
        let mut force_path = false;
        let mut skip_functions = false;
        let mut index = 0;

        while let Some(arg) = args.get(index) {
            if arg == "--" {
                index += 1;
                break;
            }
            if !arg.starts_with('-') || arg == "-" {
                break;
            }
            let normalized = normalize_type_option(arg);
            for option in normalized[1..].chars() {
                match option {
                    'a' => all = true,
                    'f' => skip_functions = true,
                    'p' => mode = TypeDescribeMode::PathOnly,
                    'P' => {
                        mode = TypeDescribeMode::PathOnly;
                        force_path = true;
                    }
                    't' => mode = TypeDescribeMode::TypeOnly,
                    other => {
                        eprintln!("{}type: -{other}: invalid option", self.diagnostic_prefix());
                        eprintln!("type: usage: type [-afptP] name [name ...]");
                        return 2;
                    }
                }
            }
            index += 1;
        }

        let mut status = 0;
        for name in &args[index..] {
            let found = if all {
                match self.describe_name_all(name, mode, force_path, skip_functions) {
                    Ok(found) => found,
                    Err(error) => {
                        eprintln!("rubash: type: {error}");
                        false
                    }
                }
            } else {
                self.describe_name(name, mode, force_path, skip_functions)
            };
            if !found {
                status = 1;
                if mode == TypeDescribeMode::Verbose {
                    eprintln!("{}type: {name}: not found", self.diagnostic_prefix());
                }
            }
        }
        status
    }

    pub(in crate::executor) fn execute_type_redirected(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = self.execute_type_with_io(&cmd.words[1..], &mut stdout, &mut stderr)?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_type_with_io<W, E>(
        &mut self,
        args: &[String],
        stdout: &mut W,
        stderr: &mut E,
    ) -> Result<i32, ExecuteError>
    where
        W: Write,
        E: Write,
    {
        if let Some(status) = self.execute_type_with_disabled_builtin_state_with_io(args, stdout)? {
            return Ok(status);
        }

        let mut mode = TypeDescribeMode::Verbose;
        let mut all = false;
        let mut force_path = false;
        let mut skip_functions = false;
        let mut index = 0;

        while let Some(arg) = args.get(index) {
            if arg == "--" {
                index += 1;
                break;
            }
            if !arg.starts_with('-') || arg == "-" {
                break;
            }
            let normalized = normalize_type_option(arg);
            for option in normalized[1..].chars() {
                match option {
                    'a' => all = true,
                    'f' => skip_functions = true,
                    'p' => mode = TypeDescribeMode::PathOnly,
                    'P' => {
                        mode = TypeDescribeMode::PathOnly;
                        force_path = true;
                    }
                    't' => mode = TypeDescribeMode::TypeOnly,
                    other => {
                        writeln!(
                            stderr,
                            "{}type: -{other}: invalid option",
                            self.diagnostic_prefix()
                        )?;
                        writeln!(stderr, "type: usage: type [-afptP] name [name ...]")?;
                        return Ok(2);
                    }
                }
            }
            index += 1;
        }

        let mut status = 0;
        for name in &args[index..] {
            let found = if all {
                self.describe_name_all_with_io(name, mode, force_path, skip_functions, stdout)?
            } else {
                self.describe_name_with_io(name, mode, force_path, skip_functions, stdout)?
            };
            if !found {
                status = 1;
                if mode == TypeDescribeMode::Verbose {
                    writeln!(
                        stderr,
                        "{}type: {name}: not found",
                        self.diagnostic_prefix()
                    )?;
                }
            }
        }
        Ok(status)
    }
}
