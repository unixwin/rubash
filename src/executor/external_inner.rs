use super::*;

/// rubash#2272: the command's own pre_exec pin list. On Unix the
/// pins carry raw fds; on other platforms the list stays empty (the
/// env channel alone delivers there) but the fd_var delivery arms
/// still record into it uniformly.
#[cfg(unix)]
type FdPinVec = Vec<(std::os::unix::io::RawFd, u32)>;
#[cfg(not(unix))]
type FdPinVec = Vec<(i32, u32)>;

/// rubash#2278: the binding a numbered dup entry delivered to its
/// slot, when that binding is not a FileFd the walk's walk_opened
/// map can hold — recorded so a LATER dup entry in the same list
/// sourcing that slot resolves to the same binding (GNU's single
/// ordered walk, redir.c:237 do_redirections, where every dup2
/// reads the descriptor as bound at its own position).
#[derive(Clone, Copy)]
enum NumberedDupBinding {
    /// The child's own wired stdio slot carries the binding: the
    /// dup's source slot is not rebound by any later entry of the
    /// list, so the wired slot the spawn arranged IS the binding
    /// the dup saw at its position (the #2277 step-5 shape).
    ChildSlot(u32),
    /// A parent-side handle captured at the dup's position: a
    /// later entry rebinds the source slot, so the at-position
    /// binding is the captured one even though the child's wired
    /// slot will hold the later binding. On the simple-command
    /// face the handle duplicates the shell's ambient stdio
    /// descriptor; on the pipeline stage face it can instead be
    /// the stage's stdin channel carrier (see StageStdioContext).
    Handle(crate::fd::HANDLE),
}

/// rubash#2293: the pipeline stage face's stdio context for the
/// shared fd delivery. GNU binds every pipeline element's fds
/// 0/1/2 to the inter-element pipes BEFORE the element's own
/// do_redirections walk runs (execute_cmd.c:2620
/// execute_pipeline), so a numbered dup of a stdio slot copies
/// the descriptor as bound at the dup's position — for a stage
/// that is the stage channel (the payload feeding fd 0, the
/// capture pipes on fds 1/2), never the shell's ambient
/// descriptor and never the binding a later list entry installs.
/// The stage spawner knows those channels; the simple-command
/// face passes None (its child's wired stdio is exactly what the
/// existing dup arms model).
pub(in crate::executor) struct StageStdioContext {
    /// Raw handle of the carrier read end holding the stage's
    /// stdin payload, when the spawner materialized one: fd 0's
    /// at-position binding for a dup that precedes the entry
    /// rebinding fd 0 (the stdin face wires the WINNER into the
    /// child's fd 0, so the payload channel exists only as this
    /// carrier). None when the stage's fd 0 is the live inherited
    /// descriptor — the ambient capture already names it.
    pub stdin_channel: Option<crate::fd::HANDLE>,
    /// Carrier write ends for numbered OUTPUT dups of fds 1/2 whose
    /// source slot a later list entry rebinds, as (dup target fd,
    /// source fd, write-end handle). The child shares ONE capture
    /// pipe per stdio slot, and the routing walk places that capture
    /// whole at the source slot's FINAL target — a dup pinned onto
    /// the wired slot would land its bytes there too, not on the
    /// stage channel the dup copied at its position. The spawner
    /// therefore wires such a dup onto its own carrier pipe; the
    /// drained payload is placed at the dup's own resolved target
    /// by route_pipeline_stage_streams.
    pub dup_channels: Vec<(u32, u32, crate::fd::HANDLE)>,
}

/// rubash#2281: apply one recorded {var} pending close during the
/// delivery walk (see apply_external_fd_delivery): perform the close
/// in the table (Executor::apply_fd_var_pending_close) and revoke
/// anything already delivered for that slot - the ambient env
/// stamps, a command pin, and the walk's own binding records - so
/// the child does not inherit the slot and a later entry can no
/// longer source it. The parent-side duplicate handles stay in the
/// caller's list and are closed after the spawn, as before: with
/// the env keys gone the child never learns them.
fn apply_pending_close_at(
    executor: &mut Executor,
    process: &mut Command,
    command_fd_pins: &mut FdPinVec,
    walk_opened: &mut std::collections::HashMap<
        u32,
        std::rc::Rc<crate::executor::fd_table::FileFd>,
    >,
    numbered_dup_bindings: &mut std::collections::HashMap<u32, NumberedDupBinding>,
    fd: u32,
) {
    executor.apply_fd_var_pending_close(fd);
    for key in [
        format!("__RUBASH_FD_HANDLE_{fd}"),
        format!("__RUBASH_FD_PATH_{fd}"),
        format!("__RUBASH_FD_WHANDLE_{fd}"),
        format!("__RUBASH_FD_WPATH_{fd}"),
    ] {
        process.env_remove(key);
    }
    command_fd_pins.retain(|&(_, target)| target != fd);
    walk_opened.remove(&fd);
    numbered_dup_bindings.remove(&fd);
}

/// rubash#1626: pin the shell's ambient fd-table descriptors at their
/// slot numbers in a forked child, between fork and exec — GNU's child
/// inherits every descriptor `exec N>file` bound, at the same number.
/// Two-phase so a pin never clobbers a not-yet-moved source: first move
/// every source above every target with F_DUPFD_CLOEXEC, then dup2 the
/// staged copies onto their slots (dup2 clears CLOEXEC on the target,
/// so the descriptor survives exec). A source already sitting at its
/// slot only needs its CLOEXEC flag cleared. Failures are ignored per
/// pin: the child then simply lacks that descriptor — the pre-fix
/// behavior — instead of failing the whole spawn. One target is never
/// clobbered: an anonymous pipe the spawn machinery itself parked at
/// that number in the child (its exec-error channel). It is
/// recognizable as an open FIFO at the target that is not one of the
/// pin sources; overwriting it would corrupt spawn error reporting.
/// Two FIFO shapes are NOT the error channel: a stdio pipe the spawn
/// machinery dup2'd onto its slot (0/1/2) and left open at its
/// original number until exec, and a carrier pipe one of the pins
/// itself draws from — the sibling end of a pin source, parked at
/// the target by the parent's fd allocation (a stage dup carrier's
/// read end can sit exactly on the dup's slot while its write end
/// is the pin source). A pipeline stage's own stdout pipe parks
/// like the first shape, and when its number collides with a pin's
/// target the pin is GNU's redirection for the element
/// (execute_cmd.c:2620 execute_pipeline runs redir.c:237
/// do_redirections in the forked child) and must win the slot
/// (rubash#2255). The shapes are told apart by identity: both ends
/// of one pipe share an inode, so a FIFO whose (dev, inode)
/// matches a stdio descriptor or a pin source is part of the
/// spawn's own fd arrangement, while the error channel is a pipe
/// of its own and never matches either.
#[cfg(unix)]
pub(in crate::executor) fn pin_ambient_fds_in_child(
    pins: &[(std::os::unix::io::RawFd, u32)],
) -> std::io::Result<()> {
    use std::os::unix::io::RawFd;
    let sources: Vec<RawFd> = pins.iter().map(|&(source, _)| source).collect();
    let ceiling = pins
        .iter()
        .map(|&(source, slot)| source.max(slot as RawFd))
        .max()
        .unwrap_or(2);
    let mut staged: Vec<(RawFd, RawFd)> = Vec::with_capacity(pins.len());
    for &(source, slot) in pins {
        let target = slot as RawFd;
        if source == target {
            unsafe { libc::fcntl(source, libc::F_SETFD, 0) };
            continue;
        }
        let temp = unsafe { libc::fcntl(source, libc::F_DUPFD_CLOEXEC, ceiling + 1) };
        if temp >= 0 {
            staged.push((temp, target));
        }
    }
    for (temp, target) in staged {
        if !sources.contains(&target)
            && fd_is_anonymous_pipe(target)
            && !fd_is_stdio_pipe_alias(target)
            && !fd_is_pin_source_pipe_alias(target, &sources)
        {
            unsafe { libc::close(temp) };
            continue;
        }
        if unsafe { libc::dup2(temp, target) } >= 0 {
            unsafe { libc::close(temp) };
        }
    }
    Ok(())
}

/// Whether `fd` is currently open in this process as an anonymous pipe.
#[cfg(unix)]
fn fd_is_anonymous_pipe(fd: std::os::unix::io::RawFd) -> bool {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return false;
    }
    // Widened through u32 like the sibling checks in src/fd/unix.rs:
    // bionic (Android) types the S_IF* constants as u16 while st_mode
    // is u32, so the raw comparison does not compile there.
    (u32::from(stat.st_mode) & u32::from(libc::S_IFMT)) == u32::from(libc::S_IFIFO)
}

/// The (dev, inode) identity of the anonymous pipe `fd` names, if
/// it names one. Both ends of one pipe share the identity, which
/// is what lets the pin guard tell the spawn's own fd arrangement
/// apart from std's exec-error channel (a pipe of its own).
#[cfg(unix)]
fn pipe_identity(fd: std::os::unix::io::RawFd) -> Option<(u64, u64)> {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return None;
    }
    // Widened through u32 like fd_is_anonymous_pipe above (bionic).
    if (u32::from(stat.st_mode) & u32::from(libc::S_IFMT)) != u32::from(libc::S_IFIFO) {
        return None;
    }
    // Identity fields are widened to u64: the raw stat field types
    // and the dev_t/ino_t aliases disagree on 32-bit Android (u64
    // fields vs u32 aliases), and only equality is ever tested.
    Some((stat.st_dev as u64, stat.st_ino as u64))
}

/// Whether `fd` is the same anonymous pipe as one of the child's
/// arranged stdio descriptors (0/1/2) — the spawn machinery's stdio
/// leftover at its pre-dup2 number (see pin_ambient_fds_in_child).
/// std's exec-error channel is a separate pipe and never matches
/// a stdio descriptor, so it keeps the pin guard's protection.
#[cfg(unix)]
fn fd_is_stdio_pipe_alias(fd: std::os::unix::io::RawFd) -> bool {
    let Some(identity) = pipe_identity(fd) else {
        return false;
    };
    [0, 1, 2]
        .iter()
        .any(|&stdio_fd| pipe_identity(stdio_fd) == Some(identity))
}

/// Whether `fd` is the sibling end of a pipe one of the pin
/// sources names — the occupant is then part of the spawn's own
/// fd arrangement (a carrier the delivery draws from its other
/// end, parked on the target by the parent's fd allocation), not
/// std's exec-error channel, whose pipe matches no source.
#[cfg(unix)]
fn fd_is_pin_source_pipe_alias(
    fd: std::os::unix::io::RawFd,
    sources: &[std::os::unix::io::RawFd],
) -> bool {
    let Some(identity) = pipe_identity(fd) else {
        return false;
    };
    sources
        .iter()
        .any(|&source| pipe_identity(source) == Some(identity))
}

/// The result of one route-seam attempt (see
/// `Executor::try_external_route`): `Handled` — the call ran
/// in-process; `Declined` — the call falls through to the
/// prepared spawn, carrying the stdin text the realization
/// already pulled (when the stdin source was a text buffer) so
/// the spawn's feed uses it instead of pulling a second time.
enum RouteAttempt {
    Handled,
    Declined(Option<String>),
}

impl Executor {
    /// The simple-command execution lookup. Under a `command -p`
    /// re-dispatch (standard_path_scope set) the lookup walks the
    /// standard path — by substituting PATH in a throwaway copy of the
    /// env map, never in the shell's own variables (rubash#1047) —
    /// and skips the hash table entirely (rubash#1648; GNU
    /// findcmd.c:364: CMDSRCH_STDPATH never consults hashed_filenames).
    pub(in crate::executor) fn find_scoped_user_command(
        &self,
        name: &str,
    ) -> Option<std::path::PathBuf> {
        match &self.standard_path_scope {
            Some(standard) => {
                let mut env_vars = self.shell_state.env_vars.clone();
                env_vars.insert("PATH".to_string(), standard.clone());
                find_user_command_without_hash(name, &env_vars)
            }
            None => find_user_command(name, &self.shell_state.env_vars),
        }
    }

    /// rubash#2317: the recorded name for the same scoped lookup as
    /// find_scoped_user_command — the as-written PATH-element join
    /// (findcmd.c:623 via find_in_path_element's sh_makepath), under the
    /// identical standard-path scoping. Used for the reader-name seeding
    /// (BASH_SOURCE / $0 / diagnostics), never for the spawn itself.
    pub(in crate::executor) fn find_scoped_user_command_as_written(
        &self,
        name: &str,
    ) -> Option<String> {
        match &self.standard_path_scope {
            Some(standard) => {
                let mut env_vars = self.shell_state.env_vars.clone();
                env_vars.insert("PATH".to_string(), standard.clone());
                crate::executor::path::find_user_command_as_written(name, &env_vars)
            }
            None => crate::executor::path::find_user_command_as_written(
                name,
                &self.shell_state.env_vars,
            ),
        }
    }

    pub(in crate::executor) fn execute_env_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // cmd.words are already expanded by expand_command_words before
        // dispatch (execute_cmd.c do_word_expansion). Re-expanding here ran
        // literal `$(...)` text a second time: `env 'X=$(id >/dev/tty)'`
        // executed the substitution and reported the redirect failure on the
        // caller's stderr (exportfunc.tests line 68).
        let args = cmd.words[1..].to_vec();
        let Some(config) = self.parse_env_command_args(args)? else {
            return Ok(());
        };
        let Some(env_vars) = self.materialize_env_command_environment(cmd, &config)? else {
            return Ok(());
        };
        // Lookup view of the composed environment for the consumers
        // that take a name->value map (program resolution, argv0
        // detection, chdir translation); composition ORDER lives only
        // in env_vars itself.
        let env_map: HashMap<String, String> = env_vars.iter().cloned().collect();

        if config.command.is_empty() {
            let mut output = Vec::new();
            // Composition order is the output order (rubash#1806): GNU
            // env prints the environment array it was exec'd with,
            // which maybe_make_export_env built temporary-block first
            // and `_` last — no sorting anywhere in the chain.
            for (key, value) in &env_vars {
                output.extend_from_slice(key.as_bytes());
                output.push(b'=');
                // GNU coreutils env writes each exported value cell's raw
                // bytes (variables.c:4850 make_env_array_from_var_list
                // copies the cell verbatim via mk_env_string).
                // Stored values carry raw bytes as U+E000 marker pairs
                // (markers.rs Storage boundary); decode to the real bytes
                // at this output boundary so `x=$(printf '\377'); export
                // x; env` prints the byte, not the pair (matching the
                // WSL GNU baseline ff for `env | grep '^x='`).
                output.extend_from_slice(
                    &crate::executor::substitution_metadata::decode_raw_byte_markers(
                        value.as_bytes(),
                    ),
                );
                output.push(if config.null_terminated { b'\0' } else { b'\n' });
            }
            self.write_default_stdout(&output)?;
            self.exit_code = 0;
            return Ok(());
        }

        if config.null_terminated {
            self.write_default_stderr(b"env: cannot specify --null (-0) with command\n")?;
            self.exit_code = 125;
            return Ok(());
        }

        // rubash#2046 B2: GNU env hands the composed environment to
        // execvp, and execvp with PATH ABSENT from that environment
        // searches its default path (glibc confstr _CS_PATH) instead of
        // failing — `env -u PATH printenv PATH` runs /usr/bin/printenv
        // and reports its rc 1, not env's 127. Mirror that one shape:
        // resolve against the standard path (path::standard_path, the
        // same confstr port `command -p` uses) whenever the composed
        // environment carries no PATH at all. A set-but-empty PATH is
        // a different shape and keeps the plain lookup — the ruling's
        // scope is the unset form only.
        let mut stdpath_lookup;
        let resolution_env = if crate::executor::path::path_is_unset(&env_map) {
            stdpath_lookup = env_map.clone();
            stdpath_lookup.insert("PATH".to_string(), standard_path(&env_map));
            &stdpath_lookup
        } else {
            &env_map
        };
        let program = find_user_command(&config.command[0], resolution_env);
        let Some(program) = program else {
            let mut stderr = Vec::new();
            writeln!(
                &mut stderr,
                "env: failed to run command '{}': No such file or directory",
                config.command[0]
            )?;
            self.write_default_stderr(&stderr)?;
            self.exit_code = 127;
            return Ok(());
        };

