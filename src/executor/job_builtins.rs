use super::*;
use std::io::Write;

/// Result of waiting for one background pid. `Interrupted` carries the
/// 128+signal status GNU's wait_intr_flag longjmp produces
/// (wait.def:181-191): the job stays live and the signal is re-queued so
/// the trap action still runs at the next command boundary.
enum WaitPidOutcome {
    NotFound,
    Status(i32),
    Interrupted(i32),
    /// The job is already known to be stopped. GNU's wait never blocks
    /// on a stopped job (jobs.c wait_for_background_pids /
    /// wait_for_job); the caller renders the form-specific warning and
    /// status (rubash#1246).
    Stopped {
        job_id: u32,
        stop_signal: i32,
    },
}

/// GNU jobs.c:1969 printable_job_status -> j_strsignal: the
/// strsignal(3) description of the recorded stop signal, used only by
/// the `jobs -l` long format. SIGTSTP's description is plain `Stopped`
/// (the caller's fallback), so it has no entry here.
fn stopped_long_text(stop_signal: Option<i32>) -> Option<&'static str> {
    let _ = stop_signal;
    #[cfg(unix)]
    {
        match stop_signal {
            Some(sig) if sig == libc::SIGSTOP as i32 => return Some("Stopped (signal)"),
            Some(sig) if sig == libc::SIGTTIN as i32 => return Some("Stopped (tty input)"),
            Some(sig) if sig == libc::SIGTTOU as i32 => return Some("Stopped (tty output)"),
            _ => {}
        }
    }
    None
}

/// `resolve_background_job` with GNU's three-way get_job_spec result
/// kept: `Ambiguous` is DUP_JOB (jobs.h:207, builtins/common.c:672-679)
/// — a name spec matched more than once, and the builtin reports
/// `{name}: ambiguous job spec` instead of "no such job".
pub(in crate::executor) enum JobOperandResolution {
    Pid(u32),
    Ambiguous,
    NoMatch,
}

/// The name GNU prints in `{builtin}: {name}: ambiguous job spec` —
/// get_job_by_name (builtins/common.c:673) sees the spec stripped of
/// its `%` marker and of `?` for the substring form.
pub(in crate::executor) fn jobspec_match_name(spec: &str) -> &str {
    let body = spec.strip_prefix('%').unwrap_or(spec);
    body.strip_prefix('?').unwrap_or(body)
}

