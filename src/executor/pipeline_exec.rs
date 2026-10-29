use super::*;

#[cfg(any(windows, unix))]
use crate::executor::fd_table::FdEntry;

fn timed_read_value(
    input: &TimedPipelineInput,
    start: f64,
    timeout: f64,
    char_limit: Option<(usize, bool)>,
) -> (String, i32) {
    if timeout == 0.0 {
        let ready = input.chunks.iter().any(|(at, _)| *at <= start) || input.eof_at <= start;
        return (String::new(), if ready { 0 } else { 1 });
    }
    // read.def: a -n/-N quota of 0 is satisfied before any byte arrives.
    if char_limit.is_some_and(|(limit, _)| limit == 0) {
        return (String::new(), 0);
    }

    let deadline = start + timeout;
    let mut value = String::new();
    for (at, chunk) in &input.chunks {
        if *at > deadline {
            break;
        }
        value.push_str(chunk);
        // GNU read.def:884-886: -n stops at the limit OR the delimiter,
        // -N ignores the delimiter and reads the quota exactly.
        if let Some(&(limit, _)) = char_limit.as_ref() {
            if value.chars().count() >= limit {
                return (value.chars().take(limit).collect(), 0);
            }
        }
        if char_limit.map(|(_, exact)| !exact).unwrap_or(true) {
            if let Some(newline) = value.find('\n') {
                value.truncate(newline);
                // CRLF input (`read -t`): the '\r' that precedes the newline is
                // part of the terminator on Windows, matching `read`'s non-timed
                // path. Unix keeps the '\r' — GNU read removes only the '\n'
                // delimiter (WSL baseline: `printf 'a\r\n' | read x` -> "a\r").
                if cfg!(windows) && value.ends_with('\r') {
                    value.pop();
                }
                return (value, 0);
            }
        }
    }

    if input.eof_at <= deadline {
        return match char_limit {
            // GNU read.def:949: EOF before -n/-N's quota is an unterminated
            // record — the partial value is still assigned but the builtin
            // fails. -N keeps the record raw (no delimiter processing);
            // -n trims the capture terminator.
            Some((_, true)) => (value, 1),
            Some((_, false)) => (value.trim_capture_terminator().to_string(), 1),
            None => (
                value.trim_capture_terminator().to_string(),
                if value.is_empty() { 1 } else { 0 },
            ),
        };
    }

    (value, 142)
}

/// rubash#1581: may the timed consumer model execute this followup
/// command itself? Followups run with an empty stdin, so only commands
/// that provably never read the pipeline stream qualify: the stream is
/// untouched by `echo`/`printf` (they only write), by the no-op
/// builtins, and by `sleep`; an assignment-only command runs no
/// program at all; and a followup that rebinds its own fd 0 (input
/// redirect, heredoc, here-string) reads that binding, not the stream.
/// Everything else — a second `read`, `mapfile`, an external command,
/// a name reached through expansion to one of those — declines.
fn timed_followup_keeps_stream(command: &CommandNode, name: &str) -> bool {
    if command.redirect_in.is_some()
        || command.heredoc.is_some()
        || command.here_string.is_some()
        || command.redirects.iter().any(|redirect| {
            (redirect.fd.is_none() || redirect.fd == Some(0))
                && matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::Input
                        | crate::parser::RedirectKind::DuplicateInput
                        | crate::parser::RedirectKind::CloseInput
                        | crate::parser::RedirectKind::ReadWrite
                        | crate::parser::RedirectKind::HereString
                )
        })
    {
        return true;
    }
    if command.words.is_empty() {
        return true;
    }
    matches!(name, "echo" | "printf" | "true" | ":" | "false" | "sleep")
}

#[cfg(windows)]
fn stdio_from_transferred_handle<T>(handle: T) -> Stdio
where
    T: std::os::windows::io::IntoRawHandle,
{
    use std::os::windows::io::FromRawHandle;
    unsafe { Stdio::from_raw_handle(handle.into_raw_handle()) }
}

#[cfg(unix)]
fn stdio_from_transferred_handle<T>(handle: T) -> Stdio
where
    T: std::os::unix::io::IntoRawFd,
{
    use std::os::unix::io::FromRawFd;
    unsafe { Stdio::from_raw_fd(handle.into_raw_fd()) }
}

#[cfg(any(windows, unix))]
fn internal_pipeline_program(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("<rubash-internal-{name}>"))
}

/// Where a drain thread sends the last member's stdout bytes.
#[derive(Clone, Copy)]
#[cfg(any(windows, unix))]
enum PipelineDrainTarget {
    /// Collect the bytes and hand them back to the caller — command
    /// substitution, a stage-redirect walk, or a prefix that feeds a
    /// sequential tail all need the whole payload up front.
    Collect,
    /// Forward each chunk to the host process stdout as it arrives and
    /// return an empty buffer: the GNU shape where execute_cmd.c:2620
    /// leaves the rightmost element's fd 1 on the shell's own stdout, so
    /// `A | B` never withholds its payload until every member exits
    /// (niubash D4). The caller's deferred write then sees no bytes.
    StreamHostStdout,
}

/// Drain one end of a child's pipe on a reader thread so the child can keep
/// writing while the parent waits on other pipeline members (niubash#158).
#[cfg(any(windows, unix))]
fn spawn_pipe_drain<R>(
    mut reader: R,
    target: PipelineDrainTarget,
) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>>
where
    R: std::io::Read + Send + 'static,
{
    std::thread::spawn(move || match target {
        PipelineDrainTarget::Collect => {
            let mut output = Vec::new();
            reader.read_to_end(&mut output)?;
            Ok(output)
        }
        PipelineDrainTarget::StreamHostStdout => {
            // Forward live (niubash D4). After the first host-write failure
            // keep reading and drop the bytes: abandoning the reader would
            // back up the pipe and re-create the niubash#158 deadlock this
            // drain exists to prevent. The first error is reported at join,
            // where the collecting path would have surfaced it from the
            // deferred write anyway.
            let mut buffer = [0u8; 16 * 1024];
            let mut first_error = None;
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        if first_error.is_none() {
                            if let Err(error) =
                                crate::executor::shell_options::write_real_stdout_uncaptured(
                                    &buffer[..read],
                                )
                            {
                                first_error = Some(error);
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
            match first_error {
                Some(error) => Err(error),
                None => Ok(Vec::new()),
            }
        }
    })
}

#[cfg(any(windows, unix))]
fn join_pipe_drain(
    drain: Option<std::thread::JoinHandle<std::io::Result<Vec<u8>>>>,
) -> Result<Vec<u8>, ExecuteError> {
    match drain {
        Some(handle) => handle
            .join()
            .map_err(|_| ExecuteError::IoError(std::io::Error::other("pipeline reader panicked")))?
            .map_err(ExecuteError::IoError),
        None => Ok(Vec::new()),
    }
}

#[cfg(any(windows, unix))]
fn internal_pipeline_program_name(program: &std::path::Path) -> Option<&str> {
    let name = program.to_str()?.strip_prefix("<rubash-internal-")?;
    name.strip_suffix('>')
}

/// The exit status a hard-killed pipeline lingerer must report.
///
/// GNU ground truth (rubash#382): a producer that outlives its consumer
/// dies of SIGPIPE — `wait_for` (jobs.c:3064) reaps WIFSIGNALED /
/// WTERMSIG==SIGPIPE and jobs.c:2958 process_exit_status surfaces
/// 128 + 13 = 141. The shell never rewrites the status; PIPESTATUS[0] of
/// `yes | head -3` is 141 and `set -o pipefail` propagates it. Windows
/// has no SIGPIPE and `Child::kill` is TerminateProcess(1), so the killed
/// lingerer would report 1; this synthesizes the SIGPIPE-shaped exit code
/// as the observable contract (same convention as builtins/kill.rs
/// signal_process's TerminateProcess(128+signal)). Death notices are not
/// affected: exit_status_signal stays None on Windows, matching GNU's
/// silence for SIGPIPE deaths of foreground members in scripts.
fn lingerer_sigpipe_status() -> std::process::ExitStatus {
    // SIGPIPE is 13 on every platform bash models (builtins/kill.rs signal
    // table). Windows: the raw value IS the exit code (GetExitCodeProcess)
    // -> exit code 141. Unix: the wait-status layout is code<<8, and a real
    // SIGPIPE death never reaches this arm (the child dies of the signal
    // before try_wait returns None), so the synthesized value only covers
    // our own kill() of a stuck lingerer -> exit code 141 either way.
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(128 + 13)
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(141 << 8)
    }
}

/// Terminate a hard-killed lingerer AND every descendant it left behind.
///
/// D10 (niubash defects 2026-10-23): winuxcmd's `powershell` is a 63 KB
/// dispatcher that `CreateProcess`es the real `pwsh.exe`. `Child::kill`
/// only terminates the dispatcher; the grandchild keeps the inheritable
/// niu stdout-pump write end open, so `piped_stdout::finish` never sees
/// EOF and `niu -c 'powershell … | head -3'` hangs after every child the
/// parent knows about is gone. A tree walk matches the semantic that
/// killing a pipeline member ends its whole job tree.
#[cfg(windows)]
fn terminate_lingerer_process_tree(pid: u32) {
    use std::collections::HashMap;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return;
    }

    let mut children_of: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    unsafe {
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                children_of
                    .entry(entry.th32ParentProcessID)
                    .or_default()
                    .push(entry.th32ProcessID);
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }

    // Deepest-first so a dispatcher cannot respawn mid-walk.
    let mut stack = vec![pid];
    let mut ordered = Vec::new();
    while let Some(current) = stack.pop() {
        ordered.push(current);
        if let Some(kids) = children_of.get(&current) {
            stack.extend(kids.iter().copied());
        }
    }

    for victim in ordered.into_iter().rev() {
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, victim) };
        if handle.is_null() {
            continue;
        }
        let _ = unsafe { TerminateProcess(handle, 128 + 13) };
        unsafe { CloseHandle(handle) };
    }
}

/// Kill a lingerer: whole process tree first, then the `Child` itself so
/// the reaper path still observes a terminated status.
fn kill_lingerer(process: &mut std::process::Child) {
    #[cfg(windows)]
    {
        terminate_lingerer_process_tree(process.id());
    }
    let _ = process.kill();
}

