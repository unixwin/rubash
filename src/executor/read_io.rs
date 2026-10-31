use super::*;
use crate::executor::substitution_metadata::bytes_to_shell_text;

/// Re-emit a consumed stdin unit to the terminal while a one-char console
/// read is active (the driver's own echo is off in that mode; read.def
/// ttfd_onechar keeps ECHO on through the tty).
fn echo_stdin_unit(guard: &Option<crate::fd::ConsoleOnecharGuard>, unit: &StdinUnit) {
    let Some(guard) = guard.as_ref() else {
        return;
    };
    match unit {
        StdinUnit::Char(ch) => {
            let mut buf = [0u8; 4];
            for byte in ch.encode_utf8(&mut buf).as_bytes() {
                guard.echo_byte(*byte);
            }
        }
        StdinUnit::RawByte { text } => {
            for byte in crate::executor::substitution_metadata::shell_text_to_raw_bytes(text) {
                guard.echo_byte(byte);
            }
        }
    }
}

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
        silent: bool,
        mut line: String,
    ) -> String {
        // GNU read.def:802-830: only an UNESCAPED trailing backslash is a
        // line continuation — an odd trailing run. An even run is `\\`
        // pairs (literal backslash data) and ends the record (rubash#1300).
        while line.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 1 {
            line.pop();
            let Some(next) =
                self.read_input_for_command(cmd, read_fd, silent, '\n', None, false, false, false)
            else {
                // GNU read.def:802-830: the `\<newline>` pair is swallowed
                // and reading continues; a continuation that hits EOF is
                // still an unterminated record (eof=1), so the builtin
                // fails with status 1 even though the partial value is
                // assigned (`printf 'a\\\n' | read x` -> x=a, rc=1). The
                // recursive call above reset read_eof_no_delimiter before
                // its transports answered EOF as "no record" — restore it.
                self.read_eof_no_delimiter = true;
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
        // rubash#1250: wait in slices and poll for an untrapped SIGINT
        // between them — a single full-deadline poll (EINTR-retried in
        // fd::wait_readable) made `read -t 30` ride out the whole timeout
        // after Ctrl-C instead of returning 128+SIGINT at once.
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                self.read_timed_out = true;
                return true;
            }
            if self.take_read_sigint_abort() {
                return true;
            }
            let remaining = deadline - now;
            let slice = remaining.min(std::time::Duration::from_millis(50));
            if crate::fd::wait_readable(handle, slice) == crate::fd::ReadWait::Timeout {
                if slice >= remaining {
                    self.read_timed_out = true;
                    return true;
                }
                continue;
            }
            return false;
        }
    }

    /// rubash#1250: consume an untrapped SIGINT routed to the pending
    /// mailbox by the foreground guard held across this read (the same
    /// partition ForegroundBuiltinSigintGuard::take_sigint_abort
    /// performs for the `sleep` fast path): the SIGINT is consumed, any
    /// other pending signal is re-queued for its own boundary handling.
    /// On success the caller abandons the read WITHOUT assigning (GNU
    /// read.def: an interrupted read returns 128+SIGINT and the
    /// variables keep their previous values).
    fn take_read_sigint_abort(&mut self) -> bool {
        if !self.read_sigint_guard_armed || self.read_interrupted_signal.is_some() {
            return self.read_interrupted_signal.is_some();
        }
        #[cfg(unix)]
        {
            let pending = crate::builtins::kill::take_pending_signals_now(std::process::id())
                .unwrap_or_default();
            if pending.is_empty() {
                return false;
            }
            let mut rest = Vec::new();
            let mut interrupted = false;
            for signal in pending {
                if signal == libc::SIGINT && crate::builtins::kill::kernel_handler_installed(signal)
                {
                    interrupted = true;
                } else {
                    rest.push(signal);
                }
            }
            crate::builtins::kill::requeue_pending_signals(rest);
            if interrupted {
                self.read_interrupted_signal = Some(libc::SIGINT);
                return true;
            }
        }
        false
    }

    pub(in crate::executor) fn read_input_for_command(
        &mut self,
        cmd: &CommandNode,
        read_fd: Option<u32>,
        silent: bool,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        // GNU builtins/read.def: NUL bytes in the input are skipped as
        // the record is read — they never reach the assigned value,
        // the IFS field splitting, or a -a array (rubash#1419:
        // `printf 'a\0b\n' | read -r v` binds "ab", not "a\0b"). Every
        // transport below serves the record with NULs intact, so strip
        // them at this single funnel. The delimiter itself is excluded
        // by the readers, and a NUL delimiter stops the record before
        // any NUL can appear, so `read -d ''` is unaffected.
        let line = self.read_input_for_command_inner(
            cmd,
            read_fd,
            silent,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        )?;
        // A record WAS served (possibly by a fallback transport after a
        // probed endpoint failed): any recorded hard read error belongs
        // to the abandoned probe, not to this read (rubash#1495).
        self.fd_table.read_error = None;
        Some(if line.contains('\0') {
            line.replace('\0', "")
        } else {
            line
        })
    }

    /// rubash#2339: the `read -e`/`-E` editor acquisition — GNU
    /// read.def:685-710 routes the read through edit_line when the
    /// input descriptor is a terminal. Returns Some(result) when the
    /// editor owned the acquisition (result mirrors the funnel: the
    /// edited record, or None with the eof/timeout/interrupt flags
    /// set), and None when the gate fails — edit was cleared by the
    /// caller's circumstances exactly as read.def:489-497 clears it —
    /// so the caller falls through to the raw transports untouched.
    ///
    /// The gate: no staged body may feed the read descriptor (a
    /// heredoc/herestring IS the input, never a terminal), and the
    /// descriptor must be the process's own stdin (or a `-u` dup of
    /// it) with a real terminal behind it. A `< file`, a pipe, a
    /// procsub or a `< /dev/tty` open all bind concrete endpoints and
    /// stay on the raw path (the /dev/tty spelling is the known edge:
    /// GNU would edit there; the endpoint cannot be told apart from
    /// any other opened file here).
    fn try_read_with_line_editor(
        &mut self,
        cmd: &CommandNode,
        read_fd: Option<u32>,
        silent: bool,
        delimiter: char,
        char_limit: Option<usize>,
    ) -> Option<Option<String>> {
        if matches!(read_fd, None | Some(0))
            && (cmd.heredoc_body.is_some() || cmd.heredoc.is_some() || cmd.here_string.is_some())
        {
            return None;
        }
        let fd: i32 = match read_fd {
            None | Some(0) => {
                if self.fd_table.fd0_bound_away_from_process_stdin() {
                    return None;
                }
                0
            }
            Some(n) => {
                if !matches!(
                    self.fd_table.read_endpoint(n),
                    Some(FdReadEndpoint::InheritedProcessStdin)
                ) {
                    return None;
                }
                n as i32
            }
        };
        let history: Vec<String> = self
            .get_session_history()
            .map(|history| history.borrow().entries.clone())
            .unwrap_or_default();
        let initial = self.read_edit_initial_text.as_deref().unwrap_or("");
        let mut editor = crate::readline_editor::LineEditor::new(
            initial,
            history,
            char_limit,
            if delimiter == '\n' {
                None
            } else {
                Some(delimiter)
            },
        );
        let registry = self.shell_state.bind_registry.clone();
        let mut kill_ring = registry.borrow().kill_ring.clone();
        let outcome = self.pump_line_editor(&mut editor, &mut kill_ring, fd, silent);
        registry.borrow_mut().kill_ring = kill_ring;
        outcome
    }

    /// Repaint the edited line on stderr (readline's rl_redisplay
    /// surface). Silent reads (-s) never repaint: the typed text must
    /// not leak (the rs15/rs16 pins).
    fn repaint_read_editor(&self, editor: &crate::readline_editor::LineEditor, clear: bool) {
        use std::io::Write;
        let prompt = self.read_edit_prompt.as_deref().unwrap_or("");
        let mut stderr = std::io::stderr().lock();
        if clear {
            let _ = stderr.write_all(b"\x1b[H\x1b[2J");
        }
        let frame = crate::console_readline::redraw_line(prompt, editor.buffer(), editor.cursor());
        let _ = stderr.write_all(frame.as_bytes());
        let _ = stderr.flush();
    }

    /// Feed decoded characters into the editor until it ends the
    /// session; returns the funnel-shaped outcome, or None when this
    /// platform/fd has no terminal editor (gate failure — the caller
    /// falls through to the raw transports).
    #[cfg(unix)]
    fn pump_line_editor(
        &mut self,
        editor: &mut crate::readline_editor::LineEditor,
        kill_ring: &mut Vec<String>,
        fd: i32,
        silent: bool,
    ) -> Option<Option<String>> {
        use crate::readline_editor::{EditEnd, Feed};
        if unsafe { libc::isatty(fd) } != 1 {
            return None;
        }
        let _tty_guard = crate::readline_editor::TtyRawGuard::new(fd)?;
        if !silent {
            self.repaint_read_editor(editor, false);
        }
        let mut pending: Vec<u8> = Vec::new();
        loop {
            // Slice discipline (the read loops' own): poll in short
            // slices so an untrapped SIGINT and the -t deadline are
            // honored while the editor waits (read.def's readline
            // timeout/interrupt shape).
            if self.take_read_sigint_abort() {
                return Some(None);
            }
            if let Some(deadline) = self.read_deadline {
                if std::time::Instant::now() >= deadline {
                    self.read_timed_out = true;
                    let partial = editor.buffer().to_string();
                    if partial.is_empty() {
                        return Some(None);
                    }
                    self.read_eof_no_delimiter = true;
                    return Some(Some(partial));
                }
            }
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, 50) };
            if ready < 0 {
                if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Some(self.read_editor_stream_eof(editor));
            }
            if ready == 0 {
                continue;
            }
            if pfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 && pfd.revents & libc::POLLIN == 0
            {
                return Some(self.read_editor_stream_eof(editor));
            }
            let mut buf = [0u8; 4096];
            let count = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if count < 0 {
                if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Some(self.read_editor_stream_eof(editor));
            }
            if count == 0 {
                return Some(self.read_editor_stream_eof(editor));
            }
            pending.extend_from_slice(&buf[..count as usize]);
            let chars = drain_decoded_chars(&mut pending);
            let mut clear = false;
            let mut ended: Option<Option<String>> = None;
            for c in chars {
                match editor.feed(c, kill_ring) {
                    Feed::Continue => {}
                    Feed::ClearScreen => clear = true,
                    Feed::End(EditEnd::Accepted(line)) => {
                        if !silent {
                            use std::io::Write;
                            let _ = std::io::stderr().write_all(b"\n");
                        }
                        ended = Some(Some(line));
                        break;
                    }
                    Feed::End(EditEnd::Eof) => {
                        self.read_eof_no_delimiter = true;
                        ended = Some(None);
                        break;
                    }
                    Feed::End(EditEnd::EofPartial(partial)) => {
                        self.read_eof_no_delimiter = true;
                        ended = Some(Some(partial));
                        break;
                    }
                }
            }
            if let Some(result) = ended {
                return Some(result);
            }
            if !silent {
                self.repaint_read_editor(editor, clear);
            }
        }
    }

    /// The Windows console pump: RawConsole delivers whole key events
    /// as char batches (ENABLE_PROCESSED_INPUT is off in raw mode, so
    /// Ctrl-C arrives as a 0x03 char rather than a signal — it aborts
    /// the read like the Unix SIGINT path, 128+SIGINT, assigning
    /// nothing). The deadline is checked between key events; a -t
    /// expiry mid-wait resolves at the next keypress.
    #[cfg(windows)]
    fn pump_line_editor(
        &mut self,
        editor: &mut crate::readline_editor::LineEditor,
        kill_ring: &mut Vec<String>,
        fd: i32,
        silent: bool,
    ) -> Option<Option<String>> {
        use crate::readline_editor::{EditEnd, Feed};
        if fd != 0 {
            return None;
        }
        let mut console = crate::console_readline::RawConsole::acquire()?;
        if !silent {
            self.repaint_read_editor(editor, false);
        }
        loop {
            if let Some(deadline) = self.read_deadline {
                if std::time::Instant::now() >= deadline {
                    self.read_timed_out = true;
                    let partial = editor.buffer().to_string();
                    if partial.is_empty() {
                        return Some(None);
                    }
                    self.read_eof_no_delimiter = true;
                    return Some(Some(partial));
                }
            }
            let batch = match console.next_key() {
                Ok(Some(chars)) => chars,
                Ok(None) => return Some(self.read_editor_stream_eof(editor)),
                Err(_) => return Some(self.read_editor_stream_eof(editor)),
            };
            if batch.contains(&'\x03') {
                self.read_interrupted_signal = Some(2);
                return Some(None);
            }
            let mut clear = false;
            let mut ended: Option<Option<String>> = None;
            for c in batch {
                match editor.feed(c, kill_ring) {
                    Feed::Continue => {}
                    Feed::ClearScreen => clear = true,
                    Feed::End(EditEnd::Accepted(line)) => {
                        if !silent {
                            use std::io::Write;
                            let _ = std::io::stderr().write_all(b"\n");
                        }
                        ended = Some(Some(line));
                        break;
                    }
                    Feed::End(EditEnd::Eof) => {
                        self.read_eof_no_delimiter = true;
                        ended = Some(None);
                        break;
                    }
                    Feed::End(EditEnd::EofPartial(partial)) => {
                        self.read_eof_no_delimiter = true;
                        ended = Some(Some(partial));
                        break;
                    }
                }
            }
            if let Some(result) = ended {
                return Some(result);
            }
            if !silent {
                self.repaint_read_editor(editor, clear);
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn pump_line_editor(
        &mut self,
        _editor: &mut crate::readline_editor::LineEditor,
        _kill_ring: &mut Vec<String>,
        _fd: i32,
        _silent: bool,
    ) -> Option<Option<String>> {
        None
    }

    /// The editor's input stream ended (read EOF, hangup, or a hard
    /// read error): a non-empty buffer is the unterminated record
    /// (read.def:949), an empty one is plain EOF.
    fn read_editor_stream_eof(
        &mut self,
        editor: &crate::readline_editor::LineEditor,
    ) -> Option<String> {
        self.read_eof_no_delimiter = true;
        let partial = editor.buffer();
        if partial.is_empty() {
            None
        } else {
            Some(partial.to_string())
        }
    }

    fn read_input_for_command_inner(
        &mut self,
        cmd: &CommandNode,
        read_fd: Option<u32>,
        silent: bool,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        // rubash#1250: hold untrapped SIGINT at the mailbox handler for
        // the whole acquisition, like the `sleep` fast path: an
        // interactive `read -t 30` (or a plain tty read) otherwise sat
        // in its blocking read while Ctrl-C waited for a command
        // boundary that could never arrive. The blocking loops below
        // poll take_read_sigint_abort in slices and abandon the read
        // with status 128+SIGINT, assigning nothing.
        #[cfg(unix)]
        let _sigint_guard =
            super::external_inner::ForegroundBuiltinSigintGuard::hold_if_interactive_untrapped(
                self,
            );
        #[cfg(unix)]
        {
            self.read_sigint_guard_armed = _sigint_guard.is_armed();
        }
        // An interrupt consumed by an earlier transport in this same
        // acquisition ends it: no second transport may block again.
        if self.read_interrupted_signal.is_some() {
            return None;
        }
        // read.def:949 eof bookkeeping: each fresh record read starts with
        // the delimiter unsatisfied; the leaf readers below set the flag
        // when they exhaust their input before finding it.
        self.read_eof_no_delimiter = false;
        self.fd_table.read_eof_no_delimiter = false;
        // niubash#684: same reset discipline for the process-stdin EOF
        // verdict — it describes THIS acquisition only.
        self.read_process_stdin_eof = false;
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
        // rubash#721: `< /dev/fd/N` with N closed is an open failure, not an
        // empty input. GNU opens the alias through the kernel and reports
        // ENOENT (`read -r z < /dev/fd/6` -> "/dev/fd/6: No such file or
        // directory", rc 1). The alias branch below stages an empty mirror,
        // which made `read` see plain EOF and return 1 with no diagnostic.
        if read_fd.is_none() {
            for redirect in &cmd.redirects {
                if redirect.kind != crate::parser::RedirectKind::Input
                    || redirect.fd.unwrap_or(0) != 0
                    || redirect.fd_var.is_some()
                {
                    continue;
                }
                let Some(source) = super::execution_misc::dev_stdio_redirect_fd(&redirect.target)
                else {
                    continue;
                };
                if !super::redirection::dup_source_fd_open(
                    &self.fd_table,
                    source,
                    &std::collections::HashMap::new(),
                ) {
                    let mut stderr = Vec::new();
                    let _ = writeln!(
                        &mut stderr,
                        "{}{}: No such file or directory",
                        self.diagnostic_prefix(),
                        redirect.target
                    );
                    let _ = self.write_redirected_command_stderr(cmd, &stderr);
                    return None;
                }
            }
        }
        // rubash#2339: `read -e`/`-E` with a terminal behind the read
        // descriptor acquires through the line editor instead of the
        // raw scanner (read.def:685-710 edit_line). The gate inside
        // decides; on any non-terminal shape it declines and the
        // transports below run untouched (read.def:489-497 clears
        // edit/prompt/initial-text for non-tty input).
        if self.read_edit_armed {
            if let Some(result) =
                self.try_read_with_line_editor(cmd, read_fd, silent, delimiter, char_limit)
            {
                return result;
            }
        }
        // An unnumbered heredoc is the last stdin redirect, so it overrides
        // an earlier `<&fd`. An explicit `read -u N` still owns the input fd.
        if read_fd.is_none() {
            if cmd.heredoc_body.is_some() {
                let input = self.expand_heredoc_body_mut_from_carrier(&cmd.heredoc_body);
                self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                    &input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                );
                return Some(trim_read_input(
                    input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                ));
            }
            if let Some(heredoc) = &cmd.heredoc {
                let input = self.expand_heredoc_body_mut(heredoc);
                self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                    &input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                );
                return Some(trim_read_input(
                    input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                ));
            }
        }

        // GNU read.def `-u N` selects the descriptor builtin_read zreads;
        // `-u 0` names fd 0 itself, so it is the ordinary fd-0 read below,
        // not a numbered-fd serve — the fd-table-only transports would
        // report EOF where a pipeline stage parks its input on the
        // FUNCTION_STDIN mirror (rubash#938).
        if let Some(fd) = read_fd.filter(|fd| *fd != 0) {
            // rubash#2305: the command's OWN redirect list is
            // authoritative for the descriptor — GNU applies it over
            // the ambient state before the builtin runs (redir.c:237
            // do_redirections). A heredoc or dup own binding therefore
            // serves BEFORE the ambient transports below; the historic
            // per-kind order (own file, coproc, ambient table entry,
            // own heredoc as a last resort) let an ambient entry
            // shadow the effective own binding, and a self-dup is
            // GNU's no-op (redir.c:1128 `redir_fd != redirector`), so
            // it keeps the ambient serve.
            match self.own_input_binding_scan(cmd, fd, None) {
                Some((_position, super::read_redirected_fd::OwnInputBinding::Heredoc)) => {
                    return self.read_heredoc_fd_input(
                        cmd,
                        fd,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                }
                Some((
                    dup_position,
                    super::read_redirected_fd::OwnInputBinding::Duplicate(source),
                )) if source != fd => {
                    return self.read_dup_fd_input(
                        cmd,
                        source,
                        dup_position,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                }
                Some((_position, super::read_redirected_fd::OwnInputBinding::Close)) => {
                    return None
                }
                _ => {}
            }
            if let Some(line) = self.read_redirected_fd(
                cmd,
                fd,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ) {
                return Some(line);
            }
            if let Some(output) = self.read_coproc_stdout(
                fd,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ) {
                // read.def:949 EOF accounting is owned by
                // read_coproc_stdout itself: it sets read_eof_no_delimiter
                // only when the pipe ended before the delimiter arrived.
                // Re-deriving EOF here from the TRIMMED record inverted
                // every success case — trim_read_input has already removed
                // the delimiter, so a full line (or a -n/-N limit stop)
                // always looked like EOF (rubash#1623).
                return Some(trim_read_input(
                    output,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                ));
            }
            // A dup of fd 0 (`exec 8<&0; read -u 8`) shares fd 0's open file
            // description (redir.c dup via fd_table dup_entry), so once the
            // descriptor reads as the inherited process stdin it is served
            // from the SAME transports as an fd 0 read — GNU builtin_read
            // reads one descriptor and the `-u N` shape must not be a weaker
            // read than `read` (niubash#279: the inherited-stdin endpoint
            // returned None here and the console keystroke never reached
            // any reader).
            if fd != 0
                && matches!(
                    self.fd_table.read_endpoint(fd),
                    Some(FdReadEndpoint::InheritedProcessStdin)
                )
            {
                return self.read_dup_of_stdin(
                    silent,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                );
            }
            return self
                .read_virtual_fd_stdin(
                    fd,
                    silent,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                )
                .or_else(|| {
                    self.read_heredoc_fd_input(
                        cmd,
                        fd,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    )
                });
        }

        // GNU redir.c:1128 do_redirection_internal gates a descriptor
        // dup's whole action — the dup2 AND the r_move_input close — on
        // `redir_fd != redirector`: `read x <&0`, `read x <&0-`,
        // `read x </dev/stdin` and the `/dev/fd/0` / `/proc/self/fd/0`
        // aliases all leave fd 0 bound exactly as it was. The builtin
        // must therefore read the ordinary fd-0 transports below like a
        // bare `read`; the descriptor serve only consults the fd table
        // and reports EOF where the pipeline stage's FUNCTION_STDIN
        // mirror holds the pipe data (rubash#938).
        // GNU redir.c: last fd-0 redirect wins (rubash#1391). If the last
        // stdin redirect is a herestring, the file-redirect path below must
        // be skipped even when cmd.redirect_in is set.
        let last_stdin_is_herestring = cmd
            .redirects
            .iter()
            .rev()
            .find(|r| {
                r.fd.unwrap_or(0) == 0
                    && matches!(
                        r.kind,
                        crate::parser::RedirectKind::Input
                            | crate::parser::RedirectKind::HereDoc
                            | crate::parser::RedirectKind::HereString
                            | crate::parser::RedirectKind::DuplicateInput
                            | crate::parser::RedirectKind::CloseInput
                    )
            })
            .is_some_and(|r| matches!(r.kind, crate::parser::RedirectKind::HereString));
        let fd0_self_dup = cmd.redirect_in.as_deref().is_some_and(|redirect| {
            redirect.fd_var.is_none()
                && redirect.fd.unwrap_or(0) == 0
                && matches!(
                    redirect_target_fd_and_move(&self.expand_redirect_target(redirect)),
                    Some((0, _))
                )
        });
        if let Some(redirect) = cmd
            .redirect_in
            .as_deref()
            .filter(|_| !fd0_self_dup && !last_stdin_is_herestring)
        {
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
                // rubash#1952: stream first — serve the record from the
                // spawned body's live pipe through a scratch fd (GNU
                // subst.c:6362 process_substitute forks the body; the
                // record reader observes real arrival). The timed
                // collection below is the spawn-failure fallback.
                if let Some(fd) = self.allocate_dynamic_fd() {
                    if let Some(file_fd) =
                        self.spawn_input_process_substitution_child(source, cmd.line)
                    {
                        self.fd_table
                            .open_input(fd, FdReadEndpoint::File(file_fd), true);
                        let line = self.fd_table.read_text(
                            fd,
                            delimiter,
                            char_limit,
                            exact_char_limit,
                            escape_aware,
                            raw,
                        );
                        self.fd_table.entries.remove(&fd);
                        return Some(match line {
                            Some(line) => trim_read_input(
                                line,
                                delimiter,
                                char_limit,
                                exact_char_limit,
                                escape_aware,
                                raw,
                            ),
                            None => {
                                if !self.fd_table.read_timed_out {
                                    self.read_eof_no_delimiter = true;
                                }
                                String::new()
                            }
                        });
                    }
                }
                if let Some((output, body_timing)) =
                    self.process_substitution_output_bytes_timed(source)
                {
                    // rubash#1141: serve the record through a scratch
                    // fd-table endpoint so the substitution body's
                    // production schedule judges it like any other
                    // materialized stream (fd_table::read_bytes).
                    if let Some(fd) = self.allocate_dynamic_fd() {
                        self.fd_table.open_input(
                            fd,
                            FdReadEndpoint::process_substitution_bytes(output),
                            true,
                        );
                        if let Some(timing) = body_timing {
                            self.fd_table.set_input_timing(fd, timing);
                        }
                        let line = self.fd_table.read_text(
                            fd,
                            delimiter,
                            char_limit,
                            exact_char_limit,
                            escape_aware,
                            raw,
                        );
                        self.fd_table.entries.remove(&fd);
                        return Some(match line {
                            Some(line) => trim_read_input(
                                line,
                                delimiter,
                                char_limit,
                                exact_char_limit,
                                escape_aware,
                                raw,
                            ),
                            None => {
                                if !self.fd_table.read_timed_out {
                                    self.read_eof_no_delimiter = true;
                                }
                                String::new()
                            }
                        });
                    }
                    let output = bytes_to_shell_text(&output);
                    // e3 (niubash#536) accounting: the pre-e3 formula
                    // `!record_contains_delimiter(..)` misreports EOF when
                    // a -n/-N limit stopped the trim before the end of the
                    // slurped text; replay the per-unit walk instead.
                    self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                        &output,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                    return Some(trim_read_input(
                        output,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    ));
                }
            }

            let expanded_target = self.expand_redirect_target(redirect);
            // `<&N`, the move form `<&N-` (make_cmd.c:704-718), and the
            // fd-alias paths (`/dev/stdin`, `/dev/fd/N`, `/proc/self/fd/N`)
            // all name a descriptor. GNU redir.c opens the alias device
            // file, which the kernel resolves to a dup of fd N — reading it
            // reads that descriptor (redir.c:1153). Windows has no such
            // filesystem (shell_path_to_windows maps `/dev/stdin` to CONIN$,
            // so the builtin blocked on the console instead of the piped
            // fd 0: niubash#700; `/dev/fd/N` opened a nonexistent path and
            // read empty: niubash#701). Resolve the alias to its descriptor
            // here and serve it through the same fd-table/dup machinery as
            // `<&N`.
            if let Some((fd, _move_fd)) = redirect_target_fd_and_move(&expanded_target) {
                // rubash#2256 (wave 2): the serves below model a DUP
                // (the `<&N` word: the consumer shares fd N's stream).
                // A path SPELLING (`< /dev/fd/N`, kind Input) is an
                // open() instead: for a regular-file source the
                // resolver's fresh open reads from offset 0 and
                // leaves fd N's own offset unmoved, so repeated
                // `read x < /dev/fd/6` re-reads the first record
                // instead of walking the shared offset.
                if !matches!(redirect.kind, crate::parser::RedirectKind::DuplicateInput) {
                    // rubash#2276: a socket endpoint's spelling-open
                    // fails ENXIO in GNU — decline the serve (the
                    // read reports no input, its failure shape; the
                    // dup machinery below is the `<&N` descriptor
                    // face and must keep streaming sockets).
                    if self.fd_alias_spelling_open_error(fd).is_some() {
                        return None;
                    }
                    if let Some(path) = self.fd_alias_reopen_path(fd) {
                        if let Ok(fresh) = FileFd::open_read(path) {
                            if let Some(scratch) = self.allocate_dynamic_fd() {
                                self.fd_table.open_input(
                                    scratch,
                                    FdReadEndpoint::File(fresh),
                                    true,
                                );
                                let line = self.fd_table.read_text(
                                    scratch,
                                    delimiter,
                                    char_limit,
                                    exact_char_limit,
                                    escape_aware,
                                    raw,
                                );
                                self.fd_table.entries.remove(&scratch);
                                return Some(match line {
                                    Some(line) => trim_read_input(
                                        line,
                                        delimiter,
                                        char_limit,
                                        exact_char_limit,
                                        escape_aware,
                                        raw,
                                    ),
                                    None => {
                                        if !self.fd_table.read_timed_out {
                                            self.read_eof_no_delimiter = true;
                                        }
                                        String::new()
                                    }
                                });
                            }
                        }
                    }
                }
                // `<&8` after `exec 8<&0` — a dup of the process stdin
                // reads from the same transports as fd 0 itself
                // (niubash#279: the console keystroke must reach the
                // dup descriptor's reader, not collapse to EOF).
                if fd != 0
                    && matches!(
                        self.fd_table.read_endpoint(fd),
                        Some(FdReadEndpoint::InheritedProcessStdin)
                    )
                {
                    return self.read_dup_of_stdin(
                        silent,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                }
                if let Some(output) = self.read_coproc_stdout(
                    fd,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                ) {
                    return Some(output);
                }
                if let Some(line) = self.read_virtual_fd_stdin(
                    fd,
                    silent,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                ) {
                    return Some(line);
                }
                return self.read_heredoc_fd_input(
                    cmd,
                    fd,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                );
            }

            // rubash#1917/#1900: synthetic /proc targets are judged by
            // the registry on their SHELL spelling, before any path
            // translation. The translated path's metadata never exists
            // for them on Windows, so the gate below misrouted them
            // into the device branch and the read failed (or read
            // empty) — the same ladder-skip family as rubash#1904's
            // $(<file) face. Content slurps here, exactly as the serve
            // further down does; a link resolves to its native target
            // and continues through the ordinary file flow with it
            // (the session cwd answers for self/cwd, per G-02).
            if let Some(bytes) = crate::special_paths::synthetic_proc_content(&expanded_target) {
                let input = crate::executor::substitution_metadata::bytes_to_shell_text(&bytes);
                if input.is_empty() {
                    return None;
                }
                self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                    &input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                );
                return Some(trim_read_input(
                    input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                ));
            }
            // rubash#2091: a /dev/tcp|udp target is judged by the
            // registry on its shell spelling too — it has no filesystem
            // path at all, so the translated-path machinery below
            // served a silent EOF and the connection GNU's
            // do_redirections performs (redir.c netopen) never
            // happened, for `<>` and plain `<` alike. Connect once here
            // and serve the record off the live socket, bound
            // transiently as fd 0 like the device branch below; a
            // failed connect is the redirection's failure, reported in
            // the serve like the dangling-alias face above
            // (rubash#721).
            if crate::special_paths::is_net_redirect_target(&expanded_target) {
                // rubash#2124: the simple-command funnel's bind may
                // already have connected this command's winning net
                // input and bound it on fd 0 — serve the bound
                // endpoint instead of connecting a second time. The
                // serve result is authoritative: the endpoint IS this
                // target's stream, so its EOF is the read's EOF.
                if self.dispatch_net_bound_fds.contains(&0) {
                    return self.read_virtual_fd_stdin(
                        0,
                        silent,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                }
                match self.open_net_redirect(&expanded_target) {
                    Ok(file) => {
                        let file = FileFd::from_file(file, expanded_target.clone().into());
                        let saved = self.fd_table.entries.get(&0).cloned();
                        self.fd_table
                            .open_input(0, FdReadEndpoint::File(file), false);
                        let line = self.read_virtual_fd_stdin(
                            0,
                            silent,
                            delimiter,
                            char_limit,
                            exact_char_limit,
                            escape_aware,
                            raw,
                        );
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
                    Err(error) => {
                        let mut stderr = Vec::new();
                        let _ = writeln!(
                            &mut stderr,
                            "{}{}",
                            self.diagnostic_prefix(),
                            crate::posix_errors::message(&error)
                        );
                        let _ = self.write_redirected_command_stderr(cmd, &stderr);
                        return None;
                    }
                }
            }
            let link_native = crate::special_paths::synthetic_link_target(
                &expanded_target,
                Some(&crate::proc_vfs::self_cwd_forms(self.session_cwd())),
            )
            .map(|link| link.native);

            // niubash#177: same operand resolution as open_input_redirect —
            // a virtual-root input operand may exist only under its `.exe`
            // spelling on disk.
            let path = match &link_native {
                Some(native) => std::path::PathBuf::from(native),
                None => self.session_path(crate::executor::path::windows_operand_file_path(
                    shell_path_to_windows(&expanded_target, &self.shell_state.env_vars),
                    &self.shell_state.env_vars,
                )),
            };
            if redirect.append {
                let _ = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&path);
            }
            // rubash#1903: a WinuxCmd fifo marker is a regular file on
            // disk, so the live-handle branch below would bind the
            // marker file itself and the slurp further down would read
            // its bytes. Bind the rendezvous read handle transiently
            // instead — the same record-wise live read, over the pipe.
            #[cfg(windows)]
            if crate::winuxcmd_fifo::path_is_winuxcmd_fifo(&path) {
                let Ok(server) = crate::winuxcmd_fifo::open_winuxcmd_fifo_read(&path, &|| {
                    crate::winuxcmd_fifo::fifo_wait_interrupted(&self.shell_state.env_vars)
                }) else {
                    return None;
                };
                let file = FileFd::from_file(server, path.clone());
                let saved = self.fd_table.entries.get(&0).cloned();
                self.fd_table
                    .open_input(0, FdReadEndpoint::File(file), false);
                let line = self.read_virtual_fd_stdin(
                    0,
                    silent,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                );
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
                let line = self.read_virtual_fd_stdin(
                    0,
                    silent,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                );
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
            // rubash#1141: a timed process-substitution stream is served
            // through a scratch fd-table endpoint so the body's
            // production schedule judges the record (fd_table::read_bytes)
            // — the plain text serve below would hand the consumer bytes
            // that, on GNU's pipe, had not arrived by the deadline. The
            // take drains the stream exactly like the text serve does.
            let procsub_taken = self.procsub_stream_take_timed(&path);
            if let Some((bytes, timing, base)) = &procsub_taken {
                if let Some(timing) = timing {
                    if let Some(shifted) = timing.borrow().shifted(*base) {
                        if let Some(fd) = self.allocate_dynamic_fd() {
                            self.fd_table.open_input(
                                fd,
                                FdReadEndpoint::process_substitution_bytes(bytes.clone()),
                                true,
                            );
                            self.fd_table.set_input_timing(fd, shifted.shared());
                            let line = self.fd_table.read_text(
                                fd,
                                delimiter,
                                char_limit,
                                exact_char_limit,
                                escape_aware,
                                raw,
                            );
                            self.fd_table.entries.remove(&fd);
                            return Some(match line {
                                Some(line) => trim_read_input(
                                    line,
                                    delimiter,
                                    char_limit,
                                    exact_char_limit,
                                    escape_aware,
                                    raw,
                                ),
                                None => {
                                    if !self.fd_table.read_timed_out {
                                        self.read_eof_no_delimiter = true;
                                    }
                                    String::new()
                                }
                            });
                        }
                    }
                }
            }
            // GNU read receives raw bytes from redir.c-opened inputs. Keep
            // invalid UTF-8 inside the RAW_BYTE_MARKER carrier instead of
            // dropping the whole record at this boundary.
            // rubash#1937: a synthetic-link target resolving to a
            // directory (/proc/self/cwd) — GNU's read fails EISDIR on
            // the descriptor itself (read.def:911). Record the read
            // error so the builtin renders
            // `read: 0: read error: Is a directory` instead of serving
            // the mapped literal path's unrelated failure. The facts
            // are a stub off Windows, where the real directory read
            // below produces the same failure natively.
            if crate::proc_vfs::proc_unary_facts(&expanded_target)
                .is_some_and(|facts| facts.is_link && facts.link_target_is_dir)
            {
                self.fd_table.read_error = Some("Is a directory".to_string());
                return None;
            }
            // Q11 /proc P1: synthetic files are served before the filesystem
            // (docs/proc-vfs-plan.md hook B).
            let input = if let Some(bytes) =
                crate::special_paths::synthetic_proc_content(&expanded_target)
            {
                crate::executor::substitution_metadata::bytes_to_shell_text(&bytes)
            } else if let Some((bytes, _, _)) = procsub_taken {
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
            self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                &input,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            );
            return Some(trim_read_input(
                input,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ));
        }

        // Here-strings are command-local input. Persistent virtual fds must
        // bypass `stdin_string_for_command`, whose legacy remaining-text view
        // does not advance the shared fd cursor.
        // rubash#1966: the body may live in EITHER store — the parser
        // places an unnumbered `<<<` in cmd.here_string but a numbered
        // `0<<<` (and `0<<`) in heredoc_redirects with fd == Some(0)
        // (parser/redirections.rs assign_here_string_redirect_raw).
        // stdin_string_for_command_mut resolves both stores (its
        // fd-0-winner logic owns the ordering); this funnel's admission
        // check looked only at the unnumbered field, so a numbered
        // here-string was invisible to `read` and the builtin fell
        // through to the process stdin (EOF / a block) instead.
        let has_fd0_body = cmd.here_string.is_some()
            || cmd
                .heredoc_redirects
                .iter()
                .any(|redirect| redirect.fd == Some(0));
        if has_fd0_body {
            if let Some(line) = self.stdin_string_for_command_mut(cmd) {
                self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                    &line,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                );
                return Some(trim_read_input(
                    line,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                ));
            }
        }

        // Persistent fd 0 owns the shared cursor. The legacy mirror can be
        // present for external setup, but must not reset a shell read to offset 0.
        // A FILE binding from an explicit stdin redirection is a real open
        // file description (`done < file` / `exec 0<file`): GNU builtin_read
        // reads one descriptor (read.def:294), so EOF on the file is final —
        // falling through to the FUNCTION_STDIN mirror would serve the
        // enclosing pipeline stage's stdin as the file's trailing record and
        // leave that mirror consumed (niubash#276: `while read; done < f`
        // then `read` saw a false EOF).
        // EXCEPTION — the null device on fd 0 is not a user binding: async
        // materialization parks the /dev/null default there, and the staged
        // FUNCTION_STDIN mirror IS the compound's own input in that state
        // (redir.tests redir5-adjacent heredoc loop: EOF on NUL must fall
        // through to the mirror or the loop's first read reports EOF and
        // never runs). NUL carries no records, so this cannot reintroduce
        // the #276 contamination — there is no file record to steal.
        let fd0_file_is_null = match self.fd_table.read_endpoint(0) {
            Some(FdReadEndpoint::File(ref file)) => {
                crate::executor::is_null_device(&file.path.to_string_lossy())
            }
            _ => false,
        };
        if !fd0_file_is_null
            && matches!(
                self.fd_table.read_endpoint(0),
                Some(FdReadEndpoint::File(_))
            )
        {
            return self.read_virtual_fd_stdin(
                0,
                silent,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            );
        }
        if fd0_file_is_null
            || matches!(
                self.fd_table.read_endpoint(0),
                Some(FdReadEndpoint::Text(_) | FdReadEndpoint::ProcessSubstitution(_))
            )
        {
            if let Some(line) = self.read_virtual_fd_stdin(
                0,
                silent,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ) {
                return Some(line);
            }
        }

        // If FUNCTION_STDIN is set (from heredoc or redirect), only read from it.
        // Do NOT fall through to process stdin - that would block on the terminal.
        if self.shell_state.env_vars.contains_key(FUNCTION_STDIN) {
            return self.read_function_stdin(
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            );
        }

        if let Some(line) = self.read_virtual_fd_stdin(
            0,
            silent,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        ) {
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

        // niubash#684: the fd-0 probe above already routed an
        // InheritedProcessStdin endpoint through the inherited reader
        // (read_virtual_fd_stdin), and it delivered the descriptor's
        // EOF verdict. The mirror → inherited chain below would ask the
        // SAME descriptor a second time — re-blocking and consuming the
        // next input line as a record — where GNU reads one descriptor
        // once (read.def:294) and ends the builtin at its EOF
        // (read.def:767, :949).
        if self.read_process_stdin_eof {
            return None;
        }

        self.read_function_stdin(delimiter, char_limit, exact_char_limit, escape_aware, raw)
            .or_else(|| {
                self.read_inherited_process_stdin(
                    silent,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                )
            })
    }

    /// Transports for a descriptor that is a dup of the process stdin
    /// (`exec 8<&0` then `read -u 8` / `read <&8`): POSIX dup shares fd 0's
    /// open file description, so the copy is served exactly like an fd 0
    /// read — the stage mirror, the flag-armed inherited-stdin reader, and
    /// finally the raw byte-wise handle reader (read_stdin_until). The
    /// fd0-bound-early guard does not apply: the dup keeps its own reference
    /// to the process stdin object even after fd 0 is rebound (niubash#279).
    fn read_dup_of_stdin(
        &mut self,
        silent: bool,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        if let Some(line) =
            self.read_function_stdin(delimiter, char_limit, exact_char_limit, escape_aware, raw)
        {
            return Some(line);
        }
        if let Some(line) = self.read_inherited_process_stdin(
            silent,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        ) {
            return Some(line);
        }
        // niubash#684: the inherited transport already delivered fd 0's
        // EOF verdict — the raw fallback reader would only re-block on
        // the same descriptor and swallow the next input line as a
        // record GNU never assigns (read.def:767/:949 end the builtin
        // at the first EOF).
        if self.read_process_stdin_eof {
            return None;
        }
        match read_stdin_until(
            self.read_deadline,
            silent,
            delimiter,
            char_limit,
            exact_char_limit,
            self.read_sigint_guard_armed,
            escape_aware,
            raw,
        ) {
            // rubash#1250: an untrapped SIGINT aborted the raw read —
            // record it so the acquisition cascade stops here.
            Ok((_, _, _, _, true)) => {
                #[cfg(unix)]
                {
                    self.read_interrupted_signal = Some(libc::SIGINT);
                }
                #[cfg(not(unix))]
                {
                    self.read_interrupted_signal = Some(2);
                }
                None
            }
            // EOF with nothing consumed: no record to serve — the caller's
            // EOF handling (read.def:762-768) decides the status.
            Ok((0, _, false, false, false)) => None,
            Ok((_, line, stdin_eof, timed_out, _)) => {
                self.read_eof_no_delimiter |= stdin_eof;
                self.read_timed_out |= timed_out;
                Some(line)
            }
            Err(_) => None,
        }
    }

    pub(in crate::executor) fn read_coproc_stdout(
        &mut self,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
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
        // True only when the pipe itself reported EOF (empty read or a
        // read error): a deadline break or a SIGINT abort leaves the
        // descriptor open in GNU (read.def keeps the fd across builtin
        // calls), so it must not fall into the EOF close below
        // (rubash#1624).
        let mut hit_eof = false;
        // rubash#1300: escaped bytes never terminate the record.
        let mut escapes = crate::executor::fd_table::ReadEscapeTracker::new(escape_aware);
        // Post-escape counting state for the char limit (rubash#1328).
        let mut escaped = false;
        // read.def:731-738: a record that is just the delimiter is served
        // (empty line, GNU rc 0) — tracked so the empty check below cannot
        // conflate it with EOF (niubash#275).
        let mut delimiter_found = false;

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
                    hit_eof = true;
                    break;
                }
                Ok(buf) => {
                    bytes.push(buf[0]);
                    let was_escaped = escaped;
                    if escaped {
                        if buf[0] != b'\n' {
                            consumed_chars += 1;
                        }
                        escaped = false;
                    } else if !raw && buf[0] == b'\\' {
                        escaped = true;
                    } else {
                        consumed_chars += 1;
                    }
                    let suppressed = escapes.suppresses_match(buf[0]);
                    if !suppressed
                        && !was_escaped
                        && delimiter_byte_matches(buf[0], delimiter)
                        && !exact_char_limit
                    {
                        delimiter_found = true;
                        break;
                    }
                    if let Some(limit) = char_limit {
                        if consumed_chars >= limit && !escaped {
                            break;
                        }
                    }
                }
                Err(_) => {
                    ended = true;
                    hit_eof = true;
                    break;
                }
            }
        }

        // read.def:949 eof: the pipe drained (or errored) before the
        // delimiter arrived; a timeout break is not EOF.
        if ended && !self.read_timed_out && !bytes.is_empty() {
            self.read_eof_no_delimiter = if escape_aware {
                // Replay the escape state: an escaped delimiter byte is
                // record data, not a found delimiter (rubash#1300).
                let mut replay = crate::executor::fd_table::ReadEscapeTracker::new(true);
                !bytes.iter().any(|byte| {
                    let suppressed = replay.suppresses_match(*byte);
                    !suppressed && delimiter_byte_matches(*byte, delimiter)
                })
            } else {
                !bytes
                    .iter()
                    .any(|byte| delimiter_byte_matches(*byte, delimiter))
            };
        }
        if hit_eof
            && matches!(
                self.fd_table.read_endpoint(fd),
                Some(FdReadEndpoint::CoprocStdout { .. })
            )
        {
            // EOF closes this shell-owned read capability. Job reaping remains
            // separate so wait can still consume the child's final status.
            self.fd_table.close_input(fd);
        }
        // EOF before any byte: no record. A delimiter-only record (empty
        // line) IS a record — served as the empty string (niubash#275).
        if bytes.is_empty() && !delimiter_found {
            return None;
        }
        Some(trim_read_input(
            bytes_to_shell_text(&bytes),
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
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
        // The spelling judgment is the registry's (redundancy audit
        // R3/D6): the local strip+parse also accepted `/dev/fd/+3`,
        // which GNU rejects on every face (the literal open fails
        // ENOENT in procfs).
        let fd = crate::special_paths::fd_alias_target(filename)?;
        // rubash#2256 (wave 2): GNU source.def hands the word to
        // _evalfile, whose open() resolves the alias through the
        // kernel — a regular-file source therefore reads FRESH from
        // offset 0 (and a second `source /dev/fd/N` re-reads it),
        // while the snapshot serve below only models byte-channel
        // endpoints (it declines File endpoints outright, which sent
        // `source /dev/fd/6` with `6<file` to the literal filesystem
        // open and ENOENT). Ask the alias-layer resolver first.
        if let Some(path) = self.fd_alias_reopen_path(fd) {
            let bytes = std::fs::read(path).ok()?;
            return Some(bytes_to_shell_text(&bytes));
        }
        let (data, offset) = self.fd_table.input_snapshot_bytes(fd)?;
        let content = data.get(offset..)?.to_vec();
        Some(bytes_to_shell_text(&content))
    }

    pub(crate) fn process_substitution_output_bytes(&mut self, source: &str) -> Option<Vec<u8>> {
        self.process_substitution_output_bytes_timed(source)
            .map(|(bytes, _timing)| bytes)
    }

    /// rubash#1141: `process_substitution_output_bytes` plus the body's
    /// production schedule. GNU runs the body concurrently on a pipe
    /// (subst.c:6362 process_substitute), so a consumer's read deadline
    /// observes real arrival times; rubash materializes the bytes, so
    /// the schedule recorded while the body ran is the consumer's only
    /// timing evidence (timed_input.rs). Raw-byte units: fd-table
    /// endpoints serve bytes, not shell text.
    pub(crate) fn process_substitution_output_bytes_timed(
        &mut self,
        source: &str,
    ) -> Option<(Vec<u8>, Option<super::timed_input::SharedTimedInput>)> {
        let ast = super::command_substitution::parse_procsub_body(self, source);
        if ast.commands.is_empty() {
            return None;
        }

        // G-02: no process-cwd save/restore — the procsub subshell is a
        // clone with its own session cwd field.
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
        // rubash#1551: the body is a forked child (subst.c:6362
        // process_substitute) running in place here — bracket the
        // process-global hash-table mirror for its duration.
        let _hashed_fork =
            crate::executor::path::hashed_cache_fork_guard(&self.shell_state.env_vars);
        subshell.stdout_capture = Some(Vec::new());
        // Direct-stdout builtins inside the substitution consult the
        // thread-local capture, which belongs to an enclosing pipeline
        // stage when this substitution runs inside one; give the body its
        // own capture.
        let timing_recorder = crate::executor::shell_options::begin_capture_timing();
        let (thread_captured, result) =
            crate::executor::shell_options::capture_stdout(|| subshell.execute_ast(&ast));
        crate::executor::shell_options::end_capture_timing();
        let mut output = subshell.stdout_capture.take().unwrap_or_default();
        // rubash#1141: field-sink bytes precede thread-sink bytes.
        crate::executor::shell_options::capture_timing_set_prefix(&output);
        output.extend_from_slice(&thread_captured);
        let body_timing = std::mem::take(&mut *timing_recorder.borrow_mut())
            .finish(output.len(), false, output.len())
            .map(|timing| timing.shared());
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
        let pid = self.register_process_substitution_status(status);
        // GNU jobs.c: the input procsub's body finished before the outer
        // command even started, and the SIGCHLD handler reaps it at the
        // next command boundary — by the time `$!` is observable the child
        // is gone (rubash#1022: `cat <(echo hi); kill -0 $!` fails on GNU
        // too). Reap the placeholder now so `kill -0` reports ESRCH the
        // same way; `wait $!` still reports the recorded status. (The
        // output form keeps its placeholder live: with /dev/fd pipes the
        // child outlives the command.)
        self.reap_procsub_placeholder(pid, true);

        match result {
            Ok(())
            | Err(ExecuteError::ExitCode(_))
            | Err(ExecuteError::ExpansionFailure(_))
            | Err(ExecuteError::Return(_)) => Some((output, body_timing)),
            Err(_) => None,
        }
    }

    pub(in crate::executor) fn read_virtual_fd_stdin(
        &mut self,
        fd: u32,
        silent: bool,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        if self.fd_table.is_open_for_read(fd) {
            if let Some(line) = self.fd_table.read_text(
                fd,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ) {
                if let Some(offset) = self.fd_table.input_offset(fd) {
                    self.shell_state
                        .env_vars
                        .insert(fd_stdin_offset_key(fd), offset.to_string());
                }
                return Some(trim_read_input(
                    line,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
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
            return self.read_inherited_process_stdin(
                silent,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            );
        }
        None
    }

    pub(in crate::executor) fn read_function_stdin(
        &mut self,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
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
            // rubash#1141: EOF is itself a timed event — it "arrives"
            // when production ends, so an EOF read against a stream
            // whose production outlasts the deadline is a timeout, not
            // an EOF (GNU: the alarm fires while zread blocks).
            if let (Some(input), Some(timing)) = (
                self.shell_state.env_vars.get(FUNCTION_STDIN),
                self.timed_stdin.clone(),
            ) {
                let mut timing = timing.borrow_mut();
                if timing.matches_len(input.len())
                    && matches!(
                        timing.judge(None, offset, self.read_deadline),
                        super::timed_input::TimedVerdict::Timeout { .. }
                    )
                {
                    self.read_timed_out = true;
                }
            }
            return None;
        }
        if char_limit == Some(0) {
            return Some(String::new());
        }

        // One record scan over a window of the stream; re-run on the
        // in-time prefix when the timing judge cuts the record short.
        let scan_record = |slice: &str| -> Option<(String, usize, bool, bool)> {
            let mut output = String::new();
            let mut consumed = 0usize;
            let mut took_any = false;
            let mut delimiter_found = false;
            let mut limit_reached = false;
            // The mirror stores RAW stream bytes, so a carrier delimiter
            // (e.g. `-d $'\x1b'`) appears as its literal char here, while
            // the normalized needle is the marker pair — match both and
            // advance by the form that matched.
            let delimiter_needle = read_delimiter_needle(delimiter);
            let delimiter_literal_len = delimiter.len_utf8();
            // Count *decoded* units, not transport chars: a raw-byte marker
            // pair (E000 + E001..=E100) is ONE input byte and consumes ONE
            // unit of the -n/-N budget, and the pair is emitted whole so a
            // limit break can never land inside it (#714). GNU counts one
            // per character and stops at the limit: read.def:874
            // `i += read_mbchar(...)` consumes a multibyte character whole,
            // read.def:884 `nr++`, read.def:886
            // `if (nchars > 0 && nr >= nchars) break;`. ESC is such a
            // character here. This is the same pairing discipline as
            // `read_split::logical_units`, the buffered splitter's unit walk.
            let chars: Vec<(usize, char)> = slice.char_indices().collect();
            let mut cursor = 0usize;
            let mut units = 0usize;
            // GNU read.def escape state for the -n/-N budget: the budget
            // is spent on POST-ESCAPE characters (an escape pair counts
            // once, a backslash-newline continuation counts zero), and an
            // escaped delimiter is data, not a terminator (rubash#1328);
            // the escape introducer itself never matches either
            // (rubash#1300).
            let mut escaped = false;
            while cursor < chars.len() {
                let (index, ch) = chars[cursor];
                if !exact_char_limit && !escaped && !(escape_aware && ch == '\\') {
                    if slice[index..].starts_with(&delimiter_needle) {
                        consumed = index + delimiter_needle.len();
                        took_any = true;
                        delimiter_found = true;
                        break;
                    }
                    if slice[index..].starts_with(delimiter) {
                        consumed = index + delimiter_literal_len;
                        took_any = true;
                        delimiter_found = true;
                        break;
                    }
                }

                let pair = ch as u32 == RAW_BYTE_MARKER_ESCAPE
                    && chars.get(cursor + 1).is_some_and(|&(_, payload)| {
                        let code = payload as u32;
                        (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&code)
                    });
                if pair {
                    let (payload_index, payload) = chars[cursor + 1];
                    output.push(ch);
                    output.push(payload);
                    consumed = payload_index + payload.len_utf8();
                    cursor += 2;
                    // A marker pair is one raw byte: consumed as a
                    // whole, it is the escaped character when an escape
                    // was pending and never an escape introducer itself.
                    escaped = false;
                } else {
                    output.push(ch);
                    consumed = index + ch.len_utf8();
                    cursor += 1;
                }
                took_any = true;
                if escaped {
                    if ch != '\n' {
                        units += 1;
                    }
                    escaped = false;
                } else if !raw && ch == '\\' && !pair {
                    escaped = true;
                } else {
                    units += 1;
                }
                if char_limit.is_some_and(|limit| units >= limit) {
                    limit_reached = true;
                    break;
                }
            }
            if !took_any {
                return None;
            }
            Some((output, consumed, delimiter_found, limit_reached))
        };
        let input_len = self.shell_state.env_vars.get(FUNCTION_STDIN)?.len();
        let (mut output, mut consumed, delimiter_found, limit_reached) = {
            let input = self.shell_state.env_vars.get(FUNCTION_STDIN)?;
            scan_record(&input[offset..])?
        };

        // rubash#1141: judge the record against the stream's production
        // schedule. GNU's read blocks in zread until the record's bytes
        // arrive or the deadline fires (read.def), so a record completed
        // only by bytes that arrive after the deadline is a timeout:
        // serve just the in-time prefix and report through
        // read_timed_out (the caller still assigns the partial value,
        // matching read.def's timeout path to assign_vars).
        let mut timed_out = false;
        if let Some(timing) = self.timed_stdin.clone() {
            let mut timing = timing.borrow_mut();
            if timing.matches_len(input_len) {
                let record_end = if delimiter_found || limit_reached {
                    Some(offset + consumed)
                } else {
                    None
                };
                if let super::timed_input::TimedVerdict::Timeout { available_end } =
                    timing.judge(record_end, offset, self.read_deadline)
                {
                    timed_out = true;
                    self.read_timed_out = true;
                    let (p_output, p_consumed, _, _) = {
                        let input = self.shell_state.env_vars.get(FUNCTION_STDIN)?;
                        let mut end = available_end;
                        while end > offset && !input.is_char_boundary(end) {
                            end -= 1;
                        }
                        scan_record(&input[offset..end]).unwrap_or((String::new(), 0, false, false))
                    };
                    output = p_output;
                    consumed = p_consumed;
                }
            }
        }

        // read.def:949 eof: the buffered stdin ran out before the
        // delimiter (a -n/-N limit break is not EOF).
        self.read_eof_no_delimiter = !delimiter_found && !limit_reached;
        if timed_out {
            self.read_eof_no_delimiter = false;
        }
        self.shell_state.env_vars.insert(
            FUNCTION_STDIN_OFFSET.to_string(),
            (offset + consumed).to_string(),
        );
        Some(trim_read_input(
            output,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        ))
    }

    pub(in crate::executor) fn read_inherited_process_stdin(
        &mut self,
        silent: bool,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
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
        // rubash#1250: an interrupt already consumed for this `read`
        // ends every transport instantly (no re-blocking).
        if self.read_interrupted_signal.is_some() {
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
        // read.def:570-590: a -n/-d read from a tty runs in one-char mode —
        // a console in cooked line mode holds every keystroke until Enter —
        // while a plain newline-terminated read stays canonical and -s only
        // silences the echo (read.def:603-621).
        //
        // rubash#806: a deadline on a CONSOLE also takes the one-char path —
        // see read_helpers::read_stdin_until for the cooked-ahead analysis
        // (the first cooked ReadFile(1) swallowed the whole typed line into
        // conhost's pending read state, the raw-queue gate then timed out
        // with a one-byte record and the rest of the line orphaned into the
        // next reader).
        let onechar = if char_limit.is_some()
            || delimiter != '\n'
            || (self.read_deadline.is_some() && crate::fd::is_console_handle(stdin_handle))
        {
            crate::fd::enter_console_onechar_mode(stdin_handle)
        } else if silent {
            crate::fd::enter_console_noecho_mode(stdin_handle)
        } else {
            None
        };
        let manual_echo = onechar.is_some() && !silent;
        // niubash#684: canonical-console VEOF translation state for the
        // byte loop below (see read_helpers::stdin_veof_armed).
        let veof_armed = super::read_helpers::stdin_veof_armed(stdin_handle, onechar.is_some());
        let mut output = String::new();
        let mut decoder = StdinCharDecoder::new();
        let mut units = 0usize;
        // GNU read.def escape state for the -n/-N budget (see the counting
        // below): true when the previous unit was an unquoted backslash.
        let mut escaped = false;
        let mut eof = false;
        let mut delimiter_found = false;
        // rubash#1300: an escaped unit is never the delimiter.
        let mut escaped = false;
        loop {
            if !decoder.has_queued() {
                if self.read_wait_timed_out(stdin_handle) {
                    // rubash#1250: timeout breaks with the partial
                    // record; an interrupt abandons the read outright
                    // (no assignment, status 128+SIGINT upstream).
                    if self.read_interrupted_signal.is_some() {
                        return None;
                    }
                    break;
                }
                // rubash#1250: with no deadline, a console read blocks
                // inside read_some (canonical mode holds bytes until a
                // full line lands, and read_some retries EINTR), so a
                // Ctrl-C could never surface. While the SIGINT guard is
                // armed, gate the read on sliced readability instead —
                // poll() on a canonical tty reports readable only when
                // a complete line is buffered, so the record shape is
                // unchanged.
                if self.read_deadline.is_none() && self.read_sigint_guard_armed {
                    if crate::fd::is_console_handle(stdin_handle) {
                        loop {
                            if self.take_read_sigint_abort() {
                                return None;
                            }
                            if crate::fd::wait_readable(
                                stdin_handle,
                                std::time::Duration::from_millis(50),
                            ) != crate::fd::ReadWait::Timeout
                            {
                                break;
                            }
                        }
                    } else if self.take_read_sigint_abort() {
                        return None;
                    }
                }
                let buf = crate::fd::read_some(stdin_handle, 1).ok()?;
                if buf.is_empty() {
                    eof = true;
                    break;
                }
                // niubash#684: canonical-console VEOF — see
                // read_helpers::stdin_veof_armed. At an empty record the
                // 0x04 marker IS the EOF (and the verdict recording
                // below treats it exactly like the zero-length read);
                // mid-record it flushes the buffered prefix — already
                // delivered byte-wise — so it is dropped and the record
                // continues.
                if veof_armed && buf[0] == 0x04 {
                    if output.is_empty() {
                        eof = true;
                        break;
                    }
                    continue;
                }
                decoder.queue_byte(buf[0]);
            }
            let Some(mut unit) = decoder.next_unit() else {
                continue;
            };
            // shtty.c:115 tt_setonechar forces ICRNL for one-char tty reads —
            // see read_helpers::read_stdin_until. ConPTY delivers VK_RETURN
            // as a CR record in every mode; the translation keeps a deadline
            // line read completing on the typed CR. "Pipe/file reads keep
            // raw CRs" holds only at this gate: on Windows
            // `trim_read_input_literal`'s wave-2 pop still strips a record's
            // trailing CR afterwards (CRLF pipes are not byte-GNU end to
            // end) — see the trim layer's NOTE; cr-816 NIT-2.
            if onechar.is_some() && delimiter == '\n' {
                if matches!(unit, StdinUnit::Char('\r')) {
                    unit = StdinUnit::Char('\n');
                }
            }
            if manual_echo {
                echo_stdin_unit(&onechar, &unit);
            }
            if !exact_char_limit
                && !escaped
                && !(escape_aware && matches!(unit, StdinUnit::Char('\\')))
                && stdin_unit_is_delimiter(&unit, delimiter)
            {
                delimiter_found = true;
                break;
            }
            let unit_char = match &unit {
                StdinUnit::Char(ch) => Some(*ch),
                StdinUnit::RawByte { .. } => None,
            };
            match unit {
                StdinUnit::Char(ch) => output.push(ch),
                StdinUnit::RawByte { text } => output.push_str(&text),
            }
            // GNU builtins/read.def processes backslash escapes inline
            // while counting: an escape pair appends ONE character (the
            // -n/-N budget is spent on stored characters) and a
            // backslash-newline continuation appends none (rubash#1328).
            // -r disables escape processing entirely.
            if escaped {
                if unit_char != Some('\n') {
                    units += 1;
                }
                escaped = false;
            } else if !raw && unit_char == Some('\\') {
                escaped = true;
            } else {
                units += 1;
            }
            if char_limit.is_some_and(|limit| units >= limit) {
                break;
            }
        }
        if eof {
            decoder.flush(&mut output);
        }
        // read.def:731-738: a record that is just the delimiter (`printf
        // '\n' | read v` — GNU rc 0, v empty) is a SERVED record, not EOF.
        // Only EOF with nothing consumed yields None here (read.def:762-768
        // break with eof=1); conflating the two made the empty line fail the
        // builtin and swallow the following record (niubash#275).
        if output.is_empty() && !delimiter_found {
            // niubash#684: a genuine EOF (the raw read returned empty)
            // with nothing served is the builtin's verdict, recorded so
            // the caller's fallback cascade does not re-block on a
            // second transport — GNU reads ONE descriptor and zread's
            // EOF ends the builtin (read.def:767, :949). Timeout and
            // interrupt breaks reach here with `eof` clear and leave
            // the flag untouched; those verdicts have their own arms.
            if eof {
                self.read_process_stdin_eof = true;
            }
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
            escape_aware,
            raw,
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

/// Incremental UTF-8 decoding for the `read -e` editor pump
/// (rubash#2339): complete characters are drained from the pending
/// byte buffer, an incomplete trailing sequence stays buffered for
/// the next chunk, and an invalid byte becomes U+FFFD (the editing
/// surface is byte-tolerant in the REPL dispatcher's shape too).
#[cfg(unix)]
fn drain_decoded_chars(pending: &mut Vec<u8>) -> Vec<char> {
    let mut out = Vec::new();
    let mut consumed = 0;
    while consumed < pending.len() {
        match std::str::from_utf8(&pending[consumed..]) {
            Ok(text) => {
                out.extend(text.chars());
                consumed = pending.len();
            }
            Err(err) => {
                let valid = err.valid_up_to();
                if valid > 0 {
                    let text = std::str::from_utf8(&pending[consumed..consumed + valid])
                        .expect("valid_up_to prefix is valid UTF-8");
                    out.extend(text.chars());
                    consumed += valid;
                    continue;
                }
                match err.error_len() {
                    Some(bad) => {
                        out.push('\u{fffd}');
                        consumed += bad;
                    }
                    None => break,
                }
            }
        }
    }
    pending.drain(..consumed);
    out
}