impl Executor {
    pub(in crate::executor) fn execute_times(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::times::execute_with_io(
            cmd.words[1..].iter().map(String::as_str),
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_caller(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        // GNU caller.def reads the FUNCNAME array, whose bottom frame is the
        // synthetic "main" (variables.c make_funcname_visible / execute_intern_function),
        // so `caller 0` inside a function called from the script reports
        // FUNCNAME[1]="main" (dbg-support.tests: "85 main ./dbg-support.tests").
        let funcname = self.indexed_array_stack("FUNCNAME");
        let lineno = self.indexed_array_stack("BASH_LINENO");
        // The executor uses `main` as the synthetic source name for function
        // calls made from an inline command string.  Bash's `caller` builtin
        // reports that frame as `environment`, while BASH_SOURCE itself keeps
        // the internal synthetic name for compatibility with the shell API.
        let source: Vec<String> = self
            .indexed_array_stack("BASH_SOURCE")
            .into_iter()
            .map(|name| {
                if name == "main" {
                    "environment".to_string()
                } else {
                    name
                }
            })
            .collect();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::caller::execute_with_io(
            &cmd.words[1..],
            &funcname,
            &lineno,
            &source,
            &self.diagnostic_prefix(),
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_jobs(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        self.refresh_background_jobs()?;
        let mut stderr = Vec::new();
        let action = crate::builtins::jobs::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        match action {
            crate::builtins::jobs::JobsAction::Complete(status) => {
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                Ok(status)
            }
            crate::builtins::jobs::JobsAction::List { options, jobs } => {
                let (stdout, status) = self.background_jobs_output(options, &jobs, &mut stderr)?;
                self.write_buffered_builtin_output(cmd, stdout.as_bytes(), &stderr)?;
                Ok(status)
            }
            crate::builtins::jobs::JobsAction::Execute(words) => {
                if !stderr.is_empty() {
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(1);
                }
                // GNU builtins/jobs.def execute_list_with_replacements: every
                // leading-`%` word is replaced with `itos (j->pgrp)` — the
                // process-GROUP ID (jobs.c:649), not the job pid — and a bad
                // spec is NOT an error: it passes through unchanged and the
                // command still runs (rubash#1096).
                let words = self.expand_jobs_x_words(words, &mut stderr)?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                let mut command = cmd.clone();
                command.words = words;
                self.execute_command(&command)?;
                Ok(self.exit_code)
            }
        }
    }

    fn expand_jobs_x_words(
        &self,
        words: Vec<String>,
        stderr: &mut Vec<u8>,
    ) -> Result<Vec<String>, ExecuteError> {
        let mut expanded = Vec::with_capacity(words.len());
        for word in words {
            if !word.starts_with('%') {
                expanded.push(word);
                continue;
            }
            // GNU only resolves live jobs here (get_job_by_jid over the
            // jobs array — no bgpids/completed-status fallback, which is
            // a wait-only path). INVALID_JOB passes through unchanged —
            // and DUP_JOB still reports the get_job_by_name diagnostic
            // (builtins/common.c:673, this_shell_builtin is `jobs`).
            match self.resolve_background_pgrp(&word) {
                Ok(pgrp) => expanded.push(pgrp.to_string()),
                Err(JobOperandResolution::Ambiguous) => {
                    writeln!(
                        stderr,
                        "{}jobs: {}: ambiguous job spec",
                        self.diagnostic_prefix(),
                        jobspec_match_name(&word)
                    )?;
                    expanded.push(word);
                }
                Err(_) => expanded.push(word),
            }
        }
        Ok(expanded)
    }

    /// Resolve a `%`-spec to its job's recorded process-group ID
    /// (jobs.c:649 j->pgrp), the value `jobs -x` substitutes.
    fn resolve_background_pgrp(&self, spec: &str) -> Result<u32, JobOperandResolution> {
        let job_id = match self.shell_state.job_table.resolve_jobspec(spec) {
            crate::jobs::table::JobspecMatch::Job(job_id) => job_id,
            crate::jobs::table::JobspecMatch::Ambiguous => {
                return Err(JobOperandResolution::Ambiguous);
            }
            crate::jobs::table::JobspecMatch::NoMatch => {
                return Err(JobOperandResolution::NoMatch);
            }
        };
        self.shell_state
            .job_table
            .jobs
            .get(&job_id)
            .map(|entry| entry.pgrp)
            .ok_or(JobOperandResolution::NoMatch)
    }

    pub(in crate::executor) fn execute_wait(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        // GNU jobs.c wait_for: a job reaped BY the wait itself has its
        // signal-death notice pretty-printed while wait still owns the
        // command's descriptors — `wait $p 2>/dev/null` silences the
        // "Killed" notice. Notices queued BEFORE this wait (reaped at
        // an earlier boundary) keep the boundary route: the raw-stderr
        // flush in report_pending_signal_notices. So: snapshot the
        // queue length, run the wait, then drain only the notices this
        // wait produced and route them through the wait command's own
        // fd-2 binding instead of leaving them for the raw flush
        // (rubash#1303).
        let notice_mark = self.pending_signal_notices.borrow().len();
        let result = self.execute_wait_inner(cmd);
        let drained: Vec<(u32, i32, String, bool)> = {
            let mut queue = self.pending_signal_notices.borrow_mut();
            let mark = notice_mark.min(queue.len());
            queue.drain(mark..).collect()
        };
        if !drained.is_empty() {
            let text = self.format_signal_notices(drained);
            if !text.is_empty() {
                if result.is_ok() {
                    self.write_buffered_builtin_output(cmd, &[], text.as_bytes())?;
                } else {
                    let _ = self.write_buffered_builtin_output(cmd, &[], text.as_bytes());
                }
            }
        }
        result
    }

    fn execute_wait_inner(&mut self, cmd: &CommandNode) -> Result<i32, ExecuteError> {
        self.refresh_background_jobs_observed()?;
        // GNU wait.def wait_builtin() internal_getopt "fnp:": a dangling
        // `-p` (no joined text, no next word) is a getopt error, reported
        // before any validation or waiting, and returns EX_USAGE
        // (rubash#1156).
        if wait_p_argument_missing(&cmd.words[1..]) {
            let mut stderr = Vec::new();
            writeln!(
                stderr,
                "{}wait: -p: option requires an argument",
                self.diagnostic_prefix()
            )?;
            writeln!(stderr, "wait: usage: wait [-fn] [-p var] [id ...]")?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            return Ok(2);
        }
        // GNU wait.def:139-152: a `-p name` option is validated and the
        // variable unbound before any waiting happens, in every wait form —
        // an invalid identifier or a readonly name fails immediately, and a
        // name that never gets bound stays unset.

        if let Some((name, name_index)) = wait_assign_var(&cmd.words[1..]) {
            let expand_once = crate::builtins::shopt::option_enabled(
                &self.shell_state.env_vars,
                "array_expand_once",
            );
            // GNU wait.def:156 SET_VFLAGS (builtins/common.h:279): the -p
            // operand's arrayflags are VA_NOEXPAND when array_expand_once
            // is on, plus VA_ONEWORD only when the option is on AND the
            // operand word itself was a syntactic array reference
            // (W_ARRAYREF; execute_cmd.c:4370 fix_arrayref_words on the
            // raw token). A joined `-pNAME` word can never carry it (the
            // raw `-pA[k]` is not a valid reference).
            let w_arrayref = cmd
                .word_metadata
                .iter()
                .find(|metadata| metadata.word_index == name_index + 1)
                .is_some_and(|metadata| {
                    crate::executor::subscript_expansion::valid_array_reference_env(
                        &metadata.raw,
                        false,
                        false,
                        &self.shell_state.env_vars,
                    )
                });
            // wait.def:157: valid_identifier OR valid_array_reference under
            // the flag set — `A[$rkey]` (`A[]]` verbatim) is legal.
            if !is_shell_name(&name)
                && !crate::executor::subscript_expansion::valid_array_reference_env(
                    &name,
                    expand_once,
                    expand_once && w_arrayref,
                    &self.shell_state.env_vars,
                )
            {
                let mut stderr = Vec::new();
                writeln!(
                    stderr,
                    "{}wait: `{name}': not a valid identifier",
                    self.diagnostic_prefix()
                )?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                return Ok(1);
            }
            // GNU builtins/common.c:1021 builtin_unbind_variable: the
            // operand is looked up with find_variable (variables.c:2363),
            // which resolves the nameref chain — the readonly and
            // non-unsettable gates test the REFERENT while the diagnostics
            // still name the operand. A chain that resolves to nothing
            // (missing or element-form cell, circular, depth overflow) is
            // v == 0: both gates are skipped and unbind_variable
            // (variables.c:3788) makunbounds the operand itself. A
            // bracketed operand is a literal name at this layer —
            // makunbound("A[k]") never matches a stored cell, so no
            // element is unbound and a readonly array cannot fail here
            // (unlike `unset A[k]`, which parses the subscript).
            let found = match self.nameref_resolution(&name) {
                NamerefResolution::Target(target)
                    if is_shell_name(&target) && self.variable_cell_exists(&target) =>
                {
                    Some(target)
                }
                NamerefResolution::NotNameref => {
                    self.variable_cell_exists(&name).then(|| name.clone())
                }
                _ => None,
            };
            if let Some(var_name) = found.as_deref() {
                if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, var_name) {
                    let mut stderr = Vec::new();
                    writeln!(
                        stderr,
                        "{}wait: {name}: cannot unset: readonly variable",
                        self.diagnostic_prefix()
                    )?;
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(1);
                }
                // GNU variables.h:161 non_unsettable_p: the att_nounset
                // dynamic variables (variables.c:1889-1893) refuse
                // unbinding outright.
                if matches!(
                    var_name,
                    "BASH_ARGC" | "BASH_ARGV" | "BASH_SOURCE" | "BASH_LINENO"
                ) {
                    let mut stderr = Vec::new();
                    writeln!(
                        stderr,
                        "{}wait: {name}: cannot unset",
                        self.diagnostic_prefix()
                    )?;
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(1);
                }
            }
            // GNU variables.c:3959 makunbound: the binding is disposed as
            // a whole — mirror entry, typed cell, attribute bits — and the
            // name leaves the export environment unconditionally
            // (variables.c:3839). Dropping only the mirror entry leaves
            // the inherited process-env copy to resurrect through
            // exact_case_env_var, nounset boundness, and the spawn env
            // base — the #943 residue class (#971). A local binding takes
            // makunbound's invisible-mark branch (variables.c:3984-4005)
            // instead of disappearing outright.
            let unbind_name = found.unwrap_or(name);
            if !self.unset_outer_local_variable(&unbind_name) {
                self.shell_state.env_vars.remove(&unbind_name);
                self.shell_state.variables.remove(&unbind_name);
                crate::executor::local_helpers::unset_process_env(&unbind_name);
                self.shell_state.env_vars.remove_unset_marks(&unbind_name);
                // GNU: an unbound dynamic variable stays unbound for the
                // shell's lifetime (the unset builtin's UNSET_DYNAMIC_VARS
                // record).
                if self.dynamic_parameter_is_set(&unbind_name) {
                    mark_env_name(
                        &mut self.shell_state.env_vars,
                        UNSET_DYNAMIC_VARS,
                        &unbind_name,
                    );
                }
            }
        }
        // GNU wait.def:193-206 first_pending_trap: a trapped signal already
        // pending makes `wait` return 128+sig without waiting at all (the
        // trap action runs at the next command boundary).
        if let Some(interrupt) = self.wait_pending_signal_status()? {
            self.write_buffered_builtin_output(cmd, &[], &[])?;
            return Ok(interrupt);
        }
        if let Some(request) = wait_any_request(&cmd.words[1..]) {
            // GNU builtins/wait.def:209-246 + jobs.c:3456 wait_for_any_job:
            // `wait -n` returns the first unnotified dead job in slot order,
            // restricted to the operand waitlist when operands are given
            // (set_waitlist, wait.def:352-399 — invalid operands diagnose but
            // valid ones still wait), and -p binds the reaped job's last
            // pid. Numeric operands consult saved reaped statuses first
            // (check_nonjobs/bgpids, wait.def:423+), which completed_statuses
            // already models.
            let mut stderr = Vec::new();
            let mut candidates: Vec<u32> = Vec::new();
            for operand in &request.operands {
                match self.resolve_background_job_operand(operand) {
                    JobOperandResolution::Pid(pid) => {
                        if !candidates.contains(&pid) {
                            candidates.push(pid);
                        }
                    }
                    JobOperandResolution::Ambiguous => {
                        // GNU wait.def:368-377 set_waitlist: DUP_JOB is
                        // INVALID_JOB — the get_job_by_name diagnostic
                        // prints, then sh_badjob reports the spec.
                        writeln!(
                            stderr,
                            "{}wait: {}: ambiguous job spec",
                            self.diagnostic_prefix(),
                            jobspec_match_name(operand)
                        )?;
                        writeln!(
                            stderr,
                            "{}wait: {operand}: no such job",
                            self.diagnostic_prefix()
                        )?;
                    }
                    JobOperandResolution::NoMatch => {
                        write_wait_operand_error(
                            operand,
                            &self.diagnostic_prefix(),
                            &self.error_prefix(),
                            &mut stderr,
                        )?;
                    }
                }
            }
            if !request.operands.is_empty() && candidates.is_empty() {
                // set_waitlist counted no waitable jobs: every operand
                // failed and wait -n returns 127.
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                return Ok(127);
            }

            loop {
                self.refresh_background_jobs_observed()?;
                // wait.def:328-331: the -n wait loop is interruptible by a
                // trapped signal just like the operand form.
                if let Some(interrupt) = self.wait_pending_signal_status()? {
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(interrupt);
                }
                if let Some((pid, status)) = self.first_completed_wait_candidate(&candidates) {
                    self.join_coproc_stderr_forwarder(pid)?;
                    // wait -n consumes the job (delete_job); the status stays
                    // in completed_statuses as the bgpids equivalent so a
                    // later operand-addressed `wait -n $pid` still reports it.
                    self.shell_state
                        .job_table
                        .remove_job_by_pid_preserve_status(pid);
                    self.forget_background_runtime(pid);
                    if let Some(wait_var) = &request.assign_var {
                        let arrayref = self.wait_var_arrayref();
                        self.bind_wait_variable(wait_var, pid.to_string(), arrayref);
                    }
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(status);
                }
                // Any candidate still running? GNU blocks in wait_for
                // (ANY_PID) until a child exits; we poll the Child handles.
                let alive = if candidates.is_empty() {
                    self.shell_state.job_table.jobs.values().any(|job| {
                        job.background
                            && job
                                .pids
                                .iter()
                                .any(|pid| self.background_children.contains_key(pid))
                    })
                } else {
                    candidates
                        .iter()
                        .any(|pid| self.background_children.contains_key(pid))
                };
                if !alive {
                    // wait_for_any_job returns -1 with nothing left to wait
                    // for; wait.def:243-245 maps that to 127 and leaves -p's
                    // variable unset.
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(127);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        if cmd.words.len() == 1
            && self
                .shell_state
                .job_table
                .jobs
                .values()
                .any(|job| job.background)
        {
            let pids = self
                .shell_state
                .job_table
                .jobs
                .values()
                .filter(|job| job.background)
                .flat_map(|job| job.pids.iter().copied())
                .collect::<Vec<_>>();
            let mut wait_stderr: Vec<u8> = Vec::new();
            for pid in pids {
                // wait.def:238-245 wait_for_any_job + :328-331: a trapped
                // signal interrupts the no-operand wait too, returning
                // 128+sig.
                match self.wait_for_background_pid(pid, false)? {
                    WaitPidOutcome::Interrupted(interrupt) => {
                        self.write_buffered_builtin_output(cmd, &[], &wait_stderr)?;
                        return Ok(interrupt);
                    }
                    WaitPidOutcome::Stopped { job_id, .. } => {
                        // GNU jobs.c:2818 wait_for_background_pids: a
                        // stopped job is reported, never waited for
                        // (rubash#1246).
                        writeln!(
                            wait_stderr,
                            "{}wait: warning: job {}[{}] stopped",
                            self.diagnostic_prefix(),
                            job_id,
                            pid
                        )?;
                    }
                    _ => {}
                }
            }
            // rubash#1022: GNU's no-operand wait also reaps finished
            // process substitutions (jobs.c waits the last procsub when it
            // is $!). Completed placeholders are already dead, so the
            // non-blocking sweep only drops their kernel shells — a later
            // `kill -0 $!` then reports ESRCH like GNU's reaped procsub.
            // Pending placeholders (still-open persistent `>(...)`) are
            // left alone: GNU would block, but an unkillable block here
            // could hang the shell, so they stay signallable instead.
            self.sweep_procsub_placeholders();
            self.write_buffered_builtin_output(cmd, &[], &wait_stderr)?;
            // Bash's no-operand wait reports success after waiting for all
            // current jobs; individual statuses require an explicit operand.
            return Ok(0);
        }

        if let Some(operands) = wait_background_operands(&cmd.words[1..]) {
            if !operands.is_empty()
                && operands
                    .iter()
                    .any(|operand| self.resolve_background_job(operand).is_some())
            {
                // GNU wait.def:338-339: -p binds the pid of the LAST operand
                // waited for (pstat.pid; NO_PID when the last operand
                // failed, leaving the pre-unbound variable unset).
                let wait_var = wait_assign_var(&cmd.words[1..])
                    .map(|(name, _index)| (name, self.wait_var_arrayref()));

                let status =
                    self.wait_for_background_operands(&operands, cmd, wait_var.as_ref())?;
                return Ok(status);
            }
        }

        if cmd.words.len() == 2 {
            match self.resolve_background_job_operand(&cmd.words[1]) {
                JobOperandResolution::Pid(pid) => match self.wait_for_background_pid(pid, true)? {
                    WaitPidOutcome::Status(status) | WaitPidOutcome::Interrupted(status) => {
                        self.write_buffered_builtin_output(cmd, &[], &[])?;
                        return Ok(status);
                    }
                    WaitPidOutcome::Stopped {
                        job_id,
                        stop_signal,
                    } => {
                        // GNU jobs.c wait_for_job (rubash#1246).
                        let mut stderr = Vec::new();
                        writeln!(
                            stderr,
                            "{}warning: wait_for_job: job {} is stopped",
                            self.diagnostic_prefix(),
                            job_id
                        )?;
                        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                        return Ok(128 + stop_signal);
                    }
                    WaitPidOutcome::NotFound => {}
                },
                JobOperandResolution::Ambiguous => {
                    // GNU wait.def:300-311: a DUP_JOB jobspec is
                    // INVALID_JOB — the get_job_by_name diagnostic is the
                    // whole report (no sh_badjob), status 127.
                    let mut stderr = Vec::new();
                    writeln!(
                        stderr,
                        "{}wait: {}: ambiguous job spec",
                        self.diagnostic_prefix(),
                        jobspec_match_name(&cmd.words[1])
                    )?;
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(127);
                }
                JobOperandResolution::NoMatch => {
                    if let Ok(pid) = cmd.words[1].parse::<u32>() {
                        match self.wait_for_background_pid(pid, true)? {
                            WaitPidOutcome::Status(status)
                            | WaitPidOutcome::Interrupted(status) => {
                                self.write_buffered_builtin_output(cmd, &[], &[])?;
                                return Ok(status);
                            }
                            WaitPidOutcome::Stopped {
                                job_id,
                                stop_signal,
                            } => {
                                // GNU jobs.c wait_for_job (rubash#1246).
                                let mut stderr = Vec::new();
                                writeln!(
                                    stderr,
                                    "{}warning: wait_for_job: job {} is stopped",
                                    self.diagnostic_prefix(),
                                    job_id
                                )?;
                                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                                return Ok(128 + stop_signal);
                            }
                            WaitPidOutcome::NotFound => {}
                        }
                    }
                }
            }
        }

        // Reaching the pure-builtin fallback means no operand resolved.
        // That path cannot see the job table, so ambiguous `%`-specs are
        // diagnosed here — in operand order, mirroring builtins/wait.rs —
        // before it reports the rest (wait.def:294-311 per-operand).
        if let Some(operands) = wait_background_operands(&cmd.words[1..]) {
            if operands.iter().any(|operand| operand.starts_with('%')) {
                let mut stderr = Vec::new();
                let mut status = 0;
                for operand in &operands {
                    status = match self.resolve_background_job_operand(operand) {
                        JobOperandResolution::Ambiguous => {
                            writeln!(
                                stderr,
                                "{}wait: {}: ambiguous job spec",
                                self.diagnostic_prefix(),
                                jobspec_match_name(operand)
                            )?;
                            127
                        }
                        JobOperandResolution::Pid(_) | JobOperandResolution::NoMatch => {
                            write_wait_operand_error(
                                operand,
                                &self.diagnostic_prefix(),
                                &self.error_prefix(),
                                &mut stderr,
                            )?
                        }
                    };
                }
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                return Ok(status);
            }
        }

        let mut stderr = Vec::new();
        let status = crate::builtins::wait::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &self.error_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    /// GNU wait.def:156 SET_VFLAGS for the `-p` operand:
    /// VA_NOEXPAND|VA_ONEWORD when array_expand_once is on OR the operand
    /// word itself was a syntactic array reference (W_ARRAYREF;
    /// execute_cmd.c:4370). `name_index` is the operand's index inside
    /// `cmd.words[1..]`; a joined `-pNAME` word can never carry W_ARRAYREF
    /// because `-pA[k]` is not a valid reference.
    /// GNU wait.def:156 SET_VFLAGS bindflags: the element subscript binds
    /// verbatim when ASS_NOEXPAND is set, which follows array_expand_once
    /// alone (VA_ONEWORD from W_ARRAYREF only widens the subscript close
    /// to the last `]`; it does not make the bind verbatim).
    fn wait_var_arrayref(&self) -> bool {
        crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "array_expand_once")
    }

    /// GNU builtin_bind_var_to_int: an element name binds that element with
    /// the operand's flag set (verbatim under VA_NOEXPAND|VA_ONEWORD, else
    /// the deferred expand_subscript_string pass); a scalar name binds the
    /// variable. An undeclared `A[k]` auto-creates the array like
    /// bind_array_element does (assoc for a non-numeric key).
    fn bind_wait_variable(&mut self, name: &str, value: String, arrayref: bool) {
        let Some((base, subscript)) = parse_array_subscript(name) else {
            self.apply_shell_assignment_command("wait", name, value);
            return;
        };
        let base = base.to_string();
        let source = if arrayref {
            SubscriptSource::Protected(subscript)
        } else {
            SubscriptSource::ExpandedOnce(subscript)
        };
        let current = self
            .shell_state
            .env_vars
            .get(&base)
            .cloned()
            .unwrap_or_default();
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &base)
            || (!is_marked_array_var(&self.shell_state.env_vars, &base)
                && !is_array_storage(&current)
                && !matches!(
                    self.eval_indexed_subscript(source),
                    IndexedSubscript::Index(_)
                ))
        {
            // A nounset trap must abandon, not fall through and write the
            // assoc entry (rubash#639).
            if self.raise_nounset_subscript_abort() {
                return;
            }
            let mut entries = assoc_entries(&current);
            let key = self.resolve_array_subscript(source);
            if let Some(slot) = entries.iter_mut().find(|(entry_key, _)| *entry_key == key) {
                slot.1 = value;
            } else {
                entries.push((key, value));
            }
            self.shell_state
                .env_vars
                .insert(base.clone(), format_assoc_storage(entries));
            if !is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &base) {
                mark_env_name(&mut self.shell_state.env_vars, ASSOC_VARS, &base);
            }
            return;
        }
        match self.eval_indexed_subscript(source) {
            IndexedSubscript::Index(index) => {
                let mut entries = indexed_array_entries(&current);
                entries.insert(index as usize, value);
                crate::executor::arrays::sync_typed_indexed_array(
                    &mut self.shell_state.variables,
                    base.as_str(),
                    &entries,
                );
                self.shell_state
                    .env_vars
                    .insert(base.clone(), format_indexed_array_storage(entries));
                self.shell_state.env_vars.clear_array_cells_dirty();
                if !is_marked_array_var(&self.shell_state.env_vars, &base) {
                    mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, &base);
                }
            }
            _ => {
                // A nounset trap must abandon, not write the scalar fallback
                // (rubash#639).
                if self.raise_nounset_subscript_abort() {
                    return;
                }
                self.apply_shell_assignment_command("wait", name, value);
            }
        }
    }

    fn wait_for_background_operands(
        &mut self,
        operands: &[String],
        cmd: &CommandNode,
        wait_var: Option<&(String, bool)>,
    ) -> Result<i32, ExecuteError> {
        let resolved = operands
            .iter()
            .map(|operand| {
                (
                    operand.clone(),
                    self.resolve_background_job_operand(operand),
                )
            })
            .collect::<Vec<_>>();
        let mut stderr = Vec::new();
        let mut status = 0;
        let mut last_pid = None;

        for (operand, resolution) in resolved {
            let pid = match resolution {
                JobOperandResolution::Pid(pid) => pid,
                JobOperandResolution::Ambiguous => {
                    // GNU wait.def:300-311: DUP_JOB is INVALID_JOB — the
                    // get_job_by_name diagnostic is the whole report,
                    // status 127 (POSIX 4.70.2).
                    writeln!(
                        stderr,
                        "{}wait: {}: ambiguous job spec",
                        self.diagnostic_prefix(),
                        jobspec_match_name(&operand)
                    )?;
                    status = 127;
                    last_pid = None;
                    continue;
                }
                JobOperandResolution::NoMatch => {
                    status = write_wait_operand_error(
                        &operand,
                        &self.diagnostic_prefix(),
                        &self.error_prefix(),
                        &mut stderr,
                    )?;
                    last_pid = None;
                    continue;
                }
            };
            match self.wait_for_background_pid(pid, true)? {
                WaitPidOutcome::Interrupted(interrupt_status) => {
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(interrupt_status);
                }
                WaitPidOutcome::Stopped {
                    job_id,
                    stop_signal,
                } => {
                    // GNU jobs.c wait_for_job: a stopped job is reported
                    // (`warning: wait_for_job: job %d is stopped`) and
                    // the wait returns the stop status 128+sig without
                    // blocking (rubash#1246).
                    writeln!(
                        stderr,
                        "{}warning: wait_for_job: job {} is stopped",
                        self.diagnostic_prefix(),
                        job_id
                    )?;
                    status = 128 + stop_signal;
                    last_pid = Some(pid);
                }
                WaitPidOutcome::Status(wait_status) => {
                    status = wait_status;
                    last_pid = Some(pid);
                }
                WaitPidOutcome::NotFound => {
                    status = write_wait_operand_error(
                        &operand,
                        &self.diagnostic_prefix(),
                        &self.error_prefix(),
                        &mut stderr,
                    )?;
                    last_pid = None;
                }
            }
        }

        if let (Some((wait_var, arrayref)), Some(pid)) = (wait_var, last_pid) {
            self.bind_wait_variable(wait_var, pid.to_string(), *arrayref);
        }
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    /// First completed job among the `wait -n` candidates, in job-table
    /// order (GNU jobs.c:3456 wait_for_any_job scans slots, not the operand
    /// list). An empty candidate list means "any background job".
    fn first_completed_wait_candidate(&self, candidates: &[u32]) -> Option<(u32, i32)> {
        for job in self.shell_state.job_table.jobs.values() {
            if !job.background {
                continue;
            }
            let Some(pid) = job.pids.last().copied() else {
                continue;
            };
            if !candidates.is_empty() && !candidates.contains(&pid) {
                continue;
            }
            if let Some(status) = self.shell_state.job_table.completed_statuses.get(&pid) {
                return Some((pid, *status));
            }
        }
        // Numeric operands may name a saved reaped pid whose job entry is
        // already gone (bgpids; wait.def check_nonjobs).
        for pid in candidates {
            if let Some(status) = self.shell_state.job_table.completed_statuses.get(pid) {
                return Some((*pid, *status));
            }
        }
        // GNU jobs.c wait_for_any_job scans the jobs table first and the
        // procsubs list second: an operand-less `wait -n` also reports a
        // finished process substitution (rubash#1022: `echo x > >(exit 9);
        // wait -n` returns 9 on GNU). Oldest first — pids grow
        // monotonically, so the minimum is the earliest-registered
        // still-completed placeholder.
        if candidates.is_empty() {
            if let Some(pid) = self
                .shell_state
                .job_table
                .procsub_placeholders
                .iter()
                .copied()
                .filter(|pid| {
                    self.shell_state
                        .job_table
                        .completed_statuses
                        .contains_key(pid)
                })
                .min()
            {
                if let Some(status) = self.shell_state.job_table.completed_statuses.get(&pid) {
                    return Some((pid, *status));
                }
            }
        }
        None
    }

    /// GNU execute_cmd.c:2975-2983 REAP(): after each loop body the shell
    /// silently reaps dead jobs when it is non-interactive or job control is
    /// off — `for/while/until/select/(( ;; ))` iterations call this.
    pub(in crate::executor) fn reap_dead_jobs_after_loop_body(&mut self) {
        // GNU execute_cmd.c:3267 REAP() runs once per loop iteration; an
        // empty table makes the scan plus cleanup below no-ops, so skip the
        // consults too. Any `&` spawn restores the full path next iteration.
        if self.shell_state.job_table.jobs.is_empty() {
            return;
        }
        let interactive = self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_INTERACTIVE");
        let job_control =
            crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor");
        if !interactive || !job_control {
            self.shell_state
                .job_table
                .reap_dead_jobs(self.shell_state.last_background_pid);
        }
    }

    /// Reap one forked procsub placeholder (rubash#1022). Returns true when
    /// the pid is no longer a live placeholder (reaped now, or already gone
    /// — ECHILD). Non-placeholder pids are never touched.
    pub(in crate::executor) fn reap_procsub_placeholder(&mut self, pid: u32, block: bool) -> bool {
        if !self
            .shell_state
            .job_table
            .procsub_placeholders
            .contains(&pid)
        {
            return false;
        }
        #[cfg(unix)]
        {
            let mut status = 0;
            let flags = if block { 0 } else { libc::WNOHANG };
            // SAFETY: waitpid on a direct-child pid is async-signal-safe;
            // the status word is a stack local. Only the placeholder set's
            // pids are ever passed, so no foreign or runtime-owned child
            // (background_children handles) can be reaped here.
            let reaped = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, flags) };
            if reaped == pid as libc::pid_t
                || (reaped < 0
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD))
            {
                self.shell_state.job_table.procsub_placeholders.remove(&pid);
                return true;
            }
            return false;
        }
        #[cfg(not(unix))]
        {
            let _ = block;
            // Placeholders are only forked on unix; drop the stale entry so
            // a cross-platform state clone never sticks.
            self.shell_state.job_table.procsub_placeholders.remove(&pid);
            true
        }
    }

    /// Reap completed procsub placeholders (rubash#1022). Only the bare
    /// `wait` uses this: GNU's no-operand wait reaps finished process
    /// substitutions too (jobs.c waits the last procsub when it is `$!`),
    /// so afterwards `kill -0` reports ESRCH like GNU's reaped procsub.
    /// Pending placeholders (no recorded status — a still-open persistent
    /// `>(...)`) are never swept: the forked zombie is the only liveness
    /// proof until close completes them, mirroring GNU's live tee. The
    /// per-command refresh deliberately does NOT sweep: with /dev/fd pipes
    /// GNU's output-procsub children outlive the creating command, so an
    /// older `$!` stays signallable until a wait consumes it.
    pub(in crate::executor) fn sweep_procsub_placeholders(&mut self) {
        if self.shell_state.job_table.procsub_placeholders.is_empty() {
            return;
        }
        let candidates: Vec<u32> = self
            .shell_state
            .job_table
            .procsub_placeholders
            .iter()
            .copied()
            .filter(|pid| {
                self.shell_state
                    .job_table
                    .completed_statuses
                    .contains_key(pid)
            })
            .collect();
        for pid in candidates {
            self.reap_procsub_placeholder(pid, false);
        }
    }

    /// Queue the stderr notice for a child that died by a signal
    /// (rubash#229). The notice itself prints at the next reader-command
    /// boundary (report_pending_signal_notices), mirroring GNU eval.c:355
    /// notify_and_cleanup -> jobs.c:4543 notify_of_job_status.
    pub(crate) fn note_child_signal_death(
        &self,
        pid: u32,
        signal: i32,
        command: String,
        foreground: bool,
    ) {
        if signal <= 0 {
            return;
        }
        self.pending_signal_notices
            .borrow_mut()
            .push((pid, signal, command, foreground));
    }

    /// Command text for a foreground child's signal-death notice
    /// (rubash#229). GNU prints the job's `command` string — the pipeline
    /// text as typed (jobs.c make_job stores the_printed_command), not a
    /// re-quoted argv. __RUBASH_LAST_COMMAND holds exactly that source text
    /// for the command currently executing (public_accessors
    /// set_current_command); the expanded `cmd`'s words no longer match
    /// their WordMetadata (the raw-vs-value filter fails and every word
    /// falls into the single-quote fallback), so prefer it and keep
    /// bash_command_source_text as the shape-preserving fallback.
    pub(crate) fn signal_notice_command_text(&self, cmd: &CommandNode) -> String {
        let text = if let Some(text) = self
            .shell_state
            .env_vars
            .get("__RUBASH_LAST_COMMAND")
            .filter(|text| !text.is_empty())
        {
            text.clone()
        } else {
            bash_command_source_text(cmd)
        };
        // GNU's job for a `( ... )` is the subshell pipeline itself, so the
        // notice prints `( <inner text> )` (jobs.c pretty_print_job wraps
        // the subshell's stored command). Rubash runs the subshell flat, and
        // the notice fires for the inner child — wrap when one is open. The
        // single-pipeline subshell (the shape the notices occur in) matches
        // byte-for-byte; a multi-command subshell reports its last member's
        // text inside the parens.
        if self.shell_state.subshell_depth.get() > 0 && !text.starts_with('(') {
            format!("( {text} )")
        } else {
            text
        }
    }

    /// Print queued signal-death notices to stderr (GNU jobs.c:4625-4652
    /// notify_of_job_status JDEAD handling, driven at the reader boundary by
    /// eval.c:355 notify_and_cleanup). Shapes, with config-top.h defining
    /// both DONT_REPORT_SIGPIPE and DONT_REPORT_SIGTERM:
    /// - untrapped, not INT/TERM/PIPE (branch 1): `name: line N: ` +
    ///   pretty_print_job(JLIST_NONINTERACTIVE) -> `%5ld pid`, one space,
    ///   signal description padded to LONGEST_SIGNAL_DESC (jobs.h:43 = 27),
    ///   command;
    /// - otherwise, FOREGROUND jobs (branch 2): print_pipeline
    ///   JLIST_STANDARD -> description padded to 27, command (this is where
    ///   a foreground TERM death lands);
    /// - background TERM/INT/PIPE or trapped deaths: silent.
    pub(crate) fn report_pending_signal_notices(&mut self) -> Result<(), ExecuteError> {
        let notices = std::mem::take(&mut *self.pending_signal_notices.borrow_mut());
        let out = self.format_signal_notices(notices);
        if !out.is_empty() {
            crate::executor::shell_options::write_stderr_bytes(out.as_bytes())?;
        }
        Ok(())
    }

    /// Format queued signal-death notices exactly as
    /// report_pending_signal_notices prints them at the command
    /// boundary. Split out so execute_wait can route the notices ITS
    /// reap produced through the wait command's own fd-2 binding
    /// (rubash#1303).
    fn format_signal_notices(&self, notices: Vec<(u32, i32, String, bool)>) -> String {
        // Signal numbers are the invariant Linux/MSVC values (libc is a
        // unix-only dependency, and job_builtins.rs compiles on both).
        const SIGINT: i32 = 2;
        const SIGPIPE: i32 = 13;
        const SIGTERM: i32 = 15;
        let mut out = String::new();
        for (pid, signal, command, foreground) in notices {
            // jobs.c:4628-4638: INT is never reported; PIPE via
            // DONT_REPORT_SIGPIPE (config-top.h:46).
            if signal == SIGINT || signal == SIGPIPE {
                continue;
            }
            let trapped = super::trap_exec::signal_trap_name(signal).is_some_and(|name| {
                crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, &name)
                    .is_some_and(|action| !action.is_empty())
            });
            let description = crate::builtins::kill::signal_death_description(signal);
            let padding = 27usize.saturating_sub(description.chars().count());
            if !trapped && signal != SIGTERM {
                // Branch 1 (jobs.c:4625-4641): the "name: line N: PID desc
                // cmd" shape. The line at notify time is the command that
                // just finished (the boundary runs before the next command
                // sets its own line), matching GNU's line_number.
                out.push_str(&self.diagnostic_prefix());
                out.push_str(&format!("{pid:>5} {description}"));
                out.push_str(&" ".repeat(padding));
                out.push_str(&command);
                out.push('\n');
            } else if foreground {
                // Branch 2 (jobs.c:4640-4651): bare description + command.
                out.push_str(&description);
                out.push_str(&" ".repeat(padding));
                out.push_str(&command);
                out.push('\n');
            }
            // Background TERM/trapped deaths print nothing.
        }
        out
    }

    /// Per-command boundary reap of background children (GNU's
    /// notify_and_cleanup -> cleanup_dead_jobs cadence, jobs.c:3642 ->
    /// :1303): every finished child is marked completed, and a finished
    /// coproc is disposed the way coproc_reap disposes it — vars unbound,
    /// endpoints closed (execute_cmd.c:2166 coproc_dispose, :2450
    /// coproc_unsetvars). rubash#1023: GNU consults nothing about the next
    /// command's redirects, so a dead coproc's `name`/`name_PID` are gone
    /// before that command expands.
    pub(in crate::executor) fn refresh_background_jobs(&mut self) -> Result<(), ExecuteError> {
        self.refresh_background_jobs_impl(false)
    }

