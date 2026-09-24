use super::*;

/// GNU background/coproc children are forks (execute_cmd.c
/// execute_in_subshell / make_child): the subshell inherits every variable
/// attribute (array/assoc/readonly/integer/nameref/trace/case transforms,
/// declared-unset), the shell option state (set flags + shopt), the hash
/// table, the directory stack, ignored-signal dispositions, ulimits, umask
/// and the SECONDS epoch. Spawned `rubash -c` children re-import their
/// process environment as `env_vars`, so forwarding these `__RUBASH_*`
/// state keys restores exactly that fork inheritance. Keys that are
/// per-process runtime state (FD tables, coproc pipes, command-substitution
/// and heredoc payloads, trap table entries, line/pid bookkeeping,
/// evaluation-context flags, test-harness sentinels) stay dropped.
fn rubash_spawn_inherited_state(key: &str) -> bool {
    const KEYS: &[&str] = &[
        "__RUBASH_ARRAY_VARS",
        "__RUBASH_ASSOC_VARS",
        "__RUBASH_ASSOC_128_VARS",
        "__RUBASH_DECLARED_UNSET_VARS",
        "__RUBASH_EXPORTED_VARS",
        "__RUBASH_EXPORTED_FUNCTIONS",
        "__RUBASH_READONLY_VARS",
        "__RUBASH_READONLY_FUNCTIONS",
        "__RUBASH_FUNC_TRACE_FUNCTIONS",
        "__RUBASH_TRACE_VARS",
        "__RUBASH_INTEGER_VARS",
        "__RUBASH_UPPERCASE_VARS",
        "__RUBASH_LOWERCASE_VARS",
        "__RUBASH_CAPCASE_VARS",
        "__RUBASH_NAMEREF_VARS",
        "__RUBASH_HASH_TABLE",
        "__RUBASH_DIR_STACK",
        "__RUBASH_DISABLED_BUILTINS",
        "__RUBASH_GETOPTS_OFFSET",
        "__RUBASH_ULIMIT_C",
        "__RUBASH_ULIMIT_F",
        "__RUBASH_ULIMIT_N",
        "__RUBASH_UMASK",
        "__RUBASH_SECONDS_OFFSET",
        "__RUBASH_SHELL_START_EPOCH",
        "__RUBASH_TRAP_ORIG_IGN",
        "__RUBASH_POSIX_MODE",
        "__RUBASH_ZSH_OPTIONS",
        "__RUBASH_ERREXIT",
        "__RUBASH_XTRACE",
        "__RUBASH_PHYSICAL_PWD",
        "__RUBASH_SHOPT_STATE",
        "__RUBASH_SHOPT_CHECKHASH",
        "__RUBASH_COMPATIBLE_SHELL_PATH",
        "__RUBASH_SHELL_ROOT",
        "__RUBASH_TEMP_PATH",
        "__RUBASH_NO_UPSTREAM_SCRIPTS",
    ];
    key.starts_with("__RUBASH_SETOPT_") || KEYS.contains(&key)
}

enum CoprocStderrForwardTarget {
    Stdout,
    Stderr,
    File(PathBuf),
    Discard,
    CoprocStdin(std::fs::File),
}

/// Resolved stdio disposition for one of a background child's fds 0-2,
/// tracking what GNU's do_redirections would leave the descriptor
/// pointing at inside the forked subshell.
enum BackgroundStdio {
    /// fd keeps the parent's handle (GNU: the subshell inherited it).
    Inherit,
    /// fd is /dev/null (async stdin default, `>&-`, `<&-`, `>/dev/null`).
    Null,
    /// fd is an open file.
    File(File),
}

impl Clone for BackgroundStdio {
    fn clone(&self) -> Self {
        match self {
            Self::Inherit => Self::Inherit,
            Self::Null => Self::Null,
            Self::File(file) => file.try_clone().map(Self::File).unwrap_or(Self::Inherit),
        }
    }
}





/// Drop redirections already consumed as spawn-time stdio from the command
/// that gets serialized into the background child's `-c` source — GNU's
/// dispose_redirects (execute_cmd.c:1762) after do_redirections in the async
/// subshell. `consumed[i]` corresponds to `command.redirects[i]`; redirects
/// the spawn-time pass could not resolve stay for the child to apply.
fn strip_consumed_redirects(command: &mut CommandNode, consumed: &[bool]) {
    if !consumed.iter().any(|consumed| *consumed) {
        return;
    }
    // Capture the consumed heredoc/here-string descriptors before retain
    // mutates the list: (fd, here_string) pairs in redirect order, matching
    // the ordinal correlation materialize_async_stdin_body uses against
    // `heredoc_redirects`.
    let mut consumed_bodies: Vec<(Option<u32>, bool)> = Vec::new();
    for (index, redirect) in command.redirects.iter().enumerate() {
        let kind = match redirect.kind {
            crate::parser::RedirectKind::HereDoc => Some(false),
            crate::parser::RedirectKind::HereString => Some(true),
            _ => None,
        };
        if let Some(here_string) = kind {
            if consumed.get(index).copied().unwrap_or(false) {
                consumed_bodies.push((redirect.fd, here_string));
            }
        }
    }
    let mut index = 0;
    command.redirects.retain(|_| {
        let keep = !consumed[index];
        index += 1;
        keep
    });
    // The dedicated Option fields mirror entries of `redirects` for the
    // serializer (redirect_err_append may even hold a `2>>file` synthesized
    // from a `2>&1` dup); clear every field whose redirect is no longer
    // carried by the remaining list so it is not re-applied in the child.
    let remaining = &command.redirects;
    for field in [
        &mut command.redirect_in,
        &mut command.redirect_out,
        &mut command.append,
        &mut command.redirect_err,
        &mut command.redirect_err_append,
    ] {
        if field
            .as_ref()
            .is_some_and(|redirect| !remaining.contains(redirect))
        {
            *field = None;
        }
    }
    // Materialized stdin bodies must not resurface in the child: drop the
    // consumed heredoc_redirects rows and clear the fd-0 legacy fields when
    // no surviving redirect still feeds them — otherwise `here_string`
    // would be re-serialized as `<<<` and double the child's input.
    if !consumed_bodies.is_empty() {
        let mut pending = consumed_bodies;
        command.heredoc_redirects.retain(|entry| {
            let key = (entry.fd, entry.here_string);
            if let Some(position) = pending.iter().position(|k| *k == key) {
                pending.remove(position);
                false
            } else {
                true
            }
        });
        if !remaining.iter().any(|redirect| {
            redirect.fd.is_none()
                && matches!(redirect.kind, crate::parser::RedirectKind::HereDoc)
        }) {
            command.heredoc = None;
            command.heredoc_body = None;
            command.heredoc_delimiter = None;
        }
        if !remaining.iter().any(|redirect| {
            redirect.fd.is_none()
                && matches!(redirect.kind, crate::parser::RedirectKind::HereString)
        }) {
            command.here_string = None;
            command.here_string_carrier = None;
        }
    }
}

fn forward_coproc_stderr(
    mut stderr: std::process::ChildStderr,
    mut target: CoprocStderrForwardTarget,
) -> Result<(), std::io::Error> {
    let mut buffer = [0_u8; 8192];
    loop {
        let count = stderr.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        match &mut target {
            CoprocStderrForwardTarget::Stdout => {
                std::io::stdout().write_all(&buffer[..count])?;
                std::io::stdout().flush()?;
            }
            CoprocStderrForwardTarget::Stderr => {
                std::io::stderr().write_all(&buffer[..count])?;
                std::io::stderr().flush()?;
            }
            CoprocStderrForwardTarget::File(path) => {
                let mut file = OpenOptions::new().create(true).append(true).open(path)?;
                file.write_all(&buffer[..count])?;
            }
            CoprocStderrForwardTarget::Discard => {}
            CoprocStderrForwardTarget::CoprocStdin(ref mut writer) => {
                writer.write_all(&buffer[..count])?;
                writer.flush()?;
            }
        }
    }
}

impl Executor {
    /// Allocate a coproc pipe fd the way GNU exposes it: the parent keeps
    /// `rpipe[0]` and `wpipe[1]` from `sh_openpipe`, which moves the pipe
    /// ends to the highest free fds below 64 via `move_to_high_fd(maxfd 64)`.
    /// That yields rpipe 63/62 and wpipe 61/60, of which the parent retains
    /// 63 and 60 — the pair `${COPROC[@]}` prints as "63 60" for every
    /// coproc in coproc.tests. `slot` 0 requests the read end (63) and
    /// `slot` 1 the write end (60). Falls back to the low-first allocator
    /// when the preferred fd is already taken.
    /// Coproc pipe endpoints live in fd_table as handle-backed FileFd slots
    /// (governance 3.6 step 4 — bookkeeping migrated off the old
    /// coproc_*_writers/readers maps). These helpers resolve a coprocess pid
    /// to its pipe handle and retire a coprocess by closing its slots.
    pub(crate) fn coproc_read_file(&self, pid: u32) -> Option<Rc<FileFd>> {
        self.fd_table
            .entries
            .values()
            .find_map(|entry| match &entry.read {
                Some(FdReadEndpoint::CoprocStdout { pid: p, fd }) if *p == pid => {
                    Some(fd.clone())
                }
                _ => None,
            })
    }

    pub(crate) fn coproc_write_file(&self, pid: u32) -> Option<Rc<FileFd>> {
        self.fd_table
            .entries
            .values()
            .find_map(|entry| match &entry.write {
                Some(FdWriteEndpoint::CoprocStdin { pid: p, fd }) if *p == pid => {
                    Some(fd.clone())
                }
                _ => None,
            })
    }

    /// First open coproc read pipe — the `read <&COPROC` (fd 0) default.
    pub(crate) fn first_coproc_read(&self) -> Option<(u32, Rc<FileFd>)> {
        self.fd_table
            .entries
            .iter()
            .find_map(|(_, entry)| match &entry.read {
                Some(FdReadEndpoint::CoprocStdout { pid, fd }) if !entry.closed => {
                    Some((*pid, fd.clone()))
                }
                _ => None,
            })
    }

    pub(crate) fn coproc_has_endpoints(&self, pid: u32) -> bool {
        self.coproc_read_file(pid).is_some() || self.coproc_write_file(pid).is_some()
    }

    /// Close every fd slot carrying this coprocess's pipe ends. Dropping the
    /// last Rc<FileFd> closes the underlying HANDLE.
    pub(crate) fn close_coproc_endpoints(&mut self, pid: u32) {
        let fds: Vec<u32> = self
            .fd_table
            .entries
            .iter()
            .filter_map(|(fd, entry)| {
                let r = matches!(
                    entry.read.as_ref(),
                    Some(FdReadEndpoint::CoprocStdout { pid: p, .. }) if *p == pid
                );
                let w = matches!(
                    entry.write.as_ref(),
                    Some(FdWriteEndpoint::CoprocStdin { pid: p, .. }) if *p == pid
                );
                (r || w).then_some(*fd)
            })
            .collect();
        for fd in fds {
            self.fd_table.close(fd);
        }
    }

    fn allocate_coproc_fd(&mut self, slot: usize) -> u32 {
        let want = if slot == 0 { 63u32 } else { 60u32 };
        let free = self.fd_table.entries.get(&want).map_or(true, |e| {
            e.closed || (e.read.is_none() && e.write.is_none())
        });
        if free {
            want
        } else {
            self.fd_table.allocate_dynamic()
        }
    }