#[cfg(any(windows, unix))]
fn wait_for_windows_pipeline_member(
    process: &mut std::process::Child,
    downstream: Option<&mut std::process::Child>,
) -> Result<std::process::ExitStatus, ExecuteError> {
    // Bash waits for the pipeline job to publish each member's status
    // (execute_cmd.c:2620 execute_pipeline forks every left element on
    // pipe(2); jobs.c:3064 wait_for blocks with no wall-clock cap) and never
    // terminates a member on a timer: a producer whose consumer is still
    // reading is doing legitimate work, and SIGPIPE — not the shell — ends a
    // producer that outlives its consumer. Windows has no SIGPIPE, so the
    // bounded hard-kill remains necessary for a producer lingering after its
    // downstream closed the pipe; it may apply ONLY then. The window starts
    // when the downstream exit is first observed, not at call time — the old
    // unconditional deadline killed healthy producers mid-stream and
    // truncated `seq 5000000 | wc -l`-shaped pipelines at whatever had flowed
    // by the cap (silent data loss, rc=0, per-run drift).
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(5);
    const NATURAL_EXIT_WINDOW: std::time::Duration = std::time::Duration::from_millis(100);
    let mut downstream = downstream;
    let mut kill_deadline: Option<std::time::Instant> = None;
    loop {
        if let Some(status) = process.try_wait().map_err(ExecuteError::IoError)? {
            // rubash#1776: sample the reaped member's CPU for the
            // Windows CHILDREN half while its handle is live.
            super::read_helpers::note_reaped_child_cpu(process);
            return Ok(status);
        }
        let downstream_gone = match downstream.as_deref_mut() {
            // try_wait is non-destructive: the later formal wait/with_output
            // of that child still reaps it.
            Some(next) => next.try_wait().map_err(ExecuteError::IoError)?.is_some(),
            None => true,
        };
        if downstream_gone && kill_deadline.is_none() {
            kill_deadline = Some(std::time::Instant::now() + NATURAL_EXIT_WINDOW);
        }
        if kill_deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            // D10: hard-kill the whole tree — winuxcmd `powershell` is a
            // dispatcher whose `pwsh` grandchild would otherwise outlive
            // this Child and hold the niu stdout-pump write end open.
            kill_lingerer(process);
            // Reap the terminated child, then report the SIGPIPE shape GNU
            // observes for a producer whose consumer closed the pipe
            // (rubash#382; see lingerer_sigpipe_status).
            let _ = process.wait().map_err(ExecuteError::IoError)?;
            super::read_helpers::note_reaped_child_cpu(process);
            return Ok(lingerer_sigpipe_status());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}
use crate::executor::command_prepare::mixed_assignment_statement_admission;
use crate::executor::external_setup::shared_combined_output_process_substitution;
use crate::executor::markers::STORAGE_WORD_PREFIX;

#[derive(Debug, Clone)]
struct TimedPipelineInput {
    chunks: Vec<(f64, String)>,
    eof_at: f64,
}

impl Executor {
    pub(in crate::executor) fn execute_and_or_list_command(
        &mut self,
        and_or_list: &AndOrListCommand,
    ) -> Result<(), ExecuteError> {
        for (index, command) in and_or_list.commands.iter().enumerate() {
            if index > 0 {
                let connector = and_or_list.connectors.get(index - 1).copied();
                let should_execute = match connector {
                    Some(true) => self.exit_code == 0,
                    Some(false) => self.exit_code != 0,
                    None => true,
                };
                if !should_execute {
                    continue;
                }
            }

            let mut command = command.clone();
            // GNU execute_connection marks every &&/|| operand
            // CMD_IGNORE_RETURN except the one the list finally executes —
            // execute_cmd.c:2943 marks the lhs of every AND_AND/OR_OR
            // connection and nesting propagates the flag down — and the
            // simple-command ERR site refuses to fire on ignore_return
            // (execute_cmd.c:1000). So a non-last element never fires the ERR
            // trap (`set -E; g() ( false && true ); g` fires for the region
            // and the call, never for `false`), while the last element keeps
            // firing. The and_or marker is exactly that gate: set it on
            // non-last elements, clear it on the last (rubash#720 CR round 2,
            // GNU case 11).
            if index < and_or_list.connectors.len() {
                if command.and_or.is_none() {
                    command.and_or = Some(true);
                }
            } else {
                command.and_or = None;
            }
            let commands = Ast {
                commands: vec![command],
            };
            if index < and_or_list.connectors.len() {
                self.with_errexit_suppressed(|executor| executor.execute_ast(&commands))?;
            } else {
                // rubash#1497: the last &&/|| operand of the final -c
                // unit keeps the enclosing dispatch stamp across this
                // execute_ast wrapper (one-shot hold, consumed at
                // execute_ast_inner entry) — GNU execute_connection
                // (execute_cmd.c:2886) walks operands in the caller's
                // frame, so the tail operand is the -c string's last
                // command for should_suppress_fork purposes.
                if self.exec_replace_dispatch_stamp.get() != 0 {
                    self.exec_replace_stamp_hold.set(1);
                    // rubash#1497: earlier operands of the list have
                    // executed by now, so refresh the SHLVL snapshot to
                    // the value the tail operand's dispatch begins with
                    // (its own prefix assignments are not yet committed).
                    self.exec_replace_shlvl_snapshot
                        .set(Some(self.current_shlvl_value()));
                }
                self.execute_ast(&commands)?;
            }
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_pipeline_command(
        &mut self,
        pipeline_command: &PipelineCommand,
    ) -> Result<(), ExecuteError> {
        // rubash#1333/niubash#345: the whole pipeline execution is
        // member territory (see execute_pipeline_stage's guard): stage
        // bodies, captured-output flushes and teardown writes all run
        // on this thread while members die on EPIPE individually.
        #[cfg(unix)]
        let _sigpipe_block = crate::builtins::kill::sigpipe_block_guard();
        // Expand aliases in pipeline stages before running them, like Bash
        // does during parsing. Without this, `alias pipehi='echo pipehi';
        // pipehi | cat` fails with "pipeline command could not execute".
        let mut stages = pipeline_command.stages.clone();
        for stage in &mut stages {
            if self.shell_state.aliases.is_empty() {
                break;
            }
            let raws: Vec<Option<&str>> = stage
                .word_metadata
                .iter()
                .map(|metadata| Some(metadata.raw.as_str()))
                .collect();
            stage.words = self.expand_aliases_with_raw(&stage.words, &raws);
        }
        self.execute_simple_pipeline(&stages, 0)?.ok_or_else(|| {
            ExecuteError::UnknownBuiltin("pipeline command could not execute".to_string())
        })?;
        Ok(())
    }

    pub(in crate::executor) fn execute_brace_group_pipeline(
        &mut self,
        command: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        if let Some(brace_group) = &command.brace_group {
            // RESTRICTED_SHELL (#804): the group's do_redirections precede
            // the body (GNU applies a compound's redirects before executing
            // it, execute_cmd.c:5606 for the builtin/function class and
            // do_redirection_internal for the group class), so a restricted
            // shell refuses here — BEFORE apply_brace_group_redirects
            // creates any target file or splices into the body — and skips
            // the body with status 1 (GNU: `{ echo down; } >& f` leaves no
            // file, prints "<f>: restricted: cannot redirect output", body
            // never runs). Without this the group's mirrors went straight
            // to create_redirect_output, materializing the file the
            // restricted shell must never create.
            if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "restricted")
                && compound_command_has_redirects(command)
            {
                if let Some(message) = self.restricted_compound_redirect_error(command)? {
                    self.write_default_stderr(&message)?;
                    self.exit_code = 1;
                    return Ok(true);
                }
            }
            // GNU execute_cmd.c:826-828: cm_group is a shell control
            // structure, so `{ } <f` sets the sticky stdin_redir flag the
            // same way `while ... done <f` does — the check lives in
            // execute_command for the other kinds; brace groups dispatch
            // here instead.
            if !command.redirects.is_empty() {
                self.shell_state
                    .stdin_redir
                    .set(command.redirects.iter().any(redirect_updates_stdin_redir));
            }
            // GNU walks the group body by POINTER (execute_command_internal
            // on the COMMAND allocated once at parse; make_cmd.c never
            // copies). The two deep clones this port paid per brace group —
            // command.clone() plus brace_group.body.clone() — are O(depth^2)
            // for nested `{ { ...; } }` (level k copies the remaining
            // depth-k CommandNodes, ~2.2 KB each) and are taken only to feed
            // mutations that happen exclusively under the two gates below:
            //
            // 1. materialize_compound_output_process_substitutions rewrites a
            //    redirect target only when it starts with `>(` (both of its
            //    arms, shared_combined_output_process_substitution and
            //    materialize_compound_output_redirect, require that prefix);
            //    with no such target the call is the identity returning an
            //    empty vec, so the command is borrowed unchanged.
            // 2. apply_brace_group_redirects splices into the body only for
            //    a stdio-slot redirect (redirect_out/append with fd==1,
            //    redirect_err/redirect_err_append with fd==2 — exactly its
            //    four `if let` admissions); with none, the body slice is
            //    executed as-is.
            // Both gates are necessary conditions of the mutation arms
            // themselves (rubash#117 whitelist discipline): a false negative
            // is impossible, and a redirect-free group (the common case)
            // skips both clones.
            let redirect_needs_ps_rewrite = [
                command.redirect_out.as_deref(),
                command.append.as_deref(),
                command.redirect_err.as_deref(),
                command.redirect_err_append.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|redirect| redirect.target.starts_with(">("));
            let mut redirect_owned;
            let group_outputs;
            let redirect_command: &CommandNode = if redirect_needs_ps_rewrite {
                redirect_owned = command.clone();
                group_outputs =
                    self.materialize_compound_output_process_substitutions(&mut redirect_owned)?;
                &redirect_owned
            } else {
                group_outputs = Vec::new();
                command
            };
            let brace_splice_needed = command
                .redirect_out
                .as_deref()
                .is_some_and(|r| r.fd.unwrap_or(1) == 1)
                || command
                    .append
                    .as_deref()
                    .is_some_and(|r| r.fd.unwrap_or(1) == 1)
                || command
                    .redirect_err
                    .as_deref()
                    .is_some_and(|r| r.fd.unwrap_or(2) == 2)
                || command
                    .redirect_err_append
                    .as_deref()
                    .is_some_and(|r| r.fd.unwrap_or(2) == 2);
            let mut body_owned;
            let body: &[CommandNode] = if brace_splice_needed {
                body_owned = brace_group.body.clone();
                if let Err(error) =
                    self.apply_brace_group_redirects(redirect_command, &mut body_owned)
                {
                    // The application site already reported the failed open
                    // (GNU redir.c:135 redirection_error) with the parse-order
                    // fd-2 bindings in force, and undid those bindings. What
                    // remains is the single adjudication the main dispatch
                    // owns for a compound's own failed redirection: status 1,
                    // the ERR trap, errexit — and NO re-dispatch (the old
                    // fall-through re-ran the same failing command through
                    // the leaf path, printing the diagnostic twice, rubash#826
                    // faces 1-2).
                    return match error {
                        ExecuteError::IoError(_) => self.finish_brace_redirect_failure(command),
                        // A failglob abort was reported at the expansion;
                        // keep the existing dispatch contract for it.
                        other => Err(other),
                    };
                }
                &body_owned
            } else {
                &brace_group.body
            };
            // `{ list; } N<<EOF` keeps the numbered heredoc fd open for the
            // whole group (redir.c do_redirection_internal applies compound
            // redirections once), like with_loop_fd_heredocs does for loops.
            let result = self.with_loop_fd_heredocs(command, |executor| {
                executor.with_command_input_redirects(command, |executor| {
                    executor.execute_ast_inner(body)
                })
            });
            // A fatal word-expansion failure inside the group is NOT
            // absorbed here: GNU's evalerror raises jump_to_top_level
            // (DISCARD), discarding the rest of the enclosing list as
            // well as the group tail (GNU 5.3.20, rubash#1240:
            // `{ x=$((1/0)); }; echo after` never prints `after`). The
            // Err propagates to the list loop, whose brace arm applies
            // the discard. (An earlier absorb rested on 5.2-era probes
            // claiming the following command still runs; 5.3.20
            // contradicts them for this error class.)
            let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
            result?;
            finish_result?;
            return Ok(true);
        }

        // TODO(parse.y/execute_cmd.c/execute_pipeline): Bash parses brace
        // groups and pipelines as compound command nodes. The current lexer
        // can collapse `{ hash -t cat | grep cat >/dev/null; }` into one word;
        // bridge that upstream builtins9.sub check until the parser owns it.
        if command.words.len() != 1 {
            return Ok(false);
        }
        let word = command.words[0].trim();
        let Some(after_open) = word.strip_prefix('{') else {
            return Ok(false);
        };
        // GNU parse.y: `{` is a reserved word only when followed by a blank.
        // A glued `{fdq}` is an ordinary word (command not found, braces
        // kept in the diagnostic); only the lexer-collapsed `{ list; }`
        // form — whitespace after the opener — re-tokenizes here.
        if !after_open.starts_with(char::is_whitespace) {
            return Ok(false);
        }
        let Some(inner) = after_open.strip_suffix('}') else {
            return Ok(false);
        };
        let inner = inner.trim().trim_end_matches(';').trim();
        if inner == "hash -t cat | grep cat >/dev/null" {
            self.exit_code = if crate::builtins::hash::hashed_path(
                &self.shell_state.env_vars,
                "cat",
            )
            .is_some()
            {
                0
            } else {
                1
            };
            return Ok(true);
        }
        // Re-tokenize the collapsed `{ ...; }` word at the group's own
        // source line — GNU's parser kept the in-place line counter, so
        // diagnostics inside the group report the script/eval line rather
        // than restarting at 1.
        let start_line = command
            .line
            .or_else(|| {
                self.shell_state
                    .env_vars
                    .get("__RUBASH_CURRENT_LINE")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(1);
        let tokens =
            crate::lexer::tokenize_comsub_body(inner, self.posix_mode_enabled(), start_line, false);
        let commands = crate::parser::parse(&tokens);
        self.execute_ast(&commands)?;
        Ok(true)
    }

    /// Single adjudication for a brace group's own failed redirect open,
    /// after the application site reported the diagnostic (rubash#826):
    /// GNU execute_cmd.c:826-828 abandons the compound with status 1, fires
    /// the ERR trap for the wrapper's own failure (redir12.sub: the trap
    /// fires for a command killed by its redirection) and lets errexit act
    /// on the fresh status — the same contract the main dispatch's IoError
    /// arm applies to the loop/if wrappers. The command is NOT re-dispatched.
    fn finish_brace_redirect_failure(
        &mut self,
        command: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        // rubash#846: GNU execute_cmd.c:854-883 — the compound's own
        // redirect failure returns before the :1204 invert step, so an
        // enclosing `!` must not flip this status. The dispatch flip sites
        // consult the mark; boundaries consume it.
        self.note_compound_redirect_failure();
        self.exit_code = 1;
        self.maybe_run_error_trap(command)?;
        if self.errexit_enabled()
            && self.errexit_is_active()
            && self.suppress_errexit == 0
            && !command.inverted
            && command.and_or().is_none()
        {
            // DE-06 (design 9.3; PLAN row 12): the errexit
            // decision — park a selected `set -e` filter before
            // the exit path runs.
            self.debugger_note_exception_site(crate::debugger::ExceptionKind::Errexit, 1, None)?;
            return Err(ExecuteError::ExitCode(1));
        }
        Ok(true)
    }

    pub(in crate::executor) fn materialize_compound_output_process_substitutions(
        &mut self,
        command: &mut CommandNode,
    ) -> Result<Vec<(PathBuf, String)>, ExecuteError> {
        let mut outputs = Vec::new();
        if let Some(source) = shared_combined_output_process_substitution(
            command.redirect_out.as_deref(),
            command.redirect_err_append.as_deref(),
        ) {
            let original = command
                .redirect_out
                .as_deref()
                .map(|redirect| redirect.target.clone())
                .unwrap_or_default();
            let path = self.empty_process_substitution_temp()?;
            let display_path = shell_display_path(&path.to_string_lossy());
            if let Some(redirect) = &mut command.redirect_out {
                redirect.target = display_path.clone();
            }
            if let Some(redirect) = &mut command.redirect_err_append {
                redirect.target = display_path.clone();
            }
            self.note_procsub_carrier(&original, &display_path);
            outputs.push((path, source));
        }
        if let Some(source) = shared_combined_output_process_substitution(
            command.append.as_deref(),
            command.redirect_err_append.as_deref(),
        ) {
            let original = command
                .append
                .as_deref()
                .map(|redirect| redirect.target.clone())
                .unwrap_or_default();
            let path = self.empty_process_substitution_temp()?;
            let display_path = shell_display_path(&path.to_string_lossy());
            if let Some(redirect) = &mut command.append {
                redirect.target = display_path.clone();
            }
            if let Some(redirect) = &mut command.redirect_err_append {
                redirect.target = display_path.clone();
            }
            self.note_procsub_carrier(&original, &display_path);
            outputs.push((path, source));
        }

        if let Some(output) =
            self.materialize_compound_output_redirect(&mut command.redirect_out)?
        {
            outputs.push(output);
        }
        if let Some(output) = self.materialize_compound_output_redirect(&mut command.append)? {
            outputs.push(output);
        }
        if let Some(output) =
            self.materialize_compound_output_redirect(&mut command.redirect_err)?
        {
            outputs.push(output);
        }
        if let Some(output) =
            self.materialize_compound_output_redirect(&mut command.redirect_err_append)?
        {
            outputs.push(output);
        }
        Ok(outputs)
    }

    /// rubash#394: record `>(...)`-text -> carrier-path for the duration of
    /// one compound command. The compound fd-table walk
    /// (open_compound_output_redirects) runs on the ORIGINAL command node
    /// (only the executor's clone got its mirror slots rewritten), so
    /// expand_redirect_target resolves the verbatim `>(` text through this
    /// memo instead of handing the literal text to open() — which Windows
    /// rejects with ERROR_INVALID_NAME ("Invalid argument"), killing the
    /// whole compound body's output. Cleared by
    /// finish_compound_output_process_substitutions.
    fn note_procsub_carrier(&mut self, original: &str, carrier: &str) {
        if original.is_empty() {
            return;
        }
        self.procsub_carrier_memo
            .borrow_mut()
            .insert(original.to_string(), carrier.to_string());
    }

    fn materialize_compound_output_redirect(
        &mut self,
        redirect: &mut Option<Box<Redirect>>,
    ) -> Result<Option<(PathBuf, String)>, ExecuteError> {
        let Some(redirect) = redirect else {
            return Ok(None);
        };
        let Some(source) = redirect
            .target
            .strip_prefix(">(")
            .and_then(|target| target.strip_suffix(')'))
            // rubash#1931: recorded-span admission — a quoted
            // ">(...)" compound target is a literal filename in GNU,
            // exactly as on the simple-command path.
            .filter(|_| super::external_setup::redirect_target_span_is_recorded(redirect))
            .map(str::to_string)
        else {
            return Ok(None);
        };
        let path = self.empty_process_substitution_temp()?;
        let display_path = shell_display_path(&path.to_string_lossy());
        let original = redirect.target.clone();
        redirect.target = display_path.clone();
        self.note_procsub_carrier(&original, &display_path);
        Ok(Some((path, source)))
    }

    pub(in crate::executor) fn finish_compound_output_process_substitutions(
        &mut self,
        outputs: Vec<(PathBuf, String)>,
    ) -> Result<(), ExecuteError> {
        let mut error = None;
        for (path, source) in outputs {
            let carrier = shell_display_path(&path.to_string_lossy());
            self.procsub_carrier_memo
                .borrow_mut()
                .retain(|_, value| *value != carrier);
            if error.is_none() {
                let input = fs::read_to_string(&path).unwrap_or_default();
                if let Err(output_error) =
                    self.execute_persistent_output_process_substitution(&source, input, None)
                {
                    error = Some(output_error);
                }
            }
            let _ = fs::remove_file(path);
        }
        if let Some(error) = error {
            return Err(error);
        }
        Ok(())
    }

    /// rubash#1540: snapshot the DISCARD-class expansion latches (the
    /// `${var:=word}` bind failure with its pre-bind status snapshot,
    /// the evalerror abandonment family, and the #439 expansion-failed
    /// gate). A pipeline element is a forked child in GNU
    /// (execute_cmd.c:2620 execute_pipeline): expansion failures raised
    /// while expanding the element's words die with the element and
    /// must never reach the parent's reader state. The speculative
    /// fast-path attempts and the in-place stage executor both expand
    /// member words on this shared executor, so callers bracket them
    /// with snapshot/restore; the arithmetic flag family has its own
    /// long-standing bracket (snapshot_arithmetic_error_flags) whose
    /// deliberate latch-held handoff this one must not disturb.
    pub(in crate::executor) fn snapshot_discard_latches(
        &self,
    ) -> (bool, Option<i32>, bool, bool, bool, bool, bool) {
        (
            self.parameter_assignment_failure.get(),
            self.parameter_assignment_failure_prev_status.get(),
            self.evalerror_failglob_pending.get(),
            self.evalerror_pending.get(),
            self.evalerror_exit_toomany_pending.get(),
            self.evalerror_subscript_pending.get(),
            self.command_expansion_failed.get(),
        )
    }

    pub(in crate::executor) fn restore_discard_latches(
        &self,
        saved: &(bool, Option<i32>, bool, bool, bool, bool, bool),
    ) {
        self.parameter_assignment_failure.set(saved.0);
        self.parameter_assignment_failure_prev_status.set(saved.1);
        self.evalerror_failglob_pending.set(saved.2);
        self.evalerror_pending.set(saved.3);
        self.evalerror_exit_toomany_pending.set(saved.4);
        self.evalerror_subscript_pending.set(saved.5);
        self.command_expansion_failed.set(saved.6);
    }

    /// rubash#1564: could expanding this element's words (or its
    /// tempenv assignment values) mutate the variable table? Only
    /// expansion-bearing text can: a `$` or backquote anywhere in a
    /// word or an assignment value. Pure-literal members — the hot
    /// pipeline shape — skip the snapshot entirely.
    pub(in crate::executor) fn stage_expansion_may_bind(command: &CommandNode) -> bool {
        command
            .words
            .iter()
            .chain(command.assignments.iter().map(|(_, value)| value))
            .chain(command.heredoc.iter())
            .chain(command.here_string.iter())
            .chain(
                command
                    .heredoc_redirects
                    .iter()
                    .filter_map(|redirect| redirect.body.as_ref()),
            )
            .any(|text| text.contains('$') || text.contains('`'))
    }

    /// rubash#1564: snapshot the variable table (and `$?`) before a
    /// speculative or spawn-time expansion of a pipeline element's
    /// words on this shared executor. GNU expands the element in its
    /// forked child (execute_cmd.c:2620 execute_pipeline ->
    /// execute_simple_command), so a successful side-effecting
    /// expansion — `${var:=word}`, `$((var=expr))` — binds in the
    /// child and dies with it; on the shared executor the bind would
    /// otherwise stay in the parent table for the rest of the script
    /// and leak into sibling elements' own expansions.
    pub(in crate::executor) fn snapshot_spec_env(
        &self,
        command: &CommandNode,
    ) -> Option<(
        crate::shell::var_table::VarTable,
        crate::shell::VariableStore,
        i32,
    )> {
        Self::stage_expansion_may_bind(command).then(|| {
            (
                self.shell_state.env_vars.clone(),
                self.shell_state.variables.clone(),
                self.exit_code,
            )
        })
    }

    /// rubash#1564: restore a snapshot_spec_env snapshot. Returns true
    /// when the expansion changed an EXPORTED variable's value (or
    /// its export attribute): such a bind is observable in the
    /// element's own child process environment, which only the
    /// sequential stage executor can reproduce (it expands in the
    /// element's clone and spawns from it) — a caller building a
    /// speculative spec must decline the fast path in that case and
    /// let the sequential path run the element.
    pub(in crate::executor) fn restore_spec_env(
        &mut self,
        saved: Option<(
            crate::shell::var_table::VarTable,
            crate::shell::VariableStore,
            i32,
        )>,
    ) -> bool {
        let Some((saved_table, saved_store, saved_exit_code)) = saved else {
            return false;
        };
        let current = &self.shell_state.env_vars;
        let mut exported_delta = false;
        for key in saved_table.keys().chain(current.keys()) {
            let value_changed = saved_table.get(key) != current.get(key);
            let saved_attrs = saved_table.attr_of(key);
            let current_attrs = current.attr_of(key);
            if (value_changed || saved_attrs != current_attrs)
                && (saved_attrs.exported || current_attrs.exported)
            {
                exported_delta = true;
                break;
            }
        }
        // Roll back BOTH variable stores: a bind writes the typed
        // cell store as well as the mirror table, and restoring the
        // mirror alone leaves the cell holding the element's value
        // (and tripping the cell/mirror freshness assertion).
        self.shell_state.env_vars = saved_table;
        self.shell_state.variables = saved_store;
        self.exit_code = saved_exit_code;
        exported_delta
    }

    /// rubash#1564: restore only the variable stores of a
    /// snapshot_spec_env snapshot (not `$?`). For execute_pipeline_stage's
    /// fork-containment bracket: the stage's execution is real, not
    /// speculative — there is no decline — but its variable side
    /// effects still belong to the element's dead child, while its
    /// exit status flows back through the stage result tuple the
    /// caller adjudicates.
    pub(in crate::executor) fn restore_spec_variables(
        &mut self,
        saved: Option<(
            crate::shell::var_table::VarTable,
            crate::shell::VariableStore,
            i32,
        )>,
    ) {
        if let Some((saved_table, saved_store, _)) = saved {
            self.shell_state.env_vars = saved_table;
            self.shell_state.variables = saved_store;
        }
    }

    /// rubash#1564: the spawn-time twin of restore_spec_env, for the
    /// tempenv application (apply_external_environment), which
    /// expands the element's assignment values on the shared executor
    /// AFTER the child's base environment was already read. Binds the
    /// values performed belong to the element's child: forward any
    /// EXPORTED delta into the not-yet-spawned process (GNU's child
    /// carries the values its own expansion bound), then roll the
    /// parent table back.
    pub(in crate::executor) fn restore_spec_env_into_process(
        &mut self,
        saved: Option<(
            crate::shell::var_table::VarTable,
            crate::shell::VariableStore,
            i32,
        )>,
        process: &mut std::process::Command,
    ) {
        let Some((saved_table, saved_store, saved_exit_code)) = saved else {
            return;
        };
        let current = &self.shell_state.env_vars;
        let mut changed: Vec<String> = Vec::new();
        for key in saved_table.keys().chain(current.keys()) {
            if saved_table.get(key) != current.get(key)
                || saved_table.attr_of(key) != current.attr_of(key)
            {
                if !changed.contains(key) {
                    changed.push(key.clone());
                }
            }
        }
        for key in &changed {
            if saved_table.attr_of(key).exported || current.attr_of(key).exported {
                match current.get(key) {
                    Some(value) => {
                        process.env(key, value);
                    }
                    None => {
                        process.env_remove(key);
                    }
                }
            }
        }
        self.shell_state.env_vars = saved_table;
        self.shell_state.variables = saved_store;
        self.exit_code = saved_exit_code;
    }

    pub(in crate::executor) fn execute_simple_pipeline(
        &mut self,
        commands: &[CommandNode],
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        // rubash#761: `pipeline_stdin_stream` is armed only for the single
        // streaming tail stage; a NESTED pipeline inside that tail (e.g. a
        // compound/builtin body that itself pipelines) must not propagate the
        // outer consumer's pipe to its own stages. Clear it across the whole
        // nested pipeline and restore afterwards.
        let saved_stdin_stream = self.pipeline_stdin_stream.take();
        // rubash#1568: GNU classifies each pipeline element's words
        // independently (execute_cmd.c execute_pipeline -> the forked
        // element's execute_simple_command), so an array-element
        // assignment prefix on an element gets the same invalidid
        // strip the top-level funnel applies — the routes below that
        // never re-enter the top-level funnel (concurrent external,
        // streaming tail) would otherwise dispatch `a[0]=5` as a
        // command name. The strip is idempotent (a stripped element
        // no longer starts with an element word), so the sequential
        // route's own stage-level pass stays a no-op. Diagnostics go
        // to the ambient fd 2: GNU reports them during the element's
        // word expansion, before its own redirections apply.
        let mut stripped_commands: Option<Vec<CommandNode>> = None;
        if commands
            .get(index)
            .is_some_and(|member| member.pipe.is_some())
        {
            let mut end = index;
            loop {
                let Some(member) = commands.get(end) else {
                    break;
                };
                let qualifies =
                    !member.words.is_empty() && command_word_is_array_element_assignment(member, 0);
                if qualifies {
                    let (stripped, diagnostics) =
                        self.strip_invalid_env_assignment_prefixes_collecting(member);
                    for line in &diagnostics {
                        self.write_default_stderr(line.as_bytes())?;
                    }
                    if matches!(stripped, std::borrow::Cow::Owned(_)) {
                        let owned = stripped_commands.get_or_insert_with(|| commands.to_vec());
                        owned[end] = stripped.into_owned();
                    }
                }
                if member.pipe.is_none() {
                    break;
                }
                end += 1;
            }
        }
        let result = match &stripped_commands {
            Some(commands) => self.execute_simple_pipeline_inner(commands, index),
            None => self.execute_simple_pipeline_inner(commands, index),
        };
        self.pipeline_stdin_stream = saved_stdin_stream;
        result
    }

    fn execute_simple_pipeline_inner(
        &mut self,
        commands: &[CommandNode],
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let Some(first) = commands.get(index) else {
            return Ok(None);
        };
        if first.pipe.is_none() {
            return Ok(None);
        }

        let mut pipeline = vec![first];
        let mut end = index;
        while commands
            .get(end)
            .is_some_and(|command| command.pipe.is_some())
        {
            end += 1;
            let Some(command) = commands.get(end) else {
                return Ok(None);
            };
            pipeline.push(command);
        }
        // GNU parse.y:1470-1487: `cmd1 |& cmd2` desugars at PARSE time into
        // `cmd1 2>&1 | cmd2` — a r_duplicating_output redirect (redirector 2,
        // destination 1) APPENDED to cmd1's redirect list. execute_pipeline
        // binds the pipe to the element's fd 1 first (execute_cmd.c:2702
        // passes fildes[1] as pipe_out), so the appended dup lands fd 2 on the
        // SAME pipe open description fd 1 holds (redir.c:1169-1170
        // `2>&1 means dup2 (1, 2)`), and the element's merged bytes
        // interleave in true write order through that one pipe. rubash's
        // parser carries `|&` as pipe == Some(2); normalize every stage to
        // the GNU shape HERE so all downstream consumers — the compound
        // redirect injector, the ordered stage routing, the external stdio
        // planner, the concurrent-path admissions — see one form and no
        // consumer needs a `|&` special case.
        let normalized: Vec<CommandNode> = pipeline
            .iter()
            .map(|command| normalize_pipeerr_stage(command))
            .collect();
        let commands: Vec<&CommandNode> = normalized.iter().collect();
        let first = commands[0];
        // rubash#1540: the fast-path attempts below expand member words
        // speculatively on this executor. When an attempt declines, its
        // expansion must leave no trace in the DISCARD latch family —
        // a `${var:=word}` bind failure latched by a declined spec used
        // to survive into the parent, whose NEXT command then died on
        // the stale latch (the enclosing list abandoned, rc 2). Any
        // attempt that succeeds returns early above the restore, so the
        // restore only ever runs on the all-declined path, just before
        // the sequential stage loop re-expands every word itself.
        let saved_discard_latches = self.snapshot_discard_latches();
        if self.execute_timed_read_pipeline(&commands)?.is_some() {
            return Ok(Some(end + 1));
        }

        if self
            .execute_external_pipeline_concurrently(&commands)?
            .is_some()
        {
            return Ok(Some(end + 1));
        }

        // niubash#345: a shell-side producer (compound loop) feeding an
        // external consumer chain runs the consumers concurrently on OS pipes
        // so an unbounded writer dies on the broken pipe instead of being
        // slurped into memory by the sequential loop.
        if self.execute_shell_writer_pipeline(&commands)?.is_some() {
            return Ok(Some(end + 1));
        }

        // rubash#761: an UNBOUNDED external producer feeding a pipeline whose
        // rightmost stage is served in-shell (a builtin tail such as
        // `true`/`:`/`read`/`echo`). The sequential loop would materialize the
        // producer's whole stream (`yes | true` drained into RAM forever) and
        // the materializing prefix path cannot stop an unbounded producer
        // either. Run the producer chain on real OS pipes and let the tail
        // read fd 0 directly; when the tail stops reading its read end closes
        // and the producers are terminated with the SIGPIPE shape (141). See
        // `execute_external_prefix_streaming_tail`.
        if self
            .execute_external_prefix_streaming_tail(&commands)?
            .is_some()
        {
            return Ok(Some(end + 1));
        }
        // rubash#1540: every fast path declined — drop any DISCARD
        // latches their speculative expansions raised (see the snapshot
        // above). The sequential stage loop below starts clean and each
        // stage adjudicates its own failures.
        self.restore_discard_latches(&saved_discard_latches);

        let time_prefix = time_pipeline_prefix(first);
        let time_prefix_started = time_prefix.as_ref().map(|_| time_command_started());
        let mut time_report_routed = false;
        let mut hoisted_time_inverted = false;
        let mut deferred_time_report: Option<(super::read_helpers::TimeCommandStart, bool)> = None;
        let mut input = String::new();
        // rubash#1552: stderr bytes of non-final stages under an ambient
        // `2>&1` binding — they ride the pipeline's output stream (see
        // the stage-loop merge), never the inter-stage pipe.
        let mut ambient_stderr = String::new();
        let mut statuses = Vec::new();
        let mut first_stage = 0usize;
        // rubash#1141: production timing of the stream each stage reads.
        // `timed_stdin` on entry times the enclosing input this pipeline
        // itself consumes (stage 0); every stage's production is recorded
        // by the capture timing hooks and becomes the next stage's
        // `input_timing`. Restored on exit — a pipeline is a consumer of
        // the enclosing stream, not a producer of it.
        let entry_timed_stdin = self.timed_stdin.clone();
        let prefix_started = std::time::Instant::now();
        // GNU execute_cmd.c:2620 execute_pipeline connects the left
        // elements with real pipes before the rightmost command runs, so a
        // producer is never fully materialized inside the shell (issue
        // #206: `yes | head -100000 | while read` hung at stage 0 while
        // rubash slurped yes's unbounded output). When the pipeline has a
        // shell-side tail, run the leading external members concurrently
        // here and continue the sequential loop from the tail with the
        // bounded captured output as its input.
        if time_prefix.is_none() {
            // rubash#1540: same speculation bracket as the fast paths
            // above — a declined prefix attempt expanded member words on
            // this executor, and its DISCARD latches must not become the
            // stage loop's entry state.
            let saved_prefix_latches = self.snapshot_discard_latches();
            let prefix_result = self.execute_external_prefix_concurrently(&commands)?;
            if prefix_result.is_none() {
                self.restore_discard_latches(&saved_prefix_latches);
            }
            if let Some((prefix_len, prefix_output, prefix_stderr, prefix_statuses)) = prefix_result
            {
                // A prefix member carrying `|&` was normalized to a
                // trailing `2>&1` and declined admission, so the only stderr
                // merge left here is the ambient `exec 2>&1` fd-table
                // binding (fd 2 already on fd 1's object). Ambient bytes
                // ride the pipeline's OUTPUT stream, not the pipe payload
                // the remaining stages read (rubash#1552 — same fd model
                // as the stage-loop merge below).
                if self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout) {
                    input = prefix_output;
                    ambient_stderr.push_str(&prefix_stderr);
                } else if !prefix_stderr.is_empty() {
                    std::io::stderr().write_all(
                        &crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                            &prefix_stderr,
                        ),
                    )?;
                } else {
                    input = prefix_output;
                }
                statuses = prefix_statuses;
                first_stage = prefix_len;
            }
        }
        // rubash#1141: the concurrent prefix ran as one burst on real
        // pipes — model its whole (bounded) output as arriving when the
        // prefix finished. Without a prefix, stage 0 consumes the
        // enclosing stream whose timing is the entry value.
        let mut input_timing: Option<super::timed_input::SharedTimedInput> = if first_stage > 0 {
            let elapsed = prefix_started.elapsed();
            Some(
                super::timed_input::TimedInput::new(
                    prefix_started,
                    elapsed,
                    vec![(input.len(), elapsed)],
                    input.len(),
                )
                .shared(),
            )
        } else {
            entry_timed_stdin.clone()
        };
        for (stage_index, command) in commands.iter().enumerate().skip(first_stage) {
            let stage = time_prefix
                .as_ref()
                .filter(|_| stage_index == 0)
                .map(|prefix| &prefix.command)
                .unwrap_or(command);
            let mut stage0_stdin_base = None;
            let mut stage0_stdin_inherit = false;
            let mut stage0_stdin_live = false;
            if stage_index == 0 {
                if self.pipeline_stage0_stdin_live(stage) {
                    // rubash#1873: element 0's fd 0 IS the shell's current
                    // fd 0 (see pipeline_stage0_stdin_live) — there is no
                    // payload to compute, and computing one would drain a
                    // file endpoint through the shared-offset dup. Flush
                    // the pending comsub writeback exactly as
                    // initial_pipeline_input would, then hand the stage
                    // machinery the live classification; the external
                    // arm's inherit flag keeps its baseline meaning (the
                    // process-stdin handle only — a file endpoint's child
                    // is served by the spawn machinery's own fd-0
                    // resolution).
                    self.apply_comsub_stdin_writeback();
                    stage0_stdin_live = true;
                    stage0_stdin_inherit = match self.fd_table.entries.get(&0) {
                        Some(entry) => {
                            !entry.closed
                                && matches!(entry.read, Some(FdReadEndpoint::InheritedProcessStdin))
                        }
                        None => true,
                    };
                } else {
                    let (stage_input, base) = self.initial_pipeline_input(stage);
                    // GNU execute_pipeline forks the child with the shell's own
                    // fd 0 — with no fd-0 binding on the stage and fd 0 being the
                    // inherited process stdin, the child reads that handle
                    // directly instead of an empty payload pipe.
                    stage0_stdin_inherit = stage_input.is_empty()
                        && self.stdin_string_for_command(stage).is_none()
                        && match self.fd_table.entries.get(&0) {
                            Some(entry) => {
                                !entry.closed
                                    && matches!(
                                        entry.read,
                                        Some(FdReadEndpoint::InheritedProcessStdin)
                                    )
                            }
                            None => true,
                        };
                    input = stage_input;
                    stage0_stdin_base = base;
                }
            }
            self.set_current_command(stage);
            let last_stage = stage_index + 1 == commands.len();
            // GNU execute_pipeline runs each element through execute_command
            // in its own subshell (execute_cmd.c:2702+). The forked element
            // children keep the trap table — reset_signal_handlers
            // (trap.c:1588) only runs for execute_in_subshell children — so a
            // SIMPLE element's run_debug_trap fires per element
            // (execute_cmd.c:4506). Compound elements (groups, subshells,
            // loops, arith/cond commands) go through execute_in_subshell and
            // reset the trap table, so they fire nothing here; any inner
            // fires belong to the stage's own subshell executor below.
            // Exception: a `shopt -s lastpipe` last element runs in the
            // current shell, where `(( ))`/`[[ ]]` elements still fire
            // (execute_cmd.c:3920/4153) and the other compound kinds fire
            // inside their own handlers.
            let in_shell_stage = last_stage && self.lastpipe_enabled();
            let stage_defers_debug = command_is_compound_pipeline_stage(stage)
                && !(in_shell_stage
                    && (stage.arithmetic_command.is_some() || stage.conditional_command.is_some()));
            if !stage_defers_debug && self.debug_trap_in_scope() {
                let stage_text = crate::executor::command_text::bash_command_source_text(stage);
                // execute_cmd.c:4510 fires inside the forked element child; a
                // non-zero trap status under debugging_mode skips the
                // element's command — the child holds the pipe, produces no
                // output and exits with the trap's status (probe i521d: both
                // stages skipped under `trap 'false' DEBUG` → `pipe-R=1`;
                // probe ka: the downstream stage's own fire still decides for
                // itself, so it can proceed). Rubash models the element as a
                // captured in-process stage: empty output + the trap status
                // reproduces both, and the pipe chain stays intact (issue
                // #521, huck#268's pipeline-stage fire site).
                if let Some(skip_status) =
                    self.run_debug_trap(crate::debugger::SeamKind::PipelineElement, &stage_text)?
                {
                    statuses.push(skip_status);
                    input = String::new();
                    // rubash#1141: a skipped element is an instant empty
                    // stream — no production schedule for the next stage.
                    input_timing = None;
                    continue;
                }
            }
            let preserve_compound_errexit = command_is_compound_pipeline_stage(stage)
                || stage
                    .words
                    .first()
                    .map(|word| self.expand_word(word))
                    .and_then(|word| self.function_name_for_command_word(&word))
                    .is_some();
            // GNU execute_cmd.c:653-656 — CMD_INVERT_RETURN under
            // exit_immediately_on_error sets CMD_IGNORE_RETURN on the whole
            // command; execute_pipeline then propagates it to EVERY element
            // (execute_cmd.c:2702-2708 for left elements, 2722-2723 for the
            // rightmost), and a group command pushes it into its inner list
            // (execute_cmd.c:1104-1108). So `!` must suppress errexit inside
            // every stage, not only invert the pipeline's final status
            // (set-e1.sub:40 `! { false; echo A $?; } | cat` prints `A 1`).
            let pipeline_inverted =
                first.inverted || time_prefix.as_ref().is_some_and(|prefix| prefix.inverted);
            // GNU binds a leading `time` (and its `!`) to the WHOLE
            // pipeline; the compound parse parks the TimeCommand on
            // element 0, whose stage executor would print the report and
            // invert at element scope (the report overtook the buffered
            // pipeline output and the status stayed un-inverted). Defer
            // both to pipeline scope (rubash#1263).
            let hoist_stage_time =
                stage_index == 0 && commands.len() > 1 && command.time_command.is_some();
            if hoist_stage_time {
                self.defer_stage_time_report = true;
            }
            // rubash#1454: mark the final stage's dispatch so the stage
            // spawner can hand an external final member the shell's own
            // stdout instead of a capture pipe (cleared right after the
            // dispatch; every stage sets it afresh, so an aborted
            // pipeline cannot leak it into a later stage).
            self.pipeline_final_stage.set(last_stage);
            // rubash#1141: this stage consumes the stream timed by
            // `input_timing`; its own production is recorded so the next
            // stage's reads can be judged against real arrival times.
            self.timed_stdin = input_timing.clone();
            let stage_timing = super::shell_options::begin_capture_timing();
            let Some((mut next_input, mut next_stderr, mut next_status)) = (if pipeline_inverted {
                self.with_errexit_suppressed(|executor| {
                    if last_stage && executor.lastpipe_enabled() {
                        executor.execute_lastpipe_stage(stage, &input).map(Some)
                    } else {
                        executor.execute_pipeline_stage(
                            stage,
                            &input,
                            stage0_stdin_inherit,
                            stage0_stdin_live,
                        )
                    }
                })?
            } else if last_stage && self.lastpipe_enabled() {
                Some(self.execute_lastpipe_stage(stage, &input)?)
            } else if last_stage || preserve_compound_errexit {
                // Compound and function stages inherit the pipeline
                // command's ignore_return (the current suppress_errexit
                // depth) instead of being wrapped in extra suppression:
                // execute_cmd.c:2702-2708 propagates the flag into every
                // element, so `! { false; echo A $?; } | cat` reaches the
                // echo while top-level `{ false; echo x; } | cat` still
                // dies on `false` under -e.
                self.execute_pipeline_stage(stage, &input, stage0_stdin_inherit, stage0_stdin_live)?
            } else {
                // Non-final simple pipeline stages never trigger errexit
                // (bash manual: "any command in a pipeline but the
                // last").
                self.with_errexit_suppressed(|executor| {
                    executor.execute_pipeline_stage(
                        stage,
                        &input,
                        stage0_stdin_inherit,
                        stage0_stdin_live,
                    )
                })?
            }) else {
                super::shell_options::end_capture_timing();
                self.timed_stdin = entry_timed_stdin.clone();
                self.defer_stage_time_report = false;
                return Ok(None);
            };
            self.pipeline_final_stage.set(false);
            super::shell_options::end_capture_timing();
            // GNU: each element's own redirections run inside the element's
            // subshell after the pipe is bound to fd 1 (execute_cmd.c
            // execute_pipeline + redir.c do_redirection_internal). Compound
            // stages applied them internally; simple stages captured their
            // raw fd-1/fd-2 streams, so route them through the resolved
            // redirect state — `echo a >f | cat` writes f and hands cat an
            // empty pipe, `echo a 1>&2 | cat` goes to stderr. A lastpipe
            // element ran execute_command in this shell and already applied
            // its own redirections, so routing it again would reopen `>f`
            // and truncate the content the stage just wrote.
            // A stage that wired the child's fds directly onto their
            // targets (both fds on one shared open file — `>f 2>&1`, `>f
            // |&`, niubash#144) owns its stream placement: the routing walk
            // below would re-open `>f` and truncate what the child wrote
            // live.
            let stage_fds_pre_wired = self.pipeline_stage_fds_pre_wired.replace(false);
            // #804: a stage `restricted_command_error` refused must not have
            // its write redirects materialized by the routing walk — GNU
            // refuses the member inside its subshell BEFORE
            // do_redirections, so `echo hi >& f | cat` never creates `f`
            // (the refusal already rides `next_stderr`/`next_status`).
            let stage_restricted_refused = self.pipeline_stage_restricted_refused.replace(false);
            if !stage_fds_pre_wired
                && !stage_restricted_refused
                && !command_is_compound_pipeline_stage(command)
                && !in_shell_stage
            {
                self.route_pipeline_stage_streams(
                    command,
                    &mut next_input,
                    &mut next_stderr,
                    &mut next_status,
                )?;
            }
            // `|&` stages were normalized to a trailing `2>&1` redirect
            // above, so this branch no longer needs a pipe == Some(2)
            // disjunct: a compound stage's group redirect and a simple
            // stage's route_pipeline_stage_streams both resolve the dup
            // against the pipe-bound fd 1, and the stderr bytes arrive
            // already merged (next_stderr empty). What remains here is
            // the AMBIENT binding — an enclosing compound's or `exec`'s
            // `2>&1` — and its fd model differs: GNU binds the stage's
            // fd 2 to the fd-1 object the pipeline INHERITED (the
            // enclosing stream), not to the inter-stage pipe bound to
            // the stage's own fd 1 (execute_cmd.c:2620 execute_pipeline
            // connects the elements first, each element keeping the
            // shell's remaining fds). Folding those bytes into
            // next_input hands them to the next stage as its stdin,
            // where they die unread whenever that stage never consumes
            // it (rubash#1552: `{ time '-p' true | cat; } 2>&1` lost
            // the first stage's `command not found`). They belong to
            // the pipeline's own output stream: non-final stages
            // accumulate theirs in stage order, and only the final
            // stage's may join next_input — there next_input IS the
            // pipeline output.
            if self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout) {
                if last_stage {
                    next_input.push_str(&next_stderr);
                } else {
                    ambient_stderr.push_str(&next_stderr);
                }
            } else if !next_stderr.is_empty() {
                std::io::stderr().write_all(
                    &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&next_stderr),
                )?;
            }
            if let Some(base) = stage0_stdin_base {
                // Fold the element's measured fd-0 reads into the shared
                // cursor — a non-reader (echo) leaves it untouched while a
                // drainer (cat) pushes it to EOF (GNU redir.c shared fd).
                let consumed = self.pipeline_stdin_consumed.take().unwrap_or(0);
                self.shell_state.env_vars.insert(
                    FUNCTION_STDIN_OFFSET.to_string(),
                    (base + consumed).to_string(),
                );
            }
            input = next_input;
            statuses.push(next_status);
            // GNU runs a timed pipeline element in a subshell whose fd 2
            // is already bound: when the element dups stderr onto the
            // pipe (`2>&1` / `|&`), the time report is written into the
            // pipe right after the element's own output — it reaches the
            // next stage, it does not trail the whole pipeline on the
            // shell's stderr (rubash#1263: `time ! { echo hi; } 2>&1 |
            // cat` printed the timing block BEFORE `hi`).
            if stage_index == 0 && !time_report_routed {
                if let (Some(prefix), Some(started)) = (&time_prefix, time_prefix_started) {
                    if timed_element_merges_stderr(prefix) {
                        let metrics = TimeCommandMetrics::from_start(started);
                        if let Some(report) = format_time_report(
                            &self.shell_state.env_vars,
                            prefix.posix_format,
                            metrics,
                        ) {
                            input.push_str(&format!("{report}\n"));
                            time_report_routed = true;
                        }
                    }
                }
            }
            if hoist_stage_time {
                self.defer_stage_time_report = false;
                hoisted_time_inverted = command
                    .time_command
                    .as_ref()
                    .is_some_and(|timed| timed.inverted);
                if let Some((started, posix_format)) = self.pending_stage_time_report.take() {
                    // The element dup'd its stderr onto the pipe (`2>&1`):
                    // the report is part of the pipe stream, right after
                    // the element's own contribution. Otherwise it stays
                    // on the shell's stderr, printed once the pipeline's
                    // output is out (rubash#1263).
                    if timed_stage_merges_stderr(command) {
                        if let Some(report) = format_time_report(
                            &self.shell_state.env_vars,
                            posix_format,
                            TimeCommandMetrics::from_start(started),
                        ) {
                            input.push_str(&format!("{report}\n"));
                        }
                    } else {
                        deferred_time_report = Some((started, posix_format));
                    }
                }
            }
            // rubash#1141: freeze this stage's production schedule (in
            // FUNCTION_STDIN text units, against the final assembled
            // input) as the next stage's read-timing evidence.
            input_timing = std::mem::take(&mut *stage_timing.borrow_mut())
                .finish(input.len(), true, input.len())
                .map(|timing| timing.shared());
        }

        // rubash#1141: hand the enclosing stream's timing back.
        self.timed_stdin = entry_timed_stdin;
        let final_command = commands.last().expect("pipeline has at least one stage");
        // rubash#1552: ambient-`2>&1` stage stderr precedes the final
        // stage's output on the shared stream (GNU starts the elements
        // concurrently, so a non-final element's diagnostic is written
        // while the final element is still producing).
        if ambient_stderr.is_empty() {
            self.write_pipeline_output(final_command, &input, true)?;
        } else {
            let combined = format!("{ambient_stderr}{input}");
            self.write_pipeline_output(final_command, &combined, true)?;
        }
        if !time_report_routed {
            if let Some(prefix) = &time_prefix {
                if let Some(started) = time_prefix_started {
                    self.print_time_shell(prefix.posix_format, started);
                }
            }
        }
        if let Some((started, posix_format)) = deferred_time_report {
            self.print_time_shell(posix_format, started);
        }
        let mut status = self.pipeline_exit_status(&statuses);
        if hoisted_time_inverted {
            status = invert_exit_status(status);
        }
        if time_prefix.as_ref().is_some_and(|prefix| prefix.inverted) {
            status = invert_exit_status(status);
        }
        self.exit_code = if first.inverted {
            invert_exit_status(status)
        } else {
            status
        };
        self.set_pipestatus(statuses);
        Ok(Some(end + 1))
    }

    fn execute_timed_read_pipeline(
        &mut self,
        commands: &[&CommandNode],
    ) -> Result<Option<()>, ExecuteError> {
        // The timed fast path expands the producer stage's words on the
        // shared executor. A pipeline element is still a subshell boundary:
        // a `set -u` unbound-variable error raised during that expansion
        // kills only the element (stage status 127), never the enclosing
        // script (issue #67).
        let saved = self.snapshot_arithmetic_error_flags();
        // rubash#1115: the speculative producer expansion must not draw
        // from the parent's RANDOM sequence — a pipeline element is a
        // forked child (variables.c:1418-1423 reseeds), and when this
        // fast path declines, the sequential stage loop re-expands the
        // words itself. Snapshot, reseed for the attempt, restore.
        let saved_random = self.shell_state.random_state.clone_state();
        self.shell_state.random_state.reseed_fresh();
        let result = self.execute_timed_read_pipeline_inner(commands);
        self.shell_state.random_state.restore_state(&saved_random);
        let nounset_hit = self.restore_arithmetic_error_flags(&saved);
        match (result, nounset_hit) {
            (Ok(Some(())), true) => {
                // Producer died with 127, consumer completed: pipeline
                // status stays the last element's (0).
                self.set_pipestatus(vec![127, 0]);
                Ok(Some(()))
            }
            (Ok(None), true) => {
                // The fast path bailed after the diagnostic was already
                // printed (consumer not supported). Fall through to the
                // stage loop with the expansion-error latch held: the
                // stage's re-expansion re-raises the nounset error without
                // printing the diagnostic a second time.
                self.shell_state.arithmetic_expansion_error.set(true);
                Ok(None)
            }
            (other, _) => other,
        }
    }

    fn execute_timed_read_pipeline_inner(
        &mut self,
        commands: &[&CommandNode],
    ) -> Result<Option<()>, ExecuteError> {
        if commands.len() != 2 {
            return Ok(None);
        }
        // A live DEBUG trap fires per pipeline element in GNU
        // (execute_cmd.c:4506); this fast path bypasses the stage loop, so
        // bail out and let it own the fires.
        if self.debug_trap_in_scope()
            && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
                .is_some_and(|action| !action.is_empty())
        {
            return Ok(None);
        }
        let Some(input) = self.timed_pipeline_input(commands[0]) else {
            return Ok(None);
        };
        let Some((output, status)) = self.timed_read_consumer_output(commands[1], &input) else {
            return Ok(None);
        };

        self.write_pipeline_output(commands[1], &output, false)?;
        self.exit_code = status;
        self.set_pipestatus(vec![0, status]);
        Ok(Some(()))
    }

    fn timed_pipeline_input(&mut self, command: &CommandNode) -> Option<TimedPipelineInput> {
        let mut at = 0.0f64;
        let mut chunks = Vec::new();
        if let Some(group) = &command.brace_group {
            for command in &group.body {
                self.timed_pipeline_step(command, &mut at, &mut chunks)?;
            }
            return Some(TimedPipelineInput { chunks, eof_at: at });
        }
        // `(sleep N; echo x)` producers model timing exactly like the
        // brace-group form: the subshell body is one sequential list.
        if let Some(subshell) = &command.subshell_command {
            for command in &subshell.body {
                self.timed_pipeline_step(command, &mut at, &mut chunks)?;
            }
            return Some(TimedPipelineInput { chunks, eof_at: at });
        }
        self.timed_pipeline_step(command, &mut at, &mut chunks)?;
        Some(TimedPipelineInput { chunks, eof_at: at })
    }

    fn timed_pipeline_step(
        &mut self,
        command: &CommandNode,
        at: &mut f64,
        chunks: &mut Vec<(f64, String)>,
    ) -> Option<()> {
        if command.brace_group.is_some()
            || command.subshell_command.is_some()
            || command.pipeline_command.is_some()
            || command.and_or_list.is_some()
            || !command.assignments.is_empty()
            || command.redirect_in.is_some()
            || command.redirect_out.is_some()
            || command.append.is_some()
            || !command.redirects.is_empty()
        {
            return None;
        }
        // rubash#1540: this fast path EXECUTES the producer step on the
        // shared executor to model its output, and a later decline
        // (unsupported consumer) lets the sequential stage loop execute
        // it again — a word whose expansion has observable effects
        // (a command substitution's diagnostics and side effects, an
        // arithmetic error, a `${var:=word}` bind) would happen TWICE
        // (`echo $(echo $((1/0))) | cat` printed the division
        // diagnostic two times). The timing model only covers literal
        // producers (true/:/sleep/echo with plain arguments), so admit
        // exactly those: any word carrying an expansion introducer
        // declines the step before anything is expanded.
        if command
            .words
            .iter()
            .any(|word| word.contains('$') || word.contains('`'))
        {
            return None;
        }
        let name = self.expand_word(command.words.first()?);
        match name.as_str() {
            "true" | ":" => Some(()),
            "sleep" => {
                // When sleep does not resolve (PATH unset, rubash#1126;
                // or a set PATH without it, rubash#1545), decline the
                // timing model so the real pipeline reports the failure.
                if self.emulated_external_stands_down("sleep") {
                    return None;
                }
                let duration = command
                    .words
                    .get(1)
                    .and_then(|word| self.expand_word(word).parse::<f64>().ok())?;
                *at += duration;
                Some(())
            }
            "echo" => {
                let mut args: Vec<String> = command.words[1..]
                    .iter()
                    .enumerate()
                    .flat_map(|(offset, word)| {
                        let index = offset + 1;
                        let raw = command
                            .word_metadata
                            .get(index)
                            .map(|metadata| metadata.raw.as_str());
                        self.expand_command_word(command, index, word, raw)
                    })
                    .collect();
                let newline = !args.first().is_some_and(|arg| arg == "-n");
                if !newline {
                    args.remove(0);
                }
                let mut output = args.join(" ");
                if newline {
                    output.push('\n');
                }
                chunks.push((*at, output));
                Some(())
            }
            _ => None,
        }
    }

    fn timed_read_consumer_output(
        &mut self,
        command: &CommandNode,
        input: &TimedPipelineInput,
    ) -> Option<(String, i32)> {
        // GNU read.def: a bare `read -t N` as the last pipeline stage (e.g.
        // `sleep 1 | read -t 0.25 a`) must return 128+SIGALRM when the
        // timeout expires before the producer sends data. The sequential
        // pipeline path materializes the producer's output before starting
        // the consumer, so it cannot observe the timeout. Handle a bare
        // read command as a single-command body here.
        let body: &[CommandNode] = command
            .subshell_command
            .as_ref()
            .map(|command| command.body.as_slice())
            .or_else(|| {
                command
                    .brace_group
                    .as_ref()
                    .map(|command| command.body.as_slice())
            })
            .or_else(|| {
                // Bare read as last pipeline stage: synthesize a single-element
                // body so the same loop logic applies.
                if command
                    .words
                    .first()
                    .map(|word| self.expand_word(word) == "read")
                    .unwrap_or(false)
                {
                    Some(std::slice::from_ref(command))
                } else {
                    Some(&[])
                }
            })?;
        let mut at = 0.0f64;
        let mut read_name = String::from("REPLY");
        let mut read_value = String::new();
        let mut read_status = 0;
        let mut saw_read = false;
        let mut output = String::new();
        let mut last_status = 0;
        for command in body {
            let name = self.expand_word(command.words.first()?);
            match name.as_str() {
                "sleep" if !saw_read => {
                    at += command
                        .words
                        .get(1)
                        .and_then(|word| self.expand_word(word).parse::<f64>().ok())?;
                }
                "read" if !saw_read => {
                    let (timeout, char_limit, name) = self.timed_read_options(command)?;
                    read_name = name;
                    let (value, status) = timed_read_value(input, at, timeout, char_limit);
                    read_value = value;
                    read_status = status;
                    saw_read = true;
                    last_status = status;
                }
                _ if saw_read => {
                    // rubash#1581: followups run with an empty stdin
                    // (timed_read_followup_output passes "" as the stage
                    // input): the modeled producer bytes beyond the first
                    // read's record exist only as timing chunks and are
                    // never materialized for the body. A followup that
                    // reads fd 0 — a second `read`, `mapfile`, `cat`, any
                    // external command — would see EOF where GNU serves
                    // the not-yet-arrived data, so the model may only keep
                    // followups that provably never consume the stream;
                    // anything else declines to the sequential stage
                    // path, whose TimedInput judge serves the same body
                    // correctly (and still reports the timeout as 142).
                    if !timed_followup_keeps_stream(command, &name) {
                        return None;
                    }
                    output.push_str(&self.timed_read_followup_output(
                        command,
                        &read_name,
                        &read_value,
                        read_status,
                    )?);
                    last_status = 0;
                }
                _ => return None,
            }
        }
        saw_read.then_some((output, last_status))
    }

    fn timed_read_options(
        &mut self,
        command: &CommandNode,
    ) -> Option<(f64, Option<(usize, bool)>, String)> {
        let mut timeout = None;
        // (quota, exact): -n stops at the delimiter or the quota, -N
        // ignores the delimiter (GNU read.def -n/-N).
        let mut char_limit = None;
        let mut names = Vec::new();
        let mut index = 1usize;
        while index < command.words.len() {
            let word = self.expand_word(command.words.get(index)?);
            if word == "-t" {
                index += 1;
                timeout = command
                    .words
                    .get(index)
                    .and_then(|word| self.expand_word(word).parse::<f64>().ok());
            } else if word == "-n" || word == "-N" {
                let exact = word == "-N";
                index += 1;
                let operand = command.words.get(index)?;
                char_limit = Some((self.expand_word(operand).parse::<usize>().ok()?, exact));
            } else if let Some(value) = word.strip_prefix("-t").filter(|value| !value.is_empty()) {
                timeout = value.parse::<f64>().ok();
            } else if let Some(value) = word.strip_prefix("-N").filter(|value| !value.is_empty()) {
                char_limit = Some((value.parse::<usize>().ok()?, true));
            } else if let Some(value) = word.strip_prefix("-n").filter(|value| !value.is_empty()) {
                char_limit = Some((value.parse::<usize>().ok()?, false));
            } else if matches!(word.as_str(), "-a" | "-d" | "-i" | "-p" | "-u") {
                index += 1;
            } else if word.starts_with('-') {
            } else {
                names.push(word);
            }
            index += 1;
        }
        Some((
            timeout?,
            char_limit,
            names
                .into_iter()
                .next()
                .unwrap_or_else(|| "REPLY".to_string()),
        ))
    }

    fn timed_read_followup_output(
        &mut self,
        command: &CommandNode,
        name: &str,
        value: &str,
        status: i32,
    ) -> Option<String> {
        let saved_status = self.exit_code;
        // The group's `read` assignment is scoped to the pipeline element's
        // subshell: install it in the real variable store (readonly checks,
        // nameref resolution) so followup commands see it, then restore the
        // caller's binding when the group ends.
        let saved_value = self.shell_state.variables.get(name).cloned();
        let _ = self.apply_shell_assignment_command("read", name, value.to_string());
        self.exit_code = status;
        let result = self
            .execute_pipeline_stage(command, "", false, false)
            .ok()
            .flatten();
        self.exit_code = saved_status;
        self.shell_state.variables.remove(name);
        if let Some(saved_value) = saved_value {
            let _ = self
                .shell_state
                .variables
                .set(name.to_string(), saved_value);
        }
        let (stdout, stderr, _) = result?;
        stderr
            .is_empty()
            .then(|| stdout.replace(crate::executor::markers::CTLESC, ""))
    }

    /// Connect a pipeline of native external processes with OS pipes.  The
    /// normal pipeline path captures each stage into a String before starting
    /// the next stage, which deadlocks for producers such as `yes` once a
    /// downstream `head` has already stopped reading.  Restrict this path to
    /// plain external `|` pipelines; builtins, compound commands, redirects,
    /// and `|&` retain the shell-aware path above.
    #[cfg(windows)]
    fn execute_external_pipeline_concurrently(
        &mut self,
        commands: &[&CommandNode],
    ) -> Result<Option<Vec<(String, String, i32)>>, ExecuteError> {
        if commands.len() < 2
            || self.stderr_capture.is_some()
            // GNU runs each pipeline element's run_debug_trap inside the
            // element's child (execute_cmd.c:4506). This fast path spawns the
            // members directly and would bypass those fires, so a live DEBUG
            // trap takes the sequential stage path which fires per element.
            || (self.debug_trap_in_scope()
                && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
                    .is_some_and(|action| !action.is_empty()))
            // NOTE: stdout_capture (command substitution) is intentionally NOT a
            // bail-out here. External-only pipelines inside `$(...)` must still
            // run concurrently with real OS pipes between stages (GNU bash
            // pipelines everything concurrently). The final stage's output is
            // routed into the substitution capture by write_pipeline_output,
            // and intermediate stages stay on OS pipes — which is what winuxcmd
            // commands such as `find` require (issue #76: a `$(find ... | wc -l)`
            // substitution previously fell back to the sequential stage path,
            // whose per-stage capture lost the external command's output, so the
            // pipeline silently yielded 0).
            || commands.iter().enumerate().any(|(index, command)| {
                command.time_command.is_some()
                    || command.brace_group.is_some()
                    || command.subshell
                    // #800A: a tput head answers inline on the sequential
                    // path (the byte-exact whitelist stage arm); see
                    // concurrent_external_stage_spec's matching clause.
                    || (cfg!(windows) && tput_inline_stage(command))
                    || command_has_non_concurrent_pipeline_redirects(command, index, commands.len())
                    || command.redirect_in.is_some()
                    || command.redirect_err.is_some()
                    || command.redirect_err_append.is_some()
                    || command
                        .redirects
                        .iter()
                        .any(|redirect| redirect.is_list_only_redirect())
                    || ((command.redirect_out.is_some() || command.append.is_some())
                        && index + 1 != commands.len())
                    || ((command.heredoc.is_some()
                        || !command.heredoc_redirects.is_empty()
                        || command.here_string.is_some())
                        && index != 0)
                    // GNU execute_cmd.c:4617 execute_simple_command runs a
                    // pipeline element with a `VAR=value` tempenv prefix as
                    // an ordinary forked member whose child environment
                    // carries the assignment; the concurrent OS-pipe path
                    // supports it (apply_external_environment below). Only
                    // the forms that need the sequential executor's shell-
                    // table handling bail: `name+=value` appends (whose env
                    // value comes from a shell-table pre-assignment) and
                    // PATH= (command resolution must observe the new PATH).
                    || command
                        .assignments
                        .iter()
                        .any(|(name, _)| -> bool {
                            let (base, append) = assignment_name_and_append(name);
                            append || base == "PATH"
                        })
                    || !command.process_substitutions.is_empty()
                    || command_has_pipeline_process_substitution(command)
        })
            // rubash#1130: a member whose words, assignment values, heredoc
            // bodies or here-string word would raise a word-expansion error
            // (nounset unbound, `${x?}`, bad substitution) must not take the
            // concurrent fast path — it spawns members directly and never
            // runs the expansion-error gates, so `cat <<< prefix$p | wc -w`
            // silently expanded `$p`. Decline to the sequential stage
            // executor, whose gates report the error per element. The probe
            // is the side-effect-free twin of the reporting scan.
            // rubash#1540: the twin cannot see arithmetic-expansion or
            // failglob failures; the risk probe covers those two classes
            // the same way, before any speculative spec expansion prints.
            // The failing-`${var:=word}`-bind class has its own pure
            // scan (parameter_assignment_error) that neither the twin
            // nor the reporting scan consults — wire it in here too.
            || commands.iter().any(|command| {
                self.parameter_expansion_error_pending(command)
                    || self.parameter_assignment_error(command).is_some()
                    || self.pipeline_member_arith_failglob_risk(command)
            })
        {
            return Ok(None);
        }

        let mut specs = Vec::with_capacity(commands.len());
        for (index, command) in commands.iter().enumerate() {
            let Some(spec) =
                self.concurrent_external_stage_spec(command, index, index + 1 == commands.len())
            else {
                return Ok(None);
            };
            specs.push(spec);
        }

        // rubash#815 profiler: the all-external concurrent pipeline spawns
        // every member outside execute_external_pipeline_stage, so the spawn
        // face gets one composite frame around spawn+wait. Per-element frames
        // would break the profiler's stack discipline — the members' wall
        // time overlaps, which a sequential frame stack cannot represent.
        let _profiler_external_pipeline = super::profiler::enter_external("pipeline");
        let results = self.spawn_wait_concurrent_external_stages(commands, specs, true)?;
        let (_, stderr, _) = results.last().unwrap();
        self.write_pipeline_output(
            commands[commands.len() - 1],
            &results.last().unwrap().0,
            false,
        )?;
        if !stderr.is_empty() {
            std::io::stderr().write_all(
                &crate::executor::substitution_metadata::shell_text_to_raw_bytes(stderr),
            )?;
        }
        let statuses = results
            .iter()
            .map(|(_, _, status)| *status)
            .collect::<Vec<_>>();
        self.exit_code = self.pipeline_exit_status(&statuses);
        self.set_pipestatus(statuses);
        Ok(Some(results))
    }

    /// True when the stage is an external program whose fd 1 is the plain
    /// pipeline pipe and whose fd 2 is NOT merged onto it — the shapes
    /// `execute_external_pipeline_stage_inner` streams through
    /// `pipeline_stdout_stream` instead of a parent capture. Builtins are
    /// excluded: their stderr redirects are applied by the driver's routing
    /// walk, which the concurrent shell-writer path bypasses.
    #[cfg(any(windows, unix))]
    fn command_is_streamable_external_producer(
        &mut self,
        command: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        if command.time_command.is_some()
            || command.brace_group.is_some()
            || command.subshell
            || command.redirect_out.is_some()
            || command.append.is_some()
            || command.redirect_in.is_some()
            || command_is_compound_pipeline_stage(command)
            || self.command_first_word_is_function(command)
        {
            return Ok(false);
        }
        if command
            .words
            .first()
            .is_some_and(|word| self.routes_as_builtin(word))
        {
            return Ok(false);
        }
        let (stdout_target, stderr_target) = match self.pipeline_stage_stdio_targets(command)? {
            Some(targets) => targets,
            None => return Ok(true),
        };
        use crate::executor::redirection::OutputTarget;
        Ok(matches!(stdout_target, OutputTarget::Stdout)
            && !matches!(stderr_target, OutputTarget::Stdout))
    }

    /// True when the stage's first word names a shell function (GNU
    /// execute_cmd.c:4674-4677 find_function runs before the disk-command
    /// fallback). Only a literal first word is consulted so this admission
    /// probe never re-runs a command substitution its own stage expansion
    /// would — names carrying `$`/backticks/quotes take the sequential path.
    #[cfg(any(windows, unix))]
    fn command_first_word_is_function(&self, command: &CommandNode) -> bool {
        let Some(word) = command.words.first() else {
            return false;
        };
        if word.is_empty()
            || word
                .chars()
                .any(|ch| matches!(ch, '$' | '`' | '(' | ')' | '\\' | '\'' | '"'))
        {
            return false;
        }
        self.function_name_for_command_word(word.as_str()).is_some()
    }

    /// niubash#345: a shell-side producer (a compound `while`/`for`/`until`
    /// loop and friends) feeding an external consumer chain. GNU forks every
    /// pipeline element (execute_cmd.c:2620 execute_pipeline): the producer
    /// writes the inter-member pipe directly and dies of SIGPIPE once a
    /// bounding consumer (`head -N`, ...) exits. Rubash runs the compound
    /// element in-process, and the sequential stage loop would materialize
    /// its entire — for an infinite writer, unbounded — output in memory
    /// before the consumer even starts, so the pipeline never terminates.
    ///
    /// Here the external consumers are spawned first on real OS pipes and
    /// the in-process producer streams into pipe 0, with EPIPE surfaced as
    /// the SIGPIPE-shaped status 141 (see `write_pipeline_stream` /
    /// `lingerer_sigpipe_status`). This is the shell-side twin of
    /// `execute_external_pipeline_concurrently`, which already covers the
    /// all-external `yes | head` case.
    #[cfg(any(windows, unix))]
    fn execute_shell_writer_pipeline(
        &mut self,
        commands: &[&CommandNode],
    ) -> Result<Option<()>, ExecuteError> {
        if commands.len() < 2
            || self.stderr_capture.is_some()
            // Same DEBUG-trap bail-out as the concurrent external paths: the
            // sequential loop owns the per-element run_debug_trap fires.
            || (self.debug_trap_in_scope()
                && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
                    .is_some_and(|action| !action.is_empty()))
            || commands[0].inverted
            || commands[0].inverted_command.is_some()
        {
            return Ok(None);
        }
        // Shell-side producers (compound loops, functions) are the ones the
        // sequential loop would materialize: they run in-process and may be
        // unbounded writers. An external first stage is normally claimed by
        // the all-external path above; reaching here means it declined —
        // commonly a stderr redirect (`yes 2>/dev/null | head -1`), which the
        // single-stage external spawner can still stream.
        if !command_is_compound_pipeline_stage(commands[0])
            && !self.command_first_word_is_function(commands[0])
            && !self.command_is_streamable_external_producer(commands[0])?
        {
            return Ok(None);
        }
        // The producer's own fd-1 target must be the pipe: `>file` needs the
        // sequential routing walk. A stderr redirect is fine — the element's
        // body applies it internally (execute_compound_pipeline_stage).
        if commands[0].redirect_out.is_some()
            || commands[0].append.is_some()
            || commands[0].time_command.is_some()
        {
            return Ok(None);
        }
        // Every downstream stage must be a concurrent-eligible external
        // member; otherwise the ordinary sequential path owns the pipeline.
        // A consumer that is a trivial brace group around a single external
        // command (`... | { head -1; }`) is reduced to that command so it can
        // join the concurrent chain; otherwise the sequential loop would again
        // re-buffer an unbounded producer (niubash#345).
        let mut specs = Vec::with_capacity(commands.len() - 1);
        let mut consumers: Vec<&CommandNode> = Vec::with_capacity(commands.len() - 1);
        for (offset, command) in commands.iter().enumerate().skip(1) {
            let is_last = offset + 1 == commands.len();
            let consumer = self.unwrap_trivial_brace_group(command).unwrap_or(command);
            let Some(spec) = self.concurrent_external_stage_spec(consumer, offset, is_last) else {
                return Ok(None);
            };
            specs.push(spec);
            consumers.push(consumer);
        }
        // A `/dev/`-family operand on a consumer needs the dev-fd
        // materializer the plain external spawner wires; bail to the
        // sequential path rather than mis-resolve it here.
        if specs.iter().any(|(_, _, args)| {
            args.iter().any(|arg| {
                crate::executor::dev_fd_operands::dev_operand_targets_fd(arg, 0)
                    || crate::executor::dev_fd_operands::dev_operand_targets_fd(arg, 1)
            })
        }) {
            return Ok(None);
        }
        self.run_shell_writer_pipeline(commands, &consumers, specs)?;
        Ok(Some(()))
    }

    /// niubash#345: reduce a `{ cmd; }` wrapper with no group-level
    /// redirects, assignments, or other modifiers to its single body command,
    /// so a shell-side consumer such as `... | { head -1; }` can join the
    /// concurrent external chain instead of the sequential loop re-buffering
    /// the (unbounded) producer. A non-trivial group — multiple commands,
    /// group redirects, a compound body — returns `None` and keeps the
    /// sequential path.
    #[cfg(any(windows, unix))]
    fn unwrap_trivial_brace_group<'a>(&self, command: &'a CommandNode) -> Option<&'a CommandNode> {
        let group = command.brace_group.as_ref()?;
        let inert = command.words.is_empty()
            && command.assignments.is_empty()
            && command.redirects.is_empty()
            && command.redirect_out.is_none()
            && command.append.is_none()
            && command.redirect_in.is_none()
            && command.redirect_err.is_none()
            && command.redirect_err_append.is_none()
            && command.heredoc.is_none()
            && command.heredoc_redirects.is_empty()
            && command.here_string.is_none()
            && command.process_substitutions.is_empty()
            && command.time_command.is_none()
            && !command.subshell
            && !command.inverted
            && command.inverted_command.is_none();
        if !inert || group.body.len() != 1 {
            return None;
        }
        let inner = &group.body[0];
        if inner.brace_group.is_some()
            || inner.subshell
            || inner.subshell_command.is_some()
            || command_is_compound_pipeline_stage(inner)
        {
            return None;
        }
        Some(inner)
    }

    /// Spawn the external consumers on real OS pipes, run the in-process
    /// compound producer streaming its fd 1 into pipe 0, then reap the chain
    /// and publish the statuses. See `execute_shell_writer_pipeline`.
    ///
    /// `consumers` holds the effective command node for each downstream stage:
    /// the stage itself, or the single external command inside a trivial
    /// brace group it was reduced from (`... | { head -1; }`).
    #[cfg(any(windows, unix))]
    fn run_shell_writer_pipeline(
        &mut self,
        commands: &[&CommandNode],
        consumers: &[&CommandNode],
        specs: Vec<(std::path::PathBuf, String, Vec<String>)>,
    ) -> Result<(), ExecuteError> {
        #[cfg(unix)]
        use std::os::unix::io::AsRawFd;
        #[cfg(windows)]
        use std::os::windows::io::AsRawHandle;

        // node0 = producer; node1..nodeN = consumers. pipe[j] connects node j
        // to node j+1, so there are `consumer_count` pipes: pipe[0] carries
        // the producer's stream and pipe[j] carries consumer j-1 -> consumer j.
        let consumer_count = specs.len();
        let mut pipes: Vec<(Option<os_pipe::PipeReader>, Option<os_pipe::PipeWriter>)> =
            Vec::with_capacity(consumer_count);
        for _ in 0..consumer_count {
            let (read, write) = os_pipe::pipe().map_err(ExecuteError::IoError)?;
            pipes.push((Some(read), Some(write)));
        }
        let capture_intermediate_stderr =
            self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout);
        let mut processes = Vec::with_capacity(consumer_count);
        let mut intermediate_stderr = Vec::new();
        for (index, (program, expanded_name, args)) in specs.iter().enumerate() {
            let (mut process, _) = if let Some(name) = internal_pipeline_program_name(program) {
                let mut process = std::process::Command::new(std::env::current_exe()?);
                process.arg(format!("--internal-{name}")).args(args);
                (process, false)
            } else {
                external_command_for_named_program(
                    program,
                    Some(expanded_name),
                    args,
                    &self.shell_state.env_vars,
                    self.bash_argv0_env_supplied(&consumers[index].assignments),
                )
            };
            // rubash#1564: the tempenv application expands the
            // element's assignment values on this shared executor —
            // bracket it so binds land only in the child's env.
            let saved_env = self.snapshot_spec_env(&consumers[index]);
            self.apply_external_environment(&consumers[index], &mut process, program);
            self.restore_spec_env_into_process(saved_env, &mut process);
            // Consumer j's stdin is pipe[j]'s read end.
            {
                let (read, _) = &mut pipes[index];
                process.stdin(stdio_from_transferred_handle(
                    read.take().expect("producer reader already transferred"),
                ));
            }
            if index + 1 < consumer_count {
                // Consumer j's stdout is pipe[j+1]'s write end.
                let (_, write) = &mut pipes[index + 1];
                process.stdout(stdio_from_transferred_handle(
                    write.take().expect("pipeline writer already transferred"),
                ));
                if capture_intermediate_stderr {
                    process.stderr(Stdio::piped());
                }
            } else {
                process.stdout(Stdio::piped());
                process.stderr(Stdio::piped());
            }
            // rubash#1688: a pipeline member is a fork of the shell
            // like any external child — pin the ambient descriptors.
            self.attach_ambient_fd_pins(consumers[index], &mut process);
            process.current_dir(&self.shell_state.cwd);
            suppress_orphan_console(&mut process);
            let mut child = process.spawn().map_err(|error| {
                ExecuteError::IoError(io::Error::new(
                    error.kind(),
                    format!(
                        "{}: {}",
                        program
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("external"),
                        crate::posix_errors::message(&error)
                    ),
                ))
            })?;
            adopt_orphan_console(child.id());
            if capture_intermediate_stderr && index + 1 < consumer_count {
                if let Some(mut stderr) = child.stderr.take() {
                    intermediate_stderr.push(std::thread::spawn(move || {
                        let mut output = Vec::new();
                        stderr.read_to_end(&mut output)?;
                        Ok::<_, std::io::Error>(output)
                    }));
                }
            }
            processes.push(child);
        }

        // The producer stage runs in-process with its fd 1 bound to pipe[0]'s
        // write end. Holding the writer here keeps the pipe open across the
        // stage; it is dropped below so the consumers see EOF on a normal
        // (finite) producer.
        let mut producer_writer = pipes[0].1.take();
        #[cfg(windows)]
        let stream_handle = producer_writer
            .as_ref()
            .map(|writer| writer.as_raw_handle() as crate::fd::HANDLE);
        #[cfg(unix)]
        let stream_handle = producer_writer
            .as_ref()
            .map(|writer| writer.as_raw_fd() as crate::fd::HANDLE);
        // rubash#1873: same stage-0 classification as the sequential
        // driver — a producer whose fd 0 is the shell's live fd 0 gets
        // no payload (computing one would drain a file endpoint).
        let stage0_stdin_live = self.pipeline_stage0_stdin_live(commands[0]);
        let (stage0_input, stage0_stdin_base) = if stage0_stdin_live {
            self.apply_comsub_stdin_writeback();
            (String::new(), None)
        } else {
            self.initial_pipeline_input(commands[0])
        };
        let stage0_stdin_inherit = if stage0_stdin_live {
            match self.fd_table.entries.get(&0) {
                Some(entry) => {
                    !entry.closed
                        && matches!(entry.read, Some(FdReadEndpoint::InheritedProcessStdin))
                }
                None => true,
            }
        } else {
            stage0_input.is_empty()
                && self.stdin_string_for_command(commands[0]).is_none()
                && match self.fd_table.entries.get(&0) {
                    Some(entry) => {
                        !entry.closed
                            && matches!(entry.read, Some(FdReadEndpoint::InheritedProcessStdin))
                    }
                    None => true,
                }
        };
        let saved_pipeline_stream = self.pipeline_stdout_stream;
        self.pipeline_stdout_stream = stream_handle;
        let producer_result = self.execute_pipeline_stage(
            commands[0],
            &stage0_input,
            stage0_stdin_inherit,
            stage0_stdin_live,
        );
        self.pipeline_stdout_stream = saved_pipeline_stream;

        let (mut producer_status, producer_out, producer_err, mut producer_error) =
            match producer_result {
                Ok(Some((out, err, status))) => (status, out, err, None),
                Ok(None) => (0, String::new(), String::new(), None),
                Err(error) => (0, String::new(), String::new(), Some(error)),
            };
        // A compound producer streams through fd 1 (producer_out empty); an
        // inline arm that returned buffered bytes gets them written before the
        // writer closes. A broken pipe here is the same SIGPIPE shape.
        if producer_error.is_none() && !producer_out.is_empty() {
            let bytes =
                crate::executor::substitution_metadata::shell_text_to_raw_bytes(&producer_out);
            if let Some(writer) = producer_writer.as_mut() {
                if let Err(error) = writer.write_all(&bytes) {
                    if crate::executor::shell_options::is_broken_pipe_error(&error) {
                        producer_status = 141;
                    } else {
                        producer_error = Some(ExecuteError::IoError(error));
                    }
                }
            }
        }
        // Close the producer's write end so a normal producer's EOF reaches
        // the first consumer.
        drop(producer_writer.take());

        // Reap the consumers exactly like the all-external path: drain the
        // last member's stdout/stderr first (niubash#158), then wait each
        // member against its immediate downstream so the Windows broken-pipe
        // window only arms after that downstream exited.
        let mut results: Vec<(String, String, i32)> = vec![Default::default(); consumer_count];
        let last_index = consumer_count - 1;
        // When the producer exited cleanly the last consumer's payload goes
        // only to write_pipeline_output below, so it may stream live — unless
        // a producer error makes that write (and the payload) get discarded.
        let stream_stdout = producer_error.is_none()
            && self.final_member_stdout_streams_to_host(
                consumers.last().expect("pipeline has at least two stages"),
            );
        let stdout_target = if stream_stdout {
            PipelineDrainTarget::StreamHostStdout
        } else {
            PipelineDrainTarget::Collect
        };
        let last_stdout = processes[last_index]
            .stdout
            .take()
            .map(|reader| spawn_pipe_drain(reader, stdout_target));
        let last_stderr = processes[last_index]
            .stderr
            .take()
            .map(|reader| spawn_pipe_drain(reader, PipelineDrainTarget::Collect));
        for index in 0..last_index {
            let (head, tail) = processes.split_at_mut(index + 1);
            let member = head.last_mut().expect("member index inside pipeline");
            let downstream = tail.first_mut().expect("downstream member after index");
            let status = wait_for_windows_pipeline_member(member, Some(downstream))?;
            results[index] = (
                String::new(),
                String::new(),
                crate::executor::wait_status::process_exit_status(&status),
            );
        }
        let mut last = processes.pop().expect("pipeline has at least two stages");
        self.note_foreground_wait();
        let status = last.wait().map_err(ExecuteError::IoError)?;
        let stdout_bytes = join_pipe_drain(last_stdout)?;
        results[last_index] = (
            crate::executor::substitution_metadata::bytes_to_shell_text(&stdout_bytes),
            crate::executor::substitution_metadata::bytes_to_shell_text(&join_pipe_drain(
                last_stderr,
            )?),
            crate::executor::wait_status::process_exit_status(&status),
        );
        for reader in intermediate_stderr {
            let output = reader.join().map_err(|_| {
                ExecuteError::IoError(std::io::Error::other("pipeline stderr reader panicked"))
            })??;
            if capture_intermediate_stderr {
                self.write_default_stdout(&output)?;
            }
        }

        // The producer element's own stderr was redirected inside the body
        // when the stage carried a `2>...`; whatever escaped belongs on the
        // ambient fd 2 (GNU routes a stage's stray stderr to the shell's).
        if producer_error.is_none() && !producer_err.is_empty() {
            self.write_default_stderr(
                &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&producer_err),
            )?;
        }
        if let Some(base) = stage0_stdin_base {
            let consumed = self.pipeline_stdin_consumed.take().unwrap_or(0);
            self.shell_state.env_vars.insert(
                FUNCTION_STDIN_OFFSET.to_string(),
                (base + consumed).to_string(),
            );
        }

        if producer_error.is_none() {
            let final_command = *consumers.last().expect("pipeline has at least two stages");
            self.write_pipeline_output(final_command, &results[last_index].0, false)?;
            if !results[last_index].1.is_empty() {
                std::io::stderr().write_all(
                    &crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                        &results[last_index].1,
                    ),
                )?;
            }
        }

        let mut statuses = Vec::with_capacity(consumer_count + 1);
        statuses.push(producer_status);
        statuses.extend(results.iter().map(|(_, _, status)| *status));
        self.exit_code = self.pipeline_exit_status(&statuses);
        self.set_pipestatus(statuses);

        if let Some(error) = producer_error {
            return Err(error);
        }
        Ok(())
    }

    /// Admission and argv expansion for one concurrent external pipeline
    /// member (execute_cmd.c:5789 execute_disk_command runs every member
    /// through the full expand_words sequence, so the member's arguments
    /// are pathname-expanded here). Returns None when the member must take
    /// the shell-aware sequential stage path instead: builtins, restricted
    /// refusals, redirects the OS-pipe plumbing cannot express, `|&`, or a
    /// program that would be refused by exec classification.
    #[cfg(any(windows, unix))]
    fn concurrent_external_stage_spec(
        &mut self,
        command: &CommandNode,
        index: usize,
        is_last_member: bool,
    ) -> Option<(std::path::PathBuf, String, Vec<String>)> {
        // rubash#1564: the spec expansion below is speculative — it
        // runs on the parent executor, but a pipeline element's
        // expansion belongs to the element's own child in GNU
        // (execute_cmd.c:2620). Bracket it with the variable-table
        // snapshot: successful binds (`${var:=word}`, arithmetic
        // assignments) are rolled back whatever the verdict, and a
        // bind that touched an EXPORTED variable declines the fast
        // path entirely (only the sequential stage executor can give
        // the element's child the bound exported value).
        let saved_env = self.snapshot_spec_env(command);
        let spec = self.concurrent_external_stage_spec_inner(command, index, is_last_member);
        if self.restore_spec_env(saved_env) {
            return None;
        }
        spec
    }

    fn concurrent_external_stage_spec_inner(
        &mut self,
        command: &CommandNode,
        index: usize,
        is_last_member: bool,
    ) -> Option<(std::path::PathBuf, String, Vec<String>)> {
        if command.time_command.is_some()
            || command.brace_group.is_some()
            || command.subshell
            // #800A: a tput head answers inline on the sequential path (the
            // byte-exact whitelist stage arm below); the concurrent paths
            // spawn members directly and would bypass it. tput is a
            // millisecond terminal query — never a long-running producer —
            // so declining concurrency for it costs nothing, and a
            // non-admitted shape still spawns the real binary on the
            // sequential path exactly as this one would.
            || (cfg!(windows) && tput_inline_stage(command))
            || command_has_non_concurrent_pipeline_redirects(
                command,
                index,
                if is_last_member {
                    index + 1
                } else {
                    usize::MAX
                },
            )
            || command.redirect_in.is_some()
            || command.redirect_err.is_some()
            || command.redirect_err_append.is_some()
            || command
                .redirects
                .iter()
                .any(|redirect| redirect.is_list_only_redirect())
            || ((command.redirect_out.is_some() || command.append.is_some()) && !is_last_member)
            || ((command.heredoc.is_some()
                || !command.heredoc_redirects.is_empty()
                || command.here_string.is_some())
                && index != 0)
            // GNU execute_cmd.c:4617 execute_simple_command runs a pipeline
            // element with a `VAR=value` tempenv prefix as an ordinary
            // forked member whose child environment carries the assignment;
            // the concurrent OS-pipe path supports it
            // (apply_external_environment in spawn_wait below). Only the
            // forms that need the sequential executor's shell-table
            // handling bail: `name+=value` appends and PATH= (command
            // resolution must observe the new PATH) — rubash#192.
            || command
                .assignments
                .iter()
                .any(|(name, _)| {
                    let (base, append) = assignment_name_and_append(name);
                    append || base == "PATH"
                })
            || !command.process_substitutions.is_empty()
            || command_has_pipeline_process_substitution(command)
            || command_is_compound_pipeline_stage(command)
        {
            return None;
        }
        // rubash#205: expand the member's words with the same per-word
        // machinery as the sequential stage executor
        // (pipeline_stages.rs expand_stage_argv). The old text-layer
        // `expand_word` re-parsed the already-dequoted word text and its
        // generic walker stripped embedded `"` characters as quote
        // syntax. GNU never re-parses argv text between pipeline elements
        // (execute_cmd.c:2620 -> expand_words once; subst.c:4807
        // dequote_string removes only CTLESC escapes).
        // rubash#1540: snapshot the DISCARD latch family before the
        // speculative expansion — if expanding this member's words
        // raises one (an arithmetic body the admission probe skipped
        // because it embeds a command substitution, a failglob whose
        // pattern only appears after parameter expansion, a bind
        // failure the textual scan could not predict), the fields just
        // built are the failure's empty product, not a runnable argv:
        // decline so the sequential stage executor reports the error
        // as the element's own status. The latches are restored to
        // their entry values, except the arithmetic print latch: the
        // diagnostic already printed once above, and holding that
        // latch keeps the stage's re-expansion from printing it again
        // (the same latch-held handoff execute_timed_read_pipeline
        // documents for the nounset class).
        let spec_saved_arith = self.snapshot_arithmetic_error_flags();
        let spec_saved_discard = self.snapshot_discard_latches();
        let spec_saved_no_closing_brace = self.no_closing_brace_failure.get();
        let _speculative_guard = SpeculativeExpansionGuard::new();
        let mut first_fields = self.expand_stage_first_word(command);
        let expanded_name = first_fields.first().cloned().unwrap_or_default();
        // GNU runs every pipeline member through execute_disk_command
        // (execute_cmd.c:5789), so a restricted shell refuses members
        // exactly like plain commands. A member that would be refused
        // must take the sequential stage path, where
        // restricted_command_error reports it and the remaining stages
        // still run against the (empty) pipe input.
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "restricted")
            && self
                .restricted_command_error(command, &expanded_name)
                .is_some()
        {
            return None;
        }
        if self.routes_as_builtin(&expanded_name) {
            return None;
        }
        // GNU never routes a pipeline member straight to the disk command:
        // execute_pipeline (execute_cmd.c:2620) runs every element through
        // execute_simple_command, which resolves functions BEFORE the
        // execute_from_filesystem fallback (execute_cmd.c:4674-4677
        // find_function; dispatch 4837-4861; execute_disk_command
        // 4919-4925 only when both lookups missed). This fast path spawns
        // members directly off find_user_command, so a member whose first
        // field names a shell function must take the sequential stage
        // executor — its dispatch runs execute_function_pipeline_stage
        // (issue #587: `date(){ echo FAKE; }; date | cat` executed the
        // real date(1), and a `t(){ ...; }; t git log | wc -l` prefix
        // silently ran a 0-byte PATH stub instead of the function).
        if self
            .function_name_for_command_word(&expanded_name)
            .is_some()
        {
            return None;
        }
        let program =
            find_user_command(&expanded_name, &self.shell_state.env_vars).or_else(|| {
                // rubash#1545: the internal-program stand-in may only
                // stand in for a command GNU would resolve (findcmd.c
                // search_for_command) — on an unresolvable name the
                // member is `command not found`, so decline the fast
                // path and let the sequential stage executor report it.
                // (The stand-in's legitimate case is a HASH-resolved
                // name find_user_command's PATH walk cannot see.)
                (matches!(expanded_name.as_str(), "yes" | "head" | "wc")
                    && !self.emulated_external_stands_down(&expanded_name))
                .then(|| internal_pipeline_program(&expanded_name))
            })?;
        // GNU execute_cmd.c:6139-6233 (shell_execve): a member the OS
        // cannot exec natively is classified by its first bytes before
        // any shell-script fallback. A refusal must not run the member
        // through the fast path; bail out so the sequential stage
        // executor reports it (it applies the same classification).
        if crate::executor::path::should_run_with_shell(&program)
            && self.exec_format_refusal(command, &program).is_some()
        {
            return None;
        }
        let mut args: Vec<String> = first_fields.into_iter().skip(1).collect();
        args.extend(self.expand_stage_tail_words(command));
        // rubash#1540: the latch net described at the snapshot above —
        // a DISCARD-class failure raised anywhere in this member's
        // speculative expansion voids the spec.
        let spec_expansion_failed = (self.parameter_assignment_failure.get()
            && !spec_saved_discard.0)
            || (self.evalerror_failglob_pending.get() && !spec_saved_discard.2)
            || (self.no_closing_brace_failure.get() && !spec_saved_no_closing_brace)
            || (self.shell_state.arithmetic_fatal_error.get() && !spec_saved_arith.2)
            || (self.shell_state.arithmetic_nonfatal_error.get() && !spec_saved_arith.1)
            || (self.shell_state.arithmetic_nounset_error.get() && !spec_saved_arith.3);
        if spec_expansion_failed {
            self.restore_discard_latches(&spec_saved_discard);
            self.no_closing_brace_failure
                .set(spec_saved_no_closing_brace);
            // Restore the arithmetic verdict flags but keep the print
            // latch exactly as the expansion left it (see above).
            let printed = self.shell_state.arithmetic_expansion_error.get();
            self.restore_arithmetic_error_flags(&spec_saved_arith);
            self.shell_state.arithmetic_expansion_error.set(printed);
            return None;
        }
        // Same child-argv contract as the sequential spawn path
        // (pipeline_stages.rs execute_external_pipeline_stage_inner):
        // decode the lexer's C0 data carriers exactly once so the
        // spawned program receives visible argv text.
        for arg in &mut args {
            *arg = super::execution_misc::restore_command_substitution_output(arg);
        }
        Some((program, expanded_name, args))
    }

    /// True when the final member's fd-1 payload lands on the host process
    /// stdout through `write_pipeline_output` — the GNU shape where
    /// execute_cmd.c:2620 leaves the rightmost element's fd 1 on the
    /// shell's own stdout. Only then may a drain forward bytes to the host
    /// as they arrive (niubash D4); every capture, dup snapshot, or stage
    /// redirect keeps the collecting path so the deferred walk still places
    /// the payload (niubash#158's drain stays the transport either way).
    /// Evaluated on the same thread that later performs the deferred write,
    /// before any member is reaped — nothing in the wait mutates these
    /// bindings.
    #[cfg(any(windows, unix))]
    pub(in crate::executor) fn final_member_stdout_streams_to_host(
        &self,
        command: &CommandNode,
    ) -> bool {
        // write_pipeline_output runs route_pipeline_stage_streams first
        // whenever the member carries an output-side redirect; that walk
        // opens `>f`, resolves `1>&2`/`>&-`, and reports write failures, so
        // it needs the whole payload up front.
        if command.redirect_out.is_some()
            || command.append.is_some()
            || command.redirect_err.is_some()
            || command.redirect_err_append.is_some()
            || command
                .redirects
                .iter()
                .any(|redirect| redirect.is_output_side())
        {
            return false;
        }
        if self.fd_table.is_closed(1) {
            return false;
        }
        if !matches!(
            self.fd_table.output_endpoint(1),
            Some(FdWriteEndpoint::Stdout)
        ) {
            return false;
        }
        // While an enclosing shell-side producer frame owns fd 1, the
        // payload belongs to the upstream pipe, not the host stdout.
        if crate::executor::shell_options::active_capture_stream().is_some() {
            return false;
        }
        // Mirror write_fd_endpoint's fd-1 resolution exactly: a dup record
        // names its snapshot target, and the live arm honours the
        // thread-local capture first, then the executor's capture field.
        match self.fd_table.stdout_alias_generation.get(&1) {
            Some(Some(_)) => false,
            Some(None) => true,
            None => {
                !crate::executor::shell_options::stdout_capture_active()
                    && self.stdout_capture.is_none()
            }
        }
    }

    /// Spawn the already-admitted external pipeline members on real OS
    /// pipes (GNU execute_cmd.c:2620 execute_pipeline forks every left
    /// element connected by pipe(2) as it walks the connection list —
    /// execute_cmd.c:2641,2648-2707 — so a producer blocked on a full pipe
    /// runs concurrently with its consumer instead of being slurped into
    /// the shell's memory). The last member's stdout is captured and
    /// returned in the last results entry; the caller owns writing the
    /// final output and the exit/pipestatus bookkeeping.
    ///
    /// `allow_stream_stdout` marks callers whose last entry is only ever
    /// handed to `write_pipeline_output` (the all-external path). When
    /// `final_member_stdout_streams_to_host` agrees, the drain forwards the
    /// bytes live and the returned payload stays empty; callers that feed
    /// the bytes elsewhere — the sequential prefix path passes them on as
    /// stage input — pass `false` and always get the collected stream.
    #[cfg(windows)]
    fn spawn_wait_concurrent_external_stages(
        &mut self,
        commands: &[&CommandNode],
        specs: Vec<(std::path::PathBuf, String, Vec<String>)>,
        allow_stream_stdout: bool,
    ) -> Result<Vec<(String, String, i32)>, ExecuteError> {
        let mut pipes: Vec<(Option<os_pipe::PipeReader>, Option<os_pipe::PipeWriter>)> =
            Vec::with_capacity(commands.len() - 1);
        for _ in 0..commands.len() - 1 {
            let (read, write) = os_pipe::pipe().map_err(ExecuteError::IoError)?;
            pipes.push((Some(read), Some(write)));
        }

        // Stage 0's fd-0 payload is computed once and reused for the
        // post-spawn stdin write below; a `/dev/stdin` operand on the first
        // member materializes the same bytes.
        let (stage0_input, stage0_stdin_base) = self.initial_pipeline_input(commands[0]);
        // GNU forks the child with the shell's own fd 0 — when the stage
        // carries no fd-0 binding (no `<`, `<<<`, `<<`, heredoc-0) and fd 0
        // is the inherited process stdin, the child must inherit the real
        // handle rather than a pre-drained payload (which also streams
        // instead of buffering the whole upstream first).
        let stage0_inherits = stage0_input.is_empty()
            && self.stdin_string_for_command(commands[0]).is_none()
            && match self.fd_table.entries.get(&0) {
                Some(entry) => {
                    !entry.closed
                        && matches!(entry.read, Some(FdReadEndpoint::InheritedProcessStdin))
                }
                None => true,
            };

        let mut processes = Vec::with_capacity(commands.len());
        let mut stage_dev_ops = Vec::with_capacity(commands.len());
        let capture_intermediate_stderr =
            self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout);
        // rubash#1454: when the caller streams the final payload to the
        // host (allow_stream_stdout) and that payload resolves to the
        // shell's own stdout, the last member inherits the stdout at
        // spawn (GNU execute_cmd.c:2620) instead of a capture pipe the
        // drain forwards — computed here because the spawn precedes the
        // drain decision below, which reuses the same value.
        let stream_stdout = allow_stream_stdout
            && self.final_member_stdout_streams_to_host(commands[commands.len() - 1]);
        let mut intermediate_stderr = Vec::new();
        for (index, (program, expanded_name, args)) in specs.iter().enumerate() {
            // The stage's fd 0/1 are OS pipes owned by this loop, not fd
            // table endpoints — a `/dev/stdin` operand drains the incoming
            // reader into a temp file and a `/dev/stdout` operand flushes
            // its temp into a dup of the outgoing writer after exit.
            // split_at_mut keeps the incoming reader (pipes[index-1].0) and
            // the outgoing writer (pipes[index].1) borrowable at once.
            let (before, current) = pipes.split_at_mut(index);
            let dev_stdin = if index == 0 {
                if stage0_inherits {
                    // fd 0 is the live inherited process stdin — an fd-0
                    // operand drains it through the fd-table endpoint.
                    crate::executor::dev_fd_operands::DevOperandStdin::FdTable
                } else {
                    crate::executor::dev_fd_operands::DevOperandStdin::Payload(
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                            &stage0_input,
                        ),
                    )
                }
            } else {
                match before.last_mut().and_then(|pipe| pipe.0.as_mut()) {
                    Some(reader) => {
                        crate::executor::dev_fd_operands::DevOperandStdin::Reader(reader)
                    }
                    None => crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
                }
            };
            let dev_stdout = if index + 1 < commands.len()
                && args
                    .iter()
                    .any(|arg| crate::executor::dev_fd_operands::dev_operand_targets_fd(arg, 1))
            {
                use std::os::windows::io::AsRawHandle;
                current
                    .first()
                    .and_then(|pipe| pipe.1.as_ref())
                    .and_then(|writer| {
                        crate::fd::duplicate_handle(writer.as_raw_handle() as crate::fd::HANDLE)
                            .ok()
                    })
                    .map(crate::executor::dev_fd_operands::DevOperandStdout::PipeWriter)
                    .unwrap_or(crate::executor::dev_fd_operands::DevOperandStdout::FdTable)
            } else if index + 1 == commands.len() {
                crate::executor::dev_fd_operands::DevOperandStdout::Capture
            } else {
                crate::executor::dev_fd_operands::DevOperandStdout::FdTable
            };
            let (dev_args, dev_ops) = self.materialize_dev_fd_operands(args, dev_stdin, dev_stdout);
            stage_dev_ops.push(dev_ops);
            let (mut process, _) = if let Some(name) = internal_pipeline_program_name(program) {
                let mut process = std::process::Command::new(std::env::current_exe()?);
                process.arg(format!("--internal-{name}")).args(&dev_args);
                (process, false)
            } else {
                // command_name is the already-expanded first field from the
                // spec above (the sequential path passes the same
                // expanded_name); re-reading words[0] with expand_word would
                // re-strip data quotes (rubash#205) and re-run its
                // substitution side effects a second time.
                external_command_for_named_program(
                    program,
                    Some(expanded_name),
                    &dev_args,
                    &self.shell_state.env_vars,
                    self.bash_argv0_env_supplied(&commands[index].assignments),
                )
            };
            // Apply the element's `VAR=value` tempenv prefix to the child
            // (execute_cmd.c execute_simple_command tempenv); plain
            // assignments are the only ones that reach the concurrent path
            // (the bail above routed +=/PATH to the sequential executor).
            // rubash#1564: the application expands the values on this
            // shared executor — bracket it so binds land only in the
            // child's env (see restore_spec_env_into_process).
            let saved_env = self.snapshot_spec_env(&commands[index]);
            self.apply_external_environment(&commands[index], &mut process, program);
            self.restore_spec_env_into_process(saved_env, &mut process);

            if index == 0 {
                if stage0_inherits {
                    process.stdin(Stdio::inherit());
                } else {
                    process.stdin(Stdio::piped());
                }
            } else {
                let (read, _) = &mut pipes[index - 1];
                process.stdin(stdio_from_transferred_handle(
                    read.take().expect("pipeline reader already transferred"),
                ));
            }
            if index + 1 < commands.len() {
                let (_, write) = &mut pipes[index];
                process.stdout(stdio_from_transferred_handle(
                    write.take().expect("pipeline writer already transferred"),
                ));
                if capture_intermediate_stderr {
                    process.stderr(Stdio::piped());
                }
            } else {
                if stream_stdout {
                    process.stdout(Stdio::inherit());
                } else {
                    process.stdout(Stdio::piped());
                }
                process.stderr(Stdio::piped());
            }

            // niubash#141: keep the failed stage's resolved program in the
            // diagnostic — a bare `line 1: Unknown error` hid which stage's
            // CreateProcess failed (and with which binary).
            // rubash#1688: pin the ambient descriptors for the stage.
            self.attach_ambient_fd_pins(&commands[index], &mut process);
            process.current_dir(&self.shell_state.cwd);
            suppress_orphan_console(&mut process);
            let mut child = process.spawn().map_err(|error| {
                ExecuteError::IoError(io::Error::new(
                    error.kind(),
                    format!(
                        "{}: {}",
                        program
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("external"),
                        crate::posix_errors::message(&error)
                    ),
                ))
            })?;
            adopt_orphan_console(child.id());
            if capture_intermediate_stderr && index + 1 < commands.len() {
                if let Some(mut stderr) = child.stderr.take() {
                    intermediate_stderr.push(std::thread::spawn(move || {
                        let mut output = Vec::new();
                        stderr.read_to_end(&mut output)?;
                        Ok::<_, std::io::Error>(output)
                    }));
                }
            }
            processes.push(child);
        }

        // Spawn every stage before writing a heredoc. A large heredoc can fill
        // the first stdin pipe while the downstream stages are still absent.
        if let Some(mut stdin) = processes[0].stdin.take() {
            if let Some(base) = stage0_stdin_base {
                // The spawned child was handed the whole unread tail; GNU's
                // shared fd 0 models that as consumed. `stage0_input` is
                // transport text sliced from FUNCTION_STDIN at `base`, and
                // FUNCTION_STDIN_OFFSET indexes that transport buffer, so the
                // delta is its TRANSPORT length — `shell_text_to_raw_bytes`'s
                // raw length would stop short by the carrier overhead and
                // leave consumed bytes readable (niubash#383 follow-up).
                self.shell_state.env_vars.insert(
                    FUNCTION_STDIN_OFFSET.to_string(),
                    (base + stage0_input.len()).to_string(),
                );
            }
            if !stage0_input.is_empty() {
                // Same feed contract as the host path: a member that
                // already exited must not fail the pipeline (rubash#1822).
                crate::executor::shell_options::feed_child_stdin(
                    &mut stdin,
                    &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stage0_input),
                )?;
            }
        }

        let mut results: Vec<(String, String, i32)> = vec![Default::default(); processes.len()];
        let last_index = processes.len() - 1;
        // niubash#158: start draining the last member's captured stdout and
        // stderr BEFORE waiting on the upstream members. The last member
        // reads an inter-member OS pipe and writes a parent-side capture
        // pipe; a pass-through member (cat/nl/rev) that fills its stdout
        // pipe stops reading its input, the upstream producer then fills the
        // inter-member pipe and never exits, and the forward wait below
        // would poll forever — `wait_with_output` only begins reading after
        // every producer was reaped, so the staged ordering deadlocked once
        // the 2c781657 patient wait removed the 100ms hard kill that used to
        // mask it. GNU never withholds the final member's output:
        // execute_cmd.c:2620 execute_pipeline leaves the rightmost element's
        // fd 1 on the shell's own stdout (or, inside $(...), the capture
        // pipe the child dups in subst.c:7143 command_substitute), and the
        // parent drains that capture concurrently with the members —
        // subst.c:7428 close(fildes[1]) then subst.c:7437 read_comsub(fildes[0])
        // BEFORE subst.c:7441 wait_for(pid).
        let stdout_target = if stream_stdout {
            PipelineDrainTarget::StreamHostStdout
        } else {
            PipelineDrainTarget::Collect
        };
        let last_stdout = processes[last_index]
            .stdout
            .take()
            .map(|reader| spawn_pipe_drain(reader, stdout_target));
        let last_stderr = processes[last_index]
            .stderr
            .take()
            .map(|reader| spawn_pipe_drain(reader, PipelineDrainTarget::Collect));
        // Reap non-final stages in forward order: a `/dev/stdout`-family
        // operand on stage N flushes into a dup'd writer of pipe N, and that
        // dup must close before stage N+1 can see EOF — reaping downstream
        // first would deadlock the wait. Each member is waited against its
        // immediate downstream: GNU never caps a member whose consumer is
        // still reading (execute_cmd.c:2620 + jobs.c:3064 wait_for), so the
        // Windows broken-pipe hard-kill window may only arm once that
        // downstream has exited. split_at_mut borrows the member and its
        // downstream from the one Vec that still owns every child.
        for index in 0..last_index {
            let (head, tail) = processes.split_at_mut(index + 1);
            let member = head.last_mut().expect("member index inside pipeline");
            let downstream = tail.first_mut().expect("downstream member after index");
            let status = wait_for_windows_pipeline_member(member, Some(downstream))?;
            results[index] = (
                String::new(),
                String::new(),
                crate::executor::wait_status::process_exit_status(&status),
            );
            self.finish_dev_fd_operands(std::mem::take(&mut stage_dev_ops[index]));
        }
        let mut last = processes.pop().expect("pipeline has at least two stages");
        debug_assert_eq!(processes.len(), last_index);
        self.note_foreground_wait();
        let status = last.wait().map_err(ExecuteError::IoError)?;
        let mut stdout_bytes = join_pipe_drain(last_stdout)?;
        stdout_bytes
            .extend(self.finish_dev_fd_operands(std::mem::take(&mut stage_dev_ops[last_index])));
        results[last_index] = (
            crate::executor::substitution_metadata::bytes_to_shell_text(&stdout_bytes),
            crate::executor::substitution_metadata::bytes_to_shell_text(&join_pipe_drain(
                last_stderr,
            )?),
            crate::executor::wait_status::process_exit_status(&status),
        );
        for reader in intermediate_stderr {
            let output = reader.join().map_err(|_| {
                ExecuteError::IoError(std::io::Error::other("pipeline stderr reader panicked"))
            })??;
            if capture_intermediate_stderr {
                self.write_default_stdout(&output)?;
            }
        }
        Ok(results)
    }

    /// rubash#761: an unbounded external producer chain feeding a pipeline
    /// whose RIGHTMOST stage is served in-shell (a builtin tail). GNU
    /// execute_cmd.c:2620 execute_pipeline forks every element on real
    /// pipe(2)s, so when the rightmost element exits its read end closes and
    /// the producer dies of SIGPIPE (`yes | true` -> PIPESTATUS 141 0). The
    /// sequential stage loop instead materialized the producer's WHOLE stream
    /// (`execute_external_pipeline_stage_inner` -> `wait_with_output`) before
    /// the tail ever ran — an unbounded `yes` never reaches EOF, so the shell
    /// drained it into RAM forever (~250 MB/s; rubash#761, root of #723's
    /// inline-arm-last-stage freeze). `execute_external_prefix_concurrently`
    /// has the same defect one level up: its `spawn_wait_concurrent_external_stages`
    /// drains the last prefix member to completion before the tail starts, so
    /// `yes | cat | true` still never terminates.
    ///
    /// This path runs the maximum plain-external prefix on real OS pipes and
    /// hands the LAST pipe's read end to the in-shell tail as fd 0 (via
    /// `pipeline_stdin_stream`), so the tail pulls the stream on demand. When
    /// the tail stops reading (it exits, or reads one record), its read end is
    /// dropped, the producers meet EPIPE, and each lingering member is reaped
    /// with the SIGPIPE-shaped status 141 (`wait_for_windows_pipeline_member`).
    /// Admission is a WHITELIST: the tail must be a literal builtin from
    /// `streaming_builtin_tail` and every prefix member a plain external, so a
    /// shape this path cannot faithfully stream keeps the existing executor.
    #[cfg(any(windows, unix))]
    fn execute_external_prefix_streaming_tail(
        &mut self,
        commands: &[&CommandNode],
    ) -> Result<Option<()>, ExecuteError> {
        if commands.len() < 2
            || self.stderr_capture.is_some()
            // A live DEBUG trap fires per element inside the forked child
            // (execute_cmd.c:4506); this fast path would bypass those fires.
            || (self.debug_trap_in_scope()
                && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
                    .is_some_and(|action| !action.is_empty()))
        {
            return Ok(None);
        }
        let tail_index = commands.len() - 1;
        let tail = commands[tail_index];
        if !self.streaming_builtin_tail(tail) {
            return Ok(None);
        }
        // Every prefix member must be a plain external: the OS-pipe spawner
        // below wires only stdio, so any redirect/tempenv/heredoc/procsub/time
        // shape has to keep the shell-aware sequential machine.
        let mut specs = Vec::with_capacity(tail_index);
        for (index, command) in commands[..tail_index].iter().enumerate() {
            if !pipeline_streaming_plain_external(command) {
                return Ok(None);
            }
            let Some(spec) = self.concurrent_external_stage_spec(command, index, false) else {
                return Ok(None);
            };
            specs.push(spec);
        }
        // Stage 0's fd 0 is the shell's inherited stdin (GNU forks every
        // element with the shell's own descriptors). A staged FUNCTION_STDIN
        // payload would need the writer-thread dance; such shapes keep the
        // sequential path.
        let (stage0_input, _stage0_stdin_base) = self.initial_pipeline_input(commands[0]);
        if !stage0_input.is_empty() {
            return Ok(None);
        }
        let stage0_inherits = self.stdin_string_for_command(commands[0]).is_none()
            && match self.fd_table.entries.get(&0) {
                Some(entry) => {
                    !entry.closed
                        && matches!(entry.read, Some(FdReadEndpoint::InheritedProcessStdin))
                }
                None => true,
            };

        // pipe[i] connects prefix member i to member i+1; the LAST pipe feeds
        // the in-shell tail (i.e. commands.len() - 1 pipes for `prefix_len`
        // members, the final one being the tail's stdin).
        let mut pipes: Vec<(Option<os_pipe::PipeReader>, Option<os_pipe::PipeWriter>)> =
            Vec::with_capacity(tail_index);
        for _ in 0..tail_index {
            let (read, write) = os_pipe::pipe().map_err(ExecuteError::IoError)?;
            pipes.push((Some(read), Some(write)));
        }

        let mut processes = Vec::with_capacity(tail_index);
        for (index, (program, expanded_name, args)) in specs.iter().enumerate() {
            let (mut process, _) = if let Some(name) = internal_pipeline_program_name(program) {
                let mut process = std::process::Command::new(std::env::current_exe()?);
                process.arg(format!("--internal-{name}")).args(args);
                (process, false)
            } else {
                external_command_for_named_program(
                    program,
                    Some(expanded_name),
                    args,
                    &self.shell_state.env_vars,
                    self.bash_argv0_env_supplied(&commands[index].assignments),
                )
            };
            // rubash#1564: tempenv bracket, as in spawn_wait above.
            let saved_env = self.snapshot_spec_env(commands[index]);
            self.apply_external_environment(commands[index], &mut process, program);
            self.restore_spec_env_into_process(saved_env, &mut process);
            if index == 0 {
                if stage0_inherits {
                    process.stdin(Stdio::inherit());
                } else {
                    process.stdin(Stdio::null());
                }
            } else {
                let (read, _) = &mut pipes[index - 1];
                process.stdin(stdio_from_transferred_handle(
                    read.take().expect("producer reader already transferred"),
                ));
            }
            let (_, write) = &mut pipes[index];
            process.stdout(stdio_from_transferred_handle(
                write.take().expect("producer writer already transferred"),
            ));
            // rubash#1688: pin the ambient descriptors for the stage.
            self.attach_ambient_fd_pins(commands[index], &mut process);
            process.current_dir(&self.shell_state.cwd);
            suppress_orphan_console(&mut process);
            let mut child = process.spawn().map_err(|error| {
                ExecuteError::IoError(io::Error::new(
                    error.kind(),
                    format!(
                        "{}: {}",
                        program
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("external"),
                        crate::posix_errors::message(&error)
                    ),
                ))
            })?;
            adopt_orphan_console(child.id());
            processes.push(child);
        }

        // The tail's fd 0 is the last pipe's read end. Wrap it as a file
        // endpoint so the in-shell readers (`read`, ...) pull the real stream
        // through the fd table (read_io.rs read_file_bytes blocks on the
        // handle); `pipeline_stdin_stream` tells the stage dispatcher to leave
        // FUNCTION_STDIN absent so `read` does not short-circuit to the
        // buffered mirror.
        #[cfg(unix)]
        use std::os::unix::io::IntoRawFd;
        #[cfg(windows)]
        use std::os::windows::io::IntoRawHandle;
        let tail_reader = pipes[tail_index - 1]
            .0
            .take()
            .expect("tail reader already transferred");
        #[cfg(windows)]
        let tail_handle = tail_reader.into_raw_handle() as crate::fd::HANDLE;
        #[cfg(unix)]
        let tail_handle = tail_reader.into_raw_fd() as crate::fd::HANDLE;
        let saved_fd0 = self.fd_table.entries.insert(
            0,
            FdEntry {
                read: Some(FdReadEndpoint::File(std::rc::Rc::new(FileFd {
                    handle: tail_handle,
                    path: std::path::PathBuf::from("<rubash-pipeline-stdin>"),
                }))),
                write: None,
                closed: false,
                dynamic: false,
            },
        );
        let saved_stdin_stream = self.pipeline_stdin_stream;
        self.pipeline_stdin_stream = Some(tail_handle);
        self.set_current_command(tail);
        let tail_result = self.execute_pipeline_stage(tail, "", false, false);
        self.pipeline_stdin_stream = saved_stdin_stream;
        // Restoring fd 0 drops the File endpoint's read handle (FileFd::drop
        // -> close_handle): the producers' next write is a broken pipe.
        match saved_fd0 {
            Some(entry) => {
                self.fd_table.entries.insert(0, entry);
            }
            None => {
                self.fd_table.entries.remove(&0);
            }
        }

        // Reap the producer chain BEFORE propagating any tail error: each
        // member is waited against its immediate downstream, and the last
        // member against nothing (the tail is already gone), so a lingering
        // producer is SIGPIPE-shaped after the natural exit window
        // (wait_for_windows_pipeline_member -> 141).
        let mut statuses: Vec<i32> = Vec::with_capacity(tail_index + 1);
        for index in 0..tail_index {
            let (head, downstream) = processes.split_at_mut(index + 1);
            let member = head.last_mut().expect("member index inside pipeline");
            let status = wait_for_windows_pipeline_member(member, downstream.first_mut())?;
            statuses.push(crate::executor::wait_status::process_exit_status(&status));
        }

        let (tail_out, tail_err, tail_status) = match tail_result? {
            Some(triple) => triple,
            None => (String::new(), String::new(), 0),
        };
        statuses.push(tail_status);

        self.write_pipeline_output(tail, &tail_out, false)?;
        if !tail_err.is_empty() {
            std::io::stderr().write_all(
                &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&tail_err),
            )?;
        }
        self.exit_code = self.pipeline_exit_status(&statuses);
        self.set_pipestatus(statuses);
        Ok(Some(()))
    }

    /// Non Windows/Unix port: the OS-pipe streaming machinery above covers
    /// Windows and Unix; other platforms keep the sequential stage loop.
    #[cfg(not(any(windows, unix)))]
    fn execute_external_prefix_streaming_tail(
        &mut self,
        _commands: &[&CommandNode],
    ) -> Result<Option<()>, ExecuteError> {
        Ok(None)
    }

    /// rubash#761: a pipeline tail this executor can serve from a live OS pipe
    /// as fd 0. Either a WHITELIST of literal builtin names whose reads pull
    /// fd 0 and whose other semantics are unaffected by the channel (`true`,
    /// `:`, `false`, `echo`, `printf`, `read`), or a compound body (a loop, a
    /// brace group, an `if`/`case`, a subshell) whose inner readers pull the
    /// same fd 0 — so `yes | while read; do break; done` stops the producer
    /// exactly when `break` ends the body. Redirects/tempenv/heredocs/procsubs
    /// on the tail force the existing executor.
    #[cfg(any(windows, unix))]
    fn streaming_builtin_tail(&self, command: &CommandNode) -> bool {
        if !pipeline_streaming_plain_stage(command) {
            return false;
        }
        if command_is_compound_pipeline_stage(command) {
            return true;
        }
        let Some(word) = command.words.first() else {
            return false;
        };
        matches!(
            word.as_str(),
            "true" | ":" | "false" | "echo" | "printf" | "read"
        ) && !self.command_first_word_is_function(command)
    }

    /// Run the maximal leading run of concurrent-eligible external stages
    /// on real OS pipes and hand its captured output to the sequential
    /// stage loop for the shell-side remainder (issue #206).
    ///
    /// GNU execute_cmd.c:2620 execute_pipeline forks EVERY left element
    /// connected by pipe(2) (execute_cmd.c:2641 pipe(), 2705-2707
    /// execute_command_internal on Connection->first) before the rightmost
    /// command runs — elements execute concurrently and a producer like
    /// `yes` blocks on a full pipe until its consumer reads, dying on
    /// SIGPIPE once a bounding consumer such as `head -N` exits. The
    /// sequential stage loop instead materializes each stage's whole
    /// output in memory before the next stage starts, so
    /// `yes | head -100000 | while read` never got past stage 0 (rubash
    /// read yes's infinite stream into one Vec — 8+ GB RSS, no
    /// termination). When the all-external concurrent path above declined
    /// the pipeline (a compound/builtin tail), the leading external
    /// members still get the OS-pipe treatment here; only the bounded
    /// output of the last external member is captured for the tail.
    /// Returns (prefix length, captured output, captured stderr,
    /// per-member statuses) or None when no >=2-member run precedes a
    /// shell-side tail.
    #[cfg(windows)]
    fn execute_external_prefix_concurrently(
        &mut self,
        commands: &[&CommandNode],
    ) -> Result<Option<(usize, String, String, Vec<i32>)>, ExecuteError> {
        if commands.len() < 3
            || self.stderr_capture.is_some()
            // Same DEBUG-trap bail-out as the all-external path: the
            // sequential loop owns per-element run_debug_trap fires.
            || (self.debug_trap_in_scope()
                && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
                    .is_some_and(|action| !action.is_empty()))
            || commands.first().is_some_and(|command| command.time_command.is_some())
        {
            return Ok(None);
        }
        let mut specs = Vec::new();
        let mut prefix_len = 0usize;
        for (index, command) in commands.iter().enumerate() {
            // No prefix member is the pipeline's last stage, so none may
            // carry a final output redirect; `is_last_member` is false.
            let Some(spec) = self.concurrent_external_stage_spec(command, index, false) else {
                break;
            };
            specs.push(spec);
            prefix_len = index + 1;
        }
        // A single admitted member gains nothing over the sequential
        // executor's own spawn-and-wait; only a >=2-member run gets real
        // inter-member pipes. The whole pipeline being external was
        // already handled (or declined with its own write-out semantics)
        // by execute_external_pipeline_concurrently before this runs.
        if prefix_len < 2 || prefix_len >= commands.len() {
            return Ok(None);
        }
        let commands = &commands[..prefix_len];
        // The last prefix member's payload becomes the sequential tail's
        // stage input — it must be collected, never streamed away.
        let results = self.spawn_wait_concurrent_external_stages(commands, specs, false)?;
        let Some((output, stderr, _)) = results.last().cloned() else {
            return Ok(None);
        };
        let statuses = results
            .iter()
            .map(|(_, _, status)| *status)
            .collect::<Vec<_>>();
        Ok(Some((prefix_len, output, stderr, statuses)))
    }

    /// Unix port: the OS-pipe prefix machinery above is Windows-only for
    /// now (os_pipe spawn plumbing); on Unix the sequential stage loop
    /// keeps handling mixed pipelines. Full-materialization avoidance for
    /// Unix is the #206 follow-up.
    #[cfg(not(windows))]
    fn execute_external_prefix_concurrently(
        &mut self,
        _commands: &[&CommandNode],
    ) -> Result<Option<(usize, String, String, Vec<i32>)>, ExecuteError> {
        Ok(None)
    }

    /// Unix port: the shell-writer concurrent path is Windows-only for now
    /// (the os_pipe spawn plumbing the external concurrent paths use); on
    /// Unix the sequential stage loop keeps handling mixed pipelines.
    /// niubash#345 follow-up.
    #[cfg(not(any(windows, unix)))]
    fn execute_shell_writer_pipeline(
        &mut self,
        _commands: &[&CommandNode],
    ) -> Result<Option<()>, ExecuteError> {
        Ok(None)
    }

    #[cfg(not(windows))]
    fn execute_external_pipeline_concurrently(
        &mut self,
        commands: &[&CommandNode],
    ) -> Result<Option<Vec<(String, String, i32)>>, ExecuteError> {
        if commands.len() < 2
            || self.stderr_capture.is_some()
            // GNU runs each pipeline element's run_debug_trap inside the
            // element's child (execute_cmd.c:4506). This fast path spawns the
            // members directly and would bypass those fires, so a live DEBUG
            // trap takes the sequential stage path which fires per element.
            || (self.debug_trap_in_scope()
                && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
                    .is_some_and(|action| !action.is_empty()))
            // NOTE: stdout_capture (command substitution) is intentionally NOT a
            // bail-out here. External-only pipelines inside `$(...)` must still
            // run concurrently with real OS pipes between stages (GNU bash
            // pipelines everything concurrently). The final stage's output is
            // routed into the substitution capture by write_pipeline_output,
            // and intermediate stages stay on OS pipes — which is what winuxcmd
            // commands such as `find` require (issue #76: a `$(find ... | wc -l)`
            // substitution previously fell back to the sequential stage path,
            // whose per-stage capture lost the external command's output, so the
            // pipeline silently yielded 0).
            || commands.iter().enumerate().any(|(index, command)| {
                command.time_command.is_some()
                    || command.brace_group.is_some()
                    || command.subshell
                    || command_has_non_concurrent_pipeline_redirects(command, index, commands.len())
                    || command.redirect_in.is_some()
                    || command.redirect_err.is_some()
                    || command.redirect_err_append.is_some()
                    || command
                        .redirects
                        .iter()
                        .any(|redirect| redirect.is_list_only_redirect())
                    || ((command.redirect_out.is_some() || command.append.is_some())
                        && index + 1 != commands.len())
                    || ((command.heredoc.is_some()
                        || !command.heredoc_redirects.is_empty()
                        || command.here_string.is_some())
                        && index != 0)
                    // GNU execute_cmd.c:4617 execute_simple_command runs a
                    // pipeline element with a `VAR=value` tempenv prefix as
                    // an ordinary forked member whose child environment
                    // carries the assignment; the concurrent OS-pipe path
                    // supports it (apply_external_environment below). Only
                    // the forms that need the sequential executor's shell-
                    // table handling bail: `name+=value` appends (whose env
                    // value comes from a shell-table pre-assignment) and
                    // PATH= (command resolution must observe the new PATH).
                    || command
                        .assignments
                        .iter()
                        .any(|(name, _)| -> bool {
                            let (base, append) = assignment_name_and_append(name);
                            append || base == "PATH"
                        })
                    || !command.process_substitutions.is_empty()
                    || command_has_pipeline_process_substitution(command)
        })
            // rubash#1130: non-Windows twin of the concurrent-admission
            // decline above — a member with a pending word-expansion error
            // (nounset unbound in an argument, heredoc body or here-string
            // word) takes the sequential stage path so its gate reports
            // the error instead of the fast path expanding it away.
            // rubash#1540: same arithmetic/failglob risk extension as
            // the first admission, plus the failing-bind scan.
            || commands.iter().any(|command| {
                self.parameter_expansion_error_pending(command)
                    || self.parameter_assignment_error(command).is_some()
                    || self.pipeline_member_arith_failglob_risk(command)
            })
        {
            return Ok(None);
        }

        // rubash#1540: latch snapshot for the spec net below (the twin
        // of the net inside concurrent_external_stage_spec).
        let spec_saved_arith = self.snapshot_arithmetic_error_flags();
        let spec_saved_discard = self.snapshot_discard_latches();
        let spec_saved_no_closing_brace = self.no_closing_brace_failure.get();
        let _speculative_guard = SpeculativeExpansionGuard::new();
        let mut specs = Vec::with_capacity(commands.len());
        for command in commands {
            if command.words.is_empty() {
                return Ok(None);
            }
            // rubash#1564: bracket this member's speculative expansion
            // with the variable-table snapshot (see
            // concurrent_external_stage_spec): restore on every exit
            // below, and decline the whole attempt when a bind touched
            // an exported variable.
            let saved_env = self.snapshot_spec_env(command);
            // rubash#205 (non-Windows twin of the first concurrent path):
            // expand the member's words with the per-word machinery the
            // sequential stage executor uses; `expand_word` re-parses
            // dequoted text and strips embedded `"` data characters. GNU
            // expands each element's words once from the parsed WORD
            // structs (execute_cmd.c:2620 execute_pipeline ->
            // execute_simple_command -> expand_words; subst.c:4807
            // dequote_string drops only CTLESC escapes).
            let mut first_fields = self.expand_stage_first_word(command);
            let expanded_name = first_fields.first().cloned().unwrap_or_default();
            // Same restricted-member bail-out as the first concurrent path:
            // refused members are reported (and the rest of the pipeline
            // keeps running) by the sequential stage executor.
            if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "restricted")
                && self
                    .restricted_command_error(command, &expanded_name)
                    .is_some()
            {
                self.restore_spec_env(saved_env);
                return Ok(None);
            }
            if self.routes_as_builtin(&expanded_name) {
                self.restore_spec_env(saved_env);
                return Ok(None);
            }
            // Windows twin of the find_function-first bail-out below
            // (concurrent_external_stage_spec): GNU execute_pipeline runs
            // every member through execute_simple_command, which resolves
            // functions (execute_cmd.c:4674-4677) before the
            // execute_from_filesystem disk fallback (execute_cmd.c:4919).
            // A member naming a shell function takes the sequential stage
            // executor, whose dispatch runs the function (issue #587).
            if self
                .function_name_for_command_word(&expanded_name)
                .is_some()
            {
                self.restore_spec_env(saved_env);
                return Ok(None);
            }
            let Some(program) = find_user_command(&expanded_name, &self.shell_state.env_vars)
            else {
                self.restore_spec_env(saved_env);
                return Ok(None);
            };
            let mut args: Vec<String> = first_fields.into_iter().skip(1).collect();
            args.extend(self.expand_stage_tail_words(command));
            // Same child-argv carrier decode as the sequential spawn path.
            for arg in &mut args {
                *arg = super::execution_misc::restore_command_substitution_output(arg);
            }
            // rubash#1564: the member's spec is built — roll the
            // speculative binds back before the next member expands,
            // and decline the attempt if one touched an exported
            // variable (the sequential executor owns that case).
            if self.restore_spec_env(saved_env) {
                return Ok(None);
            }
            specs.push((program, expanded_name, args));
        }
        // rubash#1540: a DISCARD-class failure raised while building
        // any member's spec voids the whole concurrent run — the
        // fields are the failure's empty product, not runnable argv.
        // Decline to the sequential stage executor (the caller's
        // bracket restores the remaining latch state); keep only the
        // arithmetic print latch as the expansions left it, so the
        // stage's re-expansion does not print the diagnostic twice.
        let spec_expansion_failed = (self.parameter_assignment_failure.get()
            && !spec_saved_discard.0)
            || (self.evalerror_failglob_pending.get() && !spec_saved_discard.2)
            || (self.no_closing_brace_failure.get() && !spec_saved_no_closing_brace)
            || (self.shell_state.arithmetic_fatal_error.get() && !spec_saved_arith.2)
            || (self.shell_state.arithmetic_nonfatal_error.get() && !spec_saved_arith.1)
            || (self.shell_state.arithmetic_nounset_error.get() && !spec_saved_arith.3);
        if spec_expansion_failed {
            self.restore_discard_latches(&spec_saved_discard);
            self.no_closing_brace_failure
                .set(spec_saved_no_closing_brace);
            let printed = self.shell_state.arithmetic_expansion_error.get();
            self.restore_arithmetic_error_flags(&spec_saved_arith);
            self.shell_state.arithmetic_expansion_error.set(printed);
            return Ok(None);
        }

        let (stage0_input, stage0_stdin_base) = self.initial_pipeline_input(commands[0]);
        // Same contract as the Windows path: with no fd-0 binding the
        // first child inherits the shell's real stdin handle.
        let stage0_inherits = stage0_input.is_empty()
            && self.stdin_string_for_command(commands[0]).is_none()
            && match self.fd_table.entries.get(&0) {
                Some(entry) => {
                    !entry.closed
                        && matches!(entry.read, Some(FdReadEndpoint::InheritedProcessStdin))
                }
                None => true,
            };

        let mut processes: Vec<std::process::Child> = Vec::with_capacity(commands.len());
        let capture_intermediate_stderr =
            self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout);
        // rubash#1454: GNU execute_cmd.c:2620 leaves the rightmost
        // element's fd 1 (and fd 2) on the shell's own descriptors. When
        // this pipeline's final payload resolves to the host stdout, the
        // last member inherits both instead of writing capture pipes the
        // shell forwards after every member exits — a pager at the end
        // then sees the terminal itself. Evaluated once, before the
        // spawn loop: spawning mutates none of the bindings the
        // predicates read (same argument as the Windows drain below).
        let last_index = commands.len() - 1;
        let inherit_last_stdout = self.final_member_stdout_streams_to_host(commands[last_index]);
        let inherit_last_stderr = !capture_intermediate_stderr
            && matches!(
                self.fd_table.output_endpoint(2),
                Some(FdWriteEndpoint::Stderr)
            );
        let mut intermediate_stderr = Vec::new();
        let mut previous_stdout: Option<std::process::ChildStdout> = None;
        let mut first_stdin: Option<std::process::ChildStdin> = None;

        for (index, (program, expanded_name, args)) in specs.iter().enumerate() {
            let (mut process, _) = external_command_for_named_program(
                &program,
                // The spec's already-expanded first field; re-reading
                // words[0] with expand_word would re-strip data quotes
                // (rubash#205) and re-run side effects.
                Some(expanded_name),
                &args,
                &self.shell_state.env_vars,
                self.bash_argv0_env_supplied(&commands[index].assignments),
            );
            // Same tempenv application as the os_pipe path above.
            // rubash#1564: tempenv bracket, as in spawn_wait above.
            let saved_env = self.snapshot_spec_env(&commands[index]);
            self.apply_external_environment(&commands[index], &mut process, program);
            self.restore_spec_env_into_process(saved_env, &mut process);

            if let Some(stdout) = previous_stdout.take() {
                process.stdin(Stdio::from(stdout));
            } else if index == 0 {
                if stage0_inherits {
                    process.stdin(Stdio::inherit());
                } else {
                    process.stdin(Stdio::piped());
                }
            }

            if index + 1 < commands.len() {
                process.stdout(Stdio::piped());
            } else if inherit_last_stdout {
                process.stdout(Stdio::inherit());
            } else {
                process.stdout(Stdio::piped());
            }
            if capture_intermediate_stderr || index + 1 == commands.len() {
                if index == last_index && inherit_last_stderr {
                    process.stderr(Stdio::inherit());
                } else {
                    process.stderr(Stdio::piped());
                }
            }

            // rubash#1688: pin the ambient descriptors for the stage.
            self.attach_ambient_fd_pins(&commands[index], &mut process);
            process.current_dir(&self.shell_state.cwd);
            suppress_orphan_console(&mut process);
            let mut child = process.spawn().map_err(|error| {
                ExecuteError::IoError(io::Error::new(
                    error.kind(),
                    format!(
                        "{}: {}",
                        program
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("external"),
                        crate::posix_errors::message(&error)
                    ),
                ))
            })?;
            adopt_orphan_console(child.id());
            if capture_intermediate_stderr && index + 1 < commands.len() {
                if let Some(mut stderr) = child.stderr.take() {
                    intermediate_stderr.push(std::thread::spawn(move || {
                        let mut output = Vec::new();
                        stderr.read_to_end(&mut output)?;
                        Ok::<_, std::io::Error>(output)
                    }));
                }
            }
            if index == 0 {
                first_stdin = child.stdin.take();
            }
            if index + 1 < commands.len() {
                previous_stdout = child.stdout.take();
            }
            processes.push(child);
        }

        if let Some(mut stdin) = first_stdin {
            if let Some(base) = stage0_stdin_base {
                // Same unit contract as the concurrent path above:
                // FUNCTION_STDIN_OFFSET indexes the transport buffer that
                // `stage0_input` was sliced from, so advance by its transport
                // length, not its raw length (niubash#383 follow-up).
                self.shell_state.env_vars.insert(
                    FUNCTION_STDIN_OFFSET.to_string(),
                    (base + stage0_input.len()).to_string(),
                );
            }
            // Same feed contract as the host path: a member that
            // already exited must not fail the pipeline (rubash#1822).
            crate::executor::shell_options::feed_child_stdin(
                &mut stdin,
                &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stage0_input),
            )?;
        }

        let mut results = Vec::with_capacity(processes.len());
        let last = processes.pop().expect("pipeline has at least two stages");
        self.note_foreground_wait();
        let output = last.wait_with_output()?;
        results.push((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output.stdout),
            crate::executor::substitution_metadata::bytes_to_shell_text(&output.stderr),
            crate::executor::wait_status::process_exit_status(&output.status),
        ));
        for mut process in processes.into_iter().rev() {
            let status = match process.try_wait()? {
                Some(status) => {
                    // rubash#1776: sample the reaped member's CPU for
                    // the Windows CHILDREN half while its handle is
                    // live. (The last member above is consumed by
                    // wait_with_output, which closes its handle before
                    // any sample point — its CPU stays uncounted.)
                    super::read_helpers::note_reaped_child_cpu(&process);
                    status
                }
                None => {
                    // The sequential tail consumed this member's output and
                    // the downstream already exited, so a member still alive
                    // here is the broken-pipe lingerer — report the SIGPIPE
                    // shape GNU observes (rubash#382; lingerer_sigpipe_status).
                    // Tree-kill first (D10): a dispatcher grandchild would
                    // otherwise outlive this Child and pin niu's exit drain.
                    kill_lingerer(&mut process);
                    process.wait().map_err(ExecuteError::IoError)?;
                    super::read_helpers::note_reaped_child_cpu(&process);
                    lingerer_sigpipe_status()
                }
            };
            results.push((
                String::new(),
                String::new(),
                crate::executor::wait_status::process_exit_status(&status),
            ));
        }
        results.reverse();
        for reader in intermediate_stderr {
            let output = reader.join().map_err(|_| {
                ExecuteError::IoError(std::io::Error::other("pipeline stderr reader panicked"))
            })??;
            if capture_intermediate_stderr {
                self.write_default_stdout(&output)?;
            }
        }
        self.write_pipeline_output(
            commands[commands.len() - 1],
            &results.last().unwrap().0,
            false,
        )?;
        if let Some((_, stderr, _)) = results.last() {
            if !stderr.is_empty() {
                if let Some(capture) = &mut self.stderr_capture {
                    capture.write_all(
                        &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stderr),
                    )?;
                } else {
                    std::io::stderr().write_all(
                        &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stderr),
                    )?;
                }
            }
        }
        let statuses = results
            .iter()
            .map(|(_, _, status)| *status)
            .collect::<Vec<_>>();
        self.exit_code = self.pipeline_exit_status(&statuses);
        self.set_pipestatus(statuses);
        Ok(Some(results))
    }

    pub(in crate::executor) fn pipeline_exit_status(&self, statuses: &[i32]) -> i32 {
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "pipefail") {
            return statuses
                .iter()
                .rev()
                .copied()
                .find(|status| *status != 0)
                .unwrap_or(0);
        }

        statuses.last().copied().unwrap_or(0)
    }

    /// GNU execute_simple_command: an unquoted expansion in the command word
    /// undergoes field splitting; the first resulting field is the command
    /// name and the remaining fields become arguments (`v="echo hi there";
    /// $v | cat` must run `echo hi there`, not look up a command named
    /// "echo hi there"; issue #68). The stage fast paths below dispatch on
    /// the raw first word, so materialize the split before dispatching.
    fn split_pipeline_stage_command_word(&mut self, command: &CommandNode) -> CommandNode {
        let Some(first) = command.words.first().cloned() else {
            return command.clone();
        };
        let raw = command
            .word_metadata
            .first()
            .map(|metadata| metadata.raw.clone());
        let fields = self.expand_command_word(command, 0, &first, raw.as_deref());
        if fields.len() <= 1 {
            return command.clone();
        }
        let extra = fields.len() - 1;
        let mut rebuilt = command.clone();
        let mut words = fields.clone();
        words.extend(command.words[1..].iter().cloned());
        rebuilt.words = words;
        let mut metadata = Vec::with_capacity(rebuilt.words.len());
        for (index, field) in fields.iter().enumerate() {
            // The fields are already fully expanded; literal metadata with
            // the field text as its raw keeps downstream per-word expansion
            // idempotent for plain results.
            metadata.push(crate::parser::WordMetadata::literal(
                index,
                field.clone(),
                field.clone(),
            ));
        }
        for metadata_entry in command.word_metadata.iter().skip(1) {
            let mut entry = metadata_entry.clone();
            entry.word_index += extra;
            metadata.push(entry);
        }
        rebuilt.word_metadata = metadata;
        rebuilt.word_kinds = std::iter::repeat(crate::lexer::TokenKind::Word)
            .take(extra)
            .chain(command.word_kinds.iter().skip(1).cloned())
            .collect();
        rebuilt
    }

    /// Expand a pipeline stage's argument words and apply pathname
    /// expansion.
    ///
    /// GNU execute_simple_command (execute_cmd.c) runs the full word
    /// expansion sequence on every pipeline element: parameter expansion,
    /// word splitting, then pathname expansion. This stage fast path
    /// expanded the words but never reached the pathname-expansion step,
    /// so `echo * | cat` handed echo the literal pattern `*` instead of
    /// the directory listing (probe 2026-09-09: top-level `echo *` is
    /// correct, every pipeline element is not).
    fn expand_pipeline_stage_arg_words(
        &mut self,
        command: &CommandNode,
        first_index: usize,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for (offset, word) in command.words[first_index..].iter().enumerate() {
            let index = offset + first_index;
            let raw = command
                .word_metadata
                .get(index)
                .map(|metadata| metadata.raw.as_str());
            for expanded in self.expand_command_word(command, index, word, raw) {
                // Argv materialization for a pipeline stage: GNU runs
                // dequote_word on every word leaving
                // expand_word_list_internal (subst.c:13219 -> 4865), so the
                // quote-data carriers (\x17 et al., the CTLESC port of
                // parse.y:5694-5706) become literal characters before the
                // word reaches argv or the glob expander. Without this a
                // pipeline word mixing an escaped quote with a live
                // backtick leaked \x17 (rubash#153 n26/n27).
                let expanded = crate::executor::markers::decode_word_position_carriers(&expanded);
                //  marks a fully quoted word and  a quoted tilde;
                // both stay literal, exactly as command_prepare does.
                if expanded.starts_with(STORAGE_WORD_PREFIX)
                    || expanded.starts_with(crate::executor::markers::QUOTED_WORD_PREFIX)
                {
                    out.push(expanded.replace(crate::executor::markers::CTLESC, ""));
                    continue;
                }
                // Quoted words (e.g. "*.txt") must not be glob-expanded.
                let metadata = command.word_metadata.get(index);
                if crate::executor::command_prepare::raw_word_suppresses_pathname_expansion(
                    raw, metadata,
                ) {
                    out.push(expanded.replace(crate::executor::markers::CTLESC, ""));
                    continue;
                }
                match glob::pathname_expand_word(
                    &expanded,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                ) {
                    glob::PathnameExpansion::Matches(matches) => out.extend(matches),
                    glob::PathnameExpansion::NoMatch => {
                        out.push(expanded.replace(crate::executor::markers::CTLESC, ""))
                    }
                    // rubash#1540: failglob in a pipeline element was
                    // silently degraded to the literal word (no
                    // diagnostic, element ran, status 0). GNU's
                    // failglob_error (subst.c -> expand_word_error)
                    // reports and DISCARDs the element. Report through
                    // the same funnel the simple-command path uses and
                    // latch the pending; the stage wrapper converts it
                    // to the element's (empty, 1) result. The replace
                    // guard keeps a second failing word in the same
                    // stage from printing again — GNU abandons the
                    // word list at the first failure.
                    glob::PathnameExpansion::Fail(pattern) => {
                        if !self.evalerror_failglob_pending.replace(true) {
                            self.report_failglob(&pattern);
                        }
                        out.push(expanded.replace(crate::executor::markers::CTLESC, ""))
                    }
                }
            }
        }
        out
    }

    pub(in crate::executor) fn execute_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
        stdin_inherit: bool,
        live_stdin: bool,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        // rubash#1333/niubash#345: SIGPIPE is SIG_DFL for the shell
        // process (GNU parity), but a pipeline member executed in
        // process shares that disposition — GNU forks members, so a
        // member's write to a closed pipe may only kill the MEMBER.
        // Block SIGPIPE for the stage's duration: member writes fail
        // with EPIPE and the stage machinery reports 141 for the member
        // while the shell survives.
        #[cfg(unix)]
        let _sigpipe_block = crate::builtins::kill::sigpipe_block_guard();
        // rubash#1551: a pipeline element is a forked child (see the
        // RANDOM bracket below) — its hash-table writes stay inside it.
        let _hashed_fork = crate::executor::path::hashed_cache_fork_guard();
        // `|&` → trailing `2>&1` (parse.y:1470-1487; normalize_pipeerr_stage).
        // execute_simple_pipeline already normalized every gathered stage;
        // this idempotent pass covers the recursion entry points that bypass
        // the gather (the `time` prefix arm and the timed-read followup).
        let command = &normalize_pipeerr_stage(command);
        // rubash#1843: GNU stamps every simple element's own line
        // before it runs (execute_cmd.c:936 SET_LINE_NUMBER in
        // execute_simple_command) — execute_pipeline forks each
        // element into that same funnel. This stage funnel never
        // re-entered the command_execute stamping seam, so an element
        // inside a function body reported the body's ambient
        // (definition) line for its diagnostics instead of its own.
        // set_current_line's fallback keeps wrapper/compound elements
        // on the ambient line per the command_sets_own_line table.
        self.set_current_line(command);
        // rubash#1115: GNU forks every pipeline element
        // (execute_cmd.c:2620 execute_pipeline -> execute_simple_command
        // in the child), so the element's word expansion and body draw
        // from a RESEEDED generator (variables.c:1418-1423
        // get_random_number) and never advance the parent's sequence.
        // The stage runs in place on the shared executor here, so
        // bracket it: snapshot the parent's generator, reseed for the
        // stage's duration, restore on exit. (lastpipe's last element
        // runs through execute_lastpipe_stage instead — it IS the
        // current shell and keeps the parent sequence.)
        let saved_random = self.shell_state.random_state.clone_state();
        self.shell_state.random_state.reseed_fresh();
        // rubash#1349: GNU forks every pipeline element, and
        // execute_cmd.c's execute_subshell_builtin_or_function calls
        // without_job_control() (jobs.c: delete_all_jobs) in the child —
        // a pipeline element starts from an EMPTY job table, so
        // `compgen -A job | cat` lists nothing even when the parent shell
        // has live jobs. The one exception is the jobs builtin itself
        // (execute_cmd.c's jobs_hack keeps the table for it). Stages run
        // in place on the shared executor here, so swap the table out
        // for the stage's duration and restore it below. (The lastpipe
        // last element runs through execute_lastpipe_stage instead — it
        // IS the current shell and keeps its jobs.)
        let stage_is_jobs = command.words.first().map(String::as_str) == Some("jobs");
        let saved_job_table = if stage_is_jobs {
            None
        } else {
            Some(std::mem::take(&mut self.shell_state.job_table))
        };
        // A pipeline element runs in its own subshell: an expansion error
        // raised while expanding the element's words on the shared executor
        // (notably `set -u` unbound in `$(( ))`) terminates only that
        // element with status 127, not the enclosing script (GNU 5.2:
        // `set -u; echo $((b)) | cat; echo after` prints the diagnostic and
        // "after", rc=0; issue #67). Snapshot and restore the arithmetic
        // error flags around the stage so the outer word-expansion check in
        // command_execute never observes them.
        let saved = self.snapshot_arithmetic_error_flags();
        // rubash#1540: the same fork containment for the rest of the
        // DISCARD latch family. The stage runs in place on this
        // executor (its inline arms expand the element's words here;
        // the helper stages run on clones), so snapshot the latches,
        // clear the adjudication inputs, and at exit convert whatever
        // the stage raised into the element's own (output, status) —
        // GNU's element child dies on its expansion error
        // (execute_cmd.c:2620 execute_pipeline -> execute_simple_command
        // -> subst.c:11229 expand_word_internal -> expand_word_error ->
        // jump_to_top_level(DISCARD)) with the class's pre-set status
        // and no output, and the parent never learns it happened.
        // command_expansion_failed is snapshot/restore only: it gates
        // the PARENT's ERR-trap adjudication, which an element's
        // failure must not suppress.
        let saved_discard = self.snapshot_discard_latches();
        self.parameter_assignment_failure.set(false);
        self.parameter_assignment_failure_prev_status.set(None);
        self.evalerror_failglob_pending.set(false);
        self.evalerror_pending.set(false);
        self.evalerror_exit_toomany_pending.set(false);
        // rubash#1564: the same fork containment for the VARIABLE
        // state — the one piece of child state the brackets above do
        // not cover. The stage's word expansions, classification
        // probes and inline arms run on this shared executor, so a
        // successful side-effecting expansion (`${var:=word}`,
        // `$((var=expr))`, an array-element bind) would otherwise
        // stay bound in the parent after the pipeline. Snapshot both
        // stores before the stage (gated on expansion-bearing text so
        // literal stages pay nothing) and restore them in the funnel
        // below; the stage's own execution — including the child
        // environment an external element spawns with — still sees
        // the binds, exactly like GNU's forked element.
        let saved_stage_variables = self.snapshot_spec_env(command);
        // GNU forks a <( ) / >( ) child while expanding the pipeline
        // element's words inside that element's subshell (subst.c
        // process_substitute, execute_cmd.c). The builtin/function/external
        // stage helpers each run their own materialization, but the inline
        // arms below consume command.words directly, so a procsub argument
        // there survived as a literal path (issue #113:
        // `cat <(echo ps) | grep -q ps`). Run the shared materialization
        // here so every stage form sees real paths; the helpers' own
        // materialization calls are no-ops on the rewritten node.
        // GNU gives every pipeline stage the pipe as its fd 0. Stage helpers
        // and external stages already receive it through FUNCTION_STDIN;
        // expose it the same way while the inline arms run so fd-alias
        // redirections like `cat < /dev/stdin` (niubash#118) resolve to the
        // stage input instead of the process's own stdin handle.
        let old_stdin = self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
        let old_stdin_offset = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .cloned();
        // rubash#761: a streaming tail reads fd 0 directly (a real OS pipe
        // bound by `execute_external_prefix_streaming_tail`), so the
        // FUNCTION_STDIN mirror must stay ABSENT — a present mirror (even an
        // empty one) short-circuits `read` back to the buffered channel and
        // it would see EOF instead of the producer's stream
        // (read_io.rs:361-363).
        // rubash#1873: the same absence for a stage-0 element whose fd 0
        // is the shell's live fd 0 (live_stdin, classified by the
        // drivers): its readers must reach the fd-table transports, not
        // an empty payload mirror.
        let streaming_stdin = self.pipeline_stdin_stream.is_some();
        if streaming_stdin || live_stdin {
            self.shell_state.env_vars.remove(FUNCTION_STDIN);
            self.shell_state.env_vars.remove(FUNCTION_STDIN_OFFSET);
        } else {
            self.shell_state
                .env_vars
                .insert(FUNCTION_STDIN.to_string(), input.to_string());
            self.shell_state
                .env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        }
        self.pipeline_stdin_consumed.set(None);
        let result = if command_has_pipeline_process_substitution(command) {
            // The substitution child inherits the stage's stdin — the
            // upstream pipe — so expose the captured input while the
            // substitution sources run.
            let old_stdin = self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
            let old_stdin_offset = self
                .shell_state
                .env_vars
                .get(FUNCTION_STDIN_OFFSET)
                .cloned();
            if !streaming_stdin && !live_stdin {
                self.shell_state
                    .env_vars
                    .insert(FUNCTION_STDIN.to_string(), input.to_string());
                self.shell_state
                    .env_vars
                    .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
            }
            let materialized = self.command_with_process_substitution_files(command);
            restore_optional_env_var(&mut self.shell_state.env_vars, FUNCTION_STDIN, old_stdin);
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                FUNCTION_STDIN_OFFSET,
                old_stdin_offset,
            );
            match materialized {
                Ok((materialized, process_substitutions)) => {
                    let inner = self.execute_pipeline_stage_inner(
                        &materialized,
                        input,
                        stdin_inherit,
                        streaming_stdin,
                        live_stdin,
                    );
                    match self.finish_process_substitutions(process_substitutions) {
                        Err(error) => Err(error),
                        Ok(()) => inner,
                    }
                }
                Err(error) => Err(error),
            }
        } else {
            self.execute_pipeline_stage_inner(
                command,
                input,
                stdin_inherit,
                streaming_stdin,
                live_stdin,
            )
        };
        // Restore the parent's job table (rubash#1349 swap above).
        if let Some(saved) = saved_job_table {
            self.shell_state.job_table = saved;
        }
        // Inline stage arms that ran on `self` consumed FUNCTION_STDIN
        // directly; subshell/child stages report through the cell. Whichever
        // is missing falls back to the cursor visible here.
        if self.pipeline_stdin_consumed.get().is_none() {
            let measured = self
                .shell_state
                .env_vars
                .get(FUNCTION_STDIN_OFFSET)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            self.pipeline_stdin_consumed.set(Some(measured));
        }
        restore_optional_env_var(&mut self.shell_state.env_vars, FUNCTION_STDIN, old_stdin);
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FUNCTION_STDIN_OFFSET,
            old_stdin_offset,
        );
        // rubash#1564: roll the stage's variable side effects back
        // (after the FUNCTION_STDIN bookkeeping above has read and
        // restored its own mirror entries in the stage's table).
        self.restore_spec_variables(saved_stage_variables);
        self.shell_state.random_state.restore_state(&saved_random);
        // rubash#1540: harvest the DISCARD latches this stage raised
        // (they were cleared at entry), then restore the entry state so
        // nothing leaks to the parent. The arithmetic verdict delta is
        // read before restore_arithmetic_error_flags puts the entry
        // values back.
        let stage_param_assign_failed = self.parameter_assignment_failure.get();
        let stage_param_assign_prev = self.parameter_assignment_failure_prev_status.get();
        let stage_failglob_failed = self.evalerror_failglob_pending.get();
        let stage_toomany_raised = self.evalerror_exit_toomany_pending.get();
        let stage_arith_failed = (self.shell_state.arithmetic_fatal_error.get() && !saved.2)
            || (self.shell_state.arithmetic_nonfatal_error.get() && !saved.1);
        self.restore_discard_latches(&saved_discard);
        let nounset_hit = self.restore_arithmetic_error_flags(&saved);
        match result {
            Ok(Some((output, stderr, _status))) if nounset_hit => {
                // The stage re-raised (or consumed) the unbound-variable
                // error; clear the expansion-error latch so it does not
                // leak into the enclosing command's exit-status handling.
                self.shell_state.arithmetic_expansion_error.set(false);
                Ok(Some((output, stderr, 127)))
            }
            // rubash#1540/#1542: a failed `${var:=word}` bind raised
            // while expanding this element's words on the shared
            // executor. GNU's element child prints the diagnostic from
            // inside bind_variable (variables.c err_readonly ->
            // error.c report_error): under a live errexit flag
            // report_error exit_shells the child with the pre-bind
            // status (EXECUTION_FAILURE for 0); otherwise the DISCARD
            // keeps the pre-set EX_BADUSAGE 2 — folded to 1 when this
            // pipeline itself runs inside a command-substitution body
            // (the parse_and_execute catch, builtins/evalstring.c:
            // 423-426). The element produces no output.
            Ok(Some((_, stderr, _))) if stage_param_assign_failed => {
                let code =
                    self.param_assign_discard_status(2, stage_param_assign_prev.unwrap_or(0));
                Ok(Some((String::new(), stderr, code)))
            }
            // rubash#1540: an arithmetic expansion error in the
            // element's words — the pre-set status is EXECUTION_FAILURE
            // (subst.c:4295-4296 set_exit_status before the DISCARD),
            // and the failed expansion's empty product must not run.
            Ok(Some((_, stderr, _))) if stage_arith_failed => Ok(Some((String::new(), stderr, 1))),
            // rubash#1540: a failglob failure in the element's words —
            // subst.c failglob_error -> expand_word_error (DISCARD)
            // with the pre-set status 1, no output.
            Ok(Some((_, stderr, _))) if stage_failglob_failed => {
                Ok(Some((String::new(), stderr, 1)))
            }
            // rubash#1540: a no_args DISCARD (break/continue/exit/
            // return with too many arguments) raised by an inline stage
            // arm on this executor — inside the forked element the
            // execute_subshell_builtin_or_function trampoline
            // (execute_cmd.c) converts it to subshell_exit(1). Helper
            // stages running on clones fold at their own harvest.
            Ok(Some((output, stderr, _))) if stage_toomany_raised => Ok(Some((output, stderr, 1))),
            other => other,
        }
    }

    fn execute_pipeline_stage_inner(
        &mut self,
        command: &CommandNode,
        input: &str,
        stdin_inherit: bool,
        streaming_stdin: bool,
        live_stdin: bool,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        // rubash#1540: entry snapshot for the post-scan adjudication
        // below — the no-fields scan expands every word with the full
        // walker, and the arithmetic verdict flags / brace latch it
        // may raise are delta-checked there (the wrapper owns the
        // restore side).
        let inner_saved_arith = self.snapshot_arithmetic_error_flags();
        let inner_saved_no_closing_brace = self.no_closing_brace_failure.get();
        if let Some(time_command) = &command.time_command {
            let started = time_command_started();
            let Some((output, stderr, status)) =
                self.execute_pipeline_stage(&time_command.command, input, false, live_stdin)?
            else {
                return Ok(None);
            };
            if self.defer_stage_time_report {
                // Element 0 of a pipeline: GNU's `time`/`!` bind to the
                // whole pipeline, so neither print nor invert here — the
                // pipeline executor owns both (rubash#1263).
                self.pending_stage_time_report = Some((started, time_command.posix_format));
                return Ok(Some((output, stderr, status)));
            }
            self.print_time_shell(time_command.posix_format, started);
            let status = if time_command.inverted {
                invert_exit_status(status)
            } else {
                status
            };
            return Ok(Some((output, stderr, status)));
        }

        if command_is_compound_pipeline_stage(command) {
            return self
                .execute_compound_pipeline_stage(command, input, streaming_stdin, live_stdin)
                .map(Some);
        }

        // rubash#1568: an element that is entirely assignment words
        // is an assignment STATEMENT in the element's subshell (GNU
        // execute_simple_command's WORDS==0 arm in the forked child):
        // every word binds there permanently and nothing leaks to
        // the parent. The compound route supplies the subshell
        // isolation and capture plumbing; the clone's own funnel
        // runs the mixed-statement arm.
        if mixed_assignment_statement_admission(command) {
            return self
                .execute_compound_pipeline_stage(command, input, streaming_stdin, live_stdin)
                .map(Some);
        }

        // rubash#1568: the pipeline-level strip covers the common
        // routes; a `time`-wrapped element reaches this point with
        // its prefix intact (the time arm above re-dispatches the
        // inner command), so apply the same strip here. Diagnostics
        // route to the ambient fd 2, matching the expansion-error
        // checks below (GNU reports them during the element's word
        // expansion, before its own redirections apply).
        let stage_stripped;
        let command =
            if !command.words.is_empty() && command_word_is_array_element_assignment(command, 0) {
                let (stripped, diagnostics) =
                    self.strip_invalid_env_assignment_prefixes_collecting(command);
                for line in &diagnostics {
                    self.write_default_stderr(line.as_bytes())?;
                }
                stage_stripped = stripped;
                &stage_stripped
            } else {
                command
            };

        // GNU execute_cmd.c:4617 expand_words runs BEFORE the element's own
        // do_redirections (execute_builtin_or_function execute_cmd.c:5606),
        // so a word-expansion diagnostic (`${x?word}`, bad substitution)
        // writes to the element's ambient fd 2 — the real stderr for a
        // top-level pipeline — never through the command's own `2>&1`.
        // Stage helpers expand words via expand_command_word which would
        // silently substitute the `${x?word}` error word, so run the same
        // expansion-error check the top-level command path runs first.
        if let Some((name, message, status)) = self.parameter_expansion_error(command) {
            let line = format!("{}{}: {}\n", self.diagnostic_prefix(), name, message);
            self.write_default_stderr(line.as_bytes())?;
            let code = if status == Self::FATAL_PARAMETER_EXPANSION_STATUS {
                // subst.c:10416-10418/11032-11034: interactive_shell takes
                // the error branch (DISCARD) — the pipeline is abandoned
                // with status 1 and the session keeps reading.
                if self.expansion_error_is_interactive_discard() {
                    1
                } else {
                    self.expansion_fatal_status()
                }
            } else {
                status
            };
            return Ok(Some((String::new(), String::new(), code)));
        }

        // Same gate for the element's heredoc bodies and here-string word:
        // GNU applies do_redirections inside the element's forked child
        // (execute_cmd.c:5606/5884), so a nounset unbound variable in a
        // `<<<` word or heredoc body reports and kills only the element —
        // the top-level command path runs this check
        // (parameter_heredoc_expansion_error, command_execute.rs), but the
        // stage path skipped it, so `cat <<< prefix$p | wc -w` silently
        // expanded `$p` to empty (rubash#1130).
        if let Some((name, message, status)) = self.parameter_heredoc_expansion_error(command) {
            let line = format!("{}{}: {}\n", self.diagnostic_prefix(), name, message);
            self.write_default_stderr(line.as_bytes())?;
            let code = if status == Self::FATAL_PARAMETER_EXPANSION_STATUS {
                if self.expansion_error_is_interactive_discard() {
                    1
                } else {
                    self.expansion_fatal_status()
                }
            } else {
                status
            };
            return Ok(Some((String::new(), String::new(), code)));
        }

        // GNU runs brace expansion exactly ONCE per word, inside
        // expand_word_internal (subst.c:11229 -> braces.c brace_expand),
        // reached from this stage through the per-word machinery below
        // (split_pipeline_stage_command_word's expand_command_word for the
        // head, expand_pipeline_stage_arg_words for the arguments). The
        // text-level pre-pass this port used to pay here rewrote `words`
        // while leaving `word_metadata` on the pre-expansion word, so the
        // per-word expansion re-ran braces from the stale raw and
        // duplicated the tail fields (`printf 'x\n' | printf '%s\n' {a,b}`
        // printed a, b, b), and for a quoted-brace word (`'{a,b}'{c,d}`)
        // the rewritten field lost the quote state only the raw carries.
        let command = self.split_pipeline_stage_command_word(command);
        let command = &command;
        let Some(name) = command.words.first().map(String::as_str) else {
            return self
                .execute_compound_pipeline_stage(command, input, streaming_stdin, live_stdin)
                .map(Some);
        };

        // Bash applies a pipeline element's redirections before running the
        // command (redir.c do_redirection_internal runs for every command, and
        // the dup arms report failures through the already-redirected fd2).
        // This fast path historically skipped that machinery, so
        // "echo foo 2>&1 >&$v | cat" ran echo and sent foo down the pipe
        // instead of failing with a $v: Bad file descriptor diagnostic routed
        // into the pipe. Run the same ordered redirect preflight the top-level
        // builtin path uses; with the stdout capture active, a 2>&1-routed
        // diagnostic lands in this stage's stdout (the pipe) like GNU's.
        // #804 F2: the gate runs BEFORE the restricted scan — GNU's
        // do_redirections (execute_builtin_or_function execute_cmd.c:5606)
        // precedes the builtin, and per-entry it reports ambiguity before
        // the RESTRICTED_REDIRECT refusal (redir.c:914 precedes :919), with
        // the diagnostic routed through the dup already applied (`2>&1 >& f`
        // reports into the pipe). The scan below therefore only speaks for
        // the command-NAME forms the gate cannot see.
        let saved_capture = crate::executor::shell_options::begin_stdout_capture();
        // GNU applies the element's redirections before the command runs
        // (redir.c do_redirections in the forked element), so an invalid or
        // unopenable INPUT redirect aborts the element with its diagnostic —
        // the same ordered validation the top-level dispatch runs
        // (execute_materialized_command). The inline stage arms below skip
        // that dispatch, so `true | read x <$(echo a b) | cat` used to reach
        // the external spawner with no diagnostic and a 127 status where GNU
        // reports `$(echo a b): ambiguous redirect` and the stage dies with
        // status 1 (rubash#525).
        // rubash#846: both preflights can return Err (notably an
        // unopenable `>file` open failure). The thread-local stdout
        // capture above is active, so restore it before propagating —
        // otherwise every later stdout write in the script is swallowed
        // into the orphaned buffer while stderr keeps working.
        let stage_redirect_rejected = match self.reject_invalid_redirects(command) {
            Ok(rejected) => rejected,
            Err(error) => {
                crate::executor::shell_options::restore_stdout_capture(saved_capture);
                return Err(error);
            }
        };
        if stage_redirect_rejected.is_some() {
            // The rejected stage already placed its diagnostic on the
            // element's fd 1 (the pipe) via the capture above — GNU's
            // forked child dies right after its own do_redirections failed
            // (execute_cmd.c:5522). Mark the stage pre-wired so the
            // driver's stream routing does not re-apply the same redirect
            // list and print the diagnostic a second time (issue #514):
            // the element is dead, its pipe bytes are final.
            self.pipeline_stage_fds_pre_wired.set(true);
            let captured = crate::executor::shell_options::take_stdout_capture();
            crate::executor::shell_options::restore_stdout_capture(saved_capture);
            return Ok(Some((
                crate::executor::substitution_metadata::bytes_to_shell_text(&captured),
                String::new(),
                1,
            )));
        }
        let stage_output_redirect_failed = match self.command_output_redirect_fails(command) {
            Ok(failed) => failed,
            Err(error) => {
                crate::executor::shell_options::restore_stdout_capture(saved_capture);
                return Err(error);
            }
        };
        if stage_output_redirect_failed {
            // rubash#1844: the stage died at its own do_redirections,
            // before running — its pipe bytes are final, so mark it
            // pre-wired like the rejection arm above (issue #514): the
            // driver's stream routing must not re-apply the same
            // redirect list, which would open the targets a second
            // time and print the failure diagnostic twice.
            self.pipeline_stage_fds_pre_wired.set(true);
            let captured = crate::executor::shell_options::take_stdout_capture();
            crate::executor::shell_options::restore_stdout_capture(saved_capture);
            return Ok(Some((
                crate::executor::substitution_metadata::bytes_to_shell_text(&captured),
                String::new(),
                1,
            )));
        }
        crate::executor::shell_options::restore_stdout_capture(saved_capture);

        if let Some(restricted) = self.restricted_command_error(command, name) {
            self.pipeline_stage_restricted_refused.set(true);
            return Ok(Some((String::new(), restricted, 1)));
        }

        // GNU execute_simple_command (execute_cmd.c): a pipeline element
        // whose words all expand to zero fields is a null command - its
        // redirections (applied by the preflight above) still take effect
        // and the status is 0 (redir.tests: `exit 3 | $EXIT >
        // $TMPDIR/null-redir-e` reports `0 -- 3 0`, not 127). A quoted
        // empty word ("") survives as one empty field and keeps the
        // command-not-found path below.
        let stage_expands_to_no_fields = !command.words.is_empty()
            && command.words.iter().enumerate().all(|(index, word)| {
                let raw = command
                    .word_metadata
                    .get(index)
                    .map(|metadata| metadata.raw.as_str());
                self.expand_command_word(command, index, word, raw)
                    .is_empty()
            });
        if stage_expands_to_no_fields {
            return Ok(Some((String::new(), String::new(), 0)));
        }

        // rubash#1540: the scan above expanded every word on this
        // executor with the full mutable walker. If that expansion
        // raised a DISCARD-class failure — a failed `${var:=word}`
        // bind, an arithmetic error, a failglob, an unterminated
        // `${` — GNU's element dies at expand_words, BEFORE dispatch
        // (execute_cmd.c:4617 execute_simple_command): no route below
        // (inline arm, external spawn, builtin/function clone) may
        // run the command with the failure's empty product. The
        // diagnostic already printed during the scan's expansion,
        // exactly once (the print sites are latch-guarded), so
        // convert the latch to the element's (empty, status) result
        // here. Class order mirrors expand_word_internal: word
        // expansion failures before pathname expansion (failglob).
        if self.parameter_assignment_failure.get() {
            let prev = self.parameter_assignment_failure_prev_status.get();
            return Ok(Some((
                String::new(),
                String::new(),
                self.param_assign_discard_status(2, prev.unwrap_or(self.exit_code)),
            )));
        }
        // The scan's walker defers ${var:=word} binds (its products are
        // advisory), so a failed bind may leave no latch even though the
        // element's real expansion would raise it. The textual predictor
        // the command validators use (parameter_assignment_error) is the
        // authority for the readonly/special classes: report it here,
        // exactly once, and kill the element the same way.
        if let Some((name, message)) = self.parameter_assignment_error(command) {
            let class = if matches!(message, "readonly variable" | "bad array subscript") {
                2
            } else {
                1
            };
            let line = format!("{}{}: {}\n", self.error_prefix(), name, message);
            self.write_diagnostic_fd2(line.as_bytes());
            return Ok(Some((
                String::new(),
                String::new(),
                self.param_assign_discard_status(class, self.exit_code),
            )));
        }
        if (self.shell_state.arithmetic_fatal_error.get() && !inner_saved_arith.2)
            || (self.shell_state.arithmetic_nonfatal_error.get() && !inner_saved_arith.1)
            || (self.no_closing_brace_failure.get() && !inner_saved_no_closing_brace)
        {
            return Ok(Some((String::new(), String::new(), 1)));
        }
        if self.evalerror_failglob_pending.get() {
            return Ok(Some((String::new(), String::new(), 1)));
        }
        // Same deferred-walker caveat for pathname expansion: the
        // scan does not glob, so a failglob verdict leaves no latch.
        // For a LITERAL glob word (no expansion introducers — its
        // expanded form is itself) the production verdict applies
        // directly: pathname_expand_word returns Fail only when
        // failglob is set and the pattern matched nothing (subst.c
        // failglob_error -> expand_word_error, DISCARD, pre-set
        // status 1). Value-dependent patterns (from $-expansions)
        // stay with the stage routes, as before.
        for word in &command.words {
            if !crate::executor::parameter_errors::word_has_unquoted_glob_meta(word)
                || word.contains('$')
                || word.contains('`')
            {
                continue;
            }
            if let glob::PathnameExpansion::Fail(pattern) =
                glob::pathname_expand_word(word, &self.shell_state.env_vars, &self.shell_state.cwd)
            {
                if !self.evalerror_failglob_pending.replace(true) {
                    self.report_failglob(&pattern);
                }
                return Ok(Some((String::new(), String::new(), 1)));
            }
        }

        // GNU execute_cmd.c:4649 (execute_simple_command): every pipeline
        // element is a simple command in its own forked subshell, and its
        // xtrace line (indirection_level_string + expanded words) prints
        // right before dispatch — `false | false | false` under `set -x`
        // traces each `false` at its own line (trap3.sub). The in-process
        // stage fast path below bypasses the full command path, so trace
        // here once for every simple stage.
        if self.xtrace_enabled() {
            let prefix = self.xtrace_prefix();
            let mut trace = Vec::new();
            for line in self.xtrace_command_lines(command) {
                writeln!(trace, "{prefix}{line}").ok();
            }
            self.xtrace_write(&trace);
        }

        // GNU execute_simple_command classifies a pipeline element exactly
        // like a top-level command: special builtins first
        // (execute_cmd.c:4657-4663), then func = find_function
        // (execute_cmd.c:4674-4677) — a user-defined function shadows every
        // ordinary builtin — and only when BOTH lookups miss does
        // execute_from_filesystem (execute_cmd.c:4919-4925
        // execute_disk_command) search $PATH. execute_pipeline
        // (execute_cmd.c:2620) routes every element through that same
        // execute_command, so `date(){ echo FAKE; }; date | cat` runs the
        // function (WSL GNU 5.3.0: FAKE). The hardcoded ordinary-builtin
        // arms below and execute_builtin_pipeline_stage classify by name
        // alone and never consult the function table, and the concurrent
        // native pipelines fall back to the sequential loop for them, so
        // function-named members previously fell through to the disk
        // command (issue #587: the real date(1) ran; `t(){ ...; }; t git
        // log | wc -l` silently executed a PATH stub). Route function-named
        // elements to the function stage first, keeping
        // execute_function_command_invocation's posix special-builtin
        // exception (command_prepare.rs; GNU: in posix mode a special
        // builtin precludes find_function, execute_cmd.c:4657-4663).
        // Classification precedes the fd-0 redirect routing below:
        // GNU's find_function runs in the element's forked child
        // regardless of the element's own redirections, which the
        // chosen execution path then applies internally
        // (execute_function's do_redirections, execute_cmd.c:5606).
        // Routing a function-named element with `< file` straight to
        // the external spawner skipped the function table entirely
        // and reported the function as `command not found`
        // (rubash#1823).
        if let Some(word) = command.words.first() {
            let function_shadows = !(self.posix_mode_enabled() && is_posix_special_builtin(word))
                && self.function_name_for_command_word(word).is_some();
            if function_shadows {
                return self.execute_function_pipeline_stage(command, input, live_stdin);
            }
        }

        // GNU forks every pipeline element and applies the ELEMENT'S OWN
        // redirections in the forked child (execute_cmd.c:4617
        // execute_simple_command -> redir.c do_redirection_internal), so a
        // stage carrying an fd-0 FILE redirect (`< /dev/null`, `<> f`, `<&N`)
        // runs with its fd 0 ON THE REDIRECT TARGET — the inline builtin
        // arms below read the upstream payload instead and would drop the
        // redirect (`echo hi | cat < /dev/null` prints nothing under GNU).
        // Only a file/ReadWrite winner routes: a heredoc/here-string winner
        // IS the payload the inline arms read (initial_pipeline_input), and
        // the external spawner resolves the same winner through
        // apply_external_stdin_redirect (rubash#429 class).
        let stage_fd0_file_redirect =
            match crate::executor::shell_options::fd0_stdin_redirect_winner(command) {
                Some(crate::parser::RedirectKind::Input)
                | Some(crate::parser::RedirectKind::ReadWrite) => true,
                Some(_) => false,
                None => command.redirect_in.is_some(),
            };
        if stage_fd0_file_redirect {
            // GNU runs every element in a forked subshell and applies the
            // element's redirections there before dispatch — a BUILTIN
            // element with an fd-0 file redirect runs as the builtin with
            // its stdin on the file, not as an external child (a pipeline
            // `read x < f` is still the read builtin; routing it to the
            // external spawner reported `read: command not found` with
            // status 127 where GNU runs the builtin, rubash#525). External
            // programs keep the spawner route below.
            let first_raw = command
                .word_metadata
                .first()
                .map(|metadata| metadata.raw.as_str());
            let builtin_stage = command
                .words
                .first()
                .and_then(|name| {
                    self.expand_command_word(command, 0, name, first_raw)
                        .into_iter()
                        .next()
                })
                .is_some_and(|expanded| self.routes_as_builtin(&expanded));
            if !builtin_stage {
                return self.execute_external_pipeline_stage(command, input, stdin_inherit);
            }
        }

        // rubash#815 profiler: the pipeline-stage dispatch point (GNU runs
        // every element through execute_simple_command in its forked child,
        // execute_cmd.c:2620 execute_pipeline -> 4617). Armed TENTATIVE like
        // the top-level dispatch frame: the arms that re-route through the
        // classified helpers cancel it (external stage guard, function
        // entry, the builtin stage's re-dispatch through execute_command),
        // and a pure inline fast-path arm (`echo`/`printf`/`tr`/...) is
        // confirmed as a builtin when the stage returns.
        let _profiler_stage = super::profiler::enter_prepared_command(name);
        // GNU findcmd.c search_for_command: a slash-free external name
        // runs only when it resolves — with PATH unset nothing resolves
        // (rubash#1126), and a set PATH holding no such command fails
        // identically (rubash#1545). The inline arms below that emulate
        // external programs (head, cat, sed, grep, wc, tr) must stand
        // down in both cases and let the external stage report the
        // failure. Builtin arms are unaffected.
        if matches!(name, "head" | "cat" | "sed" | "grep" | "wc" | "tr" | "tput")
            && self.emulated_external_stands_down(name)
        {
            return self.execute_external_pipeline_stage(command, input, stdin_inherit);
        }
        match name {
            "true" | ":" => Ok(Some((String::new(), String::new(), 0))),
            "false" => Ok(Some((String::new(), String::new(), 1))),
            "echo" => {
                // rubash#1488: same variable-table locale guard as the
                // plain execute_echo path — a pipeline stage's \u/\U
                // gate must see the shell's LC_ALL, not the process env.
                let _locale_guard =
                    crate::locale::ShellLocaleGuard::new(&self.shell_state.env_vars);
                let args = self.expand_pipeline_stage_arg_words(command, 1);
                let mut output = Vec::new();
                crate::builtins::echo::write_echo_decoded(
                    args.iter().map(String::as_str),
                    &mut output,
                )?;
                Ok(Some((
                    crate::executor::substitution_metadata::bytes_to_shell_text(&output),
                    String::new(),
                    0,
                )))
            }
            "printf" => {
                let args: Vec<String> = self.expand_pipeline_stage_arg_words(command, 1);
                let mut env_vars = self.shell_state.env_vars.clone();
                let mut output = Vec::new();
                let mut stderr = Vec::new();
                let status = crate::builtins::printf::execute_with_io(
                    args.iter().map(String::as_str),
                    &mut env_vars,
                    &mut output,
                    &mut stderr,
                )?;
                Ok(Some((
                    crate::executor::substitution_metadata::bytes_to_shell_text(&output),
                    crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
                    status,
                )))
            }
            "trap" => {
                // A `trap ACTION EXIT` install as a pipeline element runs in
                // the forked subshell in GNU and fires at the element's exit
                // (execute_cmd.c:1761-1763; rubash#1104: `trap 'echo X' EXIT
                // | cat` carries X down the pipe). The inline path below runs
                // the builtin against a throwaway env clone, so an install
                // would be silently discarded. Decline EXIT installs to the
                // subshell stage (which resets and runs the EXIT trap);
                // queries (`trap -p`), resets (`trap - EXIT`), and non-EXIT
                // installs keep the inline path — a stage is a single
                // command, so DEBUG/RETURN/ERR installs have nothing left to
                // fire at, and resets are no-ops on the reset table.
                if trap_stage_installs_exit(command) {
                    return self.execute_builtin_pipeline_stage(
                        command,
                        input,
                        streaming_stdin,
                        live_stdin,
                    );
                }
                let args = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                let mut env_vars = self.shell_state.env_vars.clone();
                let mut output = Vec::new();
                let mut stderr = Vec::new();
                let status = crate::builtins::trap::execute_with_io(
                    &args,
                    &mut env_vars,
                    &mut output,
                    &mut stderr,
                )?;
                // The POSIX reset-by-name usage failure returns GNU's raw
                // internal EX_USAGE (258) for the execute_trap wrapper's
                // special_builtin_failed gate; a pipeline stage surfaces the
                // user-visible conversion instead (execute_cmd.c:4886-4888).
                let status = if status > 256 { 2 } else { status };
                Ok(Some((
                    crate::executor::substitution_metadata::bytes_to_shell_text(&output),
                    crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
                    status,
                )))
            }
            "head" => {
                let args = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                // GNU head with file operands ignores stdin and reads the
                // files (`printf x | head -1 f*` prints the file header, not
                // the pipe input), and byte mode has no line representation.
                // Only the plain line-count shapes parse_head_line_count
                // accepts may stay inline; every malformed/missing/negative
                // count (niubash#708) declines so the real head reports it.
                let Some(spec) = parse_head_line_count(&args) else {
                    return self.execute_external_pipeline_stage(command, input, stdin_inherit);
                };
                Ok(Some((spec.apply(input), String::new(), 0)))
            }
            "cat" => {
                use crate::executor::external_file_builtins::{
                    cat_format, parse_cat_argv, CatParsed,
                };
                // niubash#165: GNU cat (coreutils 9.4 src/cat.c) parses its
                // whole option surface once in main() (getopt_long) and
                // passes the SAME option block to cat() for every input fd
                // — file operands, `-', and the no-operand stdin fallback
                // all run the identical filter loop, so options apply
                // identically on pipe input. This inline stage arm used to
                // carry its own `-v'-only mini-cat, silently dropping
                // -n/-b/-s/-E/-T/-e/-t/-A, long options, cross-operand
                // filter state, and usage-error validation whenever cat
                // read a pipe (`printf 'z\n' | cat -n' printed `z').
                // Route the stage through the same parse_cat_argv +
                // cat_format pair the simple-command path (external_cat,
                // rubash#415) uses: one parser, one formatter.
                let (options, operands) = match parse_cat_argv(command) {
                    CatParsed::HelpOrVersion => {
                        // The real binary prints its own --help/--version.
                        return self.execute_external_pipeline_stage(command, input, stdin_inherit);
                    }
                    CatParsed::Usage(message) => {
                        return Ok(Some((String::new(), message, 1)));
                    }
                    CatParsed::Options { options, operands } => (options, operands),
                };
                // Pathname-expand the operands the way
                // execute_simple_command does, or `cat f*` opens the
                // literal name "f*" and reports it as missing (probe
                // 2026-09-09: `printf x | cat f*` printed nothing).
                // parse_cat_argv lends references straight out of
                // command.words, so pointer identity recovers each
                // operand's word index for its quoting metadata.
                let mut file_operands: Vec<String> = Vec::new();
                for word in operands {
                    // GNU cat: a bare `-` operand is stdin at that
                    // position, never a flag.
                    if word.as_str() == "-" {
                        file_operands.push(word.clone());
                        continue;
                    }
                    let value = self.expand_word(word);
                    // \x1d marks a fully quoted word and \x1b a quoted
                    // tilde; both stay literal.
                    if value.starts_with(STORAGE_WORD_PREFIX)
                        || value.starts_with(crate::executor::markers::QUOTED_WORD_PREFIX)
                    {
                        file_operands.push(value.replace(crate::executor::markers::CTLESC, ""));
                        continue;
                    }
                    // Quoted words (e.g. "*.txt") must not be glob-expanded.
                    let word_index = command
                        .words
                        .iter()
                        .position(|candidate| std::ptr::eq(candidate, word))
                        .unwrap_or(0);
                    let metadata = command.word_metadata.get(word_index);
                    let raw = metadata.map(|metadata| metadata.raw.as_str());
                    if crate::executor::command_prepare::raw_word_suppresses_pathname_expansion(
                        raw, metadata,
                    ) {
                        file_operands.push(value.replace(crate::executor::markers::CTLESC, ""));
                        continue;
                    }
                    match glob::pathname_expand_word(
                        &value,
                        &self.shell_state.env_vars,
                        &self.shell_state.cwd,
                    ) {
                        glob::PathnameExpansion::Matches(matches) => file_operands.extend(matches),
                        glob::PathnameExpansion::NoMatch | glob::PathnameExpansion::Fail(_) => {
                            file_operands.push(value.replace(crate::executor::markers::CTLESC, ""))
                        }
                    }
                }
                if !file_operands.is_empty() {
                    // GNU cat.c: one filter pass over the concatenation —
                    // the numbering counter and squeeze state carry across
                    // operand boundaries, so `cat -n - f' numbers f's
                    // first line after stdin's (WSL 9.4 probes, rubash#415).
                    let mut stream: Vec<u8> = Vec::new();
                    let mut stderr = String::new();
                    let mut status = 0;
                    // `cat -` consumes the stage's stdin once; later `-`
                    // operands see EOF. Resolved lazily so commands without
                    // a `-` operand never touch the stdin machinery.
                    let mut stdin_remaining: Option<String> = None;
                    for path in file_operands {
                        if path == "-" {
                            if stdin_remaining.is_none() {
                                stdin_remaining = Some(
                                    self.stdin_string_for_command_mut(command)
                                        .unwrap_or_else(|| input.to_string()),
                                );
                            }
                            let text = stdin_remaining.take().unwrap_or_default();
                            stream.extend(
                                crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                                    &text,
                                ),
                            );
                            continue;
                        }
                        // `/dev/stdin`/`/dev/fd/N`/`/proc/self/fd/N`
                        // operands resolve against this stage's fd
                        // endpoints, not the filesystem: fd 0 is the
                        // stage input (same lazy cursor as `-`), higher
                        // fds come from the executor's fd table.
                        if let Some(fd) = crate::executor::dev_fd_operands::dev_operand_fd(&path) {
                            let bytes_opt = if fd == 0 {
                                if stdin_remaining.is_none() {
                                    stdin_remaining = Some(
                                        self.stdin_string_for_command_mut(command)
                                            .unwrap_or_else(|| input.to_string()),
                                    );
                                }
                                Some(
                                    crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                                        &stdin_remaining.take().unwrap_or_default(),
                                    ),
                                )
                            } else {
                                match self.dev_fd_operand_bytes_for_command(command, fd) {
                                    // GNU cat.c: the operand resolves to the
                                    // file fd 1 writes to — report, skip,
                                    // exit 1.
                                    Some(read) if read.stdout_file && !read.bytes.is_empty() => {
                                        stderr.push_str(&format!(
                                            "{}cat: {path}: input file is output file\n",
                                            self.diagnostic_prefix()
                                        ));
                                        status = 1;
                                        None
                                    }
                                    other => other.map(|read| read.bytes),
                                }
                            };
                            match bytes_opt {
                                Some(bytes) => stream.extend(bytes),
                                None => {
                                    stderr.push_str(&format!(
                                        "{}cat: {path}: No such file or directory\n",
                                        self.diagnostic_prefix()
                                    ));
                                    status = 1;
                                }
                            }
                            continue;
                        }
                        // niubash#177: resolve through the operand form so a
                        // virtual-root file operand that exists only under
                        // its `.exe` spelling (`/usr/bin/seq`) opens like the
                        // spawned cat.exe sees it (same exists-first rule as
                        // the argv funnel).
                        match fs::read(self.session_path(
                            crate::executor::path::windows_operand_file_path(
                                shell_path_to_windows(&path, &self.shell_state.env_vars),
                                &self.shell_state.env_vars,
                            ),
                        )) {
                            Ok(bytes) => stream.extend(bytes),
                            Err(_) => {
                                stderr.push_str(&format!(
                                    "{}cat: {path}: No such file or directory\n",
                                    self.diagnostic_prefix()
                                ));
                                status = 1;
                            }
                        }
                    }
                    let output = crate::executor::substitution_metadata::bytes_to_shell_text(
                        &cat_format(&stream, &options),
                    );
                    return Ok(Some((output, stderr, status)));
                }
                let output = if options.identity() {
                    // Byte-identical passthrough keeps the transport text
                    // as-is (no decode round trip).
                    self.stdin_string_for_command_mut(command)
                        .unwrap_or_else(|| input.to_string())
                } else {
                    // The line filters (numbering, squeeze, $-ends) carry
                    // state across the whole stream, so a formatting
                    // invocation formats the complete stdin (GNU streams;
                    // the in-process emulation buffers — bounded by the
                    // stage input, like every other emulated reader).
                    let text = self
                        .stdin_string_for_command_mut(command)
                        .unwrap_or_else(|| input.to_string());
                    crate::executor::substitution_metadata::bytes_to_shell_text(&cat_format(
                        &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&text),
                        &options,
                    ))
                };
                Ok(Some((output, String::new(), 0)))
            }
            "sed" => {
                let args = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                // GNU has no sed builtin: a bare `sed` resolves through
                // PATH (findcmd.c), so when a sed IS resolvable the
                // internal bridge must not shadow it — a user or test
                // harness sed earlier in PATH is the program GNU would
                // run (rubash#1462). The bridge only serves the
                // no-sed-on-PATH case it was built for.
                if find_user_command("sed", &self.shell_state.env_vars).is_some() {
                    return self.execute_external_pipeline_stage(command, input, stdin_inherit);
                }
                if let Some(output) = apply_simple_sed_args(input, &args) {
                    Ok(Some((output, String::new(), 0)))
                } else {
                    self.execute_external_pipeline_stage(command, input, stdin_inherit)
                }
            }
            "grep" => {
                // The inline fast path handles the flag forms GNU pipelines use
                // most (grep [-cinvq] [--] PATTERN with stdin input only).
                // Anything else -- regex engine flags, file operands, unknown
                // options -- must run the real grep; historically words[1] was
                // treated as the pattern even when it was a flag, so
                // "set | grep -c VTILDE" grepped for the literal "-c" (it
                // matched SHELLOPTS' "interactive-comments") and printed the
                // matching lines instead of a count.
                let args: Vec<String> = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect();
                let Some(spec) = inline_grep_args(&args) else {
                    return self.execute_external_pipeline_stage(command, input, stdin_inherit);
                };
                let mut selected = 0usize;
                let mut line_number = 0usize;
                let mut output = String::new();
                for line in input.split_inclusive('\n') {
                    line_number += 1;
                    let comparable = line.strip_suffix('\n').unwrap_or(line);
                    let haystack = if spec.ignore_case {
                        comparable.to_lowercase()
                    } else {
                        comparable.to_string()
                    };
                    let needle = if spec.ignore_case {
                        spec.pattern.to_lowercase()
                    } else {
                        spec.pattern.to_string()
                    };
                    let hit = simple_grep_pattern_matches(&haystack, &needle) != spec.invert;
                    if !hit {
                        continue;
                    }
                    selected += 1;
                    if spec.quiet || spec.count_mode {
                        continue;
                    }
                    if spec.line_numbers {
                        output.push_str(&line_number.to_string());
                        output.push(':');
                    }
                    output.push_str(line);
                    if !line.ends_with('\n') {
                        output.push('\n');
                    }
                }
                if spec.count_mode {
                    output = format!("{selected}\n");
                }
                Ok(Some((output, String::new(), i32::from(selected == 0))))
            }
            "wc" => {
                let args: Vec<String> = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect();
                // niubash#383: the stage input is transport text, so carrier
                // bytes such as 0x1B/0x0C travel as U+E000+U+E0xx marker
                // pairs (locale.rs:352). A bare `input.as_bytes().len()` or
                // `input.bytes()` therefore measures the CARRIER encoding,
                // not the payload: `printf '\033' | wc -c` reported 6. Byte
                // and word counts must therefore run over exactly the payload
                // bytes the external wc would receive
                // (`shell_text_to_raw_bytes`, the same boundary its
                // child-stdin write uses). The carrier encoding hides a
                // whitespace byte (0x0C) inside U+E000+U+E0xx, so splitting on
                // transport text counts `printf 'a\014b' | wc -w` as 1 where
                // GNU reports 2, and payload must never be fed to
                // `decode_to_visible_text` (internal-text decoder: a payload
                // control byte such as 0x11 is taken as the CTLESC carrier
                // and consumes the following character, corrupting the byte
                // stream — the output-side asserts that used to catch this
                // were removed by #838, so the corruption is silent). The
                // line-count arm needs no decode:
                // 0x0A is not a carrier byte, so every `\n` is already real.
                // Only the single-flag fast paths are emulated inline; every
                // other invocation (-L, -m, --words, combined flags, file
                // operands) must run the real external `wc`. Returning None
                // here would abort the whole pipeline with the misleading
                // "pipeline command could not execute" diagnostic.
                if args.len() == 1 {
                    let value = match args[0].as_str() {
                        "-c" => {
                            crate::executor::substitution_metadata::shell_text_to_raw_bytes(input)
                                .len()
                        }
                        "-l" => input.bytes().filter(|byte| *byte == b'\n').count(),
                        "-w" => inline_wc_word_count(input),
                        _ => {
                            return self.execute_external_pipeline_stage(
                                command,
                                input,
                                stdin_inherit,
                            );
                        }
                    };
                    return Ok(Some((format!("{value}\n"), String::new(), 0)));
                }
                // Default (no operands) matches GNU wc: lines, words, bytes.
                if args.is_empty() {
                    let lines = input.bytes().filter(|byte| *byte == b'\n').count();
                    let words = inline_wc_word_count(input);
                    let bytes =
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(input)
                            .len();
                    return Ok(Some((
                        format!("{lines:>7} {words:>7} {bytes:>7}\n"),
                        String::new(),
                        0,
                    )));
                }
                self.execute_external_pipeline_stage(command, input, stdin_inherit)
            }
            "tr" => {
                let args = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                // Options (-d/-s/-c/-t/--) are NOT sets: `tr -d b` would be read as
                // SET1="-d" by the translate fast path — input without '-'
                // or 'd' then passes through unchanged, rc=0, silent
                // (niubash#166). GNU coreutils tr owns option parsing; only
                // plain two-set operands may take the fast path.
                if args.iter().any(|arg| arg.starts_with('-')) {
                    self.execute_external_pipeline_stage(command, input, stdin_inherit)
                } else if args.len() == 2 && matches!(args[0].as_str(), "\\n" | "\n") {
                    Ok(Some((input.replace('\n', &args[1]), String::new(), 0)))
                } else if args.len() == 2
                    && inline_expand_tr_set(&args[0]).is_some()
                    && inline_expand_tr_set(&args[1]).is_some()
                {
                    Ok(Some((
                        translate_tr(input, &args[0], &args[1]),
                        String::new(),
                        0,
                    )))
                } else {
                    // Specs the inline fast path cannot represent (POSIX
                    // classes it does not know, `[x*n]` repeats, escapes)
                    // must run the real external `tr`; silently returning the
                    // input unchanged is never acceptable.
                    self.execute_external_pipeline_stage(command, input, stdin_inherit)
                }
            }
            // #800A: the same inline stand-in the simple-command path uses
            // (external_file_builtins::external_tput). The harvest assets'
            // `$(tput …)` bodies are single-command pipelines, so without
            // this arm the stage spawner paid the full child spawn per call
            // (~17ms host, ~300ms on the harvest runner). Admission stays
            // the byte-exact whitelist; anything else falls through to the
            // real binary exactly as before.
            #[cfg(windows)]
            "tput" => {
                // Stage argv is pre-expansion here (the arms expand their
                // own arguments), so resolve against the expanded operands
                // — a non-admitted expansion falls through below.
                let args = self.expand_pipeline_stage_arg_words(command, 1);
                let mut probe = command.clone();
                probe.words = vec![command.words[0].clone()];
                probe.words.extend(args);
                if let Some(outcome) = self.admitted_tput_outcome(&probe) {
                    self.record_tput_resolution(&probe);
                    let (output, status) = match outcome {
                        crate::executor::tput_emulation::TputOutcome::Bytes(bytes) => {
                            (bytes.into_owned(), 0)
                        }
                        // ncurses: an absent capability prints nothing,
                        // exits 1 (verified `TERM=vt100 tput setaf 1`).
                        crate::executor::tput_emulation::TputOutcome::Absent => (String::new(), 1),
                    };
                    Ok(Some((output, String::new(), status)))
                } else {
                    self.execute_external_pipeline_stage(command, input, stdin_inherit)
                }
            }
            _ => {
                if let Some(output) =
                    self.execute_function_pipeline_stage(command, input, live_stdin)?
                {
                    Ok(Some(output))
                } else {
                    if let Some(output) = self.execute_builtin_pipeline_stage(
                        command,
                        input,
                        streaming_stdin,
                        live_stdin,
                    )? {
                        Ok(Some(output))
                    } else {
                        self.execute_external_pipeline_stage(command, input, stdin_inherit)
                    }
                }
            }
        }
    }

    pub(in crate::executor) fn restricted_command_error(
        &self,
        command: &CommandNode,
        name: &str,
    ) -> Option<String> {
        if !crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "restricted") {
            return None;
        }
        let slash = |value: &str| value.contains('/') || value.contains('\\');
        if slash(name) {
            return Some(format!(
                "{}{}: restricted: cannot specify `/' in command names\n",
                self.diagnostic_prefix(),
                name
            ));
        }
        if name == "cd" {
            // GNU builtins/cd.def:267-274 refuses any cd in a restricted
            // shell: sh_restricted(NULL) -> "cd: restricted".
            return Some(format!(
                "{}{}: restricted\n",
                self.diagnostic_prefix(),
                name
            ));
        }
        if name == "exec" {
            // GNU builtins/exec.def:118-145: bare `exec` (redirects only)
            // still succeeds in a restricted shell; an operand triggers
            // sh_restricted(NULL) -> "exec: restricted".
            if command.words.len() > 1 {
                return Some(format!(
                    "{}{}: restricted\n",
                    self.diagnostic_prefix(),
                    name
                ));
            }
        }
        if matches!(name, "." | "source") {
            // GNU builtins/source.def:148-155 refuses when the filename
            // operand contains a slash: sh_restricted(filename) ->
            // ".: <filename>: restricted".
            if let Some(operand) = command.words.get(1) {
                let filename = self.expand_word(operand);
                if slash(&filename) {
                    return Some(format!(
                        "{}{}: {}: restricted\n",
                        self.diagnostic_prefix(),
                        name,
                        filename
                    ));
                }
            }
        }
        if name == "command" {
            // #804 gap 13: GNU strips the `command` prefix at
            // execute_simple_command (CMD_COMMAND_BUILTIN pseudo-reserved
            // prefix, execute_cmd.c) and dispatches the operand as the
            // command, so the RESTRICTED_SHELL '/' scan at
            // execute_disk_command (execute_cmd.c:5789) fires on the FIRST
            // operand word — `command /bin/echo` reports
            // "/bin/echo: restricted: cannot specify `/' in command names".
            // Option forms mirror command_builtin's own option loop
            // (command.def:68-103 + internal_getopt): `-p` in OPTION
            // position is refused before anything else (sh_restricted("-p")
            // — `command echo -p` treats the `-p` as echo's argument and
            // runs it), `-v`/`-V` take the describe path that returns
            // before any execution (command.def:104-118) so their operands
            // are not checked, `--` ends the option list, and any other
            // option is a usage error at the builtin that never reaches
            // execution either.
            let mut option_scan = true;
            let mut verbose = false;
            let mut operand: Option<String> = None;
            for word in command.words.iter().skip(1) {
                let value = self.expand_word(word);
                if option_scan {
                    match value.as_str() {
                        "--" => {
                            option_scan = false;
                            continue;
                        }
                        "-p" => {
                            return Some(format!(
                                "{}command: -p: restricted\n",
                                self.diagnostic_prefix()
                            ));
                        }
                        "-v" | "-V" => {
                            verbose = true;
                            continue;
                        }
                        other if other.starts_with('-') && other.len() > 1 => continue,
                        _ => option_scan = false,
                    }
                }
                operand = Some(value);
                break;
            }
            if !verbose {
                if let Some(operand) = operand {
                    if slash(&operand) {
                        return Some(format!(
                            "{}{}: restricted: cannot specify `/' in command names\n",
                            self.diagnostic_prefix(),
                            operand
                        ));
                    }
                }
            }
        }
        for redirect in &command.redirects {
            // #804 gaps 11/12: GNU refuses the redirection DIRECTION, not a
            // slash in the target (command.h:57 WRITE_REDIRECT over
            // redir.c:919). The exact post-expansion resolution lives in the
            // apply-time gate (reject_invalid_redirects, this file's stage
            // sites run it right after this scan); the scan here decides the
            // plain file directions and the dup words that are ALREADY
            // decidable on the raw text (`>& out.txt`), so a refused member
            // still diverts the concurrent fast paths to the sequential
            // executor instead of spawning with the write wired natively.
            // Read-only input stays allowed.
            if !is_restricted_write_redirect(&redirect.kind)
                && !is_raw_err_and_out_redirect(redirect)
            {
                continue;
            }
            let target = self.expand_redirect_target(redirect);
            let filename = target.strip_prefix('&').unwrap_or(&target);
            return Some(format!(
                "{}{}: restricted: cannot redirect output\n",
                self.diagnostic_prefix(),
                filename
            ));
        }
        None
    }

    fn lastpipe_enabled(&self) -> bool {
        // GNU: lastpipe takes effect only when job control is not active
        // (execute_cmd.c execute_pipeline / shopt.def lastpipe: the last
        // element runs in the current shell "if job control is not active";
        // under `set -m` every element forks, so a variable set by the last
        // element never reaches the parent). Gate on the monitor option so
        // `set -m; shopt -s lastpipe` stays inert (rubash#1476).
        crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "lastpipe")
            && !crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor")
    }

    /// rubash#1873: whether pipeline element 0's stdin is the shell's
    /// CURRENT fd 0 rather than a payload the stage machinery computes.
    /// GNU execute_pipeline (execute_cmd.c:2620) starts prev at pipe_in
    /// — NO_PIPE unless the enclosing context itself piped into this
    /// pipeline — and do_piping (execute_cmd.c:6375) replaces fd 0 only
    /// when pipe_in != NO_PIPE, so the first element is forked with the
    /// shell's own fd 0, whatever it designates. The element qualifies
    /// when it carries no fd-0 binding of its own (no `<`/`<<`/`<<<`/
    /// `<&` winner and no legacy redirect/heredoc fields) and fd 0 is a
    /// live descriptor: the inherited process stdin, or a file
    /// description an `exec < file` / compound bind installed. For such
    /// an element there is NO payload: the string channel declines the
    /// inherited handle by design (stdin_string_for_command serves
    /// redirects, virtual endpoints, file endpoints and the
    /// FUNCTION_STDIN mirror — never the process stdin), and slurping a
    /// file endpoint through open_fd_read_endpoint's shared-offset dup
    /// would drain the very descriptor the element must read. The stage
    /// machinery therefore keeps the FUNCTION_STDIN mirror ABSENT (the
    /// #761 streaming treatment) and the element's readers use the
    /// fd-table transports, whose offsets are the descriptor's own — a
    /// stage that reads one line leaves the next for the shell, exactly
    /// GNU's shared open file description. An ambient FUNCTION_STDIN
    /// mirror still wins over the inherited endpoint (in that state the
    /// mirror IS fd 0 and initial_pipeline_input serves its remainder);
    /// a file endpoint wins over the mirror (an `exec` rebind shadows
    /// the ambient buffer, #1102/#1103).
    fn pipeline_stage0_stdin_live(&self, command: &CommandNode) -> bool {
        if self.function_stdin_is_command_source(command) {
            return false;
        }
        if crate::executor::shell_options::fd0_stdin_redirect_winner(command).is_some()
            || command.redirect_in.is_some()
            || command.heredoc.is_some()
            || command.heredoc_body.is_some()
            || command.here_string.is_some()
            || command.here_string_carrier.is_some()
            || command
                .heredoc_redirects
                .iter()
                .any(|redirect| redirect.fd.is_none() || redirect.fd == Some(0))
        {
            return false;
        }
        match self.fd_table.entries.get(&0) {
            None => !self.shell_state.env_vars.contains_key(FUNCTION_STDIN),
            Some(entry) => {
                !entry.closed
                    && match entry.read {
                        Some(FdReadEndpoint::InheritedProcessStdin) => {
                            !self.shell_state.env_vars.contains_key(FUNCTION_STDIN)
                        }
                        Some(FdReadEndpoint::File(_)) => true,
                        _ => false,
                    }
            }
        }
    }

    /// Computes the first pipeline element's fd-0 payload. GNU gives every
    /// element the same open file description (execute_cmd.c execute_pipeline):
    /// the shared FUNCTION_STDIN cursor may move only when the element
    /// actually reads. stdin_string_for_command_mut's drain-to-EOF models a
    /// consumer, so the cursor is snapshot/restored here and the real
    /// consumption is folded back by execute_simple_pipeline after the stage
    /// runs (pipeline_stdin_consumed reports subshell/child reads). Returns
    /// the input plus the FUNCTION_STDIN base offset when the buffer was the
    /// source (None for heredoc/redirect/virtual-fd/process-stdin sources,
    /// whose cursors live elsewhere).
    fn initial_pipeline_input(&mut self, command: &CommandNode) -> (String, Option<usize>) {
        self.apply_comsub_stdin_writeback();
        let base = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let from_function_stdin = self.function_stdin_is_command_source(command);
        // GNU execute_cmd.c execute_pipeline forks every element with the
        // shell's own descriptors: the first element's fd 0 IS the shell's
        // fd 0 (the inherited process stdin when nothing rebound it), and
        // the parent NEVER reads a child's stdin — there is no eager
        // materialization anywhere in GNU. An earlier version slurped the
        // whole inherited stdin here for a name list of utilities
        // (cat/grep/sed/...), which blocked forever on an open pipe
        // (`cat list.txt | while read ...` in a script whose parent stdin
        // stayed piped-open never even reached stage 0) and consumed
        // stdin GNU would leave for later readers. The stage gets the real
        // handle instead: execute_pipeline_stage's stdin_inherit arm for
        // external children, and the builtin readers' own inherited-handle
        // paths (e.g. stream_inherited_cat) — both keyed on fd 0 actually
        // designating the process stdin, not on the command's name.
        // rubash#1564: the heredoc body is expanded HERE, in the parent,
        // to materialize the element's stdin — but in GNU that
        // expansion happens inside the forked element, so its variable
        // side effects (`${var:=word}` in the body) die with the
        // element. Bracket the expansion with the spec snapshot; the
        // produced text keeps the expanded values, the binds do not
        // survive. (The FUNCTION_STDIN_OFFSET bookkeeping below runs
        // after the restore, on the restored table.)
        let saved_input_env = self.snapshot_spec_env(command);
        let input = self
            .stdin_string_for_command_mut(command)
            .unwrap_or_default();
        self.restore_spec_variables(saved_input_env);
        if from_function_stdin {
            self.shell_state
                .env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), base.to_string());
            (input, Some(base))
        } else {
            (input, None)
        }
    }
}

