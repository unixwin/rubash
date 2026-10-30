use super::command_registry::{cmd_primary_dispatch_slot, CmdPrimarySlot};
use super::*;

impl Executor {
    pub(in crate::executor) fn execute_command_without_aliases(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // TODO(builtins/command.def/execute_cmd.c): Bash `command` skips shell
        // functions and aliases while still resolving builtins and PATH. This
        // narrow path is enough for alias.tests cases like `command true`.
        let Some(word) = cmd.words.first() else {
            self.exit_code = 0;
            return Ok(());
        };

        // rubash#2126: the `command` re-dispatch is a real dispatch of
        // the target name (execute_cmd.c:4813 assigns this_shell_builtin
        // for the re-dispatched command too — the executing_command_
        // builtin bit gates only the evalstring CHECK, not the set), so
        // `command eval 'x=1'` re-arms the tracker even though a syntax
        // error inside THAT eval is contained by the depth gate. A
        // disabled target falls to execute_external below — a disk
        // dispatch, which the note's is_disabled term already maps to
        // disarm.
        self.note_simple_command_dispatch(word);

        if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, word) {
            return self.execute_external(cmd);
        }

        // GNU Bash has no `sudo` builtin (issue #759 item 4): `command
        // sudo` resolves on PATH unless an embedding host registered an
        // elevation handler. Not a registry claim (rubash#2036 S3): the
        // cfg(windows) handler pre-check stays ahead of the table.
        #[cfg(windows)]
        if word == "sudo" && self.elevation_handler.is_some() {
            self.exit_code = self.execute_sudo(cmd)?;
            return Ok(());
        }