    /// The wait/fg form: an explicit wait observes reap state
    /// unconditionally (GNU wait_for blocks in waitchld for its
    /// operands), so deferred completions apply at once.
    pub(in crate::executor) fn refresh_background_jobs_observed(
        &mut self,
    ) -> Result<(), ExecuteError> {
        self.refresh_background_jobs_impl(true)
    }

    /// Record that this executor blocked in wait() for a foreground
    /// external process — one of GNU's reap observation points
    /// (wait_for reaps background children while it blocks,
    /// jobs.c:3064).
    pub(in crate::executor) fn note_foreground_wait(&self) {
        self.foreground_wait_count
            .set(self.foreground_wait_count.get() + 1);
    }

    fn refresh_background_jobs_impl(&mut self, force_observe: bool) -> Result<(), ExecuteError> {
        // part_036 (jobs.c waitchld lag): a reap becomes VISIBLE in the
        // job table only at a GNU observation point — the shell blocked
        // in wait() for a foreground process, the reader advanced to a
        // new line, or a fresh execute_ast run began. Between
        // observation points (a stretch of builtins inside one read)
        // `jobs` keeps showing the pre-death state: GNU 5.3.20 lists a
        // jobspec-killed job as Running until the reap boundary even
        // though the process is already dead, and the pre-fix eager
        // application here printed `Terminated` for it.
        let mark = (
            self.execute_generation.get(),
            self.reader_command_line.get(),
            self.foreground_wait_count.get(),
        );
        let observation_happened = mark != self.reap_prev_mark.get();
        // rubash#1607/#1612: publication of the shell's own kill
        // state marks uses a NARROWER event set than reap visibility.
        // A mere reader-line advance must not publish (kill -STOP,
        // one intervening builtin, then `jobs` still shows Running —
        // #1607); the events that publish are an explicit wait/fg
        // (force_observe), the shell having blocked on a foreground
        // process since the last refresh (foreground_wait_count
        // moved), or job control live on a terminal (under a pty
        // GNU publishes on the very next command — #1612). One
        // predicate, computed here at the notification point, feeds
        // both the state marks and (via `observe`) the reap side.
        let foreground_blocked = mark.2 != self.reap_prev_mark.get().2;
        self.reap_prev_mark.set(mark);
        let observe = force_observe || observation_happened;
        let state_observe = force_observe
            || foreground_blocked
            || (crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor")
                && self.stdin_source_is_terminal());
        // rubash#1494: apply the ripe kill state marks at the same
        // per-command boundary that reaps finished children.
        self.shell_state
            .job_table
            .apply_deferred_state_marks(state_observe);
        self.apply_pending_job_completions(observe)?;
        let mut finished = Vec::new();
        for (pid, child) in &mut self.background_children {
            if let Some(status) = child.try_wait()? {
                // rubash#1776: sample the reaped child's CPU for the
                // Windows CHILDREN half while its handle is live.
                super::read_helpers::note_reaped_child_cpu(child);
                // rubash#1528: stamp the reap instant for a timed
                // coproc — its deferred report must measure to the
                // death, not to whenever the completion is applied.
                if self
                    .pending_timed_coproc_reports
                    .iter()
                    .any(|(pending_pid, _, _)| pending_pid == pid)
                {
                    self.timed_coproc_death_instants
                        .insert(*pid, std::time::Instant::now());
                }
                finished.push((
                    *pid,
                    crate::executor::wait_status::process_exit_status(&status),
                    crate::executor::wait_status::exit_status_signal(&status),
                ));
            }
        }

        for (pid, status, term_signal) in finished {
            self.background_children.remove(&pid);
            // rubash#169: the reaped child no longer holds a capture pipe
            // write end — retire it from the writer set (drain when last).
            let drained = crate::executor::shell_options::retire_background_capture_writer(pid);
            if !drained.is_empty() {
                // Same buffer-priority as write_fd_endpoint's Stdout arm.
                if crate::executor::shell_options::stdout_capture_active() {
                    let _ = crate::executor::shell_options::stdout_capture_write(&drained);
                } else if let Some(buffer) = self.stdout_capture.as_mut() {
                    buffer.extend_from_slice(&drained);
                }
            }
            self.join_coproc_stderr_forwarder(pid)?;
            // Coproc disposal is per-command by contract (rubash#1023),
            // and at an observation point a completion applies at once;
            // anything else is stashed until the next observation
            // point (see the waitchld-lag note above).
            //
            // rubash#1604: one exception to "applies at once" — a job
            // the shell's OWN kill fatally signalled during this same
            // reader command. GNU's wait_for reaps such a job itself
            // when a `wait` follows, and a waited-for job's death
            // notice is suppressed (jobs.c wait_for/notify); if this
            // incidental sweep applies the completion first, the
            // notice is queued outside the wait and escapes raw —
            // even past `wait $p 2>/dev/null` — whenever the zombie
            // happens to be ready this early (~1/6 of runs). Stash
            // it instead; a wait/fg observed refresh (force_observe)
            // still applies it inside the wait, where the notice
            // drains through the wait's own fd 2.
            let fresh_fatal = !force_observe
                && term_signal.is_some()
                && !self.shell_state.coproc_names.contains_key(&pid)
                && self.shell_state.job_table.take_fresh_fatal(pid);
            if !fresh_fatal && (observe || self.shell_state.coproc_names.contains_key(&pid)) {
                self.apply_job_completion(pid, status, term_signal)?;
            } else {
                self.pending_job_completions
                    .push((pid, status, term_signal));
            }
        }
        Ok(())
    }

    /// Apply stashed reap results once an observation point arrives.
    fn apply_pending_job_completions(&mut self, observe: bool) -> Result<(), ExecuteError> {
        if !observe || self.pending_job_completions.is_empty() {
            return Ok(());
        }
        let pending = std::mem::take(&mut self.pending_job_completions);
        for (pid, status, term_signal) in pending {
            self.apply_job_completion(pid, status, term_signal)?;
        }
        Ok(())
    }

    /// rubash#1528: emit a deferred `time` report when the timed
    /// coproc's completion becomes visible. GNU measures the job's
    /// lifetime, so the metrics are only taken now — TimeCommandMetrics
    /// ::from_start turns the saved stamp into the spawn-to-completion
    /// interval (RUSAGE_CHILDREN already includes the reaped child).
    pub(in crate::executor) fn flush_timed_coproc_report(&mut self, pid: u32) {
        if let Some(position) = self
            .pending_timed_coproc_reports
            .iter()
            .position(|(pending_pid, _, _)| *pending_pid == pid)
        {
            let (_, started, posix_format) = self.pending_timed_coproc_reports.remove(position);
            // End the interval at the recorded reap instant when the
            // boundary sweep observed the death; the blocking wait
            // path reaps and flushes in one motion, so `now` is the
            // reap instant there.
            let end = self
                .timed_coproc_death_instants
                .remove(&pid)
                .unwrap_or_else(std::time::Instant::now);
            self.print_time_shell_at(posix_format, started, end);
        }
    }

    /// Make one reaped child's completion visible in the job table:
    /// the term-signal record, the jobs.c:4625 death notice, the
    /// completion mark, the CHLD trap, coproc retirement.
    fn apply_job_completion(
        &mut self,
        pid: u32,
        status: i32,
        term_signal: Option<i32>,
    ) -> Result<(), ExecuteError> {
        // The raw wait status knows whether the child died by a
        // signal; the flattened status does not. Record it so the
        // jobs listing prints the signal description (rubash#1256).
        if let Some(signal) = term_signal {
            self.shell_state.job_table.note_term_signal(pid, signal);
        }
        // rubash#229: a background child killed by a signal gets the
        // jobs.c:4625 notify_of_job_status notice at the reader
        // boundary; the command text is read from the job entry (a job
        // disowned while its completion was deferred has no entry
        // left and gets no notice, as in GNU).
        if status > 128 {
            if let Some(command) = self
                .shell_state
                .job_table
                .pid_to_job
                .get(&pid)
                .and_then(|job_id| self.shell_state.job_table.jobs.get(job_id))
                .map(|job| job.command.clone())
            {
                self.note_child_signal_death(pid, status - 128, command, false);
            }
        }
        self.shell_state.job_table.mark_completed(pid, status);
        self.flush_timed_coproc_report(pid);
        self.run_sigchld_trap_for_reaped_child()?;
        self.retire_completed_coproc(pid);
        Ok(())
    }

    fn join_coproc_stderr_forwarder(&mut self, pid: u32) -> Result<(), ExecuteError> {
        let Some(forwarder) = self.coproc_stderr_forwarders.remove(&pid) else {
            return Ok(());
        };
        let result = forwarder.join().map_err(|_| {
            ExecuteError::IoError(std::io::Error::other("coprocess stderr forwarder panicked"))
        })?;
        result.map_err(ExecuteError::IoError)
    }

    /// GNU variables.c:3959 makunbound's cell-existence test, read as
    /// "find_variable returned a SHELL_VAR": a value in either store, or
    /// any attribute bits (a valueless `declare -r x` cell still counts).
    /// Local-scope cells are live in the env_vars mirror while their scope
    /// is active.
    fn variable_cell_exists(&self, name: &str) -> bool {
        self.shell_state.env_vars.contains_key(name)
            || self.shell_state.variables.get(name).is_some()
            || !self.shell_state.env_vars.attr_of(name).is_empty()
    }

