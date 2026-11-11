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
        // GNU builtins/cd.def pwd_builtin prints
        // the_current_working_directory — the shell's own logical cwd
        // (builtins/common.c:589) — never the raw $PWD variable: a
        // bare assignment to $PWD desynchronizes the variable and the
        // logical value stands (rubash#2143, rubash#2797). The session
        // carries that value in ShellState::logical_cwd, maintained by
        // the cd family and the startup adoption; the builtin receives
        // it here and adjudicates every mode itself (including the
        // Windows bridge state, whose cd branch maintains the same
        // field). The old $PWD-verbatim echo fast path is gone: it
        // printed the variable's non-canonical spelling, which is
        // exactly the #2797 divergence.
        let outcome = crate::builtins::pwd::execute_with_env_and_io(
            args.iter().map(String::as_str),
            &self.shell_state.env_vars,
            &self.shell_state.cwd,
            self.shell_state.logical_cwd.as_deref(),
            stdout,
            stderr,
        )?;
        if outcome.physical_resolution_failed {
            // rubash#3349: GNU builtins/cd.def:183-191 resetpwd — a
            // failed physical resolution frees
            // the_current_working_directory and leaves it NULL. The
            // session's logical cache is poisoned, not merely stale:
            // clear it so the next default / -L face re-derives
            // through the getcwd gate (and fails the same way) instead
            // of printing the stale value. The poison rides the Clone
            // fork boundary (shell/state.rs:204), so a subshell's
            // failure poisons only the clone, never the parent.
            self.shell_state.logical_cwd = None;
        }
        Ok(outcome.status)
    }

    pub(in crate::executor) fn execute_loop_control(
        &mut self,
        cmd: &CommandNode,
        kind: LoopControlKind,
    ) -> Result<(), ExecuteError> {
        // GNU builtins/common.h:29-36 CHECK_HELPOPT (break.def /
        // continue.def run it first): the RAW first word `--help`
        // prints the long help to stdout and the builtin RETURNS
        // EX_USAGE (258 > EX_SHERRBASE) — before check_loop_level, so
        // there is no "only meaningful" diagnostic outside a loop, and
        // without setting `breaking`/`continuing`, so a loop body runs
        // on after it. Mirrors execute_return's --help face
        // (rubash#2128, the #1229 family's residual face).
        if cmd.words.get(1).map(String::as_str) == Some("--help") {
            let mut stdout = Vec::new();
            crate::builtins::help::print_builtin_help(kind.name(), &mut stdout)?;
            self.write_buffered_builtin_output(cmd, &stdout, &[])?;
            self.exit_code = 2;
            self.special_builtin_failed.set(true);
            return Ok(());
        }
        let mut stderr = Vec::new();
        let row = super::builtin_table::builtin_table_row(kind.name())
            .expect("loop-control row in BUILTIN_TABLE");
        let mut diag = super::builtin_diag::Diag::new(row, &mut stderr);
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
                diag.free(format_args!(
                    "only meaningful in a `for', `while', or `until' loop"
                ));
                diag.flush_with(|| self.diagnostic_prolog());
            }
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            // Bash emits the diagnostic but leaves the command status at zero
            // when an out-of-loop break/continue is followed by another
            // command in the same list.
            self.exit_code = 0;
            return Ok(());
        }

        // Stage-2 ④ Phase 3: the count goes through Diag::numeric_arg
        // (builtins/common.c:488-520 get_numeric_arg (list,
        // interactive ? 2 : 1)) on the head slice — validity before
        // the no_args surplus tail (rubash#2956), and the surplus
        // before break.def's own range check (sh_erange (word,
        // "loop count"), builtins/break.def:80-91). The unwind
        // class of each failure is the caller's (the arms below),
        // so the parse is Soft and its outcome is reified here.
        enum LoopParse {
            Level(usize),
            TooMany,
            OutOfRange,
            NotNumeric,
        }
        let words = &cmd.words[1..];
        let start = usize::from(words.first().map(String::as_str) == Some("--"));
        let parsed = if words.len() > start {
            match diag.numeric_arg(&words[..=start], super::builtin_diag::NumericFatal::Soft) {
                None => LoopParse::NotNumeric,
                Some(_) if words.len() > start + 1 => {
                    diag.report(super::builtin_diag::DiagKind::TooManyArgs);
                    LoopParse::TooMany
                }
                Some(value) if value <= 0 => {
                    diag.error(super::builtin_diag::DiagKind::Erange(
                        Some(words[start].clone()),
                        "loop count".to_string(),
                    ));
                    LoopParse::OutOfRange
                }
                Some(value) => LoopParse::Level(value as usize),
            }
        } else {
            // No count word (or a bare `--`): get_numeric_arg's
            // default count, 1.
            LoopParse::Level(1)
        };
        match parsed {
            LoopParse::Level(level) => {
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
                if self.debug_action_running && own_loop_depth > 0 {
                    // rubash#2969: inside a DEBUG trap action the builtin
                    // must not unwind — the firing site is not this loop's
                    // frame (a for head's frame is not even entered when
                    // its trap fires). GNU only ASSIGNS the pending flag
                    // (builtins/break.def:88 `breaking = newbreak`,
                    // :133 `continuing = newcont`); the command-entry
                    // gate (execute_cmd.c:635-636) skips the commands
                    // still to run and the loop driver consumes one
                    // level after the body (execute_cmd.c:3108-3118).
                    // Fork-boundary executors (own depth 0) keep the
                    // ExitCode arm below: their list end IS their flag
                    // consumption, and no live frame here could receive
                    // an unwind either.
                    self.defer_loop_control(kind, level);
                    return Ok(());
                }
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
            LoopParse::TooMany => {
                diag.flush_with(|| self.diagnostic_prolog());
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
            LoopParse::OutOfRange => {
                diag.flush_with(|| self.diagnostic_prolog());
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
                if self.debug_action_running && own_loop_depth > 0 {
                    // rubash#2969: GNU break.def:88 still assigns
                    // `breaking = loop_level` on the range failure; the
                    // parked failure status above rides to the consuming
                    // loop driver (consume_pending_loop_control), exactly
                    // as it rides ExecuteError::Break today.
                    self.defer_loop_control(LoopControlKind::Break, total_loop_depth);
                    return Ok(());
                }
                if own_loop_depth == 0 {
                    // Fork-boundary child: `breaking = loop_level` ends the
                    // child's list with the builtin's failure status (1).
                    return Err(ExecuteError::ExitCode(1));
                }
                let level = self.shell_state.loop_depth;
                Err(ExecuteError::Break(level))
            }
            LoopParse::NotNumeric => {
                diag.flush_with(|| self.diagnostic_prolog());
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
                if self.invocation_interactive() {
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

    /// rubash#2838: consume one level of the SIGTSTP loop-break count
    /// (GNU execute_cmd.c:3108/3277/3609/3840 post-body `if (breaking)
    /// { breaking--; break; }`). True when a level was consumed.
    pub(in crate::executor) fn consume_stopped_loop_breaking(&mut self) -> bool {
        if self.stopped_loop_breaking > 0 {
            self.stopped_loop_breaking -= 1;
            true
        } else {
            false
        }
    }

    /// rubash#2969: record a break/continue executed inside a DEBUG trap
    /// action as GNU's pending flag instead of unwinding (builtins/
    /// break.def:88 `breaking = newbreak`, :133 `continuing =
    /// newcont`). Assignment, not accumulation — a later action
    /// overwrites an earlier one, as in C.
    pub(in crate::executor) fn defer_loop_control(&mut self, kind: LoopControlKind, level: usize) {
        match kind {
            LoopControlKind::Break => self.shell_state.loop_breaking = level,
            LoopControlKind::Continue => self.shell_state.loop_continuing = level,
        }
    }

    /// rubash#2969: the execute_cmd.c:635-636 command-entry gate — while
    /// either pending flag stands, every command entered is skipped
    /// whole (it does not run, and the DEBUG trap does not fire for it).
    pub(in crate::executor) fn loop_control_pending(&self) -> bool {
        self.shell_state.loop_breaking > 0 || self.shell_state.loop_continuing > 0
    }

    /// rubash#2969: one loop driver consuming the pending flags after a
    /// body (GNU execute_cmd.c:3108-3118). A parked break failure status
    /// (a `break 0` in the trap action) becomes the loop's status, like
    /// the ExecuteError::Break consumer arms; a plain flag leaves the
    /// completed body's status standing, as GNU's
    /// last_command_exit_value does.
    pub(in crate::executor) fn consume_pending_loop_control(&mut self) -> LoopControlFlow {
        if self.shell_state.loop_breaking > 0 {
            self.shell_state.loop_breaking -= 1;
            if let Some(status) = self.take_break_failure_status() {
                self.exit_code = status;
            }
            return LoopControlFlow::BreakLoop;
        }
        if self.shell_state.loop_continuing > 0 {
            self.shell_state.loop_continuing -= 1;
            return if self.shell_state.loop_continuing > 0 {
                LoopControlFlow::BreakLoop
            } else {
                LoopControlFlow::NextIteration
            };
        }
        LoopControlFlow::None
    }

    /// rubash#2969: GNU execute_cmd.c:3819-3831 — a while/until whose
    /// test fails while a flag stands decrements each flag once, so the
    /// level banked against this loop does not leak into the enclosing
    /// list (no driver would ever consume it there; the command-entry
    /// gate would skip the next command forever).
    pub(in crate::executor) fn drain_pending_loop_control(&mut self) {
        if self.shell_state.loop_breaking > 0 {
            self.shell_state.loop_breaking -= 1;
        }
        if self.shell_state.loop_continuing > 0 {
            self.shell_state.loop_continuing -= 1;
        }
    }

    /// rubash#2969: a non-loop-control unwind crossing a loop frame
    /// abandons every frame it crosses (GNU's jump targets discard the
    /// whole construct), so flags banked against those frames must not
    /// survive into the enclosing list.
    pub(in crate::executor) fn clear_pending_loop_control(&mut self) {
        self.shell_state.loop_breaking = 0;
        self.shell_state.loop_continuing = 0;
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
        // (rubash#1379). Stage-2 ④ Phase 3: the status word is judged
        // through Diag::numeric_arg on the head slice (the `--` plus
        // the word), and validity is judged BEFORE the no_args
        // surplus tail (rubash#2956) — the old code checked the
        // operand count first, so `return x y` misreported
        // too-many-arguments.
        let start = usize::from(args.first().map(String::as_str) == Some("--"));
        let row =
            super::builtin_table::builtin_table_row("return").expect("return row in BUILTIN_TABLE");
        let mut diag = super::builtin_diag::Diag::new(row, &mut stderr);
        let mut numeric_failed = false;
        let mut status_word: Option<i64> = None;
        if args.len() > start {
            // GNU get_exitstat parses via general.c:248 valid_number
            // (intmax_t): whitespace-padded numerics are legal
            // (rubash#1339); out-of-range rejected (rubash#1403).
            match diag.numeric_arg(&args[..=start], super::builtin_diag::NumericFatal::Soft) {
                Some(value) => status_word = Some(value),
                None => {
                    numeric_failed = true;
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
                }
            }
        }
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
        // (rubash#705 machinery, rubash#1213). Only a VALID head
        // reaches it: an invalid head already reported above.
        if !numeric_failed && args.len() > start + 1 {
            diag.report(super::builtin_diag::DiagKind::TooManyArgs);
            diag.flush_with(|| self.diagnostic_prolog());
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            self.exit_code = 2;
            self.raise_evalerror_abort();
            self.evalerror_exit_toomany_pending.set(true);
            return Ok(());
        }
        let status = if numeric_failed {
            2
        } else if let Some(value) = status_word {
            crate::builtins::exit::normalize_status(value)
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

        // The numeric diagnostic (if any) is pending in the Diag;
        // render it into the stderr buffer before any arm below
        // ships that buffer.
        diag.flush_with(|| self.diagnostic_prolog());
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
                && !self.invocation_interactive()
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

        diag.free(format_args!(
            "can only `return' from a function or sourced script"
        ));
        diag.flush_with(|| self.diagnostic_prolog());
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