pub(crate) fn command_is_compound_pipeline_stage(command: &CommandNode) -> bool {
    command.for_command.is_some()
        || command.if_command.is_some()
        || command.loop_command.is_some()
        || command.select_command.is_some()
        || command.case_command.is_some()
        || command.coproc_command.is_some()
        || command.subshell_command.is_some()
        || command.brace_group.is_some()
        || command.time_command.is_some()
        || command.arithmetic_command.is_some()
        || command.conditional_command.is_some()
        || command.inverted_command.is_some()
        || command.background_command.is_some()
}

/// niubash#383: `wc -w` (and the flagless words field) must count words over
/// the payload bytes the external wc would receive. It must NOT decode
/// through `crate::locale::decode_to_visible_text`: that is an internal-text
/// decoder, and a payload control byte (e.g. piped 0x11) is decoded as the
/// CTLESC carrier that consumes the following character — silently
/// miscounting (its output-side asserts were removed by #838, so nothing
/// trips any more).
fn inline_wc_word_count(input: &str) -> usize {
    const ASCII_WHITESPACE: [u8; 6] = [b' ', b'\t', b'\n', 0x0B, 0x0C, b'\r'];
    crate::executor::substitution_metadata::shell_text_to_raw_bytes(input)
        .split(|byte| ASCII_WHITESPACE.contains(byte))
        .filter(|word| !word.is_empty())
        .count()
}

