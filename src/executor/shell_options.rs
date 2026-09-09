use super::types::FUNCTION_STDIN;
use super::*;
use crate::executor::glob::{pathname_expand_word, PathnameExpansion};

impl Executor {
    pub(in crate::executor) fn open_input_redirect(&self, target: &str) -> io::Result<File> {
        if is_null_device(target) {
            return File::open(shell_path_to_windows("/dev/null", &self.env_vars));
        }
        File::open(shell_path_to_windows(target, &self.env_vars))
            .map_err(|e| crate::posix_errors::path_error(target, e))
    }

    pub(in crate::executor) fn create_redirect_output(
        &self,
        target: &str,
        clobber: bool,
    ) -> io::Result<File> {
        if is_null_device(target) {
            return OpenOptions::new()
                .write(true)
                .open(shell_path_to_windows("/dev/null", &self.env_vars));
        }
        let target = self.redirect_output_path_target(target);
        let path = shell_path_to_windows(&target, &self.env_vars);
        if !clobber && crate::builtins::set::shell_option_enabled(&self.env_vars, "noclobber") {
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

        match pathname_expand_word(target, &self.env_vars) {
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
        let path = match endpoint {
            FdWriteEndpoint::File(path) => path,
            FdWriteEndpoint::ProcessSubstitution { path, .. } => path,
            FdWriteEndpoint::Stdout | FdWriteEndpoint::Stderr => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "stdio file descriptor",
                ));
            }
            FdWriteEndpoint::CoprocStdin(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "coprocess file descriptor",
                ));
            }
        };
        if is_null_device(&path.to_string_lossy()) {
            return OpenOptions::new()
                .write(true)
                .append(true)
                .open(shell_path_to_windows("/dev/null", &self.env_vars));
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
            FdWriteEndpoint::Stdout => write_stdout_bytes(output)?,
            FdWriteEndpoint::Stderr => write_stderr_bytes(output)?,
            FdWriteEndpoint::CoprocStdin(pid) => {
                let Some(writer) = self.coproc_stdin_writers.get_mut(&pid) else {
                    return Ok(false);
                };
                writer.write_all(output)?;
            }
            FdWriteEndpoint::File(path) => {
                let mut file = OpenOptions::new().create(true).append(true).open(path)?;
                file.write_all(output)?;
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
        // Thread-local capture (pipeline stages for builtins that write to
        // the process stdout) wins over the Executor field capture.
        if stdout_capture_active() {
            stdout_capture_write(output)?;
            return Ok(());
        }
        if let Some(capture) = &mut self.stdout_capture {
            capture.write_all(output)?;
            return Ok(());
        }

        self.write_fd_endpoint(1, output)?;
        Ok(())
    }

    pub(in crate::executor) fn write_default_stderr(
        &mut self,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        if let Some(capture) = &mut self.stderr_capture {
            capture.write_all(output)?;
            return Ok(());
        }
        self.write_fd_endpoint(2, output)?;
        Ok(())
    }

    pub(in crate::executor) fn has_output_fd_target(&self, target: &str) -> bool {
        redirect_target_fd(target).is_some_and(|fd| {
            !self.fd_table.is_closed(fd) && self.fd_table.output_endpoint(fd).is_some()
        })
    }

    fn write_fd_endpoint(&mut self, fd: u32, output: &[u8]) -> Result<(), ExecuteError> {
        if self.fd_table.is_closed(fd) {
            return Ok(());
        }
        let Some(endpoint) = self.fd_table.output_endpoint(fd) else {
            return Ok(());
        };
        match endpoint {
            FdWriteEndpoint::Stdout => write_stdout_bytes(output)?,
            FdWriteEndpoint::Stderr => write_stderr_bytes(output)?,
            FdWriteEndpoint::CoprocStdin(pid) => {
                let Some(writer) = self.coproc_stdin_writers.get_mut(&pid) else {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "coprocess input is closed",
                    )
                    .into());
                };
                writer.write_all(output)?;
            }
            FdWriteEndpoint::File(path) => {
                let mut file = OpenOptions::new().create(true).append(true).open(path)?;
                file.write_all(output)?;
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
                        self.env_vars
                            .insert("__RUBASH_ERREXIT".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "errexit", true);
                    }
                    ('e', false) => {
                        self.env_vars.remove("__RUBASH_ERREXIT");
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "errexit",
                            false,
                        );
                    }
                    ('x', true) => {
                        self.env_vars
                            .insert("__RUBASH_XTRACE".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "xtrace", true);
                    }
                    ('x', false) => {
                        self.env_vars.remove("__RUBASH_XTRACE");
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "xtrace", false);
                    }
                    ('u', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "nounset",
                            enabled,
                        );
                    }
                    ('C', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "noclobber",
                            enabled,
                        );
                    }
                    ('f', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "noglob",
                            enabled,
                        );
                    }
                    ('n', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "noexec",
                            enabled,
                        );
                    }
                    (flag, _) => {
                        if let Some(option) = short_set_flag_option(flag) {
                            crate::builtins::set::set_shell_option(
                                &mut self.env_vars,
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
                self.dollar_vars_changed_by_set = true;
                self.set_positional_params(args[index + 1..].to_vec());
                return true;
            }

            if arg == "-" {
                self.apply_set_flag_updates(&flag_updates);
                self.env_vars.remove("__RUBASH_XTRACE");
                crate::builtins::set::set_shell_option(&mut self.env_vars, "xtrace", false);
                if index + 1 < args.len() {
                    self.dollar_vars_changed_by_set = true;
                self.set_positional_params(args[index + 1..].to_vec());
                }
                return true;
            }

            let Some(prefix) = arg.chars().next().filter(|ch| matches!(ch, '-' | '+')) else {
                self.apply_set_flag_updates(&flag_updates);
                self.dollar_vars_changed_by_set = true;
                self.set_positional_params(args[index..].to_vec());
                return true;
            };

            let flags = &arg[1..];
            if flags.is_empty() {
                self.apply_set_flag_updates(&flag_updates);
                self.dollar_vars_changed_by_set = true;
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
                        &self.env_vars,
                        "restricted",
                    )
                {
                    return false;
                }
                let enabled = prefix == '-';
                crate::builtins::set::set_shell_option(&mut self.env_vars, option_name, enabled);
                if option_name == "posix" {
                    self.env_vars.insert(
                        "__RUBASH_POSIX_MODE".to_string(),
                        if enabled { "1" } else { "0" }.to_string(),
                    );
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
                        self.env_vars
                            .insert("__RUBASH_ERREXIT".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "errexit", true);
                    }
                    ('e', false) => {
                        self.env_vars.remove("__RUBASH_ERREXIT");
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "errexit",
                            false,
                        );
                    }
                    ('x', true) => {
                        self.env_vars
                            .insert("__RUBASH_XTRACE".to_string(), "1".to_string());
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "xtrace", true);
                    }
                    ('x', false) => {
                        self.env_vars.remove("__RUBASH_XTRACE");
                        crate::builtins::set::set_shell_option(&mut self.env_vars, "xtrace", false);
                    }
                    ('u', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "nounset",
                            enabled,
                        );
                    }
                    ('C', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "noclobber",
                            enabled,
                        );
                    }
                    ('f', _) => {
                        crate::builtins::set::set_shell_option(
                            &mut self.env_vars,
                            "noglob",
                            enabled,
                        );
                    }
                    (flag, _) => {
                        if let Some(option) = short_set_flag_option(flag) {
                            crate::builtins::set::set_shell_option(
                                &mut self.env_vars,
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
        let mut expanded =
            if let Some(value) = tilde_expand::expand_word_prefix(word, &self.env_vars) {
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

    pub(in crate::executor) fn stdin_string_for_command_mut(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<String> {
        if let Some(body) = cmd.heredoc.clone() {
            return Some(self.expand_heredoc_body_mut(&body));
        }
        // Here-string content already had quote removal applied by the parser
        // (single quotes stripped); a literal " that lived inside them is now
        // bare data. Expand only substitutions with quotes-as-data semantics so
        // that literal survives (cat <<< 'double"quote' => double"quote).
        if let Some(word) = cmd.here_string.clone() {
            let decoded = decode_ansi_c_quoted_word(&word);
            let mut input = decoded.unwrap_or_else(|| self.expand_here_string_mut(&word));
            input.push('\n');
            return Some(input);
        }
        self.stdin_string_for_command(cmd)
    }

    pub(in crate::executor) fn stdin_string_for_command(
        &self,
        cmd: &CommandNode,
    ) -> Option<String> {
        if let Some(body) = &cmd.heredoc {
            return Some(self.expand_heredoc_body_readback(body).text_lossy());
        }

        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) != 0 {
                return None;
            }
            let target = self.expand_word(&redirect.target);
            if is_closed_redirect_target(&target) {
                return None;
            }
            let path = shell_path_to_windows(&target, &self.env_vars);
            if redirect.append {
                let _ = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&path);
            }
            return fs::read_to_string(path).ok();
        }

        if let Some(input) = self.virtual_fd_stdin_remaining(0) {
            return Some(input);
        }

        // Check for FUNCTION_STDIN (set by pipeline execution for builtins)
        if let Some(input) = self.env_vars.get(FUNCTION_STDIN) {
            if !input.is_empty() {
                return Some(input.clone());
            }
        }

        let word = cmd.here_string.as_ref()?;
        let mut input = decode_ansi_c_quoted_word(word)
            .unwrap_or_else(|| self.expand_embedded_parameters_for_heredoc(word));
        input.push('\n');
        Some(input)
    }
}

fn write_stdout_bytes(output: &[u8]) -> io::Result<()> {
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

fn write_stderr_bytes(output: &[u8]) -> io::Result<()> {
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
