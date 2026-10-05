use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;
use crate::executor::substitution_metadata::bytes_to_shell_text;

impl Executor {
    pub(in crate::executor) fn finish_read_error(
        &mut self,
        cmd: &CommandNode,
        stderr: &[u8],
        status: i32,
    ) -> i32 {
        self.write_buffered_builtin_output(cmd, &[], stderr)
            .map(|_| status)
            .unwrap_or(1)
    }

    pub(in crate::executor) fn continue_read_line_after_backslash(
        &mut self,
        cmd: &CommandNode,
        read_fd: Option<u32>,
        mut line: String,
    ) -> String {
        while line.ends_with('\\') {
            line.pop();
            let Some(next) = self.read_input_for_command(cmd, read_fd, '\n', None, false) else {
                break;
            };
            line.push_str(&next);
        }
        line
    }

    /// GNU read.def read_timeout gate: before each blocking byte read, wait
    /// out the remaining deadline for the handle to become readable
    /// (shtimer_select → select). Returns true when the read must stop.
    fn read_wait_timed_out(&mut self, handle: crate::fd::HANDLE) -> bool {
        let Some(deadline) = self.read_deadline else {
            return false;
        };
        let now = std::time::Instant::now();
        if now >= deadline
            || crate::fd::wait_readable(handle, deadline - now) == crate::fd::ReadWait::Timeout
        {
            self.read_timed_out = true;
            return true;
        }
        false
    }

    pub(in crate::executor) fn read_input_for_command(
        &mut self,
        cmd: &CommandNode,
        read_fd: Option<u32>,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        // read.def:949 eof bookkeeping: each fresh record read starts with
        // the delimiter unsatisfied; the leaf readers below set the flag
        // when they exhaust their input before finding it.
        self.read_eof_no_delimiter = false;
        self.fd_table.read_eof_no_delimiter = false;
        // read.def check_read_timeout runs at the top of every read-loop
        // iteration — including before the first byte — so a deadline that
        // already expired during setup reports timeout even on buffered
        // input (e.g. `read -t .001 a <<<abcde`, whose GNU pipe herestring
        // is not S_ISREG and keeps the timeout armed).
        if let Some(deadline) = self.read_deadline {
            if std::time::Instant::now() >= deadline {
                self.read_timed_out = true;
                return None;
            }
        }
        // An unnumbered heredoc is the last stdin redirect, so it overrides
        // an earlier `<&fd`. An explicit `read -u N` still owns the input fd.
        if read_fd.is_none() {
            if cmd.heredoc_body.is_some() {
                let input = self.expand_heredoc_body_mut_from_carrier(&cmd.heredoc_body);
                self.read_eof_no_delimiter |= !input.contains(&read_delimiter_needle(delimiter));
                return Some(trim_read_input(
                    input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                ));
            }
            if let Some(heredoc) = &cmd.heredoc {
                let input = self.expand_heredoc_body_mut(heredoc);
                self.read_eof_no_delimiter |= !input.contains(&read_delimiter_needle(delimiter));
                return Some(trim_read_input(
                    input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                ));
            }
        }

        if let Some(fd) = read_fd {
            if let Some(line) =
                self.read_redirected_fd(cmd, fd, delimiter, char_limit, exact_char_limit)
            {
                return Some(line);
            }
            if let Some(output) =
                self.read_coproc_stdout(fd, delimiter, char_limit, exact_char_limit)
            {
                self.read_eof_no_delimiter |= !output.contains(&read_delimiter_needle(delimiter));
                return Some(trim_read_input(
                    output,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                ));
            }
            return self
                .read_virtual_fd_stdin(fd, delimiter, char_limit, exact_char_limit)
                .or_else(|| {
                    self.read_heredoc_fd_input(cmd, fd, delimiter, char_limit, exact_char_limit)
                });
        }

        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) != 0 {
                return None;
            }
            if is_closed_redirect_target(&self.expand_redirect_target(redirect)) {
                return None;
            }
            if let Some(source) = redirect
                .target
                .strip_prefix("<(")
                .and_then(|target| target.strip_suffix(')'))
            {
                if let Some(output) = self.process_substitution_output(source) {
                    self.read_eof_no_delimiter |=
                        !output.contains(&read_delimiter_needle(delimiter));
                    return Some(trim_read_input(
                        output,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                    ));
                }
            }