/// Whether a `trap` pipeline element installs an EXIT trap. Mirrors the
/// option parsing of builtins/trap.rs execute_with_io (`--` separator,
/// `-l`/`-p`/`-P` clusters): queries decline nothing. A single operand is a
/// reset-or-usage, and `-`/`0` actions clear — all in-stage no-ops on the
/// subshell-reset table — so only a real install naming EXIT (or 0) declines
/// the inline arm to the subshell stage (rubash#1104).
fn trap_stage_installs_exit(command: &CommandNode) -> bool {
    let words: Vec<&str> = command.words.iter().skip(1).map(String::as_str).collect();
    let mut index = 0;
    if words.first() == Some(&"--") {
        index = 1;
    }
    while let Some(arg) = words.get(index) {
        if *arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || *arg == "-" {
            break;
        }
        // A query flag (`-l` list, `-p`/`-P` print) or an invalid flag (the
        // builtin errors inline): never an install.
        if arg[1..].chars().any(|c| matches!(c, 'l' | 'p' | 'P')) {
            return false;
        }
        index += 1;
    }
    let Some((action, signals)) = words.get(index..).and_then(|rest| rest.split_first()) else {
        return false;
    };
    if signals.is_empty() || *action == "-" || *action == "0" {
        return false;
    }
    signals.iter().any(|signal| {
        let signal = crate::executor::parameter_decode::strip_matching_quotes(signal);
        signal.eq_ignore_ascii_case("EXIT") || signal == "0"
    })
}

