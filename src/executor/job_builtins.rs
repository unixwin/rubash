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
            if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &name) {
                let mut stderr = Vec::new();
                writeln!(
                    stderr,
                    "{}wait: {name}: cannot unset: readonly variable",
                    self.diagnostic_prefix()
                )?;
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                return Ok(1);
            }
            // builtin_unbind_variable: an element name unbinds just that
            // element; a scalar name unbinds the whole variable.
            if name.contains('[') {
                // SET_VFLAGS bindflags: ASS_NOEXPAND (verbatim subscript)
                // follows array_expand_once alone; W_ARRAYREF only adds
                // ASS_ONEWORD.
                self.unset_array_element(&name, expand_once);
            } else {
                self.shell_state.env_vars.remove(&name);
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
                if let Some(pid) = self.resolve_background_job(operand) {
                    if !candidates.contains(&pid) {
                        candidates.push(pid);
                    }
                } else {
                    write_wait_operand_error(operand, &self.diagnostic_prefix(), &mut stderr)?;
                }
            }
            if !request.operands.is_empty() && candidates.is_empty() {
                // set_waitlist counted no waitable jobs: every operand
                // failed and wait -n returns 127.
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                return Ok(127);
            }

            loop {
                self.refresh_background_jobs()?;
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
            for pid in pids {
                // wait.def:238-245 wait_for_any_job + :328-331: a trapped
                // signal interrupts the no-operand wait too, returning
                // 128+sig.
                if let WaitPidOutcome::Interrupted(interrupt) =
                    self.wait_for_background_pid(pid, false)?
                {
                    self.write_buffered_builtin_output(cmd, &[], &[])?;
                    return Ok(interrupt);
                }
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
            if let Some(pid) = self.resolve_background_job(&cmd.words[1]) {
                match self.wait_for_background_pid(pid, true)? {
                    WaitPidOutcome::Status(status) | WaitPidOutcome::Interrupted(status) => {
                        self.write_buffered_builtin_output(cmd, &[], &[])?;
                        return Ok(status);
                    }
                    WaitPidOutcome::NotFound => {}
                }
            } else if let Ok(pid) = cmd.words[1].parse::<u32>() {
                match self.wait_for_background_pid(pid, true)? {
                    WaitPidOutcome::Status(status) | WaitPidOutcome::Interrupted(status) => {
                        self.write_buffered_builtin_output(cmd, &[], &[])?;
                        return Ok(status);
                    }
                    WaitPidOutcome::NotFound => {}
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
                self.shell_state
                    .env_vars
                    .insert(base.clone(), format_indexed_array_storage(entries));
                if !is_marked_array_var(&self.shell_state.env_vars, &base) {
                    mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, &base);
                }
            }
            _ => {
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
            .map(|operand| (operand.clone(), self.resolve_background_job(operand)))
            .collect::<Vec<_>>();
        let mut stderr = Vec::new();
        let mut status = 0;
        let mut last_pid = None;

        for (operand, pid) in resolved {
            let Some(pid) = pid else {
                status =
                    write_wait_operand_error(&operand, &self.diagnostic_prefix(), &mut stderr)?;
                last_pid = None;
                continue;
            };
            match self.wait_for_background_pid(pid, true)? {
                WaitPidOutcome::Interrupted(interrupt_status) => {
                    self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                    return Ok(interrupt_status);
                }
                WaitPidOutcome::Status(wait_status) => {
                    status = wait_status;
                    last_pid = Some(pid);
                }
                WaitPidOutcome::NotFound => {
                    status =
                        write_wait_operand_error(&operand, &self.diagnostic_prefix(), &mut stderr)?;
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
        None
    }

    /// GNU execute_cmd.c:2975-2983 REAP(): after each loop body the shell
    /// silently reaps dead jobs when it is non-interactive or job control is
    /// off — `for/while/until/select/(( ;; ))` iterations call this.
    pub(in crate::executor) fn reap_dead_jobs_after_loop_body(&mut self) {
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
                finished.push((
                    *pid,
                    crate::executor::wait_status::process_exit_status(&status),
                ));
            }
        }

        for (pid, status) in finished {
            self.background_children.remove(&pid);
            self.join_coproc_stderr_forwarder(pid)?;
            self.shell_state.job_table.mark_completed(pid, status);
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

    /// GNU execute_cmd.c:2450 coproc_unsetvars: unbind_variable_noref
    /// removes <c_name>_PID literally (no nameref follow, no readonly gate),
    /// then check_unbind_variable resolves c_name through namerefs and
    /// refuses readonly targets with `name: cannot unset: readonly
    /// variable`. Runs for every coproc name recorded at spawn, including
    /// names whose coproc_bind failed (invalid identifier, readonly).
    fn coproc_unset_vars(&mut self, pid: u32) {
        let Some(name) = self.shell_state.coproc_names.remove(&pid) else {
            return;
        };
        let pid_name = format!("{name}_PID");
        self.shell_state.env_vars.remove(&pid_name);
        self.shell_state.variables.remove(&pid_name);
        for marker in [
            READONLY_VARS,
            NAMEREF_VARS,
            ARRAY_VARS,
            ASSOC_VARS,
            ASSOC_128_VARS,
            DECLARED_UNSET_VARS,
        ] {
            unmark_env_name(&mut self.shell_state.env_vars, marker, &pid_name);
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
        for marker in [ARRAY_VARS, ASSOC_VARS, ASSOC_128_VARS, DECLARED_UNSET_VARS] {
            unmark_env_name(&mut self.shell_state.env_vars, marker, &unbind_name);
        }
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
            self.shell_state.env_vars.remove(&fd_stdin_key(fd));
            self.shell_state.env_vars.remove(&fd_stdin_offset_key(fd));
            self.shell_state.env_vars.remove(&fd_dynamic_input_key(fd));
            self.shell_state.env_vars.remove(&fd_output_key(fd));
            self.shell_state
                .env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.shell_state
                .env_vars
                .insert(fd_closed_key(fd), "1".to_string());
        }
        self.coproc_unset_vars(pid);

        let coproc_prefix = format!("{FD_COPROC_STDIN_TARGET_PREFIX}{pid}");
        self.shell_state.env_vars.retain(|key, value| {
            !((key.starts_with(FD_STDIN_PREFIX) || key.starts_with(FD_OUTPUT_PREFIX))
                && value == &coproc_prefix)
        });
    }

    fn forget_background_runtime(&mut self, pid: u32) {
        self.background_children.remove(&pid);
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
        if let Some(status) = self
            .shell_state
            .job_table
            .completed_statuses
            .get(&pid)
            .copied()
        {
            self.join_coproc_stderr_forwarder(pid)?;
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
            if let Some(done) = child.try_wait()? {
                break crate::executor::wait_status::process_exit_status(&done);
            }
            if let Some(interrupt) = self.wait_pending_signal_status()? {
                self.background_children.insert(pid, child);
                return Ok(WaitPidOutcome::Interrupted(interrupt));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        self.join_coproc_stderr_forwarder(pid)?;
        self.shell_state.job_table.mark_completed(pid, status);
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
    /// shell with 128+sig (ExecuteError::ExitCode); a trapped signal makes
    /// `wait` return 128+sig immediately — SIGCHLD is exempt outside posix
    /// mode (wait.def:195-199). Everything drained is re-queued so the
    /// command boundary's run_pending_signal_traps still runs the trap
    /// actions after wait returns ("the trap associated with that signal
    /// shall be taken").
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
                None => return Err(ExecuteError::ExitCode(128 + signal)),
                // signals delivered but never named (signal_trap_name
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
            let mut status = 0;
            for job in requested_jobs {
                if let Some(pid) = self.resolve_background_job(job) {
                    if let Some(job_id) = self.shell_state.job_table.pid_to_job.get(&pid).copied() {
                        if let Some(entry) = self.shell_state.job_table.jobs.get(&job_id) {
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
            let output = self.render_background_jobs(options, selected);
            // jobs.c:2231 list_one_job: explicit jobspec listing is followed
            // by cleanup_dead_jobs — a listed dead job leaves the table.
            self.shell_state.job_table.cleanup_dead_jobs();
            return Ok((output, status));
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
                output.push_str(&format!(
                    "[{job_number}]{marker}  {pid} {state_text:<27}{source}{async_suffix}
"
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
            crate::builtins::disown::DisownAction::All => {
                let pids: Vec<u32> = self
                    .shell_state
                    .job_table
                    .jobs
                    .values()
                    .flat_map(|job| job.pids.iter().copied())
                    .chain(self.background_children.keys().copied())
                    .collect();
                self.background_children.clear();
                for pid in pids {
                    self.close_coproc_endpoints(pid);
                    self.fd_table.close(pid);
                }
                self.shell_state.job_table.clear_jobs();
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
                        self.close_coproc_endpoints(pid);
                        self.fd_table.close(pid);
                        self.shell_state.job_table.remove_job_by_pid(pid);
                    } else {
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
                }
                status
            }
        };
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
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
        self.shell_state.job_table.remove_job_by_pid(pid);
        true
    }

    pub(in crate::executor) fn resolve_background_job(&self, job: &str) -> Option<u32> {
        if job.starts_with('%') {
            let job_id = self.shell_state.job_table.resolve_jobspec(job)?;
            return self
                .shell_state
                .job_table
                .jobs
                .get(&job_id)?
                .pids
                .last()
                .copied();
        }
        let pid = job.parse::<u32>().ok()?;
        (self.shell_state.job_table.pid_to_job.contains_key(&pid)
            || self
                .shell_state
                .job_table
                .completed_statuses
                .contains_key(&pid))
        .then_some(pid)
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
        let job = jobs.first().map(String::as_str);
        let Some(pid) = self.resolve_requested_background_job(job) else {
            self.write_job_not_found("fg", job, stderr)?;
            return Ok(1);
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
        match self.wait_for_background_pid(pid, false)? {
            WaitPidOutcome::Status(status) | WaitPidOutcome::Interrupted(status) => Ok(status),
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
            if let Some(pid) = self.resolve_requested_background_job(job) {
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
        // builtins/history.def: The history builtin works for file I/O
        // (-r/-w/-a/-n) and listing even without `set -o history`.
        // remember_on_history only controls automatic recording. Create
        // a session history on demand so the builtin can read/list —
        // but only when no host provider is installed: an installed
        // provider IS the history list (the host owns interactive
        // history), so an auto-created empty session must not shadow it.
        if self.shell_state.session_history.is_none() && self.history_provider.is_none() {
            let session = std::rc::Rc::new(std::cell::RefCell::new(
                crate::history::SessionHistory::new(),
            ));
            self.set_session_history(Some(session));
        }
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
        let session = self.shell_state.session_history.clone();
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
                let editor_path = crate::executor::path::find_user_command(
                    &editor_name,
                    &self.shell_state.env_vars,
                )
                .unwrap_or_else(|| std::path::PathBuf::from(&editor_name));
                let edit_status = std::process::Command::new(&editor_path).arg(&path).status();
                match edit_status {
                    Ok(st) if st.success() => {}
                    Ok(st) => {
                        let _ = std::fs::remove_file(&path);
                        return Ok(crate::executor::wait_status::process_exit_status(&st));
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
                // fc.def:539: echo_input_at_read = 1 before fc_execute_file.
                // y.tab.c:5071-5075: each input line is echoed to stderr.
                for line in edited.lines() {
                    eprintln!("{line}");
                }
                if let Some(session) = session.as_ref() {
                    let control = self.get_env("HISTCONTROL").unwrap_or_default();
                    let ignore = self.get_env("HISTIGNORE").unwrap_or_default();
                    let histsize =
                        crate::history::SessionHistory::size_limit(self.get_env("HISTSIZE"));
                    session
                        .borrow_mut()
                        .record(&edited, &control, &ignore, histsize);
                }
                let tokens = crate::lexer::tokenize(&edited);
                let mut ast = crate::parser::parse_with_options(
                    &tokens,
                    crate::parser::ParseLoopOptions {
                        stray_close_is_error: true,
                        diagnostic_text: None,
                        source_text: Some(edited.clone()),
                        source_line_offset: 0,
                    },
                );
                self.apply_command_output_redirects(cmd, &mut ast)?;
                self.with_compound_output_redirects(cmd, |executor| executor.execute_ast(&ast))?;
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
                    // fc.def fc_replhist: the executed command REPLACES the
                    // fc entry (delete the last line, add the new one), so
                    // the fc command itself never appears in the history.
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
                        source_text: Some(command.clone()),
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
                                "{diagnostic_prefix}complete: {pseudo}: no completion specification"
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
                            "{diagnostic_prefix}complete: {pseudo}: no completion specification"
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
        let function_names: Vec<String> = self.shell_state.functions.keys().cloned().collect();
        let job_names: Vec<String> = self
            .shell_state
            .job_table
            .jobs
            .values()
            .filter(|job| job.background)
            .map(|job| job.command.clone())
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
                        // usage line.
                        let which = if parsed.funcname.is_some() { 'F' } else { 'C' };
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}compgen: warning: -{which} option may not work as you expect"
                        )?;
                    }
                    let word = parsed.word().to_string();
                    let mut resolved = crate::builtins::complete::CompgenDynamic::default();
                    if let Some(funcname) = parsed.funcname.as_deref() {
                        resolved.function_candidates =
                            self.run_compgen_completion_function(funcname, "compgen", &word);
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
        let status = if matches!(
            builtin,
            crate::builtins::complete::CompletionBuiltin::Compgen
        ) {
            crate::builtins::complete::execute_compgen_with_dynamic(
                &cmd.words[1..],
                &self.shell_state.env_vars,
                &self.shell_state.aliases,
                &function_names,
                &job_names,
                &diagnostic_prefix,
                dynamic,
                &mut stdout,
                &mut stderr,
            )?
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
            )?
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
                    store_indexed_array(&mut self.shell_state.env_vars, &varname, values);
                }
                stdout.clear();
            }
        }
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
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
    ) -> Vec<String> {
        if !self.has_function(funcname) {
            // pcomplete.c:1058-1066: internal_error, no candidates.
            eprintln!(
                "{}completion: function `{funcname}' not found",
                self.diagnostic_prefix()
            );
            return Vec::new();
        }
        self.bind_compfunc_variables("", 0, &[], -1);
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
        self.bind_compfunc_variables("", 0, &[], 0);
        let quoted = [cmd, word, ""]
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
        self.unbind_compfunc_variables();
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

    /// pcomplete.c:863 gen_wordlist_matches: split the -W wordlist at shell
    /// delimiters (quotes group and are removed — the splitter's
    /// GNU-parse.y-compatible pass in alias_helpers), then expand each word
    /// with the shellexp rules: a quoted word keeps one field per array
    /// element and never glob-splits; an unquoted word goes through the
    /// for-list expander (parameter expansion, field splitting, pathname
    /// expansion).
    fn expand_compgen_wordlist(&mut self, wordlist: &str) -> Vec<String> {
        let mut values = Vec::new();
        for (word, quoted) in split_shell_words_with_quote_info(wordlist) {
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

    /// pcomplete.c:929 bind_compfunc_variables with the compgen shape: an
    /// empty line and a zero index (COMP_LINE "", COMP_POINT 0,
    /// COMP_TYPE 0, COMP_KEY 0 — readline is not completing here), the
    /// COMP_WORDS array, and COMP_CWORD (cw - 1 for functions,
    /// pcomplete.c:1073-1074).
    fn bind_compfunc_variables(&mut self, line: &str, point: usize, words: &[String], cword: i64) {
        self.set_env("COMP_LINE", line);
        self.set_env("COMP_POINT", &point.to_string());
        self.set_env("COMP_TYPE", "0");
        self.set_env("COMP_KEY", "0");
        store_indexed_array(&mut self.shell_state.env_vars, "COMP_WORDS", words.to_vec());
        self.shell_state.variables.remove("COMP_WORDS");
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
            env::remove_var(name);
        }
    }
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
    // flag so `wait: A[]]: ...` diagnostics print clean text.
    Some(
        words[index..]
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
