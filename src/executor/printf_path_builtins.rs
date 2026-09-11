use super::*;

impl Executor {
    pub(in crate::executor) fn execute_printf(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::printf::execute_with_io_and_store(
            cmd.words[1..].iter().map(String::as_str),
            &mut self.env_vars,
            Some(&mut self.shell_state.variables),
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_exit(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<crate::builtins::exit::ExitAction, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let action = crate::builtins::exit::execute_with_io(
            cmd.words[1..].iter().map(String::as_str),
            self.exit_code,
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(action)
    }

    pub(in crate::executor) fn execute_logout(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        // GNU builtins/exit.def:79-90 (logout_builtin): only a login shell
        // may logout; anything else reports and continues.
        let login_shell = self
            .env_vars
            .get("__RUBASH_LOGIN_SHELL")
            .map(String::as_str)
            == Some("1");
        if !login_shell {
            let mut stderr = Vec::new();
            let status =
                crate::builtins::logout::execute_with_io(&self.diagnostic_prefix(), &mut stderr)?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            return Ok(status);
        }

        // GNU exit.def:147,156-166 (exit_or_logout -> bash_logout): a login
        // shell sources ~/.bash_logout once before exiting.
        if let Some(home) = self.env_vars.get("HOME").cloned() {
            let logout_file = format!("{home}/.bash_logout");
            if !self.bash_logout_sourced
                && std::fs::metadata(shell_path_to_windows(&logout_file, &self.env_vars)).is_ok()
            {
                self.bash_logout_sourced = true;
                let mut node = CommandNode::default();
                node.words = vec![".".to_string(), logout_file];
                self.execute_source_command(&node)?;
            }
        }

        // GNU exit.def:93-154 (exit_or_logout): jump_to_top_level(EXITBLTIN)
        // - the shell exits with the logout status (an optional numeric
        // argument overrides it), after the regular exit trap.
        let status = match cmd.words.get(1) {
            Some(word) => self.expand_word(word).trim().parse::<i32>().unwrap_or(0),
            None => self.exit_code,
        };
        let status = self.run_exit_trap_for_status(status)?;
        Err(ExecuteError::ExitCode(status))
    }

    pub(in crate::executor) fn try_execute_dirname_fast_path(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<i32> {
        let operands = simple_path_tool_operands(&cmd.words[1..])?;
        if operands.is_empty() {
            return None;
        }

        let mut stdout = Vec::new();
        for operand in operands {
            let path = self.expand_word(operand);
            stdout.extend_from_slice(dirname_value(&path).as_bytes());
            stdout.push(b'\n');
        }
        Some(self.write_path_tool_output(cmd, &stdout))
    }

    pub(in crate::executor) fn try_execute_basename_fast_path(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<i32> {
        let operands = simple_path_tool_operands(&cmd.words[1..])?;
        if operands.is_empty() || operands.len() > 2 {
            return None;
        }

        let name = self.expand_word(operands[0]);
        let mut value = basename_value(&name);
        if let Some(suffix) = operands.get(1) {
            let suffix = self.expand_word(suffix);
            value = strip_basename_suffix(&value, &suffix);
        }

        let mut stdout = Vec::new();
        stdout.extend_from_slice(value.as_bytes());
        stdout.push(b'\n');
        Some(self.write_path_tool_output(cmd, &stdout))
    }

    fn write_path_tool_output(&mut self, cmd: &CommandNode, stdout: &[u8]) -> i32 {
        if self
            .write_buffered_builtin_output(cmd, stdout, &[])
            .is_err()
        {
            return 1;
        }
        0
    }

    pub(in crate::executor) fn execute_cd(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return Ok(crate::builtins::cd::execute_with_io(
                    cmd.words[1..].iter().map(String::as_str),
                    &mut self.env_vars,
                    &mut std::io::sink(),
                    &mut std::io::stderr().lock(),
                )?);
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::cd::execute_with_io(
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
            return Ok(crate::builtins::cd::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return Ok(crate::builtins::cd::execute_with_io(
                    cmd.words[1..].iter().map(String::as_str),
                    &mut self.env_vars,
                    &mut std::io::stdout().lock(),
                    &mut std::io::sink(),
                )?);
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::cd::execute_with_io(
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
            return Ok(crate::builtins::cd::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        let status = crate::builtins::cd::execute(&cmd.words[1..], &mut self.env_vars)?;
        self.sync_cd_variables();
        Ok(status)
    }

    pub(in crate::executor) fn sync_cd_variables(&mut self) {
        for name in ["PWD", "OLDPWD"] {
            let Some(value) = self.env_vars.get(name).cloned() else {
                continue;
            };
            if let Some(variable) = self.shell_state.variables.get_mut(name) {
                variable.value = crate::shell::ShellValue::Scalar(value);
            }
        }
    }
}

fn simple_path_tool_operands(args: &[String]) -> Option<Vec<&str>> {
    let mut operands = Vec::new();
    let mut parse_options = true;
    for arg in args {
        if parse_options && arg == "--" {
            parse_options = false;
            continue;
        }
        if parse_options && arg.starts_with('-') && arg.len() > 1 {
            return None;
        }
        operands.push(arg.as_str());
    }
    Some(operands)
}

fn basename_value(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let trimmed = trim_trailing_slashes(&normalized);
    if trimmed.is_empty() {
        return "/".to_string();
    }
    trimmed
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(&trimmed)
        .to_string()
}

fn dirname_value(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let trimmed = trim_trailing_slashes(&normalized);
    if trimmed.is_empty() {
        return "/".to_string();
    }
    let Some((dir, _)) = trimmed.rsplit_once('/') else {
        return ".".to_string();
    };
    let dir = trim_trailing_slashes(dir);
    if dir.is_empty() {
        "/".to_string()
    } else {
        dir
    }
}

fn trim_trailing_slashes(value: &str) -> String {
    let trimmed = value.trim_end_matches('/');
    if trimmed.is_empty() && value.contains('/') {
        String::new()
    } else {
        trimmed.to_string()
    }
}

fn strip_basename_suffix(name: &str, suffix: &str) -> String {
    if suffix.len() < name.len() && name.ends_with(suffix) {
        name[..name.len() - suffix.len()].to_string()
    } else {
        name.to_string()
    }
}
