use super::types::FUNCTION_STDIN;
use super::*;
use crate::executor::glob::{pathname_expand_word, PathnameExpansion};

impl Executor {
    pub(in crate::executor) fn open_input_redirect(&self, target: &str) -> io::Result<File> {
        self.open_input_redirect_impl(target, true)
    }

    /// Redirect-open validity probe (GNU redir.c open_redir_file: the open
    /// itself never reads). A registered `<(cmd)` path must not be drained
    /// by a probe — the consumer (read/cat/child stdin) does that.
    pub(in crate::executor) fn probe_input_redirect(&self, target: &str) -> io::Result<File> {
        self.open_input_redirect_impl(target, false)
    }

    fn open_input_redirect_impl(&self, target: &str, consume: bool) -> io::Result<File> {
        if is_null_device(target) {
            return File::open(shell_path_to_windows(
                "/dev/null",
                &self.shell_state.env_vars,
            ));
        }
        // GNU redir.c resolves /dev/std*, /dev/fd/N, /proc/self/fd/N through
        // the OS fd-alias layer — a dup of fd N, not a filesystem path.
        // Windows has no /dev, so resolve against the virtual fd table
        // (niubash#118: `< /dev/stdin` used to open CONIN$ and block on the
        // console even when fd 0 was a pipeline).
        if let Some(fd) = dev_stdio_redirect_fd(target) {
            return self.open_fd_read_endpoint(fd, target);
        }
        // Q11 /proc P1 (docs/proc-vfs-plan.md hook B): synthetic /proc files
        // serve as a pre-filled pipe converted to a File; reads hit EOF at
        // content end, matching procfs static-snapshot semantics.
        if let Some(content) = crate::proc_vfs::proc_file_content(target) {
            use std::io::Write as _;
            let (reader, mut writer) = std::io::pipe()?;
            writer
                .write_all(&content)
                .map_err(|e| crate::posix_errors::path_error(target, e))?;
            drop(writer); // close write side: reader sees EOF after content
            #[cfg(windows)]
            {
                use std::os::windows::io::{FromRawHandle as _, IntoRawHandle as _};
                let handle = reader.into_raw_handle();
                return Ok(unsafe { File::from_raw_handle(handle) });
            }
            #[cfg(unix)]
            {
                use std::os::unix::io::{FromRawFd as _, IntoRawFd as _};
                let fd = reader.into_raw_fd();
                return Ok(unsafe { File::from_raw_fd(fd) });
            }
        }
        let win_path = shell_path_to_windows(target, &self.shell_state.env_vars);
        // A spawned child reading a `<(cmd)` carrier must get the stream's
        // REMAINING bytes — GNU hands it a dup of the shared pipe offset,
        // so the parent's next open sees what the child left
        // (subst.c:7143). Serve the remainder through a fresh temp and
        // advance the shared offset.
        if consume {
            if let Some(bytes) = self.procsub_stream_take(&win_path) {
                let path = self
                    .write_process_substitution_temp_bytes(&bytes)
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::Other, "failed to materialize input")
                    })?;
                return File::open(&path).map_err(|e| crate::posix_errors::path_error(target, e));
            }
        }
        File::open(win_path).map_err(|e| crate::posix_errors::path_error(target, e))
    }

    pub(in crate::executor) fn open_fd_read_endpoint(
        &self,
        fd: u32,
        target: &str,
    ) -> io::Result<File> {
        match self.fd_table.read_endpoint(fd) {
            // DuplicateHandle shares the kernel file object: the child sees
            // the fd at its current offset (POSIX open file description).
            Some(FdReadEndpoint::File(file_fd)) => crate::fd::duplicate_handle(file_fd.handle)
                .map(crate::fd::handle_to_file)
                .map_err(|e| crate::posix_errors::path_error(&file_fd.path.to_string_lossy(), e)),
            Some(FdReadEndpoint::Text(_)) | Some(FdReadEndpoint::ProcessSubstitution(_)) => {
                let bytes = self
                    .virtual_fd_stdin_remaining_bytes(fd)
                    .unwrap_or_default();
                // GNU hands the child a dup of the same open file
                // description: bytes the child reads move the shared
                // offset — drain the source so the next consumer sees
                // the remainder, not a replay (procsub.tests
                // count_lines → 1,0,0,0,0).
                self.fd_table.drain_input_to_eof(fd);
                let path = self
                    .write_process_substitution_temp_bytes(&bytes)
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::Other, "failed to materialize fd input")
                    })?;
                File::open(&path)
            }
            Some(FdReadEndpoint::InheritedProcessStdin) => {
                #[cfg(windows)]
                {
                    use std::os::windows::io::AsHandle;
                    let owned = std::io::stdin().as_handle().try_clone_to_owned()?;
                    return Ok(File::from(owned));
                }
                #[cfg(not(windows))]
                {
                    return File::open(target)
                        .map_err(|e| crate::posix_errors::path_error(target, e));
                }
            }
            _ => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{target}: Bad file descriptor"),
            )),
        }
    }

    /// GNU test.c: `-t fd` answers isatty(fd) — whether the descriptor's
    /// current target is a terminal. The builtin only receives env_vars,
    /// so mirror the fd table plus the command's own (not yet bound)
    /// redirects into __RUBASH_FD_TERMINAL_<fd> right before it runs.
    /// Rewritten fresh each call so a stale mark can never outlive the
    /// binding it described.
    pub(in crate::executor) fn sync_fd_terminal_marks(&mut self, cmd: Option<&CommandNode>) {
        self.shell_state
            .env_vars
            .retain(|key, _| !key.starts_with(FD_TERMINAL_PREFIX));
        let mut marks: BTreeMap<u32, bool> = BTreeMap::new();
        for (fd, entry) in &self.fd_table.entries {
            if entry.closed {
                continue;
            }
            let read_tty = match &entry.read {
                Some(FdReadEndpoint::File(file)) => crate::fd::is_console_handle(file.handle),
                Some(FdReadEndpoint::InheritedProcessStdin) => {
                    crate::fd::is_console_handle(crate::fd::process_std_handle(0))
                }
                _ => false,
            };
            let write_tty = match &entry.write {
                Some(FdWriteEndpoint::File(file)) => crate::fd::is_console_handle(file.handle),
                Some(FdWriteEndpoint::Stdout) => {
                    crate::fd::is_console_handle(crate::fd::process_std_handle(1))
                }
                Some(FdWriteEndpoint::Stderr) => {
                    crate::fd::is_console_handle(crate::fd::process_std_handle(2))
                }
                _ => false,
            };
            marks.insert(*fd, read_tty || write_tty);
        }
        if let Some(cmd) = cmd {
            // do_redirections would leave the dup'd target live while the
            // builtin runs; the virtual-stdin model never bound it, so
            // judge the redirect's effective endpoint directly.
            if let Some(redirect) = &cmd.redirect_in {
                let fd = redirect.fd.unwrap_or(0);
                let target = self.expand_redirect_target(redirect);
                if is_closed_redirect_target(&target) {
                    marks.insert(fd, false);
                } else if let Some(tty) = self.redirect_target_is_terminal(&target) {
                    marks.insert(fd, tty);
                }
            }
            for redirect in [
                &cmd.redirect_out,
                &cmd.append,
                &cmd.redirect_err,
                &cmd.redirect_err_append,
            ]
            .into_iter()
            .flatten()
            {
                let fd = redirect.fd.unwrap_or(1);
                let target = self.expand_redirect_target(redirect);
                if let Some(tty) = self.redirect_target_is_terminal(&target) {
                    marks.insert(fd, tty);
                }
            }
        }
        for (fd, tty) in marks {
            self.shell_state
                .env_vars
                .insert(fd_terminal_key(fd), if tty { "1" } else { "0" }.to_string());
        }
    }

    /// isatty() verdict for an expanded redirect target: `&N` and the
    /// /dev/fd/N aliases consult fd N's bound endpoint; a path is probed
    /// read-only (no create/truncate side effect) and judged by
    /// GetConsoleMode — true for CON, false for NUL and disk files.
    fn redirect_target_is_terminal(&self, target: &str) -> Option<bool> {
        if let Some(source_fd) = redirect_target_fd(target) {
            return Some(match self.fd_table.read_endpoint(source_fd) {
                Some(FdReadEndpoint::File(file)) => crate::fd::is_console_handle(file.handle),
                Some(FdReadEndpoint::InheritedProcessStdin) => {
                    crate::fd::is_console_handle(crate::fd::process_std_handle(0))
                }
                Some(_) => false,
                None => match self.fd_table.output_endpoint(source_fd) {
                    Some(FdWriteEndpoint::File(file)) => crate::fd::is_console_handle(file.handle),
                    Some(FdWriteEndpoint::Stdout) => {
                        crate::fd::is_console_handle(crate::fd::process_std_handle(1))
                    }
                    Some(FdWriteEndpoint::Stderr) => {
                        crate::fd::is_console_handle(crate::fd::process_std_handle(2))
                    }
                    _ => false,
                },
            });
        }
        let path = shell_path_to_windows(target, &self.shell_state.env_vars);
        File::open(&path).ok().map(|file| {
            #[cfg(windows)]
            let raw = {
                use std::os::windows::io::AsRawHandle;
                file.as_raw_handle() as crate::fd::HANDLE
            };
            #[cfg(unix)]
            let raw = {
                use std::os::unix::io::AsRawFd;
                file.as_raw_fd() as crate::fd::HANDLE
            };
            crate::fd::is_console_handle(raw)
        })
    }

    pub(in crate::executor) fn create_redirect_output(
        &self,
        target: &str,
        clobber: bool,
    ) -> io::Result<File> {
        if is_null_device(target) {
            return OpenOptions::new().write(true).open(shell_path_to_windows(
                "/dev/null",
                &self.shell_state.env_vars,
            ));
        }
        let target = self.redirect_output_path_target(target);
        let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
        if !clobber
            && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "noclobber")
        {
            // GNU wording (bash builtins/common.c): "<target>: cannot
            // overwrite existing file"; the redirect machinery prints the
            // payload after its script/line prefix.
            return OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| {
                    if e.kind() == io::ErrorKind::AlreadyExists {
                        io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            format!("{target}: cannot overwrite existing file"),
                        )
                    } else {
                        e
                    }
                });
        }
        File::create(path).map_err(|e| crate::posix_errors::path_error(&target, e))
    }

    pub(in crate::executor) fn redirect_output_path_target(&self, target: &str) -> String {
        if self.posix_mode_enabled() {
            return target.to_string();
        }

        match pathname_expand_word(target, &self.shell_state.env_vars) {
            PathnameExpansion::Matches(matches) if matches.len() == 1 => matches[0].clone(),
            _ => target.to_string(),
        }
    }

    pub(in crate::executor) fn open_output_fd_append(&self, target: &str) -> io::Result<File> {
        let fd = redirect_target_fd(target)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "bad file descriptor"))?;
        if self.fd_table.is_closed(fd) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "bad file descriptor",
            ));
        }
        let endpoint = self
            .fd_table
            .output_endpoint(fd)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "fd is not writable"))?;
        // Handle-backed endpoints duplicate the real kernel object, keeping
        // the shared file offset (POSIX open file description) instead of
        // reopening the path at offset 0.
        if let FdWriteEndpoint::File(file_fd) = endpoint {
            return crate::fd::duplicate_handle(file_fd.handle).map(crate::fd::handle_to_file);
        }
        let path = match endpoint {
            FdWriteEndpoint::ProcessSubstitution { path, .. } => path,
            FdWriteEndpoint::Stdout | FdWriteEndpoint::Stderr => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "stdio file descriptor",
                ));
            }
            FdWriteEndpoint::CoprocStdin { .. } => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "coprocess file descriptor",
                ));
            }
            FdWriteEndpoint::File(_) => unreachable!("handled above"),
        };
        if is_null_device(&path.to_string_lossy()) {
            return OpenOptions::new()
                .write(true)
                .append(true)
                .open(shell_path_to_windows(
                    "/dev/null",
                    &self.shell_state.env_vars,
                ));
        }
        OpenOptions::new().create(true).append(true).open(path)
    }

    pub(in crate::executor) fn write_output_fd_redirect(
        &mut self,
        target: &str,
        output: &[u8],
    ) -> Result<bool, ExecuteError> {
        let Some(fd) = redirect_target_fd(target) else {
            return Ok(false);
        };
        if self.fd_table.is_closed(fd) {
            return Ok(false);
        }
        let Some(endpoint) = self.fd_table.output_endpoint(fd) else {
            return Ok(false);
        };
        match endpoint {
            // Same dup2 open-file-description semantics as write_fd_endpoint:
            // a write to an fd bound to Stdout/Stderr follows the live
            // capture (pipe/command substitution), not the raw process stdio.
            FdWriteEndpoint::Stdout => {
                if stdout_capture_active() {
                    stdout_capture_write(output)?;
                } else if let Some(capture) = &mut self.stdout_capture {
                    capture.write_all(output)?;
                } else {
                    write_stdout_bytes(output)?;
                }
            }
            FdWriteEndpoint::Stderr => {
                if let Some(capture) = &mut self.stderr_capture {
                    capture.write_all(output)?;
                } else {
                    write_stderr_bytes(output)?;
                }
            }
            FdWriteEndpoint::CoprocStdin { fd, .. } => {
                crate::fd::write_all(fd.handle, output)?;
            }
            FdWriteEndpoint::File(file_fd) => {
                crate::fd::write_all(file_fd.handle, output)?;
            }
            FdWriteEndpoint::ProcessSubstitution { path, .. } => {
                let mut file = OpenOptions::new().create(true).append(true).open(path)?;
                file.write_all(output)?;
            }
        }
        Ok(true)
    }

    pub(in crate::executor) fn output_fd_redirects_to_stdout(&self, target: &str) -> bool {
        if redirect_target_fd(target).map_or(false, |fd| {
            matches!(
                self.fd_table.output_endpoint(fd),
                Some(FdWriteEndpoint::Stdout)
            )
        }) {
            return true;
        }
        false
    }

    pub(in crate::executor) fn output_fd_redirects_to_stderr(&self, target: &str) -> bool {
        if redirect_target_fd(target).map_or(false, |fd| {
            matches!(
                self.fd_table.output_endpoint(fd),
                Some(FdWriteEndpoint::Stderr)
            )
        }) {
            return true;
        }
        false
    }

    pub(in crate::executor) fn input_fd_redirects_to_process_stdin(&self, target: &str) -> bool {
        if redirect_target_fd(target).map_or(false, |fd| {
            matches!(
                self.fd_table
                    .entries
                    .get(&fd)
                    .and_then(|entry| entry.read.as_ref()),
                Some(FdReadEndpoint::InheritedProcessStdin)
            )
        }) {
            return true;
        }
        false
    }

    pub(in crate::executor) fn write_default_stdout(
        &mut self,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        // fd 1's bound endpoint decides the destination (dup2 snapshot
        // semantics): the Stdout arm inside write_fd_endpoint resolves to
        // the active capture or raw stdio, and a `1>&2` binding correctly
        // follows fd 2 instead.
        self.write_fd_endpoint(1, output)?;
        Ok(())
    }

    pub(in crate::executor) fn write_default_stderr(
        &mut self,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        // Same: fd 2 may be bound to Stdout by `2>&1`, so the capture
        // buffers are consulted inside the endpoint arms, not first.
        self.write_fd_endpoint(2, output)?;
        Ok(())
    }

    pub(in crate::executor) fn has_output_fd_target(&self, target: &str) -> bool {
        redirect_target_fd(target).is_some_and(|fd| {
            !self.fd_table.is_closed(fd) && self.fd_table.output_endpoint(fd).is_some()
        })
    }

    pub(in crate::executor) fn write_fd_endpoint(
        &mut self,
        fd: u32,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        if self.fd_table.is_closed(fd) {
            return Ok(());
        }
        let Some(endpoint) = self.fd_table.output_endpoint(fd) else {
            return Ok(());
        };
        match endpoint {
            // GNU dup2 (`2>&1`) copies the open file description: a write to
            // an fd bound to Stdout must land wherever fd 1 currently goes —
            // the active stdout capture/pipe — not the raw process stdout.
            FdWriteEndpoint::Stdout => {
                if stdout_capture_active() {
                    stdout_capture_write(output)?;
                } else if let Some(capture) = &mut self.stdout_capture {
                    capture.write_all(output)?;
                } else {
                    write_stdout_bytes(output)?;
                }
            }
            FdWriteEndpoint::Stderr => {
                if let Some(capture) = &mut self.stderr_capture {
                    capture.write_all(output)?;
                } else {
                    write_stderr_bytes(output)?;
                }
            }
            FdWriteEndpoint::CoprocStdin { fd, .. } => {
                crate::fd::write_all(fd.handle, output).map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "coprocess input is closed")
                })?;
            }
            FdWriteEndpoint::File(file_fd) => {
                crate::fd::write_all(file_fd.handle, output)?;
            }
            FdWriteEndpoint::ProcessSubstitution { path, .. } => {
                let mut file = OpenOptions::new().create(true).append(true).open(path)?;
                file.write_all(output)?;
            }
        }
        Ok(())
    }

    pub(in crate::executor) fn apply_simple_set_flags(&mut self, args: &[String]) -> bool {
        if args.is_empty() {
            return false;
        }

        for arg in args {
            let Some(prefix) = arg.chars().next().filter(|ch| matches!(ch, '-' | '+')) else {
                return false;
            };
            let flags = &arg[1..];
            if flags.is_empty()
                || flags
                    .chars()
                    .any(|flag| !self.is_supported_short_set_flag(flag))
            {
                return false;
            }

            let enabled = prefix == '-';
            for flag in flags.chars() {
                match (flag, enabled) {
                    ('e', true) => {
                        self.shell_state
                            .env_vars
                            .insert("__RUBASH_ERREXIT".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "errexit",
                            true,
                        );
                    }
                    ('e', false) => {
                        self.shell_state.env_vars.remove("__RUBASH_ERREXIT");
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "errexit",
                            false,
                        );
                    }
                    ('x', true) => {
                        self.shell_state
                            .env_vars
                            .insert("__RUBASH_XTRACE".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "xtrace",
                            true,
                        );
                    }
                    ('x', false) => {
                        self.shell_state.env_vars.remove("__RUBASH_XTRACE");
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "xtrace",
                            false,
                        );
                    }
                    ('u', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "nounset",
                            enabled,
                        );
                    }
                    ('C', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "noclobber",
                            enabled,
                        );
                    }
                    ('f', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "noglob",
                            enabled,
                        );
                    }
                    ('n', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "noexec",
                            enabled,
                        );
                    }
                    (flag, _) => {
                        if let Some(option) = short_set_flag_option(flag) {
                            crate::builtins::set::set_shell_option(
                                &mut self.shell_state.env_vars,
                                option,
                                enabled,
                            );
                        }
                    }
                }
            }
        }

        true
    }

    pub(in crate::executor) fn apply_set_positional_operands(&mut self, args: &[String]) -> bool {
        if args.is_empty() {
            return false;
        }

        let mut flag_updates = Vec::new();
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            if arg == "--" {
                self.apply_set_flag_updates(&flag_updates);
                self.shell_state.dollar_vars_changed_by_set = true;
                self.set_positional_params(args[index + 1..].to_vec());
                return true;
            }

            if arg == "-" {
                self.apply_set_flag_updates(&flag_updates);
                self.shell_state.env_vars.remove("__RUBASH_XTRACE");
                crate::builtins::set::set_shell_option(
                    &mut self.shell_state.env_vars,
                    "xtrace",
                    false,
                );
                if index + 1 < args.len() {
                    self.shell_state.dollar_vars_changed_by_set = true;
                    self.set_positional_params(args[index + 1..].to_vec());
                }
                return true;
            }

            let Some(prefix) = arg.chars().next().filter(|ch| matches!(ch, '-' | '+')) else {
                self.apply_set_flag_updates(&flag_updates);
                self.shell_state.dollar_vars_changed_by_set = true;
                self.set_positional_params(args[index..].to_vec());
                return true;
            };

            let flags = &arg[1..];
            if flags.is_empty() {
                self.apply_set_flag_updates(&flag_updates);
                self.shell_state.dollar_vars_changed_by_set = true;
                self.set_positional_params(args[index + 1..].to_vec());
                return true;
            }

            if flags == "o" {
                let Some(option_name) = args.get(index + 1) else {
                    return false;
                };
                if !crate::builtins::set::is_shell_option(option_name) {
                    return false;
                }
                // GNU flags.c change_flag (flags.c:227-235) refuses
                // "set +o restricted" in a restricted shell with FLAG_ERROR,
                // and set.def:493-497 turns that into sh_invalidoptname
                // ("invalid option name", EX_USAGE) without changing the
                // restriction. Bail to the full set parser so the refusal is
                // reported instead of being silently applied here.
                if option_name == "restricted"
                    && prefix == '+'
                    && crate::builtins::set::shell_option_enabled(
                        &self.shell_state.env_vars,
                        "restricted",
                    )
                {
                    return false;
                }
                let enabled = prefix == '-';
                crate::builtins::set::set_shell_option(
                    &mut self.shell_state.env_vars,
                    option_name,
                    enabled,
                );
                if option_name == "ignoreeof" {
                    // set.def:388-399 set_ignoreeof binds/unbinds IGNOREEOF —
                    // mirror the env write into the typed owner expansion
                    // reads first.
                    if enabled {
                        let _ = self
                            .shell_state
                            .variables
                            .set_scalar("IGNOREEOF", "10".to_string());
                    } else {
                        self.shell_state.variables.remove("IGNOREEOF");
                    }
                }
                if option_name == "posix" {
                    self.shell_state.env_vars.insert(
                        "__RUBASH_POSIX_MODE".to_string(),
                        if enabled { "1" } else { "0" }.to_string(),
                    );
                    // GNU general.c:98-128 posix_initialize: entering posix
                    // mode turns on expand_aliases (among other shopts) and
                    // saves the prior values; leaving restores the saved set
                    // or the noninteractive default (off).
                    if enabled {
                        let prior = self.alias_expansion_enabled();
                        self.shell_state.env_vars.insert(
                            "__RUBASH_POSIX_SAVED_EXPAND_ALIASES".to_string(),
                            if prior { "1" } else { "0" }.to_string(),
                        );
                        crate::builtins::shopt::set_option(
                            &mut self.shell_state.env_vars,
                            "expand_aliases",
                            true,
                        );
                    } else {
                        match self
                            .shell_state
                            .env_vars
                            .remove("__RUBASH_POSIX_SAVED_EXPAND_ALIASES")
                        {
                            Some(saved) => crate::builtins::shopt::set_option(
                                &mut self.shell_state.env_vars,
                                "expand_aliases",
                                saved == "1",
                            ),
                            // No saved state: noninteractive default is off
                            // (interactive_shell is 0 here).
                            None => crate::builtins::shopt::set_option(
                                &mut self.shell_state.env_vars,
                                "expand_aliases",
                                false,
                            ),
                        }
                    }
                }
                index += 2;
                continue;
            }

            if flags
                .chars()
                .any(|flag| !self.is_supported_short_set_flag(flag))
            {
                return false;
            }

            flag_updates.push((prefix, flags.to_string()));
            index += 1;
        }

        false
    }

    pub(in crate::executor) fn apply_set_flag_updates(&mut self, flag_updates: &[(char, String)]) {
        for (prefix, flags) in flag_updates {
            let enabled = *prefix == '-';
            for flag in flags.chars() {
                match (flag, enabled) {
                    ('e', true) => {
                        self.shell_state
                            .env_vars
                            .insert("__RUBASH_ERREXIT".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "errexit",
                            true,
                        );
                    }
                    ('e', false) => {
                        self.shell_state.env_vars.remove("__RUBASH_ERREXIT");
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "errexit",
                            false,
                        );
                    }
                    ('x', true) => {
                        self.shell_state
                            .env_vars
                            .insert("__RUBASH_XTRACE".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "xtrace",
                            true,
                        );
                    }
                    ('x', false) => {
                        self.shell_state.env_vars.remove("__RUBASH_XTRACE");
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "xtrace",
                            false,
                        );
                    }
                    ('u', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "nounset",
                            enabled,
                        );
                    }
                    ('C', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "noclobber",
                            enabled,
                        );
                    }
                    ('f', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.shell_state.env_vars,
                            "noglob",
                            enabled,
                        );
                    }
                    (flag, _) => {
                        if let Some(option) = short_set_flag_option(flag) {
                            crate::builtins::set::set_shell_option(
                                &mut self.shell_state.env_vars,
                                option,
                                enabled,
                            );
                        }
                    }
                }
            }
        }
    }

    pub(in crate::executor) fn is_supported_short_set_flag(&self, flag: char) -> bool {
        matches!(flag, 'e' | 'x' | 'u' | 'C' | 'f' | 'n') || short_set_flag_option(flag).is_some()
    }

    pub(in crate::executor) fn expand_case_word(&mut self, word: &str) -> String {
        let mut expanded = if let Some(value) =
            tilde_expand::expand_word_prefix(word, &self.shell_state.env_vars)
        {
            value
        } else {
            self.expand_word(word)
        };
        if expanded.contains("<(") || expanded.contains(">(") {
            expanded = self
                .materialize_assignment_process_substitutions(&expanded)
                .unwrap_or(expanded);
        }
        expanded
    }

    /// Fingerprint of a FUNCTION_STDIN buffer so a deferred command-
    /// substitution cursor write-back can verify it still applies to the
    /// buffer it was recorded against.
    pub(in crate::executor) fn function_stdin_fingerprint(text: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        hasher.finish()
    }

    /// Apply a deferred command-substitution stdin consumption: the comsub
    /// child ran on `&self` and could not write env back, so the next
    /// `&mut` consumer folds the child's cursor into FUNCTION_STDIN_OFFSET
    /// (GNU subst.c:7143 — the forked body shares fd 0).
    pub(in crate::executor) fn apply_comsub_stdin_writeback(&mut self) {
        let Some((offset, fingerprint)) = self.comsub_stdin_writeback.take() else {
            return;
        };
        let same_buffer = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN)
            .map(|text| Self::function_stdin_fingerprint(text) == fingerprint)
            .unwrap_or(false);
        if same_buffer {
            self.shell_state
                .env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), offset.to_string());
        }
    }

    /// GNU builtins/read.def: `read` and an external command in the same
    /// `{ ...; }` group share one fd 0 cursor — after `read -d '|'` stops at
    /// the delimiter, `cat` sees only the remainder. FUNCTION_STDIN keeps a
    /// FUNCTION_STDIN_OFFSET cursor for shell reads; feeding a child process
    /// must hand over only the unread tail.
    pub(in crate::executor) fn function_stdin_remaining(&self) -> Option<String> {
        let input = self.shell_state.env_vars.get(FUNCTION_STDIN)?;
        if input.is_empty() {
            return None;
        }
        let offset = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            .min(input.len());
        Some(input[offset..].to_string())
    }

    pub(in crate::executor) fn stdin_string_for_command_mut(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<String> {
        self.apply_comsub_stdin_writeback();
        // fd-0 stdin source: GNU redir.c do_redirections applies
        // redirections left to right, so the LAST fd-0 input redirect in
        // cmd.redirects decides — `<<`, `0<<`, `<<<`, `0<<<`, `< file` and
        // `<&N` all compete for fd 0 (`cat <<A 0<<B` reads B's body, `cat
        // <<A <file` reads the file). When no ordered redirect info exists
        // (synthesized commands), fall back to the legacy field checks.
        let last_fd0 = fd0_stdin_redirect_winner(cmd);
        let stdin_body_kind = match last_fd0 {
            Some(kind) => match kind {
                crate::parser::RedirectKind::HereDoc => Some(crate::parser::RedirectKind::HereDoc),
                crate::parser::RedirectKind::HereString => {
                    Some(crate::parser::RedirectKind::HereString)
                }
                // A later `<`/`<&`/`<>`/`<&-` outranks the stdin bodies.
                _ => return self.stdin_string_for_command(cmd),
            },
            None if cmd.heredoc.is_some()
                || cmd.here_string.is_some()
                || cmd.heredoc_redirects.iter().any(|r| r.fd == Some(0)) =>
            {
                None
            }
            None => None,
        };
        let wants_here_string = stdin_body_kind == Some(crate::parser::RedirectKind::HereString)
            || (stdin_body_kind.is_none()
                && cmd.heredoc.is_none()
                && !cmd
                    .heredoc_redirects
                    .iter()
                    .any(|r| !r.here_string && (r.fd.is_none() || r.fd == Some(0))));
        if !wants_here_string {
            if let Some(redirect) = cmd.heredoc_redirects.iter().rev().find(|redirect| {
                !redirect.here_string && (redirect.fd.is_none() || redirect.fd == Some(0))
            }) {
                if redirect.body_carrier.is_some() {
                    return Some(self.expand_heredoc_body_mut_from_carrier(&redirect.body_carrier));
                }
                if let Some(body) = redirect.body.clone() {
                    return Some(self.expand_heredoc_body_mut(&body));
                }
            }
            if cmd.heredoc_body.is_some() {
                return Some(self.expand_heredoc_body_mut_from_carrier(&cmd.heredoc_body));
            }
            if let Some(body) = cmd.heredoc.clone() {
                return Some(self.expand_heredoc_body_mut(&body));
            }
        }
        // Here-string content already had quote removal applied by the parser
        // (single quotes stripped); a literal " that lived inside them is now
        // bare data. Expand only substitutions with quotes-as-data semantics so
        // that literal survives (cat <<< 'double"quote' => double"quote).
        // Numbered `0<<<` carries its word in heredoc_redirects instead.
        if stdin_body_kind != Some(crate::parser::RedirectKind::HereDoc) && cmd.heredoc.is_none()
            || wants_here_string
        {
            if let Some(redirect) = cmd
                .heredoc_redirects
                .iter()
                .rev()
                .find(|redirect| redirect.here_string && redirect.fd == Some(0))
            {
                if redirect.body_carrier.is_some() {
                    let mut input =
                        self.expand_here_string_mut_from_carrier(&redirect.body_carrier);
                    input.push('\n');
                    return Some(input);
                }
                if let Some(body) = redirect.body.clone() {
                    if let Some(word) = body.strip_prefix('\u{1d}') {
                        let mut input = decode_ansi_c_quoted_word(word)
                            .unwrap_or_else(|| self.expand_word(word));
                        input.push('\n');
                        return Some(input);
                    }
                    return Some(self.expand_heredoc_body_mut(&body));
                }
            }
            if cmd.here_string_carrier.is_some() {
                let mut input = self.expand_here_string_mut_from_carrier(&cmd.here_string_carrier);
                input.push('\n');
                return Some(input);
            }
            if let Some(word) = cmd.here_string.clone() {
                let decoded = decode_ansi_c_quoted_word(&word);
                let mut input = decoded.unwrap_or_else(|| self.expand_here_string_mut(&word));
                input.push('\n');
                return Some(input);
            }
        }
        let function_stdin_is_source = self.function_stdin_is_command_source(cmd);
        let result = self.stdin_string_for_command(cmd);
        if result.is_some() && function_stdin_is_source {
            // The child drains the stream from the cursor onward; GNU's
            // shared fd 0 means a subsequent `read` sees EOF.
            let end = self
                .shell_state
                .env_vars
                .get(FUNCTION_STDIN)
                .map(|input| input.len())
                .unwrap_or(0);
            self.shell_state
                .env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), end.to_string());
        }
        result
    }

    /// True when the command's effective fd 0 resolves to the shared
    /// FUNCTION_STDIN buffer: no explicit `<` redirect wins and no virtual
    /// fd-0 endpoint shadows it (same predicate the drain above applies).
    pub(in crate::executor) fn function_stdin_is_command_source(&self, cmd: &CommandNode) -> bool {
        cmd.redirect_in.is_none()
            && self.virtual_fd_stdin_remaining(0).is_none()
            && self.function_stdin_remaining().is_some()
    }

    pub(in crate::executor) fn stdin_string_for_command(
        &self,
        cmd: &CommandNode,
    ) -> Option<String> {
        // Same fd-0 ordering as the mut variant (GNU redir.c
        // do_redirections): the last fd-0 input redirect in cmd.redirects
        // decides stdin; a `<`/`<&` after `<<`/`<<<` outranks the bodies.
        let last_fd0 = fd0_stdin_redirect_winner(cmd);
        match last_fd0.as_ref() {
            Some(crate::parser::RedirectKind::HereDoc) => {
                if let Some(redirect) = cmd.heredoc_redirects.iter().rev().find(|redirect| {
                    !redirect.here_string && (redirect.fd.is_none() || redirect.fd == Some(0))
                }) {
                    if redirect.body_carrier.is_some() {
                        return Some(
                            self.expand_heredoc_body_readback_from_carrier(
                                &redirect.body_carrier,
                                redirect.body.as_deref(),
                            )
                            .text_lossy(),
                        );
                    }
                    if let Some(body) = redirect.body.as_deref() {
                        return Some(self.expand_heredoc_body_readback(body).text_lossy());
                    }
                }
            }
            Some(crate::parser::RedirectKind::HereString) => {
                if let Some(redirect) = cmd
                    .heredoc_redirects
                    .iter()
                    .rev()
                    .find(|redirect| redirect.here_string && redirect.fd == Some(0))
                {
                    if redirect.body_carrier.is_some() {
                        let mut input = self
                            .expand_heredoc_body_readback_from_carrier(
                                &redirect.body_carrier,
                                redirect.body.as_deref(),
                            )
                            .text_lossy();
                        input.push('\n');
                        return Some(input);
                    }
                    if let Some(body) = redirect.body.as_deref() {
                        if let Some(word) = body.strip_prefix('\u{1d}') {
                            let mut input = decode_ansi_c_quoted_word(word)
                                .unwrap_or_else(|| self.expand_word(word));
                            input.push('\n');
                            return Some(input);
                        }
                        return Some(self.expand_heredoc_body_readback(body).text_lossy());
                    }
                }
                // Unnumbered `<<<` keeps its word in cmd.here_string; it is
                // handled at the bottom of this function.
                return cmd.here_string.as_ref().and_then(|word| {
                    if let Some(carrier) = &cmd.here_string_carrier {
                        if let crate::parser::StdinBody::Preexpanded(text) = carrier {
                            let mut input = text.clone();
                            input.push('\n');
                            return Some(input);
                        }
                    }
                    // Legacy fallback (should not be reached with typed carrier)
                    let mut input = if let Some(pre) = preexpanded_stdin_body(word) {
                        pre.to_string()
                    } else {
                        decode_ansi_c_quoted_word(word)
                            .unwrap_or_else(|| self.expand_embedded_parameters_for_heredoc(word))
                    };
                    input.push('\n');
                    Some(input)
                });
            }
            // A later `< file` / `<&N` / `<>` outranks stdin bodies.
            Some(_) => {}
            // No ordered redirect info (synthesized commands): legacy
            // body-first behavior.
            None => {
                if let Some(redirect) = cmd.heredoc_redirects.iter().rev().find(|redirect| {
                    !redirect.here_string && (redirect.fd.is_none() || redirect.fd == Some(0))
                }) {
                    if let Some(carrier) = &redirect.body_carrier {
                        if let crate::parser::StdinBody::Preexpanded(text) = carrier {
                            let mut input = text.clone();
                            input.push('\n');
                            return Some(input);
                        }
                    }
                    if let Some(body) = redirect.body.as_deref() {
                        if let Some(word) = body.strip_prefix('\u{1d}') {
                            let mut input = decode_ansi_c_quoted_word(word)
                                .unwrap_or_else(|| self.expand_word(word));
                            input.push('\n');
                            return Some(input);
                        }
                        return Some(self.expand_heredoc_body_readback(body).text_lossy());
                    }
                }
                if let Some(carrier) = &cmd.heredoc_body {
                    if let crate::parser::StdinBody::Preexpanded(text) = carrier {
                        let mut input = text.clone();
                        input.push('\n');
                        return Some(input);
                    }
                }
                if let Some(body) = &cmd.heredoc {
                    return Some(self.expand_heredoc_body_readback(body).text_lossy());
                }
            }
        }

        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) != 0 {
                return None;
            }
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target) {
                return None;
            }
            // `<&N` / `< /dev/fd/N` / `< /dev/std{N}` are fd aliases: GNU
            // dup2 makes fd 0 read from fd N's current target, i.e. the
            // shell's virtual stdin/pipe — never the host path `/dev/stdin`
            // maps to (niubash#118: `cat < /dev/stdin` must consume the
            // pipe input, not block opening CONIN$).
            if let Some(source_fd) = redirect_target_fd(&target) {
                if let Some(input) = self.virtual_fd_stdin_remaining(source_fd) {
                    // GNU dup2 hands the consumer the live stream: bytes
                    // it took are gone for the next reader (procsub.tests
                    // count_lines `wc -l < $1` five times → 1,0,0,0,0).
                    self.fd_table.drain_input_to_eof(source_fd);
                    return Some(input);
                }
                if source_fd == 0 {
                    if let Some(input) = self.shell_state.env_vars.get(FUNCTION_STDIN) {
                        return Some(input.clone());
                    }
                }
                // No virtual input on the fd: duplicate its real endpoint.
                // fd 0 inherited from the process dups the real stdin
                // handle — a terminal blocks, matching GNU; a null/closed
                // stdin yields EOF.
                if let Ok(mut file) = self.open_fd_read_endpoint(source_fd, &target) {
                    // Raw fd bytes are not necessarily UTF-8 (fd_redirects
                    // c_external_cat_reads_raw_bytes: `exec 3<bin; cat <&3`
                    // must stream 0xff through). Read bytes and re-encode to
                    // shell text; callers decode on the way out.
                    let mut buf = Vec::new();
                    let _ = file.read_to_end(&mut buf);
                    return Some(crate::executor::substitution_metadata::bytes_to_shell_text(
                        &buf,
                    ));
                }
                return None;
            }
            let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
            if redirect.append {
                let _ = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&path);
            }
            // Q11 /proc P1 (docs/proc-vfs-plan.md hook B): synthetic files
            // are served before the filesystem (read/cat/heredoc stdin path).
            if let Some(bytes) = crate::proc_vfs::proc_file_content(&target) {
                return Some(crate::executor::substitution_metadata::bytes_to_shell_text(
                    &bytes,
                ));
            }
            // GNU redir.c dup2's the descriptor — a character device has
            // no EOF, so slurping blocks forever on the console
            // (test.tests `t -t 0 < /dev/tty` hung via
            // function_call_stdin). Decline the text channel; the caller
            // binds the live fd for the command's duration instead.
            {
                #[cfg(unix)]
                use std::os::unix::io::AsRawFd;
                #[cfg(windows)]
                use std::os::windows::io::AsRawHandle;
                if let Ok(file) = File::open(&path) {
                    #[cfg(windows)]
                    let raw = file.as_raw_handle() as crate::fd::HANDLE;
                    #[cfg(unix)]
                    let raw = file.as_raw_fd() as crate::fd::HANDLE;
                    if crate::fd::is_char_device_handle(raw) {
                        return None;
                    }
                }
            }
            // A `<(cmd)` temp path is a pipe carrier, not a file: reopening
            // it must resume at the shared offset, not replay the contents
            // (subst.c:7143 command_substitute / redir.c dup semantics).
            if let Some(bytes) = self.procsub_stream_take(&path) {
                return Some(crate::executor::substitution_metadata::bytes_to_shell_text(
                    &bytes,
                ));
            }
            return fs::read_to_string(path).ok();
        }

        if let Some(input) = self.virtual_fd_stdin_remaining(0) {
            return Some(input);
        }

        // Check for FUNCTION_STDIN (set by pipeline execution for builtins)
        if let Some(input) = self.function_stdin_remaining() {
            return Some(input);
        }

        let word = cmd.here_string.as_ref()?;
        if let Some(carrier) = &cmd.here_string_carrier {
            if let crate::parser::StdinBody::Preexpanded(text) = carrier {
                let mut input = text.clone();
                input.push('\n');
                return Some(input);
            }
        }
        // Legacy fallback (should not be reached with typed carrier)
        let mut input = if let Some(pre) = preexpanded_stdin_body(word) {
            pre.to_string()
        } else {
            decode_ansi_c_quoted_word(word)
                .unwrap_or_else(|| self.expand_embedded_parameters_for_heredoc(word))
        };
        input.push('\n');
        Some(input)
    }
}

