use super::*;

impl Executor {
    pub(in crate::executor) fn execute_pwd(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = self.execute_pwd_with_io(&cmd.words[1..], &mut stdout, &mut stderr)?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        // GNU pwd.def returns sh_chkwrite(EXECUTION_SUCCESS): a failed
        // flush (closed fd) turns the builtin's status into failure.
        if self.take_builtin_write_failed() {
            return Ok(1);
        }
        Ok(status)
    }

    pub(in crate::executor) fn execute_pwd_with_io<W, E>(
        &mut self,
        args: &[String],
        stdout: &mut W,
        stderr: &mut E,
    ) -> io::Result<i32>
    where
        W: Write,
        E: Write,
    {
        // The $PWD echo fast path is the LOGICAL rendering; with the
        // `physical` shell option set (set -P) a bare `pwd` defaults to
        // physical mode and must not take it (rubash#1039).
        if (args.is_empty() || args.first().map(String::as_str) == Some("-L"))
            && !crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "physical")
        {
            if let Some(pwd) = self.shell_state.env_vars.get("PWD") {
                if pwd.starts_with('/') {
                    writeln!(stdout, "{pwd}")?;
                    return Ok(0);
                }
            }
        }

        crate::builtins::pwd::execute_with_env_and_io(
            args.iter().map(String::as_str),
            &self.shell_state.env_vars,
            &self.shell_state.cwd,
            stdout,
            stderr,
        )
    }

    pub(in crate::executor) fn execute_loop_control(
        &mut self,
        cmd: &CommandNode,
        kind: LoopControlKind,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        // GNU loop_level as seen by this executor: loops executing HERE
        // (loop_depth) plus the depth a fork boundary carried in
        // (inherited_loop_depth — command substitution bodies and simple
        // pipeline/async elements inherit it, rubash#1064/#1065).
        let own_loop_depth = self.shell_state.loop_depth;
        let total_loop_depth = own_loop_depth + self.shell_state.inherited_loop_depth;
        if total_loop_depth == 0 {
            // GNU break.def check_loop_level (BREAK_COMPLAINS): the
            // out-of-loop diagnostic is suppressed in posix mode; the status
            // stays zero either way (func5.sub posix testfunc `break`).
            if !self.posix_mode_enabled() {
                writeln!(
                    &mut stderr,
                    "{}{}: only meaningful in a `for', `while', or `until' loop",
                    self.diagnostic_prefix(),
                    kind.name()
                )?;
            }
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            // Bash emits the diagnostic but leaves the command status at zero
            // when an out-of-loop break/continue is followed by another
            // command in the same list.
            self.exit_code = 0;
            return Ok(());
        }

        match loop_control_level(&cmd.words[1..]) {
            Ok(level) => {
                let level = level.min(total_loop_depth);
                // GNU break.def/continue.def return EXECUTION_SUCCESS for an
                // in-range count — the builtin has no user-visible status of
                // its own. Establish that status before unwinding (rubash#702):
                // the loop's CONDITION status is still in self.exit_code here
                // (it ran with errexit suppressed and nothing replaced it), and
                // the dispatcher's post-command errexit / ERR-trap gate reads
                // self.exit_code — so an `until` whose condition failed would
                // otherwise attribute that failure to a body whose first
                // command is `break`/`continue` (or a group starting with one).
                // `until false; do break; done` must leave $? = 0, and
                // `until (exit 7); do break; done` must not leak the 7.
                self.exit_code = 0;
                if own_loop_depth == 0 {
                    // Fork-boundary child (comsub body / simple pipeline or
                    // async element): GNU's `breaking`/`continuing` unwind
                    // only the child's own parse_and_execute list — the
                    // clone has no live loop frames to receive a Break, so
                    // end this executor's command list with the builtin's
                    // success status. The boundary caller maps ExitCode to
                    // the element/substitution status and the parent shell
                    // (and its loop) carries on unaffected (rubash#1064,
                    // rubash#1065).
                    return Err(ExecuteError::ExitCode(0));
                }
                match kind {
                    LoopControlKind::Break => Err(ExecuteError::Break(level)),
                    LoopControlKind::Continue => Err(ExecuteError::Continue(level)),
                }
            }
            Err(LoopControlError::TooManyArguments) => {
                writeln!(
                    &mut stderr,
                    "{}{}: too many arguments",
                    self.diagnostic_prefix(),
                    kind.name()
                )?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                // GNU builtins/break.def:73 (continue.def:118): a VALID
                // numeric count goes to get_numeric_arg, whose tail calls
                // no_args (list->next, 0) (builtins/common.c:511) for the
                // extra operands. no_args (common.c:139-155) with fatal=0
                // is NEVER the EXITPROG channel: builtin_error +
                // top_level_cleanup + set_exit_status (EX_BADUSAGE) +
                // jump_to_top_level (DISCARD) in every mode — the current
                // command list (the enclosing loop, and the whole -c/eval
                // string via the cleanup) is abandoned with status 2 and
                // the reader continues at the next list. Only the
                // NON-numeric branch (common.c:497-510, fatal=1
                // non-interactive) takes EXITPROG — see the NotNumeric
                // arm below. Ride the same evalerror vehicle as
                // exit/return too-many (rubash#705/#1213): status 2 lands
                // at the reader, whose -c catch maps the class to
                // ExitCode(1) (shell.c:1477) (rubash#1519).
                self.exit_code = 2;
                self.raise_evalerror_abort();
                self.evalerror_exit_toomany_pending.set(true);
                Ok(())
            }
            Err(LoopControlError::OutOfRange(value)) => {
                writeln!(
                    &mut stderr,
                    "{}{}: {value}: loop count out of range",
                    self.diagnostic_prefix(),
                    kind.name()
                )?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                // GNU break.def:80-91 / continue.def:120-131: even when the
                // count is out of range, `breaking`/`continuing` is set to
                // loop_level — the loop STILL unwinds — and the builtin
                // returns EXECUTION_FAILURE, so the loop's exit status is 1
                // (rubash#395: the Break unwind resets exit_code at every
                // loop boundary; the failure status rides the Cell to the
                // innermost consumer arm). For `continue 0`,
                // `continuing = loop_level` with a count of 0 effectively
                // breaks the loop (the rest of the body is skipped and the
                // loop does not advance to the next iteration), so both
                // cases return Break.
                self.break_failure_status.set(Some(1));
                self.exit_code = 1;
                if own_loop_depth == 0 {
                    // Fork-boundary child: `breaking = loop_level` ends the
                    // child's list with the builtin's failure status (1).
                    return Err(ExecuteError::ExitCode(1));
                }
                let level = self.shell_state.loop_depth;
                Err(ExecuteError::Break(level))
            }
            Err(LoopControlError::NotNumeric(value)) => {
                writeln!(
                    &mut stderr,
                    "{}{}: {value}: numeric argument required",
                    self.diagnostic_prefix(),
                    kind.name()
                )?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                // GNU break.def:76 get_numeric_arg(list, interactive ? 2 : 1)
                // (builtins/common.c:488-510): a non-numeric count reports
                // sh_neednumarg and NEVER RETURNS — non-interactive fatal=1
                // is set_exit_status(EX_BADUSAGE=2) + jump_to_top_level
                // (EXITPROG): the whole shell exits 2 (verified WSL GNU
                // 5.3.0: a script dies at the first `break 2x`, rc 2);
                // interactive fatal=2 is DISCARD — the current command list
                // is abandoned and the reader continues. The status must
                // also be PARKED: the POSIX completion channel reads
                // self.exit_code rather than the error value, so without
                // the park a POSIX-mode script (or -c string) unwound
                // correctly but exited with the stale code (rubash#2127).
                self.exit_code = 2;
                if self.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
                    return Err(ExecuteError::ExpansionFailure(2));
                }
                Err(ExecuteError::ExitCode(2))
            }
        }
    }

    /// rubash#395: the status a failed loop-control builtin (break 0 /
    /// continue 0 / break -1) parked for the innermost Break consumer; a
    /// normal `break n` unwinds with None (status 0).
    pub(in crate::executor) fn take_break_failure_status(&self) -> Option<i32> {
        self.break_failure_status.take()
    }

    pub(in crate::executor) fn execute_return(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let args = &cmd.words[1..];
        let mut stderr = Vec::new();
        // GNU builtins/return.def: CHECK_HELPOPT (builtins/common.h:29)
        // inspects the RAW first word before get_exitstat's `--` skip:
        // `return --help` prints the long help to stdout and the builtin
        // RETURNS EX_USAGE (258 > EX_SHERRBASE) WITHOUT unwinding — a
        // function/source body continues after it in default mode, and in
        // POSIX non-interactive mode the special-builtin channel converts
        // the failure at this frame (execute_cmd.c:4886-4888,
        // :1004-1017). RC audit phase-1 finding ④ / errors10: pre-fix
        // this fell into the status-word path below and misreported
        // `--help` as a numeric-argument error (and unwound).
        if args.first().map(String::as_str) == Some("--help") {
            let mut stdout = Vec::new();
            crate::builtins::help::print_builtin_help("return", &mut stdout)?;
            self.write_buffered_builtin_output(cmd, &stdout, &[])?;
            self.exit_code = 2;
            self.special_builtin_failed.set(true);
            return Ok(());
        }
        // GNU builtins/common.c get_exitstat: ISOPTION(word, '-') skips a
        // leading `--` before the status word — `return -- N` returns N
        // (rubash#1379). Mirrors src/builtins/exit.rs.
        let args: &[String] = match args.first().map(String::as_str) {
            Some("--") => &args[1..],
            _ => args,
        };
        // GNU return.def: more than one operand is `return: too many
        // arguments` — the no_args class (builtins/common.c), exactly
        // like `exit 1 2`: set_exit_status(EX_BADUSAGE) then
        // jump_to_top_level(DISCARD). Oracle-verified against GNU 5.3:
        // the function/source unwinds AND the rest of the current
        // command list is abandoned (the caller's same-line commands
        // never run; an `if` whose condition dies this way takes the
        // whole line with it), while the NEXT line runs with $?=2. At
        // top level it reports this message — not the can-only-`return'
        // one. Ride the same evalerror vehicle as exit-too-many
        // (rubash#705 machinery, rubash#1213).
        if args.len() > 1 {
            writeln!(
                &mut stderr,
                "{}return: too many arguments",
                self.diagnostic_prefix()
            )?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 2;
            self.raise_evalerror_abort();
            self.evalerror_exit_toomany_pending.set(true);
            return Ok(());
        }
        let mut numeric_failed = false;
        let status = if let Some(value) = args.first() {
            // GNU get_exitstat parses via general.c:248 valid_number
            // (intmax_t): whitespace-padded numerics are legal
            // (rubash#1339); out-of-range rejected (rubash#1403).
            match crate::builtins::exit::gnu_valid_number(value) {
                Some(value) => crate::builtins::exit::normalize_status(value),
                None => {
                    numeric_failed = true;
                    writeln!(
                        &mut stderr,
                        "{}return: {value}: numeric argument required",
                        self.diagnostic_prefix()
                    )?;
                    // GNU builtins/return.def:57-69: get_exitstat's
                    // failure is EX_USAGE (258 > EX_SHERRBASE), so the
                    // builtin's completion marks special_builtin_failed
                    // (execute_cmd.c:4886-4888). return_builtin's own
                    // posix guard hands the raw code back for the
                    // conversion when the command is not
                    // command-prefixed; rubash's gate in
                    // execute_materialized_command performs the same
                    // conversion when this Return unwind lands at the
                    // enclosing call/source frame, and the
                    // `command'-builtin wrapper clears the mark for
                    // `command return ...' (execute_cmd.c:4657-4666).
                    // RC audit phase-1 finding ④ / errors10.
                    self.special_builtin_failed.set(true);
                    2
                }
            }
        } else {
            // A bare return inside a running signal trap action gets the
            // pre-trap $? only when it terminates the trap action itself
            // (posix interp 1602); a return in a function called by the
            // action uses the current $? (trap9.sub: handler's return sees
            // setexit's 111, not the pre-trap status).
            let action_depth = self
                .shell_state
                .env_vars
                .get("__RUBASH_SIGNAL_TRAP_DEPTH")
                .and_then(|value| value.parse::<usize>().ok());
            let in_trap_action = action_depth == Some(self.shell_state.function_depth);
            if in_trap_action {
                self.shell_state
                    .env_vars
                    .get("__RUBASH_SIGNAL_TRAP_STATUS")
                    .and_then(|value| value.parse::<i32>().ok())
                    .unwrap_or(self.exit_code)
            } else {
                self.exit_code
            }
        };

        let in_function = self.shell_state.function_depth > 0;
        let in_source = self
            .shell_state
            .env_vars
            .get("__RUBASH_IN_SOURCE")
            .map(String::as_str)
            == Some("1");
        if numeric_failed {
            // GNU builtins/return.def:62-65: on an invalid numeric
            // argument in a non-interactive POSIX shell (and not under
            // the `command' builtin), return_builtin RETURNS EX_USAGE
            // instead of longjmping the return_catch — "the
            // return_catch targets aren't set up to deal with
            // EXITSHELL, so we just jump_to_top_level directly". That
            // raw return precedes the return_catch check, so it fires
            // in EVERY context: inside a function/source the completion
            // machinery owns the outcome from there (below), and at the
            // top level it suppresses the can-only-`return' report
            // entirely — GNU prints only the numeric diagnostic there
            // (rubash#2127; in default mode both lines print, pinned by
            // the fall-through to the bottom arm).
            // execute_simple_command marks special_builtin_failed
            // (execute_cmd.c:4888) and the return command's own frame
            // exits the shell when the context is untested
            // (execute_cmd.c:1004-1017); in a tested context
            // (ignore_return / invert) the builtin simply fails with
            // status 2 and the body RUNS ON — no unwind at all (oracle:
            // `f(){ return abcde; echo INFUNC; }; f || echo OR' prints
            // INFUNC under -o posix). Mirror both arms here: the
            // dispatch gate cannot adjudicate this builtin, because its
            // completion is an unwind (the gate passes Return through)
            // and a function call's fast path
            // (execute_function_command_invocation) has no gate.
            // RC audit phase-1 finding ④ / errors10.
            if self.posix_mode_enabled()
                && self.command_builtin_depth == 0
                && self
                    .shell_state
                    .env_vars
                    .get("__RUBASH_INTERACTIVE")
                    .map(String::as_str)
                    != Some("1")
            {
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                self.exit_code = 2;
                if self.errexit_is_active() {
                    return Err(ExecuteError::ExitCode(2));
                }
                return Ok(());
            }
        }
        if in_function || in_source {
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            return Err(ExecuteError::Return(status));
        }

        writeln!(
            &mut stderr,
            "{}return: can only `return' from a function or sourced script",
            self.diagnostic_prefix()
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        self.exit_code = 2;
        // GNU return.def:74 returns EX_USAGE (258 > EX_SHERRBASE=256), so
        // execute_cmd.c:4888 sets special_builtin_failed for `return` (a
        // POSIX special builtin). In POSIX non-interactive mode this causes
        // the shell to exit (execute_cmd.c:1004-1017).
        self.special_builtin_failed.set(true);
        Ok(())
    }
}