    pub(in crate::executor) fn execute_inverted_ast_command(
        &mut self,
        inverted_command: &InvertedCommand,
    ) -> Result<(), ExecuteError> {
        let ast = Ast {
            commands: vec![(*inverted_command.command).clone()],
        };
        self.with_errexit_suppressed(|executor| executor.execute_ast(&ast))?;
        self.exit_code = invert_exit_status(self.exit_code);
        Ok(())
    }

    pub(in crate::executor) fn execute_background_ast_command(
        &mut self,
        background_command: &BackgroundCommand,
    ) -> Result<(), ExecuteError> {
        // GNU execute_simple_command runs run_debug_trap (execute_cmd.c:4506)
        // BEFORE make_child forks the async child (~4550), so `true &` reports
        // `DBG:true` from the parent. A compound async body ({ ...; } &,
        // ( ... ) &) instead goes through execute_in_subshell, whose child
        // resets the trap table (execute_cmd.c:1670 reset_signal_handlers /
        // trap.c:1588), so nothing fires for it. Unwrap `!` prefixes: the
        // printed command for `! true &` is the plain `true` —
        // print_simple_command does not render CMD_INVERT_RETURN.
        {
            let mut inner = &background_command.command;
            while let Some(inverted_command) = &inner.inverted_command {
                inner = &inverted_command.command;
            }
            let inner_defers_debug = inner.for_command.is_some()
                || inner.if_command.is_some()
                || inner.loop_command.is_some()
                || inner.select_command.is_some()
                || inner.case_command.is_some()
                || inner.coproc_command.is_some()
                || inner.subshell_command.is_some()
                || inner.subshell
                || inner.brace_group.is_some()
                || inner.time_command.is_some()
                || inner.background_command.is_some()
                || inner.pipeline_command.is_some()
                || inner.pipe.is_some()
                || inner.and_or_list.is_some()
                || inner.function_command.is_some()
                || inner.arithmetic_command.is_some()
                || inner.conditional_command.is_some();
            if !inner_defers_debug && self.debug_trap_in_scope() {
                let inner_text = bash_command_source_text(inner);
                let _ = self.run_debug_trap(&inner_text)?;
            }
        }
        let exe = std::env::var_os("CARGO_BIN_EXE_rubash")
            .map(std::path::PathBuf::from)
            .or_else(test_rubash_binary_from_current_exe)
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| "rubash".into());
        let display_source = bash_command_source_text(&background_command.command);
        // Environment block for the child: process env merged with the
        // shell's env_vars (same filter the old Command::env loop applied).
        let mut env_map: std::collections::BTreeMap<String, String> =
            std::env::vars().collect();
        for (key, value) in &self.shell_state.env_vars {
            if !key.starts_with("__RUBASH_") || rubash_spawn_inherited_state(key) {
                env_map.insert(key.clone(), value.clone());
            }
        }
        // POSIX 2.11 (Signals and Error Handling): caught traps reset to
        // their default in a subshell, and GNU's fork+exec background child
        // never reaches the parent's exit-trap path. The child inherits this
        // process's environ (which carries the trap table), so drop those
        // keys or the background child fires the inherited EXIT trap when
        // its command finishes (trap.tests: three stray "exiting" lines
        // around the monitored `sleep 7 & sleep 6 & sleep 5 & / wait`).
        // SIG_IGN dispositions DO cross the fork boundary (trap.c
        // original_signals -> SIG_HARD_IGNORE), so __RUBASH_TRAP_ORIG_IGN is
        // forwarded above and must survive this reset filter.
        for key in self
            .shell_state
            .env_vars
            .keys()
            .filter(|key| key.starts_with("__RUBASH_TRAP") && *key != "__RUBASH_TRAP_ORIG_IGN")
        {
            env_map.remove(key);
        }
        env_map.insert(
            "__RUBASH_SHELL_PID".to_string(),
            self.shell_pid.to_string(),
        );

        // GNU execute_cmd.c:5884 (execute_disk_command) / :1761-1763
        // (execute_in_subshell): the forked async subshell runs
        // do_redirections(command->redirects, RX_ACTIVE) on its OWN
        // descriptors and then dispose_redirects before the body executes,
        // so `sleep 1 >/dev/null 2>&1 &` leaves no process holding the
        // parent's stdout/stderr open and a caller reading the shell's pipes
        // to EOF is released when the shell exits (niubash#122). The spawned
        // `rubash -c` child stands in for that subshell: resolve the
        // command's own redirections into its stdio here, and strip the
        // consumed redirects from the serialized `-c` source so the child
        // does not re-open them — `cmd >f 2>&1` must keep fd1/fd2 on the
        // same open file description, which only the spawn-time stdio pair
        // preserves.
        let (stdio, consumed) = self.resolve_background_stdio(&background_command.command);
        let mut child_command = background_command.command.clone();
        strip_consumed_redirects(&mut child_command, &consumed);
        let source = self.background_command_source(&child_command);

        // GNU forks the async subshell: the child's fd table is a copy of the
        // parent's. std::process::Command cannot express a per-spawn handle
        // allowlist (its anonymous inherit-everything spawn leaked this
        // shell's pipe write ends — niubash#122 — and the flag-flip
        // workaround both raced other spawns and could not carry fd>=3
        // across), so resolve each stdio disposition to an owned inheritable
        // HANDLE and spawn through STARTUPINFOEXW +
        // PROC_THREAD_ATTRIBUTE_HANDLE_LIST (src/fd::spawn_whitelisted).
        #[cfg(windows)]
        use std::os::windows::io::AsRawHandle;
        let mut owned_handles: Vec<crate::fd::HANDLE> = Vec::new();
        let mut std_handles = [0 as crate::fd::HANDLE; 3];
        for (fd, resolved) in stdio.iter().enumerate() {
            let h = match resolved {
                BackgroundStdio::Inherit => {
                    let src = crate::fd::process_std_handle(fd as u32);
                    if src == 0 || src == -1 {
                        crate::fd::open_null_device_inheritable()?
                    } else {
                        crate::fd::duplicate_handle_inheritable(src)?
                    }
                }
                BackgroundStdio::Null => crate::fd::open_null_device_inheritable()?,
                BackgroundStdio::File(file) => {
                    #[cfg(windows)]
                    {
                        crate::fd::duplicate_handle_inheritable(
                            file.as_raw_handle() as crate::fd::HANDLE,
                        )?
                    }
                    #[cfg(unix)]
                    {
                        crate::fd::duplicate_handle_inheritable(
                            std::os::unix::io::AsRawFd::as_raw_fd(file),
                        )?
                    }
                }
            };
            owned_handles.push(h);
            std_handles[fd] = h;
        }

        // fd>=3 File endpoints ride the whitelist too: each is duplicated to
        // a fresh inheritable handle (the shared slot handle's inherit flag
        // is never touched, so concurrent spawns cannot race) and the child
        // learns the fd->handle binding through __RUBASH_FD_* env keys.
        let mut extra_handles: Vec<crate::fd::HANDLE> = Vec::new();
        let mut fd_shared_dups: HashMap<usize, crate::fd::HANDLE> = HashMap::new();
        for (fd, entry) in &self.fd_table.entries {
            if *fd < 3 || entry.closed {
                continue;
            }
            let read_file = match &entry.read {
                Some(FdReadEndpoint::File(f)) => Some(f),
                Some(FdReadEndpoint::CoprocStdout { fd, .. }) => Some(fd),
                _ => None,
            };
            if let Some(f) = read_file {
                let key = Rc::as_ptr(f) as usize;
                let dup = match fd_shared_dups.get(&key) {
                    Some(&h) => h,
                    None => {
                        let dup = crate::fd::duplicate_handle_inheritable(f.handle)?;
                        fd_shared_dups.insert(key, dup);
                        extra_handles.push(dup);
                        owned_handles.push(dup);
                        dup
                    }
                };
                env_map.insert(
                    format!("__RUBASH_FD_HANDLE_{fd}"),
                    format!("{:#x}", dup),
                );
                env_map.insert(
                    format!("__RUBASH_FD_PATH_{fd}"),
                    f.path.to_string_lossy().into_owned(),
                );
            }
            let write_file = match &entry.write {
                Some(FdWriteEndpoint::File(f)) => Some(f),
                Some(FdWriteEndpoint::CoprocStdin { fd, .. }) => Some(fd),
                _ => None,
            };
            if let Some(f) = write_file {
                let key = Rc::as_ptr(f) as usize;
                let dup = match fd_shared_dups.get(&key) {
                    Some(&h) => h,
                    None => {
                        let dup = crate::fd::duplicate_handle_inheritable(f.handle)?;
                        fd_shared_dups.insert(key, dup);
                        extra_handles.push(dup);
                        owned_handles.push(dup);
                        dup
                    }
                };
                env_map.insert(
                    format!("__RUBASH_FD_WHANDLE_{fd}"),
                    format!("{:#x}", dup),
                );
                env_map.insert(
                    format!("__RUBASH_FD_WPATH_{fd}"),
                    f.path.to_string_lossy().into_owned(),
                );
            }
        }