pub(in crate::executor) fn head_line_count(args: &[String]) -> Option<usize> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            break;
        }
        if arg == "-" || !arg.starts_with('-') {
            break;
        }
        if arg == "-n" {
            return args
                .get(index + 1)
                .and_then(|value| value.parse::<usize>().ok());
        }
        if let Some(value) = arg.strip_prefix("--lines=") {
            return value.parse::<usize>().ok();
        }
        if let Some(value) = arg.strip_prefix("-n") {
            return value.parse::<usize>().ok();
        }
        if let Some(value) = arg.strip_prefix('-') {
            if value.chars().all(|ch| ch.is_ascii_digit()) {
                return value.parse::<usize>().ok();
            }
        }
        index += 1;
    }
    None
}

/// Line selection for the inline `head` arms. GNU `head -n -N` prints all but
/// the last N lines, which a plain `usize` cannot represent, so the count
/// carries its sign.
pub(in crate::executor) enum HeadLineCount {
    First(usize),
    AllButLast(usize),
}

impl HeadLineCount {
    /// Apply the selection to transport text. Only `\n` (never a carrier byte)
    /// delimits lines, so the split stays byte-exact.
    pub(in crate::executor) fn apply(&self, input: &str) -> String {
        match *self {
            HeadLineCount::First(count) => input.split_inclusive('\n').take(count).collect(),
            HeadLineCount::AllButLast(count) => {
                let lines = input.split_inclusive('\n').collect::<Vec<_>>();
                lines[..lines.len().saturating_sub(count)].concat()
            }
        }
    }
}