        let (dev_args, dev_ops) = self.materialize_dev_fd_operands(
            &config.command[1..],
            crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
            crate::executor::dev_fd_operands::DevOperandStdout::FdTable,
            Some(program.as_path()),
        );
        // rubash#1905: synthetic /proc operands become spawn-time
        // snapshot files for a child without its own /proc layer; the
        // record lives until spawn_external_process has waited the
        // child out (function scope end).
        let (proc_args, _proc_ops) = crate::executor::proc_operands::materialize_proc_operands(
            &dev_args,
            Some(&config.command[0]),
            Some(program.as_path()),
            &env_map,
        );
        // rubash#2442: GNU env hands its command to execvp, whose
        // execve filename is the same search join this resolution
        // produced (the typed word itself for a slash name) — derive
        // it through the shared funnel. An `env -C DIR` child is
        // born in DIR rather than the session cwd this resolution
        // ran against, so a relative join would resolve against the
        // wrong directory there; the anchored program stands for
        // that shape.
        let exec_name = if config.chdir.is_none() {
            crate::executor::path::spawn_exec_name(
                &config.command[0],
                &program,
                resolution_env,
            )
        } else {
            None
        };
        let (mut process, used_shell) = external_command_for_named_program_with_exec_name(
            &program,
            Some(&config.command[0]),
            &proc_args,
            &env_map,
            env_map
                .get("BASH_ARGV0")
                .is_some_and(|value| !value.is_empty()),
            exec_name.as_deref(),
        );
        // GNU env (coreutils) runs the command through execvp under the
        // name as typed, so the child's argv[0] is the bare word, not
        // the PATH-resolved path Command::new carries (rubash#1276:
        // a GNU grandchild's diagnostics were prefixed with the full
        // resolved path when the parent shell was niu). Shell-wrapped
        // children pick their own argv[0] (the script name).
        #[cfg(unix)]
        if !used_shell {
            use std::os::unix::process::CommandExt;
            process.arg0(&config.command[0]);
        }
        // G-02: children are born in the session cwd; the `env -C`
        // override below replaces it for its own child.
        process.current_dir(&self.shell_state.cwd);
        apply_env_command_environment(&mut process, &env_vars, !config.ignore_environment);
        if let Some(chdir) = &config.chdir {
            let directory = shell_path_to_windows(chdir, &env_map);
            if !directory.is_dir() {
                self.write_default_stderr(b"env: cannot change directory\n")?;
                self.exit_code = 125;
                return Ok(());
            }
            process.current_dir(directory);
        }
        self.apply_external_redirects(cmd, &mut process)?;
        // rubash#1832: the environment was composed to finality by
        // the env emulation above — the seam must not stamp SHLVL
        // over it (env_composed_final).
        self.spawn_external_process(cmd, &program, process, used_shell, dev_ops, true, None)
    }

    fn parse_env_command_args(
        &mut self,
        args: Vec<String>,
    ) -> Result<Option<EnvCommandConfig>, ExecuteError> {
        let mut config = EnvCommandConfig::default();
        let mut args = args;
        let mut index = 0usize;
        let mut command_started = false;

        while index < args.len() {
            let arg = args[index].clone();
            index += 1;
            if command_started {
                config.command.push(arg);
                continue;
            }

            if arg == "--" {
                command_started = true;
                continue;
            }
            if arg == "-" || arg == "-i" || arg == "--ignore-environment" {
                config.ignore_environment = true;
                continue;
            }
            if arg == "-0" || arg == "--null" {
                config.null_terminated = true;
                continue;
            }
            if arg == "-v" || arg == "--debug" {
                config.debug = true;
                continue;
            }

            if let Some(parsed) =
                self.parse_env_short_option_cluster(&arg, &args, &mut index, &mut config)?
            {
                let Some(split) = parsed else {
                    return Ok(None);
                };
                args.splice(index..index, split);
                continue;
            }

            if let Some(value) = long_option_value(&arg, "--unset=") {
                config.unset_names.push(value);
                continue;
            }
            if arg == "-u" || arg == "--unset" {
                let Some(value) = args.get(index).cloned() else {
                    self.write_default_stderr(b"env: option --unset requires an argument\n")?;
                    self.exit_code = 125;
                    return Ok(None);
                };
                index += 1;
                config.unset_names.push(value);
                continue;
            }

            if let Some(value) = long_option_value(&arg, "--chdir=") {
                config.chdir = Some(value);
                continue;
            }
            if arg == "-C" || arg == "--chdir" {
                let Some(value) = args.get(index).cloned() else {
                    self.write_default_stderr(b"env: option --chdir requires an argument\n")?;
                    self.exit_code = 125;
                    return Ok(None);
                };
                index += 1;
                config.chdir = Some(value);
                continue;
            }

            if let Some(value) = long_option_value(&arg, "--file=") {
                config.file = Some(value);
                continue;
            }
            if arg == "-f" || arg == "--file" {
                let Some(value) = args.get(index).cloned() else {
                    self.write_default_stderr(b"env: option --file requires an argument\n")?;
                    self.exit_code = 125;
                    return Ok(None);
                };
                index += 1;
                config.file = Some(value);
                continue;
            }

            if let Some(value) = long_option_value(&arg, "--argv0=") {
                config.argv0 = Some(value);
                continue;
            }
            if arg == "-a" || arg == "--argv0" {
                let Some(value) = args.get(index).cloned() else {
                    self.write_default_stderr(b"env: option --argv0 requires an argument\n")?;
                    self.exit_code = 125;
                    return Ok(None);
                };
                index += 1;
                config.argv0 = Some(value);
                continue;
            }

            if let Some(value) = long_option_value(&arg, "--split-string=") {
                let split = crate::executor::alias_helpers::split_shell_words(&value);
                args.splice(index..index, split);
                continue;
            }
            if arg == "-S" || arg == "--split-string" {
                let Some(value) = args.get(index).cloned() else {
                    self.write_default_stderr(
                        b"env: option --split-string requires an argument\n",
                    )?;
                    self.exit_code = 125;
                    return Ok(None);
                };
                index += 1;
                let split = crate::executor::alias_helpers::split_shell_words(&value);
                args.splice(index..index, split);
                continue;
            }

            if matches!(
                arg.as_str(),
                "--default-signal"
                    | "--ignore-signal"
                    | "--block-signal"
                    | "--list-signal-handling"
            ) || arg.starts_with("--default-signal=")
                || arg.starts_with("--ignore-signal=")
                || arg.starts_with("--block-signal=")
            {
                continue;
            }

            if let Some((name, value)) = parse_env_assignment_arg(&arg) {
                config.assignments.push((name, value));
                continue;
            }

            command_started = true;
            config.command.push(arg);
        }

        if config.command.is_empty() && config.chdir.is_some() {
            self.write_default_stderr(b"env: must specify command with --chdir\n")?;
            self.exit_code = 125;
            return Ok(None);
        }

        Ok(Some(config))
    }

    fn materialize_env_command_environment(
        &mut self,
        cmd: &CommandNode,
        config: &EnvCommandConfig,
    ) -> Result<Option<Vec<(String, String)>>, ExecuteError> {
        // The composed environment is ORDERED (rubash#1806): GNU
        // variables.c maybe_make_export_env flattens the variable
        // contexts into the child env array in a fixed block order —
        // the temporary_env context (this command's prefix assignments)
        // FIRST, then the shell_variables contexts, then the exported
        // functions — with add_or_supercede semantics (a name already
        // present is replaced in place, never appended twice), so the
        // temporary binding shadows the global of the same name. Within
        // the global block GNU iterates hash chains, whose order is an
        // implementation artifact, not a contract; this composition
        // sorts that block by name so the emitted order is
        // deterministic. The bare-`env` print path emits this order
        // verbatim.
        let mut env_vars: Vec<(String, String)> = Vec::new();
        if !config.ignore_environment {
            // Block 1: this command's prefix assignments, in source
            // order, valued exactly as apply_external_environment
            // values them for the real spawn path (append assignments
            // read the value the dispatch seam already composed into
            // the table; plain assignments re-derive from the expanded
            // word — cmd.assignments are post-expansion text).
            let raws_aligned = cmd.assignment_raws.len() == cmd.assignments.len();
            for (index, (var_name, var_value)) in cmd.assignments.iter().enumerate() {
                let (base_name, append) = assignment_name_and_append(var_name);
                // GNU variables.c:3589 assign_in_env: an
                // ASSIGN_DISALLOWED (att_noassign) prefix target
                // returns 0 — the binding never enters the temporary
                // environment, so no env entry is composed for it
                // (rubash#2369).
                if self.assignment_target_is_noassign(base_name) {
                    continue;
                }
                let Some(env_name) = self.tempenv_export_name(base_name) else {
                    continue;
                };
                if append {
                    if let Some(value) = self.shell_state.env_vars.get(base_name) {
                        ordered_env_insert(&mut env_vars, env_name, value.clone());
                    }
                    continue;
                }
                let raw = raws_aligned
                    .then(|| cmd.assignment_raws.get(index))
                    .flatten()
                    .map(String::as_str);
                let expanded_value =
                    self.expand_assignment_value_with_raw(var_name, var_value, raw);
                let expanded_value = expanded_value
                    .strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
                    .unwrap_or(&expanded_value);
                ordered_env_insert(&mut env_vars, env_name, expanded_value.to_string());
            }
            // Block 2: exported shell variables. A name the prefix block
            // already emitted is shadowed by it (GNU flatten rule) and
            // must not be appended a second time. GNU iterates its hash
            // chains here (an artifact order); rubash composes this
            // block sorted by name so the print order is deterministic.
            let mut global_names = marked_env_names(&self.shell_state.env_vars, EXPORTED_VARS);
            global_names.sort();
            for name in global_names {
                if ordered_env_contains(&env_vars, &name) {
                    continue;
                }
                // GNU variables.c make_env_array_from_var_list: array and
                // associative variables never cross the child env boundary
                // (rubash#534) — `env`'s internal emulation mirrors the
                // real spawn path.
                if !is_exportable_env_name(&self.shell_state.env_vars, &name) {
                    continue;
                }
                if let Some(value) = self.shell_state.env_vars.get(&name) {
                    ordered_env_insert(&mut env_vars, name, value.clone());
                }
            }
            let mut stash_entries = local_export_env_values(&self.shell_state.env_vars);
            stash_entries.sort();
            for (name, value) in stash_entries {
                if ordered_env_contains(&env_vars, &name) {
                    continue;
                }
                // Same rule as apply_child_environment: the stash stands
                // in for the shadowed outer binding only while the local
                // is not itself a live export candidate (rubash#970).
                if !local_export_env_entry_applies(&self.shell_state.env_vars, &name) {
                    continue;
                }
                ordered_env_insert(&mut env_vars, name, value);
            }
            // GNU `env` is an external binary that inherits the shell's OS
            // environment, in which every exported function was
            // materialized as BASH_FUNC_<name>%% (variables.c:3989-4034
            // push_exported_function / make_env_array_from_var_list). The
            // internal emulation must materialize them the same way the
            // real spawn path does (apply_exported_functions_to_child),
            // or `env | grep BASH_FUNC` reports nothing while `printenv`
            // (a real external) sees the function (rubash#253).
            let mut function_names =
                marked_env_names(&self.shell_state.env_vars, EXPORTED_FUNCTIONS);
            function_names.sort();
            for name in function_names {
                if let Some(body) = self.shell_state.functions.get(&name) {
                    let def_redirects = self
                        .shell_state
                        .function_def_infos
                        .get(&name)
                        .map(|info| info.def_redirects.as_slice())
                        .unwrap_or(&[]);
                    ordered_env_insert(
                        &mut env_vars,
                        exported_function_env_name(&name),
                        exported_function_env_value(&body.commands, def_redirects),
                    );
                }
            }
            // Windows child processes need the session variables (SystemRoot
            // for crypto/socket setup, WINDIR/ComSpec for subprocess spawning)
            // even when the exporting shell state does not carry them. Unix
            // has no such hidden requirement — GNU env passes only what it is
            // given (builtins/../coreutils env semantics).
            #[cfg(windows)]
            for name in ["SystemRoot", "WINDIR", "ComSpec"] {
                if ordered_env_contains(&env_vars, name) {
                    continue;
                }
                if let Some(value) = self
                    .shell_state
                    .env_vars
                    .get(name)
                    .cloned()
                    .or_else(|| env::var(name).ok())
                {
                    ordered_env_insert(&mut env_vars, name.to_string(), value);
                }
            }
            // Final entry: `_`. GNU execute_cmd.c execute_disk_command
            // calls put_command_name_into_env (variables.c) with the
            // pathname search_for_command resolved for the command about
            // to run — for the `env` emulation that command is `env`
            // itself, so the composed environment ends with `_` naming
            // the resolved env program (coreutils env does not rewrite
            // `_` for its own child; it is inherited as-is). Resolution
            // runs against the environment composed so far, so a prefix
            // PATH assignment steers it exactly as the real spawn path
            // would steer the lookup of `env`. If `env` does not
            // resolve, no `_` is fabricated.
            let lookup_map: HashMap<String, String> = env_vars.iter().cloned().collect();
            if let Some(env_program) = find_user_command("env", &lookup_map) {
                ordered_env_insert(
                    &mut env_vars,
                    "_".to_string(),
                    env_program.to_string_lossy().into_owned(),
                );
            }
        }

        // rubash#1832: GNU's SHLVL adjustment is a mutation of the
        // shell variable BEFORE the export environment is built
        // (execute_cmd.c:5813-5817: on the nofork path
        // adjust_shell_level(-1) runs first, then
        // maybe_make_export_env), so when this env command is the
        // exec-replace tail, the environment env RECEIVES already
        // carries the adjusted level. Fold it into the composed base
        // here — after the base blocks above, before env's own
        // file/unset/assignment processing below, which must govern
        // the final value (a `-u SHLVL` removes the entry, an explicit
        // `SHLVL=` assignment supersedes it, a file entry wins over
        // the base). A prefix SHLVL word on the env command itself is
        // part of the base blocks and is superseded like on the
        // direct path (oracle: `SHLVL=99 env ...` prints the adjusted
        // level). Under `-i` there is no received environment to
        // adjust — the base blocks were skipped and nothing is
        // folded. The spawn seam must not stamp over this composition
        // afterwards (see spawn_external_process's env_composed_final).
        #[cfg(unix)]
        if !config.ignore_environment && self.exec_replace_tail_state_ready() {
            let adjusted = self.exec_replace_adjusted_shlvl();
            if ordered_env_contains(&env_vars, "SHLVL") {
                ordered_env_insert(&mut env_vars, "SHLVL".to_string(), adjusted);
            } else if let Some(pos) = env_vars.iter().position(|(name, _)| name == "_") {
                // Keep `_` the final entry (the composition invariant
                // above); the adjusted level joins the globals block.
                env_vars.insert(pos, ("SHLVL".to_string(), adjusted));
            } else {
                env_vars.push(("SHLVL".to_string(), adjusted));
            }
        }

        if let Some(file) = &config.file {
            match fs::read_to_string(shell_path_to_windows(file, &self.shell_state.env_vars)) {
                Ok(text) => {
                    for line in text.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        if let Some((name, value)) = parse_env_assignment_arg(line) {
                            ordered_env_insert(&mut env_vars, name, value);
                        }
                    }
                }
                Err(error) => {
                    let mut stderr = Vec::new();
                    writeln!(&mut stderr, "env: {file}: {error}")?;
                    self.write_default_stderr(&stderr)?;
                    self.exit_code = 1;
                    return Ok(None);
                }
            }
        }
        for name in &config.unset_names {
            ordered_env_remove(&mut env_vars, name);
        }
        for (name, value) in &config.assignments {
            ordered_env_insert(&mut env_vars, name.clone(), value.clone());
        }
        materialize_required_windows_env(
            &mut env_vars,
            &self.shell_state.env_vars,
            config.ignore_environment,
        );
        Ok(Some(env_vars))
    }

    fn parse_env_short_option_cluster(
        &mut self,
        arg: &str,
        args: &[String],
        index: &mut usize,
        config: &mut EnvCommandConfig,
    ) -> Result<Option<Option<Vec<String>>>, ExecuteError> {
        if !arg.starts_with('-') || arg.starts_with("--") || arg == "-" || arg.len() <= 2 {
            return Ok(None);
        }

        let mut chars = arg[1..].char_indices().peekable();
        while let Some((offset, option)) = chars.next() {
            match option {
                'i' => {
                    config.ignore_environment = true;
                    continue;
                }
                '0' => {
                    config.null_terminated = true;
                    continue;
                }
                'v' => {
                    config.debug = true;
                    continue;
                }
                'u' | 'C' | 'f' | 'a' | 'S' => {
                    let value = if chars.peek().is_some() {
                        arg[1 + offset + option.len_utf8()..].to_string()
                    } else {
                        let Some(value) = args.get(*index).cloned() else {
                            let message = match option {
                                'u' => "env: option --unset requires an argument\n",
                                'C' => "env: option --chdir requires an argument\n",
                                'f' => "env: option --file requires an argument\n",
                                'a' => "env: option --argv0 requires an argument\n",
                                'S' => "env: option --split-string requires an argument\n",
                                _ => unreachable!(),
                            };
                            self.write_default_stderr(message.as_bytes())?;
                            self.exit_code = 125;
                            return Ok(Some(None));
                        };
                        *index += 1;
                        value
                    };

                    match option {
                        'u' => return Ok(Some(Some(vec![format!("--unset={value}")]))),
                        'C' => return Ok(Some(Some(vec![format!("--chdir={value}")]))),
                        'f' => return Ok(Some(Some(vec![format!("--file={value}")]))),
                        'a' => return Ok(Some(Some(vec![format!("--argv0={value}")]))),
                        'S' => {
                            let split = crate::executor::alias_helpers::split_shell_words(&value);
                            return Ok(Some(Some(split)));
                        }
                        _ => unreachable!(),
                    }
                }
                _ => {
                    let mut stderr = Vec::new();
                    writeln!(&mut stderr, "env: invalid option -- '{option}'")?;
                    self.write_default_stderr(&stderr)?;
                    self.exit_code = 125;
                    return Ok(Some(None));
                }
            }
        }

        Ok(Some(Some(Vec::new())))
    }

    pub(in crate::executor) fn execute_external_inner(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        if cmd.words.is_empty() {
            return Ok(());
        }

        if self.handle_external_shortcuts(cmd)? {
            self.note_emulated_foreground_wait();
            return Ok(());
        }

        if self.handle_host_external_command(cmd)? {
            self.note_emulated_foreground_wait();
            return Ok(());
        }

        if self.handle_external_file_builtins(cmd)? {
            self.note_emulated_foreground_wait();
            return Ok(());
        }

        if let Some(name) = bash_aliases_assignment_name(&cmd.words[0]) {
            eprintln!("{}`{name}': invalid alias name", self.diagnostic_prefix());
            self.exit_code = 1;
            return Ok(());
        }

        if self.command_output_redirect_fails(cmd)? {
            return Ok(());
        }

        let Some(program) = self.find_scoped_user_command(&cmd.words[0]) else {
            // GNU execute_cmd.c (execute_simple_command /
            // command_not_found_handler): when the shell function
            // `command_not_found_handle` is defined, it is invoked with
            // the full command line as its arguments instead of the
            // `command not found` diagnostic, and its exit status becomes
            // the command's status. The executing_command_not_found_handler
            // guard keeps a missing command inside the handler itself on
            // the plain 127 path (rubash#1080).
            if !cmd.words[0].contains('/')
                && self
                    .shell_state
                    .functions
                    .contains_key("command_not_found_handle")
                && !self
                    .shell_state
                    .env_vars
                    .contains_key("__RUBASH_CNF_HANDLER_RUNNING")
            {
                self.shell_state
                    .env_vars
                    .insert("__RUBASH_CNF_HANDLER_RUNNING".to_string(), "1".to_string());
                // GNU passes the whole command line: $1 is the missing
                // command name, $2.. its arguments.
                let args: Vec<String> = cmd.words.to_vec();
                let mut handler_cmd = cmd.clone();
                handler_cmd.words = std::iter::once("command_not_found_handle".to_string())
                    .chain(cmd.words.iter().cloned())
                    .collect();
                let result = self.execute_function("command_not_found_handle", &args, &handler_cmd);
                self.shell_state
                    .env_vars
                    .remove("__RUBASH_CNF_HANDLER_RUNNING");
                result?;
                return Ok(());
            }
            let mut stderr = Vec::new();
            // GNU findcmd.c:385-386 (search_for_command): a name containing
            // a slash is never PATH-searched — it goes straight to execve,
            // so a missing file reports the errno text from
            // execute_cmd.c:6126-6159 shell_execve ("No such file or
            // directory", EX_NOTFOUND), a directory reports EISDIR
            // ("Is a directory", EX_NOEXEC), and an unexecutable file
            // reports "Permission denied" (EX_NOEXEC). Only a slash-free
            // name that PATH could not resolve is "command not found"
            // (rubash#173). All of these are internal_error via
            // error_prolog, so the prefix honors gnu_errfmt (rubash#1038).
            let error_prefix = self.error_prefix();
            let name = super::execution_misc::printable_filename(&cmd.words[0]);
            if let Some((message, status)) =
                slash_command_execve_error_message(&cmd.words[0], &self.shell_state.env_vars)
            {
                writeln!(&mut stderr, "{error_prefix}{name}: {message}",)?;
                self.finish_external_error(cmd, &stderr, status)?;
                return Ok(());
            }
            // GNU findcmd.c:279-293 + execute_cmd.c shell_execve: with
            // PATH unset the name is never searched, so the failure is
            // the execve ENOENT text, not "command not found" — bash
            // switches diagnostic on PATH being unset (rubash#1145). The
            // report is internal_error (error_prolog family), so the
            // prefix honors gnu_errfmt (rubash#1038).
            let not_found_message =
                if crate::executor::path::path_is_unset(&self.shell_state.env_vars) {
                    "No such file or directory"
                } else {
                    "command not found"
                };
            writeln!(
                &mut stderr,
                "{}{}: {}",
                self.error_prefix(),
                name,
                not_found_message
            )?;
            self.finish_external_error(cmd, &stderr, 127)?;
            return Ok(());
        };

        // GNU findcmd.c:365/416 (search_for_command): a name resolved through
        // hashed_filenames bumps times_found; one resolved via PATH enters
        // the table with times_found=1 (phash_insert found=1). Skipped for
        // absolute names (absolute_program gate), `set +h`, and command-local
        // PATH — the same gates find_user_command applies to its cache.
        // Internally-emulated commands (external_file_builtins) never reach
        // here, so they stay out of the table rather than diverting to a
        // missing host binary.
        if !cmd.words[0].contains('/')
            && !cmd.words[0].contains('\\')
            && crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "hashall")
            && self
                .shell_state
                .env_vars
                .get("__RUBASH_TEMP_PATH")
                .map(String::as_str)
                != Some("1")
        {
            let display = super::execution_misc::shell_display_path(
                &program.to_string_lossy().replace('\\', "/"),
            );
            if self.standard_path_scope.is_some() {
                // rubash#1648: a `command -p` resolution inserts
                // unconditionally (see record_standard_path_resolution).
                crate::builtins::hash::record_standard_path_resolution(
                    &mut self.shell_state.env_vars,
                    &cmd.words[0],
                    &display,
                );
            } else {
                crate::builtins::hash::record_command_resolution(
                    &mut self.shell_state.env_vars,
                    &cmd.words[0],
                    &display,
                );
            }
        }

        // GNU execute_cmd.c:6139-6233 (shell_execve): the OS-level exec is
        // attempted first, and only a file the OS refuses to exec is
        // classified by its first bytes: an unresolvable #! interpreter is
        // refused ("bad interpreter", EX_NOEXEC), a binary first line is
        // refused ("cannot execute binary file", EX_BINARY_FILE), and plain
        // text falls back to shell-script execution. Rubash's equivalent
        // boundary is should_run_with_shell: files Windows cannot exec
        // natively get the GNU first-bytes classification before the
        // shell-script fallback wraps them.
        if crate::executor::path::should_run_with_shell(&program) {
            if let Some((diagnostic, status)) = self.exec_format_refusal(cmd, &program) {
                let mut stderr = Vec::new();
                let _ = writeln!(&mut stderr, "{diagnostic}");
                self.finish_external_error(cmd, &stderr, status)?;
                return Ok(());
            }
        }

        // GNU execute_cmd.c:6139 shell_execve attempts execve directly
        // and classifies an ENOEXEC refusal by first bytes (refuse
        // binaries, run text as a script in THIS shell). std::process::
        // Command spawns through glibc execvp, whose own ENOEXEC
        // fallback silently substitutes /bin/sh — a foreign shell with
        // different syntax — so on unix the classification must happen
        // BEFORE the spawn: a regular file that is neither #!-headed
        // nor a native ELF image is exactly what execve refuses
        // (rubash#1037).
        //
        // rubash#2345: GNU's re-entry runs inside the FORKED child
        // (execute_cmd.c:6237-6260), so on a rooted, opted-in
        // executor the classification only refuses (binary / bad
        // interpreter) and otherwise RETARGETS the shared spawn tail
        // below at this binary, with the script spelling
        // (enoexec_script_spelling) as its lead argument — observably
        // the explicit `rubash script` invocation, a real process
        // with its own pid and signal domain. The in-process entry
        // remains for library hosts and fork-copy clones, whose
        // process model is the emulation.
        #[cfg(unix)]
        let enoexec_child = if is_enoexec_candidate(&program) {
            if let Some((diagnostic, status)) = self.exec_format_refusal(cmd, &program) {
                let mut stderr = Vec::new();
                let _ = writeln!(&mut stderr, "{diagnostic}");
                self.finish_external_error(cmd, &stderr, status)?;
                return Ok(());
            }
            let target = self.enoexec_spawn_target(cmd, &program);
            if target.is_none() {
                self.execute_enoexec_shell_script(cmd, &program)?;
                return Ok(());
            }
            target
        } else {
            None
        };
        #[cfg(not(unix))]
        let enoexec_child: Option<(std::path::PathBuf, String)> = None;
        // The shared tail's spawn target: the resolved program, or —
        // for a retargeted ENOEXEC child — this binary, named by its
        // own path and led by the script spelling in the argv built
        // below. Everything else about the tail (operand
        // materialization, environment, redirects, the wait and its
        // notices) is the ordinary disk-command machinery, keyed on
        // the typed command as before.
        let (spawn_program, enoexec_lead): (std::path::PathBuf, Option<String>) =
            match enoexec_child {
                Some((exe, spelling)) => (exe, Some(spelling)),
                None => (program.clone(), None),
            };
        let spawn_name: String = match &enoexec_lead {
            Some(_) => spawn_program.to_string_lossy().into_owned(),
            None => cmd.words[0].clone(),
        };

        let (dev_args, dev_ops) = self.materialize_dev_fd_operands(
            &cmd.words[1..],
            crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
            crate::executor::dev_fd_operands::DevOperandStdout::FdTable,
            Some(spawn_program.as_path()),
        );
        // rubash#1905: synthetic /proc operands become spawn-time
        // snapshot files for a child without its own /proc layer; the
        // record lives until spawn_external_process has waited the
        // child out (function scope end). A retargeted ENOEXEC child
        // IS this binary — it owns a /proc layer like any explicit
        // invocation — so its operands key on the binary, not the
        // script.
        let (proc_args, _proc_ops) = crate::executor::proc_operands::materialize_proc_operands(
            &dev_args,
            Some(&spawn_name),
            Some(spawn_program.as_path()),
            &self.shell_state.env_vars,
        );
        // rubash#2378: the execve filename is the name the search
        // constructed (GNU findcmd.c's sh_makepath join — the typed
        // word itself when it carries a slash, since a slash name is
        // never searched, findcmd.c:385), so a script child's $0 is
        // `./emit`, not the anchored absolute path. The child is born
        // in the session cwd (next line), which is what a relative
        // join resolves against. Hash-recorded resolutions keep the
        // anchored path (exec_name_for_program declines them).
        // rubash#2442: this computation is the shared funnel
        // (path::spawn_exec_name) — every spawn route derives its
        // filename through it, not just this foreground one.
        let exec_name = crate::executor::path::spawn_exec_name(
            &spawn_name,
            &spawn_program,
            &self.shell_state.env_vars,
        );
        // A retargeted ENOEXEC child's argv is the script spelling
        // followed by the command's own (materialized) operands —
        // the argv an explicit `rubash script ...` invocation hands
        // the child. The unretargeted tail passes the operands as-is.
        let spawn_args: std::borrow::Cow<'_, [String]> = match &enoexec_lead {
            Some(spelling) => {
                let mut args = Vec::with_capacity(proc_args.len() + 1);
                args.push(spelling.clone());
                args.extend(proc_args.iter().cloned());
                std::borrow::Cow::Owned(args)
            }
            None => std::borrow::Cow::Borrowed(&proc_args),
        };
        let (mut process, used_shell) = external_command_for_named_program_with_exec_name(
            &spawn_program,
            Some(&spawn_name),
            &spawn_args,
            &self.shell_state.env_vars,
            self.bash_argv0_env_supplied(&cmd.assignments),
            exec_name.as_deref(),
        );
        // G-02: children are born in the session cwd, not the process cwd.
        process.current_dir(&self.shell_state.cwd);
        self.apply_external_environment(cmd, &mut process, &spawn_program);
        self.apply_external_redirects(cmd, &mut process)?;
        // niubash#567 route seam: consult the host provider with
        // the spawn fully prepared — every redirect already
        // adjudicated by apply_external_redirects above, so a
        // handled call leaves spawn semantics byte-for-byte, and a
        // decline falls through to the prepared spawn untouched.
        // The shell-spawn leg (`used_shell`) never routes: its
        // payload IS rubash. Materialization that rewrote a word
        // (a /dev/fd or process-substitution operand became a
        // carrier path) also keeps the call on spawn, whose
        // operand cleanup the materialization record feeds.
        let mut route_pulled_stdin: Option<String> = None;
        // rubash#2345: a retargeted ENOEXEC spawn never routes — like
        // the used_shell leg, its payload IS rubash.
        if enoexec_lead.is_none()
            && !used_shell
            && !dev_ops.has_deferred_flushes()
            && proc_args.as_slice() == &cmd.words[1..]
        {
            match self.try_external_route(cmd, &program, &proc_args)? {
                RouteAttempt::Handled => {
                    self.note_emulated_foreground_wait();
                    return Ok(());
                }
                RouteAttempt::Declined(pulled_text) => route_pulled_stdin = pulled_text,
            }
        }
        // A retargeted ENOEXEC spawn takes the find_shell spawn
        // shape (used_shell = true, bookkeeping keyed on the script):
        // the prepared Command's payload is this binary, not
        // `program`, and spawn_external_process's program-keyed
        // machinery (the synthetic-ENOEXEC pre-empt, the exec-format
        // fallbacks, the spawn-error diagnostics) is about the
        // script, exactly as for the find_shell leg below.
        self.spawn_external_process(
            cmd,
            &program,
            process,
            used_shell || enoexec_lead.is_some(),
            dev_ops,
            false,
            route_pulled_stdin,
        )
    }

    fn handle_host_external_command(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // Top-level dispatch: the dispatch seam applied this command's
        // prefix assignments to the table before reaching here.
        let Some(output) = self.invoke_host_external_command(cmd, true) else {
            return Ok(false);
        };
        self.write_buffered_builtin_output(cmd, &output.stdout, &output.stderr)?;
        self.exit_code = output.status;
        Ok(true)
    }

    /// Build the env map for the embedded-host channel. `assignments_applied`
    /// records whether the command's prefix assignments were already applied
    /// to the shell variable table: true on the top-level dispatch path
    /// (the dispatch seam runs apply_temporary_assignments first), false on
    /// the pipeline-stage path (a stage's assignments live only on the
    /// cloned command node, never on the table — the same split the stage
    /// spawner faces). The handler contract itself
    /// (`FnMut(&[String], &HashMap<String, String>)`) is untouched; only
    /// the values composed into the map change (rubash#1766).
    pub(in crate::executor) fn invoke_host_external_command(
        &mut self,
        cmd: &CommandNode,
        assignments_applied: bool,
    ) -> Option<HostExternalCommandOutput> {
        let mut env_vars = self.shell_state.env_vars.clone();
        // rubash#1766, adjudication (c): a nameref cell's plain text is
        // the TARGET NAME, not a value — handing it to the host as an
        // env entry leaks an internal representation. GNU measured:
        // `declare -n ref=var; var=orig; ref+=x env` -> var=origx and
        // no ref= line. Drop nameref alias cells from the base clone,
        // selected by the NAMEREF_VARS attribute set (never by name);
        // the target cell itself stays, so no information is lost.
        // The prefix composition below is untouched.
        for name in marked_env_names(&self.shell_state.env_vars, NAMEREF_VARS) {
            env_vars.remove(&name);
        }
        let raws_aligned = cmd.assignment_raws.len() == cmd.assignments.len();
        for (index, (var_name, var_value)) in cmd.assignments.iter().enumerate() {
            let (base_name, append) = assignment_name_and_append(var_name);
            // GNU variables.c:3589 assign_in_env: an ASSIGN_DISALLOWED
            // (att_noassign) prefix target never enters the temporary
            // environment — no env entry is composed for it
            // (rubash#2369).
            if self.assignment_target_is_noassign(base_name) {
                continue;
            }
            // GNU variables.c:3564-3578 assign_in_env -> bind_variable: a
            // prefix assignment to a nameref binds the referenced variable,
            // so the child env carries the TARGET name and the nameref's
            // own env slot (its cell text) is unchanged (nameref14.sub:
            // `ref=$ref$str printenv ref` still prints "var").
            let Some(env_name) = self.tempenv_export_name(base_name) else {
                continue;
            };
            if append && assignments_applied {
                // Top-level path: the dispatch seam already composed the
                // append into the table, so the env clone above inherits
                // the composed value. Re-applying the raw RHS here would
                // overwrite it (a+=5 must see 145, not 5) — the skip the
                // direct path (apply_external_environment) performs, and
                // the rubash#1620 root-cause-2 shape on this channel.
                continue;
            }
            let raw = raws_aligned
                .then(|| cmd.assignment_raws.get(index))
                .flatten()
                .map(String::as_str);
            let expanded_value = self.expand_assignment_value_with_raw(var_name, var_value, raw);
            // GNU variables.c assign_in_env binds a compound `name=(...)`
            // tempenv word as the literal list text; the internal
            // COMPOUND_ASSIGNMENT_MARKER must not leak into the host env
            // map (mirror of apply_external_environment, niubash#121).
            let expanded_value = expanded_value
                .strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(&expanded_value);
            let expanded_value = if append {
                // Pipeline-stage path: the stage's assignments were never
                // applied to the table, so the clone still holds the window
                // value; compose it the way the stage spawner does —
                // tempenv_scalar_view of the export target (scalar itself,
                // indexed element 0, assoc empty) plus the stripped RHS —
                // which reproduces the GNU oracle values (1(9), 10(30),
                // ([j]=w)) instead of the raw RHS.
                format!("{}{}", self.tempenv_scalar_view(&env_name), expanded_value)
            } else {
                expanded_value.to_string()
            };
            if is_valid_process_env(&env_name, &expanded_value) {
                env_vars.insert(env_name, expanded_value);
            }
        }
        self.host_external_command_handler
            .as_mut()
            .and_then(|handler| (handler.0)(&cmd.words, &env_vars))
    }

    fn handle_external_shortcuts(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        if self.execute_same_shell_script(cmd)? {
            return Ok(true);
        }

        if cmd.words[0] == "cat" && self.handle_hashed_cat_checkhash()? {
            return Ok(true);
        }

        if matches!(cmd.words[0].as_str(), "/bin/echo" | "/usr/bin/echo") {
            // Issue #2093 (GNU findcmd.c:383-385): a slash-bearing word is
            // execve'd verbatim, so when the shell path layer translates
            // this spelling to an existing, executable literal file, that
            // FILE answers (--help/--version, no xpg_echo) and the emulated
            // builtin answer must stand down. Declining routes to the real
            // exec path below, whose own literal probe resolves the same
            // file. With no literal file the redirect stays (the Windows
            // no-/bin default, docs/builtins.md §4.3).
            if crate::executor::path::literal_executable_file(
                &cmd.words[0],
                &self.shell_state.env_vars,
            ) {
                return Ok(false);
            }
            // TODO(findcmd.c/execute_cmd.c): On Windows test runs, Bash-style
            // absolute utility paths should resolve through the active shell
            // environment. Keep this echo mapping until command lookup has a
            // full Unix-path compatibility layer. Output goes through the
            // buffered builtin channel so command substitution captures it
            // (execute_cmd.c pipes external stdout like any other command).
            let mut stdout = Vec::new();
            // rubash#1488: the emulated /bin/echo is still the echo
            // builtin's escape path — same locale guard as execute_echo.
            let _locale_guard = crate::locale::ShellLocaleGuard::new(&self.shell_state.env_vars);
            crate::builtins::echo::write_echo(
                cmd.words[1..].iter().map(String::as_str),
                &mut stdout,
            )?;
            self.write_buffered_builtin_output(cmd, &stdout, &[])?;
            // rubash#1496: sh_chkwrite — a write failure the routing
            // folded into the flag (e.g. /bin/echo hi > /dev/full) fails
            // the command; the unconditional 0 here erased it.
            self.exit_code = if self.take_builtin_write_failed() {
                1
            } else {
                0
            };
            return Ok(true);
        }

        if cmd.words[0] == "diff" && cmd.words.len() == 3 {
            // TODO(subst.c/execute_cmd.c): Process substitution should execute
            // each command and pass named pipes/FIFOs to `diff`. Upstream
            // shopt1.sub uses `diff <("$t1") <("$t2")` where the files are
            // executable helper scripts that differ only by a shebang.
            //
            // rubash#2043 A4: the in-process equality answer may only
            // stand in for the real diff on that documented shape —
            // both operands produced by a process substitution of THIS
            // command (a parser-recorded, unquoted `<(...)` span on
            // each operand word). For plain files the shortcut
            // silently reported a shebang-only difference as equal
            // where GNU diff reports it (rc 1 with the `1c1` block);
            // those fall through to the real diff like every other
            // operand pair.
            let both_operands_are_procsub = [1usize, 2].iter().all(|&index| {
                cmd.word_metadata.get(index).is_some_and(|metadata| {
                    metadata
                        .process_substitutions
                        .iter()
                        .any(|substitution| !substitution.output)
                })
            });
            if both_operands_are_procsub {
                let left = shell_path_to_windows(
                    &self.expand_word(&cmd.words[1]),
                    &self.shell_state.env_vars,
                );
                let right = shell_path_to_windows(
                    &self.expand_word(&cmd.words[2]),
                    &self.shell_state.env_vars,
                );
                if let (Ok(left_source), Ok(right_source)) =
                    (fs::read_to_string(left), fs::read_to_string(right))
                {
                    if strip_shebang(&left_source) == strip_shebang(&right_source) {
                        self.exit_code = 0;
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }

    fn handle_hashed_cat_checkhash(&mut self) -> Result<bool, ExecuteError> {
        // GNU findcmd.c:367-380 + execute_disk_command: a hashed name execs
        // the remembered pathname verbatim. With checkhash off a stale entry
        // fails at execve (diagnostic names the hashed path, status 127);
        // with checkhash on the entry is stat'ed first and forgotten when
        // stale, after which the PATH search runs normally.
        let Some(path) = crate::builtins::hash::hashed_path(&self.shell_state.env_vars, "cat")
        else {
            return Ok(false);
        };
        let native = shell_path_to_windows(&path, &self.shell_state.env_vars);
        if native.is_file() {
            return Ok(false);
        }
        if crate::builtins::shopt::checkhash_enabled() {
            crate::builtins::hash::remove_hashed_path(&mut self.shell_state.env_vars, "cat");
            return Ok(false);
        }
        // GNU execute_cmd.c:6126 shell_execve refusal of a hashed pathname
        // (internal_error via error_prolog) honors gnu_errfmt.
        eprintln!("{}{}: No such file or directory", self.error_prefix(), path);
        self.exit_code = 127;
        Ok(true)
    }

    /// Whether a BASH_ARGV0 value crosses the child boundary for the
    /// command about to spawn (rubash#1338): a prefix assignment in
    /// `assignments`, or a bound value carrying the exported mark (the
    /// tempenv apply marks prefix names exported for the command, so the
    /// mark test covers both). GNU variables.c:1547-1555 (set_argv0)
    /// makes such a value the child's dollar_vars[0]; the spawn
    /// builder's fabricated $0 operand for `sh|bash -c` would outrank
    /// it in the child, so the builder stands down when this is true.
    /// A plain unexported binding stays in the parent (GNU
    /// make_env_array_from_var_list) and does not count.
    pub(in crate::executor) fn bash_argv0_env_supplied(
        &self,
        assignments: &[(String, String)],
    ) -> bool {
        assignments
            .iter()
            .any(|(name, _)| assignment_name_and_append(name).0 == "BASH_ARGV0")
            || (is_marked_var(&self.shell_state.env_vars, EXPORTED_VARS, "BASH_ARGV0")
                && self
                    .shell_state
                    .env_vars
                    .get("BASH_ARGV0")
                    .is_some_and(|value| !value.is_empty()))
    }

    pub(in crate::executor) fn apply_external_environment(
        &mut self,
        cmd: &CommandNode,
        process: &mut Command,
        program: &std::path::Path,
    ) {
        self.apply_child_environment(process);
        let raws_aligned = cmd.assignment_raws.len() == cmd.assignments.len();
        for (index, (var_name, var_value)) in cmd.assignments.iter().enumerate() {
            let (base_name, append) = assignment_name_and_append(var_name);
            // GNU variables.c:3589 assign_in_env: an ASSIGN_DISALLOWED
            // (att_noassign) prefix target never enters the temporary
            // environment — no env entry is composed for it
            // (rubash#2369).
            if self.assignment_target_is_noassign(base_name) {
                continue;
            }
            if append {
                // execute_cmd.c: prefix assignment words are applied to the
                // shell variable table before the command runs; the child
                // environment inherits the already-append-assigned value via
                // apply_child_environment. Re-applying the raw RHS here would
                // overwrite the appended value (a+=5 printenv a must see 145,
                // not 5).
                continue;
            }
            let raw = raws_aligned
                .then(|| cmd.assignment_raws.get(index))
                .flatten()
                .map(String::as_str);
            let expanded_value = self.expand_assignment_value_with_raw(var_name, var_value, raw);
            // GNU variables.c assign_in_env binds a compound `name=(...)`
            // tempenv word as the literal list text; the internal
            // COMPOUND_ASSIGNMENT_MARKER must not leak into the child's
            // environment (niubash #121).
            let expanded_value = expanded_value
                .strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(&expanded_value);
            let Some(env_name) = self.tempenv_export_name(base_name) else {
                continue;
            };
            if is_valid_process_env(&env_name, expanded_value) {
                // Prefix assignments overwrite apply_child_environment above;
                // retain its native TMPDIR boundary (niubash#162).
                if cfg!(windows) && env_name == "TMPDIR" {
                    process.env(&env_name, self.child_env_value(&env_name, expanded_value));
                } else {
                    process.env(env_name, expanded_value);
                }
            }
        }
        // GNU execute_cmd.c execute_disk_command finishes the child
        // environment with put_command_name_into_env (variables.c):
        // `_` is set to the pathname search_for_command resolved for
        // the command about to run, superseding any inherited or
        // prefix-assigned entry, for every external command
        // (rubash#1806). Setting it last in this composition is what
        // makes the supersede hold over both sources above.
        process.env("_", program.to_string_lossy().into_owned());
    }

    /// The route seam's form of [`Executor::apply_external_environment`]:
    /// the complete environment of the call about to be routed, as
    /// ordered pairs in GNU block composition (variables.c
    /// maybe_make_export_env; rubash#1806) — the command's prefix
    /// assignments first (source order, values resolved by the same
    /// rules apply_external_environment applies: appends skipped
    /// because the table already carries their composed value, the
    /// compound marker stripped, the nameref export-name resolution,
    /// the Windows TMPDIR boundary), then the child-environment
    /// composition ([`Executor::compose_child_environment`]),
    /// add_or_supercede throughout, and `_` finished by
    /// put_command_name_into_env: the resolved program pathname,
    /// superseding any inherited or prefix-assigned entry — a
    /// prefix-assigned `_` keeps its prefix slot carrying the
    /// program value (the #1806 oracle case), any other `_` closes
    /// the composition.
    pub(in crate::executor) fn compose_external_env_pairs(
        &mut self,
        cmd: &CommandNode,
        program: &std::path::Path,
    ) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut underscore_prefix_assigned = false;
        let raws_aligned = cmd.assignment_raws.len() == cmd.assignments.len();
        for (index, (var_name, var_value)) in cmd.assignments.iter().enumerate() {
            let (base_name, append) = assignment_name_and_append(var_name);
            // GNU variables.c:3589 assign_in_env: an ASSIGN_DISALLOWED
            // (att_noassign) prefix target never enters the temporary
            // environment — no env entry is composed for it
            // (rubash#2369).
            if self.assignment_target_is_noassign(base_name) {
                continue;
            }
            if append {
                // Same skip as apply_external_environment: the
                // dispatch seam already composed the append into the
                // table, and the base composition below exports it.
                continue;
            }
            let raw = raws_aligned
                .then(|| cmd.assignment_raws.get(index))
                .flatten()
                .map(String::as_str);
            let expanded_value = self.expand_assignment_value_with_raw(var_name, var_value, raw);
            let expanded_value = expanded_value
                .strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(&expanded_value);
            let Some(env_name) = self.tempenv_export_name(base_name) else {
                continue;
            };
            if is_valid_process_env(&env_name, expanded_value) {
                let value = if cfg!(windows) && env_name == "TMPDIR" {
                    self.child_env_value(&env_name, expanded_value)
                } else {
                    expanded_value.to_string()
                };
                if env_name == "_" {
                    underscore_prefix_assigned = true;
                }
                super::readonly_functions::env_pair_supercede(&mut pairs, env_name, value);
            }
        }
        for (name, value) in self.compose_child_environment() {
            if name == "_" {
                // `_` is finished below, superseding (see the doc).
                continue;
            }
            super::readonly_functions::env_pair_supercede(&mut pairs, name, value);
        }
        let program_value = program.to_string_lossy().into_owned();
        if underscore_prefix_assigned {
            if let Some(slot) = pairs.iter_mut().find(|(name, _)| name == "_") {
                slot.1 = program_value;
            }
        } else {
            pairs.retain(|(name, _)| name != "_");
            pairs.push(("_".to_string(), program_value));
        }
        pairs
    }

    /// One route-seam attempt for the foreground external call
    /// whose spawn is fully prepared (niubash#567). `Handled`
    /// when the registered provider COMPLETED or was CANCELLED —
    /// the call ran in-process and this fn owns its finish (exit
    /// code, capture routing) — and `Declined(pulled_text)` when
    /// the call must spawn: no provider, an engine-side decline
    /// gate, a realization decline, or the provider's own
    /// decline. A decline leaves every piece of spawn state
    /// exactly as the preparation left it; the pulled text is the
    /// stdin text the realization already pulled with the feed's
    /// own pull (when the source was a text buffer), handed to
    /// the spawn's feed so the body's substitutions run exactly
    /// once on either route.
    fn try_external_route(
        &mut self,
        cmd: &CommandNode,
        program: &std::path::Path,
        proc_args: &[String],
    ) -> Result<RouteAttempt, ExecuteError> {
        use super::external_route::{
            route_sigint_pending, ExternalRouteOutcome, ExternalRouteRequest,
        };
        let Some(provider_slot) = self.external_route_provider.clone() else {
            return Ok(RouteAttempt::Declined(None));
        };
        // Engine-side decline gates: shapes whose plumbing is
        // spawn-internal. An fd >= 3 redirect (or an fd-var
        // binding) travels the `__RUBASH_FD_*` env channel and the
        // parent-side fd fixups pinned by the fd regression
        // suites; the legacy piped fallback means the stdio plan
        // itself declined, so there are no slots to realize.
        if cmd
            .redirects
            .iter()
            .any(|redirect| redirect.fd_var.is_some() || redirect.fd.is_some_and(|fd| fd >= 3))
            || self.external_stdio_piped_fallback
        {
            return Ok(RouteAttempt::Declined(None));
        }
        let Some(mut route_stdio) = self.realize_route_stdio() else {
            return Ok(RouteAttempt::Declined(None));
        };
        let Some(stdin) = self.route_stdin_handle(cmd) else {
            return Ok(RouteAttempt::Declined(None));
        };
        route_stdio.stdio.stdin = stdin.handle;
        let pulled_text = stdin.pulled_text;
        let env = self.compose_external_env_pairs(cmd, program);
        let request = ExternalRouteRequest {
            name: &cmd.words[0],
            // The seam argv is the full vector, argv[0] (the
            // command word as typed) included — `proc_args` is
            // its tail, proven word-identical to the typed words
            // by the caller's materialization gate.
            argv: &cmd.words,
            program,
            env: &env,
            cwd: &self.shell_state.cwd,
            stdio: &route_stdio.stdio,
            cancel_check: &route_sigint_pending,
        };
        // The drain starts BEFORE the provider runs (the spawn
        // path's rubash#2094 discipline): a provider writing past
        // the pipe buffer into a command-substitution capture must
        // never deadlock against an undrained reader.
        let drain = route_stdio.capture.map(|(reader, generation)| {
            (
                std::thread::spawn(move || {
                    let mut reader = reader;
                    let mut bytes = Vec::new();
                    let _ = std::io::Read::read_to_end(&mut reader, &mut bytes);
                    bytes
                }),
                generation,
            )
        });
        // A re-entrant consult (the provider somehow re-entered
        // the dispatcher on this executor) declines to spawn
        // rather than deadlocking on the slot's borrow.
        let outcome = match provider_slot.try_borrow_mut() {
            Ok(mut slot) => slot.0.execute_route(&request),
            Err(_) => ExternalRouteOutcome::Declined,
        };
        // Release every realized writer before joining the drain:
        // EOF on the capture reader follows the last writer clone.
        drop(route_stdio.keep_alive);
        drop(stdin.keep_alive);
        let captured =
            drain.map(|(handle, generation)| (handle.join().unwrap_or_default(), generation));
        match outcome {
            // A decline leaves every piece of spawn state exactly
            // as the preparation left it (this fn's contract above)
            // — INCLUDING the spawn plan's capture outcome record.
            // The record owns the capture pipe's reader when the
            // plan bound fd 1 to the active capture, and the
            // prepared spawn's drain is keyed off it: spending the
            // record here dropped the reader and disarmed that
            // drain, so the spawned child wrote into a pipe nobody
            // routed into the capture and its output vanished
            // (niubash#729: `command perl ... | wc -c` printed 0 in
            // niubash, whose session always registers a provider,
            // while the provider-less rubash binary exited this fn
            // at the no-provider gate above and stayed correct).
            // The prepared spawn is the record's owner on this
            // path: spawn_external_process consumes it on every
            // success path and drops it on the spawn-error path,
            // so keeping it leaks nothing into the next command.
            ExternalRouteOutcome::Declined => Ok(RouteAttempt::Declined(pulled_text)),
            ExternalRouteOutcome::Completed(status) => {
                // The route ran the call and no spawn follows: the
                // spawn plan's capture outcome record is spent here
                // so it cannot leak into the NEXT command's take;
                // the routed bytes enter the capture through the
                // realization's own drain instead.
                self.external_stdio_outcome = None;
                if let Some((bytes, generation)) = captured {
                    super::shell_options::write_stdout_capture_at_generation(&bytes, generation)?;
                }
                self.exit_code = status;
                Ok(RouteAttempt::Handled)
            }
            ExternalRouteOutcome::Cancelled => {
                // Same record spend as Completed: this fn owns the
                // cancelled call's finish and no spawn follows.
                self.external_stdio_outcome = None;
                if let Some((bytes, generation)) = captured {
                    super::shell_options::write_stdout_capture_at_generation(&bytes, generation)?;
                }
                // The interrupted status is the dispatch layer's
                // (the provider's contract): 130 unless the
                // pending-signal adjudication — the same one a
                // blocked spawn wait runs — hands back a verdict
                // (a trapped SIGINT requeues for the boundary and
                // the command completes AT 130; an untrapped one
                // unwinds exactly like the wait's abort).
                self.exit_code = 130;
                self.observe_signals_while_blocked()?;
                Ok(RouteAttempt::Handled)
            }
        }
    }

    /// GNU variables.c:3564-3578 assign_in_env -> bind_variable (ASS_NAMEREF):
    /// a tempenv `name=value` whose name resolves through a nameref binds the
    /// final target, so the child environment exports the target name while
    /// the nameref keeps its cell text. A cell resolving to an array element
    /// (`ref -> a[1]`) has no env representation, so nothing is exported.
    /// Unresolvable namerefs keep the literal name (nameref11.sub `r=/ f`
    /// exports `r` as a plain variable).
    pub(in crate::executor) fn tempenv_export_name(&self, base_name: &str) -> Option<String> {
        match self.nameref_resolution(base_name) {
            NamerefResolution::Target(target) => {
                if target.contains('[') {
                    None
                } else {
                    Some(target)
                }
            }
            _ => Some(base_name.to_string()),
        }
    }

    /// Foreground wait for a spawned external command. GNU jobs.c:3064
    /// wait_for blocks in waitpid but the shell stays interruptible: a
    /// caught signal is recorded (trap.c:545 trap_handler -> trap.c:537
    /// set_trap_state, `pending_traps[sig]++`) and its trap action runs at
    /// the next command boundary (execute_cmd.c:643 run_pending_traps —
    /// bash manual, SIGNALS: the trap "will not be executed until the
    /// command completes"); an untrapped terminating signal kills the shell
    /// mid-wait through SIG_DFL. The mailbox port slices the kernel wait
    /// (fd wait_child_slice) and records deliveries between slices with the
    /// same deferral (trap_exec observe_signals_while_blocked); on unix the
    /// kernel backend already records arrivals during the plain blocking
    /// wait.
    fn wait_external_child(
        &mut self,
        child: &mut std::process::Child,
    ) -> Result<std::process::ExitStatus, ExecuteError> {
        #[cfg(not(unix))]
        {
            const SLICE: std::time::Duration = std::time::Duration::from_millis(50);
            loop {
                if let Some(status) =
                    crate::fd::wait_child_slice(child, SLICE).map_err(ExecuteError::IoError)?
                {
                    // rubash#1776: sample the reaped child's CPU for the
                    // Windows CHILDREN half while its handle is live.
                    super::read_helpers::note_reaped_child_cpu(child);
                    return Ok(status);
                }
                if let Err(error) = self.observe_signals_while_blocked() {
                    // Untrapped SIGINT (console CTRL_C via note_console_ctrl_c,
                    // or a mailbox delivery) aborts the wait. Reap the child so
                    // it does not linger when the console event did not kill it.
                    let _ = child.kill();
                    let _ = child.wait();
                    super::read_helpers::note_reaped_child_cpu(child);
                    return Err(error);
                }
            }
        }
        #[cfg(unix)]
        {
            // GNU jobs.c:3086-3102 wait_for installs wait_sigint_handler
            // for the wait: the handler only records (trap.c
            // trap_handler sets pending_traps[SIGINT]) instead of dying,
            // and after the wait the shell reports the dead child's
            // 128+SIGINT status and continues. Without this, SIG_DFL
            // kills an interactive shell inside waitpid on Ctrl-C
            // (rubash#1250; GNU survives with rc=130). Non-interactive
            // shells keep SIG_DFL and die like GNU scripts do.
            let sigint_guard = ForegroundWaitSigintGuard::hold_if_interactive_untrapped(self);
            self.note_foreground_wait();
            // rubash#1990: with the guard armed (interactive, untrapped),
            // wait in slices so a recorded terminal SIGINT can be
            // forwarded to a child the group delivery never reached;
            // unarmed waits keep the plain blocking waitpid.
            let status = if sigint_guard.is_armed() {
                wait_fg_child_sliced(child, &sigint_guard)?
            } else {
                child.wait().map_err(ExecuteError::IoError)?
            };
            // rubash#1639: the foreground job died by SIGINT in an
            // interactive reader session with SIGINT untrapped — GNU
            // jobs.c's waitchld tail (the job_control/IS_FOREGROUND/
            // WTERMSIG==SIGINT block) does ADDINTERRUPT there, and
            // the executing list throws to the top-level reader,
            // discarding the rest of the line instead of continuing
            // it with the 130 status. No shell-side receipt is
            // required under job control: the job's own SIGINT death
            // is the verdict. (Without job control the shell shares
            // the child's process group, the same ^C is what the
            // guard recorded, and the verdict is identical.) Mark the
            // event; the list boundary converts it (the callers still
            // record the 130 status from this ExitStatus first).
            if sigint_guard.is_armed()
                && crate::executor::wait_status::exit_status_signal(&status) == Some(libc::SIGINT)
            {
                self.fg_sigint_unwind = true;
            }
            Ok(status)
        }
    }

    /// rubash#2398: the interactive foreground wait with job control
    /// live (see job_control_wait for the gate). GNU's shape: the
    /// child was made a process-group leader at spawn (jobs.c:2390),
    /// wait_for operates on a registered job (jobs.c:3064), the
    /// terminal belongs to the child's pgrp for the whole wait
    /// (give_terminal_to, jobs.c:4999 — handed over at jobs.c:2400
    /// for a fresh job, reclaimed for shell_pgrp at jobs.c:3253), and
    /// waitchld waits with WUNTRACED (jobs.c:4077), so a stop is a
    /// wait outcome, not a hang: the job is marked Stopped, becomes
    /// current, and the wait's verdict is 128 + the stop signal
    /// (jobs.c:3243). A completed foreground job is deleted from the
    /// table at once; a stopped one stays — listed by `jobs`,
    /// resumable by fg/bg — with its child handle parked in
    /// background_children, the same runtime slot background jobs use.
    #[cfg(unix)]
    fn wait_fg_child_job_control(
        &mut self,
        mut child: std::process::Child,
        command: String,
        stage_commands: Vec<String>,
    ) -> Result<std::process::ExitStatus, ExecuteError> {
        use std::os::unix::process::ExitStatusExt;
        let pid = child.id();
        self.shell_state.job_table.register_process_with_stages(
            pid,
            command,
            false,
            pid,
            stage_commands,
        );
        self.shell_state.job_table.set_job_control(pid, true);
        self.note_foreground_wait();
        let Some(tty) = super::job_control_wait::controlling_tty() else {
            // The gate saw a tty a moment ago; if it is gone, degrade
            // to the plain wait rather than stranding the child.
            let status = child.wait().map_err(ExecuteError::IoError)?;
            self.shell_state.job_table.remove_job_by_pid(pid);
            return Ok(status);
        };
        super::job_control_wait::give_terminal_to(&tty, pid as i32);
        // GNU wait_for installs the recording SIGINT handler for the
        // wait (jobs.c:3086-3102, see wait_external_child): with the
        // terminal handed over, a terminal ^C reaches only the job,
        // so a shell-side SIGINT arrival during the wait is forwarded
        // to the job's group by the poll below, once — the
        // wait_fg_child_sliced discipline (rubash#1990).
        let sigint_guard = ForegroundWaitSigintGuard::hold_if_interactive_untrapped(self);
        enum FgWaitOutcome {
            Exited(std::process::ExitStatus),
            Stopped(i32),
        }
        let mut forwarded = false;
        let outcome: Result<FgWaitOutcome, std::io::Error> = loop {
            let mut raw: libc::c_int = 0;
            let waited =
                unsafe { libc::waitpid(pid as i32, &mut raw, libc::WUNTRACED | libc::WNOHANG) };
            if waited == pid as i32 {
                if libc::WIFSTOPPED(raw) {
                    break Ok(FgWaitOutcome::Stopped(libc::WSTOPSIG(raw)));
                }
                break Ok(FgWaitOutcome::Exited(std::process::ExitStatus::from_raw(
                    raw,
                )));
            }
            if waited < 0 {
                let error = std::io::Error::last_os_error();
                match error.raw_os_error() {
                    Some(libc::EINTR) => {}
                    Some(libc::ECHILD) => {
                        // Nobody else waits on this child; an ECHILD
                        // here means its state was already collected
                        // through the handle — take the handle's word.
                        match child.wait() {
                            Ok(status) => break Ok(FgWaitOutcome::Exited(status)),
                            Err(error) => break Err(error),
                        }
                    }
                    _ => break Err(error),
                }
            }
            if sigint_guard.is_armed() && !forwarded && fg_forward_drain(&sigint_guard) {
                forward_sigint_to_fg_child(pid as i32);
                forwarded = true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        // jobs.c:3253: the shell takes its terminal back on every way
        // out of the wait — exit, signal death, stop, or error.
        super::job_control_wait::give_terminal_to(&tty, super::job_control_wait::shell_pgrp());
        match outcome {
            Ok(FgWaitOutcome::Exited(status)) => {
                self.shell_state.job_table.remove_job_by_pid(pid);
                // rubash#1639, as in wait_external_child: a foreground
                // job's SIGINT death unwinds the executing list. Under
                // job control the verdict is the job's own death —
                // the terminal delivery never reached the shell.
                if sigint_guard.is_armed()
                    && crate::executor::wait_status::exit_status_signal(&status)
                        == Some(libc::SIGINT)
                {
                    self.fg_sigint_unwind = true;
                }
                Ok(status)
            }
            Ok(FgWaitOutcome::Stopped(stop_signal)) => {
                // waitchld's stop bookkeeping (jobs.c:4057): the
                // process and job go Stopped and the job becomes
                // current (the table's mark_stopped does all three);
                // flipping it to a background-listed job is what lets
                // jobs/fg/bg/wait resolve it from here on.
                self.shell_state
                    .job_table
                    .mark_stopped(pid, Some(stop_signal));
                if let Some(job_id) = self.shell_state.job_table.job_id_for_pid(pid) {
                    if let Some(entry) = self.shell_state.job_table.jobs.get_mut(&job_id) {
                        entry.foreground = false;
                        entry.background = true;
                    }
                }
                // The stop notice prints here, as the wait completes
                // (jobs.c:3385) — see format_stopped_job_notice for
                // why it cannot ride the boundary flush.
                if let Some(line) = self.format_stopped_job_notice(pid) {
                    crate::executor::shell_options::write_stderr_bytes(line.as_bytes())?;
                }
                self.background_children
                    .insert(pid, crate::fd::SpawnedChild::from(child));
                // jobs.c:3243: the wait's verdict for a stopped job
                // is 128 + the stop signal, encoded as a plain exit
                // status so the shared post-wait handling records
                // exactly that $? and no signal-death notice.
                Ok(std::process::ExitStatus::from_raw((128 + stop_signal) << 8))
            }
            Err(error) => {
                self.shell_state.job_table.remove_job_by_pid(pid);
                Err(ExecuteError::IoError(error))
            }
        }
    }

    /// rubash#1497: execution-time gates for tail exec replacement,
    /// mirroring GNU builtins/evalstring.c:106 should_optimize_fork as
    /// re-checked when the command runs (a trap armed by an earlier
    /// command of the same -c string still blocks the replacement),
    /// plus this port's capture bookkeeping: replacement is only sound
    /// when the prepared Command inherits the process's own stdio, no
    /// capture drain or deferred fd writeback is pending, and no
    /// virtual stdin text waits in this process's memory. The DEBUG
    /// trap does NOT block (it is not a signal trap; it still fires
    /// before the command, in the shell), and neither does a RETURN
    /// trap at top level (variable_context == 0 there).
    #[cfg(unix)]
    /// rubash#1832: the state-only half of the exec-replace tail
    /// conditions — everything `exec_replace_tail_ready` checks that
    /// depends on the dispatch stamp and shell state alone, not on
    /// the resolved program, the stdio plan, or the dev-fd
    /// materialization. The env emulation folds the SHLVL adjustment
    /// into its composition under this gate (the composition is what
    /// env receives, and GNU adjusts before building it); the spawn
    /// seam still re-checks the full predicate before replacing.
    fn exec_replace_tail_state_ready(&self) -> bool {
        if self.exec_replace_dispatch_stamp.get() == 0 {
            return false;
        }
        if self.invocation_interactive() {
            return false;
        }
        if self
            .shell_state
            .env_vars
            .contains_key(crate::executor::types::FUNCTION_STDIN)
            || self.virtual_fd_stdin_remaining(0).is_some()
        {
            return false;
        }
        let env = &self.shell_state.env_vars;
        // EXIT and ERR traps block by presence (GNU signal_is_trapped
        // counts `trap ''` too); get_trap_action is reset-aware.
        if crate::builtins::trap::get_trap_action(env, "EXIT").is_some()
            || crate::builtins::trap::get_trap_action(env, "ERR").is_some()
        {
            return false;
        }
        // Any real signal trap blocks, again by presence. Pseudo
        // signals and bookkeeping keys share the trap prefix without
        // being signal traps.
        let reset = env
            .get(crate::builtins::trap::TRAP_RESET)
            .map(String::as_str)
            .unwrap_or("");
        for key in env.keys() {
            let Some(signal) = key.strip_prefix(crate::builtins::trap::TRAP_PREFIX) else {
                continue;
            };
            match signal {
                "EXIT" | "ERR" | "ERROR" | "DEBUG" | "RETURN" | "RESET" | "ORIG_IGN" => {}
                _ if reset.split(':').any(|name| name == signal) => {}
                _ => return false,
            }
        }
        true
    }

    #[cfg(unix)]
    fn exec_replace_tail_ready(
        &self,
        cmd: &CommandNode,
        program: &PathBuf,
        dev_ops: &crate::executor::dev_fd_operands::DevOperandMaterialization,
    ) -> bool {
        if !self.exec_replace_tail_state_ready() {
            return false;
        }
        if !crate::executor::path::program_execs_directly(program) {
            return false;
        }
        // CapturePipe means the parent must drain the child's output
        // itself — impossible after a process replacement.
        // LiveHandles is the opposite shape: the prepared Command
        // already carries the real destination handles (files, pipes,
        // fd-table bindings), which an exec inherits unchanged, and
        // the parent's only post-spawn duty was the wait.
        if matches!(
            self.external_stdio_outcome,
            Some(super::external_redirects::ExternalStdioOutcome::CapturePipe(..))
        ) || self.external_needs_fd_copy_capture(cmd)
        {
            return false;
        }
        if dev_ops.has_deferred_flushes() {
            return false;
        }
        true
    }

    /// rubash#1497: GNU execute_cmd.c:5813-5819 — on the nofork path
    /// the shell decrements its level (adjust_shell_level(-1)) before
    /// building the export environment, so the replaced process sees
    /// the level the shell itself inherited. The adjustment reads the
    /// CURRENT SHLVL variable (prefix temp assignments are not yet
    /// committed when execute_disk_command runs), clamps at 0, and the
    /// variable is (re)created in the child environment even when the
    /// shell variable is absent or unexported.
    /// rubash#1497: the shell's current SHLVL as a number — an absent
    /// or unparseable value counts as 0 (GNU adjust_shell_level works
    /// on the variable's numeric value; see execute_cmd.c:5819).
    pub(in crate::executor) fn current_shlvl_value(&self) -> i64 {
        self.shell_state
            .env_vars
            .get("SHLVL")
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0)
    }

    #[cfg(unix)]
    fn exec_replace_adjusted_shlvl(&self) -> String {
        // GNU reads the CURRENT variable, which never includes the
        // tail command's own prefix temp assignments; this port has
        // committed them into env_vars by spawn time, so the value
        // frozen at dispatch (exec_replace_shlvl_snapshot) is the one
        // to adjust. The live read remains as a defensive fallback.
        let current = self
            .exec_replace_shlvl_snapshot
            .get()
            .unwrap_or_else(|| self.current_shlvl_value());
        (current - 1).max(0).to_string()
    }

    /// rubash#1626/#1688: the ambient-fd pin table for one external
    /// child — (parent descriptor, slot) for every fd-table entry >= 3
    /// backed by a real descriptor (a File endpoint, or a coproc
    /// endpoint's raw descriptor), for `pin_ambient_fds_in_child` to
    /// dup2 onto the slot numbers between fork and exec. GNU forks every
    /// external child — simple command or pipeline element alike
    /// (execute_cmd.c:2620 execute_pipeline) — from the shell, so the
    /// child inherits the shell's descriptors at their numbers; the pin
    /// mirrors that inheritance for the table's CLOEXEC descriptors.
    /// Slots the command itself redirects are skipped — the command's
    /// binding is the more local one and travels its own pins / the env
    /// channel — except an injected group redirect, whose binding this
    /// entry already IS. Shared by spawn_external_process and the
    /// pipeline spawn loops, which build their stage processes by hand.
    #[cfg(unix)]
    pub(in crate::executor) fn ambient_fd_pins_for(
        &self,
        cmd: &crate::parser::CommandNode,
    ) -> Vec<(std::os::unix::io::RawFd, u32)> {
        let mut pins: Vec<(std::os::unix::io::RawFd, u32)> = Vec::new();
        for (fd, entry) in &self.fd_table.entries {
            if *fd < 3 || entry.closed {
                continue;
            }
            if let Some(redirect) = cmd
                .redirects
                .iter()
                .find(|redirect| redirect.fd == Some(*fd))
            {
                // The command rebinds this slot itself (see the doc
                // comment); only the injected-group case falls through
                // and pins the entry that already is its binding.
                if !crate::executor::support_names::is_injected_group_redirect(redirect) {
                    continue;
                }
            }
            let mut take =
                |endpoint: Option<&Rc<crate::executor::fd_table::FileFd>>,
                 pins: &mut Vec<(std::os::unix::io::RawFd, u32)>| {
                    if let Some(file) = endpoint {
                        let pin = (file.handle, *fd);
                        if !pins.contains(&pin) {
                            pins.push(pin);
                        }
                    }
                };
            take(
                entry.read.as_ref().and_then(|read| match read {
                    crate::executor::fd_table::FdReadEndpoint::File(file) => Some(file),
                    crate::executor::fd_table::FdReadEndpoint::CoprocStdout { fd, .. } => Some(fd),
                    _ => None,
                }),
                &mut pins,
            );
            take(
                entry.write.as_ref().and_then(|write| match write {
                    crate::executor::fd_table::FdWriteEndpoint::File(file) => Some(file),
                    crate::executor::fd_table::FdWriteEndpoint::CoprocStdin { fd, .. } => Some(fd),
                    _ => None,
                }),
                &mut pins,
            );
        }
        pins
    }

    /// rubash#2292: deliver the fd table's ambient Text endpoints
    /// to a spawn. A Text endpoint is a heredoc / here-string body
    /// bound into the table by an enclosing face — a brace group's,
    /// function's, or script's redirects, applied scoped by
    /// bind_numbered_heredoc_redirects, or a persistent `exec`
    /// binding. The ambient stamp in apply_external_fd_delivery and
    /// ambient_fd_pins_for only carry endpoints backed by a real
    /// handle; a Text endpoint has none, so a spawned grandchild
    /// inherited nothing at the slot and read EBADF where GNU's
    /// forked child inherits the descriptor here_document_to_fd
    /// created (redir.c:423, applied by do_redirections,
    /// redir.c:237). The {var} channel solved the same problem for
    /// its own bindings (#2272) by materializing the snapshot into
    /// a carrier file; this pass is that materialization for
    /// ambient bindings, in the one shared delivery segment.
    ///
    /// The skip rules preserve the owner-tag rationale documented
    /// on ambient_fd_pins_for (2026-09-29): a slot the command's
    /// OWN redirect list binds is the walk's to deliver —
    /// materializing the ambient body there would clobber the
    /// command's binding with the outer one. The same holds for a
    /// slot named by the command's own heredoc_redirects (those
    /// bindings deliver from their entries directly) and for a
    /// slot a {var} redirect of this command claims (the applier
    /// bound it; the {var} delivery arms own it). Injected group
    /// redirects fall through, exactly as in ambient_fd_pins_for:
    /// the table entry already IS the command's binding.
    /// Process-substitution endpoints are not snapshots to
    /// materialize — a live producer pipe must reach the child as
    /// itself — and a slot also carrying a write endpoint keeps
    /// today's composed behavior; both stay untouched.
    fn deliver_ambient_text_endpoints(
        &mut self,
        cmd: &CommandNode,
        process: &mut Command,
        command_fd_pins: &mut FdPinVec,
        parent_side_dups: &mut Vec<crate::fd::HANDLE>,
    ) {
        let mut slots: Vec<u32> = Vec::new();
        for (fd, entry) in &self.fd_table.entries {
            if *fd < 3 || entry.closed || entry.write.is_some() {
                continue;
            }
            if !matches!(
                entry.read.as_ref(),
                Some(crate::executor::fd_table::FdReadEndpoint::Text(_))
            ) {
                continue;
            }
            if let Some(redirect) = cmd
                .redirects
                .iter()
                .find(|redirect| redirect.fd == Some(*fd))
            {
                if !crate::executor::support_names::is_injected_group_redirect(redirect) {
                    continue;
                }
            }
            if cmd
                .heredoc_redirects
                .iter()
                .any(|redirect| redirect.fd == Some(*fd))
            {
                continue;
            }
            let var_claimed = cmd
                .redirects
                .iter()
                .filter_map(|redirect| redirect.fd_var.as_deref())
                .chain(
                    cmd.heredoc_redirects
                        .iter()
                        .filter_map(|redirect| redirect.fd_var.as_deref()),
                )
                .any(|name| self.dynamic_fd_variable_value(name) == Some(*fd));
            if var_claimed {
                continue;
            }
            slots.push(*fd);
        }
        for fd in slots {
            self.deliver_fd_var_text_binding(fd, fd, process, command_fd_pins, parent_side_dups);
        }
    }

    /// rubash#1688: register the ambient-fd pin on a stage process built
    /// by hand in the pipeline spawn loops (they bypass
    /// spawn_external_process, so without this a pipeline member sees
    /// EBADF on every ambient descriptor).
    #[cfg(unix)]
    pub(in crate::executor) fn attach_ambient_fd_pins(
        &self,
        cmd: &crate::parser::CommandNode,
        process: &mut std::process::Command,
    ) {
        let pins = self.ambient_fd_pins_for(cmd);
        if !pins.is_empty() {
            use std::os::unix::process::CommandExt;
            // SAFETY: same contract as the spawn_external_process
            // registration — the closure runs only in the forked child
            // before exec and only dup2/closes raw descriptors.
            unsafe {
                process.pre_exec(move || pin_ambient_fds_in_child(&pins));
            }
        }
    }

    #[cfg(not(unix))]
    pub(in crate::executor) fn attach_ambient_fd_pins(
        &self,
        _cmd: &crate::parser::CommandNode,
        _process: &mut std::process::Command,
    ) {
    }

    /// rubash#2308: the Text half of the ambient delivery for a stage
    /// process built by hand in the pipeline spawn loops. Those loops
    /// register the handle-backed ambient pins through
    /// attach_ambient_fd_pins, but a Text endpoint (heredoc /
    /// here-string body) has no handle to pin — #2292 taught the
    /// sequential faces to materialize it (deliver_ambient_text_endpoints,
    /// inside apply_external_fd_delivery), and the concurrent loops
    /// never ran that pass, so a concurrent stage saw EBADF on every
    /// ambient heredoc descriptor. This runs the SAME pass — same slot
    /// selection, same skip rules, no stage-local copy — registers its
    /// pins as a second pre_exec closure (closures compose:
    /// spawn_external_process stacks the delivery, signal-reset and
    /// /dev/fd closures on one Command), and returns the parent-side
    /// duplicates for the caller to close once its spawn resolves —
    /// the apply_external_fd_delivery contract. The carrier files
    /// themselves live in procsub_adhoc_carriers and die at
    /// finish_process_substitutions, after every stage of the pipeline
    /// has been reaped; the only per-stage artifact is the dup, whose
    /// required lifetime is exactly the one spawn (fork copies the
    /// parent's table; CreateProcess inherits inheritable handles at
    /// the call). GNU materializes the body once (redir.c:423
    /// here_document_to_fd) and every forked pipeline element inherits
    /// the descriptor (redir.c:237 do_redirections; execute_cmd.c:2620
    /// execute_pipeline); the per-spawn snapshot carrier is the
    /// standing #2272/#2292 delivery model, not a new compromise.
    pub(in crate::executor) fn attach_ambient_text_delivery(
        &mut self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Vec<crate::fd::HANDLE> {
        let mut command_fd_pins: FdPinVec = Vec::new();
        let mut parent_side_dups: Vec<crate::fd::HANDLE> = Vec::new();
        self.deliver_ambient_text_endpoints(
            cmd,
            process,
            &mut command_fd_pins,
            &mut parent_side_dups,
        );
        #[cfg(unix)]
        if !command_fd_pins.is_empty() {
            use std::os::unix::process::CommandExt;
            // SAFETY: same contract as the attach_ambient_fd_pins
            // registration — the closure runs only in the forked child
            // before exec and only dup2/closes raw descriptors.
            unsafe {
                process.pre_exec(move || pin_ambient_fds_in_child(&command_fd_pins));
            }
        }
        parent_side_dups
    }

    /// rubash#2255: deliver a command's fd bindings to a hand-built
    /// child process — the segment of spawn_external_process that
    /// (1) stamps every fd-table entry >= 3 through the __RUBASH_FD_*
    /// env channel, (2) opens the command's OWN numbered (fd >= 3)
    /// redirects and stamps/pins them the same way, and (3) registers
    /// the unix pre_exec pin that dup2's the ambient + own descriptors
    /// onto their slot numbers. GNU runs ONE ordered do_redirections
    /// walk in every forked child (redir.c:237 do_redirections;
    /// execute_cmd.c:2620 execute_pipeline forks every element
    /// through the same machinery), so a hand-built pipeline stage
    /// needs exactly this segment, not a stage-local copy:
    /// spawn_external_process (simple commands) and
    /// execute_external_pipeline_stage_inner (sequential pipeline
    /// stages) share it. Returns the parent-side inheritable
    /// duplicates the caller closes once its spawn resolves (the
    /// child holds its own copies).
    ///
    /// The pin registration rides inside the helper so both callers
    /// register identically; spawn_external_process's signal-reset
    /// pre_exec (rubash#229) therefore registers after the pin
    /// instead of before it — the two closures are independent (one
    /// only dup2/closes descriptors, the other only resets signal
    /// dispositions), so their relative order is inert.
    pub(in crate::executor) fn apply_external_fd_delivery(
        &mut self,
        cmd: &CommandNode,
        mut process: &mut Command,
        stage_stdio: Option<&StageStdioContext>,
    ) -> Vec<crate::fd::HANDLE> {
        // rubash#180: GNU's forked disk-command child inherits every fd >= 3
        // the command opened (`"$SH" -c 'echo hi >&3' 3>f` writes f) or that
        // a persistent `exec 3>f` bound in the parent. std::process::Command
        // cannot attach numbered descriptors, so route the binding through
        // the __RUBASH_FD_* env-key channel the background-spawn path and
        // the child's startup (init.rs) already speak: duplicate each handle
        // as a fresh INHERITABLE handle (the shared slot's inherit flag is
        // never touched, so no cross-spawn race), name it in the child's
        // environment, and close the parent-side duplicate after the spawn —
        // exactly GNU's open->fork->close-unwind shape. Non-rubash children
        // simply see one extra env var (same limitation the background path
        // documents).
        let mut parent_side_dups: Vec<crate::fd::HANDLE> = Vec::new();
        // rubash#2272/#2281: this command's deferred {var} closes
        // (close-form targets and move-form dup sources, recorded by
        // the dynamic-fd applier in fd_var_pending_closes with their
        // redirect-list positions) are performed in the table
        // POSITIONALLY during the delivery walk below - GNU applies
        // them in the fork, in list order (redir.c:237), so a close
        // revokes what earlier entries delivered at its slot and a
        // later entry's dup source is still open at its own turn if
        // its position precedes the close. The snapshots taken as
        // each close applies are what undo_child_fd_var_redirects
        // restores afterwards; the parent's bindings survive the
        // command. The ambient stamp loop below runs first and stamps
        // every open slot - a close applied later in the walk revokes
        // its slot's stamps again (apply_pending_close_at).
        let pending_closes = std::mem::take(&mut self.fd_var_pending_closes);
        let mut pending_close_idx = 0usize;
        for (fd, entry) in &self.fd_table.entries {
            if *fd < 3 || entry.closed {
                continue;
            }
            let mut pass = |endpoint: Option<&Rc<crate::executor::fd_table::FileFd>>,
                            write: bool,
                            process: &mut Command,
                            dups: &mut Vec<crate::fd::HANDLE>| {
                let Some(file) = endpoint else {
                    return;
                };
                let Ok(dup) = crate::fd::duplicate_handle_inheritable(file.handle) else {
                    return;
                };
                process.env(
                    format!("__RUBASH_FD_{}HANDLE_{fd}", if write { "W" } else { "" }),
                    format!("{dup:#x}"),
                );
                process.env(
                    format!("__RUBASH_FD_{}PATH_{fd}", if write { "W" } else { "" }),
                    file.path.to_string_lossy().into_owned(),
                );
                dups.push(dup);
            };
            pass(
                entry.read.as_ref().and_then(|read| match read {
                    crate::executor::fd_table::FdReadEndpoint::File(file) => Some(file),
                    _ => None,
                }),
                false,
                &mut process,
                &mut parent_side_dups,
            );
            pass(
                entry.write.as_ref().and_then(|write| match write {
                    crate::executor::fd_table::FdWriteEndpoint::File(file) => Some(file),
                    _ => None,
                }),
                true,
                &mut process,
                &mut parent_side_dups,
            );
        }
        // The command's own fd>=3 file redirects are not fd-table entries on
        // the simple-command path (only compounds bind them scoped), so open
        // them here for the child alone: GNU do_redirections opens them in
        // the parent, the fork inherits, and the parent's copy closes at
        // command end (redir.c undo) — the parent's fd table stays untouched.
        // rubash#1686: on Unix each opened descriptor is also recorded as a
        // pin (merged into the pin registration below): the env channel
        // reaches only rubash children, so a foreign child otherwise gets
        // EBADF on the command's own fds even though the file was created.
        let mut command_fd_pins: FdPinVec = Vec::new();
        // rubash#2272: descriptions this walk itself delivered, by
        // slot, so a later {var} dup entry in the same list copies
        // the command's OWN earlier binding (GNU's single ordered
        // walk, redir.c:237 do_redirections) instead of the ambient
        // table entry the dynamic-fd applier resolved against.
        let mut walk_opened: std::collections::HashMap<
            u32,
            std::rc::Rc<crate::executor::fd_table::FileFd>,
        > = std::collections::HashMap::new();
        // rubash#2278: bindings the numbered dup arm below delivers
        // that are not FileFds (stdio sources), by slot, so a later
        // dup entry sourcing that slot chains onto the same binding
        // (see NumberedDupBinding).
        let mut numbered_dup_bindings: std::collections::HashMap<u32, NumberedDupBinding> =
            std::collections::HashMap::new();
        let mut fd_var_claimed: std::collections::HashSet<u32> = std::collections::HashSet::new();
        // rubash#2292: ambient Text endpoints (compound-bound
        // heredoc bodies) have no handle for the stamp above or
        // the pin computation below; materialize and deliver them
        // here, before the walk, so the pass and the walk never
        // deliver the same slot twice (the pass skips every slot
        // the walk owns — see deliver_ambient_text_endpoints).
        self.deliver_ambient_text_endpoints(
            cmd,
            &mut process,
            &mut command_fd_pins,
            &mut parent_side_dups,
        );
        for (position, redirect) in cmd.redirects.iter().enumerate() {
            // rubash#2281: closes recorded at earlier positions take
            // effect before this entry is served - they revoke this
            // slot's earlier deliveries (see apply_pending_close_at)
            // and mark the table entry closed, so a {var} dup below
            // can no longer resolve a source through them.
            while pending_close_idx < pending_closes.len()
                && pending_closes[pending_close_idx].0 < position
            {
                let fd = pending_closes[pending_close_idx].1;
                pending_close_idx += 1;
                apply_pending_close_at(
                    self,
                    &mut process,
                    &mut command_fd_pins,
                    &mut walk_opened,
                    &mut numbered_dup_bindings,
                    fd,
                );
            }
            if redirect.fd_var.is_some() {
                self.deliver_fd_var_redirect(
                    cmd,
                    position,
                    redirect,
                    &mut process,
                    &mut command_fd_pins,
                    &mut parent_side_dups,
                    &walk_opened,
                    &mut fd_var_claimed,
                );
                continue;
            }
            let fd = match redirect.fd {
                Some(fd) if fd >= 3 => fd,
                _ => continue,
            };
            // rubash#2270: `mut` — the heredoc arm further down
            // rewrites `target` to the materialized carrier path.
            let mut target = self.expand_redirect_target(redirect);
            // rubash#2235: a numbered input entry whose target is an
            // fd-alias PATH spelling (/dev/fd/M, /proc/self/fd/M) —
            // above all a streamed list-level input procsub after
            // materialization — binds fd to M's endpoint: GNU
            // services the alias with an open over the alias layer
            // (rubash#1963), which for a file/procsub endpoint yields
            // the endpoint itself. The generic skip below would
            // leave the slot unbound and the child would read EBADF
            // at fd. Only File endpoints are served here (the
            // streamed procsub registration is always one); other
            // endpoint kinds keep the skip.
            if matches!(
                redirect.kind,
                crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite
            ) && !redirect_target_is_dup_spelling(&target)
                && (target.starts_with("/dev/fd/") || target.starts_with("/proc/self/fd/"))
            {
                if let Some(alias_fd) = redirect_target_fd(&target) {
                    // rubash#2329: the endpoint GNU's open resolves
                    // is the one bound AT THIS POSITION (redir.c:237
                    // do_redirections) — when this command's own
                    // earlier entries bound the alias fd, the
                    // in-list binding decides (a plain file opens
                    // FRESH); the ambient table serves only unbound
                    // or ambient-chained aliases.
                    let endpoint_file = {
                        use crate::executor::external_setup::InListBinding;
                        let ambient_file =
                            |source_fd: u32| match self.fd_table.read_endpoint(source_fd) {
                                Some(crate::executor::fd_table::FdReadEndpoint::File(file)) => {
                                    Some(file.clone())
                                }
                                _ => None,
                            };
                        match self.in_list_binding(cmd, position, alias_fd) {
                            InListBinding::File(path) => {
                                crate::executor::fd_table::FileFd::open_read(path).ok()
                            }
                            InListBinding::AmbientOf(source_fd) => ambient_file(source_fd),
                            InListBinding::NotBound => ambient_file(alias_fd),
                            InListBinding::Other | InListBinding::Closed => None,
                        }
                    };
                    if let Some(file) = endpoint_file {
                        walk_opened.insert(fd, file.clone());
                        if let Ok(dup) = crate::fd::duplicate_handle_inheritable(file.handle) {
                            #[cfg(unix)]
                            command_fd_pins.push((dup, fd));
                            process.env(format!("__RUBASH_FD_HANDLE_{fd}"), format!("{dup:#x}"));
                            process.env(
                                format!("__RUBASH_FD_PATH_{fd}"),
                                file.path.to_string_lossy().into_owned(),
                            );
                            parent_side_dups.push(dup);
                        }
                    }
                }
                continue;
            }
            // rubash#2278: a numbered dup entry (`3<&0`, `4>&3`,
            // `3>&1`) — GNU's ordered walk services it with a dup2
            // of the source AS BOUND AT THIS POSITION
            // (redir.c:237 do_redirections; redir.c:1138
            // do_redirection_internal, the dup2 itself at :1193),
            // and the generic skip below would leave the slot
            // unbound in the child (EBADF). Plain (non-move) dup
            // spellings resolve through deliver_numbered_dup; move
            // spellings (`4<&3-`) keep the fall-through they have
            // today.
            if matches!(
                redirect.kind,
                crate::parser::RedirectKind::DuplicateInput
                    | crate::parser::RedirectKind::DuplicateOutput,
            ) && redirect_target_is_dup_spelling(&target)
            {
                if let Some(source_fd) = redirect_target_fd(&target) {
                    self.deliver_numbered_dup(
                        cmd,
                        position,
                        redirect,
                        fd,
                        source_fd,
                        &mut process,
                        &mut command_fd_pins,
                        &mut parent_side_dups,
                        &mut walk_opened,
                        &mut numbered_dup_bindings,
                        stage_stdio,
                    );
                    continue;
                }
            }
            // rubash#2295: a numbered OUTPUT entry whose target is
            // an fd-alias PATH spelling (`3>/dev/fd/6`) — GNU
            // services the alias with an open over the alias layer
            // (redir.c:725 redir_open via do_redirections,
            // redir.c:237), which for a regular-file endpoint is a
            // FRESH open of that file (rubash#2256; the stdio faces
            // gained the same resolver in rubash#2285). The generic
            // skip below would leave the slot unbound — the child
            // gets EBADF while the parent-side routing probe has
            // already truncated the target. Serve the fresh open
            // through the same stamp/pin channels as the ordinary
            // file arm below; an open failure skips the same way
            // that arm's does (the redirect preflights already
            // reported it). A resolver miss (non-regular endpoint)
            // falls through to the skip, as before.
            if matches!(
                redirect.kind,
                crate::parser::RedirectKind::Output
                    | crate::parser::RedirectKind::Append
                    | crate::parser::RedirectKind::ClobberOutput
            ) && !redirect_target_is_dup_spelling(&target)
                && (target.starts_with("/dev/fd/") || target.starts_with("/proc/self/fd/"))
            {
                let alias_path = redirect_target_fd(&target).and_then(|alias_fd| {
                    // rubash#2329: an in-list file binding at this
                    // position (redir.c:237) is what the spelling
                    // opens; the ambient resolver serves only
                    // unbound / ambient-chained aliases.
                    use crate::executor::external_setup::InListBinding;
                    match self.in_list_binding(cmd, position, alias_fd) {
                        InListBinding::File(path) => Some(path),
                        InListBinding::AmbientOf(source_fd) => self.fd_alias_reopen_path(source_fd),
                        InListBinding::NotBound => self.fd_alias_reopen_path(alias_fd),
                        InListBinding::Other | InListBinding::Closed => None,
                    }
                });
                if let Some(path) = alias_path {
                    if let Ok(file) = self.open_fresh_alias_file(
                        &path,
                        &target,
                        redirect.append,
                        redirect.clobber,
                    ) {
                        #[cfg(windows)]
                        let raw = {
                            use std::os::windows::io::AsRawHandle;
                            file.as_raw_handle() as crate::fd::HANDLE
                        };
                        #[cfg(unix)]
                        let raw = {
                            use std::os::unix::io::AsRawFd;
                            file.as_raw_fd()
                        };
                        self.stamp_child_fd_handle(
                            &mut process,
                            &mut command_fd_pins,
                            &mut parent_side_dups,
                            fd,
                            true,
                            raw,
                            path.to_string_lossy().into_owned(),
                        );
                        if let Ok(clone) = file.try_clone() {
                            walk_opened.insert(
                                fd,
                                crate::executor::fd_table::FileFd::from_file(clone, path),
                            );
                        }
                    }
                    continue;
                }
            }
            // rubash#2304: the null device is NOT skipped here. GNU
            // do_redirection_internal hands every file target to
            // redir_open (redir.c:949), and /dev/null is not one of
            // redir.c's special filenames, so it takes the plain open
            // with the kind's flags (redir.c:745; `<>` is
            // O_RDWR|O_CREAT, make_cmd.c:682) and the descriptor is
            // delivered like any file's. The #180 skip predates the
            // classified open ladders; all three arms below now own
            // the null class explicitly and return a real descriptor
            // of the mapped device (open_classified_input's null arm,
            // create_redirect_output's null arm,
            // probe_input_redirect_readwrite's NullDevice fall-through
            // to the raw mapped open), so dropping the binding here
            // only manufactured EBADF in the child.
            if redirect_target_fd(&target).is_some()
                || is_closed_redirect_target(&target)
                || target.starts_with("<(")
            {
                continue;
            }
            let mut write = !matches!(
                redirect.kind,
                crate::parser::RedirectKind::Input
                    | crate::parser::RedirectKind::DuplicateInput
                    | crate::parser::RedirectKind::CloseInput
                    | crate::parser::RedirectKind::ReadWrite
            );
            if self.fd_table.entries.contains_key(&fd)
                && crate::executor::support_names::is_injected_group_redirect(redirect)
            {
                // The table entry already IS this redirect's binding (a
                // compound applied it to its real descriptors and injected
                // the mirror); re-opening would truncate the shared
                // description. The pin registration below delivers the
                // table entry instead (rubash#1686). A merely ambient
                // entry does NOT skip: the command's binding is the more
                // local one and must win the slot.
                continue;
            }
            // rubash#2235: the fd-0 dup resolution may already have
            // opened this entry's binding (see own_input_bindings):
            // stamp/pin THAT description — one description shared
            // with the child's stdin dup, GNU's dup2 shape — instead
            // of opening the target a second time (a second open of
            // a streamed procsub word would name the producer pipe
            // through the alias layer with a fresh offset, and a
            // plain file would restart at offset 0).
            // Merge-order note (#2235 × #2270): this lookup runs
            // BEFORE the heredoc arm below on purpose. A heredoc /
            // here-string entry still classifies as `write` at this
            // point, so it can never claim a stash. Since #2292 the
            // dup replay DOES stash a materialized carrier for a
            // heredoc source (own_input_binding_before's heredoc
            // arm) — that stash belongs to the entry the replay
            // resolved, the last binding of the fd BEFORE the dup:
            // if it is an earlier plain entry, the heredoc arm's own
            // stamp, applied after it in list order, keeps GNU's
            // last-wins for the slot; if it is THIS entry, the arm
            // materializes the same body again for the fd itself
            // (the dup and the fd get independent carriers — the
            // snapshot model's standing cost, where GNU shares one
            // description).
            let stashed = if write {
                None
            } else {
                self.own_input_bindings.get(&fd).cloned()
            };
            if matches!(
                redirect.kind,
                crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString
            ) {
                // rubash#2270: a heredoc / here-string entry carries no
                // filename target — `target` is the delimiter word.
                // GNU do_redirection_internal treats r_reading_until /
                // r_deblank_reading_until / r_reading_string like every
                // other input binding at ANY redirector fd:
                // here_document_to_fd (redir.c:423) places the body in
                // a temporary file and the resulting descriptor is
                // dup2'd onto the redirector (redir.c:1065-1072). The
                // spawn-side mirror of that contract: materialize the
                // canonical body text (external_fd_heredoc_input — the
                // same ladder the /dev/fd operand and <&N faces use)
                // into a carrier file, then let the ordinary input arm
                // below open it and deliver it through the same env /
                // pin channels as a file input redirect, so fd 3 and
                // fd 0's body (apply_external_stdin_redirect) obey one
                // installation contract. Without this arm the entry
                // fell into the write bucket and "opened" the delimiter
                // as an output file, leaving the child's fd unbound.
                let Some(body) = self.external_fd_heredoc_input(cmd, fd) else {
                    continue;
                };
                let Ok(carrier) = self.write_process_substitution_temp(&body) else {
                    continue;
                };
                self.shell_state
                    .procsub_adhoc_carriers
                    .borrow_mut()
                    .push(carrier.clone());
                target = carrier.display().to_string();
                write = false;
            }
            // rubash#2298: a `<>` entry classifies as input for the
            // stash lookup above, but its open is neither the input
            // ladder (O_RDONLY — the child then holds a read-only
            // description and a write through the slot fails EBADF)
            // nor the output ladder: GNU r_input_output opens
            // O_RDWR|O_CREAT (make_cmd.c:682; redir.c:725 redir_open)
            // and the single description serves both directions in
            // the child. probe_input_redirect_readwrite is that
            // classified readwrite open (the same ladder the builtin
            // faces probe `<>` with).
            let readwrite = redirect.kind == crate::parser::RedirectKind::ReadWrite;
            let mut opened_file: Option<std::fs::File> = None;
            let raw = if let Some(file) = &stashed {
                file.handle
            } else {
                let opened = if write {
                    if redirect.append {
                        open_redirect_append(&self.session_path(shell_path_to_windows(
                            &target,
                            &self.shell_state.env_vars,
                        )))
                    } else {
                        self.create_redirect_output(&target, redirect.clobber)
                            .map_err(|error| crate::posix_errors::path_error(&target, error))
                    }
                } else if readwrite {
                    self.probe_input_redirect_readwrite(&target)
                } else {
                    self.open_input_redirect(&target)
                };
                let Ok(file) = opened else {
                    continue;
                };
                #[cfg(windows)]
                let raw = {
                    use std::os::windows::io::AsRawHandle;
                    file.as_raw_handle() as crate::fd::HANDLE
                };
                #[cfg(unix)]
                let raw = {
                    use std::os::unix::io::AsRawFd;
                    file.as_raw_fd()
                };
                opened_file = Some(file);
                raw
            };
            let Ok(dup) = crate::fd::duplicate_handle_inheritable(raw) else {
                continue;
            };
            #[cfg(unix)]
            command_fd_pins.push((dup, fd));
            let path_label = match &stashed {
                Some(file) => file.path.to_string_lossy().into_owned(),
                None => target.clone(),
            };
            // rubash#2298: a self-opened `<>` description serves
            // both directions, so stamp BOTH env channels with the
            // one dup — the child's startup restore pairs equal
            // HANDLE/WHANDLE values into a single shared FileFd
            // (init.rs), and the pin above already delivers the
            // descriptor itself to foreign children. A stash-backed
            // entry keeps the read-channel stamp only: the stash is
            // an earlier entry's description, whose access mode is
            // that entry's own.
            let stamp_read = !write;
            let stamp_write = write || (readwrite && opened_file.is_some());
            if stamp_read {
                process.env(format!("__RUBASH_FD_HANDLE_{fd}"), format!("{dup:#x}"));
                process.env(format!("__RUBASH_FD_PATH_{fd}"), path_label.clone());
            }
            if stamp_write {
                process.env(format!("__RUBASH_FD_WHANDLE_{fd}"), format!("{dup:#x}"));
                process.env(format!("__RUBASH_FD_WPATH_{fd}"), path_label);
            }
            parent_side_dups.push(dup);
            // rubash#2272: remember the description delivered at this
            // slot for a later {var} dup in the same list (see
            // walk_opened above).
            let walk_file = match (&stashed, &opened_file) {
                (Some(file), _) => Some(file.clone()),
                (None, Some(file)) => file.try_clone().ok().map(|clone| {
                    crate::executor::fd_table::FileFd::from_file(clone, target.clone().into())
                }),
                (None, None) => None,
            };
            if let Some(file) = walk_file {
                walk_opened.insert(fd, file);
            }
            // The parent's own open was transient (GNU closes it after the
            // fork); the dup above is what the child holds.
            drop(opened_file);
        }
        // rubash#2281: closes whose position is the list's last
        // entry (or any defensive remainder) land now, before the
        // heredoc deliveries below and before the ambient pin
        // computation reads the table.
        while pending_close_idx < pending_closes.len() {
            let fd = pending_closes[pending_close_idx].1;
            pending_close_idx += 1;
            apply_pending_close_at(
                self,
                &mut process,
                &mut command_fd_pins,
                &mut walk_opened,
                &mut numbered_dup_bindings,
                fd,
            );
        }
        // rubash#2272: `{var}<<EOF` bindings live in
        // heredoc_redirects, not redirects; the applier installed
        // their bodies as Text endpoints in the fd table. Deliver
        // each at its var's number through the same carrier shape as
        // the redirects-list text arm (deliver_fd_var_redirect).
        for redirect in &cmd.heredoc_redirects {
            let Some(name) = redirect.fd_var.as_deref() else {
                continue;
            };
            let Some(var_fd) = self.dynamic_fd_variable_value(name) else {
                continue;
            };
            fd_var_claimed.insert(var_fd);
            self.deliver_fd_var_text_binding(
                var_fd,
                var_fd,
                &mut process,
                &mut command_fd_pins,
                &mut parent_side_dups,
            );
        }
        // rubash#1626: the env channel above only reaches rubash children
        // (init.rs restores the named handles at startup); a foreign child
        // — the /usr/bin/bash, cat, ls of the report — gets neither the
        // descriptor nor a usable number, and the table's handles are all
        // opened O_CLOEXEC, so nothing leaks through the raw fd space
        // either: every ambient fd bound by `exec N>file` / `exec N<file`
        // / `exec N<>file` fails EBADF there. GNU's fork simply inherits
        // the descriptors at their numbers. Mirror that on Unix with a
        // pre_exec pin (registered below); the pin table itself is
        // computed by ambient_fd_pins_for so hand-built pipeline stage
        // processes inherit the same descriptors (rubash#1688).
        #[cfg(unix)]
        let ambient_fd_pins: Vec<(std::os::unix::io::RawFd, u32)> = {
            let mut pins = self.ambient_fd_pins_for(cmd);
            // rubash#1686: the command's own redirect descriptors ride
            // the same registration, appended after the ambient pins so
            // a command binding wins any slot both sides name (the pin
            // helper dup2's in list order).
            pins.extend(command_fd_pins);
            pins
        };
        // rubash#1626: pin the ambient descriptors at their slot numbers
        // (see the pin table built above). Registered before the /dev/fd
        // operand attach below so a command-scoped operand pin — the
        // most local binding — wins any slot collision.
        #[cfg(unix)]
        if !ambient_fd_pins.is_empty() {
            use std::os::unix::process::CommandExt;
            let pins = ambient_fd_pins;
            unsafe {
                process.pre_exec(move || pin_ambient_fds_in_child(&pins));
            }
        }
        parent_side_dups
    }

    /// rubash#2272: the env + pin stamp the numbered walk's tail
    /// performs, factored for the {var} delivery arms: duplicate the
    /// source handle as inheritable, name it in the child's
    /// environment at `fd`, and (on Unix) record the pre_exec pin so
    /// a foreign child receives the raw descriptor at that number —
    /// the same two channels as every other spawn delivery.
    fn stamp_child_fd_handle(
        &mut self,
        process: &mut Command,
        command_fd_pins: &mut FdPinVec,
        parent_side_dups: &mut Vec<crate::fd::HANDLE>,
        fd: u32,
        write: bool,
        raw: crate::fd::HANDLE,
        path_label: String,
    ) {
        let Ok(dup) = crate::fd::duplicate_handle_inheritable(raw) else {
            return;
        };
        #[cfg(unix)]
        command_fd_pins.push((dup, fd));
        #[cfg(not(unix))]
        let _ = &*command_fd_pins;
        process.env(
            format!("__RUBASH_FD_{}HANDLE_{fd}", if write { "W" } else { "" }),
            format!("{dup:#x}"),
        );
        process.env(
            format!("__RUBASH_FD_{}PATH_{fd}", if write { "W" } else { "" }),
            path_label,
        );
        parent_side_dups.push(dup);
    }

    /// rubash#2272: deliver one `{var}` redirect's binding to the
    /// spawn. The dynamic-fd applier (trap_exec.rs
    /// apply_dynamic_fd_var_redirects) ran ahead of dispatch and
    /// bound the var's descriptor into the fd table — the mirror of
    /// GNU binding in the forked child (redir.c:237 do_redirections;
    /// the {var} assignment proper is redir_varassign, redir.c:1010
    /// and :1113). Delivery then splits by what the applier
    /// installed:
    ///
    /// - File kinds (`<`, `>`, `>>`, `<>`): a File endpoint in the
    ///   table — the ambient stamp/pin in apply_external_fd_delivery
    ///   already delivers it at the var's number. Nothing to add.
    /// - Close forms: the applier recorded the close with this
    ///   entry's position; the delivery walk applies it when the
    ///   walk passes that position (apply_pending_close_at), so the
    ///   ambient stamp is revoked for the slot and the child
    ///   inherits it closed, as GNU's child does. Nothing to add.
    /// - Heredoc / here-string: a Text endpoint — no OS descriptor
    ///   exists to stamp. Materialize the body into a carrier file
    ///   and deliver the carrier at the var's number (the #2270
    ///   shape for numbered heredocs).
    /// - Dup forms: the applier copied the source endpoint into the
    ///   var's slot. A File copy rides the ambient delivery, but a
    ///   text or stdio copy has no stampable handle, and a source
    ///   bound earlier in the command's OWN list was never visible
    ///   to the applier (it resolves against the ambient table
    ///   only). GNU walks ONE ordered list in the child
    ///   (redir.c:237), so deliver_fd_var_dup resolves the source
    ///   here — own list first, then the ambient table.
    fn deliver_fd_var_redirect(
        &mut self,
        cmd: &CommandNode,
        position: usize,
        redirect: &crate::parser::Redirect,
        process: &mut Command,
        command_fd_pins: &mut FdPinVec,
        parent_side_dups: &mut Vec<crate::fd::HANDLE>,
        walk_opened: &std::collections::HashMap<
            u32,
            std::rc::Rc<crate::executor::fd_table::FileFd>,
        >,
        fd_var_claimed: &mut std::collections::HashSet<u32>,
    ) {
        let Some(name) = redirect.fd_var.as_deref() else {
            return;
        };
        match redirect.kind {
            crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString => {
                if let Some(var_fd) = self.dynamic_fd_variable_value(name) {
                    fd_var_claimed.insert(var_fd);
                    self.deliver_fd_var_text_binding(
                        var_fd,
                        var_fd,
                        process,
                        command_fd_pins,
                        parent_side_dups,
                    );
                }
            }
            crate::parser::RedirectKind::DuplicateInput
            | crate::parser::RedirectKind::DuplicateOutput => {
                self.deliver_fd_var_dup(
                    cmd,
                    position,
                    redirect,
                    name,
                    process,
                    command_fd_pins,
                    parent_side_dups,
                    walk_opened,
                    fd_var_claimed,
                );
            }
            // File kinds and close forms ride the ambient delivery
            // (see the doc comment above).
            _ => {}
        }
    }

    /// rubash#2272: materialize the text body bound at
    /// `snapshot_fd` (a heredoc / here-string body the dynamic-fd
    /// applier stored as a Text endpoint) into a carrier file and
    /// stamp/pin the carrier at `deliver_fd` — the installation the
    /// numbered heredoc arm performs with
    /// external_fd_heredoc_input (#2270), except the body here
    /// comes from the fd table's snapshot: the applier already
    /// expanded it, and a partially consumed persistent binding
    /// delivers its remaining text, as a GNU descriptor positioned
    /// at the same offset would. The two numbers differ only for
    /// the numbered dup arm (#2278), which reads the snapshot at
    /// the dup's source and delivers at the dup's target.
    fn deliver_fd_var_text_binding(
        &mut self,
        snapshot_fd: u32,
        deliver_fd: u32,
        process: &mut Command,
        command_fd_pins: &mut FdPinVec,
        parent_side_dups: &mut Vec<crate::fd::HANDLE>,
    ) {
        let Some((text, offset)) = self.fd_table.input_snapshot(snapshot_fd) else {
            return;
        };
        let body = text.get(offset.min(text.len())..).unwrap_or("").to_string();
        let Ok(carrier) = self.write_process_substitution_temp(&body) else {
            return;
        };
        self.shell_state
            .procsub_adhoc_carriers
            .borrow_mut()
            .push(carrier.clone());
        let carrier = carrier.display().to_string();
        let Ok(file) = self.open_input_redirect(&carrier) else {
            return;
        };
        #[cfg(windows)]
        let raw = {
            use std::os::windows::io::AsRawHandle;
            file.as_raw_handle() as crate::fd::HANDLE
        };
        #[cfg(unix)]
        let raw = {
            use std::os::unix::io::AsRawFd;
            file.as_raw_fd()
        };
        self.stamp_child_fd_handle(
            process,
            command_fd_pins,
            parent_side_dups,
            deliver_fd,
            false,
            raw,
            carrier,
        );
    }

    /// rubash#2272: the descriptor number a `{var}` delivery lands
    /// on: the number the applier recorded in the var when it bound
    /// the entry. When the applier could not bind — its source
    /// resolution sees only the ambient table, and an own-list
    /// source is the case in point — allocate the number the way
    /// the applier's allocator does (fd_table.rs
    /// allocate_dynamic_with_limit): the lowest slot >= 10 free in
    /// the fd table, not claimed by the command's own numbered
    /// redirects, and not already claimed by an earlier {var}
    /// delivery in this walk. The parent variable itself stays
    /// untouched: GNU's assignment for an external command happens
    /// in the child and never reaches the parent (redir.c
    /// redir_varassign in the fork), so only the child's slot needs
    /// the number.
    fn fd_var_delivery_fd(
        &self,
        cmd: &CommandNode,
        name: &str,
        claimed: &mut std::collections::HashSet<u32>,
    ) -> u32 {
        if let Some(fd) = self.dynamic_fd_variable_value(name) {
            claimed.insert(fd);
            return fd;
        }
        let mut candidate = 10u32;
        loop {
            let taken = claimed.contains(&candidate)
                || cmd
                    .redirects
                    .iter()
                    .any(|entry| entry.fd == Some(candidate))
                || cmd
                    .heredoc_redirects
                    .iter()
                    .any(|entry| entry.fd == Some(candidate))
                || self.fd_table.entries.get(&candidate).is_some_and(|entry| {
                    !entry.closed && (entry.read.is_some() || entry.write.is_some())
                });
            if !taken {
                claimed.insert(candidate);
                return candidate;
            }
            candidate += 1;
        }
    }

    /// rubash#2272: deliver a `{var}` dup entry (`{v}<&N`, `{v}>&N`,
    /// and the move spellings — GNU's child observes a move as a
    /// copy plus a close of the source, and the close rides
    /// fd_var_pending_closes). Resolution order mirrors GNU's
    /// single ordered walk (redir.c:237 do_redirections):
    /// (1) a binding this walk already delivered for the source,
    /// (2) the fd-0 replay for the command's own stdin binding,
    /// (3) the applier's ambient copy when it is a File endpoint
    /// (already delivered by the ambient stamp), (4) the applier's
    /// copy when it is a text snapshot (carrier), (5) a stdio
    /// source, pinned child-locally.
    #[allow(clippy::too_many_arguments)]
    fn deliver_fd_var_dup(
        &mut self,
        cmd: &CommandNode,
        position: usize,
        redirect: &crate::parser::Redirect,
        name: &str,
        process: &mut Command,
        command_fd_pins: &mut FdPinVec,
        parent_side_dups: &mut Vec<crate::fd::HANDLE>,
        walk_opened: &std::collections::HashMap<
            u32,
            std::rc::Rc<crate::executor::fd_table::FileFd>,
        >,
        fd_var_claimed: &mut std::collections::HashSet<u32>,
    ) {
        let target = self.expand_redirect_target(redirect);
        let Some((source_fd, _move_source)) = redirect_target_fd_and_move(&target) else {
            return;
        };
        let write = matches!(redirect.kind, crate::parser::RedirectKind::DuplicateOutput);
        // (1) The command's own earlier binding for the source.
        if let Some(file) = walk_opened.get(&source_fd).cloned() {
            let var_fd = self.fd_var_delivery_fd(cmd, name, fd_var_claimed);
            let path = file.path.to_string_lossy().into_owned();
            self.stamp_child_fd_handle(
                process,
                command_fd_pins,
                parent_side_dups,
                var_fd,
                write,
                file.handle,
                path,
            );
            return;
        }
        // (2) fd-0 sources: fd-0 entries never enter this walk (the
        // stdin face owns them), so resolve the command's own stdin
        // binding through the #2235 replay.
        if !write && source_fd == 0 {
            if let Some(file) = self.own_input_binding_before(cmd, 0, position) {
                let var_fd = self.fd_var_delivery_fd(cmd, name, fd_var_claimed);
                let path = file.path.to_string_lossy().into_owned();
                self.stamp_child_fd_handle(
                    process,
                    command_fd_pins,
                    parent_side_dups,
                    var_fd,
                    false,
                    file.handle,
                    path,
                );
                return;
            }
        }
        let Some(var_fd) = self.dynamic_fd_variable_value(name) else {
            // The applier bound nothing and no own binding resolves
            // the source: the dup already failed with GNU's
            // diagnostic on the preflight face (the dup-before-bind
            // shapes) — there is nothing to deliver.
            return;
        };
        fd_var_claimed.insert(var_fd);
        // (3) The applier's copy is a File endpoint: the ambient
        // stamp already delivers it at the var's number.
        let file_delivered = if write {
            matches!(
                self.fd_table.write_endpoint(var_fd),
                Some(crate::executor::fd_table::FdWriteEndpoint::File(_))
            )
        } else {
            matches!(
                self.fd_table.read_endpoint(var_fd),
                Some(crate::executor::fd_table::FdReadEndpoint::File(_))
            )
        };
        if file_delivered {
            return;
        }
        // (4) The applier's copy is a text snapshot (a heredoc
        // body, a materialized stdin): deliver its remaining bytes
        // through a carrier, like the heredoc arm.
        if !write && self.fd_table.input_snapshot(var_fd).is_some() {
            self.deliver_fd_var_text_binding(
                var_fd,
                var_fd,
                process,
                command_fd_pins,
                parent_side_dups,
            );
            return;
        }
        // (5) A stdio source: the description the child must share
        // is its OWN wired stdio slot, which exists only in the
        // child — pin the slot number across the fork.
        // std::process arranges the child's stdio before the
        // pre_exec callbacks run, and the pin helper stages sources
        // above the pin ceiling before dup2'ing, so the pin copies
        // the wired slot (the command's redirected stdin/stdout),
        // never the parent's, and never clobbers the slot itself.
        if source_fd <= 2 {
            #[cfg(unix)]
            command_fd_pins.push((source_fd as std::os::unix::io::RawFd, var_fd));
            #[cfg(not(unix))]
            let _ = &*command_fd_pins;
        }
    }

    /// rubash#2278: deliver one NUMBERED dup entry (`3<&0`,
    /// `4>&3`, `3>&1`) to the spawn. The numbered walk skipped dup
    /// spellings outright, so the child's slot stayed unbound
    /// (EBADF) even though GNU's single ordered walk services
    /// every dup with a dup2 of the source as bound at that walk
    /// position (redir.c:237 do_redirections; redir.c:1138
    /// do_redirection_internal, dup2 at :1193). Resolution order
    /// mirrors deliver_fd_var_dup's ladder, adapted to a walk with
    /// no dynamic-fd applier: (1) a binding this walk already
    /// delivered for the source slot, (2) the fd-0 replay for the
    /// command's own stdin binding, (3) the ambient fd table's
    /// endpoint at the source (a File endpoint stamps/pins
    /// directly; a text snapshot rides a carrier), (4) a stdio
    /// source — pinned child-locally when no later entry rebinds
    /// the source slot (the wired slot then IS the at-position
    /// binding), or captured parent-side from the shell's ambient
    /// stdio descriptor when a later entry does rebind it
    /// (`3<&0 <seed` must deliver the AMBIENT stdin, not the seed
    /// the later entry wires into the child's fd 0). An
    /// unresolvable source delivers nothing: a closed source was
    /// already diagnosed on the redirect preflight face, exactly
    /// as in the {var} ladder.
    #[allow(clippy::too_many_arguments)]
    fn deliver_numbered_dup(
        &mut self,
        cmd: &CommandNode,
        position: usize,
        redirect: &crate::parser::Redirect,
        fd: u32,
        source_fd: u32,
        process: &mut Command,
        command_fd_pins: &mut FdPinVec,
        parent_side_dups: &mut Vec<crate::fd::HANDLE>,
        walk_opened: &mut std::collections::HashMap<
            u32,
            std::rc::Rc<crate::executor::fd_table::FileFd>,
        >,
        numbered_dup_bindings: &mut std::collections::HashMap<u32, NumberedDupBinding>,
        stage_stdio: Option<&StageStdioContext>,
    ) {
        let write = matches!(redirect.kind, crate::parser::RedirectKind::DuplicateOutput);
        // (1) A binding this walk already delivered for the source.
        if let Some(file) = walk_opened.get(&source_fd).cloned() {
            let path = file.path.to_string_lossy().into_owned();
            self.stamp_child_fd_handle(
                process,
                command_fd_pins,
                parent_side_dups,
                fd,
                write,
                file.handle,
                path,
            );
            walk_opened.insert(fd, file);
            return;
        }
        if let Some(binding) = numbered_dup_bindings.get(&source_fd).copied() {
            #[cfg(unix)]
            {
                let source = match binding {
                    NumberedDupBinding::ChildSlot(slot) => slot as std::os::unix::io::RawFd,
                    NumberedDupBinding::Handle(handle) => handle,
                };
                command_fd_pins.push((source, fd));
            }
            #[cfg(not(unix))]
            let _ = &*command_fd_pins;
            numbered_dup_bindings.insert(fd, binding);
            return;
        }
        // (2) fd-0 sources: fd-0 entries never enter this walk
        // (the stdin face owns them), so resolve the command's own
        // stdin binding through the #2235 replay, as the {var}
        // ladder's step (2) does.
        if !write && source_fd == 0 {
            if let Some(file) = self.own_input_binding_before(cmd, 0, position) {
                let path = file.path.to_string_lossy().into_owned();
                self.stamp_child_fd_handle(
                    process,
                    command_fd_pins,
                    parent_side_dups,
                    fd,
                    false,
                    file.handle,
                    path,
                );
                walk_opened.insert(fd, file);
                return;
            }
        }
        // (3) The ambient fd table's endpoint at the source.
        let table_file = if write {
            match self.fd_table.write_endpoint(source_fd) {
                Some(crate::executor::fd_table::FdWriteEndpoint::File(file)) => Some(file),
                Some(crate::executor::fd_table::FdWriteEndpoint::CoprocStdin {
                    fd: file, ..
                }) => Some(file),
                _ => None,
            }
        } else {
            match self.fd_table.read_endpoint(source_fd) {
                Some(crate::executor::fd_table::FdReadEndpoint::File(file)) => Some(file),
                Some(crate::executor::fd_table::FdReadEndpoint::CoprocStdout {
                    fd: file, ..
                }) => Some(file),
                _ => None,
            }
        };
        if let Some(file) = table_file {
            let path = file.path.to_string_lossy().into_owned();
            self.stamp_child_fd_handle(
                process,
                command_fd_pins,
                parent_side_dups,
                fd,
                write,
                file.handle,
                path,
            );
            walk_opened.insert(fd, file);
            return;
        }
        if !write && self.fd_table.input_snapshot(source_fd).is_some() {
            // A text endpoint (a heredoc body, a materialized
            // stdin): deliver its remaining bytes through a
            // carrier, like the {var} ladder's step (4).
            self.deliver_fd_var_text_binding(
                source_fd,
                fd,
                process,
                command_fd_pins,
                parent_side_dups,
            );
            return;
        }
        // (4) A stdio source: the binding at this position is the
        // shell's ambient stdio, or an own-list binding of the
        // slot that the stdio faces already wired (an earlier
        // `>file` for a `3>&1`) — both are exactly what the
        // child's wired slot holds UNLESS a later entry rebinds
        // the slot, in which case the ambient descriptor must be
        // captured parent-side before the spawn rewires it.
        if source_fd <= 2 {
            if self.fd_table.is_closed(source_fd) {
                return;
            }
            let rebound_later = cmd.redirects.iter().enumerate().any(|(index, entry)| {
                index > position
                    && entry.fd_var.is_none()
                    && entry
                        .fd
                        .unwrap_or(if entry.is_input_side() { 0 } else { 1 })
                        == source_fd
            });
            if rebound_later {
                // rubash#2293: on the pipeline stage face the
                // at-position binding is the stage channel, not the
                // shell's ambient descriptor — GNU binds the
                // element's stdio to the inter-element pipes before
                // the element's own walk runs (execute_cmd.c:2620
                // execute_pipeline). Fd 0 is wired to the later
                // WINNER by the stdin face, so its at-position
                // binding is the payload channel the spawner
                // materialized as the context's stdin_channel; when
                // the stage's fd 0 is the live inherited descriptor
                // the channel is None and the ambient capture below
                // already names it. Fds 1/2 are wired to the stage's
                // capture pipes, but the routing walk places a
                // slot's captured payload WHOLE at the slot's final
                // target — a dup sharing fd 1's capture would land
                // in the file `1>f.txt` installs, not on the stage
                // pipe the dup copied. When the spawner materialized
                // a carrier for this dup (StageStdioContext::
                // dup_channels), pinning the carrier's write end is
                // the delivery; the child-local pin below remains
                // for stages the spawner served without a carrier.
                if let Some(stage) = stage_stdio {
                    if source_fd == 0 {
                        if let Some(channel) = stage.stdin_channel {
                            #[cfg(unix)]
                            command_fd_pins.push((channel, fd));
                            #[cfg(not(unix))]
                            let _ = &*command_fd_pins;
                            numbered_dup_bindings.insert(fd, NumberedDupBinding::Handle(channel));
                            return;
                        }
                    } else if let Some(channel) = stage
                        .dup_channels
                        .iter()
                        .find(|(target_fd, source, _)| *target_fd == fd && *source == source_fd)
                        .map(|(_, _, handle)| *handle)
                    {
                        #[cfg(unix)]
                        command_fd_pins.push((channel, fd));
                        #[cfg(not(unix))]
                        let _ = &*command_fd_pins;
                        numbered_dup_bindings.insert(fd, NumberedDupBinding::Handle(channel));
                        return;
                    } else {
                        #[cfg(unix)]
                        command_fd_pins.push((source_fd as std::os::unix::io::RawFd, fd));
                        #[cfg(not(unix))]
                        let _ = &*command_fd_pins;
                        numbered_dup_bindings.insert(fd, NumberedDupBinding::ChildSlot(source_fd));
                        return;
                    }
                }
                #[cfg(unix)]
                if let Ok(dup) =
                    crate::fd::duplicate_handle_inheritable(source_fd as std::os::unix::io::RawFd)
                {
                    command_fd_pins.push((dup, fd));
                    parent_side_dups.push(dup);
                    numbered_dup_bindings.insert(fd, NumberedDupBinding::Handle(dup));
                    return;
                }
                // The ambient descriptor could not be captured
                // (or the platform has no raw stdio handle to
                // capture): fall through to the child-local pin,
                // which at least delivers the slot's final binding.
            }
            #[cfg(unix)]
            command_fd_pins.push((source_fd as std::os::unix::io::RawFd, fd));
            #[cfg(not(unix))]
            let _ = &*command_fd_pins;
            numbered_dup_bindings.insert(fd, NumberedDupBinding::ChildSlot(source_fd));
        }
    }

    fn spawn_external_process(
        &mut self,
        cmd: &CommandNode,
        program: &PathBuf,
        mut process: Command,
        used_shell: bool,
        dev_ops: crate::executor::dev_fd_operands::DevOperandMaterialization,
        // rubash#1832: true when the caller composed the process
        // environment to finality itself (the `env` emulation — its
        // composition already folded the tail SHLVL adjustment in and
        // applied env's own -i/-u/assignments). The exec-replace seam
        // must not stamp SHLVL over such an environment after the
        // fact; the direct disk-command path composes from shell
        // state and passes false.
        env_composed_final: bool,
        // niubash#567: the stdin text a declined route attempt
        // already pulled with the feed's own pull (route_stdin_handle),
        // when the stdin source was a text buffer. The feed below
        // uses it instead of pulling again, so the body's embedded
        // substitutions run exactly once on either route.
        pre_materialized_stdin: Option<String>,
    ) -> Result<(), ExecuteError> {
        // rubash#2255: the fd delivery (ambient table stamps, the
        // command's own numbered redirects, the unix pin
        // registration) is shared with the sequential pipeline stage
        // spawner — see apply_external_fd_delivery.
        let parent_side_dups = self.apply_external_fd_delivery(cmd, &mut process, None);
        // rubash#2398: the job-control channel (see job_control_wait)
        // claims a plain interactive foreground spawn end to end —
        // the child leads its own process group (pre_exec below), the
        // parent repeats the setpgid after the fork (jobs.c:2455),
        // and the wait below registers the job and hands it the
        // terminal. Capture-shaped spawns keep the existing waits:
        // their readers own the child's output, and GNU's comsub
        // children share the shell's group (no WUNTRACED wait,
        // jobs.c:4077 subshell_environment condition).
        #[cfg(unix)]
        let fg_job_control = self.job_control_channel_active()
            && !matches!(
                self.external_stdio_outcome,
                Some(super::external_redirects::ExternalStdioOutcome::CapturePipe(..))
            )
            && !self.external_needs_fd_copy_capture(cmd);
        // rubash#229: restore GNU's child signal dispositions between fork
        // and exec (trap.c:1489 restore_original_signals) — without this a
        // policy/trap SIG_IGN in the shell survives exec and makes children
        // unkillable by that signal (sh -c 'kill -QUIT $$' exited 0).
        #[cfg(unix)]
        unsafe {
            use std::os::unix::process::CommandExt;
            process.pre_exec(crate::builtins::kill::child_pre_exec_reset);
        }
        // rubash#2398: a job-control foreground child leads its own
        // process group (jobs.c:2390 make_child setpgid; a one-process
        // job's pipeline_pgrp is the child's own pid, jobs.c:2446) so
        // the terminal can be handed to its group and a ^Z stops the
        // child alone instead of the shell's whole group. Registered
        // after the signal reset; both run in the child before exec.
        #[cfg(unix)]
        if fg_job_control {
            unsafe {
                use std::os::unix::process::CommandExt;
                process.pre_exec(crate::builtins::kill::child_pre_exec_setpgid);
            }
        }

        // rubash#438: pin the buffered process-substitution pipe read ends at
        // their /dev/fd/N numbers in the child (fork-exec dup2), so the
        // literal argv word resolves through the OS /dev/fd layer like a
        // GNU fork.
        #[cfg(unix)]
        crate::executor::dev_fd_operands::attach_unix_dev_fd_operands(&mut process, &dev_ops);
        suppress_orphan_console(&mut process);
        // rubash#1497: GNU's nofork path (builtins/evalstring.c:126
        // should_suppress_fork, execute_cmd.c:5813
        // execute_disk_command): the last plain command of a -c string
        // REPLACES the shell process instead of forking. The Command
        // is fully prepared here (environment, stdio, fd channel,
        // pre_exec signal reset and /dev/fd pins), so exec it in
        // place: std's Command::exec runs the pre_exec closures
        // in-process, then execve; it returns only on failure, where
        // GNU's model has already committed — the shell diagnoses and
        // exits with the classified status rather than forking.
        #[cfg(unix)]
        if !used_shell && self.exec_replace_tail_ready(cmd, program, &dev_ops) {
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            // rubash#2179: the image is about to be replaced; any
            // output-procsub feed still in flight would die with it.
            // Complete the transfers first (the consumers themselves
            // keep running, as GNU's do across this boundary).
            self.drain_procsub_feeds();
            // rubash#1832: the direct path composed this process
            // environment from the shell state, whose SHLVL is the
            // live level — stamp the adjusted one (GNU's
            // adjust_shell_level(-1) before maybe_make_export_env,
            // execute_cmd.c:5813-5817). An env-emulation process
            // (env_composed_final) already carries the composition's
            // final value: stamping here would resurrect a variable
            // env's -i/-u deliberately removed or supersede an
            // explicit assignment.
            if !env_composed_final {
                process.env("SHLVL", self.exec_replace_adjusted_shlvl());
            }
            use std::os::unix::process::CommandExt;
            let error = process.exec();
            let _ = self.report_external_spawn_error(cmd, program, error);
            std::process::exit(self.exit_code);
        }
        // rubash#1035: GNU shell_execve (execute_cmd.c:6126) execs the
        // program directly and only consults the file's first bytes when
        // the kernel refuses (ENOEXEC). Rust's spawn (posix_spawn) never
        // surfaces that refusal for scripts without `#!`: libc runs them
        // under /bin/sh instead — the WRONG shell (no `trap -p`, wrong
        // traps, wrong POSIX mode). Pre-empt the spawn for files the
        // kernel could not exec and feed the refusal classifier below a
        // synthetic ENOEXEC, so the ENOEXEC shell-script fallback (or the
        // bad-interpreter/binary refusal) applies exactly as if execve
        // had refused. The 4-byte read costs microseconds against the
        // millisecond spawn; `used_shell` spawns already run under a shell
        // and skip it. Unix-only: Windows routes scripts through the shell
        // fallback before reaching any spawn (should_run_with_shell), so a
        // magic sniff here would only misroute batch files and MZ images.
        // The synthetic refusal must be ENOEXEC specifically: a file
        // without any execute bit would fail execve with EACCES instead,
        // and GNU reports that as Permission denied / 126 with no
        // shell-script fallback (execute_cmd.c:6126-6159 shell_execve;
        // rubash#1459). Non-executable files therefore take the real
        // spawn so the genuine EACCES reaches the diagnostic path.
        let spawned = if !used_shell
            && cfg!(unix)
            && !crate::executor::path::program_execs_directly(program)
            && crate::executor::path::is_executable_file(program)
        {
            Err(std::io::Error::from_raw_os_error(8))
        } else {
            process.spawn()
        };
        // Drop the Command NOW: std retains every `Stdio::from(..)` it was
        // given (the #144 shared capture-pipe write clones among them), so a
        // still-live Command keeps the pipe's write end open in the parent
        // and the post-exit drain below would block on an EOF that cannot
        // arrive even after the child is gone. The child already holds its
        // own duplicated inheritable handles; GNU closes the parent's copies
        // right after the fork (redir.c do_redirection_internal runs in the
        // child).
        drop(process);
        match spawned {
            Ok(mut child) => {
                adopt_orphan_console(child.id());
                // rubash#2398: jobs.c:2455 — the parent repeats the
                // setpgid after the fork, closing the race where the
                // terminal handoff (or a group signal) targets the
                // child's new group before the child's own pre_exec
                // setpgid has run.
                #[cfg(unix)]
                if fg_job_control {
                    crate::builtins::kill::group_spawned_child(child.id());
                }
                for dup in parent_side_dups {
                    crate::fd::close_handle(dup);
                }
                // rubash#2094: when the stdio plan left a capture pipe to
                // collect, start draining it NOW — before the stdin feed
                // below and before the child wait. The feed is a
                // synchronous write_all of the whole payload; once the
                // payload and the child's output each exceed the pipe
                // buffer, the child stops reading (its output pipe is
                // full), the feed blocks mid-payload, and a drain that
                // only starts after the feed never runs — a three-way
                // deadlock (`out=$(dl | command sed …)` with ~128 KiB in
                // flight; measured hang threshold between 135,000 and
                // 140,000 bytes of payload). GNU forks, so its drain is
                // concurrent by construction: subst.c:7143
                // command_substitute dups the capture pipe into the
                // child, and the parent reads it (subst.c:7437
                // read_comsub) BEFORE waiting (subst.c:7441 wait_for).
                // This is the same discipline niubash#158 applied to the
                // concurrent pipeline members (drain the last member
                // before waiting on the producers). A non-capture outcome
                // is put back untouched for the handling below.
                let capture_drain = match self.external_stdio_outcome.take() {
                    Some(super::external_redirects::ExternalStdioOutcome::CapturePipe(
                        reader,
                        capture_generation,
                    )) => Some((
                        std::thread::spawn(move || {
                            use std::io::Read;
                            let mut reader = reader;
                            let mut merged = Vec::new();
                            reader.read_to_end(&mut merged).map(|_| merged)
                        }),
                        capture_generation,
                    )),
                    other => {
                        self.external_stdio_outcome = other;
                        None
                    }
                };
                // Only materialize the virtual stdin text when the child
                // actually owns a pipe — `<&N` on a real file handle uses
                // Stdio::from(dup) (no child.stdin to take), and draining
                // the fd to build a dropped string would corrupt the shared
                // offset.
                if let Some(mut stdin) = child.stdin.take() {
                    if let Some(input) =
                        pre_materialized_stdin.or_else(|| self.stdin_string_for_command_mut(cmd))
                    {
                        // The input is shell text: raw-byte marker pairs
                        // (heredoc bodies, expanded bytes) must decode to
                        // real bytes for the child (GNU writes fd bytes).
                        // The feed itself must never fail the command or
                        // kill the shell when the child exits without
                        // reading (rubash#1822) — see feed_child_stdin.
                        crate::executor::shell_options::feed_child_stdin(
                            &mut stdin,
                            &crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                                &input,
                            ),
                        )?;
                    }
                }

                if let Some((drain, capture_generation)) = capture_drain {
                    // rubash#2094: the capture drain has been running on
                    // its thread since before the stdin feed; the child
                    // wait, the signal-death note, the SIGCHLD trap and
                    // the generation-addressed routing keep exactly the
                    // order the serial drain produced (drain -> wait ->
                    // note -> trap -> route is observationally wait ->
                    // note -> trap -> route once the drain overlaps the
                    // feed). EOF follows the child's exit — the parent's
                    // own write-end copies dropped with the Command — so
                    // the join only collects the drained bytes.
                    let foreground_pid = child.id();
                    let status = self.wait_external_child(&mut child)?;
                    if let Some(signal) = crate::executor::wait_status::exit_status_signal(&status)
                    {
                        self.note_child_signal_death(
                            foreground_pid,
                            signal,
                            self.signal_notice_command_text(cmd),
                            true,
                        );
                    }
                    self.run_sigchld_trap_for_reaped_child()?;
                    let merged = drain.join().map_err(|_| {
                        ExecuteError::IoError(std::io::Error::other(
                            "stdout capture drain thread panicked",
                        ))
                    })??;
                    super::shell_options::write_stdout_capture_at_generation(
                        &merged,
                        capture_generation,
                    )
                    .map_err(ExecuteError::IoError)?;
                    self.exit_code = crate::executor::wait_status::process_exit_status(&status);
                } else if let Some(outcome) = self.external_stdio_outcome.take() {
                    // niubash#144: the stdio plan already gave the child real
                    // handles; only the shared capture pipe needs collecting.
                    // Drain it to EOF first — the merged bytes stream in true
                    // write order through the ONE pipe both fds dup onto
                    // (GNU redir.c:1169-1170), unlike the retired two-pipe
                    // capture-replay.
                    let foreground_pid = child.id();
                    if let super::external_redirects::ExternalStdioOutcome::CapturePipe(
                        mut reader,
                        capture_generation,
                    ) = outcome
                    {
                        // Fallback for an outcome re-armed after the
                        // drain start above (no producer does so today —
                        // CapturePipe outcomes are claimed by capture_drain
                        // before the feed). Kept so this branch stays
                        // total over the outcome enum.
                        use std::io::Read;
                        let mut merged = Vec::new();
                        reader.read_to_end(&mut merged)?;
                        let status = self.wait_external_child(&mut child)?;
                        if let Some(signal) =
                            crate::executor::wait_status::exit_status_signal(&status)
                        {
                            self.note_child_signal_death(
                                foreground_pid,
                                signal,
                                self.signal_notice_command_text(cmd),
                                true,
                            );
                        }
                        self.run_sigchld_trap_for_reaped_child()?;
                        // Route the merged stream into the capture the
                        // plan materialized the pipe for — addressed by
                        // its recorded generation, NOT through fd 1's
                        // current endpoint: a function-call wrap may have
                        // rebound fd 1 since the plan (rubash#2031:
                        // `$(f 2>&1 >/dev/null)`, f's external leaf wrote
                        // its stderr into the capture pipe, and resolving
                        // the drain through fd 1 landed the bytes in the
                        // /dev/null target file instead of the
                        // substitution's capture).
                        super::shell_options::write_stdout_capture_at_generation(
                            &merged,
                            capture_generation,
                        )
                        .map_err(ExecuteError::IoError)?;
                        self.exit_code = crate::executor::wait_status::process_exit_status(&status);
                    } else {
                        // rubash#2398: the LiveHandles outcome is the
                        // plain interactive shape (the stdio plan arms
                        // it for every planned spawn) — the job-control
                        // wait claims it exactly like the no-outcome
                        // plain branch below; only CapturePipe outcomes
                        // stay on the existing wait (see fg_job_control).
                        #[cfg(unix)]
                        let wait_result = if fg_job_control {
                            self.wait_fg_child_job_control(
                                child,
                                self.signal_notice_command_text(cmd),
                                bash_job_stage_commands(cmd),
                            )
                        } else {
                            self.wait_external_child(&mut child)
                        };
                        #[cfg(not(unix))]
                        let wait_result = self.wait_external_child(&mut child);
                        let status = wait_result?;
                        if let Some(signal) =
                            crate::executor::wait_status::exit_status_signal(&status)
                        {
                            self.note_child_signal_death(
                                foreground_pid,
                                signal,
                                self.signal_notice_command_text(cmd),
                                true,
                            );
                        }
                        self.exit_code = crate::executor::wait_status::process_exit_status(&status);
                        // A reaped foreground child delivers SIGCHLD in
                        // GNU bash; a set trap runs once at this boundary
                        // (trap8.sub).
                        self.run_sigchld_trap_for_reaped_child()?;
                    }
                } else if self.external_needs_fd_copy_capture(cmd) {
                    let foreground_pid = child.id();
                    self.note_foreground_wait();
                    match child.wait_with_output() {
                        Ok(output) => {
                            self.exit_code = 0;
                            // rubash#229: a foreground child killed by a
                            // signal gets the jobs.c notify_of_job_status
                            // stderr notice at the reader boundary.
                            if let Some(signal) =
                                crate::executor::wait_status::exit_status_signal(&output.status)
                            {
                                self.note_child_signal_death(
                                    foreground_pid,
                                    signal,
                                    self.signal_notice_command_text(cmd),
                                    true,
                                );
                            }
                            // A reaped foreground child delivers SIGCHLD in
                            // GNU bash; a set trap runs once at this
                            // boundary (trap8.sub).
                            self.run_sigchld_trap_for_reaped_child()?;
                            self.write_external_captured_streams(
                                cmd,
                                &output.stdout,
                                &output.stderr,
                            )?;
                            if self.exit_code == 0 {
                                self.exit_code = crate::executor::wait_status::process_exit_status(
                                    &output.status,
                                );
                            }
                        }
                        Err(error) => self.report_external_spawn_error(cmd, program, error)?,
                    }
                } else {
                    let foreground_pid = child.id();
                    // rubash#2398: with the job-control channel live,
                    // the wait is the WUNTRACED one — it registers the
                    // job, hands over the terminal, and returns 128 +
                    // stop-signal for a stopped child (jobs.c:3243),
                    // parking the child handle in background_children
                    // for fg/bg/wait. The post-wait handling below
                    // (signal-death note, exit code, SIGCHLD trap) is
                    // shared: a stopped status carries no signal, so
                    // only the exit code moves.
                    #[cfg(unix)]
                    let wait_result = if fg_job_control {
                        self.wait_fg_child_job_control(
                            child,
                            self.signal_notice_command_text(cmd),
                            bash_job_stage_commands(cmd),
                        )
                    } else {
                        self.wait_external_child(&mut child)
                    };
                    #[cfg(not(unix))]
                    let wait_result = self.wait_external_child(&mut child);
                    match wait_result {
                        Ok(status) => {
                            // rubash#229: queue the signal-death notice for
                            // the reader boundary.
                            if let Some(signal) =
                                crate::executor::wait_status::exit_status_signal(&status)
                            {
                                self.note_child_signal_death(
                                    foreground_pid,
                                    signal,
                                    self.signal_notice_command_text(cmd),
                                    true,
                                );
                            }
                            self.exit_code =
                                crate::executor::wait_status::process_exit_status(&status);
                            // A reaped foreground child delivers SIGCHLD in
                            // GNU bash; a set trap runs once at this boundary
                            // (trap8.sub).
                            self.run_sigchld_trap_for_reaped_child()?;
                        }
                        Err(ExecuteError::IoError(error)) => {
                            self.report_external_spawn_error(cmd, program, error)?
                        }
                        // An untrapped terminating signal observed mid-wait:
                        // GNU's SIG_DFL kills the shell while it is still
                        // inside wait_for, so the 128+sig exit propagates
                        // instead of being reported as a spawn failure.
                        Err(error) => return Err(error),
                    }
                }
                // /dev/fd operands materialized as temps resolve now: write
                // endpoints flush into their target and consumed input fds
                // advance (GNU dup shares the fd offset).
                self.finish_dev_fd_operands(dev_ops);
            }
            Err(error) => {
                for dup in parent_side_dups {
                    crate::fd::close_handle(dup);
                }
                // The failed spawn never consumed the stdio plan; drop it so
                // no later command mistakes it for its own.
                self.external_stdio_outcome = None;
                if !used_shell && is_exec_format_error(&error) {
                    // GNU execute_cmd.c:6139-6233 (shell_execve): when the OS
                    // refuses to exec a file, its first bytes decide. A #!
                    // interpreter specifier whose interpreter cannot be
                    // resolved is refused ("bad interpreter", EX_NOEXEC =
                    // 126) instead of falling back to shell-script
                    // execution; a binary file (NUL in the first line, ELF
                    // magic) is refused as "cannot execute binary file"
                    // (EX_BINARY_FILE = 126).
                    if let Some((diagnostic, _)) = self.exec_format_refusal(cmd, program) {
                        let mut stderr = Vec::new();
                        let _ = writeln!(&mut stderr, "{diagnostic}");
                        self.finish_external_error(cmd, &stderr, 126)?;
                        return Ok(());
                    }
                    // GNU execute_cmd.c:6237-6260: a text file the kernel
                    // refuses (ENOEXEC) is executed by the forked child as
                    // THIS shell in-process (args[0] = shell_name,
                    // sh_longjmp subshell_top_level -> shell.c:429-464
                    // shell_reinitialize) — a fresh shell with exported-env
                    // only and kernel-preserved SIG_IGN dispositions, NOT
                    // /bin/sh (trap2.sub's ERR trap and trap1.sub's
                    // `trap -p` must run under bash semantics). The Windows
                    // mailbox keeps the find_shell spawn: it cannot observe
                    // a real execve refusal for extensionless text files.
                    if cfg!(unix) {
                        // rubash#2345: on a rooted, opted-in executor
                        // the re-entry is a real spawn of this binary
                        // on the script — the forked-child boundary
                        // itself — prepared exactly like the
                        // find_shell spawn below (same cwd, env, and
                        // redirect application; the script argument
                        // carries the re-entry spelling so the child's
                        // $0/BASH_SOURCE match GNU's recorded name).
                        // Hosts and clones keep the in-process entry.
                        if self.real_shell_children_apply() {
                            if let (Ok(exe), Some(spelling)) = (
                                std::env::current_exe(),
                                self.enoexec_script_spelling(cmd, program),
                            ) {
                                let mut exe_process = Command::new(exe);
                                exe_process.arg(spelling);
                                exe_process.args(&cmd.words[1..]);
                                // G-02: children are born in the session cwd.
                                exe_process.current_dir(&self.shell_state.cwd);
                                self.apply_external_environment(cmd, &mut exe_process, program);
                                self.apply_external_redirects(cmd, &mut exe_process)?;
                                return self.spawn_external_process(
                                    cmd,
                                    program,
                                    exe_process,
                                    true,
                                    dev_ops,
                                    env_composed_final,
                                    pre_materialized_stdin,
                                );
                            }
                        }
                        self.execute_enoexec_shell_script(cmd, program)?;
                        return Ok(());
                    }
                    if let Some(shell) = find_shell(&self.shell_state.env_vars) {
                        let mut shell_process = Command::new(shell);
                        shell_process.arg(program);
                        shell_process.args(&cmd.words[1..]);
                        // G-02: children are born in the session cwd.
                        shell_process.current_dir(&self.shell_state.cwd);
                        self.apply_external_environment(cmd, &mut shell_process, program);
                        self.apply_external_redirects(cmd, &mut shell_process)?;
                        return self.spawn_external_process(
                            cmd,
                            program,
                            shell_process,
                            true,
                            dev_ops,
                            env_composed_final,
                            pre_materialized_stdin,
                        );
                    }
                }
                self.report_external_spawn_error(cmd, program, error)?;
            }
        }

        Ok(())
    }

    /// GNU execute_cmd.c:6139-6233 (shell_execve + execute_shell_script):
    /// classify a file the OS refused to exec. Returns the refusal
    /// diagnostic and exit status when the file must not fall back to
    /// shell-script execution: an unresolvable #! interpreter ("bad
    /// interpreter", EX_NOEXEC) or a binary first line ("cannot execute
    /// binary file", EX_BINARY_FILE). None means the ENOEXEC fallback
    /// (running the file as a shell script) may proceed.
    pub(in crate::executor) fn exec_format_refusal(
        &self,
        cmd: &CommandNode,
        program: &PathBuf,
    ) -> Option<(String, i32)> {
        let sample = std::fs::read(program).ok()?;
        if sample.len() >= 2 && sample[0] == b'#' && sample[1] == b'!' {
            let line_end = sample
                .iter()
                .position(|&byte| byte == b'\n')
                .unwrap_or(sample.len());
            let line = String::from_utf8_lossy(&sample[2..line_end]);
            let mut interp_words = line.split_whitespace();
            if let Some(interp) = interp_words.next() {
                // Kernel #! exec (execve(2)): the interpreter is the first
                // word with one optional argument. `#!/usr/bin/env sh` makes
                // the kernel exec /usr/bin/env with `sh` as its single
                // argument, and env(1) then execs the first `sh` found on
                // PATH (GNU baseline: `PATH=<dir>:$PATH tool alpha beta`
                // prints script:<joined-path>:alpha:2, exit 0). GNU bash
                // never sees that exchange on Linux — shell_execve
                // (execute_cmd.c:6126) only reports "bad interpreter"
                // (execute_cmd.c:6184) when the kernel's own execve failed,
                // e.g. `#!/no/such/interp` -> 126. Windows has no kernel #!
                // and no /usr/bin/env, so this mailbox stands in for both:
                // an `env` launcher resolves through the FOLLOWING word —
                // the program env would exec — and that target decides the
                // refusal. When the target itself is missing, GNU's exit is
                // env's 127 with env's own diagnostic, approximated here by
                // the same refusal shape with env's message text.
                let (effective_interp, env_launcher) = if std::path::Path::new(interp)
                    .file_name()
                    .is_some_and(|base| base == "env")
                {
                    match interp_words.next() {
                        Some(target) => (target, true),
                        None => (interp, false),
                    }
                } else {
                    (interp, false)
                };
                // GNU execute_shell_script re-execs through the named
                // interpreter; when it resolves, the shell-script fallback
                // stands in for that exec here. The standard shells resolve
                // through the fallback without a refusal.
                let interp_resolves = matches!(effective_interp, "sh" | "bash" | "dash" | "rubash")
                    || effective_interp.ends_with("/sh")
                    || effective_interp.ends_with("/bash")
                    || crate::executor::path::find_user_command(
                        effective_interp,
                        &self.shell_state.env_vars,
                    )
                    .is_some();
                if !interp_resolves {
                    // A missing env target is the env child's own failure
                    // (GNU: `env: 'x': No such file or directory`, exit
                    // 127, printed by /usr/bin/env itself); a missing
                    // direct interpreter is the shell_execve refusal
                    // (execute_cmd.c:6184, exit 126).
                    let diagnostic = if env_launcher {
                        format!("env: '{effective_interp}': No such file or directory")
                    } else {
                        format!(
                            "bash: {}: {}: bad interpreter",
                            cmd.words.first().map(String::as_str).unwrap_or_default(),
                            interp
                        )
                    };
                    return Some((diagnostic, if env_launcher { 127 } else { 126 }));
                }
                return None;
            }
        }
        if crate::script_driver::check_binary_file(&sample) {
            return Some((
                format!(
                    "bash: {}: cannot execute binary file",
                    cmd.words.first().map(String::as_str).unwrap_or_default()
                ),
                126,
            ));
        }
        None
    }

    fn report_external_spawn_error(
        &mut self,
        cmd: &CommandNode,
        program: &PathBuf,
        error: io::Error,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        // GNU report_prolog (errors.c:75-120): a script-context exec failure
        // prints `$0: line N: word: message` — the same prolog every other
        // diagnostic uses (rubash#231: the bare `rubash:` prefix dropped the
        // script/line attribution GNU prints). shell_execve reports via
        // internal_error, so the prefix honors gnu_errfmt (rubash#1038).
        // GNU shell_execve (execute_cmd.c:6126-6159) reports the execve
        // failure against the RESOLVED pathname (findcmd.c file_to_lose_on
        // fallback included), not the typed word — `noexec.sh` found via
        // PATH prints the full candidate path (rubash#231).
        //
        // The status is NOT uniformly EX_NOEXEC: shell_execve sets
        // last_command_exit_value = (errno == ENOENT) ? EX_NOTFOUND (127)
        // : EX_NOEXEC (126) (execute_cmd.c:6143). A command the PATH walk
        // resolved whose image the kernel then cannot load — an ELF whose
        // PT_INTERP loader is missing is the common case — fails execve
        // with ENOENT and must surface as a LOOKUP failure (127), not an
        // exec refusal (126): scripts branch on 126-vs-127 (rubash#1172).
        // The diagnostic follows the same switch: an existing executable
        // file with a #! line names its interpreter ("bad interpreter",
        // EX_NOEXEC); an existing executable non-#! file is "cannot
        // execute: required file not found" (EX_NOTFOUND); anything else
        // keeps the plain errno text via file_error.
        let (message, status) = spawn_error_diagnostic(program, &error);
        writeln!(
            &mut stderr,
            "{}{}: {}",
            self.error_prefix(),
            program.display(),
            message
        )?;
        self.finish_external_error(cmd, &stderr, status)
    }
}

#[cfg(unix)]
/// Holds untrapped SIGINT at the recording handler across one
/// foreground blocking wait when the shell is an interactive reader
/// session (kernel disposition is SIG_DFL). GNU jobs.c:3086-3102
/// wait_for swaps in wait_sigint_handler for the wait: the handler
/// only RECORDS the delivery (trap.c trap_handler ->
/// set_trap_state), so an interactive shell never dies mid-wait —
/// including when it shares the child's process group (no job
/// control) and its own copy of ^C arrives during waitpid
/// (rubash#1250). This used to install SIG_IGN:
/// the shell survived, but its own copy of the delivery vanished
/// unrecorded, leaving the line-discard verdict nothing to consult.
/// The hold is the same one the builtin-sleep guard uses
/// (kill::hold_fg_sigint): arrivals queue in the kernel backend while
/// FG_INT_HOLD keeps reconcile from flipping SIGINT back to Default
/// mid-wait; Drop releases the hold. Trapped SIGINT keeps its handler
/// (disposition != DFL) so trap recording is untouched, and
/// entry-ignored SIGINT stays ignored.
struct ForegroundWaitSigintGuard {
    armed: bool,
    /// Kernel-SIGINT epoch snapshot taken when the wait starts
    /// (rubash#1990): the sliced wait forwards the interrupt to the
    /// child once the epoch moves past this value.
    sigint_epoch_at_arm: u64,
}

#[cfg(unix)]
impl ForegroundWaitSigintGuard {
    fn hold_if_interactive_untrapped(executor: &Executor) -> Self {
        if !foreground_sigint_needs_protection(executor) {
            return Self {
                armed: false,
                sigint_epoch_at_arm: 0,
            };
        }
        crate::builtins::kill::hold_fg_sigint();
        Self {
            armed: true,
            sigint_epoch_at_arm: crate::builtins::kill::sigint_kernel_epoch(),
        }
    }

    pub(in crate::executor) fn is_armed(&self) -> bool {
        self.armed
    }

    pub(in crate::executor) fn sigint_epoch_at_arm(&self) -> u64 {
        self.sigint_epoch_at_arm
    }
}

/// Drain the kernel pending queue the way the foreground waits
/// consume it (shared by wait_fg_child_sliced and the rubash#2398
/// job-control wait): arrivals move from the kernel backend into the
/// pending queues (a kernel SIGINT advances the epoch). Everything is
/// requeued for the command boundary EXCEPT the SIGINT that advanced
/// the epoch: that arrival is THIS wait's interrupt, and the wait
/// consumes it the way the builtin guard does — leaving it queued
/// would re-adjudicate it against the NEXT input line (discarding
/// that line as if it were the interrupted one). If the child
/// survives the forward and exits normally, the consumed interrupt is
/// forgotten with it, matching the child_caught_sigint outcome (see
/// wait_fg_child_sliced). Returns true when the SIGINT epoch moved
/// past the guard's arm snapshot, i.e. the caller should forward.
#[cfg(unix)]
fn fg_forward_drain(sigint_guard: &ForegroundWaitSigintGuard) -> bool {
    let pending =
        crate::builtins::kill::take_pending_signals_now(std::process::id()).unwrap_or_default();
    let epoch_moved =
        crate::builtins::kill::sigint_kernel_epoch() != sigint_guard.sigint_epoch_at_arm();
    if !pending.is_empty() {
        let rest: Vec<i32> = if epoch_moved {
            pending
                .into_iter()
                .filter(|&signal| signal != libc::SIGINT)
                .collect()
        } else {
            pending
        };
        if !rest.is_empty() {
            crate::builtins::kill::requeue_pending_signals(rest);
        }
    }
    epoch_moved
}

/// rubash#1990: sliced form of the armed foreground wait. GNU never
/// needs to forward: its children lead their own process groups and
/// the terminal's group delivery reaches them directly, so wait_for
/// (jobs.c:3064) only has to record the shell's own copy. rubash
/// children share the shell's process group, so on a healthy terminal
/// the same delivery reaches them too — but when the spawn
/// arrangement leaves the child outside the terminal's foreground
/// group (the Termux/Android PTY sessions behind rubash#1990, a child
/// that re-sessions itself), the delivery reaches ONLY the shell: the
/// plain blocking wait then sat until the child's natural end and the
/// recorded interrupt was forgotten ($?=0), while the session had no
/// way to make the current command die. Forwarding the recorded
/// kernel SIGINT to the child restores the invariant the group
/// delivery was supposed to provide: the foreground command dies on
/// ^C, is reaped here, and the wait reports its SIGINT death to the
/// #1639 unwind as usual. A child that catches or blocks SIGINT is
/// forwarded to exactly once and then left alone — GNU's
/// child_caught_sigint case (jobs.c waitchld cleanup): the wait
/// completes with the child's own status.
#[cfg(unix)]
fn wait_fg_child_sliced(
    child: &mut std::process::Child,
    sigint_guard: &ForegroundWaitSigintGuard,
) -> Result<std::process::ExitStatus, ExecuteError> {
    const SLICE: std::time::Duration = std::time::Duration::from_millis(10);
    let child_pid = child.id() as i32;
    let mut forwarded = false;
    loop {
        if let Some(status) = child.try_wait().map_err(ExecuteError::IoError)? {
            return Ok(status);
        }
        if !forwarded && fg_forward_drain(sigint_guard) {
            forward_sigint_to_fg_child(child_pid);
            forwarded = true;
        }
        std::thread::sleep(SLICE);
    }
}

/// Deliver the forwarded SIGINT for wait_fg_child_sliced: group form
/// when the child made itself a process-group leader (the arrangement
/// where it missed the terminal's group delivery), directed form
/// otherwise — the group form against the shell's own group would
/// interrupt the shell and every sibling in it.
#[cfg(unix)]
fn forward_sigint_to_fg_child(child_pid: i32) {
    unsafe {
        let pgid = libc::getpgid(child_pid);
        if pgid > 0 && pgid == child_pid {
            libc::kill(-pgid, libc::SIGINT);
        } else {
            libc::kill(child_pid, libc::SIGINT);
        }
    }
}

/// Shared predicate for both foreground SIGINT guards (rubash#1250):
/// an interactive reader session and SIGINT is untrapped, with the
/// kernel disposition in one of the two untrapped steady states —
/// SIG_DFL, or the mailbox handler the rubash#1662 interactive pin
/// installs from the command boundary on (GNU sig.c:315-316: an
/// interactive shell catches SIGINT itself from startup). Before the
/// pin, SIG_DFL alone proved "untrapped"; the pin made that test
/// read Handler for every untrapped interactive SIGINT, the guards
/// never armed, and deliveries queued in the mailbox with no consumer
/// during in-process blocking waits — a `read` ate the next input
/// line and a subshell frame's virtual child slept through the ^C
/// (rubash#1824). A handler alone still does not arm the guard: the
/// trap table decides ownership — a trapped SIGINT (any table entry,
/// including an ignore) keeps the trap path, and an entry-ignored
/// SIGINT (SIG_IGN, no handler) stays untouched.
/// rubash#1639: the session test is interactive_reader_session, not
/// the full expansion-discard rule — the discard rule's frame
/// conditions (depth 0, no comsub body) govern where an expansion
/// error is FATAL, while a foreground wait needs protection at any
/// depth: in the flat model the subshell frame's wait is the
/// process's foreground wait, and GNU protects the corresponding
/// wait_for (the parent's, for the subshell job) identically.
#[cfg(unix)]
pub(in crate::executor) fn foreground_sigint_needs_protection(executor: &Executor) -> bool {
    if !executor.interactive_reader_session() {
        return false;
    }
    if sigint_disposition_is_default() {
        return true;
    }
    crate::builtins::kill::kernel_handler_installed(libc::SIGINT)
        && crate::builtins::trap::get_trap_action(&executor.shell_state.env_vars, "SIGINT")
            .is_none()
}

#[cfg(unix)]
fn sigint_disposition_is_default() -> bool {
    // sigaction(null) queries without installing (same probe as
    // kill.rs entry-ignored snapshot): only SIG_DFL means "no trap
    // installed, would die" — a handler (trapped) or SIG_IGN
    // (entry-ignored) must be left alone.
    let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
    if unsafe { libc::sigaction(libc::SIGINT, std::ptr::null(), &mut current) } != 0 {
        return false;
    }
    current.sa_sigaction == libc::SIG_DFL
}

#[cfg(unix)]
impl Drop for ForegroundWaitSigintGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        crate::builtins::kill::release_fg_sigint();
    }
}

/// SIGINT guard for builtins that block in-process (rubash#1250: the
/// `sleep` fast path, which is what the issue's `sleep 30` repro actually
/// exercises — `sleep` is a rubash builtin). SIG_IGN would be WRONG here:
/// GNU's sleep is an external child that DIES from the pgroup SIGINT and
/// the shell reports 130, while an ignored SIGINT would let the builtin
/// sleep run to completion with rc=0. So this guard routes untrapped SIGINT
/// to the mailbox handler and HOLDS it there (kill::hold_fg_sigint) for
/// the wait: the delivery becomes an observable pending entry, the slice
/// loop's observe_signals_while_blocked converts it to Err(ExitCode(130))
/// via the same untrapped path external deaths take, and Drop restores
/// Default. Trapped SIGINT keeps its handler (predicate false, mailbox
/// path untouched); non-interactive shells keep SIG_DFL and die like GNU
/// scripts. (A pthread_sigmask BLOCK was tried first and reverted: the
/// process is multithreaded, so a per-thread mask cannot stop delivery
/// to the DFL main thread — verified by probe, guard armed yet shell
/// still died.)
#[cfg(unix)]
pub(in crate::executor) struct ForegroundBuiltinSigintGuard {
    armed: bool,
}

#[cfg(unix)]
impl ForegroundBuiltinSigintGuard {
    pub(in crate::executor) fn hold_if_interactive_untrapped(executor: &Executor) -> Self {
        if !foreground_sigint_needs_protection(executor) {
            return Self { armed: false };
        }
        crate::builtins::kill::hold_fg_sigint();
        Self { armed: true }
    }

    pub(in crate::executor) fn is_armed(&self) -> bool {
        self.armed
    }

    /// Consume a pending untrapped SIGINT for this wait (true = caller aborts
    /// with 128+SIGINT). Gated on armed: a TRAPPED SIGINT must flow through
    /// observe_signals_while_blocked so its action runs — stealing it here
    /// would silently drop the trap. kernel_handler_installed rejects stale
    /// registry flags (same stale rule observe uses); anything else goes
    /// back for the trapped-abort check and observe.
    pub(in crate::executor) fn take_sigint_abort(&self) -> bool {
        if !self.armed {
            return false;
        }
        let pending =
            crate::builtins::kill::take_pending_signals_now(std::process::id()).unwrap_or_default();
        if pending.is_empty() {
            return false;
        }
        let mut rest = Vec::new();
        let mut interrupted = false;
        for signal in pending {
            // rubash#1640: only a KERNEL-caused SIGINT (the terminal
            // driver's group delivery, ^C) kills the virtual child —
            // that is the delivery GNU's real foreground child dies
            // from. A kill(2) addressed to the shell alone leaves the
            // real child running, so it is not an abort: put it back
            // for observe_signals_while_blocked, which consumes it
            // without ending the wait (the recorded interrupt is
            // forgotten when the child completes normally, exactly
            // like the trapped path's origin check in
            // sleep_interruptibly).
            if signal == libc::SIGINT
                && crate::builtins::kill::kernel_handler_installed(signal)
                && crate::builtins::kill::last_delivery_kernel_caused(signal)
            {
                interrupted = true;
            } else {
                rest.push(signal);
            }
        }
        crate::builtins::kill::requeue_pending_signals(rest);
        interrupted
    }
}

#[cfg(unix)]
impl Drop for ForegroundBuiltinSigintGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        crate::builtins::kill::release_fg_sigint();
    }
}