    /// GNU execute_cmd.c:2450 coproc_unsetvars: unbind_variable_noref
    /// removes <c_name>_PID literally (no nameref follow, no readonly gate),
    /// then check_unbind_variable resolves c_name through namerefs and
    /// refuses readonly targets with `name: cannot unset: readonly
    /// variable`. Runs for every coproc name recorded at spawn, including
    /// names whose coproc_bind failed (invalid identifier, readonly).
    /// Each makunbound drops the whole SHELL_VAR — mirror entry, typed
    /// cell, every attribute bit (dispose_variable, variables.c:3973-4019)
    /// — and the export-environment copy with it (variables.c:3839);
    /// leaving the inherited process-env name behind lets the table-miss
    /// fallbacks resurrect it (the #943 residue class, #971).
    fn coproc_unset_vars(&mut self, pid: u32) {
        let Some(name) = self.shell_state.coproc_names.remove(&pid) else {
            return;
        };
        let pid_name = format!("{name}_PID");
        self.shell_state.env_vars.remove(&pid_name);
        self.shell_state.variables.remove(&pid_name);
        crate::executor::local_helpers::unset_process_env(&pid_name);
        self.shell_state.env_vars.remove_unset_marks(&pid_name);
        // The __RUBASH_COPROC_{STDIN,STDOUT}_<pid> bookkeeping keys written
        // at bind are this coproc's fd-channel state — same drop rule.
        for key in [
            format!("__RUBASH_COPROC_STDIN_{pid}"),
            format!("__RUBASH_COPROC_STDOUT_{pid}"),
        ] {
            self.shell_state.env_vars.remove(&key);
            crate::executor::local_helpers::unset_process_env(&key);
        }
        let unbind_name = self
            .resolved_variable_name(&name)
            .unwrap_or_else(|| name.clone());
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &unbind_name) {
            eprintln!(
                "{}{}: cannot unset: readonly variable",
                self.diagnostic_prefix(),
                unbind_name
            );
            return;
        }
        self.shell_state.env_vars.remove(&unbind_name);
        self.shell_state.variables.remove(&unbind_name);
        crate::executor::local_helpers::unset_process_env(&unbind_name);
        self.shell_state.env_vars.remove_unset_marks(&unbind_name);
    }

    fn retire_completed_coproc(&mut self, pid: u32) {
        let is_coproc = self.fd_table.entries.values().any(|entry| {
            matches!(
                entry.read.as_ref(),
                Some(FdReadEndpoint::CoprocStdout { pid: endpoint_pid, .. }) if *endpoint_pid == pid
            ) || matches!(
                entry.write.as_ref(),
                Some(FdWriteEndpoint::CoprocStdin { pid: endpoint_pid, .. }) if *endpoint_pid == pid
            )
        });
        if !is_coproc {
            return;
        }

        self.close_coproc_endpoints(pid);

        let endpoint_fds = self
            .fd_table
            .entries
            .iter()
            .filter_map(|(fd, entry)| {
                let matches_read = matches!(
                    entry.read.as_ref(),
                    Some(FdReadEndpoint::CoprocStdout { pid: endpoint_pid, .. }) if *endpoint_pid == pid
                );
                let matches_write = matches!(
                    entry.write.as_ref(),
                    Some(FdWriteEndpoint::CoprocStdin { pid: endpoint_pid, .. }) if *endpoint_pid == pid
                );
                (matches_read || matches_write).then_some(*fd)
            })
            .collect::<Vec<_>>();
        for fd in endpoint_fds {
            self.fd_table.close(fd);
            // The fd bookkeeping keys are __RUBASH_-prefixed, so a parent
            // mirror write can leave an inherited process-env copy behind;
            // drop it with the table entry (#971).
            for key in [
                fd_stdin_key(fd),
                fd_stdin_offset_key(fd),
                fd_dynamic_input_key(fd),
                fd_output_key(fd),
                fd_output_process_substitution_key(fd),
            ] {
                self.shell_state.env_vars.remove(&key);
                crate::executor::local_helpers::unset_process_env(&key);
            }
            self.shell_state
                .env_vars
                .insert(fd_closed_key(fd), "1".to_string());
        }
        self.coproc_unset_vars(pid);

        let coproc_prefix = format!("{FD_COPROC_STDIN_TARGET_PREFIX}{pid}");
        self.shell_state.env_vars.retain(|key, value| {
            let dropped = (key.starts_with(FD_STDIN_PREFIX) || key.starts_with(FD_OUTPUT_PREFIX))
                && value == &coproc_prefix;
            if dropped {
                crate::executor::local_helpers::unset_process_env(key);
            }
            !dropped
        });
    }

    fn forget_background_runtime(&mut self, pid: u32) {
        // rubash#1022: wait paths land here after consuming a status — reap
        // a placeholder's zombie without blocking (it is already dead; the
        // WNOHANG reap only drops the kernel shell).
        self.reap_procsub_placeholder(pid, false);
        self.background_children.remove(&pid);
        // rubash#169: a background child spawned under a capture scope holds
        // a duplicate of the scope's capture pipe; reaping it (wait paths)
        // must retire it from the writer set and, when it was the last one,
        // fold whatever it wrote into the capture buffer now — the same
        // point GNU's kernel pipe delivered those bytes. Field capture
        // (substitution stdout) is the byte-order authority; a stage-local
        // thread capture takes them otherwise.
        let drained = crate::executor::shell_options::retire_background_capture_writer(pid);
        if !drained.is_empty() {
            // Byte-order authority is wherever in-process writes land
            // (write_fd_endpoint's Stdout arm): thread-local capture first,
            // executor field capture second.
            if crate::executor::shell_options::stdout_capture_active() {
                let _ = crate::executor::shell_options::stdout_capture_write(&drained);
            } else if let Some(buffer) = self.stdout_capture.as_mut() {
                buffer.extend_from_slice(&drained);
            }
        }
        // Close the coproc endpoint fds for this pid before dropping the
        // pipe maps, mirroring retire_completed_coproc's fd cleanup so the
        // high fds 63/60 become reusable for the next coproc (coproc.tests
        // expects 63 60 for each of the three coprocs after wait).
        let endpoint_fds = self
            .fd_table
            .entries
            .iter()
            .filter_map(|(fd, entry)| {
                let matches_read = matches!(
                    entry.read.as_ref(),
                    Some(FdReadEndpoint::CoprocStdout { pid: endpoint_pid, .. }) if *endpoint_pid == pid
                );
                let matches_write = matches!(
                    entry.write.as_ref(),
                    Some(FdWriteEndpoint::CoprocStdin { pid: endpoint_pid, .. }) if *endpoint_pid == pid
                );
                (matches_read || matches_write).then_some(*fd)
            })
            .collect::<Vec<_>>();
        for fd in endpoint_fds {
            self.fd_table.close(fd);
        }
        self.close_coproc_endpoints(pid);
        self.fd_table.close(pid);
        // GNU reaps dead coprocs through wait_for too; coproc_unsetvars runs
        // there, not only on the background-refresh path.
        self.coproc_unset_vars(pid);
    }

    fn wait_for_background_pid(
        &mut self,
        pid: u32,
        _retain_for_explicit_wait: bool,
    ) -> Result<WaitPidOutcome, ExecuteError> {
        // GNU trap.c:604-611: a trapped signal arriving while the wait
        // builtin blocks longjmps out of the wait FROM THE SIGNAL HANDLER --
        // the signal wins over the child's exit even when both are pending
        // (the killer child sends before it exits, so the delivery always
        // precedes the exit in wall time). Poll-loop equivalent: drain the
        // pending queue BEFORE consulting the completed-status cache and
        // before try_wait, so the signal's earlier arrival is observed
        // first (rubash#228).
        if let Some(interrupt) = self.wait_pending_signal_status()? {
            return Ok(WaitPidOutcome::Interrupted(interrupt));
        }
        // GNU jobs.c wait_for_background_pids (bare wait) and
        // wait_for_job (explicit wait): a job the shell already knows
        // is stopped is never blocked on — a stopped child never exits,
        // so the poll loop below would hang forever (rubash#1246,
        // verified GNU 5.3.20: bare wait warns and returns 0, an
        // explicit wait warns and returns 128+stopsig). The caller
        // renders the form-specific warning.
        if let Some((job_id, stop_signal)) = self
            .shell_state
            .job_table
            .pid_to_job
            .get(&pid)
            .and_then(|job_id| self.shell_state.job_table.jobs.get(job_id))
            .filter(|job| job.state == crate::jobs::ProcessState::Stopped)
            .map(|job| (job.id, job.stop_signal.unwrap_or(19)))
        {
            return Ok(WaitPidOutcome::Stopped {
                job_id,
                stop_signal,
            });
        }
        if let Some(status) = self
            .shell_state
            .job_table
            .completed_statuses
            .get(&pid)
            .copied()
        {
            self.join_coproc_stderr_forwarder(pid)?;
            // rubash#1022: reap the fork placeholder (born zombie, so the
            // blocking waitpid returns at once) — a later `kill -0` then
            // reports ESRCH like GNU's reaped procsub. The recorded status
            // stays in completed_statuses for repeat waits (bgpids parity).
            self.reap_procsub_placeholder(pid, true);
            // Waiting consumes the jobs-table entry, but the completed status
            // remains available for a later explicit wait of the same PID.
            self.shell_state
                .job_table
                .remove_job_by_pid_preserve_status(pid);
            self.forget_background_runtime(pid);
            return Ok(WaitPidOutcome::Status(status));
        }
        let Some(mut child) = self.background_children.remove(&pid) else {
            return Ok(WaitPidOutcome::NotFound);
        };
        // GNU jobs.c:3064 wait_for + wait.def:178-191 wait_intr_flag: the
        // blocking waitpid is interruptible — a trapped signal longjmps out
        // and wait returns 128+sig while the job stays live. The mailbox
        // emulation cannot interrupt WaitForSingleObject, so poll the child
        // and drain pending signals between polls; on interruption put the
        // handle back so the job remains waitable and reportable.
        let status = loop {
            // Signal queue first (see trap.c:604-611 note above): a trap
            // delivered before the child exited must win the race.
            match self.wait_pending_signal_status() {
                Ok(Some(interrupt)) => {
                    self.background_children.insert(pid, child);
                    return Ok(WaitPidOutcome::Interrupted(interrupt));
                }
                Ok(None) => {}
                Err(error) => {
                    // An untrapped signal aborts the wait with
                    // ExitCode(128+sig): put the handle back before
                    // propagating, like the trapped branch above, so
                    // the job remains waitable and reportable — the
                    // child may still be alive.
                    self.background_children.insert(pid, child);
                    return Err(error);
                }
            }
            if let Some(done) = child.try_wait()? {
                // rubash#1776: sample the reaped child's CPU for the
                // Windows CHILDREN half while its handle is live.
                super::read_helpers::note_reaped_child_cpu(&child);
                // rubash#229: queue the signal-death notice (the job entry
                // still exists here, so its command text is available).
                if let Some(signal) = crate::executor::wait_status::exit_status_signal(&done) {
                    let command = self
                        .shell_state
                        .job_table
                        .pid_to_job
                        .get(&pid)
                        .and_then(|job_id| self.shell_state.job_table.jobs.get(job_id))
                        .map(|job| job.command.clone())
                        .unwrap_or_default();
                    self.note_child_signal_death(pid, signal, command, false);
                }
                break crate::executor::wait_status::process_exit_status(&done);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        self.join_coproc_stderr_forwarder(pid)?;
        self.shell_state.job_table.mark_completed(pid, status);
        self.flush_timed_coproc_report(pid);
        self.run_sigchld_trap_for_reaped_child()?;
        // Remove the visible job after any wait, while retaining the exit
        // status for repeated explicit PID waits.
        self.shell_state
            .job_table
            .remove_job_by_pid_preserve_status(pid);
        self.forget_background_runtime(pid);
        Ok(WaitPidOutcome::Status(status))
    }

    /// GNU wait.def:193-206 + :328-331: drain this process's pending-signal
    /// mailbox while waiting. An untrapped non-SIGCHLD signal aborts the
    /// wait with 128+sig (ExecuteError::ExitCode, classified discard vs
    /// exit exactly like the boundary dispatch's untrapped branch —
    /// rubash#1825); a trapped signal makes `wait` return 128+sig
    /// immediately — SIGCHLD is exempt outside posix mode
    /// (wait.def:195-199). Everything drained on the non-aborting paths
    /// is re-queued so the command boundary's run_pending_signal_traps
    /// still runs the trap actions after wait returns ("the trap
    /// associated with that signal shall be taken").
    fn wait_pending_signal_status(&mut self) -> Result<Option<i32>, ExecuteError> {
        let signals =
            crate::builtins::kill::take_pending_signals_now(std::process::id()).unwrap_or_default();
        if signals.is_empty() {
            return Ok(None);
        }
        let posixly =
            crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "posix");
        let mut interrupt = None;
        for &signal in &signals {
            // SIGCHLD only breaks out of `wait` under posixly_correct.
            // SIGCHLD follows the target numbering (17 Linux / 20 Darwin)
            // via kill::SIGCHLD_NUMBER; the non-unix mailbox wire format
            // keeps Linux numbering by contract.
            if signal == crate::builtins::kill::SIGCHLD_NUMBER && !posixly {
                continue;
            }
            let Some(name) = super::trap_exec::signal_trap_name(signal) else {
                continue;
            };
            match crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, &name) {
                Some(action) if !action.is_empty() => {
                    interrupt = Some(signal);
                    break;
                }
                // Explicitly ignored (trap '' USR1) or SIGCHLD under posix:
                // not fatal, not interrupting.
                Some(_) => continue,
                None => {
                    // rubash#1825: the same classification the boundary
                    // dispatch's untrapped branch makes
                    // (run_pending_signal_traps): record the status and,
                    // in an interactive shell, mark the unit for
                    // DISCARD — GNU jobs.c:3315-3316 (ADDINTERRUPT)
                    // throws the interrupted wait to the top level as a
                    // discard, so the session survives with $?=128+sig
                    // instead of ending. A bare ExitCode here was read
                    // by the source driver as an exit jump and killed
                    // the whole interactive session at the wait.
                    // Non-interactive shells keep the exit
                    // classification, and the drained signals are not
                    // re-queued on this path: this delivery IS the
                    // wait's verdict.
                    self.exit_code = 128 + signal;
                    if self
                        .shell_state
                        .env_vars
                        .get("__RUBASH_INTERACTIVE")
                        .map(String::as_str)
                        == Some("1")
                    {
                        self.signal_discard_pending = true;
                    }
                    return Err(ExecuteError::ExitCode(128 + signal));
                } // signals delivered but never named (signal_trap_name
                  // gives None for out-of-range) are dropped with the rest.
            }
        }
        crate::builtins::kill::requeue_pending_signals(signals);
        Ok(interrupt.map(|signal| 128 + signal))
    }

    fn background_jobs_output(
        &mut self,
        options: crate::builtins::jobs::JobsListOptions,
        requested_jobs: &[String],
        stderr: &mut Vec<u8>,
    ) -> Result<(String, i32), ExecuteError> {
        let jobs = if requested_jobs.is_empty() {
            // GNU builtins/jobs.def + jobs.c list_*_jobs: every listing form
            // calls cleanup_dead_jobs first, so dead jobs already notified
            // are gone before a line is printed.
            self.shell_state.job_table.cleanup_dead_jobs();
            self.ordered_background_jobs()
        } else {
            let mut selected = Vec::new();
            // GNU jobs.def jobs_builtin: each operand goes through
            // get_job_spec; a bad spec is sh_badjob'd and counted in
            // any_failed, and the builtin returns
            // `any_failed ? EXECUTION_FAILURE : EXECUTION_SUCCESS`
            // (jobs.def:174). The rubash#549 leniency is only about the
            // MESSAGE/resolution (a bare operand warns about the missing
            // `%' and is then resolved as if it had one), not the status:
            // GNU 5.3.20 probes give rc=1 for `jobs %99`, `jobs 99`,
            // `jobs foo` and `jobs %99 %1`, rc=0 when every operand
            // resolves (rubash#1123).
            let mut any_failed = false;
            for job in requested_jobs {
                let (spec, report) = if job.starts_with('%') {
                    (job.clone(), job.clone())
                } else {
                    writeln!(
                        stderr,
                        "{}jobs: warning: {job}: job specification requires leading `%'",
                        self.diagnostic_prefix()
                    )?;
                    (format!("%{job}"), job.clone())
                };
                match self.resolve_background_job_operand(&spec) {
                    JobOperandResolution::Pid(pid) => {
                        if let Some(job_id) =
                            self.shell_state.job_table.pid_to_job.get(&pid).copied()
                        {
                            if let Some(entry) = self.shell_state.job_table.jobs.get(&job_id) {
                                selected.push((
                                    self.background_job_number(pid),
                                    pid,
                                    entry.command.clone(),
                                ));
                            }
                        }
                    }
                    JobOperandResolution::Ambiguous => {
                        // GNU jobs.def:158-164: DUP_JOB is INVALID_JOB —
                        // the get_job_by_name diagnostic prints, then
                        // sh_badjob reports the spec itself.
                        any_failed = true;
                        writeln!(
                            stderr,
                            "{}jobs: {}: ambiguous job spec",
                            self.diagnostic_prefix(),
                            jobspec_match_name(&spec)
                        )?;
                        writeln!(
                            stderr,
                            "{}jobs: {report}: no such job",
                            self.diagnostic_prefix()
                        )?;
                    }
                    JobOperandResolution::NoMatch => {
                        any_failed = true;
                        writeln!(
                            stderr,
                            "{}jobs: {report}: no such job",
                            self.diagnostic_prefix()
                        )?;
                    }
                }
            }
            let output = self.render_background_jobs(options, selected);
            // jobs.c:2231 list_one_job: explicit jobspec listing is followed
            // by cleanup_dead_jobs — a listed dead job leaves the table.
            self.shell_state.job_table.cleanup_dead_jobs();
            return Ok((output, i32::from(any_failed)));
        };
        let output = self.render_background_jobs(options, jobs);
        // jobs.def:137-141 JSTATE_ANY (plain `jobs`, `-l`, `-p`, `-n`): after
        // the listing, notify_and_cleanup removes the terminated jobs just
        // reported — the next `jobs`/`disown` must not see them. `-r`/`-s`
        // (JSTATE_RUNNING/JSTATE_STOPPED) skip this second cleanup.
        if !options.running_only && !options.stopped_only {
            self.shell_state.job_table.cleanup_dead_jobs();
        }
        Ok((output, 0))
    }

    fn ordered_background_jobs(&self) -> Vec<(usize, u32, String)> {
        // GNU jobs.c: the printed job number is the job's slot index
        // (alloc_job_entry), stable across removals — holes stay open, so
        // with jobs %1 and %3 the list shows `[1] [3]`, never a renumbered
        // `[1] [2]`.
        self.shell_state
            .job_table
            .jobs
            .values()
            .filter(|job| job.background)
            .filter_map(|job| {
                job.pids
                    .last()
                    .copied()
                    .map(|pid| (job.id as usize, pid, job.command.clone()))
            })
            .collect()
    }

    fn render_background_jobs(
        &mut self,
        options: crate::builtins::jobs::JobsListOptions,
        jobs: Vec<(usize, u32, String)>,
    ) -> String {
        let mut output = String::new();
        for (job_number, pid, source) in jobs {
            let state_text_opt = self
                .shell_state
                .job_table
                .pid_to_job
                .get(&pid)
                .and_then(|job_id| self.shell_state.job_table.jobs.get(job_id))
                .map(|job| match job.state {
                    crate::jobs::ProcessState::Running => "Running".to_string(),
                    crate::jobs::ProcessState::Stopped => "Stopped".to_string(),
                    // GNU jobs.c pretty_print_job: a job killed by a
                    // signal lists the signal description (Terminated,
                    // Killed, ...), not the flattened 128+signal status
                    // (rubash#1256).
                    crate::jobs::ProcessState::Completed => match job.term_signal {
                        Some(signal) => crate::builtins::kill::signal_death_description(signal),
                        None => match job.exit_status.unwrap_or(1) {
                            0 => "Done".to_string(),
                            status => format!("Exit {status}"),
                        },
                    },
                });
            // Apply state-based filters
            if options.running_only {
                if !state_text_opt.as_ref().map_or(false, |s| s == "Running") {
                    continue;
                }
            }
            if options.stopped_only {
                if !state_text_opt.as_ref().map_or(false, |s| s == "Stopped") {
                    continue;
                }
            }
            let job_id = self.shell_state.job_table.pid_to_job.get(&pid).copied();
            // GNU jobs.c J_NOTIFIED is a per-job flag, not a positional
            // index — job numbers shift as earlier jobs are removed.
            if options.changed_only
                && job_id
                    .and_then(|id| self.shell_state.job_table.jobs.get(&id))
                    .is_some_and(|entry| entry.notified)
            {
                continue;
            }
            let state_text = state_text_opt.unwrap_or_else(|| "Unknown".to_string());

            // GNU jobs.c:1969 printable_job_status: in the LONG format a
            // stopped job renders the stop signal's strsignal text —
            // SIGSTOP `Stopped (signal)`, SIGTTIN `Stopped (tty input)`,
            // SIGTTOU `Stopped (tty output)` (SIGTSTP's description is
            // plain `Stopped`). The short format and the notification
            // lines always use the plain state text (rubash#1246,
            // verified against GNU 5.3.20 in a pty).
            let display_text = if options.long && state_text == "Stopped" {
                let stop_signal = job_id
                    .and_then(|id| self.shell_state.job_table.jobs.get(&id))
                    .and_then(|job| job.stop_signal);
                stopped_long_text(stop_signal).unwrap_or(&state_text)
            } else {
                &state_text
            };

            // GNU jobs.c:2118-2120 pretty_print_pipeline: the trailing ` &`
            // is printed only while the job is RUNNING and not foreground —
            // the stored command text never contains it.
            let job_background = job_id
                .and_then(|id| self.shell_state.job_table.jobs.get(&id))
                .is_some_and(|entry| entry.background);
            let async_suffix = if state_text == "Running" && job_background {
                " &"
            } else {
                ""
            };

            // GNU jobs.c:2207 pretty_print_job: the job flag column is `+`
            // for the current job, `-` for the previous job, and a space
            // otherwise; a second space follows for the standard format and
            // the state field pads to LONGEST_SIGNAL_DESC (27, jobs.h:43).
            let marker = match self.shell_state.job_table.pid_to_job.get(&pid) {
                Some(job_id) if self.shell_state.job_table.current_job() == Some(*job_id) => '+',
                Some(job_id) if self.shell_state.job_table.previous_job() == Some(*job_id) => '-',
                _ => ' ',
            };

            if options.pids_only {
                output.push_str(&format!(
                    "{pid}
"
                ));
            } else if options.long {
                // jobs.c:2207 pretty_print_job prints `[%d]%c ` (one space
                // after the flag), then jobs.c:2065 print_pipeline formats
                // the pid with `%5ld` followed by one space — a 3-digit pid
                // gets `[1]-   492` (flag + 4 spaces), a 5-digit pid
                // `[1]+ 17063` (rubash#233: the old `{marker}  {pid}` shape
                // printed a fixed two spaces with a left-aligned pid).
                output.push_str(&format!(
                    "[{job_number}]{marker} {pid:>5} {display_text:<27}{source}{async_suffix}\n"
                ));
            } else {
                output.push_str(&format!(
                    "[{job_number}]{marker}  {state_text:<27}{source}{async_suffix}
"
                ));
            }
            // jobs.c:2219 pretty_print_job — printing a job's status IS
            // the notification, for every listing mode (plain `jobs`,
            // `-l`, `-n`); the flag is cleared on the next state change.
            if let Some(entry) = job_id.and_then(|id| self.shell_state.job_table.jobs.get_mut(&id))
            {
                entry.notified = true;
            }
        }
        output
    }

    pub(in crate::executor) fn execute_disown(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let action = crate::builtins::disown::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        let status = match action {
            crate::builtins::disown::DisownAction::Complete(status) => status,
            crate::builtins::disown::DisownAction::All { nohup_only } => {
                if nohup_only {
                    // GNU jobs.c:5111 nohup_all_jobs: mark, never delete.
                    for job in self.shell_state.job_table.jobs.values_mut() {
                        job.nohup = true;
                    }
                    0
                } else {
                    let pids: Vec<u32> = self
                        .shell_state
                        .job_table
                        .jobs
                        .values()
                        .flat_map(|job| job.pids.iter().copied())
                        .chain(self.background_children.keys().copied())
                        .collect();
                    self.background_children.clear();
                    for pid in &pids {
                        // rubash#1304: `disown -a` leaves nothing
                        // waitable (GNU: a later `wait <pid>` reports
                        // "not a child of this shell", 127) — unlike a
                        // single disown, no status is preserved here.
                        self.close_coproc_endpoints(*pid);
                        self.fd_table.close(*pid);
                    }
                    self.shell_state.job_table.clear_jobs();
                    0
                }
            }
            crate::builtins::disown::DisownAction::Current { nohup_only } => {
                let acted = if nohup_only {
                    self.nohup_current_job()
                } else {
                    self.disown_current_job()
                };
                if acted {
                    0
                } else {
                    writeln!(
                        stderr,
                        "{}disown: current: no such job",
                        self.diagnostic_prefix()
                    )?;
                    1
                }
            }
            crate::builtins::disown::DisownAction::Jobs { specs, nohup_only } => {
                let mut status = 0;
                for job in specs {
                    match self.resolve_disown_job_operand(&job) {
                        JobOperandResolution::Ambiguous => {
                            // GNU jobs.def:287-291: DUP_JOB takes the
                            // INVALID_JOB arm — the get_job_by_name
                            // diagnostic prints, then sh_badjob too.
                            writeln!(
                                stderr,
                                "{}disown: {}: ambiguous job spec",
                                self.diagnostic_prefix(),
                                jobspec_match_name(&job)
                            )?;
                            writeln!(
                                stderr,
                                "{}disown: {job}: no such job",
                                self.diagnostic_prefix()
                            )?;
                            status = 1;
                        }
                        JobOperandResolution::NoMatch => {
                            // GNU jobs.def:283-289 disown_builtin: non-numeric
                            // operands go through get_job_spec, which warns when
                            // the spec lacks a leading `%` (common.c:705-710)
                            // before failing with "no such job".
                            if !job.starts_with('%') && job.parse::<u32>().is_err() {
                                writeln!(
                                    stderr,
                                    "{}disown: warning: {job}: job specification requires leading `%'",
                                    self.diagnostic_prefix()
                                )?;
                            }
                            writeln!(
                                stderr,
                                "{}disown: {job}: no such job",
                                self.diagnostic_prefix()
                            )?;
                            status = 1;
                        }
                        JobOperandResolution::Pid(pid) => {
                            if nohup_only {
                                // GNU jobs.c:1548 nohup_job: J_NOHUP mark
                                // only — the job stays in the table, so
                                // `jobs` still lists it and `wait %N` still
                                // finds it (rubash#1048).
                                if let Some(job_id) =
                                    self.shell_state.job_table.pid_to_job.get(&pid).copied()
                                {
                                    if let Some(entry) =
                                        self.shell_state.job_table.jobs.get_mut(&job_id)
                                    {
                                        entry.nohup = true;
                                    }
                                }
                            } else {
                                self.background_children.remove(&pid);
                                self.close_coproc_endpoints(pid);
                                self.fd_table.close(pid);
                                self.disown_job_by_pid(pid);
                            }
                        }
                    }
                }
                status
            }
        };
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    /// `disown -h` with no jobspec: mark the current job J_NOHUP
    /// (jobs.c nohup_job via get_job_spec(NULL)); the job stays live in
    /// the table (rubash#1048).
    fn nohup_current_job(&mut self) -> bool {
        let Some(job_id) = self.shell_state.job_table.current_job() else {
            return false;
        };
        match self.shell_state.job_table.jobs.get_mut(&job_id) {
            Some(entry) => {
                entry.nohup = true;
                true
            }
            None => false,
        }
    }

    fn disown_current_job(&mut self) -> bool {
        let Some(pid) = self.shell_state.last_background_pid else {
            return false;
        };
        if !self.background_children.contains_key(&pid)
            && !self.shell_state.job_table.pid_to_job.contains_key(&pid)
        {
            return false;
        }

        self.background_children.remove(&pid);
        self.close_coproc_endpoints(pid);
        self.fd_table.close(pid);
        self.disown_job_by_pid(pid);
        true
    }

    /// GNU jobs.c delete_job as driven by disown_builtin, with bgpids
    /// (jobs.c bgp_search) semantics for the survivors: the job leaves
    /// the table, but its pids stay waitable. A status already recorded
    /// for a terminated job survives the removal, and a still-running
    /// job's pids read as already-waited — `wait <pid>` after disown
    /// returns that status or 0, silently and repeatably, instead of
    /// 127 "not a child of this shell" (rubash#1304). (DisownAction::All
    /// is the exception: GNU `disown -a` leaves nothing waitable, which
    /// clear_jobs' purge of completed_statuses already models.)
    fn disown_job_by_pid(&mut self, pid: u32) {
        let job_pids: Vec<u32> = self
            .shell_state
            .job_table
            .pid_to_job
            .get(&pid)
            .and_then(|job_id| self.shell_state.job_table.jobs.get(job_id))
            .map(|job| job.pids.clone())
            .unwrap_or_else(|| vec![pid]);
        self.shell_state
            .job_table
            .remove_job_by_pid_preserve_status(pid);
        for p in job_pids {
            self.shell_state
                .job_table
                .completed_statuses
                .entry(p)
                .or_insert(0);
        }
    }

    /// GNU wait.def:294-311 / kill.def:226 / fg_bg.def:143 — the jobspec
    /// operand funnels through get_job_spec; `Ambiguous` (DUP_JOB)
    /// keeps its identity so callers can print
    /// `{name}: ambiguous job spec` rather than "no such job".
    pub(in crate::executor) fn resolve_background_job_operand(
        &self,
        job: &str,
    ) -> JobOperandResolution {
        if job.starts_with('%') {
            return match self.shell_state.job_table.resolve_jobspec(job) {
                crate::jobs::table::JobspecMatch::Job(job_id) => self
                    .shell_state
                    .job_table
                    .jobs
                    .get(&job_id)
                    .and_then(|entry| entry.pids.last().copied())
                    .map_or(JobOperandResolution::NoMatch, JobOperandResolution::Pid),
                crate::jobs::table::JobspecMatch::Ambiguous => JobOperandResolution::Ambiguous,
                crate::jobs::table::JobspecMatch::NoMatch => JobOperandResolution::NoMatch,
            };
        }
        let Some(pid) = job.parse::<u32>().ok() else {
            return JobOperandResolution::NoMatch;
        };
        // rubash#1022: pending procsub placeholders (a still-open persistent
        // `>(...)` — no recorded status yet) resolve like completed ones so
        // `kill -0`/`kill` reach the forked child; `wait` on one still
        // reports "not a child" (127) instead of blocking forever.
        if self.shell_state.job_table.pid_to_job.contains_key(&pid)
            || self
                .shell_state
                .job_table
                .completed_statuses
                .contains_key(&pid)
            || self
                .shell_state
                .job_table
                .procsub_placeholders
                .contains(&pid)
        {
            JobOperandResolution::Pid(pid)
        } else {
            JobOperandResolution::NoMatch
        }
    }

    pub(in crate::executor) fn resolve_background_job(&self, job: &str) -> Option<u32> {
        if job.starts_with('%') {
            return match self.resolve_background_job_operand(job) {
                JobOperandResolution::Pid(pid) => Some(pid),
                JobOperandResolution::Ambiguous | JobOperandResolution::NoMatch => None,
            };
        }
        match self.resolve_background_job_operand(job) {
            JobOperandResolution::Pid(pid) => Some(pid),
            JobOperandResolution::Ambiguous | JobOperandResolution::NoMatch => None,
        }
    }

    /// GNU jobs.def:283-289 disown_builtin: a numeric operand resolves by
    /// PID through get_job_by_pid -> jobs.c:1916 -> jobs.c:1878 find_job,
    /// which scans the LIVE jobs table only — bgpids (saved exit statuses)
    /// are NOT consulted. A dead job that a `jobs` listing already removed
    /// (POSIX cleanup, jobs.def:138-139 notify_and_cleanup) or a `wait`
    /// consumed (jobs.c:3438 wait_for_job J_NOTIFIED + delete) is therefore
    /// "no such job" for disown (rc 1). Contrast wait_for_single_pid
    /// (jobs.c:2741), which DOES fall back to bgp_search — the wait path
    /// keeps resolve_background_job's completed_statuses arm. The
    /// DUP_JOB (ambiguous) arm is kept too — jobs.def:287-291 reports
    /// the ambiguous diagnostic AND `sh_badjob` for it.
    fn resolve_disown_job_operand(&self, job: &str) -> JobOperandResolution {
        if job.starts_with('%') {
            return self.resolve_background_job_operand(job);
        }
        let Some(pid) = job.parse::<u32>().ok() else {
            return JobOperandResolution::NoMatch;
        };
        if self.shell_state.job_table.pid_to_job.contains_key(&pid) {
            JobOperandResolution::Pid(pid)
        } else {
            JobOperandResolution::NoMatch
        }
    }

    fn background_job_number(&self, pid: u32) -> usize {
        // Same slot-index numbering as ordered_background_jobs — the
        // printed `%N` must resolve to the same job `wait %N` sees.
        self.shell_state
            .job_table
            .pid_to_job
            .get(&pid)
            .map(|job_id| *job_id as usize)
            .unwrap_or(1)
    }

    pub(in crate::executor) fn execute_fg_bg(
        &mut self,
        cmd: &CommandNode,
        builtin: crate::builtins::fg_bg::JobControlBuiltin,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let action = crate::builtins::fg_bg::execute_with_io(
            builtin,
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        // GNU fg_bg.def:108-113 — `job_control == 0` (the monitor option,
        // off by default in non-interactive shells) reports "no job
        // control" before any operand processing, regardless of whether
        // background jobs exist. A background job table entry alone does
        // not imply job control.
        let has_job_control =
            crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor");
        let status = match action {
            // Bash reports the non-interactive job-control failure before
            // validating fg/bg operands or options when no jobs exist.
            crate::builtins::fg_bg::FgBgAction::Complete(_status) if !has_job_control => {
                stderr.clear();
                crate::builtins::fg_bg::write_no_job_control(
                    builtin,
                    &self.diagnostic_prefix(),
                    &mut stderr,
                )?
            }
            crate::builtins::fg_bg::FgBgAction::Complete(status) => status,
            crate::builtins::fg_bg::FgBgAction::Jobs(jobs) => {
                if !has_job_control {
                    crate::builtins::fg_bg::write_no_job_control(
                        builtin,
                        &self.diagnostic_prefix(),
                        &mut stderr,
                    )?
                } else {
                    match builtin {
                        crate::builtins::fg_bg::JobControlBuiltin::Fg => {
                            self.execute_fg_jobs(jobs, &mut stdout, &mut stderr)?
                        }
                        crate::builtins::fg_bg::JobControlBuiltin::Bg => {
                            self.execute_bg_jobs(jobs, &mut stdout, &mut stderr)?
                        }
                    }
                }
            }
        };
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    fn execute_fg_jobs(
        &mut self,
        jobs: Vec<String>,
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
    ) -> Result<i32, ExecuteError> {
        // fg blocks in wait_for for its job: an observation point, so
        // deferred reap state applies before the operand resolves.
        self.refresh_background_jobs_observed()?;
        let job = jobs.first().map(String::as_str);
        let pid = match self.resolve_fg_bg_operand("fg", job, stderr)? {
            JobOperandResolution::Pid(pid) => pid,
            JobOperandResolution::Ambiguous => {
                // GNU fg_bg.def:144-152: DUP_JOB skips sh_badjob — the
                // get_job_by_name diagnostic is the whole report.
                writeln!(
                    stderr,
                    "{}fg: {}: ambiguous job spec",
                    self.diagnostic_prefix(),
                    job.map_or("current", jobspec_match_name)
                )?;
                return Ok(1);
            }
            JobOperandResolution::NoMatch => {
                self.write_job_not_found("fg", job, stderr)?;
                return Ok(1);
            }
        };

        // GNU fg_bg.def:154-160 — a job started without job control
        // (J_JOBCONTROL unset, e.g. spawned before `set -m`) cannot be
        // foregrounded; refuse instead of waiting on it.
        if !self.shell_state.job_table.job_control_for_pid(pid) {
            let job_id = self.shell_state.job_table.job_id_for_pid(pid).unwrap_or(0);
            writeln!(
                stderr,
                "{}fg: job {} started without job control",
                self.diagnostic_prefix(),
                job_id
            )?;
            return Ok(1);
        }
        let Some(job_id) = self.shell_state.job_table.job_id_for_pid(pid) else {
            self.write_job_not_found("fg", job, stderr)?;
            return Ok(1);
        };
        // GNU jobs.c:3837-3843 start_job: a command substitution child
        // sharing the shell's process group cannot take terminal control —
        // fg/bg inside `$( )` reports "no current jobs". This gate precedes
        // the DEADJOB/live-child checks, just like GNU.
        if self.shell_state.in_command_substitution.get() {
            writeln!(stderr, "{}fg: no current jobs", self.diagnostic_prefix())?;
            return Ok(1);
        }

        if !self.background_children.contains_key(&pid)
            && !self
                .shell_state
                .job_table
                .completed_statuses
                .contains_key(&pid)
        {
            self.write_job_not_found("fg", job, stderr)?;
            return Ok(1);
        }

        // GNU jobs.c:3826 start_job(foreground=1): the job becomes current
        // and foreground, its command text is printed (jobs.c:3880-3895),
        // SIGCONT resumes a stopped process group (jobs.c:3926-3928), then
        // wait_for blocks — interruptibly — on the last pid.
        self.shell_state.job_table.set_current_job(job_id);
        let (command, pids) = self
            .shell_state
            .job_table
            .jobs
            .get(&job_id)
            .map(|entry| (entry.command.clone(), entry.pids.clone()))
            .unwrap_or_default();
        writeln!(stdout, "{command}")?;
        // SIGCONT follows the target numbering (18 Linux / 19 Darwin) via
        // kill::SIGCONT_NUMBER — jobs.c:3928 `killpg (..., SIGCONT)` uses
        // the platform's own number, not a Linux literal.
        for member in &pids {
            let _ =
                crate::builtins::kill::send_signal(*member, crate::builtins::kill::SIGCONT_NUMBER);
        }
        self.shell_state.job_table.mark_running(pid);
        if let Some(entry) = self.shell_state.job_table.jobs.get_mut(&job_id) {
            entry.foreground = true;
            entry.background = false;
        }
        // rubash#1252: GNU jobs.c:3064 wait_for blocks with the shell's
        // own signal handling installed — the terminal's ^C is aimed at
        // the foreground JOB, and the shell survives it, returning
        // 128+SIGINT as fg's status. The emulated wait must therefore
        // hold the same foreground SIGINT guard as an external-child
        // wait; without it the shell process takes SIGINT at SIG_DFL
        // while fg blocks and dies outright (pty-verified). The wait's
        // untrapped-signal ExitCode(128+sig) is fg's ordinary status,
        // not a shell exit, so convert it back here.
        #[cfg(unix)]
        let _sigint_guard =
            super::external_inner::ForegroundBuiltinSigintGuard::hold_if_interactive_untrapped(
                self,
            );
        let wait_outcome = match self.wait_for_background_pid(pid, false) {
            Ok(outcome) => Ok(outcome),
            Err(ExecuteError::ExitCode(code)) if code > 128 => {
                Ok(WaitPidOutcome::Interrupted(code))
            }
            Err(error) => Err(error),
        };
        match wait_outcome? {
            WaitPidOutcome::Status(status) | WaitPidOutcome::Interrupted(status) => Ok(status),
            // Unreachable in practice: fg SIGCONTs and marks the job
            // running above, before the wait. Mapped for exhaustiveness.
            WaitPidOutcome::Stopped { stop_signal, .. } => Ok(128 + stop_signal),
            WaitPidOutcome::NotFound => {
                self.write_job_not_found("fg", job, stderr)?;
                Ok(1)
            }
        }
    }

    fn execute_bg_jobs(
        &mut self,
        jobs: Vec<String>,
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
    ) -> Result<i32, ExecuteError> {
        let requested = if jobs.is_empty() {
            vec![None]
        } else {
            jobs.iter()
                .map(|job| Some(job.as_str()))
                .collect::<Vec<_>>()
        };
        let posixly =
            crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "posix");

        let mut status = 0;
        for job in requested {
            match self.resolve_fg_bg_operand("bg", job, stderr)? {
                JobOperandResolution::Pid(pid) => {
                    if !self.shell_state.job_table.job_control_for_pid(pid) {
                        let job_id = self.shell_state.job_table.job_id_for_pid(pid).unwrap_or(0);
                        writeln!(
                            stderr,
                            "{}bg: job {} started without job control",
                            self.diagnostic_prefix(),
                            job_id
                        )?;
                        status = 1;
                        continue;
                    }
                    let Some(job_id) = self.shell_state.job_table.job_id_for_pid(pid) else {
                        self.write_job_not_found("bg", job, stderr)?;
                        status = 1;
                        continue;
                    };
                    // GNU jobs.c:3837-3843 start_job: same comsub gate as fg.
                    if self.shell_state.in_command_substitution.get() {
                        writeln!(stderr, "{}bg: no current jobs", self.diagnostic_prefix())?;
                        status = 1;
                        continue;
                    }
                    // GNU jobs.c:3849-3855 start_job: `bg` on an already-running
                    // job is a diagnostic but NOT an error under XPG6/SUSv3 —
                    // the message prints and status stays 0.
                    let already_running = self
                        .shell_state
                        .job_table
                        .jobs
                        .get(&job_id)
                        .is_some_and(|entry| entry.state == crate::jobs::ProcessState::Running);
                    if already_running {
                        writeln!(
                            stderr,
                            "{}bg: job {} already in background",
                            self.diagnostic_prefix(),
                            job_id
                        )?;
                        continue;
                    }
                    // GNU jobs.c:3826 start_job(foreground=0): print the
                    // "[N]± command &" line with the pre-reset +/− marker
                    // (jobs.c:3872-3899), SIGCONT the process group
                    // (jobs.c:3926-3928), then reset_current (jobs.c:3949).
                    let marker = if posixly {
                        " "
                    } else if self.shell_state.job_table.current_job() == Some(job_id) {
                        "+ "
                    } else if self.shell_state.job_table.previous_job() == Some(job_id) {
                        "- "
                    } else {
                        " "
                    };
                    let (command, pids) = self
                        .shell_state
                        .job_table
                        .jobs
                        .get(&job_id)
                        .map(|entry| (entry.command.clone(), entry.pids.clone()))
                        .unwrap_or_default();
                    writeln!(stdout, "[{job_id}]{marker}{command} &")?;
                    // SIGCONT follows the target numbering (18 Linux / 19
                    // Darwin) via kill::SIGCONT_NUMBER — see the fg twin above
                    // (jobs.c:3928).
                    for member in &pids {
                        let _ = crate::builtins::kill::send_signal(
                            *member,
                            crate::builtins::kill::SIGCONT_NUMBER,
                        );
                    }
                    self.shell_state.job_table.mark_running(pid);
                    if let Some(entry) = self.shell_state.job_table.jobs.get_mut(&job_id) {
                        entry.background = true;
                        entry.foreground = false;
                    }
                    self.shell_state.job_table.reset_current();
                }
                JobOperandResolution::Ambiguous => {
                    // GNU fg_bg.def:144-152: DUP_JOB skips sh_badjob —
                    // the get_job_by_name diagnostic is the whole report.
                    writeln!(
                        stderr,
                        "{}bg: {}: ambiguous job spec",
                        self.diagnostic_prefix(),
                        job.map_or("current", jobspec_match_name)
                    )?;
                    status = 1;
                }
                JobOperandResolution::NoMatch => {
                    self.write_job_not_found("bg", job, stderr)?;
                    status = 1;
                }
            }
        }
        Ok(status)
    }

    fn resolve_requested_background_job(&self, job: Option<&str>) -> JobOperandResolution {
        match job {
            Some(job) => self.resolve_background_job_operand(job),
            None => self
                .current_background_pid()
                .map_or(JobOperandResolution::NoMatch, JobOperandResolution::Pid),
        }
    }

    /// GNU fg_bg.def / jobs.c get_job_from_arg: an fg/bg operand that does
    /// not start with `%` is NOT read as a pid. Bash warns
    /// `job specification requires leading '%'` and then resolves the
    /// operand as a jobspec anyway — `bg 1` selects job 1, `bg sleep`
    /// matches by name, and a bare pid like 4788 matches nothing because
    /// no jobspec spells it. wait/disown/kill keep the pid reading of bare
    /// numbers; only fg and bg warn and reinterpret (rubash#1408,
    /// rubash#1409). The caller has already applied the no-job-control
    /// gate, which GNU reports before any operand processing.
    fn resolve_fg_bg_operand(
        &self,
        builtin: &str,
        job: Option<&str>,
        stderr: &mut Vec<u8>,
    ) -> Result<JobOperandResolution, ExecuteError> {
        let Some(job) = job else {
            return Ok(self.resolve_requested_background_job(None));
        };
        if job.starts_with('%') {
            return Ok(self.resolve_background_job_operand(job));
        }
        writeln!(
            stderr,
            "{}{builtin}: warning: {job}: job specification requires leading `%'",
            self.diagnostic_prefix()
        )?;
        Ok(self.resolve_background_job_operand(&format!("%{job}")))
    }

    fn current_background_pid(&self) -> Option<u32> {
        let job_id = self.shell_state.job_table.current_job()?;
        self.shell_state
            .job_table
            .jobs
            .get(&job_id)?
            .pids
            .last()
            .copied()
    }

    fn write_job_not_found(
        &self,
        builtin: &str,
        job: Option<&str>,
        stderr: &mut Vec<u8>,
    ) -> Result<(), ExecuteError> {
        let job = job.unwrap_or("current");
        writeln!(
            stderr,
            "{}{builtin}: {job}: no such job",
            self.diagnostic_prefix()
        )?;
        Ok(())
    }

    pub(in crate::executor) fn execute_suspend(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let (status, too_many_arguments) = crate::builtins::suspend::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        if too_many_arguments {
            // GNU builtins/common.c no_args (the `exit 1 2` class,
            // rubash#705): the diagnostic is out, $? is EX_BADUSAGE (2),
            // and a DISCARD jump abandons the rest of the current list —
            // script readers resume at the next line, a `-c` string is
            // abandoned whole and the shell exits 1 (shell.c:1477). An
            // `||` cannot rescue it (rubash#1331).
            self.exit_code = 2;
            self.raise_evalerror_abort();
            self.evalerror_exit_toomany_pending.set(true);
            return Ok(2);
        }
        Ok(status)
    }

    pub(in crate::executor) fn execute_history(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let args = &cmd.words[1..];
        // builtins/history.def: The history builtin works for file I/O
        // (-r/-w/-a/-n) and listing even without `set -o history`.
        // remember_on_history only controls automatic recording. Create
        // a session history on demand so the builtin can read/list —
        // but only when no host provider is installed: an installed
        // provider IS the history list (the host owns interactive
        // history), so an auto-created empty session must not shadow
        // it (Executor::ensure_session_history owns the rule).
        self.ensure_session_history();
        if let Some(session) = self.shell_state.session_history.clone() {
            // The shell's own session history (scripts that ran
            // "set -o history") takes precedence over the host provider.
            let status = super::history_exec::execute_history_session(
                self,
                args,
                session,
                &mut stdout,
                &mut stderr,
            )?;
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(status);
        }
        let provider = self.history_provider.as_ref().cloned();
        if let Some(provider) = provider {
            // Detect history file I/O modes (-a/-n/-r/-w) and optional -c.
            let mut has_io = false;
            let mut do_append = false;
            let mut do_read_new = false;
            let mut do_read = false;
            let mut do_write = false;
            let mut do_clear = false;
            let mut file_arg: Option<String> = None;
            {
                let mut idx = 0;
                while let Some(arg) = args.get(idx) {
                    if arg == "--" {
                        idx += 1;
                        break;
                    }
                    if !arg.starts_with('-') || arg == "-" {
                        break;
                    }
                    for option in arg[1..].chars() {
                        match option {
                            'a' => {
                                has_io = true;
                                do_append = true;
                            }
                            'n' => {
                                has_io = true;
                                do_read_new = true;
                            }
                            'r' => {
                                has_io = true;
                                do_read = true;
                            }
                            'w' => {
                                has_io = true;
                                do_write = true;
                            }
                            'c' => {
                                do_clear = true;
                            }
                            _ => {}
                        }
                    }
                    idx += 1;
                }
                if has_io {
                    file_arg = args.get(idx).cloned();
                }
                // GNU history.def:145: only one of -anrw may be given.
                let io_flags = [do_append, do_read_new, do_read, do_write]
                    .iter()
                    .filter(|flag| **flag)
                    .count();
                if io_flags > 1 {
                    writeln!(
                        stderr,
                        "{}history: cannot use more than one of -anrw",
                        self.diagnostic_prefix()
                    )?;
                    self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                    return Ok(1);
                }
            }

            if has_io {
                let from_histfile = file_arg.is_none();
                // builtins/history.def history_builtin: the
                // null-filename block prints its diagnostic and returns
                // `interactive ? EXECUTION_SUCCESS : EXECUTION_FAILURE'
                // (shell.c:143 `interactive'; rubash#1830) — the
                // engine's interactive_reader_session predicate.
                let null_filename_status = if self.interactive_reader_session() {
                    0
                } else {
                    1
                };
                let Some(file) = file_arg.or_else(|| self.get_env("HISTFILE").map(String::from))
                else {
                    // builtins/history.def history_builtin file branch:
                    // no operand and $HISTFILE unset is the `parameter
                    // null or not set' diagnostic (rubash#1633).
                    writeln!(
                        stderr,
                        "{}history: HISTFILE: parameter null or not set",
                        self.diagnostic_prefix()
                    )?;
                    self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                    return Ok(null_filename_status);
                };
                if file.is_empty() {
                    // builtins/history.def: a null $HISTFILE and an
                    // explicitly empty operand are distinct diagnostics
                    // (rubash#1289 B3, rubash#1633).
                    writeln!(
                        stderr,
                        "{}history: {}",
                        self.diagnostic_prefix(),
                        if from_histfile {
                            "HISTFILE: parameter null or not set"
                        } else {
                            "empty filename"
                        }
                    )?;
                    self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                    return Ok(null_filename_status);
                }
                let mut p = provider.borrow_mut();
                if do_clear {
                    p.clear()?;
                    let _ = std::fs::write(&file, "");
                }
                // The file modes report through the builtin's own
                // diagnostic surface, exactly like the session path in
                // history_exec.rs: an io failure is a builtin outcome
                // with status 1, never an executor error leaking the
                // bare io text out of the builtin (rubash#1830 —
                // `history -a <missing-dir>/f` through a host provider
                // surfaced as just `No such file or directory`).
                if do_append {
                    if let Err(err) = p.append_history(&file) {
                        // bashhist.c:450 maybe_append_history: the
                        // `cannot create' line belongs to the branch
                        // where the target does not exist (stat ->
                        // ENOENT, the pre-create open fails); every
                        // other append failure stays silent. The
                        // portable form of that condition is the
                        // error's NotFound kind.
                        if err.kind() == std::io::ErrorKind::NotFound {
                            writeln!(
                                stderr,
                                "{}history: {file}: cannot create: {}",
                                self.diagnostic_prefix(),
                                crate::posix_errors::message(&err)
                            )?;
                        }
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        return Ok(1);
                    }
                }
                if do_read_new {
                    if p.read_new_history(&file).is_err() {
                        // GNU read failures are silent: the
                        // read-error report in builtins/history.def
                        // is wrapped in `#if 0' in both the RFLAG
                        // branch (history.def:303) and the NFLAG
                        // branch (history.def:322), so -r/-n return
                        // EXECUTION_FAILURE with no diagnostic
                        // (rubash#1852).
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        return Ok(1);
                    }
                }
                if do_read {
                    if p.read_history(&file).is_err() {
                        // Silent, like -n above (rubash#1852).
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        return Ok(1);
                    }
                }
                if do_write {
                    if p.write_history(&file).is_err() {
                        // GNU write_history failures are silent
                        // (histfile.c history_do_write returns the
                        // errno unreported).
                        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                        return Ok(1);
                    }
                }
                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                return Ok(0);
            }

            let clear = do_clear || args.iter().any(|arg| arg == "-c" || arg == "-pc");
            let save = args.iter().position(|arg| arg == "-s" || arg == "-ps");
            let delete = args.iter().position(|arg| arg == "-d");
            let entries = if clear {
                provider.borrow_mut().clear()?;
                Vec::new()
            } else if let Some(index) = save {
                let command = args[index + 1..].join(" ");
                if !command.is_empty() {
                    provider.borrow_mut().append(command)?;
                }
                Vec::new()
            } else if let Some(index) = delete {
                let mut entries = provider.borrow_mut().entries()?;
                // GNU 5.3 history.def:190-222: `history -d start-end`
                // deletes the INCLUSIVE range. The separator scan starts
                // after a leading '-' (so `-2-4` is start -2, end 4);
                // negative numbers count back from the end of the list
                // (-1 is the last entry), positive numbers are the
                // displayed numbers (offset by history_base = 1).
                // remove_history_range (lib/readline/history.c) silently
                // refuses first > last and out-of-range positions.
                let history_number = |text: &str| -> Option<i128> {
                    let value: i128 = text.parse().ok()?;
                    let length = entries.len() as i128;
                    if text.starts_with('-') && value < 0 {
                        Some(value + length)
                    } else if value > 0 {
                        Some(value - 1)
                    } else {
                        Some(0)
                    }
                };
                if let Some(arg) = args.get(index + 1) {
                    let search_from = if arg.starts_with('-') { 1 } else { 0 };
                    let range_pos = arg[search_from..].find('-').map(|pos| pos + search_from);
                    if let Some(pos) = range_pos {
                        let (start_text, end_text) = (&arg[..pos], &arg[pos + 1..]);
                        match (history_number(start_text), history_number(end_text)) {
                            (Some(start), Some(end))
                                if start >= 0
                                    && end >= 0
                                    && (start as usize) < entries.len()
                                    && (end as usize) < entries.len()
                                    && start <= end =>
                            {
                                entries.drain(start as usize..=end as usize);
                                provider.borrow_mut().replace(entries.clone())?;
                            }
                            _ => {
                                writeln!(
                                    stderr,
                                    "{}history: {arg}: history position out of range",
                                    self.diagnostic_prefix()
                                )?;
                                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                                return Ok(1);
                            }
                        }
                    } else if let Some(value) = history_number(arg) {
                        if value >= 0 && (value as usize) < entries.len() {
                            entries.remove(value as usize);
                            provider.borrow_mut().replace(entries.clone())?;
                        } else {
                            writeln!(
                                stderr,
                                "{}history: {arg}: history position out of range",
                                self.diagnostic_prefix()
                            )?;
                            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                            return Ok(1);
                        }
                    }
                }
                Vec::new()
            } else {
                provider.borrow_mut().entries()?
            };
            let mut surplus = false;
            let status = crate::builtins::history::execute_with_history(
                args,
                &self.diagnostic_prefix(),
                &entries,
                &mut stdout,
                &mut stderr,
                &mut surplus,
            )?;
            if surplus {
                // rubash#1660: no_args DISCARD (see history_exec.rs).
                self.raise_evalerror_abort();
                self.evalerror_exit_toomany_pending.set(true);
            }
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(status);
        }
        let (status, surplus) = crate::builtins::history::execute_with_io(
            args,
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        if surplus {
            // rubash#1660: no_args DISCARD (see history_exec.rs).
            self.raise_evalerror_abort();
            self.evalerror_exit_toomany_pending.set(true);
        }
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_fc(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let posix_mode = self
            .get_env("__RUBASH_POSIX_MODE")
            .map(|v| v == "1")
            .unwrap_or(false);
        let session = self.shell_state.session_history.clone();
        let result = if let Some(session) = session.as_ref() {
            let (entries, base, last_added) = {
                let shell = session.borrow();
                (shell.entries.clone(), shell.base, shell.last_line_added)
            };
            // fc.def:334: rh = remember_on_history (the `history` set -o
            // option; on for interactive shells).
            let remember =
                crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "history");
            crate::builtins::fc::execute_with_history(
                &cmd.words[1..],
                &self.diagnostic_prefix(),
                entries,
                base,
                last_added,
                remember,
                posix_mode,
                &mut stdout,
                &mut stderr,
            )?
        } else {
            let provider_entries = match self.history_provider.as_ref() {
                Some(provider) => provider.borrow_mut().entries()?,
                None => Vec::new(),
            };
            crate::builtins::fc::execute_with_history(
                &cmd.words[1..],
                &self.diagnostic_prefix(),
                provider_entries,
                1,
                false,
                false,
                posix_mode,
                &mut stdout,
                &mut stderr,
            )?
        };
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        match result {
            crate::builtins::fc::FcResult::EditWith {
                editor,
                start,
                end,
                rev,
                drop_last_entry,
            } => {
                // fc.def edit_and_execute_command: write the selected
                // entries to a temp file, run the editor with inherited
                // stdio, read the result back, remember and execute it.
                // The fc-line deletion (fc.def:403-421) happens first:
                // EditWith indices address the post-deletion list. An
                // empty selection still runs the editor on an empty
                // file, exactly like GNU (#1117).
                if drop_last_entry {
                    if let Some(session) = session.as_ref() {
                        let mut shell = session.borrow_mut();
                        if !shell.entries.is_empty() {
                            shell.entries.pop();
                            shell.timestamps.pop();
                        }
                    }
                }
                let indices: Vec<usize> = if rev {
                    (start..=end).rev().collect()
                } else {
                    (start..=end).collect()
                };
                let session_entries: Vec<String> = session
                    .as_ref()
                    .map(|s| s.borrow().entries.clone())
                    .unwrap_or_default();
                let selected: Vec<String> = indices
                    .iter()
                    .filter(|i| **i < session_entries.len())
                    .map(|i| session_entries[*i].clone())
                    .collect();
                let editor_name = editor
                    .or_else(|| self.get_env("FCEDIT").map(String::from))
                    .or_else(|| self.get_env("EDITOR").map(String::from))
                    .unwrap_or_else(|| String::from("vi"));
                // fc.def: sh_mktmpfp ("bash-fc", MT_USERANDOM|MT_USETMPDIR)
                // -- a six-character random suffix, created exclusively.
                let file_text = if selected.is_empty() {
                    String::new()
                } else {
                    selected.join("\n") + "\n"
                };
                let mut path = std::env::temp_dir();
                let mut rng_state = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0x9e37_79b9_7f4a_7c15)
                    ^ (std::process::id() as u64).rotate_left(17)
                    | 1;
                let mut created = false;
                for _ in 0..16 {
                    let suffix: String = (0..6)
                        .map(|_| {
                            // xorshift64*; take a letter/digit like mkstemp.
                            rng_state ^= rng_state >> 12;
                            rng_state ^= rng_state << 25;
                            rng_state ^= rng_state >> 27;
                            let v = (rng_state.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 58) as u8;
                            match v % 62 {
                                0..=25 => char::from(b'a' + v % 62),
                                26..=51 => char::from(b'A' + v % 62 - 26),
                                _ => char::from(b'0' + v % 62 - 52),
                            }
                        })
                        .collect();
                    path.push(format!("bash-fc.{suffix}"));
                    match std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                    {
                        Ok(mut file) => {
                            created = file.write_all(file_text.as_bytes()).is_ok();
                            break;
                        }
                        Err(_) => {
                            path.pop();
                        }
                    }
                }
                if !created {
                    writeln!(
                        stderr,
                        "{pfx}fc: cannot open temp file",
                        pfx = self.diagnostic_prefix(),
                    )?;
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(1);
                }
                // fc.def edit_and_execute_command: the editor string is
                // a shell COMMAND LINE run through parse_and_execute with
                // the temp file appended (sprintf "%s %s", ename, fn) —
                // builtins, functions and multi-word editors all work
                // (`fc -e :`, `fc -e 'sed -i s/a/b/'`), and any failure
                // makes fc fail with EXECUTION_FAILURE (#1117).
                let editor_line = format!("{editor_name} {}", path.display());
                let editor_tokens = crate::lexer::tokenize(&editor_line);
                let mut editor_ast = crate::parser::parse_with_options(
                    &editor_tokens,
                    crate::parser::ParseLoopOptions {
                        stray_close_is_error: true,
                        diagnostic_text: None,
                        source_text: Some(editor_line.as_str().into()),
                        source_line_offset: 0,
                    },
                );
                self.apply_command_output_redirects(cmd, &mut editor_ast)?;
                let editor_result = self.with_compound_output_redirects(cmd, |executor| {
                    executor.execute_ast(&editor_ast)
                });
                let editor_status = self.exit_code;
                match editor_result {
                    Ok(()) if editor_status == 0 => {}
                    Ok(()) => {
                        let _ = std::fs::remove_file(&path);
                        self.exit_code = 1;
                        return Ok(1);
                    }
                    Err(ExecuteError::ExitCode(code)) => {
                        let _ = std::fs::remove_file(&path);
                        return Err(ExecuteError::ExitCode(code));
                    }
                    Err(_) => {
                        let _ = std::fs::remove_file(&path);
                        self.exit_code = 1;
                        return Ok(1);
                    }
                }
                let edited = std::fs::read_to_string(&path).unwrap_or_default();
                let _ = std::fs::remove_file(&path);
                let edited = edited.trim_end().to_string();
                if edited.is_empty() {
                    return Ok(0);
                }
                // fc.def:539: echo_input_at_read = 1 before fc_execute_file.
                // y.tab.c:5071-5075: each input line is echoed to stderr —
                // the COMMAND's stderr, so an `fc ... 2>file` redirect
                // captures the echo like any builtin diagnostic. The raw
                // eprintln! here bypassed the redirect entirely
                // (rubash#1211). The echo is written below, inside the
                // compound-redirect scope: that scope is what binds fd 2
                // to the command's redirect target, and writing before it
                // opens would lose the bytes to its truncation.
                let mut echo = Vec::new();
                for line in edited.lines() {
                    use std::io::Write as _;
                    let _ = writeln!(echo, "{line}");
                }
                if let Some(session) = session.as_ref() {
                    let control = self.get_env("HISTCONTROL").unwrap_or_default();
                    let ignore = self.get_env("HISTIGNORE").unwrap_or_default();
                    let histsize =
                        crate::history::SessionHistory::size_limit(self.get_env("HISTSIZE"));
                    let mut shell = session.borrow_mut();
                    // The fc-line entry was already deleted above
                    // (fc.def:403-421); the executed text is remembered
                    // in its place (fc_execute_file with
                    // remember_on_history forced on, fc.def:530).
                    let was_recorded = shell.record(&edited, &control, &ignore, histsize);
                    shell.last_line_added = was_recorded;
                }
                let tokens = crate::lexer::tokenize(&edited);
                let mut ast = crate::parser::parse_with_options(
                    &tokens,
                    crate::parser::ParseLoopOptions {
                        stray_close_is_error: true,
                        diagnostic_text: None,
                        source_text: Some(edited.as_str().into()),
                        source_line_offset: 0,
                    },
                );
                self.apply_command_output_redirects(cmd, &mut ast)?;
                self.with_compound_output_redirects(cmd, |executor| {
                    // rubash#1211: the fc echo (built above) goes to fd 2
                    // as bound by this command's redirects, BEFORE the
                    // re-executed text runs — GNU's echo_input_at_read
                    // order (fc.def:539).
                    executor.write_default_stderr(&echo)?;
                    executor.execute_ast(&ast)
                })?;
                Ok(self.exit_code)
            }
            crate::builtins::fc::FcResult::Status(status) => Ok(status),
            crate::builtins::fc::FcResult::Reexec { command } => {
                // fc -s: the substituted command is remembered (the C
                // parse_and_execute remembers it) and then executed.
                if let Some(session) = session.as_ref() {
                    let control = self.get_env("HISTCONTROL").unwrap_or_default();
                    let ignore = self.get_env("HISTIGNORE").unwrap_or_default();
                    let histsize =
                        crate::history::SessionHistory::size_limit(self.get_env("HISTSIZE"));
                    let mut shell = session.borrow_mut();
                    // fc.def fc_replhist: the executed command REPLACES
                    // the last entry (bash_delete_last_history +
                    // maybe_add_history), so the fc command itself never
                    // appears in the history. Rubash pops when the last
                    // entry is the current line's (hist_last_line_added),
                    // which covers both the interactive fc line and the
                    // entry an `fc -s`/`fc -e -` re-execution resolved.
                    if shell.last_line_added && !shell.entries.is_empty() {
                        shell.entries.pop();
                        shell.timestamps.pop();
                    }
                    let was_recorded = shell.record(&command, &control, &ignore, histsize);
                    shell.last_line_added = was_recorded;
                }
                let tokens = crate::lexer::tokenize(&command);
                let mut ast = crate::parser::parse_with_options(
                    &tokens,
                    crate::parser::ParseLoopOptions {
                        stray_close_is_error: true,
                        diagnostic_text: None,
                        source_text: Some(command.as_str().into()),
                        source_line_offset: 0,
                    },
                );
                self.apply_command_output_redirects(cmd, &mut ast)?;
                self.with_compound_output_redirects(cmd, |executor| executor.execute_ast(&ast))?;
                Ok(self.exit_code)
            }
        }
    }
    pub(in crate::executor) fn execute_completion_builtin(
        &mut self,
        cmd: &CommandNode,
        builtin: crate::builtins::complete::CompletionBuiltin,
    ) -> Result<i32, ExecuteError> {
        let mut stdout: Vec<u8> = Vec::new();
        let mut stderr: Vec<u8> = Vec::new();
        let diagnostic_prefix = self.diagnostic_prefix();
        if matches!(
            builtin,
            crate::builtins::complete::CompletionBuiltin::Complete
        ) {
            let args = &cmd.words[1..];
            // complete_builtin (complete.def:386-489): parse the words once,
            // then -p print / -r remove / register per name against the
            // registry. Bare complete (no words) prints all specs.
            if args.is_empty() {
                for (name, cs) in self.shell_state.completion_specs.iter() {
                    crate::builtins::complete::print_compspec_line(name, cs, &mut stdout)?;
                }
                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                return Ok(0);
            }
            let parsed = match crate::builtins::complete::parse_completion_options(
                builtin,
                args,
                &diagnostic_prefix,
                &mut stderr,
            )? {
                Err(status) => {
                    self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                    return Ok(status);
                }
                Ok(parsed) => parsed,
            };
            // -D overrides -E overrides -I (complete.def:417-424); the
            // pseudo names are the registry keys themselves.
            let pseudo: Option<&str> = if parsed.dflag {
                Some("-D")
            } else if parsed.eflag {
                Some("-E")
            } else if parsed.iflag {
                Some("-I")
            } else {
                None
            };
            // GNU names the pseudo compspecs by their pcomplete.h magic
            // names in diagnostics (`complete: _DefaultCmD_: no completion
            // specification`), while -p prints the -D/-E flag form
            // (rubash#1090).
            let pseudo_diag_name = pseudo.map(|flag| match flag {
                "-D" => "_DefaultCmD_",
                "-E" => "_EmptycmD_",
                _ => "_InitialWorD_",
            });
            if parsed.pflag {
                // -p overrides everything else (complete.def:426-441).
                let mut status = 0;
                if let Some(pseudo) = pseudo {
                    match self.shell_state.completion_specs.get(pseudo) {
                        Some(cs) => {
                            crate::builtins::complete::print_compspec_line(
                                pseudo,
                                cs,
                                &mut stdout,
                            )?;
                        }
                        None => {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}complete: {}: no completion specification",
                                pseudo_diag_name.unwrap_or(pseudo)
                            )?;
                            status = 1;
                        }
                    }
                } else if !parsed.operands.is_empty() {
                    // print_cmd_completions (complete.def:630-650): argument
                    // order, unknown names error and fail the builtin.
                    for target in &parsed.operands {
                        match self.shell_state.completion_specs.get(target.as_str()) {
                            Some(cs) => {
                                crate::builtins::complete::print_compspec_line(
                                    target,
                                    cs,
                                    &mut stdout,
                                )?;
                            }
                            None => {
                                writeln!(
                                    stderr,
                                    "{diagnostic_prefix}complete: {target}: no completion specification"
                                )?;
                                status = 1;
                            }
                        }
                    }
                } else {
                    for (name, cs) in self.shell_state.completion_specs.iter() {
                        crate::builtins::complete::print_compspec_line(name, cs, &mut stdout)?;
                    }
                }
                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                return Ok(status);
            }
            if parsed.rflag {
                // -r next (complete.def:443-458); remove_cmd_completions
                // errors on unknown names, bare -r flushes the table.
                let mut status = 0;
                if let Some(pseudo) = pseudo {
                    if !self.shell_state.completion_specs.remove(pseudo) {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}complete: {}: no completion specification",
                            pseudo_diag_name.unwrap_or(pseudo)
                        )?;
                        status = 1;
                    }
                } else if !parsed.operands.is_empty() {
                    for target in &parsed.operands {
                        if !self.shell_state.completion_specs.remove(target) {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}complete: {target}: no completion specification"
                            )?;
                            status = 1;
                        }
                    }
                } else {
                    self.shell_state.completion_specs.flush();
                }
                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                return Ok(status);
            }
            if parsed.operands.is_empty() && parsed.opt_given && pseudo.is_none() {
                // complete.def:460-464 `if (wl == 0 && list == 0 && opt_given)`:
                // options but no names and no -p/-r/-D/-E/-I print usage and
                // fail with EX_USAGE. The -D/-E/-I word list (wl, built at
                // complete.def:417-424 from DEFAULTCMD/EMPTYCMD/INITIALWORD)
                // counts as a name, so `complete -D -F fn` registers the
                // default-command compspec silently with status 0 —
                // bash_completion:3617's dynamic loader depends on it
                // (rubash#133).
                crate::builtins::complete::write_usage(builtin, &mut stderr)?;
                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                return Ok(2); // EX_USAGE
            }
            // Register the compspec for every name (complete.def:480-485).
            let spec = crate::builtins::complete::Compspec::from_parsed(&parsed);
            if let Some(pseudo) = pseudo {
                self.shell_state
                    .completion_specs
                    .insert(pseudo, spec.clone());
            }
            for target in &parsed.operands {
                self.shell_state
                    .completion_specs
                    .insert(target, spec.clone());
            }
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(0);
        }
        if matches!(
            builtin,
            crate::builtins::complete::CompletionBuiltin::Compopt
        ) {
            // compopt_builtin (complete.def:817-917). The NAME forms need
            // the completion registry, so the executor arm owns them; the
            // registry-free bare form lives in complete.rs execute_compopt.
            let args = &cmd.words[1..];
            let parsed = match crate::builtins::complete::parse_compopt_options(
                args,
                &diagnostic_prefix,
                &mut stderr,
            )? {
                Err(status) => {
                    self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                    return Ok(status);
                }
                Ok(parsed) => parsed,
            };
            // complete.def:852-861: -D wins over -E wins over -I (the
            // if/else chain builds exactly one pseudo word). GNU's word is
            // the pcomplete.h:109-111 magic name; rubash's registry keys the
            // pseudo specs as "-D"/"-E"/"-I" (see the complete arm above), so
            // the lookup key and the reported name differ by design.
            let pseudo: Option<(&str, &str)> = if parsed.dflag {
                Some(("_DefaultCmD_", "-D"))
            } else if parsed.eflag {
                Some(("_EmptycmD_", "-E"))
            } else if parsed.iflag {
                Some(("_InitialWorD_", "-I"))
            } else {
                None
            };
            // complete.def:862-869: `list == 0 && wl == 0` is the
            // completion-state form (RL_STATE_COMPLETING is never set
            // outside readline completion, so it always errors here).
            if parsed.operands.is_empty() && pseudo.is_none() {
                writeln!(
                    stderr,
                    "{diagnostic_prefix}compopt: not currently executing completion function"
                )?;
                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                return Ok(1); // EXECUTION_FAILURE
            }
            // complete.def:893-911: `for (l = wl ? wl : list; ...)` — the
            // pseudo word replaces the operand list when present.
            let mut status = 0;
            // (print name, diagnostic name, registry key): GNU compopt
            // prints the pseudo specs by their -D/-E flag form but names
            // the pcomplete.h magic names in diagnostics (rubash#1090).
            let operand_refs: Vec<(&str, &str, &str)> = parsed
                .operands
                .iter()
                .map(|name| (name.as_str(), name.as_str(), name.as_str()))
                .collect();
            let pseudo_ref: Vec<(&str, &str, &str)> = pseudo
                .into_iter()
                .map(|(magic, flag)| (flag, magic, flag))
                .collect();
            for (report_name, diag_name, key) in pseudo_ref.into_iter().chain(operand_refs) {
                match self.shell_state.completion_specs.get_mut(key) {
                    None => {
                        // complete.def:896-898 progcomp_search miss.
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}compopt: {diag_name}: no completion specification"
                        )?;
                        status = 1;
                    }
                    Some(cs) => {
                        if parsed.on_options == 0 && parsed.off_options == 0 {
                            // complete.def:870-874 print_compopts(cmd, cs, 1)
                            // writes to stdout.
                            crate::builtins::complete::print_compopts_line(
                                report_name,
                                cs,
                                &mut stdout,
                            )?;
                        } else {
                            // complete.def:901-909: set the -o bits and clear
                            // the +o bits on the existing compspec.
                            cs.options |= parsed.on_options;
                            cs.options &= !parsed.off_options;
                        }
                    }
                }
            }
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(status);
        }
        let function_names: Vec<String> = self.shell_state.functions.keys().cloned().collect();
        // GNU job completion names a job by its command name only (the
        // first word — `%sleep`, pcomplete.c it_jobs / jobs.c job names),
        // not the full command line (rubash#1052).
        let job_names: Vec<String> = self
            .shell_state
            .job_table
            .jobs
            .values()
            .filter(|job| job.background)
            .map(|job| {
                job.command
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        // compgen's dynamic inputs — complete.def:669 compgen_builtin drives
        // pcomplete.c:1251 gen_compspec_completions, which runs the -F
        // completion function with COMP_* bound (pcomplete.c:1046
        // gen_shell_function_matches + pcomplete.c:929
        // bind_compfunc_variables; compgen passes an empty line so
        // COMP_CWORD is cw-1 = -1), runs the -C command with $0..$3 set
        // (pcomplete.c:1140 gen_command_matches), and expands -W through
        // the full word expander (pcomplete.c:863 gen_wordlist_matches →
        // split_at_delims + expand_words_shellexp). All three need the
        // shell engine, so resolve them here before the builtin generates
        // candidates from the static actions.
        let mut dynamic: Option<crate::builtins::complete::CompgenDynamic> = None;
        // rubash#1798: the fd table saved while the compgen command's
        // own stdio redirects are bound as the resolution frame (see
        // the will_run branch below); restored after the final output.
        let mut framed_fd_table: Option<crate::executor::fd_table::FdTable> = None;
        if matches!(
            builtin,
            crate::builtins::complete::CompletionBuiltin::Compgen
        ) {
            match crate::builtins::complete::parse_completion_options(
                builtin,
                &cmd.words[1..],
                &diagnostic_prefix,
                &mut stderr,
            )? {
                Err(status) => {
                    // execute_compgen would re-parse and re-emit the same
                    // usage diagnostics; report once and stop here.
                    self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                    return Ok(status);
                }
                Ok(parsed) => {
                    if parsed.funcname.is_some() || parsed.command.is_some() {
                        // complete.def:693-696: builtin_error warning, no
                        // usage line. GNU's builtin_error reaches stderr
                        // BEFORE the -F function / -C command runs, so
                        // when one will actually execute, flush it through
                        // the buffered-output path (which honors this
                        // command's redirects) before it produces its own
                        // output (rubash#1090). A missing -F function has
                        // no run to order against — leave the warning
                        // buffered so it and the not-found diagnostic land
                        // in the SAME end-of-builtin flush; each buffered
                        // flush re-applies `>`/`2>` opens, and a second
                        // flush would truncate the first's content
                        // (rubash#1041).
                        let which = if parsed.funcname.is_some() { 'F' } else { 'C' };
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}compgen: warning: -{which} option may not work as you expect"
                        )?;
                        let will_run = parsed.command.is_some()
                            || parsed
                                .funcname
                                .as_deref()
                                .is_some_and(|funcname| self.has_function(funcname));
                        if will_run && compgen_stdio_redirected(cmd) {
                            // rubash#1798: GNU installs a builtin's
                            // redirections before it runs (execute_cmd.c
                            // do_redirections), so everything the -F
                            // function / -C command writes to fd 1/2 —
                            // including a nested compopt's `not
                            // currently executing` diagnostic — travels
                            // through THIS command's redirect frame.
                            // Bind the frame into the fd table for the
                            // resolution and the final output, and write
                            // the warning through it now (its GNU
                            // position precedes the run). The buffered
                            // flush cannot be used while framed: it
                            // re-opens the targets, and the second open
                            // truncates what the frame already wrote
                            // (the #1090/#1041 hazard — it was already
                            // erasing this very warning on the will_run
                            // path).
                            framed_fd_table = Some(self.fd_table.clone());
                            if let Err(error) = self.bind_conditional_stdio_redirects(cmd) {
                                self.fd_table = framed_fd_table.take().unwrap();
                                return Err(error);
                            }
                            if let Err(error) = self.write_default_stderr(&stderr) {
                                self.fd_table = framed_fd_table.take().unwrap();
                                return Err(error);
                            }
                            stderr.clear();
                        } else if will_run {
                            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                            stdout.clear();
                            stderr.clear();
                        }
                    }
                    let word = parsed.word().to_string();
                    let mut resolved = crate::builtins::complete::CompgenDynamic::default();
                    // rubash#1799: -A variable lists PIPESTATUS only
                    // while its cell exists as a visible variable —
                    // materialized by a completed pipeline (LiveArray)
                    // or created by a scalar bind (Scalar); the
                    // valueless declare shapes stay att_invisible in
                    // GNU and an absent cell is no variable at all.
                    resolved.pipestatus = self.pipestatus_listing();
                    if let Some(funcname) = parsed.funcname.as_deref() {
                        // GNU pcomplete.c:1058-1066 gen_shell_function_matches:
                        // a missing -F function is internal_error
                        // (error_prolog family, so gnu_errfmt-gated) printed
                        // to the builtin's stderr — the command's redirects
                        // apply, and it follows the -F/-C warning in order.
                        let error_prefix = self.error_prefix();
                        resolved.function_candidates = self.run_compgen_completion_function(
                            funcname,
                            "compgen",
                            &word,
                            &error_prefix,
                            &mut stderr,
                        );
                    }
                    if let Some(command) = parsed.command.as_deref() {
                        resolved.command_candidates =
                            self.run_compgen_completion_command(command, "compgen", &word);
                    }
                    if parsed.words.is_some() {
                        resolved.expanded_wordlist = Some(
                            self.expand_compgen_wordlist(parsed.words.as_deref().unwrap_or("")),
                        );
                    }
                    dynamic = Some(resolved);
                }
            }
        }
        let status = match if matches!(
            builtin,
            crate::builtins::complete::CompletionBuiltin::Compgen
        ) {
            // rubash#1569: compgen -e/-v/-A candidates adjudicate
            // names bound by this command's own tempenv frame.
            let underlay = self.tempenv_listing_underlay();
            crate::builtins::complete::execute_compgen_with_dynamic(
                &cmd.words[1..],
                &self.shell_state.env_vars,
                &self.shell_state.aliases,
                &function_names,
                &job_names,
                &diagnostic_prefix,
                dynamic,
                &underlay,
                &mut stdout,
                &mut stderr,
            )
        } else {
            crate::builtins::complete::execute_with_io(
                builtin,
                &cmd.words[1..],
                &self.shell_state.env_vars,
                &self.shell_state.aliases,
                &function_names,
                &job_names,
                &diagnostic_prefix,
                &mut stdout,
                &mut stderr,
            )
        } {
            Ok(status) => status,
            Err(error) => {
                // rubash#1798: never leak a bound frame past this
                // builtin, whatever failed inside it.
                if let Some(saved) = framed_fd_table.take() {
                    self.fd_table = saved;
                }
                return Err(error.into());
            }
        };
        if matches!(
            builtin,
            crate::builtins::complete::CompletionBuiltin::Compgen
        ) {
            // compgen -V varname (complete.def:761-770): store the matches in
            // the indexed array VARNAME instead of printing them; the builtin
            // still fails (1) when nothing matched.
            if let Some(varname) = crate::builtins::complete::compgen_varname(&cmd.words[1..]) {
                if status == 0 {
                    let values = String::from_utf8_lossy(&stdout)
                        .lines()
                        .map(str::to_string)
                        .collect();
                    store_indexed_array_state(&mut self.shell_state, &varname, values);
                }
                stdout.clear();
            }
        }
        if let Some(saved) = framed_fd_table.take() {
            // rubash#1798: framed mode — the matches (and any residue)
            // go through the bound frame in order, then the frame is
            // restored. The buffered flush would re-open the targets
            // and truncate the resolution's own writes.
            let mut result: Result<(), ExecuteError> = Ok(());
            if !stdout.is_empty() {
                result = self.write_default_stdout(&stdout);
            }
            if result.is_ok() && !stderr.is_empty() {
                result = self.write_default_stderr(&stderr);
            }
            self.fd_table = saved;
            result?;
        } else {
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        }
        Ok(status)
    }

    /// pcomplete.c:1046 gen_shell_function_matches specialized to compgen:
    /// bind the COMP_* variables the completion function expects
    /// (pcomplete.c:929 bind_compfunc_variables — compgen passes an empty
    /// line and `pcomp_ind - start == 0`, so `command_line_to_word_list`
    /// yields nw=cw=0 and COMP_CWORD is bound to cw-1 = -1), run the
    /// function with build_arg_list's argument shape (pcomplete.c:1004:
    /// $1 = command name, $2 = word being completed, $3 = previous word —
    /// empty here because the word list is empty), then return COMPREPLY
    /// and unbind COMP_* and COMPREPLY (pcomplete.c:985, 976-989
    /// unbind_compfunc_variables + unbind_variable_noref("COMPREPLY")).
    fn run_compgen_completion_function(
        &mut self,
        funcname: &str,
        cmd: &str,
        word: &str,
        error_prefix: &str,
        stderr: &mut Vec<u8>,
    ) -> Vec<String> {
        if !self.has_function(funcname) {
            // pcomplete.c:1058-1066: internal_error, no candidates. Route
            // through the caller's stderr buffer (not eprintln!): GNU runs
            // the compgen builtin under its redirects, so the diagnostic
            // lands in `2>file` with the warning, in order (rubash#1041).
            let _ = writeln!(
                stderr,
                "{error_prefix}completion: function `{funcname}' not found",
            );
            return Vec::new();
        }
        self.bind_compfunc_variables("", 0, &[], -1, false);
        let args = [cmd.to_string(), word.to_string(), String::new()];
        let _ = self.call_function(funcname, args);
        let reply = self
            .array_at_word_values("${COMPREPLY[@]}")
            .unwrap_or_default();
        self.unbind_compfunc_variables();
        reply
    }

    /// pcomplete.c:1140 gen_command_matches: the -C command string runs
    /// with $0 = the command itself and $1/$2/$3 = command name, word being
    /// completed, previous word (all sh_single_quoted and appended), under
    /// command substitution with the COMP_* variables exported
    /// (bind_compfunc_variables' exported=1 arm); the output splits at
    /// newlines with backslash-newline continuation (pcomplete.c:1226-1234).
    fn run_compgen_completion_command(
        &mut self,
        command: &str,
        cmd: &str,
        word: &str,
    ) -> Vec<String> {
        self.bind_compfunc_variables("", 0, &[], 0, true);
        let result = self.run_completion_command_line(command, cmd, word, "");
        self.unbind_compfunc_variables();
        result
    }

    /// The -C execution core shared by the compgen shape above and the
    /// live-completion shape below: the caller owns the COMP_* binding.
    fn run_completion_command_line(
        &mut self,
        command: &str,
        cmd: &str,
        word: &str,
        prev: &str,
    ) -> Vec<String> {
        let quoted = [cmd, word, prev]
            .iter()
            .map(|arg| {
                // lib/sh/shquote.c sh_single_quote: wrap in single quotes,
                // rendering each embedded quote as '\''.
                let mut quoted = String::with_capacity(arg.len() + 2);
                quoted.push('\'');
                for ch in arg.chars() {
                    if ch == '\'' {
                        quoted.push_str("'\\''");
                    } else {
                        quoted.push(ch);
                    }
                }
                quoted.push('\'');
                quoted
            })
            .collect::<Vec<_>>()
            .join(" ");
        let command_line = format!("{command} {quoted}");
        let output = self
            .expand_command_substitution_mut_typed_with_context(
                command_line.as_str(),
                crate::executor::substitution_metadata::SubstitutionQuoteContext::Unquoted,
            )
            .assignment_text();
        if output.is_empty() {
            return Vec::new();
        }
        let mut words = Vec::new();
        let mut current = String::new();
        let mut chars = output.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' && chars.peek() == Some(&'\n') {
                chars.next();
                continue;
            }
            if ch == '\n' {
                words.push(std::mem::take(&mut current));
            } else {
                current.push(ch);
            }
        }
        words.push(current);
        words
    }

    /// niubash#520: resolve the dynamic inputs for a LIVE completion
    /// (`Executor::complete_line`). The compspec governing the line
    /// (bashline.c attempt_shell_completion: the command's own, else the
    /// `-D` default; on an empty line the `-E` compspec, else `-D`) runs
    /// its -F function / -C command with the real completion context
    /// bound, and its -W wordlist is expanded, mirroring the compgen
    /// resolution in `execute_compgen` but over the actual word list.
    /// Returns None when no compspec governs or it carries no dynamic
    /// action. For the empty line GNU passes the placeholder command name
    /// `_EmptycmD_` with an empty COMP_WORDS and COMP_CWORD -1 (probed on
    /// GNU bash 5.3.20).
    pub(in crate::executor) fn completion_dynamic_for_line(
        &mut self,
        line: &str,
        cursor: usize,
    ) -> Option<crate::builtins::complete::CompgenDynamic> {
        let (words, cur_idx, cur_word) =
            crate::builtins::complete::split_for_completion(line, cursor);
        let spec = crate::builtins::complete::resolve_compspec(
            &self.shell_state.completion_specs,
            &words,
            cur_idx,
        )
        .cloned()?;
        if spec.funcname.is_none() && spec.command.is_none() && spec.words.is_none() {
            return None;
        }
        let (cmd_name, prev_word, bind_words, cword): (String, String, Vec<String>, i64) =
            if words.is_empty() {
                ("_EmptycmD_".to_string(), String::new(), Vec::new(), -1)
            } else {
                let prev = if cur_idx >= 1 {
                    words.get(cur_idx - 1).cloned().unwrap_or_default()
                } else {
                    String::new()
                };
                (
                    words.first().cloned().unwrap_or_default(),
                    prev,
                    words.clone(),
                    cur_idx as i64,
                )
            };
        let mut dynamic = crate::builtins::complete::CompgenDynamic::default();
        // rubash#1799: same PIPESTATUS cell rule as the compgen arm.
        dynamic.pipestatus = self.pipestatus_listing();
        if let Some(funcname) = spec.funcname.as_deref() {
            dynamic.function_candidates = self.run_completion_function_live(
                funcname,
                line,
                cursor,
                &bind_words,
                cword,
                &cmd_name,
                &cur_word,
                &prev_word,
            );
        }
        if let Some(command) = spec.command.as_deref() {
            dynamic.command_candidates = self.run_completion_command_live(
                command,
                line,
                cursor,
                &bind_words,
                cword,
                &cmd_name,
                &cur_word,
                &prev_word,
            );
        }
        if let Some(wordlist) = spec.words.as_deref() {
            dynamic.expanded_wordlist = Some(self.expand_compgen_wordlist(wordlist));
        }
        Some(dynamic)
    }

    /// pcomplete.c:1046 gen_shell_function_matches for a live completion:
    /// the same COMP_* binding and COMPREPLY capture as the compgen shape
    /// (`run_compgen_completion_function`), but over the real line and
    /// word list, with build_arg_list's arguments (pcomplete.c:1004:
    /// $1 = command name, $2 = word being completed, $3 = previous word).
    #[allow(clippy::too_many_arguments)]
    fn run_completion_function_live(
        &mut self,
        funcname: &str,
        line: &str,
        point: usize,
        words: &[String],
        cword: i64,
        cmd_name: &str,
        cur_word: &str,
        prev_word: &str,
    ) -> Vec<String> {
        if !self.has_function(funcname) {
            // GNU writes internal_error to the completion's stderr; the
            // host completion channel carries no stderr — no candidates.
            return Vec::new();
        }
        self.bind_compfunc_variables_live(line, point, words, cword);
        let args = [
            cmd_name.to_string(),
            cur_word.to_string(),
            prev_word.to_string(),
        ];
        let _ = self.call_function(funcname, args);
        let reply = self
            .array_at_word_values("${COMPREPLY[@]}")
            .unwrap_or_default();
        self.unbind_compfunc_variables();
        reply
    }

    /// pcomplete.c:1140 gen_command_matches for a live completion: the -C
    /// command runs under command substitution with the live COMP_*
    /// context bound and build_arg_list's arguments appended.
    #[allow(clippy::too_many_arguments)]
    fn run_completion_command_live(
        &mut self,
        command: &str,
        line: &str,
        point: usize,
        words: &[String],
        cword: i64,
        cmd_name: &str,
        cur_word: &str,
        prev_word: &str,
    ) -> Vec<String> {
        self.bind_compfunc_variables_live(line, point, words, cword);
        let result = self.run_completion_command_line(command, cmd_name, cur_word, prev_word);
        self.unbind_compfunc_variables();
        result
    }

    /// pcomplete.c:929 bind_compfunc_variables with the live completion's
    /// own line and word list: unlike the compgen shape, COMP_WORDS is the
    /// actual list being completed — GNU rebuilds it from the readline
    /// word list on every completion attempt, so a caller-set COMP_WORDS
    /// is overwritten here rather than inherited (the rubash#1088
    /// pass-through applies only outside a live completion).
    fn bind_compfunc_variables_live(
        &mut self,
        line: &str,
        point: usize,
        words: &[String],
        cword: i64,
    ) {
        self.set_env("COMP_LINE", line);
        self.set_env("COMP_POINT", &point.to_string());
        self.set_env("COMP_TYPE", "0");
        self.set_env("COMP_KEY", "0");
        store_indexed_array_state(&mut self.shell_state, "COMP_WORDS", words.to_vec());
        self.shell_state.variables.remove("COMP_WORDS");
        self.set_env("COMP_CWORD", &cword.to_string());
    }

    /// pcomplete.c:863 gen_wordlist_matches: split the -W wordlist with
    /// split_at_delims (subst.c:2616) at the CURRENT $IFS (quotes group
    /// and are retained in the pieces), then expand each word with the
    /// shellexp rules: a quoted word keeps one field per array element
    /// and never glob-splits; an unquoted word goes through the
    /// for-list expander (parameter expansion, field splitting, pathname
    /// expansion).
    fn expand_compgen_wordlist(&mut self, wordlist: &str) -> Vec<String> {
        // subst.c:2630: with delims == NULL the splitter reads
        // ifs_value — the shell's current IFS (this command's tempenv
        // frame included), or the default " \t\n" while IFS is unset.
        // An empty IFS splits nowhere.
        let ifs = self
            .shell_state
            .env_vars
            .get("IFS")
            .cloned()
            .unwrap_or_else(|| " \t\n".to_string());
        let mut values = Vec::new();
        for (word, quoted) in split_compgen_wordlist(wordlist, &ifs) {
            if quoted {
                // expand_words_shellexp keeps W_QUOTED words whole; a word
                // that IS an array reference yields one field per element
                // ("${toks[@]}" — pcomplete.c:877-880 comment).
                if let Some(elements) = self.array_at_word_values(&word) {
                    values.extend(elements);
                } else {
                    values.push(self.expand_word(&word));
                }
            } else {
                values.extend(
                    self.expand_for_word_values_result(&word, None, None)
                        .unwrap_or_default(),
                );
            }
        }
        values
    }

    /// rubash#1799: the PIPESTATUS cell facts for the -A variable /
    /// -A arrayvar listings, read from the cell shape the status writer
    /// maintains: a completed pipeline (or array bind) leaves a
    /// LiveArray cell, a scalar bind a Scalar cell — both visible
    /// variables in GNU's table; the valueless declare shapes are
    /// att_invisible and an Absent shape is no cell at all.
    fn pipestatus_listing(&self) -> crate::builtins::complete::PipestatusListing {
        use crate::shell::state::PipestatusShape;
        crate::builtins::complete::PipestatusListing {
            cell_visible: matches!(
                self.shell_state.pipestatus_shape,
                PipestatusShape::LiveArray | PipestatusShape::Scalar
            ),
            live_array: self.shell_state.pipestatus_shape == PipestatusShape::LiveArray,
        }
    }

    /// pcomplete.c:929 bind_compfunc_variables with the compgen shape: an
    /// empty line and a zero index (COMP_LINE "", COMP_POINT 0,
    /// COMP_TYPE 0, COMP_KEY 0 — readline is not completing here), the
    /// COMP_WORDS array, and COMP_CWORD (cw - 1 for functions,
    /// pcomplete.c:1073-1074).
    ///
    /// The `exported` arm is the -C shape (pcomplete.c:1156
    /// gen_command_matches passes exported=1): only the four scalar
    /// variables are bound — COMP_WORDS and COMP_CWORD are bound solely
    /// in the exported==0 (function) arm, so a -C command observes them
    /// exactly as the caller left them (unset in a fresh shell,
    /// rubash#1796). unbind_compfunc_variables removes all six either
    /// way (pcomplete.c:976), matching GNU.
    fn bind_compfunc_variables(
        &mut self,
        line: &str,
        point: usize,
        words: &[String],
        cword: i64,
        exported: bool,
    ) {
        self.set_env("COMP_LINE", line);
        self.set_env("COMP_POINT", &point.to_string());
        self.set_env("COMP_TYPE", "0");
        self.set_env("COMP_KEY", "0");
        if exported {
            return;
        }
        // GNU compgen passes the caller's COMP_WORDS through to the -F
        // function: outside a live completion there is no word list to
        // bind, so a COMP_WORDS the script already set stays visible
        // (only an unset one is bound, to the empty list) (rubash#1088).
        if !self.shell_state.env_vars.contains_key("COMP_WORDS")
            && self.shell_state.variables.get("COMP_WORDS").is_none()
        {
            store_indexed_array_state(&mut self.shell_state, "COMP_WORDS", words.to_vec());
            self.shell_state.variables.remove("COMP_WORDS");
        }
        self.set_env("COMP_CWORD", &cword.to_string());
    }

    /// pcomplete.c:976-989 unbind_compfunc_variables + the COMPREPLY
    /// unbind at pcomplete.c:1133-1136: all six COMP_* variables and
    /// COMPREPLY are removed outright.
    fn unbind_compfunc_variables(&mut self) {
        for name in [
            "COMP_WORDS",
            "COMP_CWORD",
            "COMP_LINE",
            "COMP_POINT",
            "COMP_TYPE",
            "COMP_KEY",
            "COMPREPLY",
        ] {
            self.shell_state.env_vars.remove(name);
            self.shell_state.variables.remove(name);
            // G-03: journal the completion-state cleanup removal.
            crate::executor::local_helpers::mirror_env_remove(name);
        }
    }
}

