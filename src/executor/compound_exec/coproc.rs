use super::background::async_job_pgrp;
use super::transport::{
    rubash_spawn_inherited_state, test_rubash_binary_from_current_exe, test_rubash_binary_from_env,
};
use super::{encode_root_line_spans, first_leaf_command_line};
use crate::executor::*;

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

/// One side of a coproc child's stdio while the ordered output
/// redirects are applied (rubash#1644, transplanted from PR #1695):
/// a dup copies the current spec, so it must be cloneable without
/// consuming it.
enum CoprocFdSpec {
    Null,
    Pipe(std::io::PipeWriter),
    File(std::fs::File),
}

impl CoprocFdSpec {
    fn dup(&self) -> std::io::Result<Self> {
        Ok(match self {
            CoprocFdSpec::Null => CoprocFdSpec::Null,
            CoprocFdSpec::Pipe(writer) => CoprocFdSpec::Pipe(writer.try_clone()?),
            CoprocFdSpec::File(file) => CoprocFdSpec::File(file.try_clone()?),
        })
    }

    fn into_stdio(self) -> Stdio {
        match self {
            CoprocFdSpec::Null => Stdio::null(),
            CoprocFdSpec::Pipe(writer) => Stdio::from(writer),
            CoprocFdSpec::File(file) => Stdio::from(file),
        }
    }
}

/// rubash#4170: report a coproc redirect diagnostic through the
/// walk's current fd-2 binding. GNU's coproc child reports a
/// do_redirections failure through fd 2 as bound by the list's
/// earlier entries (redir.c:135 redirection_error), so a
/// `2>diag.txt` earlier in the list captures the diagnostic, a
/// closed fd 2 loses it, and a `2>&1` onto the coproc pipe
/// delivers it to the parent's read end. A failed write loses
/// the diagnostic like GNU's builtin_error — never a second one.
fn write_coproc_redirect_diagnostic(spec: &CoprocFdSpec, text: &str) {
    use std::io::Write as _;
    match spec {
        CoprocFdSpec::File(file) => {
            if let Ok(mut dup) = file.try_clone() {
                let _ = dup.write_all(text.as_bytes());
            }
        }
        CoprocFdSpec::Pipe(writer) => {
            if let Ok(mut dup) = writer.try_clone() {
                let _ = dup.write_all(text.as_bytes());
            }
        }
        CoprocFdSpec::Null => {}
    }
}

/// rubash#4170: the fd-0 disposition a Declined walk hands to
/// the stillborn child — the memory body outranks a redirect_in
/// binding, which outranks the coproc pipe reader (the same
/// precedence the Proceed path applies to the real child).
fn coproc_doomed_stdin(
    body_stdin: Option<std::process::Stdio>,
    stdin_override: Option<std::process::Stdio>,
    stdin_reader: std::io::PipeReader,
) -> std::process::Stdio {
    body_stdin
        .or(stdin_override)
        .unwrap_or_else(|| std::process::Stdio::from(stdin_reader))
}

/// rubash#4170: a coproc list entry's open failure is a
/// do_redirections failure in GNU's child, not a parent-side
/// spawn error: report it through the current fd-2 binding (the
/// `{target}: {message}` text the IoError reporter composes for
/// the same error) and hand back None so the caller returns the
/// walk's Declined outcome.
fn coproc_open_or_report(
    result: std::io::Result<std::fs::File>,
    err_spec: &CoprocFdSpec,
    prefix: &str,
) -> Option<std::fs::File> {
    match result {
        Ok(file) => Some(file),
        Err(error) => {
            write_coproc_redirect_diagnostic(
                err_spec,
                &format!("{}{}\n", prefix, crate::posix_errors::message(&error)),
            );
            None
        }
    }
}

/// Seed for a coproc dup target (rubash#1644): a fresh owned handle
/// duplicating the parent's process fd 1/2, so a `>&2` child stdout
/// writes where the parent's stderr writes. Unlike
/// dup_process_std_handle this reports failure (and maps a missing
/// std handle to /dev/null / NUL) because the seed feeds the ordered
/// redirect application, whose errors already have a reporting path.
fn dup_coproc_process_fd(fd: u32) -> std::io::Result<std::fs::File> {
    let handle = crate::fd::process_std_handle(fd);
    if handle == 0 || handle == -1 {
        return if cfg!(windows) {
            std::fs::OpenOptions::new().write(true).open("NUL")
        } else {
            std::fs::File::create("/dev/null")
        };
    }
    let dup = crate::fd::duplicate_handle(handle)?;
    Ok(crate::fd::handle_to_file(dup))
}

/// rubash#1632 save-slot hand-back under the ordered redirect pass
/// (rubash#1644): replacing the fd-1 spec must not drop the coproc
/// pipe writer while the coproc lives — when the spec still holds
/// it, the writer moves to the displaced slot (the caller keeps it
/// alive for the coproc's lifetime) instead of closing here.
fn replace_coproc_out_spec(
    spec: &mut CoprocFdSpec,
    replacement: CoprocFdSpec,
    displaced: &mut Option<std::io::PipeWriter>,
) {
    let old = std::mem::replace(spec, replacement);
    if let CoprocFdSpec::Pipe(writer) = old {
        *displaced = Some(writer);
    }
}

/// rubash#2200: a construct-level binding for a numbered fd
/// (>= 3), resolved during apply_coproc_redirects' ordered walk —
/// the state GNU's coproc child holds after do_redirections
/// (execute_cmd.c:4244 execute_in_subshell). File carries the
/// opened description plus the access the binding was created
/// with (the __RUBASH_FD_* env channel names read and write
/// handles separately).
#[derive(Clone)]
enum CoprocFdBinding {
    File {
        file: Rc<FileFd>,
        read: bool,
        write: bool,
    },
    Closed,
}

/// How a numbered-fd dup in the coproc redirect walk resolved
/// (rubash#2200).
enum CoprocDupBind {
    /// Nothing bound (the source has no transportable
    /// description, or the dup is a self-dup).
    Unbound,
    /// Bound. If Some, it carries the fd-1/2 source of a move
    /// (`>&N-`) whose spec the caller must still close — the
    /// helper cannot touch the specs from behind the borrow.
    Bound(Option<u32>),
}

/// The File binding a coproc fd-1/2 spec contributes to a
/// numbered-fd dup (rubash#2200): a fresh owned copy of the
/// spec's description. Spec files are write descriptions, so
/// the binding is write-only; a Null spec binds Closed; the
/// coproc pipe writer clones like any other description (a
/// `3>&1` on the construct gives the body a second handle on
/// its own output pipe, as GNU's fork leaves it).
fn coproc_spec_binding(spec: &CoprocFdSpec, path: &Option<PathBuf>) -> Option<CoprocFdBinding> {
    match spec {
        CoprocFdSpec::File(file) => {
            let cloned = file.try_clone().ok()?;
            Some(CoprocFdBinding::File {
                file: Rc::new(FileFd {
                    handle: crate::fd::into_handle(cloned),
                    closed: std::cell::Cell::new(false),
                    adopted: false,
                    path: path.clone().unwrap_or_default(),
                }),
                read: false,
                write: true,
            })
        }
        CoprocFdSpec::Null => Some(CoprocFdBinding::Closed),
        CoprocFdSpec::Pipe(writer) => {
            let cloned = writer.try_clone().ok()?;
            Some(CoprocFdBinding::File {
                file: Rc::new(FileFd {
                    handle: crate::fd::into_handle(cloned),
                    closed: std::cell::Cell::new(false),
                    adopted: false,
                    path: PathBuf::from("coproc:pipe"),
                }),
                read: false,
                write: true,
            })
        }
    }
}

/// Resolve a `N>&M` / `N<&M` dup whose TARGET fd is >= 3 in the
/// coproc redirect walk (rubash#2200): GNU's child dups whatever
/// description the source fd holds at that point in the sequence
/// — a binding established earlier in this walk, an fd-1/2 spec,
/// or the parent shell's fd-table endpoint.
#[allow(clippy::too_many_arguments)]
fn coproc_numbered_dup_binding(
    fd_table: &FdTable,
    bindings: &mut std::collections::BTreeMap<u32, CoprocFdBinding>,
    out_spec: &CoprocFdSpec,
    err_spec: &CoprocFdSpec,
    out_path: &Option<PathBuf>,
    err_path: &Option<PathBuf>,
    fd: u32,
    target: &str,
    write_side: bool,
) -> CoprocDupBind {
    if is_closed_redirect_target(target) {
        bindings.insert(fd, CoprocFdBinding::Closed);
        return CoprocDupBind::Bound(None);
    }
    let Some((source_fd, move_source)) = redirect_target_fd_and_move(target) else {
        // Non-numeric word target: the `>&word` shared-file form
        // is an fd-1/2 shape in GNU (redir.c r_err_and_out); a
        // numbered target with a word source binds nothing here.
        return CoprocDupBind::Unbound;
    };
    if source_fd == fd {
        // A dup of an fd onto itself is a no-op in GNU, and the
        // move form's self-close is excepted (rubash#1236).
        return CoprocDupBind::Unbound;
    }
    let binding = if let Some(binding) = bindings.get(&source_fd) {
        Some(binding.clone())
    } else {
        match source_fd {
            1 => coproc_spec_binding(out_spec, out_path),
            2 => coproc_spec_binding(err_spec, err_path),
            _ => {
                let read_file = match fd_table.read_endpoint(source_fd) {
                    Some(FdReadEndpoint::File(file)) => Some(file),
                    Some(FdReadEndpoint::CoprocStdout { fd, .. }) => Some(fd),
                    _ => None,
                };
                let write_file = match fd_table.write_endpoint(source_fd) {
                    Some(FdWriteEndpoint::File(file)) => Some(file),
                    Some(FdWriteEndpoint::CoprocStdin { fd, .. }) => Some(fd),
                    _ => None,
                };
                match (read_file, write_file) {
                    (Some(read), Some(write)) if Rc::ptr_eq(&read, &write) => {
                        Some(CoprocFdBinding::File {
                            file: read,
                            read: true,
                            write: true,
                        })
                    }
                    (read, write) => {
                        let preferred = if write_side {
                            write
                                .map(|file| (file, false, true))
                                .or_else(|| read.map(|file| (file, true, false)))
                        } else {
                            read.map(|file| (file, true, false))
                                .or_else(|| write.map(|file| (file, false, true)))
                        };
                        preferred.map(|(file, read, write)| CoprocFdBinding::File {
                            file,
                            read,
                            write,
                        })
                    }
                }
            }
        }
    };
    let Some(binding) = binding else {
        return CoprocDupBind::Unbound;
    };
    bindings.insert(fd, binding);
    if move_source {
        if source_fd >= 3 {
            bindings.insert(source_fd, CoprocFdBinding::Closed);
        } else {
            return CoprocDupBind::Bound(Some(source_fd));
        }
    }
    CoprocDupBind::Bound(None)
}