/// GNU shell_execve (execute_cmd.c:6126-6159) classification of a failed
/// execve/spawn against the resolved program path. Returns the diagnostic
/// body (the caller prepends the report prolog and the resolved pathname)
/// and the exit status. Non-ENOENT failures are EX_NOEXEC with the plain
/// errno text, exactly the historical behavior of this call site.
fn spawn_error_diagnostic(program: &PathBuf, error: &io::Error) -> (String, i32) {
    const EX_NOTFOUND: i32 = 127;
    const EX_NOEXEC: i32 = 126;
    // ENOENT on unix and ERROR_FILE_NOT_FOUND on Windows share the value 2;
    // no libc dependency is needed for this classification.
    const OS_ERROR_FILE_NOT_FOUND: i32 = 2;
    if error.raw_os_error() != Some(OS_ERROR_FILE_NOT_FOUND) {
        return (crate::posix_errors::message(error).to_string(), EX_NOEXEC);
    }
    // errno == ENOENT: the status is EX_NOTFOUND unless the file itself
    // proves the #! bad-interpreter refusal (EX_NOEXEC).
    let metadata = std::fs::metadata(program).ok();
    let executable = metadata
        .as_ref()
        .is_some_and(|meta| meta.is_file() && file_has_exec_bit(meta));
    if executable {
        if let Ok(sample) = std::fs::read(program) {
            if sample.len() >= 2 && sample[0] == b'#' && sample[1] == b'!' {
                let line_end = sample
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .unwrap_or(sample.len());
                let line = String::from_utf8_lossy(&sample[2..line_end]);
                // GNU getinterp (execute_cmd.c) terminates the interpreter
                // name at a blank or tab only — a CR stays part of the
                // name (and is displayed as ^M below), so do not split on
                // the full whitespace class here.
                let interp = line.split([' ', '\t']).next().unwrap_or_default();
                if !interp.is_empty() {
                    // GNU getinterp + sys_error (execute_cmd.c:6170-6184):
                    // a trailing CR in the interpreter name is displayed
                    // as ^M (`#!/bin/sh\r` -> "/bin/sh^M").
                    let interp = match interp.strip_suffix('\r') {
                        Some(stem) => format!("{stem}^M"),
                        None => interp.to_string(),
                    };
                    return (
                        format!("{interp}: bad interpreter: No such file or directory"),
                        EX_NOEXEC,
                    );
                }
            }
        }
        // An executable file the kernel cannot load at all (missing
        // PT_INTERP loader): execute_cmd.c:6188-6192 internal_error.
        return (
            "cannot execute: required file not found".to_string(),
            EX_NOTFOUND,
        );
    }
    (crate::posix_errors::message(error).to_string(), EX_NOTFOUND)
}

