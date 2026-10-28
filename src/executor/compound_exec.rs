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
pub(in crate::executor) fn rubash_spawn_inherited_state(key: &str) -> bool {
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
        "__RUBASH_UNSTREAMED_FUNCTION_BODIES",
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
        // A GNU fork keeps the variable context stack, so a local that
        // shadows an exported outer binding keeps the shadowed name's
        // child-env contribution — the LOCAL_EXPORT_ENV stash is that
        // model's transport across the respawn (rubash#970).
        "__RUBASH_LOCAL_EXPORT_ENV",
        "__RUBASH_ULIMIT_C",
        "__RUBASH_ULIMIT_F",
        "__RUBASH_ULIMIT_N",
        "__RUBASH_UMASK",
        "__RUBASH_SECONDS_OFFSET",
        "__RUBASH_SHELL_START_EPOCH",
        "__RUBASH_TRAP_ORIG_IGN",
        "__RUBASH_POSIX_MODE",
        "__RUBASH_ZSH_OPTIONS",
        "__RUBASH_PHYSICAL_PWD",
        "__RUBASH_SHOPT_STATE",
        "__RUBASH_SHOPT_CHECKHASH",
        "__RUBASH_COMPATIBLE_SHELL_PATH",
        "__RUBASH_SHELL_ROOT",
        "__RUBASH_TEMP_PATH",
    ];
    key.starts_with("__RUBASH_SETOPT_") || KEYS.contains(&key)
}

/// GNU jobs.c:2455/2460 pipeline_pgrp at fork time, recorded as j->pgrp
/// (jobs.c:649) and substituted by `jobs -x` (builtins/jobs.def
/// execute_list_with_replacements `itos (j->pgrp)`, rubash#1096): with job
/// control the first pipeline pid becomes the group; without job control
/// every job shares the shell's own pgrp. Rubash never calls setpgid, but
/// the RECORDED value is what `jobs -x` prints, and without job control
/// GNU's value is exactly the live shell pgrp — so read getpgid(0) here.
/// Non-unix has no pgrp concept; the job pid keeps today's observable.
fn async_job_pgrp(first_pid: u32, job_control: bool) -> u32 {
    if !job_control {
        #[cfg(unix)]
        {
            // SAFETY: getpgid(0) only queries our own process group.
            let pgid = unsafe { libc::getpgid(0) };
            if pgid > 0 {
                return pgid as u32;
            }
        }
    }
    first_pid
}

/// rubash#1095: whether the wrapper spawn may absorb the body's outermost
/// `( list )` — serialize the inner list because the spawn itself is GNU's
/// single async/subshell fork (execute_cmd.c:701). The parens must carry
/// nothing the inner list cannot: no assignments (they apply inside the
/// child) and no surviving outer redirects or heredoc payloads (consumed
/// stdio rides the spawn handles; anything left behind would be dropped
/// with the parens). `! (...)` and heredoc-carrying parens keep the old
/// shape (rare corners, behavior unchanged).
fn background_paren_absorbable(command: &CommandNode, consumed: &[bool]) -> bool {
    if command.subshell_command.is_none() || !command.assignments.is_empty() {
        return false;
    }
    // Heredoc payloads live beside `redirects` (legacy mirrors plus
    // heredoc_redirects); never drop the parens under those.
    if command.heredoc.is_some()
        || command.heredoc_body.is_some()
        || command.here_string.is_some()
        || command.here_string_carrier.is_some()
        || !command.heredoc_redirects.is_empty()
    {
        return false;
    }
    for (index, redirect) in command.redirects.iter().enumerate() {
        if matches!(
            redirect.kind,
            crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString
        ) {
            return false;
        }
        if !consumed.get(index).copied().unwrap_or(false) {
            return false;
        }
    }
    true
}

/// niubash#362: true when a respawned async `&` child's `-c`
/// payload is too large to ride the OS command line / a single argv entry.
///
/// Windows caps the whole `CreateProcessW` lpCommandLine at 32767 UTF-16
/// units, so measure the exact encoded length (program + `-c` + script).
/// Unix caps one execve argument at MAX_ARG_STRLEN (32 pages * 4096 =
/// 131072 bytes), so compare the raw byte length. Either way the caller
/// switches to the temp-file transport (__RUBASH_CMD_FILE).
#[cfg(windows)]
pub(in crate::executor) fn respawn_source_needs_file(exe: &std::path::Path, source: &str) -> bool {
    const CMDLINE_LIMIT: usize = 32_767;
    // Headroom for the exe path, the "-c " separator and quoting growth.
    const HEADROOM: usize = 256;
    let args = ["-c".to_string(), source.to_string()];
    crate::fd::encoded_command_line_len(exe, &args) + HEADROOM > CMDLINE_LIMIT
}

#[cfg(unix)]
pub(in crate::executor) fn respawn_source_needs_file(_exe: &std::path::Path, source: &str) -> bool {
    const ARG_STRLEN_LIMIT: usize = 131_072;
    const HEADROOM: usize = 256;
    source.len() + HEADROOM > ARG_STRLEN_LIMIT
}

#[cfg(not(any(unix, windows)))]
pub(in crate::executor) fn respawn_source_needs_file(
    _exe: &std::path::Path,
    _source: &str,
) -> bool {
    false
}

/// Pipe + forwarder targets (rubash#1882): only endpoints that could
/// not be handed to the child as a descriptor — the process
/// substitution carrier path, plus the dup-failure fallbacks for the
/// process-stdout and file endpoints.
enum CoprocStderrForwardTarget {
    Stdout,
    File(PathBuf),
}

/// How a coproc child's fd 2 is provided at spawn (rubash#1882).
enum CoprocStderrPlan {
    /// The child holds the resolved endpoint itself (GNU fork model):
    /// its diagnostics survive this shell's exit.
    Direct(std::process::Stdio),
    /// Pipe + forwarder transport, kept for endpoints with no portable
    /// child handle (process-substitution carrier) and dup failures.
    Forward(CoprocStderrForwardTarget),
}

/// Duplicate one of this process's std descriptors (0/1/2) into an
/// inheritable file a spawned child can hold as its own stdio.
fn dup_process_std_handle(fd: u32) -> Option<std::fs::File> {
    let handle = crate::fd::process_std_handle(fd);
    crate::fd::duplicate_handle_inheritable(handle)
        .ok()
        .map(crate::fd::handle_to_file)
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
            redirect.fd.is_none() && matches!(redirect.kind, crate::parser::RedirectKind::HereDoc)
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
    // Forwarding into a closed stdout/stderr pipe must end this thread
    // with an io error, not kill the shell (SIGPIPE is SIG_DFL
    // process-wide since rubash#1333).
    #[cfg(unix)]
    crate::builtins::kill::block_sigpipe_on_this_thread();
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
            CoprocStderrForwardTarget::File(path) => {
                let mut file = open_redirect_append(path)?;
                file.write_all(&buffer[..count])?;
            }
        }
    }
}

