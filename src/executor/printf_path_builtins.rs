use super::*;

impl Executor {
    pub(in crate::executor) fn execute_printf(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        // TODO(redir.c/execute_cmd.c/builtins/printf.def): Redirections are a
        // general command property in Bash. This covers stdout redirection for
        // builtin `printf`, which upstream builtins.tests uses to create files
        // later sourced by `.`.
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if target == "&2" {
                return Ok(crate::builtins::printf::execute_with_io(
                    cmd.words[1..].iter().map(String::as_str),
                    &mut self.env_vars,
                    &mut std::io::stderr().lock(),
                    &mut std::io::stderr().lock(),
                )?);
            }
            if is_null_device(&target) {
                return Ok(crate::builtins::printf::execute_with_io(
                    cmd.words[1..].iter().map(String::as_str),
                    &mut self.env_vars,
                    &mut std::io::sink(),
                    &mut std::io::stderr().lock(),
                )?);
            }
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            return Ok(crate::builtins::printf::execute_with_io(
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
            return Ok(crate::builtins::printf::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::printf::execute_with_io(
            cmd.words[1..].iter().map(String::as_str),
            &mut self.env_vars,
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
        let mut stderr = Vec::new();
        let status =
            crate::builtins::logout::execute_with_io(&self.diagnostic_prefix(), &mut stderr)?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_dirname(&mut self, cmd: &CommandNode) -> i32 {
        let mut paths = Vec::new();
        for arg in &cmd.words[1..] {
            if !arg.starts_with('-') {
                paths.push(self.expand_word(arg));
            }
        }
        if paths.is_empty() {
            eprintln!("{}dirname: missing operand", self.diagnostic_prefix());
            return 1;
        }
        for path in &paths {
            let normalized = path.replace('\\', "/");
            let dir = if let Some(pos) = normalized.rfind('/') {
                let d = &normalized[..pos];
                if d.is_empty() {
                    "/"
                } else {
                    d
                }
            } else {
                "."
            };
            println!("{}", dir);
        }
        0
    }

    pub(in crate::executor) fn execute_basename(&mut self, cmd: &CommandNode) -> i32 {
        let mut args = Vec::new();
        let mut suffix: Option<String> = None;
        let mut i = 1;
        while i < cmd.words.len() {
            match cmd.words[i].as_str() {
                "-a" | "--multiple" => {
                    i += 1;
                }
                "-s" | "--suffix" => {
                    suffix = cmd.words.get(i + 1).map(|w| self.expand_word(w));
                    i += 2;
                }
                "-z" | "--zero" => {
                    i += 1;
                }
                "--" => {
                    i += 1;
                    break;
                }
                arg if arg.starts_with('-') && arg.len() > 1 => {
                    i += 1;
                }
                _ => {
                    args.push(self.expand_word(&cmd.words[i]));
                    i += 1;
                }
            }
        }
        while i < cmd.words.len() {
            args.push(self.expand_word(&cmd.words[i]));
            i += 1;
        }
        if args.is_empty() {
            eprintln!("{}basename: missing operand", self.diagnostic_prefix());
            return 1;
        }
        fn strip_name(name: &str, suf: &str) -> String {
            if suf.len() < name.len() && name.ends_with(suf) {
                name[..name.len() - suf.len()].to_string()
            } else {
                name.to_string()
            }
        }
        if suffix.is_none() && args.len() == 2 {
            let normalized = args[0].replace('\\', "/");
            let name = if let Some(pos) = normalized.rfind('/') {
                &normalized[pos + 1..]
            } else {
                &normalized
            };
            let name = if name.is_empty() { "/" } else { name };
            println!("{}", strip_name(name, &args[1]));
        } else {
            for arg in &args {
                let normalized = arg.replace('\\', "/");
                let name = if let Some(pos) = normalized.rfind('/') {
                    &normalized[pos + 1..]
                } else {
                    &normalized
                };
                let name = if name.is_empty() { "/" } else { name };
                if let Some(suf) = &suffix {
                    println!("{}", strip_name(name, suf));
                } else {
                    println!("{}", name);
                }
            }
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

        Ok(crate::builtins::cd::execute(
            &cmd.words[1..],
            &mut self.env_vars,
        )?)
    }
}