/// The fd-0 input redirect that wins under GNU left-to-right application
/// (redir.c do_redirections): the last cmd.redirects entry targeting fd 0
/// whose kind supplies stdin (`<`, `<&`, `<>`, `<&-`, `<<`, `<<<`).
/// Returns None when the command carries no ordered redirect info.
pub(in crate::executor) fn fd0_stdin_redirect_winner(
    cmd: &CommandNode,
) -> Option<crate::parser::RedirectKind> {
    cmd.redirects
        .iter()
        .rev()
        .find(|redirect| {
            redirect.fd.unwrap_or(0) == 0
                && matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::Input
                        | crate::parser::RedirectKind::DuplicateInput
                        | crate::parser::RedirectKind::ReadWrite
                        | crate::parser::RedirectKind::CloseInput
                        | crate::parser::RedirectKind::HereDoc
                        | crate::parser::RedirectKind::HereString
                )
        })
        .map(|redirect| redirect.kind.clone())
}

pub(crate) fn write_stdout_bytes(output: &[u8]) -> io::Result<()> {
    // Builtins that write directly to the process stdout (set -o, declare,
    // alias, ...) must still be captured by command substitution and
    // pipeline stage capture. Thread-local capture makes that visible to
    // every writer, not just the Executor-aware write_default_stdout path.
    if stdout_capture_active() {
        return stdout_capture_write(output);
    }

    #[cfg(windows)]
    {
        return trace_stdio_write("stdout", output.len(), || {
            windows_raw_stdio::write_stdout(output)
        });
    }

    #[cfg(not(windows))]
    {
        trace_stdio_write("stdout", output.len(), || {
            std::io::stdout().lock().write_all(output)
        })
    }
}