/// The fd-1/2 spec a numbered SOURCE fd contributes to a coproc
/// dup (rubash#2200), with its path label: a binding from this
/// walk first, then the parent fd table's File endpoint. The
/// spec owns a fresh dup of the description.
fn coproc_source_spec(
    fd_table: &FdTable,
    bindings: &std::collections::BTreeMap<u32, CoprocFdBinding>,
    source_fd: u32,
) -> Option<(CoprocFdSpec, Option<PathBuf>)> {
    if let Some(binding) = bindings.get(&source_fd) {
        return match binding {
            CoprocFdBinding::File { file, .. } => {
                let dup = crate::fd::duplicate_handle_inheritable(file.handle).ok()?;
                Some((
                    CoprocFdSpec::File(crate::fd::handle_to_file(dup)),
                    Some(file.path.clone()),
                ))
            }
            CoprocFdBinding::Closed => Some((CoprocFdSpec::Null, None)),
        };
    }
    let file = match fd_table.write_endpoint(source_fd) {
        Some(FdWriteEndpoint::File(file)) => Some(file),
        Some(FdWriteEndpoint::CoprocStdin { fd, .. }) => Some(fd),
        _ => match fd_table.read_endpoint(source_fd) {
            Some(FdReadEndpoint::File(file)) => Some(file),
            Some(FdReadEndpoint::CoprocStdout { fd, .. }) => Some(fd),
            _ => None,
        },
    }?;
    let dup = crate::fd::duplicate_handle_inheritable(file.handle).ok()?;
    Some((
        CoprocFdSpec::File(crate::fd::handle_to_file(dup)),
        Some(file.path.clone()),
    ))
}

/// Outcome of applying a coproc command's own redirects at spawn.
enum CoprocRedirectOutcome {
    /// Spawn proceeds; carries the coproc pipe writer when an fd-1
    /// redirect displaced it from the child's stdout (rubash#1632:
    /// GNU's redirect save slot in the forked child; the caller holds
    /// it for the body's redirect scope — dropped at spawn for bodies
    /// GNU applies non-undoably, dropped at the child's exit for
    /// group/compound bodies, rubash#4008), plus the rubash#2200
    /// numbered-fd transport: the parent-side inheritable dups
    /// backing the child's fd>=3 bindings (the caller closes them
    /// once the spawn settles), the env pairs naming them for a
    /// respawned child, and the (dup, fd) pairs a direct-route
    /// caller pins onto the child's raw fds.
    Proceed {
        displaced_writer: Option<std::io::PipeWriter>,
        fd_dups: Vec<crate::fd::HANDLE>,
        fd_env: Vec<(String, String)>,
        fd_pins: Vec<(crate::fd::HANDLE, u32)>,
    },
    /// A redirect was declined after its diagnostic was reported
    /// (rubash#1644 `2>&word`; rubash#4170 for the disposition).
    /// GNU applies a coproc's redirects in the forked child
    /// (execute_cmd.c:2546 execute_coproc -> execute_in_subshell),
    /// so a do_redirections failure kills only the child: the
    /// parent registers the coproc and execute_coproc returns
    /// EXECUTION_SUCCESS once the fork succeeds
    /// (execute_cmd.c:2592). The caller mirrors that with a
    /// stillborn child carrying `child_stdout` — the fd-1 binding
    /// as it stood when the walk stopped (the coproc pipe writer
    /// unless an earlier fd-1 redirect displaced it) — plus the
    /// `displaced_writer` the Proceed path would have carried,
    /// and `child_stdin`: the fd-0 disposition the walk had
    /// reached (a memory body, then a redirect_in binding, then
    /// the coproc pipe reader), so the pipe's read end lives
    /// exactly as long as GNU's failing child does.
    Declined {
        child_stdout: std::process::Stdio,
        child_stdin: std::process::Stdio,
        displaced_writer: Option<std::io::PipeWriter>,
    },
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

    /// rubash#1584: plan a direct spawn for a simple-command coproc
    /// whose GNU execution ends in execve of an external program.
    ///
    /// GNU execute_cmd.c:2525 execute_coproc make_child(FORK_ASYNC)
    /// forks once, and the child runs execute_in_subshell on the same
    /// parse tree: for a simple external command the forked child
    /// execs the program itself, so the coproc pid ($!, <NAME>_PID)
    /// IS the program's pid (ps shows comm=cat for `coproc cat`).
    /// Rubash's respawn route interposes a `rubash -c` wrapper, so $!
    /// named the wrapper and the program ran as a grandchild — kill
    /// $! hit the wrapper, and process inspection saw the wrong
    /// image. This planner admits the direct route only where the
    /// parent can reproduce the child's view exactly:
    ///
    /// * the body is a simple command with no prefix assignments
    ///   (tempenv binding is the child shell's work);
    /// * every word's expansion is context-free between parent and
    ///   coproc child: no command/process substitution and no
    ///   arithmetic expansion (the child evaluates those AFTER the
    ///   fork, asynchronously and in its own variable scope), no
    ///   process-identity dynamics (BASHPID/PPID/BASH_SUBSHELL —
    ///   the child's values name the coproc process itself), and no
    ///   RANDOM/SRANDOM draws (the child's sequence is a fork copy;
    ///   a parent draw would advance the wrong sequence);
    /// * error placement stays in the child: under `set -u` any
    ///   expansion at all, and under failglob any glob word, stands
    ///   down, because a parent-side expansion failure would report
    ///   (and, for -u, abort) in the parent where GNU's failure is
    ///   scoped to the coproc child;
    /// * the expanded head resolves the way the child's dispatch
    ///   would resolve it, minus the shell: a function, alias, or
    ///   builtin head stands down (GNU's child runs those AS a
    ///   shell, so the wrapper's shell image is the faithful one),
    ///   as does an unresolvable name (the child owns the 127);
    /// * the resolved file is a kernel-exec'able image (ELF, Mach-O,
    ///   or a `#!` script): an exec-format failure in GNU reruns
    ///   the file as a shell script inside the child, which only
    ///   the respawn route can reproduce.
    ///
    /// Every stand-down falls back to the respawn route unchanged,
    /// so a miss costs process-tree fidelity, never semantics. The
    /// route is Unix-only for now: the Windows spawn model (no
    /// fork/exec, handle stdio) keeps the respawn route until its
    /// own adjudication.
    fn plan_direct_coproc_command(
        &mut self,
        cmd: &CommandNode,
        coproc_cmd: &crate::parser::CoprocCommand,
    ) -> Option<(std::path::PathBuf, Vec<String>)> {
        #[cfg(not(unix))]
        return {
            let _ = (cmd, coproc_cmd);
            None
        };
        #[cfg(unix)]
        self.plan_direct_coproc_command_unix(cmd, coproc_cmd)
    }