        // rubash#2036 S3: the name set is the registry's CmdPrimary
        // projection (command_registry::CMD_PRIMARY_DISPATCH); the
        // slot match carries the historical per-name bodies verbatim,
        // and the fall-through is the historical late table.
        match cmd_primary_dispatch_slot(word) {
            Some(CmdPrimarySlot::Colon) => {
                // GNU command.def -> execute_builtin: the builtin's
                // redirections are applied before it runs (redir.c
                // do_redirections). `command : <&8` with a closed fd 8
                // fails "8: Bad file descriptor" like every other builtin
                // (modernish BUG_SCLOSEDFD probe).
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = crate::builtins::colon::colon();
                Ok(())
            }
            Some(CmdPrimarySlot::True) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "true") {
                    return self.execute_external(cmd);
                }
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = crate::builtins::colon::true_builtin();
                Ok(())
            }
            Some(CmdPrimarySlot::False) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "false") {
                    return self.execute_external(cmd);
                }
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = crate::builtins::colon::false_builtin();
                Ok(())
            }
            Some(CmdPrimarySlot::Echo) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "echo") {
                    return self.execute_external(cmd);
                }
                self.execute_echo(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Cd) => {
                self.exit_code = self.execute_cd(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Pwd) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "pwd") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_pwd(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Exec) => self.execute_exec_command(cmd),
            // builtins/command.def execute_builtin -> execute_cmd.c: `command`
            // suppresses only functions and aliases; `exit` stays a builtin
            // (modernish calls `command exit "$status"` in _Msh_exit).
            Some(CmdPrimarySlot::Exit) => self.execute_exit_command_word(cmd)?,
            Some(CmdPrimarySlot::Logout) => {
                self.exit_code = self.execute_logout(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Eval) => self.execute_eval(cmd),
            Some(CmdPrimarySlot::Set) => self.execute_set_command(cmd),
            Some(CmdPrimarySlot::Getopts) => {
                self.exit_code = self.execute_getopts_command(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Shopt) => {
                self.exit_code = self.execute_shopt(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Enable) => {
                self.exit_code = self.execute_enable(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Source) => self.execute_source_from_command_builtin(cmd),
            Some(CmdPrimarySlot::Return) => self.execute_return(cmd),
            Some(CmdPrimarySlot::Break) => self.execute_loop_control(cmd, LoopControlKind::Break),
            Some(CmdPrimarySlot::Continue) => {
                self.execute_loop_control(cmd, LoopControlKind::Continue)
            }
            Some(CmdPrimarySlot::Command) => self.execute_command_builtin_without_aliases(cmd),
            Some(CmdPrimarySlot::Builtin) => self.execute_builtin_direct_command(cmd),
            Some(CmdPrimarySlot::Printf) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "printf") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_printf(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Hash) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "hash") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_hash(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Help) => {
                self.exit_code = self.execute_help(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Alias) => {
                self.exit_code = self.execute_alias(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Unalias) => {
                self.exit_code = self.execute_unalias(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Export) => {
                self.exit_code = self.execute_export(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Readonly) => {
                self.exit_code = self.execute_readonly(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Declare) => self.execute_declare_command(cmd),
            Some(CmdPrimarySlot::Local) => {
                self.exit_code = self.execute_local(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Unset) => {
                self.exit_code = self.execute_unset(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Pushd) => {
                self.exit_code =
                    self.execute_stack_builtin(cmd, crate::builtins::pushd::StackBuiltin::Pushd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Popd) => {
                self.exit_code =
                    self.execute_stack_builtin(cmd, crate::builtins::pushd::StackBuiltin::Popd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Dirs) => {
                self.exit_code =
                    self.execute_stack_builtin(cmd, crate::builtins::pushd::StackBuiltin::Dirs)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Kill) => {
                self.exit_code = self.execute_kill(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Let) => {
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = self.execute_let(&cmd.words[1..]);
                Ok(())
            }
            // No "time" arm (rubash#1369): `time` is a reserved word,
            // not a shell builtin (GNU builtins/*.def has `times`, not
            // `time`), so the `command` re-dispatch must fall through
            // to the ordinary PATH lookup — `command time true` is
            // `time: command not found`, rc=127.
            Some(CmdPrimarySlot::Umask) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "umask") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_umask(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Ulimit) => {
                self.exit_code = self.execute_ulimit(cmd)?;
                Ok(())
            }
            Some(CmdPrimarySlot::Read) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "read") {
                    return self.execute_external(cmd);
                }
                self.exit_code = self.execute_read(cmd);
                Ok(())
            }
            Some(CmdPrimarySlot::Mapfile) => {
                if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, &cmd.words[0]) {
                    return self.execute_external(cmd);
                }
                // Redirect preflight (GNU execute_builtin applies the
                // redirections before mapfile runs; rubash#324: `< missing`
                // reports and returns 1 with the array untouched).
                self.apply_no_output_builtin_redirects(cmd)?;
                self.exit_code = self.execute_mapfile(cmd);
                Ok(())
            }
            Some(CmdPrimarySlot::Shift) => self.execute_shift_command(cmd),
            _ => self.execute_command_without_aliases_late_builtin(cmd, word),
        }
    }

    fn execute_command_builtin_without_aliases(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // rubash#289d: describe output always takes the buffered
        // single-truth path — the former raw `println!` fallback leaked
        // under comsub capture (see command_dispatch_primary.rs).
        if self.execute_command_describe_redirected(cmd)? {
            return Ok(());
        }

        // GNU execute_cmd.c: the builtin's own diagnostics and describe
        // output go to the shell's CURRENTLY BOUND fd 1/2 (redir.c), so an
        // `exec 2>/dev/null` contains them (rubash#218/#222-era leak: the
        // raw `builtins::command::execute` wrote straight to process
        // stdio, bypassing every enclosing redirect).
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let action = crate::builtins::command::execute_with_io(
            cmd.words[1..].iter().map(String::as_str),
            &self.diagnostic_prefix(),
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        match action {
            crate::builtins::command::CommandAction::Complete(status) => {
                self.exit_code = status;
                Ok(())
            }
            crate::builtins::command::CommandAction::Execute {
                words,
                use_standard_path,
            } => {
                let mut command = cmd.clone();
                command.words = words;
                self.execute_command_without_aliases_with_path(&command, use_standard_path)
            }
        }
    }

    pub(in crate::executor) fn execute_command_without_aliases_with_path(
        &mut self,
        cmd: &CommandNode,
        use_standard_path: bool,
    ) -> Result<(), ExecuteError> {
        // GNU execute_cmd.c:4732-4736: the re-dispatched argument of the
        // `command' builtin runs with executing_command_builtin set (unwind-
        // protected, restored on the way out). The eval/source parse-error
        // containment reads it (evalstring.c:590): `command eval '( '' must
        // not take the posix ERREXIT exit that bare `eval '( '' takes.
        self.command_builtin_depth += 1;
        let result = self.execute_command_without_aliases_with_path_inner(cmd, use_standard_path);
        self.command_builtin_depth -= 1;
        // GNU execute_cmd.c:4657-4666: the special-builtin lookup — and with
        // it builtin_is_special and special_builtin_failed — never runs for
        // this re-dispatch (check_command_builtin set CMD_NO_FUNCTIONS at
        // :4723), so whatever the inner builtin set must not leak to the
        // OUTER command's posix fatal check (:1004-1017): `command return 16'
        // under `set -o posix' reports and continues (errors8.sub ok 5).
        self.special_builtin_failed.set(false);
        result
    }

    fn execute_command_without_aliases_with_path_inner(
        &mut self,
        cmd: &CommandNode,
        use_standard_path: bool,
    ) -> Result<(), ExecuteError> {
        if !use_standard_path {
            return self.execute_command_without_aliases(cmd);
        }

        // GNU builtins/command.def: -p (CDESC_STDPATH) changes only the
        // PATH used to look up THIS command (findcmd.c search_for_
        // command with the standard path); the $PATH variable itself is
        // untouched, so the executed command's environment and any
        // nested builtin (`command -p command -v ls`) keep the user's
        // PATH. The old implementation swapped env_vars["PATH"] for the
        // whole re-dispatch, exporting the default PATH into children
        // (rubash#1047). Scope the standard path to the execution
        // lookup via standard_path_scope instead.
        let standard = standard_path(&self.shell_state.env_vars);
        let saved_scope = self.standard_path_scope.replace(standard);
        let result = self.execute_command_without_aliases(cmd);
        self.standard_path_scope = saved_scope;
        result
    }
}
