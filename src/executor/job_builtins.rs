use super::*;
use std::io::Write;

impl Executor {
    pub(in crate::executor) fn execute_times(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::times::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::times::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut file,
                &mut std::io::stderr().lock(),
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            if is_null_device(&target) {
                return Ok(crate::builtins::times::execute_with_io(
                    cmd.words[1..].iter().map(String::as_str),
                    &mut std::io::stdout().lock(),
                    &mut std::io::sink(),
                )?);
            }
            let mut file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::times::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            return Ok(crate::builtins::times::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut std::io::stdout().lock(),
                &mut file,
            )?);
        }

        Ok(crate::builtins::times::execute(&cmd.words[1..])?)
    }

    pub(in crate::executor) fn execute_caller(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let funcname = self.funcname_stack();
        let lineno = self.indexed_array_stack("BASH_LINENO");
        let source = self.indexed_array_stack("BASH_SOURCE");
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
                let mut command = cmd.clone();
                command.words = words;
                self.execute_command(&command)?;
                Ok(self.exit_code)
            }
        }
    }

    pub(in crate::executor) fn execute_wait(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some((pid, wait_var)) = self.wait_any_background_request(cmd) {
            if let Some(status) = self.wait_for_background_pid(pid)? {
                if let Some(wait_var) = wait_var {
                    self.apply_shell_assignment(&wait_var, pid.to_string());
                }
                self.write_buffered_builtin_output(cmd, &[], &[])?;
                return Ok(status);
            }
        }

        if cmd.words.len() == 1 && !self.background_children.is_empty() {
            let mut status = 0;
            for (_, mut child) in std::mem::take(&mut self.background_children) {
                let wait_status = child.wait()?;
                status = wait_status.code().unwrap_or(1);
            }
            self.background_jobs.clear();
            self.background_job_order.clear();
            self.write_buffered_builtin_output(cmd, &[], &[])?;
            return Ok(status);
        }

        if cmd.words.len() == 2 {
            if let Some(pid) = self.resolve_background_job(&cmd.words[1]) {
                if let Some(status) = self.wait_for_background_pid(pid)? {
                    self.write_buffered_builtin_output(cmd, &[], &[])?;
                    return Ok(status);
                }
            } else if let Ok(pid) = cmd.words[1].parse::<u32>() {
                if let Some(status) = self.wait_for_background_pid(pid)? {
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

    fn wait_any_background_request(&self, cmd: &CommandNode) -> Option<(u32, Option<String>)> {
        let request = wait_any_request(&cmd.words[1..])?;
        let pid = if let Some(first) = request.operands.first() {
            self.resolve_background_job(first)?
        } else {
            self.background_job_order
                .iter()
                .copied()
                .find(|pid| self.background_children.contains_key(pid))?
        };
        Some((pid, request.assign_var))
    }

    fn wait_for_background_pid(&mut self, pid: u32) -> Result<Option<i32>, ExecuteError> {
        let Some(mut child) = self.background_children.remove(&pid) else {
            return Ok(None);
        };
        let status = child.wait()?.code().unwrap_or(1);
        self.background_jobs.remove(&pid);
        self.background_job_order.retain(|job_pid| *job_pid != pid);
        Ok(Some(status))
    }

    fn background_jobs_output(
        &self,
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
                    if let Some(source) = self.background_jobs.get(&pid) {
                        selected.push((self.background_job_number(pid), pid, source.clone()));
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
        self.background_job_order
            .iter()
            .enumerate()
            .filter_map(|(index, pid)| {
                self.background_jobs
                    .get(pid)
                    .map(|source| (index + 1, *pid, source.clone()))
            })
            .collect()
    }

    fn render_background_jobs(
        &self,
        options: crate::builtins::jobs::JobsListOptions,
        jobs: Vec<(usize, u32, String)>,
    ) -> String {
        let mut output = String::new();
        for (job_number, pid, source) in jobs {
            if options.pids_only {
                output.push_str(&format!("{pid}\n"));
            } else if options.long {
                output.push_str(&format!(
                    "[{job_number}]  {pid} Running                 {source} &\n"
                ));
            } else {
                output.push_str(&format!(
                    "[{job_number}]  Running                 {source} &\n"
                ));
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
                self.background_children.clear();
                self.background_jobs.clear();
                self.background_job_order.clear();
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
        true
    }

    fn resolve_background_job(&self, job: &str) -> Option<u32> {
        if let Some(number) = job.strip_prefix('%') {
            return self.resolve_background_job_number(number);
        }

        job.parse::<u32>().ok().filter(|pid| {
            self.background_children.contains_key(pid) || self.background_jobs.contains_key(pid)
        })
    }

    fn resolve_background_job_number(&self, number: &str) -> Option<u32> {
        let index = match number {
            "" | "+" | "%" => 1,
            _ => number.parse::<usize>().ok()?,
        };
        if index == 0 {
            return None;
        }

        self.background_job_order
            .iter()
            .copied()
            .filter(|pid| self.background_jobs.contains_key(pid))
            .nth(index - 1)
    }

    fn background_job_number(&self, pid: u32) -> usize {
        self.background_job_order
            .iter()
            .position(|job_pid| *job_pid == pid)
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
        let status = match action {
            crate::builtins::fg_bg::FgBgAction::Complete(status) => status,
            crate::builtins::fg_bg::FgBgAction::Jobs(jobs) => {
                if self.background_jobs.is_empty() && self.background_children.is_empty() {
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
            self.write_job_not_found("fg", job, stderr)?;
            return Ok(1);
        };
        self.background_jobs.remove(&pid);
        self.background_job_order.retain(|job_pid| *job_pid != pid);
        let status = child.wait()?.code().unwrap_or(1);
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
            if self.resolve_requested_background_job(job).is_none() {
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
        let pid = self.last_background_pid?;
        (self.background_children.contains_key(&pid) || self.background_jobs.contains_key(&pid))
            .then_some(pid)
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
        let mut stderr = Vec::new();
        let status = crate::builtins::history::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
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
        let mut stderr = Vec::new();
        let status = crate::builtins::fc::execute_with_io(
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(status)
    }

    pub(in crate::executor) fn execute_completion_builtin(
        &mut self,
        cmd: &CommandNode,
        builtin: crate::builtins::complete::CompletionBuiltin,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::complete::execute_with_io(
            builtin,
            &cmd.words[1..],
            &self.diagnostic_prefix(),
            &mut stdout,
            &mut stderr,
        )?;
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

        let mut chars = word[1..].chars().peekable();
        while let Some(option) = chars.next() {
            match option {
                'n' => wait_any = true,
                'f' => {}
                'p' => {
                    if chars.peek().is_some() {
                        break;
                    }
                    index += 1;
                    let name = words.get(index)?;
                    if !is_shell_name(name) {
                        return None;
                    }
                    assign_var = Some(name.clone());
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
