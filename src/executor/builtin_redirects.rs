use super::*;

impl Executor {
    pub(in crate::executor) fn write_builtin_not_found(
        &mut self,
        cmd: &CommandNode,
        name: &str,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        writeln!(
            &mut stderr,
            "{}builtin: {name}: not a shell builtin",
            self.diagnostic_prefix()
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)
    }

    pub(in crate::executor) fn apply_no_output_builtin_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            self.create_redirect_output(&target, redirect.clobber)?;
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if !is_null_device(&target) {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
        }

        Ok(())
    }
}