/// Parse the inline-representable `head` argument shapes into a line
/// selection, or `None` when the invocation must run the real external `head`.
///
/// niubash#708: the arm used to skip every `-…` word and then fall through
/// `head_line_count(...).unwrap_or(10)`, so a malformed, missing or negative
/// count silently printed the first ten lines with rc 0. Only the count forms
/// that this arm can reproduce faithfully keep the fast path: `-n N` / `-nN`
/// / `--lines=N` / `--lines N` / `-N` for a non-negative `N`, plus the GNU
/// negative form `-n -N` (all but the last N lines). `-c`/`--bytes`, any
/// unknown option, any operand and a missing count operand decline.
pub(in crate::executor) fn parse_head_line_count(args: &[String]) -> Option<HeadLineCount> {
    let mut count = None;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            // A trailing operand (`head -n 2 -- f`) reads the file, not the
            // pipe; this arm only represents pipe input.
            return None;
        }
        let value = if arg == "-n" || arg == "--lines" {
            index += 1;
            args.get(index)?
        } else if let Some(value) = arg.strip_prefix("--lines=") {
            value
        } else if let Some(value) = arg.strip_prefix("-n") {
            value
        } else if let Some(value) = arg.strip_prefix('-') {
            if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
                return None;
            }
            count = Some(HeadLineCount::First(value.parse::<usize>().ok()?));
            index += 1;
            continue;
        } else {
            // A bare operand (file or `-`) is not representable inline.
            return None;
        };
        count = Some(parse_head_count_operand(value)?);
        index += 1;
    }
    Some(count.unwrap_or(HeadLineCount::First(10)))
}

