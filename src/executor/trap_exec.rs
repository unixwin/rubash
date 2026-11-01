use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;

/// rubash#1166: rebase a trap action's parsed line numbers onto the line
/// the trap fired at. GNU parses the action string with line_number
/// starting at the triggering command's line (trap.c _run_trap_internal
/// via parse_and_execute), so EVERY node in the action — nested bodies
/// included — reports trigger_line + (its action-relative line − 1). The
/// old per-runner stamping rewrote only the top-level nodes' `line`,
/// leaving nested commands (a `return` inside an `if` in the action) at
/// their action-relative lines, so diagnostics cited the trap's own
/// definition line instead of the triggering line. `delta` is signed
/// because the DEBUG action cache reuses one parse across firings at
/// different lines.
pub(crate) fn shift_trap_action_lines(commands: &mut [CommandNode], delta: i64) {
    if delta == 0 {
        return;
    }
    fn shift_slot(slot: &mut Option<usize>, delta: i64) {
        if let Some(value) = slot {
            *value = (*value as i64 + delta).max(1) as usize;
        }
    }
    fn shift_node(node: &mut CommandNode, delta: i64) {
        shift_slot(&mut node.line, delta);
        shift_slot(&mut node.logical_line, delta);
        shift_slot(&mut node.end_line, delta);
        shift_slot(&mut node.heredoc_gather_line, delta);
        if let Some(pipeline) = &mut node.pipeline_command {
            shift_trap_action_lines(&mut pipeline.stages, delta);
        }
        if let Some(list) = &mut node.and_or_list {
            shift_trap_action_lines(&mut list.commands, delta);
        }
        if let Some(wrapper) = &mut node.time_command {
            shift_node(&mut wrapper.command, delta);
        }
        if let Some(wrapper) = &mut node.background_command {
            shift_node(&mut wrapper.command, delta);
        }
        if let Some(wrapper) = &mut node.inverted_command {
            shift_node(&mut wrapper.command, delta);
        }
        if let Some(for_command) = &mut node.for_command {
            shift_trap_action_lines(&mut for_command.body, delta);
        }
        if let Some(if_command) = &mut node.if_command {
            shift_trap_action_lines(&mut if_command.condition, delta);
            shift_trap_action_lines(&mut if_command.then_body, delta);
            for branch in &mut if_command.elif_branches {
                shift_trap_action_lines(&mut branch.condition, delta);
                shift_trap_action_lines(&mut branch.body, delta);
            }
            if let Some(else_body) = &mut if_command.else_body {
                shift_trap_action_lines(else_body, delta);
            }
        }
        if let Some(loop_command) = &mut node.loop_command {
            shift_trap_action_lines(&mut loop_command.condition, delta);
            shift_trap_action_lines(&mut loop_command.body, delta);
        }
        if let Some(subshell) = &mut node.subshell_command {
            shift_trap_action_lines(&mut subshell.body, delta);
        }
        if let Some(case_command) = &mut node.case_command {
            for clause in &mut case_command.clauses {
                shift_trap_action_lines(&mut clause.body, delta);
            }
        }
        if let Some(select_command) = &mut node.select_command {
            shift_trap_action_lines(&mut select_command.body, delta);
        }
        if let Some(function) = &mut node.function_command {
            shift_trap_action_lines(&mut function.body, delta);
        }
        if let Some(group) = &mut node.brace_group {
            shift_trap_action_lines(&mut group.body, delta);
        }
        if let Some(coproc) = &mut node.coproc_command {
            if let Some(body) = &mut coproc.body {
                shift_trap_action_lines(body, delta);
            }
        }
    }
    for command in commands {
        shift_node(command, delta);
    }
}

/// The stderr destination for an `exec` spawn's diagnostics — and, on
/// the execute_with_io arms, for the child's own piped stderr
/// (rubash#2395). GNU has exactly one fd 2: execute_cmd.c:4244/4268
/// performs the command's redirections, then exec_builtin
/// (builtins/exec.def:231) hands the image the shell's descriptors as
/// bound. When the executing executor's fd-2 stream is being collected
/// (a pipeline stage's or substitution's clone arms `stderr_capture`
/// so the driver can route the bytes through the stage's own
/// redirections), bytes written straight to the process stderr bypass
/// both the capture and that routing — `printf '' | exec bash -c
/// 'echo e >&2' 2>/dev/null` printed `e` to the terminal. With no
/// capture armed this is the process stderr, the historical
/// destination, unchanged.
enum ExecDiagStderr<'a> {
    Capture(&'a mut Vec<u8>),
    Process(std::io::Stderr),
}

impl std::io::Write for ExecDiagStderr<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            ExecDiagStderr::Capture(capture) => capture.write(buf),
            ExecDiagStderr::Process(stderr) => stderr.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            ExecDiagStderr::Capture(capture) => capture.flush(),
            ExecDiagStderr::Process(stderr) => stderr.flush(),
        }
    }
}

/// Run `body` with the exec spawn's stderr destination bound (see
/// ExecDiagStderr). The capture slot is taken out for the call so the
/// destination can borrow the buffer while the body also borrows the
/// rest of the executor, then restored — including on the body's
/// error path, so a failed spawn cannot orphan the capture.
fn with_exec_diag_stderr<R>(
    capture_slot: &mut Option<Vec<u8>>,
    body: impl FnOnce(&mut ExecDiagStderr<'_>) -> Result<R, ExecuteError>,
) -> Result<R, ExecuteError> {
    let mut capture = capture_slot.take();
    let result = {
        let mut target = match capture.as_mut() {
            Some(buffer) => ExecDiagStderr::Capture(buffer),
            None => ExecDiagStderr::Process(std::io::stderr()),
        };
        body(&mut target)
    };
    *capture_slot = capture;
    result
}

impl Executor {
    pub(in crate::executor) fn set_fd_input_text(&mut self, fd: u32, input: String, dynamic: bool) {
        // input is shell text: decode marker pairs back to the raw bytes
        // the fd byte channel carries (set_fd_input_bytes re-encodes for
        // the text env mirror, so into_bytes() here would double-encode).
        self.set_fd_input_bytes(
            fd,
            crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input),
            dynamic,
        );
    }

    pub(in crate::executor) fn set_fd_input_bytes(
        &mut self,
        fd: u32,
        input: Vec<u8>,
        dynamic: bool,
    ) {
        self.fd_table
            .open_input(fd, FdReadEndpoint::bytes(input.clone()), dynamic);
        self.shell_state.env_vars.insert(
            fd_stdin_key(fd),
            crate::executor::substitution_metadata::bytes_to_shell_text(&input),
        );
        self.shell_state
            .env_vars
            .insert(fd_stdin_offset_key(fd), "0".to_string());
        if dynamic {
            self.shell_state
                .env_vars
                .insert(fd_dynamic_input_key(fd), "1".to_string());
        } else {
            self.shell_state.env_vars.remove(&fd_dynamic_input_key(fd));
        }
        self.shell_state.env_vars.remove(&fd_closed_key(fd));
    }

    /// `exec N<file` — install a real kernel handle in the slot instead of
    /// snapshotting file contents. The endpoint's Rc is shared by every
    /// `N<&M` duplicate and by subshell table clones, so the file offset is
    /// shared exactly like GNU's open file description (redir.c dup2 /
    /// execute_cmd.c execute_in_subshell fd inheritance).
    pub(in crate::executor) fn set_fd_input_file(
        &mut self,
        fd: u32,
        file: Rc<FileFd>,
        dynamic: bool,
    ) {
        self.fd_table
            .open_input(fd, FdReadEndpoint::File(file), dynamic);
        self.shell_state.env_vars.remove(&fd_closed_key(fd));
        self.shell_state.env_vars.remove(&fd_stdin_key(fd));
        self.shell_state.env_vars.remove(&fd_stdin_offset_key(fd));
        if dynamic {
            self.shell_state
                .env_vars
                .insert(fd_dynamic_input_key(fd), "1".to_string());
        } else {
            self.shell_state.env_vars.remove(&fd_dynamic_input_key(fd));
        }
    }

    /// `exec N<directory` reached through a synthetic link
    /// (rubash#1937): GNU's O_RDONLY open of a directory succeeds —
    /// only the READ fails (EISDIR). Windows cannot open a directory
    /// handle at all, so the bind installs the Directory endpoint and
    /// the read side fabricates the failure (see
    /// FdReadEndpoint::Directory). Bookkeeping mirrors
    /// set_fd_input_file.
    pub(in crate::executor) fn set_fd_input_directory(&mut self, fd: u32, dynamic: bool) {
        self.fd_table
            .open_input(fd, FdReadEndpoint::Directory, dynamic);
        self.shell_state.env_vars.remove(&fd_closed_key(fd));
        self.shell_state.env_vars.remove(&fd_stdin_key(fd));
        self.shell_state.env_vars.remove(&fd_stdin_offset_key(fd));
        if dynamic {
            self.shell_state
                .env_vars
                .insert(fd_dynamic_input_key(fd), "1".to_string());
        } else {
            self.shell_state.env_vars.remove(&fd_dynamic_input_key(fd));
        }
    }

    /// `[N]<>file` — one O_RDWR handle feeding both directions of the slot
    /// (GNU redir.c r_input_output opens a single descriptor).
    pub(in crate::executor) fn set_fd_readwrite_file(
        &mut self,
        fd: u32,
        target: &str,
        dynamic: bool,
    ) -> std::io::Result<()> {
        // rubash#1026: `N<>/dev/tcp/host/port` binds the connected
        // socket (one descriptor, both directions — redir.c netopen).
        if crate::special_paths::is_net_redirect_target(target) {
            // Raw error: every caller of this function applies
            // posix_errors::path_error itself.
            let file = self.open_net_redirect_raw(target)?;
            let file_fd = FileFd::from_file(file, target.into());
            self.set_fd_input_file(fd, file_fd.clone(), dynamic);
            self.fd_table
                .open_output(fd, FdWriteEndpoint::File(file_fd), dynamic);
            self.shell_state.env_vars.remove(&fd_closed_key(fd));
            self.shell_state
                .env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.shell_state
                .env_vars
                .insert(fd_output_key(fd), target.to_string());
            return Ok(());
        }
        // rubash#1898: the zero/full devices bind directly — reads
        // come from the pump-fed zero source; /dev/zero writes go to
        // the null sink; /dev/full writes bind the FullDevice
        // endpoint, whose faces fail ENOSPC through sh_chkwrite
        // exactly like the direct redirect faces. The entropy
        // devices (niubash#758) bind the same way: reads from the
        // pump-fed CSPRNG source, writes to the null sink.
        if crate::special_paths::is_zero_device(target)
            || crate::special_paths::is_full_device(target)
            || crate::special_paths::is_random_device(target)
        {
            let full = crate::special_paths::is_full_device(target);
            let read_file = if crate::special_paths::is_random_device(target) {
                Self::random_device_file(&target)?
            } else {
                Self::zero_device_file(&target)?
            };
            let read_fd = FileFd::from_file(read_file, target.into());
            self.set_fd_input_file(fd, read_fd, dynamic);
            if full {
                self.fd_table
                    .open_output(fd, FdWriteEndpoint::FullDevice, dynamic);
            } else {
                let write_file = self.create_redirect_output("/dev/null", true)?;
                let write_fd = FileFd::from_file(write_file, target.into());
                self.fd_table
                    .open_output(fd, FdWriteEndpoint::File(write_fd), dynamic);
            }
            self.shell_state.env_vars.remove(&fd_closed_key(fd));
            self.shell_state
                .env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.shell_state
                .env_vars
                .insert(fd_output_key(fd), target.to_string());
            return Ok(());
        }
        // rubash#1935: synthetic /proc content — the entry's write
        // class decides the open, the same answer the probe ladder
        // gives: the 0444 class fails the open EACCES (redir.c
        // r_input_output on the real entry); the 0644 class binds —
        // reads serve the generated snapshot through the byte
        // channel, writes fail EINVAL through the symbolic
        // ProcContentWrite endpoint (rubash#2042: GNU's procfs 0644
        // open succeeds and its writes fail EINVAL; the absorbed
        // sink made them silently succeed).
        if let Some(allowed) = crate::special_paths::synthetic_proc_write_allowed(target) {
            if !allowed {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Permission denied",
                ));
            }
            let bytes = crate::special_paths::synthetic_proc_content(target).unwrap_or_default();
            self.set_fd_input_bytes(fd, bytes, dynamic);
            self.fd_table
                .open_output(fd, FdWriteEndpoint::ProcContentWrite, dynamic);
            self.shell_state.env_vars.remove(&fd_closed_key(fd));
            self.shell_state
                .env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.shell_state
                .env_vars
                .insert(fd_output_key(fd), target.to_string());
            return Ok(());
        }
        // rubash#1963: `<> /dev/fd/N` opens the alias through the
        // kernel on GNU — a closed alias fails the open ENOENT.
        // rubash#2256: an open one whose endpoint is a regular file
        // re-opens the target fresh (below); every other endpoint
        // binds a dup of the descriptor in both directions.
        if let Some(alias_fd) = crate::special_paths::fd_alias_target(target) {
            if !super::redirection::dup_source_fd_open(
                &self.fd_table,
                alias_fd,
                &std::collections::HashMap::new(),
            ) {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "No such file or directory",
                ));
            }
            // rubash#2276: a socket endpoint's spelling-open fails
            // ENXIO (see fd_alias_spelling_open_error) — the dup pair
            // below is the pipe rule, not the socket rule.
            if let Some(error) = self.fd_alias_spelling_open_error(alias_fd) {
                return Err(error);
            }
            if let Some(path) = self.fd_alias_reopen_path(alias_fd) {
                // One O_RDWR description of the target serves both
                // directions (the open_readwrite_pair contract); the
                // source fd's offset does not move.
                let (read_file, write_file) = FileFd::open_readwrite_pair(path)?;
                self.set_fd_input_file(fd, read_file, dynamic);
                self.fd_table
                    .open_output(fd, FdWriteEndpoint::File(write_file), dynamic);
                self.shell_state.env_vars.remove(&fd_closed_key(fd));
                self.shell_state
                    .env_vars
                    .remove(&fd_output_process_substitution_key(fd));
                self.shell_state
                    .env_vars
                    .insert(fd_output_key(fd), target.to_string());
                return Ok(());
            }
            let _ = self.fd_table.dup_input(fd, alias_fd);
            let _ = self.fd_table.dup_output(fd, alias_fd);
            // rubash#2385: `<>` of a pipe read end is ONE O_RDWR
            // description of the pipe in GNU — the dup pair binds the
            // read direction, but the write direction is the pipe's
            // OTHER end, which no dup can copy out of a read-only
            // descriptor. When the pair left the slot without a write
            // endpoint, the complementary resolver's endpoint is the
            // write direction.
            if self.fd_table.output_endpoint(fd).is_none() {
                if let Some(endpoint) = self.fd_alias_complementary_write_endpoint(alias_fd) {
                    self.fd_table.open_output(fd, endpoint, dynamic);
                }
            }
            self.shell_state.env_vars.remove(&fd_closed_key(fd));
            self.shell_state
                .env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.shell_state
                .env_vars
                .insert(fd_output_key(fd), target.to_string());
            return Ok(());
        }
        // SyntheticLink (rubash#1900): the open goes to the resolved
        // native target, as the input arms bind it — the pair split
        // serves `<>`; a directory target fails the pair open, like
        // GNU's O_RDWR open of a directory fails.
        if let Some(link) = {
            let (cwd_forms, root_forms) = self.proc_link_forms();
            crate::special_paths::synthetic_link_target(target, Some(&cwd_forms), Some(&root_forms))
        } {
            let (read_file, write_file) =
                FileFd::open_readwrite_pair(std::path::PathBuf::from(&link.native))?;
            self.set_fd_input_file(fd, read_file, dynamic);
            self.fd_table
                .open_output(fd, FdWriteEndpoint::File(write_file), dynamic);
            self.shell_state.env_vars.remove(&fd_closed_key(fd));
            self.shell_state
                .env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.shell_state
                .env_vars
                .insert(fd_output_key(fd), target.to_string());
            return Ok(());
        }
        // G-02 session-cwd anchor (rubash#1887): GNU redir.c:725
        // redir_open opens in the shell process, whose only cwd is the
        // shell's cwd; in this port's split model every session open
        // anchors at ShellState.cwd via session_path (see its doc), or
        // a relative operand resolves against the process cwd and the
        // bind misses files the shell itself just created after `cd`.
        let path = self.session_path(shell_path_to_windows(target, &self.shell_state.env_vars));
        // open_readwrite_pair splits `<>CON` into its CONIN$/CONOUT$
        // halves (niubash#519: CON rejects a read+write handle).
        let (read_file, write_file) = FileFd::open_readwrite_pair(path)?;
        self.set_fd_input_file(fd, read_file, dynamic);
        self.fd_table
            .open_output(fd, FdWriteEndpoint::File(write_file), dynamic);
        self.shell_state.env_vars.remove(&fd_closed_key(fd));
        self.shell_state
            .env_vars
            .remove(&fd_output_process_substitution_key(fd));
        self.shell_state
            .env_vars
            .insert(fd_output_key(fd), target.to_string());
        Ok(())
    }

    /// `exec N>file` / `N>>file` — hold the real write handle on the slot.
    /// `append` selects FILE_APPEND_DATA (GNU O_APPEND); truncation /
    /// noclobber checks must have run already (create_redirect_output).
    pub(in crate::executor) fn set_fd_output_file(
        &mut self,
        fd: u32,
        target: String,
        dynamic: bool,
        append: bool,
    ) -> std::io::Result<()> {
        // rubash#1026: `N>/dev/tcp/host/port` binds the connected
        // socket, not a filesystem path (redir.c netopen).
        if crate::special_paths::is_net_redirect_target(&target) {
            let file = self.open_net_redirect(&target)?;
            let file_fd = FileFd::from_file(file, target.clone().into());
            self.fd_table
                .open_output(fd, FdWriteEndpoint::File(file_fd), dynamic);
            self.shell_state.env_vars.remove(&fd_closed_key(fd));
            self.shell_state
                .env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.shell_state.env_vars.insert(fd_output_key(fd), target);
            return Ok(());
        }
        // rubash#1898: device write binds — /dev/zero discards
        // through the null sink; /dev/full binds the FullDevice
        // endpoint (its writes fail ENOSPC through sh_chkwrite).
        // The entropy devices discard writes through the same null
        // sink (niubash#758).
        if crate::special_paths::is_zero_device(&target)
            || crate::special_paths::is_full_device(&target)
            || crate::special_paths::is_random_device(&target)
        {
            if crate::special_paths::is_full_device(&target) {
                self.fd_table
                    .open_output(fd, FdWriteEndpoint::FullDevice, dynamic);
            } else {
                let file = self.create_redirect_output("/dev/null", true)?;
                let file_fd = FileFd::from_file(file, target.clone().into());
                self.fd_table
                    .open_output(fd, FdWriteEndpoint::File(file_fd), dynamic);
            }
            self.shell_state.env_vars.remove(&fd_closed_key(fd));
            self.shell_state
                .env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.shell_state.env_vars.insert(fd_output_key(fd), target);
            return Ok(());
        }
        // rubash#1935 + rubash#2042: synthetic /proc content — the
        // 0444 class fails the open EACCES; the 0644 class binds the
        // symbolic ProcContentWrite endpoint (GNU's procfs open
        // succeeds and its writes fail EINVAL; the fall-through below
        // would create a phantom enclave file at the literal path and
        // its writes would silently succeed).
        if let Some(allowed) = crate::special_paths::synthetic_proc_write_allowed(&target) {
            if !allowed {
                return Err(crate::posix_errors::path_error(
                    &target,
                    io::Error::new(io::ErrorKind::PermissionDenied, "Permission denied"),
                ));
            }
            self.fd_table
                .open_output(fd, FdWriteEndpoint::ProcContentWrite, dynamic);
            self.shell_state.env_vars.remove(&fd_closed_key(fd));
            self.shell_state
                .env_vars
                .remove(&fd_output_process_substitution_key(fd));
            self.shell_state.env_vars.insert(fd_output_key(fd), target);
            return Ok(());
        }
        // G-02 session-cwd anchor (rubash#1887): see set_fd_readwrite_file.
        let path = self.session_path(shell_path_to_windows(&target, &self.shell_state.env_vars));
        let file = FileFd::open_write(path, append, false)
            .map_err(|e| crate::posix_errors::path_error(&target, e))?;
        self.fd_table
            .open_output(fd, FdWriteEndpoint::File(file), dynamic);
        self.shell_state.env_vars.remove(&fd_closed_key(fd));
        self.shell_state
            .env_vars
            .remove(&fd_output_process_substitution_key(fd));
        self.shell_state.env_vars.insert(fd_output_key(fd), target);
        Ok(())
    }

    /// GNU `r_err_and_out`/`r_append_err_and_out` (redir.c:832-838,
    /// do_redirection_internal): `>&word`/`&>word` open the file once and
    /// dup2 it onto fd 2 — both descriptors refer to the same open file
    /// description and share one file offset. Opening the path twice
    /// gives two offsets, so the second write overwrites the first at
    /// offset 0 (redir4.sub: `exec >&file; echo to stdout; echo to
    /// stderr >&2` must produce both lines, not just stderr's).
    pub(in crate::executor) fn share_fd_output_file(
        &mut self,
        source_fd: u32,
        fd: u32,
        target: &str,
        dynamic: bool,
    ) {
        if let Some(endpoint) = self.fd_table.write_endpoint(source_fd) {
            self.fd_table.open_output(fd, endpoint, dynamic);
        }
        self.shell_state.env_vars.remove(&fd_closed_key(fd));
        self.shell_state
            .env_vars
            .remove(&fd_output_process_substitution_key(fd));
        self.shell_state
            .env_vars
            .insert(fd_output_key(fd), target.to_string());
    }

    pub(in crate::executor) fn execute_eval(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let args = cmd.words[1..].to_vec();
        match crate::builtins::eval::execute_with_io(
            args.iter().map(String::as_str),
            &self.diagnostic_prefix(),
            &mut stdout,
            &mut stderr,
        )? {
            crate::builtins::eval::EvalAction::Complete(status) => {
                self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
                self.exit_code = status;
                // rubash#2117: eval's option faces (an invalid option,
                // --help) return EX_USAGE (builtins/eval.def:
                // CASE_HELPOPT / the invalidopt return) — a special
                // builtin's usage error marks special_builtin_failed
                // (execute_cmd.c:4886-4888). The Complete arm's status
                // vocabulary is only success or this face; statuses of
                // the executed string ride the Execute arm and never
                // mark it (an inner command's plain 2 is not > EX_SHERRBASE).
                if status == crate::builtins::eval::EX_USAGE {
                    self.special_builtin_failed.set(true);
                }
                Ok(())
            }
            crate::builtins::eval::EvalAction::Execute(source) => {
                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                let source = eval_source_for_reparse(&source);
                // GNU parse.y: EOF inside a matched-pair construct of the
                // eval string reports "unexpected EOF while looking for
                // matching `X'" naming the innermost close delimiter
                // (eval3.sub: `eval 'x() { _;}>_[${' -> "eval: line N:
                // unexpected EOF while looking for matching `}'"). The
                // probe runs on the raw eval text: GNU's parser sees the
                // string as written, before alias expansion splices bodies
                // (which can consume `${ $() }' text and hide the error).
                // Heredoc bodies are raw text, so inputs carrying `<<` skip
                // this probe like the script driver does.
                if !source.contains("<<") {
                    // rubash#318/#333: an unclosed `(' in the eval string is
                    // NOT the generic close-char shape. GNU re-parses the
                    // string as fresh parser input continuing the caller's
                    // line counter (evalstring.c:345-357), and the failure
                    // class decides both the diagnostic and the abort:
                    //
                    // 1. A token error inside a comsub body (`$(echo (`)
                    //    fails the comsub's own yyparse and parse.y:4588-
                    //    4596 jumps FORCE_EOF — exit 1, nothing after the
                    //    eval, no `command' gate.
                    // 2. A token error inside a compound-assignment list
                    //    (`x=((`) dies the same way in posix mode
                    //    (parse.y:7173-7179, exit 1) and is otherwise
                    //    contained with eval status 1 (DISCARD arm).
                    // 3. Everything else — an unclosed comsub at EOF
                    //    (parse.y:4576-4587 returns matched_pair_error
                    //    without jumping), a plain subshell at the main
                    //    level, or a main-level token error — fails the
                    //    MAIN yyparse into evalstring.c:584-600. That arm
                    //    only exits when posixly_correct AND the builtin is
                    //    eval/source AND NOT under `command'
                    //    (executing_command_builtin, execute_cmd.c:4735):
                    //    ERREXIT exit 2 before the || branch runs. In every
                    //    other case the error is CONTAINED to the eval
                    //    string — push_stream/pop_stream clear EOF_Reached
                    //    per stream (parse.y:1908/1915), so it can never
                    //    stop the outer script's reader by itself.
                    if let Some((shape, open_line)) =
                        crate::lexer::unclosed_comsub_eof_shape(&source)
                    {
                        let caller_line: usize = self
                            .shell_state
                            .env_vars
                            .get("__RUBASH_CURRENT_LINE")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(1);
                        // evalstring.c:359-451: complete commands before the
                        // failing construct's line already ran.
                        if open_line > 1 {
                            let prefix = source
                                .lines()
                                .take(open_line - 1)
                                .collect::<Vec<_>>()
                                .join("\n");
                            if !prefix.trim().is_empty() {
                                self.execute_eval_source(&prefix, caller_line, cmd)?;
                            }
                        }
                        let saved_eval_context = self
                            .shell_state
                            .env_vars
                            .insert("__RUBASH_EVAL_CONTEXT".to_string(), "1".to_string());
                        let diagnostic = match &shape {
                            crate::lexer::UnclosedComsubShape::ComsubBodyTokenError {
                                line,
                                token,
                            } => {
                                // parse.y:6858-6865 with shell_eof_token==')'
                                // (parse.y:4519): the paren shifted as a
                                // function-definition candidate, the NEXT
                                // token named, plus the offending source
                                // line, all at the paren's line.
                                let report = caller_line + line - 1;
                                let prefix = self.parser_error_prefix_for_line(report);
                                let offending =
                                    source.lines().nth(line - 1).unwrap_or_default().to_string();
                                format!(
                                    "{prefix}syntax error near unexpected token `{token}' while looking for matching `)'\n{prefix}`{offending}'\n"
                                )
                            }
                            crate::lexer::UnclosedComsubShape::ComsubEof { line } => format!(
                                "{}unexpected EOF while looking for matching `)'\n",
                                self.parser_error_prefix_for_line(caller_line + line)
                            ),
                            crate::lexer::UnclosedComsubShape::ArithEof { line } => format!(
                                // parse.y:3908-3915 parse_matched_pair:
                                // the arithmetic scan's EOF names its
                                // start_lineno — the `$((` open line,
                                // mapped like the other open-line reports
                                // (caller_line + line - 1).
                                "{}unexpected EOF while looking for matching `)'\n",
                                self.parser_error_prefix_for_line(caller_line + line - 1)
                            ),
                            crate::lexer::UnclosedComsubShape::PlainSubshellEof {
                                line,
                                open_line,
                            } => {
                                // y.tab.c:9258-9260: the bash-5.3 compoundcmd
                                // EOF report — a plain `(` leaves
                                // shell_eof_token == 0, so yyerror's EOF
                                // branch names the construct instead of a
                                // close delimiter. Reported at one past the
                                // last input line; the `('s line is in the
                                // message text.
                                format!(
                                    "{}syntax error: unexpected end of file from `(' command on line {}\n",
                                    self.parser_error_prefix_for_line(caller_line + line),
                                    caller_line + open_line - 1
                                )
                            }
                            crate::lexer::UnclosedComsubShape::MainLevelTokenError {
                                line,
                                token,
                            } => {
                                // y.tab.c:9208-9215 yyerror branch 1 with
                                // shell_eof_token == 0: same token naming as
                                // the comsub class but WITHOUT the
                                // "while looking for matching" suffix.
                                let report = caller_line + line - 1;
                                let prefix = self.parser_error_prefix_for_line(report);
                                let offending =
                                    source.lines().nth(line - 1).unwrap_or_default().to_string();
                                format!(
                                    "{prefix}syntax error near unexpected token `{token}'\n{prefix}`{offending}'\n"
                                )
                            }
                            crate::lexer::UnclosedComsubShape::CompassignTokenError { line } => {
                                // parse.y:7141-7151: the compound-assignment
                                // word list rejects the `(' itself.
                                let report = caller_line + line - 1;
                                let prefix = self.parser_error_prefix_for_line(report);
                                let offending =
                                    source.lines().nth(line - 1).unwrap_or_default().to_string();
                                format!(
                                    "{prefix}syntax error near unexpected token `('\n{prefix}`{offending}'\n"
                                )
                            }
                        };
                        let write_result =
                            self.write_buffered_builtin_output(cmd, &[], diagnostic.as_bytes());
                        match saved_eval_context {
                            Some(previous) => {
                                self.shell_state
                                    .env_vars
                                    .insert("__RUBASH_EVAL_CONTEXT".to_string(), previous);
                            }
                            None => {
                                self.shell_state.env_vars.remove("__RUBASH_EVAL_CONTEXT");
                            }
                        }
                        write_result?;
                        let posix_mode =
                            self.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
                        let interactive = self.invocation_interactive();
                        // evalstring.c:588-595: the ONLY exit for a
                        // main-grammar parse error in the eval'd string —
                        // non-interactive + posix + bare eval/source (not
                        // under `command`). ERREXIT unwinds before the ||
                        // branch runs (GNU probes p3/p5a/p8a/p17/p18: rc 2,
                        // empty stdout).
                        let posix_errexit =
                            posix_mode && !interactive && self.command_builtin_depth == 0;
                        match shape {
                            // parse.y:4588-4596: the comsub body's own
                            // yyparse failed (r != 0) — FORCE_EOF exits 1
                            // with no `command' gate (GNU probes p12a-c:
                            // `command eval '$(echo ('` dies rc 1 too). The
                            // status is stored like exit.def does so the
                            // grouped reader's break keeps it as the shell's
                            // exit status.
                            crate::lexer::UnclosedComsubShape::ComsubBodyTokenError { .. } => {
                                self.exit_code = 1;
                                return Err(ExecuteError::ExitCode(1));
                            }
                            // parse.y:7173-7179: a token error inside a
                            // compound assignment — posix dies rc 1 with no
                            // `command' gate (GNU probe p24), otherwise the
                            // DISCARD arm contains it with status 1.
                            crate::lexer::UnclosedComsubShape::CompassignTokenError { .. } => {
                                // error.c:324-327: a parser_error printed
                                // while errexit is live exits 2 before the
                                // FORCE_EOF/DISCARD decision.
                                if self.errexit_enabled() && self.errexit_is_active() {
                                    self.exit_code = 2;
                                    return Err(ExecuteError::ExitCode(2));
                                }
                                if posix_mode && !interactive {
                                    self.exit_code = 1;
                                    return Err(ExecuteError::ExitCode(1));
                                }
                                self.exit_code = 1;
                                return Ok(());
                            }
                            // Main-grammar parse errors: contained to the
                            // eval string (never a reader abort — pop_stream
                            // clears EOF_Reached, parse.y:1915), except the
                            // posix ERREXIT gate above. eval returns 2
                            // (EX_BADUSAGE, evalstring.c:586).
                            _ => {
                                if self.errexit_enabled() && self.errexit_is_active() {
                                    self.exit_code = 2;
                                    return Err(ExecuteError::ExitCode(2));
                                }
                                if posix_errexit {
                                    self.exit_code = 2;
                                    return Err(ExecuteError::ExitCode(2));
                                }
                                self.exit_code = 2;
                                return Ok(());
                            }
                        }
                    }
                    let eval_posix = self.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
                    if let Some(unclosed) =
                        crate::lexer::unclosed_input_close_char_posix(&source, eval_posix)
                    {
                        let (close, open_line, eof_line, report_open, array_list) = (
                            unclosed.close,
                            unclosed.open_line,
                            unclosed.eof_line,
                            unclosed.report_open,
                            unclosed.array_list,
                        );
                        // GNU eval continues the caller's line numbering:
                        // eval-input line i sits at caller_line+i-1;
                        // quote/arithmetic constructs report their open line,
                        // while `}'/`)' report the end-of-input line
                        // (parse.y:3912 start_lineno vs 6891 line_number).
                        let caller_line: usize = self
                            .shell_state
                            .env_vars
                            .get("__RUBASH_CURRENT_LINE")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(1);
                        // GNU's eval reader executes the complete input lines
                        // before the line where the unclosed construct
                        // opened; that line itself is part of the failed
                        // parse and runs nothing.
                        if open_line > 1 {
                            let prefix = source
                                .lines()
                                .take(open_line - 1)
                                .collect::<Vec<_>>()
                                .join("\n");
                            if !prefix.trim().is_empty() {
                                self.execute_eval_source(&prefix, caller_line, cmd)?;
                            }
                        }
                        let internal = if report_open {
                            open_line.saturating_sub(1)
                        } else {
                            eof_line
                        };
                        let saved_eval_context = self
                            .shell_state
                            .env_vars
                            .insert("__RUBASH_EVAL_CONTEXT".to_string(), "1".to_string());
                        // GNU eval_builtin binds the builtin's redirections
                        // before parse_and_execute reads the string
                        // (redir.c do_redirection_internal), so the EOF
                        // diagnostic honors eval's own `2>/dev/null` — route
                        // it through the command's fd-2 endpoint instead of a
                        // raw eprintln that bypasses every redirect
                        // (rubash#167 family, seen from rubash#190).
                        let diagnostic = format!(
                            "{}unexpected EOF while looking for matching `{close}'\n",
                            self.parser_error_prefix_for_line(caller_line + internal)
                        );
                        self.write_buffered_builtin_output(cmd, &[], diagnostic.as_bytes())?;
                        match saved_eval_context {
                            Some(previous) => {
                                self.shell_state
                                    .env_vars
                                    .insert("__RUBASH_EVAL_CONTEXT".to_string(), previous);
                            }
                            None => {
                                self.shell_state.env_vars.remove("__RUBASH_EVAL_CONTEXT");
                            }
                        }
                        // GNU error.c:324-327: this single-line diagnostic is
                        // also a parser_error, so with live errexit the shell
                        // exit_shell(2)s right after it — even inside a
                        // function body — instead of returning eval's rc=2.
                        if self.errexit_enabled() && self.errexit_is_active() {
                            self.exit_code = 2;
                            return Err(ExecuteError::ExitCode(2));
                        }
                        // rubash#318: an unclosed `name=(` compound list is
                        // the parse_compound_assignment EOF family — eval
                        // returns its exit 1 (verified vs WSL GNU 5.3.0:
                        // `eval "x=("` -> rc 1), like the script-level path
                        // (parse.y:7140-7152). In posix mode the failure is
                        // fatal instead: parse.y:7173-7179 jumps FORCE_EOF
                        // with status 1 and NO `command'-builtin gate (GNU
                        // probe p13/p24: `command eval 'x=('` posix dies
                        // rc 1, nothing after the eval runs), while
                        // non-posix keeps the DISCARD containment (p6b).
                        if array_list && eval_posix {
                            self.exit_code = 1;
                            return Err(ExecuteError::ExitCode(1));
                        }
                        self.exit_code = if array_list { 1 } else { 2 };
                        return Ok(());
                    }
                }
                // GNU parse.y re-reads the eval string as parser input, so
                // alias expansion applies at command position
                // (parse.y alias_expand_token / push_string; comsub21.sub
                // `eval my_alias` inside a substitution body expands here).
                let source = self.comsub_body_alias_splice(&source);
                // GNU eval continues the caller's line numbering: eval-input
                // line i sits at script line caller_line+i-1, and EOF inside
                // the input reports at one past the last input line.
                let caller_line: usize = self
                    .shell_state
                    .env_vars
                    .get("__RUBASH_CURRENT_LINE")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1);
                self.execute_eval_source(&source, caller_line, cmd)
            }
        }
    }

    /// Reparse and execute one eval-source string with the caller's line
    /// numbering (GNU variables.c:parse_and_execute SEVAL flags; eval-input
    /// line i sits at script line caller_line+i-1).
    ///
    /// rubash#817: the string is its own read unit — evalstring.c
    /// parse_and_execute pushes it as fresh parser input — so alias
    /// definitions inside record a fresh read-generation (enter_read_unit)
    /// and an eval-body line that renumbers onto the caller's physical
    /// line N must not collide with a use at the caller's line N (GNU
    /// expands there: the definition was read in the eval string's stream,
    /// earlier than the caller's token). The generation drops when the
    /// string's execution returns, on every path.
    // pub(in crate::executor): the mapfile -C callback path
    // (mapfile_helpers.rs) runs its command string through this same
    // parse_and_execute unit (rubash#1586).
    pub(in crate::executor) fn execute_eval_source(
        &mut self,
        source: &str,
        caller_line: usize,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let saved_read_generation = self.enter_read_unit();
        // rubash#1563: the string runs through GNU's parse_and_execute
        // (evalstring.c) — push its frame (the DISCARD fold catch, and
        // the CMD_IGNORE_RETURN errexit-clearing frame when this eval
        // was itself dispatched from an exempt position) for the body's
        // duration, restored on every path below.
        let saved_frame = self.enter_parse_and_execute_frame();
        let result = self.execute_eval_source_inner(source, caller_line, cmd);
        self.exit_parse_and_execute_frame(saved_frame);
        self.exit_read_unit(saved_read_generation);
        // rubash#846: eval runs its string as a fresh frame (GNU evalstring
        // parse_and_execute) — an inner compound's own redirect failure
        // arrives here as plain status, never flipped. Discard the mark;
        // the caller judges by status only.
        self.compound_redirect_failed.set(false);
        result
    }

    fn execute_eval_source_inner(
        &mut self,
        source: &str,
        caller_line: usize,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // GNU evalstring.c parse_and_execute parses the string with the
        // CURRENT extglob_flag (builtins/shopt.def:640 propagates the
        // option to the parser when the builtin RUNS). The lexer global
        // is a lex-time simulation for the script stream: the feeder
        // flips it when it LEXES a `shopt -s/-u extglob` line, which can
        // be lines ahead of execution — an eval running before that
        // shopt line executes would see the gate flipped early and
        // accept extglob patterns GNU rejects (rubash#1114). Pin the
        // gate to the live runtime shopt state for this reparse
        // (tokenize + parse; the case parser consults the global at
        // parse time) and restore the stream value afterwards — a shopt
        // executed BY the eval'd code re-pins it through set_option.
        let saved_extglob = crate::lexer::parse_extended_glob();
        crate::lexer::set_parse_extended_glob(crate::builtins::shopt::option_enabled(
            &self.shell_state.env_vars,
            "extglob",
        ));
        // rubash#1520: eval's string is a fresh reader unit parsed at
        // this moment (evalstring.c parse_and_execute), so its initial
        // parse mode is the live one — the plain tokenize() default
        // (posix off) would freeze parse-time verdicts (e.g. the `time`
        // dash-word rule, parse.y:3286) under the wrong mode.
        let mut tokens =
            crate::lexer::tokenize_with_initial_posix(source, self.posix_mode_enabled());
        // GNU eval reports errors with the caller line numbering:
        // the string lines continue the script line counter
        // (posix2.tests: "eval: line 199: syntax error ...").
        if caller_line > 1 {
            for token in tokens.iter_mut() {
                token.position += caller_line - 1;
            }
        }
        let mut ast = crate::parser::parse_with_options(
            &tokens,
            crate::parser::ParseLoopOptions {
                stray_close_is_error: true,
                diagnostic_text: None,
                source_text: Some(source.into()),
                source_line_offset: caller_line.saturating_sub(1),
            },
        );
        crate::lexer::set_parse_extended_glob(saved_extglob);
        self.apply_command_output_redirects(cmd, &mut ast)?;
        crate::script_driver::apply_parse_error_unit_voiding(&mut ast);
        // GNU builtins/evalstring.c:351 parse_and_execute: last_result starts
        // at EXECUTION_SUCCESS, so a string that parses to zero commands
        // (empty, blank, or comment-only) returns 0 instead of preserving $?
        // (rubash#1013: `false; eval ""; echo $?` -> 0).
        if ast.commands.is_empty() {
            self.exit_code = 0;
        }
        // A syntax error inside the eval string makes eval return 2;
        // it does not abort the calling script the way a top-level
        // parse error does. An inner exit still exits the shell, so
        // only the parse-error marker (status 2) becomes a status.
        let has_parse_error = ast
            .commands
            .iter()
            .any(|command| command.has_assignment("__RUBASH_PARSE_ERROR__"));
        // rubash#1013: an eval string that parses to no commands at all
        // (empty, blanks, or comments only) succeeds — GNU evalstring.c
        // parse_and_execute on an empty string leaves eval_builtin
        // returning EXECUTION_SUCCESS, so the caller's $? must not
        // survive it (`false; eval ""; echo $?` -> 0).
        let no_commands = ast.commands.is_empty();
        let saved_eval_context = self
            .shell_state
            .env_vars
            .insert("__RUBASH_EVAL_CONTEXT".to_string(), "1".to_string());
        // eval re-reads its string as fresh parser input; the alias
        // expansion GNU applies there (parse.y alias_expand_token)
        // already ran on `source` above via comsub_body_alias_splice,
        // so executor-level expansion must not fire a second time.
        let saved_alias_streamed = self.mark_alias_streamed();
        // GNU eval_builtin runs the reparsed body with eval's own
        // redirections still bound (redir.c do_redirection_internal for the
        // builtin's duration), so a body's `1>&3` resolves fd 3 to eval's
        // `3>&1`, not the leaf's own fd 1.
        let saved_eval_list_depth = self
            .eval_list_depth
            .replace(Some(self.evalerror_exec_depth.get() + 1));
        // GNU builtins/evalstring.c:348: parse_and_execute raises
        // indirection_level around the eval'd list, so under `set -x`
        // the body's traces render one PS4 level deeper than the eval
        // line itself (`++ echo ...`) — rubash#1341.
        let result = self.with_compound_output_redirects(cmd, |executor| {
            executor.with_xtrace_indirection(|executor| executor.execute_ast(&ast))
        });
        self.eval_list_depth.set(saved_eval_list_depth);

        self.resume_alias_streamed(saved_alias_streamed);
        match saved_eval_context {
            Some(previous) => {
                self.shell_state
                    .env_vars
                    .insert("__RUBASH_EVAL_CONTEXT".to_string(), previous);
            }
            None => {
                self.shell_state.env_vars.remove("__RUBASH_EVAL_CONTEXT");
            }
        }
        // GNU evalstring.c:579-599 (parse_and_execute parse-failure arm):
        // a syntax error in the string being evaluated only `break`s the
        // parse_and_execute loop and RETURNS EX_BADUSAGE to the builtin;
        // `should_jump_to_top_level` fires solely when `posixly_correct`
        // (evalstring.c:583-589). The eval'd parse-error diagnosis sets the
        // executor's reader-abort flag (command_execute mark_parse_error on
        // the __RUBASH_PARSE_ERROR__ node) on its way up; take it now so the
        // containment below cannot leave it set — otherwise the script
        // driver treats the eval'd parse error as a top-level syntax error
        // and aborts the whole script instead of continuing with eval's
        // rc=2 (rubash#306: `eval "echo >"` must not kill the caller).
        // The flag also covers parse errors diagnosed in nested layers the
        // top-level AST marker cannot see (comsub bodies inside the eval
        // string, ast_exec last_command_substitution_parse_error), which
        // GNU's single yyparse rejects wholesale before running anything.
        let parse_error_raised_during_eval = self.take_parse_error();
        // GNU error.c:324-327: a parse-error diagnostic printed while errexit
        // was live already exit_shell(2)ed inside parser_error — before
        // parse_and_execute's containment could see it. Such an error is a
        // real shell exit, not eval's rc=2.
        let parser_error_errexited = self.parser_error_errexited.take();
        let eval_result = match result {
            Err(ExecuteError::ExitCode(code))
                if code == 2
                    && !parser_error_errexited
                    && (has_parse_error || parse_error_raised_during_eval) =>
            {
                self.exit_code = code;
                // rubash#2126: GNU builtins/evalstring.c:589-596 — a
                // syntax error (EX_BADSYNTAX) out of parse_and_execute
                // in a posixly_correct non-interactive shell whose most
                // recent dispatch is eval/source itself is NOT contained
                // by the eval boundary: jump_to_top_level (ERREXIT) with
                // EX_BADUSAGE. The containment arms own every other
                // shape (non-POSIX mode, `command eval` via the depth
                // gate inside evalstring_syntax_error_exits, a string
                // whose own commands already dispatched something else).
                if self.evalstring_syntax_error_exits() {
                    Err(ExecuteError::ExitCode(2))
                } else {
                    Ok(())
                }
            }
            // Same containment when an inner boundary already absorbed the
            // error and execute_ast came back Ok with the flag set: GNU's
            // parse-failure arm still breaks the eval loop with
            // EX_BADUSAGE, never the caller's reader.
            Ok(()) if parse_error_raised_during_eval && !parser_error_errexited => {
                self.exit_code = 2;
                // rubash#2126: same evalstring.c:589-596 conversion for
                // the surface where the driver swallowed the abort into
                // an Ok result (parse unit voided after a good prefix).
                if self.evalstring_syntax_error_exits() {
                    Err(ExecuteError::ExitCode(2))
                } else {
                    Ok(())
                }
            }
            // GNU evalstring.c:372 installs a local top_level catch for the
            // string being evaluated ("prevents errors in substitution from
            // restarting the reader loop directly"); its DISCARD arm
            // (evalstring.c:423-450) runs the pe_dispose cleanup, forces
            // last_command_exit_value = EXECUTION_FAILURE, and — outside a
            // subshell — RETURNS from parse_and_execute instead of
            // unwinding further. A bad substitution (or any DISCARD-class
            // word-expansion failure) inside `eval '...'` therefore ends
            // only the eval'd string: eval yields the failure status and
            // the CALLER's remaining commands run — inside a function body
            // `f() { eval 'x=${.sh.version}'; return 5; }` reaches the
            // `return 5` (rubash#188), and at script top level the next
            // line runs. The flat-subshell boundary in the eval'd list
            // absorbs its own ExpansionFailure before it can reach here,
            // matching GNU's forked-subshell re-jump.
            Err(ExecuteError::ExpansionFailure(code)) => {
                // rubash#1563: in a subshell environment GNU's
                // parse_and_execute DISCARD arm does NOT return — it
                // folds the status and re-jumps to the enclosing catch
                // (evalstring.c:427-450 should_jump_to_top_level), so
                // the enclosing body (comsub body, `( )` region) is
                // abandoned with the folded status instead of
                // continuing past the eval. The class cleanup still
                // runs (pe_dispose), then the Err keeps unwinding.
                if self.in_subshell_environment() {
                    self.absorb_failglob_discard();
                    return Err(ExecuteError::ExpansionFailure(code));
                }
                self.exit_code = code;
                // rubash#753: the escaped Err can only be a failglob /
                // redirect-no-match discard raised where the eval'd list's
                // own dispatch could not consume it (a function body called
                // from the string keeps function_depth > 0, so its frame
                // absorbed the Err instead). GNU's evalstring DISCARD arm
                // contains it here — drop the class flag (and the abandon
                // it armed) so the caller's reader loop does not also skip
                // its same-line commands (`f() { eval 'set -- nope*'; };
                // f; echo after` prints `after`). Other classes keep their
                // established escaping behavior (rubash#637/#639).
                self.absorb_failglob_discard();
                Ok(())
            }
            // GNU evalstring.c:365,422-442: a DISCARD-class word-expansion
            // failure (failglob / redirect no-match, rubash#753) inside the
            // eval'd string is caught at the eval boundary — eval returns
            // the failure status and the CALLER's remaining commands run
            // (`eval 'set -- /no/such/*.zzz'; echo K` prints K, verified
            // GNU 5.3). The same-line skip inside the string already ran at
            // the eval'd list level (`eval_list_depth` makes that list the
            // discard boundary); this arm owns the trailing case — the
            // failing command was the string's last, so the armed abandon
            // survived the list — and must not leak into the caller's
            // reader loop. The exit code is left alone: the failglob site
            // already set 1, and a later line that ran fine yields eval rc
            // 0, GNU's parse_and_execute loop semantics.
            Ok(()) if self.absorb_failglob_discard() => Ok(()),
            Ok(()) if self.evalerror_subscript_pending.replace(false) => {
                // GNU evalstring.c:365,422-442: DISCARD inside the eval'd
                // string is caught at the eval boundary — eval returns
                // failure and the caller continues (rubash#637). Absorb
                // exactly the bad-subscript class (armed alongside the
                // abort); arithmetic and nounset failures keep escaping,
                // and a real Err below propagates untouched. The exit code
                // is left alone: it already holds the last status (a later
                // line that ran fine yields eval rc 0, GNU's
                // parse_and_execute loop semantics).
                self.evalerror_pending.set(false);
                self.evalerror_line.set(None);
                Ok(())
            }
            other => {
                // A real error outranks containment: drop a subscript mark
                // the eval did not absorb so an outer eval cannot mistake
                // it for its own string's failure. Plain Ok results never
                // reach here with the mark set (the absorb arm above owns
                // Ok+marked), so an ancestor eval's mark survives a nested
                // successful eval untouched.
                if other.is_err() {
                    self.evalerror_subscript_pending.set(false);
                }
                // Not a contained parse error (e.g. a real `exit`): the
                // parse-error flag raised inside the eval'd string keeps its
                // abort meaning for the enclosing reader.
                if parse_error_raised_during_eval {
                    self.mark_parse_error();
                }
                other
            }
        };
        // rubash#1013: see `no_commands` above — an empty eval string
        // that ran without error reports success, not the stale status.
        if eval_result.is_ok() && no_commands {
            self.exit_code = 0;
        }
        eval_result
    }

    pub fn run_exit_trap(&mut self) -> Result<i32, ExecuteError> {
        self.run_exit_trap_for_status(self.exit_code)
    }

    pub fn run_exit_trap_with_status(&mut self, exit_status: i32) -> Result<i32, ExecuteError> {
        self.run_exit_trap_for_status(exit_status)
    }

    pub(in crate::executor) fn run_exit_trap_for_status(
        &mut self,
        exit_status: i32,
    ) -> Result<i32, ExecuteError> {
        self.run_exit_trap_for_status_with_output_redirects(exit_status, None)
    }

    pub(in crate::executor) fn run_exit_trap_for_status_with_output_redirects(
        &mut self,
        exit_status: i32,
        redirect_cmd: Option<&CommandNode>,
    ) -> Result<i32, ExecuteError> {
        // rubash#1012 (GNU builtins/exec.def exec_builtin ->
        // execute_cmd.c:6126 shell_execve): a successful `exec command`
        // replaces the shell image with no exit path, and trap.c:1440
        // reset_or_restore_signal_handlers cleared SIG_TRAPPED for the exit
        // trap before a failing shell_execve — neither runs the EXIT trap.
        // The exec-replacement unwind carries the exec_replaced latch.
        if self.exec_replaced {
            return Ok(exit_status);
        }
        // rubash#2212: the EXIT trap runs once per exit (see the
        // exit_trap_ran field) — a later funnel entry for the same
        // exit must not fire an action the first handler re-registered.
        if self.exit_trap_ran.get() {
            return Ok(exit_status);
        }
        let Some(action) = crate::builtins::trap::take_exit_trap(&mut self.shell_state.env_vars)
        else {
            return Ok(exit_status);
        };
        if action.is_empty() {
            return Ok(exit_status);
        }
        self.exit_trap_ran.set(true);

        self.exit_code = exit_status;
        // GNU trap.c:1128 run_exit_trap saves the exiting status in
        // trap_saved_exit_value before the action runs; a bare `exit`
        // inside the EXIT action exits with that saved status
        // (builtins/common.c get_exitstat, interp 1602). The exit path
        // wipes the function/source nesting before the trap fires
        // (execute_cmd.c:6090), and a bare `exit` inside a FUNCTION called
        // by the EXIT action still exits with the saved status (GNU probes
        // 2026-10-12: entry $? 5, `false` then bare `exit` -> 5, both at
        // the action's own level and inside a called function) — so the
        // context guard is open at every depth here, unlike ERR/RETURN
        // (where a called function's bare exit uses the current $?
        // instead; the C-level nesting bookkeeping behind the difference
        // is not settled by the vendored source, so this cell is pinned
        // behaviorally).
        let prev_saved_exit = std::mem::replace(&mut self.trap_saved_exit_value, Some(exit_status));
        let prev_saved_context = std::mem::replace(&mut self.trap_saved_exit_context, usize::MAX);
        // A trap action is fresh parser input (parse.y alias_expand_token
        // applies at its read); expand at stream level and mark the batch
        // so executor-level expansion does not fire a second time.
        let action = self.comsub_body_alias_splice(&action);
        let tokens = crate::lexer::tokenize(&action);
        let mut ast = crate::parser::parse(&tokens);
        if let Some(redirect_cmd) = redirect_cmd {
            self.apply_inherited_command_output_redirects(redirect_cmd, &mut ast)?;
        }
        let saved_trap_command = self.shell_state.debug_trap_command.borrow().clone();
        let has_command = self.shell_state.debug_trap_command.borrow().is_none();
        if has_command {
            *self.shell_state.debug_trap_command.borrow_mut() = self
                .shell_state
                .env_vars
                .get("__RUBASH_LAST_COMMAND")
                .or_else(|| self.shell_state.env_vars.get("__RUBASH_CURRENT_COMMAND"))
                .cloned();
        }
        let saved_alias_streamed = self.mark_alias_streamed();
        // GNU trap.c run_exit_trap runs the action while the subshell's
        // redirections are still in force — execute_in_subshell never undoes
        // the child's do_redirections before run_exit_trap
        // (execute_cmd.c:1576-1763). The caller runs this inside the scoped
        // fd-table binding (with_command_input_redirects), so the action's
        // output rides the same open file description as the body — no
        // re-binding here, which would re-open `>file` and truncate what
        // the body already wrote.
        // rubash#2212: the action runs under the running-trap umbrella
        // (GNU's `running_trap != 0` at every run_debug_trap call
        // site), so commands inside the EXIT action take no DEBUG
        // firing; debug_trap_running is the umbrella bit the funnel
        // consults.
        let saved_debug_trap_running = std::mem::replace(&mut self.debug_trap_running, true);
        let result = self.with_xtrace_indirection(|executor| executor.execute_ast(&ast));
        self.debug_trap_running = saved_debug_trap_running;
        self.resume_alias_streamed(saved_alias_streamed);
        *self.shell_state.debug_trap_command.borrow_mut() = saved_trap_command;
        self.trap_saved_exit_value = prev_saved_exit;
        self.trap_saved_exit_context = prev_saved_context;
        match result {
            Ok(()) => {
                self.exit_code = exit_status;
                Ok(exit_status)
            }
            Err(ExecuteError::ExitCode(code)) => {
                self.exit_code = code;
                Ok(code)
            }
            Err(error) => Err(error),
        }
    }

    /// Runs the DEBUG trap action before a command, mirroring Bash's
    /// per-command debug hook. Nested executions of the trap action are
    /// suppressed (Bash does not re-enter the DEBUG trap while an action
    /// is running). `command_text` is the text of the command about to run,
    /// exposed to the trap action through BASH_COMMAND like Bash does.
    ///
    /// Returns `Some(trap_status)` when the pending unit must be SKIPPED —
    /// GNU's fire-site contract (execute_cmd.c:3057/3191/3532/3672/3924/
    /// 4157/4510/5391): `debugging_mode && retval != EXECUTION_SUCCESS`,
    /// i.e. only under extdebug and with ANY non-zero trap status. rubash's
    /// old rule (status == 2 with no extdebug gate) failed both directions:
    /// a status-1 trap under extdebug never skipped, and a status-2 trap
    /// skipped without extdebug (issue #521). `$?` is preserved across the
    /// firing (probe 2026-10-12: `trap 'false' DEBUG; true; echo $?` prints
    /// 0); the action's status reaches the fire site through this return
    /// value only.
    pub(crate) fn run_debug_trap(
        &mut self,
        seam: crate::debugger::SeamKind,
        command_text: &str,
    ) -> Result<Option<i32>, ExecuteError> {
        // GNU gates every run_debug_trap call site on
        // `signal_in_progress (DEBUG_TRAP) == 0 && running_trap == 0`
        // (execute_cmd.c:704/3047/3519/...): the DEBUG trap never
        // fires for commands executed inside a trap action. The
        // EXIT window holds debug_trap_running itself (see
        // run_exit_trap_for_status), which is the half rubash#2212
        // proved missing: the EXIT handler's own first command took
        // a DEBUG firing GNU never makes. The ERR/RETURN/signal
        // windows are deliberately NOT consulted here — the
        // post-ERR re-presentation seams (the DEBUG firing GNU
        // makes for the failing command as its failure propagates
        // to the caller) run inside this executor's ERR bookkeeping
        // window, and gating on error_trap_running silences them
        // (probe rDbg: GNU fires DEBUG between the inner and outer
        // ERR firings; a gated funnel drops it).
        if self.debug_trap_running || self.host_internal_depth.get() > 0 {
            return Ok(None);
        }
        // niubash#681 (design 4.2/9.1): the consult below decides
        // about THIS seam, so its location facts are snapshotted at
        // funnel arrival — before the user's trap action runs. The
        // action executes in full ahead of the consult (the design's
        // order), and an action that calls a function leaves the
        // ambient line stamp at that function's definition line on
        // exit (execute_function's deliberate restamp for the RETURN
        // trap, function_calls.rs). A consult that re-read the
        // ambient stamp after the action would adjudicate this
        // seam's breakpoints against the trap function's definition
        // line: line breakpoints silently never match, or replay at
        // the stale line wearing this seam's command text. GNU
        // establishes the seam's line in the caller before
        // run_debug_trap (execute_cmd.c) and runs the action inside
        // a saved/restored parser state (trap.c _run_trap_internal),
        // so the post-action line state is the seam's by
        // construction; the snapshot is the same guarantee here.
        // Taken only while a session is armed (the gate load the
        // funnel already pays), so the debugger-off path is
        // unchanged.
        let seam_facts = if crate::debugger::SESSION_ACTIVE.is_armed() {
            Some(self.debugger_location())
        } else {
            None
        };
        // WP-3 (design 4.4; WP-1 gap G-02; 150-c REPORT section 5
        // note 1): the session gate is consulted BEFORE the
        // trap-action lookup below, so an attached session is asked
        // on every funnel arrival — including the no-action early
        // return — while a disarmed gate costs one atomic load and
        // the funnel behaves exactly as before. The first arrival
        // serves the entry stop (note 4).
        if crate::debugger::SESSION_ACTIVE.is_armed() {
            self.debugger_serve_entry_stop()?;
        }
        self.debug_trap_running = true;
        // rubash#375: borrowing, allocation-free consult (GNU trap.c:1154
        // reads trap_list[sig] once; the old path built a BTreeSet from the
        // reset list and cloned the action string per firing).
        let Some(action) =
            crate::builtins::trap::active_trap_action_ref(&self.shell_state.env_vars, "DEBUG")
        else {
            // G-03 (WP-1): the no-action path is still a seam
            // arrival — the consult is asked here too (150-c
            // REPORT section 5, note 1).
            if crate::debugger::SESSION_ACTIVE.is_armed() {
                // The re-entrancy flag stays up for the consult
                // itself (its condition evaluations rely on it),
                // but it comes down before any error propagates:
                // a forced return (design 6.1) unwinds out of the
                // consult with the run still alive in the caller,
                // and a stuck flag would silence every later seam
                // of this executor.
                let consult_result = self.debugger_consult(seam, command_text, seam_facts);
                self.debug_trap_running = false;
                consult_result?;
                return Ok(None);
            }
            self.debug_trap_running = false;
            return Ok(None);
        };
        let saved_exit_code = self.exit_code;
        // rubash#546 (cell 2): GNU get_bash_command (variables.c:1558) reads
        // the_printed_command_except_trap, which the printer only refreshes
        // while NO trap action is executing. Inside an ERR/DEBUG/signal/
        // RETURN action the value stays frozen at whatever the trap fired
        // with — the ERR action's entry DEBUG reports the FAILED command
        // (`false`), never the action's own first command (probe
        // 2026-10-12: `trap 'echo DEBUG: $BASH_COMMAND' DEBUG;
        // trap 'echo ACT; false' ERR; false` GNU prints `DEBUG: false` for
        // every in-action firing, not `DEBUG: echo ACT`).
        let update_bash_command =
            !(self.error_trap_running || self.signal_trap_running || self.return_trap_running);
        if update_bash_command {
            *self.shell_state.debug_trap_command.borrow_mut() = Some(command_text.to_string());
        }
        let call_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok());
        // rubash#375: parse the action once per distinct spliced text (GNU
        // parses at trap invocation, trap.c read_trap_action); per-firing
        // tokenize+parse of a stable action is dead work. The AST is taken
        // out of the cache cell, re-stamped and executed, then put back —
        // no clone on the per-firing path.
        let action = self.comsub_body_alias_splice(action);
        let (mut ast, ast_base) = {
            let taken = self.debug_trap_action_cache.borrow_mut().take();
            match taken {
                Some((key, cached_ast, base)) if key == action => (cached_ast, base),
                _ => {
                    let tokens = crate::lexer::tokenize(&action);
                    (crate::parser::parse(&tokens), 1)
                }
            }
        };
        // rubash#1166: rebase the whole action onto this firing's line —
        // nested nodes keep their action-relative offsets from it (see
        // shift_trap_action_lines). The cache tracks the base the stored
        // lines currently reflect, so reuse shifts by the delta only.
        let ast_base = match call_line {
            Some(line) => {
                shift_trap_action_lines(&mut ast.commands, line as i64 - ast_base);
                line as i64
            }
            None => ast_base,
        };
        let saved_alias_streamed = self.mark_alias_streamed();
        let result = self.with_xtrace_indirection(|executor| executor.execute_ast(&ast));
        // Cache the (re-based) AST under its spliced text for the next
        // firing; the recorded base makes the next rebase exact.
        *self.debug_trap_action_cache.borrow_mut() = Some((action, ast, ast_base));
        self.resume_alias_streamed(saved_alias_streamed);
        // rubash#546 (cell 2): when the firing happened inside a trap
        // action the frozen BASH_COMMAND belongs to the enclosing action —
        // leave it exactly as it was found.
        if update_bash_command {
            *self.shell_state.debug_trap_command.borrow_mut() = None;
        }
        self.debug_trap_running = false;
        result?;
        let trap_status = self.exit_code;
        self.exit_code = saved_exit_code;
        let extdebug =
            crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "extdebug");
        let skip_command = extdebug && trap_status != 0;
        if skip_command {
            // Design 4.2's order: the extdebug veto (step 4) is
            // decided before the consult (step 5) — a vetoed unit
            // neither executes nor parks at a stop.
            return Ok(Some(trap_status));
        }
        if crate::debugger::SESSION_ACTIVE.is_armed() {
            self.debugger_consult(seam, command_text, seam_facts)?;
        }
        Ok(None)
    }

    /// Runs the RETURN trap action when a function (or sourced script)
    /// returns. Mirrors Bash's `trap ... RETURN` hook used by debuggers.
    pub(crate) fn run_return_trap(&mut self) -> Result<(), ExecuteError> {
        if self.return_trap_running || self.host_internal_depth.get() > 0 {
            return Ok(());
        }
        self.return_trap_running = true;
        let Some(action) =
            crate::builtins::trap::active_trap_action_ref(&self.shell_state.env_vars, "RETURN")
        else {
            self.return_trap_running = false;
            return Ok(());
        };
        // GNU trap.c:367: the RETURN fire saves the pre-trap $? for a bare
        // `exit`/`return` inside the action (interp 1602).
        let prev_saved_exit =
            std::mem::replace(&mut self.trap_saved_exit_value, Some(self.exit_code));
        let prev_saved_context = std::mem::replace(
            &mut self.trap_saved_exit_context,
            self.shell_state.function_depth + self.source_nest_depth,
        );
        let tokens = crate::lexer::tokenize(&action);
        let mut ast = crate::parser::parse(&tokens);
        // GNU trap.c:_run_trap_internal:1196 only sets SEVAL_RESETLINE for
        // ordinary signal traps; the RETURN trap keeps the caller's current
        // line_number, so $LINENO inside the action (and the DEBUG fire for
        // the trap command itself, execute_simple_command:4506) reports the
        // caller's line. Stamp the parsed action with that line, mirroring
        // run_debug_trap above.
        let call_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok());
        if let Some(call_line) = call_line {
            // rubash#1166: rebase, don't just stamp the top level (see
            // shift_trap_action_lines).
            shift_trap_action_lines(&mut ast.commands, call_line as i64 - 1);
        }
        // rubash#1165: the function's return status is fixed BEFORE the
        // RETURN trap runs (execute_cmd.c's return_catch value becomes
        // last_command_exit_value; trap.c _run_trap_internal saves and
        // restores it around the action) — the action's own last-command
        // status must not replace it in either direction (`trap 'echo X'
        // RETURN` after `false` keeps rc 1; `trap 'false' RETURN` after
        // `true` keeps rc 0). Save/restore like the ERR/signal runners;
        // unwinds (exit/return in the action) still propagate.
        //
        // rubash#1406: `break`/`continue` in the RETURN trap must NOT leak
        // into the caller's loop (GNU errors "only meaningful in a loop").
        // The trap runs in the function's return context, which has no loop,
        // even if the caller is in a loop. Save/clear loop_depth around the
        // action so break/continue see no loop.
        let saved_exit = self.exit_code;
        let saved_loop_depth = self.shell_state.loop_depth;
        let saved_inherited = self.shell_state.inherited_loop_depth;
        self.shell_state.loop_depth = 0;
        self.shell_state.inherited_loop_depth = 0;
        let result = self.with_xtrace_indirection(|executor| executor.execute_ast(&ast));
        self.shell_state.loop_depth = saved_loop_depth;
        self.shell_state.inherited_loop_depth = saved_inherited;
        self.trap_saved_exit_value = prev_saved_exit;
        self.trap_saved_exit_context = prev_saved_context;
        self.return_trap_running = false;
        if result.is_ok() {
            self.exit_code = saved_exit;
        }
        result
    }

    /// Whether the DEBUG trap is in execution scope at the current context.
    /// GNU removes the DEBUG trap inside functions that do not inherit it
    /// (execute_cmd.c:5270: no trace attribute and functrace off), so fires
    /// for compound-command sub-parts (for / arith-for expressions) only
    /// happen at the top level or inside inheriting functions.
    /// GNU builtins/source.def:208-216 unsets the ENTRY DEBUG trap for a
    /// sourced file when functrace is off — the same TABLE-driven shape as
    /// the function case, so a trap the file INSTALLS fires like any other
    /// command (trap.c:750-756: "If the function or sourced file changes
    /// the DEBUG trap, SIG_TRAPPED will be set and we don't bother
    /// restoring the original trap string"); only the entry trap's absence
    /// is scoped (rubash#797).
    /// Whether the DEBUG trap is in execution scope at the current context.
    /// Function-level suppression is table-driven: execute_function removes
    /// the inherited DEBUG trap at entry unless the function carries the
    /// trace attribute or functrace is on (execute_cmd.c:5270), and a trap
    /// set inside the body fires like any other command (trap.tests
    /// "func[29] funcdebug"). Subshell environments (command substitutions,
    /// pipeline members) inherit the DEBUG trap only under functrace, which
    /// the cli trap contract encodes: without functrace the
    /// substitution-internal commands do not fire, with functrace they do
    /// (dbg-support.tests caller echoes carry the debug lines).
    pub(crate) fn debug_trap_in_scope(&self) -> bool {
        if self.host_internal_depth.get() > 0 {
            return false;
        }
        if self.shell_state.subshell_depth.get() > 0 {
            // Table-driven (rubash#345): every subshell entry marks an
            // inherited DEBUG trap reset unless functrace is on
            // (trap.rs reset_for_subshell, GNU trap.c:1588), so an ACTIVE
            // action inside a subshell was armed there and must fire for
            // the remaining subshell commands
            // (`echo "$(trap 'echo DC' DEBUG; echo x)"` prints DC then x).
            // functrace keeps the inherited action active instead.
            return crate::builtins::set::shell_option_enabled(
                &self.shell_state.env_vars,
                "functrace",
            ) || crate::builtins::trap::has_active_trap_action(
                &self.shell_state.env_vars,
                "DEBUG",
                "__RUBASH_TRAP_DEBUG",
            );
        }
        true
    }

    /// Whether a function carries the trace attribute (declare -ft name;
    /// trap.def/execute_cmd.c trace_p(var)) making it inherit the DEBUG and
    /// RETURN traps even with the global functrace option off.
    pub(crate) fn function_has_trace_attribute(&self, name: &str) -> bool {
        marked_env_names(&self.shell_state.env_vars, FUNC_TRACE_FUNCTIONS)
            .iter()
            .any(|entry| entry == name)
    }

    /// Whether the RETURN trap can fire at a sourced-file exit here.
    /// GNU evalfile.c:395 (source_file) runs the RETURN trap unconditionally,
    /// but inside a function that does not inherit it the trap was already
    /// restored to default (execute_cmd.c:5295: `signal_in_progress
    /// (DEBUG_TRAP) || ((trace_p (var) == 0) && function_trace_mode == 0)`),
    /// so nothing fires (dbg-support.tests:96/97: no `return lineno` when
    /// functrace is off, but `return lineno: 98 main` at the top level).
    pub(crate) fn return_trap_in_scope(&self) -> bool {
        if self.host_internal_depth.get() > 0 {
            return false;
        }
        if self.shell_state.function_depth == 0 {
            return true;
        }
        if self.debug_trap_running {
            return false;
        }
        crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "functrace")
    }

    /// GNU builtins/source.def:209-217 (source_builtin): snapshot the ENTRY
    /// DEBUG action and `restore_default_signal (DEBUG_TRAP)` it, so the
    /// sourced file's commands do not fire the trap that was live at entry
    /// (functrace off). The live trap table decides every fire inside the
    /// file — a trap the file INSTALLS fires normally (trap.c:750-756).
    /// Membership, not action text: an entry IGNORE (`trap '' DEBUG`) is
    /// skipped whole — TRAP_STRING is NULL for an ignored trap
    /// (trap.h:57-59), so source.def:211's `if (debug_trap && ...)` never
    /// arms the snapshot/remove/restore chain and the ignore stays trapped
    /// through the file (`trap -p DEBUG` keeps showing `trap -- '' DEBUG`).
    /// Returns the entry action for `source_exit_debug_trap_restore`.
    pub(crate) fn source_entry_debug_trap_save(&mut self) -> Option<String> {
        // get_trap_action's Some/None IS the SIG_TRAPPED analog: Some("")
        // for a live ignore, None for untrapped or subshell-reset — the
        // __RUBASH_TRAPS membership the CR review keys on.
        let entry = crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")?;
        if entry.is_empty() {
            return None;
        }
        crate::builtins::trap::remove_trap(&mut self.shell_state.env_vars, "DEBUG");
        Some(entry)
    }

    /// GNU trap.c:737-763 (trap_if_untrapped via uw_maybe_set_debug_trap,
    /// popped by source.def:216's unwind-protect after source_file's
    /// run_return_trap): the ENTRY action comes back only when DEBUG is
    /// UNTRAPPED at exit — SIG_TRAPPED is set for a plain install AND for
    /// an ignore (trap.c:900 marks ignored traps SIG_TRAPPED too), so a
    /// `trap '' DEBUG` from the file (or its RETURN action) survives and
    /// the entry action stays retired (rubash#797 CR R1/R3: membership,
    /// not action text — get_trap_action's Some("") must read as trapped).
    pub(crate) fn source_exit_debug_trap_restore(&mut self, entry: Option<String>) {
        if let Some(action) = entry {
            if crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG").is_none()
            {
                crate::builtins::trap::set_trap(&mut self.shell_state.env_vars, "DEBUG", &action);
            }
        }
    }

    /// rubash#2233: GNU jobs.c:1783 hangup_all_jobs — SIGHUP to every
    /// job's process group, plus SIGCONT to a stopped job's group so
    /// the HUP is actually taken; jobs marked with `disown -h`
    /// (jobs.h J_NOHUP) are skipped. Called from finish_shell on the
    /// two hangup funnels: an interactive shell that received SIGHUP
    /// (sig.c:621-622 termsig_handler) and an interactive LOGIN shell
    /// exiting with `shopt -s huponexit' (shell.c:1017-1020
    /// exit_shell). Completed jobs are skipped: their processes are
    /// already reaped, and unlike GNU — whose table drops a job at
    /// notification — this table keeps completed entries until a
    /// later cleanup, so signalling their groups could only hit a
    /// recycled pgrp.
    #[cfg(unix)]
    pub(crate) fn hangup_all_jobs(&self) {
        for job in self.shell_state.job_table.jobs.values() {
            if job.nohup || job.pgrp == 0 {
                continue;
            }
            if job.state == crate::jobs::ProcessState::Completed {
                continue;
            }
            // SAFETY: killpg on recorded job process groups; a group
            // that vanished since the snapshot answers ESRCH, ignored.
            unsafe {
                libc::killpg(job.pgrp as libc::pid_t, libc::SIGHUP);
                if job.state == crate::jobs::ProcessState::Stopped {
                    libc::killpg(job.pgrp as libc::pid_t, libc::SIGCONT);
                }
            }
        }
    }

    /// rubash#2233, generalized by rubash#2346: whether GNU settles an
    /// untrapped delivery of `signal` through termsig_handler
    /// (sig.c:590) in this shell — the terminating set is caught when
    /// an EXIT trap is armed (sig.c:226-229, the kill.rs exit_reroutes
    /// set is exactly that catch set), and SIGHUP is additionally
    /// caught in interactive shells (sig.c:315-316). Without a catch,
    /// the signal stays at SIG_DFL and the kernel kills the shell in
    /// place; no drain ever sees the delivery. The one exception is
    /// SIGINT in an interactive shell: GNU routes it to the dedicated
    /// sigint handler (sig.c sigint_sighandler), whose verdict is a
    /// top-level DISCARD, never the termsig death — so it is never
    /// armed here, EXIT trap or not.
    /// rubash#2130: interactivity is judged by the typed invocation
    /// shape, not the env carrier.
    #[cfg(unix)]
    fn termsig_death_armed(&self, signal: i32) -> bool {
        let interactive = self.invocation_interactive();
        if interactive && signal == libc::SIGINT as i32 {
            return false;
        }
        if crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "EXIT").is_some()
            && crate::builtins::kill::exit_reroutes(signal)
        {
            return true;
        }
        interactive && signal == libc::SIGHUP as i32
    }

    /// rubash#2233, generalized by rubash#2346: the untrapped
    /// terminating-signal verdict shared by every mailbox drain
    /// (boundary dispatch, blocked-wait observer, the `wait` builtin's
    /// drain): where GNU catches the delivery (termsig_death_armed) it
    /// is neither a discard (the shell does NOT survive —
    /// termsig_handler runs the EXIT trap and dies by the signal) nor
    /// a plain exit (the death must be BY the signal, after the finish
    /// funnel runs the EXIT trap). Record the termsig-death mark and
    /// unwind exit-class with 128+sig; finish_shell consumes the mark.
    /// Returns true when the delivery was a termsig death (the caller
    /// then unwinds with the returned error class), false otherwise.
    /// (Built for SIGHUP under #2233; the audit found TERM, INT and
    /// USR1 taking the plain-exit path below the drains instead, so
    /// the verdict now covers the whole caught terminating set.)
    #[cfg(unix)]
    pub(crate) fn note_untrapped_termsig(&mut self, signal: i32) -> bool {
        if !self.termsig_death_armed(signal) {
            return false;
        }
        // rubash#2352: capture the EXIT trap's entry status BEFORE
        // the 128+sig overwrite — at every drain that reaches here,
        // exit_code is still the last completed command's status
        // (trap.c:1036 trap_saved_exit_value). The boundary dispatch
        // may replace the capture for an ancestor-lane delivery with
        // the entry's sender snapshot (the pre-carrier value).
        self.termsig_death_trap_status = Some(self.exit_code);
        self.exit_code = 128 + signal;
        self.termsig_death_pending = Some(signal);
        true
    }

    /// rubash#2353: the ancestor lane's complementary verdict. A
    /// #1654 routed delivery is synthetic — no kernel disposition
    /// ever fires for it — so the boundary dispatch must enact the
    /// addressed shell's verdict itself. Where GNU catches an
    /// untrapped terminating delivery (termsig_death_armed: an EXIT
    /// trap is armed, or an interactive SIGHUP), note_untrapped_
    /// termsig above settles it. In the complementary cell — no
    /// EXIT trap, non-interactive — GNU installs NO terminating-
    /// signal handler at all (sig.c:230-236: for non-interactive
    /// shells initialize_terminating_signals runs only when an EXIT
    /// trap is defined), so the delivery stays at SIG_DFL and the
    /// kernel kills the shell in place, BY the signal. Arm the same
    /// termsig-death mark so finish_shell performs that death.
    /// Exclusions, each load-bearing:
    ///   * SIGINT: while a script shell waits for a foreground job,
    ///     GNU's wait_sigint_handler (jobs.c:2908) records the
    ///     delivery and otherwise ignores it — the shell dies of it
    ///     only when the job itself died of SIGINT (jobs.c:4384,
    ///     4410 child_caught_sigint), a cooperative path the
    ///     ancestor lane is not (the carrier completed normally by
    ///     construction). The dispatch absorbs that cell instead.
    ///   * a contained in-process child (direct-child route armed):
    ///     its death is simulated — the adjudicated record carries
    ///     the signaled form to the parent's wait and the hosting
    ///     process must not die (rubash#2236).
    ///   * an interactive shell: untrapped deliveries there take
    ///     the DISCARD classification the dispatch already records
    ///     (rubash#1662); an interactive shell also ignores SIGTERM
    ///     outright (sig.c:337-341).
    /// The signal universe is the same terminating set as
    /// exit_reroutes: the default-ignore pair (URG/WINCH), the stop
    /// class, SIGCONT and the realtime block have no SIG_DFL death
    /// to enact; SIGQUIT never reaches here (the dispatch drops it
    /// above — bash ignores QUIT for itself, sig.c:333).
    #[cfg(unix)]
    pub(crate) fn note_routed_untrapped_termsig(&mut self, signal: i32) -> bool {
        if signal == libc::SIGINT as i32
            || self.invocation_interactive()
            || crate::builtins::kill::direct_child_route_active()
            || !crate::builtins::kill::exit_reroutes(signal)
        {
            return false;
        }
        self.exit_code = 128 + signal;
        self.termsig_death_pending = Some(signal);
        true
    }

    /// Non-unix: no routed lane exists (the ancestor queue is a unix
    /// mechanism), so there is no routed verdict to record.
    #[cfg(not(unix))]
    pub(crate) fn note_routed_untrapped_termsig(&mut self, _signal: i32) -> bool {
        false
    }

    /// Non-unix shells have no signal death to settle (the Windows
    /// console hangup face is a different mechanism); the drains'
    /// untrapped classification is unchanged there.
    #[cfg(not(unix))]
    pub(crate) fn note_untrapped_termsig(&mut self, _signal: i32) -> bool {
        false
    }

    /// rubash#2233: poll the mailbox for an untrapped interactive
    /// SIGHUP WITHOUT running any trap actions. The interactive
    /// reader's tty-read-error path needs exactly this asymmetry:
    /// GNU settles an untrapped hangup by signal death even when the
    /// dead terminal's EIO is what surfaced (termsig preempts — the
    /// trapped-HUP control dies 128 via the read-error arm because
    /// the queued trap is NOT serviced first), so the reader may
    /// classify a queued hangup here but must not dispatch the queue.
    /// Non-hangup deliveries are re-queued untouched. The kernel
    /// delivery races the failed read (the handler can land a hair
    /// after the read returns), so an empty first drain earns one
    /// short-settle retry before the caller takes the read-error arm.
    /// Classify one drained batch for the hangup verdict: returns
    /// (found, saw_sighup) — found when an untrapped, termsig-armed
    /// SIGHUP was in the batch (it is consumed and the hangup-death
    /// mark recorded); every other signal is re-queued untouched.
    /// The trap table keys by the canonical SIGNALS spelling
    /// ("SIGHUP"); a "HUP" lookup reads a phantom key and would
    /// misclassify a TRAPPED hangup as untrapped (the drains above
    /// reach note_untrapped_termsig only after the canonical lookup
    /// found no action).
    #[cfg(unix)]
    fn classify_drained_hangup(&mut self, signals: Vec<i32>) -> (bool, bool) {
        let mut rest = Vec::new();
        let mut found = false;
        let mut saw_sighup = false;
        for signal in signals {
            if signal == libc::SIGHUP as i32 {
                saw_sighup = true;
                if !found
                    && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "SIGHUP")
                        .is_none()
                    && self.termsig_death_armed(signal)
                {
                    found = true;
                    continue;
                }
            }
            rest.push(signal);
        }
        if !rest.is_empty() {
            crate::builtins::kill::requeue_pending_signals(rest);
        }
        if found {
            // rubash#2352: same trap-entry capture as
            // note_untrapped_termsig — the reader's exit_code here
            // is still the last completed command's status.
            self.termsig_death_trap_status = Some(self.exit_code);
            self.exit_code = 128 + libc::SIGHUP as i32;
            self.termsig_death_pending = Some(libc::SIGHUP as i32);
        }
        (found, saw_sighup)
    }

    #[cfg(unix)]
    pub(crate) fn poll_untrapped_hangup(&mut self) -> bool {
        if self.hangup_death_pending() {
            return true;
        }
        for attempt in 0..2 {
            if attempt == 1 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let signals = crate::builtins::kill::take_pending_signals_now(std::process::id())
                .unwrap_or_default();
            if signals.is_empty() {
                continue;
            }
            let (found, _) = self.classify_drained_hangup(signals);
            return found;
        }
        false
    }

    /// Non-unix: no hangup delivery exists to poll for.
    #[cfg(not(unix))]
    pub(crate) fn poll_untrapped_hangup(&mut self) -> bool {
        false
    }

    /// rubash#2233: the settled variant for callers that KNOW the
    /// terminal is dead (the slave read returned EOF with POLLHUP on
    /// the fd, or EIO): the kernel delivered SIGHUP at the same
    /// moment it retired the terminal, but the mailbox queues it
    /// asynchronously, and under load the queueing can lag the
    /// reader's fold by far more than poll_untrapped_hangup's 5 ms
    /// settle — the fold then wins and the hangup is lost (or is
    /// noted mid-finish by the EXIT trap's drain, surfacing as a
    /// plain 129 with no signal death). Drain until the delivery
    /// lands or the budget expires; a SIGHUP that IS queued resolves
    /// at once (an untrapped one classifies as the hangup, a trapped
    /// or ignored one ends the wait as a plain fold — its trap must
    /// not run on this path). GNU never faces this window: its
    /// termsig handler is synchronous with the delivery.
    #[cfg(unix)]
    pub(crate) fn poll_untrapped_hangup_settled(&mut self, budget: std::time::Duration) -> bool {
        if self.hangup_death_pending() {
            return true;
        }
        let deadline = std::time::Instant::now() + budget;
        loop {
            let signals = crate::builtins::kill::take_pending_signals_now(std::process::id())
                .unwrap_or_default();
            if !signals.is_empty() {
                let (found, saw_sighup) = self.classify_drained_hangup(signals);
                if found || saw_sighup {
                    return found;
                }
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Non-unix: no hangup delivery exists to poll for.
    #[cfg(not(unix))]
    pub(crate) fn poll_untrapped_hangup_settled(&mut self, _budget: std::time::Duration) -> bool {
        false
    }

    pub(crate) fn run_pending_signal_traps(&mut self) -> Result<(), ExecuteError> {
        // Pending signals belong to the shell process. A subshell sharing
        // this process (the flat `( ... )` emulation, the in-process
        // ${THIS_SH} child) must not consume the parent's mailbox or
        // dispatch the parent's traps after resetting caught dispositions.
        // A respawned `rubash -c` background child is different: it owns its
        // own mailbox (`{pid}.alive` names the real process id), and GNU
        // runs a trap set INSIDE its subshell body inside that subshell —
        // execute_cmd.c:1670 execute_in_subshell resets only the INHERITED
        // dispositions, a `trap` in the body installs fresh ones
        // (builtins/trap.def), and the pending TERM dispatches at the next
        // execute_cmd.c:643 run_pending_traps before the body scope ends.
        if self.signal_trap_running
            || (self.shell_state.subshell_depth.get() > 0 && self.shell_pid == std::process::id())
        {
            return Ok(());
        }

        // Unix kernel-disposition reconcile (rubash#226): mirror GNU's
        // per-signal handler state before dispatching. trap.c trap_builtin
        // installs one handler per trapped signal (sig.c:830, sa_flags=0 --
        // no SA_RESTART); every other terminating signal stays SIG_DFL
        // (sig.c:315-316: initialize_terminating_signals is
        // interactive-only), so an untrapped signal kills the process at the
        // KERNEL, never through this queue. The boundary poll (and the trap
        // builtin's own mutation path) keeps the two in lockstep.
        #[cfg(unix)]
        crate::builtins::kill::reconcile_kernel_trap_dispositions(&self.shell_state.env_vars);

        // rubash#1654: deliveries a flat subshell addressed to the
        // shell's own pid were queued for THIS layer's adjudication
        // (kill.rs ANCESTOR_SIGNALS). They go first — in GNU the
        // addressed (parent) process acts on them while the subshell
        // is still its foreground job, ahead of anything that arrived
        // later — and the stale-handler check does not apply: they
        // were never routed through this layer's kernel dispositions,
        // so the CURRENT (restored top-level) trap table below is
        // their only judge (trapped -> run, ignored -> drop,
        // untrapped -> the 128+sig death).
        let signals = crate::builtins::kill::take_pending_signals(std::process::id())?;
        // #682: with_capacity(4) heap-allocated at every command boundary
        // even when no delivery is pending (the common case); Vec::new()
        // defers the allocation to the first push — identical growth
        // semantics, one malloc/free per command removed from the floor.
        // The third element is the ancestor entry's sender-status
        // snapshot (rubash#2352; kernel-lane entries carry None).
        let mut queue: Vec<(i32, bool, Option<i32>)> = Vec::new();
        #[cfg(unix)]
        queue.extend(
            crate::builtins::kill::take_ancestor_signals()
                .into_iter()
                .map(|(signal, sender_status)| (signal, false, Some(sender_status))),
        );
        queue.extend(signals.into_iter().map(|signal| (signal, true, None)));
        for (signal, stale_check, sender_status) in queue {
            // Stale registry flag from a disposition flip (trap cleared while
            // a delivery raced): a signal the kernel no longer routes here
            // cannot be a legitimate pending trap.
            #[cfg(unix)]
            if stale_check && !crate::builtins::kill::kernel_handler_installed(signal) {
                continue;
            }
            let Some(signal_name) = signal_trap_name(signal) else {
                continue;
            };
            let action =
                crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, &signal_name);
            let Some(action) = action else {
                // Bash's default disposition for SIGCHLD is to ignore it.
                // Child completion/reaping notifications must not turn into
                // a synthetic 128+SIGCHLD shell exit when no CHLD trap is
                // installed (busybox ash `reap*.tests`). SIGCHLD follows
                // the TARGET numbering: 17 on Linux, 20 on Darwin/BSD
                // (crate::builtins::kill::SIGCHLD_NUMBER resolves libc on
                // unix; non-unix keeps the mailbox wire-format 17).
                if signal == crate::builtins::kill::SIGCHLD_NUMBER {
                    continue;
                }
                // sig.c:333 (initialize_shell_signals): every shell sets
                // SIGQUIT to SIG_IGN for itself before the interactive-only
                // SIGINT/SIGTERM installs, so an untrapped QUIT delivery is
                // dropped, never a 128+SIGQUIT shell exit. Only a user trap
                // overrides the ignore, and `trap - QUIT` returns to it
                // (trap.c marks SIGQUIT SIG_SPECIAL for that reset).
                if signal == crate::builtins::kill::SIGQUIT_NUMBER {
                    continue;
                }
                // rubash#2233/#2346: an untrapped terminating signal
                // GNU catches (an EXIT trap is armed, or an interactive
                // SIGHUP) is a termsig death (sig.c:590
                // termsig_handler): the session does NOT survive as
                // the discard classification below would have it, and
                // the finish funnel owes the EXIT trap (and, for an
                // interactive hangup, the jobs' hangup) before the
                // shell dies BY the signal. The helper records the
                // mark and the exit status; unwind exit-class.
                if self.note_untrapped_termsig(signal) {
                    // rubash#2352: for an ancestor-lane delivery the
                    // trap's entry status is the entry's sender
                    // snapshot, not this layer's exit_code — the
                    // carrier construct already committed its own
                    // status here, but GNU's addressed shell was
                    // still waiting for it, so its last_command_
                    // exit_value is the pre-carrier one (trap.c:1036;
                    // the wait is preempted, never completed).
                    if let Some(sender_status) = sender_status {
                        self.termsig_death_trap_status = Some(sender_status);
                    }
                    // rubash#2236: record the death so an in-process
                    // child's exit boundary can queue the parent's
                    // job-death notice for it (a real child's wait(2)
                    // status carries the same fact).
                    crate::builtins::kill::note_adjudicated_signal_death(signal);
                    return Err(ExecuteError::ExitCode(128 + signal));
                }
                // rubash#2353: a routed (ancestor-lane) delivery in
                // the complementary cell — no EXIT trap caught it
                // above, non-interactive — is a SIG_DFL death GNU
                // never runs a handler for (sig.c:230-236), enacted
                // here by arming the same termsig mark. Kernel-lane
                // entries keep the plain path below: their stale
                // check already proved a handler story this lane
                // never has, and a contained child's routed entries
                // are excluded inside the helper (its death is the
                // simulated plain path below, rubash#2236).
                if !stale_check && self.note_routed_untrapped_termsig(signal) {
                    crate::builtins::kill::note_adjudicated_signal_death(signal);
                    return Err(ExecuteError::ExitCode(128 + signal));
                }
                // rubash#2353 (SIGINT cell): a routed SIGINT the
                // verdicts above did not take is absorbed. GNU's
                // script shell waiting on the carrier — this lane's
                // only posture, since the entry surfaces after the
                // carrier completed — records an untrapped SIGINT
                // and otherwise ignores it (jobs.c:2908
                // wait_sigint_handler); it dies of SIGINT only when
                // the foreground job itself died of it (jobs.c:4384,
                // 4410), which a completed carrier did not. Drop the
                // entry: the carrier's status stands and the script
                // continues. A contained child's entries keep the
                // plain path (its simulated death is adjudicated
                // below), as do interactive shells (DISCARD, #1662).
                #[cfg(unix)]
                if !stale_check
                    && signal == libc::SIGINT as i32
                    && !self.invocation_interactive()
                    && !crate::builtins::kill::direct_child_route_active()
                {
                    continue;
                }
                self.exit_code = 128 + signal;
                // rubash#2236: record the death so an in-process
                // child's exit boundary can queue the parent's
                // job-death notice for it (a real child's wait(2)
                // status carries the same fact).
                crate::builtins::kill::note_adjudicated_signal_death(signal);
                // rubash#1662: in an interactive shell this abort is a
                // DISCARD-class unwind (GNU: the interrupted command
                // ends with 128+sig and the reader prompts again), not
                // an exit — mark it so run_source_impl does not record
                // an exit jump for the unit. Non-interactive shells keep
                // the exit classification (the delivery is how a script
                // shell dies at 128+sig, running any EXIT trap).
                if self.invocation_interactive() {
                    self.signal_discard_pending = true;
                }
                return Err(ExecuteError::ExitCode(128 + signal));
            };
            if action.is_empty() {
                continue;
            }

            let saved_exit = self.exit_code;
            self.exit_code = saved_exit;
            self.signal_trap_running = true;
            let old_signal_status = self.shell_state.env_vars.insert(
                "__RUBASH_SIGNAL_TRAP_STATUS".to_string(),
                saved_exit.to_string(),
            );
            // The pre-trap status override for a bare "return" applies only
            // to the return that terminates the trap action itself (posix
            // interp 1602). A "return" in a function CALLED by the action
            // uses the current $?, so record the action's entry
            // function_depth for the return builtin to compare.
            let old_signal_depth = self.shell_state.env_vars.insert(
                "__RUBASH_SIGNAL_TRAP_DEPTH".to_string(),
                self.shell_state.function_depth.to_string(),
            );
            // GNU trap.c _run_trap_internal:373-385 binds BASH_TRAPSIG to the
            // signal number of the trap being executed for the duration of
            // the action, restoring the previous value afterwards
            // (save_bash_trapsig/set_bash_trapsig/restore_bash_trapsig).
            let old_bash_trapsig = self.shell_state.env_vars.get("BASH_TRAPSIG").cloned();
            self.shell_state
                .env_vars
                .insert("BASH_TRAPSIG".to_string(), signal.to_string());
            // BASH_TRAPSIG is bound unexported (bind_var_to_int attrs 0), but
            // command substitutions run as child processes that only receive
            // exported variables, so the trap action's $(kill -l
            // $BASH_TRAPSIG) would otherwise see nothing. Export it for the
            // duration of the action; this is invisible to the script.
            let bash_trapsig_was_exported =
                marked_env_names(&self.shell_state.env_vars, EXPORTED_VARS)
                    .iter()
                    .any(|name| name == "BASH_TRAPSIG");
            if !bash_trapsig_was_exported {
                mark_env_name(
                    &mut self.shell_state.env_vars,
                    EXPORTED_VARS,
                    "BASH_TRAPSIG",
                );
            }
            // GNU trap.c:496 runs the action through parse_and_execute
            // with the input name "trap" ("interrupt trap" for SIGINT):
            // parser_error's prolog becomes `<script>: trap: line N:`.
            // Mirror the __RUBASH_EVAL_CONTEXT marker for the duration
            // of the action.
            let old_trap_context = self.shell_state.env_vars.insert(
                "__RUBASH_TRAP_CONTEXT".to_string(),
                if signal == 2 {
                    "interrupt trap".to_string()
                } else {
                    "trap".to_string()
                },
            );
            let tokens = crate::lexer::tokenize(&action);
            let ast = crate::parser::parse(&tokens);
            // trap.c:496 runs the action through parse_and_execute: the
            // xtrace indirection level rises by one for the action.
            let result = self.with_xtrace_indirection(|executor| executor.execute_ast(&ast));
            match old_trap_context {
                Some(value) => {
                    self.shell_state
                        .env_vars
                        .insert("__RUBASH_TRAP_CONTEXT".to_string(), value);
                }
                None => {
                    self.shell_state.env_vars.remove("__RUBASH_TRAP_CONTEXT");
                }
            }
            if !bash_trapsig_was_exported {
                unmark_env_name(
                    &mut self.shell_state.env_vars,
                    EXPORTED_VARS,
                    "BASH_TRAPSIG",
                );
            }
            match old_bash_trapsig {
                Some(value) => {
                    self.shell_state
                        .env_vars
                        .insert("BASH_TRAPSIG".to_string(), value);
                }
                None => {
                    self.shell_state.env_vars.remove("BASH_TRAPSIG");
                }
            }
            match old_signal_depth {
                Some(value) => {
                    self.shell_state
                        .env_vars
                        .insert("__RUBASH_SIGNAL_TRAP_DEPTH".to_string(), value);
                }
                None => {
                    self.shell_state
                        .env_vars
                        .remove("__RUBASH_SIGNAL_TRAP_DEPTH");
                }
            }
            match old_signal_status {
                Some(value) => {
                    self.shell_state
                        .env_vars
                        .insert("__RUBASH_SIGNAL_TRAP_STATUS".to_string(), value);
                }
                None => {
                    self.shell_state
                        .env_vars
                        .remove("__RUBASH_SIGNAL_TRAP_STATUS");
                }
            }
            self.signal_trap_running = false;
            // GNU trap.c:496 (parse_and_execute inside _run_trap_internal)
            // contains a syntax error in the action: the diagnostic
            // stands, the action yields its failure, and the shell goes
            // on with the pre-trap $? — the parse error must not unwind
            // the whole script the way a top-level syntax error does
            // (rubash#1351). Consume the flag either way so it cannot
            // leak into the script driver; a parse error that already
            // exit_shell(2)ed under live errexit (parser_error_errexited)
            // is a real shell exit and keeps propagating.
            let parse_error_raised = self.take_parse_error();
            let parser_error_errexited = self.parser_error_errexited.take();
            match result {
                Err(ExecuteError::ExitCode(2)) if parse_error_raised && !parser_error_errexited => {
                    self.take_exit_jump_pending();
                    self.exit_code = saved_exit;
                }
                Ok(()) => self.exit_code = saved_exit,
                Err(error @ ExecuteError::Return(_)) => return Err(error),
                Err(error @ ExecuteError::ExitCode(_)) => return Err(error),
                Err(error) => return Err(error),
            }
        }

        // WP-3 (design 4.4; WP-1 gaps G-05/G-10; 150-c REPORT
        // section 5, note 5): the asynchronous stop is serviced
        // here — after any signal trap for it has been dispatched
        // above — at the boundary poll. While a trap action runs,
        // servicing waits: the poll's later arrivals (after the
        // action completes) synchronize on the same point.
        if !self.debug_trap_running && !self.return_trap_running && !self.error_trap_running {
            self.debugger_service_async_stop()?;
        }
        Ok(())
    }

    /// The Windows-mailbox port of GNU's "record the signal while blocked"
    /// step: trap.c:545-627 trap_handler only calls set_trap_state
    /// (trap.c:537-543, `pending_traps[sig]++`) when a caught signal arrives
    /// while the shell is blocked inside a command; the ACTION itself is
    /// deferred to the next command boundary (execute_cmd.c:643
    /// run_pending_traps — bash manual, SIGNALS: "If bash is waiting for a
    /// command to complete and receives a signal for which a trap has been
    /// set, the trap will not be executed until the command completes"). An
    /// untrapped terminating signal has no handler at all in GNU: SIG_DFL
    /// kills the shell immediately, even mid-wait_for.
    ///
    /// Called from the sleep fast path and the foreground external wait.
    /// Trapped deliveries are re-queued into the in-process queue (which the
    /// boundary poll drains unconditionally), ignored ones are dropped (the
    /// boundary would skip them anyway, trap_exec run_pending_signal_traps
    /// `action.is_empty()`), and an untrapped terminating delivery ends the
    /// shell now with 128+sig — the SIG_DFL equivalent. The subshell guard
    /// mirrors run_pending_signal_traps above: a flat subshell sharing this
    /// process must leave the shared mailbox alone, while a respawned
    /// background child observes its own mailbox even inside its subshell
    /// body.
    pub(crate) fn observe_signals_while_blocked(&mut self) -> Result<(), ExecuteError> {
        if self.signal_trap_running
            || (self.shell_state.subshell_depth.get() > 0 && self.shell_pid == std::process::id())
        {
            return Ok(());
        }
        let signals =
            crate::builtins::kill::take_pending_signals_now(std::process::id()).unwrap_or_default();
        if signals.is_empty() {
            return Ok(());
        }
        let mut trapped = Vec::new();
        for &signal in &signals {
            // Stale registry flag from a disposition flip: drop it (see
            // run_pending_signal_traps above).
            #[cfg(unix)]
            if !crate::builtins::kill::kernel_handler_installed(signal) {
                continue;
            }
            let Some(signal_name) = signal_trap_name(signal) else {
                continue;
            };
            let action =
                crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, &signal_name);
            match action {
                Some(action) if !action.is_empty() => trapped.push(signal),
                Some(_) => {}
                None if signal == crate::builtins::kill::SIGCHLD_NUMBER => {}
                // sig.c:333: every shell ignores SIGQUIT for itself; an
                // untrapped delivery while blocked is dropped, not fatal.
                None if signal == crate::builtins::kill::SIGQUIT_NUMBER => {}
                None => {
                    // rubash#2233/#2346: an untrapped terminating
                    // signal GNU catches, delivered while blocked, is
                    // the termsig death, not the interactive discard
                    // the generic arm below records (see the boundary
                    // dispatch's untrapped branch).
                    if self.note_untrapped_termsig(signal) {
                        return Err(ExecuteError::ExitCode(128 + signal));
                    }
                    // Record $? before the unwind so an interactive host that
                    // only maps ExitCode(code) still reports 130 after Ctrl+C.
                    self.exit_code = 128 + signal;
                    // niubash#698: the same classification the other two
                    // mailbox drains make for this exact event — the
                    // boundary dispatch's untrapped branch above
                    // (run_pending_signal_traps) and the `wait` builtin's
                    // drain (job_builtins wait_pending_signal_status,
                    // rubash#1825): in an interactive session an untrapped
                    // delivery is a DISCARD-class unwind (GNU jobs.c:3315
                    // ADDINTERRUPT throws the interrupted wait to the
                    // top-level reader; the session survives with
                    // $?=128+sig), not a jump_to_top_level exit. This
                    // mid-wait drain is the ONLY consumer on the paths
                    // where no kernel delivery adjudicates first — the
                    // Windows foreground/pipeline waits (#2020) and the
                    // non-unix builtin sleep — so without the mark the
                    // source driver read the bare ExitCode as an exit
                    // jump and an interactive niu died at rc=130 on the
                    // first untrapped Ctrl+C of a foreground command.
                    // The test reads the executor's session flag (the
                    // host-declared interactive state every sibling
                    // classification reads), never the process env.
                    // Non-interactive sessions set nothing and keep the
                    // exit classification byte-identically.
                    if self.invocation_interactive() {
                        self.signal_discard_pending = true;
                    }
                    // rubash#2236: same death record as the boundary
                    // dispatch — a blocked wait inside an in-process
                    // child adjudicates the child's death too.
                    crate::builtins::kill::note_adjudicated_signal_death(signal);
                    return Err(ExecuteError::ExitCode(128 + signal));
                }
            }
        }
        crate::builtins::kill::requeue_pending_signals(trapped);
        Ok(())
    }

    /// GNU `sleep N` is an external child process, so the waiting shell
    /// stays interruptible (jobs.c:3064 wait_for; a caught signal is
    /// recorded by trap.c:545 trap_handler and dispatched when the sleep
    /// completes, an untrapped one kills the shell via SIG_DFL). The
    /// in-process fast path emulates that by slicing the sleep and running
    /// observe_signals_while_blocked between slices.
    ///
    /// rubash#1034: the emulation has one gap a real child does not —
    /// a group-delivered terminating signal (^C) kills GNU's real
    /// sleep instantly, wait_for returns, and the shell's trap
    /// dispatches at once; the virtual child cannot die, so without a
    /// model the trap sits queued for the sleep's whole remainder.
    /// But the model must key on WHO the delivery could have killed:
    /// a kill(2) addressed to the shell alone (`kill -INT $$` from a
    /// background job) leaves GNU's real sleep untouched, and GNU
    /// defers the trap until the sleep completes — the first port of
    /// this model aborted on ANY trapped terminating delivery and
    /// fired the trap immediately there too, the mirror-image bug.
    /// The observable boundary is the delivery's origin (siginfo
    /// si_code, recorded by the kernel backend's drain): a
    /// KERNEL-caused delivery is the terminal driver's group signal
    /// (^C/^\\), which a real foreground child in the group dies
    /// from — model the child as killed by it: dispatch the pending
    /// traps now and end the sleep, reporting the killing signal so
    /// the caller records 128+sig as the command's status, exactly
    /// like a reaped signal death. Kill(2)-caused deliveries (any
    /// sender process, including this shell's own kill builtin) keep
    /// GNU's defer-to-completion semantics untouched, as do
    /// non-terminating trapped signals (WINCH, CHLD...).
    ///
    /// rubash#2241 refines the kill(2) boundary: siginfo cannot say
    /// whether a kill(2) addressed this pid or this whole process
    /// group, but the SENDER's group membership can (see
    /// delivery_would_kill_virtual_child). bats' CTRL-C tests deliver
    /// `kill -INT -- -pgid` from a driver process outside the group;
    /// GNU's real sleep dies from that delivery and the trap runs at
    /// once, so the virtual child dies the same death. A sender
    /// inside the group is one of the shell's own jobs (`kill -INT
    /// $$`), whose addressing was the shell alone — deferral stands.
    /// rubash#2241: would a real foreground child have died from the
    /// latest kill(2) delivery of `signal`? The emulated sleep's
    /// virtual child shares the shell's process group exactly when
    /// job control is off (GNU jobs.c:2390 make_child groups the
    /// child only under job control; without it the child stays in
    /// the shell's group, and with it a delivery that reached the
    /// shell never reached the child's own group). A kill(2) arriving
    /// from a sender OUTSIDE the shell's process group was addressed
    /// to the shell alone or to its whole group; addressed to the
    /// group, GNU's real child dies from it and jobs.c:3064 wait_for
    /// dispatches the pending trap immediately. A sender INSIDE the
    /// group — one of the shell's own jobs running `kill -INT $$` —
    /// is rubash#1034's shape: the addressing was the shell alone and
    /// the child sleeps on, so the trap keeps deferring. An exited
    /// sender (getpgid fails) also defers: the #1034 sender exits
    /// right after sending, while a group-kill sender (bats' driver)
    /// is still alive, waiting on the group it killed. The one
    /// indistinguishable residue — an outside sender pid-addressing
    /// the shell alone — is accepted and modeled as a group delivery:
    /// no observable separates the two, and deferring there is the
    /// failure mode this issue is about.
    #[cfg(unix)]
    fn delivery_would_kill_virtual_child(&self, signal: i32) -> bool {
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor") {
            return false;
        }
        let Some(sender) = crate::builtins::kill::last_delivery_sender_pid(signal) else {
            return false;
        };
        if sender == std::process::id() {
            return false;
        }
        let sender_pgid = unsafe { libc::getpgid(sender as libc::pid_t) };
        if sender_pgid <= 0 {
            return false;
        }
        let own_pgid = unsafe { libc::getpgid(0) };
        own_pgid > 0 && sender_pgid != own_pgid
    }

    pub(crate) fn sleep_interruptibly(
        &mut self,
        seconds: f64,
    ) -> Result<Option<i32>, ExecuteError> {
        const SLICE: std::time::Duration = std::time::Duration::from_millis(50);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
        // rubash#1250: the issue's `sleep 30` repro lands here (sleep is a
        // builtin), not in wait_external_child. An interactive shell with
        // untrapped SIGINT must survive Ctrl-C with rc=130 like GNU, which
        // reaps its signal-killed external sleep; SIG_DFL would kill this
        // process inside the slice sleep. The guard BLOCKS (not ignores)
        // so the arrival stays observable: poll_abort ends the sleep with
        // 128+SIGINT instead of running it to completion (rc=0).
        #[cfg(unix)]
        let _builtin_guard =
            super::external_inner::ForegroundBuiltinSigintGuard::hold_if_interactive_untrapped(
                self,
            );
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            std::thread::sleep((deadline - now).min(SLICE));
            // rubash#1250: the hold above routed untrapped SIGINT to the
            // mailbox for this wait. Consume it HERE (before observe drains
            // it as an exit-jump error): abort the virtual child with
            // 128+SIGINT like a reaped signal death, and keep the reader
            // going. take_sigint_abort is armed-gated so a TRAPPED SIGINT
            // still flows through observe and runs its action.
            #[cfg(unix)]
            if _builtin_guard.take_sigint_abort() {
                return Ok(Some(libc::SIGINT));
            }
            // The µs race (delivery lands between the take above and
            // observe's own take) surfaces as the untrapped error; fold it
            // into the same abort instead of an exit jump — but only for
            // a kernel-caused delivery (rubash#1640): a shell-only
            // kill(2) SIGINT leaves GNU's real child running, and the
            // interrupt observe just consumed is forgotten, so the
            // virtual child sleeps on to completion (rc=0).
            #[cfg(unix)]
            match self.observe_signals_while_blocked() {
                Err(ExecuteError::ExitCode(code)) if code == 128 + libc::SIGINT => {
                    if crate::builtins::kill::last_delivery_kernel_caused(libc::SIGINT) {
                        return Ok(Some(libc::SIGINT));
                    }
                    // The delivery is forgotten (see above), so the
                    // discard classification observe attached to its
                    // unwind is forgotten with it: the mark classifies
                    // the unwind it was set for, and a swallowed
                    // unwind must not leave it behind to misclassify a
                    // later genuine exit in the same unit (niubash#698).
                    self.take_signal_discard_pending();
                }
                other => other?,
            }
            #[cfg(not(unix))]
            self.observe_signals_while_blocked()?;
            // observe_signals_while_blocked re-queued any trapped
            // deliveries; take them back for the virtual-child check
            // and put back whatever does not abort the sleep.
            let pending = crate::builtins::kill::take_pending_signals_now(std::process::id())
                .unwrap_or_default();
            if pending.is_empty() {
                continue;
            }
            let abort = pending.iter().copied().find(|&signal| {
                #[cfg(unix)]
                {
                    // rubash#2344: a stop-class signal never KILLS a
                    // real foreground child (it stops it), so the death
                    // model below cannot fire for one — the delivery
                    // defers to the sleep's completion like any other
                    // trapped arrival. Before the stop class joined
                    // the managed set no such delivery could be pending
                    // here at all; the exclusion keeps that invariant.
                    !crate::builtins::kill::kernel_is_stop_class(signal)
                        && crate::builtins::kill::kernel_handler_installed(signal)
                        // Kernel-caused (terminal ^C): the real child
                        // dies, rubash#1034. Kill(2)-caused: the real
                        // child dies only when the delivery addressed
                        // the shared process group (rubash#2241).
                        && (crate::builtins::kill::last_delivery_kernel_caused(signal)
                            || self.delivery_would_kill_virtual_child(signal))
                        && signal_trap_name(signal).is_some_and(|name| {
                            crate::builtins::trap::get_trap_action(
                                &self.shell_state.env_vars,
                                &name,
                            )
                            .is_some_and(|action| !action.is_empty())
                        })
                }
                #[cfg(not(unix))]
                {
                    let _ = signal;
                    false
                }
            });
            crate::builtins::kill::requeue_pending_signals(pending);
            if let Some(signal) = abort {
                self.run_pending_signal_traps()?;
                return Ok(Some(signal));
            }
        }
    }

    /// Run the ERR trap after a command (or pipeline) completes with a
    /// non-zero status. GNU execute_cmd.c fires the error trap for every
    /// failing command that is not part of a &&/|| list, not inverted with
    /// the ! keyword, and not suppressed by errexit handling; functions
    /// inherit it only under "set -o errtrace" (trap.c error_trace_mode).
    pub(crate) fn maybe_run_error_trap(
        &mut self,
        command: &CommandNode,
    ) -> Result<(), ExecuteError> {
        self.maybe_run_error_trap_inner(command, None, None, false)
    }

    /// Run the ERR adjudication for a flat `f() ( list )` region boundary —
    /// the rubash#720 form 2 fire. GNU runs a user subshell's completion ERR
    /// check in the frame that waited for the fork, after wait_for
    /// (execute_cmd.c:769-778), keyed on the subshell command itself: the
    /// site checks user_subshell, was_error_trap, ignore_return, invert and
    /// the status — there is NO &&/|| exemption there (probe: `f() ( false
    /// && true ); f` fires). The forkless engine has no command node for
    /// the region, so `command_text` carries the region's print_cmd text —
    /// GNU's the_printed_command_except_trap is make_command_string of the
    /// subshell, fixed by the fork preamble (execute_cmd.c:703-710) — and
    /// `line` is the subshell's own close-paren line (execute_cmd.c:696
    /// SET_LINE_NUMBER(command->value.Subshell->line), the line the :769
    /// fire reports as $LINENO).
    pub(crate) fn maybe_run_error_trap_for_flat_region(
        &mut self,
        command: &CommandNode,
        command_text: String,
        line: Option<usize>,
    ) -> Result<(), ExecuteError> {
        self.maybe_run_error_trap_inner(command, Some(command_text), line, true)
    }

    fn maybe_run_error_trap_inner(
        &mut self,
        command: &CommandNode,
        command_text_override: Option<String>,
        line_override: Option<usize>,
        bypass_and_or_gate: bool,
    ) -> Result<(), ExecuteError> {
        // GNU trap.c run_error_trap (execute_cmd.c callers): the ERR trap
        // refuses to run while it is already in progress —
        // `signal_in_progress (ERROR_TRAP)` — so a failing command inside
        // the ERR action cannot recursively re-trigger it. Rubash's
        // error_trap_running flag mirrors SIG_INPROGRESS.
        if self.exit_code == 0
            || command.inverted
            || (command.and_or().is_some() && !bypass_and_or_gate)
            || self.suppress_errexit != 0
            || self.error_trap_running
            // rubash#2117: the dispatch-level special_builtin_failed
            // gate already fired this command's ERR trap (and the
            // trap's own commands rescued the POSIX exit by clearing
            // the flag, execute_cmd.c:4480) — GNU fires it once, in
            // execute_command_internal; a second fire here would
            // double the action's side effects.
            || self.err_trap_fired_at_dispatch.get()
            || self.host_internal_depth.get() > 0
            // rubash#439: a word/assignment expansion error (GNU subst.c
            // expand_wdesc_error -> exp_jump_to_top_level(DISCARD)) abandons
            // the command BEFORE it executes, and the DISCARD jump bypasses
            // the run_error_trap call sites in execute_cmd.c — so the trap
            // never fires for the expansion failure itself (`trap 'echo E'
            // ERR; x=${!bad}` must print only the diagnostic). A command
            // substitution failure (`x=$(false)`) is NOT this class: the
            // inner command ran and the assignment carries its status, so
            // the ERR trap still fires there.
            || self.command_expansion_failed.get()
        {
            return Ok(());
        }
        // No function-depth gate here (rubash#1163): GNU execute_cmd.c
        // execute_function (5284-5290) saves and CLEARS an inherited ERR
        // trap at function entry when errtrace is off and restores it at
        // exit — function_calls.rs mirrors that — so any ERR action
        // visible while function_depth > 0 was set inside a function and
        // must fire for that function's failing commands. The former
        // `function_depth > 0 && !errtrace` early return here suppressed
        // exactly those function-local traps, checking (in effect) only
        // the function's final status at depth 0.
        // rubash#1164: GNU adjudicates against was_error_trap sampled
        // before the failing command started (execute_cmd.c:914) — a trap
        // installed BY that command does not fire for it. The action string
        // itself is read live here, as GNU reads trap_list at fire time (a
        // trap replaced mid-command fires its NEW action at the enclosing
        // frame whose sample was armed).
        if self.err_trap_armed_sample.get() == Some(false) {
            return Ok(());
        }
        let Some(action) =
            crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "ERR")
        else {
            return Ok(());
        };
        if action.is_empty() {
            return Ok(());
        }
        // DE-06 (design 9.3; PLAN row 12): every gate passed and
        // the action is about to run — a selected `err-trap`
        // filter reports the site and parks the session here,
        // before the action executes.
        self.debugger_note_exception_site(
            crate::debugger::ExceptionKind::ErrTrap,
            self.exit_code,
            None,
        )?;
        self.error_trap_running = true;
        let saved_exit = self.exit_code;
        // GNU trap.c:367 _run_trap_internal saves last_command_exit_value in
        // trap_saved_exit_value before running the action (the failing
        // command's status here); trap.c:383 records funcnest + sourcenest
        // as trap_return_context. A bare `exit` inside the action resolves
        // to the saved value (builtins/common.c get_exitstat, POSIX interp
        // 1602) — commands inside the action cannot change it.
        let prev_saved_exit = std::mem::replace(&mut self.trap_saved_exit_value, Some(saved_exit));
        let prev_saved_context = std::mem::replace(
            &mut self.trap_saved_exit_context,
            self.shell_state.function_depth + self.source_nest_depth,
        );
        let saved_trap_command = self.shell_state.debug_trap_command.borrow().clone();
        // GNU get_bash_command (variables.c:1558) reads
        // the_printed_command_except_trap — the last command printed
        // while no trap was in progress — not necessarily the node that
        // failed: a failing function call reports the function body's
        // last inner command (cond-error1.sub: `func` at line 28 reports
        // `[[ -z nonempty ]]`), and a failing pipeline reports its last
        // stage (`true | false` reports `false`). The flat-region caller
        // overrides with the region's own `( list )` text: GNU's
        // except_trap at the :769 wait still holds make_command_string of
        // the subshell from the fork preamble, not the body's last command.
        *self.shell_state.debug_trap_command.borrow_mut() = command_text_override
            .or_else(|| {
                self.shell_state
                    .env_vars
                    .get("__RUBASH_LAST_COMMAND")
                    .or_else(|| self.shell_state.env_vars.get("__RUBASH_CURRENT_COMMAND"))
                    .cloned()
            })
            .or_else(|| {
                Some(crate::executor::command_text::bash_command_source_text(
                    command,
                ))
            });
        // GNU executes the ERR trap action with LINENO bound to the failed
        // command's line (trap3.sub: `false | false | false` on line 8 makes
        // the ERR action's $LINENO print 8). The action AST re-parses with
        // position-1 lines, which set_current_line would otherwise clobber
        // the tracking value with — pin the action's node lines instead, the
        // way run_debug_trap does. The flat-region caller pins the subshell's
        // close-paren line (execute_cmd.c:696, honored by the :769 fire).
        let failed_line = line_override.or(command.line);
        let action = self.comsub_body_alias_splice(&action);
        let tokens = crate::lexer::tokenize(&action);
        let mut ast = crate::parser::parse(&tokens);
        if let Some(line) = failed_line {
            // rubash#1166: rebase the whole action onto the failed
            // command's line, nested nodes included (see
            // shift_trap_action_lines).
            shift_trap_action_lines(&mut ast.commands, line as i64 - 1);
        }
        let saved_alias_streamed = self.mark_alias_streamed();
        let result = self.with_xtrace_indirection(|executor| executor.execute_ast(&ast));
        self.resume_alias_streamed(saved_alias_streamed);
        *self.shell_state.debug_trap_command.borrow_mut() = saved_trap_command;
        self.error_trap_running = false;
        self.trap_saved_exit_value = prev_saved_exit;
        self.trap_saved_exit_context = prev_saved_context;
        // GNU builtins/exit.def (exit_or_logout): an `exit` inside ANY trap
        // action unwinds to the top level immediately (the EXITBLTIN jump) —
        // the unwind passes THROUGH run_error_trap, and the shell
        // terminates with that status instead of returning to the
        // interrupted flow (execute_cmd.c:5537 honors the jump by ending
        // execution with last_command_exit_value; eval.c reader_loop ends
        // the run). The discarded execute_ast result used to swallow that
        // unwind, so `trap 'echo ETRAP; exit 42' ERR; false; echo never`
        // printed ETRAP + never and exited 0; GNU prints ETRAP and exits 42
        // (rubash#524). A bare `exit` exits with the trap-time $?, which
        // GNU resolves from trap_saved_exit_value (builtins/common.c
        // get_exitstat; probe: GNU exits 1). Only the exit unwind
        // propagates — other action-level states keep the historical
        // swallow here.
        let exit_unwind = matches!(result, Err(ExecuteError::ExitCode(_)));
        // rubash#1164: a `return` inside the ERR action targets the frame
        // the trap fired in (GNU runs the action in the interrupted
        // function's context, so return_catch is live): when the trap
        // fired while a function or sourced script was on the stack
        // (trap_return_context > 0), the Return unwind must reach that
        // frame instead of being swallowed — the function's remaining
        // body is skipped and it returns the action's status. At the top
        // level the return builtin itself errors and produces no unwind,
        // so a stray Return here keeps the historical swallow.
        let return_unwind = matches!(result, Err(ExecuteError::Return(_)))
            && (self.shell_state.function_depth + self.source_nest_depth) > 0;
        // rubash#1405: `break`/`continue` in the ERR trap action must
        // propagate to the enclosing loop (GNU honors it). The trap runs
        // in the loop's context, so the Break/Continue unwind must reach
        // the loop frame instead of being swallowed.
        let loop_unwind = matches!(
            result,
            Err(ExecuteError::Break(_)) | Err(ExecuteError::Continue(_))
        );
        self.exit_code = saved_exit;
        if exit_unwind || return_unwind || loop_unwind {
            // For Break/Continue from the trap, preserve the failing
            // command's status (saved_exit) as the loop's exit status.
            // The loop's Break handler uses break_failure_status if set.
            if loop_unwind {
                self.break_failure_status.set(Some(saved_exit));
            }
            return result;
        }
        Ok(())
    }

    /// Run the SIGCHLD trap once for one reaped child. GNU bash processes
    /// each child-death notification after waitpid and runs a set SIGCHLD
    /// trap at the next boundary (trap8.sub: four CHLD firings for the
    /// reaped background subshell, two background sleeps, and the
    /// foreground sleep).
    pub(crate) fn run_sigchld_trap_for_reaped_child(&mut self) -> Result<(), ExecuteError> {
        if self.signal_trap_running {
            // A child death notification arrived while the SIGCHLD trap
            // action is already running: queue it, and the outermost run
            // re-executes the action once per pending notification (bash
            // delivers one SIGCHLD per reaped child; trap.tests expects
            // three catches for three background jobs).
            self.sigchld_notifications_pending
                .set(self.sigchld_notifications_pending.get() + 1);
            return Ok(());
        }
        if self.shell_state.subshell_depth.get() > 0 {
            return Ok(());
        }
        let Some(action) =
            crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "SIGCHLD")
        else {
            return Ok(());
        };
        if action.is_empty() {
            return Ok(());
        }
        let saved_exit = self.exit_code;
        self.signal_trap_running = true;
        let mut result: Result<(), ExecuteError> = Ok(());
        loop {
            let tokens = crate::lexer::tokenize(&action);
            let ast = crate::parser::parse(&tokens);
            result = self.with_xtrace_indirection(|executor| executor.execute_ast(&ast));
            if result.is_err() {
                break;
            }
            if self.sigchld_notifications_pending.get() == 0 {
                break;
            }
            self.sigchld_notifications_pending
                .set(self.sigchld_notifications_pending.get() - 1);
        }
        self.signal_trap_running = false;
        match result {
            Ok(()) => self.exit_code = saved_exit,
            Err(error) => return Err(error),
        }
        Ok(())
    }

    pub(crate) fn run_function_return_trap(&mut self) -> Result<Option<i32>, ExecuteError> {
        // GNU execute_cmd.c:5295 (execute_function): while the DEBUG trap is
        // executing (signal_in_progress (DEBUG_TRAP)), a function does not
        // inherit the RETURN trap at all (restore_default_signal(RETURN_TRAP)
        // plus the unwind-protect restore), so no RETURN trap fires when the
        // DEBUG trap handler function itself returns
        // (dbg-support.tests: no "return lineno" lines for print_debug_trap).
        if self.debug_trap_running {
            return Ok(None);
        }
        // GNU execute_cmd.c:5377-5407: the fire itself is unconditional —
        // run_return_trap runs whatever trap_list[RETURN_TRAP] currently
        // holds. Inheritance scoping happens at ENTRY (execute_function
        // removes the inherited trap for untraced functions; see
        // execute_function_dispatch), not here, so a nested body install is
        // global last-set-wins: the outer return fires it too (rubash#1105).
        // The old function-scoped gate (__RUBASH_RETURN_TRAP_FUNCTION:
        // outer-must-match) dropped the outer fire.
        //
        // A `return N` inside the RETURN action REFIRES the trap in GNU:
        // trap.c _run_trap_internal rethrows through return_catch and
        // execute_cmd.c:5377 runs the trap again because the
        // from_return_trap suppression setter is #if 0'd out, so
        // `trap 'return 9' RETURN` spins forever (oracle: timeout kill,
        // rubash#1106). Mirror it: a Return out of the action restarts
        // the fire with that status as the pending return value; when a
        // fire completes without returning, the function returns the
        // LAST thrown status (finite-chain oracle: fires that return 9
        // while n<3, then complete, leave rc=9 with n=3 — not the body's
        // original status and not the completing fire's $?).
        let mut trap_status = None;
        loop {
            match self.run_return_trap() {
                Err(ExecuteError::Return(status)) => trap_status = Some(status),
                Ok(()) => break,
                Err(error) => return Err(error),
            }
        }
        Ok(trap_status)
    }

    pub(crate) fn apply_command_output_redirects(
        &mut self,
        cmd: &CommandNode,
        ast: &mut Ast,
    ) -> Result<(), ExecuteError> {
        self.apply_command_output_redirects_inner(cmd, ast, true)
    }

    pub(crate) fn apply_inherited_command_output_redirects(
        &mut self,
        cmd: &CommandNode,
        ast: &mut Ast,
    ) -> Result<(), ExecuteError> {
        self.apply_command_output_redirects_inner(cmd, ast, false)
    }

    /// GNU execute_cmd.c applies a compound command's redirections in the
    /// parent context before the body runs: a `cd` inside `( ... ) > f`
    /// cannot relocate `f`. The body commands here re-open the injected
    /// redirect target at their own execution time, so a relative target
    /// must be anchored to the current directory now (niubash#118).
    pub(in crate::executor) fn anchor_compound_redirect_target(&self, target: &str) -> String {
        if is_closed_redirect_target(target)
            || redirect_target_fd(target).is_some()
            || is_null_device(target)
            || target.starts_with('&')
        {
            return target.to_string();
        }
        let path = shell_path_to_windows(target, &self.shell_state.env_vars);
        if path.is_absolute() {
            return target.to_string();
        }
        // G-02: relative redirect targets display-resolve against the
        // session cwd, not the process cwd.
        shell_display_path(&self.shell_state.cwd.join(&path).to_string_lossy())
    }

    fn apply_command_output_redirects_inner(
        &mut self,
        cmd: &CommandNode,
        ast: &mut Ast,
        prepare_targets: bool,
    ) -> Result<(), ExecuteError> {
        // The stdio mirror fields are fd-blind parser conveniences:
        // `3>&1`/`2>&1` also land in `redirect_out`. Only an fd-1 (or
        // default) entry is a stdout redirect; numbered entries propagate
        // through splice_numbered_output_redirects_in_order below instead.
        //
        // rubash#940: the stdio arms run in PARSE ORDER (their position in
        // `command.redirects`), matching GNU do_redirection_internal
        // (redir.c:767-955) and the brace path's prepare_brace_group_redirects
        // — `( :; ) 2>cap.txt > /missing` must perform the `2>` open
        // (truncate/create side effect) BEFORE the `>` open fails, and the
        // failure report must see the binding it established. The old
        // field-order arms ran `>` first, so the earlier `2>` side effects
        // never happened.
        for arm in compound_stdio_arms_in_parse_order(cmd) {
            match arm {
                CompoundStdioArm::Out => self.apply_compound_out_arm(cmd, ast, prepare_targets)?,
                CompoundStdioArm::Append => {
                    self.apply_compound_append_arm(cmd, ast, prepare_targets)?
                }
                CompoundStdioArm::Err => self.apply_compound_err_arm(cmd, ast, prepare_targets)?,
                CompoundStdioArm::ErrAppend => {
                    self.apply_compound_err_append_arm(cmd, ast, prepare_targets)?
                }
            }
        }

        // Numbered (fd-prefixed) output redirects — `3>&1`, `10>f` — bind
        // real fd-table entries for the body's duration through
        // open_compound_output_redirects (with_command_input_redirects /
        // with_compound_output_redirects), matching GNU
        // do_redirection_internal which dups/opens the group's descriptors
        // before the body runs (redir.c:767-955). Leaves then resolve
        // `1>&3` against the group's binding rather than a spliced-in
        // re-dup of their own fd 1 (which mis-binds inside pipelines).

        Ok(())
    }

    /// rubash#2246: the entry half of a compound command's stdio
    /// redirections, WITHOUT the per-leaf splice. GNU runs
    /// do_redirections ONCE at compound entry (execute_cmd.c:854-883 —
    /// save_redirects (854), REDIRECTIONS saved per command (866), the
    /// do_redirections call itself (883); dispose at 1762,
    /// dispose_command_redirects): the body then executes against the
    /// bound fd table and no leaf ever re-opens the target. The fd-table
    /// binding half of that model already lives in
    /// with_command_input_redirects (open_compound_output_redirects
    /// walks cmd.redirects in list order and binds each target once);
    /// what the splice arms additionally performed, and what this pass
    /// preserves verbatim, is the parse-order ENTRY side effects that
    /// must land before the binder and the body run:
    ///
    /// * target expansion in parse order (each arm's expansion can
    ///   consume state — command substitutions, arithmetic — that a
    ///   later arm or the body observes), including the failglob abort
    ///   (subst.c:12663-12668);
    /// * the ONE create/truncate of each non-append file target
    ///   (create_redirect_output_prepared), which also records the
    ///   noclobber prepared target the binder consumes instead of
    ///   re-creating — including the rubash#940 ordering where an
    ///   earlier arm's create precedes a later arm's failure.
    ///
    /// Every consumer of the old injected entries already treated them
    /// as "realized by the fd-table binding — skip" (the ordered walk
    /// in redirection.rs, plan_external_stdio, the builtin setup walk),
    /// and command_output_fd_state seeds from the fd table, so dropping
    /// the splice changes no leaf's visible endpoint — only who pays
    /// for it: one entry binding instead of a deep clone of the whole
    /// compound subtree plus one splice walk per wrapper call.
    ///
    /// Error contract is the apply pass's: an IoError from a create
    /// propagates to the wrapper, which adjudicates it through
    /// note_compound_redirect_failure (rubash#846) exactly as before.
    pub(in crate::executor) fn prepare_compound_output_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        for arm in compound_stdio_arms_in_parse_order(cmd) {
            match arm {
                CompoundStdioArm::Out => self.prepare_compound_out_arm(cmd)?,
                CompoundStdioArm::Append => self.prepare_compound_append_arm(cmd)?,
                CompoundStdioArm::Err => self.prepare_compound_err_arm(cmd)?,
                CompoundStdioArm::ErrAppend => self.prepare_compound_err_append_arm(cmd)?,
            }
        }
        Ok(())
    }

    fn prepare_compound_out_arm(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        if let Some(redirect) = cmd
            .redirect_out
            .as_deref()
            .filter(|r| r.fd.unwrap_or(1) == 1)
        {
            // The expansion reports the word as written, exactly as in
            // apply_compound_out_arm; the binder re-expands the same
            // word when it opens the target (the prepared-target record
            // below is what keeps the create itself single).
            let expanded = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: abort before creating.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            // Mirror of the apply arm's prepare step: `>&1`-style dup
            // targets are entry-bound already (never created), and
            // closed targets (`>&-`) create nothing.
            if !is_closed_redirect_target(&expanded) && redirect_target_fd(&expanded).is_none() {
                self.create_redirect_output_prepared(&expanded, redirect.clobber)?;
            }
        }
        Ok(())
    }

    fn prepare_compound_append_arm(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        if let Some(redirect) = cmd.append.as_deref().filter(|r| r.fd.unwrap_or(1) == 1) {
            let _expanded = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: abort before binding.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            // The apply arm creates nothing for appends (the binder's
            // own open creates on demand); the expansion above is the
            // arm's only entry side effect.
        }
        Ok(())
    }

    fn prepare_compound_err_arm(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        if let Some(redirect) = cmd
            .redirect_err
            .as_deref()
            .filter(|r| r.fd.unwrap_or(2) == 2)
        {
            let expanded = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: abort before creating.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            // Mirror of the apply arm's prepare step, including the
            // /dev/null and closed-target carve-outs.
            if redirect_target_fd(&expanded).is_none()
                && !is_closed_redirect_target(&expanded)
                && !is_null_device(&expanded)
            {
                self.create_redirect_output_prepared(&expanded, redirect.clobber)?;
            }
        }
        Ok(())
    }

    fn prepare_compound_err_append_arm(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        if let Some(redirect) = cmd
            .redirect_err_append
            .as_deref()
            .filter(|r| r.fd.unwrap_or(2) == 2)
        {
            let _expanded = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: abort before binding.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
        }
        Ok(())
    }

    fn apply_compound_out_arm(
        &mut self,
        cmd: &CommandNode,
        ast: &mut Ast,
        prepare_targets: bool,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = cmd
            .redirect_out
            .as_deref()
            .filter(|r| r.fd.unwrap_or(1) == 1)
        {
            // GNU do_redirections opens the expanded word once, relative to
            // the cwd in force when the compound command starts; the
            // diagnostic reports that word (redir.c report_error on
            // redirectee.filename->word). The anchored spelling is only for
            // the propagated per-leaf redirect, which opens later and must
            // survive a `cd` inside the body (niubash#118).
            let expanded = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: abort before the target is
            // created/anchored and spliced into the body leaves.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            // fd-dup targets (`>&1`) are entry-time dup2s — see the
            // redirect_err guard above (rubash#170).
            if redirect_target_fd(&expanded).is_none() {
                if prepare_targets
                    && !is_closed_redirect_target(&expanded)
                    && redirect_target_fd(&expanded).is_none()
                {
                    self.create_redirect_output_prepared(&expanded, redirect.clobber)?;
                }
                let target = self.anchor_compound_redirect_target(&expanded);
                let append_redirect = Redirect {
                    fd: redirect.fd,
                    fd_var: redirect.fd_var.clone(),
                    operator: ">>".to_string(),
                    operator_metadata: Box::new(crate::parser::WordMetadata::new(
                        0,
                        ">>".to_string(),
                        ">>".to_string(),
                    )),
                    kind: crate::parser::RedirectKind::Append,
                    target_metadata: Box::new(crate::parser::WordMetadata::new(
                        0,
                        target.clone(),
                        target.clone(),
                    )),
                    target,
                    append: true,
                    clobber: false,
                };
                apply_stdout_append_redirect(&mut ast.commands, &append_redirect);
            }
        }
        Ok(())
    }

    fn apply_compound_append_arm(
        &mut self,
        cmd: &CommandNode,
        ast: &mut Ast,
        _prepare_targets: bool,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = cmd.append.as_deref().filter(|r| r.fd.unwrap_or(1) == 1) {
            let expanded = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: abort before anchoring.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            let target = self.anchor_compound_redirect_target(&expanded);
            let append_redirect = Redirect {
                fd: redirect.fd,
                fd_var: redirect.fd_var.clone(),
                operator: redirect.operator.clone(),
                operator_metadata: redirect.operator_metadata.clone(),
                kind: redirect.kind.clone(),
                target_metadata: Box::new(crate::parser::WordMetadata::new(
                    0,
                    target.clone(),
                    target.clone(),
                )),
                target,
                append: true,
                clobber: false,
            };
            apply_stdout_append_redirect(&mut ast.commands, &append_redirect);
        }
        Ok(())
    }

    fn apply_compound_err_arm(
        &mut self,
        cmd: &CommandNode,
        ast: &mut Ast,
        prepare_targets: bool,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = cmd
            .redirect_err
            .as_deref()
            .filter(|r| r.fd.unwrap_or(2) == 2)
        {
            let expanded = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: abort before the target is
            // created/anchored (see the redirect_out arm above).
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            // An fd-dup target (`2>&1`) is a dup2 GNU applies ONCE at
            // compound entry (redir.c do_redirection_internal), snapshotted
            // by open_compound_output_redirects' scoped fd-table binding.
            // Splicing a per-leaf re-dup instead re-evaluates fd 1 at every
            // leaf — after the body's own `exec > f` rebound it, `>&2`
            // writes land in f while GNU keeps them on the original stdout
            // (rubash#170 nest4). Same guard on every stdio mirror branch
            // below.
            if redirect_target_fd(&expanded).is_none() {
                if prepare_targets
                    && !is_closed_redirect_target(&expanded)
                    && redirect_target_fd(&expanded).is_none()
                    && !is_null_device(&expanded)
                {
                    self.create_redirect_output_prepared(&expanded, redirect.clobber)?;
                }
                let target = self.anchor_compound_redirect_target(&expanded);
                let append_redirect = Redirect {
                    fd: redirect.fd,
                    fd_var: redirect.fd_var.clone(),
                    operator: "2>>".to_string(),
                    operator_metadata: Box::new(crate::parser::WordMetadata::new(
                        0,
                        "2>>".to_string(),
                        "2>>".to_string(),
                    )),
                    kind: crate::parser::RedirectKind::Append,
                    target_metadata: Box::new(crate::parser::WordMetadata::new(
                        0,
                        target.clone(),
                        target.clone(),
                    )),
                    target,
                    append: true,
                    clobber: false,
                };
                apply_stderr_append_redirect(&mut ast.commands, &append_redirect);
            }
        }
        Ok(())
    }

    fn apply_compound_err_append_arm(
        &mut self,
        cmd: &CommandNode,
        ast: &mut Ast,
        _prepare_targets: bool,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = cmd
            .redirect_err_append
            .as_deref()
            .filter(|r| r.fd.unwrap_or(2) == 2)
        {
            let expanded = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: abort before anchoring.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            // fd-dup targets (`2>>&1` is still a dup2 in GNU — the append
            // marker on an fd target is ignored) bind at entry; see the
            // redirect_err guard above (rubash#170).
            if redirect_target_fd(&expanded).is_none() {
                let target = self.anchor_compound_redirect_target(&expanded);
                let append_redirect = Redirect {
                    fd: redirect.fd,
                    fd_var: redirect.fd_var.clone(),
                    operator: redirect.operator.clone(),
                    operator_metadata: redirect.operator_metadata.clone(),
                    kind: redirect.kind.clone(),
                    target_metadata: Box::new(crate::parser::WordMetadata::new(
                        0,
                        target.clone(),
                        target.clone(),
                    )),
                    target,
                    append: true,
                    clobber: false,
                };
                apply_stderr_append_redirect(&mut ast.commands, &append_redirect);
            }
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_exec(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        if exec_has_only_redirects(cmd) {
            if let Some(status) = self.execute_stdio_only_exec_redirect_scoped(cmd)? {
                return Ok(status);
            }
        }

        // GNU applies the redirections to a temporary undo scope while
        // `exec` parses its options (redir.c do_redirections with
        // REDIRECTIONS_UNDOABLE for a builtin that may fail): the option
        // diagnostics land in the redirected targets, but when option
        // parsing fails (EX_BADUSAGE) the redirections are undone and the
        // shell keeps its original stdout/stderr (rubash#1230). Only a
        // bare `exec` whose options parse cleanly (the redirects-only
        // branch above) makes them permanent. Snapshot both persistence
        // stores and roll back on EX_BADUSAGE.
        if self.exec_has_no_command_operand_after_expansion(cmd) {
            let saved_env_vars = self.shell_state.env_vars.clone();
            let saved_fd_table = self.fd_table.clone();
            self.execute_stdio_only_exec_redirect_scoped(cmd)?;
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let status = crate::builtins::exec::execute_with_io(
                &cmd.words[1..],
                &self.shell_state.env_vars,
                &self.shell_state.cwd,
                &mut stdout,
                &mut stderr,
                // No spawn happens on this path (a bare `exec` with
                // redirects only reports option diagnostics); inherit is
                // the no-op value.
                Stdio::inherit(),
            )?;
            // GNU exec.def reports option errors through builtin_error,
            // which carries the script:line context prefix
            // (redir.tests:53: "./redir.tests: line 53: exec: -1: invalid
            // option"). The builtin emits the bare shell-name form; swap
            // the prefix when running inside a script.
            let prefix = self.diagnostic_prefix();
            if prefix != "rubash: " {
                let text = String::from_utf8_lossy(&stderr).into_owned();
                if text.starts_with("rubash: ") {
                    stderr.clear();
                    stderr.extend_from_slice(
                        format!("{}{}", prefix, &text["rubash: ".len()..]).as_bytes(),
                    );
                }
            }
            // Route the buffered output while the temporary redirections
            // are still in effect (the diagnostics belong in the redirect
            // targets), then undo them if the exec failed.
            self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
            if status == 2 {
                self.shell_state.env_vars = saved_env_vars;
                self.fd_table = saved_fd_table;
            }
            return Ok(status);
        }

        // GNU redir.c do_redirections → redir_varassign: `exec cmd {var}>f`
        // still allocates the descriptor and assigns var persistently
        // before running the operand. The stdio-only paths above already
        // covered the no-operand forms; apply the fd_var redirects here so
        // a real operand sees them (and they persist after it exits).
        if self.apply_dynamic_fd_var_redirects(cmd, false)? {
            // rubash#2125: a `{var}` apply failure is a do_redirections
            // failure (redir_varassign), reached BEFORE exec_builtin's
            // search/execve — so in POSIX mode it takes the
            // execute_cmd.c:4865-4875 EX_REDIRFAIL jump like every
            // other special-builtin redirect failure, and outside
            // POSIX mode the command simply completes at status 1.
            // It must NOT flow into finish_exec_replacement as a bare
            // status: that path is exec.def's failed_exec, which
            // covers only the search/execve failure and would exit a
            // non-POSIX shell (and wrongly defer to `execfail` in
            // POSIX mode). The mark tells execute_exec_command to
            // complete at status 1 without the replacement finish.
            if self.posix_special_redirect_failure_exits(cmd) {
                return Err(ExecuteError::ExitCode(1));
            }
            self.exec_do_redirections_failed.set(true);
            return Ok(1);
        }

        // GNU redir.c do_redirection_internal applies the command's fd-0
        // input redirections (`exec cat < f`, here-strings, heredocs,
        // `<&N` dups) BEFORE execve, and subst.c command_substitute keeps
        // the shell's fd 0 open inside command substitutions — the exec'd
        // image reads exactly the descriptor the shell carried. rubash#575:
        // the exec builtin's spawn sites never set a child stdin, and std's
        // Command::output() defaults stdin to Stdio::null(), so a
        // `$(exec cat)` read EOF where WSL bash 5.3.0 reads the outer
        // stdin. Bind the node's input redirects for the spawn's duration —
        // the same primitives with_command_input_redirects uses for
        // compound bodies — resolve the child handle from the bound state
        // (see exec_inherited_stdin), then unwind the shell's fd 0.
        if let Some(message) = self.compound_stdin_open_failure(cmd) {
            let mut stderr = Vec::new();
            writeln!(&mut stderr, "{message}")?;
            self.write_redirected_command_stderr(cmd, &stderr)?;
            self.exit_code = 1;
            return Ok(1);
        }
        // rubash#2030: the same dup preflight the compound wrappers run —
        // an `exec` whose `<&N` source is unbound fails EBADF (`N: Bad
        // file descriptor`, redir.c:1179 dup2 + redir.c:149-158
        // redirection_error), the command is not exec'd, and the shell
        // continues at status 1 (GNU exec.def exec_builtin: a failed
        // do_redirections returns before the execve).
        if let Some((index, name)) = self.compound_dup_bad_fd(cmd) {
            self.abandon_compound_for_dup_bad_fd::<()>(cmd, index, &name)?;
            return Ok(1);
        }
        // `<&N` dups and numbered input redirects are real descriptor
        // operations on the spawn's fd table (the numbered path already
        // applies them for compounds; open_compound_numbered_input_redirects
        // deliberately covers fd-0 dups).
        let saved_numbered = self.open_compound_numbered_input_redirects(cmd)?;
        let guard = self.bind_command_fd0_input(cmd)?;
        let status = self.execute_exec_spawn(cmd);
        if let Some(guard) = guard {
            self.restore_command_fd0_input(guard);
        }
        self.restore_compound_numbered_input_redirects(saved_numbered);
        // rubash#1493: an `exec`ed command that died by a signal inside a
        // subshell — GNU's forked subshell was replaced by the command,
        // and the PARENT shell's waitchld reports the job's death at the
        // next notification point (jobs.c:4625 notify_of_job_status:
        // `<pid> File size limit exceeded ( <subshell text> )`). The exec
        // spawn helpers flatten the wait status to 128+signal, so the
        // death is handed back through the builtin's record. At the top
        // level there is nothing to report (GNU's shell is gone — it WAS
        // the exec'd process), hence the subshell gate. The notice text
        // is the WHOLE subshell command recorded at subshell entry
        // (shell_state.subshell_source_text) — GNU's dead job IS the
        // parens command, so pretty_print_job renders all of it. The
        // last-member convention of signal_notice_command_text remains
        // as the fallback for entries that recorded no text (respawned
        // child executors).
        if let Some((pid, signal)) = crate::builtins::exec::take_last_exec_signal_death() {
            if self.shell_state.subshell_depth.get() > 0 {
                let text = self
                    .shell_state
                    .subshell_source_text
                    .clone()
                    .unwrap_or_else(|| self.signal_notice_command_text(cmd));
                self.note_child_signal_death(pid, signal, text, true);
            }
        }
        status
    }

    /// rubash#1175: the gates for a true execve at the tail arm of
    /// execute_exec_spawn. Every condition is an invariant the new
    /// image depends on; any failure keeps the spawn-and-wait
    /// simulation, whose observable behavior is already GNU-shaped
    /// apart from the PID itself:
    /// - the host binary opted in (a library host's process is not
    ///   the shell's to replace);
    /// - this executor owns the process image — fork-copy clones
    ///   (comsub / pipeline stage / read) share the real shell's
    ///   process and must never execve it away;
    /// - no flat-subshell frame is live on this executor
    ///   (subshell_depth > 0): `( exec ... )` runs in-process here,
    ///   while GNU's forked subshell is what gets replaced;
    /// - fds 0/1/2 are exactly the process's own inherited descriptors
    ///   and no other modeled fd is open: a modeled redirect, capture,
    ///   or numbered fd has no real descriptor to hand the new image.
    ///   (The command's own output redirects are handled by the
    ///   earlier arms of execute_exec_spawn and never reach here; its
    ///   fd-0 input redirects were bound by execute_exec's guard and
    ///   show up in the fd table, failing the fd-0 check.)
    /// - no /dev/fd or /dev/std* word operands: those name modeled
    ///   descriptors the materializer below resolves per-spawn.
    #[cfg(unix)]
    fn exec_image_replacement_applies(&self, cmd: &CommandNode) -> bool {
        crate::builtins::exec::process_replacement_enabled()
            && self.owns_process_image
            && self.shell_state.subshell_depth.get() == 0
            && self.stdout_capture.is_none()
            && self.stderr_capture.is_none()
            && !crate::executor::shell_options::stdout_capture_active()
            && matches!(
                self.fd_table.write_endpoint(1),
                Some(FdWriteEndpoint::Stdout) | Some(FdWriteEndpoint::File(_))
            )
            && matches!(
                self.fd_table.write_endpoint(2),
                Some(FdWriteEndpoint::Stderr) | Some(FdWriteEndpoint::File(_))
            )
            && !self.fd_table.is_closed(0)
            && !self.fd_table.is_closed(1)
            && !self.fd_table.is_closed(2)
            && match self.fd_table.entries.get(&0) {
                Some(entry) => {
                    !entry.closed
                        && matches!(
                            entry.read,
                            Some(FdReadEndpoint::InheritedProcessStdin)
                                | Some(FdReadEndpoint::File(_))
                        )
                }
                None => true,
            }
            && self
                .fd_table
                .entries
                .iter()
                .all(|(fd, entry)| *fd <= 2 || entry.closed)
            && !cmd.words[1..].iter().any(|word| {
                word.starts_with("/dev/fd/")
                    || matches!(word.as_str(), "/dev/stdin" | "/dev/stdout" | "/dev/stderr")
            })
    }

    /// rubash#1175: attempt a true image replacement for an exec whose
    /// own stdio redirects were just resolved by a spawn arm (the arm
    /// opened the target file itself). `stdout`/`stderr` carry the arm's
    /// binding for that descriptor; the other descriptors resolve from
    /// the live fd table exactly like the tail arm. None keeps the
    /// arm's spawn-and-wait simulation.
    #[cfg(unix)]
    fn try_exec_image_replacement_with_stdio(
        &mut self,
        cmd: &CommandNode,
        stdout: Option<Stdio>,
        stderr: Option<Stdio>,
    ) -> Option<Result<i32, ExecuteError>> {
        if !self.exec_image_replacement_applies(cmd) {
            return None;
        }
        let stdin = self.exec_inherited_stdin().ok()?;
        let stdout = match stdout {
            Some(stdio) => stdio,
            None => self.exec_inherited_stdio(1).ok()?,
        };
        let stderr = match stderr {
            Some(stdio) => stdio,
            None => self.exec_inherited_stdio(2).ok()?,
        };
        // rubash#2179: a successful replacement ends this process
        // lifetime; complete any outstanding output-procsub feeds
        // first. (A decline inside replace_process_image leaves the
        // shell running with the payloads already delivered — the
        // state GNU is in at this point by construction.)
        self.drain_procsub_feeds();
        let status = crate::builtins::exec::replace_process_image(
            &cmd.words[1..],
            &self.shell_state.env_vars,
            &self.shell_state.cwd,
            Some((stdin, stdout, stderr)),
        )?;
        Some(match status {
            Ok(status) => Ok(status),
            Err(error) => Err(ExecuteError::from(error)),
        })
    }

    /// The spawn arms of execute_exec, run with the command's fd-0 input
    /// redirects already bound (see the guard in execute_exec). Every arm
    /// passes the child stdin resolved by exec_inherited_stdin explicitly —
    /// none of the spawn helpers may rely on std's defaults.
    fn execute_exec_spawn(&mut self, cmd: &CommandNode) -> Result<i32, ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_redirect_target(redirect);
            // GNU redir.c:832-838: `exec cmd >&WORD` with a non-numeric
            // WORD translates to r_err_and_out - the child's stdout AND
            // stderr go to WORD. The raw target keeps the `&` dup marker,
            // so strip it before opening (redir4.sub: exec >&$fd).
            if redirect.fd.unwrap_or(1) == 1 && redirect.fd_var.is_none() && target.starts_with('&')
            {
                let path = target.strip_prefix('&').unwrap_or(&target).to_string();
                let mut file = self.create_redirect_output(&path, redirect.clobber)?;
                // r_err_and_out: the child's stdout and stderr both go to
                // WORD, so the builtin's own diagnostics follow the file too.
                let child_stdout = Stdio::from(file.try_clone()?);
                let child_stderr = Stdio::from(file.try_clone()?);
                let mut child_diag = file.try_clone()?;
                let (exec_args, dev_ops, _proc_ops) = self.materialize_exec_operands(
                    cmd,
                    &cmd.words[1..],
                    crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
                    crate::executor::dev_fd_operands::DevOperandStdout::Path(
                        shell_path_to_windows(&path, &self.shell_state.env_vars),
                    ),
                );
                let status = crate::builtins::exec::execute_with_child_stdio(
                    &exec_args,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                    &mut file,
                    &mut child_diag,
                    child_stdout,
                    child_stderr,
                    self.exec_inherited_stdin()?,
                )?;
                self.finish_dev_fd_operands(dev_ops);
                return Ok(status);
            }
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            let child_stdout = Stdio::from(file.try_clone()?);
            // rubash#1175: `exec cmd >file` — GNU replaces the image with
            // fd 1 bound to the file; try that before the simulation.
            #[cfg(unix)]
            if let Some(result) = self.try_exec_image_replacement_with_stdio(
                cmd,
                Some(Stdio::from(file.try_clone()?)),
                None,
            ) {
                return result;
            }
            let (exec_args, dev_ops, _proc_ops) = self.materialize_exec_operands(
                cmd,
                &cmd.words[1..],
                crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
                crate::executor::dev_fd_operands::DevOperandStdout::Path(shell_path_to_windows(
                    &target,
                    &self.shell_state.env_vars,
                )),
            );
            let child_stdin = self.exec_inherited_stdin()?;
            let status = with_exec_diag_stderr(&mut self.stderr_capture, |diag| {
                crate::builtins::exec::execute_with_child_stdio(
                    &exec_args,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                    &mut file,
                    diag,
                    child_stdout,
                    Stdio::inherit(),
                    child_stdin,
                )
                .map_err(ExecuteError::from)
            })?;
            self.finish_dev_fd_operands(dev_ops);
            return Ok(status);
        }

        if let Some(redirect) = &cmd.append {
            let target = self.expand_redirect_target(redirect);
            let mut file = open_redirect_append(
                &self.session_path(shell_path_to_windows(&target, &self.shell_state.env_vars)),
            )?;
            let child_stdout = Stdio::from(file.try_clone()?);
            // rubash#1175: `exec cmd >>file` — same replacement attempt
            // as the `>file` arm above.
            #[cfg(unix)]
            if let Some(result) = self.try_exec_image_replacement_with_stdio(
                cmd,
                Some(Stdio::from(file.try_clone()?)),
                None,
            ) {
                return result;
            }
            let (exec_args, dev_ops, _proc_ops) = self.materialize_exec_operands(
                cmd,
                &cmd.words[1..],
                crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
                crate::executor::dev_fd_operands::DevOperandStdout::Path(shell_path_to_windows(
                    &target,
                    &self.shell_state.env_vars,
                )),
            );
            let child_stdin = self.exec_inherited_stdin()?;
            let status = with_exec_diag_stderr(&mut self.stderr_capture, |diag| {
                crate::builtins::exec::execute_with_child_stdio(
                    &exec_args,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                    &mut file,
                    diag,
                    child_stdout,
                    Stdio::inherit(),
                    child_stdin,
                )
                .map_err(ExecuteError::from)
            })?;
            self.finish_dev_fd_operands(dev_ops);
            return Ok(status);
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_redirect_target(redirect);
            // GNU redir.c r_duplicating_output: a `2>&N` / `2>&-` target is
            // a descriptor dup, never a file name — opening it literally
            // wrote the exec-failure diagnostic into a file named "&1"
            // and the message vanished (rubash#1144).
            if let Some(dup) = target.strip_prefix('&') {
                // The close form `2>&-` is exactly "-"; the move form
                // `2>&1-` strips one trailing '-'.
                let dup_fd = if dup == "-" {
                    dup
                } else {
                    dup.strip_suffix('-').unwrap_or(dup)
                };
                let (exec_args, dev_ops, _proc_ops) = self.materialize_exec_operands(
                    cmd,
                    &cmd.words[1..],
                    crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
                    crate::executor::dev_fd_operands::DevOperandStdout::FdTable,
                );
                let status = match dup_fd {
                    "-" => crate::builtins::exec::execute_with_child_stdio(
                        &exec_args,
                        &self.shell_state.env_vars,
                        &self.shell_state.cwd,
                        &mut crate::executor::GlobalStdout,
                        &mut std::io::sink(),
                        Stdio::inherit(),
                        Stdio::null(),
                        self.exec_inherited_stdin()?,
                    )?,
                    "1" => crate::builtins::exec::execute_with_io(
                        &exec_args,
                        &self.shell_state.env_vars,
                        &self.shell_state.cwd,
                        &mut crate::executor::GlobalStdout,
                        &mut crate::executor::GlobalStdout,
                        self.exec_inherited_stdin()?,
                    )?,
                    _ => {
                        let child_stdout = self.exec_inherited_stdio(1)?;
                        let dup_stdio = dup_fd
                            .parse::<u32>()
                            .ok()
                            .and_then(|fd| self.exec_inherited_stdio(fd).ok())
                            .unwrap_or_else(Stdio::inherit);
                        let child_stdin = self.exec_inherited_stdin()?;
                        with_exec_diag_stderr(&mut self.stderr_capture, |diag| {
                            crate::builtins::exec::execute_with_child_stdio(
                                &exec_args,
                                &self.shell_state.env_vars,
                                &self.shell_state.cwd,
                                &mut crate::executor::GlobalStdout,
                                diag,
                                child_stdout,
                                dup_stdio,
                                child_stdin,
                            )
                            .map_err(ExecuteError::from)
                        })?
                    }
                };
                self.finish_dev_fd_operands(dev_ops);
                return Ok(status);
            }
            let mut file = self.create_redirect_output(&target, redirect.clobber)?;
            let child_stderr = Stdio::from(file.try_clone()?);
            // rubash#1175: `exec cmd 2>file` — replacement attempt with
            // fd 2 bound to the file (the capture case declines via the
            // gates inside the helper).
            #[cfg(unix)]
            if let Some(result) = self.try_exec_image_replacement_with_stdio(
                cmd,
                None,
                Some(Stdio::from(file.try_clone()?)),
            ) {
                return result;
            }
            let (exec_args, dev_ops, _proc_ops) = self.materialize_exec_operands(
                cmd,
                &cmd.words[1..],
                crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
                crate::executor::dev_fd_operands::DevOperandStdout::FdTable,
            );
            // GNU redir.c do_redirections: `exec cmd 2>/dev/null` rebinds
            // ONLY fd 2 — the child's fd 1 keeps whatever the shell carried,
            // which inside a command substitution is the capture pipe
            // (subst.c:7143). Stdio::inherit() here instead gave the child
            // the CONSOLE stdout, leaking its output past the capture
            // (modernish goodsh.sh `_Msh_Gsh_doTestShell`, rubash#252).
            // Route through the capture-aware sink under the same
            // fd1_lands_in_capture condition as the no-redirect arm below.
            let fd1_lands_in_capture = self.fd_table.write_endpoint(1)
                == Some(FdWriteEndpoint::Stdout)
                && (self.stdout_capture.is_some()
                    || crate::executor::shell_options::stdout_capture_active());
            let status = if fd1_lands_in_capture {
                crate::builtins::exec::execute_with_io(
                    &exec_args,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                    &mut crate::executor::GlobalStdout,
                    &mut file,
                    self.exec_inherited_stdin()?,
                )?
            } else {
                crate::builtins::exec::execute_with_child_stdio(
                    &exec_args,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                    &mut crate::executor::GlobalStdout,
                    &mut file,
                    Stdio::inherit(),
                    child_stderr,
                    self.exec_inherited_stdin()?,
                )?
            };
            self.finish_dev_fd_operands(dev_ops);
            return Ok(status);
        }

        if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_redirect_target(redirect);
            let mut file = open_redirect_append(
                &self.session_path(shell_path_to_windows(&target, &self.shell_state.env_vars)),
            )?;
            let child_stderr = Stdio::from(file.try_clone()?);
            // rubash#1175: `exec cmd 2>>file` — same replacement attempt
            // as the `2>file` arm above.
            #[cfg(unix)]
            if let Some(result) = self.try_exec_image_replacement_with_stdio(
                cmd,
                None,
                Some(Stdio::from(file.try_clone()?)),
            ) {
                return result;
            }
            let (exec_args, dev_ops, _proc_ops) = self.materialize_exec_operands(
                cmd,
                &cmd.words[1..],
                crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
                crate::executor::dev_fd_operands::DevOperandStdout::FdTable,
            );
            // Same capture-aware fd-1 routing as the redirect_err arm above
            // (GNU redir.c: the append redirect on fd 2 never rebinds fd 1).
            let fd1_lands_in_capture = self.fd_table.write_endpoint(1)
                == Some(FdWriteEndpoint::Stdout)
                && (self.stdout_capture.is_some()
                    || crate::executor::shell_options::stdout_capture_active());
            let status = if fd1_lands_in_capture {
                crate::builtins::exec::execute_with_io(
                    &exec_args,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                    &mut crate::executor::GlobalStdout,
                    &mut file,
                    self.exec_inherited_stdin()?,
                )?
            } else {
                crate::builtins::exec::execute_with_child_stdio(
                    &exec_args,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                    &mut crate::executor::GlobalStdout,
                    &mut file,
                    Stdio::inherit(),
                    child_stderr,
                    self.exec_inherited_stdin()?,
                )?
            };
            self.finish_dev_fd_operands(dev_ops);
            return Ok(status);
        }

        self.apply_no_output_builtin_redirects(cmd)?;
        // rubash#1175: with every gate in exec_image_replacement_applies
        // satisfied, replace this process's image for real (GNU
        // shell_execve) instead of simulating it with a child whose PID
        // differs from the shell's. A None result keeps the simulation
        // below for the shapes replace_process_image declines.
        #[cfg(unix)]
        if self.exec_image_replacement_applies(cmd) {
            // rubash#1175 redirected forms: hand the new image the fd
            // table's live bindings resolved to real descriptors (an
            // ambient `exec >log` or a group redirect bound fd 1 to a
            // File endpoint — GNU's image inherits exactly that file).
            let stdio = match (
                self.exec_inherited_stdin(),
                self.exec_inherited_stdio(1),
                self.exec_inherited_stdio(2),
            ) {
                (Ok(stdin), Ok(stdout), Ok(stderr)) => Some((stdin, stdout, stderr)),
                _ => None,
            };
            // If a live binding could not be resolved to a real
            // descriptor, keep the simulation rather than replacing
            // with the wrong (inherited) descriptors.
            if let Some(stdio) = stdio {
                // rubash#2179: complete outstanding output-procsub
                // feeds before a replacement ends this process
                // lifetime (see try_exec_image_replacement_with_stdio).
                self.drain_procsub_feeds();
                if let Some(status) = crate::builtins::exec::replace_process_image(
                    &cmd.words[1..],
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                    Some(stdio),
                ) {
                    return Ok(status?);
                }
            }
        }
        // GNU exec.def exec_builtin -> shell_execve (execute_cmd.c:6126):
        // the exec'd program inherits the shell's fd 1 — inside a command
        // substitution that descriptor IS the capture pipe (subst.c:7143
        // command_substitute dup2'd it before the body ran), so
        // `exec prog` at the end of a script captured via `$(script)` feeds
        // the capture, not the console (rubash#193: rbenv's
        // `exec rbenv---version`). An fd-1 Stdout endpoint with an active
        // in-process capture has no inheritable OS handle, so run the child
        // piped and forward its bytes through the capture-aware sink — the
        // same shape run_external_command_substitution uses for external
        // comsub bodies.
        let fd1_lands_in_capture = self.fd_table.write_endpoint(1) == Some(FdWriteEndpoint::Stdout)
            && (self.stdout_capture.is_some()
                || crate::executor::shell_options::stdout_capture_active());
        let (exec_args, dev_ops, _proc_ops) = self.materialize_exec_operands(
            cmd,
            &cmd.words[1..],
            crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
            crate::executor::dev_fd_operands::DevOperandStdout::FdTable,
        );
        let status = if fd1_lands_in_capture {
            // `exec cmd 2>&1` inside the substitution resolves fd 2 to the
            // same Stdout endpoint — its diagnostics join the capture too;
            // a plain fd 2 stays the terminal stream.
            if self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout) {
                crate::builtins::exec::execute_with_io(
                    &exec_args,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                    &mut crate::executor::GlobalStdout,
                    &mut crate::executor::GlobalStdout,
                    self.exec_inherited_stdin()?,
                )?
            } else {
                // rubash#2395: the child's piped stderr belongs to the
                // executor's fd-2 stream — under an armed stderr
                // capture that is the capture, not the process stderr
                // (the pipeline driver's redirect application owns
                // those bytes; writing them to the terminal dropped
                // the stage's `2>/dev/null` on the floor).
                let child_stdin = self.exec_inherited_stdin()?;
                with_exec_diag_stderr(&mut self.stderr_capture, |diag| {
                    crate::builtins::exec::execute_with_io(
                        &exec_args,
                        &self.shell_state.env_vars,
                        &self.shell_state.cwd,
                        &mut crate::executor::GlobalStdout,
                        diag,
                        child_stdin,
                    )
                    .map_err(ExecuteError::from)
                })?
            }
        } else {
            // GNU exec.def: `exec cmd` runs cmd in the shell's current fd
            // context — a group/ambient redirect already bound on the fd
            // table (`${THIS_SH} script >file`, `{ exec cmd; } >file`) must
            // reach the spawned child. Inheriting raw process stdio
            // bypasses the bound file and leaks the child's output to the
            // console while leaving the redirect target empty (niubash
            // run-test gate).
            let child_stdout = self.exec_inherited_stdio(1)?;
            let child_stderr = self.exec_inherited_stdio(2)?;
            let child_stdin = self.exec_inherited_stdin()?;
            with_exec_diag_stderr(&mut self.stderr_capture, |diag| {
                crate::builtins::exec::execute_with_child_stdio(
                    &exec_args,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                    &mut crate::executor::GlobalStdout,
                    diag,
                    child_stdout,
                    child_stderr,
                    child_stdin,
                )
                .map_err(ExecuteError::from)
            })?
        };
        self.finish_dev_fd_operands(dev_ops);
        Ok(status)
    }

    /// Resolve the concrete child stdio for an `exec`'d fd from the fd
    /// table. GNU's fork-exec model inherits whatever the group's
    /// redirections already dup2'd onto the descriptor (redir.c
    /// do_redirections runs before exec_builtin), so a bound file endpoint
    /// or an `N>&M` dup chain must be realized as the child's real std
    /// handle. `M` dup targets are resolved transitively (`2>&1` follows
    /// fd 1's endpoint, sharing its open file description).
    fn exec_inherited_stdio(&self, fd: u32) -> Result<Stdio, ExecuteError> {
        self.exec_stdio_for_endpoint(fd, 0)
    }

    /// Resolve the concrete child stdin for an `exec`'d program from the
    /// fd table's endpoint 0 — the read-side twin of
    /// exec_inherited_stdio(1)/(2). GNU exec.def exec_builtin ->
    /// shell_execve replaces the shell image in place, so the exec'd
    /// program keeps fd 0 exactly as do_redirections bound it (redir.c),
    /// and subst.c command_substitute dups ONLY fd 1 for the capture pipe,
    /// leaving fd 0 inherited — a `$(exec cat)` therefore reads the outer
    /// stdin (WSL bash 5.3.0 matrix: pipe, file and read-then-exec shapes
    /// all feed the child; rubash#575). std's Command::output() defaults
    /// stdin to Stdio::null() unless .stdin() is set, so this channel must
    /// be resolved and passed explicitly at every exec spawn site.
    fn exec_inherited_stdin(&self) -> Result<Stdio, ExecuteError> {
        // FUNCTION_STDIN stages bound virtual text (command_input_scope's
        // bind_command_fd0_input, pipeline staging); the unread remainder
        // is what the exec'd image would read off fd 0. background_stdio
        // materializes it the same way for async subshells.
        if let Some(remaining) = self.function_stdin_remaining() {
            let bytes = crate::executor::substitution_metadata::shell_text_to_raw_bytes(&remaining);
            let file = self.materialize_exec_stdin_bytes(&bytes)?;
            return Ok(file.map(Stdio::from).unwrap_or(Stdio::null()));
        }
        match self.fd_table.read_endpoint(0) {
            Some(FdReadEndpoint::File(file))
            | Some(FdReadEndpoint::CoprocStdout { fd: file, .. }) => {
                let dup = crate::fd::duplicate_handle_inheritable(file.handle)?;
                Ok(Stdio::from(crate::fd::handle_to_file(dup)))
            }
            // Virtual descriptors have no OS handle: materialize the unread
            // remainder on a temp file — the shared-open-file-description
            // shape GNU's dup2 hands the image.
            Some(FdReadEndpoint::Text(_)) | Some(FdReadEndpoint::ProcessSubstitution(_)) => {
                match self.fd_table.input_snapshot_bytes(0) {
                    Some((data, offset)) => {
                        let file =
                            self.materialize_exec_stdin_bytes(&data[offset.min(data.len())..])?;
                        Ok(file.map(Stdio::from).unwrap_or(Stdio::null()))
                    }
                    None => Ok(Stdio::null()),
                }
            }
            Some(FdReadEndpoint::InheritedProcessStdin) => Ok(Stdio::inherit()),
            // rubash#1937: a directory endpoint has no handle to
            // hand the image; the null device is the closest
            // spawnable state (cross-boundary residual).
            Some(FdReadEndpoint::Directory) => Ok(Stdio::null()),
            // fd 0 closed (`exec 0<&-`): the closest spawnable state to
            // GNU's inherited-but-unreadable descriptor.
            None if self.fd_table.has_entry(0) => Ok(Stdio::null()),
            None => Ok(Stdio::inherit()),
        }
    }

    /// Write `bytes` to a fresh temp file and open it for the child.
    /// process_substitution_temp_path names are unique per call. An empty
    /// payload spawns nothing — Stdio::null() yields the same EOF.
    ///
    /// rubash#2395: the carrier must not outlive the descriptor that
    /// owns it. The write-then-open form left the named file in $TMPDIR
    /// forever — no exec arm knows the path, so nobody could reap it,
    /// and every `exec` over a virtual fd 0 (pipeline-stage text,
    /// heredoc, here-string) leaked one. The carrier is therefore
    /// anonymous from birth, the same shape as GNU's here-document
    /// temp (redir.c:423 here_document_to_fd, an unnamed fd): on Unix
    /// the name is unlinked while the descriptor is open; on Windows
    /// the handle carries FILE_FLAG_DELETE_ON_CLOSE (the
    /// synthetic_proc_write_sink discipline, shell_options.rs). The
    /// file dies with the last descriptor — the spawn's wait, or the
    /// replacement image's own lifetime.
    fn materialize_exec_stdin_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<Option<std::fs::File>, ExecuteError> {
        if bytes.is_empty() {
            return Ok(None);
        }
        let path = self.process_substitution_temp_path()?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            // FILE_FLAG_DELETE_ON_CLOSE.
            options.custom_flags(0x0400_0000);
        }
        let mut file = options.open(&path).map_err(ExecuteError::IoError)?;
        #[cfg(unix)]
        {
            // The descriptor stays valid after the name is unlinked.
            let _ = std::fs::remove_file(&path);
        }
        use std::io::{Seek as _, SeekFrom, Write as _};
        file.write_all(bytes).map_err(ExecuteError::IoError)?;
        file.seek(SeekFrom::Start(0))
            .map_err(ExecuteError::IoError)?;
        Ok(Some(file))
    }

    fn exec_stdio_for_endpoint(&self, fd: u32, depth: u32) -> Result<Stdio, ExecuteError> {
        if depth > 4 {
            return Ok(Stdio::inherit());
        }
        if self.fd_table.has_entry(fd) && !self.fd_table.is_open_for_write(fd) {
            return Ok(Stdio::null());
        }
        match self.fd_table.write_endpoint(fd) {
            Some(FdWriteEndpoint::File(file_fd)) => {
                let dup = crate::fd::duplicate_handle_inheritable(file_fd.handle)?;
                Ok(Stdio::from(crate::fd::handle_to_file(dup)))
            }
            Some(FdWriteEndpoint::Stdout) if fd != 1 => self.exec_stdio_for_endpoint(1, depth + 1),
            // A Stderr endpoint on fd != 2 is a dup2 snapshot of the
            // ORIGINAL stderr object (rubash#279): resolving through the
            // live fd 2 would send a child's `>&3` output into the group's
            // later `2>/dev/null`. Stdio::inherit() passes the process's
            // real stderr handle, which is exactly the object the dup
            // copied (a bound fd 2 clones its File entry instead of
            // carrying the Stderr marker, so the marker really means the
            // implicit process stderr).
            Some(FdWriteEndpoint::Stderr) if fd != 2 => Ok(Stdio::inherit()),
            _ => Ok(Stdio::inherit()),
        }
    }

    /// rubash#1021 helper: spawn a `<(source)` body as a child shell
    /// writing to a pipe, the way GNU process_substitute forks instead
    /// of collecting output first (subst.c:6362). The child gets the
    /// coproc spawn's environment treatment (shell vars plus the full
    /// function table as BASH_FUNC entries), is registered for
    /// reaping, and becomes `$!`. Returns the pipe's read end, or None
    /// when the spawn could not be set up; callers then fall back to
    /// synchronous collection.
    pub(in crate::executor) fn spawn_input_process_substitution_child(
        &mut self,
        source: &str,
        line: Option<usize>,
    ) -> Option<Rc<FileFd>> {
        use std::process::{Command, Stdio};
        let exe = std::env::var_os("CARGO_BIN_EXE_rubash")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| "rubash".into());
        let Ok((reader, writer)) = std::io::pipe() else {
            return None;
        };
        let mut child = Command::new(&exe);
        child.arg("-c").arg(source);
        // rubash#2356: gather the respawn environment into a Vec
        // first (a Command's env cannot be read back), so the
        // state-channel spill can move oversized entries onto the
        // state-file transport before anything is applied — the
        // fork-context stamp scales with the function table and a
        // single entry past MAX_ARG_STRLEN fails the execve. Entry
        // order is preserved exactly; spill + apply happen just
        // before the spawn below.
        let mut env_entries: Vec<(String, String)> = Vec::new();
        for (key, value) in &self.shell_state.env_vars {
            if !key.starts_with("__RUBASH_")
                || super::compound_exec::rubash_spawn_inherited_state(key)
            {
                env_entries.push((key.clone(), value.clone()));
            }
        }
        for (name, body) in &self.shell_state.functions {
            if !is_exportable_function_name(name) {
                continue;
            }
            let def_redirects = self
                .shell_state
                .function_def_infos
                .get(name)
                .map(|info| info.def_redirects.as_slice())
                .unwrap_or(&[]);
            env_entries.push((
                exported_function_env_name(name),
                exported_function_env_value(&body.commands, def_redirects),
            ));
        }
        // rubash#1184 f4: seed the child's line frame from the
        // constructing `exec` command's line, the `&` respawn's protocol
        // — GNU's process_substitute fork (subst.c:6362) inherits the
        // parent's script name and line_number, so a body diagnostic
        // (`3: Bad file descriptor` for a redirect failing inside the
        // body) attributes to the construction line, not line 1. The
        // body here is the raw `-c` source (functions ride BASH_FUNC
        // entries, no prelude), so the offset is exactly line - 1.
        // __RUBASH_SHELL_PID marks the spawn as an internal respawn —
        // the marker the child init requires before it keeps
        // __RUBASH_SCRIPT_NAME at all (init.rs), the same stamp the
        // output-substitution spawn carries.
        env_entries.push(("__RUBASH_SHELL_PID".to_string(), self.shell_pid.to_string()));
        // rubash#2136: the provenance stamp the child's init validates
        // against its actual parent (init.rs).
        env_entries.push((
            super::RESPAWN_SPAWNER_PID_KEY.to_string(),
            super::spawner_stamp_value(),
        ));
        // subst.c:6362 process_substitute: the producer is a forked
        // child one subshell level deeper than its parent, like the
        // consumer form (external_setup.rs) — this builder never
        // stamped the depth, so the child's $BASH_SUBSHELL read 0
        // where GNU reads parent + 1.
        env_entries.push((
            "__RUBASH_SUBSHELL_DEPTH".to_string(),
            (self.shell_state.subshell_depth.get() + 1).to_string(),
        ));
        // rubash#1582: the same fork makes the producer's own pid the
        // pid of the seeded level (variables.c:1511 get_bashpid =
        // getpid), so the BASHPID origin base rides with the depth —
        // the consumer form (external_setup.rs) stamps the pair, and
        // without the base this child's $BASHPID read its real pid + 1
        // while its own children observed the real pid as $PPID.
        env_entries.push((
            "__RUBASH_BG_BASHPID_BASE".to_string(),
            (self.shell_state.subshell_depth.get() + 1).to_string(),
        ));
        // rubash#2032: process_substitute's producer is a forked child
        // (subst.c:6362), so it inherits the parent's positional
        // parameters (POSITIONAL_PARAMS_TRANSPORT). Before this stamp
        // the body saw $# = 0 and `"$@"` collapsed to one empty word —
        // the bats zero-test-execution regression.
        env_entries.push((
            super::compound_exec::POSITIONAL_PARAMS_TRANSPORT.to_string(),
            self.positional_params_transport_value(),
        ));
        // rubash#2058: the fork-context stamp (call-stack views,
        // PIPESTATUS cell, ARGV bottom state) — same fork, same
        // transport set (see fork_context_transport_entries).
        for (key, value) in self.fork_context_transport_entries() {
            env_entries.push((key, value));
        }
        // rubash#2456: process_substitute's fork (subst.c:6412-6445)
        // never resets login_shell — the reset sites are
        // execute_cmd.c:1636-1637 and 5474-5475, neither on this
        // path — so the child inherits the parent's CURRENT login
        // global: the effective state the logout gate reads
        // (invocation_login && !subshell_login_cleared), which is why
        // a procsub constructed inside a `( )` region is born
        // non-login while one at the top of a login shell (or inside
        // a command substitution, subst.c:7143) keeps login-ness.
        if self.invocation_login() && !self.shell_state.subshell_login_cleared.get() {
            env_entries.push((
                super::compound_exec::FORK_LOGIN_SHELL_TRANSPORT.to_string(),
                "1".to_string(),
            ));
        }
        if let Some(script_name) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
            env_entries.push(("__RUBASH_SCRIPT_NAME".to_string(), script_name.clone()));
        }
        if let Some(line) = line {
            env_entries.push(("__RUBASH_CURRENT_LINE".to_string(), line.to_string()));
            env_entries.push((
                "__RUBASH_LINE_OFFSET".to_string(),
                line.saturating_sub(1).to_string(),
            ));
        }
        // rubash#2212: GNU's process_substitute fork (subst.c:6362)
        // inherits the shell's WHOLE ambient fd table. The output
        // form's spawn adjudicates it through ambient_procsub_spawn_fd;
        // the input form must too — an `exec 3>file` / `exec 3<&1` fd
        // opened by an earlier command reaches a `<(...)` body that
        // writes `>&3`. Unix-only here: this spawn is a plain
        // Command::spawn, whose child inherits the inheritable dups
        // through the raw fd space; the Windows handle-whitelist
        // transport belongs to the spawn_whitelisted form the output
        // spawn uses. Parent-side dups close once the spawn lands.
        let mut ambient_fd_dups: Vec<crate::fd::HANDLE> = Vec::new();
        #[cfg(unix)]
        let mut ambient_pins: Vec<(std::os::unix::io::RawFd, u32)> = Vec::new();
        #[cfg(unix)]
        for (fd, entry) in &self.fd_table.entries {
            if *fd < 3 || entry.closed {
                continue;
            }
            match self.ambient_procsub_spawn_fd(*fd) {
                super::external_setup::ProcsubSpawnFd::File { read, write } => {
                    if let Some(file) = &read {
                        if let Ok(dup) = crate::fd::duplicate_handle_inheritable(file.handle) {
                            env_entries
                                .push((format!("__RUBASH_FD_HANDLE_{fd}"), format!("{dup:#x}")));
                            env_entries.push((
                                format!("__RUBASH_FD_PATH_{fd}"),
                                file.path.to_string_lossy().into_owned(),
                            ));
                            ambient_fd_dups.push(dup);
                            ambient_pins.push((dup, *fd));
                        }
                    }
                    if let Some(file) = &write {
                        if let Ok(dup) = crate::fd::duplicate_handle_inheritable(file.handle) {
                            env_entries
                                .push((format!("__RUBASH_FD_WHANDLE_{fd}"), format!("{dup:#x}")));
                            env_entries.push((
                                format!("__RUBASH_FD_WPATH_{fd}"),
                                file.path.to_string_lossy().into_owned(),
                            ));
                            ambient_fd_dups.push(dup);
                            ambient_pins.push((dup, *fd));
                        }
                    }
                }
                super::external_setup::ProcsubSpawnFd::CarrierWrite(path) => {
                    if let Ok(file) = std::fs::OpenOptions::new().append(true).open(&path) {
                        let handle = crate::fd::into_handle(file);
                        if let Ok(dup) = crate::fd::duplicate_handle_inheritable(handle) {
                            env_entries
                                .push((format!("__RUBASH_FD_WHANDLE_{fd}"), format!("{dup:#x}")));
                            env_entries.push((
                                format!("__RUBASH_FD_WPATH_{fd}"),
                                path.to_string_lossy().into_owned(),
                            ));
                            ambient_fd_dups.push(dup);
                            ambient_pins.push((dup, *fd));
                        }
                        crate::fd::close_handle(handle);
                    }
                }
                _ => {}
            }
        }
        // rubash#2356: spill + apply the gathered environment (see
        // the gather above).
        let argv_bytes = exe.as_os_str().len() + source.len() + 8;
        let (env_entries, state_temp) = self.spill_respawn_env_entries(env_entries, argv_bytes);
        for (key, value) in env_entries {
            child.env(key, value);
        }
        child.stdin(Stdio::inherit());
        child.stdout(writer);
        child.stderr(Stdio::inherit());
        child.current_dir(&self.shell_state.cwd);
        // A foreign child ignores the __RUBASH_FD_* env stamps: it
        // needs the descriptors themselves at their slot numbers
        // (rubash#1626's pin transport, reused). The pins are the
        // inheritable dups above; dup2 in pre_exec lands each on its
        // slot after stdio setup, just before exec.
        #[cfg(unix)]
        if !ambient_pins.is_empty() {
            use std::os::unix::process::CommandExt;
            let pins = ambient_pins;
            unsafe {
                child.pre_exec(move || super::external_inner::pin_ambient_fds_in_child(&pins));
            }
        }
        match child.spawn() {
            Ok(child_proc) => {
                for dup in &ambient_fd_dups {
                    crate::fd::close_handle(*dup);
                }
                let pid = child_proc.id();
                self.background_children
                    .insert(pid, crate::fd::SpawnedChild::from(child_proc));
                // rubash#1952: mark the pid as a streamed procsub
                // producer so the boundary reap can spend its one
                // bounded grace wait on it (job_builtins.rs).
                self.procsub_stream_children.insert(pid, false);
                // GNU process_substitute records the child in $!
                // (subst.c assigns last_made_pid).
                self.shell_state.last_background_pid = Some(pid);
                let file = crate::fd::handle_to_file(crate::fd::into_handle(reader));
                Some(FileFd::from_file(file, format!("<({source})").into()))
            }
            Err(_) => {
                for dup in &ambient_fd_dups {
                    crate::fd::close_handle(*dup);
                }
                if let Some(path) = &state_temp {
                    let _ = std::fs::remove_file(path);
                }
                None
            }
        }
    }

    /// rubash#1021: bind the read end of a spawned `<(source)` body on
    /// `fd` (see spawn_input_process_substitution_child). Returns false
    /// when the spawn could not be set up; the caller then falls back
    /// to synchronous collection.
    pub(in crate::executor) fn bind_input_process_substitution_child(
        &mut self,
        fd: u32,
        source: &str,
        line: Option<usize>,
    ) -> bool {
        let Some(file_fd) = self.spawn_input_process_substitution_child(source, line) else {
            return false;
        };
        self.set_fd_input_file(fd, file_fd, fd != 0);
        true
    }

    /// The no-operand exec forms (redirects only, or operands that expand
    /// to nothing) with the `builtin`-wrapper scoping of rubash#1412:
    /// plain `exec >f` persists its redirections, but `builtin exec >f`
    /// applies them for that command only — the target file is still
    /// created/truncated, yet the shell's fd state afterwards is the
    /// pre-command one (GNU: `builtin exec >f; echo x` writes x to the
    /// original stdout and f stays empty). Snapshot the fd table plus
    /// the env-side fd records (the closed-fd keys and any `{var}`
    /// bindings) and restore them after the persistent path runs.
    fn execute_stdio_only_exec_redirect_scoped(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<i32>, ExecuteError> {
        if self.builtin_wrapper_depth == 0 {
            return self.execute_stdio_only_exec_redirect(cmd);
        }
        let saved_fd_table = self.fd_table.clone();
        let mut saved_env: Vec<(String, Option<String>)> = Vec::new();
        let mut fds: Vec<u32> = cmd
            .redirects
            .iter()
            .map(|r| {
                r.fd.unwrap_or(if r.operator.starts_with('<') { 0 } else { 1 })
            })
            .collect();
        fds.sort_unstable();
        fds.dedup();
        for fd in fds {
            let key = super::command_text::fd_closed_key(fd);
            let value = self.shell_state.env_vars.get(&key).cloned();
            saved_env.push((key, value));
        }
        let mut var_names: Vec<String> = cmd
            .redirects
            .iter()
            .filter_map(|r| r.fd_var.clone())
            .collect();
        var_names.extend(
            cmd.heredoc_redirects
                .iter()
                .filter_map(|r| r.fd_var.clone()),
        );
        var_names.sort();
        var_names.dedup();
        for name in var_names {
            let value = self.shell_state.env_vars.get(&name).cloned();
            saved_env.push((name, value));
        }
        let result = self.execute_stdio_only_exec_redirect(cmd);
        self.fd_table = saved_fd_table;
        for (key, value) in saved_env {
            match value {
                Some(value) => {
                    self.shell_state.env_vars.insert(key, value);
                }
                None => {
                    self.shell_state.env_vars.remove(&key);
                }
            }
        }
        result
    }

    fn execute_stdio_only_exec_redirect(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<i32>, ExecuteError> {
        // GNU redir.c: `exec` with only redirections applies every
        // redirection left to right (do_redirection_internal over the whole
        // redirect list, redir.c:767+), keeping each descriptor open
        // persistently and undoing nothing (RX_ACTIVE without undo). The
        // parser mirrors every stdio shortcut field into cmd.redirects, so
        // iterating that list in source order covers `exec >file 2>&1`,
        // `exec 1>&3 2>&4` (redir4.sub:54), `exec 4>&1 >&3 3>&-`
        // (redir7.sub:23), `exec 4>&- 5>&-` (redir.tests:88) and
        // single-redirect forms alike.
        let mut handled = false;
        // rubash#2282: running set of the numbered slots this walk has
        // bound so far — a {var} allocation counts them as occupied
        // (see apply_dynamic_fd_var_redirects).
        let mut occupied_slots: std::collections::HashSet<u32> = std::collections::HashSet::new();
        // A compound command's redirect injected into cmd.redirects (see
        // GROUP_REDIRECT_INJECTED_MARK) is the ambient fd state GNU applies
        // before exec runs — it participates in `>&N` resolution but must
        // not be persisted, since exec only persists its own redirect list
        // (execute_cmd.c exec_builtin) and the group undoes its own
        // redirections (redir.c REDIRECTION_SAVEFD). Track those targets in
        // `ambient` instead of the persistent fd table (niubash#118:
        // `{ exec 10>&1; } >file` must not leave fd 1 pointing at file).
        let mut ambient: std::collections::BTreeMap<u32, String> =
            std::collections::BTreeMap::new();
        for redirect in &cmd.redirects {
            if let Some(fd) = redirect.fd {
                occupied_slots.insert(fd);
            }
            let target = self.expand_redirect_target(redirect);
            if crate::executor::support_names::is_injected_group_redirect(redirect) {
                let fd = redirect.fd.unwrap_or_else(|| match redirect.kind {
                    crate::parser::RedirectKind::Input
                    | crate::parser::RedirectKind::ReadWrite
                    | crate::parser::RedirectKind::DuplicateInput
                    | crate::parser::RedirectKind::CloseInput
                    | crate::parser::RedirectKind::HereString
                    | crate::parser::RedirectKind::HereDoc => 0,
                    _ => 1,
                });
                ambient.insert(fd, target);
                continue;
            }
            // Mark the redirect as handled before dispatching: every arm
            // below ends in a `continue`, which would otherwise bypass a
            // bottom-of-loop flag and make `exec >&file` fall through to
            // the legacy redirect_out shortcut with the raw `&word` target
            // (GNU redir.c do_redirection_internal applies the whole list;
            // exec with only redirections always ends with status 0 unless
            // a redirection itself fails).
            handled = true;
            // GNU redir.c do_redirection_internal → redir_varassign
            // (redir.c:1133-1166): `exec {var}>file` allocates a fresh
            // descriptor in list order and assigns it to var persistently
            // (exec redirections are never undone). A failed redirect stops
            // the list — do_redirections returns on the first error.
            if redirect.fd_var.is_some() {
                match self.execute_dynamic_fd_var_redirect(
                    redirect,
                    false,
                    false,
                    &occupied_slots,
                    &std::collections::HashSet::new(),
                    0,
                ) {
                    Ok(_) => {}
                    Err(ExecuteError::IoError(error)) => {
                        let mut stderr = Vec::new();
                        // GNU redir.c redirection_error (internal_error
                        // family) honors gnu_errfmt (rubash#1038).
                        writeln!(
                            &mut stderr,
                            "{}{}",
                            self.error_prefix(),
                            crate::posix_errors::message(&error)
                        )?;
                        self.write_default_stderr(&stderr)?;
                        self.exit_code = 1;
                        // rubash#2125: this inline arm is bare `exec`'s
                        // own do_redirections — a redir_varassign failure
                        // here is EX_REDIRFAIL (execute_cmd.c:5613), and
                        // `exec` is a special builtin, so a posixly_correct
                        // non-interactive shell takes the
                        // execute_cmd.c:4865-4875 jump at status 1 instead
                        // of completing the command. The diagnostic above
                        // is the redirection_error report GNU emits
                        // before the jump (redir.c:135).
                        if self.posix_special_redirect_failure_exits(cmd) {
                            return Err(ExecuteError::ExitCode(1));
                        }
                        return Ok(Some(1));
                    }
                    Err(error) => return Err(error),
                }
                continue;
            }
            match redirect.kind {
                crate::parser::RedirectKind::Output
                | crate::parser::RedirectKind::Append
                | crate::parser::RedirectKind::ClobberOutput
                | crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => {
                    let fd = redirect.fd.unwrap_or(1);
                    if is_closed_redirect_target(&target) {
                        if let Some(name) = redirect.fd_var.as_deref() {
                            self.close_dynamic_fd(name)?;
                        } else {
                            // GNU redir.c r_close_this: close(redirector)
                            // closes the descriptor wholesale — both
                            // directions die, whichever spelling (`>&-` or
                            // `<&-`) named it (rubash#1590).
                            self.close_persistent_fd(fd)?;
                            self.shell_state
                                .env_vars
                                .insert(fd_closed_key(fd), "1".to_string());
                        }
                        continue;
                    }
                    // rubash#2385: a PATH-spelled target naming a pipe
                    // READ end is an open, not a dup — the dup apply
                    // below would copy the read end's empty write side
                    // and strand every later write at EBADF, where
                    // GNU's open attaches to the pipe's other end.
                    // Only this shape diverts (a `&N` dup spelling and
                    // every other target keep the arms below).
                    if !super::execution_misc::redirect_target_is_dup_spelling(&target) {
                        if let Some(source_fd) =
                            super::execution_misc::dev_stdio_redirect_fd(&target)
                        {
                            if self.fd_table.output_endpoint(source_fd).is_none()
                                && self.fd_alias_complementary_write_available(source_fd)
                            {
                                if let Some(endpoint) =
                                    self.fd_alias_complementary_write_endpoint(source_fd)
                                {
                                    self.flush_persistent_output_process_substitution(fd)?;
                                    self.fd_table.open_output(fd, endpoint, fd >= 10);
                                    self.shell_state.env_vars.remove(&fd_closed_key(fd));
                                    self.record_output_fd_ledger(fd);
                                    continue;
                                }
                            }
                        }
                    }
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        // GNU redir.c: the dup/move apply is guarded by
                        // `redir_fd != redirector` — a self-dup/self-move
                        // is a no-op even for a closed fd, and a self-move
                        // must not close the fd (rubash#1236).
                        if source_fd == fd {
                            continue;
                        }
                        // dup2 onto an fd holding a process substitution
                        // closes the old description first (rubash#1336).
                        self.flush_persistent_output_process_substitution(fd)?;
                        self.copy_exec_output_fd_resolving_ambient(fd, source_fd, &ambient);
                        if move_source {
                            // GNU redir.c dup/move case: a move closes the
                            // source descriptor wholesale (close(redir_fd)).
                            self.close_persistent_fd(source_fd)?;
                        }
                        continue;
                    }
                    // GNU redir.c:832-838: exec >&WORD with a non-numeric
                    // WORD and redirector 1 translates to r_err_and_out -
                    // both stdout and stderr go to WORD
                    // (redir4.sub: exec >&${TMPDIR}/err-and-out).
                    if redirect.fd.unwrap_or(1) == 1
                        && redirect.fd_var.is_none()
                        && target.starts_with('&')
                        && redirect_target_fd(&target).is_none()
                    {
                        let path = target.strip_prefix('&').unwrap_or(&target).to_string();
                        if !is_null_device(&path) {
                            self.create_redirect_output(&path, redirect.clobber)?;
                        }
                        self.flush_persistent_output_process_substitution(1)?;
                        self.set_fd_output_file(1, path.clone(), false, false)?;
                        self.flush_persistent_output_process_substitution(2)?;
                        self.share_fd_output_file(1, 2, &path, false);
                        continue;
                    }
                    // rubash#2385: a spelling target naming a pipe
                    // READ end binds the pipe's other end — GNU's open
                    // of the alias attaches there (redir.c redir_open),
                    // while the literal open below would bind the
                    // process's own same-numbered descriptor. Only the
                    // complementary case diverts: a regular-file
                    // endpoint keeps the fresh-open path below, a
                    // socket keeps its ENXIO failure, and a source
                    // WITH a write endpoint keeps the dup serve the
                    // fall-through reaches.
                    if let Some(source_fd) = super::execution_misc::dev_stdio_redirect_fd(&target) {
                        if let Some(error) = self.fd_alias_spelling_open_error(source_fd) {
                            return Err(ExecuteError::from(crate::posix_errors::path_error(
                                &target, error,
                            )));
                        }
                        if self.fd_alias_reopen_path(source_fd).is_none()
                            && self.fd_table.output_endpoint(source_fd).is_none()
                        {
                            if let Some(endpoint) =
                                self.fd_alias_complementary_write_endpoint(source_fd)
                            {
                                self.flush_persistent_output_process_substitution(fd)?;
                                self.fd_table.open_output(fd, endpoint, fd >= 10);
                                self.shell_state.env_vars.remove(&fd_closed_key(fd));
                                self.record_output_fd_ledger(fd);
                                continue;
                            }
                        }
                    }
                    // rubash#2090: a quoted ">(...)" target is a literal
                    // filename (subst.c:11365-11372), not a span — only a
                    // parser-recorded span may bind a substitution here.
                    if super::external_setup::redirect_target_span_is_recorded(redirect)
                        && self.open_persistent_output_process_substitution(fd, &target)?
                    {
                        continue;
                    }
                    let append = matches!(
                        redirect.kind,
                        crate::parser::RedirectKind::Append
                            | crate::parser::RedirectKind::CombinedAppend
                    );
                    if !is_null_device(&target) && !append {
                        // noclobber/existing-file diagnostics live here;
                        // the real handle open in set_fd_output_file follows.
                        self.create_redirect_output(&target, redirect.clobber)?;
                    }
                    // Rebinding closes the descriptor's previous
                    // description first in GNU; run a pending process
                    // substitution's deferred body (rubash#1336).
                    self.flush_persistent_output_process_substitution(fd)?;
                    self.set_fd_output_file(fd, target.clone(), fd >= 10, append)?;
                    if matches!(
                        redirect.kind,
                        crate::parser::RedirectKind::CombinedOutput
                            | crate::parser::RedirectKind::CombinedAppend
                    ) {
                        // &>file / &>>file: stderr follows stdout through
                        // the same open file description (redir.c
                        // r_err_and_out / r_append_err_and_out).
                        self.flush_persistent_output_process_substitution(2)?;
                        self.share_fd_output_file(fd, 2, &target, fd >= 10);
                    }
                }
                crate::parser::RedirectKind::DuplicateOutput => {
                    let fd = redirect.fd.unwrap_or(1);
                    if is_closed_redirect_target(&target) {
                        self.close_persistent_fd(fd)?;
                        self.shell_state
                            .env_vars
                            .insert(fd_closed_key(fd), "1".to_string());
                        continue;
                    }
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        // Self-dup/self-move: no-op (see the output arm
                        // above; rubash#1236).
                        if source_fd == fd {
                            continue;
                        }
                        self.flush_persistent_output_process_substitution(fd)?;
                        self.copy_exec_output_fd_resolving_ambient(fd, source_fd, &ambient);
                        if move_source {
                            self.close_persistent_fd(source_fd)?;
                        }
                        continue;
                    }
                    if redirect.fd.unwrap_or(1) == 1
                        && redirect.fd_var.is_none()
                        && target.starts_with('&')
                    {
                        // GNU redir.c:832-838 r_duplicating_output_word
                        // translation (see above).
                        let path = target.strip_prefix('&').unwrap_or(&target).to_string();
                        if !is_null_device(&path) {
                            self.create_redirect_output(&path, redirect.clobber)?;
                        }
                        self.flush_persistent_output_process_substitution(1)?;
                        self.set_fd_output_file(1, path.clone(), false, false)?;
                        self.flush_persistent_output_process_substitution(2)?;
                        self.share_fd_output_file(1, 2, &path, false);
                        continue;
                    }
                    // Other non-numeric dup targets stay AMBIGUOUS_REDIRECT
                    // (redir.c:839-843); reject_ambiguous_redirects already
                    // reported them before exec ran.
                }
                crate::parser::RedirectKind::CloseOutput => {
                    let fd = redirect.fd.unwrap_or(1);
                    self.close_persistent_fd(fd)?;
                    self.shell_state
                        .env_vars
                        .insert(fd_closed_key(fd), "1".to_string());
                }
                crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite => {
                    let fd = redirect.fd.unwrap_or(0);
                    if is_closed_redirect_target(&target) {
                        self.close_persistent_fd(fd)?;
                        self.shell_state
                            .env_vars
                            .insert(fd_closed_key(fd), "1".to_string());
                        continue;
                    }
                    // rubash#2385: a ReadWrite PATH spelling naming a
                    // pipe read end is ONE O_RDWR open in GNU — the
                    // dup copy below would bind the read direction
                    // only and strand the write direction at EBADF.
                    // Copy the read side, then install the
                    // complementary write end. (A plain Input spelling
                    // keeps the copy alone: an input open of a read
                    // end IS the read end.)
                    if redirect.kind == crate::parser::RedirectKind::ReadWrite
                        && !super::execution_misc::redirect_target_is_dup_spelling(&target)
                    {
                        if let Some(source_fd) =
                            super::execution_misc::dev_stdio_redirect_fd(&target)
                        {
                            if self.fd_table.output_endpoint(source_fd).is_none()
                                && self.fd_alias_complementary_write_available(source_fd)
                            {
                                if let Some(endpoint) =
                                    self.fd_alias_complementary_write_endpoint(source_fd)
                                {
                                    self.copy_persistent_input_fd(fd, source_fd);
                                    self.fd_table.open_output(fd, endpoint, fd >= 10);
                                    self.shell_state.env_vars.remove(&fd_closed_key(fd));
                                    self.record_output_fd_ledger(fd);
                                    continue;
                                }
                            }
                        }
                    }
                    // rubash#2479: an fd-alias PATH spelling (/dev/fd/N,
                    // /dev/std*, /proc/self/fd/N) is an OPEN, not a dup —
                    // the dup applies below answer for it only because
                    // redirect_target_fd_and_move falls back to the alias
                    // table for words without `&`. GNU's open of the alias
                    // (redir.c redir_open over the magic link) binds a
                    // FRESH description of a regular-file target (offset
                    // 0; the source fd does not move) and a dup of every
                    // other endpoint kind — the compound binder's dispatch
                    // (command_input_scope.rs, rubash#2256). An alias whose
                    // endpoint is absent falls through to the ladder below,
                    // keeping the dangling-alias outcome it has today.
                    if !super::execution_misc::redirect_target_is_dup_spelling(&target) {
                        if let Some(source_fd) =
                            super::execution_misc::dev_stdio_redirect_fd(&target)
                        {
                            if self.fd_table.read_endpoint(source_fd).is_some() {
                                if let Some(error) = self.fd_alias_spelling_open_error(source_fd) {
                                    return Err(ExecuteError::from(
                                        crate::posix_errors::path_error(&target, error),
                                    ));
                                }
                                if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                                    self.set_fd_readwrite_file(fd, &target, fd != 0)
                                        .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                                } else if let Some(path) = self.fd_alias_reopen_path(source_fd) {
                                    match FileFd::open_read(path) {
                                        Ok(fresh) => self.set_fd_input_file(fd, fresh, fd != 0),
                                        Err(_) => self.copy_persistent_input_fd(fd, source_fd),
                                    }
                                } else {
                                    self.copy_persistent_input_fd(fd, source_fd);
                                }
                                continue;
                            }
                        }
                    }
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        // Self-dup/self-move: no-op (rubash#1236).
                        if source_fd == fd {
                            continue;
                        }
                        self.copy_persistent_input_fd(fd, source_fd);
                        if move_source {
                            self.close_persistent_fd(source_fd)?;
                        }
                        continue;
                    }
                    if let Some(source_fd) = redirect_target_fd(&target) {
                        self.copy_persistent_input_fd(fd, source_fd);
                        continue;
                    }

                    if let Some(source) = target
                        .strip_prefix("<(")
                        .and_then(|target| target.strip_suffix(')'))
                        // rubash#2090: value-text shape alone does not
                        // admit the span — GNU expand_word_internal
                        // (subst.c:11365-11372) refuses a `<(` whose `<`
                        // was quoted (Q_DOUBLE_QUOTES), so only a
                        // parser-recorded (unquoted) span is live; a
                        // quoted target falls through to the literal
                        // open below. Same recorded-span admission as
                        // every other materialization arm (rubash#1931).
                        .filter(|_| {
                            super::external_setup::redirect_target_span_is_recorded(redirect)
                        })
                    {
                        // rubash#1021: GNU process_substitute (subst.c)
                        // forks the body and binds the pipe read end
                        // immediately; collecting its output first (the
                        // fallback below) blocks `exec` until the
                        // producer exits — forever for `tail -f`. Spawn
                        // the body as a child shell on a real pipe
                        // instead (same shape as the coproc spawn).
                        if self.bind_input_process_substitution_child(fd, source, cmd.line) {
                            continue;
                        }
                        if let Some(input) = self.process_substitution_output(source) {
                            self.fd_table.open_input(
                                fd,
                                FdReadEndpoint::process_substitution(&input),
                                fd != 0,
                            );
                            self.set_fd_input_text(fd, input, fd != 0);
                            continue;
                        }
                    }

                    // The fd-0 spelling judgment is the registry's (issue
                    // rubash#1973): `fd_alias_target == Some(0)` carries
                    // exactly the canonical spellings the literal table
                    // matched — GNU rejects the leading-zero family on
                    // every face, so `exec 0</dev/fd/00` falls through to
                    // the filesystem open and fails ENOENT there.
                    if crate::special_paths::fd_alias_target(&target) == Some(0) {
                        self.fd_table.open_input(
                            fd,
                            FdReadEndpoint::InheritedProcessStdin,
                            fd != 0,
                        );
                        continue;
                    }

                    // The operator token keeps any numeric redirector prefix
                    // (`exec 6<>file` lexes as `6<>`), so match on the
                    // suffix (redir.tests: `exec 6<>$TMPDIR/bash-c` then
                    // `echo to c 1>&6`). Real handle on the slot: [N]<> is a
                    // single O_RDWR open file description (redir.c
                    // r_input_output) shared by both directions.
                    if redirect.operator.ends_with("<>") {
                        self.set_fd_readwrite_file(fd, &target, fd != 0)
                            .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                    } else if crate::special_paths::is_net_redirect_target(&target) {
                        // rubash#1026: `N</dev/tcp/host/port` also
                        // connects (redir.c netopen serves both
                        // directions); the socket binds as the read end.
                        let file = self.open_net_redirect(&target)?;
                        let file_fd = FileFd::from_file(file, target.clone().into());
                        self.set_fd_input_file(fd, file_fd, fd != 0);
                    } else if crate::special_paths::is_zero_device(&target)
                        || crate::special_paths::is_full_device(&target)
                    {
                        // rubash#1898: the zero/full devices read as
                        // an unbounded zero stream — bind the
                        // pump-fed source handle (a real handle, so
                        // the descriptor behaves like the device).
                        let file = Self::zero_device_file(&target)
                            .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                        let file_fd = FileFd::from_file(file, target.clone().into());
                        self.set_fd_input_file(fd, file_fd, fd != 0);
                    } else if crate::special_paths::is_random_device(&target) {
                        // niubash#758: the entropy devices read as
                        // an unbounded CSPRNG stream — the pump-fed
                        // random source, the zero arm's construction
                        // with a different byte source.
                        let file = Self::random_device_file(&target)
                            .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                        let file_fd = FileFd::from_file(file, target.clone().into());
                        self.set_fd_input_file(fd, file_fd, fd != 0);
                    } else if let Some(bytes) =
                        crate::special_paths::synthetic_proc_content(&target)
                    {
                        // Registry first (rubash#1917 exec-fd face): a
                        // synthetic /proc file has no filesystem file
                        // behind the mapped path on Windows, so the
                        // plain open below reported it ENOENT and the
                        // descriptor never bound. Serve the generated
                        // snapshot bytes through the fd byte channel —
                        // procfs static-snapshot semantics, the same
                        // shape the process-substitution fallback above
                        // binds.
                        self.set_fd_input_bytes(fd, bytes, fd != 0);
                    } else if let Some(link) = {
                        let (cwd_forms, root_forms) = self.proc_link_forms();
                        crate::special_paths::synthetic_link_target(
                            &target,
                            Some(&cwd_forms),
                            Some(&root_forms),
                        )
                    } {
                        // SyntheticLink (rubash#1900): the open goes
                        // to the resolved native target (the session
                        // cwd answers for self/cwd), exactly as the
                        // open ladder binds it.
                        // rubash#1937: a DIRECTORY target (self/cwd)
                        // has no openable handle on Windows, but
                        // GNU's bind succeeds — only reads fail
                        // (EISDIR). Bind the directory endpoint
                        // instead of failing the whole redirect at
                        // open. (Off Windows proc_unary_facts is a
                        // stub and the real open below serves.)
                        if crate::proc_vfs::proc_unary_facts(&target)
                            .is_some_and(|facts| facts.is_link && facts.link_target_is_dir)
                        {
                            self.set_fd_input_directory(fd, fd != 0);
                        } else {
                            let file = FileFd::open_read(std::path::PathBuf::from(&link.native))
                                .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                            self.set_fd_input_file(fd, file, fd != 0);
                        }
                    } else {
                        // G-02 session-cwd anchor (rubash#1887): see
                        // set_fd_readwrite_file — without it `exec
                        // 10<infile` after `cd` opens against the
                        // process cwd and reports ENOENT for a file in
                        // the shell's own cwd (redir8.sub's shape, once
                        // the session and process cwds diverge).
                        let path = self.session_path(shell_path_to_windows(
                            &target,
                            &self.shell_state.env_vars,
                        ));
                        let file = FileFd::open_read(path)
                            .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                        self.set_fd_input_file(fd, file, fd != 0);
                    }
                }
                crate::parser::RedirectKind::CloseInput => {
                    let fd = redirect.fd.unwrap_or(0);
                    self.close_persistent_fd(fd)?;
                    self.shell_state
                        .env_vars
                        .insert(fd_closed_key(fd), "1".to_string());
                }
                crate::parser::RedirectKind::DuplicateInput => {
                    let fd = redirect.fd.unwrap_or(0);
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        // Self-dup/self-move: no-op, succeeds even when
                        // the fd is closed (rubash#1236) — judge it before
                        // the is_open check below.
                        if source_fd == fd {
                            continue;
                        }
                        if self.fd_table.is_open(source_fd) {
                            self.copy_persistent_input_fd(fd, source_fd);
                            if move_source {
                                // `N<&M-` moves the descriptor wholesale:
                                // both directions of M close (redir.c
                                // dup_redirects move case).
                                let _ = self.close_persistent_fd(source_fd);
                            }
                        } else {
                            self.shell_state
                                .env_vars
                                .insert(fd_closed_key(fd), "1".to_string());
                            // GNU redirection_error (redir.c:149-161): an
                            // EBADF from a dup/move redirection names the
                            // *source* fd (redirectee.dest), not the target.
                            let _ = self.write_default_stderr(
                                format!(
                                    "{}{}: Bad file descriptor
",
                                    self.diagnostic_prefix(),
                                    source_fd
                                )
                                .as_bytes(),
                            );
                            self.exit_code = 1;
                        }
                    }
                }
                _ => {}
            }
            handled = true;
        }

        // `{var}<<EOF` heredocs live in heredoc_redirects, not redirects.
        // GNU redir.c make_here_document writes the body to a temp file and
        // redir_varassign binds the fresh fd to var persistently under exec
        // (vredir1.sub: `exec {v}<<-EOF` leaves v holding a readable fd).
        for redirect in &cmd.heredoc_redirects {
            if redirect.fd_var.is_none() {
                continue;
            }
            handled = true;
            // rubash#2282: heredoc entries apply after the plain list
            // here too, so every numbered slot counts as occupied.
            occupied_slots.extend(command_own_numbered_slots(cmd));
            match self.execute_dynamic_fd_var_heredoc(redirect, false, &occupied_slots) {
                Ok(_) => {}
                Err(ExecuteError::IoError(error)) => {
                    let mut stderr = Vec::new();
                    writeln!(
                        &mut stderr,
                        "{}{}",
                        self.diagnostic_prefix(),
                        crate::posix_errors::message(&error)
                    )?;
                    self.write_default_stderr(&stderr)?;
                    self.exit_code = 1;
                    return Ok(Some(1));
                }
                Err(error) => return Err(error),
            }
        }

        if cmd.here_string.is_some() || cmd.here_string_carrier.is_some() {
            // `exec <<<word`: a plain here-string lives in the command's
            // stdin mirror fields, not in heredoc_redirects
            // (parser/redirections.rs assign_here_string_redirect_raw), so
            // the heredoc path below never sees it. Materialize it exactly
            // like a command's staged stdin (command_input_redirect) and
            // bind it persistently to fd 0 (rubash#1413).
            let mut input = if cmd.here_string_carrier.is_some() {
                self.expand_here_string_mut_from_carrier(&cmd.here_string_carrier)
            } else {
                self.expand_here_string_mut(cmd.here_string.as_deref().unwrap_or_default())
            };
            input.push('\n');
            self.set_fd_input_text(0, input, false);
            return Ok(Some(0));
        }

        if let Some((fd, input)) = self.exec_heredoc_fd_input(cmd) {
            self.set_fd_input_text(fd, input, fd != 0);
            return Ok(Some(0));
        }

        if handled {
            // GNU applys the redirections and keeps `exec` itself as a
            // no-op with status 0 (redir.c / builtins/exec.def: `exec`
            // with only redirections).
            return Ok(Some(0));
        }
        Ok(None)
    }

    fn exec_heredoc_fd_input(&self, cmd: &CommandNode) -> Option<(u32, String)> {
        let redirect = cmd
            .heredoc_redirects
            .iter()
            .rev()
            // `{var}` heredocs are bound by the fd_var loop above; every
            // other heredoc/here-string with a body belongs to its explicit
            // fd, or to fd 0 when the redirect names no fd. The old
            // `fd.is_some()` gate silently dropped `exec <<EOF` and
            // `exec <<<word`, whose persistence is the whole point of the
            // no-operand exec form (GNU redir.c: a redirect-only exec makes
            // the redirection permanent; rubash#1413).
            .find(|redirect| redirect.fd_var.is_none() && redirect.body.is_some())?;
        let fd = redirect.fd.unwrap_or(0);
        let body = redirect.body.as_deref()?;
        let input = if let Some(carrier) = &redirect.body_carrier {
            if let crate::parser::StdinBody::Preexpanded(text) = carrier {
                text.clone()
            } else {
                // NeedsExpansion case: fall through to legacy checks
                if let Some(word) = body.strip_prefix(STORAGE_WORD_PREFIX) {
                    let mut input =
                        decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
                    input.push('\n');
                    input
                } else {
                    strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string()
                }
            }
        } else if let Some(word) = body.strip_prefix(STORAGE_WORD_PREFIX) {
            let mut input =
                decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
            input.push('\n');
            input
        } else if let Some(pre) = preexpanded_stdin_body(body) {
            pre.to_string()
        } else {
            strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string()
        };
        // A here-string's stored word is the lexer's transport text:
        // `$'p\nq\n'` carries ANSI_C_IFS_GUARD pairs around every decoded
        // blank (lexer/quotes.rs — field-split transport, GNU's CTLESC-
        // quoted bytes) and the DATA_* carriers hold its literal
        // quote/dollar bytes. Once the text crosses into the fd byte
        // channel there is no later strip boundary, so the guards leak
        // into every consumer of the descriptor (rubash#1208: `exec
        // 3<<<$'p\nq\n'` + `mapfile -u 3` stored "p<U+E401>" elements).
        // Resolve them at the entry boundary like the fd-0 here-string
        // path does (expand_here_string_mut's decode tail).
        let input = if redirect.here_string {
            crate::executor::markers::strip_ansi_c_ifs_guard(
                &crate::executor::markers::decode_word_position_carriers(&input),
            )
        } else {
            input
        };
        Some((fd, input))
    }

    /// `exec fd>&src` inside a redirected compound must resolve `src`
    /// against the ambient fd state the compound's own redirects created
    /// (GROUP_REDIRECT_INJECTED_MARK entries collect into `ambient`),
    /// without persisting those ambient writes. Follows alias chains and
    /// file targets; falls back to the persistent fd table when the source
    /// fd has no ambient entry (niubash#118).
    fn copy_exec_output_fd_resolving_ambient(
        &mut self,
        target_fd: u32,
        source_fd: u32,
        ambient: &std::collections::BTreeMap<u32, String>,
    ) {
        let mut seen = std::collections::BTreeSet::new();
        let mut current = source_fd;
        while let Some(target) = ambient.get(&current) {
            if !seen.insert(current) {
                break;
            }
            if is_closed_redirect_target(target) {
                let _ = self.close_persistent_fd(target_fd);
                self.shell_state
                    .env_vars
                    .insert(fd_closed_key(target_fd), "1".to_string());
                return;
            }
            if let Some((next_fd, _)) = redirect_target_fd_and_move(target) {
                current = next_fd;
                continue;
            }
            // dup semantics: point the fd at the same target without
            // re-opening/truncating — the compound's own open already
            // created and truncated the file (niubash#118: a fresh
            // create here erased earlier group output).
            if self.fd_table.dup_output(target_fd, current).is_err() {
                let path = shell_path_to_windows(target, &self.shell_state.env_vars);
                if let Ok(file) = FileFd::open_write(path, true, false) {
                    self.fd_table.open_output(
                        target_fd,
                        FdWriteEndpoint::File(file),
                        target_fd >= 10,
                    );
                    self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
                    self.shell_state
                        .env_vars
                        .insert(fd_output_key(target_fd), target.clone());
                }
            }
            return;
        }
        self.copy_persistent_output_fd(target_fd, current);
    }

    /// Writes the `__RUBASH_FD_*` write-side ledger for `fd` from whatever
    /// write endpoint is installed (or clears it). Used after dup/open.
    pub(in crate::executor) fn record_output_fd_ledger(&mut self, target_fd: u32) {
        match self.fd_table.output_endpoint(target_fd) {
            Some(FdWriteEndpoint::Stdout) => {
                self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
                self.shell_state
                    .env_vars
                    .insert(fd_output_key(target_fd), FD_STDOUT_TARGET.to_string());
                self.shell_state
                    .env_vars
                    .remove(&fd_output_process_substitution_key(target_fd));
            }
            Some(FdWriteEndpoint::Stderr) => {
                self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
                self.shell_state
                    .env_vars
                    .insert(fd_output_key(target_fd), FD_STDERR_TARGET.to_string());
                self.shell_state
                    .env_vars
                    .remove(&fd_output_process_substitution_key(target_fd));
            }
            Some(FdWriteEndpoint::File(file_fd)) => {
                self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
                self.shell_state.env_vars.insert(
                    fd_output_key(target_fd),
                    shell_display_path(&file_fd.path.to_string_lossy()),
                );
                self.shell_state
                    .env_vars
                    .remove(&fd_output_process_substitution_key(target_fd));
            }
            Some(FdWriteEndpoint::CoprocStdin { pid, .. }) => {
                self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
                self.shell_state.env_vars.insert(
                    fd_output_key(target_fd),
                    format!("{FD_COPROC_STDIN_TARGET_PREFIX}{pid}"),
                );
                self.shell_state
                    .env_vars
                    .remove(&fd_output_process_substitution_key(target_fd));
            }
            Some(FdWriteEndpoint::ProcessSubstitution { path, command, .. }) => {
                self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
                self.shell_state.env_vars.insert(
                    fd_output_key(target_fd),
                    shell_display_path(&path.to_string_lossy()),
                );
                self.shell_state
                    .env_vars
                    .insert(fd_output_process_substitution_key(target_fd), command);
            }
            Some(FdWriteEndpoint::FullDevice) => {
                // rubash#1898: the ledger records the device
                // spelling itself, so a respawned child re-derives
                // the class from the registry.
                self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
                self.shell_state
                    .env_vars
                    .insert(fd_output_key(target_fd), "/dev/full".to_string());
                self.shell_state
                    .env_vars
                    .remove(&fd_output_process_substitution_key(target_fd));
            }
            Some(FdWriteEndpoint::ProcContentWrite) => {
                // rubash#2042: the ledger records a canonical 0644
                // spelling, so a respawned child re-derives the same
                // write class from the registry (the endpoint itself
                // carries no spelling — the class, not the pid, is
                // what travels; same device-spelling precedent as the
                // /dev/full arm above).
                self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
                self.shell_state
                    .env_vars
                    .insert(fd_output_key(target_fd), "/proc/self/status".to_string());
                self.shell_state
                    .env_vars
                    .remove(&fd_output_process_substitution_key(target_fd));
            }
            Some(FdWriteEndpoint::PipeChannel(_)) => {
                // rubash#2385: a complementary channel end has no
                // respellable target — the in-memory queue cannot
                // travel into a respawned shell — so the ledger
                // clears, exactly the None arm's record; the fd's
                // spawn delivery (sink materialization) is the
                // boundary a child actually meets.
                self.shell_state.env_vars.remove(&fd_output_key(target_fd));
                self.shell_state
                    .env_vars
                    .remove(&fd_output_process_substitution_key(target_fd));
            }
            None => {
                self.shell_state.env_vars.remove(&fd_output_key(target_fd));
                self.shell_state
                    .env_vars
                    .remove(&fd_output_process_substitution_key(target_fd));
            }
        }
    }

    fn copy_persistent_output_fd(&mut self, target_fd: u32, source_fd: u32) {
        // GNU dup2 semantics: `N>&M` copies the whole descriptor — a
        // read-only source (`exec 0>&3`) also installs the read side.
        if self.fd_table.is_open(source_fd) {
            let source_endpoint = self.fd_table.write_endpoint(source_fd);
            let target_endpoint = self.fd_table.write_endpoint(target_fd);
            if target_fd != source_fd
                && target_endpoint.as_ref().is_some_and(|endpoint| {
                    matches!(endpoint, FdWriteEndpoint::ProcessSubstitution { .. })
                })
                && target_endpoint != source_endpoint
            {
                let _ = self.close_persistent_output_fd(target_fd);
            }
            if self.fd_table.dup_output(target_fd, source_fd).is_ok() {
                self.record_output_fd_ledger(target_fd);
                self.record_input_fd_ledger(target_fd);
            } else {
                let _ = self.close_persistent_output_fd(target_fd);
                self.shell_state
                    .env_vars
                    .insert(fd_closed_key(target_fd), "1".to_string());
            }
            return;
        }
        if self
            .shell_state
            .env_vars
            .contains_key(&fd_closed_key(source_fd))
        {
            let _ = self.close_persistent_output_fd(target_fd);
            self.shell_state
                .env_vars
                .insert(fd_closed_key(target_fd), "1".to_string());
        } else if self.coproc_write_file(source_fd).is_some() {
            self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
            self.shell_state.env_vars.insert(
                fd_output_key(target_fd),
                format!("{FD_COPROC_STDIN_TARGET_PREFIX}{source_fd}"),
            );
            self.shell_state
                .env_vars
                .remove(&fd_output_process_substitution_key(target_fd));
        } else if let Some(target) = self
            .shell_state
            .env_vars
            .get(&fd_output_key(source_fd))
            .cloned()
        {
            self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
            self.shell_state
                .env_vars
                .insert(fd_output_key(target_fd), target);
            if let Some(source) = self
                .shell_state
                .env_vars
                .get(&fd_output_process_substitution_key(source_fd))
                .cloned()
            {
                self.shell_state
                    .env_vars
                    .insert(fd_output_process_substitution_key(target_fd), source);
            } else {
                self.shell_state
                    .env_vars
                    .remove(&fd_output_process_substitution_key(target_fd));
            }
        } else if let Some(target) = stdio_output_target(source_fd) {
            self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
            self.shell_state
                .env_vars
                .insert(fd_output_key(target_fd), target.to_string());
            self.shell_state
                .env_vars
                .remove(&fd_output_process_substitution_key(target_fd));
        } else {
            let _ = self.close_persistent_output_fd(target_fd);
            self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
        }
    }

    fn open_persistent_output_process_substitution(
        &mut self,
        fd: u32,
        target: &str,
    ) -> Result<bool, ExecuteError> {
        let Some(source) = target
            .strip_prefix(">(")
            .and_then(|target| target.strip_suffix(')'))
        else {
            return Ok(false);
        };

        let path = self.empty_process_substitution_temp()?;
        // rubash#1022: GNU subst.c:6362 forks the procsub at expansion, so
        // $! names it while the body runs. The persistent body only runs at
        // close; register the fork placeholder now (pending: no recorded
        // status yet) so `kill -0 $!`/`kill $!` reach the forked child in
        // the meantime. The status lands in completed_statuses when the fd
        // closes and the body runs (close_persistent_output_fd).
        // Rebinding an fd that still holds a process substitution closes
        // the old descriptor first in GNU (dup2 onto it): the old body
        // receives everything written so far. Run the deferred body via
        // the regular close instead of abandoning it (rubash#1336:
        // `exec > >(tee a); echo 1; exec > >(tee b)` lost "1" entirely).
        self.flush_persistent_output_process_substitution(fd)?;
        let previous =
            self.fd_table
                .entries
                .get(&fd)
                .and_then(|entry| match entry.write.as_ref() {
                    Some(FdWriteEndpoint::ProcessSubstitution { procsub_pid, .. }) => *procsub_pid,
                    _ => None,
                });
        if let Some(previous) = previous {
            // Defensive (unreachable after the flush above): reap an
            // orphaned placeholder so a rebinding loop cannot accumulate
            // one zombie per iteration.
            self.reap_procsub_placeholder(previous, true);
        }
        #[cfg(unix)]
        let procsub_pid = match Self::fork_procsub_placeholder(0) {
            Some(pid) => {
                self.shell_state.job_table.procsub_placeholders.insert(pid);
                self.shell_state.last_background_pid = Some(pid);
                Some(pid)
            }
            None => None,
        };
        #[cfg(not(unix))]
        let procsub_pid = None;
        self.fd_table.open_output(
            fd,
            FdWriteEndpoint::ProcessSubstitution {
                path: path.clone(),
                command: source.to_string(),
                procsub_pid,
            },
            fd >= 10,
        );
        self.shell_state.env_vars.remove(&fd_closed_key(fd));
        self.shell_state
            .env_vars
            .insert(fd_output_key(fd), path.to_string_lossy().into_owned());
        self.shell_state
            .env_vars
            .insert(fd_output_process_substitution_key(fd), source.to_string());
        Ok(true)
    }

    /// Run the deferred body of a persistent output process substitution
    /// bound to `fd`, if any (no-op for every other endpoint). GNU binds
    /// `>(...)` to a real pipe (subst.c process_substitute): rebinding or
    /// closing the descriptor delivers EOF and the substitution reader
    /// consumes everything written so far. rubash's deferred model buffers
    /// the bytes in a carrier file and runs the body at close
    /// (close_persistent_output_fd), so every persistent overwrite of the
    /// descriptor must close it first — the sites that silently overwrote
    /// the endpoint dropped the body and all buffered output (rubash#1336:
    /// `exec > >(tee log)` followed by any later rebind or by shell exit
    /// lost the whole stream).
    pub(in crate::executor) fn flush_persistent_output_process_substitution(
        &mut self,
        fd: u32,
    ) -> Result<(), ExecuteError> {
        let is_procsub = self.fd_table.entries.get(&fd).is_some_and(|entry| {
            !entry.closed
                && matches!(
                    entry.write.as_ref(),
                    Some(FdWriteEndpoint::ProcessSubstitution { .. })
                )
        });
        if is_procsub {
            self.close_persistent_output_fd(fd)?;
        }
        Ok(())
    }

    /// Shell-exit sweep (script_driver::finish_shell, after the EXIT trap):
    /// close every still-open persistent output process substitution so
    /// its deferred body runs with the accumulated input. GNU's exit path
    /// closes the shell's descriptors; each substitution reader sees EOF
    /// and finishes (subst.c process_substitute pipe semantics). Without
    /// the sweep, `exec > >(tee log)` at the top of a script — the standard
    /// logging idiom — silently discarded the entire stream when the
    /// script simply ran off its end (rubash#1336).
    pub fn close_all_persistent_output_process_substitutions(&mut self) {
        let mut fds: Vec<u32> = self
            .fd_table
            .entries
            .iter()
            .filter(|(_, entry)| {
                !entry.closed
                    && matches!(
                        entry.write.as_ref(),
                        Some(FdWriteEndpoint::ProcessSubstitution { .. })
                    )
            })
            .map(|(fd, _)| *fd)
            .collect();
        // Descending: within one substitution's dup group the final
        // close must be the lowest fd, so that when fds 1/2 belong to
        // the group the deferred body runs from a close of that very fd
        // and close_persistent_output_fd's stdio-marker fixup gives the
        // body its stdout/stderr back (rubash#1336).
        fds.sort_unstable_by(|a, b| b.cmp(a));
        for fd in fds {
            let _ = self.close_persistent_output_fd(fd);
        }
    }

    /// Fork-boundary sweep (called from restore_flat_subshell): close the
    /// persistent output process substitutions the closing region OPENED
    /// (or rebound) before the parent's state is restored. GNU's forked
    /// child owns copies of the parent's descriptors
    /// (execute_cmd.c:1576 execute_in_subshell; subst.c:7143
    /// command_substitute); at child exit the copies close, so a
    /// substitution created inside sees EOF and finishes, while an
    /// inherited one keeps streaming for the parent. Identity is the
    /// registration the open left in the env (carrier path + body source):
    /// an fd whose parent-env registration matches the live endpoint is
    /// inherited and left alone; anything else is region-owned
    /// (rubash#1336).
    pub(in crate::executor) fn flush_region_owned_output_process_substitutions(
        &mut self,
        parent_env: &crate::shell::var_table::VarTable,
    ) {
        let mut owned: Vec<u32> = self
            .fd_table
            .entries
            .iter()
            .filter(|(fd, entry)| {
                if entry.closed {
                    return false;
                }
                let Some(FdWriteEndpoint::ProcessSubstitution { path, command, .. }) =
                    entry.write.as_ref()
                else {
                    return false;
                };
                let path_text = path.to_string_lossy();
                parent_env
                    .get(&super::command_text::fd_output_process_substitution_key(
                        **fd,
                    ))
                    .map(String::as_str)
                    != Some(command.as_str())
                    || parent_env
                        .get(&super::command_text::fd_output_key(**fd))
                        .map(String::as_str)
                        != Some(path_text.as_ref())
            })
            .map(|(fd, _)| *fd)
            .collect();
        // Descending, same dup-group ordering as the exit sweep.
        owned.sort_unstable_by(|a, b| b.cmp(a));
        for fd in owned {
            let _ = self.close_persistent_output_fd(fd);
        }
    }

    pub(in crate::executor) fn close_persistent_output_fd(
        &mut self,
        fd: u32,
    ) -> Result<(), ExecuteError> {
        let coproc_pid = self
            .fd_table
            .entries
            .get(&fd)
            .filter(|entry| !entry.closed)
            .and_then(|entry| match entry.write.as_ref() {
                Some(FdWriteEndpoint::CoprocStdin { pid, .. }) => Some(*pid),
                _ => None,
            });
        // rubash#1022: complete the placeholder registered at open with the
        // body's status (close runs the deferred body); $! itself is left
        // alone — it has named this pid since the open.
        let pending_pid =
            self.fd_table
                .entries
                .get(&fd)
                .and_then(|entry| match entry.write.as_ref() {
                    Some(FdWriteEndpoint::ProcessSubstitution { procsub_pid, .. }) => *procsub_pid,
                    _ => None,
                });
        self.fd_table.close_output(fd);
        if let Some(pid) = coproc_pid {
            self.mark_coproc_array_endpoint_closed(pid, fd);
        }
        let target = self.shell_state.env_vars.remove(&fd_output_key(fd));
        let source = self
            .shell_state
            .env_vars
            .remove(&fd_output_process_substitution_key(fd));
        // GNU delivers EOF to the substitution reader only when the LAST
        // descriptor referring to the pipe closes: a dup'd copy (`N>&M`,
        // `exec >&$fd`) keeps the stream alive across an intermediate
        // close. While another open entry still references this
        // substitution (same carrier path + body source), this close
        // drops only this descriptor's registration — the deferred body
        // and the carrier teardown wait for the final close
        // (rubash#1336).
        if let (Some(target_text), Some(source_text)) = (&target, &source) {
            let carrier = shell_path_to_windows(target_text, &self.shell_state.env_vars);
            let still_referenced = self.fd_table.entries.values().any(|entry| {
                !entry.closed
                    && matches!(
                        entry.write.as_ref(),
                        Some(FdWriteEndpoint::ProcessSubstitution { path, command, .. })
                            if *path == carrier && command == source_text
                    )
            });
            if still_referenced {
                return Ok(());
            }
        }
        if let (Some(target), Some(source)) = (target, source) {
            let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
            let input = fs::read_to_string(&path).unwrap_or_default();
            // The deferred body runs as the substitution child GNU forked
            // at open time: its stdio is the shell's as of the fork. For
            // fds 1/2 that is the process stdout/stderr the binding
            // covered — but close_output just cleared the slot, so a body
            // writing to the same descriptor (tee's stdout copy on fd 1)
            // would be silently discarded. Reinstate the stdio marker
            // for the body's duration, then put the closed slot back
            // exactly as close_output left it (rubash#1336).
            let saved_entry = self.fd_table.entries.get(&fd).cloned();
            if fd == 1 || fd == 2 {
                let marker = if fd == 1 {
                    FdWriteEndpoint::Stdout
                } else {
                    FdWriteEndpoint::Stderr
                };
                self.fd_table.open_output(fd, marker, false);
            }
            let result =
                self.execute_persistent_output_process_substitution(&source, input, pending_pid);
            match saved_entry {
                Some(entry) => {
                    self.fd_table.entries.insert(fd, entry);
                }
                None => {
                    self.fd_table.entries.remove(&fd);
                }
            }
            result?;
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    fn close_persistent_input_fd(&mut self, fd: u32) {
        let coproc_pid = self
            .fd_table
            .read_endpoint(fd)
            .and_then(|endpoint| match endpoint {
                FdReadEndpoint::CoprocStdout { pid, .. } => Some(pid),
                _ => None,
            });
        self.fd_table.close_input(fd);
        if let Some(pid) = coproc_pid {
            self.mark_coproc_array_endpoint_closed(pid, fd);
        }
        self.shell_state.env_vars.remove(&fd_stdin_key(fd));
        self.shell_state.env_vars.remove(&fd_stdin_offset_key(fd));
        self.shell_state.env_vars.remove(&fd_dynamic_input_key(fd));
    }

    fn mark_coproc_array_endpoint_closed(&mut self, pid: u32, fd: u32) {
        let names: Vec<String> = self
            .shell_state
            .env_vars
            .iter()
            .filter_map(|(name, value)| {
                (name.ends_with("_PID") && value.parse::<u32>().ok() == Some(pid))
                    .then(|| name.trim_end_matches("_PID").to_string())
            })
            .collect();
        for name in names {
            let Some(storage) = self.shell_state.env_vars.get(&name).cloned() else {
                continue;
            };
            let mut entries = indexed_array_entries(&storage);
            let mut changed = false;
            for value in entries.values_mut() {
                if value == &fd.to_string() {
                    *value = "-1".to_string();
                    changed = true;
                }
            }
            if changed {
                crate::executor::arrays::sync_typed_indexed_array(
                    &mut self.shell_state.variables,
                    name.as_str(),
                    &entries,
                );
                self.shell_state
                    .env_vars
                    .insert(name, format_indexed_array_storage(entries));
                self.shell_state.env_vars.clear_array_cells_dirty();
            }
        }
    }

    /// rubash#1646: close the descriptors recorded for `varredir_close`
    /// by the command that just completed, newest first (GNU closes a
    /// command's redirections in undo order). Watermark discipline: only
    /// records at or above `watermark` belong to the finishing command —
    /// records an enclosing command left pending stay for its own end.
    /// An entry an external command's undo already closed during
    /// dispatch is skipped, so a recycled descriptor number is never
    /// closed by mistake. Errors are swallowed like the shell-exit sweep
    /// (close_all_persistent_output_process_substitutions): a `>(...)`
    /// body's own status never fails the owning command.
    pub(in crate::executor) fn drain_deferred_varredir_closes(&mut self, watermark: usize) {
        while self.deferred_varredir_closes.len() > watermark {
            let Some(fd) = self.deferred_varredir_closes.pop() else {
                break;
            };
            let still_open = self
                .fd_table
                .entries
                .get(&fd)
                .is_some_and(|entry| !entry.closed);
            if still_open {
                let _ = self.close_persistent_fd(fd);
            }
        }
    }

    fn close_persistent_fd(&mut self, fd: u32) -> Result<(), ExecuteError> {
        self.close_persistent_output_fd(fd)?;
        self.close_persistent_input_fd(fd);
        self.fd_table.close(fd);
        self.shell_state
            .env_vars
            .insert(fd_closed_key(fd), "1".to_string());
        Ok(())
    }

    pub(in crate::executor) fn execute_persistent_output_process_substitution(
        &mut self,
        source: &str,
        input: String,
        pending_pid: Option<u32>,
    ) -> Result<(), ExecuteError> {
        // rubash#1024: bracket the inline body run with the enclosing
        // command's lines (same discipline as the non-persistent output
        // form in external_setup.rs) so body diagnostics report the
        // script line, not the body-local line 1.
        let old_current_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .cloned();
        let old_cmd_start_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CMD_START_LINE")
            .cloned();
        let ast = super::command_substitution::parse_procsub_body(self, source);
        let old_stdin = self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
        let old_offset = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .cloned();
        let old_fd0 = self.fd_table.entries.get(&0).cloned();
        let fd0_key = fd_stdin_key(0);
        let fd0_offset_key = fd_stdin_offset_key(0);
        let fd0_dynamic_key = fd_dynamic_input_key(0);
        let fd0_closed_key = fd_closed_key(0);
        let old_fd0_stdin = self.shell_state.env_vars.get(&fd0_key).cloned();
        let old_fd0_offset = self.shell_state.env_vars.get(&fd0_offset_key).cloned();
        let old_fd0_dynamic = self.shell_state.env_vars.get(&fd0_dynamic_key).cloned();
        let old_fd0_closed = self.shell_state.env_vars.get(&fd0_closed_key).cloned();
        self.set_fd_input_text(0, input.clone(), false);
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input);
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        let result = self.execute_ast(&ast);
        match old_fd0 {
            Some(entry) => {
                self.fd_table.entries.insert(0, entry);
            }
            None => {
                self.fd_table.entries.remove(&0);
            }
        }
        restore_optional_env_var(&mut self.shell_state.env_vars, &fd0_key, old_fd0_stdin);
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            &fd0_offset_key,
            old_fd0_offset,
        );
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            &fd0_dynamic_key,
            old_fd0_dynamic,
        );
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            &fd0_closed_key,
            old_fd0_closed,
        );
        restore_optional_env_var(&mut self.shell_state.env_vars, FUNCTION_STDIN, old_stdin);
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FUNCTION_STDIN_OFFSET,
            old_offset,
        );
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            "__RUBASH_CURRENT_LINE",
            old_current_line,
        );
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            "__RUBASH_CMD_START_LINE",
            old_cmd_start_line,
        );
        // rubash#1022: record the deferred body's status the way the inline
        // forms do. A pending pid (registered at open) is only completed —
        // $! has named it since the open and stays put. Without one (a
        // same-command body: compound or assignment form) register fresh so
        // $! names the substitution and `wait $!` reports this status.
        let status = match &result {
            Ok(()) => self.exit_code,
            Err(ExecuteError::ExitCode(code))
            | Err(ExecuteError::ExpansionFailure(code))
            | Err(ExecuteError::Return(code)) => *code,
            _ => 1,
        };
        match pending_pid {
            Some(pid) => {
                self.shell_state
                    .job_table
                    .completed_statuses
                    .insert(pid, status);
                self.reap_procsub_placeholder(pid, true);
            }
            None => {
                self.register_process_substitution_status(status);
            }
        }
        result
    }

    /// GNU redir.c do_redirections → redir_varassign (redir.c:1133-1166):
    /// every `{var}` redirection allocates a fresh descriptor and assigns
    /// the number to the variable — for builtins, functions and external
    /// commands alike — before the command runs. Applies all fd_var
    /// redirects of `cmd` in list order; returns whether any failed (the
    /// command then does not run, matching GNU's redirection-error
    /// abort). `auto_close` mirrors the caller's varredir_close handling
    /// (the option itself is checked inside execute_dynamic_fd_var_redirect).
    pub(in crate::executor) fn apply_dynamic_fd_var_redirects(
        &mut self,
        cmd: &CommandNode,
        auto_close: bool,
    ) -> Result<bool, ExecuteError> {
        // RESTRICTED_SHELL (#804 F1/N2): the refusal must precede
        // redir_varassign — GNU checks restricted before the varassign arm
        // opens or creates the target — and it speaks at the FIRST
        // restricted-write entry in the list (`> s1.txt {v}> s2.txt`
        // refuses `s1.txt`, never materializing either; `2>/dev/null
        // {v}> g.txt` refuses the /dev/null form itself, GNU 5.3), on
        // every applier path (simple-command preflight and the
        // exec-with-operand site alike).
        if self.reject_restricted_write_redirect_entry(cmd)? {
            return Ok(true);
        }
        let mut redirect_failed = false;
        // rubash#2281: stamp this run's generation on the close records
        // it is about to produce, so a later gate can tell this
        // command's records from an enclosing command's (see the
        // fd_var_close_generation field doc).
        self.fd_var_close_generation += 1;
        self.fd_var_pending_close_generation = self.fd_var_close_generation;
        self.fd_var_external_undo.clear();
        self.fd_var_pending_closes.clear();
        self.fd_var_close_restore.clear();
        // rubash#2282: GNU's do_redirections is one ordered walk over
        // one descriptor space (redir.c:237): when a REDIR_VARASSIGN
        // allocation (redir.c:940 F_DUPFD) runs, the numbered entries
        // BEFORE it in the list already hold their slots, so
        // `cmd 10<f2 {v}<f1` allocates v=11 — while a numbered entry
        // AFTER the {var} entry does not count yet (`cmd {v}<f1 10<f2`
        // allocates v=10 and the later bind wins the slot; both shapes
        // oracle-verified against GNU 5.3.20). The numbered slots are
        // not fd-table entries on this path, so carry the running set
        // of slots bound so far in the walk and count them as occupied.
        let mut occupied_slots: std::collections::HashSet<u32> = std::collections::HashSet::new();
        // rubash#2322: descriptors an earlier entry of THIS list has
        // closed at the current walk position. GNU's do_redirections
        // is one ordered walk (redir.c:246): a close at position p
        // poisons its descriptor for every later entry, and a bind
        // after the close revives it. The deferred-close design only
        // RECORDS `{var}` closes (fd_var_pending_closes), so without
        // this overlay a later `{w}<&N` dup resolved its source
        // against the still-open ambient table, bound `w`, and left
        // the assignment (and a copied descriptor) behind when the
        // redirect gate rejected the command — GNU never binds the
        // variable of a dup whose own entry fails (redir.c
        // do_redirection_internal dup arm assigns only after the
        // dup succeeds), while an EARLIER entry's successful bind
        // survives the rejection. Folded incrementally below from
        // the pending-close records plus the list's fixed entries.
        let mut list_closed: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let mut folded_closes = 0usize;
        for (position, redirect) in cmd.redirects.iter().enumerate() {
            if let Some(fd) = redirect.fd {
                occupied_slots.insert(fd);
            }
            let Some(name) = redirect.fd_var.as_deref() else {
                // A fixed entry still moves the walk state: a close
                // poisons its descriptor, any other bind revives it.
                let effective_fd =
                    redirect
                        .fd
                        .unwrap_or(if redirect.is_input_side() { 0 } else { 1 });
                if matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::CloseInput
                        | crate::parser::RedirectKind::CloseOutput
                ) {
                    list_closed.insert(effective_fd);
                } else if redirect.fd.is_some()
                    || matches!(
                        redirect.kind,
                        crate::parser::RedirectKind::Input
                            | crate::parser::RedirectKind::Output
                            | crate::parser::RedirectKind::Append
                            | crate::parser::RedirectKind::ClobberOutput
                            | crate::parser::RedirectKind::ReadWrite
                            | crate::parser::RedirectKind::DuplicateInput
                            | crate::parser::RedirectKind::DuplicateOutput
                            | crate::parser::RedirectKind::HereDoc
                            | crate::parser::RedirectKind::HereString
                    )
                {
                    list_closed.remove(&effective_fd);
                }
                continue;
            };
            // Record the variable's prior value so commands that fork
            // before do_redirections (external programs, forced-fork null
            // commands — execute_cmd.c:4216) can undo the binding. The
            // scalar store lives in both env_vars and the typed VariableStore
            // (temporary_assignments.rs apply_shell_assignment_inner), so the
            // snapshot covers both.
            let resolved = self
                .resolved_variable_name(name)
                .unwrap_or_else(|| name.to_string());
            let prior = self.shell_state.env_vars.get(&resolved).cloned();
            let prior_typed = self.shell_state.variables.get(&resolved).cloned();
            let outcome = self.execute_dynamic_fd_var_redirect(
                redirect,
                auto_close,
                true,
                &occupied_slots,
                &list_closed,
                position,
            );
            // rubash#2322: fold the closes THIS entry recorded
            // (close forms and move-form dup sources, stamped with
            // their positions) into the walk overlay, and let a
            // descriptor this entry bound revive its slot.
            while folded_closes < self.fd_var_pending_closes.len()
                && self.fd_var_pending_closes[folded_closes].0 <= position
            {
                list_closed.insert(self.fd_var_pending_closes[folded_closes].1);
                folded_closes += 1;
            }
            // rubash#2281: remember the descriptor THIS entry bound
            // (read back from the variable after a successful apply)
            // so the undo closes exactly that one. A pure close entry
            // binds nothing — the variable still names the caller's
            // own descriptor, and recording it here is what let the
            // old value-based undo close the parent's binding when a
            // gate-rejected command's closes were never delivered.
            let bound_fd = match &outcome {
                Ok(_)
                    if !matches!(
                        redirect.kind,
                        crate::parser::RedirectKind::CloseInput
                            | crate::parser::RedirectKind::CloseOutput
                    ) =>
                {
                    self.dynamic_fd_variable_value(&resolved)
                }
                _ => None,
            };
            // rubash#2322: a descriptor this entry bound is open
            // again for later entries (a same-list rebind after a
            // close revives the slot, redir.c:246 ordered walk).
            if let Some(fd) = bound_fd {
                list_closed.remove(&fd);
            }
            self.fd_var_external_undo
                .push((resolved, prior, prior_typed, bound_fd));
            match outcome {
                Ok(_) => {}
                Err(ExecuteError::IoError(error)) => {
                    let mut stderr = Vec::new();
                    writeln!(
                        &mut stderr,
                        "{}{}",
                        self.diagnostic_prefix(),
                        crate::posix_errors::message(&error)
                    )?;
                    self.write_default_stderr(&stderr)?;
                    self.exit_code = 1;
                    redirect_failed = true;
                }
                Err(error) => return Err(error),
            }
        }
        // `{var}<<EOF` heredocs live in heredoc_redirects, not redirects.
        // GNU redir.c make_here_document writes the body to a temp file and
        // redir_varassign binds the fresh fd to var, so `exec {v}<<-EOF`
        // leaves v holding a readable descriptor (vredir1.sub).
        // rubash#2282: heredoc entries are applied after the plain list in
        // this AST split, so every numbered slot of the command counts as
        // occupied for their allocations (the split does not preserve a
        // heredoc entry's source position among the plain entries; GNU
        // processes it at its list position).
        occupied_slots.extend(command_own_numbered_slots(cmd));
        for redirect in &cmd.heredoc_redirects {
            let Some(name) = redirect.fd_var.as_deref() else {
                continue;
            };
            let resolved = self
                .resolved_variable_name(name)
                .unwrap_or_else(|| name.to_string());
            let prior = self.shell_state.env_vars.get(&resolved).cloned();
            let prior_typed = self.shell_state.variables.get(&resolved).cloned();
            let outcome =
                self.execute_dynamic_fd_var_heredoc(redirect, auto_close, &occupied_slots);
            // rubash#2281: the applier-bound descriptor, as at the
            // plain-list site above (a heredoc entry always binds).
            let bound_fd = match &outcome {
                Ok(_) => self.dynamic_fd_variable_value(&resolved),
                _ => None,
            };
            self.fd_var_external_undo
                .push((resolved, prior, prior_typed, bound_fd));
            match outcome {
                Ok(_) => {}
                Err(ExecuteError::IoError(error)) => {
                    let mut stderr = Vec::new();
                    writeln!(
                        &mut stderr,
                        "{}{}",
                        self.diagnostic_prefix(),
                        crate::posix_errors::message(&error)
                    )?;
                    self.write_default_stderr(&stderr)?;
                    self.exit_code = 1;
                    redirect_failed = true;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(redirect_failed)
    }

    /// `{var}<<[-]EOF` / `{var}<<<word` — allocate a dynamic fd holding the
    /// heredoc/herestring body and bind it to var (redir.c make_here_document
    /// + redir_varassign).
    fn execute_dynamic_fd_var_heredoc(
        &mut self,
        redirect: &crate::parser::HereDocRedirect,
        auto_close: bool,
        // rubash#2282: the descriptor slots the ordered walk has
        // already bound, counted as occupied by the allocation (see
        // apply_dynamic_fd_var_redirects).
        occupied_slots: &std::collections::HashSet<u32>,
    ) -> Result<bool, ExecuteError> {
        let Some(name) = redirect.fd_var.as_deref() else {
            return Ok(false);
        };
        if self.dynamic_fd_assignment_readonly(name) {
            let prefix = self.diagnostic_prefix();
            let payload =
                format!("{name}: readonly variable\n{prefix}{name}: cannot assign fd to variable");
            return Err(ExecuteError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                payload,
            )));
        }
        let close_after_command = auto_close
            && crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "varredir_close");
        // The canonical ladder (expand_heredoc_redirect_text): a
        // `{var}<<<word` dynamic fd holds the same bytes GNU's
        // here_document_to_fd produces — storage prefix decoded and
        // the here-string newline present (rubash#1867).
        let body = self.expand_heredoc_redirect_text(redirect);
        let Some(fd) = self.allocate_dynamic_fd_avoiding(occupied_slots) else {
            // GNU exhausts {var} heredocs through the same REDIR_VARASSIGN
            // fcntl F_DUPFD path (probe: ulimit -Sn 6 then exec {v}<<E
            // prints "redirection error: cannot duplicate fd: ..." too), so
            // carry the identical errno-aware payload; GNU's own line-2
            // target word for heredocs is the internal doc pointer, the
            // variable name is the honest stand-in.
            return Err(ExecuteError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                self.fd_dup_error_payload(name),
            )));
        };
        self.fd_table
            .open_input(fd, FdReadEndpoint::text(&body), true);
        if !self.set_dynamic_fd_variable(name, fd) {
            self.close_persistent_fd(fd)?;
            return Err(ExecuteError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{name}: cannot assign fd to variable"),
            )));
        }
        if close_after_command {
            // rubash#1646: defer to the owning command's completion like
            // the redirection form (see execute_dynamic_fd_var_redirect);
            // closing at application time EBADF'd the command's own
            // `<&$var` reads.
            self.deferred_varredir_closes.push(fd);
        }
        Ok(true)
    }

    /// GNU execute_cmd.c: an external command forks before do_redirections,
    /// so a `{var}` redirection binds only in the child — the parent's
    /// variable is untouched and the freshly allocated descriptor dies
    /// with the child. Undoes what apply_dynamic_fd_var_redirects recorded
    /// for the just-finished command.
    pub(in crate::executor) fn undo_child_fd_var_redirects(&mut self) {
        let undo = std::mem::take(&mut self.fd_var_external_undo);
        for (name, prior, prior_typed, bound_fd) in undo {
            // rubash#2281: close exactly the descriptor the applier
            // bound for this variable — never the descriptor the
            // variable merely NAMES. A pure close record ({v}<&-)
            // bound nothing: the variable still names the parent's
            // own descriptor (v=9 after `exec 9<file`), and the old
            // value-based close destroyed that parent binding
            // whenever no delivery consumed the record — a
            // gate-rejected external command
            // (`cmd {v}<&- {w}<&9`) left the parent with fd 9 gone,
            // where GNU's parent keeps it (the close ran only in
            // the forked child, execute_cmd.c do_redirections).
            if let Some(fd) = bound_fd {
                let _ = self.close_persistent_fd(fd);
            }
            match prior {
                Some(value) => {
                    self.shell_state.env_vars.insert(name.clone(), value);
                }
                None => {
                    self.shell_state.env_vars.remove(&name);
                }
            }
            match prior_typed {
                Some(variable) => {
                    let _ = self.shell_state.variables.set(name, variable);
                }
                None => {
                    let _ = self.shell_state.variables.remove(&name);
                }
            }
        }
        // rubash#2272: closes the finishing command recorded
        // (fd_var_pending_closes) were performed in the table by
        // apply_external_fd_delivery for the spawn; restore the
        // parent's bindings from its snapshots now that the command
        // has completed. Closes no delivery consumed (force-fork
        // null faces) are dropped unapplied — GNU scopes them to
        // the command as well. (In-process commands consume their
        // records at dispatch and restore their own snapshots,
        // rubash#2280.)
        self.fd_var_pending_closes.clear();
        let close_restore = std::mem::take(&mut self.fd_var_close_restore);
        self.restore_fd_var_close_snapshots(close_restore);
    }

    /// rubash#2310: the same-exe script face's ordered input binding.
    /// GNU applies a command's redirections as ONE ordered walk in
    /// the child (redir.c:237 do_redirections; each dup copies the
    /// descriptor as bound at its own position, redir.c:1138-1193,
    /// dup2 at :1193). rubash splits that walk: the dynamic-fd
    /// applier (apply_dynamic_fd_var_redirects) runs in the parent
    /// ahead of dispatch and resolves every `{var}` dup against the
    /// AMBIENT table, while the numbered input binds land only in
    /// execute_direct_shell_script — so a `{var}` dup whose source
    /// an earlier entry of the command's OWN list binds kept the
    /// ambient copy (`rubash obs.sh {v}<&- 9<f9 {w}<&9` delivered
    /// the parent's seed at the var's slot where GNU delivers f9),
    /// and the `{var}` closes the applier recorded were never
    /// performed for the in-process child at all. This binder
    /// restores the walk order for the input side: it binds the
    /// numbered inputs in segments split at each `{var}` entry and
    /// applies that entry's child-visible effect
    /// (apply_script_fd_var_entry) at its list position, so at each
    /// such entry the fd table holds exactly the bindings GNU's
    /// walk would have produced by then. rubash#2322: the numbered
    /// output binds ride the same segments (see the saved_outputs
    /// accumulator in the body); before, they landed in a later
    /// pass and the output side of the walk was never at-position.
    pub(in crate::executor) fn bind_script_numbered_inputs_and_fd_vars(
        &mut self,
        cmd: &CommandNode,
        heredoc_saved: &[(u32, Option<crate::executor::fd_table::FdEntry>)],
    ) -> Result<
        (
            Vec<super::command_input_scope::SavedNumberedFd>,
            Vec<super::command_input_scope::SavedOutputFd>,
        ),
        ExecuteError,
    > {
        // The applier's deferred-close records belong to THIS
        // command; the script face consumes them here (performed at
        // their positions below). The spawn delivery prologue is the
        // other consumer and never runs for a command the script
        // face takes; unperformed records are dropped by
        // undo_child_fd_var_redirects either way (see the
        // fd_var_pending_closes field comment in mod.rs).
        let mut pending_closes = std::mem::take(&mut self.fd_var_pending_closes);
        let mut performed_closes: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let mut saved: Vec<super::command_input_scope::SavedNumberedFd> = Vec::new();
        // rubash#2322: the numbered OUTPUT binds join the same
        // segmented walk (they used to land in one later pass,
        // open_compound_output_redirects, after every `{var}` entry
        // was already serviced — so an output-side `{var}` dup kept
        // the applier's ambient copy and an output-side close was
        // silently overwritten by the later output bind:
        // `obs.sh 9>o1 {w}>&9 {v}>&-` gave the child fd 10 = the
        // ambient binding and fd 9 still open). Within one segment
        // inputs bind before outputs, as the passes were ordered
        // before; across segments the walk is exact.
        let mut saved_outputs: Vec<super::command_input_scope::SavedOutputFd> = Vec::new();
        let mut start = 0usize;
        for position in 0..cmd.redirects.len() {
            if cmd.redirects[position].fd_var.is_none() {
                continue;
            }
            self.open_compound_numbered_input_redirects_in(cmd, start, position, &mut saved)?;
            self.open_compound_output_redirects_in(cmd, start, position, &mut saved_outputs)?;
            self.apply_script_fd_var_entry(
                cmd,
                position,
                &mut pending_closes,
                &mut performed_closes,
                &saved,
                &saved_outputs,
                heredoc_saved,
            )?;
            start = position + 1;
        }
        self.open_compound_numbered_input_redirects_in(
            cmd,
            start,
            cmd.redirects.len(),
            &mut saved,
        )?;
        self.open_compound_output_redirects_in(
            cmd,
            start,
            cmd.redirects.len(),
            &mut saved_outputs,
        )?;
        Ok((saved, saved_outputs))
    }

    /// rubash#2310: whether an entry of the command's OWN list
    /// before `position` binds `fd` (a numbered input, output,
    /// append, read-write or dup entry in either spelling, or a
    /// numbered heredoc) — i.e. whether the fd table's state for
    /// `fd` at that walk position is governed by the list rather
    /// than the ambient table. The test is direction-agnostic:
    /// GNU's walk holds one table entry per descriptor, so an
    /// input bind governs a later output-spelled dup and vice
    /// versa (`{v}<&- 9<f9 {w}>&9` reads the f9 binding through
    /// the `>&` dup). `{var}` entries do not count: the applier
    /// binds them into the same table in list order, so its own
    /// resolution already sees them at their positions. A CLOSE
    /// entry does not count either: it is not a bind — the close
    /// machinery (performed / pending closes) governs that state.
    /// Note the entry test uses `fd.unwrap_or(0)`: a bare `>file`
    /// entry defaults to fd 1, so it never governs a dup source
    /// >= 3 either way.
    fn script_own_binding_before(&self, cmd: &CommandNode, position: usize, fd: u32) -> bool {
        if cmd
            .heredoc_redirects
            .iter()
            .any(|entry| entry.fd == Some(fd))
        {
            return true;
        }
        cmd.redirects[..position].iter().any(|entry| {
            entry.fd_var.is_none()
                && entry.fd.unwrap_or(0) == fd
                && matches!(
                    entry.kind,
                    crate::parser::RedirectKind::Input
                        | crate::parser::RedirectKind::Output
                        | crate::parser::RedirectKind::Append
                        | crate::parser::RedirectKind::ClobberOutput
                        | crate::parser::RedirectKind::ReadWrite
                        | crate::parser::RedirectKind::DuplicateInput
                        | crate::parser::RedirectKind::DuplicateOutput
                )
        })
    }

    /// rubash#2310: perform one deferred `{var}` close for the
    /// in-process script child, mirroring the spawn delivery
    /// prologue's bookkeeping (apply_external_fd_delivery): snapshot
    /// the slot into fd_var_close_restore so
    /// undo_child_fd_var_redirects restores the parent's binding
    /// once the command completes, then flag the slot closed for
    /// the child's run (the endpoints stay in place; readers consult
    /// the closed flag). The snapshot must carry the PRE-COMMAND
    /// state of the slot: when an earlier own-list bind already
    /// snapshotted the slot (the numbered-input accumulator, or the
    /// heredoc saves), that snapshot is the pre-command state — the
    /// current entry is the child's own binding, and replaying it
    /// at undo time would leak the binding into the parent (the
    /// undo's variable loop closes the descriptor the close entry's
    /// variable names; only this replay puts the parent's binding
    /// back). `performed` guards against performing one descriptor
    /// twice (two entries can name the same fd), which would
    /// snapshot the already-closed state.
    fn perform_script_fd_var_close(
        &mut self,
        fd: u32,
        performed: &mut std::collections::HashSet<u32>,
        saved_inputs: &[super::command_input_scope::SavedNumberedFd],
        saved_outputs: &[super::command_input_scope::SavedOutputFd],
        heredoc_saved: &[(u32, Option<crate::executor::fd_table::FdEntry>)],
    ) {
        if !performed.insert(fd) {
            return;
        }
        // rubash#2322: a slot bound by an earlier output segment
        // (saved_outputs) holds its pre-command state there, like
        // an input-bound slot in saved_inputs — the current entry
        // is the child's own output binding, not the parent's.
        let output_saved = saved_outputs.iter().find(|saved| saved.fd() == fd);
        let (entry, marker) = match saved_inputs.iter().find(|saved| saved.fd() == fd) {
            Some(saved) => (
                saved.saved_entry().cloned(),
                saved.saved_fd_closed().cloned(),
            ),
            None => match heredoc_saved.iter().find(|(saved_fd, _)| *saved_fd == fd) {
                Some((_, saved_entry)) => (
                    saved_entry.clone(),
                    self.shell_state.env_vars.get(&fd_closed_key(fd)).cloned(),
                ),
                None => match output_saved {
                    Some(saved) => (
                        saved.saved_entry().cloned(),
                        saved.saved_fd_closed().cloned(),
                    ),
                    None => (
                        self.fd_table.entries.get(&fd).cloned(),
                        self.shell_state.env_vars.get(&fd_closed_key(fd)).cloned(),
                    ),
                },
            },
        };
        self.fd_var_close_restore.push((fd, entry, marker));
        if let Some(entry) = self.fd_table.entries.get_mut(&fd) {
            entry.closed = true;
        }
        self.shell_state
            .env_vars
            .insert(fd_closed_key(fd), "1".to_string());
    }

    /// rubash#2310: apply one `{var}` entry's child-visible effect
    /// at its list position during the script face's segmented
    /// input binding (bind_script_numbered_inputs_and_fd_vars). The
    /// dynamic-fd applier already ran in the parent: it bound the
    /// file kinds outright (an open installs the same binding at
    /// any position), copied dup sources from the AMBIENT table,
    /// and recorded closes instead of performing them. At the
    /// entry's segment point the fd table holds the command's own
    /// earlier binds, so:
    /// - a dup whose source an earlier own-list entry binds is
    ///   re-resolved against the table as of this position — the
    ///   same own-list-first correction the spawn face's
    ///   deliver_fd_var_dup makes at delivery time (GNU's one
    ///   ordered walk, redir.c:237);
    /// - a close is performed in the table (the spawn face
    ///   performs the same records in its delivery prologue).
    /// A dup whose source no earlier own-list entry binds keeps
    /// the applier's ambient copy — already the at-position
    /// binding.
    fn apply_script_fd_var_entry(
        &mut self,
        cmd: &CommandNode,
        position: usize,
        pending_closes: &mut Vec<(usize, u32)>,
        performed_closes: &mut std::collections::HashSet<u32>,
        saved_inputs: &[super::command_input_scope::SavedNumberedFd],
        saved_outputs: &[super::command_input_scope::SavedOutputFd],
        heredoc_saved: &[(u32, Option<crate::executor::fd_table::FdEntry>)],
    ) -> Result<(), ExecuteError> {
        let redirect = &cmd.redirects[position];
        let Some(name) = redirect.fd_var.as_deref() else {
            return Ok(());
        };
        match redirect.kind {
            crate::parser::RedirectKind::CloseInput | crate::parser::RedirectKind::CloseOutput => {
                // GNU redir.c:1239-1248 (r_close_this +
                // REDIR_VARASSIGN): the descriptor the variable
                // holds at this position closes for the child.
                if let Some(fd) = self.dynamic_fd_variable_value(name) {
                    self.perform_script_fd_var_close(
                        fd,
                        performed_closes,
                        saved_inputs,
                        saved_outputs,
                        heredoc_saved,
                    );
                    if let Some(index) = pending_closes.iter().position(|pending| pending.1 == fd) {
                        pending_closes.remove(index);
                    }
                }
                Ok(())
            }
            crate::parser::RedirectKind::DuplicateInput
            | crate::parser::RedirectKind::DuplicateOutput => {
                let target = self.expand_redirect_target(redirect);
                let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) else {
                    return Ok(());
                };
                let write = matches!(redirect.kind, crate::parser::RedirectKind::DuplicateOutput);
                if self.script_own_binding_before(cmd, position, source_fd) {
                    match self.dynamic_fd_variable_value(name) {
                        Some(var_fd) => {
                            // Replace the applier's ambient copy
                            // with a dup of the descriptor as bound
                            // at this position — one shared
                            // description, like dup2 (redir.c:1193).
                            if write {
                                self.copy_persistent_output_fd(var_fd, source_fd);
                            } else {
                                self.copy_persistent_input_fd(var_fd, source_fd);
                            }
                        }
                        None => {
                            // The applier bound nothing: it saw
                            // only the ambient table, where the
                            // source was closed, but an own-list
                            // entry opened it. Bind now, at this
                            // position — the allocator sees the
                            // command's own binds, so the number
                            // is the one GNU's walk assigns
                            // (redir.c:1144-1150 F_DUPFD).
                            if self.fd_table.is_open(source_fd) {
                                let Some(fd) = self.allocate_dynamic_fd() else {
                                    return Err(ExecuteError::IoError(std::io::Error::new(
                                        std::io::ErrorKind::Other,
                                        self.fd_dup_error_payload(&target),
                                    )));
                                };
                                if write {
                                    self.copy_persistent_output_fd(fd, source_fd);
                                } else {
                                    self.copy_persistent_input_fd(fd, source_fd);
                                }
                                if !self.set_dynamic_fd_variable(name, fd) {
                                    // GNU redir.c:1161-1166:
                                    // redir_varassign failure closes
                                    // the just-duplicated fd.
                                    self.close_persistent_fd(fd)?;
                                    return Err(ExecuteError::IoError(std::io::Error::new(
                                        std::io::ErrorKind::Other,
                                        format!("{name}: cannot assign fd to variable"),
                                    )));
                                }
                            }
                        }
                    }
                }
                if move_source {
                    // A move's source close takes effect at the
                    // dup's position for the child. The applier
                    // recorded it in fd_var_pending_closes when it
                    // serviced the dup; when it could not (the
                    // repair above just performed the dup), the
                    // move applies from that binding instead.
                    let recorded = pending_closes
                        .iter()
                        .position(|pending| pending.1 == source_fd)
                        .map(|index| pending_closes.remove(index));
                    if recorded.is_some() || self.dynamic_fd_variable_value(name).is_some() {
                        self.perform_script_fd_var_close(
                            source_fd,
                            performed_closes,
                            saved_inputs,
                            saved_outputs,
                            heredoc_saved,
                        );
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(in crate::executor) fn execute_dynamic_fd_var_redirect(
        &mut self,
        redirect: &Redirect,
        auto_close: bool,
        // rubash#2272: when true (the simple-command funnel), the
        // closes a `{var}` entry asks for — close-form targets and
        // move-form dup sources — are only RECORDED in
        // fd_var_pending_closes, not performed in this table: GNU
        // scopes them to the command (in the fork for external
        // commands), and performing them here destroyed the parent's
        // bindings and made the ordered redirect preflight reject
        // the very dup the applier had just serviced. The exec face
        // and the compound scope path pass false: exec's closes are
        // persistent by definition, and the scope restores its own
        // snapshots.
        defer_closes: bool,
        // rubash#2282: the descriptor slots the ordered walk has
        // already bound, counted as occupied by every allocation
        // below (see apply_dynamic_fd_var_redirects).
        occupied_slots: &std::collections::HashSet<u32>,
        // rubash#2322: descriptors an earlier entry of the owning
        // list has closed at this walk position (see
        // apply_dynamic_fd_var_redirects). A dup whose source is in
        // this set fails at its own position, exactly like a source
        // the ambient table never had open — the variable stays
        // unbound. The immediate faces (defer_closes=false) perform
        // their closes in the table, so they pass an empty set.
        list_closed: &std::collections::HashSet<u32>,
        // rubash#2281: this entry's index in the owning command's
        // redirects list, stamped on every close record the entry
        // produces so consumers can apply the close at its walk
        // position. Meaningful only when defer_closes is set; the
        // immediate faces pass 0.
        position: usize,
    ) -> Result<bool, ExecuteError> {
        let Some(name) = redirect.fd_var.as_deref() else {
            return Ok(false);
        };
        let target = self.expand_redirect_target(redirect);
        let is_close = matches!(
            redirect.kind,
            crate::parser::RedirectKind::CloseInput | crate::parser::RedirectKind::CloseOutput
        );
        if !is_close && self.dynamic_fd_assignment_readonly(name) {
            // GNU sets up the redirection (creating output targets, opening
            // input files) before failing the assignment, and the command
            // itself never runs.
            match redirect.kind {
                crate::parser::RedirectKind::Output => {
                    self.create_redirect_output(&target, redirect.clobber)?;
                }
                crate::parser::RedirectKind::Append => {
                    open_redirect_append(
                        &self.session_path(shell_path_to_windows(
                            &target,
                            &self.shell_state.env_vars,
                        )),
                    )?;
                }
                crate::parser::RedirectKind::ReadWrite => {
                    OpenOptions::new()
                        .create(true)
                        .read(true)
                        .write(true)
                        .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
                }
                crate::parser::RedirectKind::Input => {
                    let _ = crate::executor::substitution_metadata::read_shell_input_file(
                        shell_path_to_windows(&target, &self.shell_state.env_vars),
                    )
                    .map_err(|io| crate::posix_errors::path_error(&target, io))?;
                }
                _ => {}
            }
            let prefix = self.diagnostic_prefix();
            let payload =
                format!("{name}: readonly variable\n{prefix}{name}: cannot assign fd to variable");
            return Err(ExecuteError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                payload,
            )));
        }
        let close_after_command = auto_close
            && crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "varredir_close");

        let close_after_success = |executor: &mut Self, fd: u32| -> Result<(), ExecuteError> {
            if close_after_command {
                // rubash#1646: record only — execute_command closes the
                // descriptor when the owning command completes (GNU
                // undo_redirections timing). Closing here, mid-application,
                // rejected the command's own `>&$var` writes with EBADF
                // and dropped a `>(...)` consumer's stream wholesale.
                executor.deferred_varredir_closes.push(fd);
            }
            Ok(())
        };

        match redirect.kind {
            crate::parser::RedirectKind::CloseInput => {
                // GNU redir.c:1219-1223 (r_close_this + REDIR_VARASSIGN):
                // the fd to close comes from redir_varvalue — an unset or
                // non-numeric variable is an ambiguous redirect.
                if self.dynamic_fd_variable_value(name).is_none() {
                    return Err(ExecuteError::IoError(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("{name}: ambiguous redirect"),
                    )));
                }
                // rubash#2272: on the funnel path the close is only
                // recorded (see the defer_closes parameter) — GNU
                // closes the descriptor in the command's own context,
                // and the parent's binding must survive the command.
                if defer_closes {
                    self.record_fd_var_pending_close(name, position);
                } else {
                    self.close_dynamic_input_fd(name);
                }
                return Ok(true);
            }
            crate::parser::RedirectKind::CloseOutput => {
                if self.dynamic_fd_variable_value(name).is_none() {
                    return Err(ExecuteError::IoError(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("{name}: ambiguous redirect"),
                    )));
                }
                if defer_closes {
                    self.record_fd_var_pending_close(name, position);
                } else {
                    self.close_dynamic_output_fd(name)?;
                }
                return Ok(true);
            }
            crate::parser::RedirectKind::DuplicateInput => {
                if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                    // GNU redir.c do_redirection_internal dup/move case:
                    // the descriptor is copied wholesale — the </> letter
                    // never gates on the source's direction, only on it
                    // being open at all (rubash#1591). rubash#2322:
                    // "open" is judged at this entry's walk position —
                    // a same-list close recorded earlier poisons the
                    // source even though the deferred close has not
                    // touched the table.
                    if !self.fd_table.is_open(source_fd) || list_closed.contains(&source_fd) {
                        return Ok(true);
                    }
                    let Some(fd) = self.allocate_dynamic_fd_avoiding(occupied_slots) else {
                        return Err(ExecuteError::IoError(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            self.fd_dup_error_payload(&target),
                        )));
                    };
                    self.copy_persistent_input_fd(fd, source_fd);
                    if !self.set_dynamic_fd_variable(name, fd) {
                        // GNU redir.c:1161-1166: redir_varassign failure
                        // closes the just-duplicated fd — no slot leak.
                        self.close_persistent_fd(fd)?;
                        return Err(ExecuteError::IoError(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("{name}: cannot assign fd to variable"),
                        )));
                    }
                    if move_source {
                        // rubash#2272: the move close is scoped to
                        // the command like every other close on the
                        // funnel path (see defer_closes).
                        if defer_closes {
                            self.fd_var_pending_closes.push((position, source_fd));
                        } else {
                            self.close_persistent_fd(source_fd)?;
                        }
                    }
                    close_after_success(self, fd)?;
                    return Ok(true);
                }
            }
            crate::parser::RedirectKind::DuplicateOutput => {
                if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                    // rubash#2322: at-position judgment, as in the
                    // DuplicateInput arm above.
                    if !self.fd_table.is_open(source_fd) || list_closed.contains(&source_fd) {
                        return Ok(true);
                    }
                    let Some(fd) = self.allocate_dynamic_fd_avoiding(occupied_slots) else {
                        return Err(ExecuteError::IoError(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            self.fd_dup_error_payload(&target),
                        )));
                    };
                    self.copy_persistent_output_fd(fd, source_fd);
                    if !self.set_dynamic_fd_variable(name, fd) {
                        self.close_persistent_fd(fd)?;
                        return Err(ExecuteError::IoError(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("{name}: cannot assign fd to variable"),
                        )));
                    }
                    if move_source {
                        if defer_closes {
                            self.fd_var_pending_closes.push((position, source_fd));
                        } else {
                            self.close_persistent_fd(source_fd)?;
                        }
                    }
                    close_after_success(self, fd)?;
                    return Ok(true);
                }
            }
            crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite => {
                if let Some(source) = target
                    .strip_prefix("<(")
                    .and_then(|target| target.strip_suffix(')'))
                    // rubash#2090: only a parser-recorded (unquoted) span
                    // is a live process substitution — GNU
                    // expand_word_internal (subst.c:11365-11372) refuses
                    // a quoted `<(`; the quoted form is a literal filename
                    // whose open the plain path below owns.
                    .filter(|_| super::external_setup::redirect_target_span_is_recorded(redirect))
                {
                    // rubash#1952: stream first, exactly like the plain
                    // exec arm — GNU process_substitute (subst.c:6362)
                    // forks the body and binds the pipe read end
                    // immediately; collecting the output first starves
                    // the consumer until the producer exits, forever
                    // for a non-terminating producer. The collected form
                    // below remains the fallback when the spawn could
                    // not be set up. (No constructing-command line is
                    // plumbed into this entry, so the child's diagnostic
                    // frame starts from the body's own text.)
                    let mut bound_fd = None;
                    if let Some(fd) = self.allocate_dynamic_fd_avoiding(occupied_slots) {
                        if self.bind_input_process_substitution_child(fd, source, None) {
                            bound_fd = Some(fd);
                        }
                    }
                    if bound_fd.is_none() {
                        if let Some(input) = self.process_substitution_output(source) {
                            let Some(fd) = self.allocate_dynamic_fd_avoiding(occupied_slots) else {
                                return Err(ExecuteError::IoError(std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    self.fd_dup_error_payload(&target),
                                )));
                            };
                            self.fd_table.open_input(
                                fd,
                                FdReadEndpoint::process_substitution(&input),
                                true,
                            );
                            self.set_fd_input_text(fd, input, true);
                            bound_fd = Some(fd);
                        }
                    }
                    if let Some(fd) = bound_fd {
                        if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                            // `<(...)` has no openable path; ledger-only
                            // companion is best-effort.
                            let _ = self.set_fd_output_file(fd, target.clone(), true, true);
                        }
                        if !self.set_dynamic_fd_variable(name, fd) {
                            self.close_persistent_fd(fd)?;
                            return Err(ExecuteError::IoError(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                format!("{name}: cannot assign fd to variable"),
                            )));
                        }
                        close_after_success(self, fd)?;
                        return Ok(true);
                    }
                }

                // GNU redir.c redir_open: on Linux `/dev/stdin` is a symlink
                // to /proc/self/fd/N, so opening it hands back the LIVE
                // stream behind that fd (same pipe buffer, same terminal) —
                // never a fresh device. Windows has no such filesystem and
                // the host path translation maps /dev/stdin to the CONIN$
                // console device, which blocks forever on read. Recognize
                // the fd-alias spellings here and dup the referenced fd,
                // exactly like the `N<&M` branch above (niubash#118 fixed
                // the plain fd-0 read path the same way; bashdb's
                // `exec {fd}</dev/stdin` command-file reader hits this).
                if redirect.kind == crate::parser::RedirectKind::Input {
                    if let Some(source_fd) = redirect_target_fd(&target) {
                        if self.fd_table.is_open_for_read(source_fd) {
                            let Some(fd) = self.allocate_dynamic_fd_avoiding(occupied_slots) else {
                                return Err(ExecuteError::IoError(std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    self.fd_dup_error_payload(&target),
                                )));
                            };
                            self.copy_persistent_input_fd(fd, source_fd);
                            if !self.set_dynamic_fd_variable(name, fd) {
                                self.close_persistent_fd(fd)?;
                                return Err(ExecuteError::IoError(std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    format!("{name}: cannot assign fd to variable"),
                                )));
                            }
                            close_after_success(self, fd)?;
                            return Ok(true);
                        }
                    }
                }

                // G-02 session-cwd anchor (rubash#1887): see
                // set_fd_readwrite_file.
                let path =
                    self.session_path(shell_path_to_windows(&target, &self.shell_state.env_vars));
                let (read_file, write_file) =
                    if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                        FileFd::open_readwrite_pair(path)
                            .map_err(|io| crate::posix_errors::path_error(&target, io))?
                    } else {
                        let file = FileFd::open_read(path)
                            .map_err(|io| crate::posix_errors::path_error(&target, io))?;
                        (file.clone(), file)
                    };
                let file = read_file;
                let Some(fd) = self.allocate_dynamic_fd_avoiding(occupied_slots) else {
                    return Err(ExecuteError::IoError(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        self.fd_dup_error_payload(&target),
                    )));
                };
                self.set_fd_input_file(fd, file, true);
                if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                    self.fd_table
                        .open_output(fd, FdWriteEndpoint::File(write_file), true);
                    self.shell_state.env_vars.remove(&fd_closed_key(fd));
                    self.shell_state
                        .env_vars
                        .insert(fd_output_key(fd), target.clone());
                }
                if !self.set_dynamic_fd_variable(name, fd) {
                    self.close_persistent_fd(fd)?;
                    return Err(ExecuteError::IoError(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("{name}: cannot assign fd to variable"),
                    )));
                }
                close_after_success(self, fd)?;
                return Ok(true);
            }
            crate::parser::RedirectKind::Output
            | crate::parser::RedirectKind::Append
            | crate::parser::RedirectKind::ClobberOutput => {
                let Some(fd) = self.allocate_dynamic_fd_avoiding(occupied_slots) else {
                    return Err(ExecuteError::IoError(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        self.fd_dup_error_payload(&target),
                    )));
                };
                if let Some(source) = target
                    .strip_prefix(">(")
                    .and_then(|target| target.strip_suffix(')'))
                    // rubash#2090: recorded-span admission, as in the
                    // Input arm above — a quoted ">(...)" target is a
                    // literal filename (subst.c:11365-11372).
                    .filter(|_| super::external_setup::redirect_target_span_is_recorded(redirect))
                {
                    if self.open_persistent_output_process_substitution(fd, &target)? {
                        if !self.set_dynamic_fd_variable(name, fd) {
                            self.close_persistent_fd(fd)?;
                            return Err(ExecuteError::IoError(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                format!("{name}: cannot assign fd to variable"),
                            )));
                        }
                        close_after_success(self, fd)?;
                        return Ok(true);
                    }
                    let _ = source;
                }
                if redirect.kind != crate::parser::RedirectKind::Append {
                    self.create_redirect_output(&target, redirect.clobber)?;
                }
                self.set_fd_output_file(
                    fd,
                    target,
                    true,
                    redirect.kind == crate::parser::RedirectKind::Append,
                )?;
                if !self.set_dynamic_fd_variable(name, fd) {
                    self.close_persistent_fd(fd)?;
                    return Err(ExecuteError::IoError(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("{name}: cannot assign fd to variable"),
                    )));
                }
                close_after_success(self, fd)?;
                return Ok(true);
            }
            _ => {}
        }

        Ok(true)
    }

    /// Writes the `__RUBASH_FD_*` read-side ledger for `fd` from whatever
    /// read endpoint is installed (or clears it). Used after dup/open.
    fn record_input_fd_ledger(&mut self, target_fd: u32) {
        match self
            .fd_table
            .entries
            .get(&target_fd)
            .and_then(|entry| entry.read.clone())
        {
            Some(FdReadEndpoint::InheritedProcessStdin) => {
                self.shell_state
                    .env_vars
                    .insert(fd_stdin_key(target_fd), FD_PROCESS_STDIN_TARGET.to_string());
                self.shell_state
                    .env_vars
                    .remove(&fd_stdin_offset_key(target_fd));
                self.shell_state
                    .env_vars
                    .remove(&fd_dynamic_input_key(target_fd));
            }
            Some(FdReadEndpoint::Text(_)) | Some(FdReadEndpoint::ProcessSubstitution(_)) => {
                if let Some((input, offset)) = self.fd_table.input_snapshot(target_fd) {
                    self.shell_state
                        .env_vars
                        .insert(fd_stdin_key(target_fd), input);
                    self.shell_state
                        .env_vars
                        .insert(fd_stdin_offset_key(target_fd), offset.to_string());
                    self.shell_state
                        .env_vars
                        .insert(fd_dynamic_input_key(target_fd), "1".to_string());
                }
            }
            Some(FdReadEndpoint::File(file_fd)) => {
                self.shell_state.env_vars.insert(
                    fd_stdin_key(target_fd),
                    shell_display_path(&file_fd.path.to_string_lossy()),
                );
                self.shell_state
                    .env_vars
                    .remove(&fd_stdin_offset_key(target_fd));
                self.shell_state
                    .env_vars
                    .remove(&fd_dynamic_input_key(target_fd));
            }
            Some(FdReadEndpoint::CoprocStdout { pid, .. }) => {
                self.shell_state.env_vars.insert(
                    fd_stdin_key(target_fd),
                    format!("{FD_COPROC_STDIN_TARGET_PREFIX}{pid}"),
                );
                self.shell_state
                    .env_vars
                    .remove(&fd_stdin_offset_key(target_fd));
                self.shell_state
                    .env_vars
                    .remove(&fd_dynamic_input_key(target_fd));
            }
            Some(FdReadEndpoint::Directory) => {
                // rubash#1937: the directory endpoint cannot cross
                // a respawn — clear the ledger like an unbound fd.
                self.shell_state.env_vars.remove(&fd_stdin_key(target_fd));
                self.shell_state
                    .env_vars
                    .remove(&fd_stdin_offset_key(target_fd));
                self.shell_state
                    .env_vars
                    .remove(&fd_dynamic_input_key(target_fd));
            }
            None => {
                self.shell_state.env_vars.remove(&fd_stdin_key(target_fd));
                self.shell_state
                    .env_vars
                    .remove(&fd_stdin_offset_key(target_fd));
                self.shell_state
                    .env_vars
                    .remove(&fd_dynamic_input_key(target_fd));
            }
        }
    }

    fn copy_persistent_input_fd(&mut self, target_fd: u32, source_fd: u32) {
        // GNU dup2 semantics (redir.c dup_redirects): `N<&M` copies the whole
        // descriptor — the `<`/`>` letter only picks the default fd number.
        // A write-only source (e.g. `exec 8<&1`) still installs fd 8's write
        // side, which is what makes `echo >&8` work.
        //
        // fd 0's input may live on the FUNCTION_STDIN text channel (an
        // in-process THIS_SH child's `< file` installs it there while
        // fd_table[0] stays InheritedProcessStdin). GNU dup2 of fd 0 copies
        // the redirected open file description, so materialize the channel
        // into a shared Text endpoint first — the Rc<RefCell<TextInput>>
        // then gives both slots one cursor (redir4.sub `exec 3<&0`).
        if source_fd == 0
            && matches!(
                self.fd_table.read_endpoint(0),
                Some(FdReadEndpoint::InheritedProcessStdin)
            )
        {
            // FUNCTION_STDIN keeps the full buffer and its offset cursor for
            // consumers that read it directly; the fd-table endpoint gets the
            // unread tail so both channels agree on what fd 0 serves next.
            if let Some(remaining) = self.function_stdin_remaining() {
                self.set_fd_input_bytes(
                    0,
                    crate::executor::substitution_metadata::shell_text_to_raw_bytes(&remaining),
                    false,
                );
            }
        }
        if self.fd_table.is_open(source_fd) {
            if self.fd_table.dup_input(target_fd, source_fd).is_ok() {
                self.record_input_fd_ledger(target_fd);
                self.record_output_fd_ledger(target_fd);
                self.shell_state.env_vars.remove(&fd_closed_key(target_fd));
            } else {
                self.close_persistent_input_fd(target_fd);
                self.shell_state
                    .env_vars
                    .insert(fd_closed_key(target_fd), "1".to_string());
            }
            return;
        }
        // A source that is absent from FdTable is closed. Do not resurrect
        // shell input from the legacy environment mirror.
        self.close_persistent_input_fd(target_fd);
    }

    /// The RLIMIT_NOFILE *soft* limit the {var} allocator must stay below
    /// (GNU redir.c:940 fcntl F_DUPFD semantics), or None for unlimited.
    ///
    /// The Windows emulation stores the pair as `"<soft>:<hard>"` decimal
    /// under `__RUBASH_ULIMIT_N` (builtins/ulimit.rs plat::set; u64::MAX
    /// means unlimited) and forwards it to spawned children, so parse the
    /// soft side. An absent key is NOT unlimited: it is the default soft
    /// limit (10240 — the WSL GNU 5.3.0 oracle's RLIMIT_NOFILE soft, which
    /// is also what the emulation reports before any `ulimit` call).
    /// rubash#773: the old parser ran `parse::<u32>()` over the whole
    /// `"soft:hard"` cell, so it failed on every stored value and the
    /// allocator silently fell back to its 1024 hard cap.
    pub(in crate::executor) fn open_files_limit(&self) -> Option<u32> {
        const UNLIMITED: u64 = u64::MAX;
        let raw = self.shell_state.env_vars.get("__RUBASH_ULIMIT_N");
        let Some(value) = raw else {
            return Some(FdTable::DEFAULT_OPEN_FILES_SOFT_LIMIT);
        };
        let soft = value.split(':').next().unwrap_or(value);
        if soft == "unlimited" {
            return None;
        }
        match soft.parse::<u64>() {
            Ok(UNLIMITED) => None,
            Ok(soft) => Some(u32::try_from(soft).unwrap_or(u32::MAX)),
            Err(_) => Some(FdTable::DEFAULT_OPEN_FILES_SOFT_LIMIT),
        }
    }

    pub(in crate::executor) fn allocate_dynamic_fd(&mut self) -> Option<u32> {
        let limit = self.open_files_limit();
        self.fd_table.allocate_dynamic_with_limit(limit)
    }

    /// rubash#2282: allocate a dynamic fd counting `avoid` — the owning
    /// command's own numbered redirect slots — as occupied, the same
    /// caliber the spawn-delivery helper (fd_var_delivery_fd) already
    /// uses when it computes the child's descriptor for a {var} entry.
    pub(in crate::executor) fn allocate_dynamic_fd_avoiding(
        &mut self,
        avoid: &std::collections::HashSet<u32>,
    ) -> Option<u32> {
        let limit = self.open_files_limit();
        self.fd_table
            .allocate_dynamic_with_limit_avoiding(limit, avoid)
    }

    /// GNU redir.c:940-946 (REDIR_VARASSIGN): a failed
    /// `fcntl(fd, F_DUPFD, SHELL_FD_BASE)` reports
    /// `sys_error("redirection error: cannot duplicate fd")` carrying the
    /// F_DUPFD errno. Which errno that is depends on where the limit bit
    /// the request (WSL GNU 5.3.0 probes, 2026-10-19): EMFILE — "Too many
    /// open files" — when the table is saturated below the soft limit
    /// (default 10240: exhaustion lands at ~10229 allocations); EINVAL —
    /// "Invalid argument" — when the base 10 itself sits at/above the soft
    /// limit (`ulimit -Sn 6`). rubash#773: report the same split instead of
    /// the hardcoded EINVAL text, which sent EMFILE scripts debugging the
    /// wrong layer.
    fn fd_dup_error_payload(&self, target: &str) -> String {
        let errno = match self.open_files_limit() {
            Some(limit) if limit <= 10 => "Invalid argument",
            _ => "Too many open files",
        };
        format!(
            "redirection error: cannot duplicate fd: {errno}\n{}{target}: {errno}",
            self.diagnostic_prefix()
        )
    }

    /// rubash#2272: record the descriptor a `{var}<&-` / `{var}>&-`
    /// entry closes — the number the var currently holds — in
    /// fd_var_pending_closes instead of closing it in this table
    /// (see the defer_closes parameter of
    /// execute_dynamic_fd_var_redirect). rubash#2281: the record
    /// carries the entry's position in the redirects list.
    fn record_fd_var_pending_close(&mut self, name: &str, position: usize) {
        if let Some(fd) = self.dynamic_fd_variable_value(name) {
            self.fd_var_pending_closes.push((position, fd));
        }
    }

    /// rubash#2281: perform one recorded pending close in the table —
    /// snapshot the entry and its fd-closed marker into
    /// fd_var_close_restore (undo_child_fd_var_redirects replays them
    /// at command completion), mark the entry closed, and set the
    /// marker. Only the closed flag changes: the endpoints stay in
    /// place so a {var} dup resolution can still read a source
    /// description from the slot.
    pub(in crate::executor) fn apply_fd_var_pending_close(&mut self, fd: u32) {
        let snapshot = self.perform_fd_var_pending_close(fd);
        self.fd_var_close_restore.push(snapshot);
    }

    /// The table step of a pending close, returning the restore
    /// snapshot instead of queueing it: the in-process application
    /// (rubash#2280) keeps its snapshots locally, because a command
    /// the body runs can clear the shared fd_var_close_restore list
    /// (the applier resets it on entry) before the owning command
    /// completes.
    pub(in crate::executor) fn perform_fd_var_pending_close(
        &mut self,
        fd: u32,
    ) -> (
        u32,
        Option<crate::executor::fd_table::FdEntry>,
        Option<String>,
    ) {
        let entry = self.fd_table.entries.get(&fd).cloned();
        let marker = self.shell_state.env_vars.get(&fd_closed_key(fd)).cloned();
        if let Some(entry) = self.fd_table.entries.get_mut(&fd) {
            entry.closed = true;
        }
        self.shell_state
            .env_vars
            .insert(fd_closed_key(fd), "1".to_string());
        (fd, entry, marker)
    }

    /// Replay close snapshots back into the table: reinsert each
    /// snapshotted entry (or remove the slot when the snapshot found
    /// none) and restore each fd-closed marker. The replay runs in
    /// REVERSE order — the snapshots are a stack: a descriptor a
    /// command closed twice must end at its oldest snapshot, the
    /// binding the parent held before the command.
    pub(in crate::executor) fn restore_fd_var_close_snapshots(
        &mut self,
        snapshots: Vec<(
            u32,
            Option<crate::executor::fd_table::FdEntry>,
            Option<String>,
        )>,
    ) {
        for (fd, entry, marker) in snapshots.into_iter().rev() {
            match entry {
                Some(entry) => {
                    self.fd_table.entries.insert(fd, entry);
                }
                None => {
                    self.fd_table.entries.remove(&fd);
                }
            }
            match marker {
                Some(value) => {
                    self.shell_state.env_vars.insert(fd_closed_key(fd), value);
                }
                None => {
                    self.shell_state.env_vars.remove(&fd_closed_key(fd));
                }
            }
        }
    }

    /// rubash#2280: for a command that will execute IN-PROCESS (a
    /// builtin or a function), perform its recorded pending closes
    /// in the table now and return the restore snapshots; the caller
    /// replays them (restore_fd_var_close_snapshots) when the
    /// command completes. GNU runs a builtin/function's
    /// do_redirections in the shell's own context
    /// (execute_cmd.c:5648 execute_builtin_or_function), so the
    /// close is visible for the whole execution — a body reading
    /// the just-closed descriptor fails — and undo_redirections
    /// restores the binding at command end. Before this, the
    /// records were only ever consumed by the spawn delivery, so an
    /// in-process command observed the descriptor still open (and
    /// the parent binding survived only by the records being
    /// dropped unapplied at undo). External commands pass
    /// in_process=false: their delivery consumes the records
    /// positionally instead (rubash#2281).
    pub(in crate::executor) fn apply_fd_var_closes_for_in_process(
        &mut self,
        in_process: bool,
    ) -> Vec<(
        u32,
        Option<crate::executor::fd_table::FdEntry>,
        Option<String>,
    )> {
        if !in_process
            || self.fd_var_pending_closes.is_empty()
            || self.fd_var_pending_close_generation != self.fd_var_close_generation
        {
            return Vec::new();
        }
        let pending = std::mem::take(&mut self.fd_var_pending_closes);
        pending
            .iter()
            .map(|&(_, fd)| self.perform_fd_var_pending_close(fd))
            .collect()
    }

    /// rubash#2281: the function fast path's variant of the close
    /// application above. GNU applies a function call's WHOLE
    /// redirect list in one ordered walk before the body runs
    /// (execute_cmd.c:5606 execute_builtin_or_function's
    /// do_redirections), so a close that FOLLOWS a bind of the same
    /// descriptor in the list (`f 3<f {v}<&-`, v=3) revokes the
    /// bind. The fast path's pre-application (the twin hook in
    /// execute_function_command_invocation) ran before
    /// execute_function installed the call's binds, so the bind
    /// landed after the close and the body read the descriptor
    /// open. Defer instead: move this command's records into
    /// fd_var_deferred_closes, and execute_function performs them
    /// (apply_deferred_fd_var_closes) once its redirect wraps have
    /// installed the call's binds.
    pub(in crate::executor) fn defer_fd_var_closes_for_function_call(&mut self) {
        if self.fd_var_pending_closes.is_empty()
            || self.fd_var_pending_close_generation != self.fd_var_close_generation
        {
            return;
        }
        self.fd_var_deferred_closes = std::mem::take(&mut self.fd_var_pending_closes);
    }

    /// Perform closes deferred by defer_fd_var_closes_for_function_call
    /// against the call's installed binds, honoring list order: a
    /// close is skipped when a LATER entry of the call's own list
    /// re-decides the descriptor (a numbered bind or heredoc bind —
    /// the walk applies it after the close, so it wins, and the
    /// close's effect on the pre-bind state is moot). Returns the
    /// restore snapshots; the caller replays them when the body
    /// finishes, BEFORE the redirect wraps unwind their own
    /// restores (the snapshots hold the call's bind entries, which
    /// the wraps' restores then displace with the ambient ones —
    /// GNU's undo stack order, redir.c undo_redirections).
    pub(in crate::executor) fn apply_deferred_fd_var_closes(
        &mut self,
        closes: &[(usize, u32)],
        call_cmd: &CommandNode,
    ) -> Vec<(
        u32,
        Option<crate::executor::fd_table::FdEntry>,
        Option<String>,
    )> {
        let mut snapshots = Vec::new();
        for &(position, fd) in closes {
            let later_entry_decides =
                call_cmd
                    .redirects
                    .iter()
                    .enumerate()
                    .any(|(index, redirect)| {
                        index > position && redirect.fd_var.is_none() && redirect.fd == Some(fd)
                    })
                    || call_cmd
                        .heredoc_redirects
                        .iter()
                        .any(|redirect| redirect.fd == Some(fd));
            if later_entry_decides {
                continue;
            }
            snapshots.push(self.perform_fd_var_pending_close(fd));
        }
        snapshots
    }

    pub(in crate::executor) fn close_dynamic_fd(&mut self, name: &str) -> Result<(), ExecuteError> {
        if let Some(fd) = self.dynamic_fd_variable_value(name) {
            self.close_persistent_fd(fd)?;
        }
        Ok(())
    }

    fn close_dynamic_input_fd(&mut self, name: &str) {
        if let Some(fd) = self.dynamic_fd_variable_value(name) {
            self.close_persistent_input_fd(fd);
            if !self.fd_table.is_open_for_write(fd) {
                self.fd_table.close(fd);
                self.shell_state
                    .env_vars
                    .insert(fd_closed_key(fd), "1".to_string());
            }
        }
    }

    fn close_dynamic_output_fd(&mut self, name: &str) -> Result<(), ExecuteError> {
        let Some(fd) = self.dynamic_fd_variable_value(name) else {
            return Ok(());
        };

        // A dynamic fd opened with `<>` is one shell descriptor.  Closing its
        // output side with `>&-` must release the descriptor completely;
        // otherwise its still-live input side prevents Bash's lowest-free fd
        // allocation from reusing the slot. Coprocess endpoints are modeled
        // as one-sided entries, so they retain the capability-specific path.
        if self.fd_table.read_endpoint(fd).is_some() {
            self.close_persistent_fd(fd)?;
            return Ok(());
        }

        // Coprocess input/output endpoints currently share the child PID as
        // their virtual descriptor. Close only the output capability so
        // `exec {COPROC[1]}>&-` does not invalidate `COPROC[0]` as well.
        self.close_persistent_output_fd(fd)?;
        if !self.fd_table.is_open_for_read(fd) {
            self.fd_table.close(fd);
            self.shell_state
                .env_vars
                .insert(fd_closed_key(fd), "1".to_string());
        }
        Ok(())
    }

    pub(in crate::executor) fn dynamic_fd_variable_value(&self, name: &str) -> Option<u32> {
        if let Some((array_name, index)) = parse_array_numeric_subscript(name) {
            return self
                .array_element_parameter_value(&format!("{array_name}[{index}]"))
                .and_then(|value| value.parse::<u32>().ok());
        }

        let storage_name = self.resolved_variable_name(name)?;
        self.shell_state
            .env_vars
            .get(&storage_name)
            .and_then(|value| value.parse::<u32>().ok())
    }

    fn dynamic_fd_assignment_readonly(&self, name: &str) -> bool {
        let base_name = parse_array_numeric_subscript(name)
            .map(|(array_name, _)| array_name)
            .unwrap_or(name);
        let resolved = self
            .resolved_variable_name(base_name)
            .unwrap_or_else(|| base_name.to_string());
        is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &resolved)
    }

    /// GNU redir.c bind_dynamic_variable -> bind_variable: a `{var}` fd
    /// assignment is a real variable binding — nameref targets resolve, an
    /// empty-cell nameref stores the fd number as its cell and is validated
    /// (`exec {r}>f` on `declare -n r` reports `` `10': not a valid
    /// identifier `` under the command name and fails), and element-form
    /// names assign through assign_array_element. Returns false when the
    /// binding was rejected.
    fn set_dynamic_fd_variable(&mut self, name: &str, fd: u32) -> bool {
        // GNU redir.c:1477 (redir_varassign): ASSIGN_DISALLOWED refuses
        // the fd bind — for an att_noassign target with only the bare
        // "cannot assign fd to variable" diagnostic (no err_readonly
        // line), after the redirection target itself was set up. The
        // funnel swallows noassign binds as success, so the verdict is
        // taken here; every caller already turns a false into that
        // diagnostic (rubash#2369).
        if self.assignment_target_is_noassign(name) {
            return false;
        }
        self.apply_shell_assignment(name, fd.to_string())
    }

    /// GNU exec.def hands shell_execve an environment built from the
    /// full export list, exported functions included (variables.c
    /// make_env_array_from_var_list renders them as BASH_FUNC_*%%
    /// entries). Rubash's exec spawn arms build the child environment
    /// from env_vars alone (builtins/exec.rs apply_exported_environment
    /// walks the EXPORTED_VARS marks), and env_vars never carries the
    /// function entries — plain spawns add them per-process in
    /// apply_exported_functions_to_child — so `exec` silently dropped
    /// every exported function (rubash#1274). Materialize them for the
    /// duration of the exec attempt; the caller restores the prior
    /// state, which matters when the exec fails and the shell continues
    /// (execfail / interactive). Returns (env key, prior value, prior
    /// exported mark) triples for restore.
    fn materialize_exported_functions_for_exec(&mut self) -> Vec<(String, Option<String>, bool)> {
        let mut saved = Vec::new();
        for name in marked_env_names(&self.shell_state.env_vars, EXPORTED_FUNCTIONS) {
            let Some(body) = self.shell_state.functions.get(&name) else {
                continue;
            };
            let def_redirects = self
                .shell_state
                .function_def_infos
                .get(&name)
                .map(|info| info.def_redirects.as_slice())
                .unwrap_or(&[])
                .to_vec();
            let env_name = exported_function_env_name(&name);
            let prior_value = self.shell_state.env_vars.get(&env_name).cloned();
            let prior_marked = marked_env_names(&self.shell_state.env_vars, EXPORTED_VARS)
                .iter()
                .any(|marked| marked == &env_name);
            saved.push((env_name.clone(), prior_value, prior_marked));
            let value = exported_function_env_value(&body.commands, &def_redirects);
            self.shell_state.env_vars.insert(env_name.clone(), value);
            mark_env_name(&mut self.shell_state.env_vars, EXPORTED_VARS, &env_name);
        }
        saved
    }

    fn restore_exported_functions_after_exec(
        &mut self,
        saved: Vec<(String, Option<String>, bool)>,
    ) {
        for (env_name, prior_value, prior_marked) in saved {
            match prior_value {
                Some(value) => {
                    self.shell_state.env_vars.insert(env_name.clone(), value);
                }
                None => {
                    self.shell_state.env_vars.remove(&env_name);
                }
            }
            if !prior_marked {
                unmark_env_name(&mut self.shell_state.env_vars, EXPORTED_VARS, &env_name);
            }
        }
    }

    pub(in crate::executor) fn execute_exec_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // GNU builtins/exec.def:151 exec_builtin resolves the operand with
        // search_for_command and shell_execve's it — on POSIX `env` is the
        // external coreutils binary that consumes the VAR=value prefixes and
        // execs the target. Rubash ports the external `env` as an internally
        // emulated command (external_inner.rs execute_env_command) that a
        // plain `env VAR=x cmd` always dispatches to; `exec env VAR=x cmd`
        // must resolve the same way. The PATH-search fallback instead binds
        // to whatever `env`-named file comes first in PATH — on a Windows
        // profile that is the extensionless ~/.local/bin/env POSIX shim,
        // which ignores its operands and exits 0, so the whole exec chain
        // silently no-ops (rubash#172).
        let saved = self.materialize_exported_functions_for_exec();
        let status_result = match self.execute_exec_env_command(cmd) {
            Ok(Some(status)) => {
                self.exit_code = status;
                Ok(status)
            }
            Ok(None) => match self.execute_exec(cmd) {
                Ok(status) => {
                    // rubash#2117: GNU exec.def's option-parse failure
                    // returns EX_USAGE (258 > EX_SHERRBASE), which
                    // execute_cmd.c:4886-4888 marks
                    // special_builtin_failed. The builtin layer returns
                    // the converted EX_BADUSAGE for that face, and a
                    // command operand's own exit 2 is indistinguishable
                    // by value — the layer's own parse is the
                    // discriminator (builtins/exec.rs
                    // status_is_option_parse_failure).
                    if crate::builtins::exec::status_is_option_parse_failure(
                        &cmd.words[1..],
                        status,
                    ) {
                        self.special_builtin_failed.set(true);
                    }
                    self.exit_code = status;
                    Ok(status)
                }
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        let result = match status_result {
            Ok(status) => {
                // rubash#2125: the status came from a do_redirections
                // failure inside execute_exec (the `{var}` apply) —
                // the command never reached the search/execve stage
                // finish_exec_replacement adjudicates, so complete at
                // status 1 with the shell alive (see the mark's doc).
                if self.exec_do_redirections_failed.replace(false) {
                    self.exit_code = status;
                    Ok(())
                } else {
                    self.finish_exec_replacement(cmd, status)
                }
            }
            Err(error) => Err(error),
        };
        self.restore_exported_functions_after_exec(saved);
        result
    }

    /// The exit contract of `exec_builtin` (exec.def:248-260): with a
    /// command operand exec replaces the shell, so the script terminates
    /// with the command's status instead of continuing past it.
    fn finish_exec_replacement(
        &mut self,
        cmd: &CommandNode,
        status: i32,
    ) -> Result<(), ExecuteError> {
        // GNU exec.def: a failed `exec command` exits a noninteractive
        // shell (EXECUTION_FAILURE). A `{var}` word that reached argv is a
        // spaced form — an ordinary operand — while the adjacent
        // `{var}redir` form was already consumed by the parser into
        // redirect.fd_var (parse.y:5821-5843 REDIR_WORD), so it never
        // appears here.
        if crate::builtins::exec::replaces_shell(&cmd.words[1..]) {
            // GNU exec.def failed_exec: `if (subshell_environment ||
            // (interactive == 0 && no_exit_on_failed_exec == 0))
            // exit_shell (exit_value)` — a subshell always dies, a
            // noninteractive shell dies unless the execfail shopt
            // (no_exit_on_failed_exec, shopt.def) is set, and an
            // interactive shell always continues. A SUCCESSFUL exec never
            // returns (execve replaced the process), so the exemption only
            // applies to failing statuses (rubash#347: `shopt -s execfail;
            // exec /no/such/cmd` prints the diagnostic and the script keeps
            // running, $? = 127).
            let execfail =
                crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "execfail");
            let interactive = self.invocation_interactive();
            if status == 0
                || self.shell_state.subshell_depth.get() > 0
                || (!interactive && !execfail)
            {
                // rubash#1012: only the pre-reset search failure keeps the
                // EXIT trap (sh_notfound -> failed_exec -> exit_shell with
                // SIG_TRAPPED still set). A resolved command — or any slash
                // operand, which bypasses search — passed
                // reset_signal_handlers before shell_execve, so the
                // replacement unwind must not run the trap.
                if self.exec_reached_post_reset(&cmd.words[1..]) {
                    self.exec_replaced = true;
                }
                return Err(ExecuteError::ExitCode(status));
            }
        }
        Ok(())
    }

    /// Whether GNU exec_builtin would have passed reset_signal_handlers
    /// (trap.c:1480) before attempting shell_execve — i.e. the command word
    /// resolved to a non-null `command` (builtins/exec.def:151-169). A slash
    /// anywhere in the name bypasses search_for_command (always post-reset);
    /// otherwise only a successful PATH search reaches the reset. The
    /// pre-reset search failure (sh_notfound -> failed_exec -> exit_shell)
    /// keeps the EXIT trap; everything post-reset does not (rubash#1012).
    fn exec_reached_post_reset(&self, args: &[String]) -> bool {
        let Some(index) = crate::builtins::exec::command_operand_index(args) else {
            return false;
        };
        let Some(command) = args.get(index) else {
            return false;
        };
        if command.contains('/') || command.contains('\\') {
            return true;
        }
        crate::executor::path::find_user_command(command, &self.shell_state.env_vars).is_some()
    }

    /// Run `exec env ...` through rubash's internal emulation of the
    /// external `env` command. Returns None when the operand shape is not
    /// the plain `exec env ...` form (exec's own options like `-c`/`-a`
    /// keep the execve semantics of the fallback path) or when the
    /// emulation is disabled.
    fn execute_exec_env_command(&mut self, cmd: &CommandNode) -> Result<Option<i32>, ExecuteError> {
        if cmd.words.get(1).map(String::as_str) != Some("env") {
            return Ok(None);
        }
        if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, "env") {
            return Ok(None);
        }
        let mut env_cmd = cmd.clone();
        env_cmd.words = cmd.words[1..].to_vec();
        self.execute_env_command(&env_cmd)?;
        Ok(Some(self.exit_code))
    }

    fn exec_has_no_command_operand_after_expansion(&self, cmd: &CommandNode) -> bool {
        if cmd.words.len() <= 1 {
            return false;
        }

        let expanded_args: Vec<String> = cmd.words[1..]
            .iter()
            .map(|word| self.expand_word(word))
            .collect();
        !crate::builtins::exec::replaces_shell(&expanded_args)
            && (cmd.redirect_out.is_some()
                || cmd.append.is_some()
                || cmd.redirect_err.is_some()
                || cmd.redirect_err_append.is_some()
                || cmd.redirect_in.is_some()
                || !cmd.redirects.is_empty())
    }
}

pub(crate) fn signal_trap_name(signal: i32) -> Option<String> {
    if signal <= 0 {
        return None;
    }
    crate::builtins::kill::translate_signal(&signal.to_string()).map(|name| format!("SIG{name}"))
}

/// rubash#2282: the full set of descriptor slots a command's own
/// numbered redirect entries (in `redirects` and `heredoc_redirects`)
/// will bind. The applier faces that walk the list in order carry a
/// running prefix of this set (only the entries before the allocation
/// point count — GNU's one ordered walk, redir.c:237); faces that
/// apply entries only after the whole plain list (the heredoc arms)
/// count the full set. The spawn-delivery helper (fd_var_delivery_fd)
/// likewise avoids the full set when it computes the child's
/// descriptor.
pub(in crate::executor) fn command_own_numbered_slots(
    cmd: &CommandNode,
) -> std::collections::HashSet<u32> {
    let mut slots: std::collections::HashSet<u32> = cmd
        .redirects
        .iter()
        .filter_map(|redirect| redirect.fd)
        .collect();
    slots.extend(
        cmd.heredoc_redirects
            .iter()
            .filter_map(|redirect| redirect.fd),
    );
    slots
}

fn exec_has_only_redirects(cmd: &CommandNode) -> bool {
    if cmd.words.len() == 1 {
        return true;
    }

    matches!(
        cmd.words.as_slice(),
        [command, fd_word]
            if command == "exec"
                && fd_word.chars().all(|ch| ch.is_ascii_digit())
                && cmd
                    .redirects
                    .iter()
                    .any(|redirect| redirect.fd.is_some_and(|fd| fd.to_string() == *fd_word))
    )
}

/// The four stdio-slot redirect arms of a compound command, ordered by
/// their position in the command's parse-order redirect list (rubash#940 —
/// the same contract prepare_brace_group_redirects follows for `{ }`,
/// rubash#826). An arm whose mirror entry cannot be found in the list (a
/// synthesized desugar) keeps its field order after every matched arm, so
/// the sort is total and stable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CompoundStdioArm {
    Out,
    Append,
    Err,
    ErrAppend,
}

fn compound_stdio_arms_in_parse_order(command: &CommandNode) -> Vec<CompoundStdioArm> {
    let candidates = [
        (
            command
                .redirect_out
                .as_deref()
                .filter(|r| r.fd.unwrap_or(1) == 1),
            CompoundStdioArm::Out,
        ),
        (
            command.append.as_deref().filter(|r| r.fd.unwrap_or(1) == 1),
            CompoundStdioArm::Append,
        ),
        (
            command
                .redirect_err
                .as_deref()
                .filter(|r| r.fd.unwrap_or(2) == 2),
            CompoundStdioArm::Err,
        ),
        (
            command
                .redirect_err_append
                .as_deref()
                .filter(|r| r.fd.unwrap_or(2) == 2),
            CompoundStdioArm::ErrAppend,
        ),
    ];
    let fallback = command.redirects.len();
    let mut order: Vec<(usize, CompoundStdioArm)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(field_index, (redirect, arm))| {
            let redirect = (*redirect)?;
            let position = command
                .redirects
                .iter()
                .position(|entry| entry == redirect)
                .unwrap_or(fallback + field_index);
            Some((position, *arm))
        })
        .collect();
    order.sort_by_key(|(position, _)| *position);
    order.into_iter().map(|(_, arm)| arm).collect()
}

// ---------------------------------------------------------------------
// WP-3 consult wiring (1.5.0 debugger design sections 4/5/6; WP-1
// gap register G-02/G-03/G-05/G-06/G-08/G-13; 150-c REPORT section 5).
//
// The executor half of the debugger: the DEBUG-trap funnel above
// consults the attached session through these methods. Everything
// here is inert while no driver is installed on this executor —
// the funnel's gate load is the only cost — and child executors
// never carry a driver, so no stop can fire inside a synthesized
// frame (DE-12's model belongs to a later work package).
// ---------------------------------------------------------------------

/// The verdict of one breakpoint-condition evaluation (design
/// 5.3): the condition's exit status decides true/false, and a
/// failed evaluation — an execution error, or a fatal arm the
/// condition raised against its own list — counts as false and
/// is reported once per breakpoint. The failure's captured
/// diagnostic (possibly empty) rides along for that report.
enum DebugConditionVerdict {
    True,
    False,
    Failed(String),
}

impl Executor {
    /// Is a debug session attached to THIS executor? The process
    /// gate is consulted first (the zero-cost rule of design 4.4);
    /// the per-executor driver is what makes the attachment ours.
    pub(crate) fn debugger_session_here(&self) -> bool {
        crate::debugger::SESSION_ACTIVE.is_armed() && self.debug_driver.is_some()
    }

    /// Could the consult on THIS executor stop the debuggee at
    /// the next seam? The session's own predicate
    /// ([`crate::debugger::DebugSession::can_stop_at_seam`]),
    /// plus the no-driver case: an executor that carries no
    /// session has nothing to consult, whatever the process
    /// gate says. The funnel consult asks this before its
    /// stop-decision machinery (S4; design 4.4).
    fn debugger_consult_can_stop(&self) -> bool {
        match self.debug_driver.as_ref() {
            Some(driver) => driver.session().can_stop_at_seam(),
            None => false,
        }
    }

    /// WP-8 (design 5.1): register one parsed reader unit's syntax
    /// view with the attached session, under the file identity the
    /// engine itself reports for the unit. Called by the two
    /// file-backed group drivers — the grouped script reader (via
    /// the `debug_syntax_unit_file` hand-off into
    /// `run_source_impl`) and the `source` builtin's group runner —
    /// after parsing and before execution, so a unit's own lines
    /// are resolvable from its first seam on. With no session
    /// attached this is one gate check and out.
    pub(crate) fn debugger_register_syntax(&mut self, file: &str, ast: &crate::parser::Ast) {
        if !self.debugger_session_here() {
            return;
        }
        let view = crate::api::syntax::SyntaxView::from_ast(file, ast);
        if let Some(driver) = self.debug_driver.as_mut() {
            driver.session_mut().register_syntax_view(view);
        }
    }

    /// WP-8 (design 5.1): note that a file's reader reached its
    /// end, so the session may soundly convict lines no parsed
    /// unit covered. Called by the same two group drivers that
    /// register views, on their normal-completion path only.
    pub(crate) fn debugger_note_syntax_complete(&mut self, file: &str) {
        if !self.debugger_session_here() {
            return;
        }
        if let Some(driver) = self.debug_driver.as_mut() {
            driver.session_mut().note_syntax_file_complete(file);
        }
    }

    /// Install a debug session on this executor. Gate pairing is
    /// the driver layer's job (150-c REPORT section 5, note 7):
    /// installation activates the session; termination
    /// ([`Executor::debugger_note_exit`]) or detach deactivates it.
    pub(crate) fn install_debug_session(&mut self, driver: crate::debugger::SessionDriver) {
        // Store first, activate second (PLAN row 11): storing
        // drops any driver being replaced, and a dropped driver
        // disarms the gate (SessionDriver's Drop) — that disarm
        // must land before this session's activation, or the
        // replacement would leave the new session's gate lowered.
        self.debug_driver = Some(driver);
        if let Some(driver) = &self.debug_driver {
            driver.session().activate();
        }
    }

    /// The attached session's driver, if any (frontend and test
    /// access; the consult itself works through the field).
    pub(crate) fn debug_driver(&self) -> Option<&crate::debugger::SessionDriver> {
        self.debug_driver.as_ref()
    }

    /// The attached session's driver, mutably.
    pub(crate) fn debug_driver_mut(&mut self) -> Option<&mut crate::debugger::SessionDriver> {
        self.debug_driver.as_mut()
    }

    /// Bind this executor's standard streams for a DAP host
    /// (design section 13): the protocol owns the process's
    /// stdio, so the debuggee's fd 0 reads the given (empty)
    /// file — a deterministic EOF — and its fd 1 and fd 2 write
    /// the given capture files, which the adapter drains into
    /// DAP `output` events. External children inherit the
    /// materialized file endpoints, so their output lands in
    /// the same captures. Returns false when a file could not
    /// be opened; the caller then refuses the launch rather
    /// than letting the debuggee write onto the protocol
    /// channel.
    pub(crate) fn debugger_bind_stdio(
        &mut self,
        stdin_path: &std::path::Path,
        stdout_path: &std::path::Path,
        stderr_path: &std::path::Path,
    ) -> bool {
        let stdin = match FileFd::open_read(stdin_path.to_path_buf()) {
            Ok(file) => file,
            Err(_) => return false,
        };
        self.fd_table
            .open_input(0, FdReadEndpoint::File(stdin), false);
        for (fd, path) in [(1u32, stdout_path), (2u32, stderr_path)] {
            let handle = match std::fs::File::create(path) {
                Ok(file) => file,
                Err(_) => return false,
            };
            let file = FileFd::from_file(handle, path.to_path_buf());
            self.fd_table
                .open_output(fd, FdWriteEndpoint::File(file), false);
        }
        true
    }

    /// The location facts a stop reports, read from the same state
    /// the trap path reads: the ambient line stamp and the
    /// BASH_SOURCE stack top (the innermost file), plus the frame
    /// depth.
    fn debugger_location(&self) -> (Option<crate::debugger::SourceLocation>, usize) {
        let line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|value| value.parse::<u32>().ok());
        let file = self.shell_state.bash_source_stack.first().cloned();
        let location = match (file, line) {
            (Some(file), Some(line)) => Some(crate::debugger::SourceLocation::new(file, line)),
            _ => None,
        };
        (location, self.shell_state.function_depth)
    }

    /// Serve the entry stop if it is still pending (DE-01; WP-1
    /// gap G-08; 150-c REPORT section 5, note 4). The entry stop is
    /// an already-established stop — the session was constructed
    /// stopped — so the first funnel arrival serves it instead of
    /// arming anything new.
    fn debugger_serve_entry_stop(&mut self) -> Result<(), ExecuteError> {
        let pending = match self.debug_driver.as_mut() {
            Some(driver) => driver.take_entry_stop_pending(),
            None => false,
        };
        if pending {
            self.debugger_serve_stop()?;
        }
        Ok(())
    }

    /// The stop-and-diff watch re-check (design section 5.4;
    /// PLAN row 10): at every stop, each enabled watch entry is
    /// re-evaluated and diffed against its previous stop's
    /// snapshot via [`crate::debugger::Breakpoint::diff_watch`];
    /// a difference queues a `WatchChanged` event (DE-05) on the
    /// session before the stop loop serves.
    ///
    /// A `watch NAME` entry renders through the variable views
    /// and carries the VarTable mirror epoch, so an unchanged
    /// tracked name is proven unchanged by the epoch alone. A
    /// `watche EXPR` entry re-evaluates under the implicit power
    /// checker (design section 7.5): a rejected or failed
    /// re-check is skipped — the entry keeps its previous
    /// snapshot and nothing executes — exactly the design's
    /// "skipped and marked" rule. Watch entries never keep the
    /// consult awake on their own (the table's seam predicate
    /// excludes them), so a watch alone never slows a free run:
    /// the design's documented limit, pinned by the row-10
    /// wiring tests.
    fn debugger_watch_prepass(&mut self) {
        // Gather the enabled watch entries first: evaluation
        // below runs engine machinery and must not hold the
        // driver borrow while it does.
        let watches: Vec<(
            crate::debugger::BreakpointId,
            crate::debugger::BreakpointKind,
            String,
        )> = match self.debug_driver.as_ref() {
            Some(driver) => driver
                .session()
                .breakpoints()
                .watch_entries()
                .filter(|entry| entry.enabled)
                .filter_map(|entry| match &entry.target {
                    crate::debugger::BreakpointTarget::Expression { text } => {
                        Some((entry.id, entry.kind, text.clone()))
                    }
                    _ => None,
                })
                .collect(),
            None => return,
        };
        if watches.is_empty() {
            return;
        }
        let mut changes: Vec<crate::debugger::WatchChange> = Vec::new();
        for (id, kind, text) in watches {
            let (rendered, epoch) = match kind {
                crate::debugger::BreakpointKind::Watch => {
                    // Render from the stop's own inspection
                    // snapshot (installed by debugger_serve_stop
                    // just before this pass): the whole value,
                    // arrays included — a plain `$NAME` reading
                    // would show only element 0 and hide element
                    // writes. The VarTable epoch rides along as
                    // the unchanged-proof (design section 5.4).
                    let rendered = self
                        .debug_driver
                        .as_ref()
                        .and_then(|driver| driver.session().inspection())
                        .and_then(|snapshot| snapshot.variable(&text))
                        .map(crate::debugger::render_watch_value)
                        .unwrap_or_default();
                    (rendered, self.shell_state.env_vars.mirror_epoch_of(&text))
                }
                crate::debugger::BreakpointKind::WatchExpression => {
                    let request = crate::debugger::EvaluationRequest {
                        mode: crate::debugger::PowerMode::Implicit,
                        text: text.clone(),
                        frame: None,
                    };
                    match self.debugger_evaluate(&request, None) {
                        Ok(outcome) => match outcome.result {
                            crate::debugger::EvaluationResult::Value(value) => (Some(value), None),
                            // Rejected or failed: skipped, snapshot
                            // kept (design section 7.5).
                            _ => continue,
                        },
                        Err(_) => continue,
                    }
                }
                _ => continue,
            };
            if let Some(driver) = self.debug_driver.as_mut() {
                if let Some(entry) = driver.session_mut().breakpoints_mut().get_mut(id) {
                    if let Some((old, new)) = entry.diff_watch(rendered, epoch) {
                        changes.push(crate::debugger::WatchChange {
                            id,
                            expression: text,
                            old_value: old,
                            new_value: new,
                        });
                    }
                }
            }
        }
        if let Some(driver) = self.debug_driver.as_mut() {
            for change in changes {
                driver
                    .session_mut()
                    .emit(crate::debugger::DebugEvent::WatchChanged(change));
            }
        }
    }

    /// Serve the stop the session is parked at: run the driver's
    /// stop loop, then apply its outcome.
    fn debugger_serve_stop(&mut self) -> Result<(), ExecuteError> {
        // PLAN row 6 / WP-6: capture the stop's inspection snapshot
        // from the live engine state and install it on the session
        // before the stop loop runs, so the frontend renders exactly
        // the state the debuggee is stopped in — the same views the
        // §5.3 API exposes, never a second rendering.
        let inspection = self.capture_inspection();
        if let Some(driver) = self.debug_driver.as_mut() {
            driver.session_mut().set_inspection(inspection);
        }
        // Design section 5.4 (stop-and-diff): before the stop
        // loop runs, re-check the session's watch entries against
        // the state the debuggee is stopped in, so a change is
        // already queued as DE-05 when the frontend renders this
        // stop. The check lives here — the stop funnel — not on
        // any write path.
        self.debugger_watch_prepass();
        // The serve loop: an evaluation outcome (WP-7) runs the
        // request through the consult's own evaluation machinery,
        // installs the outcome on the session, and serves the
        // same stop again — the debuggee stays parked. Every
        // other outcome is applied once, below, and ends the
        // serve.
        loop {
            let outcome = match self.debug_driver.as_mut() {
                Some(driver) => driver.serve_stop(),
                None => return Ok(()),
            };
            match outcome {
                crate::debugger::StopLoopOutcome::Evaluate(request) => {
                    let frame_count = self.debug_driver.as_ref().and_then(|driver| {
                        driver
                            .session()
                            .inspection()
                            .map(|snapshot| snapshot.frames.len())
                    });
                    let evaluation = match self.debugger_evaluate(&request, frame_count) {
                        Ok(evaluation) => evaluation,
                        Err(unwind) => {
                            // A control unwind the user typed at
                            // the stop (`exit` / `return` / `break`
                            // / `continue` in an explicit
                            // evaluation; design 7.4) propagates as
                            // the debuggee's own flow — which makes
                            // it a resume: the debuggee is running
                            // again the moment the unwind leaves
                            // this serve. Mirror the ForceReturn
                            // route's discipline (resume first,
                            // then raise): a session left Stopped
                            // here would wedge — the consult serves
                            // only a running session, so no later
                            // breakpoint, step, or frame-return
                            // note could ever land again. The
                            // arming is Continue: the typed unwind
                            // installed no rule.
                            if let Some(driver) = self.debug_driver.as_mut() {
                                let _ = driver
                                    .session_mut()
                                    .resume(crate::debugger::StepArming::Continue);
                            }
                            return Err(unwind);
                        }
                    };
                    if let Some(driver) = self.debug_driver.as_mut() {
                        driver.session_mut().set_last_evaluation(evaluation);
                    }
                    // An explicit evaluation may have written
                    // state (design 7.6): refresh the stop's
                    // snapshot so the frontend renders the state
                    // the debuggee is now stopped in.
                    let inspection = self.capture_inspection();
                    if let Some(driver) = self.debug_driver.as_mut() {
                        driver.session_mut().set_inspection(inspection);
                    }
                }
                crate::debugger::StopLoopOutcome::Resume(arming) => {
                    let Some(driver) = self.debug_driver.as_mut() else {
                        return Ok(());
                    };
                    return match driver.session_mut().resume(arming) {
                        Ok(()) => Ok(()),
                        Err(_) => {
                            // The stop loop ended the session while
                            // parked (exit/die) and still asked to
                            // resume: no debuggee is left to continue,
                            // so the run unwinds with the session's
                            // final status — the same shape as the
                            // debuggee exiting at this point.
                            let status = driver
                                .session()
                                .termination()
                                .map(|termination| termination.status())
                                .unwrap_or(self.exit_code);
                            Err(ExecuteError::ExitCode(status))
                        }
                    };
                }
                crate::debugger::StopLoopOutcome::ForceReturn(status) => {
                    // Design 6.1 (`return [status]`): force the frame
                    // the debuggee is stopped in to return now. The
                    // unwind is the engine's own return path — the
                    // same ExecuteError::Return the `return` builtin
                    // raises — so the frame's return site reports the
                    // return (DE-04) carrying the forced status, and
                    // the finish-style arming installed here stops the
                    // session in the caller at the return point.
                    let function_depth = self.shell_state.function_depth;
                    let in_function = function_depth > 0;
                    let in_source = self.source_nest_depth > 0;
                    let forced_status = status.unwrap_or(self.exit_code);
                    let Some(driver) = self.debug_driver.as_mut() else {
                        return Ok(());
                    };
                    if !in_function && !in_source {
                        // Rejected at the main frame: there is no frame
                        // to return from, so the debuggee simply
                        // resumes (the frontend owns telling the user
                        // why). A session the stop loop ended while
                        // parked unwinds with its final status, exactly
                        // like the Resume outcome's failure shape.
                        return match driver
                            .session_mut()
                            .resume(crate::debugger::StepArming::Continue)
                        {
                            Ok(()) => Ok(()),
                            Err(_) => Err(ExecuteError::ExitCode(
                                driver
                                    .session()
                                    .termination()
                                    .map(|termination| termination.status())
                                    .unwrap_or(self.exit_code),
                            )),
                        };
                    }
                    let arming = if in_function {
                        crate::debugger::StepArming::Finish {
                            frame_depth: function_depth,
                        }
                    } else {
                        // A sourced file's return has no function
                        // return site to service a Finish arming: the
                        // return point in the caller is its next seam,
                        // which a step-over armed at this depth names.
                        crate::debugger::StepArming::StepOver {
                            max_depth: function_depth,
                        }
                    };
                    return match driver.session_mut().resume(arming) {
                        Ok(()) => Err(ExecuteError::Return(forced_status)),
                        Err(_) => Err(ExecuteError::ExitCode(
                            driver
                                .session()
                                .termination()
                                .map(|termination| termination.status())
                                .unwrap_or(self.exit_code),
                        )),
                    };
                }
                crate::debugger::StopLoopOutcome::Detach => {
                    if let Some(driver) = self.debug_driver.take() {
                        driver.session().deactivate();
                    }
                    return Ok(());
                }
            }
        }
    }

    /// Park the session at a decided stop and serve it. The
    /// consult only decides stops from the running state; a
    /// rejection here means the state moved under the decision
    /// (the session was terminated concurrently) and the stop
    /// simply does not land.
    fn debugger_stop_and_serve(
        &mut self,
        info: crate::debugger::StopInfo,
    ) -> Result<(), ExecuteError> {
        if let Some(driver) = self.debug_driver.as_mut() {
            if driver.session_mut().stop(info).is_err() {
                return Ok(());
            }
        } else {
            return Ok(());
        }
        self.debugger_serve_stop()
    }

    /// The funnel consult (design 4.2 step 5; WP-1 gap G-06;
    /// 150-c REPORT section 5, notes 1/3/8). Asked at every funnel
    /// arrival while a session is attached — including the
    /// no-action early return — after the trap action ran and was
    /// not vetoed. Decides breakpoint, function-breakpoint, and
    /// stepping stops; a decided stop is served before this
    /// returns, so the debuggee parks inside the funnel and
    /// resumes from the same seam. `command_text` is the funnel's
    /// own frozen text — never reconstructed here (note 8).
    fn debugger_consult(
        &mut self,
        seam: crate::debugger::SeamKind,
        command_text: &str,
        seam_facts: Option<(Option<crate::debugger::SourceLocation>, usize)>,
    ) -> Result<(), ExecuteError> {
        // WP-8 (design 5.2): a function-entry seam proves the
        // entered function is defined. The note is resolution
        // bookkeeping, not a stop decision — later breakpoint
        // adjudication reads the record it leaves — so it is
        // taken on every entry arrival while the session runs,
        // ahead of the inert fast exit below, exactly as it was
        // taken inside the consult body before.
        if seam == crate::debugger::SeamKind::FunctionEntry {
            if let Some(name) = self.shell_state.function_name_stack.first().cloned() {
                if let Some(driver) = self.debug_driver.as_mut() {
                    let session = driver.session_mut();
                    if session.is_running() {
                        session.note_function_defined(&name);
                    }
                }
            }
        }
        // S4 inert fast exit (design 4.4): when nothing on this
        // executor can stop the debuggee at this seam — no
        // driver installed here at all (child executors share
        // the process gate but never carry a session), or a
        // session that is attached but inert (running under a
        // Continue arming with no enabled seam entry) — the
        // consult does not run. Nothing the consult body does
        // is observable in that shape: there is no condition to
        // evaluate, no stop to serve, and the $BASH_COMMAND
        // refresh below exists only for a stop's own reads.
        // The inert path therefore costs the gate load the
        // funnel already paid plus this one predicate — the
        // zero-cost shape the design budgets for the off path.
        if !self.debugger_consult_can_stop() {
            return Ok(());
        }
        // $BASH_COMMAND at a stop is the about-to-run command
        // (design 7.3; the section 4.2 ordering sets it before
        // the consult): for the duration of the consult — the
        // condition evaluations and any stop it decides, whose
        // inspection snapshot reads the dynamic face through the
        // row-6 views — the engine's BASH_COMMAND cell holds
        // this seam's frozen text. The previous value is restored
        // on every exit, including a forced return's unwind, so
        // the trap-action freeze semantics (rubash#546) and the
        // post-command fallback reads are exactly as they were
        // outside the consult window. (150-views REPORT section
        // 5, note 1: the dynamic read used to lag one command,
        // reporting the last command that had *begun*.)
        let saved_trap_command = self.shell_state.debug_trap_command.borrow().clone();
        *self.shell_state.debug_trap_command.borrow_mut() = Some(command_text.to_string());
        let result = self.debugger_consult_at_seam(seam, command_text, seam_facts);
        *self.shell_state.debug_trap_command.borrow_mut() = saved_trap_command;
        result
    }

    /// The consult body proper; [`Executor::debugger_consult`]
    /// wraps it with the stop-time `$BASH_COMMAND` refresh.
    fn debugger_consult_at_seam(
        &mut self,
        seam: crate::debugger::SeamKind,
        command_text: &str,
        seam_facts: Option<(Option<crate::debugger::SourceLocation>, usize)>,
    ) -> Result<(), ExecuteError> {
        use crate::debugger::{HitOutcome, SeamKind, StopInfo, StopReason};

        // niubash#681: the seam's location/depth as captured at
        // funnel arrival, before the user's DEBUG trap action ran
        // (see run_debug_trap). The fallback is defensive only —
        // the funnel always passes a snapshot while a session is
        // armed, which is the only state in which a consult runs.
        let (location, depth) = seam_facts.unwrap_or_else(|| self.debugger_location());
        let function_name = if seam == SeamKind::FunctionEntry {
            self.shell_state.function_name_stack.first().cloned()
        } else {
            None
        };

        enum Pending {
            None,
            Stop(StopInfo),
            Condition(crate::debugger::BreakpointId, StopReason),
        }
        let mut pending = Pending::None;
        if let Some(driver) = self.debug_driver.as_mut() {
            let session = driver.session_mut();
            if session.is_running() {
                // The WP-8 function-definition note for this
                // arrival was taken by the consult wrapper, ahead
                // of its inert fast exit — it lands before this
                // hit accounting, so a just-resolved entry can
                // stop at this very entry.
                // Breakpoint accounting (design 5.1/5.2/5.5):
                // register_hit counts the hit and spends ignore
                // credit before any condition is evaluated.
                let mut candidates: Vec<(crate::debugger::BreakpointId, bool)> = Vec::new();
                if let Some(location) = &location {
                    candidates.extend(
                        session
                            .breakpoints()
                            .line_entries_at(&location.file, location.line)
                            .map(|entry| (entry.id, false)),
                    );
                }
                if let Some(name) = &function_name {
                    candidates.extend(
                        session
                            .breakpoints()
                            .function_entries_named(name)
                            .map(|entry| (entry.id, true)),
                    );
                }
                for (id, is_function) in candidates {
                    let reason = if is_function {
                        StopReason::FunctionBreakpoint(id)
                    } else {
                        StopReason::Breakpoint(id)
                    };
                    match session.breakpoints_mut().register_hit(id) {
                        HitOutcome::Stop { .. } => {
                            pending = Pending::Stop(StopInfo {
                                reason,
                                location: location.clone(),
                                seam: Some(seam),
                                frame_depth: depth,
                                command_text: Some(command_text.to_string()),
                            });
                            break;
                        }
                        HitOutcome::NeedsCondition { id } => {
                            pending = Pending::Condition(id, reason);
                            break;
                        }
                        HitOutcome::Disabled | HitOutcome::Ignored | HitOutcome::NoSuchEntry => {}
                    }
                }
                if matches!(pending, Pending::None) {
                    // Stepping predicates (design section 6; WP-4):
                    // the arming installed by the last resume is a
                    // one-shot rule, evaluated by the predicates in
                    // debugger::stepping — the arming's one owner —
                    // against the engine's own function depth. A
                    // Finish arming is primarily serviced at the
                    // frame's return site; the seam predicate
                    // carries its shallower-frame backstop.
                    // The file-aware form (PLAN row 11): `step+`
                    // scopes to the recorded file identity, so
                    // the seam's spelling is converged here, at
                    // the same point registrations converge.
                    let seam_identity = location
                        .as_ref()
                        .map(|loc| crate::debugger::file_identity(&loc.file));
                    let step_stop = crate::debugger::arming_stops_at_seam_in_file(
                        session.arming(),
                        depth,
                        seam_identity.as_deref(),
                        session.step_file_scope(),
                    );
                    if step_stop {
                        pending = Pending::Stop(StopInfo {
                            reason: StopReason::Step,
                            location: location.clone(),
                            seam: Some(seam),
                            frame_depth: depth,
                            command_text: Some(command_text.to_string()),
                        });
                    }
                }
            }
        }

        match pending {
            Pending::None => Ok(()),
            Pending::Stop(info) => self.debugger_stop_and_serve(info),
            Pending::Condition(id, reason) => {
                // The condition is evaluated without the driver
                // borrowed (evaluation runs engine code). An entry
                // that vanished while deciding cannot stop.
                let text = self
                    .debug_driver
                    .as_ref()
                    .and_then(|driver| driver.session().breakpoints().get(id))
                    .and_then(|entry| entry.condition.clone());
                let Some(text) = text else {
                    return Ok(());
                };
                match self.evaluate_debug_condition(&text, location.as_ref().map(|loc| loc.line)) {
                    DebugConditionVerdict::True => {
                        // Committing the stop is what consumes a
                        // one-shot entry (design 5.3).
                        if let Some(driver) = self.debug_driver.as_mut() {
                            driver
                                .session_mut()
                                .breakpoints_mut()
                                .commit_conditional_stop(id);
                        }
                        self.debugger_stop_and_serve(StopInfo {
                            reason,
                            location,
                            seam: Some(seam),
                            frame_depth: depth,
                            command_text: Some(command_text.to_string()),
                        })
                    }
                    DebugConditionVerdict::False => Ok(()),
                    DebugConditionVerdict::Failed(diagnostic) => {
                        // Design 5.3: a failed evaluation counts as
                        // false and is reported once per breakpoint.
                        // The report is the debugger speaking (the
                        // captured engine diagnostic rides along,
                        // indented, quoted as the debuggee-context
                        // output it is); the driver owns the
                        // once-per-breakpoint record and the
                        // announcement.
                        if let Some(driver) = self.debug_driver.as_mut() {
                            let mut report =
                                format!("breakpoint {}: condition '{text}' failed", id.number());
                            if let Some(loc) = &location {
                                report.push_str(&format!(" at {}:{}", loc.file, loc.line));
                            }
                            report.push_str(", treated as false");
                            if !diagnostic.is_empty() {
                                for line in diagnostic.lines() {
                                    report.push_str("\n  ");
                                    report.push_str(line);
                                }
                            }
                            driver.announce_condition_error(id, &report);
                        }
                        Ok(())
                    }
                }
            }
        }
    }

    /// Design 5.3: a breakpoint condition is a shell command,
    /// evaluated through the engine's own execution in the current
    /// frame; its exit status is the verdict. The evaluation runs
    /// under the funnel's re-entrancy guard (restored afterwards),
    /// so neither the DEBUG trap nor the consult fires for the
    /// evaluation's own commands — the fire that led here counts
    /// as in progress. `$?` is preserved across the evaluation,
    /// like across a trap firing.
    ///
    /// rubash#1962: the evaluation is a guest in the debuggee's
    /// executor, and three guest disciplines keep it from
    /// disturbing the host run:
    ///
    /// - *Fatal state is snapshot-cloned, never inherited.* The
    ///   evalerror family and the nounset DISCARD latch are
    ///   process-of-the-run state: a condition that trips one
    ///   (`break 1 2` arming the too-many-arguments DISCARD,
    ///   expansion fatals) must not leave it armed for the
    ///   debuggee's own abandon checks to consume — that leak
    ///   silently terminated the debuggee. The whole family is
    ///   saved on entry and restored on exit; an arm raised
    ///   *during* the evaluation reclassifies it as failed
    ///   (design 5.3's error bucket), never as a verdict.
    /// - *Diagnostics are captured, then placed by the caller.*
    ///   The evaluation's stderr goes to the executor's capture
    ///   for its duration (the pipeline-stage mechanism). A
    ///   verdict's captured bytes are replayed untouched; a
    ///   failure's ride along in the once-per-breakpoint report
    ///   instead of spraying per hit, misattributed to the
    ///   debuggee.
    /// - *Lines belong to the consult site.* The condition text
    ///   is re-parsed with the consult line as its base — the
    ///   evalstring convention (execute_eval_source_inner) — so
    ///   a diagnostic raised inside the condition reads with the
    ///   debuggee's file at the line whose consult raised it,
    ///   not at the condition text's own line 1.
    fn evaluate_debug_condition(
        &mut self,
        text: &str,
        consult_line: Option<u32>,
    ) -> DebugConditionVerdict {
        let line_offset = consult_line.map(|line| line.saturating_sub(1)).unwrap_or(0);
        let mut tokens = crate::lexer::tokenize(text);
        if line_offset > 0 {
            for token in tokens.iter_mut() {
                token.position += line_offset as usize;
            }
        }
        let ast = crate::parser::parse_with_options(
            &tokens,
            crate::parser::ParseLoopOptions {
                source_line_offset: line_offset as usize,
                ..Default::default()
            },
        );
        let saved_exit_code = self.exit_code;
        let saved_trap_running = std::mem::replace(&mut self.debug_trap_running, true);
        // The fatal-state family (see the doc comment): snapshot
        // every latch the evaluation could arm, so the restore
        // below is exact whatever the condition ran.
        let saved_evalerror_pending = self.evalerror_pending.get();
        let saved_evalerror_subscript_pending = self.evalerror_subscript_pending.get();
        let saved_evalerror_failglob_pending = self.evalerror_failglob_pending.get();
        let saved_evalerror_arith_pending = self.evalerror_arith_pending.get();
        let saved_evalerror_exit_toomany_pending = self.evalerror_exit_toomany_pending.get();
        let saved_evalerror_line = self.evalerror_line.get();
        let saved_nounset_force_eof = self.nounset_force_eof.get();
        let saved_break_failure_status = self.break_failure_status.take();
        // The evaluation's diagnostics are captured for its
        // duration (the outer capture, if any, is restored after).
        let saved_stderr_capture = self.stderr_capture.take();
        self.stderr_capture = Some(Vec::new());
        let result = self.execute_ast(&ast);
        let status = self.exit_code;
        let captured = self.stderr_capture.take().unwrap_or_default();
        self.stderr_capture = saved_stderr_capture;
        // A fatal arm raised by the evaluation itself (one that
        // was not already armed on entry) is an evaluation error,
        // not a verdict: GNU raised it to discard or exit the
        // *condition's* list, and the debuggee must never see it.
        let fatal_armed = (self.evalerror_pending.get() && !saved_evalerror_pending)
            || (self.evalerror_subscript_pending.get() && !saved_evalerror_subscript_pending)
            || (self.evalerror_failglob_pending.get() && !saved_evalerror_failglob_pending)
            || (self.evalerror_arith_pending.get() && !saved_evalerror_arith_pending)
            || (self.evalerror_exit_toomany_pending.get() && !saved_evalerror_exit_toomany_pending)
            || (self.nounset_force_eof.get() && !saved_nounset_force_eof);
        self.evalerror_pending.set(saved_evalerror_pending);
        self.evalerror_subscript_pending
            .set(saved_evalerror_subscript_pending);
        self.evalerror_failglob_pending
            .set(saved_evalerror_failglob_pending);
        self.evalerror_arith_pending
            .set(saved_evalerror_arith_pending);
        self.evalerror_exit_toomany_pending
            .set(saved_evalerror_exit_toomany_pending);
        self.evalerror_line.set(saved_evalerror_line);
        self.nounset_force_eof.set(saved_nounset_force_eof);
        self.break_failure_status.set(saved_break_failure_status);
        self.debug_trap_running = saved_trap_running;
        self.exit_code = saved_exit_code;
        match result {
            Ok(()) if !fatal_armed => {
                // A verdict: the captured stderr was ordinary
                // debuggee-context output of the user's own
                // command — replay it where it would have gone
                // (into an outer capture when one owns the
                // stream, else the process stderr).
                if !captured.is_empty() {
                    if let Some(capture) = self.stderr_capture.as_mut() {
                        capture.extend_from_slice(&captured);
                    } else {
                        let _ = super::shell_options::write_stderr_bytes(&captured);
                    }
                }
                if status == 0 {
                    DebugConditionVerdict::True
                } else {
                    DebugConditionVerdict::False
                }
            }
            _ => {
                DebugConditionVerdict::Failed(String::from_utf8_lossy(&captured).trim().to_string())
            }
        }
    }

    /// WP-7 (design 7.4/7.5/7.6): run one evaluation the stop
    /// loop asked for through [`crate::debugger::StopLoopOutcome::Evaluate`].
    /// The classification is `crate::debugger::evaluation`'s —
    /// the single owner of the power rules; this path only
    /// executes what the classification admits:
    ///
    /// - Implicit: the whitelist decides; an admitted word is
    ///   expanded by the engine's own word expansion, a rejected
    ///   form is never executed — its diagnostic is the outcome.
    /// - Explicit, one expansion: expanded the same way, at full
    ///   power (an expansion that assigns or substitutes is the
    ///   user's own act against their own script, design 5.3).
    /// - Explicit, anything else: executed as commands in the
    ///   stopped frame through the engine's own execution, with
    ///   the DEBUG path suppressed for its duration, exactly as
    ///   conditions run. This is also the only write-back path
    ///   (7.6): `x=5`, `arr[2]=v`, `local y=1` are assignments
    ///   because the real assignment machinery runs them, with
    ///   its attributes, scope and refusal semantics — there is
    ///   no debugger-private write channel.
    ///
    /// Dynamic faces (`$?`, PIPESTATUS, `$BASH_COMMAND`) read
    /// through the engine's own dynamic reads — the same getters
    /// the row-6 views snapshot — never a shadow copy. `frame`
    /// is validated against the stop's captured frame count;
    /// execution touches the live frame context, which under the
    /// shell's dynamic scoping is what stopped-frame evaluation
    /// can mean (see [`crate::debugger::EvaluationRequest`]).
    ///
    /// A control unwind the user deliberately typed (`exit`,
    /// `return`, `break`, `continue` in an `eval`) propagates as
    /// the debuggee's own flow, like the same command typed in
    /// the script; any other execution error becomes a Failed
    /// outcome carrying the engine's diagnostic and status, and
    /// the session stays parked.
    fn debugger_evaluate(
        &mut self,
        request: &crate::debugger::EvaluationRequest,
        frame_count: Option<usize>,
    ) -> Result<crate::debugger::EvaluationOutcome, ExecuteError> {
        use crate::debugger::{
            DiagnosticKind, EvaluationDiagnostic, EvaluationOutcome, EvaluationResult, PowerMode,
        };
        let outcome = |result: EvaluationResult| EvaluationOutcome {
            text: request.text.clone(),
            mode: request.mode,
            result,
        };
        if let Some(frame) = request.frame {
            let valid = frame_count.is_some_and(|count| frame < count);
            if !valid {
                return Ok(outcome(EvaluationResult::Failed(EvaluationDiagnostic {
                    kind: DiagnosticKind::Runtime,
                    message: format!("no frame {frame} at this stop"),
                    source_name: None,
                    line: None,
                    status: None,
                })));
            }
        }
        let tokens = crate::lexer::tokenize(&request.text);
        let ast = crate::parser::parse(&tokens);
        if let Some(diagnostic) = crate::debugger::parse_failure(&ast) {
            return Ok(outcome(EvaluationResult::Failed(diagnostic)));
        }
        match request.mode {
            PowerMode::Implicit => match crate::debugger::check_implicit(&ast, &request.text) {
                Ok(word) => {
                    let word = word.to_string();
                    let value = self.debugger_expand_evaluation_word(&word);
                    Ok(outcome(EvaluationResult::Value(value)))
                }
                Err(rejection) => Ok(outcome(EvaluationResult::Rejected(rejection.diagnostic()))),
            },
            PowerMode::Explicit => {
                if let Some(word) = crate::debugger::single_expansion_word(&ast) {
                    let word = word.to_string();
                    let value = self.debugger_expand_evaluation_word(&word);
                    return Ok(outcome(EvaluationResult::Value(value)));
                }
                match self.debugger_execute_evaluation(&ast)? {
                    (status, None) => Ok(outcome(EvaluationResult::Status(status))),
                    (status, Some(message)) => {
                        Ok(outcome(EvaluationResult::Failed(EvaluationDiagnostic {
                            kind: DiagnosticKind::Runtime,
                            message,
                            source_name: None,
                            line: None,
                            status: Some(status),
                        })))
                    }
                }
            }
        }
    }

    /// Expand one evaluation word through the entry command
    /// preparation itself uses (`expand_word_mut`): its `:=`
    /// pre-scan applies assignment expansions exactly as a real
    /// command word's would (an explicit print of `${x:=y}` is a
    /// full-power act, design 5.3), and the expansion is the
    /// engine's own. For whitelist-admitted implicit words the
    /// pre-scan is inert by construction — the checker has
    /// already refused every writing form — so both power rules
    /// share this one expansion path and neither builds a shadow.
    /// Arithmetic writes logged during the expansion are synced
    /// to the variable table, as the arithmetic eval entries do.
    fn debugger_expand_evaluation_word(&mut self, word: &str) -> String {
        let value = self.expand_word_mut(word);
        crate::executor::arithmetic::sync_arith_writes(self);
        value
    }

    /// Execute an explicit evaluation's command list under the
    /// funnel's re-entrancy guard (restored afterwards), so
    /// neither the DEBUG trap nor the consult fires for the
    /// evaluation's own commands. Unlike conditions, `$?` is NOT
    /// preserved: the executed commands are the user's own, and
    /// their status is the evaluation's result.
    ///
    /// Returns the run's `(status, error)`: `error` is the
    /// engine's diagnostic text when execution failed without a
    /// control unwind. A control unwind the user deliberately
    /// typed (`exit`, `return`, `break`, `continue`) propagates
    /// as `Err` — the debuggee's own flow, not an outcome.
    fn debugger_execute_evaluation(
        &mut self,
        ast: &crate::parser::Ast,
    ) -> Result<(i32, Option<String>), ExecuteError> {
        let saved_trap_running = std::mem::replace(&mut self.debug_trap_running, true);
        let result = self.execute_ast(ast);
        self.debug_trap_running = saved_trap_running;
        match result {
            Ok(()) => Ok((self.exit_code, None)),
            Err(
                error @ (ExecuteError::ExitCode(_)
                | ExecuteError::Return(_)
                | ExecuteError::Break(_)
                | ExecuteError::Continue(_)),
            ) => Err(error),
            Err(error) => Ok((self.exit_code, Some(error.to_string()))),
        }
    }

    /// Service an asynchronous stop request at the boundary poll
    /// (DE-07; design 4.4; 150-c REPORT section 5, note 5). Called
    /// from `run_pending_signal_traps` after signal-trap dispatch:
    /// a pending request emits its serviced event and parks the
    /// session with reason [`StopReason::Pause`].
    fn debugger_service_async_stop(&mut self) -> Result<(), ExecuteError> {
        if !self.debugger_session_here() {
            return Ok(());
        }
        let serve = match self.debug_driver.as_mut() {
            Some(driver) => {
                let session = driver.session_mut();
                session.take_async_stop() && session.is_running()
            }
            None => false,
        };
        if !serve {
            return Ok(());
        }
        if let Some(driver) = self.debug_driver.as_mut() {
            driver
                .session_mut()
                .emit(crate::debugger::DebugEvent::AsyncStopServiced);
        }
        let (location, depth) = self.debugger_location();
        self.debugger_stop_and_serve(crate::debugger::StopInfo {
            reason: crate::debugger::StopReason::Pause,
            location,
            seam: Some(crate::debugger::SeamKind::BoundaryPoll),
            frame_depth: depth,
            command_text: None,
        })
    }

    /// DE-06 (design 9.3; PLAN row 12): an exception site whose
    /// filter the user selected is reached — an ERR trap action
    /// is about to run, an errexit decision is about to end the
    /// shell, or a parse error is about to end the current input.
    /// The site is reported (DE-06) and the session parks at it
    /// with reason [`StopReason::Exception`], served through the
    /// one stop path, BEFORE the pending action runs; resuming
    /// lets the action proceed unchanged — the park is not a
    /// veto (an errexit exit cannot be cancelled from the stop,
    /// only inspected before it happens). A site whose filter
    /// is not selected is neither reported nor parked: the
    /// selection, written identically by both frontends at the
    /// WP-15 operation layer, is the session option the
    /// design's M2 parking item names. A site reached while the
    /// session is not running free — inside an evaluation at a
    /// parked stop — produces nothing: a stop is already being
    /// served, and no second stop may interleave. Callers pass
    /// the site location when they know it better than the
    /// ambient line stamp (the parse drivers know the failed
    /// unit's first line); `None` reads the stamp.
    pub(crate) fn debugger_note_exception_site(
        &mut self,
        kind: crate::debugger::ExceptionKind,
        status: i32,
        location: Option<crate::debugger::SourceLocation>,
    ) -> Result<(), ExecuteError> {
        if !self.debugger_session_here() {
            return Ok(());
        }
        let (fallback_location, depth) = self.debugger_location();
        let location = location.or(fallback_location);
        let park = match self.debug_driver.as_mut() {
            Some(driver) => {
                let session = driver.session_mut();
                if !session.is_running() || !session.exception_filter_covers(kind) {
                    return Ok(());
                }
                let site = crate::debugger::ExceptionSite {
                    kind,
                    status,
                    location: location.clone(),
                };
                session.note_exception_site(site.clone());
                session.emit(crate::debugger::DebugEvent::ExceptionSite(site));
                true
            }
            None => false,
        };
        if park {
            self.debugger_stop_and_serve(crate::debugger::StopInfo {
                reason: crate::debugger::StopReason::Exception(kind),
                location,
                seam: None,
                frame_depth: depth,
                command_text: None,
            })?;
        }
        Ok(())
    }

    /// DE-09 (WP-1 gap G-13; 150-c REPORT section 5, note 6): the
    /// session learns the debuggee's final status from the
    /// driver/dispatch finish side — never from the EXIT funnel,
    /// which has dead faces (exec replacement, the no-action
    /// path). Idempotent: the first finish point to speak wins;
    /// later ones find the session already terminated. Ends the
    /// gate pairing.
    pub(crate) fn debugger_note_exit(&mut self, status: i32) {
        let Some(driver) = self.debug_driver.as_mut() else {
            return;
        };
        if driver.session().is_terminated() {
            return;
        }
        let _ = driver.session_mut().exit(status);
        driver.session().deactivate();
    }

    /// DE-04 (design 5.4): report a frame return to the session,
    /// emitted at the return site for every return route, and
    /// service a `Finish` arming naming that frame: the session
    /// stops in the caller, before the caller's next seam.
    /// `frame_depth` is read here, while the returning frame is
    /// still installed; `caller_location` is the call site's
    /// location as the caller will present it.
    pub(crate) fn debugger_note_frame_returned(
        &mut self,
        function: &str,
        route: crate::debugger::ReturnRoute,
        status: i32,
        caller_location: Option<crate::debugger::SourceLocation>,
    ) -> Result<(), ExecuteError> {
        if !self.debugger_session_here() {
            return Ok(());
        }
        let frame_depth = self.shell_state.function_depth;
        let mut finish_stop = false;
        if let Some(driver) = self.debug_driver.as_mut() {
            let session = driver.session_mut();
            let frame = crate::debugger::FrameReturn {
                function: Some(function.to_string()),
                frame_depth,
                status,
                route,
            };
            // WP-4 (design 6.1): the finish predicate is the
            // stepping module's, evaluated against the frame as
            // reported here — the same payload DE-04 carries.
            finish_stop = session.is_running()
                && crate::debugger::arming_stops_at_frame_return(session.arming(), &frame);
            session.emit(crate::debugger::DebugEvent::FrameReturned(frame));
        }
        if finish_stop {
            self.debugger_stop_and_serve(crate::debugger::StopInfo {
                reason: crate::debugger::StopReason::Step,
                location: caller_location,
                seam: Some(crate::debugger::SeamKind::FunctionReturn),
                frame_depth: frame_depth.saturating_sub(1),
                command_text: None,
            })?;
        }
        Ok(())
    }
}