        let spawn_result = crate::fd::spawn_whitelisted(&crate::fd::WhitelistedSpawn {
            program: exe.clone(),
            args: vec!["-c".to_string(), source.clone()],
            env: env_map.into_iter().collect(),
            std_handles,
            extra_handles,
        });
        for h in owned_handles {
            crate::fd::close_handle(h);
        }
        let child = spawn_result?;
        let pid = child.id();
        self.background_children.insert(pid, child);
        self.shell_state.job_table
            .register_process(pid, display_source.clone(), true);
        self.shell_state.job_table.set_job_control(
            pid,
            crate::builtins::set::shell_option_enabled(
                &self.shell_state.env_vars,
                "monitor",
            ),
        );
        self.shell_state.last_background_pid = Some(pid);
        self.exit_code = 0;
        Ok(())
    }

    /// Mirror of GNU do_redirections (redir.c) restricted to fds 0-2 for the
    /// spawned background subshell: walk the command's redirections in parse
    /// order so `2>&1 >/dev/null` keeps stderr on the inherited pipe while
    /// `>/dev/null 2>&1` releases both, exactly as GNU's sequential dup2
    /// ordering does. Best-effort: a target that cannot be resolved here
    /// (virtual fds, failed opens, expansion side effects) is left in the
    /// returned `consumed` map as false so it stays in the child's `-c`
    /// source, where the child re-runs it and reports the same diagnostic
    /// GNU's subshell would.
    fn resolve_background_stdio(&mut self, command: &CommandNode) -> ([BackgroundStdio; 3], Vec<bool>) {
        use crate::parser::RedirectKind;
        // GNU execute_cmd.c:2837-2841 (execute_connection '&') + :595
        // async_redirect_stdin: CMD_STDIN_REDIR is set — and the spawned
        // child dups /dev/null onto fd 0 — only when the shell is in a
        // subshell or lacks job control AND the enclosing control structure
        // did not redirect stdin. The global stdin_redir (execute_cmd.c:828,
        // cleared per reader command at eval.c:181) decides: `{ read; } &`
        // inside `while ... done <<EOF` inherits the heredoc, while a bare
        // `sleep 5 &` gets /dev/null.
        let job_control = crate::builtins::set::shell_option_enabled(
            &self.shell_state.env_vars,
            "monitor",
        );
        let subshell_environment = self.shell_state.subshell_depth.get() > 0
            || self.shell_state.in_command_substitution.get();
        let mut fd0 = if (subshell_environment || !job_control)
            && !self.shell_state.stdin_redir.get()
        {
            BackgroundStdio::Null
        } else {
            self.inherited_async_stdin()
        };
        let mut fd1 = BackgroundStdio::Inherit;
        let mut fd2 = BackgroundStdio::Inherit;
        let mut consumed = vec![false; command.redirects.len()];

        let target_of =
            |fd: u32, fd0: &BackgroundStdio, fd1: &BackgroundStdio, fd2: &BackgroundStdio| match fd
            {
                0 => fd0.clone(),
                1 => fd1.clone(),
                2 => fd2.clone(),
                _ => BackgroundStdio::Inherit,
            };

        for (index, redirect) in command.redirects.iter().enumerate() {
            match redirect.kind {
                // GNU here_document_to_fd (redir.c): a forked async subshell
                // finds the heredoc already staged on a real fd — the body
                // text cannot ride the child's `-c` source, so expand and
                // materialize it into a temp file here and hand the child
                // that descriptor, exactly like do_redirections does in the
                // subshell.
                RedirectKind::HereDoc | RedirectKind::HereString => {
                    let fd = redirect.fd.unwrap_or(0);
                    if fd > 2 || redirect.fd_var.is_some() {
                        continue;
                    }
                    match self.materialize_async_stdin_body(command, index) {
                        Some(file) => {
                            consumed[index] = true;
                            match fd {
                                0 => fd0 = BackgroundStdio::File(file),
                                1 => fd1 = BackgroundStdio::File(file),
                                _ => fd2 = BackgroundStdio::File(file),
                            }
                        }
                        None => continue,
                    }
                    continue;
                }
                _ => {}
            }
            // `{fd}>file` dynamic fd bindings and `<(cmd)`/`>(cmd)` process
            // substitutions are owned by the child's executor, which knows
            // how to bind them; do not mistake them for plain fd 0-2 files.
            if redirect.fd_var.is_some()
                || redirect.target.starts_with("<(")
                || redirect.target.starts_with(">(")
            {
                continue;
            }
            let fd = redirect.fd.unwrap_or(match redirect.kind {
                RedirectKind::Input
                | RedirectKind::DuplicateInput
                | RedirectKind::CloseInput
                | RedirectKind::ReadWrite => 0,
                _ => 1,
            });
            if fd > 2 {
                continue;
            }
            let resolved = match redirect.kind {
                RedirectKind::Input => {
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        BackgroundStdio::Null
                    } else if redirect_target_fd(&target).is_some() {
                        continue;
                    } else {
                        match self.open_input_redirect(&target) {
                            Ok(file) => BackgroundStdio::File(file),
                            Err(_) => continue,
                        }
                    }
                }
                RedirectKind::DuplicateInput | RedirectKind::DuplicateOutput => {
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        BackgroundStdio::Null
                    } else if let Some(source_fd) = redirect_target_fd(&target) {
                        // `N>&M`: fd N ends up wherever fd M currently
                        // points — GNU's dup2 ordering semantics. Source fds
                        // above 2 are the child's own virtual fd table, so
                        // leave them for the child to resolve.
                        if source_fd > 2 {
                            continue;
                        }
                        target_of(source_fd, &fd0, &fd1, &fd2)
                    } else {
                        continue;
                    }
                }
                RedirectKind::CloseInput | RedirectKind::CloseOutput => BackgroundStdio::Null,
                RedirectKind::Output | RedirectKind::ClobberOutput => {
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        BackgroundStdio::Null
                    } else if redirect_target_fd(&target).is_some() {
                        continue;
                    } else if is_null_device(&target) {
                        BackgroundStdio::Null
                    } else {
                        match self.create_redirect_output(&target, redirect.clobber) {
                            Ok(file) => BackgroundStdio::File(file),
                            Err(_) => continue,
                        }
                    }
                }
                RedirectKind::Append => {
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        BackgroundStdio::Null
                    } else if redirect_target_fd(&target).is_some() {
                        continue;
                    } else if is_null_device(&target) {
                        BackgroundStdio::Null
                    } else {
                        match OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(shell_path_to_windows(&target, &self.shell_state.env_vars))
                        {
                            Ok(file) => BackgroundStdio::File(file),
                            Err(_) => continue,
                        }
                    }
                }
                RedirectKind::ReadWrite => {
                    let target = self.expand_redirect_target(redirect);
                    match OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .open(shell_path_to_windows(&target, &self.shell_state.env_vars))
                    {
                        Ok(file) => BackgroundStdio::File(file),
                        Err(_) => continue,
                    }
                }
                RedirectKind::CombinedOutput | RedirectKind::CombinedAppend => {
                    // `&>file` / `&>>file`: one open, both fds share it.
                    let target = self.expand_redirect_target(redirect);
                    let file = if is_null_device(&target) {
                        fd1 = BackgroundStdio::Null;
                        fd2 = BackgroundStdio::Null;
                        consumed[index] = true;
                        continue;
                    } else {
                        let opened = if redirect.kind == RedirectKind::CombinedAppend {
                            OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(shell_path_to_windows(&target, &self.shell_state.env_vars))
                        } else {
                            self.create_redirect_output(&target, false)
                        };
                        match opened {
                            Ok(file) => file,
                            Err(_) => continue,
                        }
                    };
                    fd1 = match file.try_clone() {
                        Ok(clone) => BackgroundStdio::File(clone),
                        Err(_) => continue,
                    };
                    fd2 = BackgroundStdio::File(file);
                    consumed[index] = true;
                    continue;
                }
                _ => continue,
            };
            consumed[index] = true;
            match fd {
                0 => fd0 = resolved,
                1 => fd1 = resolved,
                _ => fd2 = resolved,
            }
        }

        ([fd0, fd1, fd2], consumed)
    }

    /// GNU make_child fork semantics for an async command that inherits
    /// fd 0 (stdin_redir set, or job-control shell outside a subshell):
    /// the child shares the parent's open file description, file offset
    /// included. Handle-backed endpoints map to DuplicateHandle dups, which
    /// share the file pointer exactly like a fork. Text endpoints and the
    /// FUNCTION_STDIN channel have no kernel object, so the unread tail is
    /// written to a temp file and fd 0 is moved onto it — the same trick
    /// GNU gets for free because here-documents already live in an
    /// unlinked temp file (redir.c here_document_to_fd).
    fn inherited_async_stdin(&mut self) -> BackgroundStdio {
        // FUNCTION_STDIN is the newest fd-0 authority when present: a
        // compound `done <<EOF` stages the body on the text channel while a
        // previous `exec 0<real` may still hold a File endpoint on slot 0 —
        // the virtual text wins, exactly like GNU's dup2 of the heredoc
        // temp file over the earlier descriptor.
        if let Some(remaining) = self.function_stdin_remaining() {
            let bytes =
                crate::executor::substitution_metadata::shell_text_to_raw_bytes(&remaining);
            match self.materialize_virtual_stdin_fd0(&bytes) {
                Some(file) => {
                    // fd 0 now owns a real endpoint; drop the text channel
                    // so no consumer double-sources it. The compound-command
                    // restore puts the parent's FUNCTION_STDIN back when the
                    // scope ends.
                    self.shell_state.env_vars.remove(FUNCTION_STDIN);
                    self.shell_state.env_vars.remove(FUNCTION_STDIN_OFFSET);
                    return BackgroundStdio::File(file);
                }
                None => return BackgroundStdio::Null,
            }
        }
        match self.fd_table.read_endpoint(0) {
            Some(FdReadEndpoint::File(file))
            | Some(FdReadEndpoint::CoprocStdout { fd: file, .. }) => {
                match crate::fd::duplicate_handle_inheritable(file.handle) {
                    Ok(dup) => BackgroundStdio::File(crate::fd::handle_to_file(dup)),
                    Err(_) => BackgroundStdio::Null,
                }
            }
            Some(FdReadEndpoint::Text(_)) | Some(FdReadEndpoint::ProcessSubstitution(_)) => {
                let bytes = self
                    .fd_table
                    .input_snapshot_bytes(0)
                    .map(|(data, offset)| data[offset.min(data.len())..].to_vec())
                    .unwrap_or_default();
                match self.materialize_virtual_stdin_fd0(&bytes) {
                    Some(file) => BackgroundStdio::File(file),
                    None => BackgroundStdio::Null,
                }
            }
            Some(FdReadEndpoint::InheritedProcessStdin) => BackgroundStdio::Inherit,
            // fd 0 closed or absent: the closest spawnable state to GNU's
            // inherited-but-unreadable descriptor.
            None => BackgroundStdio::Null,
        }
    }

    /// Move fd 0 onto a real temp file holding `bytes`: the FileFd on the
    /// slot keeps the parent's read position in lockstep with any child's
    /// dup (shared file object), which is exactly what GNU's inherited fd
    /// gives a forked async subshell.
    fn materialize_virtual_stdin_fd0(&mut self, bytes: &[u8]) -> Option<std::fs::File> {
        let path = self.process_substitution_temp_path().ok()?;
        fs::write(&path, bytes).ok()?;
        let file_fd = FileFd::open_read(path).ok()?;
        let dup = crate::fd::duplicate_handle_inheritable(file_fd.handle).ok()?;
        self.set_fd_input_file(0, file_fd, false);
        Some(crate::fd::handle_to_file(dup))
    }

    /// Expand the body behind `command.redirects[index]` (a HereDoc or
    /// HereString entry) and stage it on a real temp file, mirroring GNU's
    /// here_document_to_fd so the spawned `-c` child reads an actual
    /// descriptor. `heredoc_redirects` is the parallel store for bodies; an
    /// entry correlates by fd, here-string flag, and ordinal among the
    /// matching `redirects` entries, with the fd-0 legacy fields
    /// (`heredoc`/`here_string` and their carriers) as fallback.
    fn materialize_async_stdin_body(
        &mut self,
        command: &CommandNode,
        index: usize,
    ) -> Option<std::fs::File> {
        use crate::parser::RedirectKind;
        let redirect = &command.redirects[index];
        let want_string = matches!(redirect.kind, RedirectKind::HereString);
        let ordinal = command.redirects[..index]
            .iter()
            .filter(|entry| {
                matches!(
                    entry.kind,
                    RedirectKind::HereDoc | RedirectKind::HereString
                ) && matches!(entry.kind, RedirectKind::HereString) == want_string
                    && entry.fd == redirect.fd
            })
            .count();
        let expanded = command
            .heredoc_redirects
            .iter()
            .filter(|entry| entry.fd == redirect.fd && entry.here_string == want_string)
            .nth(ordinal)
            .map(|entry| {
                if entry.body_carrier.is_some() {
                    if entry.here_string {
                        self.expand_here_string_mut_from_carrier(&entry.body_carrier)
                    } else {
                        self.expand_heredoc_body_mut_from_carrier(&entry.body_carrier)
                    }
                } else {
                    entry
                        .body
                        .as_deref()
                        .map(|body| self.expand_heredoc_body_mut(body))
                        .unwrap_or_default()
                }
            })
            .or_else(|| {
                if redirect.fd.is_some() {
                    return None;
                }
                if want_string {
                    if command.here_string_carrier.is_some() {
                        Some(
                            self.expand_here_string_mut_from_carrier(
                                &command.here_string_carrier,
                            ),
                        )
                    } else {
                        let body = command.here_string.clone()?;
                        Some(self.expand_here_string_mut(&body))
                    }
                } else if command.heredoc_body.is_some() {
                    Some(self.expand_heredoc_body_mut_from_carrier(&command.heredoc_body))
                } else {
                    let body = command.heredoc.clone()?;
                    Some(self.expand_heredoc_body_mut(&body))
                }
            })?;
        let path = self.process_substitution_temp_path().ok()?;
        fs::write(&path, expanded.as_bytes()).ok()?;
        std::fs::File::open(&path).ok()
    }

    fn background_command_source(&self, command: &CommandNode) -> String {
        let mut source = String::new();
        for (name, body) in &self.shell_state.functions {
            if is_exportable_function_name(name) {
                source.push_str(name);
                source.push_str("() { ");
                source.push_str(&bash_command_sequence_text(&body.commands));
                source.push_str("; }; ");
            }
        }
        source.push_str(&bash_command_source_text(command));
        source
    }

    pub(in crate::executor) fn execute_time_ast_command(
        &mut self,
        time_command: &TimeCommand,
    ) -> Result<(), ExecuteError> {
        let started = time_command_started();
        if let Some(coproc_cmd) = &time_command.command.coproc_command {
            self.execute_coproc_command(&time_command.command, coproc_cmd)?;
        } else if time_command.command.words.is_empty()
            && command_has_no_effect(&time_command.command)
        {
            self.exit_code = 0;
        } else if let Some(pipeline_command) = &time_command.command.pipeline_command {
            self.execute_pipeline_command(pipeline_command)?;
        } else if time_command.command.brace_group.is_some() {
            self.execute_brace_group_pipeline(&time_command.command)?;
        } else if time_command.command.words.first().map(String::as_str) == Some("coproc") {
            self.execute_time_reparsed_coproc(&time_command.command)?;
        } else {
            // GNU execute_time_command dispatches the inner command through
            // execute_command, so a simple/`(( ))`/`[[ ]]` inner fires its own
            // run_debug_trap (execute_cmd.c:4506/3920/4153) — `time true`
            // reports `DBG:true`, never `DBG:time true`. Compound inners
            // (for/if/while/select/case/subshell/brace/pipeline/coproc/and-or)
            // fire inside their own handlers, so they are skipped here.
            let inner = &time_command.command;
            let inner_defers_debug = inner.for_command.is_some()
                || inner.if_command.is_some()
                || inner.loop_command.is_some()
                || inner.select_command.is_some()
                || inner.case_command.is_some()
                || inner.coproc_command.is_some()
                || inner.subshell_command.is_some()
                || inner.subshell
                || inner.brace_group.is_some()
                || inner.inverted_command.is_some()
                || inner.background_command.is_some()
                || inner.pipeline_command.is_some()
                || inner.pipe.is_some()
                || inner.and_or_list.is_some()
                || inner.function_command.is_some()
                || inner.time_command.is_some();
            if !inner_defers_debug && self.debug_trap_in_scope() {
                let inner_text = bash_command_source_text(inner);
                let _ = self.run_debug_trap(&inner_text)?;
            }
            self.execute_command(&time_command.command)?;
        }
        print_time(&self.shell_state.env_vars, time_command.posix_format, started);
        if time_command.inverted {
            self.exit_code = invert_exit_status(self.exit_code);
        }
        Ok(())
    }

    fn execute_time_reparsed_coproc(&mut self, command: &CommandNode) -> Result<(), ExecuteError> {
        let source = bash_command_source_text(command);
        let tokens = crate::lexer::tokenize(&source);
        let reparsed = crate::parser::parse(&tokens);
        if let Some(coproc_command) = reparsed
            .commands
            .first()
            .and_then(|command| command.coproc_command.as_ref())
        {
            return self.execute_coproc_command(command, coproc_command);
        }

        self.execute_command(command)
    }

    pub(in crate::executor) fn execute_time_prefixed_compound_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        let inverted = time_prefix_parts(&cmd.words)
            .map(|parts| parts.inverted)
            .unwrap_or(false);
        let started = time_command_started();
        let result = if let Some(for_command) = &cmd.for_command {
            self.execute_for_command_with_redirects(for_command, cmd)
        } else if let Some(if_command) = &cmd.if_command {
            self.execute_if_command_with_redirects(cmd, if_command)
        } else if let Some(loop_command) = &cmd.loop_command {
            self.execute_loop_command_with_redirects(cmd, loop_command)
        } else if let Some(select_command) = &cmd.select_command {
            self.execute_select_command(cmd, select_command)
        } else if let Some(case_command) = &cmd.case_command {
            self.execute_case_command_with_redirects(cmd, case_command)
        } else if let Some(coproc_cmd) = &cmd.coproc_command {
            self.execute_coproc_command(cmd, coproc_cmd)
        } else if let Some(subshell_command) = &cmd.subshell_command {
            self.execute_subshell_command_with_redirects(cmd, subshell_command)
        } else if cmd.brace_group.is_some() {
            self.execute_brace_group_pipeline(cmd).map(|_| ())
        } else {
            Ok(())
        };
        print_time(
            &self.shell_state.env_vars,
            time_prefix_parts(&cmd.words).is_some_and(|parts| parts.posix_format),
            started,
        );
        result?;
        if inverted {
            self.exit_code = invert_exit_status(self.exit_code);
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_time_prefixed_command_sequence(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let Some(command) = ast.commands.get(index) else {
            return Ok(None);
        };
        let Some(prefix) = time_prefix_parts(&command.words) else {
            return Ok(None);
        };
        if !matches!(
            command.words.get(prefix.command_index).map(String::as_str),
            Some("if" | "while" | "until")
        ) {
            return Ok(None);
        }

        let mut timed_ast = Ast {
            commands: ast.commands[index..].to_vec(),
        };
        if let Some(first) = timed_ast.commands.first_mut() {
            first.words = command.words[prefix.command_index..].to_vec();
            if command.word_kinds.len() == command.words.len() {
                first.word_kinds = command.word_kinds[prefix.command_index..].to_vec();
            }
            if command.word_metadata.len() == command.words.len() {
                first.word_metadata = command.word_metadata[prefix.command_index..].to_vec();
            }
        }

        let started = time_command_started();
        let next_index = match timed_ast.commands[0].words.first().map(String::as_str) {
            Some("if") => crate::builtins::source::execute_simple_if(self, &timed_ast, 0)?,
            Some("while" | "until") => self.execute_simple_loop(&timed_ast, 0)?,
            _ => None,
        };
        let Some(next_index) = next_index else {
            return Ok(None);
        };

        print_time(&self.shell_state.env_vars, prefix.posix_format, started);
        if prefix.inverted {
            self.exit_code = invert_exit_status(self.exit_code);
        }
        Ok(Some(index + next_index))
    }

    pub(in crate::executor) fn execute_arithmetic_for_command(
        &mut self,
        arithmetic: &ArithmeticForCommand,
        body: &[CommandNode],
    ) -> Result<(), ExecuteError> {
        let mut arithmetic_failed = false;
        // GNU 5.3 execute_arith_for_command increments loop_level before
        // evaluating init and always decrements after, even on init failure.
        // Git Bash 5.2 (and the temporary baseline in this task) omits the
        // decrement on failed init, leaving loop_level==1 so the following
        // `break` is considered in-loop, suppresses its diagnostic and aborts
        // the remainder of the script (arith-for.tests: `for ((j=;;))` with
        // `j=` empty RHS). Match the Git Bash baseline for now.
        self.shell_state.loop_depth += 1;
        // GNU execute_arith_for_command:3236 sets line_number = arith_lineno
        // = arith_for_command->line, and eval_arith_for_expr:3187 runs the
        // DEBUG trap before each expression evaluation (init once; test and
        // step once per iteration) with that line restored, so $LINENO inside
        // the fire is the for command's line (dbg-support.tests: the double
        // "debug lineno: 108 main" per iteration).
        let for_line = self.shell_state.env_vars.get("__RUBASH_CURRENT_LINE").cloned();
        let restore_for_line = |executor: &mut Executor| {
            if let Some(line) = &for_line {
                executor
                    .shell_state.env_vars
                    .insert("__RUBASH_CURRENT_LINE".to_string(), line.clone());
            }
        };
        if self.debug_trap_in_scope() && !arithmetic.init.trim().is_empty() {
            restore_for_line(self);
            self.run_debug_trap(&arithmetic.init)?;
        }
        if !arithmetic.init.trim().is_empty() && {
            // GNU execute_cmd.c:3201 (eval_arith_for_expr): if `set -x`
            // is on, print `(( expr ))` before evaluating each for-loop
            // expression (init, test, update).  Use the raw expression to
            // preserve original whitespace (e.g. `i++ ` trailing space).
            self.xtrace_print_arith_cmd(&arithmetic.init_metadata.expression);
            self.eval_arithmetic_command_value(&arithmetic.init)
                .is_none()
        } {
            self.report_arithmetic_error_raw_display(&arithmetic.init_metadata.expression);
            self.exit_code = 1;
            arithmetic_failed = true;
        }

        let mut ran_body = false;
        let body_ast = Ast {
            commands: body.to_vec(),
        };
        while !arithmetic_failed {
            if self.debug_trap_in_scope() && !arithmetic.test.trim().is_empty() {
                restore_for_line(self);
                self.run_debug_trap(&arithmetic.test)?;
            }
            if !arithmetic.test.trim().is_empty() {
                let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_FOR_TEST);
                // GNU execute_cmd.c:3201: xtrace before test expression.
                // Use raw expression to preserve original whitespace.
                self.xtrace_print_arith_cmd(&arithmetic.test_metadata.expression);
                match self.eval_arithmetic_command_value(&arithmetic.test) {
                    Some(0) => break,
                    Some(_) => {}
                    None => {
                        self.report_arithmetic_error_raw_display(
                            &arithmetic.test_metadata.expression,
                        );
                        self.exit_code = 1;
                        arithmetic_failed = true;
                        break;
                    }
                }
            }

            // GNU execute_cmd.c:3267 REAP() runs once per arith-for
            // iteration, after the test succeeds and before the body.
            self.reap_dead_jobs_after_loop_body();
            ran_body = true;
            self.shell_state.loop_depth += 1;
            let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_FOR_BODY);
            // execute_cmd.c:3236: line_number = arith_for_command->line is
            // the ambient for the body's non-line-setting commands.
            let for_ambient = for_line
                .as_deref()
                .and_then(|line| line.parse::<usize>().ok());
            let result = self.with_ambient_line(for_ambient, |executor| {
                executor.execute_ast(&body_ast)
            });
            drop(_t);
            self.shell_state.loop_depth -= 1;
            match result {
                Ok(()) => {}
                Err(ExecuteError::Break(level)) if level <= 1 => {
                    self.exit_code = 0;
                    break;
                }
                Err(ExecuteError::Break(level)) => return Err(ExecuteError::Break(level - 1)),
                Err(ExecuteError::Continue(level)) if level <= 1 => {
                    self.exit_code = 0;
                }
                Err(ExecuteError::Continue(level)) => {
                    return Err(ExecuteError::Continue(level - 1));
                }
                Err(error) => return Err(error),
            }

            if self.debug_trap_in_scope() && !arithmetic.update.trim().is_empty() {
                restore_for_line(self);
                self.run_debug_trap(&arithmetic.update)?;
            }
            if !arithmetic.update.trim().is_empty() {
                let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_FOR_UPDATE);
                // GNU execute_cmd.c:3201: xtrace before update expression.
                // Use raw expression to preserve original whitespace.
                self.xtrace_print_arith_cmd(&arithmetic.update_metadata.expression);
                if self
                    .eval_arithmetic_command_value(&arithmetic.update)
                    .is_none()
                {
                    self.report_arithmetic_error_raw_display(
                        &arithmetic.update_metadata.expression,
                    );
                    self.exit_code = 1;
                    arithmetic_failed = true;
                    break;
                }
            }
        }

        if arithmetic_failed {
            self.exit_code = 1;
        } else if !ran_body {
            self.exit_code = 0;
        }
        // GNU 5.3 execute_arith_for_command always decrements loop_level
        // after the loop, even when init failed. Git Bash 5.2 omitted the
        // decrement on failed init, leaving loop_level==1 so a following
        // `break` was treated as in-loop and aborted the rest of the script
        // (arith-for.tests: `for ((j=;;))` with `j=` empty RHS). Match GNU.
        self.shell_state.loop_depth -= 1;
        Ok(())
    }

    pub(in crate::executor) fn execute_if_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        if_command: &IfCommand,
    ) -> Result<(), ExecuteError> {
        if self.if_command_needs_alias_scan(if_command) {
            let flat = flatten_if_command_for_alias_scan(cmd, if_command);
            crate::builtins::source::execute_simple_if(self, &Ast { commands: flat }, 0)?;
            return Ok(());
        }

        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut if_command = if_command.clone();
        let result = apply_if_redirects(self, &redirect_cmd, &mut if_command).and_then(|()| {
            self.with_command_input_redirects(cmd, |executor| {
                executor.execute_if_command(&if_command)
            })
        });
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    pub(in crate::executor) fn execute_loop_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        loop_command: &LoopCommand,
    ) -> Result<(), ExecuteError> {
        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut loop_command = loop_command.clone();
        let result = apply_redirects_to_commands(self, &redirect_cmd, &mut loop_command.body)
            .and_then(|()| {
                self.with_loop_fd_heredocs(cmd, |executor| {
                    executor.with_command_input_redirects(cmd, |executor| {
                        executor.execute_loop_command(&loop_command)
                    })
                })
            });
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    pub(in crate::executor) fn execute_subshell_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        subshell_command: &SubshellCommand,
    ) -> Result<(), ExecuteError> {
        // GNU execute_cmd.c:1576 execute_in_subshell: the forked child's
        // mutable state is a whole-copy of the parent's. The in-place body
        // gets the same boundary from a ShellState clone — assignments,
        // aliases, functions, set --, IFS, env-carried traps, positional
        // params, and job bookkeeping all restore wholesale at the end.
        let saved_state = self.shell_state.clone();
        // The fd table is executor state, not ShellState: a forked child's
        // descriptor table is a copy (execute_in_subshell after make_child),
        // so `( exec 3<&- )` cannot close the parent's fd 3. Rc-shared
        // entries still alias the same open file description — reads in the
        // subshell advance the shared offset, matching fork().
        let saved_fd_table = self.fd_table.clone();
        let saved_depth = self.shell_state.subshell_depth.get();
        // GNU execute_cmd.c runs `( list )` via execute_in_subshell ->
        // make_child: the forked child owns its own cwd, so a `cd` in the
        // body never reaches the parent. The body here runs in place on
        // this executor, so the process directory is part of the subshell
        // environment and must be restored like env_vars (niubash#100).
        // Same convention as command substitution's saved_dir handling.
        let saved_cwd = env::current_dir().ok();
        crate::builtins::trap::reset_for_subshell(&mut self.shell_state.env_vars);
        self.shell_state.subshell_depth.set(saved_depth + 1);
        self.shell_state.loop_depth = 0;

        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut body = Ast {
            commands: subshell_command.body.clone(),
        };
        // Numbered redirects are duplicated in parser stdio fields. Keep only
        // true stdio fields in compound preparation; the original redirect
        // list is applied to each body command below.
        let mut stdio_redirect_cmd = redirect_cmd.clone();
        let is_numbered = |redirect: &Redirect| {
            redirect
                .operator
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit())
        };
        if stdio_redirect_cmd
            .redirect_in
            .as_ref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_in = None;
        }
        if stdio_redirect_cmd
            .redirect_out
            .as_ref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_out = None;
        }
        if stdio_redirect_cmd.append.as_ref().is_some_and(is_numbered) {
            stdio_redirect_cmd.append = None;
        }
        if stdio_redirect_cmd
            .redirect_err
            .as_ref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_err = None;
        }
        if stdio_redirect_cmd
            .redirect_err_append
            .as_ref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_err_append = None;
        }
        self.apply_command_output_redirects(&stdio_redirect_cmd, &mut body)?;
        // GNU execute_cmd.c resolves subshell redirections in the parent
        // context before the body runs: expand and anchor relative file
        // targets now, or a `cd` in the body relocates them (niubash#118).
        // Numbered output redirects bind fd-table slots for the body's
        // duration through open_compound_output_redirects inside
        // with_command_input_redirects (redir.c do_redirection_internal).
        // GNU execute_cmd.c:696 SET_LINE_NUMBER(command->value.Subshell->line):
        // the subshell body runs under the `)` line as its ambient
        // line_number — inner while/if/group diagnostics report it, while
        // inner simple commands stamp and restore their own.
        let subshell_line = cmd.end_line.or(cmd.line);
        // The subshell's EXIT trap runs inside the scoped fd bindings: GNU's
        // execute_in_subshell forks, applies do_redirections to the child's
        // real descriptors, and runs run_exit_trap before the child exits —
        // the redirections are never undone in the child, so the trap writes
        // through the same open file descriptions as the body
        // (execute_cmd.c:1576-1763, redir.c:767-955). Re-binding `>file`
        // after the scope would re-open and truncate it, clobbering the
        // body's output.
        let mut body_status: Option<i32> = None;
        let mut trap_result: Option<Result<i32, ExecuteError>> = None;
        let result = self.with_loop_fd_heredocs(cmd, |executor| {
            executor.with_ambient_line(subshell_line, |executor| {
                executor.with_command_input_redirects(cmd, |executor| {
                    let body_result = executor.execute_ast(&body);
                    // GNU execute_cmd.c execute_in_subshell: expr.c
                    // evalerror's jump_to_top_level(DISCARD) reaches only the
                    // forked subshell's own top level — the abort dies with
                    // the subshell (status 1) and cannot discard parent
                    // commands (`( a[$bad]=v ); echo after` prints `after` —
                    // verified GNU 5.3).
                    executor.evalerror_pending.set(false);
                    executor.evalerror_line.set(None);
                    // Bash runs a subshell with errexit active; a failing
                    // command exits the subshell with that status but the
                    // parent script continues. Catch ExitCode errors at the
                    // subshell boundary. `return N` inside a subshell
                    // likewise only ends the subshell with status N: the
                    // forked child longjmps to its own copy of
                    // execute_function's return_catch (return.def
                    // return_builtin) and exits N, so the function continues
                    // with $? = N (func.tests: `( return 5 ); status=$?`
                    // prints 5, 5).
                    let status = match body_result {
                        Ok(()) => executor.exit_code,
                        Err(ExecuteError::ExitCode(code))
                        | Err(ExecuteError::ExpansionFailure(code))
                        | Err(ExecuteError::FatalFunctionError(code))
                        | Err(ExecuteError::Return(code)) => code,
                        Err(error) => return Err(error),
                    };
                    body_status = Some(status);
                    // GNU execute_cmd.c runs the subshell's own EXIT trap
                    // (trap.c run_exit_trap) while the subshell state is
                    // still live: `( trap "echo T" EXIT; echo body )` prints
                    // body then T (niubash#70). reset_for_subshell cleared
                    // the inherited traps at entry, so a pending EXIT here
                    // can only have been registered by the body itself; a
                    // trap action that calls exit N replaces the subshell
                    // status (bash exit_shell semantics).
                    trap_result = Some(executor
                        .run_exit_trap_for_status_with_output_redirects(
                            status,
                            Some(&stdio_redirect_cmd),
                        ));
                    Ok(())
                })
            })
        });
        let status = match result {
            Ok(()) => body_status.unwrap_or(self.exit_code),
            Err(error) => {
                self.restore_flat_subshell(saved_state.clone(), saved_cwd.clone());
                self.fd_table = saved_fd_table.clone();
                return Err(error);
            }
        };
        let status = match trap_result {
            Some(Ok(trap_status)) => trap_status,
            Some(Err(error)) => {
                self.restore_flat_subshell(saved_state.clone(), saved_cwd.clone());
                self.fd_table = saved_fd_table.clone();
                return Err(error);
            }
            None => status,
        };

        self.restore_flat_subshell(saved_state, saved_cwd);
        self.fd_table = saved_fd_table;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    fn execute_loop_command(&mut self, loop_command: &LoopCommand) -> Result<(), ExecuteError> {
        let mut ran_body = false;
        let mut last_body_status = 0;
        let condition = Ast {
            commands: loop_command.condition.clone(),
        };
        let body = Ast {
            commands: crate::builtins::source::normalize_inline_compound_commands(
                loop_command.body.clone(),
            ),
        };

        loop {
            self.with_errexit_suppressed(|executor| executor.execute_ast(&condition))?;
            self.run_pending_signal_traps()?;
            let condition_matched = self.exit_code == 0;
            if condition_matched == loop_command.until {
                break;
            }

            ran_body = true;
            self.shell_state.loop_depth += 1;
            let result = self.execute_ast(&body);
            self.shell_state.loop_depth -= 1;
            self.run_pending_signal_traps()?;
            match result {
                Ok(()) => {
                    last_body_status = self.exit_code;
                }
                Err(ExecuteError::Break(level)) if level <= 1 => {
                    self.exit_code = 0;
                    break;
                }
                Err(ExecuteError::Break(level)) => return Err(ExecuteError::Break(level - 1)),
                Err(ExecuteError::Continue(level)) if level <= 1 => {
                    self.exit_code = 0;
                    continue;
                }
                Err(ExecuteError::Continue(level)) => {
                    return Err(ExecuteError::Continue(level - 1));
                }
                Err(error) => return Err(error),
            }
        }

        if !ran_body {
            self.exit_code = 0;
        } else if self.exit_code != 0 {
            self.exit_code = last_body_status;
        }
        Ok(())
    }

    pub(in crate::executor) fn with_loop_fd_heredocs<F>(&mut self, cmd: &CommandNode, f: F) -> Result<(), ExecuteError>
    where
        F: FnOnce(&mut Executor) -> Result<(), ExecuteError>,
    {
        let mut saved_fd_inputs = Vec::new();
        for redirect in &cmd.heredoc_redirects {
            let (Some(fd), Some(body)) = (redirect.fd, redirect.body.clone()) else {
                continue;
            };
            // A loop's numbered heredoc is still an ordinary unquoted
            // heredoc. Keep its expansion rules identical to the command's
            // stdin heredoc, including parameter and command substitutions.
            // The typed carrier wins: a Preexpanded body was already expanded
            // at the do_redirections point and must not expand again.
            let body = if redirect.body_carrier.is_some() {
                if redirect.here_string {
                    self.expand_here_string_mut_from_carrier(&redirect.body_carrier)
                } else {
                    self.expand_heredoc_body_mut_from_carrier(&redirect.body_carrier)
                }
            } else {
                self.expand_heredoc_body_mut(&body)
            };
            saved_fd_inputs.push((fd, self.fd_table.entries.get(&fd).cloned()));
            self.fd_table
                .open_input(fd, FdReadEndpoint::text(&body), true);
        }

        let result = f(self);
        for (fd, old_entry) in saved_fd_inputs {
            match old_entry {
                Some(entry) => {
                    self.fd_table.entries.insert(fd, entry);
                }
                None => {
                    self.fd_table.entries.remove(&fd);
                }
            }
        }
        result
    }

    fn if_command_needs_alias_scan(&self, if_command: &IfCommand) -> bool {
        if !self.alias_expansion_enabled() {
            return false;
        }

        if self.commands_contain_alias_if_control(&if_command.then_body) {
            return true;
        }
        if if_command.elif_branches.iter().any(|branch| {
            self.commands_contain_alias_if_control(&branch.condition)
                || self.commands_contain_alias_if_control(&branch.body)
        }) {
            return true;
        }
        if let Some(body) = &if_command.else_body {
            return self.commands_contain_alias_if_control(body);
        }
        false
    }

    fn commands_contain_alias_if_control(&self, commands: &[CommandNode]) -> bool {
        commands.iter().any(|command| {
            let words = self.expand_aliases(&command.words);
            matches!(
                words.first().map(String::as_str),
                Some("if" | "then" | "elif" | "else" | "fi")
            )
        })
    }

    fn execute_if_command(&mut self, if_command: &IfCommand) -> Result<(), ExecuteError> {
        // GNU Bash 5.2 (probe f4, 2026-08-24): a word-expansion failure in
        // an if/elif condition abandons the whole compound command instead
        // of selecting the else branch.
        let matched = match self.if_condition_matches(&if_command.condition)? {
            Some(matched) => matched,
            None => return Ok(()),
        };
        if matched {
            return self.execute_ast(&Ast {
                commands: crate::builtins::source::normalize_inline_compound_commands(
                    if_command.then_body.clone(),
                ),
            });
        }

        for branch in &if_command.elif_branches {
            let matched = match self.if_condition_matches(&branch.condition)? {
                Some(matched) => matched,
                None => return Ok(()),
            };
            if matched {
                return self.execute_ast(&Ast {
                    commands: crate::builtins::source::normalize_inline_compound_commands(
                        branch.body.clone(),
                    ),
                });
            }
        }

        if let Some(body) = &if_command.else_body {
            return self.execute_ast(&Ast {
                commands: crate::builtins::source::normalize_inline_compound_commands(body.clone()),
            });
        }

        self.exit_code = 0;
        Ok(())
    }

    /// Returns `None` when a word-expansion failure abandoned the whole
    /// enclosing if command; `Some(true)` when the condition held.
    fn if_condition_matches(
        &mut self,
        condition: &[CommandNode],
    ) -> Result<Option<bool>, ExecuteError> {
        let ast = Ast {
            commands: condition.to_vec(),
        };
        let saved_condition = self.inside_compound_condition.replace(true);
        let result = self.with_errexit_suppressed(|executor| executor.execute_ast(&ast));
        self.inside_compound_condition.set(saved_condition);
        match result {
            Err(ExecuteError::ExpansionFailure(code)) => {
                self.exit_code = code;
                Ok(None)
            }
            other => {
                other?;
                Ok(Some(self.exit_code == 0))
            }
        }
    }

    pub(in crate::executor) fn execute_coproc_command(
        &mut self,
        cmd: &CommandNode,
        coproc_cmd: &crate::parser::CoprocCommand,
    ) -> Result<(), ExecuteError> {
        let array_name = coproc_cmd
            .name
            .clone()
            .unwrap_or_else(|| "COPROC".to_string());
        use std::process::{Command, Stdio};
        let exe = std::env::var_os("CARGO_BIN_EXE_rubash")
            .map(std::path::PathBuf::from)
            .or_else(test_rubash_binary_from_current_exe)
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| "rubash".into());

        let mut child = if let Some(body) = &coproc_cmd.body {
            // Compound command body: coproc [NAME] { body; } or ( body )
            let body_text = bash_command_sequence_text(body);
            let mut child = Command::new(&exe);
            child.arg("-c").arg(&body_text);
            child
        } else if !coproc_cmd.words.is_empty() {
            // Simple command: coproc [NAME] command [args...]
            let mut child = Command::new(&exe);
            if coproc_cmd
                .words
                .first()
                .is_some_and(|word| word.starts_with('-'))
            {
                for word in &coproc_cmd.words {
                    child.arg(word);
                }
            } else {
                child.arg("-c").arg(command_words_source_text(
                    &coproc_cmd.words,
                    &coproc_cmd.word_metadata,
                ));
            }
            child
        } else {
            eprintln!(
                "{}coproc: usage: coproc [NAME] command [args...]",
                self.diagnostic_prefix()
            );
            self.exit_code = 1;
            return Ok(());
        };

        for (key, value) in &self.shell_state.env_vars {
            if !key.starts_with("__RUBASH_") || rubash_spawn_inherited_state(key) {
                child.env(key, value);
            }
        }
        // Preserve the parent script location for diagnostics emitted by the
        // coprocess shell, while keeping internal executor state isolated.
        if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
            child.env("__RUBASH_SCRIPT_NAME", script);
            let line = cmd
                .line
                .map(|line| line.to_string())
                .or_else(|| self.shell_state.env_vars.get("__RUBASH_CURRENT_LINE").cloned())
                .unwrap_or_else(|| "1".to_string());
            child.env("__RUBASH_CURRENT_LINE", line.clone());
            // The child re-parses `-c` source from line 1. Preserve the
            // parent's physical call-site line when assigning child AST lines.
            let line_offset = line.parse::<usize>().unwrap_or(1).saturating_sub(1);
            child.env("__RUBASH_LINE_OFFSET", line_offset.to_string());
        }
        // Coprocess children own a shell-created stdin pipe. Keep builtin
        // readers attached to it and let TERM use native termination when a
        // blocked read cannot consume the shell signal mailbox.
        child.env(INHERIT_PROCESS_STDIN, "1");
        child.env("__RUBASH_COPROC_CHILD", "1");

        // Create pipes for bidirectional communication
        let stdin_result = std::io::pipe();
        let stdout_result = std::io::pipe();

        if let (Ok((stdin_reader, stdin_writer)), Ok((stdout_reader, stdout_writer))) =
            (stdin_result, stdout_result)
        {
            child.stdin(stdin_reader);
            child.stdout(stdout_writer);
            let coproc_stderr_target = self.coproc_stderr_forward_target();
            if coproc_stderr_target.is_some() {
                child.stderr(Stdio::piped());
            } else {
                child.stderr(Stdio::inherit());
            }
            self.apply_coproc_redirects(cmd, &mut child)?;

            match child.spawn() {
                Ok(mut child_proc) => {
                    // Rubash does not expose real coproc file descriptors yet,
                    // but keep the parent ends until after spawn so stdio uses
                    // the correct pipe direction on all hosts.
                    let pid = child_proc.id();
                    if let Some(target) = coproc_stderr_target {
                        if let Some(stderr) = child_proc.stderr.take() {
                            let forwarder =
                                std::thread::spawn(|| forward_coproc_stderr(stderr, target));
                            self.coproc_stderr_forwarders.insert(pid, forwarder);
                        }
                    }
                    self.background_children.insert(pid, crate::fd::SpawnedChild::from(child_proc));
                    let job_id =
                        self.shell_state.job_table
                            .register_process(pid, bash_command_source_text(cmd), true);
                    self.shell_state.job_table.set_job_control(
                        pid,
                        crate::builtins::set::shell_option_enabled(
                            &self.shell_state.env_vars,
                            "monitor",
                        ),
                    );
                    // GNU coproc.c: the coproc is an async child, so $!
                    // (last_made_pid) tracks it like any `&` spawn.
                    self.shell_state.last_background_pid = Some(pid);
                    // GNU sh_openpipe moves the pipe ends to the highest free
                    // fds below 64 (move_to_high_fd with maxfd 64): rpipe
                    // 63/62 and wpipe 61/60, of which the parent keeps 63 and
                    // 60. All three coprocs in coproc.tests reuse that same
                    // pair, so `${COPROC[@]}` is literally "63 60". The parent
                    // ends live in fd_table as real HANDLE endpoints (Rc'd
                    // FileFd), so dup/close/fork share them like GNU's fork.
                    let coproc_write_file = Rc::new(FileFd {
                        handle: crate::fd::into_handle(stdin_writer),
                        path: std::path::PathBuf::from(format!("coproc:{pid}:stdin")),
                    });
                    let coproc_read_file = Rc::new(FileFd {
                        handle: crate::fd::into_handle(stdout_reader),
                        path: std::path::PathBuf::from(format!("coproc:{pid}:stdout")),
                    });
                    let coproc_read_fd = self.allocate_coproc_fd(0);
                    self.fd_table.open_input(
                        coproc_read_fd,
                        FdReadEndpoint::CoprocStdout {
                            pid,
                            fd: coproc_read_file,
                        },
                        true,
                    );
                    let coproc_write_fd = self.allocate_coproc_fd(1);
                    self.fd_table.open_output(
                        coproc_write_fd,
                        FdWriteEndpoint::CoprocStdin {
                            pid,
                            fd: coproc_write_file,
                        },
                        true,
                    );
                    self.shell_state.job_table.attach_coproc_endpoint(job_id, pid);
                    // Store the file descriptors in env for COPROC array
                    let stdin_key = format!("__RUBASH_COPROC_STDIN_{}", pid);
                    let stdout_key = format!("__RUBASH_COPROC_STDOUT_{}", pid);
                    self.shell_state.env_vars.insert(stdin_key, "pipe".to_string());
                    self.shell_state.env_vars.insert(stdout_key, "pipe".to_string());

                    // Windows has no inherited POSIX fd for this pipe. Expose
                    // two shell-owned virtual descriptors instead; the PID
                    // remains job identity, not the observable fd value.
                    //
                    // GNU execute_cmd.c:2380-2445 coproc_bind: find_variable
                    // resolves the name through namerefs first; a nameref
                    // whose target is unset rewrites c_name to the cell
                    // (array and <target>_PID bind on the TARGET while the
                    // nameref stays), an invisible/empty-cell nameref drops
                    // the attribute with a "removing nameref attribute"
                    // warning (find_variable_nameref_for_create,
                    // variables.c:2194-2197), and an existing readonly
                    // variable fails with err_readonly, binding NOTHING —
                    // <name>_PID is bound at :2441-2444 only after the
                    // array bind succeeds.
                    let array_value = format!("({coproc_read_fd} {coproc_write_fd})");
                    let mut bind_name = array_name.clone();
                    let mut pid_name = format!("{array_name}_PID");
                    let mut can_bind = true;
                    // GNU execute_cmd.c:2383 check_identifier: an invalid
                    // coproc name is diagnosed and binds nothing; the coproc
                    // itself still runs and c_name stays recorded for
                    // coproc_unsetvars.
                    let mut c_name = array_name.clone();
                    if !is_shell_name(&array_name) {
                        eprintln!(
                            "{}`{}': not a valid identifier",
                            self.diagnostic_prefix(),
                            array_name
                        );
                        can_bind = false;
                    }
                    if can_bind {
                        match self.nameref_resolution(&array_name) {
                            NamerefResolution::Target(target) => {
                                // GNU execute_cmd.c coproc_bind ->
                                // find_variable_nameref_for_create: the cell must
                                // be a bare identifier — `coproc ref` with ref ->
                                // `XXX[0]` fails sh_invalidid and binds nothing
                                // (nameref18.sub line 51).
                                if parse_array_subscript(&target).is_some() {
                                    eprintln!(
                                        "{}`{target}': not a valid identifier",
                                        self.diagnostic_prefix()
                                    );
                                    can_bind = false;
                                } else {
                                    let target_base =
                                        target.split('[').next().unwrap_or(target.as_str());
                                    let target_exists = self.shell_state.env_vars.contains_key(target_base)
                                        || self.shell_state.variables.get(target_base).is_some();
                                    if target_exists {
                                        // v != 0: ASSIGN_DISALLOWED on the resolved
                                        // var; elements bind to the target but
                                        // <name>_PID still uses the original name.
                                        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, target_base)
                                        {
                                            eprintln!(
                                                "{}{}: readonly variable",
                                                self.diagnostic_prefix(),
                                                array_name
                                            );
                                            can_bind = false;
                                        } else {
                                            bind_name = target_base.to_string();
                                        }
                                    } else {
                                        // v == 0: c_name rewritten to the cell, so
                                        // both the array and <target>_PID bind on
                                        // the target name; the nameref keeps its
                                        // cell untouched.
                                        bind_name = target_base.to_string();
                                        pid_name = format!("{target_base}_PID");
                                        // c_name rewritten to the nameref cell
                                        // (execute_cmd.c:2401-2404).
                                        c_name = target_base.to_string();
                                    }
                                }
                            }
                            NamerefResolution::Unresolved => {
                                // Invisible/empty-cell nameref: GNU drops the
                                // attribute with a warning, then treats it as a
                                // plain variable (readonly check still applies).
                                eprintln!(
                                    "{}warning: {}: removing nameref attribute",
                                    self.diagnostic_prefix(),
                                    array_name
                                );
                                unmark_env_name(&mut self.shell_state.env_vars, NAMEREF_VARS, &array_name);
                                if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &array_name) {
                                    eprintln!(
                                        "{}{}: readonly variable",
                                        self.diagnostic_prefix(),
                                        array_name
                                    );
                                    can_bind = false;
                                }
                            }
                            NamerefResolution::NotNameref => {
                                if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &array_name) {
                                    eprintln!(
                                        "{}{}: readonly variable",
                                        self.diagnostic_prefix(),
                                        array_name
                                    );
                                    can_bind = false;
                                }
                            }
                            // Circular/depth-overflow chains resolve to nothing
                            // (INVALID_NAMEREF_VALUE) — GNU binds nothing.
                            NamerefResolution::Circular | NamerefResolution::MaxDepth => {
                                can_bind = false;
                            }
                        }
                    }
                    // GNU stores c_name regardless of bind success so
                    // coproc_unsetvars can still attempt the unbinds.
                    self.shell_state.coproc_names.insert(pid, c_name.clone());
                    if can_bind {
                        self.shell_state.env_vars.insert(bind_name.clone(), array_value);
                        mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", &bind_name);
                        // bind_variable (execute_cmd.c:2441) is nameref-aware:
                        // `coproc ref` with ref_PID a nameref assigns through
                        // it and hits the resolved target's readonly check.
                        // err_readonly carries no builtin command segment.
                        self.apply_shell_assignment(&pid_name, pid.to_string());
                    }
                    self.exit_code = 0;
                }
                Err(e) => {
                    eprintln!("{}coproc: failed to spawn: {}", self.diagnostic_prefix(), e);
                    self.exit_code = 126;
                }
            }
        } else {
            eprintln!("{}coproc: failed to create pipes", self.diagnostic_prefix());
            self.exit_code = 1;
        }

        Ok(())
    }

    fn coproc_stderr_forward_target(&mut self) -> Option<CoprocStderrForwardTarget> {
        if self.fd_table.is_closed(2) {
            return Some(CoprocStderrForwardTarget::Discard);
        }

        match self.fd_table.write_endpoint(2)? {
            FdWriteEndpoint::Stdout => Some(CoprocStderrForwardTarget::Stdout),
            FdWriteEndpoint::Stderr => Some(CoprocStderrForwardTarget::Stderr),
            FdWriteEndpoint::File(file_fd) => {
                Some(CoprocStderrForwardTarget::File(file_fd.path.clone()))
            }
            FdWriteEndpoint::ProcessSubstitution { path, .. } => {
                Some(CoprocStderrForwardTarget::File(path))
            }
            FdWriteEndpoint::CoprocStdin { fd, .. } => crate::fd::duplicate_handle_inheritable(fd.handle)
                .ok()
                .map(|h| CoprocStderrForwardTarget::CoprocStdin(crate::fd::handle_to_file(h))),
        }
    }

    fn apply_coproc_redirects(
        &self,
        cmd: &CommandNode,
        child: &mut Command,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0 {
                let target = self.expand_redirect_target(redirect);
                if is_closed_redirect_target(&target) {
                    child.stdin(Stdio::null());
                } else if redirect_target_fd(&target).is_none() {
                    child.stdin(Stdio::from(self.open_input_redirect(&target)?));
                }
            }
        }

        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target) {
                child.stdout(Stdio::null());
            } else if redirect_target_fd(&target).is_none() {
                child.stdout(Stdio::from(
                    self.create_redirect_output(&target, redirect.clobber)?,
                ));
            }
        } else if let Some(redirect) = &cmd.append {
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target) {
                child.stdout(Stdio::null());
            } else if redirect_target_fd(&target).is_none() {
                child.stdout(Stdio::from(
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?,
                ));
            }
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target) {
                child.stderr(Stdio::null());
            } else if redirect_target_fd(&target).is_none() {
                child.stderr(Stdio::from(
                    self.create_redirect_output(&target, redirect.clobber)?,
                ));
            }
        } else if let Some(redirect) = &cmd.redirect_err_append {
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target) {
                child.stderr(Stdio::null());
            } else if redirect_target_fd(&target).is_none() {
                child.stderr(Stdio::from(
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?,
                ));
            }
        }

        Ok(())
    }

    pub(in crate::executor) fn execute_case_command(
        &mut self,
        case_command: &CaseCommand,
    ) -> Result<(), ExecuteError> {
        // TODO(parse.y/execute_cmd.c/pathexp.c): Bash case execution uses the
        // full pattern matcher, fall-through operators, expansion flags, and
        // compound-list control flow. This handles the common shell glob
        // operators used by simple `case` clauses.
        // GNU execute_cmd.c:3679-3680 traces the case head (with the raw
        // unexpanded word, print_cmd.c:742) before pattern expansion.
        if self.xtrace_enabled() {
            let prefix = self.xtrace_prefix();
            self.xtrace_write(format!("{prefix}case {} in\n", case_command.word).as_bytes());
        }
        // GNU execute_cmd.c:3660-3668: the case head is printed
        // (print_case_command_head, print_cmd.c:731 -> `case WORD in `) and
        // run_debug_trap fires before the word is expanded, so the trap sees
        // the raw unexpanded word — `case "$v" in ` keeps the quotes.
        if self.debug_trap_in_scope() {
            let raw_word = if case_command.word_metadata.raw.is_empty() {
                case_command.word.as_str()
            } else {
                case_command.word_metadata.raw.as_str()
            };
            let _ = self.run_debug_trap(&format!("case {raw_word} in "))?;
        }
        let word = self.expand_case_word(&case_command.word);
        let word = tilde_expand::strip_assignment_quote_marker(&word);
        self.abandon_on_arithmetic_expansion_error()?;
        let nocasematch = crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "nocasematch");
        let mut fall_through = false;
        let mut matched_any = false;
        let mut index = 0;
        while let Some(clause) = case_command.clauses.get(index) {
            let matched = fall_through
                || clause.pattern_nodes.iter().any(|pattern| {
                    let stripped = self.expand_case_pattern(pattern);
                    if self.shell_state.arithmetic_fatal_error.get() || self.shell_state.arithmetic_nounset_error.get() {
                        return false;
                    }
                    if case_pattern_has_extglob(&stripped) {
                        if nocasematch {
                            crate::executor::conditional::extglob_case_pattern_matches_nocase(
                                &stripped, &word,
                            )
                        } else {
                            crate::executor::conditional::extglob_case_pattern_matches(
                                &stripped, &word,
                            )
                        }
                    } else if nocasematch {
                        case_pattern_matches_nocase(&stripped, &word)
                    } else {
                        case_pattern_matches(&stripped, &word)
                    }
                });
            self.abandon_on_arithmetic_expansion_error()?;
            if matched {
                matched_any = true;
                if clause.body.is_empty() {
                    self.exit_code = 0;
                } else {
                    let body = Ast {
                        commands: clause.body.clone(),
                    };
                    self.execute_ast(&body)?;
                }
                match clause.terminator {
                    CaseTerminator::Break => return Ok(()),
                    CaseTerminator::FallThrough => {
                        fall_through = true;
                    }
                    CaseTerminator::TestNext => {
                        fall_through = false;
                    }
                }
            }
            index += 1;
        }

        if !matched_any {
            self.exit_code = 0;
        }
        Ok(())
    }

    fn expand_case_pattern(&mut self, pattern: &crate::parser::CasePattern) -> String {
        const PROTECTED_CASE_PATTERN_BACKSLASH: char = crate::executor::markers::CASE_PATTERN_BACKSLASH_GUARD;

        if !case_pattern_raw_has_quotes(&pattern.raw_text) {
            // Backslashes in a case pattern are escape characters, not quote
            // removal subjects (bash execute_cmd.c: patterns do not undergo
            // quote removal). Protect every `\` (and the legacy `\x18` marker)
            // through expansion and decode_parameter_pattern_quotes, then
            // restore the real backslash so case_pattern_matches can apply its
            // escape semantics (`\]` is a bracket member, `\"` matches `"`).
            let protected = pattern.text.replace(
                |c| c == '\\' || c == crate::executor::markers::DATA_DQUOTE,
                &PROTECTED_CASE_PATTERN_BACKSLASH.to_string(),
            );
            // Use the mutable expander: case pattern expansion is not an
            // isolated sub-expression — `$((x=1))` in a pattern must keep its
            // assignment side effects (bash execute_cmd.c evaluates patterns
            // with the current variable state; case.tests `;&` fall-through).
            let expanded = self.expand_word_mut(&protected);
            let decoded = decode_parameter_pattern_quotes(&expanded);
            let restored = decoded.replace(PROTECTED_CASE_PATTERN_BACKSLASH, "\\");
            return strip_surrounding_quotes(&restored);
        }

        quote_aware_case_pattern(&pattern.raw_text, |word| self.expand_word_mut(word))
    }

    pub(in crate::executor) fn execute_case_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        case_command: &CaseCommand,
    ) -> Result<(), ExecuteError> {
        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut case_command = case_command.clone();
        let result = self.apply_case_command_redirects(&redirect_cmd, &mut case_command);
        let status = self.exit_code;
        if let Err(error) = result {
            let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
            self.exit_code = status;
            finish_result?;
            return Err(error);
        }
        // GNU execute_cmd.c:3653 `line_number = case_command->line`: the
        // `case` keyword's line is the ambient while a clause body runs.
        let case_line = cmd.line;
        let result = self.with_command_input_redirects(cmd, |executor| {
            executor.with_ambient_line(case_line, |executor| {
                executor.execute_case_command(&case_command)
            })
        });
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    fn apply_case_command_redirects(
        &mut self,
        cmd: &CommandNode,
        case_command: &mut CaseCommand,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_redirect_target(redirect);
            if redirect_target_fd(&target).is_none() {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let mut append_redirect = redirect.clone();
            append_redirect.target = target;
            append_redirect.append = true;
            append_redirect.clobber = false;
            for clause in &mut case_command.clauses {
                apply_stdout_append_redirect(&mut clause.body, &append_redirect);
            }
        } else if let Some(redirect) = &cmd.append {
            let mut append_redirect = redirect.clone();
            append_redirect.target = self.expand_redirect_target(redirect);
            for clause in &mut case_command.clauses {
                apply_stdout_append_redirect(&mut clause.body, &append_redirect);
            }
        }

        if let Some(redirect) = &cmd.redirect_err {
            let target = self.expand_redirect_target(redirect);
            if redirect_target_fd(&target).is_none() && !is_null_device(&target) {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            let mut append_redirect = redirect.clone();
            append_redirect.target = target;
            append_redirect.append = true;
            append_redirect.clobber = false;
            for clause in &mut case_command.clauses {
                apply_stderr_append_redirect(&mut clause.body, &append_redirect);
            }
        } else if let Some(redirect) = &cmd.redirect_err_append {
            let mut append_redirect = redirect.clone();
            append_redirect.target = self.expand_redirect_target(redirect);
            for clause in &mut case_command.clauses {
                apply_stderr_append_redirect(&mut clause.body, &append_redirect);
            }
        }

        Ok(())
    }
}

pub(in crate::executor) fn command_is_time_prefixed_compound(cmd: &CommandNode) -> bool {
    cmd.words.first().map(String::as_str) == Some("time")
        && (cmd.for_command.is_some()
            || cmd.if_command.is_some()
            || cmd.loop_command.is_some()
            || cmd.select_command.is_some()
            || cmd.case_command.is_some()
            || cmd.coproc_command.is_some()
            || cmd.subshell_command.is_some()
            || cmd.brace_group.is_some())
}

fn apply_if_redirects(
    executor: &mut Executor,
    cmd: &CommandNode,
    if_command: &mut IfCommand,
) -> Result<(), ExecuteError> {
    apply_redirects_to_commands(executor, cmd, &mut if_command.condition)?;
    apply_redirects_to_commands(executor, cmd, &mut if_command.then_body)?;
    for branch in &mut if_command.elif_branches {
        apply_redirects_to_commands(executor, cmd, &mut branch.condition)?;
        apply_redirects_to_commands(executor, cmd, &mut branch.body)?;
    }
    if let Some(body) = &mut if_command.else_body {
        apply_redirects_to_commands(executor, cmd, body)?;
    }
    Ok(())
}

fn apply_redirects_to_commands(
    executor: &mut Executor,
    cmd: &CommandNode,
    commands: &mut Vec<CommandNode>,
) -> Result<(), ExecuteError> {
    let mut ast = Ast {
        commands: std::mem::take(commands),
    };
    executor.apply_command_output_redirects(cmd, &mut ast)?;
    *commands = ast.commands;
    Ok(())
}

fn flatten_if_command_for_alias_scan(
    cmd: &CommandNode,
    if_command: &IfCommand,
) -> Vec<CommandNode> {
    let mut commands = Vec::new();
    push_if_condition(&mut commands, "if", &if_command.condition);
    commands.push(command_with_words(["then"]));
    commands.extend(if_command.then_body.clone());
    for branch in &if_command.elif_branches {
        push_if_condition(&mut commands, "elif", &branch.condition);
        commands.push(command_with_words(["then"]));
        commands.extend(branch.body.clone());
    }
    if let Some(body) = &if_command.else_body {
        commands.push(command_with_words(["else"]));
        commands.extend(body.clone());
    }
    let mut fi = command_with_words(["fi"]);
    fi.redirect_in = cmd.redirect_in.clone();
    fi.redirect_out = cmd.redirect_out.clone();
    fi.append = cmd.append.clone();
    fi.redirect_err = cmd.redirect_err.clone();
    fi.redirect_err_append = cmd.redirect_err_append.clone();
    fi.heredoc = cmd.heredoc.clone();
    fi.heredoc_body = cmd.heredoc_body.clone();
    fi.heredoc_delimiter = cmd.heredoc_delimiter.clone();
    fi.heredoc_redirects = cmd.heredoc_redirects.clone();
    fi.here_string = cmd.here_string.clone();
    fi.here_string_carrier = cmd.here_string_carrier.clone();
    commands.push(fi);
    commands
}

fn push_if_condition(commands: &mut Vec<CommandNode>, keyword: &str, condition: &[CommandNode]) {
    let Some((first, rest)) = condition.split_first() else {
        commands.push(command_with_words([keyword]));
        return;
    };

    let mut first = first.clone();
    first.words.insert(0, keyword.to_string());
    commands.push(first);
    commands.extend(rest.iter().cloned());
}

fn command_with_words<const N: usize>(words: [&str; N]) -> CommandNode {
    let mut command = CommandNode::new();
    command.words = words.iter().map(|word| (*word).to_string()).collect();
    command
}

struct TimePrefixParts {
    command_index: usize,
    inverted: bool,
    posix_format: bool,
}

fn time_prefix_parts(words: &[String]) -> Option<TimePrefixParts> {
    if words.first().map(String::as_str) != Some("time") {
        return None;
    }

    let mut index = 1;
    let mut inverted = false;
    let mut posix_format = false;
    while let Some(word) = words.get(index).map(String::as_str) {
        match word {
            "-p" => {
                posix_format = true;
                index += 1;
            }
            "--" => index += 1,
            "!" => {
                inverted = !inverted;
                index += 1;
            }
            _ => break,
        }
    }
    Some(TimePrefixParts {
        command_index: index,
        inverted,
        posix_format,
    })
}

fn test_rubash_binary_from_current_exe() -> Option<std::path::PathBuf> {
    let current = std::env::current_exe().ok()?;
    let deps = current.parent()?;
    if deps.file_name().and_then(|name| name.to_str()) != Some("deps") {
        return None;
    }
    let debug_dir = deps.parent()?;
    let binary_name = if cfg!(windows) {
        "rubash.exe"
    } else {
        "rubash"
    };
    let candidate = debug_dir.join(binary_name);
    candidate.is_file().then_some(candidate)
}

fn case_pattern_has_extglob(pattern: &str) -> bool {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index + 1 < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if matches!(chars[index], '@' | '*' | '+' | '?' | '!') && chars[index + 1] == '(' {
            return true;
        }
        index += 1;
    }
    false
}