/// GNU executable_file (execute_cmd.c / findcmd.c file_status FS_EXECABLE):
/// any execute permission bit set. Non-unix hosts have no mode bits; an
/// existing regular file counts as executable there.
fn file_has_exec_bit(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        true
    }
}

/// The binary-file gate itself is `crate::script_driver::check_binary_file`
/// (general.c:718-741) — one shared implementation for the script-entry
/// and external-command gates (redundancy R9: the two copies were
/// byte-identical logic and could only drift apart).

#[derive(Default)]
struct EnvCommandConfig {
    ignore_environment: bool,
    null_terminated: bool,
    debug: bool,
    unset_names: Vec<String>,
    // Kept in argument order: coreutils env applies its NAME=value
    // operands in order (later operands supersede in place), and the
    // composed environment's print order exposes it (rubash#1806).
    assignments: Vec<(String, String)>,
    file: Option<String>,
    chdir: Option<String>,
    argv0: Option<String>,
    command: Vec<String>,
}

fn parse_env_assignment_arg(arg: &str) -> Option<(String, String)> {
    let (name, value) = arg.split_once('=')?;
    (!name.is_empty()).then(|| (name.to_string(), value.to_string()))
}

fn long_option_value(arg: &str, long_prefix: &str) -> Option<String> {
    if let Some(value) = arg.strip_prefix(long_prefix) {
        return Some(value.to_string());
    }
    None
}

