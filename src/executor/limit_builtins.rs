use super::*;

impl Executor {
    pub(in crate::executor) fn execute_kill(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if let Some(status) = self.execute_tracked_background_kill(cmd)? {
            return Ok(status);
        }

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status =
            crate::builtins::kill::execute_with_io(&cmd.words[1..], &mut stdout, &mut stderr)?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(status)
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
                    .is_some_and(|pid| self.job_table.pid_to_job.contains_key(&pid))
        });
        if !should_handle {
            let mut stderr = Vec::new();
            let mut status = 0;
            for operand in request.operands {
                let Some(pid) = operand.parse::<u32>().ok() else {
                    // Not a u32 pid or job spec — fall through to kill::execute_with_io
                    // for negative pids (Unix process groups) and other formats.
                    return Ok(None);
                };
                if request.check_only && pid == 0 {
                    continue;
                }
                if !process_exists(pid) {
                    writeln!(
                        stderr,
                        "{}kill: ({pid}) - No such process",
                        self.diagnostic_prefix()
                    )?;
                    status = 1;
                }
            }
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            if status != 0 || request.check_only {
                return Ok(Some(status));
            }
            return Ok(None);
        }

        let mut stderr = Vec::new();
        let mut status = 0;
        for operand in request.operands {
            let Some(pid) = self.resolve_background_job(&operand) else {
                writeln!(
                    stderr,
                    "{}kill: {operand}: no such job",
                    self.diagnostic_prefix()
                )?;
                status = 1;
                continue;
            };

            if request.check_only {
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
                status = signal_status;
                stderr.extend(signal_stderr);
                continue;
            }
            if request.signal == 18 {
                self.job_table.mark_running(pid);
            } else if matches!(request.signal, 19 | 20) {
                self.job_table.mark_stopped(pid);
            } else if operand.starts_with('%') {
                self.job_table.mark_completed(pid, 128 + request.signal);
                self.background_jobs.remove(&pid);
                self.background_job_order.retain(|job_pid| *job_pid != pid);
                self.coproc_stdin_writers.remove(&pid);
                self.coproc_stdout_readers.remove(&pid);
                self.fd_table.close(pid);
                self.job_table.remove_job_by_pid(pid);
            }
            continue;
        }

        self.write_buffered_builtin_output(cmd, &[], &stderr)?;
        Ok(Some(status))
    }

    pub(in crate::executor) fn execute_ulimit(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::ulimit::execute_with_io(
            &cmd.words[1..],
            &mut self.env_vars,
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
            index += 2;
            continue;
        }
        if word.starts_with('-') && word != "-" {
            if word != "-0"
                && crate::builtins::kill::translate_signal(word.trim_start_matches('-')).is_none()
            {
                return None;
            }
            signal = crate::builtins::kill::signal_number_for_spec(word.trim_start_matches('-'))?;
            check_only |= signal == 0;
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