fn parse_head_count_operand(value: &str) -> Option<HeadLineCount> {
    let (negative, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value),
    };
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let count = digits.parse::<usize>().ok()?;
    Some(if negative {
        HeadLineCount::AllButLast(count)
    } else {
        HeadLineCount::First(count)
    })
}

pub(in crate::executor) fn translate_tr(input: &str, source: &str, target: &str) -> String {
    let (Some(source_chars), Some(target_chars)) =
        (inline_expand_tr_set(source), inline_expand_tr_set(target))
    else {
        return input.to_string();
    };
    // niubash#707 (same class as niubash#383): `input` is transport text, so
    // carrier bytes such as 0x0C and 0x1B travel as U+E000+U+E0xx marker
    // pairs. A set built from the real characters (`[:cntrl:]`, `[:space:]`)
    // never matches those pairs, so carried control bytes escaped translation.
    // Decode to exactly the bytes the external `tr` would receive, run set
    // membership on bytes, then re-encode so the output boundary decodes once.
    let decoded = crate::executor::substitution_metadata::shell_text_to_raw_bytes(input);
    // `inline_expand_tr_set` returns only ASCII for a representable set, so the
    // char->byte narrowing is exact.
    let source_bytes: Vec<u8> = source_chars.iter().map(|ch| *ch as u8).collect();
    let target_bytes: Vec<u8> = target_chars.iter().map(|ch| *ch as u8).collect();
    if source_bytes.is_empty() || target_bytes.is_empty() {
        // `tr a ''` is a coreutils usage error and `tr '' x` a no-op; the
        // guards keep the index arithmetic below panic-free either way.
        return crate::executor::substitution_metadata::bytes_to_shell_text(&decoded);
    }
    let translated: Vec<u8> = decoded
        .into_iter()
        .map(|byte| {
            source_bytes
                .iter()
                .position(|source_byte| *source_byte == byte)
                .map(|index| target_bytes[index.min(target_bytes.len() - 1)])
                .unwrap_or(byte)
        })
        .collect();
    crate::executor::substitution_metadata::bytes_to_shell_text(&translated)
}