/// Insert into an ordered environment composition with GNU
/// add_or_supercede semantics (variables.c): an existing name is
/// replaced in place, a new name is appended (rubash#1806).
fn ordered_env_insert(entries: &mut Vec<(String, String)>, name: String, value: String) {
    if let Some(slot) = entries.iter_mut().find(|(existing, _)| *existing == name) {
        slot.1 = value;
    } else {
        entries.push((name, value));
    }
}

fn ordered_env_contains(entries: &[(String, String)], name: &str) -> bool {
    entries.iter().any(|(existing, _)| existing == name)
}

fn ordered_env_get<'a>(entries: &'a [(String, String)], name: &str) -> Option<&'a String> {
    entries
        .iter()
        .find(|(existing, _)| existing == name)
        .map(|(_, value)| value)
}

fn ordered_env_remove(entries: &mut Vec<(String, String)>, name: &str) {
    entries.retain(|(existing, _)| existing != name);
}

fn apply_env_command_environment(
    process: &mut Command,
    env_vars: &[(String, String)],
    inherit_required_windows: bool,
) {
    process.env_clear();
    let env_map: HashMap<String, String> = env_vars.iter().cloned().collect();
    for (name, value) in env_vars {
        if is_valid_process_env(name, value) {
            let value = if cfg!(windows) && name.eq_ignore_ascii_case("PATH") {
                shell_path_to_process(value, &env_map)
            } else if cfg!(windows) && name == "TMPDIR" {
                // Same child-boundary contract as child_env_value
                // (unixwin/niubash#162): exported TMPDIR crosses in
                // Windows-native forward-slash form, never the slash-drive
                // `/c/...` display form that foreign children (Bun-compiled
                // executables) resolve drive-relative and fail on.
                shell_path_to_windows(value, &env_map)
                    .to_string_lossy()
                    .replace('\\', "/")
            } else {
                // Raw-byte marker pairs in a stored value must map to byte
                // chars at the child boundary (same rubash#141 contract as
                // child_env_value / the argv boundary), never leak the
                // U+E000 pair into the child's environment block.
                crate::executor::substitution_metadata::decode_raw_byte_markers_to_byte_chars(value)
            };
            process.env(name, value);
        }
    }
    if inherit_required_windows {
        apply_required_windows_child_environment(process, &env_map);
    }
}

