use super::*;

impl Executor {
    pub(in crate::executor) fn execute_hash(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        // TODO(redir.c/builtins/hash.def): Redirections are command-level in
        // Bash. This covers `hash -t cat 2>/dev/null` from builtins9.sub.
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        // GNU hash.def never hashes builtins or functions (rubash#1215);
        // the pure builtin cannot see either set, so supply their union.
        let mut unhashable_names: std::collections::HashSet<String> =
            self.shell_state.functions.keys().cloned().collect();
        unhashable_names.extend(builtin_names().iter().map(|name| name.to_string()));
        let status = crate::builtins::hash::execute_with_io_and_functions(
            &cmd.words[1..],
            &mut self.shell_state.env_vars,
            &unhashable_names,
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_shopt(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::shopt::execute_with_io(
            &cmd.words[1..],
            &mut self.shell_state.env_vars,
            &mut stdout,
            &mut stderr,
        )?;
        self.sync_shopt_rebound_var();
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    /// GNU has one variable store; rubash mirrors direct assignments into
    /// the typed owner for expansion. `shopt -s/-u compatNN` rebinds
    /// BASH_COMPAT in the mirror only, so a typed cell from an earlier
    /// direct assignment would shadow it on expansion (verified:
    /// `BASH_COMPAT=99; shopt -s compat44; echo $BASH_COMPAT` printed 99
    /// while `declare -p` showed 44). Re-sync the rebound name after every
    /// shopt invocation (the unset_arrays dual-store precedent); a missing
    /// typed cell needs nothing (expansion falls back to the mirror).
    pub(in crate::executor) fn sync_shopt_rebound_var(&mut self) {
        if self.shell_state.variables.get("BASH_COMPAT").is_none() {
            return;
        }
        if let Some(value) = self.shell_state.env_vars.get("BASH_COMPAT").cloned() {
            if let Some(variable) = self.shell_state.variables.get_mut("BASH_COMPAT") {
                variable.value = crate::shell::ShellValue::Scalar(value);
            }
        } else {
            self.shell_state.variables.remove("BASH_COMPAT");
        }
    }

    pub(in crate::executor) fn execute_umask(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let umask_before = self.shell_state.env_vars.get("__RUBASH_UMASK").cloned();
        let status = crate::builtins::umask::execute_with_io(
            &cmd.words[1..],
            &mut self.shell_state.env_vars,
            &mut stdout,
            &mut stderr,
        )?;
        // rubash#2084: journal a session-set umask so the root
        // executor's Drop can restore the host's birth value when the
        // process still carries this session's setting (the builtin
        // moves the process-global mask; the session must not leak it
        // into the embedding host).
        if status == 0 {
            if let Some(after) = self.shell_state.env_vars.get("__RUBASH_UMASK") {
                if Some(after) != umask_before.as_ref() {
                    if let Ok(value) = u32::from_str_radix(after, 8) {
                        self.session_set_umask = Some(value);
                    }
                }
            }
        }
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }
}