/// GNU wait.def wait_builtin(): options are scanned with
/// internal_getopt "fnp:"; a `-p` at the end of a cluster with no joined
/// text consumes the NEXT word as its argument. When no next word exists,
/// getopt reports `-p: option requires an argument` and the builtin falls
/// into its default case: usage + EX_USAGE (2). Returns true when the word
/// list has such a dangling `-p` (rubash#1156 — the operand scanners below
/// used to walk one word past the end and panic on the slice).
fn wait_p_argument_missing(words: &[String]) -> bool {
    let mut index = 0;
    while let Some(word) = words.get(index) {
        if word == "--" || !word.starts_with('-') || word == "-" {
            break;
        }
        for (offset, option) in word[1..].char_indices() {
            match option {
                'n' | 'f' => {}
                'p' => {
                    let value_start = 1 + offset + option.len_utf8();
                    if value_start < word.len() {
                        // Joined form `-pNAME`: argument is in this word;
                        // `-p` ends the cluster either way.
                        break;
                    }
                    return words.get(index + 1).is_none();
                }
                _ => return false,
            }
        }
        index += 1;
    }
    false
}

struct WaitAnyRequest {
    operands: Vec<String>,
    assign_var: Option<String>,
    assign_var_index: Option<usize>,
}

fn wait_any_request(words: &[String]) -> Option<WaitAnyRequest> {
    let mut index = 0;
    let mut wait_any = false;
    let mut assign_var = None;
    let mut assign_var_index = None;
    while let Some(word) = words.get(index) {
        if word == "--" {
            index += 1;
            break;
        }
        if !word.starts_with('-') || word == "-" {
            break;
        }

        for (offset, option) in word[1..].char_indices() {
            match option {
                'n' => wait_any = true,
                'f' => {}
                'p' => {
                    let value_start = 1 + offset + option.len_utf8();
                    let (name, name_index) = if value_start < word.len() {
                        (&word[value_start..], index)
                    } else {
                        index += 1;
                        (words.get(index)?.as_str(), index)
                    };

                    // GNU wait.def:156-157: validity is decided downstream
                    // by valid_identifier/valid_array_reference under
                    // SET_VFLAGS — array-subscript names are legal and must
                    // reach the execute path for the real check. The in-band
                    // ARRAYREF_FLAG prefix is W_ARRAYREF's carrier
                    // (execute_cmd.c:4366), consumed by wait_var_arrayref —
                    // strip it so `A` never becomes the bound base name.

                    assign_var = Some(
                        crate::builtins::arrayref::take_arrayref_flag(name)
                            .1
                            .to_string(),
                    );
                    assign_var_index = Some(name_index);
                    break;
                }
                _ => return None,
            }
        }
        index += 1;
    }

    wait_any.then(|| WaitAnyRequest {
        operands: words[index..].to_vec(),
        assign_var,
        assign_var_index,
    })
}