#[cfg(windows)]
fn materialize_required_windows_env(
    env_vars: &mut Vec<(String, String)>,
    shell_env_vars: &HashMap<String, String>,
    ignore_environment: bool,
) {
    if ignore_environment {
        return;
    }

    for name in ["SystemRoot", "WINDIR", "ComSpec"] {
        if ordered_env_contains(env_vars, name) {
            continue;
        }
        if let Some(value) = shell_env_vars
            .get(name)
            .cloned()
            .or_else(|| env::var(name).ok())
        {
            ordered_env_insert(env_vars, name.to_string(), value);
        }
    }

    let home = ordered_env_get(env_vars, "USERPROFILE")
        .cloned()
        .or_else(|| shell_env_vars.get("USERPROFILE").cloned())
        .or_else(|| env::var("USERPROFILE").ok())
        .or_else(|| {
            ordered_env_get(env_vars, "HOME")
                .or_else(|| shell_env_vars.get("HOME"))
                .map(|value| {
                    shell_path_to_windows(value, shell_env_vars)
                        .to_string_lossy()
                        .into_owned()
                })
        });
    let Some(home) = home.filter(|value| !value.trim().is_empty() && !value.contains('\0')) else {
        return;
    };

    let native_home = home.replace('/', "\\");
    if !ordered_env_contains(env_vars, "USERPROFILE") {
        ordered_env_insert(env_vars, "USERPROFILE".to_string(), native_home.clone());
    }
    if !ordered_env_contains(env_vars, "HOME") {
        ordered_env_insert(env_vars, "HOME".to_string(), native_home.clone());
    }
    if let Some((drive, path)) = windows_drive_and_home_path(&native_home) {
        if !ordered_env_contains(env_vars, "HOMEDRIVE") {
            ordered_env_insert(env_vars, "HOMEDRIVE".to_string(), drive);
        }
        if !ordered_env_contains(env_vars, "HOMEPATH") {
            ordered_env_insert(env_vars, "HOMEPATH".to_string(), path);
        }
    }
    let base = native_home.trim_end_matches('\\');
    if !ordered_env_contains(env_vars, "APPDATA") {
        ordered_env_insert(
            env_vars,
            "APPDATA".to_string(),
            format!("{base}\\AppData\\Roaming"),
        );
    }
    if !ordered_env_contains(env_vars, "LOCALAPPDATA") {
        ordered_env_insert(
            env_vars,
            "LOCALAPPDATA".to_string(),
            format!("{base}\\AppData\\Local"),
        );
    }
}