/// rubash#1611: true when a timed command's inner is itself a time
/// frame through a "pure" chain — directly (`time time ...`) or
/// through sole-command subshells (`time (time ...)`, recursively).
/// GNU
/// prints exactly ONE report for a pure chain — the innermost
/// frame's — for completions and unwinds alike (verified against GNU
/// bash 5.3.20: `time time false`, `time time exit 3`,
/// `time (time false)`, `time ((time false))` all print one report;
/// a brace group, a function call, or a subshell with further
/// commands breaks the chain and every frame prints). The outer
/// frames therefore delegate: they still adjudicate status, traps,
/// and errexit, but emit no report of their own.
fn time_inner_is_pure_time_chain(inner: &CommandNode) -> bool {
    if inner.time_command.is_some() {
        return true;
    }
    // A pipeline inner never delegates: the pipeline frame owns the
    // single report (a stage's own time report is hoisted into it,
    // rubash#1263) — GNU `time time false | cat` prints one report,
    // from the outer frame, and `time (time false) | cat` prints two.
    if let Some(subshell) = &inner.subshell_command {
        return subshell.body.len() == 1 && time_inner_is_pure_time_chain(&subshell.body[0]);
    }
    inner.words.first().map(String::as_str) == Some("time")
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
                Some(FdReadEndpoint::CoprocStdout { pid: p, fd }) if *p == pid => Some(fd.clone()),
                _ => None,
            })
    }

    pub(crate) fn coproc_write_file(&self, pid: u32) -> Option<Rc<FileFd>> {
        self.fd_table
            .entries
            .values()
            .find_map(|entry| match &entry.write {
                Some(FdWriteEndpoint::CoprocStdin { pid: p, fd }) if *p == pid => Some(fd.clone()),
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

    fn allocate_coproc_fds(&mut self) -> (u32, u32) {
        // GNU sh_openpipe (execute_cmd.c/coproc) moves ALL FOUR pipe ends,
        // in rpipe[0], rpipe[1], wpipe[0], wpipe[1] order, to the highest
        // free fds below 64 (move_to_high_fd, general.c, maxfd 64). The
        // parent keeps rpipe[0] as its read end and wpipe[1] as its write
        // end; its copies of rpipe[1]/wpipe[0] are closed after the fork,
        // but they still consume high slots during allocation. So with an
        // empty table the pair is (63, 60); a second live coproc gets
        // (62, 58), a third (61, 56): the scan must step over the slots
        // the child ends of earlier coprocs consumed, not just the two
        // parent ends. Reserving all four slots against the current
        // fd_table reproduces that numbering exactly.
        let mut reserved: Vec<u32> = Vec::with_capacity(4);
        let mut candidate = 63u32;
        while reserved.len() < 4 && candidate >= 10 {
            let free = !reserved.contains(&candidate)
                && self.fd_table.entries.get(&candidate).map_or(true, |e| {
                    e.closed || (e.read.is_none() && e.write.is_none())
                });
            if free {
                reserved.push(candidate);
            }
            candidate -= 1;
        }
        if reserved.len() == 4 {
            (reserved[0], reserved[3])
        } else {
            // High band exhausted: fall back to the limit-aware low-first
            // allocator (bounded by RLIMIT_NOFILE, rubash#773). GNU's
            // move_to_high_fd would keep scanning below 10; the fds there
            // are the shell's own stdio, so this only diverges when more
            // than ~13 coprocs/dynamic fds are already live.
            let read = self.allocate_dynamic_fd().unwrap_or(63);
            let write = self.allocate_dynamic_fd().unwrap_or(60);
            (read, write)
        }
    }

    pub(in crate::executor) fn execute_inverted_ast_command(
        &mut self,
        inverted_command: &InvertedCommand,
    ) -> Result<(), ExecuteError> {
        let commands = Ast {
            commands: vec![(*inverted_command.command).clone()],
        };
        self.with_errexit_suppressed(|executor| executor.execute_ast(&commands))?;
        // GNU execute_cmd.c:854-883 vs :1204: a compound's OWN redirect
        // failure returns before the invert step, so the flip is suppressed
        // while compound_redirect_failed is set (rubash#846). The mark is
        // left set on purpose: an outer `! ! C` wrapper suppresses in turn
        // (GNU: the inner command never runs — the outer redirect fails
        // first — so both levels observe status 1), and the owning
        // boundary consumes it (subshell-region single-inverted-body flip
        // at :1831, pipeline wait-side flip at :763, comsub/function/eval
        // completion discard, flat-loop backstop clear).
        if self.compound_redirect_failed.get() {
            return Ok(());
        }
        self.exit_code = invert_exit_status(self.exit_code);
        Ok(())
    }

    /// Record a compound's own do_redirections failure (GNU
    /// execute_cmd.c:854-883). Call only with the apply-stage IoError —
    /// never ExpansionFailure (failglob aborts before do_redirections and
    /// flips normally) and never a body write error.
    pub(in crate::executor) fn note_compound_redirect_failure(&self) {
        self.compound_redirect_failed.set(true);
    }

    /// Consume the compound-redirect-failure mark (fork/call boundaries and
    /// the pipeline wait-side flip own the adjudicated status from here).
    pub(in crate::executor) fn take_compound_redirect_failed(&self) -> bool {
        self.compound_redirect_failed.replace(false)
    }

    /// rubash#1256: GNU prints the async start notice `[<job>] <pid>`
    /// when a background job launches in an INTERACTIVE shell
    /// (execute_cmd.c async launch -> jobs.c pretty_print_job; verified
    /// against GNU 5.3.20: printed with `monitor` on or off, and never
    /// in a non-interactive shell — not even with `set -m`). The
    /// termination notices already run at the reader boundary
    /// (job_builtins notify path); the start notice was the missing half.
    pub(in crate::executor) fn notify_async_job_started(
        &mut self,
        job_id: crate::jobs::JobId,
        pid: u32,
    ) {
        if !self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_INTERACTIVE")
        {
            return;
        }
        let _ = self.write_default_stderr(format!("[{job_id}] {pid}\n").as_bytes());
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
                let _ =
                    self.run_debug_trap(crate::debugger::SeamKind::BackgroundLaunch, &inner_text)?;
            }
        }
        let display_source = bash_command_source_text(&background_command.command);
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
        let (stdio, consumed, captured_pipe_writer) =
            self.resolve_background_stdio(&background_command.command);
        // rubash#809: GNU executes a background SIMPLE command as fork+exec —
        // execute_connection '&' (execute_cmd.c:5562+) forks, and the async
        // child's execute_simple_command ends in shell_execve for an external,
        // so no second shell process survives. The `rubash -c` wrapper respawn
        // below pays one extra process spawn plus a full shell init for every
        // `&` job (Windows cell: bgwait 750x `sleep &` measured ~2.2ms/job
        // with the wrapper against ~0.05ms for a direct external spawn; the
        // Android #809 2.47x gap has the same shape — wrapper respawn vs fork
        // floor). When the background body is a bare external — no compound
        // constructs, no assignments or redirects, no alias/function/builtin
        // resolution, and not a script needing the shell fallback — spawn the
        // external directly on the same async stdio (resolve_background_stdio
        // rules apply unchanged). $! then names the external's pid (GNU's
        // value), and the job table / wait / kill paths see the same
        // registration shape they already handle for wrappers.
        //
        // The gate runs BEFORE any wrapper-only setup (exe resolution, env
        // block, handle duplication): a direct spawn pays none of it.
        let direct_plan = if !self.xtrace_enabled() && !captured_pipe_writer {
            self.background_direct_external_command(
                &background_command.command,
                !consumed.is_empty() && consumed.iter().all(|entry| *entry),
            )
        } else {
            None
        };
        if let Some((program, argv)) = direct_plan {
            let mut process = std::process::Command::new(&program);
            // argv[0] is the child's image name; the words after the
            // command name are the real arguments.
            process.args(&argv[1..]);
            let [stdin_stdio, stdout_stdio, stderr_stdio] = stdio.map(|resolved| match resolved {
                BackgroundStdio::Inherit => std::process::Stdio::inherit(),
                BackgroundStdio::Null => std::process::Stdio::null(),
                BackgroundStdio::File(file) => std::process::Stdio::from(file),
            });
            process
                .stdin(stdin_stdio)
                .stdout(stdout_stdio)
                .stderr(stderr_stdio);
            // Same environment application as every foreground external
            // (exported shell vars; no assignments exist at this gate).
            self.apply_external_environment(&background_command.command, &mut process, &program);
            // rubash#229/#737 unix arm: reset inherited signal dispositions
            // in the child exactly like the foreground spawn path. The #737
            // pdeathsig arm keys off __RUBASH_BACKGROUND_WRAPPER, which is
            // never set here — a directly-exec'd async child survives shell
            // exit, as GNU's does.
            // rubash#1035: GNU execute_cmd.c setup_async_signals ignores
            // SIGINT/SIGQUIT in the async child while job_control is off
            // (the `monitor` option is unset) — a background `sleep`
            // survives `kill -INT $!`. The async pre_exec variant forces
            // that ignore after the entry-disposition restore; with job
            // control on the plain reset applies, as in GNU.
            #[cfg(unix)]
            unsafe {
                use std::os::unix::process::CommandExt;
                if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor")
                {
                    process.pre_exec(crate::builtins::kill::child_pre_exec_reset);
                } else {
                    process.pre_exec(crate::builtins::kill::child_pre_exec_reset_async);
                }
            }
            process.current_dir(&self.shell_state.cwd);
            suppress_orphan_console(&mut process);
            match process.spawn() {
                Ok(child) => {
                    let pid = child.id();
                    adopt_orphan_console(pid);
                    self.background_children
                        .insert(pid, crate::fd::SpawnedChild::from(child));
                    let job_control = crate::builtins::set::shell_option_enabled(
                        &self.shell_state.env_vars,
                        "monitor",
                    );
                    let job_id = self.shell_state.job_table.register_process_with_stages(
                        pid,
                        display_source.clone(),
                        true,
                        async_job_pgrp(pid, job_control),
                        bash_job_stage_commands(&background_command.command),
                    );
                    self.shell_state.job_table.set_job_control(pid, job_control);
                    self.shell_state.last_background_pid = Some(pid);
                    self.notify_async_job_started(job_id, pid);
                    // GNU expands the async simple command's words in the
                    // exec'd child; no substitution status may leak into the
                    // parent's bands (the next command must see the state it
                    // had before).
                    self.last_command_substitution_status.set(None);
                    self.exit_code = 0;
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            }
        }
        let exe = std::env::var_os("CARGO_BIN_EXE_rubash")
            .map(std::path::PathBuf::from)
            .or_else(test_rubash_binary_from_current_exe)
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| "rubash".into());
        // Environment block for the child: process env merged with the
        // shell's env_vars (same filter the old Command::env loop applied).
        let mut env_map: std::collections::BTreeMap<String, String> = std::env::vars().collect();
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
        // Strip by the KEYS PRESENT IN env_map, not the live trap table:
        // stale OS-env copies of removed traps (the std::env mirror writes
        // through setattr apply.rs but is not cleaned on trap removal) ride
        // into env_map via std::env::vars() above, and a strip driven by
        // env_vars would leave them (trap.tests: __RUBASH_TRAP_SIGHUP/
        // __RUBASH_TRAP_SIGUSR2 leaked into the monitored `sleep N &`
        // children after `trap - HUP`/`trap - USR2` had cleared the table).
        env_map.retain(|key, _| {
            !(key.starts_with("__RUBASH_TRAP") && key != "__RUBASH_TRAP_ORIG_IGN")
        });
        // rubash#1630: dropping the table outright also dropped its
        // VISIBILITY — GNU's forked child lists the inherited traps in
        // reset form (`trap -p` inside `( ... ) &` prints the parent's
        // USR1 and EXIT entries) while the dispositions stay reset, so
        // nothing fires and the inherited EXIT trap in particular never
        // runs in the child. The in-process subshell path gets exactly
        // that shape from reset_for_subshell_child applied to the live
        // table; run the same transform on a clone of the live table
        // and forward the resulting __RUBASH_TRAP* keys, so the
        // respawned child seeds the identical reset-but-listed table
        // (including the save/restore idiom `eval "$(trap -p ...)"`).
        // The retain above ran first, so only the live table's entries
        // can land here — no stale OS-env copies.
        let mut child_trap_table = self.shell_state.env_vars.clone();
        crate::builtins::trap::reset_for_subshell_child(&mut child_trap_table);
        for (key, value) in &child_trap_table {
            if key.starts_with("__RUBASH_TRAP") {
                env_map.insert(key.clone(), value.clone());
            }
        }
        // rubash#1035: GNU execute_cmd.c setup_async_signals ignores
        // SIGINT/SIGQUIT in the forked async child while job_control is off
        // (the `monitor` option is unset). A respawned `rubash -c` child
        // cannot inherit that disposition through spawn (the parent holds
        // no ignore itself), so stamp the async marker the child's startup
        // consumes (main.rs -> kill::apply_async_child_entry_ignores)
        // before its trap table seeds. With job control on the marker is
        // removed instead — including a user-forged ambient copy — so
        // monitored children keep the entry dispositions, as in GNU.
        // An exec-like body (a bare simple command GNU would exec: an
        // external or a script file) additionally inherits the ignore as
        // SIG_HARD_IGNORE — merge SIGINT/SIGQUIT into the forwarded
        // ORIG_IGN set so the child's `trap ... INT` is silently refused
        // and `trap -p INT` shows `trap -- '' SIGINT` (trap.c set_signal).
        // Compound bodies stay trappable: GNU runs those in-process in the
        // forked child (trap.c's ASYNCSIG special-case in set_signal), so
        // no ORIG_IGN merge — only the kernel ignore, which a later `trap`
        // overrides through the normal reconcile.
        #[cfg(unix)]
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor") {
            env_map.remove(crate::builtins::kill::ASYNC_CHILD_ENV_KEY);
        } else {
            env_map.insert(
                crate::builtins::kill::ASYNC_CHILD_ENV_KEY.to_string(),
                "1".to_string(),
            );
            if self.background_body_execs_in_child(&background_command.command) {
                let mut ignores: Vec<String> = env_map
                    .get(crate::builtins::trap::TRAP_ORIG_IGNORES)
                    .map(|value| {
                        value
                            .split(':')
                            .filter(|name| !name.is_empty())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                for signal in ["SIGINT", "SIGQUIT"] {
                    if !ignores.iter().any(|ignored| ignored == signal) {
                        ignores.push(signal.to_string());
                    }
                }
                ignores.sort();
                env_map.insert(
                    crate::builtins::trap::TRAP_ORIG_IGNORES.to_string(),
                    ignores.join(":"),
                );
            }
        }
        env_map.insert("__RUBASH_SHELL_PID".to_string(), self.shell_pid.to_string());
        // rubash#1074: the wrapper child is one subshell level deeper than
        // this shell (execute_in_subshell's subshell_level++); carry the
        // target depth so the child's BASH_SUBSHELL starts at parent + 1.
        env_map.insert(
            "__RUBASH_SUBSHELL_DEPTH".to_string(),
            (self.shell_state.subshell_depth.get() + 1).to_string(),
        );
        // GNU execute_cmd.c:1761 (execute_in_subshell): a forked async
        // subshell keeps the parent's $0 and line number, so diagnostics
        // inside `xcase &` print `./script: line N:` rather than `bash:`. The
        // spawned `rubash -c` child inherits the script identity through
        // these markers (init.rs keeps them for __RUBASH_SHELL_PID carriers).
        if let Some(script_name) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
            env_map.insert("__RUBASH_SCRIPT_NAME".to_string(), script_name.clone());
            let line = background_command
                .command
                .line
                .map(|line| line.to_string())
                .or_else(|| {
                    self.shell_state
                        .env_vars
                        .get("__RUBASH_CURRENT_LINE")
                        .cloned()
                })
                .unwrap_or_else(|| "1".to_string());
            env_map.insert("__RUBASH_CURRENT_LINE".to_string(), line.clone());
            let line_offset = line.parse::<usize>().unwrap_or(1).saturating_sub(1);
            env_map.insert("__RUBASH_LINE_OFFSET".to_string(), line_offset.to_string());
        }
        // rubash#737: mark this as a background-job wrapper. Children it
        // spawns (via pre_exec) set PR_SET_PDEATHSIG so they die with the
        // wrapper when it's killed — otherwise `kill $!` leaves orphaned
        // grandchildren running (the wrapper is killed but the real work
        // survives).
        // rubash#866: the marker is an unguessable spawn-time nonce carried
        // by BOTH handshake channels (this env entry and the
        // --internal-bg-wrapper argv flag spliced in below). The wrapper
        // child arms its in-process role only when the two agree, then
        // consumes both — so a user `export __RUBASH_BACKGROUND_WRAPPER=1`
        // cannot arm PDEATHSIG for children that never went through this
        // spawn path, and this constant "1" is never trusted again.
        let wrapper_nonce = crate::builtins::kill::new_background_wrapper_nonce();
        env_map.insert(
            crate::builtins::kill::BACKGROUND_WRAPPER_ENV_KEY.to_string(),
            wrapper_nonce.clone(),
        );

        // rubash#1095: GNU execute_cmd.c:701 runs an async `( list )`
        // through a SINGLE make_child fork — the fork IS the subshell, so
        // the child's BASHPID (variables.c:1511 get_bashpid = getpid())
        // equals the parent's $!. The wrapper spawn is that fork, but the
        // child would re-enter the parens in-process (subshell_depth + 1)
        // and report pid + 1. Coalesce the two exactly like GNU: serialize
        // the INNER list (the spawn already realized the subshell level)
        // and seed the child's depth so both $BASH_SUBSHELL and $BASHPID
        // land where the single fork puts them. Non-paren bodies
        // (`{ }`, simple commands) equally run one level down in GNU
        // (probed 5.3.20: `{ echo $BASH_SUBSHELL; } &` and
        // `echo $BASH_SUBSHELL &` both report 1), so the child always
        // starts one level below the parent — only an unstripped paren
        // node still bumps in-process.
        let parent_depth = self.shell_state.subshell_depth.get();
        let bashpid_base = parent_depth + 1;
        let strip_parens = background_paren_absorbable(&background_command.command, &consumed);
        let mut child_command = background_command.command.clone();
        strip_consumed_redirects(&mut child_command, &consumed);
        let source = if strip_parens {
            // SAFETY: strip gate guarantees subshell_command.is_some().
            let inner = child_command
                .subshell_command
                .as_ref()
                .map(|subshell| subshell.body.clone())
                .unwrap_or_default();
            self.background_command_source_for_body(&inner)
        } else {
            self.background_command_source(&child_command)
        };
        env_map.insert(
            "__RUBASH_BG_DEPTH".to_string(),
            (if strip_parens || background_command.command.subshell_command.is_none() {
                // GNU execute_cmd.c:1588 execute_in_subshell: the forked
                // async child runs subshell_level++ for EVERY `&` body
                // (simple, `{ }`, `( )`, coproc). A stripped paren absorbs
                // the fork (no in-process bump), and a non-paren body never
                // bumps in-process, so both seed parent + 1. Only an
                // UNSTRIPPED paren node still bumps in-process
                // (execute_subshell_command_with_redirects), so it seeds
                // the parent level (rubash#1074 remainder: `echo
                // $BASH_SUBSHELL &` and `{ ...; } &` reported 0).
                bashpid_base
            } else {
                parent_depth
            })
            .to_string(),
        );
        env_map.insert(
            "__RUBASH_BG_BASHPID_BASE".to_string(),
            bashpid_base.to_string(),
        );

        // niubash#362: the serialized source (every function the
        // parent carries + the command) rides the child's `-c` argument. When
        // that exceeds the OS limit — CreateProcessW's 32767-unit
        // lpCommandLine on Windows, or a single argv entry over MAX_ARG_STRLEN
        // on Unix — hand the script to the child through a temp file named by
        // __RUBASH_CMD_FILE instead. The child's entry point splices the file
        // contents back into its `-c` operand before parsing
        // (rubash::invocation::materialize_command_file), so argv semantics
        // are unchanged. A function-heavy shell ("echo hello &" after
        // oh-my-niu sourced hundreds of functions) otherwise dies with
        // "File name too long".
        let mut spawn_args = vec!["-c".to_string(), source.clone()];
        let mut respawn_temp: Option<PathBuf> = None;
        if respawn_source_needs_file(&exe, &source) {
            if let Ok(path) = self.write_respawn_command_file(&source) {
                env_map.insert(
                    "__RUBASH_CMD_FILE".to_string(),
                    path.to_string_lossy().into_owned(),
                );
                spawn_args[1] = String::new();
                respawn_temp = Some(path);
            }
        }
        // rubash#866: second handshake channel — the nonce rides argv too,
        // ahead of every other argument. The host CLI consumes it (with the
        // env entry above) before any option parser or the `-c` driver sees
        // anything; the wrapper role arms only when both channels agree.
        // rubash#873: the flag is niu-CLI-specific, so it rides ONLY when
        // this process's own CLI declared itself a handshake consumer
        // (rubash main.rs — and adopting hosts — call
        // consume_background_wrapper_handshake at startup). A launcher that
        // never consumes (niubash's niu) or any foreign face would reject
        // the unknown option — `bash: --internal-bg-wrapper=<nonce>:
        // invalid option` + usage — and the background command would die at
        // spawn (the niubash background_spawn_args CI red). For those hosts
        // the respawn is env-nonce-only: the key is inert for a
        // non-consumer (never read; scrubbed by any rubash that does
        // start), and a single channel can never arm — the rubash#866
        // contract holds either way.
        //
        // cfg(test): this crate's OWN unit tests drive the Executor
        // in-process — the test harness process never runs main.rs, so the
        // consumer bit would make the attach order-dependent (true only
        // after the handshake unit tests happened to run first). In the
        // unit-test process the respawn target is always this repo's own
        // engine binary (CARGO_BIN_EXE / test_rubash_binary_from_current_
        // exe), whose main does consume — attaching unconditionally there
        // is exactly the pre-gate behavior the suite was green under.
        if crate::builtins::kill::background_wrapper_flag_consumer_present() || cfg!(test) {
            spawn_args.insert(
                0,
                format!(
                    "{}{}",
                    crate::builtins::kill::BACKGROUND_WRAPPER_FLAG,
                    wrapper_nonce
                ),
            );
        }

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
                            file.as_raw_handle() as crate::fd::HANDLE
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
                env_map.insert(format!("__RUBASH_FD_HANDLE_{fd}"), format!("{:#x}", dup));
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
                env_map.insert(format!("__RUBASH_FD_WHANDLE_{fd}"), format!("{:#x}", dup));
                env_map.insert(
                    format!("__RUBASH_FD_WPATH_{fd}"),
                    f.path.to_string_lossy().into_owned(),
                );
            }
        }

        let spawn_result = crate::fd::spawn_whitelisted(&crate::fd::WhitelistedSpawn {
            program: exe.clone(),
            args: spawn_args,
            env: env_map.into_iter().collect(),
            std_handles,
            extra_handles,
            // G-02: the respawned child starts in the session cwd.
            cwd: Some(self.shell_state.cwd.clone()),
        });
        for h in owned_handles {
            crate::fd::close_handle(h);
        }
        // A failed spawn never ran the child, so the out-of-band script file
        // (niubash#362) was never read — remove it here instead of leaking it.
        let child = match spawn_result {
            Ok(child) => child,
            Err(error) => {
                if let Some(path) = &respawn_temp {
                    let _ = fs::remove_file(path);
                }
                return Err(error.into());
            }
        };
        let pid = child.id();
        // rubash#169: the child holds a capture write end — registering its
        // pid lets the reap path (wait/refresh) drain the shared pipe into
        // the capture buffer at the moment the shell observes the exit.
        if captured_pipe_writer {
            crate::executor::shell_options::register_background_capture_writer(pid);
        }
        self.background_children.insert(pid, child);
        let job_control =
            crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor");
        let job_id = self.shell_state.job_table.register_process_with_stages(
            pid,
            display_source.clone(),
            true,
            async_job_pgrp(pid, job_control),
            bash_job_stage_commands(&background_command.command),
        );
        self.shell_state.job_table.set_job_control(pid, job_control);
        self.shell_state.last_background_pid = Some(pid);
        self.notify_async_job_started(job_id, pid);
        self.exit_code = 0;
        Ok(())
    }

    /// rubash#1035: whether GNU would EXECUTE the background body as a fresh
    /// process image (as opposed to running it in-process in the forked
    /// async child). Only exec-like bodies inherit the async SIGINT/SIGQUIT
    /// ignore as SIG_HARD_IGNORE (trap.c set_signal refuses to trap them):
    /// a bare simple command resolving to an external or a script file.
    /// Compound bodies (subshells, brace groups, pipelines, `&&`/`||`
    /// lists, loops, conditionals, ...) run in-process in GNU's forked
    /// child, where trap.c's ASYNCSIG special-case keeps `trap ... INT`
    /// installable — and function/builtin bodies likewise run in-process
    /// (execute_subshell_builtin_or_function, async path). Aliases stay
    /// conservative: an alias can expand to a compound body, so an aliased
    /// name counts as in-process. Anything else bare-simple (externals,
    /// scripts, even unresolvable names — GNU still exec-attempts those)
    /// counts as exec-like.
    #[cfg(unix)]
    fn background_body_execs_in_child(&self, command: &CommandNode) -> bool {
        if command.pipe.is_some()
            || command.pipeline_command.is_some()
            || command.and_or_list.is_some()
            || command.and_or.is_some()
            || command.time_command.is_some()
            || command.background_command.is_some()
            || command.inverted_command.is_some()
            || command.inverted
            || command.subshell
            || command.subshell_end
            || command.subshell_command.is_some()
            || command.for_command.is_some()
            || command.arithmetic_command.is_some()
            || command.if_command.is_some()
            || command.loop_command.is_some()
            || command.conditional_command.is_some()
            || command.case_command.is_some()
            || command.select_command.is_some()
            || command.function_command.is_some()
            || command.brace_group.is_some()
            || command.coproc_command.is_some()
        {
            return false;
        }
        if command.words.is_empty() {
            return false;
        }
        let name = &command.words[0];
        if name.starts_with('%') {
            return false;
        }
        if let Some((prefix, _)) = name.split_once('=') {
            if super::support_names::is_shell_name(prefix) {
                return false;
            }
        }
        if super::builtin_names::is_shell_builtin_name(name)
            || self.shell_state.functions.contains_key(name)
            || self.shell_state.aliases.contains_key(name)
        {
            return false;
        }
        true
    }

    /// rubash#809: resolve a background body to a directly-spawnable external
    /// — `(program, argv)` — or None when the wrapper respawn must run.
    /// The gate is deliberately narrow: every construct the `rubash -c`
    /// wrapper exists to carry (compound semantics, assignment tempenv,
    /// redirection application, substitution execution, fd>=3 forwarding,
    /// restricted-shell rejection, alias/function/builtin dispatch, script
    /// fallback, capture plumbing) falls back to the wrapper, whose behavior
    /// is unchanged.
    fn background_direct_external_command(
        &mut self,
        command: &CommandNode,
        redirects_fully_consumed: bool,
    ) -> Option<(std::path::PathBuf, Vec<String>)> {
        // A restricted shell refuses command names carrying `/' before any
        // dispatch (execute_prepared_command -> restricted_command_error,
        // pipeline_exec.rs:3564). The wrapper keeps that rejection exactly
        // where it is today, so any `set -r` session never takes this path.
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "restricted") {
            return None;
        }
        // Only a bare simple command: any compound carrier, connector, or
        // nested execution form needs the wrapper's subshell semantics.
        if command.pipe.is_some()
            || command.pipeline_command.is_some()
            || command.and_or_list.is_some()
            || command.and_or.is_some()
            || command.time_command.is_some()
            || command.background_command.is_some()
            || command.inverted_command.is_some()
            || command.inverted
            || command.subshell
            || command.subshell_end
            || command.subshell_command.is_some()
            || command.for_command.is_some()
            || command.arithmetic_command.is_some()
            || command.if_command.is_some()
            || command.loop_command.is_some()
            || command.conditional_command.is_some()
            || command.case_command.is_some()
            || command.select_command.is_some()
            || command.function_command.is_some()
            || command.brace_group.is_some()
            || command.coproc_command.is_some()
        {
            return None;
        }
        // No assignments (tempenv belongs to the child in GNU).
        // Redirections are admissible only when resolve_background_stdio
        // consumed EVERY entry of `command.redirects` into the fd 0-2
        // handles the direct spawn receives: the Option mirror fields
        // (redirect_out & friends) duplicate entries of that list, so a
        // fully-consumed list means the stdio triple carries the whole
        // redirection set and nothing is left for a child shell to
        // re-apply. Any unconsumed remainder (fd>2, {fd} vars, procsub
        // targets, failed opens whose diagnostic belongs to the job)
        // keeps the wrapper (rubash#1033: a redirected simple external
        // used to ALWAYS take the wrapper, so $! named the wrapper and
        // kill/wait never reached the real job's traps).
        let redirect_fields_present = !command.redirects.is_empty()
            || command.redirect_in.is_some()
            || command.redirect_out.is_some()
            || command.append.is_some()
            || command.redirect_err.is_some()
            || command.redirect_err_append.is_some()
            || command.heredoc.is_some()
            || command.heredoc_body.is_some()
            || command.heredoc_delimiter.is_some()
            || !command.heredoc_redirects.is_empty()
            || command.here_string.is_some()
            || command.here_string_carrier.is_some();
        if !command.assignments.is_empty()
            || !command.compound_assignments.is_empty()
            || !command.array_element_assignments.is_empty()
            || !command.assignment_raws.is_empty()
            || (redirect_fields_present && !redirects_fully_consumed)
        {
            return None;
        }
        // Process substitutions need the materialization plumbing the
        // foreground external path owns; leave them to the wrapper.
        if !command.process_substitutions.is_empty() {
            return None;
        }
        // rubash#809 CR F2: GNU expands the async child's words INSIDE the
        // forked child. Expanding substitution- or assignment-bearing words
        // here in the parent would (a) double-fire side effects whenever the
        // shape then falls back to the wrapper (the wrapper child re-expands
        // the serialized source — `echo $(echo tick >> log) &` would append
        // twice), (b) leak `${x:=v}` assignments into the parent shell, and
        // (c) block the parent on the substitution's runtime. Such shapes
        // keep the wrapper, whose child expands them exactly as today — the
        // hot path (`sleep 0 &`, plain/parameter-expansion words) is
        // unaffected. Raw-text scan: the structured parameter_expansion
        // nodes are not populated by this parser, and `$((...))` rides the
        // same `$(` marker as command substitution.
        if command.word_metadata.iter().any(|metadata| {
            metadata.raw.contains("$(")
                || metadata.raw.contains('`')
                || metadata.raw.contains(":=")
                || parameter_expansion_assigns(&metadata.raw)
        }) {
            return None;
        }
        // rubash#180: a persistent `exec 3<file` descriptor must reach the
        // child. The wrapper forwards live fd>=3 entries through the
        // __RUBASH_FD_* env channel (spawn_external_process semantics); a
        // native direct exec cannot, so any open fd>=3 keeps the wrapper.
        if self
            .fd_table
            .entries
            .iter()
            .any(|(fd, entry)| *fd >= 3 && !entry.closed)
        {
            return None;
        }
        if command.words.is_empty() {
            return None;
        }
        // rubash#809 CR F2-R2: the fatal parameter pre-scan lives on the
        // dispatch path (command_execute.rs -> validate_command_parameter_
        // expansions), NOT inside expand_command_words — without it the gate
        // would swallow `set -u` unbound-variable and `${x:?msg}` /
        // `${x?msg}` verdicts and hand degraded words to argv (sleep executed
        // with an empty/message argument). Any fatal shape keeps the
        // wrapper, whose child runs this same scan on its own dispatch and
        // reports exactly as today. Safe to call here: the F2 arm above
        // already routed every command-substitution-bearing word to the
        // wrapper, so the scan's prefix-comsub side-effect replay can never
        // fire in the parent.
        if self.parameter_expansion_error(command).is_some() {
            return None;
        }
        // A %jobspec leads execute_prepared_command's fg/bg dispatch, and an
        // assignment-shaped first word is an assignment command — wrapper.
        let first_raw = &command.words[0];
        if first_raw.starts_with('%') {
            return None;
        }
        if let Some((name, _)) = first_raw.split_once('=') {
            if super::support_names::is_shell_name(name) {
                return None;
            }
        }
        // Expand with the same machinery the foreground dispatch uses; any
        // expansion failure falls back to the wrapper, whose child re-parses
        // the source and reports the error exactly as before. The probe is
        // SIDE-EFFECT-FREE: GNU expands the async child's words inside the
        // forked child, so the parent never observes a failglob verdict —
        // a probe hit used to print `no match` here (the wrapper child
        // re-reported → doubled, 753.11 line 4) AND arm the parent-side
        // evalerror discard, which swallowed the rest of the launch line
        // (`set -- /no/such/*.zzz & echo BG_after` lost its echo). Suppress
        // the report and restore the expansion latches on the Err path.
        let saved_exit_code = self.exit_code;
        let saved_expansion_failed = self.command_expansion_failed.get();
        let saved_evalerror_pending = self.evalerror_pending.get();
        let saved_failglob_pending = self.evalerror_failglob_pending.get();
        let saved_evalerror_line = self.evalerror_line.get();
        self.expansion_probe_active.set(true);
        let probe = self.expand_command_words(command);
        self.expansion_probe_active.set(false);
        let expanded = match probe {
            Ok(expanded) => expanded,
            Err(_) => {
                self.exit_code = saved_exit_code;
                self.command_expansion_failed.set(saved_expansion_failed);
                self.evalerror_pending.set(saved_evalerror_pending);
                self.evalerror_failglob_pending.set(saved_failglob_pending);
                self.evalerror_line.set(saved_evalerror_line);
                return None;
            }
        };
        if self.shell_state.arithmetic_nounset_error.replace(false) {
            self.shell_state.arithmetic_expansion_error.set(false);
            return None;
        }
        if expanded.words.is_empty() {
            return None;
        }
        let name = &expanded.words[0];
        // Dispatch precedence (execute_prepared_command): jobspec, builtin,
        // function and alias names all mean this is not a disk command.
        if name.starts_with('%')
            || super::builtin_names::is_shell_builtin_name(name)
            || self.shell_state.functions.contains_key(name)
            || (!self.shell_state.aliases.is_empty() && self.shell_state.aliases.contains_key(name))
        {
            return None;
        }
        // Same PATH resolution execute_external_inner uses; a name that does
        // not resolve keeps today's child-side "command not found" behavior.
        let Some(program) =
            crate::executor::path::find_user_command(name, &self.shell_state.env_vars)
        else {
            return None;
        };
        // Script files need the shell fallback the wrapper's child provides.
        if crate::executor::path::should_run_with_shell(&program) {
            return None;
        }
        // On unix a shebangless text file is an execve-ENOEXEC case:
        // a native spawn would hand it to glibc execvp's /bin/sh
        // fallback instead of this shell (rubash#1037) — the wrapper's
        // foreground-classification path owns it, so stay out.
        #[cfg(unix)]
        if super::external_inner::is_enoexec_candidate(&program) {
            return None;
        }
        Some((program, expanded.words))
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
    ///
    /// rubash#1862: one class is NOT resolved here even when its open
    /// would succeed — FIFO targets. GNU's async child applies the
    /// redirections on its own descriptors after the fork
    /// (execute_cmd.c:1761-1763 execute_in_subshell), so a FIFO open's
    /// rendezvous wait happens in the child; resolving it in this
    /// parent walk deadlocks the shell's own execution thread when the
    /// peer is a command that has not run yet (`( ... ) > fifo &` with
    /// the reader still to come). A FIFO redirect therefore defers its
    /// whole fd (background_stdio_defer_fifo_fds), and the child
    /// replays that fd's redirect subsequence in order.
    fn resolve_background_stdio(
        &mut self,
        command: &CommandNode,
    ) -> ([BackgroundStdio; 3], Vec<bool>, bool) {
        use crate::parser::RedirectKind;
        // GNU execute_cmd.c:2837-2841 (execute_connection '&') + :595
        // async_redirect_stdin: CMD_STDIN_REDIR is set — and the spawned
        // child dups /dev/null onto fd 0 — only when the shell is in a
        // subshell or lacks job control AND the enclosing control structure
        // did not redirect stdin. The global stdin_redir (execute_cmd.c:828,
        // cleared per reader command at eval.c:181) decides: `{ read; } &`
        // inside `while ... done <<EOF` inherits the heredoc, while a bare
        // `sleep 5 &` gets /dev/null.
        let job_control =
            crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor");
        let subshell_environment = self.shell_state.subshell_depth.get() > 0
            || self.shell_state.in_command_substitution.get();
        let mut stdio: [BackgroundStdio; 3] = [
            if (subshell_environment || !job_control) && !self.shell_state.stdin_redir.get() {
                BackgroundStdio::Null
            } else {
                self.inherited_async_stdin()
            },
            BackgroundStdio::Inherit,
            BackgroundStdio::Inherit,
        ];
        // rubash#169: GNU's substitution child holds a real capture pipe as
        // fd 1 (subst.c:7143 command_substitute), and an async child forked
        // inside (`{ ...; } & wait` — nvm `nvm ls`) inherits THAT pipe, so
        // its output is captured. In-process captures are Vec buffers an
        // OS-spawned child cannot write into, so `Inherit` (duplicating the
        // process stdout) leaks the output past the capture. When a capture
        // scope is live, replace the inherited fd 1 with a duplicate of the
        // scope's shared background pipe — every background child of the
        // scope shares one write end and the kernel merges their writes, and
        // the drain at reap/capture-end reads it back in that order. The
        // command's own fd-1 redirects below still override this, and
        // `2>&1`-style dups copy it, matching the sequential dup2 ordering a
        // forked GNU child would apply on top of the inherited pipe.
        let mut captured_pipe_writer = false;
        if let Some(write) = crate::executor::shell_options::background_capture_write_end() {
            stdio[1] = BackgroundStdio::File(write);
            // Registration is conservative: a later fd-1 redirect may drop
            // the pipe for THIS child, but reaping it then only drains an
            // empty pipe — never another child's bytes (the drain fires when
            // the whole writer set is reaped).
            captured_pipe_writer = true;
        }
        // rubash#1862 bookkeeping for the FIFO deferral further down:
        // the stdio the child spawns with before any of the command's
        // own redirects (base_stdio), plus per-slot stacks of the
        // redirects this walk has consumed — (redirect index, slot
        // value after the write, dup source fd, staged-heredoc flag) —
        // so a deferral can roll each slot back to exactly the value
        // its surviving writes imply.
        let base_stdio = stdio.clone();
        let mut slot_writes: [Vec<(usize, BackgroundStdio, Option<u32>, bool)>; 3] =
            [Vec::new(), Vec::new(), Vec::new()];
        let mut deferred_fd = [false; 3];
        let mut consumed = vec![false; command.redirects.len()];

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
                    // rubash#1862: a heredoc on an already-deferred fd
                    // stays in the child's source with the rest of that
                    // fd's redirect subsequence.
                    if deferred_fd[fd as usize] {
                        continue;
                    }
                    match self.materialize_async_stdin_body(command, index) {
                        Some(file) => {
                            let value = BackgroundStdio::File(file);
                            stdio[fd as usize] = value.clone();
                            slot_writes[fd as usize].push((index, value, None, true));
                            consumed[index] = true;
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
            // rubash#1862: once an fd is deferred to the child (a FIFO
            // redirect below), every later redirect on that fd stays
            // unconsumed as well — the child replays the fd's whole
            // redirect subsequence in order on top of the spawn stdio,
            // which is the only ordering that stays GNU-faithful.
            if deferred_fd[fd as usize] {
                continue;
            }
            let mut dup_source: Option<u32> = None;
            let resolved = match redirect.kind {
                RedirectKind::Input => {
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        BackgroundStdio::Null
                    } else if redirect_target_fd(&target).is_some() {
                        continue;
                    } else if self.background_redirect_target_is_fifo(&target)
                        && Self::background_stdio_defer_fifo_fds(
                            &mut stdio,
                            &base_stdio,
                            &mut slot_writes,
                            &mut consumed,
                            &mut deferred_fd,
                            &[fd as usize],
                        )
                    {
                        // rubash#1862: the FIFO open belongs to the
                        // child; this redirect stays in its source.
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
                        // rubash#1862: a dup OF a deferred fd is
                        // replayed by the child too — this walk's slot
                        // for the source still holds the spawn value,
                        // not the FIFO the child will open onto it.
                        if deferred_fd[source_fd as usize] {
                            continue;
                        }
                        dup_source = Some(source_fd);
                        stdio[source_fd as usize].clone()
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
                    } else if self.background_redirect_target_is_fifo(&target)
                        && Self::background_stdio_defer_fifo_fds(
                            &mut stdio,
                            &base_stdio,
                            &mut slot_writes,
                            &mut consumed,
                            &mut deferred_fd,
                            &[fd as usize],
                        )
                    {
                        // rubash#1862: the FIFO open belongs to the
                        // child; this redirect stays in its source.
                        continue;
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
                    } else if self.background_redirect_target_is_fifo(&target)
                        && Self::background_stdio_defer_fifo_fds(
                            &mut stdio,
                            &base_stdio,
                            &mut slot_writes,
                            &mut consumed,
                            &mut deferred_fd,
                            &[fd as usize],
                        )
                    {
                        // rubash#1862: the FIFO open belongs to the
                        // child; this redirect stays in its source.
                        continue;
                    } else {
                        match open_redirect_append(&self.session_path(shell_path_to_windows(
                            &target,
                            &self.shell_state.env_vars,
                        ))) {
                            Ok(file) => BackgroundStdio::File(file),
                            Err(_) => continue,
                        }
                    }
                }
                RedirectKind::ReadWrite => {
                    let target = self.expand_redirect_target(redirect);
                    let path = self
                        .session_path(shell_path_to_windows(&target, &self.shell_state.env_vars));
                    // rubash#1862: keep the parent's hands off FIFO
                    // opens uniformly, even though `<>` itself would
                    // not block — the open belongs to the child.
                    if super::shell_options::redirect_path_is_fifo(&path)
                        && Self::background_stdio_defer_fifo_fds(
                            &mut stdio,
                            &base_stdio,
                            &mut slot_writes,
                            &mut consumed,
                            &mut deferred_fd,
                            &[fd as usize],
                        )
                    {
                        continue;
                    }
                    // niubash#519: CON rejects a read+write handle; the
                    // stdin side needs only its CONIN$ half.
                    let opened = if is_con_device_path(&path) {
                        File::open("CONIN$")
                    } else {
                        OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create(true)
                            .open(&path)
                    };
                    match opened {
                        Ok(file) => BackgroundStdio::File(file),
                        Err(_) => continue,
                    }
                }
                RedirectKind::CombinedOutput | RedirectKind::CombinedAppend => {
                    // `&>file` / `&>>file`: one open, both fds share it.
                    // rubash#1862: fd 2 already deferred (fd 1 is gated
                    // by the loop-top check) defers the pair as well.
                    if deferred_fd[2] {
                        continue;
                    }
                    let target = self.expand_redirect_target(redirect);
                    if self.background_redirect_target_is_fifo(&target)
                        && Self::background_stdio_defer_fifo_fds(
                            &mut stdio,
                            &base_stdio,
                            &mut slot_writes,
                            &mut consumed,
                            &mut deferred_fd,
                            &[1, 2],
                        )
                    {
                        // The FIFO open belongs to the child; this
                        // redirect stays in its source.
                        continue;
                    }
                    let file = if is_null_device(&target) {
                        stdio[1] = BackgroundStdio::Null;
                        stdio[2] = BackgroundStdio::Null;
                        slot_writes[1].push((index, BackgroundStdio::Null, None, false));
                        slot_writes[2].push((index, BackgroundStdio::Null, None, false));
                        consumed[index] = true;
                        continue;
                    } else {
                        let opened = if redirect.kind == RedirectKind::CombinedAppend {
                            open_redirect_append(&self.session_path(shell_path_to_windows(
                                &target,
                                &self.shell_state.env_vars,
                            )))
                        } else {
                            self.create_redirect_output(&target, false)
                        };
                        match opened {
                            Ok(file) => file,
                            Err(_) => continue,
                        }
                    };
                    stdio[1] = match file.try_clone() {
                        Ok(clone) => BackgroundStdio::File(clone),
                        Err(_) => continue,
                    };
                    stdio[2] = BackgroundStdio::File(file);
                    slot_writes[1].push((index, stdio[1].clone(), None, false));
                    slot_writes[2].push((index, stdio[2].clone(), None, false));
                    consumed[index] = true;
                    continue;
                }
                _ => continue,
            };
            stdio[fd as usize] = resolved.clone();
            slot_writes[fd as usize].push((index, resolved, dup_source, false));
            consumed[index] = true;
        }

        (stdio, consumed, captured_pipe_writer)
    }

    /// True when a background redirect target resolves to an existing
    /// FIFO (rubash#1862): the one real open of such a target carries
    /// rendezvous semantics, so it must happen in the async child,
    /// never in the parent walk of resolve_background_stdio.
    fn background_redirect_target_is_fifo(&self, target: &str) -> bool {
        let path = self.session_path(shell_path_to_windows(target, &self.shell_state.env_vars));
        super::shell_options::redirect_path_is_fifo(&path)
    }

    /// Defer fds to the background child (rubash#1862): mark them
    /// deferred and roll back every redirect this walk already
    /// consumed for them — plus every consumed dup whose SOURCE is one
    /// of them — so the child replays each deferred fd's whole
    /// redirect subsequence, in order, on top of the spawn stdio, and
    /// performs the FIFO open itself. Slots keep the value their
    /// surviving writes imply (base_stdio when none survive). Returns
    /// false — deferring nothing — when a target fd already carries a
    /// staged heredoc: its parent-materialized temp file cannot be
    /// replayed from the child's source, so that fd mix keeps today's
    /// parent resolution.
    fn background_stdio_defer_fifo_fds(
        stdio: &mut [BackgroundStdio; 3],
        base_stdio: &[BackgroundStdio; 3],
        slot_writes: &mut [Vec<(usize, BackgroundStdio, Option<u32>, bool)>; 3],
        consumed: &mut [bool],
        deferred_fd: &mut [bool; 3],
        fds: &[usize],
    ) -> bool {
        for &fd in fds {
            if slot_writes[fd].iter().any(|entry| entry.3) {
                return false;
            }
        }
        for &fd in fds {
            deferred_fd[fd] = true;
        }
        for slot in 0..3 {
            let writes = &mut slot_writes[slot];
            let mut i = 0;
            while i < writes.len() {
                let entry_index = writes[i].0;
                let entry_source = writes[i].2;
                if fds.contains(&slot)
                    || entry_source.is_some_and(|source| fds.contains(&(source as usize)))
                {
                    consumed[entry_index] = false;
                    writes.remove(i);
                } else {
                    i += 1;
                }
            }
            stdio[slot] = writes
                .last()
                .map(|entry| entry.1.clone())
                .unwrap_or_else(|| base_stdio[slot].clone());
        }
        true
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
            let bytes = crate::executor::substitution_metadata::shell_text_to_raw_bytes(&remaining);
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
                matches!(entry.kind, RedirectKind::HereDoc | RedirectKind::HereString)
                    && matches!(entry.kind, RedirectKind::HereString) == want_string
                    && entry.fd == redirect.fd
            })
            .count();
        let expanded = command
            .heredoc_redirects
            .iter()
            .filter(|entry| entry.fd == redirect.fd && entry.here_string == want_string)
            .nth(ordinal)
            .map(|entry| self.expand_heredoc_redirect_text(entry))
            .or_else(|| {
                if redirect.fd.is_some() {
                    return None;
                }
                if want_string {
                    if command.here_string_carrier.is_some() {
                        Some(self.expand_here_string_mut_from_carrier(&command.here_string_carrier))
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
        let mut source = self.background_function_prelude();
        source.push_str(&bash_command_source_text(command));
        source
    }

    /// rubash#1095: `background_command_source` for an absorbed `( list )`
    /// body — the inner sequence rides the child's `-c` source because the
    /// spawn itself realized the subshell level (GNU execute_cmd.c:701).
    fn background_command_source_for_body(&self, body: &[CommandNode]) -> String {
        let mut source = self.background_function_prelude();
        source.push_str(&bash_command_sequence_text(body));
        source
    }

    /// Exportable-function prelude shared by both background source forms
    /// (print_cmd-style `name() { ...; }; ` definitions ahead of the body).
    pub(in crate::executor) fn background_function_prelude(&self) -> String {
        let mut source = String::new();
        for (name, body) in &self.shell_state.functions {
            if is_exportable_function_name(name) {
                source.push_str(name);
                source.push_str("() { ");
                let body_text = bash_command_sequence_text(&body.commands);
                source.push_str(&body_text);
                // print_cmd.c:1520-1529 semicolon(): no `;' after a body
                // whose last command printed as `... &` — parse.y:1275/1290
                // only admits newline_list after `&`, and `& ; }` is the
                // syntax error that killed respawned background children
                // (rubash#294). `&\n}` is the valid GNU spelling.
                if body_text.ends_with('&') {
                    source.push_str("\n}; ");
                } else {
                    source.push_str("; }; ");
                }
            }
        }
        source
    }

    /// Write the serialized `-c` script of a respawned async `&` child to a
    /// temp file for the out-of-band transport (niubash#362). The child
    /// removes the file after reading it
    /// (rubash::invocation::materialize_command_file); a spawn failure removes
    /// it in the caller instead.
    pub(in crate::executor) fn write_respawn_command_file(
        &self,
        source: &str,
    ) -> Result<PathBuf, ExecuteError> {
        let path = self.respawn_command_temp_path()?;
        fs::write(&path, source.as_bytes())?;
        Ok(path)
    }

    pub(in crate::executor) fn execute_time_ast_command(
        &mut self,
        time_command: &TimeCommand,
    ) -> Result<(), ExecuteError> {
        // rubash#1611: the errexit-conversion mark is scoped to THIS
        // timed command's inner dispatch — clear any stale value before
        // the inner runs (the mark is taken right after it below).
        self.errexit_status_converted.set(false);
        // rubash#1218: in posix mode a dash word right after `time`
        // means `time` was never a keyword (GNU parse.y), so the
        // reconstructed command (`time`, the prefix words, then the
        // inner command's words) dispatches as an ordinary command
        // named `time` (PATH-resolved; 127 when absent). Only the simple
        // inner shape is rebuilt; pipeline/compound inners keep the
        // timed path.
        // rubash#1520: the verdict is the parse-time-frozen one recorded
        // on the node (keyword_was_plain_word), never the live mode —
        // the parser folds only keyword heads, so a same-unit `set -o
        // posix` that executed after the fold cannot reach this branch.
        if time_command.keyword_was_plain_word
            && time_command.command.pipeline_command.is_none()
            && !time_command.command.words.is_empty()
        {
            let inner = &time_command.command;
            let mut plain = CommandNode::new();
            plain.line = inner.line;
            plain.logical_line = inner.logical_line;
            let mut words = vec![time_command.keyword.clone()];
            words.extend(time_command.prefix_words.iter().cloned());
            words.extend(inner.words.iter().cloned());
            plain.words = words;
            plain.redirects = inner.redirects.clone();
            plain.redirect_in = inner.redirect_in.clone();
            plain.redirect_out = inner.redirect_out.clone();
            plain.append = inner.append.clone();
            plain.redirect_err = inner.redirect_err.clone();
            plain.redirect_err_append = inner.redirect_err_append.clone();
            return self.execute_command(&plain);
        }
        // rubash#1528: a timed coproc returns at spawn, but GNU times
        // the job's whole lifetime and reports at completion notice
        // (measured on the 5.3.20 oracle: `time coproc C { sleep 2; }`
        // + wait reports real 2.017s at the wait, an instant body
        // reports 0.000s — both at notice time, never at spawn).
        // Snapshot the child set so the spawn below can be identified
        // and its report deferred to the completion funnel.
        // rubash#1611: a pure time chain delegates its report to the
        // innermost frame (see time_inner_is_pure_time_chain) — this
        // frame adjudicates status/traps/errexit but prints nothing.
        let delegated_report = time_inner_is_pure_time_chain(&time_command.command);
        let timed_coproc_shape = time_command.command.coproc_command.is_some()
            || time_command.command.words.first().map(String::as_str) == Some("coproc");
        let children_before: std::collections::HashSet<u32> = if timed_coproc_shape {
            self.background_children.keys().copied().collect()
        } else {
            std::collections::HashSet::new()
        };
        let started = time_command_started();
        // rubash#1611: for the span of the inner dispatch below, the
        // dispatch-level errexit conversion sites convert WITHOUT
        // firing the ERR trap — the timed command completes here and
        // the ast_exec time branch performs the single trap
        // adjudication for it (the rubash#1277 design). The depth is
        // raised/lowered around the single dispatch statement; the
        // statement contains no `?` (the DEBUG-trap arm captures its
        // result instead of propagating), so the lowering cannot be
        // bypassed.
        self.timed_dispatch_depth
            .set(self.timed_dispatch_depth.get() + 1);
        // rubash#1611: clear the conversion mark immediately before the
        // inner dispatch (the DEBUG-trap call in the simple-command arm
        // below runs its own commands first; their conversions must not
        // be mistaken for the inner command's).
        self.errexit_status_converted.set(false);
        // Snapshot the parse-error latch: a parse-diagnostic ExitCode(2)
        // out of the inner dispatch means the timed command never
        // executed (GNU reports no timing for it), unlike a genuine
        // runtime unwind.
        let parse_error_before_inner = self.parse_error_occurred;
        let inner_result = if let Some(coproc_cmd) = &time_command.command.coproc_command {
            self.execute_coproc_command(&time_command.command, coproc_cmd)
        } else if time_command.command.words.is_empty()
            && command_has_no_effect(&time_command.command)
        {
            self.exit_code = 0;
            Ok(())
        } else if let Some(pipeline_command) = &time_command.command.pipeline_command {
            self.execute_pipeline_command(pipeline_command)
        } else if time_command.command.brace_group.is_some() {
            self.execute_brace_group_pipeline(&time_command.command)
                .map(|_| ())
        } else if time_command.command.words.first().map(String::as_str) == Some("coproc") {
            self.execute_time_reparsed_coproc(&time_command.command)
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
            // rubash#1611: the trap result is captured, not `?`-
            // propagated, so the dispatch statement below the timed-
            // dispatch depth raise contains no early exit and the
            // depth lowering after it always runs. An erroring trap
            // still abandons the timed command with that error, as
            // the `?` did.
            let trap_result = if !inner_defers_debug && self.debug_trap_in_scope() {
                let inner_text = bash_command_source_text(inner);
                self.run_debug_trap(crate::debugger::SeamKind::TimeInner, &inner_text)
                    .map(|_| ())
            } else {
                Ok(())
            };
            match trap_result {
                Ok(()) => {
                    // rubash#1611: the DEBUG trap above is the time
                    // wrapper's own pre-step, not part of the inner
                    // command — a conversion inside the trap action
                    // must not be read as the inner command's
                    // conversion mark.
                    self.errexit_status_converted.set(false);
                    self.execute_command(&time_command.command)
                }
                Err(error) => Err(error),
            }
        };
        self.timed_dispatch_depth
            .set(self.timed_dispatch_depth.get() - 1);
        // rubash#1611: take the errexit-conversion mark left by the
        // inner dispatch (cleared at entry above).
        let errexit_converted = self.errexit_status_converted.replace(false);
        let parse_error_during_inner = self.parse_error_occurred && !parse_error_before_inner;
        match inner_result {
            Ok(()) => {}
            // GNU execute_cmd.c execute_time_command reports the timed
            // command's status either way: a DISCARD-class expansion
            // failure inside it (expr.c evalerror / failglob) is the
            // command's failure status, not a shell unwind — time prints
            // its report and returns that status, while the armed
            // reader-level abandon still discards the rest of the line
            // (rubash#753 case 5: the discard penetrates `time`;
            // rubash#1240).
            Err(ExecuteError::ExpansionFailure(code)) => {
                self.exit_code = code;
            }
            Err(ExecuteError::ExitCode(code)) if errexit_converted => {
                // rubash#1611: the ExitCode is the dispatch-level
                // errexit conversion of a COMPLETED inner command
                // (execute_materialized_command's tail and its siblings
                // in execute_command), not a jump out of the timed
                // command. GNU's execute_time_command completes and
                // prints its report; the errexit jump belongs to ITS
                // caller (the ast_exec time branch re-adjudicates after
                // flushing the queued report). A genuine unwind (exit
                // builtin, FORCE_EOF) leaves the mark clear and takes
                // the print-and-propagate arm below.
                self.exit_code = code;
            }
            Err(ExecuteError::ExitCode(code)) if !parse_error_during_inner => {
                // rubash#1611: a genuine runtime unwind whose target is
                // NOT inside this frame — the exit builtin (and exit
                // reached through eval, a brace group, or a function).
                // GNU's execute_time_command still prints its report
                // before the jump takes effect (verified against GNU
                // bash 5.3.20: `time exit 3`, `time { exit 3; }`,
                // `time eval "exit 3"` all print, rc preserved). The
                // report cannot ride the defer queue — the caller's
                // unwind arm discards it — so print it now, then let
                // the unwind proceed. A parse-diagnostic ExitCode is
                // excluded above: the timed command never executed, and
                // GNU prints no report for it.
                self.exit_code = code;
                if !delegated_report {
                    self.print_time_shell(time_command.posix_format, started);
                }
                return Err(ExecuteError::ExitCode(code));
            }
            Err(error @ (ExecuteError::Break(_) | ExecuteError::Continue(_))) => {
                // rubash#1611: break/continue unwind to the enclosing
                // loop, outside this frame — GNU's execute_time_command
                // prints its report first (`time break` in a loop
                // prints, verified against GNU bash 5.3.20). Print now
                // (the caller's unwind arms discard the defer queue),
                // then propagate.
                if !delegated_report {
                    self.print_time_shell(time_command.posix_format, started);
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        }
        // rubash#1528: when the timed inner was a coproc that is still
        // running, defer the report to the job-completion funnel
        // (flush_timed_coproc_report) instead of measuring the spawn.
        // A coproc already reaped by the time control reaches here
        // (no longer in background_children) keeps the immediate print:
        // its lifetime is the elapsed interval either way.
        let running_timed_coproc = if timed_coproc_shape {
            self.background_children.keys().copied().find(|pid| {
                !children_before.contains(pid) && self.shell_state.coproc_names.contains_key(pid)
            })
        } else {
            None
        };
        if delegated_report {
            // rubash#1611: pure time chain — the innermost frame owns
            // the report; this frame emits none (no queue entry either,
            // or the caller's flush would print it anyway).
        } else if let Some(pid) = running_timed_coproc {
            self.pending_timed_coproc_reports
                .push((pid, started, time_command.posix_format));
        } else if self.defer_list_time_report {
            // rubash#1277: under the list loop's defer flag the report is
            // queued, not printed — the loop flushes it after the ERR trap
            // adjudication (GNU fires the trap inside the inner
            // execute_command, before execute_time_command's print).
            self.pending_list_time_reports
                .push((started, time_command.posix_format));
        } else {
            self.print_time_shell(time_command.posix_format, started);
        }
        if time_command.inverted {
            // rubash#846 (GNU execute_cmd.c:854-883 vs :1204): a compound
            // inner's OWN redirect failure suppresses the `!` flip (mark
            // left set for an outer `! !` wrapper, as elsewhere).
            if !self.compound_redirect_failed.get() {
                self.exit_code = invert_exit_status(self.exit_code);
            }
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
        // Classification already applied the parse-time-frozen #1218
        // decision (command_is_time_prefixed_compound, rubash#1520), so
        // prefix extraction here must not re-veto against the live mode.
        let inverted = time_prefix_parts(cmd, false)
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
        self.print_time_shell(
            time_prefix_parts(cmd, false).is_some_and(|parts| parts.posix_format),
            started,
        );
        result?;
        if inverted {
            // rubash#846 (GNU execute_cmd.c:854-883 vs :1204): a compound
            // inner's OWN redirect failure suppresses the `!` flip (mark
            // left set for an outer `! !` wrapper, as elsewhere).
            if !self.compound_redirect_failed.get() {
                self.exit_code = invert_exit_status(self.exit_code);
            }
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_time_prefixed_command_sequence(
        &mut self,
        commands: &[CommandNode],
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let Some(command) = commands.get(index) else {
            return Ok(None);
        };
        // rubash#1355: an assignment prefix makes `time` a plain command
        // name (GNU parse.y), never the timing keyword.
        if !command.assignments.is_empty()
            || !command.compound_assignments.is_empty()
            || !command.array_element_assignments.is_empty()
        {
            return Ok(None);
        }
        // rubash#1520: classification uses the parse-time-frozen #1218
        // decision recorded on the node, not the live mode.
        if time_dash_word_not_keyword(command) {
            return Ok(None);
        }
        let Some(prefix) = time_prefix_parts(command, false) else {
            return Ok(None);
        };
        if !matches!(
            command.words.get(prefix.command_index).map(String::as_str),
            Some("if" | "while" | "until")
        ) {
            return Ok(None);
        }

        let mut timed_ast = Ast {
            commands: commands[index..].to_vec(),
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
            Some("if") => crate::builtins::source::execute_simple_if(self, &timed_ast.commands, 0)?,
            Some("while" | "until") => self.execute_simple_loop(&timed_ast.commands, 0)?,
            _ => None,
        };
        let Some(next_index) = next_index else {
            return Ok(None);
        };

        self.print_time_shell(prefix.posix_format, started);
        if prefix.inverted {
            // rubash#846 (GNU execute_cmd.c:854-883 vs :1204): a compound
            // inner's OWN redirect failure suppresses the `!` flip (mark
            // left set for an outer `! !` wrapper, as elsewhere).
            if !self.compound_redirect_failed.get() {
                self.exit_code = invert_exit_status(self.exit_code);
            }
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
        // wt34/perf2: the capture is the parsed line number (GNU's C int),
        // not the env entry's String — the restore is then one equality-
        // gated write through the single CURRENT_LINE author instead of a
        // key+value allocation pair per expression evaluation, and the
        // body's ambient line below reuses the parsed value instead of
        // re-parsing the string once per iteration.
        let for_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok());
        let restore_for_line = |executor: &mut Executor| {
            if let Some(line) = for_line {
                executor.set_current_line_value(line);
            }
        };
        // execute_cmd.c:3191-3199 (eval_arith_for_expr): a non-zero DEBUG
        // status under debugging_mode makes the expression evaluate as 0
        // with *okp = 1 — the evaluation is skipped, not failed.
        let init_skipped = if self.debug_trap_in_scope() && !arithmetic.init.trim().is_empty() {
            restore_for_line(self);
            // An Err here unwinds past the loop: balance the ENTRY
            // increment before returning (single-exit invariant,
            // execute_cmd.c:3296; see the body match below, #1911).
            match self.run_debug_trap(crate::debugger::SeamKind::ArithForInit, &arithmetic.init) {
                Ok(outcome) => outcome.is_some(),
                Err(error) => {
                    self.shell_state.loop_depth -= 1;
                    return Err(error);
                }
            }
        } else {
            false
        };
        if !init_skipped && !arithmetic.init.trim().is_empty() && {
            // GNU execute_cmd.c:3201 (eval_arith_for_expr): if `set -x`
            // is on, print `(( expr ))` before evaluating each for-loop
            // expression (init, test, update).  Use the raw expression to
            // preserve original whitespace (e.g. `i++ ` trailing space).
            // execute_cmd.c:3236 keeps line_number at the arith-for's own
            // line across iterations (execute_command_internal restores it
            // after each body command), so the same restore that precedes
            // the DEBUG fire must precede this trace — PS4's $LINENO renders
            // the for line, not the line the last body command left behind
            // (rubash#275: iteration-2 `(( i++ ))` printed the body line).
            restore_for_line(self);
            self.xtrace_print_arith_cmd(&arithmetic.init_metadata.expression);
            self.eval_arithmetic_command_value(&arithmetic.init)
                .is_none()
        } {
            self.report_arithmetic_error_raw_display(&arithmetic.init_metadata.expression);
            self.exit_code = 1;
            arithmetic_failed = true;
        }

        let mut ran_body = false;
        // Canonical-counter fast loop (see run_arith_counter_fast_loop): the
        // slow init above already ran with full semantics (DEBUG fire,
        // xtrace, failure handling), so a failed init never reaches here.
        if !arithmetic_failed {
            if let Some(counter) = self.try_enter_arith_counter_fast_loop(arithmetic, body) {
                let result = self.run_arith_counter_fast_loop(&counter, body, for_line);
                // Mirror the function tail below: the entry `loop_depth += 1`
                // is balanced here; the fast loop always ends with status 0.
                self.shell_state.loop_depth -= 1;
                return result;
            }
        }
        // Header strings are borrowed from the parsed node, so the body
        // cannot mutate them: trim once per loop entry. GNU eval_arith_for_
        // expr (execute_cmd.c:3163) consults the trap table per evaluation;
        // the probe is its cheap gate (no DEBUG action means run_debug_trap
        // returns None), re-probed after the body, which can change it.
        let test_empty = arithmetic.test.trim().is_empty();
        let update_empty = arithmetic.update.trim().is_empty();
        while !arithmetic_failed {
            // execute_cmd.c:3191-3199: the skipped test evaluates as 0 —
            // the loop exits like any failing test (probe k2/m-series).
            let mut debug_may_fire = crate::builtins::trap::has_active_trap_action(
                &self.shell_state.env_vars,
                "DEBUG",
                "__RUBASH_TRAP_DEBUG",
            );
            let mut test_skipped = false;
            if !test_empty
                && ((debug_may_fire && self.debug_trap_in_scope()) || self.debugger_session_here())
            {
                restore_for_line(self);
                // Balance the ENTRY increment on an unwinding Err
                // (single-exit invariant, execute_cmd.c:3296; #1911).
                match self.run_debug_trap(crate::debugger::SeamKind::ArithForTest, &arithmetic.test)
                {
                    Ok(outcome) => test_skipped = outcome.is_some(),
                    Err(error) => {
                        self.shell_state.loop_depth -= 1;
                        return Err(error);
                    }
                }
            }
            if test_skipped {
                break;
            }
            if !test_empty {
                let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_FOR_TEST);
                // GNU execute_cmd.c:3201: xtrace before test expression.
                // Use raw expression to preserve original whitespace. The
                // line restore before it mirrors the DEBUG fire above
                // (execute_cmd.c:3236, rubash#275).
                restore_for_line(self);
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
            // rubash#1390: NO second loop_depth increment for the body —
            // the entry increment above already spans the whole arith-for
            // (GNU execute_arith_for_command bumps loop_level once,
            // before init). Counting the body again made the body see
            // depth 2 for a single loop, so `break 2`/`continue 2` were
            // not clamped to the real depth and escaped as a spurious
            // Break(1)/Continue(1) that aborted the script.
            let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_FOR_BODY);
            // execute_cmd.c:3236: line_number = arith_for_command->line is
            // the ambient for the body's non-line-setting commands. GNU
            // execute_arith_for_command (execute_cmd.c:3219) re-walks the
            // stored body by POINTER per iteration; execute_ast_inner walks
            // the borrowed slice the same way (while/until and for-in bodies
            // already skip the reader-level wrapper through it).
            let result =
                self.with_ambient_line(for_line, |executor| executor.execute_ast_inner(body));
            drop(_t);
            match result {
                Ok(()) => {}
                Err(ExecuteError::Break(level)) if level <= 1 => {
                    self.exit_code = self.take_break_failure_status().unwrap_or(0);
                    break;
                }
                // Propagating past this loop: balance the ENTRY
                // increment before returning (the function tail's
                // decrement is skipped by the early return), or the
                // leaked depth makes later `break`s look in-loop.
                Err(ExecuteError::Break(level)) => {
                    self.shell_state.loop_depth -= 1;
                    return Err(ExecuteError::Break(level - 1));
                }
                Err(ExecuteError::Continue(level)) if level <= 1 => {
                    self.exit_code = 0;
                }
                Err(ExecuteError::Continue(level)) => {
                    self.shell_state.loop_depth -= 1;
                    return Err(ExecuteError::Continue(level - 1));
                }
                // Any other unwind crossing this loop (an expansion
                // failure from the body, an exit/return jump, ...):
                // balance the ENTRY increment before returning, like
                // the Break/Continue arms above. GNU
                // execute_arith_for_command (execute_cmd.c:3219) has a
                // single exit whose loop_level-- (execute_cmd.c:3296)
                // runs unconditionally — the body result there is a
                // status, never an escape past the frame. Leaving the
                // depth raised here defeats the enclosing list walk's
                // DISCARD absorption, whose guard reads
                // loop_depth == 0: an evalerror raised in the body
                // (expr.c evalerror -> jump_to_top_level (DISCARD))
                // then escapes as a script-fatal error instead of
                // discarding just this for command (rubash#1911).
                Err(error) => {
                    self.shell_state.loop_depth -= 1;
                    return Err(error);
                }
            }

            // execute_cmd.c:3191-3199: the skipped update evaluates as 0 —
            // the index variable keeps its value, so the next test sees the
            // unchanged index (probe k2: the body re-ran with the same `i`).
            // Re-probe: the body above may have installed/removed the trap.
            debug_may_fire = crate::builtins::trap::has_active_trap_action(
                &self.shell_state.env_vars,
                "DEBUG",
                "__RUBASH_TRAP_DEBUG",
            );
            let update_skipped = if !update_empty
                && ((debug_may_fire && self.debug_trap_in_scope()) || self.debugger_session_here())
            {
                restore_for_line(self);
                // Balance the ENTRY increment on an unwinding Err
                // (single-exit invariant, execute_cmd.c:3296; #1911).
                match self.run_debug_trap(
                    crate::debugger::SeamKind::ArithForUpdate,
                    &arithmetic.update,
                ) {
                    Ok(outcome) => outcome.is_some(),
                    Err(error) => {
                        self.shell_state.loop_depth -= 1;
                        return Err(error);
                    }
                }
            } else {
                false
            };
            if !update_skipped && !update_empty {
                let _t = super::exec_profile::PhaseTimer::new(&super::exec_profile::P_FOR_UPDATE);
                // GNU execute_cmd.c:3201: xtrace before update expression.
                // Use raw expression to preserve original whitespace. The
                // line restore before it mirrors the DEBUG fire above
                // (execute_cmd.c:3236, rubash#275).
                restore_for_line(self);
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

    /// Admission probe for the canonical-counter fast loop: `Some` admits,
    /// `None` is doubt and the caller runs the slow loop unchanged.
    /// Whitelist only (rubash#117): the counter shapes
    /// (`arith_counter_loop_shapes`), a single bare-`:` body, the canonical
    /// init binding left by the slow init, clean variable marks, an empty
    /// arithmetic write log, no installed trap actions, no tracing, an
    /// enabled builtin-`:` target and no background jobs. Every condition
    /// the skipped per-iteration work could observe is verified here; the
    /// `:` body cannot change any of them (it runs no code at all on the
    /// fast path), and no trap handler can run either, so entry validation
    /// holds for the whole loop by induction.
    fn try_enter_arith_counter_fast_loop(
        &self,
        arithmetic: &ArithmeticForCommand,
        body: &[CommandNode],
    ) -> Option<crate::executor::arithmetic::ArithCounterLoop> {
        for_body_is_bare_colon(body)?;
        let counter = crate::executor::arithmetic::arith_counter_loop_shapes(
            &arithmetic.init,
            &arithmetic.test,
            &arithmetic.update,
        )?;
        // The slow init ran first: the binding must be exactly its canonical
        // decimal rendering, proving the variable reads plainly.
        let rendered = counter.init.to_string();
        if self
            .shell_state
            .env_vars
            .get(&counter.var)
            .map(String::as_str)
            != Some(rendered.as_str())
        {
            return None;
        }
        if crate::executor::arithmetic::arith_counter_var_doubtful(
            &self.shell_state.env_vars,
            &counter.var,
        ) {
            return None;
        }
        if !crate::executor::arithmetic::arith_write_log_is_empty() {
            return None;
        }
        if self.arith_counter_trap_installed() {
            return None;
        }
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "xtrace")
            || crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "verbose")
        {
            return None;
        }
        // `enable -n :` is mishandled by the slow path today (rubash still
        // runs the builtin where GNU reports 127); a function named `:`
        // overrides the builtin in both shells. Either doubt runs slow so
        // the fast loop can never diverge from the slow one.
        if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, ":")
            || self.shell_state.functions.contains_key(":")
        {
            return None;
        }
        if !self.shell_state.job_table.jobs.is_empty() || !self.background_children.is_empty() {
            return None;
        }
        Some(counter)
    }

    /// Whether any trap action is installed. The fast loop executes no
    /// commands, so a trapped signal delivered mid-loop would sit in the
    /// mailbox until the loop ends while the slow path dispatches it at the
    /// next `:` boundary. Any non-empty `__RUBASH_TRAP_*` action —
    /// DEBUG/ERR/EXIT/RETURN/signal alike, including reset-suppressed ones
    /// (doubt runs slow) — disqualifies. Empty actions (`trap '' SIG` =
    /// SIG_IGN) install no shell code and stay admissible: dispositions
    /// cannot change without a `trap` call, which nothing on the fast path
    /// can issue. Metadata keys (`TRAP_RESET`, `TRAP_ORIG_IGNORES`) are not
    /// actions and are skipped.
    fn arith_counter_trap_installed(&self) -> bool {
        use crate::builtins::trap::{TRAP_ORIG_IGNORES, TRAP_PREFIX, TRAP_RESET};
        self.shell_state.env_vars.iter().any(|(key, action)| {
            key.starts_with(TRAP_PREFIX)
                && key.as_str() != TRAP_RESET
                && key.as_str() != TRAP_ORIG_IGNORES
                && !action.is_empty()
        })
    }

    /// Fast driver for canonical `for ((v=C0; v<C1; v++)); do :; done`
    /// loops admitted by `try_enter_arith_counter_fast_loop`. GNU
    /// execute_arith_for_command (execute_cmd.c:3219) re-evaluates the test
    /// and update through expr.c descent and dispatches the `:` body through
    /// execute_command_internal on every iteration; the fast loop performs
    /// the identical counter directly (i64 wrapping, mirroring bash_arith),
    /// the identical update write pipeline (env insert, write-log record,
    /// process-env mirror, typed-cell sync — the same calls in the same
    /// order as the slow `PostInc` path) and the identical net end-state
    /// (line/command stamps, `_`, status 0). Per-iteration work the slow
    /// path repeats — the DEBUG/line/xtrace probes, the arithmetic ticket
    /// (pre-expansion scans, nounset machinery, dynamic-context snapshot,
    /// parse and diagnostics) and the full `:` dispatch — is provably
    /// unobservable here: no DEBUG action or trace can fire (gated at
    /// entry, nothing mid-loop can arm one), the counter variable stays
    /// canonical (only fast writes touch it), and no jobs, traps or
    /// failures can appear. A fatal signal soldiers through the periodic
    /// mailbox drain exactly as the slow body's per-command boundary does
    /// (delivery timing is inherently racy either way).
    fn run_arith_counter_fast_loop(
        &mut self,
        counter: &crate::executor::arithmetic::ArithCounterLoop,
        body: &[CommandNode],
        for_line: Option<usize>,
    ) -> Result<(), ExecuteError> {
        // Pure-function admission over the same slice: entry verified a bare
        // `:` body, so this re-derivation yields the same node (precedent:
        // the `changed implies present` expect).
        let colon = for_body_is_bare_colon(body).expect("fast-loop entry admits only bare-`:`");
        let var = counter.var.clone();
        let mut current = counter.init;
        let mut iters: u64 = 0;
        // GNU pins the for line as the ambient line_number while the body
        // runs (execute_cmd.c:3236); the envelope keeps that net effect
        // while nothing inside observes it.
        let result: Result<(), ExecuteError> = self.with_ambient_line(for_line, |executor| {
            loop {
                if current >= counter.bound {
                    break Ok(());
                }
                // Signal-death parity with the slow body's per-command
                // boundary (see above); with no actions installed only the
                // fatal-delivery Err paths can trigger, which propagate
                // exactly as a slow body error does. The 1024-iteration tick
                // amortizes the mailbox poll to ~zero per iteration.
                if iters & 1023 == 0 {
                    executor.run_pending_signal_traps()?;
                }
                // Wrapping increment mirrors bash_arith (`value as i64`).
                let updated = current.wrapping_add(1);
                let new_value = updated.to_string();
                // By induction `old` is exactly the stored rendering: entry
                // verified the canonical init and only fast writes (same
                // rendering) touch the binding mid-loop.
                let old_value = Some(current.to_string());
                executor
                    .shell_state
                    .env_vars
                    .insert(var.clone(), new_value.clone());
                crate::executor::arithmetic::record_arith_write(&var, old_value);
                crate::executor::sync_shell_assignment_process_env(
                    &executor.shell_state.env_vars,
                    &var,
                    &new_value,
                );
                crate::executor::arithmetic::sync_arith_writes(executor);
                current = updated;
                iters += 1;
            }
        });
        result?;
        if iters > 0 {
            // The skipped `:` iterations each stamped these byte-identical
            // values; one call each leaves the identical net state (all
            // three helpers are equality-gated no-ops on repeat).
            self.set_current_line(colon);
            self.set_current_command(colon);
            self.bind_underscore(":");
        }
        // Net CURRENT_LINE of the slow loop: the update-restore stamps the
        // for line after the last body (and the failing test-restore stamps
        // it on a zero-trip loop).
        if let Some(line) = for_line {
            self.set_current_line_value(line);
        }
        // Every fast evaluation succeeds, as the slow ones provably do for
        // admitted shapes; no diagnostic slot is ever written.
        self.shell_state.arithmetic_last_error_category.set(None);
        // Verified against GNU 5.3: the loop always ends 0 here — a ran body
        // of `:` leaves 0, and a zero-trip loop returns 0 (`false;
        // for ((i=5;i<3;i++)); do :; done; echo $?` prints 0).
        self.exit_code = 0;
        Ok(())
    }

    pub(in crate::executor) fn execute_if_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        if_command: &IfCommand,
    ) -> Result<(), ExecuteError> {
        // RESTRICTED_SHELL (#804): the compound's redirects apply before
        // the body runs, so the refusal must land before any clone, splice
        // or target creation — GNU leaves no file behind and never runs
        // the body.
        if compound_command_has_redirects(cmd) {
            if let Some(message) = self.restricted_compound_redirect_error(cmd)? {
                self.write_default_stderr(&message)?;
                self.exit_code = 1;
                return Ok(());
            }
        }
        if self.if_command_needs_alias_scan(if_command) {
            let flat = flatten_if_command_for_alias_scan(cmd, if_command);
            crate::builtins::source::execute_simple_if(self, &flat, 0)?;
            return Ok(());
        }

        // rubash#619: GNU execute_cmd.c execute_if_command walks the stored
        // condition/branch tree by POINTER and applies redirections only when
        // the statement carries them. With no redirect/procsub fields on `cmd`
        // the clone of the ~40-field CommandNode plus the deep clone of
        // if_command (condition/then/elif/else CommandNode trees) and the
        // guaranteed no-op redirect application are pure per-execution
        // overhead (measured: `if :; then :; fi` +10.9us/op vs GNU 1.3us/op).
        if !compound_command_has_redirects(cmd) {
            return self.execute_if_command(if_command);
        }

        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut if_command = if_command.clone();
        // rubash#846 (GNU execute_cmd.c:854-883): an apply-stage IoError
        // is the if's OWN redirect failure — mark it for the `!` flip
        // sites. Body-stage errors flow through unmarked and flip normally.
        let result = match apply_if_redirects(self, &redirect_cmd, &mut if_command) {
            Err(error) => {
                if matches!(error, ExecuteError::IoError(_)) {
                    self.note_compound_redirect_failure();
                }
                Err(error)
            }
            // rubash#1867: numbered heredoc/herestring staging wraps
            // every compound executor (GNU do_redirections binds them
            // for the compound's lifetime) — the loop wrap is the
            // template; if/case/for/select carried no staging at all.
            Ok(()) => self.with_loop_fd_heredocs(cmd, |executor| {
                executor.with_command_input_redirects(cmd, |executor| {
                    executor.execute_if_command(&if_command)
                })
            }),
        };
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
        // RESTRICTED_SHELL (#804): refuse before any splice or creation.
        if compound_command_has_redirects(cmd) {
            if let Some(message) = self.restricted_compound_redirect_error(cmd)? {
                self.write_default_stderr(&message)?;
                self.exit_code = 1;
                return Ok(());
            }
        }
        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut loop_command = loop_command.clone();
        // rubash#846 (GNU execute_cmd.c:854-883): an apply-stage IoError
        // is the loop's OWN redirect failure — mark it for the `!` flip
        // sites. Body-stage errors flow through unmarked and flip normally.
        let result = match apply_redirects_to_commands(self, &redirect_cmd, &mut loop_command.body)
        {
            Err(error) => {
                if matches!(error, ExecuteError::IoError(_)) {
                    self.note_compound_redirect_failure();
                }
                Err(error)
            }
            Ok(()) => self.with_loop_fd_heredocs(cmd, |executor| {
                executor.with_command_input_redirects(cmd, |executor| {
                    executor.execute_loop_command(&loop_command)
                })
            }),
        };
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
        let saved_state = self.shell_state.clone_for_child_save();
        // rubash#1551: the fork bracket also covers the process-global
        // hash-table mirror (see executor::path::hashed_cache_fork_guard).
        let _hashed_fork = crate::executor::path::hashed_cache_fork_guard();
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
        // this executor; since G-02 the cwd is a ShellState field, so the
        // saved_state clone below carries the boundary (niubash#100).
        // rubash#439: the body's word-expansion error mark dies with the
        // forked child (GNU's DISCARD jump cannot cross the fork boundary) —
        // the parent judges the subshell only by its exit STATUS, so
        // `(x=${!bad})` still fires the parent's ERR trap.
        let saved_expansion_failed = self.command_expansion_failed.get();
        self.command_expansion_failed.set(false);
        // rubash#1012: the body runs in place on this executor, but GNU
        // forked it — an `exec` in the body replaces only the child's image
        // (setting the child's exec_replaced latch), while the parent keeps
        // its own EXIT trap. Save/restore the latch like the fd table so a
        // subshell exec cannot disarm the parent's trap.
        let saved_exec_replaced = self.exec_replaced;
        crate::builtins::trap::reset_for_subshell_child(&mut self.shell_state.env_vars);
        self.shell_state.subshell_depth.set(saved_depth + 1);
        // rubash#1493: GNU's signal-death notice for a job killed inside
        // the parens names the WHOLE subshell command (jobs.c:4625
        // pretty_print_job on the dead job). Record the node's print_cmd
        // text for the exec-death queue site in trap_exec.rs; the entry
        // clone saved the parent's value, so the wholesale boundary
        // restore reverts this automatically.
        self.shell_state.subshell_source_text =
            Some(crate::executor::command_text::bash_command_source_text(cmd));
        // rubash#1115: the forked child reseeds RANDOM
        // (variables.c:1418-1423 get_random_number); saved_state keeps
        // the parent's sequence for the wholesale restore.
        self.shell_state.random_state.reseed_fresh();
        self.shell_state.loop_depth = 0;
        // execute_in_subshell resets loop_level (execute_cmd.c:1642), so
        // fork-inherited depth does not cross a `( )` boundary either.
        self.shell_state.inherited_loop_depth = 0;

        // RESTRICTED_SHELL (#804): the subshell's redirects apply inside
        // the child before the body (do_redirections precedes the list), so
        // the refusal lands before any splice, anchor or target creation —
        // nothing is created and the body never runs, like GNU's child
        // dying at do_redirections with status 1.
        if compound_command_has_redirects(cmd) {
            if let Some(message) = self.restricted_compound_redirect_error(cmd)? {
                self.write_default_stderr(&message)?;
                self.exit_code = 1;
                return Ok(());
            }
        }

        // Assignment prefix (`x=1 (cmd)`, #452): GNU apply_shell_assignments
        // runs the prefix inside the forked child, so the values live only
        // for the body; the whole-state save above restores the parent
        // afterwards. Scalar assignments only — compound/array-element
        // prefixes have no GNU verdict in the audit corpus yet and never
        // reach the parser's subshell gate.
        for (name, value) in &cmd.assignments {
            let result = self.expand_assignment_value_result_with_raw(name, value, None);
            if !self.apply_shell_assignment(name, result.value) {
                self.exit_code = 1;
                return Err(ExecuteError::ExpansionFailure(1));
            }
        }

        // GNU executes the `( list )` body by POINTER (execute_in_subshell
        // walks the Subshell->command the parser allocated once; make_cmd.c
        // never copies). The clones below exist only to feed mutations:
        // materialize_compound_output_process_substitutions rewrites a
        // target only when it starts with `>(` (both arms require that
        // prefix — see execute_brace_group_pipeline), and
        // apply_command_output_redirects splices into the body only for a
        // stdio-slot redirect; with neither, the stored slice runs as-is.
        let redirect_needs_ps_rewrite = [
            cmd.redirect_out.as_deref(),
            cmd.append.as_deref(),
            cmd.redirect_err.as_deref(),
            cmd.redirect_err_append.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|redirect| redirect.target.starts_with(">("));
        let mut redirect_owned;
        let group_outputs;
        let redirect_cmd: &CommandNode = if redirect_needs_ps_rewrite {
            redirect_owned = cmd.clone();
            group_outputs =
                self.materialize_compound_output_process_substitutions(&mut redirect_owned)?;
            &redirect_owned
        } else {
            group_outputs = Vec::new();
            cmd
        };
        // Numbered redirects are duplicated in parser stdio fields. Keep only
        // true stdio fields in compound preparation; the original redirect
        // list is applied to each body command below. Conservative admission:
        // any stdio slot present means apply_command_output_redirects may
        // splice, so the body is copied; a slot-free subshell borrows.
        let body_splice_needed = cmd.redirect_out.is_some()
            || cmd.append.is_some()
            || cmd.redirect_err.is_some()
            || cmd.redirect_err_append.is_some();
        let mut stdio_redirect_cmd = redirect_cmd.clone();
        // Compound redirections on fd 0-2 keep their stdio mirror fields;
        // only a genuinely NUMBERED redirect (fd > 2) is stripped, because
        // those propagate exclusively through the ordered `redirects` list
        // (the mirror fields are fd-blind — a `3>&1` can land in
        // `redirect_out`). Treating the explicit fd digit of `2>`/`1>&2` as
        // "numbered" stripped fd-1/fd-2 redirections too, defeating both the
        // per-leaf splice and `cd`'s redirected-io binding: a subshell
        // `( cd /no_such ) 2>e` leaked the diagnostic to the real stderr
        // (rubash#699 R1.4).
        let is_numbered = |redirect: &Redirect| {
            let fd = redirect.fd.unwrap_or_else(|| {
                redirect
                    .operator
                    .chars()
                    .take_while(|ch| ch.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            });
            fd > 2
        };
        if stdio_redirect_cmd
            .redirect_in
            .as_deref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_in = None;
        }
        if stdio_redirect_cmd
            .redirect_out
            .as_deref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_out = None;
        }
        if stdio_redirect_cmd
            .append
            .as_deref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.append = None;
        }
        if stdio_redirect_cmd
            .redirect_err
            .as_deref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_err = None;
        }
        if stdio_redirect_cmd
            .redirect_err_append
            .as_deref()
            .is_some_and(is_numbered)
        {
            stdio_redirect_cmd.redirect_err_append = None;
        }
        let mut body_owned: Option<Vec<CommandNode>> = None;
        if body_splice_needed {
            let mut body_ast = Ast {
                commands: subshell_command.body.clone(),
            };
            // rubash#846 (GNU execute_cmd.c:1759-1762): the subshell's own
            // redirect failure in the forked child exits inverted — the
            // parent wait-side (:763) flips it back, netting status 1 with
            // no flip. Mark it so an enclosing `!` suppresses its flip.
            if let Err(error) =
                self.apply_command_output_redirects(&stdio_redirect_cmd, &mut body_ast)
            {
                if matches!(error, ExecuteError::IoError(_)) {
                    self.note_compound_redirect_failure();
                }
                // rubash#940: the subshell boundary owns its saved state on
                // every exit path — GNU's forked child dies with its own
                // copy and nothing leaks back (execute_cmd.c:1576). The
                // early apply-failure return skipped the restore below and
                // leaked subshell_depth + the child trap reset into the
                // parent.
                self.restore_flat_subshell(saved_state);
                self.fd_table = saved_fd_table;
                self.command_expansion_failed.set(saved_expansion_failed);
                self.exec_replaced = saved_exec_replaced;
                return Err(error);
            }
            body_owned = Some(body_ast.commands);
        }
        let body: &[CommandNode] = body_owned.as_deref().unwrap_or(&subshell_command.body);
        // rubash#720 form 1: GNU execute_cmd.c:1799-1806 (execute_in_subshell)
        // marks a user subshell's WHOLE body CMD_NO_FORK when it is a single
        // simple or subshell command; execute_cmd.c:662-663 runs a subshell
        // body so marked IN PLACE inside the already-forked child — no
        // second fork, no wait_for, and therefore no :769 run_error_trap in
        // that frame. With `( (false) )` the middle frame never observes the
        // status, so only the innermost `false` and the outermost waiter
        // fire ERR (GNU 5.3.0: 2 fires, not 3). The flag scopes exactly the
        // body execution of a single-subshell-command body.
        let body_fork_collapsed = body.len() == 1 && body[0].subshell_command.is_some();
        let saved_fork_collapse = if body_fork_collapsed {
            Some(self.subshell_fork_collapse.replace(true))
        } else {
            None
        };
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
                    // rubash#815 profiler: the flat-subshell frame (GNU
                    // execute_cmd.c:1576 execute_in_subshell). The guard
                    // spans the EXIT trap too — GNU runs it inside the
                    // forked child.
                    let _profiler_subshell = super::profiler::enter_subshell();
                    let body_result = executor.execute_ast_inner(body);
                    // GNU execute_cmd.c execute_in_subshell: expr.c
                    // evalerror's jump_to_top_level(DISCARD) reaches only the
                    // forked subshell's own top level — the abort dies with
                    // the subshell (status 1) and cannot discard parent
                    // commands (`( a[$bad]=v ); echo after` prints `after` —
                    // verified GNU 5.3).
                    executor.evalerror_pending.set(false);
                    executor.nounset_force_eof.set(false);
                    executor.evalerror_subscript_pending.set(false);
                    executor.evalerror_failglob_pending.set(false);
                    executor.evalerror_line.set(None);
                    // rubash#1541: a no_args DISCARD raised inside the
                    // region, when the region itself sits inside a
                    // command-substitution body, folds to 1: the forked
                    // region inherits the body's parse_and_execute frame,
                    // whose DISCARD catch forces EXECUTION_FAILURE
                    // (builtins/evalstring.c:423-426). At the main level
                    // the same region keeps the pre-set EX_BADUSAGE 2
                    // (execute_in_subshell, execute_cmd.c:1821-1825).
                    if executor.evalerror_exit_toomany_pending.get()
                        && executor.discard_fold_frame()
                    {
                        executor.evalerror_exit_toomany_pending.set(false);
                        executor.exit_code = 1;
                    }
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
                    // rubash#1461: this layer was killed by a signal (a
                    // `kill` to its fake BASHPID, intercepted in
                    // execute_kill). A GNU process killed by a signal runs
                    // no EXIT trap; consume the death mark here and let
                    // the code after the state restore queue the parent's
                    // jobs.c notice instead.
                    if executor.subshell_signal_death == Some((saved_depth + 1, status - 128))
                        && status >= 128
                    {
                        trap_result = None;
                        return Ok(());
                    }
                    // GNU execute_cmd.c runs the subshell's own EXIT trap
                    // (trap.c run_exit_trap) while the subshell state is
                    // still live: `( trap "echo T" EXIT; echo body )` prints
                    // body then T (niubash#70). reset_for_subshell cleared
                    // the inherited traps at entry, so a pending EXIT here
                    // can only have been registered by the body itself; a
                    // trap action that calls exit N replaces the subshell
                    // status (bash exit_shell semantics).
                    trap_result = Some(executor.run_exit_trap_for_status_with_output_redirects(
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
                if let Some(saved) = saved_fork_collapse {
                    self.subshell_fork_collapse.set(saved);
                }
                self.restore_flat_subshell(saved_state.clone());
                self.fd_table = saved_fd_table.clone();
                self.command_expansion_failed.set(saved_expansion_failed);
                self.exec_replaced = saved_exec_replaced;
                return Err(error);
            }
        };
        let status = match trap_result {
            Some(Ok(trap_status)) => trap_status,
            Some(Err(error)) => {
                if let Some(saved) = saved_fork_collapse {
                    self.subshell_fork_collapse.set(saved);
                }
                self.restore_flat_subshell(saved_state.clone());
                self.fd_table = saved_fd_table.clone();
                self.command_expansion_failed.set(saved_expansion_failed);
                self.exec_replaced = saved_exec_replaced;
                return Err(error);
            }
            None => status,
        };

        if let Some(saved) = saved_fork_collapse {
            self.subshell_fork_collapse.set(saved);
        }
        self.restore_flat_subshell(saved_state);
        self.fd_table = saved_fd_table;
        self.command_expansion_failed.set(saved_expansion_failed);
        self.exec_replaced = saved_exec_replaced;
        // rubash#1461: this layer died by a signal (see the trap skip in
        // the body closure above) — queue the parent-side death notice
        // with the layer's fake BASHPID, like a reaped job's (jobs.c:4625
        // notify_of_job_status via note_child_signal_death).
        if let Some((depth, signal)) = self.subshell_signal_death {
            if depth == saved_depth + 1 {
                self.subshell_signal_death = None;
                let layer_pid = std::process::id().saturating_add(
                    u32::try_from(depth.saturating_sub(self.bashpid_origin_depth))
                        .unwrap_or(u32::MAX),
                );
                let text = self.signal_notice_command_text(cmd);
                self.note_child_signal_death(layer_pid, signal, text, true);
            }
        }
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        // rubash#846 (GNU execute_cmd.c:1808-1833 execute_in_subshell): the
        // forked child lifts CMD_INVERT_RETURN off the direct body and flips
        // the adjudicated result — `( ! C>bad )` is 0 while a top-level
        // `! C>bad` is 1. Only the single-inverted-command body qualifies
        // (tcom is the direct body; a `!` nested in a list keeps its flag
        // into execute_command_internal and hits the :854 early return).
        // Consume the mark when flipping; otherwise leave it for an
        // enclosing `!` (`! ( C>bad )` suppresses) or the flat-loop backstop.
        let mut status = status;
        if self.compound_redirect_failed.get()
            && body.len() == 1
            && (body[0].inverted_command.is_some() || body[0].inverted)
        {
            self.take_compound_redirect_failed();
            status = invert_exit_status(status);
        } else {
            // Body-set marks die with the forked child (GNU
            // execute_in_subshell): the parent judges the region by status
            // only. (The region's OWN redirect failure returns before this
            // point, leaving the mark for an enclosing `!`.)
            self.compound_redirect_failed.set(false);
        }
        self.exit_code = status;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    fn execute_loop_command(&mut self, loop_command: &LoopCommand) -> Result<(), ExecuteError> {
        let mut ran_body = false;
        let mut last_body_status = 0;
        // rubash#395: a break consumed by THIS loop ends it with the break
        // builtin's own status (GNU break.def: EXECUTION_FAILURE for an
        // out-of-range count); the tail normalization below must not
        // clobber it with last_body_status.
        let mut ended_by_break = false;
        // GNU execute_while_or_until (execute_cmd.c:3801+) walks BOTH the
        // condition and the body by POINTER — the COMMAND lists make_cmd.c
        // allocated once at parse are re-walked per iteration, never copied.
        // The clones this port paid once per loop (condition Vec clone plus
        // the body's normalize rewrite) reduce to: no clone for the
        // condition, and a body copy only when normalize_inline_compound_
        // commands would actually rewrite something. Its ONLY rewrite arm
        // (inline_arithmetic_for_command, builtins/source/flow.rs) requires a
        // command whose first word is literally `for`; a body with no such
        // word normalizes to itself, so the stored slice executes as-is.
        let body_needs_normalize = loop_command
            .body
            .iter()
            .any(|command| command.words.first().map(String::as_str) == Some("for"));
        let body_owned = body_needs_normalize.then(|| {
            crate::builtins::source::normalize_inline_compound_commands(loop_command.body.clone())
        });
        let body: &[CommandNode] = body_owned.as_deref().unwrap_or(&loop_command.body);

        loop {
            // GNU execute_cmd.c:3801 (execute_while_or_until) increments
            // loop_level before the first test evaluation, so `break` /
            // `continue` anywhere in the loop CONDITION are in-loop
            // (modernish bin/modernish:866 `while case $# in (0) break;;`).
            self.shell_state.loop_depth += 1;
            let condition_result = self.with_errexit_suppressed(|executor| {
                executor.execute_ast_inner(&loop_command.condition)
            });
            self.shell_state.loop_depth -= 1;
            match condition_result {
                Ok(()) => {}
                // GNU break-in-test: execute_cmd.c:635 makes the body a
                // no-op (returns last status, which `break` set to 0), then
                // the post-body `if (breaking) { breaking--; break; }` at
                // execute_cmd.c:3840 exits this loop. A level > 1 break
                // propagates one level outward.
                Err(ExecuteError::Break(level)) if level <= 1 => {
                    self.exit_code = self.take_break_failure_status().unwrap_or(0);
                    ended_by_break = true;
                    break;
                }
                Err(ExecuteError::Break(level)) => return Err(ExecuteError::Break(level - 1)),
                // GNU continue-in-test: post-body check at
                // execute_cmd.c:3846 decrements `continuing`; with 1 left
                // over it does not break, so the loop skips the body for
                // this iteration and re-evaluates the test.
                Err(ExecuteError::Continue(level)) if level <= 1 => {
                    self.exit_code = 0;
                    continue;
                }
                Err(ExecuteError::Continue(level)) => {
                    return Err(ExecuteError::Continue(level - 1));
                }
                Err(error) => return Err(error),
            }
            self.run_pending_signal_traps()?;
            let condition_matched = self.exit_code == 0;
            if condition_matched == loop_command.until {
                break;
            }

            ran_body = true;
            self.shell_state.loop_depth += 1;
            let result = self.execute_ast_inner(body);
            self.shell_state.loop_depth -= 1;
            self.run_pending_signal_traps()?;
            match result {
                Ok(()) => {
                    last_body_status = self.exit_code;
                }
                Err(ExecuteError::Break(level)) if level <= 1 => {
                    self.exit_code = self.take_break_failure_status().unwrap_or(0);
                    ended_by_break = true;
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
        } else if !ended_by_break {
            // GNU execute_cmd.c execute_while_or_until returns body_status
            // unconditionally — the loop's status is the last body command's,
            // not the final condition test's (rubash#1358: `until` exiting
            // on a true condition must still report the body's status).
            self.exit_code = last_body_status;
        }
        Ok(())
    }

    pub(in crate::executor) fn with_loop_fd_heredocs<T, F>(
        &mut self,
        cmd: &CommandNode,
        f: F,
    ) -> Result<T, ExecuteError>
    where
        F: FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    {
        let mut saved_fd_inputs = Vec::new();
        for redirect in &cmd.heredoc_redirects {
            let Some(fd) = redirect.fd else {
                continue;
            };
            if redirect.body.is_none() && redirect.body_carrier.is_none() {
                continue;
            }
            // One canonical expansion for every staged numbered
            // heredoc/herestring (expand_heredoc_redirect_text): the
            // typed carrier wins (a Preexpanded body was already
            // expanded at the do_redirections point and must not expand
            // again), a here-string's storage word decodes/expands and
            // gains its trailing newline. Feeding the raw stored body
            // to the heredoc expansion leaked the storage prefix into
            // the stream and lost the last line (rubash#1867).
            let body = self.expand_heredoc_redirect_text(redirect);
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
        // rubash#661: GNU parse.y alias_expand_token passes words through
        // untouched when the alias table is empty (and words from the
        // grouped alias-stream driver are already expanded, so executor
        // expansion is the identity there too) — the raw first word IS the
        // expanded first word. Skip the per-command words.to_vec() plus the
        // per-word table probes of expand_aliases in that case.
        if self.shell_state.aliases.is_empty() || self.alias_streamed() {
            return commands.iter().any(|command| {
                matches!(
                    command.words.first().map(String::as_str),
                    Some("if" | "then" | "elif" | "else" | "fi")
                )
            });
        }
        commands.iter().any(|command| {
            let words = self.expand_aliases(&command.words);
            matches!(
                words.first().map(String::as_str),
                Some("if" | "then" | "elif" | "else" | "fi")
            )
        })
    }

    fn execute_if_command(&mut self, if_command: &IfCommand) -> Result<(), ExecuteError> {
        // GNU 5.3.20 (rubash#1240/#1253): a word-expansion failure in
        // an if/elif condition abandons the whole enclosing list
        // (neither branch runs, and the commands after the `fi` on the
        // same line are discarded) — if_condition_matches propagates
        // the failure instead of selecting the else branch.
        let matched = match self.if_condition_matches(&if_command.condition)? {
            Some(matched) => matched,
            None => return Ok(()),
        };
        if matched {
            // GNU execute_if_command (execute_cmd.c:5200 family) walks the
            // branch body by POINTER; the normalize pass only rewrites
            // lexer-collapsed `for ((...))` words (first word `for`), so a
            // body without one executes the stored slice without the clone.
            return self.execute_if_branch_body(&if_command.then_body);
        }

        for branch in &if_command.elif_branches {
            let matched = match self.if_condition_matches(&branch.condition)? {
                Some(matched) => matched,
                None => return Ok(()),
            };
            if matched {
                return self.execute_if_branch_body(&branch.body);
            }
        }

        if let Some(body) = &if_command.else_body {
            return self.execute_if_branch_body(body);
        }

        self.exit_code = 0;
        Ok(())
    }

    /// GNU runs if/elif/else bodies by POINTER (execute_cmd.c:5200+; no
    /// compound executor copies its body). The stored slice executes
    /// directly unless the inline-arithmetic-for normalize pass would
    /// rewrite it (its only arm needs a `for` first word — see
    /// normalize_inline_compound_commands, builtins/source/flow.rs).
    fn execute_if_branch_body(&mut self, body: &[CommandNode]) -> Result<(), ExecuteError> {
        if body
            .iter()
            .any(|command| command.words.first().map(String::as_str) == Some("for"))
        {
            let normalized =
                crate::builtins::source::normalize_inline_compound_commands(body.to_vec());
            return self.execute_ast_inner(&normalized);
        }
        self.execute_ast_inner(body)
    }

    /// Returns `Some(true)` when the condition held, `Some(false)` when
    /// it did not. A fatal word-expansion failure in the condition is
    /// NOT absorbed into a branch decision: it propagates as
    /// ExecuteError::ExpansionFailure so the enclosing list is discarded
    /// (GNU jump_to_top_level (DISCARD); rubash#1240/#1253).
    fn if_condition_matches(
        &mut self,
        condition: &[CommandNode],
    ) -> Result<Option<bool>, ExecuteError> {
        // GNU evaluates the condition list by pointer (execute_cmd.c:5200
        // execute_if_command -> execute_command on the stored COMMAND); the
        // slice entry re-walks the stored nodes without the Vec clone.
        // rubash#661: a side-effect-free condition exits 0 with no
        // expansion work, so it can neither fail (the only reader of the
        // compound-condition marking) nor trip errexit (the only reader of
        // the suppression). Run it unmarked; GNU's execute_if_command
        // (execute_cmd.c:3862) pays no equivalent state traffic for a
        // trivial condition either.
        // A shadowing function or alias makes `:`/`true` execute user code
        // that can fail (a `true()` returning nonzero under `set -e` must
        // keep the slow path's errexit suppression). Gate it here.
        let condition_word_unshadowed = matches!(condition, [command]
            if command.words.len() == 1
                && !self.shell_state.functions.contains_key(command.words[0].as_str())
                && !self.shell_state.aliases.contains_key(command.words[0].as_str()));
        let result = if condition_word_unshadowed && if_condition_is_side_effect_free(condition) {
            self.execute_ast_inner(condition)
        } else {
            let saved_condition = self.inside_compound_condition.replace(true);
            let result =
                self.with_errexit_suppressed(|executor| executor.execute_ast_inner(condition));
            self.inside_compound_condition.set(saved_condition);
            result
        };
        match result {
            Err(ExecuteError::ExpansionFailure(code)) => {
                self.exit_code = code;
                // GNU expr.c evalerror -> shell.c jump_to_top_level
                // (DISCARD): a fatal expansion failure in a condition
                // discards the whole enclosing list, not just the if
                // command (rubash#1240/#1253, GNU 5.3.20: `if [ $((1/0))
                // -eq 1 ]; then ...; fi; echo after` prints neither
                // branch and never reaches `after`). Keep the error
                // unwinding; the reader-level loop applies the discard.
                Err(ExecuteError::ExpansionFailure(code))
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
        // GNU execute_cmd.c execute_coproc validates an explicit NAME
        // before anything is spawned: valid_identifier fails ->
        // err_invalidid (`` `<name>': not a valid identifier ``) and
        // EXECUTION_FAILURE, and no coprocess is created (rubash#1153:
        // `coproc "" { echo hi; }` printed the same text but exited 0
        // after spawning). The default COPROC name never takes this
        // branch, and a nameref name is a valid identifier itself — its
        // target is adjudicated at bind time below.
        if let Some(name) = &coproc_cmd.name {
            if !is_shell_name(name) {
                eprintln!(
                    "{}`{}': not a valid identifier",
                    self.diagnostic_prefix(),
                    name
                );
                self.exit_code = 1;
                return Ok(());
            }
        }
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
                // rubash#1653: the child re-parses this text, so a bare
                // head word `time` would regain keyword status in the
                // child even though the coproc parser never granted it
                // (GNU parse.y:1125-1175: after `coproc`, `time` is a
                // plain word — GNU forks the same parse tree, rubash
                // respawns through `-c`). Quote the bare head word so
                // the re-parse keeps it an ordinary command name: the
                // child's lookup then matches GNU's forked child
                // (function, builtin, PATH, else 127 `time: command
                // not found`). A head word already quoted in source
                // keeps its own raw spelling; only the bare `time`
                // spelling needs the quote.
                let mut metadata = coproc_cmd.word_metadata.clone();
                if coproc_cmd.words.first().map(String::as_str) == Some("time")
                    && metadata
                        .first()
                        .is_some_and(|meta| meta.value == "time" && meta.raw == "time")
                {
                    metadata[0].raw = "'time'".to_string();
                }
                child
                    .arg("-c")
                    .arg(command_words_source_text(&coproc_cmd.words, &metadata));
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
        // GNU coproc forks (execute_cmd.c coproc_getfd -> make_child), so the
        // coprocess shell inherits the parent's FULL function table — not
        // just `export -f` names. A DEBUG trap installed before the coproc
        // (e.g. oh-my-bash's bash-preexec `__bp_preexec_invoke_exec`) fires
        // inside the child and must resolve (unixwin/rubash#441). Ship a
        // snapshot as BASH_FUNC_name%% entries: the child's Executor::new
        // imports them as functions and strips the carrier keys, so the
        // entries never surface as variables and never leak back into the
        // parent's table (the parent keeps its own functions untouched).
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
            child.env(
                exported_function_env_name(name),
                exported_function_env_value(&body.commands, def_redirects),
            );
        }
        // Preserve the parent script location for diagnostics emitted by the
        // coprocess shell, while keeping internal executor state isolated.
        if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
            child.env("__RUBASH_SCRIPT_NAME", script);
            let line = cmd
                .line
                .map(|line| line.to_string())
                .or_else(|| {
                    self.shell_state
                        .env_vars
                        .get("__RUBASH_CURRENT_LINE")
                        .cloned()
                })
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
        // rubash#1074: the coproc body runs in a forked subshell
        // (execute_in_subshell: subshell_level++), so the child's
        // BASH_SUBSHELL starts at this shell's depth + 1.
        child.env(
            "__RUBASH_SUBSHELL_DEPTH",
            (self.shell_state.subshell_depth.get() + 1).to_string(),
        );

        // Create pipes for bidirectional communication
        let stdin_result = std::io::pipe();
        let stdout_result = std::io::pipe();

        if let (Ok((stdin_reader, stdin_writer)), Ok((stdout_reader, stdout_writer))) =
            (stdin_result, stdout_result)
        {
            child.stdin(stdin_reader);
            // rubash#1882: GNU's coproc child is a fork (execute_cmd.c
            // make_child): its fd 2 IS the shell's resolved fd-2 endpoint,
            // held as a real descriptor, so diagnostics keep flowing after
            // the parent shell exits. The old pipe + forwarder transport
            // died with the parent process and took every late diagnostic
            // with it. Handle-backed endpoints are therefore handed to
            // the child directly; only the process-substitution carrier
            // (a rendezvous path, no portable child handle) keeps the
            // forwarding thread.
            let coproc_stderr_target = match self.coproc_stderr_plan() {
                CoprocStderrPlan::Direct(stdio) => {
                    child.stderr(stdio);
                    None
                }
                CoprocStderrPlan::Forward(target) => {
                    child.stderr(Stdio::piped());
                    Some(target)
                }
            };
            // rubash#846 (GNU execute_cmd.c:1759-1762 + :2583): the
            // coproc child's own redirect failure exits inverted and the
            // parent flips it back — net status 1, no flip. Mark it so an
            // enclosing `!` suppresses its flip.
            let displaced_stdout_writer =
                match self.apply_coproc_redirects(cmd, &mut child, stdout_writer) {
                    Ok(writer) => writer,
                    Err(error) => {
                        if matches!(error, ExecuteError::IoError(_)) {
                            self.note_compound_redirect_failure();
                        }
                        return Err(error);
                    }
                };
            // rubash#1632: a coproc-level `>file`/`>>file` (or `>&-`)
            // displaced the pipe writer from the child's stdout. GNU
            // applies the redirect in the forked child, where the
            // displaced end survives in a redirect save slot for the
            // child's whole life — the parent's read end therefore sees
            // no EOF while the coproc lives (a `read -t` times out
            // instead of reporting EOF, and the file receives the data).
            // On Unix the writer rides into the child at the first free
            // fd in the save-slot range (dup2 clears CLOEXEC on the
            // target, so it survives the exec); elsewhere the parent
            // holds it in coproc_displaced_stdout_writers until the
            // coproc retires.
            #[cfg(unix)]
            if let Some(writer) = &displaced_stdout_writer {
                use std::os::unix::io::AsRawFd as _;
                let writer_fd = writer.as_raw_fd();
                unsafe {
                    use std::os::unix::process::CommandExt as _;
                    child.pre_exec(move || {
                        let mut slot = -1;
                        for candidate in 10..64 {
                            if libc::fcntl(candidate, libc::F_GETFD) == -1 {
                                slot = candidate;
                                break;
                            }
                        }
                        if slot == -1 {
                            return Err(std::io::Error::from_raw_os_error(libc::EMFILE));
                        }
                        if slot == writer_fd {
                            if libc::fcntl(writer_fd, libc::F_SETFD, 0) == -1 {
                                return Err(std::io::Error::last_os_error());
                            }
                        } else if libc::dup2(writer_fd, slot) == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
            }

            suppress_orphan_console(&mut child);
            child.current_dir(&self.shell_state.cwd);
            match child.spawn() {
                Ok(mut child_proc) => {
                    // Rubash does not expose real coproc file descriptors yet,
                    // but keep the parent ends until after spawn so stdio uses
                    // the correct pipe direction on all hosts.
                    let pid = child_proc.id();
                    adopt_orphan_console(pid);
                    // rubash#1632: settle the displaced stdout writer —
                    // on Unix the child holds its own copy (pre_exec) and
                    // the parent's copy closes here; elsewhere the parent
                    // holds it until the coproc retires.
                    #[cfg(unix)]
                    drop(displaced_stdout_writer);
                    #[cfg(not(unix))]
                    if let Some(writer) = displaced_stdout_writer {
                        self.coproc_displaced_stdout_writers.insert(pid, writer);
                    }
                    if let Some(target) = coproc_stderr_target {
                        if let Some(stderr) = child_proc.stderr.take() {
                            let forwarder =
                                std::thread::spawn(|| forward_coproc_stderr(stderr, target));
                            self.coproc_stderr_forwarders.insert(pid, forwarder);
                        }
                    }
                    self.background_children
                        .insert(pid, crate::fd::SpawnedChild::from(child_proc));
                    let job_control = crate::builtins::set::shell_option_enabled(
                        &self.shell_state.env_vars,
                        "monitor",
                    );
                    let job_id = self.shell_state.job_table.register_process_with_stages(
                        pid,
                        bash_command_source_text(cmd),
                        true,
                        async_job_pgrp(pid, job_control),
                        bash_job_stage_commands(cmd),
                    );
                    self.shell_state.job_table.set_job_control(pid, job_control);
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
                    let (coproc_read_fd, coproc_write_fd) = self.allocate_coproc_fds();
                    self.fd_table.open_input(
                        coproc_read_fd,
                        FdReadEndpoint::CoprocStdout {
                            pid,
                            fd: coproc_read_file,
                        },
                        true,
                    );
                    self.fd_table.open_output(
                        coproc_write_fd,
                        FdWriteEndpoint::CoprocStdin {
                            pid,
                            fd: coproc_write_file,
                        },
                        true,
                    );
                    self.shell_state
                        .job_table
                        .attach_coproc_endpoint(job_id, pid);
                    // Store the file descriptors in env for COPROC array
                    let stdin_key = format!("__RUBASH_COPROC_STDIN_{}", pid);
                    let stdout_key = format!("__RUBASH_COPROC_STDOUT_{}", pid);
                    self.shell_state
                        .env_vars
                        .insert(stdin_key, "pipe".to_string());
                    self.shell_state
                        .env_vars
                        .insert(stdout_key, "pipe".to_string());

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
                                    let target_exists = self
                                        .shell_state
                                        .env_vars
                                        .contains_key(target_base)
                                        || self.shell_state.variables.get(target_base).is_some();
                                    if target_exists {
                                        // v != 0: ASSIGN_DISALLOWED on the resolved
                                        // var; elements bind to the target but
                                        // <name>_PID still uses the original name.
                                        if is_marked_var(
                                            &self.shell_state.env_vars,
                                            READONLY_VARS,
                                            target_base,
                                        ) {
                                            eprintln!(
                                                "{}{}: readonly variable",
                                                self.error_prefix(),
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
                                    self.error_prefix(),
                                    array_name
                                );
                                unmark_env_name(
                                    &mut self.shell_state.env_vars,
                                    NAMEREF_VARS,
                                    &array_name,
                                );
                                if is_marked_var(
                                    &self.shell_state.env_vars,
                                    READONLY_VARS,
                                    &array_name,
                                ) {
                                    eprintln!(
                                        "{}{}: readonly variable",
                                        self.error_prefix(),
                                        array_name
                                    );
                                    can_bind = false;
                                }
                            }
                            NamerefResolution::NotNameref => {
                                if is_marked_var(
                                    &self.shell_state.env_vars,
                                    READONLY_VARS,
                                    &array_name,
                                ) {
                                    eprintln!(
                                        "{}{}: readonly variable",
                                        self.error_prefix(),
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
                        self.shell_state
                            .env_vars
                            .insert(bind_name.clone(), array_value);
                        mark_env_name(
                            &mut self.shell_state.env_vars,
                            "__RUBASH_ARRAY_VARS",
                            &bind_name,
                        );
                        // Keep the structured cell in lockstep so typed
                        // reads trust it (verified against the mirror by
                        // debug_assert in the read path). The two coproc
                        // fds occupy indexes 0 and 1.
                        {
                            use std::collections::BTreeMap;
                            let mut entries = BTreeMap::new();
                            entries.insert(0usize, coproc_read_fd.to_string());
                            entries.insert(1usize, coproc_write_fd.to_string());
                            crate::executor::arrays::sync_typed_indexed_array(
                                &mut self.shell_state.variables,
                                &bind_name,
                                &entries,
                            );
                        }
                        self.shell_state.env_vars.clear_array_cells_dirty();
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

    /// rubash#1882: resolve the coproc child's fd 2 the way GNU's fork
    /// leaves it — a dup of the shell's current fd-2 endpoint, held by
    /// the child as a real descriptor (execute_cmd.c make_child applies
    /// the coproc's own redirections inside the child, on top of the
    /// inherited set). Handle-backed endpoints are handed over directly
    /// (`CoprocStderrPlan::Direct`), so a diagnostic the child writes
    /// after this shell has exited still lands; the pipe + forwarder
    /// transport survives only for the process-substitution carrier,
    /// whose endpoint is a rendezvous path with no portable child
    /// handle, and as a fallback when a dup fails.
    fn coproc_stderr_plan(&mut self) -> CoprocStderrPlan {
        if self.fd_table.is_closed(2) {
            // GNU's forked child inherits the closed fd; /dev/null is
            // the spawn-level equivalent — writes can neither block on
            // a dead reader nor kill the writer with SIGPIPE.
            return CoprocStderrPlan::Direct(Stdio::null());
        }

        match self.fd_table.write_endpoint(2) {
            None | Some(FdWriteEndpoint::Stderr) => CoprocStderrPlan::Direct(Stdio::inherit()),
            Some(FdWriteEndpoint::Stdout) => match dup_process_std_handle(1) {
                Some(file) => CoprocStderrPlan::Direct(Stdio::from(file)),
                None => CoprocStderrPlan::Forward(CoprocStderrForwardTarget::Stdout),
            },
            Some(FdWriteEndpoint::File(file_fd)) => {
                match crate::fd::duplicate_handle_inheritable(file_fd.handle) {
                    Ok(handle) => {
                        CoprocStderrPlan::Direct(Stdio::from(crate::fd::handle_to_file(handle)))
                    }
                    Err(_) => CoprocStderrPlan::Forward(CoprocStderrForwardTarget::File(
                        file_fd.path.clone(),
                    )),
                }
            }
            Some(FdWriteEndpoint::ProcessSubstitution { path, .. }) => {
                CoprocStderrPlan::Forward(CoprocStderrForwardTarget::File(path))
            }
            Some(FdWriteEndpoint::CoprocStdin { fd, .. }) => {
                match crate::fd::duplicate_handle_inheritable(fd.handle) {
                    Ok(handle) => {
                        CoprocStderrPlan::Direct(Stdio::from(crate::fd::handle_to_file(handle)))
                    }
                    // The pre-#1882 resolution fell through to inherit
                    // when this dup failed; keep that floor.
                    Err(_) => CoprocStderrPlan::Direct(Stdio::inherit()),
                }
            }
        }
    }

    fn apply_coproc_redirects(
        &mut self,
        cmd: &CommandNode,
        child: &mut Command,
        stdout_writer: std::io::PipeWriter,
    ) -> Result<Option<std::io::PipeWriter>, ExecuteError> {
        // rubash#1651: a memory-backed stdin body (here-string,
        // heredoc) that wins fd 0 replaces the coproc pipe as the
        // child's stdin. GNU redir.c do_redirections binds fd 0
        // to the body before the child runs, so the coproc
        // consumes it and exits; the old code left the child on
        // the pipe and `coproc cat <<< hi` blocked forever. The
        // body rides a pre-filled pipe (writer thread: a body
        // larger than the pipe buffer must not block the spawn
        // waiting for a reader that does not exist yet; EPIPE
        // when the child exits early is ignored). Only
        // HereDoc/HereString winners take this path: `< file`
        // keeps the real-file path below, and `<&N` dups keep
        // the pipe — streaming a dup through a string buffer
        // would change blocking behavior. When the body wins,
        // it outranks redirect_in (GNU left-to-right order: the
        // last fd-0 redirect wins).
        let mut body_stdin: Option<std::process::Stdio> = None;
        if let Some(text) = self.coproc_stdin_body(cmd) {
            use std::io::Write as _;
            if let Ok((reader, mut writer)) = std::io::pipe() {
                std::thread::spawn(move || {
                    let _ = writer.write_all(text.as_bytes());
                });
                #[cfg(windows)]
                {
                    use std::os::windows::io::{FromRawHandle as _, IntoRawHandle as _};
                    let handle = reader.into_raw_handle();
                    body_stdin = Some(std::process::Stdio::from(unsafe {
                        std::fs::File::from_raw_handle(handle)
                    }));
                }
                #[cfg(unix)]
                {
                    use std::os::unix::io::{FromRawFd as _, IntoRawFd as _};
                    let fd = reader.into_raw_fd();
                    body_stdin = Some(std::process::Stdio::from(unsafe {
                        std::fs::File::from_raw_fd(fd)
                    }));
                }
            }
        }
        if body_stdin.is_none() {
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
        }

        // rubash#1632: the coproc command's own stdout redirections
        // displace the pipe writer instead of replacing-and-dropping it:
        // the writer is handed back to the caller, which keeps it alive
        // for the coproc's lifetime (GNU's redirect save slot in the
        // forked child) so the parent's read end cannot see EOF early.
        // A dup target (`>&2`) replaces nothing — the pipe stays the
        // child's stdout and the writer is consumed here as before.
        let mut stdout_displaced = false;
        if let Some(redirect) = &cmd.redirect_out {
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target) {
                child.stdout(Stdio::null());
                stdout_displaced = true;
            } else if redirect_target_fd(&target).is_none() {
                child.stdout(Stdio::from(
                    self.create_redirect_output(&target, redirect.clobber)?,
                ));
                stdout_displaced = true;
            }
        } else if let Some(redirect) = &cmd.append {
            let target = self.expand_redirect_target(redirect);
            if is_closed_redirect_target(&target) {
                child.stdout(Stdio::null());
                stdout_displaced = true;
            } else if redirect_target_fd(&target).is_none() {
                child.stdout(Stdio::from(open_redirect_append(&self.session_path(
                    shell_path_to_windows(&target, &self.shell_state.env_vars),
                ))?));
                stdout_displaced = true;
            }
        }
        let displaced_writer = if stdout_displaced {
            Some(stdout_writer)
        } else {
            child.stdout(stdout_writer);
            None
        };

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
                child.stderr(Stdio::from(open_redirect_append(&self.session_path(
                    shell_path_to_windows(&target, &self.shell_state.env_vars),
                ))?));
            }
        }

        // The body pipe replaces the coproc pipe set by the caller
        // (the parent's write end stays registered as ${COPROC[1]},
        // useless but present, like GNU's rebound pipe end).
        if let Some(stdio) = body_stdin {
            child.stdin(stdio);
        }

        Ok(displaced_writer)
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
        // xtrace_print_case_command_head (print_cmd.c:742-749) prints
        // case_command->word->word — the WORD_DESC text, which in GNU still
        // carries the source quote characters ("$x" stays "$x"). rubash's
        // equivalent text is word_metadata.raw; the lexer `.word` value is
        // the quote-stripped transport form whose PARAM_NAME_END_MARKER
        // boundary byte (0x13, where a deleted closing quote terminated an
        // unbraced $name) must never reach the trace output.
        if self.xtrace_enabled() {
            let prefix = self.xtrace_prefix();
            let raw_word = if case_command.word_metadata.raw.is_empty() {
                case_command.word.as_str()
            } else {
                case_command.word_metadata.raw.as_str()
            };
            let visible = crate::locale::decode_to_visible_text(raw_word);
            self.xtrace_write(format!("{prefix}case {visible} in\n").as_bytes());
        }
        // GNU execute_cmd.c:3660-3668: the case head is printed
        // (print_case_command_head, print_cmd.c:731 -> `case WORD in `) and
        // run_debug_trap fires before the word is expanded, so the trap sees
        // the raw unexpanded word — `case "$v" in ` keeps the quotes.
        // GNU's run_debug_trap (trap.c) is a no-op when no DEBUG action is
        // armed; gate the head-text `format!` on that presence test so the
        // common no-trap loop does not allocate a String per `case` (same
        // folded admission as the ast_exec per-command DEBUG preamble,
        // rubash#186).
        if !self.debug_trap_running
            && ((self.debug_trap_in_scope()
                && crate::builtins::trap::has_active_trap_action(
                    &self.shell_state.env_vars,
                    "DEBUG",
                    "__RUBASH_TRAP_DEBUG",
                ))
                || self.debugger_session_here())
        {
            let raw_word = if case_command.word_metadata.raw.is_empty() {
                case_command.word.as_str()
            } else {
                case_command.word_metadata.raw.as_str()
            };
            // execute_cmd.c:3672-3676: under extdebug a non-zero DEBUG status
            // skips the whole case — the word is never expanded, no clause
            // runs, and the case command returns EXECUTION_SUCCESS (probe
            // k9/m-issue: `R=0` after the skipped head).
            if self
                .run_debug_trap(
                    crate::debugger::SeamKind::CaseHead,
                    &format!("case {raw_word} in "),
                )?
                .is_some()
            {
                self.exit_code = 0;
                return Ok(());
            }
        }
        // GNU execute_cmd.c:3695: the case subject goes through
        // expand_word_leave_quoted, so a nounset unbound variable (or
        // another fatal expansion error) in it raises the FORCE_EOF
        // class before any clause is tested (rubash#1130) — the
        // simple-command pre-scan never sees the subject.
        {
            let scan_text = if case_command.word_metadata.raw.is_empty() {
                case_command.word.as_str()
            } else {
                case_command.word_metadata.raw.as_str()
            };
            if let Some(error) = self.parameter_expansion_error_in_word(scan_text) {
                return self.raise_word_expansion_error(error);
            }
        }
        // GNU execute_cmd.c:3695-3699: the case word is expanded by
        // expand_word_leave_quoted and then passed through dequote_string —
        // every single-quoted span is literal data (subst.c:11882-11886
        // never consults the command-substitution scanner inside '...'), and
        // only unquoted segments go through the full expansion pipeline.
        // The raw token text carries those span boundaries; the lexer's word
        // value has them stripped, which made `case 'a'\''`b`'` re-execute
        // `b` as a live substitution (rubash#153). Walk the raw text like
        // the pattern side (quote_aware_case_pattern) but without the
        // glob-literal escaping, which belongs to the pattern domain only.
        let word = if case_command.word_metadata.raw.is_empty()
            || case_command.word_metadata.raw == case_command.word
        {
            self.expand_case_word(&case_command.word)
        } else {
            let raw = case_command.word_metadata.raw.clone();
            quote_aware_case_word(&raw, |segment| self.expand_case_word(segment))
        };
        let word = tilde_expand::strip_assignment_quote_marker(&word);
        // GNU: the case word's command substitution recorded its status in
        // last_command_exit_value (subst.c command_substitute), and the case
        // command itself commits no status until a clause body finishes — so
        // $? inside the first body command still sees the substitution's
        // status (issue #485: `case $(false) in "") echo $?;; esac` prints 1).
        // Commit the word-expansion overlay to the real status here; the
        // clause body's execute_ast_inner clears the overlay at its node
        // boundary, which would otherwise lose it.
        if let Some(status) = self.word_expansion_comsub_exit.take() {
            self.exit_code = status;
        }
        self.abandon_on_arithmetic_expansion_error()?;
        let nocasematch =
            crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "nocasematch");
        let mut fall_through = false;
        let mut matched_any = false;
        // GNU execute_cmd.c:3644 execute_case_command builds `word` once
        // (expand_word_leave_quoted + dequote_string, 3695-3699) and reuses
        // it for every arm's strmatch at 3736. Stage its `Vec<char>` once
        // here so the N-arm loop below does not re-collect it per candidate;
        // the pattern side still stages per arm (it differs per arm) while
        // the literal / simple-glob fast paths stay borrow-only. The cell is
        // LAZY: a loop of static literal arms (GNU strmatch's no-staging
        // path) never builds it.
        let mut word_chars: Option<Vec<char>> = None;
        let mut index = 0;
        while let Some(clause) = case_command.clauses.get(index) {
            let matched = fall_through
                || clause.pattern_nodes.iter().any(|pattern| {
                    let stripped = self.expand_case_pattern(pattern);
                    if self.shell_state.arithmetic_fatal_error.get()
                        || self.shell_state.arithmetic_nounset_error.get()
                    {
                        return false;
                    }
                    if case_pattern_has_extglob(&stripped) {
                        // Extglob staging needs the word chars immediately
                        // (no literal fast path in extglob_matches_at); seed
                        // the lazy cell here.
                        let word_chars =
                            word_chars.get_or_insert_with(|| word.chars().collect());
                        if nocasematch {
                            crate::executor::conditional::extglob_case_pattern_matches_nocase_with_word_chars(
                                &stripped, word_chars,
                            )
                        } else {
                            crate::executor::conditional::extglob_case_pattern_matches_with_word_chars(
                                &stripped, word_chars,
                            )
                        }
                    } else if nocasematch {
                        case_pattern_matches_nocase_with_word_chars(
                            &stripped,
                            &word,
                            &mut word_chars,
                        )
                    } else {
                        case_pattern_matches_with_word_chars(&stripped, &word, &mut word_chars)
                    }
                });
            self.abandon_on_arithmetic_expansion_error()?;
            if matched {
                matched_any = true;
                if clause.body.is_empty() {
                    self.exit_code = 0;
                } else {
                    // GNU execute_case_command (execute_cmd.c:5200 family)
                    // walks the clause body by POINTER; no clone is taken.
                    self.execute_ast_inner(&clause.body)?;
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

    fn expand_case_pattern<'a>(
        &mut self,
        pattern: &'a crate::parser::CasePattern,
    ) -> std::borrow::Cow<'a, str> {
        // GNU execute_cmd.c:3686/3712 re-expands every arm per match attempt
        // (expand_word_leave_quoted, matched via match.c/cond.c): dynamic
        // re-expansion is semantics and stays. A static literal expands to
        // itself, so borrow the stored text: zero alloc, skip the pipeline.
        if case_arm_pattern_is_static_literal(&pattern.text, &pattern.raw_text) {
            return std::borrow::Cow::Borrowed(pattern.text.as_str());
        }
        const PROTECTED_CASE_PATTERN_BACKSLASH: char =
            crate::executor::markers::CASE_PATTERN_BACKSLASH_GUARD;

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
            return std::borrow::Cow::Owned(strip_surrounding_quotes(&restored));
        }

        let array_at = self.case_pattern_array_at_map(&pattern.raw_text);
        std::borrow::Cow::Owned(quote_aware_case_pattern(
            &pattern.raw_text,
            |word| self.expand_word_mut(word),
            &array_at,
        ))
    }

    /// First element + element count for every `${name[@]}` unit in a
    /// case pattern's raw text, in the engine's own `[@]` enumeration
    /// order (hash-slot order for associative arrays, index order for
    /// indexed ones — the order join_array_parameter_values joins in).
    /// Precomputed because quote_aware_case_pattern walks the pattern
    /// with only an expansion callback (rubash#1393).
    fn case_pattern_array_at_map(
        &self,
        raw: &str,
    ) -> std::collections::HashMap<String, (String, usize)> {
        let mut map = std::collections::HashMap::new();
        let mut rest = raw;
        while let Some((_, end, name)) = find_case_pattern_array_at(rest) {
            if !map.contains_key(&name) {
                let entry = self.case_pattern_array_at_first(&name);
                map.insert(name, entry);
            }
            rest = &rest[end..];
        }
        map
    }

    fn case_pattern_array_at_first(&self, name: &str) -> (String, usize) {
        let Some(value) = self.shell_state.env_vars.get(name) else {
            return (String::new(), 0);
        };
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name) {
            let values = super::assignment_helpers::assoc_hash_ordered_values(
                value,
                super::assignment_helpers::assoc_nbuckets(&self.shell_state.env_vars, name),
            );
            let count = values.len();
            (values.into_iter().next().unwrap_or_default(), count)
        } else {
            let entries = super::indexed_array_entries_cached(value);
            let count = entries.len();
            (entries.values().next().cloned().unwrap_or_default(), count)
        }
    }

    pub(in crate::executor) fn execute_case_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        case_command: &CaseCommand,
    ) -> Result<(), ExecuteError> {
        // rubash#619: same pointer-walk fast path as if — GNU execute_cmd.c
        // execute_case_command walks the stored clauses by pointer and applies
        // redirections only when the statement carries them. Without redirect
        // fields the cmd + case_command deep clones and the no-op redirect
        // application only add per-execution churn; keep the ambient line
        // binding (line_number = case_command->line) semantics intact.
        // RESTRICTED_SHELL (#804): refuse before any splice or creation.
        let has_redirects = compound_command_has_redirects(cmd);
        if has_redirects {
            if let Some(message) = self.restricted_compound_redirect_error(cmd)? {
                self.write_default_stderr(&message)?;
                self.exit_code = 1;
                return Ok(());
            }
        }
        if !has_redirects {
            let case_line = cmd.line;
            return self.with_ambient_line(case_line, |executor| {
                executor.execute_case_command(case_command)
            });
        }

        let mut redirect_cmd = cmd.clone();
        let group_outputs =
            self.materialize_compound_output_process_substitutions(&mut redirect_cmd)?;
        let mut case_command = case_command.clone();
        let result = self.apply_case_command_redirects(&redirect_cmd, &mut case_command);
        let status = self.exit_code;
        if let Err(error) = result {
            // rubash#846 (GNU execute_cmd.c:854-883): the case's OWN
            // redirect failure — mark it for the `!` flip sites.
            if matches!(error, ExecuteError::IoError(_)) {
                self.note_compound_redirect_failure();
            }
            let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
            self.exit_code = status;
            finish_result?;
            return Err(error);
        }
        // GNU execute_cmd.c:3653 `line_number = case_command->line`: the
        // `case` keyword's line is the ambient while a clause body runs.
        let case_line = cmd.line;
        let result = self.with_loop_fd_heredocs(cmd, |executor| {
            executor.with_command_input_redirects(cmd, |executor| {
                executor.with_ambient_line(case_line, |executor| {
                    executor.execute_case_command(&case_command)
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

    fn apply_case_command_redirects(
        &mut self,
        cmd: &CommandNode,
        case_command: &mut CaseCommand,
    ) -> Result<(), ExecuteError> {
        if let Some(redirect) = cmd
            .redirect_out
            .as_deref()
            .filter(|r| is_stdout_splice_redirect(cmd, r))
        {
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
        } else if let Some(redirect) = cmd
            .append
            .as_deref()
            .filter(|r| is_stdout_write_redirect(r))
        {
            let mut append_redirect = redirect.clone();
            append_redirect.target = self.expand_redirect_target(redirect);
            for clause in &mut case_command.clauses {
                apply_stdout_append_redirect(&mut clause.body, &append_redirect);
            }
        }

        if let Some(redirect) = cmd
            .redirect_err
            .as_deref()
            .filter(|r| is_stderr_splice_redirect(cmd, r))
        {
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
        } else if let Some(redirect) = cmd
            .redirect_err_append
            .as_deref()
            .filter(|r| is_stderr_splice_redirect(cmd, r))
        {
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
    // rubash#1218: the posix dash-word rule applies to compounds too —
    // `time -p ( ... )` parsed under posix mode is not the time keyword.
    // rubash#1520: the decision is the parse-time-frozen one recorded on
    // the node, never re-derived from the live mode.
    !time_dash_word_not_keyword(cmd)
        && cmd.words.first().map(String::as_str) == Some("time")
        && (cmd.for_command.is_some()
            || cmd.if_command.is_some()
            || cmd.loop_command.is_some()
            || cmd.select_command.is_some()
            || cmd.case_command.is_some()
            || cmd.coproc_command.is_some()
            || cmd.subshell_command.is_some()
            || cmd.brace_group.is_some())
}

/// Whitelist for the rubash#661 condition-state fast path: exactly one
/// bare `:` or `true` with every assignment/redirect/substitution channel empty
/// (shadowing is gated at the call site: a `true()` function or alias
/// makes the word execute user code that can fail).
/// (same trigger-free class as #607's literal admission — keep in sync).
/// GNU's execute_if_command (execute_cmd.c:3862) runs such a condition with
/// no observable state traffic: the literal word cannot fail expansion, it
/// exits 0 without touching variables, fds, or traps, and it observes
/// neither the compound-condition marking (consulted only on
/// ExpansionFailure paths) nor the errexit suppression (consulted only on
/// nonzero status). Anything else falls through to the marked path, so a
/// false negative only costs speed.
fn if_condition_is_side_effect_free(condition: &[CommandNode]) -> bool {
    let [command] = condition else {
        return false;
    };
    if !matches!(command.words.as_slice(), [word] if word == ":" || word == "true") {
        return false;
    }
    command.assignments.is_empty()
        && command.assignment_raws.is_empty()
        && command.compound_assignments.is_empty()
        && command.array_element_assignments.is_empty()
        && command.process_substitutions.is_empty()
        && command.extglob_patterns.is_empty()
        && command.pathname_patterns.is_empty()
        && command.redirects.is_empty()
        && command.redirect_in.is_none()
        && command.redirect_out.is_none()
        && command.append.is_none()
        && command.redirect_err.is_none()
        && command.redirect_err_append.is_none()
        && command.heredoc.is_none()
        && command.heredoc_body.is_none()
        && command.heredoc_delimiter.is_none()
        && command.heredoc_redirects.is_empty()
        && command.here_string.is_none()
        && command.here_string_carrier.is_none()
        && command.pipe.is_none()
        && !command.background
        && command.and_or.is_none()
        && !command.inverted
        && !command.subshell
        && !command.subshell_end
        && command.pipeline_command.is_none()
        && command.and_or_list.is_none()
        && command.time_command.is_none()
        && command.background_command.is_none()
        && command.inverted_command.is_none()
        && command.for_command.is_none()
        && command.arithmetic_command.is_none()
        && command.if_command.is_none()
        && command.loop_command.is_none()
        && command.conditional_command.is_none()
        && command.subshell_command.is_none()
        && command.case_command.is_none()
        && command.select_command.is_none()
        && command.function_command.is_none()
        && command.brace_group.is_none()
        && command.coproc_command.is_none()
}

/// Whitelist for the arith-for counter fast driver: exactly one bare `:`
/// command with every assignment/redirect/substitution/channel field empty —
/// the same structural admission as #661's `if_condition_is_side_effect_free`
/// (kept in sync field-for-field), narrowed to `:` alone. `Some` admits and
/// hands back the node for the fast loop's end-state replication;
/// everything else is doubt and runs the slow loop. Shadowing (a `:`
/// function, a disabled `:`) is gated at the call site.
fn for_body_is_bare_colon(body: &[CommandNode]) -> Option<&CommandNode> {
    let [command] = body else {
        return None;
    };
    if !matches!(command.words.as_slice(), [word] if word == ":") {
        return None;
    }
    if !(command.assignments.is_empty()
        && command.assignment_raws.is_empty()
        && command.compound_assignments.is_empty()
        && command.array_element_assignments.is_empty()
        && command.process_substitutions.is_empty()
        && command.extglob_patterns.is_empty()
        && command.pathname_patterns.is_empty()
        && command.redirects.is_empty()
        && command.redirect_in.is_none()
        && command.redirect_out.is_none()
        && command.append.is_none()
        && command.redirect_err.is_none()
        && command.redirect_err_append.is_none()
        && command.heredoc.is_none()
        && command.heredoc_body.is_none()
        && command.heredoc_delimiter.is_none()
        && command.heredoc_redirects.is_empty()
        && command.here_string.is_none()
        && command.here_string_carrier.is_none()
        && command.pipe.is_none()
        && !command.background
        && command.and_or.is_none()
        && !command.inverted
        && !command.subshell
        && !command.subshell_end
        && command.pipeline_command.is_none()
        && command.and_or_list.is_none()
        && command.time_command.is_none()
        && command.background_command.is_none()
        && command.inverted_command.is_none()
        && command.for_command.is_none()
        && command.arithmetic_command.is_none()
        && command.if_command.is_none()
        && command.loop_command.is_none()
        && command.conditional_command.is_none()
        && command.subshell_command.is_none()
        && command.case_command.is_none()
        && command.select_command.is_none()
        && command.function_command.is_none()
        && command.brace_group.is_none()
        && command.coproc_command.is_none())
    {
        return None;
    }
    Some(command)
}

/// True when the compound statement's own CommandNode carries any redirect,/// heredoc, here-string, or structured process substitution — i.e. when the
/// clone-and-apply machinery in the `*_with_redirects` wrappers has work to
/// do. Mirrors every field consulted by materialize_compound_output_
/// process_substitutions, apply_*_redirects, and with_command_input_redirects
/// (rubash#619 fast path).
pub(in crate::executor) fn compound_command_has_redirects(cmd: &CommandNode) -> bool {
    cmd.redirect_in.is_some()
        || cmd.redirect_out.is_some()
        || cmd.append.is_some()
        || cmd.redirect_err.is_some()
        || cmd.redirect_err_append.is_some()
        || !cmd.redirects.is_empty()
        || cmd.heredoc.is_some()
        || cmd.heredoc_body.is_some()
        || !cmd.heredoc_redirects.is_empty()
        || cmd.here_string.is_some()
        || cmd.here_string_carrier.is_some()
        || !cmd.process_substitutions.is_empty()
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

/// GNU parse.y, posix mode (rubash#1218): `time` is a reserved word
/// only when the word after it does not begin with `-`. With a dash
/// word (`time -p cmd`, `time -- cmd`, `time -x cmd`) the command is an
/// ordinary external command named `time`, resolved through PATH —
/// oracle-verified against GNU 5.3 (a fake `time` in PATH runs with
/// those arguments; without one the report is `time: command not
/// found`, status 127).
pub(in crate::executor) fn posix_time_dash_word(words: &[String], posix_mode: bool) -> bool {
    posix_mode
        && words
            .get(1)
            .is_some_and(|word| word.len() > 1 && word.starts_with('-'))
}

/// rubash#1520: the parse-time-frozen form of the #1218 veto, for nodes
/// the parser produced. GNU decides whether `time` is the reserved word
/// when the token is READ (parse.y:3286 `time_command_acceptable`, via
/// `CHECK_FOR_RESERVED_WORD` at parse.y:3179), under the mode in effect
/// for that reader unit; the lexer freezes that outcome into the token
/// kind (see `freeze_posix_time_dash_tokens`) and the parser stamps it
/// onto the node (`time_head_plain_word`) as the head word is recorded.
/// A head token recorded as a plain Word — reclassified by the lexer
/// under a POSIX pass because a dash word followed, or quoted in the
/// source — means the command is an ordinary external command named
/// `time`, whatever follows it and no matter how the live mode changed
/// after the unit was parsed. Synthesized nodes (never stamped) keep
/// the keyword reading, matching the pre-#1520 behaviour.
pub(in crate::executor) fn time_dash_word_not_keyword(cmd: &CommandNode) -> bool {
    cmd.words.first().map(String::as_str) == Some("time") && cmd.time_head_plain_word
}

fn time_prefix_parts(cmd: &CommandNode, posix_mode: bool) -> Option<TimePrefixParts> {
    let words = &cmd.words;
    if words.first().map(String::as_str) != Some("time") {
        return None;
    }
    if posix_time_dash_word(words, posix_mode) {
        return None;
    }

    // rubash#1548: operand window per parse.y special_case_tokens —
    // a quoted '-p' is the timed command's name, never time's option
    // (see crate::parser::scan_time_prefix_words).
    let aligned_metadata = if cmd.word_metadata.len() == words.len() {
        Some(cmd.word_metadata.as_slice())
    } else {
        None
    };
    let scan = crate::parser::scan_time_prefix_words(words, aligned_metadata);
    Some(TimePrefixParts {
        command_index: scan.next,
        inverted: scan.inverted,
        posix_format: scan.posix_format,
    })
}

/// rubash#809 CR F2: does a raw word carry an assignment-typed braced
/// parameter expansion — `${x=v}` / `${x:=v}` (also `${x=[w]}`)? These assign
/// in the expanding shell, so they must expand in the wrapper child, never in
/// the parent. `${x:-v}` / `${x-v}` / `${x:?v}` are reads and stay on the
/// fast path.
fn parameter_expansion_assigns(raw: &str) -> bool {
    // All indexing below is on the BYTE slice: every offset is a raw byte
    // position (no char counting). UTF-8 continuation bytes are >= 0x80, so
    // the two-byte "${" window can only match at a character boundary, and
    // the identifier walk only advances over ASCII bytes — both keep
    // `cursor` on a boundary by construction without slicing `raw`.
    let bytes = raw.as_bytes();
    let mut scan_from = 0usize;
    while scan_from + 1 < bytes.len() {
        let Some(rel) = bytes[scan_from..]
            .windows(2)
            .position(|window| window == b"${")
        else {
            break;
        };
        let mut cursor = scan_from + rel + 2;
        let name_start = cursor;
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_')
        {
            cursor += 1;
        }
        if cursor == name_start && cursor < bytes.len() && bytes[cursor] >= 0x80 {
            // A multibyte character sits where the parameter name would be
            // (`${中文=…}`): unicode names cannot be classified by the ASCII
            // walk, so stay conservative and keep the wrapper.
            return true;
        }
        if cursor > name_start
            && (bytes[cursor..].starts_with(b"=") || bytes[cursor..].starts_with(b":="))
        {
            return true;
        }
        scan_from = name_start.max(scan_from + 1);
    }
    false
}

pub(in crate::executor) fn test_rubash_binary_from_current_exe() -> Option<std::path::PathBuf> {
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
    // Zero-alloc scan (no `Vec<char>` staging): extglob operators and the
    // backslash escape are single-byte ASCII, so a byte scan is equivalent
    // to the char scan — multibyte continuation bytes never equal them.
    let bytes = pattern.as_bytes();
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if matches!(bytes[index], b'@' | b'*' | b'+' | b'?' | b'!') && bytes[index + 1] == b'(' {
            return true;
        }
        index += 1;
    }
    false
}

/// Static-literal gate for case arms (perf #662): GNU execute_cmd.c:3686
/// re-expands each arm per match attempt, but a pattern whose stored text
/// and raw source carry no expansion trigger expands to itself. Whitelist
/// (default-deny): plain chars only — `$`, backquote, quotes, `\`, plus
/// `~{<>}` (tilde/brace/process-subst act in expand_word_mut), controls,
/// and the markers.rs PUA carriers. Any doubt falls to the slow path.
fn case_arm_pattern_is_static_literal(text: &str, raw: &str) -> bool {
    fn plain(word: &str) -> bool {
        word.chars().all(|c| {
            !matches!(
                c,
                '$' | '`' | '\'' | '"' | '\\' | '~' | '{' | '}' | '<' | '>'
            ) && !c.is_control()
                && !('\u{E000}'..='\u{F8FF}').contains(&c)
        })
    }
    plain(text) && plain(raw)
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

/// Word-position twin of `quote_aware_case_pattern` (rubash#153).
///
/// GNU execute_cmd.c:3695-3699 expands the case word with
/// expand_word_leave_quoted and then dequote_string: single-quoted spans
/// are literal data — subst.c:11882-11886 hands the whole '...' body to
/// add_quoted_string without ever consulting the command-substitution
/// scanner — and only unquoted segments reach the full expansion
/// pipeline. Double-quoted spans expand parameters/substitutions but keep
/// a `'` inside literal (parse.y read_token_word's double-quote scanner
/// never opens single-quote state). Unlike the pattern twin this applies
/// no glob-literal escaping: the case word is the *subject* of
/// strmatch(), not a pattern (execute_cmd.c:3727-3730).
fn quote_aware_case_word(raw: &str, mut expand_word: impl FnMut(&str) -> String) -> String {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')) {
            let quote = chars[index + 1];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 2, quote, quote == '\'') {
                let body = chars[index + 2..end].iter().collect::<String>();
                if quote == '\'' {
                    output.push_str(&decode_ansi_c_escapes(&body));
                } else {
                    output.push_str(&expand_word(&body));
                }
                index = end + 1;
                continue;
            } else {
                // No closer: an unclosed quote is a parse error before
                // execution in GNU; never spin the walker on it.
                output.push(chars[index]);
                index += 1;
                continue;
            }
        }

        if matches!(chars[index], '\'' | '"') {
            let quote = chars[index];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 1, quote, false) {
                let body = chars[index + 1..end].iter().collect::<String>();
                if quote == '\'' {
                    // GNU subst.c:11882-11886: every byte of a '...' span is
                    // quoted data — a backtick here must never reach the
                    // comsub scanner (rubash#153: `case 'a'\''`b`'` ran `b`).
                    output.push_str(&body);
                } else {
                    // parse.y read_token_word: a `'` inside double quotes is
                    // literal data; tag it so expand_word's quote removal
                    // does not collapse it (same protection the pattern
                    // twin applies). Quotes inside a `${...}`/`$(...)`/
                    // backtick unit belong to the substitution's own
                    // grammar (parse.y:3877 parse_matched_pair) and stay
                    // raw — `${x#'foo'}` in a double-quoted case word must
                    // keep stripping the quoted pattern (rubash#258
                    // BUG_PSUBSQUOT).
                    let body = crate::lexer::quotes::mark_data_squotes_around_substitutions(&body);
                    output.push_str(&expand_word(&body));
                }
                index = end + 1;
                continue;
            } else {
                output.push(chars[index]);
                index += 1;
                continue;
            }
        }

        let mut pending_start = index;
        while index < chars.len()
            && chars[index] != '\''
            && chars[index] != '"'
            && !(chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')))
        {
            // Quoting inside a substitution belongs to the substitution's
            // own input (GNU parse_matched_pair), so skip the unit whole.
            if chars[index] == '$' && chars.get(index + 1) == Some(&'(') {
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
                // GNU parse.y:5366-5398 read_token_word: an unquoted
                // backslash quotes exactly one character, which survives
                // as a quoted literal (got_escaped_character,
                // parse.y:5694-5706). Flush the pending expandable run,
                // then emit the escaped character as data — `case \x in`
                // must expand the word to `x` and match pattern `x`
                // (case1.sub ok 9).
                if pending_start < index {
                    let segment = chars[pending_start..index].iter().collect::<String>();
                    output.push_str(&expand_word(&segment));
                }
                output.push(chars[index + 1]);
                index += 2;
                pending_start = index;
                continue;
            }
            index += 1;
        }
        if pending_start < index {
            let segment = chars[pending_start..index].iter().collect::<String>();
            output.push_str(&expand_word(&segment));
        }
    }

    output
}

/// The first `${name[@]}` unit in `text`: (byte offset of the `$`, byte
/// offset just past the closing `}`, array name). Braced and
/// parenthesized units that are not exactly `${name[@]}` are skipped
/// whole, so a `[@]` nested inside another expansion is not mistaken
/// for a top-level one.
fn find_case_pattern_array_at(text: &str) -> Option<(usize, usize, String)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let plain: Vec<char> = chars.iter().map(|(_, c)| *c).collect();
    let mut index = 0usize;
    while index < plain.len() {
        if plain[index] == '$' && plain.get(index + 1) == Some(&'(') {
            if let Some(close) = crate::lexer::skip_parenthesized_unit_corrected(&plain, index + 1)
            {
                index = close.min(plain.len());
                continue;
            }
        }
        if plain[index] == '$' && plain.get(index + 1) == Some(&'{') {
            if let Some(close) = skip_braced_case_pattern_unit(&plain, index + 1) {
                let unit: String = plain[index..=close].iter().collect();
                let name = unit
                    .strip_prefix("${")
                    .and_then(|unit| unit.strip_suffix("[@]}"))
                    .filter(|name| {
                        !name.is_empty()
                            && name
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    });
                if let Some(name) = name {
                    let start = chars[index].0;
                    let end = if close + 1 < chars.len() {
                        chars[close + 1].0
                    } else {
                        text.len()
                    };
                    return Some((start, end, name.to_string()));
                }
                index = close + 1;
                continue;
            }
        }
        index += 1;
    }
    None
}

enum CasePatternSegment {
    /// The segment's literal contribution; later segments still glue
    /// onto the same pattern word.
    Continues(String),
    /// The pattern word is complete: a multi-element `${name[@]}`
    /// produced a second field, so everything after it (the rest of
    /// this segment and all later segments) is discarded.
    Finished(String),
}

/// Expand one double-quoted case-pattern segment under GNU's field rule
/// (rubash#1393): the pattern is a single word, but a quoted
/// `"${name[@]}"` expands to a field LIST and only the first field
/// belongs to the pattern — text before the expansion glues onto that
/// field, text after it belongs to later fields. A one-element
/// expansion glues whole; an empty one contributes nothing.
fn case_pattern_quoted_segment(
    body: &str,
    expand_word: &mut impl FnMut(&str) -> String,
    array_at: &std::collections::HashMap<String, (String, usize)>,
) -> CasePatternSegment {
    let mut literal = String::new();
    let mut rest = body;
    loop {
        let Some((start, end, name)) = find_case_pattern_array_at(rest) else {
            literal.push_str(&expand_word(rest));
            return CasePatternSegment::Continues(literal);
        };
        let (first, count) = array_at.get(&name).cloned().unwrap_or_default();
        literal.push_str(&expand_word(&rest[..start]));
        if count == 0 {
            rest = &rest[end..];
            continue;
        }
        literal.push_str(&first);
        if count >= 2 {
            return CasePatternSegment::Finished(literal);
        }
        rest = &rest[end..];
    }
}

fn quote_aware_case_pattern(
    raw: &str,
    mut expand_word: impl FnMut(&str) -> String,
    array_at: &std::collections::HashMap<String, (String, usize)>,
) -> String {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')) {
            let quote = chars[index + 1];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 2, quote, quote == '\'') {
                let body = chars[index + 2..end].iter().collect::<String>();
                if quote == '"' {
                    match case_pattern_quoted_segment(&body, &mut expand_word, array_at) {
                        CasePatternSegment::Continues(literal) => {
                            output.push_str(&escape_case_pattern_literal(&literal));
                            index = end + 1;
                            continue;
                        }
                        CasePatternSegment::Finished(literal) => {
                            output.push_str(&escape_case_pattern_literal(&literal));
                            return output;
                        }
                    }
                }
                let literal = decode_ansi_c_escapes(&body);
                output.push_str(&escape_case_pattern_literal(&literal));
                index = end + 1;
                continue;
            } else {
                // Unclosed unit: parse error before execution in GNU;
                // advance as data rather than spin.
                output.push(chars[index]);
                index += 1;
                continue;
            }
        }

        if matches!(chars[index], '\'' | '"') {
            let quote = chars[index];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 1, quote, false) {
                let body = chars[index + 1..end].iter().collect::<String>();
                if quote == '"' {
                    // parse.y read_token_word's double-quote scanner never
                    // opens single-quote state: a `'` inside the body is
                    // literal data (GNU case patterns keep it, so
                    // `case $s in "''")` matches the two quote characters).
                    // A bare `'` pair fed to expand_word_mut collapses to
                    // the empty string via quote removal, so tag each one
                    // with the lexer's decoded-quote marker first — the
                    // same protection escape_decoded_ansi_c_quotes applies.
                    // Quotes inside a `${...}`/`$(...)`/backtick unit stay
                    // raw (parse.y:3877 parse_matched_pair owns them;
                    // rubash#258 BUG_PSUBSQUOT).
                    let body = crate::lexer::quotes::mark_data_squotes_around_substitutions(&body);
                    match case_pattern_quoted_segment(&body, &mut expand_word, array_at) {
                        CasePatternSegment::Continues(literal) => {
                            output.push_str(&escape_case_pattern_literal(&literal));
                            index = end + 1;
                            continue;
                        }
                        CasePatternSegment::Finished(literal) => {
                            output.push_str(&escape_case_pattern_literal(&literal));
                            return output;
                        }
                    }
                }
                output.push_str(&escape_case_pattern_literal(&body));
                index = end + 1;
                continue;
            } else {
                output.push(chars[index]);
                index += 1;
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
pub(in crate::executor) fn skip_braced_case_pattern_unit(
    chars: &[char],
    open: usize,
) -> Option<usize> {
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

pub(in crate::executor) fn quoted_case_pattern_end(
    chars: &[char],
    start: usize,
    quote: char,
    ansi_c: bool,
) -> Option<usize> {
    let mut index = start;
    let mut escaped = false;
    while index < chars.len() {
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        // GNU parse.y:3992-3996 parse_matched_pair: inside a `$'...'`
        // ANSI-C unit (P_ALLOWESC, set by read_token_word at
        // parse.y:5559) a backslash sets LEX_PASSNEXT — the next
        // character, including the closing `'`, is consumed as escaped
        // data and never ends the span. A plain `'...'` span honors no
        // escapes, so `ansi_c` is only true for the `$'` caller.
        if (quote == '"' || ansi_c) && chars[index] == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if quote == '"' {
            // GNU parse.y skip_double_quoted (via parse_matched_pair,
            // parse.y:5416-5432): inside `"..."` a substitution unit owns
            // its own quoting — `$(` takes parse_comsub/parse_matched_pair,
            // `${` takes parse_matched_pair, and a backquote opens backquote
            // mode — so an inner `"` never ends the span. Stopping at the
            // first inner quote truncated `$(<"file")` to `$(<` and made
            // the comsub look unclosed (rubash#259: bats-preprocess
            // `<<<"$(<"$test_file")"$'\n'` lost the whole here-string).
            match chars[index] {
                '`' => {
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
                '$' if chars.get(index + 1) == Some(&'(') => {
                    if let Some(close) =
                        crate::lexer::skip_parenthesized_unit_corrected(chars, index + 1)
                    {
                        index = close.min(chars.len());
                        continue;
                    }
                }
                '$' if chars.get(index + 1) == Some(&'{') => {
                    if let Some(close) = skip_braced_case_pattern_unit(chars, index + 1) {
                        index = close + 1;
                        continue;
                    }
                }
                _ => {}
            }
        }
        if chars[index] == quote {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn escape_case_pattern_literal(value: &str) -> String {
    // GNU subst.c add_quoted_string, reached from the case-pattern
    // expansion path (execute_cmd.c:3670 -> expand_word_leave_quoted):
    // every character of a quoted literal is CTLESC-protected, and the
    // pattern matcher (sm_loop.c) then treats it as plain data. In
    // particular a quoted `]' inside a bracket pattern is a MEMBER, never
    // the closer — `case c in ( *["ab]cd"]* )' matches (modernish
    // FTL_BRACSQBR). Backslash-escaping instead carried posixpat member
    // semantics (posixpat ok 21 keeps `[\]` open) and left `]' entirely
    // unprotected, so the bracket closed early and the pattern could
    // never match.
    //
    // One rubash carrier rides as TWO chars: a byte the text pipeline
    // cannot hold is stored as a raw-byte marker pair (markers.rs
    // RAW_BYTE_MARKER_ESCAPE + RAW_BYTE_MARKER_FIRST+byte; owner
    // substitution_metadata.rs) — the pair IS one character unit, the
    // carrier for one byte. GNU's per-character CTLESC therefore guards
    // the pair once, as a unit: wrapping each half separately (the
    // d3e8affa form) read the introducer as a literal U+E000 and
    // destroyed the pair, so byte-flattening
    // (conditional/pattern.rs flatten_pattern_to_byte_chars) decoded
    // garbage and every quoted pattern holding such a byte stopped
    // matching — `z=$'\v\f\a\b'; case "$z" in "$z")` and the
    // `$'\v\f\a\b')` clause of nquote.tests line 79 printed `bad'
    // (nquote snapshot red gate, regression window beb1eca6..d3e8affa).
    use crate::executor::markers::{
        CTLESC, RAW_BYTE_MARKER_ESCAPE, RAW_BYTE_MARKER_FIRST, RAW_BYTE_MARKER_LAST,
    };
    let mut escaped = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        escaped.push(CTLESC);
        escaped.push(ch);
        if (ch as u32) == RAW_BYTE_MARKER_ESCAPE {
            if let Some(&payload) = chars.peek() {
                if (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&(payload as u32)) {
                    escaped.push(chars.next().expect("peeked raw-byte payload"));
                }
            }
        }
    }
    escaped
}

#[cfg(test)]
mod case_arm_static_tests {
    //! Perf #662: the static-literal short-circuit must agree with the
    //! slow path per pattern shape — GNU execute_cmd.c:3686/3712
    //! re-expands dynamic arms every attempt (match.c/cond.c), so only
    //! proven-static text may borrow.
    use super::case_arm_pattern_is_static_literal;
    use crate::executor::Executor;
    use crate::parser::CasePattern;
    use std::borrow::Cow;

    #[test]
    fn static_shapes_borrow_stored_text() {
        for shape in [
            "foo",
            "*.sh",
            "[a-z]",
            "a?c",
            "a b",
            "caf\u{e9}",
            "+x%/=:;!#&@()|",
        ] {
            let pattern = CasePattern::new(shape.to_string(), 0, 0);
            assert!(
                case_arm_pattern_is_static_literal(&pattern.text, &pattern.raw_text),
                "static {shape:?} rejected"
            );
            match Executor::new().expand_case_pattern(&pattern) {
                Cow::Borrowed(text) => assert_eq!(text, shape),
                Cow::Owned(_) => panic!("static {shape:?} took the slow path"),
            }
        }
    }

    #[test]
    fn dynamic_shapes_take_slow_path() {
        for (text, raw) in [
            ("$v", "$v"),
            ("`id`", "`id`"),
            ("*", "\"*\""),
            ("a", "'a'"),
            ("\\a", "\\a"),
            ("~/x", "~/x"),
            ("{a,b}", "{a,b}"),
            ("<(x)", "<(x)"),
            ("a\u{e317}b", "a\u{e317}b"),
            ("a\u{11}b", "a\u{11}b"),
        ] {
            assert!(
                !case_arm_pattern_is_static_literal(text, raw),
                "dynamic {raw:?} admitted"
            );
        }
        let mut executor = Executor::new();
        executor.set_env("V", "foo");
        match executor.expand_case_pattern(&CasePattern::new("$V".to_string(), 0, 0)) {
            Cow::Owned(text) => assert_eq!(text, "foo"),
            Cow::Borrowed(_) => panic!("dynamic $V borrowed"),
        }
    }

    #[test]
    fn arms_match_like_gnu_per_shape() {
        for (script, code) in [
            ("case foo in foo) true;; *) false;; esac", 0),
            ("case foo in bar) true;; *) false;; esac", 1),
            ("case foo in f*) true;; *) false;; esac", 0),
            ("case 'a b' in a\\ b) true;; *) false;; esac", 0),
            ("v=foo; case foo in $v) true;; *) false;; esac", 0),
            ("v=bar; case foo in $v) true;; *) false;; esac", 1),
            ("case foo in $(echo foo)) true;; *) false;; esac", 0),
            ("v='f*'; case foo in \"$v\") true;; *) false;; esac", 1),
            ("case a in \\a) true;; *) false;; esac", 0),
        ] {
            let tokens = crate::lexer::tokenize(script);
            let ast = crate::parser::parse(&tokens);
            let mut executor = Executor::new();
            executor.execute_ast(&ast).ok();
            assert_eq!(executor.last_exit_code(), code, "{script:?}");
        }
    }
}

#[cfg(test)]
mod arith_counter_fast_loop_tests {
    //! Contract for the canonical-counter fast driver
    //! (`try_enter_arith_counter_fast_loop` / `run_arith_counter_fast_loop`):
    //! whitelisted shapes must admit and evaluate exactly like the slow
    //! loop, and every doubt shape must fall through to it. The differential
    //! test below runs each script twice — once fast, once forced slow via
    //! an undelivered WINCH trap that installs an action (so the trap gate
    //! disqualifies) without changing any other semantics — and compares
    //! the full observable end-state.
    use super::for_body_is_bare_colon;
    use crate::executor::arithmetic::{arith_counter_loop_shapes, ArithCounterLoop};
    use crate::executor::Executor;

    fn for_body(script: &str) -> Vec<crate::parser::CommandNode> {
        let tokens = crate::lexer::tokenize(script);
        let ast = crate::parser::parse(&tokens);
        ast.commands
            .first()
            .and_then(|cmd| cmd.for_command.as_ref())
            .map(|for_command| for_command.body.clone())
            .unwrap_or_else(|| panic!("no for body in {script:?}"))
    }

    #[test]
    fn counter_shapes_admit_canonical() {
        assert_eq!(
            arith_counter_loop_shapes("i=0", "i<200000", "i++"),
            Some(ArithCounterLoop {
                var: "i".to_string(),
                init: 0,
                bound: 200000,
            })
        );
        // Whitespace around the tokens still parses to the same counter.
        assert_eq!(
            arith_counter_loop_shapes("i = 0", "i < 200000", "i ++"),
            Some(ArithCounterLoop {
                var: "i".to_string(),
                init: 0,
                bound: 200000,
            })
        );
    }

    #[test]
    fn counter_shapes_reject_doubt() {
        // Empty sections, other operators, other updates, name mismatches,
        // signed/octal/huge bounds, dynamic names: all slow.
        for (init, test, update) in [
            ("", "i<3", "i++"),
            ("i=0", "", "i++"),
            ("i=0", "i<3", ""),
            ("i=0", "i<=3", "i++"),
            ("i=0", "i==3", "i++"),
            ("i=0", "i!=3", "i++"),
            ("i=0", "i>3", "i++"),
            ("i=0", "i<3", "i--"),
            ("i=0", "i<3", "++i"),
            ("i=0", "i<3", "i+=1"),
            ("i=0", "i<3", "i=i+1"),
            ("i=0", "i<3", "i=3"),
            ("j=0", "i<3", "i++"),
            ("i=0", "i<3", "j++"),
            ("i=0", "i<N", "i++"),
            ("i=+0", "i<3", "i++"),
            ("i=-1", "i<3", "i++"),
            ("i=00", "i<3", "i++"),
            ("i=0", "i<007", "i++"),
            ("i=0", "i<99999999999999999999999", "i++"),
            ("i=99999999999999999999999", "i<3", "i++"),
            ("RANDOM=0", "RANDOM<3", "RANDOM++"),
            ("i=0", "i<3", "RANDOM++"),
            ("i=0", "i++", "i++"),
            ("i<3", "i<3", "i++"),
        ] {
            assert!(
                arith_counter_loop_shapes(init, test, update).is_none(),
                "admitted doubt ({init:?}, {test:?}, {update:?})"
            );
        }
    }

    #[test]
    fn bare_colon_body_admission() {
        assert!(for_body_is_bare_colon(&for_body("for ((i=0;i<3;i++)); do :; done")).is_some());
        for script in [
            "for ((i=0;i<3;i++)); do : x; done",
            "for ((i=0;i<3;i++)); do :>f; done",
            "for ((i=0;i<3;i++)); do true; done",
            "for ((i=0;i<3;i++)); do :; :; done",
            "for ((i=0;i<3;i++)); do ((x++)); done",
            "for ((i=0;i<3;i++)); do ! :; done",
            "for ((i=0;i<3;i++)); do : & done",
        ] {
            assert!(
                for_body_is_bare_colon(&for_body(script)).is_none(),
                "admitted non-trivial body {script:?}"
            );
        }
    }

    /// Observable end-state compared field-for-field between the fast loop
    /// and the same loop forced slow.
    fn run_and_snapshot(script: &str) -> (i32, Vec<(String, String)>) {
        let tokens = crate::lexer::tokenize(script);
        let ast = crate::parser::parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).ok();
        let code = executor.last_exit_code();
        let mut watched = [
            "i",
            "_",
            "__RUBASH_CURRENT_LINE",
            "__RUBASH_CMD_START_LINE",
            "__RUBASH_LAST_COMMAND",
            "__RUBASH_CURRENT_COMMAND",
        ];
        watched.sort();
        let mut snapshot = Vec::new();
        for key in watched {
            snapshot.push((
                key.to_string(),
                executor.get_env(key).unwrap_or("<unset>").to_string(),
            ));
        }
        // The arithmetic write log must drain on both paths.
        assert!(
            crate::executor::arithmetic::arith_write_log_is_empty(),
            "write log pending after {script:?}"
        );
        (code, snapshot)
    }

    #[test]
    fn fast_and_slow_loops_agree() {
        // (script body, expected final `i`)
        for (body, expected_i) in [
            ("for ((i=0;i<0;i++)); do :; done", "0"),
            ("for ((i=0;i<1;i++)); do :; done", "1"),
            ("for ((i=0;i<50;i++)); do :; done", "50"),
            ("for ((i = 0; i < 50; i ++)); do :; done", "50"),
            ("i=999; for ((i=0;i<50;i++)); do :; done", "50"),
            ("for ((i=7;i<60;i++)); do :; done", "60"),
        ] {
            // A `: PIVOT` prefix pins `_` identically on both arms: the
            // slow-forcing `trap` command below would otherwise bind its own
            // last word (`WINCH`) and skew the comparison.
            let fast_script = format!(": PIVOT; {body}");
            let (fast_code, fast_state) = run_and_snapshot(&fast_script);
            // An installed-but-undelivered WINCH action disqualifies the
            // fast loop via the trap gate without changing any semantics.
            let slow_script = format!("trap 'true' WINCH; : PIVOT; {body}");
            let (slow_code, slow_state) = run_and_snapshot(&slow_script);
            assert_eq!(fast_code, slow_code, "exit of {body:?}");
            assert_eq!(fast_state, slow_state, "state of {body:?}");
            assert_eq!(fast_code, 0, "loop must end 0: {body:?}");
            let final_i = fast_state
                .iter()
                .find(|(key, _)| key == "i")
                .map(|(_, value)| value.as_str());
            assert_eq!(final_i, Some(expected_i), "final i of {body:?}");
        }
    }

    #[test]
    fn fast_loop_binds_canonical_counter() {
        let tokens = crate::lexer::tokenize("for ((i=0;i<5;i++)); do :; done");
        let ast = crate::parser::parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).ok();
        assert_eq!(executor.get_env("i"), Some("5"));
        assert_eq!(executor.get_env("_"), Some(":"));
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn gates_fall_back_without_changing_semantics() {
        // Each gate-breaker forces the slow loop; results must still match
        // GNU (verified byte-for-byte against system bash during review).
        for script in [
            // DEBUG action installed: trap fires on the slow path.
            "trap 'echo -n' DEBUG; for ((i=0;i<3;i++)); do :; done; echo \"i=$i\"",
            // `:` shadowed by a function.
            ":() { echo -n FN; }; for ((i=0;i<3;i++)); do :; done; echo \"i=$i\"",
            // Non-canonical body.
            "for ((i=0;i<3;i++)); do echo -n \"B$i \"; done",
            // Non-canonical update.
            "for ((i=0;i<3;i++)); do :; done; for ((i=0;i<3;i+=1)); do :; done; echo \"i=$i\"",
        ] {
            let tokens = crate::lexer::tokenize(script);
            let ast = crate::parser::parse(&tokens);
            let mut executor = Executor::new();
            executor.execute_ast(&ast).ok();
            assert_eq!(executor.get_env("i"), Some("3"), "{script:?}");
        }
    }
}