fn wait_background_operands(words: &[String]) -> Option<Vec<String>> {
    let mut index = 0;
    while let Some(word) = words.get(index) {
        if word == "--" {
            index += 1;
            break;
        }
        if !word.starts_with('-') || word == "-" {
            break;
        }

        // GNU internal_getopt "fnp:": -p consumes the rest of the word or
        // the next word as its variable-name argument, so it must not leak
        // into the operand list.
        for (offset, option) in word[1..].char_indices() {
            match option {
                'f' => {}
                'n' => return None,
                'p' => {
                    if offset + 2 >= word.len() {
                        index += 1;
                    }
                    break;
                }
                _ => return None,
            }
        }
        index += 1;
    }

    // Jobspec operands keep no W_ARRAYREF consumer — strip the in-band
    // flag so `wait: A[]]: ...` diagnostics print clean text. A dangling
    // `-p` walks `index` one past the end (its argument word is counted,
    // then the option word itself); saturate instead of slicing past the
    // end (rubash#1156).
    Some(
        words[index.min(words.len())..]
            .iter()
            .map(|word| {
                crate::builtins::arrayref::take_arrayref_flag(word)
                    .1
                    .to_string()
            })
            .collect(),
    )
}

/// The `-p` variable name from a wait command's option cluster, mirroring
/// GNU internal_getopt "fnp:" argument consumption (wait.def:120-137).
/// Returns (name, operand_word_index) — the index of the word that carries
/// the name inside `words` (the `-pNAME` word itself for the joined form),
/// so the caller can consult its raw token for W_ARRAYREF.
fn wait_assign_var(words: &[String]) -> Option<(String, usize)> {
    let mut index = 0;
    while let Some(word) = words.get(index) {
        if word == "--" || !word.starts_with('-') || word == "-" {
            break;
        }
        for (offset, option) in word[1..].char_indices() {
            match option {
                'n' | 'f' => {}
                'p' => {
                    let value_start = 1 + offset + option.len_utf8();
                    if value_start < word.len() {
                        return Some((word[value_start..].to_string(), index));
                    }
                    // wait.def:156 SET_VFLAGS consumes W_ARRAYREF off the
                    // raw word; the in-band ARRAYREF_FLAG prefix is a word
                    // flag, never operand text (execute_cmd.c:4366).
                    return words.get(index + 1).map(|name| {
                        (
                            crate::builtins::arrayref::take_arrayref_flag(name)
                                .1
                                .to_string(),
                            index + 1,
                        )
                    });
                }
                _ => return None,
            }
        }
        index += 1;
    }
    None
}

