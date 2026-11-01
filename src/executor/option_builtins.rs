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
        let mut o_writes: Vec<(String, bool)> = Vec::new();
        let mut login_shell_write = false;
        let status = crate::builtins::shopt::execute_with_io_and_o_writes(
            &cmd.words[1..],
            &mut self.shell_state.env_vars,
            &mut stdout,
            &mut stderr,
            &mut o_writes,
            &mut login_shell_write,
        )?;
        // GNU builtins/shopt.def:594-606 set_shopt_o_options funnels
        // every `-o` write through builtins/set.def:476-491
        // set_minus_o_option, so a `shopt -s -o vi` lands in the same
        // set_edit_mode setter (set.def:424-447) as `set -o vi`: each
        // FLAG_ON write of vi/emacs runs rl_variable_bind
        // ("editing-mode", name) — a redundant re-set rebinds too, and
        // a FLAG_OFF write never touches the variable (oracle-verified
        // round trips). Replay the reported writes in operand order,
        // exactly as the `set -o` fast path applies its own writes
        // (rubash#2435; the #2386 bridge covered only the set paths).
        for (name, enabled) in &o_writes {
            if *enabled && (name == "vi" || name == "emacs") {
                self.bind_registry()
                    .borrow_mut()
                    .set_readline_variable("editing-mode", name);
            }
        }
        // rubash#2457: GNU's login_shell shopt entry carries a custom
        // setter (builtins/shopt.def:244; set_login_shell at
        // shopt.def:762-766) that IGNORES the requested value and
        // re-copies the CURRENT login_shell global into the shadow
        // the queries read. So `shopt -u login_shell` at the top of
        // a login shell leaves the report on, `shopt -s login_shell`
        // in a non-login shell leaves it off, and inside a `( )`
        // region either spelling snaps it off (execute_in_subshell
        // cleared the global, execute_cmd.c:1636-1637). The pure
        // builtin applied the requested value like any other option;
        // re-derive the bit from this executor's effective login
        // state — the same expression the logout gate reads
        // (printf_path_builtins.rs). Plain queries never raise the
        // attempt flag, so the query face keeps reading the stored
        // shadow verbatim — which is also GNU's behavior, since a
        // fork copies the shadow without re-syncing it
        // (reset_shopt_options, shopt.def:440, runs at shell init,
        // not at subshell entry).
        if login_shell_write {
            let effective =
                self.invocation_login() && !self.shell_state.subshell_login_cleared.get();
            self.set_shopt_option("login_shell", effective);
        }
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
