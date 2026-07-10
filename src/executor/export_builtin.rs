use super::*;

impl Executor {
    pub(in crate::executor) fn execute_export(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if export_args_request_functions(&cmd.words[1..]) {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let status =
                self.execute_export_functions(&cmd.words[1..], &mut stdout, &mut stderr)?;
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(status);
        }
        self.mark_posix_function_export_touches(&cmd.words[1..]);

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::setattr::export_with_io(
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
            return Ok(crate::builtins::setattr::export_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return Ok(crate::builtins::setattr::export_with_io(
                    cmd.words[1..].iter().map(String::as_str),
                    &mut self.env_vars,
                    &mut std::io::stdout().lock(),
                    &mut std::io::sink(),
                )?);
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::setattr::export_with_io(
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
            return Ok(crate::builtins::setattr::export_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        Ok(crate::builtins::setattr::export(
            &cmd.words[1..],
            &mut self.env_vars,
        )?)
    }

    pub(in crate::executor) fn mark_posix_function_export_touches(&mut self, args: &[String]) {
        if self.function_depth == 0 || !self.posix_mode_enabled() {
            return;
        }
        let mut names_started = false;
        for arg in args {
            if arg == "--" {
                names_started = true;
                continue;
            }
            if !names_started && arg.starts_with('-') && arg != "-" {
                continue;
            }
            names_started = true;
            let Some(name) = local_assignment_name(arg) else {
                continue;
            };
            mark_env_name(&mut self.env_vars, POSIX_FUNCTION_EXPORT_TOUCHED, name);
        }
    }

    pub(in crate::executor) fn execute_export_functions<W, E>(
        &mut self,
        args: &[String],
        stdout: &mut W,
        stderr: &mut E,
    ) -> io::Result<i32>
    where
        W: Write,
        E: Write,
    {
        let mut unset = false;
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
                    'n' => unset = true,
                    'p' => print = true,
                    other => {
                        writeln!(
                            stderr,
                            "{}export: -{other}: invalid option",
                            self.diagnostic_prefix()
                        )?;
                        writeln!(
                            stderr,
                            "export: usage: export [-fn] [name[=value] ...] or export -p"
                        )?;
                        return Ok(2);
                    }
                }
            }
            index += 1;
        }

        if print && index >= args.len() {
            let mut names = marked_env_names(&self.env_vars, EXPORTED_FUNCTIONS);
            names.sort();
            for name in names {
                if let Some(body) = self.functions.get(&name) {
                    self.write_function_definition(&name, body, true, stdout)?;
                }
            }
            return Ok(0);
        }

        let mut status = 0;
        for name in &args[index..] {
            if !self.functions.contains_key(name) {
                writeln!(
                    stderr,
                    "{}export: {name}: not a function",
                    self.diagnostic_prefix()
                )?;
                status = 1;
                continue;
            }
            if !unset && !is_exportable_function_name(name) {
                writeln!(
                    stderr,
                    "{}export: {name}: cannot export",
                    self.diagnostic_prefix()
                )?;
                status = 1;
                continue;
            }
            if unset {
                unmark_env_name(&mut self.env_vars, EXPORTED_FUNCTIONS, name);
            } else {
                mark_env_name(&mut self.env_vars, EXPORTED_FUNCTIONS, name);
            }
        }

        Ok(status)
    }
}
