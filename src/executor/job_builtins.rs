use super::*;
use std::io::Write;

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
                let Some(words) = self.expand_jobs_x_words(words, &mut stderr)? else {
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(1);
                };
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
    ) -> Result<Option<Vec<String>>, ExecuteError> {
        let mut expanded = Vec::with_capacity(words.len());
        for word in words {
            if word.starts_with('%') {
                let Some(pid) = self.resolve_background_job(&word) else {
                    writeln!(
                        stderr,
                        "{}jobs: {word}: no such job",
                        self.diagnostic_prefix()
                    )?;
                    return Ok(None);
                };
                expanded.push(pid.to_string());
            } else {
                expanded.push(word);
            }
        }
        Ok(Some(expanded))
    }

    pub(in crate::executor) fn execute_wait(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        self.refresh_background_jobs()?;
        if let Some((pid, wait_var)) = self.wait_any_background_request(cmd) {
            // wait -n consumes the selected completion; explicit waits retain
            // it so a repeated wait for the same pid returns the same status.
            if let Some(status) = self.wait_for_background_pid(pid, false)? {
                if let Some(wait_var) = wait_var {
                    self.apply_shell_assignment(&wait_var, pid.to_string());
                }
                self.write_buffered_builtin_output(cmd, &[], &[])?;
                return Ok(status);
            }
        }

        if cmd.words.len() == 1 && self.job_table.jobs.values().any(|job| job.background) {
            let pids = self
                .job_table
                .jobs
                .values()
                .filter(|job| job.background)
                .flat_map(|job| job.pids.iter().copied())
                .collect::<Vec<_>>();
            for pid in pids {
                let _ = self.wait_for_background_pid(pid, false)?;
            }
            self.write_buffered_builtin_output(cmd, &[], &[])?;
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
                let status = self.wait_for_background_operands(&operands, cmd)?;
                return Ok(status);
            }
        }

        if cmd.words.len() == 2 {
            if let Some(pid) = self.resolve_background_job(&cmd.words[1]) {
                if let Some(status) = self.wait_for_background_pid(pid, true)? {
                    self.write_buffered_builtin_output(cmd, &[], &[])?;
                    return Ok(status);
                }
            } else if let Ok(pid) = cmd.words[1].parse::<u32>() {
                if let Some(status) = self.wait_for_background_pid(pid, true)? {
                    self.write_buffered_builtin_output(cmd, &[], &[])?;
                    return Ok(status);
                }
            }
        }

        let mut stderr = Vec::new();
        let status = crate::builtins::wait::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    fn wait_for_background_operands(
        &mut self,
        operands: &[String],
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let resolved = operands
            .iter()
            .map(|operand| (operand.clone(), self.resolve_background_job(operand)))
            .collect::<Vec<_>>();
        let mut stderr = Vec::new();
        let mut status = 0;

        for (operand, pid) in resolved {
            let Some(pid) = pid else {
                status =
                    write_wait_operand_error(&operand, &self.diagnostic_prefix(), &mut stderr)?;
                continue;
            };
            if let Some(wait_status) = self.wait_for_background_pid(pid, true)? {
                status = wait_status;
            } else {
                status =
                    write_wait_operand_error(&operand, &self.diagnostic_prefix(), &mut stderr)?;
            }
        }

        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    fn wait_any_background_request(&self, cmd: &CommandNode) -> Option<(u32, Option<String>)> {
        let request = wait_any_request(&cmd.words[1..])?;
        let pid = if let Some(first) = request.operands.first() {
            self.resolve_background_job(first)?
        } else {
            // A completed child may no longer have a Child handle, but its
            // status remains in JobTable until an explicit wait consumes it.
            self.job_table
                .completed_statuses
                .keys()
                .next()
                .copied()
                .or_else(|| {
                    self.job_table
                        .jobs
                        .values()
                        .filter(|job| {
                            job.background && job.state != crate::jobs::ProcessState::Completed
                        })
                        .filter_map(|job| job.pids.last().copied())
                        .next()
                })?
        };
        Some((pid, request.assign_var))
    }

    pub(in crate::executor) fn refresh_background_jobs(&mut self) -> Result<(), ExecuteError> {
        self.refresh_background_jobs_with_protected_coprocs(&[])
    }

    pub(in crate::executor) fn refresh_background_jobs_with_protected_coprocs(
        &mut self,
        protected_coprocs: &[u32],
    ) -> Result<(), ExecuteError> {
        let mut finished = Vec::new();
        for (pid, child) in &mut self.background_children {
            if let Some(status) = child.try_wait()? {
                finished.push((*pid, status.code().unwrap_or(1)));
            }
        }

        for (pid, status) in finished {
            self.background_children.remove(&pid);
            self.join_coproc_stderr_forwarder(pid)?;
            self.job_table.mark_completed(pid, status);
            self.run_sigchld_trap_for_reaped_child()?;
            if !protected_coprocs.contains(&pid) {
                self.retire_completed_coproc(pid);
            }
        }
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

    fn retire_completed_coproc(&mut self, pid: u32) {
        let is_coproc = self.coproc_stdin_writers.contains_key(&pid)
            || self.coproc_stdout_readers.contains_key(&pid)
            || self.fd_table.entries.values().any(|entry| {
                matches!(
                    entry.read.as_ref(),
                    Some(FdReadEndpoint::CoprocStdout(endpoint_pid)) if *endpoint_pid == pid
                ) || matches!(
                    entry.write.as_ref(),
                    Some(FdWriteEndpoint::CoprocStdin(endpoint_pid)) if *endpoint_pid == pid
                )
            });
        if !is_coproc {
            return;
        }

        self.coproc_stdin_writers.remove(&pid);
        self.coproc_stdout_readers.remove(&pid);

        let endpoint_fds = self
            .fd_table
            .entries
            .iter()
            .filter_map(|(fd, entry)| {
                let matches_read = matches!(
                    entry.read.as_ref(),
                    Some(FdReadEndpoint::CoprocStdout(endpoint_pid)) if *endpoint_pid == pid
                );
                let matches_write = matches!(
                    entry.write.as_ref(),
                    Some(FdWriteEndpoint::CoprocStdin(endpoint_pid)) if *endpoint_pid == pid
                );
                (matches_read || matches_write).then_some(*fd)
            })
            .collect::<Vec<_>>();
        for fd in endpoint_fds {
            self.fd_table.close(fd);
            self.env_vars.remove(&fd_stdin_key(fd));
            self.env_vars.remove(&fd_stdin_offset_key(fd));
            self.env_vars.remove(&fd_dynamic_input_key(fd));
            self.env_vars.remove(&fd_output_key(fd));
            self.env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.env_vars.insert(fd_closed_key(fd), "1".to_string());
        }
        let coproc_names = self
            .env_vars
            .iter()
            .filter_map(|(key, value)| {
                key.strip_suffix("_PID")
                    .filter(|_| value == &pid.to_string())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        for name in coproc_names {
            self.env_vars.remove(&name);
            self.env_vars.remove(&format!("{name}_PID"));
            unmark_env_name(&mut self.env_vars, ARRAY_VARS, &name);
        }

        let coproc_prefix = format!("{FD_COPROC_STDIN_TARGET_PREFIX}{pid}");
        self.env_vars.retain(|key, value| {
            !((key.starts_with(FD_STDIN_PREFIX) || key.starts_with(FD_OUTPUT_PREFIX))
                && value == &coproc_prefix)
        });
    }

    fn forget_background_runtime(&mut self, pid: u32) {
        self.background_children.remove(&pid);
        self.background_jobs.remove(&pid);
        self.background_job_order.retain(|job_pid| *job_pid != pid);
        self.coproc_stdin_writers.remove(&pid);
        self.coproc_stdout_readers.remove(&pid);
        self.fd_table.close(pid);
    }

    fn wait_for_background_pid(
        &mut self,
        pid: u32,
        _retain_for_explicit_wait: bool,
    ) -> Result<Option<i32>, ExecuteError> {
        if let Some(status) = self.job_table.completed_statuses.get(&pid).copied() {
            self.join_coproc_stderr_forwarder(pid)?;
            // Waiting consumes the jobs-table entry, but the completed status
            // remains available for a later explicit wait of the same PID.
            self.job_table.remove_job_by_pid_preserve_status(pid);
            self.forget_background_runtime(pid);
            return Ok(Some(status));
        }
        let Some(mut child) = self.background_children.remove(&pid) else {
            return Ok(None);
        };
        let status = child.wait()?.code().unwrap_or(1);
        self.join_coproc_stderr_forwarder(pid)?;
        self.job_table.mark_completed(pid, status);
        self.run_sigchld_trap_for_reaped_child()?;
        // Remove the visible job after any wait, while retaining the exit
        // status for repeated explicit PID waits.
        self.job_table.remove_job_by_pid_preserve_status(pid);
        self.forget_background_runtime(pid);
        Ok(Some(status))
    }

    fn background_jobs_output(
        &mut self,
        options: crate::builtins::jobs::JobsListOptions,
        requested_jobs: &[String],
        stderr: &mut Vec<u8>,
    ) -> Result<(String, i32), ExecuteError> {
        let jobs = if requested_jobs.is_empty() {
            self.ordered_background_jobs()
        } else {
            let mut selected = Vec::new();
            let mut status = 0;
            for job in requested_jobs {
                if let Some(pid) = self.resolve_background_job(job) {
                    if let Some(job_id) = self.job_table.pid_to_job.get(&pid).copied() {
                        if let Some(entry) = self.job_table.jobs.get(&job_id) {
                            selected.push((
                                self.background_job_number(pid),
                                pid,
                                entry.command.clone(),
                            ));
                        }
                    }
                } else {
                    writeln!(
                        stderr,
                        "{}jobs: {job}: no such job",
                        self.diagnostic_prefix()
                    )?;
                    status = 1;
                }
            }
            return Ok((self.render_background_jobs(options, selected), status));
        };
        Ok((self.render_background_jobs(options, jobs), 0))
    }

    fn ordered_background_jobs(&self) -> Vec<(usize, u32, String)> {
        self.job_table
            .jobs
            .values()
            .filter(|job| job.background)
            .enumerate()
            .filter_map(|(index, job)| {
                job.pids
                    .last()
                    .copied()
                    .map(|pid| (index + 1, pid, job.command.clone()))
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
                .job_table
                .pid_to_job
                .get(&pid)
                .and_then(|job_id| self.job_table.jobs.get(job_id))
                .map(|job| match job.state {
                    crate::jobs::ProcessState::Running => "Running".to_string(),
                    crate::jobs::ProcessState::Stopped => "Stopped".to_string(),
                    crate::jobs::ProcessState::Completed => match job.exit_status.unwrap_or(1) {
                        0 => "Done".to_string(),
                        status => format!("Exit {status}"),
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
            if options.changed_only && self.last_notified_job_ids.contains(&job_number) {
                continue;
            }
            let state_text = state_text_opt.unwrap_or_else(|| "Unknown".to_string());

            if options.pids_only {
                output.push_str(&format!("{pid}\n"));
            } else if options.long {
                output.push_str(&format!(
                    "[{job_number}]  {pid} {state_text:<22} {source} &\n"
                ));
            } else {
                output.push_str(&format!("[{job_number}]  {state_text:<22} {source} &\n"));
            }
            if options.changed_only {
                self.last_notified_job_ids.insert(job_number);
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
            crate::builtins::disown::DisownAction::All => {
                let pids: Vec<u32> = self
                    .background_jobs
                    .keys()
                    .copied()
                    .chain(self.background_children.keys().copied())
                    .collect();
                self.background_children.clear();
                self.background_jobs.clear();
                self.background_job_order.clear();
                self.coproc_stdin_writers.clear();
                self.coproc_stdout_readers.clear();
                for pid in pids {
                    self.fd_table.close(pid);
                    self.job_table.remove_job_by_pid(pid);
                }
                self.job_table.clear_jobs();
                0
            }
            crate::builtins::disown::DisownAction::Current => {
                if self.disown_current_job() {
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
            crate::builtins::disown::DisownAction::Jobs(jobs) => {
                let mut status = 0;
                for job in jobs {
                    if let Some(pid) = self.resolve_background_job(&job) {
                        self.background_children.remove(&pid);
                        self.background_jobs.remove(&pid);
                        self.background_job_order.retain(|job_pid| *job_pid != pid);
                        self.coproc_stdin_writers.remove(&pid);
                        self.coproc_stdout_readers.remove(&pid);
                        self.fd_table.close(pid);
                        self.job_table.remove_job_by_pid(pid);
                    } else {
                        writeln!(
                            stderr,
                            "{}disown: {job}: no such job",
                            self.diagnostic_prefix()
                        )?;
                        status = 1;
                    }
                }
                status
            }
        };
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    fn disown_current_job(&mut self) -> bool {
        let Some(pid) = self.last_background_pid else {
            return false;
        };
        if !self.background_children.contains_key(&pid) && !self.background_jobs.contains_key(&pid)
        {
            return false;
        }

        self.background_children.remove(&pid);
        self.background_jobs.remove(&pid);
        self.background_job_order.retain(|job_pid| *job_pid != pid);
        self.coproc_stdin_writers.remove(&pid);
        self.coproc_stdout_readers.remove(&pid);
        self.fd_table.close(pid);
        self.job_table.remove_job_by_pid(pid);
        true
    }

    pub(in crate::executor) fn resolve_background_job(&self, job: &str) -> Option<u32> {
        if job.starts_with('%') {
            let job_id = self.job_table.resolve_jobspec(job)?;
            return self.job_table.jobs.get(&job_id)?.pids.last().copied();
        }
        let pid = job.parse::<u32>().ok()?;
        (self.job_table.pid_to_job.contains_key(&pid)
            || self.job_table.completed_statuses.contains_key(&pid))
        .then_some(pid)
    }

    fn background_job_number(&self, pid: u32) -> usize {
        self.job_table
            .pid_to_job
            .get(&pid)
            .and_then(|job_id| {
                self.job_table
                    .jobs
                    .keys()
                    .position(|candidate| candidate == job_id)
            })
            .map(|index| index + 1)
            .unwrap_or(1)
    }

    pub(in crate::executor) fn execute_fg_bg(
        &mut self,
        cmd: &CommandNode,
        builtin: crate::builtins::fg_bg::JobControlBuiltin,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let action = crate::builtins::fg_bg::execute_with_io(
            builtin,
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        let has_job_control = self.job_table.jobs.values().any(|job| job.background);
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
                            self.execute_fg_jobs(jobs, &mut stderr)?
                        }
                        crate::builtins::fg_bg::JobControlBuiltin::Bg => {
                            self.execute_bg_jobs(jobs, &mut stderr)?
                        }
                    }
                }
            }
        };
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    fn execute_fg_jobs(
        &mut self,
        jobs: Vec<String>,
        stderr: &mut Vec<u8>,
    ) -> Result<i32, ExecuteError> {
        let job = jobs.first().map(String::as_str);
        let Some(pid) = self.resolve_requested_background_job(job) else {
            self.write_job_not_found("fg", job, stderr)?;
            return Ok(1);
        };

        let Some(mut child) = self.background_children.remove(&pid) else {
            self.background_jobs.remove(&pid);
            self.background_job_order.retain(|job_pid| *job_pid != pid);
            self.coproc_stdin_writers.remove(&pid);
            self.coproc_stdout_readers.remove(&pid);
            self.fd_table.close(pid);
            self.write_job_not_found("fg", job, stderr)?;
            return Ok(1);
        };
        self.background_jobs.remove(&pid);
        self.background_job_order.retain(|job_pid| *job_pid != pid);
        self.coproc_stdin_writers.remove(&pid);
        self.coproc_stdout_readers.remove(&pid);
        self.fd_table.close(pid);
        let status = child.wait()?.code().unwrap_or(1);
        self.job_table.mark_completed(pid, status);
        self.run_sigchld_trap_for_reaped_child()?;
        let status = self.job_table.wait_pid(pid).unwrap_or(status);
        self.job_table.remove_job_by_pid(pid);
        Ok(status)
    }

    fn execute_bg_jobs(
        &mut self,
        jobs: Vec<String>,
        stderr: &mut Vec<u8>,
    ) -> Result<i32, ExecuteError> {
        let requested = if jobs.is_empty() {
            vec![None]
        } else {
            jobs.iter()
                .map(|job| Some(job.as_str()))
                .collect::<Vec<_>>()
        };

        let mut status = 0;
        for job in requested {
            if let Some(pid) = self.resolve_requested_background_job(job) {
                self.job_table.mark_running(pid);
                if let Some(job_id) = self.job_table.pid_to_job.get(&pid).copied() {
                    if let Some(entry) = self.job_table.jobs.get_mut(&job_id) {
                        entry.background = true;
                        entry.foreground = false;
                    }
                }
            } else {
                self.write_job_not_found("bg", job, stderr)?;
                status = 1;
            }
        }
        Ok(status)
    }

    fn resolve_requested_background_job(&self, job: Option<&str>) -> Option<u32> {
        match job {
            Some(job) => self.resolve_background_job(job),
            None => self.current_background_pid(),
        }
    }

    fn current_background_pid(&self) -> Option<u32> {
        let job_id = self.job_table.current_job()?;
        self.job_table.jobs.get(&job_id)?.pids.last().copied()
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
        let status = crate::builtins::suspend::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_history(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let args = &cmd.words[1..];
        if let Some(session) = self.session_history.clone() {
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
            }

            if has_io {
                let file = file_arg
                    .or_else(|| self.get_env("HISTFILE").map(String::from))
                    .ok_or_else(|| {
                        let _ = writeln!(stderr, "history: filename not specified");
                        ExecuteError::ExitCode(2)
                    })?;
                let mut p = provider.borrow_mut();
                if do_clear {
                    p.clear()?;
                    let _ = std::fs::write(&file, "");
                }
                if do_append {
                    p.append_history(&file)?;
                }
                if do_read_new {
                    p.read_new_history(&file)?;
                }
                if do_read {
                    p.read_history(&file)?;
                }
                if do_write {
                    p.write_history(&file)?;
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
            let status = crate::builtins::history::execute_with_history(
                args,
                &self.diagnostic_prefix(),
                &entries,
                &mut stdout,
                &mut stderr,
            )?;
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(status);
        }
        let status = crate::builtins::history::execute_with_io(
            args,
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_bind(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stderr = Vec::new();
        let status = crate::builtins::bind::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
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
        let session = self.session_history.clone();
        let result = if let Some(session) = session.as_ref() {
            let (entries, base, last_added) = {
                let shell = session.borrow();
                (shell.entries.clone(), shell.base, shell.last_line_added)
            };
            crate::builtins::fc::execute_with_history(
                &cmd.words[1..],
                &self.diagnostic_prefix(),
                &entries,
                base,
                last_added,
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
                &provider_entries,
                1,
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
            } => {
                // fc.def edit_and_execute_command: write the selected
                // entries to a temp file, run the editor with inherited
                // stdio, read the result back, remember and execute it.
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
                if selected.is_empty() {
                    return Ok(0);
                }
                let editor_name = editor
                    .or_else(|| self.get_env("FCEDIT").map(String::from))
                    .or_else(|| self.get_env("EDITOR").map(String::from))
                    .unwrap_or_else(|| String::from("vi"));
                let mut path = std::env::temp_dir();
                path.push(format!("bash-fc.{}", std::process::id()));
                if std::fs::write(&path, selected.join("\n") + "\n").is_err() {
                    writeln!(
                        stderr,
                        "{pfx}fc: cannot open temp file",
                        pfx = self.diagnostic_prefix(),
                    )?;
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(1);
                }
                let editor_path =
                    crate::executor::path::find_user_command(&editor_name, &self.env_vars)
                        .unwrap_or_else(|| std::path::PathBuf::from(&editor_name));
                let edit_status = std::process::Command::new(&editor_path).arg(&path).status();
                match edit_status {
                    Ok(st) if st.success() => {}
                    Ok(st) => {
                        let _ = std::fs::remove_file(&path);
                        return Ok(st.code().unwrap_or(1));
                    }
                    Err(err) => {
                        writeln!(
                            stderr,
                            "{pfx}fc: {editor_name}: {err}",
                            pfx = self.diagnostic_prefix(),
                        )?;
                        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                        let _ = std::fs::remove_file(&path);
                        return Ok(127);
                    }
                }
                let edited = std::fs::read_to_string(&path).unwrap_or_default();
                let _ = std::fs::remove_file(&path);
                let edited = edited.trim_end().to_string();
                if edited.is_empty() {
                    return Ok(0);
                }
                if let Some(session) = session.as_ref() {
                    let control = self.get_env("HISTCONTROL").unwrap_or_default();
                    let ignore = self.get_env("HISTIGNORE").unwrap_or_default();
                    let histsize = self
                        .get_env("HISTSIZE")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(500);
                    session
                        .borrow_mut()
                        .record(&edited, &control, &ignore, histsize);
                }
                let tokens = crate::lexer::tokenize(&edited);
                let mut ast = crate::parser::parse_with_options(
                    &tokens,
                    crate::parser::ParseLoopOptions {
                        stray_close_is_error: true,
                        source_text: Some(edited.clone()),
                        source_line_offset: 0,
                    },
                );
                self.apply_command_output_redirects(cmd, &mut ast)?;
                self.execute_ast(&ast)?;
                Ok(self.exit_code)
            }
            crate::builtins::fc::FcResult::Status(status) => Ok(status),
            crate::builtins::fc::FcResult::Reexec { command } => {
                // fc -s: the substituted command is remembered (the C
                // parse_and_execute remembers it) and then executed.
                if let Some(session) = session.as_ref() {
                    let control = self.get_env("HISTCONTROL").unwrap_or_default();
                    let ignore = self.get_env("HISTIGNORE").unwrap_or_default();
                    let histsize = self
                        .get_env("HISTSIZE")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(500);
                    let mut shell = session.borrow_mut();
                    // fc.def fc_replhist: the executed command REPLACES the
                    // fc entry (delete the last line, add the new one), so
                    // the fc command itself never appears in the history.
                    if shell.last_line_added && !shell.entries.is_empty() {
                        shell.entries.pop();
                    }
                    let was_recorded = shell.record(&command, &control, &ignore, histsize);
                    shell.last_line_added = was_recorded;
                }
                let tokens = crate::lexer::tokenize(&command);
                let mut ast = crate::parser::parse_with_options(
                    &tokens,
                    crate::parser::ParseLoopOptions {
                        stray_close_is_error: true,
                        source_text: Some(command.clone()),
                        source_line_offset: 0,
                    },
                );
                self.apply_command_output_redirects(cmd, &mut ast)?;
                self.execute_ast(&ast)?;
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
                for (name, cs) in self.completion_specs.iter() {
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
            if parsed.pflag {
                // -p overrides everything else (complete.def:426-441).
                let mut status = 0;
                if let Some(pseudo) = pseudo {
                    match self.completion_specs.get(pseudo) {
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
                                "{diagnostic_prefix}complete: {pseudo}: no completion specification"
                            )?;
                            status = 1;
                        }
                    }
                } else if !parsed.operands.is_empty() {
                    // print_cmd_completions (complete.def:630-650): argument
                    // order, unknown names error and fail the builtin.
                    for target in &parsed.operands {
                        match self.completion_specs.get(target.as_str()) {
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
                    for (name, cs) in self.completion_specs.iter() {
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
                    if !self.completion_specs.remove(pseudo) {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}complete: {pseudo}: no completion specification"
                        )?;
                        status = 1;
                    }
                } else if !parsed.operands.is_empty() {
                    for target in &parsed.operands {
                        if !self.completion_specs.remove(target) {
                            writeln!(
                                stderr,
                                "{diagnostic_prefix}complete: {target}: no completion specification"
                            )?;
                            status = 1;
                        }
                    }
                } else {
                    self.completion_specs.flush();
                }
                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                return Ok(status);
            }
            if parsed.operands.is_empty() && parsed.opt_given {
                // complete.def:460-464: options but no names and no
                // -p/-r/-D/-E/-I print usage and fail with EX_USAGE.
                crate::builtins::complete::write_usage(builtin, &mut stderr)?;
                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                return Ok(2); // EX_USAGE
            }
            // Register the compspec for every name (complete.def:480-485).
            let spec = crate::builtins::complete::Compspec::from_parsed(&parsed);
            if let Some(pseudo) = pseudo {
                self.completion_specs.insert(pseudo, spec.clone());
            }
            for target in &parsed.operands {
                self.completion_specs.insert(target, spec.clone());
            }
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            return Ok(0);
        }
        let function_names: Vec<String> = self.functions.keys().cloned().collect();
        let job_names: Vec<String> = self
            .job_table
            .jobs
            .values()
            .filter(|job| job.background)
            .map(|job| job.command.clone())
            .collect();
        let status = crate::builtins::complete::execute_with_io(
            builtin,
            &cmd.words[1..],
            &self.env_vars,
            &self.aliases,
            &function_names,
            &job_names,
            &diagnostic_prefix,
            &mut stdout,
            &mut stderr,
        )?;
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
                    store_indexed_array(&mut self.env_vars, &varname, values);
                }
                stdout.clear();
            }
        }
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
    }
}

struct WaitAnyRequest {
    operands: Vec<String>,
    assign_var: Option<String>,
}

fn wait_any_request(words: &[String]) -> Option<WaitAnyRequest> {
    let mut index = 0;
    let mut wait_any = false;
    let mut assign_var = None;
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
                    let name = if value_start < word.len() {
                        &word[value_start..]
                    } else {
                        index += 1;
                        words.get(index)?
                    };
                    if !is_shell_name(name) {
                        return None;
                    }
                    assign_var = Some(name.to_string());
                    if value_start < word.len() {
                        break;
                    }
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

        let mut chars = word[1..].chars().peekable();
        while let Some(option) = chars.next() {
            match option {
                'f' => {}
                'n' | 'p' => return None,
                _ => return None,
            }
        }
        index += 1;
    }

    Some(words[index..].to_vec())
}

fn write_wait_operand_error<E>(
    operand: &str,
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> Result<i32, ExecuteError>
where
    E: Write,
{
    if operand.starts_with('%') {
        writeln!(stderr, "{diagnostic_prefix}wait: {operand}: no such job")?;
        return Ok(127);
    }

    if operand.chars().all(|ch| ch.is_ascii_digit()) {
        writeln!(
            stderr,
            "{diagnostic_prefix}wait: pid {operand} is not a child of this shell"
        )?;
        return Ok(127);
    }

    writeln!(
        stderr,
        "{diagnostic_prefix}wait: `{operand}': not a pid or valid job spec"
    )?;
    Ok(1)
}
