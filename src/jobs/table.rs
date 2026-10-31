//! Bash-observable job identity and completion state.
//!
//! Windows process handles and waiting primitives stay in the executor/backend.

use std::collections::{BTreeMap, HashMap, HashSet};

pub type JobId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Stopped,
    Completed,
}

/// A job-state transition the shell has SENT (via its own `kill`) but
/// not yet LEARNED (rubash#1494/#1607/#1612). GNU's kill_pid is pure
/// signal delivery; the shell learns of the stop/continue only in
/// waitchld (jobs.c), which runs asynchronously from the SIGCHLD
/// handler and at notification points (parse.y read_command's
/// notify_and_cleanup, wait_for's blocking wait). There is no
/// per-command counter in GNU: the observable lag is signal-delivery
/// latency. In scripts a fast builtin-only sequence therefore still
/// shows the pre-signal state (and the old current/previous marks)
/// at the next command, while any real observation opportunity —
/// the shell blocking on a foreground process, job control active
/// on a terminal, or simply several milliseconds of wall time —
/// publishes the transition, including the stopped job's promotion
/// to current. Rubash has no waitchld/WUNTRACED poller for its own
/// kill, so the kill path records the transition here stamped with
/// the reader command sequence and the send instant, and the
/// per-command refresh applies it once one of those observation
/// events occurs (see apply_deferred_state_marks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferredJobState {
    Stopped(Option<i32>),
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub state: ProcessState,
    pub exit_status: Option<i32>,
    /// The signal that killed the process, when it died by one. GNU
    /// keeps WIFSIGNALED/WTERMSIG in the wait status, and the jobs
    /// listing prints the signal description ("Terminated") instead of
    /// `Exit 143` — the flattened 128+signal status alone cannot tell
    /// a signal death from a plain `exit 143` (rubash#1256).
    pub term_signal: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobEntry {
    pub id: JobId,
    pub pids: Vec<u32>,
    pub processes: BTreeMap<u32, ProcessEntry>,
    /// GNU jobs.c:649 `newjob->pgrp = pipeline_pgrp` — the process-group ID
    /// `jobs -x` substitutes (builtins/jobs.def execute_list_with_replacements
    /// `itos (j->pgrp)`). With job control it is the first pipeline pid;
    /// without job control every job shares the shell's own pgrp
    /// (jobs.c:2459-2461 `pipeline_pgrp = shell_pgrp`).
    pub pgrp: u32,
    pub command: String,
    /// GNU jobs.c: each PROCESS in `j->pipe` carries its own printed
    /// command text, and builtins/common.c:632 get_job_by_name matches
    /// `%name`/`%?name` against those per-stage strings — never against
    /// the joined `command`. For a pipeline `a | b` this is `["a", "b"]`;
    /// for a plain command it is `[command]` verbatim.
    pub stage_commands: Vec<String>,
    pub state: ProcessState,
    pub exit_status: Option<i32>,
    /// GNU jobs.c: the WSTOPSIG of the process whose stop report made
    /// the job JSTOPPED — `jobs -l` renders it through
    /// printable_job_status/j_strsignal (`Stopped (signal)` for
    /// SIGSTOP, `Stopped (tty input)` for SIGTTIN, ...; rubash#1246).
    /// Recorded by the kill path, the shell's only stop detector.
    pub stop_signal: Option<i32>,
    /// Killing signal of the job's last process, mirroring
    /// ProcessEntry::term_signal at job granularity (rubash#1256).
    pub term_signal: Option<i32>,
    pub background: bool,
    pub foreground: bool,
    pub notified: bool,
    /// GNU jobs.h J_NOHUP: set by `disown -h` (jobs.c:1548 nohup_job) —
    /// the job stays in the table; the flag only suppresses the SIGHUP
    /// the shell would otherwise send when it receives one itself.
    pub nohup: bool,
    pub coproc_endpoints: Vec<u32>,
    /// GNU jobs.c J_JOBCONTROL: whether the job was started while job
    /// control (`set -m`) was active. fg/bg must refuse jobs without it
    /// (fg_bg.def:159 "job %d started without job control").
    pub job_control: bool,
}

/// GNU builtins/common.c:689 get_job_spec result, with jobs.h:206-208
/// mapped onto variants: `Job` is a valid slot, `Ambiguous` is DUP_JOB
/// (get_job_by_name counted more than one match — the caller reports
/// `{name}: ambiguous job spec`), `NoMatch` is NO_JOB/BAD_JOBSPEC
/// (the caller reports `{spec}: no such job`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobspecMatch {
    Job(JobId),
    Ambiguous,
    NoMatch,
}