            let expanded_target = self.expand_redirect_target(redirect);
            if let Some(fd) = expanded_target.strip_prefix('&') {
                let fd = fd.trim_matches(|ch| ch == '"' || ch == STORAGE_WORD_PREFIX);
                // The move form `<&N-` (make_cmd.c:704-718) dups fd N onto
                // fd 0 — the same open file description (redir.c:1153), so
                // reading N's endpoint reads the dup.
                let fd = fd.strip_suffix('-').unwrap_or(fd);
                if let Ok(fd) = fd.parse::<u32>() {
                    if let Some(output) =
                        self.read_coproc_stdout(fd, delimiter, char_limit, exact_char_limit)
                    {
                        return Some(output);
                    }
                    if let Some(line) =
                        self.read_virtual_fd_stdin(fd, delimiter, char_limit, exact_char_limit)
                    {
                        return Some(line);
                    }
                    return self.read_heredoc_fd_input(
                        cmd,
                        fd,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                    );
                }
            }

            // niubash#177: same operand resolution as open_input_redirect —
            // a virtual-root input operand may exist only under its `.exe`
            // spelling on disk.
            let path = crate::executor::path::windows_operand_file_path(
                shell_path_to_windows(&expanded_target, &self.shell_state.env_vars),
                &self.shell_state.env_vars,
            );
            if redirect.append {
                let _ = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&path);
            }
            // GNU redir.c opens the target as the command's fd 0; for
            // non-regular inputs (/dev/tty → CON, NUL, a fifo) the read must
            // come byte-wise off the live handle — a slurp-to-EOF would
            // block forever on a device and `read -t` needs the bounded
            // wait. Bind it transiently so fd_table's deadline-aware record
            // reader handles the stream.
            if !std::fs::metadata(&path)
                .map(|m| m.is_file())
                .unwrap_or(false)
            {
                let Ok(file) = FileFd::open_read(path) else {
                    return None;
                };
                let saved = self.fd_table.entries.get(&0).cloned();
                self.fd_table
                    .open_input(0, FdReadEndpoint::File(file), false);
                let line = self.read_virtual_fd_stdin(0, delimiter, char_limit, exact_char_limit);
                match saved {
                    Some(entry) => {
                        self.fd_table.entries.insert(0, entry);
                    }
                    None => {
                        self.fd_table.entries.remove(&0);
                    }
                }
                return line;
            }
            // GNU read receives raw bytes from redir.c-opened inputs. Keep
            // invalid UTF-8 inside the RAW_BYTE_MARKER carrier instead of
            // dropping the whole record at this boundary.
            // Q11 /proc P1: synthetic files are served before the filesystem
            // (docs/proc-vfs-plan.md hook B).
            let input = if let Some(bytes) = crate::proc_vfs::proc_file_content(&expanded_target) {
                crate::executor::substitution_metadata::bytes_to_shell_text(&bytes)
            } else if let Some(bytes) = self.procsub_stream_take(&path) {
                // `<(cmd)` carrier path: the word names a draining
                // stream — serve the shared remainder (subst.c:7143).
                crate::executor::substitution_metadata::bytes_to_shell_text(&bytes)
            } else {
                match crate::executor::substitution_metadata::read_shell_input_file(path) {
                    Ok(text) => text,
                    Err(_) => return None,
                }
            };
            if input.is_empty() {
                return None;
            }
            self.read_eof_no_delimiter |= !input.contains(&read_delimiter_needle(delimiter));
            return Some(trim_read_input(
                input,
                delimiter,
                char_limit,
                exact_char_limit,
            ));
        }

        // Here-strings are command-local input. Persistent virtual fds must
        // bypass `stdin_string_for_command`, whose legacy remaining-text view
        // does not advance the shared fd cursor.
        if cmd.here_string.is_some() {
            if let Some(line) = self.stdin_string_for_command_mut(cmd) {
                self.read_eof_no_delimiter |= !line.contains(&read_delimiter_needle(delimiter));
                return Some(trim_read_input(
                    line,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                ));
            }
        }

        // Persistent fd 0 owns the shared cursor. The legacy mirror can be
        // present for external setup, but must not reset a shell read to offset 0.
        if matches!(
            self.fd_table.read_endpoint(0),
            Some(
                FdReadEndpoint::Text(_)
                    | FdReadEndpoint::ProcessSubstitution(_)
                    | FdReadEndpoint::File(_)
            )
        ) {
            if let Some(line) =
                self.read_virtual_fd_stdin(0, delimiter, char_limit, exact_char_limit)
            {
                return Some(line);
            }
        }

        // If FUNCTION_STDIN is set (from heredoc or redirect), only read from it.
        // Do NOT fall through to process stdin - that would block on the terminal.
        if self.shell_state.env_vars.contains_key(FUNCTION_STDIN) {
            return self.read_function_stdin(delimiter, char_limit, exact_char_limit);
        }

        if let Some(line) = self.read_virtual_fd_stdin(0, delimiter, char_limit, exact_char_limit) {
            return Some(line);
        }

        if self.fd_table.is_closed(0) {
            return None;
        }

        // GNU builtin_read reads ONE descriptor: fd 0 exactly as
        // do_redirections bound it (read.def:294; redir.c:767-955). When
        // the fd table owns fd 0 as a real endpoint — the enclosing
        // compound's `done < file` binding, an `exec 0<` persistent fd, a
        // virtual text fd — the None above IS EOF on that descriptor:
        // read.def:762-768 breaks with eof=1 and :949 returns
        // EXECUTION_FAILURE. GNU has no second transport to fall through
        // to, so reaching for the inherited process stdin here would
        // block forever on the parent's still-open stdin pipe (readhang:
        // `while read -r f; do :; done < list.txt` never saw loop EOF).
        // The mirror → inherited chain below stays reachable only while
        // fd 0 designates the process stdin or has no fd-table entry.
        if self.fd_table.fd0_bound_away_from_process_stdin() {
            return None;
        }

        self.read_function_stdin(delimiter, char_limit, exact_char_limit)
            .or_else(|| self.read_inherited_process_stdin(delimiter, char_limit, exact_char_limit))
    }

    fn read_coproc_stdout(
        &mut self,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        // Bash exposes the coprocess output as COPROC[0] (and NAME[0]).
        // Rubash stores that endpoint as a PipeReader keyed by the child PID.
        // A zero descriptor retains the legacy unnamed-coproc behavior; named
        // coprocess arrays carry their PID as a virtual descriptor.
        // The pipe read end lives in fd_table as a real HANDLE; read_some
        // maps a drained anonymous pipe's ERROR_BROKEN_PIPE to EOF.
        let (pid, pipe) = if fd == 0 {
            self.first_coproc_read()?
        } else if let Some(FdReadEndpoint::CoprocStdout { pid, fd: pipe }) =
            self.fd_table.read_endpoint(fd)
        {
            (pid, pipe)
        } else {
            // Named coprocess arrays carry their PID as a virtual descriptor.
            let pipe = self.coproc_read_file(fd)?;
            (fd, pipe)
        };
        let _ = pid;
        let mut bytes = Vec::new();
        let mut consumed_chars = 0usize;
        let mut ended = false;

        // Bash keeps the coprocess descriptor open across read builtin calls.
        // Read only one logical input record (or the requested character
        // limit), then retain the pipe for the next call instead of
        // draining it and losing unread records.
        loop {
            if self.read_wait_timed_out(pipe.handle) {
                ended = true;
                break;
            }
            match crate::fd::read_some(pipe.handle, 1) {
                Ok(buf) if buf.is_empty() => {
                    ended = true;
                    break;
                }
                Ok(buf) => {
                    bytes.push(buf[0]);
                    if buf[0] == delimiter as u8 && !exact_char_limit {
                        break;
                    }
                    if let Some(limit) = char_limit {
                        consumed_chars += 1;
                        if consumed_chars >= limit {
                            break;
                        }
                    }
                }
                Err(_) => {
                    ended = true;
                    break;
                }
            }
        }

        // read.def:949 eof: the pipe drained (or errored) before the
        // delimiter arrived; a timeout break is not EOF.
        if ended && !self.read_timed_out && !bytes.is_empty() {
            self.read_eof_no_delimiter = !bytes.contains(&(delimiter as u8));
        }
        if ended
            && matches!(
                self.fd_table.read_endpoint(fd),
                Some(FdReadEndpoint::CoprocStdout { .. })
            )
        {
            // EOF closes this shell-owned read capability. Job reaping remains
            // separate so wait can still consume the child's final status.
            self.fd_table.close_input(fd);
        }
        if bytes.is_empty() {
            return None;
        }
        Some(trim_read_input(
            bytes_to_shell_text(&bytes),
            delimiter,
            char_limit,
            exact_char_limit,
        ))
    }

    pub(crate) fn process_substitution_output(&mut self, source: &str) -> Option<String> {
        self.process_substitution_output_bytes(source)
            .map(|output| bytes_to_shell_text(&output))
    }

    /// rubash#396: text behind a `/dev/fd/N` (or `/proc/self/fd/N`) word —
    /// the `source <(cmd)` / `. <(cmd)` route once the expansion layer has
    /// materialized the process substitution into an fd-table endpoint.
    /// GNU source.def hands the expanded word to `_evalfile`, whose open()
    /// (builtins/evalfile.c:104) reopens the pipe end process_substitute
    /// parked at fd >= 64 (subst.c:6392 move_to_high_fd); on Windows the
    /// bytes live in the fd table, so the read consults it — the same
    /// endpoint lookup the comsub operand path uses. `None` (no live
    /// Text/ProcessSubstitution read endpoint at N) falls through to the
    /// filesystem open, which stays correct for real files.
    pub(crate) fn sourced_dev_fd_text(&self, filename: &str) -> Option<String> {
        let fd_text = filename
            .strip_prefix("/dev/fd/")
            .or_else(|| filename.strip_prefix("/proc/self/fd/"))?;
        let fd = fd_text.parse::<u32>().ok()?;
        let (data, offset) = self.fd_table.input_snapshot_bytes(fd)?;
        let content = data.get(offset..)?.to_vec();
        Some(bytes_to_shell_text(&content))
    }

    pub(crate) fn process_substitution_output_bytes(&mut self, source: &str) -> Option<Vec<u8>> {
        let tokens = crate::lexer::tokenize(source);
        let ast = crate::parser::parse(&tokens);
        if ast.commands.is_empty() {
            return None;
        }

        let saved_dir = env::current_dir().ok();
        let mut subshell = self.command_substitution_executor();
        // GNU subst.c:6618 runs the process-substitution body through
        // parse_and_execute ("process substitution") → evalstring.c:348
        // indirection_level++, so `<(...)` bodies trace one PS4 level deeper
        // than the command carrying them (`cat <(echo hi)` under `set -x`:
        // `+ cat /dev/fd/63` then `++ echo hi`) — the same body-reader bump
        // as the command-substitution paths (rubash#254).
        subshell
            .shell_state
            .xtrace_indirection_level
            .set(subshell.shell_state.xtrace_indirection_level.get() + 1);
        crate::builtins::trap::reset_for_subshell(&mut subshell.shell_state.env_vars);
        subshell.stdout_capture = Some(Vec::new());
        // Direct-stdout builtins inside the substitution consult the
        // thread-local capture, which belongs to an enclosing pipeline
        // stage when this substitution runs inside one; give the body its
        // own capture.
        let (thread_captured, result) =
            crate::executor::shell_options::capture_stdout(|| subshell.execute_ast(&ast));
        let mut output = subshell.stdout_capture.take().unwrap_or_default();
        output.extend_from_slice(&thread_captured);
        // GNU process_substitute leaves the fork's status for `wait $!`
        // (subst.c:6362+); rubash ran the substitution inline, so register
        // the finished subshell status under $! before returning.
        let status = match &result {
            Ok(()) => subshell.exit_code,
            Err(ExecuteError::ExitCode(code))
            | Err(ExecuteError::ExpansionFailure(code))
            | Err(ExecuteError::Return(code)) => *code,
            _ => 1,
        };
        self.register_process_substitution_status(status);

        if let Some(saved_dir) = saved_dir {
            let _ = env::set_current_dir(saved_dir);
        }

        match result {
            Ok(())
            | Err(ExecuteError::ExitCode(_))
            | Err(ExecuteError::ExpansionFailure(_))
            | Err(ExecuteError::Return(_)) => Some(output),
            Err(_) => None,
        }
    }

    pub(in crate::executor) fn read_virtual_fd_stdin(
        &mut self,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        if self.fd_table.is_open_for_read(fd) {
            if let Some(line) = self
                .fd_table
                .read_text(fd, delimiter, char_limit, exact_char_limit)
            {
                if let Some((_, offset)) = self.fd_table.input_snapshot(fd) {
                    self.shell_state
                        .env_vars
                        .insert(fd_stdin_offset_key(fd), offset.to_string());
                }
                return Some(trim_read_input(
                    line,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                ));
            }
            if self.fd_table.is_closed(fd) {
                return None;
            }
        }
        // Coprocess readers are stream endpoints, not text mirrors. Once the
        // pipe reaches EOF, do not interpret the legacy environment adapter
        // value as shell input.
        if matches!(
            self.fd_table.read_endpoint(fd),
            Some(FdReadEndpoint::CoprocStdout { .. })
        ) {
            return None;
        }
        if matches!(
            self.fd_table.read_endpoint(fd),
            Some(FdReadEndpoint::InheritedProcessStdin)
        ) {
            return self.read_inherited_process_stdin(delimiter, char_limit, exact_char_limit);
        }
        None
    }

    pub(in crate::executor) fn read_function_stdin(
        &mut self,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        self.apply_comsub_stdin_writeback();
        // GNU read builtin: one zread record off the shared fd-0 cursor
        // (builtins/read.def:731-738) — O(record), never a copy of the
        // remaining stream. The FUNCTION_STDIN mirror must match that
        // cost: borrowing here keeps `while read` over an S-byte pipeline
        // input at O(S) total instead of the O(S) clone per record that
        // made 100k-line loops take minutes and ballooned RSS (rubash
        // issue #206 consumer side).
        let offset = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);

        if self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN)
            .is_none_or(|input| offset >= input.len())
        {
            return None;
        }
        if char_limit == Some(0) {
            return Some(String::new());
        }

        let (output, consumed, delimiter_found, limit_reached) = {
            let input = self.shell_state.env_vars.get(FUNCTION_STDIN)?;
            let slice = &input[offset..];
            let mut output = String::new();
            let mut consumed = 0usize;
            let mut took_any = false;
            let mut delimiter_found = false;
            let mut limit_reached = false;
            let delimiter_needle = read_delimiter_needle(delimiter);
            for (index, ch) in slice.char_indices() {
                if !exact_char_limit && slice[index..].starts_with(&delimiter_needle) {
                    consumed = index + delimiter_needle.len();
                    took_any = true;
                    delimiter_found = true;
                    break;
                }

                output.push(ch);
                consumed = index + ch.len_utf8();
                took_any = true;
                if char_limit.is_some_and(|limit| output.chars().count() >= limit) {
                    limit_reached = true;
                    break;
                }
            }
            if !took_any {
                return None;
            }
            (output, consumed, delimiter_found, limit_reached)
        };

        // read.def:949 eof: the buffered stdin ran out before the
        // delimiter (a -n/-N limit break is not EOF).
        self.read_eof_no_delimiter = !delimiter_found && !limit_reached;
        self.shell_state.env_vars.insert(
            FUNCTION_STDIN_OFFSET.to_string(),
            (offset + consumed).to_string(),
        );
        Some(trim_read_input(
            output,
            delimiter,
            char_limit,
            exact_char_limit,
        ))
    }

    pub(in crate::executor) fn read_inherited_process_stdin(
        &mut self,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        if self
            .shell_state
            .env_vars
            .get(INHERIT_PROCESS_STDIN)
            .map(String::as_str)
            != Some("1")
        {
            return None;
        }
        if char_limit == Some(0) {
            return Some(String::new());
        }

        // GNU builtins/read.def reads fd 0 through zread — one raw syscall
        // per byte. io::stdin() owns a process-wide BufReader that prefetches
        // far past the delimiter, so a sibling process sharing this stdin's
        // file offset (async `{ read; } &` under a redirected compound —
        // redir.tests) would inherit an already-drained descriptor. Read the
        // raw OS handle byte-wise so only the consumed bytes move the offset.
        let stdin_handle = crate::fd::process_std_handle(0);
        let mut output = String::new();
        let mut decoder = StdinCharDecoder::new();
        let mut units = 0usize;
        let mut eof = false;
        loop {
            if !decoder.has_queued() {
                if self.read_wait_timed_out(stdin_handle) {
                    break;
                }
                let buf = crate::fd::read_some(stdin_handle, 1).ok()?;
                if buf.is_empty() {
                    eof = true;
                    break;
                }
                decoder.queue_byte(buf[0]);
            }
            let Some(unit) = decoder.next_unit() else {
                continue;
            };
            if !exact_char_limit && stdin_unit_is_delimiter(&unit, delimiter) {
                break;
            }
            match unit {
                StdinUnit::Char(ch) => {
                    output.push(ch);
                    units += 1;
                }
                StdinUnit::RawByte { text } => {
                    output.push_str(&text);
                    units += 1;
                }
            }
            if char_limit.is_some_and(|limit| units >= limit) {
                break;
            }
        }
        if eof {
            decoder.flush(&mut output);
        }
        if output.is_empty() {
            return None;
        }
        // read.def:949 eof: the handle closed before the delimiter.
        // Served-record invariant: only a returned record may set the eof
        // flag — a None-returning probe must leave it untouched so a later
        // fallback path's record is not poisoned with a bogus EOF status.
        self.read_eof_no_delimiter = eof;

        Some(trim_read_input(
            output,
            delimiter,
            char_limit,
            exact_char_limit,
        ))
    }

    pub(in crate::executor) fn read_inherited_process_stdin_to_string(&self) -> Option<String> {
        if self
            .shell_state
            .env_vars
            .get(INHERIT_PROCESS_STDIN)
            .map(String::as_str)
            != Some("1")
        {
            return None;
        }
        // A captured pipeline-stage stdin IS the command's fd 0, empty or
        // not: execute_cmd.c execute_pipeline dup2s the pipe over fd 0 for
        // every element after the first (execute_cmd.c:2702+), so an empty
        // buffer is the drained pipe — EOF for this command — never an
        // invitation to read the shell's own stdin. The fallback below
        // guards the child-shell case (`printf x | rubash -c 'cat'`, where
        // fd 0 is genuinely the inherited process stdin); inside an
        // in-process pipeline stage (execute_pipeline_stage arms
        // FUNCTION_STDIN for every stage) it would consume input GNU leaves
        // for the shell's own reader — under an interactive driver it
        // drained the session's pending input lines, and on a live console
        // it blocked forever (rubash#436). external_finish's
        // function-stdin arm already removes INHERIT_PROCESS_STDIN for the
        // function-call shape (external_finish.rs function_call_stdin);
        // the pipeline-stage arm keeps the flag armed, so the armed-buffer
        // check here is the boundary.
        if self.shell_state.env_vars.contains_key(FUNCTION_STDIN) {
            return None;
        }

        let mut stdin = io::stdin().lock();
        let mut output = String::new();
        stdin.read_to_string(&mut output).ok()?;
        if output.is_empty() {
            return None;
        }
        Some(output)
    }
}
