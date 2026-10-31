use super::*;

impl Executor {
    pub(in crate::executor) fn execute_kill(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        // rubash#1654: inside a flat subshell layer (same process,
        // depth > 0), a kill addressed to the shell's own pid must be
        // judged by the pid's owner — the top-level layer — not by
        // this layer's kernel dispositions. Arm the ancestor route for
        // the whole dispatch; the builtin's delivery queues such
        // signals for the depth-0 boundary (kill.rs ANCESTOR_SIGNALS).
        #[cfg(unix)]
        let _ancestor_route = (self.shell_state.subshell_depth.get() > 0
            && self.shell_pid == std::process::id())
        .then(crate::builtins::kill::arm_ancestor_route);
        if let Some(result) = self.intercept_fake_bashpid_kill(cmd) {
            return result;
        }
        if let Some(status) = self.execute_tracked_background_kill(cmd)? {
            return Ok(status);
        }

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::kill::execute_with_io_posix(
            &cmd.words[1..],
            &mut stdout,
            &mut stderr,
            self.posix_mode_enabled(),
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    /// rubash#1461: a `kill` whose operand is the CURRENT in-process
    /// subshell layer's BASHPID. GNU's subshell is a real forked process
    /// (variables.c:1511 get_bashpid = getpid), so `kill -KILL $BASHPID`
    /// inside `( ... )` kills only that process: the parent survives,
    /// sees status 128+sig, and gets the jobs.c death notice. Here the
    /// layer runs in place and its BASHPID is fabricated as
    /// pid + (depth - origin_depth) (dynamic_arrays.rs bashpid_value) —
    /// delivering that number to kill(2) can hit the thread whose tid it
    /// collides with and kill the WHOLE shell (or an unrelated process).
    /// Intercept before any real delivery when every operand is that
    /// fake pid: signal 0 is a liveness probe on a live layer (success);
    /// an untrapped, unignored default-fatal signal records the layer's
    /// signal death (consumed by the subshell boundary in compound_exec,
    /// which skips the EXIT trap and queues the notice) and unwinds the
    /// body with ExitCode(128+sig), the same channel `exit` uses.
    /// Anything else — trapped or ignored signals (no in-subshell trap
    /// delivery channel exists), stop/cont signals, mixed operand lists,
    /// spawn-realized levels (depth == origin, BASHPID is the real pid),
    /// and fork-copy executors (comsub/pipeline clones, which do not own
    /// the flat subshell frame) — keeps the legacy path; see REPORT.
    fn intercept_fake_bashpid_kill(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<Result<i32, ExecuteError>> {
        let depth = self.shell_state.subshell_depth.get();
        if depth <= self.bashpid_origin_depth || !self.owns_process_image {
            return None;
        }
        let request = kill_request(&cmd.words[1..])?;
        if request.operands.is_empty() {
            return None;
        }
        let fake_pid = self.bashpid_value();
        if !request
            .operands
            .iter()
            .all(|operand| operand.parse::<u32>() == Ok(fake_pid))
        {
            return None;
        }
        if request.check_only || request.signal == 0 {
            return Some(Ok(0));
        }
        // Default-fatal set (Linux numbers, the kill wire contract):
        // HUP INT QUIT ILL ABRT BUS FPE KILL USR1 SEGV USR2 PIPE ALRM
        // TERM STKFLT XCPU XFSZ VTALRM PROF SYS.
        if !matches!(
            request.signal,
            1 | 2
                | 3
                | 4
                | 6
                | 7
                | 8
                | 9
                | 10
                | 11
                | 12
                | 13
                | 14
                | 15
                | 16
                | 24
                | 25
                | 26
                | 27
                | 31
        ) {
            return None;
        }
        let name = super::trap_exec::signal_trap_name(request.signal)?;
        if crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, &name).is_some() {
            // Trapped (action) or explicitly ignored (empty action):
            // GNU would deliver to the layer and run its trap there;
            // that delivery channel does not exist in-process.
            return None;
        }
        self.subshell_signal_death = Some((depth, request.signal));
        Some(Err(ExecuteError::ExitCode(128 + request.signal)))
    }

    fn execute_tracked_background_kill(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<i32>, ExecuteError> {
        let Some(request) = kill_request(&cmd.words[1..]) else {
            return Ok(None);
        };
        if request.operands.is_empty() {
            return Ok(None);
        }

        let should_handle = request.operands.iter().any(|operand| {
            operand.starts_with('%')
                || operand
                    .parse::<u32>()
                    .ok()
                    .is_some_and(|pid| self.shell_state.job_table.pid_to_job.contains_key(&pid))
        });
        if !should_handle {
            let mut stderr = Vec::new();
            // GNU kill.def:87-268 counters — see the tracked loop below:
            // probe/delivery attempts decide any_succeeded/any_failed,
            // spec diagnostics feed neither.
            let mut any_succeeded = false;
            let mut any_failed = false;
            for operand in request.operands {
                let Some(pid) = operand.parse::<u32>().ok() else {
                    // Not a u32 pid or job spec — fall through to kill::execute_with_io
                    // for negative pids (Unix process groups) and other formats.
                    return Ok(None);
                };
                if request.check_only && pid == 0 {
                    // GNU kill(0, 0) probes the shell's own group and
                    // succeeds — it counts as a delivered probe.
                    any_succeeded = true;
                    continue;
                }
                if !process_exists(pid) {
                    writeln!(
                        stderr,
                        "{}kill: ({pid}) - No such process",
                        self.diagnostic_prefix()
                    )?;
                    any_failed = true;
                } else {
                    any_succeeded = true;
                }
            }
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            if any_failed || request.check_only {
                return Ok(Some(if self.posix_mode_enabled() {
                    i32::from(any_failed)
                } else {
                    i32::from(!any_succeeded)
                }));
            }
            return Ok(None);
        }

        let mut stderr = Vec::new();
        // GNU kill.def:87-268: only real kill_pid attempts move the
        // counters — any_succeeded for a delivered signal, any_failed for
        // a failed delivery (ESRCH etc.). Spec-layer diagnostics
        // (sh_badjob/sh_badpid/DUP_JOB via CONTINUE_OR_FAIL) touch
        // neither counter, so `kill -0 %1 %?dup` returns 0 while
        // `kill -0 %?dup` alone returns 1. The final status is
        // `any_succeeded ? 0 : 1` (posix: `any_failed == 0 ? 0 : 1`).
        let mut any_succeeded = false;
        let mut any_failed = false;
        for operand in request.operands {
            // GNU kill.def:262 + jobs.c:3961-4007 (kill_pid): a jobspec
            // operand is delivered by WALKING the job's pipeline from
            // its first process, not by signaling one representative
            // pid. The walk signals live members in pipeline order and
            // stops at the first member whose death the table has
            // already recorded (jobs.c:3999-4000's PALIVE skip is a
            // `continue` that never advances the cursor: a dead head
            // ends the walk at once, and a dead later member leaves
            // GNU spinning on that member — its delivery set is the
            // live prefix either way, and the hang itself is a GNU
            // defect that is not cloned). The per-member kill(2)
            // results never fail the operand, so a retained job whose
            // members all died unwaited reports success with no
            // diagnostic, for real signals and for the -0 probe alike.
            // Collapsing the spec to the job's last pid and judging
            // that pid by raw kill(2) reported ESRCH for exactly that
            // retained-dead job (rubash#2381). Resolve the job entry
            // here; classification diagnostics (ambiguous / no such
            // job) stay on the shared path below.
            if operand.starts_with('%') {
                if let crate::jobs::table::JobspecMatch::Job(job_id) =
                    self.shell_state.job_table.resolve_jobspec(&operand)
                {
                    // The walk is GNU's `pgrp == shell_pgrp` arm. A
                    // job-control job (own pgrp) takes the killpg arm
                    // in GNU (jobs.c:4010-4020), whose rubash model is
                    // the shared path below — leave those jobs to it.
                    let members: Vec<(u32, bool)> = self
                        .shell_state
                        .job_table
                        .jobs
                        .get(&job_id)
                        .filter(|entry| !entry.job_control)
                        .map(|entry| {
                            entry
                                .pids
                                .iter()
                                .map(|member| {
                                    let recorded_completed =
                                        entry.processes.get(member).is_some_and(|process| {
                                            process.state == crate::jobs::ProcessState::Completed
                                        }) || self
                                            .shell_state
                                            .job_table
                                            .completed_statuses
                                            .contains_key(member);
                                    // A procsub placeholder's status is
                                    // recorded at registration while its
                                    // forked zombie stays unreaped and
                                    // signallable (rubash#1022) — it is
                                    // not a dead member.
                                    let dead = recorded_completed
                                        && !self
                                            .shell_state
                                            .job_table
                                            .procsub_placeholders
                                            .contains(member);
                                    (*member, dead)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if !members.is_empty() {
                        if request.check_only {
                            // The walk's kill(pid, 0) results are
                            // ignored (jobs.c:4001-4004): the retained
                            // job's existence is the whole verdict.
                            any_succeeded = true;
                            continue;
                        }
                        let mut operand_failed = false;
                        for (member, dead) in members {
                            if dead {
                                // The walk stops at the first
                                // recorded-dead member (see above):
                                // later members are never reached.
                                break;
                            }
                            let signal_args =
                                vec![format!("-{}", request.signal), member.to_string()];
                            let mut signal_stderr = Vec::new();
                            let signal_status = crate::builtins::kill::execute_with_io(
                                &signal_args,
                                &mut std::io::sink(),
                                &mut signal_stderr,
                            )?;
                            if signal_status != 0 {
                                if !process_exists(member) {
                                    // Died between the snapshot and
                                    // the delivery — the same skip as
                                    // a recorded death.
                                    continue;
                                }
                                operand_failed = true;
                                stderr.extend(signal_stderr);
                                continue;
                            }
                            self.note_kill_delivery(member, request.signal);
                        }
                        if operand_failed {
                            any_failed = true;
                        } else {
                            any_succeeded = true;
                        }
                        continue;
                    }
                }
            }
            let (pid, tracked_job) = match self.resolve_background_job_operand(&operand) {
                super::job_builtins::JobOperandResolution::Pid(pid) => (pid, true),
                super::job_builtins::JobOperandResolution::Ambiguous => {
                    // GNU kill.def:226-232 + builtins/common.c:672-679:
                    // DUP_JOB is INVALID_JOB, but kill skips sh_badjob
                    // for it — the get_job_by_name diagnostic is the
                    // whole report ("kill: name: ambiguous job spec").
                    writeln!(
                        stderr,
                        "{}kill: {}: ambiguous job spec",
                        self.diagnostic_prefix(),
                        super::job_builtins::jobspec_match_name(&operand)
                    )?;
                    continue;
                }
                super::job_builtins::JobOperandResolution::NoMatch => {
                    if operand.starts_with('%') {
                        // kill.def:227-229 sh_badjob — message only.
                        writeln!(
                            stderr,
                            "{}kill: {operand}: no such job",
                            self.diagnostic_prefix()
                        )?;
                        continue;
                    }
                    match operand.parse::<u32>() {
                        // kill.def:194-216: a numeric operand takes the
                        // pid path even outside the jobs table — the
                        // delivery below decides the counters.
                        Ok(pid) => (pid, false),
                        Err(_) => {
                            if operand.parse::<i64>().is_ok() {
                                // A signed numeric operand is still a
                                // pid operand (kill.def:194-216): a
                                // negative value names a process group
                                // and takes a real kill(2) attempt, whose
                                // failure reports `({pid}) - No such
                                // process` and moves any_failed — it is
                                // NOT a badpid. Delegate this operand to
                                // the builtin (owner of group delivery)
                                // and fold its verdict into the counters.
                                // rubash#1631 (`kill -n 10 -15 $job`).
                                let signal_args =
                                    vec![format!("-{}", request.signal), operand.clone()];
                                let mut signed_stderr = Vec::new();
                                let signed_status = crate::builtins::kill::execute_with_io(
                                    &signal_args,
                                    &mut std::io::sink(),
                                    &mut signed_stderr,
                                )?;
                                stderr.extend(signed_stderr);
                                if signed_status != 0 {
                                    any_failed = true;
                                } else {
                                    any_succeeded = true;
                                }
                                continue;
                            }
                            // kill.def:205-216 sh_badpid — message only.
                            writeln!(
                                stderr,
                                "{}kill: `{operand}': not a pid or valid job spec",
                                self.diagnostic_prefix()
                            )?;
                            continue;
                        }
                    }
                }
            };

            if request.check_only {
                // GNU kill -0 is a real liveness probe (kill(2) with sig 0
                // returns ESRCH for a dead pid), not an unconditional yes.
                // A pid the job table already marked Completed is dead even
                // before the process object is reaped — except a live
                // procsub placeholder (rubash#1022): its status is recorded
                // at registration while the forked zombie stays
                // unreaped (signallable) until an explicit wait or sweep,
                // exactly like GNU's just-exited procsub.
                let job_dead = self
                    .shell_state
                    .job_table
                    .completed_statuses
                    .contains_key(&pid)
                    && !self
                        .shell_state
                        .job_table
                        .procsub_placeholders
                        .contains(&pid);
                if job_dead || !process_exists(pid) {
                    writeln!(
                        stderr,
                        "{}kill: ({pid}) - No such process",
                        self.diagnostic_prefix()
                    )?;
                    any_failed = true;
                } else {
                    any_succeeded = true;
                }
                continue;
            }

            let signal_args = vec![format!("-{}", request.signal), pid.to_string()];
            let mut signal_stderr = Vec::new();
            let signal_status = crate::builtins::kill::execute_with_io(
                &signal_args,
                &mut std::io::sink(),
                &mut signal_stderr,
            )?;
            if signal_status != 0 {
                any_failed = true;
                stderr.extend(signal_stderr);
                continue;
            }
            any_succeeded = true;
            if !tracked_job {
                // A bare numeric pid outside the jobs table has no
                // job-table side effects (GNU kill_pid is a plain
                // signal delivery).
                continue;
            }
            self.note_kill_delivery(pid, request.signal);
            continue;
        }

        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        // kill.def:265-268: posixly_correct fails on any_failed, the
        // default mode on no successes at all — spec diagnostics feed
        // neither counter (rubash#1054's jobspec exemption is the same
        // rule, only stated for % specs).
        let status = if self.posix_mode_enabled() {
            i32::from(any_failed)
        } else {
            i32::from(!any_succeeded)
        };
        Ok(Some(status))
    }

    /// The job-table bookkeeping one successful signal delivery leaves
    /// behind, shared by the single-pid path and the jobspec pipeline
    /// walk (jobs.c:3988-4007) in `execute_tracked_background_kill`.
    fn note_kill_delivery(&mut self, pid: u32, signal: i32) {
        // GNU: the shell learns that a background job stopped from
        // waitchld's WUNTRACED report, which only exists under job
        // control — a non-monitor shell never notices, so `jobs` keeps
        // reporting the job as Running even after the shell's OWN kill
        // sent SIGSTOP (nojobs.c; rubash#1247). Rubash has no WUNTRACED
        // poller, so the kill path's marking is its only stop detector;
        // gate it on monitor mode or a `set +m` shell would report its
        // jobs Stopped where GNU reports Running.
        //
        // rubash#1494: GNU kill_pid is pure delivery — the waitchld
        // report lands at a notification point observably ONE reader
        // command later (an immediate `jobs` still shows the
        // pre-signal state and +/- marks; one intervening command
        // lets the new state appear). Defer the mark instead of
        // applying it synchronously inside the kill.
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor") {
            if signal == 18 {
                self.shell_state
                    .job_table
                    .defer_state_mark(pid, crate::jobs::table::DeferredJobState::Running);
            } else if matches!(signal, 19 | 20) {
                self.shell_state.job_table.defer_state_mark(
                    pid,
                    crate::jobs::table::DeferredJobState::Stopped(Some(signal)),
                );
            }
        }
        // A terminating signal changes nothing in the table here:
        // the boundary reap marks the job Completed when the process
        // actually dies. rubash#796 used to mark + remove the entry
        // at delivery time for the DONT_REPORT signals (TERM/INT/
        // PIPE), conflating the suppressed reader-boundary NOTICE
        // (report_pending_signal_notices already stays silent for
        // background TERM deaths) with the job's lifetime — GNU
        // keeps the job, and the next `jobs` lists it once as
        // `Terminated` before cleaning it up (rubash#1256). The old
        // code also lied for a process that ignores TERM: it
        // vanished from the table while still running.
        //
        // rubash#1604: the one piece of bookkeeping a terminating
        // signal DOES leave here is the delivery record itself.
        // If the process dies at once, the post-kill refresh's
        // incidental sweep must not apply the completion before a
        // following `wait` can reap the job itself (a waited-for
        // job's notice is suppressed — jobs.c wait_for); the
        // refresh consults this record and stashes the reap for
        // the rest of this reader command and the next one (the
        // incidental reap lands at the next command's
        // pre-execution sweep — see JobTable::note_reader_command).
        // Signals whose default
        // disposition is stop/continue/ignore (CONT, the stop
        // family, CHLD, URG, WINCH) and the 0 probe never kill,
        // so they leave no record.
        if !matches!(signal, 0 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 28) {
            self.shell_state.job_table.note_fatal_signal(pid);
        }
    }

    pub(in crate::executor) fn execute_ulimit(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::ulimit::execute_with_io(
            &cmd.words[1..],
            &mut self.shell_state.env_vars,
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    crate::builtins::kill::process_exists(pid)
}

#[cfg(windows)]
fn process_exists(pid: u32) -> bool {
    crate::builtins::kill::process_exists(pid)
}

#[cfg(not(any(unix, windows)))]
fn process_exists(_pid: u32) -> bool {
    false
}

struct KillRequest {
    operands: Vec<String>,
    signal: i32,
    check_only: bool,
}

fn kill_request(words: &[String]) -> Option<KillRequest> {
    let mut index = 0;
    let mut signal = 15;
    let mut check_only = false;
    // GNU builtins/kill.def option loop (kill_builtin): only the FIRST
    // signal specification is consumed as an option — the bare -SIG
    // branch is gated on `saw_signal == 0` (kill.def: "We only process
    // the first one seen; other arguments may signify process groups
    // (e.g., -num == process group num)"). A later -SIG word ends the
    // scan and becomes an operand, judged by the operand loop (badpid
    // diagnostic for -TERM, process-group delivery for -15). The -s/-n
    // branches stay live after a bare spec (`kill -TERM -s USR1 pid`
    // still delivers USR1). rubash#1631: this pre-parser used to assign
    // every later -SIG over the earlier one, silently retargeting the
    // tracked-job delivery.
    let mut saw_signal = false;
    while let Some(word) = words.get(index) {
        if word == "--" {
            index += 1;
            break;
        }
        if word == "-l" || word == "--list" {
            return None;
        }
        if word == "-s" || word == "-n" {
            if words
                .get(index + 1)
                .is_none_or(|signal| crate::builtins::kill::translate_signal(signal).is_none())
            {
                return None;
            }
            let Some(sigspec) = words.get(index + 1) else {
                return None;
            };
            signal = crate::builtins::kill::signal_number_for_spec(sigspec)?;
            check_only |= signal == 0;
            saw_signal = true;
            index += 2;
            continue;
        }
        if word.starts_with('-') && word != "-" {
            if saw_signal {
                break;
            }
            // GNU strips exactly the one leading dash (sigspec = word+1);
            // trimming every dash would misread `--15`-style operands.
            let spec = &word[1..];
            if spec != "0" && crate::builtins::kill::translate_signal(spec).is_none() {
                return None;
            }
            signal = crate::builtins::kill::signal_number_for_spec(spec)?;
            check_only |= signal == 0;
            saw_signal = true;
            index += 1;
            continue;
        }
        break;
    }

    Some(KillRequest {
        operands: words[index..].to_vec(),
        signal,
        check_only,
    })
}