#[derive(Debug, Default, Clone)]
pub struct JobTable {
    pub jobs: BTreeMap<JobId, JobEntry>,
    pub pid_to_job: HashMap<u32, JobId>,
    pub completed_statuses: HashMap<u32, i32>,
    /// rubash#1022: pids of forked process-substitution placeholders (real
    /// OS children, born zombies) whose recorded status lives in
    /// completed_statuses. Membership means "signallable but unreaped":
    /// kill -0 succeeds (the zombie exists), an explicit `wait` or the bare
    /// `wait` reaps it while the recorded status stays for repeat waits
    /// (bgpids parity). The per-command refresh deliberately does not reap:
    /// with /dev/fd pipes GNU's output-procsub children outlive the
    /// creating command, so older `$!` holders stay signallable until a
    /// wait consumes them. Synthetic fallback pids (non-unix / fork
    /// failure) never enter.
    pub procsub_placeholders: HashSet<u32>,
    current_job: Option<JobId>,
    previous_job: Option<JobId>,
    /// Monotonic count of top-level (reader) commands started; the
    /// executor bumps it once per command at depth 1 (rubash#1494).
    pub reader_command_seq: u64,
    /// Stop/continue transitions sent by the shell's own `kill`,
    /// stamped with `reader_command_seq` and the send instant,
    /// awaiting an observation event (rubash#1494, see
    /// [`DeferredJobState`]).
    pub deferred_state_marks: Vec<(u32, DeferredJobState, u64, std::time::Instant)>,
    /// Pids the shell's own `kill` sent a terminating signal to
    /// during the current reader command, stamped with
    /// `reader_command_seq` (rubash#1604): an incidental reap of one
    /// of these must not publish its death notice before a `wait`
    /// has had its chance to reap the job itself.
    pub fresh_fatal_marks: Vec<(u32, u64)>,
}

impl JobTable {
    pub fn register_process(
        &mut self,
        pid: u32,
        command: impl Into<String>,
        background: bool,
        pgrp: u32,
    ) -> JobId {
        let command = command.into();
        let stage_commands = vec![command.clone()];
        self.register_pipeline_stages(vec![pid], command, background, pgrp, stage_commands)
    }

    /// `register_process` plus the per-stage printed commands GNU records
    /// on `j->pipe` (builtins/common.c:632 get_job_by_name matches them).
    /// Pass the output of `bash_job_stage_commands`; a plain command
    /// contributes just its own text.
    pub fn register_process_with_stages(
        &mut self,
        pid: u32,
        command: impl Into<String>,
        background: bool,
        pgrp: u32,
        stage_commands: Vec<String>,
    ) -> JobId {
        self.register_pipeline_stages(vec![pid], command, background, pgrp, stage_commands)
    }

    pub fn register_pipeline(
        &mut self,
        pids: Vec<u32>,
        command: impl Into<String>,
        background: bool,
        pgrp: u32,
    ) -> JobId {
        let command = command.into();
        let stage_commands = vec![command.clone()];
        self.register_pipeline_stages(pids, command, background, pgrp, stage_commands)
    }

    pub fn register_pipeline_stages(
        &mut self,
        pids: Vec<u32>,
        command: impl Into<String>,
        background: bool,
        pgrp: u32,
        stage_commands: Vec<String>,
    ) -> JobId {
        // GNU jobs.c:586-614 alloc_job_entry: a non-interactive shell scans
        // forward from js.j_lastj and takes the first free slot — the job
        // number is the slot index, which keeps holes open for jobs that
        // were reaped earlier and is only lowered again when the tail of
        // the table is freed (delete_job/cleanup_dead_jobs recompute
        // j_lastj, jobs.c:1270-1292). The next id is therefore
        // highest-occupied-id + 1, NOT a monotonic counter and NOT the
        // lowest free slot: with {1,3} live, the next job is %4; once %3
        // is removed the next is %2.
        let id = self
            .jobs
            .keys()
            .next_back()
            .map_or(1, |highest| highest.saturating_add(1));
        let processes = pids
            .iter()
            .copied()
            .map(|pid| {
                (
                    pid,
                    ProcessEntry {
                        pid,
                        state: ProcessState::Running,
                        exit_status: None,
                        term_signal: None,
                    },
                )
            })
            .collect();
        let command = command.into();
        // get_job_by_name needs one entry per pipeline stage (GNU's
        // j->pipe PROCESS list); a caller that cannot describe them keeps
        // the joined text so the list is never empty.
        let stage_commands = if stage_commands.is_empty() {
            vec![command.clone()]
        } else {
            stage_commands
        };
        let entry = JobEntry {
            id,
            pids: pids.clone(),
            // GNU jobs.c:649 `newjob->pgrp = pipeline_pgrp`, recorded at
            // fork time by the caller (first pipeline pid with job
            // control, else the shell's own pgrp).
            pgrp,
            processes,
            command,
            stage_commands,
            state: ProcessState::Running,
            exit_status: None,
            stop_signal: None,
            term_signal: None,
            background,
            foreground: !background,
            notified: false,
            nohup: false,
            coproc_endpoints: Vec::new(),
            job_control: false,
        };
        for pid in pids {
            self.pid_to_job.insert(pid, id);
        }
        self.jobs.insert(id, entry);
        self.set_current_job(id);
        id
    }