fn write_wait_operand_error<E>(
    operand: &str,
    diagnostic_prefix: &str,
    error_prefix: &str,
    stderr: &mut E,
) -> Result<i32, ExecuteError>
where
    E: Write,
{
    if operand.starts_with('%') {
        writeln!(stderr, "{diagnostic_prefix}wait: {operand}: no such job")?;
        return Ok(127);
    }

    if !operand.is_empty() && operand.chars().all(|ch| ch.is_ascii_digit()) {
        // GNU jobs.c:2753-2754 wait_for_single_pid JWAIT_PERROR: the
        // pid-not-a-child report is internal_error (error_prolog family),
        // so it honors gnu_errfmt while the sibling builtin_error lines
        // keep `line N:`.
        writeln!(
            stderr,
            "{error_prefix}wait: pid {operand} is not a child of this shell"
        )?;
        return Ok(127);
    }

    writeln!(
        stderr,
        "{diagnostic_prefix}wait: `{operand}': not a pid or valid job spec"
    )?;
    Ok(1)
}

/// subst.c:2616 split_at_delims with delims == NULL, as driven by
/// pcomplete.c:863 gen_wordlist_matches for the -W wordlist: the
/// delimiter set is the current $IFS; a maximal run of delimiter
/// characters separates two fields and leading/trailing runs vanish,
/// so no empty fields are ever produced (with delims == NULL the d2
/// empty-field branch of split_at_delims never fires). Grouping
/// follows subst.c:2199 skip_to_delim: a backslash quotes the next
/// character before any delimiter test, and single/double quotes,
/// backquotes, $'...', $(...) and ${...} spans group — delimiter
/// characters inside a span are field content. Pieces keep their
/// quote characters for the shellexp expansion arms; a piece is
/// reported quoted when it carries top-level quoting (a quote, an
/// escape, or a $'...' span), which is what keeps its expansion
/// unsplit (rubash#1797).
fn split_compgen_wordlist(wordlist: &str, ifs: &str) -> Vec<(String, bool)> {
    CompgenWordlistScanner {
        chars: wordlist.chars().collect(),
        ifs,
        pos: 0,
    }
    .split()
}

