use super::*;

impl Executor {
    /// GNU builtins/test.c `test -v name[sub]` (and `[ -v ... ]`): the `-v`
    /// operand is argv data that already went through word expansion, so its
    /// subscript resolves under the ExpandedOnce rules — verbatim with
    /// array_expand_once (VA_NOEXPAND/ASS_NOEXPAND via SET_VFLAGS,
    /// builtins/common.h:277-289), a deferred expand_subscript_string pass
    /// without it. A failing indexed subscript is an expr.c evalerror that
    /// discards the rest of the command list (`test -v 'a[$x]'; echo x`
    /// never prints `x`); the diagnostic + abort flag are raised inside
    /// rewrite_operand_array_subscript and the builtin returns status 1.
    pub(in crate::executor) fn execute_test_words(
        &mut self,
        args: &[String],
        bracket: bool,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        // W_ARRAYREF (in-band ARRAYREF_FLAG) is a no-op for test/[ — GNU
        // marks arrayref-shaped operands (execute_cmd.c:4366) but test.def
        // never consults it; strip so operand text compares clean.
        let mut args: Vec<String> = args
            .iter()
            .map(|arg| {
                crate::builtins::arrayref::take_arrayref_flag(arg)
                    .1
                    .to_string()
            })
            .collect();
        let mut index = 0;
        while index + 1 < args.len() {
            if args[index] == "-v" {
                match self.rewrite_operand_array_subscript(&args[index + 1]) {
                    Ok(rewritten) => args[index + 1] = rewritten,
                    Err(()) => return Ok(1),
                }
                index += 1;
            }
            index += 1;
        }
        self.sync_fd_terminal_marks(Some(cmd));
        Ok(crate::builtins::test::execute(
            &args,
            bracket,
            &self.shell_state.env_vars,
        )?)
    }

    pub(in crate::executor) fn execute_late_builtin_command(
        &mut self,
        cmd: &CommandNode,
        word: &str,
    ) -> Result<(), ExecuteError> {
        match word {
            "let" => {
                self.exit_code = self.execute_let(&cmd.words[1..]);
                Ok(())
            }
            "umask" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "umask") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_umask(cmd)?;
                    Ok(())
                }
            }
            "ulimit" => {
                self.exit_code = self.execute_ulimit(cmd)?;
                Ok(())
            }
            "unset" => {
                self.exit_code = self.execute_unset(cmd)?;
                Ok(())
            }
            "read" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "read") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_read(cmd);
                    Ok(())
                }
            }
            "mapfile" | "readarray" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, word) {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_mapfile(cmd);
                    Ok(())
                }
            }
            "recho" => self.execute_recho_command(cmd),
            "zecho" => self.execute_zecho_command(cmd),
            "shift" => self.execute_shift_command(cmd),
            "times" => {
                self.exit_code = self.execute_times(cmd)?;
                Ok(())
            }
            "caller" => {
                self.exit_code = self.execute_caller(cmd)?;
                Ok(())
            }
            "jobs" => {
                self.exit_code = self.execute_jobs(cmd)?;
                Ok(())
            }
            "disown" => {
                self.exit_code = self.execute_disown(cmd)?;
                Ok(())
            }
            "wait" => {
                self.exit_code = self.execute_wait(cmd)?;
                Ok(())
            }
            "fg" => {
                self.exit_code =
                    self.execute_fg_bg(cmd, crate::builtins::fg_bg::JobControlBuiltin::Fg)?;
                Ok(())
            }
            "bg" => {
                self.exit_code =
                    self.execute_fg_bg(cmd, crate::builtins::fg_bg::JobControlBuiltin::Bg)?;
                Ok(())
            }
            "suspend" => {
                self.exit_code = self.execute_suspend(cmd)?;
                Ok(())
            }
            "history" => {
                self.exit_code = self.execute_history(cmd)?;
                Ok(())
            }
            "bind" => {
                self.exit_code = self.execute_bind(cmd)?;
                Ok(())
            }
            "fc" => {
                self.exit_code = self.execute_fc(cmd)?;
                Ok(())
            }
            "complete" => {
                self.exit_code = self.execute_completion_builtin(
                    cmd,
                    crate::builtins::complete::CompletionBuiltin::Complete,
                )?;
                Ok(())
            }
            "compgen" => {
                self.exit_code = self.execute_completion_builtin(
                    cmd,
                    crate::builtins::complete::CompletionBuiltin::Compgen,
                )?;
                Ok(())
            }
            "compopt" => {
                self.exit_code = self.execute_completion_builtin(
                    cmd,
                    crate::builtins::complete::CompletionBuiltin::Compopt,
                )?;
                Ok(())
            }
            "time" => {
                self.execute_time_command_node(cmd)?;
                Ok(())
            }
            "trap" => {
                self.exit_code = self.execute_trap(cmd)?;
                Ok(())
            }
            "type" => {
                // niubash #108: route every invocation through the buffered
                // path. Command substitution (`$(type -t ls)`) swaps in
                // `stdout_capture` above this layer and is NOT visible to
                // `command_has_output_redirects`, so gating on explicit
                // redirects leaked the description to the process stdout.
                // `execute_type_with_io` also handles the disabled-builtin
                // state (`enable -n type`) internally.
                self.exit_code = self.execute_type_redirected(cmd)?;
                Ok(())
            }
            "test" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "test") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_test_words(&cmd.words[1..], false, cmd)?;
                    Ok(())
                }
            }
            "[" => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "[") {
                    self.execute_external(cmd)
                } else {
                    self.exit_code = self.execute_test_words(&cmd.words[1..], true, cmd)?;
                    Ok(())
                }
            }
            "[[" => {
                // GNU has no `[[` builtin: the word only works when the parser
                // built a conditional command node. `v=9 [[ -n a ]]` or an
                // expanded `[[` therefore hits `[[: command not found` (127)
                // like any other missing command.
                match cmd.conditional_command.as_ref() {
                    Some(command) => {
                        self.apply_no_output_builtin_redirects(cmd)?;
                        self.exit_code = self.execute_conditional_command(command);
                        Ok(())
                    }
                    None => self.execute_external(cmd),
                }
            }
            "((" => {
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = self.execute_arithmetic_command(cmd);
                // GNU expr.c: an unbound variable under `set -u` inside `((
                // ))` raises FORCE_EOF and terminates the noninteractive
                // shell (probe a2: `set -u; ((b)); echo after` never prints
                // "after"). The status alone would otherwise keep the script
                // running.
                if self.shell_state.arithmetic_nounset_error.get() {
                    self.shell_state.arithmetic_nounset_error.set(false);
                    self.shell_state.arithmetic_expansion_error.set(false);
                    self.exit_code = 127;
                    return Err(ExecuteError::ExitCode(127));
                }
                Ok(())
            }
            "dirname" => {
                if let Some(status) = self.try_execute_dirname_fast_path(cmd) {
                    self.exit_code = status;
                    Ok(())
                } else {
                    self.execute_external(cmd)
                }
            }
            "basename" => {
                if let Some(status) = self.try_execute_basename_fast_path(cmd) {
                    self.exit_code = status;
                    Ok(())
                } else {
                    self.execute_external(cmd)
                }
            }
            _ if self.shell_state.functions.contains_key(word) => {
                self.execute_function(word, &cmd.words[1..], cmd)
            }
            _ => self.execute_external(cmd),
        }
    }
}
