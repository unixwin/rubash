use super::*;

impl Executor {
    pub(in crate::executor) fn execute_printf(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        // GNU builtins/printf.def: `printf -v name[sub]` resolves the
        // operand's subscript under the ExpandedOnce argv rules — verbatim
        // with array_expand_once, one deferred expand_subscript_string pass
        // without it (SET_VFLAGS, builtins/common.h:277-289). A failed
        // indexed subscript is an expr.c evalerror: diagnostic already
        // printed, rest of the command list discarded, status 1.
        let mut words = cmd.words[1..].to_vec();
        let mut scan = 0usize;
        while scan < words.len() {
            let arg = &words[scan];
            if arg == "--" || !arg.starts_with('-') || arg == "-" {
                break;
            }
            if arg == "-v" {
                if let Some(operand) = words.get(scan + 1).cloned() {
                    // GNU printf.def:305 SET_VFLAGS reads W_ARRAYREF off the
                    // operand word (`list_optflags`): with array_expand_once
                    // it adds VA_ONEWORD, so `printf -v "A[]]"` keys on the
                    // LAST `]`. `words` is cmd.words[1..], so the operand
                    // sits at cmd index scan+2.
                    let oneword = self.word_is_arrayref(cmd, scan + 2);
                    match self.rewrite_operand_array_subscript_flags(&operand, oneword) {
                        Ok(rewritten) => words[scan + 1] = rewritten,
                        Err(()) => return Ok(1),
                    }
                }
                break;
            }
            if let Some(fused) = arg.strip_prefix("-v") {
                if !fused.is_empty() {
                    match self.rewrite_operand_array_subscript(fused) {
                        Ok(rewritten) => words[scan] = format!("-v{rewritten}"),
                        Err(()) => return Ok(1),
                    }
                    break;
                }
            }
            scan += 1;
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = crate::builtins::printf::execute_with_io_and_store(
            words.iter().map(String::as_str),
            &mut self.shell_state.env_vars,
            Some(&mut self.shell_state.variables),
            &mut stdout,
            &mut stderr,
        )?;
        // GNU printf.def emits conversion diagnostics through
        // builtin_error (unbuffered) while the converted output sits in
        // stdout's stdio buffer until the builtin returns: in a merged
        // 2>&1 stream the diagnostics come first (rubash#1134).
        self.write_buffered_builtin_output_stderr_first(cmd, &stdout, &stderr)?;
        // GNU builtins/printf.def printf_builtin returns
        // sh_chkwrite(format/operands success): a failed flush (a closed
        // stdout fd) folds the write failure into the builtin's status —
        // `exec >&-; printf "end\n"` reports
        // `printf: write error: Bad file descriptor` AND exits 1 (issue
        // #510; builtins/common.c:320-334). The routing already printed
        // the diagnostic and set the flag; merge it here so the dispatch
        // site's `self.exit_code = self.execute_printf(cmd)?` keeps the
        // failure instead of overwriting it with the nominal zero (the
        // same merge execute_pwd already applies).
        if self.take_builtin_write_failed() {
            return Ok(1);
        }
        Ok(status)
    }

    pub(in crate::executor) fn execute_exit(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<crate::builtins::exit::ExitAction, ExecuteError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        // builtins/exit.def:59-62: an interactive shell echoes "exit" (or
        // "logout" in a login shell) to stderr before parsing arguments;
        // CHECK_HELPOPT runs first, so `exit --help` stays silent. GNU's
        // condition is `interactive && sourcelevel == 0`: the echo is
        // suppressed while a sourced file runs (`sourcelevel > 0`), which is
        // why `bash --rcfile rc -i -c 'echo AFTER'` with `exit 3` in the rc
        // prints nothing while a reader-phase `exit 3` still echoes
        // (niubash#404). GNU's `interactive` C global is also 0 while the
        // startup files run (verified with gdb on WSL GNU 5.3.0: exit_builtin
        // in a --rcfile sees interactive=0, interactive_shell=1) and for an
        // `bash -i script` (shell.c:1715-1717 init_interactive_script ->
        // init_noninteractive zeroes interactive while interactive_shell
        // stays 1), so the echo is suppressed in those phases too
        // (rubash#297). `__RUBASH_IN_SOURCE` is the engine's sourcelevel.
        // The echo is likewise the interactive shell's OWN farewell: an
        // `exit` inside a `( )` subshell runs in the forked child and
        // prints nothing (pty-verified GNU 5.3.20: interactive `(exit 3)`
        // is silent, top-level `exit` echoes; rubash#1254) — the engine's
        // subshell_depth carries that distinction.
        if self.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1")
            && self.get_env("__RUBASH_INTERACTIVE_FLAG_OFF").is_none()
            && self.get_env("__RUBASH_IN_SOURCE").is_none()
            && self.shell_state.subshell_depth.get() == 0
            && cmd.words.get(1).map(String::as_str) != Some("--help")
        {
            let login = self.get_env("__RUBASH_LOGIN_SHELL").as_deref() == Some("1");
            writeln!(stderr, "{}", if login { "logout" } else { "exit" })?;
        }
        let args: Vec<&str> = cmd.words[1..].iter().map(String::as_str).collect();
        // GNU builtins/common.c:519 get_exitstat (POSIX interp 1602): a bare
        // `exit` inside a trap action exits with trap_saved_exit_value
        // (trap.c:367) — the $? saved BEFORE the trap ran; commands inside
        // the action cannot change it. The guard is the trap's recorded
        // function/source nesting level (trap.c:383): for ERR/RETURN the
        // exit must run at that level (inside a called function it is the
        // current $? instead — GNU probes 2026-10-12), while usize::MAX
        // marks the EXIT trap whose guard is open at every depth (see
        // run_exit_trap_for_status_with_output_redirects). Outside a trap
        // action it is the current $? (last_command_exit_value).
        // GNU builtins/common.c get_exitstat (bash 5.3): a leading `--` is
        // skipped before the status word, so `exit --` is argument-less for
        // this purpose — inside a trap action it exits with
        // trap_saved_exit_value exactly like a bare `exit`.
        let args_are_empty = args.is_empty() || (args.len() == 1 && args[0] == "--");
        let last_status = if args_are_empty {
            let trap_context = self.trap_saved_exit_value.is_some()
                && !self.debug_trap_running
                && (self.trap_saved_exit_context == usize::MAX
                    || (self.shell_state.function_depth + self.source_nest_depth
                        == self.trap_saved_exit_context));
            if trap_context {
                crate::builtins::exit::normalize_status(
                    self.trap_saved_exit_value.unwrap_or(self.exit_code),
                )
            } else {
                crate::builtins::exit::normalize_status(self.exit_code)
            }
        } else {
            self.exit_code
        };
        let action = crate::builtins::exit::execute_with_io(
            cmd.words[1..].iter().map(String::as_str),
            last_status,
            &mut stdout,
            &mut stderr,
        )?;
        self.write_buffered_builtin_output(cmd, &stdout, &stderr)?;
        Ok(action)
    }

    pub(in crate::executor) fn execute_logout(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        // GNU builtins/exit.def:79-90 (logout_builtin): only a login shell
        // may logout; anything else reports and continues.
        let login_shell = self
            .shell_state
            .env_vars
            .get("__RUBASH_LOGIN_SHELL")
            .map(String::as_str)
            == Some("1");
        if !login_shell {
            let mut stderr = Vec::new();
            let status =
                crate::builtins::logout::execute_with_io(&self.diagnostic_prefix(), &mut stderr)?;
            self.write_buffered_builtin_output(cmd, &[], &stderr)?;
            return Ok(status);
        }

        // GNU exit.def:147,156-166 (exit_or_logout -> bash_logout): a login
        // shell sources ~/.bash_logout once before exiting.
        if let Some(home) = self.shell_state.env_vars.get("HOME").cloned() {
            let logout_file = format!("{home}/.bash_logout");
            if !self.bash_logout_sourced
                && std::fs::metadata(shell_path_to_windows(
                    &logout_file,
                    &self.shell_state.env_vars,
                ))
                .is_ok()
            {
                self.bash_logout_sourced = true;
                let mut node = CommandNode::default();
                node.words = vec![".".to_string(), logout_file];
                self.execute_source_command(&node)?;
            }
        }

        // GNU exit.def:93-154 (exit_or_logout): jump_to_top_level(EXITBLTIN)
        // - the shell exits with the logout status (an optional numeric
        // argument overrides it), after the regular exit trap.
        let status = match cmd.words.get(1) {
            Some(word) => self.expand_word(word).trim().parse::<i32>().unwrap_or(0),
            None => self.exit_code,
        };
        let status = self.run_exit_trap_for_status(status)?;
        Err(ExecuteError::ExitCode(status))
    }

    pub(in crate::executor) fn try_execute_dirname_fast_path(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<i32> {
        let operands = simple_path_tool_operands(&cmd.words[1..])?;
        if operands.is_empty() {
            return None;
        }

        let mut stdout = Vec::new();
        for operand in operands {
            let path = self.expand_word(operand);
            stdout.extend_from_slice(dirname_value(&path).as_bytes());
            stdout.push(b'\n');
        }
        Some(self.write_path_tool_output(cmd, &stdout))
    }

    pub(in crate::executor) fn try_execute_basename_fast_path(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<i32> {
        let operands = simple_path_tool_operands(&cmd.words[1..])?;
        if operands.is_empty() || operands.len() > 2 {
            return None;
        }

        let name = self.expand_word(operands[0]);
        let mut value = basename_value(&name);
        if let Some(suffix) = operands.get(1) {
            let suffix = self.expand_word(suffix);
            value = strip_basename_suffix(&value, &suffix);
        }

        let mut stdout = Vec::new();
        stdout.extend_from_slice(value.as_bytes());
        stdout.push(b'\n');
        Some(self.write_path_tool_output(cmd, &stdout))
    }

    fn write_path_tool_output(&mut self, cmd: &CommandNode, stdout: &[u8]) -> i32 {
        if self
            .write_buffered_builtin_output(cmd, stdout, &[])
            .is_err()
        {
            return 1;
        }
        0
    }

    /// `uname` / `arch` engine builtins (rubash#154). Identity is strategic
    /// information: the engine answers these itself — persona-valued fields
    /// come from `crate::executor::identity` — instead of spawning whatever
    /// `uname.exe` happens to sit on PATH (which also removes the spawn
    /// cost, #130). Unlike the dirname/basename fast paths there is no
    /// external fallback: full option parsing is handled in
    /// `builtins/uname.rs`, so PATH never decides availability.
    pub(in crate::executor) fn execute_identity_tool(
        &mut self,
        cmd: &CommandNode,
        tool: &str,
    ) -> i32 {
        // cmd.words are already expanded by expand_command_words (see
        // external_inner.rs), same contract as the sleep fast path.
        let output = match tool {
            "arch" => crate::builtins::uname::execute_arch(&cmd.words[1..]),
            _ => crate::builtins::uname::execute_uname(&cmd.words[1..]),
        };
        if self
            .write_buffered_builtin_output(cmd, &output.stdout, &output.stderr)
            .is_err()
        {
            return 1;
        }
        output.status
    }

    /// `stty` console-mode builtin (niubash#282): same buffered-output
    /// routing as the identity tools, so redirections apply.
    pub(in crate::executor) fn execute_console_mode_tool(&mut self, cmd: &CommandNode) -> i32 {
        let output = crate::builtins::stty::execute_stty(&cmd.words[1..]);
        if self
            .write_buffered_builtin_output(cmd, &output.stdout, &output.stderr)
            .is_err()
        {
            return 1;
        }
        output.status
    }

    pub(in crate::executor) fn execute_cd(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<i32, ExecuteError> {
        // GNU execute_cmd.c:4400+ executes a builtin's redirections in the
        // current shell (do_redirections/undo_redirection around the builtin,
        // no fork), so builtins/cd.def:136-175 bindpwd() updates PWD/OLDPWD
        // in the live shell for every redirect shape (`cd dir > f`, `>>f`,
        // `2>f`, ...), exactly as for the unredirected form. Every branch
        // must therefore funnel through the one sync point below.
        let status = self.execute_cd_redirect_bound(cmd)?;
        self.sync_cd_variables();
        Ok(status)
    }

    fn execute_cd_redirect_bound(&mut self, cmd: &CommandNode) -> Result<i32, ExecuteError> {
        use std::io::Write as _;
        // GNU redir.c do_redirections applies EVERY redirect of the simple
        // command in the current shell before the builtin runs and undoes
        // them afterwards (execute_cmd.c execute_simple_command builtin
        // path — no fork). The old first-match-wins binding dropped every
        // redirect after the first: `cd nosuch >f 2>g` wrote the diagnostic
        // to real stderr and never created g, and `cd - >&2` created a junk
        // file named "&2". Both channels are resolved here (fd 1 from
        // redirect_out/append, fd 2 from redirect_err/redirect_err_append)
        // and fd-dup targets are dispatched after the builtin returns.
        let out_target = cmd
            .redirect_out
            .as_deref()
            .filter(|redirect| redirect.fd_var.is_none())
            .map(|redirect| self.expand_redirect_target(redirect));
        let append_target = cmd
            .append
            .as_deref()
            .filter(|redirect| redirect.fd_var.is_none())
            .map(|redirect| self.expand_redirect_target(redirect));
        let err_target = cmd
            .redirect_err
            .as_deref()
            .filter(|redirect| redirect.fd_var.is_none())
            .map(|redirect| self.expand_redirect_target(redirect));
        let err_append_target = cmd
            .redirect_err_append
            .as_deref()
            .filter(|redirect| redirect.fd_var.is_none())
            .map(|redirect| self.expand_redirect_target(redirect));

        // ---- fd 1 (stdout): `>`, `>>`, `>&N`, `>&word`, `>/dev/null` ----
        let mut stdout_file: Option<File> = None;
        let mut stdout_dup: Option<String> = None;
        let mut stdout_null = false;
        let stdout_target = out_target.clone().or(append_target.clone());
        if let Some(target) = stdout_target.as_deref() {
            if is_null_device(target) || is_closed_redirect_target(target) {
                stdout_null = true;
            } else if redirect_target_fd(target).is_some() {
                stdout_dup = Some(target.to_string());
            } else {
                let append_mode = append_target.is_some();
                let file = open_cd_redirect_file(
                    target.strip_prefix('&').unwrap_or(target),
                    append_mode,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                )?;
                stdout_file = Some(file);
            }
        }

        // ---- fd 2 (stderr): `2>`, `2>>`, `2>&N`, `2>/dev/null` ----
        // GNU redir.c:832-838: `>&word` with a non-numeric word on
        // redirector 1 is r_err_and_out — fd 2 also binds to word unless
        // an explicit fd 2 redirect came with the command.
        let mut stderr_file: Option<File> = None;
        let mut stderr_dup: Option<String> = None;
        let mut stderr_null = false;
        let stderr_target = err_target.clone().or(err_append_target.clone());
        let err_and_out_peer = stderr_target.is_none()
            && out_target
                .as_deref()
                .is_some_and(|target| target.starts_with('&'));
        if let Some(target) = stderr_target.clone() {
            let append_mode = err_append_target.is_some();
            if is_null_device(&target) || is_closed_redirect_target(&target) {
                stderr_null = true;
            } else if redirect_target_fd(&target).is_some() {
                stderr_dup = Some(target);
            } else {
                let file = open_cd_redirect_file(
                    &target,
                    append_mode,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                )?;
                stderr_file = Some(file);
            }
        } else if err_and_out_peer {
            if let Some(file) = stdout_file.as_ref() {
                stderr_file = Some(file.try_clone()?);
            }
        }

        let (status, stdout_buffer, stderr_buffer) = {
            let mut stdout_default: Vec<u8> = Vec::new();
            let mut stderr_default: Vec<u8> = Vec::new();
            let mut stdout_sink = match (&mut stdout_file, &stdout_dup, stdout_null) {
                (Some(file), _, _) => CdOutSink::File(file),
                (None, Some(_), _) => CdOutSink::Dup(Vec::new()),
                (None, None, true) => CdOutSink::Null,
                (None, None, false) => CdOutSink::Default(&mut stdout_default),
            };
            let mut stderr_sink = match (&mut stderr_file, &stderr_dup, stderr_null) {
                (Some(file), _, _) => CdErrSink::File(file),
                (None, Some(_), _) => CdErrSink::Dup(Vec::new()),
                (None, None, true) => CdErrSink::Null,
                (None, None, false) => CdErrSink::Default(&mut stderr_default),
            };
            let status = crate::builtins::cd::execute_with_io(
                cmd.words[1..].iter().map(String::as_str),
                &mut self.shell_state.env_vars,
                &mut self.shell_state.cwd,
                &mut stdout_sink,
                &mut stderr_sink,
            )?;
            let stdout_buffer = match stdout_sink {
                CdOutSink::Dup(buffer) => Some(buffer),
                _ => None,
            };
            let stderr_buffer = match stderr_sink {
                CdErrSink::Dup(buffer) => Some(buffer),
                _ => None,
            };
            // A channel with no command-level redirect (Default) still must
            // honour an ENCLOSING compound's binding — `( cd /no_such ) 2>&1`
            // / `if ...; then cd ...; fi 2>&1` route fd 2 through the group's
            // dup2. When the group's fd 2 is the LIVE fd-1 alias (`2>&1`),
            // the bytes belong at the same live destination fd 1 writes —
            // the pipeline stage capture, or the real stdout — exactly like
            // OutputTarget::Stdout in the ordered walk. Routing the frozen
            // dup2-generation snapshot instead sent them to the wrong capture
            // (rubash#699 R1.4). File/Stderr/other bindings go through the
            // same ordered machinery every other buffered builtin uses.
            if !stdout_default.is_empty() {
                if self.route_builtin_buffered_output(cmd, &stdout_default, &[])? {
                    // Routed through the command's ordered redirect state.
                } else {
                    // The walk declines only when the command carries no
                    // output redirect (external_redirects'
                    // write_external_captured_streams runs the same
                    // fallback). write_default_stdout resolves the fd
                    // table — Stdout, an `exec >file` File binding, a
                    // `2>&1` fd-1 alias, or Closed — exactly where the
                    // ordered walk would have put the bytes. The old
                    // endpoint==Stdout shortcut covered only that one
                    // shape and let an `exec >file` binding drop the
                    // output on the floor.
                    self.write_default_stdout(&stdout_default)?;
                }
            }
            if !stderr_default.is_empty() {
                if self.route_builtin_buffered_output(cmd, &[], &stderr_default)? {
                    // Routed through the command's ordered redirect state.
                } else {
                    // Same contract as the stdout arm above. Without the
                    // fallback an unredirected `cd a b` / `cd nosuch` lost
                    // its diagnostic entirely: the walk declines (no
                    // output redirect on the command), the old
                    // `let _ =` swallowed the false return, and the bytes
                    // never reached fd 2 (rubash#705 R7.4's branding probe
                    // found `cd` printing nothing at all, rc 2).
                    self.write_default_stderr(&stderr_default)?;
                }
            }
            (status, stdout_buffer, stderr_buffer)
        };

        // Dispatch fd-dup buffers. GNU dup2 shares the open file
        // description, so a dup of a channel this command rebound follows
        // the rebound destination: `>f 2>&1` puts stderr in f, and `2>g
        // >&2` puts stdout in g (redir.c do_redirections order).
        if let (Some(buffer), Some(target)) = (stdout_buffer, stdout_dup.as_deref()) {
            if redirect_target_fd(target) == Some(2) {
                if let Some(file) = stderr_file.as_mut() {
                    file.write_all(&buffer)?;
                } else if let Some(peer) = stderr_dup.as_deref() {
                    self.write_output_fd_redirect(peer, &buffer)?;
                } else {
                    self.write_output_fd_redirect(target, &buffer)?;
                }
            } else {
                self.write_output_fd_redirect(target, &buffer)?;
            }
        }
        if let (Some(buffer), Some(target)) = (stderr_buffer, stderr_dup.as_deref()) {
            if redirect_target_fd(target) == Some(1) {
                if let Some(file) = stdout_file.as_mut() {
                    file.write_all(&buffer)?;
                } else if let Some(peer) = stdout_dup.as_deref() {
                    self.write_output_fd_redirect(peer, &buffer)?;
                } else {
                    self.write_output_fd_redirect(target, &buffer)?;
                }
            } else {
                self.write_output_fd_redirect(target, &buffer)?;
            }
        }

        Ok(status)
    }

    pub(in crate::executor) fn sync_cd_variables(&mut self) {
        for name in ["PWD", "OLDPWD"] {
            let Some(value) = self.shell_state.env_vars.get(name).cloned() else {
                continue;
            };
            if let Some(variable) = self.shell_state.variables.get_mut(name) {
                variable.value = crate::shell::ShellValue::Scalar(value);
            }
        }
    }
}

/// One bound output channel for the `cd` builtin's composed redirection
/// (see `execute_cd_redirect_bound`). `Dup` buffers the writes of an
/// fd-dup target (`>&N` / `2>&N`) so they can be dispatched to the
/// channel's rebound destination after the builtin returns.
enum CdOutSink<'a> {
    Default(&'a mut Vec<u8>),
    Null,
    File(&'a mut File),
    Dup(Vec<u8>),
}

impl std::io::Write for CdOutSink<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            CdOutSink::Default(buffer) => {
                buffer.extend_from_slice(buf);
                Ok(buf.len())
            }
            CdOutSink::Null => Ok(buf.len()),
            CdOutSink::File(file) => file.write(buf),
            CdOutSink::Dup(buffer) => {
                buffer.extend_from_slice(buf);
                Ok(buf.len())
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            CdOutSink::File(file) => file.flush(),
            _ => Ok(()),
        }
    }
}

enum CdErrSink<'a> {
    Default(&'a mut Vec<u8>),
    Null,
    File(&'a mut File),
    Dup(Vec<u8>),
}

impl std::io::Write for CdErrSink<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            CdErrSink::Default(buffer) => {
                buffer.extend_from_slice(buf);
                Ok(buf.len())
            }
            CdErrSink::Null => Ok(buf.len()),
            CdErrSink::File(file) => file.write(buf),
            CdErrSink::Dup(buffer) => {
                buffer.extend_from_slice(buf);
                Ok(buf.len())
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            CdErrSink::File(file) => file.flush(),
            _ => Ok(()),
        }
    }
}

fn open_cd_redirect_file(
    target: &str,
    append: bool,
    env_vars: &std::collections::HashMap<String, String>,
    cwd: &std::path::Path,
) -> std::io::Result<File> {
    // G-02: relative targets anchor at the session cwd.
    let path = shell_path_to_windows(target, env_vars);
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    if append {
        open_redirect_append(&path)
    } else {
        File::create(path)
    }
}

fn simple_path_tool_operands(args: &[String]) -> Option<Vec<&str>> {
    let mut operands = Vec::new();
    let mut parse_options = true;
    for arg in args {
        if parse_options && arg == "--" {
            parse_options = false;
            continue;
        }
        if parse_options && arg.starts_with('-') && arg.len() > 1 {
            return None;
        }
        operands.push(arg.as_str());
    }
    Some(operands)
}

fn basename_value(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    // niubash#709: an empty operand is the empty basename in GNU (`basename ''`
    // prints a blank line), but an all-slash operand stays `/` here because
    // that is the spelling the external WinuxCmd basename reports too
    // (the `//` root divergence is shared, not a fast-path defect).
    if normalized.is_empty() {
        return String::new();
    }
    let trimmed = trim_trailing_slashes(&normalized);
    if trimmed.is_empty() {
        return "/".to_string();
    }
    trimmed
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(&trimmed)
        .to_string()
}

/// POSIX `dirname` (GNU lib/mdir_name + lib/dirname-lgpl.c dir_len): the final
/// component and any redundant slashes before it are stripped, `.` is returned
/// when nothing is left, and a leading `//` (exactly two slashes before a
/// non-slash, or a two-slash path) is the one prefix preserved. niubash#709:
/// the old `trim_trailing_slashes` collapsed both `//` and `''` to `/`.
/// Backslashes are normalized first, matching the other fast-path tools.
fn dirname_value(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let bytes = normalized.as_bytes();
    let len = bytes.len();
    // GNU dir_len: only a leading `//`-but-not-`///` prefix survives.
    let prefix_length = if len == 0 {
        0
    } else if bytes[0] == b'/' {
        if bytes.get(1) == Some(&b'/') && bytes.get(2) != Some(&b'/') {
            2
        } else {
            1
        }
    } else {
        0
    };
    // GNU last_component: skip leading slashes, then track the start of the
    // final non-slash run.
    let mut base = 0usize;
    while base < len && bytes[base] == b'/' {
        base += 1;
    }
    let mut saw_slash = false;
    let mut index = base;
    while index < len {
        if bytes[index] == b'/' {
            saw_slash = true;
        } else if saw_slash {
            base = index;
            saw_slash = false;
        }
        index += 1;
    }
    // Strip the basename and the redundant slashes before it, never eating
    // into the preserved prefix.
    let mut length = base;
    while prefix_length < length && bytes[length - 1] == b'/' {
        length -= 1;
    }
    if length == 0 {
        ".".to_string()
    } else {
        normalized[..length].to_string()
    }
}

fn trim_trailing_slashes(value: &str) -> String {
    let trimmed = value.trim_end_matches('/');
    if trimmed.is_empty() && value.contains('/') {
        String::new()
    } else {
        trimmed.to_string()
    }
}

fn strip_basename_suffix(name: &str, suffix: &str) -> String {
    if suffix.len() < name.len() && name.ends_with(suffix) {
        name[..name.len() - suffix.len()].to_string()
    } else {
        name.to_string()
    }
}