    /// GNU builtins/common.c:689 get_job_spec. A name spec (`%name` /
    /// `%?name`) goes through get_job_by_name (common.c:632), which
    /// COUNTS matches and reports DUP_JOB (`Ambiguous` here) when more
    /// than one fires.
    pub fn resolve_jobspec(&self, spec: &str) -> JobspecMatch {
        // common.c:704-712: strip the leading `%` (a word without one
        // takes the same path after the caller's
        // "job specification requires leading `%'" warning).
        let body = spec.strip_prefix('%').unwrap_or(spec);
        // common.c:714-718: `DIGIT (*word) && all_digits (word)` — a
        // pure digit spec is a slot index; out-of-range or empty slots
        // are NO_JOB. The map-key check subsumes both tests.
        if !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit()) {
            return match body.parse::<JobId>() {
                Ok(id) if self.jobs.contains_key(&id) => JobspecMatch::Job(id),
                _ => JobspecMatch::NoMatch,
            };
        }
        // common.c:720-738: the switch is on the FIRST byte, so `%+x`
        // is the current job and `%-x` the previous one.
        match body.chars().next() {
            None | Some('%') | Some('+') => self
                .current_job
                .or_else(|| self.jobs.keys().next_back().copied())
                .map_or(JobspecMatch::NoMatch, JobspecMatch::Job),
            Some('-') => self
                .previous_job
                .map_or(JobspecMatch::NoMatch, JobspecMatch::Job),
            Some('?') => self.get_job_by_name(&body[1..], true),
            Some(_) => self.get_job_by_name(body, false),
        }
    }

    /// GNU builtins/common.c:632 get_job_by_name as get_job_spec drives
    /// it (never JM_FIRSTMATCH/JM_EXACT/JM_STOPPED). Jobs scan
    /// newest-first (`i = js.j_jobslots - 1; i >= 0; i--`); each job's
    /// PROCESS commands scan from `j->pipe`. The counting is quirky by
    /// construction: the matched `p` is NOT advanced before `job` is
    /// assigned, so a first match on any stage other than `j->pipe`
    /// (`index != 0`) loops back onto the same process and is
    /// ambiguous with itself, while a match on `j->pipe` counts one.
    /// `%?name` is strcasestr (case-insensitive); a bare `%name` is an
    /// STREQN prefix on the stage text.
    fn get_job_by_name(&self, name: &str, substring: bool) -> JobspecMatch {
        let mut job = None;
        let folded = name.to_ascii_lowercase();
        for (id, entry) in self.jobs.iter().rev() {
            for (index, command) in entry.stage_commands.iter().enumerate() {
                let matched = if substring {
                    command.to_ascii_lowercase().contains(&folded)
                } else {
                    command.starts_with(name)
                };
                if !matched {
                    continue;
                }
                if job.is_some() || index != 0 {
                    return JobspecMatch::Ambiguous;
                }
                job = Some(*id);
                break;
            }
        }
        job.map_or(JobspecMatch::NoMatch, JobspecMatch::Job)
    }

    pub fn current_job(&self) -> Option<JobId> {
        self.current_job
    }

    pub fn previous_job(&self) -> Option<JobId> {
        self.previous_job
    }

    /// GNU jobs.c:3668 most_recent_job_in_state: newest job with an id
    /// below `below` in the requested state.
    fn most_recent_in_state(&self, below: JobId, state: ProcessState) -> Option<JobId> {
        self.jobs
            .range(..below)
            .rev()
            .find(|(_, job)| job.state == state)
            .map(|(id, _)| *id)
    }

    /// GNU jobs.c:3705-3757 set_current_job: JOB becomes current; previous
    /// is (1) the old current if still a stopped job, (2) the newest
    /// stopped job older than current when current is stopped, (3) the
    /// newest running job older than current (or newest overall when
    /// current is stopped), (4) current itself when it is the only job.
    pub fn set_current_job(&mut self, id: JobId) {
        if self.current_job != Some(id) {
            self.previous_job = self.current_job;
            self.current_job = Some(id);
        }
        let Some(current) = self.current_job else {
            return;
        };
        if self.previous_job != Some(current)
            && self
                .previous_job
                .and_then(|prev| self.jobs.get(&prev))
                .is_some_and(|job| job.state == ProcessState::Stopped)
        {
            return;
        }
        let current_stopped = self
            .jobs
            .get(&current)
            .is_some_and(|job| job.state == ProcessState::Stopped);
        if current_stopped {
            if let Some(candidate) = self.most_recent_in_state(current, ProcessState::Stopped) {
                self.previous_job = Some(candidate);
                return;
            }
        }
        let running_candidate = if !current_stopped {
            self.most_recent_in_state(current, ProcessState::Running)
        } else {
            self.jobs
                .iter()
                .rev()
                .find(|(_, job)| job.state == ProcessState::Running)
                .map(|(id, _)| *id)
        };
        match running_candidate {
            Some(candidate) => self.previous_job = Some(candidate),
            None => self.previous_job = Some(current),
        }
    }

    /// GNU jobs.c:3759-3797 reset_current: recompute current/previous after
    /// jobs die or are deleted — current stays if it is a stopped job, else
    /// the stopped previous, else the newest stopped, else the newest
    /// running job; NO_JOB when the table is empty.
    pub fn reset_current(&mut self) {
        let current_stopped = self
            .current_job
            .and_then(|id| self.jobs.get(&id))
            .is_some_and(|job| job.state == ProcessState::Stopped);
        let candidate = if current_stopped {
            self.current_job
        } else {
            let previous_stopped = self
                .previous_job
                .and_then(|id| self.jobs.get(&id))
                .is_some_and(|job| job.state == ProcessState::Stopped);
            if previous_stopped {
                self.previous_job
            } else {
                self.most_recent_in_state(JobId::MAX, ProcessState::Stopped)
                    .or_else(|| self.most_recent_in_state(JobId::MAX, ProcessState::Running))
            }
        };
        match candidate {
            Some(id) => self.set_current_job(id),
            None => {
                self.current_job = None;
                self.previous_job = None;
            }
        }
    }

    pub fn mark_completed(&mut self, pid: u32, status: i32) {
        self.completed_statuses.insert(pid, status);
        let Some(job_id) = self.pid_to_job.get(&pid).copied() else {
            return;
        };
        let Some(job) = self.jobs.get_mut(&job_id) else {
            return;
        };
        if let Some(process) = job.processes.get_mut(&pid) {
            process.state = ProcessState::Completed;
            process.exit_status = Some(status);
        }
        // jobs.c:2219 contract — waitchld clears J_NOTIFIED on a state
        // change so the next `jobs -n` reports this job again.
        job.notified = false;
        self.recompute_job(job_id);
    }

    /// GNU jobs.c J_JOBCONTROL flag, recorded at spawn: fg/bg refuse jobs
    /// started without job control (fg_bg.def:159).
    pub fn set_job_control(&mut self, pid: u32, job_control: bool) {
        if let Some(job_id) = self.pid_to_job.get(&pid).copied() {
            if let Some(job) = self.jobs.get_mut(&job_id) {
                job.job_control = job_control;
            }
        }
    }

    pub fn job_control_for_pid(&self, pid: u32) -> bool {
        self.pid_to_job
            .get(&pid)
            .and_then(|job_id| self.jobs.get(job_id))
            .is_some_and(|job| job.job_control)
    }

    pub fn job_id_for_pid(&self, pid: u32) -> Option<JobId> {
        self.pid_to_job.get(&pid).copied()
    }

    /// Record that a new top-level reader command has started
    /// (rubash#1494's notification-lag clock).
    pub fn note_reader_command(&mut self) {
        self.reader_command_seq += 1;
        // rubash#1604: fatal-signal marks guard the reader command
        // that sent them AND the one that follows it. The incidental
        // reap that must not publish lands at the FOLLOWING command's
        // pre-execution refresh (ast_exec runs the boundary sweep
        // before a command executes, not after the previous one), and
        // that following command is typically the `wait` itself —
        // its redirects are not installed yet at sweep time, so
        // applying the completion there prints the notice to raw
        // stderr, past the wait's `2>/dev/null`. After the second
        // advance an incidental reap publishes normally again.
        self.fresh_fatal_marks
            .retain(|(_, seq)| *seq + 1 >= self.reader_command_seq);
    }

    /// Queue a stop/continue transition sent by the shell's own `kill`
    /// for application at a later notification point (rubash#1494).
    pub fn defer_state_mark(&mut self, pid: u32, state: DeferredJobState) {
        // A newer signal to the same pid supersedes an unapplied older
        // one (STOP then CONT inside the lag window nets to Running).
        self.deferred_state_marks
            .retain(|(marked_pid, _, _, _)| *marked_pid != pid);
        self.deferred_state_marks.push((
            pid,
            state,
            self.reader_command_seq,
            std::time::Instant::now(),
        ));
    }

    /// Record that the shell's own `kill` delivered a terminating
    /// signal to `pid` during the current reader command
    /// (rubash#1604).
    pub fn note_fatal_signal(&mut self, pid: u32) {
        if !self
            .fresh_fatal_marks
            .iter()
            .any(|(marked, _)| *marked == pid)
        {
            self.fresh_fatal_marks.push((pid, self.reader_command_seq));
        }
    }

    /// True (once) when `pid` was fatally signalled by the shell's
    /// own `kill` during the current or the immediately preceding
    /// reader command (see note_reader_command for why the window
    /// spans the boundary); the mark is consumed so a later reap of
    /// a recycled pid cannot match it.
    pub fn take_fresh_fatal(&mut self, pid: u32) -> bool {
        let now = self.reader_command_seq;
        if let Some(position) = self
            .fresh_fatal_marks
            .iter()
            .position(|(marked, seq)| *marked == pid && *seq + 1 >= now)
        {
            self.fresh_fatal_marks.remove(position);
            true
        } else {
            false
        }
    }

    /// Publish deferred marks at an observation event
    /// (rubash#1607/#1612). The caller passes `observe_event` when
    /// this refresh is itself a GNU notification point for state:
    /// a wait/fg observed refresh, the shell having blocked on a
    /// foreground process since the last refresh, or job control
    /// active on a terminal (under a pty GNU's cadence publishes on
    /// the very next command). Without such an event a mark
    /// publishes only once it is at least one reader command old
    /// AND several milliseconds of wall time have passed since the
    /// signal was sent — the SIGCHLD-delivery latency GNU exhibits
    /// across a busy stretch of builtins (a ~50ms builtin loop
    /// publishes; a microsecond `echo` between kill and `jobs`
    /// does not). The old `seq + 1 < now` counting rule published
    /// one command too early for that microsecond case (#1607) and
    /// never published at all where the reader's cadence differs
    /// (#1612).
    pub fn apply_deferred_state_marks(&mut self, observe_event: bool) {
        const AGED: std::time::Duration = std::time::Duration::from_millis(5);
        let now = self.reader_command_seq;
        let mut pending = std::mem::take(&mut self.deferred_state_marks);
        let mut ready = Vec::new();
        pending.retain(|(pid, state, seq, sent)| {
            if observe_event || (*seq < now && sent.elapsed() >= AGED) {
                ready.push((*pid, *state));
                false
            } else {
                true
            }
        });
        self.deferred_state_marks = pending;
        for (pid, state) in ready {
            match state {
                DeferredJobState::Stopped(signal) => self.mark_stopped(pid, signal),
                DeferredJobState::Running => self.mark_running(pid),
            }
        }
    }

    pub fn mark_stopped(&mut self, pid: u32, stop_signal: Option<i32>) {
        let Some(job_id) = self.pid_to_job.get(&pid).copied() else {
            return;
        };
        if let Some(job) = self.jobs.get_mut(&job_id) {
            if let Some(process) = job.processes.get_mut(&pid) {
                process.state = ProcessState::Stopped;
            }
            job.stop_signal = stop_signal;
            job.notified = false;
        }
        self.recompute_job(job_id);
        // GNU jobs.c waitchld: a job that has just stopped becomes the
        // current job.
        self.set_current_job(job_id);
    }

    pub fn mark_running(&mut self, pid: u32) {
        let Some(job_id) = self.pid_to_job.get(&pid).copied() else {
            return;
        };
        if let Some(job) = self.jobs.get_mut(&job_id) {
            if let Some(process) = job.processes.get_mut(&pid) {
                process.state = ProcessState::Running;
            }
            job.stop_signal = None;
        }
        // jobs.c start_job/set_job_running: a Running transition does not
        // pick the job as current. Callers decide: fg -> set_current_job,
        // bg -> reset_current.
        self.recompute_job(job_id);
    }

    fn recompute_job(&mut self, job_id: JobId) {
        let Some(job) = self.jobs.get_mut(&job_id) else {
            return;
        };
        let any_running = job
            .processes
            .values()
            .any(|p| p.state == ProcessState::Running);
        let any_stopped = job
            .processes
            .values()
            .any(|p| p.state == ProcessState::Stopped);
        job.state = if any_running {
            ProcessState::Running
        } else if any_stopped {
            ProcessState::Stopped
        } else {
            ProcessState::Completed
        };
        job.exit_status = job
            .pids
            .last()
            .and_then(|pid| job.processes.get(pid))
            .and_then(|process| process.exit_status);
        job.term_signal = job
            .pids
            .last()
            .and_then(|pid| job.processes.get(pid))
            .and_then(|process| process.term_signal);
    }

    /// Record the signal that killed a process (taken from the raw
    /// wait status at a reap site), so the jobs listing can print the
    /// signal description for the job (rubash#1256).
    pub fn note_term_signal(&mut self, pid: u32, signal: i32) {
        let Some(job_id) = self.pid_to_job.get(&pid).copied() else {
            return;
        };
        let Some(job) = self.jobs.get_mut(&job_id) else {
            return;
        };
        if let Some(process) = job.processes.get_mut(&pid) {
            process.term_signal = Some(signal);
        }
        self.recompute_job(job_id);
    }

    /// jobs.c:1303 cleanup_dead_jobs — delete dead jobs the user was
    /// already notified about (POSIX: a terminated job leaves the list once
    /// `jobs` reports it). Completed statuses stay in `completed_statuses`
    /// (the bgpids equivalent) so a later operand-addressed `wait $pid`
    /// still reports the exit status.
    pub fn cleanup_dead_jobs(&mut self) {
        let dead_notified: Vec<JobId> = self
            .jobs
            .iter()
            .filter(|(_, job)| job.state == ProcessState::Completed && job.notified)
            .map(|(job_id, _)| *job_id)
            .collect();
        let mut removed_current_or_previous = false;
        for job_id in dead_notified {
            removed_current_or_previous |=
                Some(job_id) == self.current_job || Some(job_id) == self.previous_job;
            if let Some(job) = self.jobs.remove(&job_id) {
                for pid in &job.pids {
                    self.pid_to_job.remove(pid);
                }
            }
        }
        // GNU delete_job (jobs.c:1535-1543): current/previous are reset only
        // when the deleted job held one of the markers; deleting any other
        // job leaves them alone.
        if removed_current_or_previous {
            self.reset_current();
        }
    }

    /// jobs.c:3434-3437 (wait_for_job) and jobs.c:2776-2779
    /// (wait_for_single_pid): a job whose wait just completed is marked
    /// notified but NOT deleted — POSIX.2 permits removing it, and bash
    /// instead keeps the entry until the next cleanup_dead_jobs pass
    /// (inside a later wait's notify_and_cleanup, a `jobs` listing, or a
    /// job creation). rubash#2340.
    pub fn mark_job_notified_by_pid(&mut self, pid: u32) {
        if let Some(job_id) = self.pid_to_job.get(&pid).copied() {
            if let Some(job) = self.jobs.get_mut(&job_id) {
                job.notified = true;
            }
        }
    }

    /// The bare-`wait` endgame: jobs.c:2863 wait_for_background_pids calls
    /// mark_dead_jobs_as_notified(1) and then cleanup_dead_jobs. The force
    /// pass (jobs.c:5192-5201) marks every dead job notified EXCEPT — in a
    /// non-interactive shell — the job whose last pid is
    /// last_asynchronous_pid ($!): that entry survives the wait and stays
    /// listed until the next notification point. Interactive shells mark
    /// all dead jobs (force_all). rubash#2340.
    pub fn settle_bare_wait(&mut self, last_asynchronous_pid: Option<u32>, force_all: bool) {
        let exempt = if force_all {
            None
        } else {
            last_asynchronous_pid
        };
        for job in self.jobs.values_mut() {
            if job.state != ProcessState::Completed {
                continue;
            }
            if exempt.is_some() && job.pids.last().copied() == exempt {
                continue;
            }
            job.notified = true;
        }
        self.cleanup_dead_jobs();
    }

    /// jobs.c:3652 reap_dead_jobs — non-interactive shells (and shells
    /// without job control) remove dead jobs without printing:
    /// mark_dead_jobs_as_notified + cleanup_dead_jobs. GNU calls this via
    /// the REAP() macro after every loop body (execute_cmd.c:2979,
    /// for/select/while/until/arith-for) and from compact_jobs_list.
    ///
    /// jobs.c:5179-5230 mark_dead_jobs_as_notified keeps CHILD_MAX
    /// (DEFAULT_CHILD_MAX 4096, jobs.c:92) dead processes unnotified so
    /// `wait` can still report their statuses, and never marks the job
    /// holding last_asynchronous_pid ($!) — so REAP is a no-op for any
    /// realistic script unless a listing already reported the job.
    pub fn reap_dead_jobs(&mut self, last_asynchronous_pid: Option<u32>) {
        const CHILD_MAX: usize = 4096;
        let dead_processes: usize = self
            .jobs
            .values()
            .filter(|job| job.state == ProcessState::Completed)
            .map(|job| job.pids.len())
            .sum();
        if dead_processes > CHILD_MAX {
            let mut keep = dead_processes;
            for job in self.jobs.values_mut() {
                if job.state != ProcessState::Completed {
                    continue;
                }
                if keep <= CHILD_MAX {
                    break;
                }
                if job.pids.last().copied() == last_asynchronous_pid {
                    continue;
                }
                job.notified = true;
                keep = keep.saturating_sub(job.pids.len());
            }
        }
        self.cleanup_dead_jobs();
    }

    pub fn reap_finished<I>(&mut self, statuses: I)
    where
        I: IntoIterator<Item = (u32, i32)>,
    {
        for (pid, status) in statuses {
            self.mark_completed(pid, status);
        }
    }

    pub fn wait_pid(&mut self, pid: u32) -> Option<i32> {
        self.completed_statuses.remove(&pid).or_else(|| {
            let job_id = self.pid_to_job.get(&pid).copied()?;
            self.jobs.get(&job_id)?.exit_status
        })
    }

    pub fn wait_any(&mut self) -> Option<(u32, i32)> {
        let pid = self.completed_statuses.keys().next().copied()?;
        let status = self.completed_statuses.remove(&pid)?;
        Some((pid, status))
    }

    pub fn wait_all(&mut self) -> Vec<(u32, i32)> {
        let mut result: Vec<_> = self.completed_statuses.drain().collect();
        result.sort_by_key(|(pid, _)| *pid);
        result
    }

    pub fn remove_job(&mut self, job_id: JobId) -> bool {
        let Some(job) = self.jobs.remove(&job_id) else {
            return false;
        };
        for pid in &job.pids {
            self.pid_to_job.remove(pid);
            self.completed_statuses.remove(pid);
        }
        // jobs.c:1541-1543 delete_job: reset_current only when the removed
        // job held the current or previous marker.
        if Some(job_id) == self.current_job || Some(job_id) == self.previous_job {
            self.reset_current();
        }
        true
    }

    pub fn remove_job_by_pid(&mut self, pid: u32) -> bool {
        self.pid_to_job
            .get(&pid)
            .copied()
            .is_some_and(|job_id| self.remove_job(job_id))
    }

    pub fn remove_job_by_pid_preserve_status(&mut self, pid: u32) -> bool {
        let Some(job_id) = self.pid_to_job.get(&pid).copied() else {
            return false;
        };
        let Some(job) = self.jobs.remove(&job_id) else {
            return false;
        };
        for candidate in &job.pids {
            self.pid_to_job.remove(candidate);
        }
        if Some(job_id) == self.current_job || Some(job_id) == self.previous_job {
            self.reset_current();
        }
        true
    }

    pub fn clear_jobs(&mut self) {
        self.jobs.clear();
        self.pid_to_job.clear();
        self.completed_statuses.clear();
        self.current_job = None;
        self.previous_job = None;
    }

    pub fn attach_coproc_endpoint(&mut self, job_id: JobId, fd: u32) {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.coproc_endpoints.push(fd);
        }
    }

    pub fn take_coproc_endpoint(&mut self, job_id: JobId, fd: u32) -> bool {
        self.jobs.get_mut(&job_id).map_or(false, |job| {
            let before = job.coproc_endpoints.len();
            job.coproc_endpoints.retain(|candidate| *candidate != fd);
            before != job.coproc_endpoints.len()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_pgrp_is_recorded_for_jobs_x_substitution() {
        // GNU jobs.c:649 `newjob->pgrp = pipeline_pgrp`: `jobs -x` substitutes
        // the recorded pgrp (builtins/jobs.def execute_list_with_replacements),
        // not the job pid (rubash#1096).
        let mut table = JobTable::default();
        let job = table.register_pipeline(vec![10, 11], "producer | consumer", true, 99);
        assert_eq!(table.jobs[&job].pgrp, 99);
        let solo = table.register_process(42, "sleep", true, 42);
        assert_eq!(table.jobs[&solo].pgrp, 42);
    }

    #[test]
    fn completion_can_be_waited_explicitly_after_reaping() {
        let mut table = JobTable::default();
        table.register_process(42, "sleep", true, 42);
        table.reap_finished([(42, 7)]);
        assert_eq!(table.wait_pid(42), Some(7));
    }

    #[test]
    fn jobspecs_resolve_in_job_order() {
        let mut table = JobTable::default();
        let first = table.register_process(1, "one", true, 1);
        let second = table.register_process(2, "two", true, 2);
        assert_eq!(table.resolve_jobspec("%+"), JobspecMatch::Job(second));
        assert_eq!(table.resolve_jobspec("%-"), JobspecMatch::Job(first));
        assert_eq!(table.resolve_jobspec("%1"), JobspecMatch::Job(first));
    }

    #[test]
    fn name_jobspecs_count_matches_like_get_job_by_name() {
        // GNU builtins/common.c:632-684: two jobs matching `%?pat` or
        // `%pat` is DUP_JOB, reported as `{name}: ambiguous job spec`;
        // a single match resolves newest-first (rubash#1302).
        let mut table = JobTable::default();
        table.register_process(1, "sleep 50", true, 1);
        let second = table.register_process(2, "sleep 51", true, 2);
        assert_eq!(table.resolve_jobspec("%?sleep"), JobspecMatch::Ambiguous);
        assert_eq!(table.resolve_jobspec("%sleep"), JobspecMatch::Ambiguous);
        // strcasestr — the `%?` form folds case.
        assert_eq!(table.resolve_jobspec("%?SLEEP"), JobspecMatch::Ambiguous);
        assert_eq!(table.resolve_jobspec("%?51"), JobspecMatch::Job(second));
        assert_eq!(table.resolve_jobspec("%?zzz"), JobspecMatch::NoMatch);
    }

    #[test]
    fn name_jobspec_match_beyond_first_stage_is_self_ambiguous() {
        // common.c:656-679: the matched `p` is not advanced before `job`
        // is checked, so a match on any stage other than j->pipe makes
        // the job ambiguous with itself (`sleep 50 | sleep 51` + `%?51`),
        // while a first-stage match counts once (`%?sleep` resolves).
        let mut table = JobTable::default();
        let job = table.register_process_with_stages(
            1,
            "sleep 50 | sleep 51",
            true,
            1,
            vec!["sleep 50".to_string(), "sleep 51".to_string()],
        );
        assert_eq!(table.resolve_jobspec("%?sleep"), JobspecMatch::Job(job));
        assert_eq!(table.resolve_jobspec("%?51"), JobspecMatch::Ambiguous);
        assert_eq!(table.resolve_jobspec("%sleep"), JobspecMatch::Job(job));
    }

    #[test]
    fn pipeline_state_waits_for_all_processes_and_uses_last_status() {
        let mut table = JobTable::default();
        let job = table.register_pipeline(vec![10, 11], "producer | consumer", true, 10);
        assert_eq!(table.jobs[&job].state, ProcessState::Running);
        table.mark_completed(10, 3);
        assert_eq!(table.jobs[&job].state, ProcessState::Running);
        table.mark_completed(11, 7);
        assert_eq!(table.jobs[&job].state, ProcessState::Completed);
        assert_eq!(table.jobs[&job].exit_status, Some(7));
    }

    #[test]
    fn stopped_and_continued_processes_update_job_state() {
        let mut table = JobTable::default();
        let job = table.register_process(20, "sleep", true, 20);
        table.mark_stopped(20, Some(19));
        assert_eq!(table.jobs[&job].state, ProcessState::Stopped);
        assert_eq!(table.jobs[&job].stop_signal, Some(19));
        table.mark_running(20);
        assert_eq!(table.jobs[&job].state, ProcessState::Running);
        assert_eq!(table.jobs[&job].stop_signal, None);
    }

    #[test]
    fn removing_a_job_clears_pid_and_current_previous_indexes() {
        let mut table = JobTable::default();
        let first = table.register_process(30, "first", true, 30);
        table.register_process(31, "second", true, 31);
        assert!(table.remove_job_by_pid(31));
        assert!(!table.pid_to_job.contains_key(&31));
        assert_eq!(table.resolve_jobspec("%+"), JobspecMatch::Job(first));
        assert_eq!(table.resolve_jobspec("%1"), JobspecMatch::Job(first));
        assert!(table.remove_job(first));
        assert!(table.jobs.is_empty());
    }

    #[test]
    fn wait_minus_removes_job_but_retains_explicit_pid_status() {
        let mut table = JobTable::default();
        let job = table.register_process(40, "false", true, 40);
        table.mark_completed(40, 1);
        assert!(table.remove_job_by_pid_preserve_status(40));
        assert!(!table.jobs.contains_key(&job));
        assert_eq!(table.completed_statuses.get(&40), Some(&1));
        assert_eq!(table.pid_to_job.get(&40), None);
        assert_eq!(table.wait_pid(40), Some(1));
    }
}