struct CompgenWordlistScanner<'a> {
    chars: Vec<char>,
    ifs: &'a str,
    pos: usize,
}

impl CompgenWordlistScanner<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    fn is_delim(&self, c: char) -> bool {
        self.ifs.contains(c)
    }

    fn split(mut self) -> Vec<(String, bool)> {
        let mut words = Vec::new();
        loop {
            // A delimiter run is one separator; leading runs vanish.
            while matches!(self.peek(), Some(c) if self.is_delim(c)) {
                self.pos += 1;
            }
            if self.peek().is_none() {
                break;
            }
            let (piece, quoted) = self.field();
            if !piece.is_empty() {
                words.push((piece, quoted));
            }
        }
        words
    }

    /// One field: consume until an unquoted delimiter or end of input.
    /// The test order mirrors skip_to_delim: backslash and backquote
    /// are recognized before the delimiter test; quotes and dollar
    /// spans after it.
    fn field(&mut self) -> (String, bool) {
        let mut piece = String::new();
        let mut quoted = false;
        while let Some(c) = self.peek() {
            match c {
                '\\' => {
                    // A top-level escape quotes the field. The escaped
                    // character is decoded with the same carrier
                    // encoding the alias_helpers splitter produces, so
                    // the shellexp arms see the piece in the form they
                    // already handle (a raw backslash would survive
                    // expand_word verbatim: `a\ b` must yield `a b`).
                    quoted = true;
                    self.bump();
                    match self.bump() {
                        None => piece.push('\\'),
                        Some('\n') => {}
                        Some('\\') => piece.push(crate::executor::markers::DATA_BACKSLASH),
                        Some('$') => piece.push(crate::executor::markers::DATA_DOLLAR),
                        Some('`') => piece.push(crate::executor::markers::DATA_BACKTICK),
                        Some('\'') => piece.push(crate::executor::markers::DATA_SQUOTE),
                        Some('"') => piece.push(crate::executor::markers::DATA_DQUOTE),
                        Some(other) => piece.push(other),
                    }
                }
                '`' => self.backquote_span(&mut piece),
                _ if self.is_delim(c) => break,
                '\'' => {
                    quoted = true;
                    self.squote_span(&mut piece);
                }
                '"' => {
                    quoted = true;
                    self.dquote_span(&mut piece);
                }
                '$' if self.peek_next() == Some('\'') => {
                    quoted = true;
                    self.ansi_c_span(&mut piece);
                }
                '$' if self.peek_next() == Some('"') => {
                    quoted = true;
                    piece.push(self.bump().unwrap());
                    self.dquote_span(&mut piece);
                }
                '$' if self.peek_next() == Some('(') => self.dollar_paren_span(&mut piece),
                '$' if self.peek_next() == Some('{') => self.dollar_brace_span(&mut piece),
                _ => piece.push(self.bump().unwrap()),
            }
        }
        (piece, quoted)
    }

    fn squote_span(&mut self, piece: &mut String) {
        piece.push(self.bump().unwrap_or('\''));
        while let Some(c) = self.peek() {
            piece.push(self.bump().unwrap());
            if c == '\'' {
                break;
            }
        }
    }

    fn ansi_c_span(&mut self, piece: &mut String) {
        piece.push(self.bump().unwrap_or('$'));
        piece.push(self.bump().unwrap_or('\''));
        while let Some(c) = self.peek() {
            if c == '\\' {
                piece.push(self.bump().unwrap());
                if let Some(escaped) = self.bump() {
                    piece.push(escaped);
                }
                continue;
            }
            piece.push(self.bump().unwrap());
            if c == '\'' {
                break;
            }
        }
    }

    fn dquote_span(&mut self, piece: &mut String) {
        piece.push(self.bump().unwrap_or('"'));
        while let Some(c) = self.peek() {
            match c {
                '"' => {
                    piece.push(self.bump().unwrap());
                    break;
                }
                '\\' => {
                    piece.push(self.bump().unwrap());
                    // Inside double quotes a backslash quotes only
                    // $, `, ", \ and newline (skip_double_quoted).
                    if matches!(self.peek(), Some('$' | '`' | '"' | '\\' | '\n')) {
                        piece.push(self.bump().unwrap());
                    }
                }
                '`' => self.backquote_span(piece),
                '$' if self.peek_next() == Some('(') => self.dollar_paren_span(piece),
                '$' if self.peek_next() == Some('{') => self.dollar_brace_span(piece),
                _ => piece.push(self.bump().unwrap()),
            }
        }
    }

    fn backquote_span(&mut self, piece: &mut String) {
        piece.push(self.bump().unwrap_or('`'));
        while let Some(c) = self.peek() {
            if c == '\\' {
                piece.push(self.bump().unwrap());
                if let Some(escaped) = self.bump() {
                    piece.push(escaped);
                }
                continue;
            }
            piece.push(self.bump().unwrap());
            if c == '`' {
                break;
            }
        }
    }

    fn dollar_paren_span(&mut self, piece: &mut String) {
        piece.push(self.bump().unwrap_or('$'));
        piece.push(self.bump().unwrap_or('('));
        let mut depth = 1usize;
        while depth > 0 {
            let Some(c) = self.peek() else { break };
            match c {
                '\\' => {
                    piece.push(self.bump().unwrap());
                    if let Some(escaped) = self.bump() {
                        piece.push(escaped);
                    }
                }
                '\'' => self.squote_span(piece),
                '"' => self.dquote_span(piece),
                '`' => self.backquote_span(piece),
                '$' if self.peek_next() == Some('\'') => self.ansi_c_span(piece),
                '$' if self.peek_next() == Some('(') => {
                    piece.push(self.bump().unwrap());
                    piece.push(self.bump().unwrap());
                    depth += 1;
                }
                '$' if self.peek_next() == Some('{') => self.dollar_brace_span(piece),
                '(' => {
                    piece.push(self.bump().unwrap());
                    depth += 1;
                }
                ')' => {
                    piece.push(self.bump().unwrap());
                    depth -= 1;
                }
                _ => piece.push(self.bump().unwrap()),
            }
        }
    }

    fn dollar_brace_span(&mut self, piece: &mut String) {
        piece.push(self.bump().unwrap_or('$'));
        piece.push(self.bump().unwrap_or('{'));
        let mut depth = 1usize;
        while depth > 0 {
            let Some(c) = self.peek() else { break };
            match c {
                '\\' => {
                    piece.push(self.bump().unwrap());
                    if let Some(escaped) = self.bump() {
                        piece.push(escaped);
                    }
                }
                '\'' => self.squote_span(piece),
                '"' => self.dquote_span(piece),
                '`' => self.backquote_span(piece),
                '$' if self.peek_next() == Some('\'') => self.ansi_c_span(piece),
                '$' if self.peek_next() == Some('(') => self.dollar_paren_span(piece),
                '$' if self.peek_next() == Some('{') => {
                    piece.push(self.bump().unwrap());
                    piece.push(self.bump().unwrap());
                    depth += 1;
                }
                '{' => {
                    piece.push(self.bump().unwrap());
                    depth += 1;
                }
                '}' => {
                    piece.push(self.bump().unwrap());
                    depth -= 1;
                }
                _ => piece.push(self.bump().unwrap()),
            }
        }
    }
}

/// Whether a command node carries its own fd 1 / fd 2 output
/// redirection (an ordered `redirects` entry with an output kind
/// naming fd 1 or 2, or defaulting to fd 1) — the trigger for running
/// compgen's dynamic resolution under a bound fd frame
/// (rubash#1798). fd_var entries bind a fresh descriptor, never
/// fd 1/2, and input-side kinds name fd 0; neither installs a stdio
/// frame in GNU either (do_redirections binds exactly the listed
/// descriptors).
fn compgen_stdio_redirected(cmd: &CommandNode) -> bool {
    use crate::parser::RedirectKind;
    cmd.redirects.iter().any(|redirect| {
        if redirect.fd_var.is_some() {
            return false;
        }
        match redirect.kind {
            RedirectKind::Output
            | RedirectKind::Append
            | RedirectKind::ClobberOutput
            | RedirectKind::CombinedOutput
            | RedirectKind::CombinedAppend
            | RedirectKind::DuplicateOutput
            | RedirectKind::CloseOutput => matches!(redirect.fd, None | Some(1) | Some(2)),
            _ => false,
        }
    })
}
