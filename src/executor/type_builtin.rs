use super::*;

impl Executor {
    // rubash#289d: the raw-stdout `execute_command_describe` (raw println!
    // describe, no buffering) was removed — every `command -v/-V`
    // invocation must go through `execute_command_describe_redirected`,
    // whose buffered output reaches the comsub capture via
    // write_buffered_builtin_output and honors redirections elsewhere
    // (mirrors the niubash #108 `type` precedent on this file).

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
            &self.diagnostic_prefix(),
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
            if !use_standard_path && !name.contains('/') && !name.contains('\\') {
                crate::builtins::hash::bump_hashed_path_hit(&mut self.shell_state.env_vars, name);
            }
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

        let saved_path = self.shell_state.env_vars.get("PATH").cloned();
        self.shell_state.env_vars.insert(
            "PATH".to_string(),
            standard_path(&self.shell_state.env_vars),
        );
        Some(saved_path)
    }

    pub(in crate::executor) fn restore_lookup_path(&mut self, saved_path: Option<Option<String>>) {
        let Some(saved_path) = saved_path else {
            return;
        };

        match saved_path {
            Some(path) => {
                self.shell_state.env_vars.insert("PATH".to_string(), path);
            }
            None => {
                self.shell_state.env_vars.remove("PATH");
            }
        }
    }

    // niubash #108: the raw-stdout `execute_type` was removed — every `type`
    // invocation must go through `execute_type_redirected`, whose buffered
    // output reaches `self.stdout_capture` under command substitution
    // (`$(type -t ls)`) and honors explicit redirections elsewhere.

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
        // GNU builtins/type.def:153-154 — `-f` sets CDESC_NOFUNCS, which
        // suppresses ONLY the function lookup (type.def:281); aliases,
        // keywords, builtins and files are still reported.
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
            // GNU type.def -> describe_command -> find_user_command: each
            // non-absolute name lookup runs phash_search, whose hit bumps
            // times_found (hashlib.c:254). `-P` (force_path) skips the table.
            if !force_path && !name.contains('/') && !name.contains('\\') {
                crate::builtins::hash::bump_hashed_path_hit(&mut self.shell_state.env_vars, name);
            }
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