/// Global stdout write honoring the thread-local capture used by command
/// substitution and pipeline stages. Builtins that write through a plain
/// `std::io::Stdout` handle (set -o, declare -p, shopt) must route through
/// here so their output is captured too.
pub(crate) fn write_global_stdout(output: &[u8]) -> io::Result<()> {
    if stdout_capture_active() {
        return stdout_capture_write(output);
    }
    write_stdout_bytes(output)
}

/// A `std::io::Write` adapter over the global capture-aware stdout.
pub(crate) struct GlobalStdout;

impl std::io::Write for GlobalStdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        write_global_stdout(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

thread_local! {
    static STDOUT_CAPTURE: std::cell::RefCell<Option<Vec<u8>>> =
        const { std::cell::RefCell::new(None) };
}

pub(in crate::executor) fn stdout_capture_active() -> bool {
    STDOUT_CAPTURE.with(|capture| capture.borrow().is_some())
}

fn stdout_capture_write(output: &[u8]) -> io::Result<()> {
    STDOUT_CAPTURE.with(|capture| {
        if let Some(buffer) = capture.borrow_mut().as_mut() {
            buffer.write_all(output)?;
        }
        Ok(())
    })
}

/// Begins thread-local stdout capture, returning the previous capture buffer
/// (if any) so callers can nest captures and restore afterwards.
pub(in crate::executor) fn begin_stdout_capture() -> Option<Vec<u8>> {
    STDOUT_CAPTURE.with(|capture| {
        let previous = capture.borrow_mut().take();
        *capture.borrow_mut() = Some(Vec::new());
        previous
    })
}

/// Ends thread-local stdout capture and returns the captured bytes.
pub(in crate::executor) fn take_stdout_capture() -> Vec<u8> {
    STDOUT_CAPTURE.with(|capture| capture.borrow_mut().take().unwrap_or_default())
}

/// Restores a previously saved capture buffer (used after nested captures).
pub(in crate::executor) fn restore_stdout_capture(previous: Option<Vec<u8>>) {
    STDOUT_CAPTURE.with(|capture| {
        *capture.borrow_mut() = previous;
    });
}

/// Runs `body` under a fresh thread-local stdout capture and returns the
/// captured bytes together with the body's result.
///
/// Command substitutions, process substitutions, and pipeline stages run
/// bodies on executors whose `stdout_capture` field only intercepts the
/// `write_default_stdout` path. Builtins that write the process stdout
/// directly (`write_stdout_bytes` / `write_global_stdout`) consult the
/// thread-local capture — which, when the substitution runs inside a
/// pipeline stage, belongs to the enclosing stage, leaking substitution
/// output into the stage's pipe (GNU subst.c command_substitute forks and
/// captures the fd itself). Installing a fresh capture for the body keeps
/// every write in the substitution's own output; the outer buffer is
/// restored afterwards.
pub(in crate::executor) fn capture_stdout<R>(body: impl FnOnce() -> R) -> (Vec<u8>, R) {
    let saved = begin_stdout_capture();
    let result = body();
    let captured = take_stdout_capture();
    restore_stdout_capture(saved);
    (captured, result)
}

pub(crate) fn write_stderr_bytes(output: &[u8]) -> io::Result<()> {
    #[cfg(windows)]
    {
        return windows_raw_stdio::write_stderr(output);
    }

    #[cfg(not(windows))]
    {
        std::io::stderr().lock().write_all(output)
    }
}

fn trace_stdio_write(
    stream: &str,
    bytes: usize,
    write: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    if std::env::var_os("RUBASH_STDIO_TRACE").is_none() {
        return write();
    }

    let start = std::time::Instant::now();
    let result = write();
    if bytes >= 1024 {
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let _ = writeln!(
            std::io::stderr().lock(),
            "rubash_stdio_trace stream={stream} bytes={bytes} elapsed_ms={elapsed_ms:.1}"
        );
    }
    result
}

#[cfg(windows)]
mod windows_raw_stdio {
    use std::ffi::c_void;
    use std::io;
    use std::ptr;

    const CP_UTF8: u32 = 65001;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;
    const WRITE_CHUNK_SIZE: usize = 1024 * 1024;

    extern "system" {
        fn GetStdHandle(nStdHandle: u32) -> *mut c_void;
        fn SetConsoleOutputCP(wCodePageID: u32) -> i32;
        fn WriteFile(
            hFile: *mut c_void,
            lpBuffer: *const c_void,
            nNumberOfBytesToWrite: u32,
            lpNumberOfBytesWritten: *mut u32,
            lpOverlapped: *mut c_void,
        ) -> i32;
    }

    pub(super) fn write_stdout(output: &[u8]) -> io::Result<()> {
        write_handle(STD_OUTPUT_HANDLE, output)
    }

    pub(super) fn write_stderr(output: &[u8]) -> io::Result<()> {
        write_handle(STD_ERROR_HANDLE, output)
    }

    fn write_handle(std_handle: u32, mut output: &[u8]) -> io::Result<()> {
        if output.is_empty() {
            return Ok(());
        }

        let handle = unsafe { GetStdHandle(std_handle) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        unsafe {
            SetConsoleOutputCP(CP_UTF8);
        }

        while !output.is_empty() {
            let chunk_len = output.len().min(WRITE_CHUNK_SIZE);
            let mut written = 0u32;
            let ok = unsafe {
                WriteFile(
                    handle,
                    output.as_ptr().cast(),
                    chunk_len as u32,
                    &mut written,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write stdout bytes",
                ));
            }
            output = &output[written as usize..];
        }

        Ok(())
    }
}