fn case_pattern_raw_has_quotes(raw: &str) -> bool {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        match chars[index] {
            '\'' | '"' => return true,
            '$' if matches!(chars.get(index + 1), Some('\'' | '"')) => return true,
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    false
}

fn quote_aware_case_pattern(raw: &str, mut expand_word: impl FnMut(&str) -> String) -> String {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')) {
            let quote = chars[index + 1];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 2, quote) {
                let body = chars[index + 2..end].iter().collect::<String>();
                let literal = if quote == '\'' {
                    decode_ansi_c_escapes(&body)
                } else {
                    expand_word(&body)
                };
                output.push_str(&escape_case_pattern_literal(&literal));
                index = end + 1;
                continue;
            }
        }

        if matches!(chars[index], '\'' | '"') {
            let quote = chars[index];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 1, quote) {
                let body = chars[index + 1..end].iter().collect::<String>();
                let literal = if quote == '\'' {
                    body
                } else {
                    expand_word(&body)
                };
                output.push_str(&escape_case_pattern_literal(&literal));
                index = end + 1;
                continue;
            }
        }

        let start = index;
        while index < chars.len()
            && chars[index] != '\''
            && chars[index] != '"'
            && !(chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')))
        {
            // GNU read_token_word: quoting inside a substitution belongs to
            // the substitution's own input, not to the pattern — `$( echo
            // "$bar")` must not split the segment at the inner `"`, or the
            // `$(` looks unclosed and the expansion reports EOF
            // (comsub-posix6.sub case-pattern substitution).
            if chars[index] == '$' && chars.get(index + 1) == Some(&'(') {
                // Returns the index just past the closing `)`.
                if let Some(close) =
                    crate::lexer::skip_parenthesized_unit_corrected(&chars, index + 1)
                {
                    index = close.min(chars.len());
                    continue;
                }
            }
            if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
                if let Some(close) = skip_braced_case_pattern_unit(&chars, index + 1) {
                    index = close + 1;
                    continue;
                }
            }
            if chars[index] == '`' {
                index += 1;
                while index < chars.len() && chars[index] != '`' {
                    if chars[index] == '\\' && index + 1 < chars.len() {
                        index += 1;
                    }
                    index += 1;
                }
                if index < chars.len() {
                    index += 1;
                }
                continue;
            }
            if chars[index] == '\\' && index + 1 < chars.len() {
                index += 2;
            } else {
                index += 1;
            }
        }
        let segment = chars[start..index].iter().collect::<String>();
        let expanded = expand_word(&segment);
        output.push_str(&strip_surrounding_quotes(&decode_parameter_pattern_quotes(
            &expanded,
        )));
    }

    output
}

/// Balanced `${...}` skip for the case-pattern segment scanner — braces,
/// quotes, and escapes inside the expansion are its own (GNU
/// parse_matched_pair). Returns the index of the closing `}`.
fn skip_braced_case_pattern_unit(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;
    let mut single = false;
    let mut double = false;
    while index < chars.len() {
        match chars[index] {
            '\\' if !single => {
                index += 2;
                continue;
            }
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '{' if !single && !double => depth += 1,
            '}' if !single && !double => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn quoted_case_pattern_end(chars: &[char], start: usize, quote: char) -> Option<usize> {
    let mut index = start;
    let mut escaped = false;
    while index < chars.len() {
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if quote == '"' && chars[index] == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if chars[index] == quote {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn escape_case_pattern_literal(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        if matches!(ch, '*' | '?' | '[' | '\\' | '@' | '!' | '+' | '(') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}