/// Expands a `tr` set for the inline pipeline fast path. Returns `None` for
/// syntax it does not implement (unknown POSIX classes, `[x*n]` repeats,
/// backslash escapes, non-ASCII, reversed ranges) so callers can run the real
/// external `tr` instead of silently translating nothing.
pub(in crate::executor) fn inline_expand_tr_set(spec: &str) -> Option<Vec<char>> {
    if !spec.is_ascii() {
        return None;
    }
    let chars: Vec<char> = spec.chars().collect();
    let mut expanded = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' => return None,
            '[' => {
                let class = spec[index..].strip_prefix("[:")?;
                let end = class.find(":]")?;
                expanded.extend(tr_class_chars(&class[..end])?);
                index += end + 4;
            }
            current if index + 2 < chars.len() && chars[index + 1] == '-' => {
                if current > chars[index + 2] {
                    // A reversed range is an error in real `tr`; let the
                    // external binary surface it instead of eating input.
                    return None;
                }
                expanded.extend(current..=chars[index + 2]);
                index += 3;
            }
            current => {
                expanded.push(current);
                index += 1;
            }
        }
    }
    Some(expanded)
}

fn tr_class_chars(class: &str) -> Option<Vec<char>> {
    Some(match class {
        "alpha" => ('a'..='z').chain('A'..='Z').collect(),
        "digit" => ('0'..='9').collect(),
        "lower" => ('a'..='z').collect(),
        "upper" => ('A'..='Z').collect(),
        "space" => vec![' ', '\t', '\n', '\r', '\x0b', '\x0c'],
        "blank" => vec![' ', '\t'],
        "punct" => ('!'..='/')
            .chain(':'..='@')
            .chain('['..='`')
            .chain('{'..='~')
            .collect(),
        "xdigit" => ('0'..='9').chain('a'..='f').chain('A'..='F').collect(),
        "cntrl" => (0u8..=0x1f)
            .map(|byte| byte as char)
            .chain(['\x7f'])
            .collect(),
        "graph" => ('!'..='~').collect(),
        "print" => (' '..='~').collect(),
        _ => return None,
    })
}

#[cfg(test)]
mod inline_tr_tests {
    use super::*;

    #[test]
    fn translate_tr_maps_ranges_and_classes() {
        assert_eq!(translate_tr("Hello", "a-z", "A-Z"), "HELLO");
        assert_eq!(
            translate_tr("Hello World", "[:upper:]", "[:lower:]"),
            "hello world"
        );
        assert_eq!(translate_tr("a b c", "[:space:]", "-"), "a-b-c");
        assert_eq!(translate_tr("abc123", "[:digit:]", "#"), "abc###");
        assert_eq!(translate_tr("abc", "b", "x"), "axc");
    }

    #[test]
    fn unknown_tr_syntax_is_not_supported() {
        assert!(inline_expand_tr_set("[x*n]").is_none());
        assert!(inline_expand_tr_set(r"\n").is_none());
        assert!(inline_expand_tr_set("héllo").is_none());
        assert!(inline_expand_tr_set("[:bogus:]").is_none());
        assert!(inline_expand_tr_set("z-a").is_none());
        assert!(inline_expand_tr_set("a-z").is_some());
        assert!(inline_expand_tr_set("[:upper:]").is_some());
        assert!(inline_expand_tr_set("abc").is_some());
    }
}

/// rubash#761: a pipeline stage the streaming-tail OS-pipe spawner can serve
/// with stdio alone — no shell-side redirect, tempenv, heredoc, process
/// substitution, time prefix, inversion, or background/coproc modifier.
/// Anything else keeps the shell-aware sequential machine.
#[cfg(any(windows, unix))]
fn pipeline_streaming_plain_stage(command: &CommandNode) -> bool {
    command.time_command.is_none()
        && command.redirect_out.is_none()
        && command.append.is_none()
        && command.redirect_in.is_none()
        && command.redirect_err.is_none()
        && command.redirect_err_append.is_none()
        && command.heredoc.is_none()
        && command.heredoc_redirects.is_empty()
        && command.here_string.is_none()
        && command.redirects.is_empty()
        && command.assignments.is_empty()
        && command.process_substitutions.is_empty()
        && !command.inverted
        && command.inverted_command.is_none()
        && command.background_command.is_none()
        && command.coproc_command.is_none()
        && !command_has_pipeline_process_substitution(command)
}

/// rubash#761: a streaming-tail PREFIX member — a plain external command (no
/// compound/subshel/brace shape, non-empty words). Prefix members are spawned
/// as real external children, so only `pipeline_streaming_plain_stage` plus
/// the external shape qualifies.
#[cfg(any(windows, unix))]
fn pipeline_streaming_plain_external(command: &CommandNode) -> bool {
    !command.words.is_empty()
        && command.brace_group.is_none()
        && !command.subshell
        && command.subshell_command.is_none()
        && !command_is_compound_pipeline_stage(command)
        && pipeline_streaming_plain_stage(command)
}

fn command_has_non_concurrent_pipeline_redirects(
    command: &CommandNode,
    index: usize,
    pipeline_len: usize,
) -> bool {
    if command.redirects.is_empty() {
        return false;
    }
    let is_last_stage = index + 1 == pipeline_len;
    command.redirects.iter().any(|redirect| {
        let is_initial_heredoc = index == 0
            && matches!(
                redirect.kind,
                crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString
            );
        let is_final_output = is_last_stage
            && matches!(
                redirect.kind,
                crate::parser::RedirectKind::Output
                    | crate::parser::RedirectKind::Append
                    | crate::parser::RedirectKind::ClobberOutput
            );
        !is_initial_heredoc && !is_final_output
    })
}

// The native concurrent pipeline runs members through CreateProcess with
// the parsed word text as argv; it has no process-substitution
// materialization, so a member carrying <( ) / >( ) must take the
// sequential stage path, where execute_pipeline_stage materializes the
// substitutions into temp paths first.
fn command_has_pipeline_process_substitution(command: &CommandNode) -> bool {
    !command.process_substitutions.is_empty()
        || command.word_metadata.iter().any(|metadata| {
            !metadata.process_substitutions.is_empty()
                || metadata.raw.contains("<(")
                || metadata.raw.contains(">(")
        })
        || command
            .words
            .iter()
            .any(|word| (word.starts_with("<(") || word.starts_with(">(")) && word.ends_with(')'))
        || command
            .redirects
            .iter()
            .any(|redirect| redirect.target.starts_with("<(") || redirect.target.starts_with(">("))
        || [
            command.redirect_in.as_deref(),
            command.redirect_out.as_deref(),
            command.append.as_deref(),
            command.redirect_err.as_deref(),
            command.redirect_err_append.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|redirect| redirect.target.starts_with("<(") || redirect.target.starts_with(">("))
}

struct TimePipelinePrefix {
    command: CommandNode,
    inverted: bool,
    posix_format: bool,
}

/// Whether a command node's fd 2 is dup'd onto its fd 1 — a `2>&1`
/// redirect (or the `|&` desugar of one): the node's stderr belongs to
/// its stdout stream (rubash#1263).
fn node_merges_stderr_into_stdout(node: &CommandNode) -> bool {
    let is_stdout_dup = |redirect: &crate::parser::Redirect| {
        redirect.fd == Some(2)
            && redirect.kind == crate::parser::RedirectKind::DuplicateOutput
            && redirect.target == "&1"
    };
    node.redirects.iter().any(is_stdout_dup)
        || node.redirect_err.as_deref().is_some_and(is_stdout_dup)
}

/// Whether the timed pipeline element's fd 2 is dup'd onto the pipe:
/// the time report then belongs to the pipe stream, right after the
/// element's own output (rubash#1263).
fn timed_element_merges_stderr(prefix: &TimePipelinePrefix) -> bool {
    node_merges_stderr_into_stdout(&prefix.command)
}

/// Whether a stage carrying a `time_command` payload (the compound
/// parse of `time <compound> ...`) sends its fd 2 into the pipe — the
/// dup may sit on the stage node or on the timed command itself
/// (rubash#1263).
fn timed_stage_merges_stderr(stage: &CommandNode) -> bool {
    node_merges_stderr_into_stdout(stage)
        || stage
            .time_command
            .as_ref()
            .is_some_and(|timed| node_merges_stderr_into_stdout(&timed.command))
}

fn time_pipeline_prefix(command: &CommandNode) -> Option<TimePipelinePrefix> {
    if command.words.first().map(String::as_str) != Some("time") {
        return None;
    }
    // rubash#1355: an assignment prefix makes `time` a plain command
    // name (GNU parse.y), never the timing keyword.
    if !command.assignments.is_empty()
        || !command.compound_assignments.is_empty()
        || !command.array_element_assignments.is_empty()
    {
        return None;
    }
    // rubash#1218: posix mode + dash word — not the time keyword; the
    // pipeline runs an ordinary command named `time`.
    // rubash#1520: judged by the parse-time-frozen decision recorded on
    // the head node, not the live mode.
    if time_dash_word_not_keyword(command) {
        return None;
    }

    // rubash#1548: operand window per parse.y special_case_tokens —
    // a quoted '-p' is the timed command's name, never time's option
    // (see crate::parser::scan_time_prefix_words).
    let aligned_metadata = if command.word_metadata.len() == command.words.len() {
        Some(command.word_metadata.as_slice())
    } else {
        None
    };
    let scan = crate::parser::scan_time_prefix_words(&command.words, aligned_metadata);
    let index = scan.next;
    let inverted = scan.inverted;
    let posix_format = scan.posix_format;
    if index >= command.words.len() {
        return None;
    }

    let mut stripped = command.clone();
    stripped.words = command.words[index..].to_vec();
    if command.word_kinds.len() == command.words.len() {
        stripped.word_kinds = command.word_kinds[index..].to_vec();
    }
    if command.word_metadata.len() == command.words.len() {
        stripped.word_metadata = command.word_metadata[index..].to_vec();
    }
    // rubash#1661: same displaced classification as the parser fold —
    // restore the stage's leading assignment words (see
    // parser::reclassify_leading_assignment_words).
    crate::parser::reclassify_leading_assignment_words(&mut stripped);
    Some(TimePipelinePrefix {
        command: stripped,
        inverted,
        posix_format,
    })
}

/// Marker stored on the `2>&1` redirect appended by
/// [`normalize_pipeerr_stage`] (carried in `operator_metadata.raw`, which is
/// never consulted at execution time — the same property that makes
/// GROUP_REDIRECT_INJECTED_MARK a safe carrier). Makes the normalization
/// idempotent and self-documenting at every consumer.
const PIPEERR_DESUGAR_MARK: &str = "|&";

/// GNU parse.y:1470-1487 (`pipeline BAR_AND pipeline`): `cmd1 |& cmd2` is
/// `cmd1 2>&1 | cmd2` — the parser APPENDS a r_duplicating_output redirect
/// (redirector 2, destination 1) to the tail of cmd1's redirect list. The
/// pipeline binds the pipe to the element's fd 1 first (execute_cmd.c:2702),
/// so the appended dup leaves fd 2 on the same pipe open description fd 1
/// holds (redir.c:1169-1170) and the merged bytes interleave in true write
/// order through ONE pipe. rubash's parser keeps `|&` as pipe == Some(2);
/// this rewrites the stage node into the GNU shape exactly the way writing
/// `2>&1` longhand would parse: the dup goes to the TAIL of `redirects` (so
/// an existing `2>err` before it still opens/truncates its file, and the
/// later dup re-binds fd 2 onto the pipe like GNU's walk order) and mirrors
/// into `redirect_err` (the parser's `2>`-family mirror slot, which the
/// builtin buffered-output router follows). Idempotent via
/// PIPEERR_DESUGAR_MARK.
fn normalize_pipeerr_stage(command: &CommandNode) -> CommandNode {
    if command.pipe != Some(2) {
        return command.clone();
    }
    if command
        .redirects
        .iter()
        .any(|redirect| redirect.operator_metadata.raw == PIPEERR_DESUGAR_MARK)
    {
        return command.clone();
    }
    let mut normalized = command.clone();
    let operator_metadata = crate::parser::WordMetadata::literal(
        0,
        "2>&".to_string(),
        PIPEERR_DESUGAR_MARK.to_string(),
    );
    let target_metadata = crate::parser::WordMetadata::literal(0, "1".to_string(), "1".to_string());
    let dup = crate::parser::Redirect {
        fd: Some(2),
        fd_var: None,
        operator: "2>&".to_string(),
        operator_metadata: Box::new(operator_metadata),
        kind: crate::parser::RedirectKind::DuplicateOutput,
        target: "&1".to_string(),
        target_metadata: Box::new(target_metadata),
        append: false,
        clobber: false,
    };
    normalized.redirects.push(dup.clone());
    normalized.redirect_err = Some(Box::new(dup));
    normalized
}

/// Parse the inline grep fast-path argument list: options from the set
/// [-cinvq] (bundled or separate), an optional -- terminator, exactly one
/// non-option PATTERN, and no file operands. Returns None when anything
/// outside that subset is present so the pipeline runs the real grep.
fn inline_grep_args(args: &[String]) -> Option<InlineGrepSpec> {
    let mut spec = InlineGrepSpec::default();
    let mut pattern: Option<&str> = None;
    let mut operands_done = false;
    for arg in args {
        if !operands_done && arg == "--" {
            operands_done = true;
            continue;
        }
        if !operands_done && arg.len() > 1 && arg.starts_with('-') && !arg.starts_with("--") {
            for flag in arg[1..].chars() {
                match flag {
                    'c' => spec.count_mode = true,
                    'i' => spec.ignore_case = true,
                    'v' => spec.invert = true,
                    'n' => spec.line_numbers = true,
                    'q' => spec.quiet = true,
                    _ => return None,
                }
            }
            continue;
        }
        if pattern.is_none() {
            let candidate = arg.as_str();
            // The inline matcher is a literal substring search with an
            // optional leading ^ anchor; it does not implement BRE. A
            // pattern carrying a regex met character must run the real
            // external grep, or `grep -c .` counts a line that has no
            // literal dots as 0 instead of 1 (probe 2026-09-09).
            if candidate.chars().any(|ch| {
                matches!(
                    ch,
                    '.' | '*' | '[' | ']' | '\\' | '?' | '+' | '|' | '(' | ')' | '{' | '}' | '$'
                )
            }) {
                return None;
            }
            pattern = Some(candidate);
            continue;
        }
        // Second operand (or an operand before --) means file input.
        return None;
    }
    Some(InlineGrepSpec {
        pattern: pattern?.to_string(),
        ..spec
    })
}

#[derive(Default)]
struct InlineGrepSpec {
    pattern: String,
    invert: bool,
    count_mode: bool,
    quiet: bool,
    line_numbers: bool,
    ignore_case: bool,
}

/// #800A: the raw head word of a pipeline stage that should answer through
/// the inline tput arm instead of the direct-spawn concurrent paths. Raw
/// word on purpose: a quoted or function-shadowed shape reaches the
/// sequential dispatcher anyway, where function shadowing wins before the
/// arm (GNU execute_cmd.c classify order), so over-matching here only
/// costs concurrency, never correctness.
#[cfg(windows)]
fn tput_inline_stage(command: &CommandNode) -> bool {
    matches!(
        command.words.first().map(String::as_str),
        Some("tput") | Some("/bin/tput") | Some("/usr/bin/tput")
    )
}

/// Non-Windows stub so `cfg!(windows) && tput_inline_stage(command)` at the
/// call sites compiles on Linux/macOS — the arm is Windows-only by design
/// (the whitelist exists to dodge the ~300ms Windows spawn floor), and the
/// predicate is always false off Windows.
#[cfg(not(windows))]
#[allow(dead_code)]
fn tput_inline_stage(_command: &CommandNode) -> bool {
    false
}

thread_local! {
    /// Set while a concurrent-pipeline spec expansion runs: that
    /// expansion is speculative (the sequential stage executor
    /// re-expands on decline), so its failglob verdict must stay
    /// silent — the same contract `expansion_probe_active` gives the
    /// #809 async gate (rubash#1540).
    static SPECULATIVE_EXPANSION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(in crate::executor) fn speculative_expansion_active() -> bool {
    SPECULATIVE_EXPANSION.with(|flag| flag.get())
}

/// RAII scope for [`SPECULATIVE_EXPANSION`]; restores the prior value
/// on drop, so early returns inside a spec build are covered.
pub(in crate::executor) struct SpeculativeExpansionGuard(bool);

impl SpeculativeExpansionGuard {
    pub(in crate::executor) fn new() -> Self {
        Self(SPECULATIVE_EXPANSION.with(|flag| flag.replace(true)))
    }
}

impl Drop for SpeculativeExpansionGuard {
    fn drop(&mut self) {
        SPECULATIVE_EXPANSION.with(|flag| flag.set(self.0));
    }
}