    #[cfg(unix)]
    fn plan_direct_coproc_command_unix(
        &mut self,
        cmd: &CommandNode,
        coproc_cmd: &crate::parser::CoprocCommand,
    ) -> Option<(std::path::PathBuf, Vec<String>)> {
        {
            use std::os::unix::fs::PermissionsExt as _;
            if coproc_cmd.body.is_some()
                || coproc_cmd.body_kind != crate::parser::CoprocBodyKind::SimpleCommand
            {
                return None;
            }
            let words = &coproc_cmd.words;
            if words.is_empty() || words[0].starts_with('-') {
                return None;
            }
            // Prefix assignments (tempenv) bind in the child shell.
            // The first word that is not assignment-shaped is the
            // head; any assignment before it stands the route down,
            // as does an all-assignment body (no command at all).
            let mut head_index = 0;
            while head_index < words.len() {
                let word = &words[head_index];
                let raw = coproc_cmd
                    .word_metadata
                    .get(head_index)
                    .map(|meta| meta.raw.as_str())
                    .unwrap_or(word.as_str());
                let Some(eq) = raw.find('=') else { break };
                let name = raw[..eq].strip_suffix('+').unwrap_or(&raw[..eq]);
                if name.is_empty() || !is_shell_name(name) || !raw.starts_with(name) {
                    break;
                }
                head_index += 1;
            }
            if head_index > 0 {
                return None;
            }
            let nounset =
                crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset");
            let failglob =
                crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "failglob");
            for (index, word) in words.iter().enumerate() {
                let meta = coproc_cmd.word_metadata.get(index);
                let value = meta.map(|meta| meta.value.as_str()).unwrap_or(word);
                let raw = meta.map(|meta| meta.raw.as_str()).unwrap_or(word);
                for text in [word.as_str(), value, raw] {
                    if text.contains("$(")
                        || text.contains('`')
                        || text.contains("<(")
                        || text.contains(">(")
                        || text.contains("BASHPID")
                        || text.contains("PPID")
                        || text.contains("BASH_SUBSHELL")
                        || text.contains("RANDOM")
                        || text.contains("SRANDOM")
                    {
                        return None;
                    }
                    if nounset && text.contains('$') {
                        return None;
                    }
                    if failglob && (text.contains('*') || text.contains('?') || text.contains('['))
                    {
                        return None;
                    }
                }
            }
            // Redirect targets are expanded by the parent on both
            // routes, but a process substitution among them is
            // created for the executing child; an {fd} allocation
            // binds in the parent shell. Both stay with the respawn.
            let mut redirect_texts: Vec<&str> = cmd
                .redirects
                .iter()
                .map(|redirect| redirect.target.as_str())
                .collect();
            if let Some(redirect_in) = &cmd.redirect_in {
                redirect_texts.push(redirect_in.target.as_str());
                if redirect_in.fd_var.is_some() {
                    return None;
                }
            }
            if cmd
                .redirects
                .iter()
                .any(|redirect| redirect.fd_var.is_some())
            {
                return None;
            }
            for text in redirect_texts {
                if text.contains("<(") || text.contains(">(") {
                    return None;
                }
            }
            // Expand in the parent with the ordinary simple-command
            // pipeline; the admitted word classes make this a pure
            // read of the same table the child's snapshot would hold.
            let mut scratch = CommandNode::new();
            scratch.line = cmd.line;
            scratch.logical_line = cmd.logical_line;
            scratch.words = words.clone();
            scratch.word_metadata = coproc_cmd.word_metadata.clone();
            let expanded = self.expand_command_words(&scratch).ok()?;
            if !expanded.assignments.is_empty() {
                return None;
            }
            let argv = expanded.words;
            let head = argv.first()?;
            if self.shell_state.functions.contains_key(head)
                || self.shell_state.aliases.contains_key(head)
                || is_shell_builtin_name(head)
                || is_shell_keyword(head)
            {
                return None;
            }
            let program = if head.contains('/') {
                let path = if std::path::Path::new(head).is_absolute() {
                    std::path::PathBuf::from(head)
                } else {
                    self.shell_state.cwd.join(head)
                };
                path
            } else {
                std::path::PathBuf::from(self.command_path_without_bridge(head, false)?)
            };
            let metadata = std::fs::metadata(&program).ok()?;
            if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
                return None;
            }
            // Kernel-exec'able image check: ELF, Mach-O (either
            // endianness, thin or fat), or a `#!` interpreter script.
            // Anything else fails execve with ENOEXEC, which GNU
            // answers by running the file as a shell script in the
            // child — respawn-route territory.
            let mut magic = [0u8; 4];
            {
                use std::io::Read as _;
                let mut file = std::fs::File::open(&program).ok()?;
                let read = file.read(&mut magic).ok()?;
                if read < 2 {
                    return None;
                }
            }
            let execable = magic[..2] == *b"#!"
                || magic == [0x7f, b'E', b'L', b'F']
                || magic == [0xfe, 0xed, 0xfa, 0xce]
                || magic == [0xfe, 0xed, 0xfa, 0xcf]
                || magic == [0xce, 0xfa, 0xed, 0xfe]
                || magic == [0xcf, 0xfa, 0xed, 0xfe]
                || magic == [0xca, 0xfe, 0xba, 0xbe]
                || magic == [0xbe, 0xba, 0xfe, 0xca];
            if !execable {
                return None;
            }
            Some((program, argv))
        }
    }

    /// rubash#3267: spellings a fresh `-c` re-parse reads as non-words
    /// at command position — `!`, `{`, `}`, `[[`, `]]`, and the
    /// reserved-word set. The coproc simple-body respawn re-renders
    /// collected words back to source text, so an unquoted head word
    /// with one of these spellings regains keyword/operator status in
    /// the child (`coproc >x done` came back a DONE token and errored
    /// as a syntax error where GNU's forked child ran `done` and
    /// reported `done: command not found`). `time` is deliberately
    /// included even though GNU keeps it a timespec (parse.y:1416):
    /// the #1653 pins assert `time: command not found` and rewriting
    /// them to timespec output is a separate ledger.
    fn coproc_head_word_needs_quote(word: &str) -> bool {
        matches!(word, "!" | "{" | "}" | "[[" | "]]")
            || crate::executor::support_names::is_reserved_word(word)
    }

    pub(in crate::executor) fn execute_coproc_command(
        &mut self,
        cmd: &CommandNode,
        coproc_cmd: &crate::parser::CoprocCommand,
    ) -> Result<(), ExecuteError> {
        let mut array_name = coproc_cmd
            .name
            .clone()
            .unwrap_or_else(|| "COPROC".to_string());
        // GNU execute_cmd.c:2502-2513 execute_coproc expands the name
        // BEFORE validating — expand_string_unsplit_to_string
        // (command->value.Coproc->name, 0) — then valid_identifier on
        // the EXPANSION, and the expanded text replaces the name so the
        // coproc's fd array and _PID variable land under it. When the
        // word expands to zero fields the expansion is NULL (an
        // unquoted unset/empty `$nv`), and err_invalidid
        // (error.c:459-462) renders the NULL %s as `(null)`; a quoted
        // word still yields one field — even an empty one — and
        // reports `` `': `` (rubash#3032; the `coproc ""` shape is
        // rubash#1153's). The default COPROC name never takes this
        // branch, and a nameref name is a valid identifier itself —
        // its target is adjudicated at bind time below.
        if let Some(name) = &coproc_cmd.name {
            let raw = coproc_cmd
                .name_metadata
                .as_ref()
                .map(|meta| meta.raw.as_str())
                .unwrap_or(name.as_str());
            let expanded = self.expand_word(raw);
            let expanded_name = if expanded.is_empty() && !raw.contains('"') && !raw.contains('\'')
            {
                None
            } else {
                Some(expanded)
            };
            let name_text = expanded_name.as_deref().unwrap_or("(null)");
            if !is_shell_name(name_text) {
                eprintln!(
                    "{}`{}': not a valid identifier",
                    self.diagnostic_prefix(),
                    name_text
                );
                self.exit_code = 1;
                return Ok(());
            }
            array_name = name_text.to_string();
        }
        use std::process::{Command, Stdio};
        let exe = test_rubash_binary_from_env()
            .or_else(test_rubash_binary_from_current_exe)
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| "rubash".into());

        // rubash#1584: a simple-command coproc that the planner
        // admits spawns the program itself as the coproc process;
        // everything below (pipes, stderr plan, coproc redirects,
        // job registration, endpoint binding) is shared with the
        // respawn route — only the child image and its environment
        // differ (an external child gets the ordinary external
        // environment, not the shell-state snapshot a `rubash -c`
        // child re-imports).
        let direct_plan = self.plan_direct_coproc_command(cmd, coproc_cmd);
        // rubash#2356: byte size of the respawn argv, for the
        // state-channel spill's aggregate budget (the respawn route
        // gathers its environment into a Vec first — see the
        // direct_plan.is_none() block below).
        let mut respawn_argv_bytes = exe.as_os_str().len() + 16;
        let mut child = if let Some((program, argv)) = &direct_plan {
            // rubash#2442: a direct coproc child is spawned from the
            // same execute_disk_command resolution as any external
            // (execute_cmd.c:5810 -> 6133) — its execve filename comes
            // from the shared funnel and argv[0] is the typed word,
            // mirroring the named-program constructor. The shared
            // tail below births the child in the session cwd, which
            // is what a relative join resolves against; a plan whose
            // program the funnel cannot reproduce (the bridge-aware
            // lookup above) declines to the anchored program.
            let exec_name = crate::executor::path::spawn_exec_name(
                &argv[0],
                program,
                &self.shell_state.env_vars,
            );
            let mut command = match &exec_name {
                Some(name) => Command::new(name),
                None => Command::new(program),
            };
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.arg0(&argv[0]);
            }
            command.args(&argv[1..]);
            self.apply_external_environment(&CommandNode::new(), &mut command, program);
            command
        } else if let Some(body) = &coproc_cmd.body {
            // Compound command body: coproc [NAME] { body; } or ( body )
            let body_text = bash_command_sequence_text(body);
            respawn_argv_bytes += body_text.len();
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
                    respawn_argv_bytes += word.len() + 1;
                    child.arg(word);
                }
            } else {
                // rubash#1653 + rubash#3267: the child re-parses this
                // text, so a bare head word whose spelling is a
                // keyword or operator AT COMMAND POSITION would regain
                // non-word status on re-parse even though the coproc
                // parser already bound it as a word — GNU forks the
                // parse tree, so a `!` collected after a redirection
                // stays the command name (`coproc >x ! echo hi`
                // reports `!: command not found` in GNU; `done`, `}`,
                // `[[`, `]]`, `function` the same). Quote such a head
                // word's raw spelling so the re-parse keeps it the
                // command name. The quote must be `"` — not `'`: the
                // lexer folds `'!'` back into a BANG token (GNU's
                // lexer never does; the fold lives in continuation.rs
                // and is a captain-lane residual). `{`, `coproc` and
                // `time` are fold-blocked in every quoting form, so
                // head words with those spellings still re-parse as
                // their keywords — the same anti-GNU residual the
                // #1653 `time` expectations already pin. A head word
                // already quoted in source keeps its own raw
                // spelling.
                let mut metadata = coproc_cmd.word_metadata.clone();
                if let Some(meta) = metadata.first_mut() {
                    if meta.raw == meta.value && Self::coproc_head_word_needs_quote(&meta.value) {
                        meta.raw = format!("\"{}\"", meta.value);
                    }
                }
                let words_text = command_words_source_text(&coproc_cmd.words, &metadata);
                respawn_argv_bytes += words_text.len();
                child.arg("-c").arg(words_text);
            }
            child
        } else if !cmd.redirects.is_empty()
            || !cmd.heredoc_redirects.is_empty()
            || cmd.heredoc.is_some()
        {
            // rubash#2746: a redirection-only coproc body (`coproc >
            // file') — GNU's grammar admits a simple command of only
            // redirections as the shell_command, and its child applies
            // them (through the coproc redirect walk below) and exits.
            // The respawn child runs an empty command string.
            let mut child = Command::new(&exe);
            child.arg("-c").arg("");
            child
        } else {
            eprintln!(
                "{}coproc: usage: coproc [NAME] command [args...]",
                self.diagnostic_prefix()
            );
            self.exit_code = 1;
            return Ok(());
        };

        // The env snapshot and the rubash-child state keys below are
        // the respawn route's inheritance transport; a direct child
        // (rubash#1584) is a foreign program and already received
        // the ordinary external environment at construction.
        //
        // rubash#2356: the respawn route gathers its entries into a
        // Vec first (a Command's env cannot be read back), so the
        // state-channel spill can move oversized entries onto the
        // state-file transport before anything is applied. Entry
        // order is preserved exactly; the spill + apply happen just
        // before the spawn, so no temp file exists on the
        // redirect-error / pipe-failure paths in between.
        // rubash#3300: mint the respawn proof before the carrier
        // dups are created (apply_coproc_redirects below) — respawn
        // route only; a direct (foreign) child carries no proof. A
        // mint failure is fail-open (None): the spawn proceeds
        // without proof keys, i.e. today's behavior.
        #[cfg(unix)]
        let respawn_proof = if direct_plan.is_none() {
            crate::fd::mint_respawn_proof(&crate::builtins::kill::new_background_wrapper_nonce())
        } else {
            None
        };
        let mut env_entries: Vec<(String, String)> = Vec::new();
        if direct_plan.is_none() {
            // rubash#2486: the snapshot materializes the attribute
            // markers (`__RUBASH_*_VARS`) the write-behind live table no
            // longer carries; the coproc fork (execute_cmd.c coproc_getfd
            // -> make_child) inherits the parent's attributes whole.
            for (key, value) in self.env_vars_snapshot() {
                if !key.starts_with("__RUBASH_") || rubash_spawn_inherited_state(&key) {
                    env_entries.push((key, value));
                }
            }
            // rubash#2894: GNU's coproc child is a fork that passes
            // through execute_in_subshell (execute_cmd.c:2546,
            // SUBSHELL_COPROC), which runs reset_signal_handlers with
            // SUBSHELL_RESETTRAP — the parent's trap strings stay
            // VISIBLE in the child (`trap -p` lists them) with their
            // dispositions reset, exactly the shape the async-subshell
            // respawn path forwards (rubash#1630). The snapshot above
            // admits no trap-table keys, so run the same transform on
            // a clone of the live table and forward the resulting
            // __RUBASH_TRAP* entries; the child seeds the identical
            // reset-but-listed table and nothing fires there.
            env_entries.retain(|(key, _)| !key.starts_with("__RUBASH_TRAP"));
            let mut child_trap_table = self.shell_state.env_vars.clone();
            crate::builtins::trap::reset_for_subshell_child(&mut child_trap_table);
            for (key, value) in &child_trap_table {
                if key.starts_with("__RUBASH_TRAP") {
                    env_entries.push((key.clone(), value.clone()));
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
                env_entries.push((
                    exported_function_env_name(name),
                    exported_function_env_value(&body.commands, def_redirects),
                ));
            }
            // Preserve the parent script location for diagnostics emitted by the
            // coprocess shell, while keeping internal executor state isolated.
            if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
                env_entries.push(("__RUBASH_SCRIPT_NAME".to_string(), script.clone()));
                // rubash#2058: seed where the transported body text
                // starts (first leaf command's line), not the coproc
                // node's own line. Simple-command coprocs keep the
                // node's line — the command IS the body.
                let line = coproc_cmd
                    .body
                    .as_ref()
                    .and_then(|body| body.first())
                    .and_then(first_leaf_command_line)
                    .or_else(|| cmd.line)
                    .map(|line| line.to_string())
                    .or_else(|| {
                        self.shell_state
                            .env_vars
                            .get("__RUBASH_CURRENT_LINE")
                            .cloned()
                    })
                    .unwrap_or_else(|| "1".to_string());
                env_entries.push(("__RUBASH_CURRENT_LINE".to_string(), line.clone()));
                // The child re-parses `-c` source from line 1. Preserve the
                // parent's physical call-site line when assigning child AST lines.
                let line_offset = line.parse::<usize>().unwrap_or(1).saturating_sub(1);
                env_entries.push(("__RUBASH_LINE_OFFSET".to_string(), line_offset.to_string()));
            }
            // rubash#2032: the coproc child is a GNU fork (execute_cmd.c
            // coproc_getfd -> make_child), so it inherits the parent's
            // positional parameters like every other fork-shaped respawn
            // (POSITIONAL_PARAMS_TRANSPORT).
            env_entries.push((
                POSITIONAL_PARAMS_TRANSPORT.to_string(),
                self.positional_params_transport_value(),
            ));
            // rubash#2058: the fork-context stamp, like every other
            // fork-shaped respawn (see fork_context_transport_entries).
            for (key, value) in self.fork_context_transport_entries() {
                env_entries.push((key, value));
            }
            // rubash#3605: the coproc child executes the parent's
            // flagged command node (execute_cmd.c:2546
            // execute_coproc -> execute_in_subshell), so a `!`- or
            // condition-suppressed coproc keeps CMD_IGNORE_RETURN
            // and `set -e` inside is ignored
            // (SUPPRESS_ERREXIT_TRANSPORT).
            if self.suppress_errexit > 0 {
                env_entries.push((SUPPRESS_ERREXIT_TRANSPORT.to_string(), "1".to_string()));
            }
            // rubash#2332: the compound body's line spans, collected
            // over the same list bash_command_sequence_text rendered
            // into the child's `-c` source above; prelude count 0 —
            // coproc children take their functions via the BASH_FUNC
            // entries, not prelude text. Simple-command coprocs have
            // no body statements to re-seed (the command IS the body
            // and keeps the node-line seed above). The stamp joins
            // env_entries (not a direct child.env) so it rides the
            // rubash#2356 state-channel spill with every sibling
            // transport entry at this site.
            if let Some(body) = &coproc_cmd.body {
                let mut root_spans = Vec::new();
                crate::parser::collect_command_line_spans(body, &mut root_spans);
                if !root_spans.is_empty() {
                    env_entries.push((
                        FORK_ROOT_SPANS_TRANSPORT.to_string(),
                        encode_root_line_spans(0, &root_spans),
                    ));
                }
            }
            // rubash#2058: this site never stamped the shell-pid
            // carrier, so the coproc child's $$ projected neither the
            // parent shell's pid (the #1582 projection its sibling
            // spawn sites produce) nor its own BASHPID.
            env_entries.push(("__RUBASH_SHELL_PID".to_string(), self.shell_pid.to_string()));
            // Coprocess children own a shell-created stdin pipe. Keep builtin
            // readers attached to it and let TERM use native termination when a
            // blocked read cannot consume the shell signal mailbox.
            env_entries.push((INHERIT_PROCESS_STDIN.to_string(), "1".to_string()));
            env_entries.push(("__RUBASH_COPROC_CHILD".to_string(), "1".to_string()));
            // rubash#2136: provenance stamp — the child's init validates
            // the coproc claim against its actual parent (init.rs).
            env_entries.push((
                crate::executor::RESPAWN_SPAWNER_PID_KEY.to_string(),
                crate::executor::spawner_stamp_value(),
            ));
            // rubash#3300: the per-spawn proof pair (fd number +
            // nonce). Per-spawn, never forwarded — NOT in
            // FORK_TRANSPORT_KEYS / rubash_spawn_inherited_state.
            #[cfg(unix)]
            if let Some((proof_fd, proof_nonce)) = &respawn_proof {
                env_entries.push((
                    crate::executor::RESPAWN_PROOF_FD_KEY.to_string(),
                    proof_fd.to_string(),
                ));
                env_entries.push((
                    crate::executor::RESPAWN_PROOF_KEY.to_string(),
                    proof_nonce.clone(),
                ));
            }
            // rubash#1074: the coproc body runs in a forked subshell
            // (execute_in_subshell: subshell_level++), so the child's
            // BASH_SUBSHELL starts at this shell's depth + 1.
            env_entries.push((
                "__RUBASH_SUBSHELL_DEPTH".to_string(),
                (self.shell_state.subshell_depth.get() + 1).to_string(),
            ));
            // rubash#1582: the same fork makes the coproc child's own
            // pid the pid of the seeded level (variables.c:1511
            // get_bashpid = getpid), so the BASHPID origin base rides
            // with the depth — the `&` respawn and both procsub forms
            // stamp the pair, and without the base this child's
            // $BASHPID read its real pid + 1 while its own children
            // observed the real pid as $PPID.
            env_entries.push((
                "__RUBASH_BG_BASHPID_BASE".to_string(),
                (self.shell_state.subshell_depth.get() + 1).to_string(),
            ));
        }

        // Create pipes for bidirectional communication
        let stdin_result = std::io::pipe();
        let stdout_result = std::io::pipe();

        // rubash#2200: parent-side dups backing the child's
        // numbered-fd bindings; closed after the spawn settles
        // (the child holds its own copies by then).
        let mut coproc_fd_dups: Vec<crate::fd::HANDLE> = Vec::new();
        if let (Ok((stdin_reader, stdin_writer)), Ok((stdout_reader, stdout_writer))) =
            (stdin_result, stdout_result)
        {
            // rubash#4170: the pipe reader is handed to
            // apply_coproc_redirects, which owns the child's fd-0
            // disposition (the Proceed path binds it as the
            // default stdin; a Declined walk returns it in the
            // outcome so the stillborn child holds the read end
            // for exactly its own lifetime, like GNU's child).
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
            // rubash#4170 (GNU execute_cmd.c:2546 + :2592): the
            // coproc child's own redirect failure is the CHILD's
            // exit status (reaped by `wait` as 1); the launch
            // statement itself reports the fork's success, and an
            // enclosing `!` inverts that 0 like any coproc launch.
            // (The rubash#846 inverted-exit convention governs a
            // coproc's status as a pipeline stage, not the launch
            // statement.)
            let displaced_stdout_writer = match self.apply_coproc_redirects(
                cmd,
                &mut child,
                stdin_reader,
                stdout_writer,
                &coproc_stderr_target,
            ) {
                Ok(CoprocRedirectOutcome::Proceed {
                    displaced_writer,
                    fd_dups,
                    fd_env,
                    fd_pins,
                }) => {
                    // rubash#2200: a respawned child learns its
                    // numbered-fd bindings from the env pairs (its
                    // init restores them into its fd table); a
                    // direct (foreign-program) child gets them
                    // pinned onto its raw fds on unix — registered
                    // BEFORE the displaced-writer pre_exec below,
                    // whose first-free-slot scan must skip the
                    // pinned numbers.
                    if direct_plan.is_none() {
                        for (key, value) in fd_env {
                            child.env(key, value);
                        }
                    } else {
                        let _ = &fd_env;
                        #[cfg(unix)]
                        if !fd_pins.is_empty() {
                            let pins = fd_pins.clone();
                            unsafe {
                                use std::os::unix::process::CommandExt as _;
                                child.pre_exec(move || {
                                    crate::executor::external_inner::pin_ambient_fds_in_child(
                                        &pins,
                                        &[],
                                    )
                                    .map(|_| ())
                                });
                            }
                        }
                        #[cfg(not(unix))]
                        let _ = &fd_pins;
                    }
                    coproc_fd_dups = fd_dups;
                    displaced_writer
                }
                Ok(CoprocRedirectOutcome::Declined {
                    child_stdout,
                    child_stdin,
                    displaced_writer,
                }) => {
                    // rubash#4170: the ordered pass declined a
                    // redirect after reporting its diagnostic
                    // through the fd-2 binding in force at the
                    // failure point. GNU's coproc child applies
                    // the redirects after the fork
                    // (execute_cmd.c:2546 execute_in_subshell) and
                    // exits 1 when they fail, while the parent
                    // registers the coproc and execute_coproc
                    // returns EXECUTION_SUCCESS
                    // (execute_cmd.c:2592) — the #846 convention
                    // the old arm cited governs the child's exit
                    // status inside pipelines, not the launch
                    // statement's. Mirror it: swap the body child
                    // for a stillborn respawn that exits 1 without
                    // running the body — carrying the walk's fd-0
                    // disposition, so the coproc pipe's read end
                    // (when no body / redirect_in binding took it)
                    // lives until the stillborn child exits,
                    // exactly as long as GNU's failing child holds
                    // it — and fall through to the shared
                    // spawn/registration below, which binds the
                    // COPROC array/_PID, the job entry and $! and
                    // leaves the statement status 0. `wait` reaps
                    // the stillborn child's 1, and the ordinary
                    // death bookkeeping retires the coproc.
                    let mut doomed = Command::new(&exe);
                    doomed.arg("-c").arg("exit 1");
                    doomed.stdin(child_stdin);
                    doomed.stdout(child_stdout);
                    child = doomed;
                    displaced_writer
                }
                Err(error) => {
                    if matches!(error, ExecuteError::IoError(_)) {
                        self.note_compound_redirect_failure();
                    }
                    return Err(error);
                }
            };
            // rubash#1632: a coproc-level `>file`/`>>file` (or `>&-`)
            // displaced the pipe writer from the child's stdout. GNU
            // binds the trailing redirect list to the INNER command
            // (parse.y:1130/1153 `COPROC shell_command redirection_list`
            // appends it to $2->redirects), so the displaced end's
            // lifetime is the inner command's own redirect scope: a
            // `{ }` group or compound body applies it undoably
            // (execute_cmd.c:854 do_redirections RX_ACTIVE|RX_UNDOABLE
            // -> redir.c add_undo_redirect's fcntl F_DUPFD to a save
            // slot >= SHELL_FD_BASE), so the write end lives until the
            // body finishes — the parent's read end sees no EOF while
            // the coproc lives (a `read -t` times out instead of
            // reporting EOF, and the file receives the data). A `( )`
            // subshell body or a simple command applies its redirects
            // without the undo list (execute_cmd.c:1761/5522
            // RX_ACTIVE), so the displaced end closes at apply time and
            // the read end sees EOF immediately (rubash#4008). On Unix
            // a live save rides into the child at the first free fd in
            // the save-slot range (dup2 clears CLOEXEC on the target,
            // so it survives the exec); elsewhere a detached waiter
            // drops the parent-held writer when the child exits — the
            // parent must never hold it past the coproc's death, or
            // `cat <&${COPROC[0]}` hangs forever waiting on an EOF the
            // retirement-deferred drop never delivered.
            let hold_until_child_exit = matches!(
                coproc_cmd.body_kind,
                crate::parser::CoprocBodyKind::BraceGroup
                    | crate::parser::CoprocBodyKind::CompoundCommand
                    | crate::parser::CoprocBodyKind::CommandSequence
            );
            #[cfg(unix)]
            if hold_until_child_exit {
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
            }

            // rubash#2241: a coproc is an async child — under job
            // control it leads its own process group like any other
            // async spawn (GNU jobs.c:2390 make_child).
            #[cfg(unix)]
            if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor") {
                unsafe {
                    use std::os::unix::process::CommandExt as _;
                    child.pre_exec(crate::builtins::kill::child_pre_exec_setpgid);
                }
            }
            // rubash#2893: the coproc child must not inherit a blocked
            // SIGPIPE mask across the exec — when the coproc command
            // runs as a pipeline stage, this spawn happens under the
            // stage's sigpipe_block_guard, and the inherited block made
            // the child survive EPIPE and print a spurious
            // `Broken pipe' diagnostic. GNU's forked child carries the
            // shell's ordinary (unblocked, SIG_DFL) disposition.
            #[cfg(unix)]
            unsafe {
                use std::os::unix::process::CommandExt as _;
                child.pre_exec(crate::builtins::kill::child_pre_exec_unblock_sigpipe);
            }
            // rubash#3300: the mint already cleared CLOEXEC on the
            // proof fd; re-clear it here so the fd survives exec at
            // its number even if a carrier pass re-armed it (the
            // pin-pass pre_exec above never touches it, but the
            // guarantee is cheap). The parent's copy closes with the
            // other numbered-fd dups after the spawn attempt (the
            // loop at the end of this fn, rubash#2200).
            #[cfg(unix)]
            if let Some((proof_fd, _)) = respawn_proof {
                unsafe {
                    use std::os::unix::process::CommandExt as _;
                    child.pre_exec(move || {
                        let flags = libc::fcntl(proof_fd, libc::F_GETFD);
                        if flags >= 0 {
                            libc::fcntl(proof_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                        }
                        Ok(())
                    });
                }
                coproc_fd_dups.push(proof_fd);
            }
            suppress_orphan_console(&mut child);
            self.stamp_spawn_cwd(&mut child);
            // rubash#2356: spill + apply the gathered respawn
            // environment (see the gather at the construction site).
            // A direct (foreign) child gathered nothing and keeps
            // the ordinary external environment from construction.
            let mut state_temp: Option<PathBuf> = None;
            if direct_plan.is_none() {
                let (entries, temp) =
                    self.spill_respawn_env_entries(env_entries, respawn_argv_bytes);
                state_temp = temp;
                for (key, value) in entries {
                    // rubash#3965 (captain graft of #4010): this arm is the
                    // respawn STATE channel, not a foreign-child boundary —
                    // the entries are the parent's stored spellings and a
                    // validated respawn's import restores them verbatim
                    // (init.rs). Decoding raw-byte marker pairs to raw
                    // bytes here would hand the child a non-UTF-8 env
                    // entry, which its process_env_pairs import drops
                    // wholesale (rubash#2064): the variable would arrive
                    // UNSET instead of carrying its byte.
                    child.env(key, value);
                }
            }
            // rubash#3219: a coproc child born inside an in-process
            // fork region takes the region's recorded ulimit pairs at
            // birth (see spawn_external_process's arm) — GNU's coproc
            // child is a fork of the region's shell and carries its
            // pair. No-op when no emulated-limit cells exist.
            crate::builtins::ulimit::arm_emulated_limits(&mut child, &self.shell_state.env_vars);
            match child.spawn() {
                Ok(mut child_proc) => {
                    // Rubash does not expose real coproc file descriptors yet,
                    // but keep the parent ends until after spawn so stdio uses
                    // the correct pipe direction on all hosts.
                    let pid = child_proc.id();
                    adopt_orphan_console(pid);
                    // rubash#1632/#4008: settle the displaced stdout
                    // writer — on Unix the child holds its own copy
                    // (pre_exec) and the parent's copy closes here;
                    // elsewhere a detached waiter drops it when the
                    // child exits (GNU's save slot is a child fd, so it
                    // dies with the coproc — the old retirement-lifetime
                    // parent hold left ${COPROC[0]} readers blocked past
                    // the coproc's death forever). When GNU closes the
                    // displaced end at apply time (simple/`( )` bodies),
                    // the writer drops right here — immediate EOF.
                    #[cfg(unix)]
                    drop(displaced_stdout_writer);
                    #[cfg(not(unix))]
                    match (hold_until_child_exit, displaced_stdout_writer) {
                        (true, Some(writer)) => {
                            Self::hold_coproc_writer_until_child_exit(&child_proc, writer);
                        }
                        (_, writer) => drop(writer),
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
                    // rubash#2241: parent half of the grouping
                    // (GNU jobs.c:2455).
                    #[cfg(unix)]
                    if job_control {
                        crate::builtins::kill::group_spawned_child(pid);
                    }
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
                    // GNU prints the async start notice for a coproc
                    // too (execute_cmd.c:2580 DESCRIBE_PID (coproc_pid),
                    // same `interactive` gate as any async launch —
                    // verified against GNU 5.3.20 under a PTY); route it
                    // through the same #2214-gated notify as `&` spawns.
                    self.notify_async_job_started(job_id, pid);
                    // GNU sh_openpipe moves the pipe ends to the highest free
                    // fds below 64 (move_to_high_fd with maxfd 64): rpipe
                    // 63/62 and wpipe 61/60, of which the parent keeps 63 and
                    // 60. All three coprocs in coproc.tests reuse that same
                    // pair, so `${COPROC[@]}` is literally "63 60". The parent
                    // ends live in fd_table as real HANDLE endpoints (Rc'd
                    // FileFd), so dup/close/fork share them like GNU's fork.
                    let coproc_write_file = Rc::new(FileFd {
                        handle: crate::fd::into_handle(stdin_writer),
                        closed: std::cell::Cell::new(false),
                        adopted: false,
                        path: std::path::PathBuf::from(format!("coproc:{pid}:stdin")),
                    });
                    let coproc_read_file = Rc::new(FileFd {
                        handle: crate::fd::into_handle(stdout_reader),
                        closed: std::cell::Cell::new(false),
                        adopted: false,
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
                        self.apply_shell_assignment(&pid_name, pid.to_string(), None);
                    }
                    self.exit_code = 0;
                }
                Err(e) => {
                    if let Some(path) = &state_temp {
                        let _ = fs::remove_file(path);
                    }
                    eprintln!("{}coproc: failed to spawn: {}", self.diagnostic_prefix(), e);
                    self.exit_code = 126;
                }
            }
        } else {
            eprintln!("{}coproc: failed to create pipes", self.diagnostic_prefix());
            self.exit_code = 1;
        }

        // rubash#2200: the child holds its own copies of the
        // numbered-fd dups (init-restored handles on the respawn
        // route, raw-fd pins on the direct route); the parent's
        // close here, on every path past the spawn attempt. (The
        // redirect-error returns above precede dup creation in
        // apply_coproc_redirects, so there is nothing to close on
        // those paths; a Declined launch reaches here carrying at
        // most the respawn proof dup, which the stillborn child
        // never reads.)
        for dup in coproc_fd_dups {
            crate::fd::close_handle(dup);
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
            // rubash#1898: fd 2 bound to /dev/full — the child's
            // stderr writes are absorbed (the ENOSPC profile cannot
            // cross the spawn boundary; the in-shell faces carry it).
            Some(FdWriteEndpoint::FullDevice) => CoprocStderrPlan::Direct(Stdio::null()),
            // rubash#2042: fd 2 bound to a 0644-class synthetic /proc
            // content entry — same boundary: the EINVAL write profile
            // cannot cross into the child's handle; writes absorb.
            Some(FdWriteEndpoint::ProcContentWrite) => CoprocStderrPlan::Direct(Stdio::null()),
            // rubash#2385: fd 2 bound to a complementary channel end
            // — same boundary: the in-memory queue cannot cross the
            // spawn boundary; the child's stderr writes absorb (the
            // in-shell fd-2 faces append to the queue exactly).
            Some(FdWriteEndpoint::PipeChannel(_)) => CoprocStderrPlan::Direct(Stdio::null()),
        }
    }

    /// Applies the coproc command's output redirects to the child
    /// in list order, on top of its stdin body / input redirect.
    /// Returns Proceed (carrying the displaced pipe writer, if any)
    /// to continue to spawn, or Declined when a redirect was
    /// refused after reporting its diagnostic through the fd-2
    /// binding in force at the failure point (rubash#4170: the
    /// caller then registers a stillborn coproc and the statement
    /// reports 0, per execute_cmd.c:2592).
    /// rubash#4008: on hosts without child-fd injection, GNU's
    /// redirect save slot (redir.c add_undo_redirect — a CHILD fd
    /// holding the displaced coproc pipe writer for the body's
    /// redirect scope) is emulated by a detached thread that waits on
    /// a duplicate of the child process handle and drops the writer
    /// at the child's exit. The parent's `${COPROC[0]}` read end
    /// therefore sees EOF exactly when the coproc dies, matching
    /// GNU's child-lifetime hold instead of holding the writer to
    /// coproc retirement — a `cat <&${COPROC[0]}` / `read -u` issued
    /// before the next disposal point used to block forever on an EOF
    /// the retired-later drop never delivered.
    #[cfg(not(unix))]
    fn hold_coproc_writer_until_child_exit(
        child: &std::process::Child,
        writer: std::io::PipeWriter,
    ) {
        use std::os::windows::io::AsRawHandle as _;
        fn wait_process_exit(h: crate::fd::HANDLE) {
            use windows_sys::Win32::System::Threading::WaitForSingleObject;

            unsafe { WaitForSingleObject(crate::win32_ffi::sys_handle(h), u32::MAX) };
        }
        let raw = child.as_raw_handle() as crate::fd::HANDLE;
        let Ok(watch) = crate::fd::duplicate_handle(raw) else {
            // No watch possible — drop now; an early EOF beats the
            // retirement-lifetime hold this replaced.
            return;
        };
        std::thread::spawn(move || {
            wait_process_exit(watch);
            crate::fd::close_handle(watch);
            drop(writer);
        });
    }

    fn apply_coproc_redirects(
        &mut self,
        cmd: &CommandNode,
        child: &mut Command,
        stdin_reader: std::io::PipeReader,
        stdout_writer: std::io::PipeWriter,
        stderr_forward: &Option<CoprocStderrForwardTarget>,
    ) -> Result<CoprocRedirectOutcome, ExecuteError> {
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
        // rubash#4170: the fd-0 disposition the walk reaches —
        // a redirect_in binding recorded here and applied to the
        // child at the Proceed end (or handed to the stillborn
        // child on a Declined return), with the precedence
        // body > redirect_in > coproc pipe reader.
        let mut stdin_override: Option<Stdio> = None;
        if body_stdin.is_none() {
            if let Some(redirect) = &cmd.redirect_in {
                if redirect.fd.unwrap_or(0) == 0 {
                    // rubash#4099: same gate as the ordered loop
                    // below — an erroring input word must not reach
                    // the silent expander (see there). rubash#4170:
                    // the pre-pass runs before any list entry can
                    // rebind fd 2, so the diagnostic goes to the
                    // process stderr (GNU's child inherits the
                    // shell's fd 2 here) and an open failure takes
                    // the same Declined disposition as the loop's.
                    if let Some((name, message, _)) = self.redirect_word_expansion_error(redirect) {
                        eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
                        return Ok(CoprocRedirectOutcome::Declined {
                            child_stdout: CoprocFdSpec::Pipe(stdout_writer).into_stdio(),
                            child_stdin: coproc_doomed_stdin(
                                body_stdin,
                                stdin_override,
                                stdin_reader,
                            ),
                            displaced_writer: None,
                        });
                    }
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        stdin_override = Some(Stdio::null());
                    } else if redirect_target_fd(&target).is_none() {
                        let file = match self.open_input_redirect(&target) {
                            Ok(file) => file,
                            Err(error) => {
                                eprintln!(
                                    "{}{}",
                                    self.diagnostic_prefix(),
                                    crate::posix_errors::message(&error)
                                );
                                return Ok(CoprocRedirectOutcome::Declined {
                                    child_stdout: CoprocFdSpec::Pipe(stdout_writer).into_stdio(),
                                    child_stdin: coproc_doomed_stdin(
                                        body_stdin,
                                        stdin_override,
                                        stdin_reader,
                                    ),
                                    displaced_writer: None,
                                });
                            }
                        };
                        stdin_override = Some(Stdio::from(file));
                    }
                }
            }
        }

        // rubash#1644/#1655 (transplanted from PR #1695 by
        // a-rookie-of-C-language): the coproc command's OUTPUT
        // redirects apply to the child in GNU's left-to-right order
        // — redir.c do_redirections runs in the forked child after
        // the coproc pipes are dup'd. The old code only handled the
        // single-field file redirects and dropped every dup (`>&2`
        // never reached the child: a simple command's output landed
        // in the coproc pipe instead of the shell's stderr, and a
        // named group's output sat in the pipe unread — lost
        // entirely). The ordered loop below replaces the field
        // blocks, which additionally misread `>file 2>&1` (the
        // `2>&1` arrived as a bogus Append field). Input kinds are
        // skipped here: stdin is handled above (redirect_in) and by
        // the stdin body.
        //
        // rubash#2200: numbered fds (>= 3) are bound during the
        // same walk into extra_bindings below. GNU's coproc child
        // applies do_redirections for ANY fd in the forked child
        // (execute_cmd.c:4244), so a construct-level `3>file` is
        // live in the body; rubash's spawn model has no fork, so
        // the bindings ride the __RUBASH_FD_* env channel to a
        // respawned child (init.rs restores them) and raw-fd pins
        // to a direct child — see the transport build after the
        // loop. The fd-1/2 spec paths are tracked alongside so a
        // dup FROM a spec can label its binding.
        //
        // rubash#1632/#4008 interplay: the fd-1 spec starts as the
        // coproc pipe writer; whenever a redirect replaces it,
        // replace_coproc_out_spec hands the writer back as the
        // displaced writer (held by the caller for the body's
        // redirect scope — GNU's save slot dies with the child, so
        // the caller drops it at apply-equivalent time or at the
        // child's exit) instead of dropping it here.
        let mut out_spec = CoprocFdSpec::Pipe(stdout_writer);
        let mut displaced_writer: Option<std::io::PipeWriter> = None;
        // The child's stderr target, for dup resolution only: the
        // caller has already set child.stderr (the direct endpoint
        // or the forwarder's pipe), so this spec mirrors where
        // stderr WOULD go; it is applied to the child only when an
        // err redirect touches fd 2.
        let mut err_spec = match stderr_forward {
            Some(CoprocStderrForwardTarget::Stdout) => {
                CoprocFdSpec::File(dup_coproc_process_fd(1)?)
            }
            Some(CoprocStderrForwardTarget::File(path)) => {
                // The forwarder itself append-opens this path per
                // chunk, so an append open here lands in the same
                // file with the same semantics.
                CoprocFdSpec::File(open_redirect_append(path)?)
            }
            None => self.coproc_stderr_direct_spec()?,
        };
        let mut err_touched = false;
        // rubash#2200: session paths behind the fd-1/2 File specs
        // (labels for dup bindings), and the numbered-fd bindings.
        let mut out_path: Option<PathBuf> = None;
        let mut err_path: Option<PathBuf> = match &stderr_forward {
            Some(CoprocStderrForwardTarget::File(path)) => Some(path.clone()),
            _ => None,
        };
        let mut extra_bindings: std::collections::BTreeMap<u32, CoprocFdBinding> =
            std::collections::BTreeMap::new();
        for redirect in &cmd.redirects {
            // rubash#4099: a redirect word carrying a parameter-
            // expansion error (nounset hit, `${x:?}`, bad
            // substitution) is adjudicated before the silent
            // expander can manufacture a bogus target from it (the
            // `:?` message text opened as a file, a nounset word's
            // empty expansion opened as the session directory).
            // GNU applies the coproc child's redirections in the
            // forked child (execute_cmd.c:2555 execute_in_subshell),
            // where the error is reported and only the child dies;
            // this parent walk's counterpart is the Declined
            // disposition the ambiguous-redirect arms below already
            // use: report the diagnostic once — through the fd-2
            // binding in force at this point in the list, as GNU's
            // child reports through its bound fd 2 (rubash#4170) —
            // and let the caller register a stillborn coproc.
            if let Some((name, message, _)) = self.redirect_word_expansion_error(redirect) {
                write_coproc_redirect_diagnostic(
                    &err_spec,
                    &format!("{}{}: {}\n", self.diagnostic_prefix(), name, message),
                );
                return Ok(CoprocRedirectOutcome::Declined {
                    child_stdout: out_spec.into_stdio(),
                    child_stdin: coproc_doomed_stdin(body_stdin, stdin_override, stdin_reader),
                    displaced_writer,
                });
            }
            let target = self.expand_redirect_target(redirect);
            match redirect.kind {
                crate::parser::RedirectKind::Output
                | crate::parser::RedirectKind::ClobberOutput => {
                    let Some(file) = coproc_open_or_report(
                        self.create_redirect_output_for(Some(redirect), &target, redirect.clobber),
                        &err_spec,
                        &self.diagnostic_prefix(),
                    ) else {
                        return Ok(CoprocRedirectOutcome::Declined {
                            child_stdout: out_spec.into_stdio(),
                            child_stdin: coproc_doomed_stdin(
                                body_stdin,
                                stdin_override,
                                stdin_reader,
                            ),
                            displaced_writer,
                        });
                    };
                    let fd = redirect.fd.unwrap_or(1);
                    match fd {
                        1 => {
                            replace_coproc_out_spec(
                                &mut out_spec,
                                CoprocFdSpec::File(file),
                                &mut displaced_writer,
                            );
                            out_path = Some(self.session_path(shell_path_to_windows(
                                &target,
                                &self.shell_state.env_vars,
                            )));
                        }
                        2 => {
                            err_spec = CoprocFdSpec::File(file);
                            err_touched = true;
                            err_path = Some(self.session_path(shell_path_to_windows(
                                &target,
                                &self.shell_state.env_vars,
                            )));
                        }
                        _ => {
                            // rubash#2200: the file was already
                            // created by the open above (as before);
                            // now the binding is kept for the child
                            // instead of being dropped.
                            extra_bindings.insert(
                                fd,
                                CoprocFdBinding::File {
                                    file: Rc::new(FileFd {
                                        handle: crate::fd::into_handle(file),
                                        closed: std::cell::Cell::new(false),
                                        adopted: false,
                                        path: self.session_path(shell_path_to_windows(
                                            &target,
                                            &self.shell_state.env_vars,
                                        )),
                                    }),
                                    read: false,
                                    write: true,
                                },
                            );
                        }
                    }
                }
                crate::parser::RedirectKind::Append => {
                    let session_target =
                        self.session_path(self.redirect_target_filesystem_path(&target));
                    let Some(file) = coproc_open_or_report(
                        open_redirect_append(&session_target),
                        &err_spec,
                        &self.diagnostic_prefix(),
                    ) else {
                        return Ok(CoprocRedirectOutcome::Declined {
                            child_stdout: out_spec.into_stdio(),
                            child_stdin: coproc_doomed_stdin(
                                body_stdin,
                                stdin_override,
                                stdin_reader,
                            ),
                            displaced_writer,
                        });
                    };
                    let fd = redirect.fd.unwrap_or(1);
                    match fd {
                        1 => {
                            replace_coproc_out_spec(
                                &mut out_spec,
                                CoprocFdSpec::File(file),
                                &mut displaced_writer,
                            );
                            out_path = Some(session_target);
                        }
                        2 => {
                            err_spec = CoprocFdSpec::File(file);
                            err_touched = true;
                            err_path = Some(session_target);
                        }
                        _ => {
                            extra_bindings.insert(
                                fd,
                                CoprocFdBinding::File {
                                    file: Rc::new(FileFd {
                                        handle: crate::fd::into_handle(file),
                                        closed: std::cell::Cell::new(false),
                                        adopted: false,
                                        path: session_target,
                                    }),
                                    read: false,
                                    write: true,
                                },
                            );
                        }
                    }
                }
                crate::parser::RedirectKind::CombinedOutput => {
                    let Some(file) = coproc_open_or_report(
                        self.create_redirect_output_for(Some(redirect), &target, redirect.clobber),
                        &err_spec,
                        &self.diagnostic_prefix(),
                    ) else {
                        return Ok(CoprocRedirectOutcome::Declined {
                            child_stdout: out_spec.into_stdio(),
                            child_stdin: coproc_doomed_stdin(
                                body_stdin,
                                stdin_override,
                                stdin_reader,
                            ),
                            displaced_writer,
                        });
                    };
                    let shared = file.try_clone()?;
                    replace_coproc_out_spec(
                        &mut out_spec,
                        CoprocFdSpec::File(file),
                        &mut displaced_writer,
                    );
                    err_spec = CoprocFdSpec::File(shared);
                    err_touched = true;
                    let session_target =
                        self.session_path(self.redirect_target_filesystem_path(&target));
                    out_path = Some(session_target.clone());
                    err_path = Some(session_target);
                }
                crate::parser::RedirectKind::CombinedAppend => {
                    let session_target =
                        self.session_path(self.redirect_target_filesystem_path(&target));
                    let Some(file) = coproc_open_or_report(
                        open_redirect_append(&session_target),
                        &err_spec,
                        &self.diagnostic_prefix(),
                    ) else {
                        return Ok(CoprocRedirectOutcome::Declined {
                            child_stdout: out_spec.into_stdio(),
                            child_stdin: coproc_doomed_stdin(
                                body_stdin,
                                stdin_override,
                                stdin_reader,
                            ),
                            displaced_writer,
                        });
                    };
                    let shared = file.try_clone()?;
                    replace_coproc_out_spec(
                        &mut out_spec,
                        CoprocFdSpec::File(file),
                        &mut displaced_writer,
                    );
                    err_spec = CoprocFdSpec::File(shared);
                    err_touched = true;
                    out_path = Some(session_target.clone());
                    err_path = Some(session_target);
                }
                crate::parser::RedirectKind::CloseOutput => {
                    let fd = redirect.fd.unwrap_or(1);
                    match fd {
                        1 => {
                            replace_coproc_out_spec(
                                &mut out_spec,
                                CoprocFdSpec::Null,
                                &mut displaced_writer,
                            );
                            out_path = None;
                        }
                        2 => {
                            err_spec = CoprocFdSpec::Null;
                            err_touched = true;
                            err_path = None;
                        }
                        _ => {
                            extra_bindings.insert(fd, CoprocFdBinding::Closed);
                        }
                    }
                }
                crate::parser::RedirectKind::DuplicateOutput => {
                    let fd = redirect.fd.unwrap_or(1);
                    if fd >= 3 {
                        // rubash#2200: a numbered target binds from
                        // the walk state (bindings, specs, fd
                        // table) instead of being dropped. A
                        // non-numeric, non-close target is an
                        // ambiguous redirect in GNU (the `>&word`
                        // shared-file form exists only for fd 1,
                        // redir.c r_err_and_out) — the same
                        // diagnostic and Declined disposition as
                        // the fd-2 word arm below.
                        if !is_closed_redirect_target(&target)
                            && redirect_target_fd_and_move(&target).is_none()
                        {
                            let raw_word = redirect
                                .target_metadata
                                .raw
                                .strip_prefix('&')
                                .unwrap_or(&redirect.target_metadata.raw);
                            write_coproc_redirect_diagnostic(
                                &err_spec,
                                &format!(
                                    "{}{}: ambiguous redirect\n",
                                    self.diagnostic_prefix(),
                                    raw_word
                                ),
                            );
                            return Ok(CoprocRedirectOutcome::Declined {
                                child_stdout: out_spec.into_stdio(),
                                child_stdin: coproc_doomed_stdin(
                                    body_stdin,
                                    stdin_override,
                                    stdin_reader,
                                ),
                                displaced_writer,
                            });
                        }
                        let outcome = coproc_numbered_dup_binding(
                            &self.fd_table,
                            &mut extra_bindings,
                            &out_spec,
                            &err_spec,
                            &out_path,
                            &err_path,
                            fd,
                            &target,
                            true,
                        );
                        if let CoprocDupBind::Bound(Some(source)) = outcome {
                            // Move form with an fd-1/2 source: the
                            // helper bound the target; the source
                            // spec still closes here.
                            if source == 1 {
                                replace_coproc_out_spec(
                                    &mut out_spec,
                                    CoprocFdSpec::Null,
                                    &mut displaced_writer,
                                );
                                out_path = None;
                            } else {
                                err_spec = CoprocFdSpec::Null;
                                err_touched = true;
                                err_path = None;
                            }
                        }
                        continue;
                    }
                    if fd != 1 && fd != 2 {
                        continue;
                    }
                    if is_closed_redirect_target(&target) {
                        match fd {
                            1 => {
                                replace_coproc_out_spec(
                                    &mut out_spec,
                                    CoprocFdSpec::Null,
                                    &mut displaced_writer,
                                );
                                out_path = None;
                            }
                            _ => {
                                err_spec = CoprocFdSpec::Null;
                                err_touched = true;
                                err_path = None;
                            }
                        }
                        continue;
                    }
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        // A dup of an fd onto itself is a no-op in
                        // GNU, and the move form's self-close is
                        // excepted (rubash#1236): leave both specs
                        // untouched.
                        if source_fd == fd {
                            continue;
                        }
                        // rubash#2200: a numbered SOURCE resolves
                        // through this walk's bindings and the
                        // parent fd table (coproc_source_spec);
                        // before, it hit the catch-all and the dup
                        // was dropped.
                        let (spec, source_path) = match source_fd {
                            1 => (out_spec.dup()?, out_path.clone()),
                            2 => (err_spec.dup()?, err_path.clone()),
                            _ => {
                                match coproc_source_spec(&self.fd_table, &extra_bindings, source_fd)
                                {
                                    Some(pair) => pair,
                                    None => continue,
                                }
                            }
                        };
                        match fd {
                            1 => {
                                replace_coproc_out_spec(&mut out_spec, spec, &mut displaced_writer);
                                out_path = source_path;
                            }
                            _ => {
                                err_spec = spec;
                                err_touched = true;
                                err_path = source_path;
                            }
                        }
                        // GNU `>&M-` moves: the source closes in
                        // the child after the dup — except a
                        // self-move, handled above (rubash#1236).
                        if move_source {
                            match source_fd {
                                1 => {
                                    replace_coproc_out_spec(
                                        &mut out_spec,
                                        CoprocFdSpec::Null,
                                        &mut displaced_writer,
                                    );
                                    out_path = None;
                                }
                                2 => {
                                    err_spec = CoprocFdSpec::Null;
                                    err_touched = true;
                                    err_path = None;
                                }
                                _ => {
                                    if extra_bindings.contains_key(&source_fd) {
                                        extra_bindings.insert(source_fd, CoprocFdBinding::Closed);
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    // Non-numeric word target.
                    if fd == 1 {
                        // GNU redir.c:832-838 r_err_and_out:
                        // `>&word` / `1>&word` opens ONE file
                        // for both fds (shared description).
                        // The stored target still carries the
                        // operator's `&` (like the compound
                        // arms); strip it for the open.
                        let path = target.strip_prefix('&').unwrap_or(&target);
                        let Some(file) = coproc_open_or_report(
                            self.create_redirect_output_for(Some(redirect), path, redirect.clobber),
                            &err_spec,
                            &self.diagnostic_prefix(),
                        ) else {
                            return Ok(CoprocRedirectOutcome::Declined {
                                child_stdout: out_spec.into_stdio(),
                                child_stdin: coproc_doomed_stdin(
                                    body_stdin,
                                    stdin_override,
                                    stdin_reader,
                                ),
                                displaced_writer,
                            });
                        };
                        let shared = file.try_clone()?;
                        replace_coproc_out_spec(
                            &mut out_spec,
                            CoprocFdSpec::File(file),
                            &mut displaced_writer,
                        );
                        err_spec = CoprocFdSpec::File(shared);
                        err_touched = true;
                        let session_target = self
                            .session_path(shell_path_to_windows(path, &self.shell_state.env_vars));
                        out_path = Some(session_target.clone());
                        err_path = Some(session_target);
                    } else {
                        // `2>&word` is an ambiguous redirect in
                        // GNU (and in rubash's plain commands):
                        // report it once and decline the spawn
                        // (Declined); an Err here would make the
                        // IoError reporter print the message a
                        // second time.
                        let raw_word = redirect
                            .target_metadata
                            .raw
                            .strip_prefix('&')
                            .unwrap_or(&redirect.target_metadata.raw);
                        write_coproc_redirect_diagnostic(
                            &err_spec,
                            &format!(
                                "{}{}: ambiguous redirect\n",
                                self.diagnostic_prefix(),
                                raw_word
                            ),
                        );
                        return Ok(CoprocRedirectOutcome::Declined {
                            child_stdout: out_spec.into_stdio(),
                            child_stdin: coproc_doomed_stdin(
                                body_stdin,
                                stdin_override,
                                stdin_reader,
                            ),
                            displaced_writer,
                        });
                    }
                }
                crate::parser::RedirectKind::DuplicateInput => {
                    let fd = redirect.fd.unwrap_or(0);
                    if fd >= 3 {
                        // rubash#2200: a dup INTO a numbered fd,
                        // resolved like the output dup's numbered
                        // target (coproc_numbered_dup_binding). A
                        // non-numeric, non-close target is an
                        // ambiguous redirect in GNU, as on the
                        // output side.
                        if !is_closed_redirect_target(&target)
                            && redirect_target_fd_and_move(&target).is_none()
                        {
                            let raw_word = redirect
                                .target_metadata
                                .raw
                                .strip_prefix('&')
                                .unwrap_or(&redirect.target_metadata.raw);
                            write_coproc_redirect_diagnostic(
                                &err_spec,
                                &format!(
                                    "{}{}: ambiguous redirect\n",
                                    self.diagnostic_prefix(),
                                    raw_word
                                ),
                            );
                            return Ok(CoprocRedirectOutcome::Declined {
                                child_stdout: out_spec.into_stdio(),
                                child_stdin: coproc_doomed_stdin(
                                    body_stdin,
                                    stdin_override,
                                    stdin_reader,
                                ),
                                displaced_writer,
                            });
                        }
                        let outcome = coproc_numbered_dup_binding(
                            &self.fd_table,
                            &mut extra_bindings,
                            &out_spec,
                            &err_spec,
                            &out_path,
                            &err_path,
                            fd,
                            &target,
                            false,
                        );
                        if let CoprocDupBind::Bound(Some(source)) = outcome {
                            // Move form with an fd-1/2 source: the
                            // helper bound the target; the source
                            // spec still closes here.
                            if source == 1 {
                                replace_coproc_out_spec(
                                    &mut out_spec,
                                    CoprocFdSpec::Null,
                                    &mut displaced_writer,
                                );
                                out_path = None;
                            } else {
                                err_spec = CoprocFdSpec::Null;
                                err_touched = true;
                                err_path = None;
                            }
                        }
                    }
                    // Fds 0-2 keep the pre-#2200 behavior: stdin
                    // dups belong to the redirect_in pre-pass, and
                    // list-level input dups on 1/2 were never
                    // applied here.
                }
                crate::parser::RedirectKind::Input => {
                    let fd = redirect.fd.unwrap_or(0);
                    if fd >= 3 {
                        // rubash#2200: a numbered input redirect
                        // binds in the child; the file opens here,
                        // as the output arms' files do.
                        if is_closed_redirect_target(&target) {
                            extra_bindings.insert(fd, CoprocFdBinding::Closed);
                        } else if redirect_target_fd(&target).is_none() {
                            // /dev/fd-style alias targets keep the
                            // alias machinery (the fd-0 pre-pass
                            // makes the same distinction); plain
                            // paths open now.
                            let Some(file) = coproc_open_or_report(
                                self.open_input_redirect(&target),
                                &err_spec,
                                &self.diagnostic_prefix(),
                            ) else {
                                return Ok(CoprocRedirectOutcome::Declined {
                                    child_stdout: out_spec.into_stdio(),
                                    child_stdin: coproc_doomed_stdin(
                                        body_stdin,
                                        stdin_override,
                                        stdin_reader,
                                    ),
                                    displaced_writer,
                                });
                            };
                            extra_bindings.insert(
                                fd,
                                CoprocFdBinding::File {
                                    file: Rc::new(FileFd {
                                        handle: crate::fd::into_handle(file),
                                        closed: std::cell::Cell::new(false),
                                        adopted: false,
                                        path: self.session_path(shell_path_to_windows(
                                            &target,
                                            &self.shell_state.env_vars,
                                        )),
                                    }),
                                    read: true,
                                    write: false,
                                },
                            );
                        }
                    }
                }
                crate::parser::RedirectKind::ReadWrite => {
                    let fd = redirect.fd.unwrap_or(0);
                    if fd >= 3 {
                        // rubash#2200: a numbered read-write
                        // redirect binds both directions.
                        let session_target = self.session_path(shell_path_to_windows(
                            &target,
                            &self.shell_state.env_vars,
                        ));
                        let Some(file) = coproc_open_or_report(
                            std::fs::OpenOptions::new()
                                .read(true)
                                .write(true)
                                .open(&session_target),
                            &err_spec,
                            &self.diagnostic_prefix(),
                        ) else {
                            return Ok(CoprocRedirectOutcome::Declined {
                                child_stdout: out_spec.into_stdio(),
                                child_stdin: coproc_doomed_stdin(
                                    body_stdin,
                                    stdin_override,
                                    stdin_reader,
                                ),
                                displaced_writer,
                            });
                        };
                        extra_bindings.insert(
                            fd,
                            CoprocFdBinding::File {
                                file: Rc::new(FileFd {
                                    handle: crate::fd::into_handle(file),
                                    closed: std::cell::Cell::new(false),
                                    adopted: false,
                                    path: session_target,
                                }),
                                read: true,
                                write: true,
                            },
                        );
                    }
                }
                crate::parser::RedirectKind::CloseInput => {
                    let fd = redirect.fd.unwrap_or(0);
                    if fd >= 3 {
                        extra_bindings.insert(fd, CoprocFdBinding::Closed);
                    }
                }
                _ => {}
            }
        }
        // rubash#2200: materialize the numbered-fd bindings. Each
        // File binding is duplicated to a fresh inheritable
        // handle; the binding crosses as __RUBASH_FD_* env pairs
        // (a respawned child's init restores them into its fd
        // table) and, for a direct-route child on unix, as a
        // (dup, fd) pin pair the caller registers in pre_exec.
        // Closed bindings need no transport: the child inherits
        // nothing at >= 3 except through this channel. Dups are
        // created only here, after the walk — the Declined and
        // Err exits above precede this point, so no dup can leak
        // on a refused spawn.
        let mut fd_dups: Vec<crate::fd::HANDLE> = Vec::new();
        let mut fd_env: Vec<(String, String)> = Vec::new();
        let mut fd_pins: Vec<(crate::fd::HANDLE, u32)> = Vec::new();
        for (fd, binding) in &extra_bindings {
            let CoprocFdBinding::File { file, read, write } = binding else {
                continue;
            };
            let Ok(dup) = crate::fd::duplicate_handle_inheritable(file.handle) else {
                continue;
            };
            if *read {
                fd_env.push((format!("__RUBASH_FD_HANDLE_{fd}"), format!("{dup:#x}")));
                fd_env.push((
                    format!("__RUBASH_FD_PATH_{fd}"),
                    file.path.to_string_lossy().into_owned(),
                ));
            }
            if *write {
                fd_env.push((format!("__RUBASH_FD_WHANDLE_{fd}"), format!("{dup:#x}")));
                fd_env.push((
                    format!("__RUBASH_FD_WPATH_{fd}"),
                    file.path.to_string_lossy().into_owned(),
                ));
            }
            fd_pins.push((dup, *fd));
            fd_dups.push(dup);
        }
        child.stdout(out_spec.into_stdio());
        if err_touched {
            child.stderr(err_spec.into_stdio());
        }

        // The fd-0 disposition tracked above: a memory body
        // outranks a redirect_in binding, which outranks the
        // coproc pipe reader handed over by the caller (the
        // parent's write end stays registered as ${COPROC[1]},
        // useless but present when a body took fd 0, like GNU's
        // rebound pipe end).
        if let Some(stdio) = body_stdin.or(stdin_override) {
            child.stdin(stdio);
        } else {
            child.stdin(stdin_reader);
        }

        Ok(CoprocRedirectOutcome::Proceed {
            displaced_writer,
            fd_dups,
            fd_env,
            fd_pins,
        })
    }

    /// rubash#1644: the fd-2 spec a coproc dup (`>&2`) must copy
    /// when the child's stderr was handed over directly at spawn
    /// (rubash#1882 CoprocStderrPlan::Direct): an owned dup of the
    /// same endpoint coproc_stderr_plan resolved from fd 2.
    fn coproc_stderr_direct_spec(&mut self) -> Result<CoprocFdSpec, ExecuteError> {
        if self.fd_table.is_closed(2) {
            // The plan hands the child /dev/null; the spec is Null.
            return Ok(CoprocFdSpec::Null);
        }
        Ok(match self.fd_table.write_endpoint(2) {
            None | Some(FdWriteEndpoint::Stderr) => CoprocFdSpec::File(dup_coproc_process_fd(2)?),
            Some(FdWriteEndpoint::Stdout) => CoprocFdSpec::File(dup_coproc_process_fd(1)?),
            Some(FdWriteEndpoint::File(file_fd)) => CoprocFdSpec::File(crate::fd::handle_to_file(
                crate::fd::duplicate_handle_inheritable(file_fd.handle)?,
            )),
            Some(FdWriteEndpoint::ProcessSubstitution { path, .. }) => {
                // The plan always forwards this endpoint, so the
                // direct seed never serves it in practice; mirror
                // the forwarder's append-open if it ever did.
                CoprocFdSpec::File(open_redirect_append(&path)?)
            }
            Some(FdWriteEndpoint::CoprocStdin { fd, .. }) => CoprocFdSpec::File(
                crate::fd::handle_to_file(crate::fd::duplicate_handle_inheritable(fd.handle)?),
            ),
            // rubash#1898: mirrors the plan's /dev/full arm.
            Some(FdWriteEndpoint::FullDevice) => CoprocFdSpec::Null,
            // rubash#2042: mirrors the plan's synthetic-/proc arm —
            // the child's stderr writes absorb (Null), exactly like
            // the /dev/full binding's ENOSPC ceiling.
            Some(FdWriteEndpoint::ProcContentWrite) => CoprocFdSpec::Null,
            // rubash#2385: mirrors the plan's channel arm.
            Some(FdWriteEndpoint::PipeChannel(_)) => CoprocFdSpec::Null,
        })
    }
}