#[cfg(not(windows))]
fn materialize_required_windows_env(
    _env_vars: &mut Vec<(String, String)>,
    _shell_env_vars: &HashMap<String, String>,
    _ignore_environment: bool,
) {
}

#[cfg(windows)]
fn windows_drive_and_home_path(path: &str) -> Option<(String, String)> {
    let bytes = path.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let drive = path[..2].to_string();
    let rest = path[2..].trim_start_matches(['\\', '/']);
    Some((drive, format!("\\{}", rest.replace('/', "\\"))))
}

fn is_exec_format_error(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(8)
    }

    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

/// Whether execve(2) would refuse `program` with ENOEXEC: a regular
/// file whose first bytes are neither a `#!` interpreter line nor the
/// ELF magic of a native image (the only two formats the kernel takes
/// directly on modern Linux). Directories and unreadable files are
/// left to the normal spawn-error classification.
#[cfg(unix)]
pub(in crate::executor) fn is_enoexec_candidate(program: &std::path::Path) -> bool {
    use std::io::Read;
    if !std::fs::metadata(program).is_ok_and(|meta| meta.is_file()) {
        return false;
    }
    // The ENOEXEC shell-script fallback (execute_cmd.c:6139-6233
    // shell_execve) is only reached when execve actually fails with
    // ENOEXEC. A file without any execute bit fails earlier with
    // EACCES, and GNU reports it as Permission denied / exit 126
    // (findcmd.c:591/695 keep such files out of the runnable set the
    // same way). Without this gate a non-executable text file was run
    // as a script with status 0 (rubash#1459).
    if !crate::executor::path::is_executable_file(program) {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(program) else {
        return false;
    };
    let mut head = [0u8; 4];
    let read = file.read(&mut head).unwrap_or(0);
    let head = &head[..read];
    !head.starts_with(b"#!") && !head.starts_with(b"\x7fELF")
}

/// GNU general.c:843 absolute_program: a name containing `/` (or `\` on a
/// MSYS-like host). Such a name is never PATH-searched (findcmd.c:385-386
/// search_for_command), so its failure text is classified by the execve
/// result, not by the PATH miss: execute_cmd.c:6126-6159 shell_execve —
/// ENOENT ("No such file or directory", 127), a directory (EISDIR text,
/// 126), an unexecutable file ("Permission denied", 126). Returns None for
/// slash-free names, which keep the "command not found" PATH wording
/// (execute_cmd.c:5910 notfound_str).
pub(crate) fn slash_command_execve_error_message(
    name: &str,
    env_vars: &HashMap<String, String>,
) -> Option<(&'static str, i32)> {
    // Mirror path.rs has_path_separator (general.c:843 absolute_program):
    // the backslash is a separator only on Windows/MSYS hosts; on unix
    // `a\b` is an ordinary name that goes through the PATH search and
    // earns "command not found", not the execve classification
    // (rubash#1482).
    let has_separator = if cfg!(windows) {
        name.contains('/') || name.contains('\\')
    } else {
        name.contains('/')
    };
    if !has_separator {
        return None;
    }
    let candidate = crate::executor::path::shell_path_to_windows(name, env_vars);
    let metadata = std::fs::metadata(&candidate)
        .or_else(|_| std::fs::metadata(name))
        .ok();
    match metadata {
        None => Some(("No such file or directory", 127)),
        Some(metadata) if metadata.is_dir() => Some(("Is a directory", 126)),
        Some(_) => Some(("Permission denied", 126)),
    }
}
