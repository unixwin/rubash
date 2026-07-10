use super::*;

impl Executor {
    pub(in crate::executor) fn execute_eval(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        let args = cmd.words[1..]
            .iter()
            .map(|word| unescape_remaining_shell_escapes(word))
            .collect::<Vec<_>>();
        match crate::builtins::eval::execute_with_io(args.iter().map(String::as_str), &mut stderr)?
        {
            crate::builtins::eval::EvalAction::Complete(status) => {
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                self.exit_code = status;
                Ok(())
            }
            crate::builtins::eval::EvalAction::Execute(source) => {
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                let source = eval_source_for_reparse(&source);
                let tokens = crate::lexer::tokenize(&source);
                let mut ast = crate::parser::parse(&tokens);
                self.apply_command_output_redirects(cmd, &mut ast)?;
                self.execute_ast(&ast)
            }
        }
    }

    pub fn run_exit_trap(&mut self) -> Result<i32, ExecuteError> {
        self.run_exit_trap_for_status(self.exit_code)
    }

    pub fn run_exit_trap_with_status(&mut self, exit_status: i32) -> Result<i32, ExecuteError> {
        self.run_exit_trap_for_status(exit_status)
    }

    pub(in crate::executor) fn run_exit_trap_for_status(
        &mut self,
        exit_status: i32,
    ) -> Result<i32, ExecuteError> {
        let Some(action) = crate::builtins::trap::take_exit_trap(&mut self.env_vars) else {
            return Ok(exit_status);
        };
        if action.is_empty() {
            return Ok(exit_status);
        }

        self.exit_code = exit_status;
        let tokens = crate::lexer::tokenize(&action);
        let ast = crate::parser::parse(&tokens);
        match self.execute_ast(&ast) {
            Ok(()) => {
                self.exit_code = exit_status;
                Ok(exit_status)
            }
            Err(ExecuteError::ExitCode(code)) => {
                self.exit_code = code;
                Ok(code)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn apply_command_output_redirects(
        &mut self,
        cmd: &CommandNode,
        ast: &mut Ast,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let append_redirect = Redirect {
                fd: redirect.fd,
                target,
                append: true,
                clobber: false,
            };
            apply_stdout_append_redirect(&mut ast.commands, &append_redirect);
        } else if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            let append_redirect = Redirect {
                fd: redirect.fd,
                target,
                append: true,
                clobber: false,
            };
            apply_stdout_append_redirect(&mut ast.commands, &append_redirect);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if !is_closed_redirect_target(&target)
                && redirect_target_fd(&target).is_none()
                && !is_null_device(&target)
            {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let append_redirect = Redirect {
                fd: redirect.fd,
                target,
                append: true,
                clobber: false,
            };
            apply_stderr_append_redirect(&mut ast.commands, &append_redirect);
        } else if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let append_redirect = Redirect {
                fd: redirect.fd,
                target,
                append: true,
                clobber: false,
            };
            apply_stderr_append_redirect(&mut ast.commands, &append_redirect);
        }

        Ok(())
    }

    pub(in crate::executor) fn execute_exec(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            return Ok(crate::builtins::exec::execute_with_io(
                &cmd.words[1..],
                &self.env_vars,
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
            return Ok(crate::builtins::exec::execute_with_io(
                &cmd.words[1..],
                &self.env_vars,
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            return Ok(crate::builtins::exec::execute_with_io(
                &cmd.words[1..],
                &self.env_vars,
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
            return Ok(crate::builtins::exec::execute_with_io(
                &cmd.words[1..],
                &self.env_vars,
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        self.apply_no_output_builtin_redirects(cmd)?;
        Ok(crate::builtins::exec::execute(
            &cmd.words[1..],
            &self.env_vars,
        )?)
    }

    pub(in crate::executor) fn execute_exec_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let status = self.execute_exec(cmd)?;
        self.exit_code = status;
        if crate::builtins::exec::replaces_shell(&cmd.words[1..]) {
            return Err(ExecuteError::ExitCode(status));
        }
        Ok(())
    }
}
