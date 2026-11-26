use super::control_flow::parameter_expansion_assigns;
use super::transport::{
    respawn_source_needs_file, rubash_spawn_inherited_state, test_rubash_binary_from_current_exe,
    test_rubash_binary_from_env,
};
use super::BackgroundStdio;
use super::{encode_root_line_spans, first_leaf_command_line};
use crate::executor::*;

/// GNU jobs.c:2455/2460 pipeline_pgrp at fork time, recorded as j->pgrp
/// (jobs.c:649) and substituted by `jobs -x` (builtins/jobs.def
/// execute_list_with_replacements `itos (j->pgrp)`, rubash#1096): with job
/// control the first pipeline pid becomes the group; without job control
/// every job shares the shell's own pgrp. The RECORDED value is what
/// `jobs -x` prints, and without job control GNU's value is exactly the
/// live shell pgrp — so read getpgid(0) here. Since rubash#2241 the
/// job-control case is also the child's real group: the async spawn
/// sites setpgid the child into it (child side jobs.c:2390, parent
/// side jobs.c:2455), so the record and the kernel state agree.
/// Non-unix has no pgrp concept; the job pid keeps today's observable.
pub(super) fn async_job_pgrp(first_pid: u32, job_control: bool) -> u32 {
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
            // rubash#3894: a `{var}<<` entry never owns the fd-0
            // mirror, so it must not keep it alive during the prune.
            redirect.fd_var.is_none()
                && redirect.fd.is_none()
                && matches!(redirect.kind, crate::parser::RedirectKind::HereDoc)
        }) {
            command.heredoc = None;
            command.heredoc_body = None;
            command.heredoc_delimiter = None;
        }
        if !remaining.iter().any(|redirect| {
            // rubash#3894: see the HereDoc arm above — a `{var}<<<`
            // entry never owns the fd-0 mirror either.
            redirect.fd_var.is_none()
                && redirect.fd.is_none()
                && matches!(redirect.kind, crate::parser::RedirectKind::HereString)
        }) {
            command.here_string = None;
            command.here_string_carrier = None;
        }
    }
}

impl Executor {
    /// rubash#1256: GNU prints the async start notice `[<job>] <pid>`
    /// when a background job launches in an INTERACTIVE shell
    /// (execute_cmd.c async launch -> jobs.c pretty_print_job; verified
    /// against GNU 5.3.20: printed with `monitor` on or off, and never
    /// in a non-interactive shell — not even with `set -m`). The
    /// termination notices already run at the reader boundary
    /// (job_builtins notify path); the start notice was the missing half.
    ///
    /// rubash#2214: the notice is gated on the INTERACTIVE flag of the
    /// context that starts the job, not of the session. GNU's print
    /// site is `DESCRIBE_PID` (execute_cmd.c:609 — `if (interactive)
    /// describe_pid (pid)`, jobs.c:1947), and every subshell
    /// environment clears `interactive` (execute_cmd.c:1637
    /// execute_in_subshell: "Subshells are neither login nor
    /// interactive"), so background children started inside `( )`,
    /// `$( )`, a pipeline stage, or a process substitution never
    /// print — while a function body running in the main interactive
    /// context DOES print for its own `&` children (both verified
    /// against GNU 5.3.20 under a PTY). The fork copies that execute
    /// those environments carry the session's `__RUBASH_INTERACTIVE`
    /// marker in their cloned state, so the session-level check alone
    /// leaked the notice (observed: interactive `nvm ls`, whose
    /// `( ... & ... ) | sort` alias printers produced `[1] <pid>`
    /// noise GNU never prints). `in_subshell_environment()` is the
    /// executor's existing model of exactly that boundary.
    pub(in crate::executor) fn notify_async_job_started(
        &mut self,
        job_id: crate::jobs::JobId,
        pid: u32,
    ) {
        if !self.invocation_interactive() {
            return;
        }
        if self.in_subshell_environment() {
            return;
        }
        let _ = self.write_default_stderr(format!("[{job_id}] {pid}\n").as_bytes());
    }

    pub(in crate::executor) fn execute_background_ast_command(
        &mut self,
        background_command: &BackgroundCommand,
    ) -> Result<(), ExecuteError> {
        // rubash#3932 async face: GNU's forked async child marks
        // CMD_NO_FORK unconditionally (execute_cmd.c:4553 sets it in the
        // make_child child half, no shape or trap consult), so the plain
        // external this dispatch runs exports live-1 — redirects travel
        // (resolved into the fork's stdio) and a parent-set EXIT/ERR
        // trap does not block. Arm for the simple-command shapes only:
        // compounds re-mark through their own faces (the in-process
        // paren body walk, the wrapper respawn child) and pipeline
        // members keep the forked face (SUBSHELL_PIPE, execute_cmd.c
        // 5816). Saved and restored so the arm never leaks past this
        // dispatch — the wrapper fallback respawns a child that arms
        // through its own wrapper-role face.
        let saved_stamp = self.nofork_spawn_dispatch_stamp.get();
        let saved_unconditional = self.nofork_spawn_face_unconditional.get();
        let saved_allows_redirects = self.nofork_spawn_face_allows_redirects.get();
        if Self::nofork_spawn_async_simple_shape_ok(&background_command.command) {
            let stamp = self.nofork_spawn_stamp_counter.get() + 1;
            self.nofork_spawn_stamp_counter.set(stamp);
            self.nofork_spawn_dispatch_stamp.set(stamp);
            self.nofork_spawn_face_unconditional.set(true);
            self.nofork_spawn_face_allows_redirects.set(true);
        } else {
            self.nofork_spawn_dispatch_stamp.set(0);
            self.nofork_spawn_face_unconditional.set(false);
        }
        let result = self.execute_background_ast_command_inner(background_command);
        self.nofork_spawn_dispatch_stamp.set(saved_stamp);
        self.nofork_spawn_face_unconditional
            .set(saved_unconditional);
        self.nofork_spawn_face_allows_redirects
            .set(saved_allows_redirects);
        result
    }

    fn execute_background_ast_command_inner(
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
            // rubash#2442: the async child is an execute_disk_command
            // spawn like any other (execute_cmd.c:5810 resolution,
            // 6133 execve) — its execve filename comes from the shared
            // funnel (typed word / as-written search join), so a
            // background script's $0 matches GNU, and argv[0] is the
            // typed word exactly as the named-program constructor
            // sets it. The child is born in the session cwd below,
            // which is what a relative join resolves against.
            let exec_name = crate::executor::path::spawn_exec_name(
                &argv[0],
                &program,
                &self.shell_state.env_vars,
            );
            let job_control =
                crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor");
            // rubash#2925: on unix the direct spawn is a raw fork, not
            // `Command::spawn` — the spawn's error-pipe sync blocks the
            // parent until the child execs, so the parent's kill always
            // arrives after the child's SIGINT-ignore install (`w=0`).
            // The raw fork returns the pid at once (see
            // `spawn_async_direct`); the child installs the same
            // dispositions, so the delayed-kill steady state is unchanged.
            // rubash#1035: GNU execute_cmd.c setup_async_signals ignores
            // SIGINT/SIGQUIT in the async child while job_control is off
            // — a background `sleep` survives `kill -INT $!`; with job
            // control on, the grouped reset applies instead (GNU
            // jobs.c:2390 make_child). The #737 pdeathsig arm keys off
            // __RUBASH_BACKGROUND_WRAPPER, which is never set here — a
            // directly-exec'd async child survives shell exit, as GNU's
            // does.
            #[cfg(unix)]
            let spawned = {
                let exec_path: &std::path::Path = exec_name
                    .as_deref()
                    .map(std::path::Path::new)
                    .unwrap_or(&program);
                // Same environment composition as every foreground external
                // (exported shell vars; no assignments exist at this gate).
                let env_pairs =
                    self.compose_external_environment(&background_command.command, &program);
                // rubash#3932: the direct async spawn is GNU's forked
                // async child (execute_cmd.c:4553 CMD_NO_FORK, :5816
                // adjust_shell_level(-1) before maybe_make_export_env) —
                // export live-1 in its environment. The async arm above
                // unconditional: no trap gate applies.
                let env_pairs = if self.nofork_spawn_adjust_allowed() {
                    let mut pairs = env_pairs;
                    crate::executor::readonly_functions::env_pair_supercede(
                        &mut pairs,
                        "SHLVL".to_string(),
                        self.nofork_spawn_adjusted_shlvl(),
                    );
                    pairs
                } else {
                    env_pairs
                };
                self.spawn_async_direct(
                    exec_path,
                    &argv,
                    stdio,
                    &env_pairs,
                    &self.shell_state.cwd,
                    job_control,
                )
                .map(crate::fd::SpawnedChild::from_pid)
            };
            #[cfg(not(unix))]
            let spawned = {
                let mut process = match &exec_name {
                    Some(name) => std::process::Command::new(name),
                    None => std::process::Command::new(&program),
                };
                process.args(&argv[1..]);
                let [stdin_stdio, stdout_stdio, stderr_stdio] =
                    stdio.map(|resolved| match resolved {
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
                self.apply_external_environment(
                    &background_command.command,
                    &mut process,
                    &program,
                );
                // rubash#3932: the direct async spawn exports live-1
                // (execute_cmd.c:4553/5816) — same fold as the unix twin.
                if self.nofork_spawn_adjust_allowed() {
                    process.env("SHLVL", self.nofork_spawn_adjusted_shlvl());
                }
                self.stamp_spawn_cwd(&mut process);
                suppress_orphan_console(&mut process);
                process.spawn().map(crate::fd::SpawnedChild::from)
            };
            match spawned {
                Ok(child) => {
                    let pid = child.id();
                    adopt_orphan_console(pid);
                    self.background_children.insert(pid, child);
                    // rubash#2241: parent half of the grouping
                    // (GNU jobs.c:2455) — the recorded pgrp in
                    // async_job_pgrp already assumed this group exists.
                    #[cfg(unix)]
                    if job_control {
                        crate::builtins::kill::group_spawned_child(pid);
                    }
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
        // rubash#3300: mint the respawn proof FIRST, before any
        // carrier dup is created (the stdio / fd>=3 dup loops below
        // take the lowest free fds; the mint's F_DUPFD_CLOEXEC >= 100
        // can never collide with them, and minting first guarantees
        // the proof number is stable for the whole spawn). A mint
        // failure is fail-open (None): the spawn proceeds without
        // proof keys, i.e. today's behavior.
        #[cfg(unix)]
        let respawn_proof =
            crate::fd::mint_respawn_proof(&crate::builtins::kill::new_background_wrapper_nonce());
        let exe = test_rubash_binary_from_env()
            .or_else(test_rubash_binary_from_current_exe)
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| "rubash".into());
        // Environment block for the child: process env merged with the
        // shell's env_vars (same filter the old Command::env loop applied).
        // rubash#2259: the process-env base crosses through the same
        // spawn-inherited adjudication as the env_vars loop below —
        // pre-fix it ingested the raw environ, so any internal
        // `__RUBASH_*` state a foreign parent (or an older binary's
        // mirror) had planted there rode into the respawned child
        // unadjudicated, including the parent's invocation
        // classification. Explicit stamps inserted after this merge
        // (SHELL_PID, the spawner stamp, the trap reset table, ...)
        // are unaffected.
        let mut env_map: std::collections::BTreeMap<String, String> =
            crate::executor::env_helpers::process_env_pairs()
                .into_iter()
                .filter(|(key, _)| {
                    !key.starts_with("__RUBASH_") || rubash_spawn_inherited_state(key)
                })
                .collect();
        // rubash#3932: the wrapper child replays the GNU face its payload
        // carries — a simple command rides the forked async child's
        // unconditional mark (execute_cmd.c:4553), an absorbed `( list )`
        // body replays the subshell mark (execute_cmd.c:1799-1806, trap
        // and redirect gated), and any other compound body keeps the
        // live level (optimize_subshell_command requires cm_simple,
        // execute_cmd.c:1756). The key rides the validated nonce
        // handshake; a forged ambient value arms nothing.
        {
            let payload_face =
                if Self::nofork_spawn_async_simple_shape_ok(&background_command.command) {
                    "simple"
                } else if background_command.command.subshell_command.is_some()
                    || background_command.command.subshell
                {
                    "subshell"
                } else {
                    "compound"
                };
            env_map.insert(
                crate::builtins::kill::BACKGROUND_WRAPPER_PAYLOAD_FACE_KEY.to_string(),
                payload_face.to_string(),
            );
        }
        // rubash#2486: iterate the SNAPSHOT, not the live table. The
        // VarTable is write-behind for the attribute keys (#916): the
        // serialized `__RUBASH_*_VARS` strings inside it are stale/absent
        // and only env_vars_snapshot materializes them from the structured
        // flags. Iterating the raw table shipped a bg child that had lost
        // every variable attribute — `declare -A building=()` imported as a
        // plain string, so `building[app]=1` took the INDEXED path, whose
        // subscript is arithmetic (arrayfunc.c:425 array_expand_index ->
        // evalexp), where a bare `app` under `set -u` raised `app: unbound
        // variable`. GNU's async child is a FORK (execute_cmd.c:1761-1763
        // execute_in_subshell), inheriting the attribute table whole; for
        // assoc arrays the subscript is word-expanded
        // (arrayfunc.c:396 expand_subscript_string, subst.c:11063) and used
        // literally as the key (arrayfunc.c:408 bind_assoc_variable) — no
        // unbound check of the expanded key ever runs.
        for (key, value) in self.env_vars_snapshot() {
            if !key.starts_with("__RUBASH_") || rubash_spawn_inherited_state(&key) {
                env_map.insert(key, value);
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
        // through setattr apply.rs but is not cleaned on trap removal) used
        // to ride into env_map via the process-env ingest above — the
        // rubash#2259 adjudication on that ingest now drops them there —
        // and a strip driven by env_vars would leave any copy that still
        // arrived (trap.tests: __RUBASH_TRAP_SIGHUP/__RUBASH_TRAP_SIGUSR2
        // leaked into the monitored `sleep N &` children after
        // `trap - HUP`/`trap - USR2` had cleared the table). The retain
        // stays as the boundary assertion regardless of the ingest route.
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
        // rubash#2982: the wrapper child re-defines the parent's
        // functions from a textual prelude — its table is empty until
        // the prelude is evaluated — so the functrace/errtrace-kept
        // DEBUG and ERR traps must not arrive live (they would fire
        // before the first prelude definition, while the handler
        // functions do not exist yet). Reset them here; the -c source
        // re-arms them after the prelude (background_trap_rearm_source).
        crate::builtins::trap::reset_traps_for_background_wrapper(&mut child_trap_table);
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
        // rubash#2136: the provenance stamp the child's init validates
        // against its actual parent (init.rs) — this process's own pid,
        // which is the pid the spawned child will observe via getppid.
        env_map.insert(
            crate::executor::RESPAWN_SPAWNER_PID_KEY.to_string(),
            crate::executor::spawner_stamp_value(),
        );
        // rubash#3300: the per-spawn proof pair (fd number + nonce).
        // Per-spawn, never forwarded: these keys are NOT in
        // FORK_TRANSPORT_KEYS / rubash_spawn_inherited_state, so a
        // grandchild never inherits them — its own spawner mints a
        // fresh proof for it. The #2356 spill below moves them onto
        // the state-file transport only if they grow oversized; they
        // never do (a decimal fd + 32 hex chars).
        #[cfg(unix)]
        if let Some((proof_fd, proof_nonce)) = &respawn_proof {
            env_map.insert(
                crate::executor::RESPAWN_PROOF_FD_KEY.to_string(),
                proof_fd.to_string(),
            );
            env_map.insert(
                crate::executor::RESPAWN_PROOF_KEY.to_string(),
                proof_nonce.clone(),
            );
        }
        // rubash#2521: this spawn is the whitelist transport
        // (fd::spawn_whitelisted below) — its fd stamps name
        // descriptors delivered AT their stamped numbers, and the
        // child's restore must adopt them there, not at the slot
        // (FD_DELIVERY_AT_STAMPED_KEY, init.rs). The pin transports
        // (external / input-procsub / coproc) never set this key —
        // the output procsub form does (rubash#3221: it carries no
        // pre_exec pin pass, so its stamped dups are authoritative).
        env_map.insert(
            crate::executor::FD_DELIVERY_AT_STAMPED_KEY.to_string(),
            "1".to_string(),
        );
        // rubash#2032: GNU's forked async/subshell child inherits the
        // parent's positional parameters (execute_cmd.c:1576
        // execute_in_subshell runs a whole-copy fork); stamp the live
        // window for the child's init to seed (POSITIONAL_PARAMS_TRANSPORT).
        env_map.insert(
            POSITIONAL_PARAMS_TRANSPORT.to_string(),
            self.positional_params_transport_value(),
        );
        // rubash#2058: the fork-context stamp (call-stack views,
        // PIPESTATUS cell, ARGV bottom state) — same fork, same
        // transport set (see fork_context_transport_entries).
        for (key, value) in self.fork_context_transport_entries() {
            env_map.insert(key, value);
        }
        // rubash#3605: the async child is the same execute_in_subshell
        // fork the flag model travels (execute_cmd.c:1576); an
        // inverted/exempt `&` body keeps CMD_IGNORE_RETURN
        // (SUPPRESS_ERREXIT_TRANSPORT).
        if self.suppress_errexit > 0 {
            env_map.insert(SUPPRESS_ERREXIT_TRANSPORT.to_string(), "1".to_string());
        }
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
            // rubash#2586: the top-level name is part of the same script
            // identity — a fork keeps it (GNU fork inherits all
            // variables), so the respawn forwards it with SCRIPT_NAME
            // instead of letting the child re-derive it from the
            // respawn's own $0.
            if let Some(top_level_name) = self.shell_state.env_vars.get("__RUBASH_TOP_LEVEL_NAME") {
                env_map.insert(
                    "__RUBASH_TOP_LEVEL_NAME".to_string(),
                    top_level_name.clone(),
                );
            }
            // rubash#2058: the seed line is where the transported body
            // text starts — the first leaf command's line. A compound
            // node's own `line` is its closing line, which put the
            // seed at the END of a multi-line `( ... ) &` body.
            let line = first_leaf_command_line(&background_command.command)
                .map(|line| line.to_string())
                .or_else(|| {
                    self.shell_state
                        .env_vars
                        .get("__RUBASH_CURRENT_LINE")
                        .cloned()
                })
                .unwrap_or_else(|| "1".to_string());
            env_map.insert("__RUBASH_CURRENT_LINE".to_string(), line.clone());
            // The child source is prelude + body text, so the body's
            // first line sits prelude_newlines + 1 lines into it; the
            // offset compensates exactly like the output-procsub
            // consumer site (external_setup.rs) — without it every
            // body line (and every BASH_LINENO call line pushed inside)
            // drifts by the prelude's newline count, which varies
            // with the transported function bodies (rubash#2058).
            let prelude_newlines = self.background_function_prelude().matches('\n').count();
            let line_offset = line
                .parse::<usize>()
                .unwrap_or(1)
                .saturating_sub(1)
                .saturating_sub(prelude_newlines);
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
        // rubash#2332: the root body's line spans ride along with the
        // source built here — collect them over exactly the nodes each
        // form renders (the absorbed inner list, or the single
        // transported command after redirect stripping), so the
        // child's re-parse can be re-seeded onto the parent's parse
        // lines (see docs/design/respawn-root-line-spans.md). The
        // prelude count is background_function_prelude's own
        // definition count, taken over the same map and filter.
        let mut root_spans = Vec::new();
        let source = if strip_parens {
            // SAFETY: strip gate guarantees subshell_command.is_some().
            let inner = child_command
                .subshell_command
                .as_ref()
                .map(|subshell| subshell.body.clone())
                .unwrap_or_default();
            crate::parser::collect_command_line_spans(&inner, &mut root_spans);
            self.background_command_source_for_body(&inner)
        } else {
            crate::parser::collect_command_line_spans(
                std::slice::from_ref(&child_command),
                &mut root_spans,
            );
            self.background_command_source(&child_command)
        };
        if !root_spans.is_empty() {
            // rubash#2543: count exactly the definitions the rendered
            // prelude carries (background_prelude_function_entries),
            // or the child's root-span line math drifts by the skipped
            // env-carried names.
            let prelude_defs = self.background_prelude_function_entries().len();
            env_map.insert(
                FORK_ROOT_SPANS_TRANSPORT.to_string(),
                encode_root_line_spans(prelude_defs, &root_spans),
            );
        }
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
        //
        // rubash#4185: the deliveries below are the respawned child's
        // fd-table snapshot, and GNU's forked child is born with NO
        // coproc endpoints — coproc_closeall runs at every fork site
        // (execute_cmd.c:1714 execute_in_subshell, :4229 the
        // null-command fork, :4575 the simple-command fork), whatever
        // the payload shape. Run the #4181 severance
        // (FdTable::close_coproc_endpoints_for_subshell) over a clone
        // of the live table and let it decide what the snapshot
        // carries: a coproc arm the snapshot dropped is not delivered,
        // so the child's slot stays closed and a redirect naming it
        // there fails EBADF, as in GNU. Deliveries still take their
        // handles from the LIVE entry (handle identity and cross-slot
        // description sharing are the live table's facts), nothing is
        // retired, and no shared handle is closed — the parent's own
        // endpoints are untouched, so its coproc sees no false EOF.
        let child_fd_snapshot = {
            let mut snapshot = self.fd_table.clone();
            snapshot.close_coproc_endpoints_for_subshell();
            snapshot
        };
        let snapshot_carries_read = |fd: &u32| {
            child_fd_snapshot
                .entries
                .get(fd)
                .map_or(false, |entry| entry.read.is_some())
        };
        let snapshot_carries_write = |fd: &u32| {
            child_fd_snapshot
                .entries
                .get(fd)
                .map_or(false, |entry| entry.write.is_some())
        };
        let mut extra_handles: Vec<crate::fd::HANDLE> = Vec::new();
        let mut fd_shared_dups: HashMap<usize, crate::fd::HANDLE> = HashMap::new();
        for (fd, entry) in &self.fd_table.entries {
            if *fd < 3 || entry.closed {
                continue;
            }
            // Each delivery is (source handle, path label, shared-dup
            // key): File/Coproc endpoints key by their Rc identity so
            // two slots sharing one description share one dup;
            // standard-stream endpoints (rubash#2212 — an `N<&0` /
            // `N>&1` / `N>&2` dup carries no FileFd) key by the
            // standard handle itself and deliver the process standard
            // descriptor GNU's fork would inherit.
            let read_delivery = match &entry.read {
                Some(FdReadEndpoint::File(f)) => Some((
                    f.handle,
                    f.path.to_string_lossy().into_owned(),
                    Rc::as_ptr(f) as usize,
                )),
                Some(FdReadEndpoint::CoprocStdout {
                    fd: coproc_file, ..
                }) if snapshot_carries_read(fd) => Some((
                    coproc_file.handle,
                    coproc_file.path.to_string_lossy().into_owned(),
                    Rc::as_ptr(coproc_file) as usize,
                )),
                Some(endpoint) => endpoint.standard_backing().and_then(|(handle, label)| {
                    // A Windows GUI parent has no standard handle
                    // (process_std_handle == 0); leave the endpoint
                    // unforwarded rather than fail the spawn on a
                    // bad dup. On Unix the handle IS the fd (0 is a
                    // valid descriptor).
                    #[cfg(windows)]
                    if handle == 0 {
                        return None;
                    }
                    Some((handle, label.to_string(), handle as usize))
                }),
                _ => None,
            };
            if let Some((src_handle, src_path, key)) = read_delivery {
                let dup = match fd_shared_dups.get(&key) {
                    Some(&h) => h,
                    None => {
                        let dup = crate::fd::duplicate_handle_inheritable(src_handle)?;
                        fd_shared_dups.insert(key, dup);
                        extra_handles.push(dup);
                        owned_handles.push(dup);
                        dup
                    }
                };
                env_map.insert(format!("__RUBASH_FD_HANDLE_{fd}"), format!("{:#x}", dup));
                env_map.insert(format!("__RUBASH_FD_PATH_{fd}"), src_path);
            }
            let write_delivery = match &entry.write {
                Some(FdWriteEndpoint::File(f)) => Some((
                    f.handle,
                    f.path.to_string_lossy().into_owned(),
                    Rc::as_ptr(f) as usize,
                )),
                Some(FdWriteEndpoint::CoprocStdin {
                    fd: coproc_file, ..
                }) if snapshot_carries_write(fd) => Some((
                    coproc_file.handle,
                    coproc_file.path.to_string_lossy().into_owned(),
                    Rc::as_ptr(coproc_file) as usize,
                )),
                Some(endpoint) => endpoint.standard_backing().and_then(|(handle, label)| {
                    #[cfg(windows)]
                    if handle == 0 {
                        return None;
                    }
                    Some((handle, label.to_string(), handle as usize))
                }),
                _ => None,
            };
            if let Some((src_handle, src_path, key)) = write_delivery {
                let dup = match fd_shared_dups.get(&key) {
                    Some(&h) => h,
                    None => {
                        let dup = crate::fd::duplicate_handle_inheritable(src_handle)?;
                        fd_shared_dups.insert(key, dup);
                        extra_handles.push(dup);
                        owned_handles.push(dup);
                        dup
                    }
                };
                env_map.insert(format!("__RUBASH_FD_WHANDLE_{fd}"), format!("{:#x}", dup));
                env_map.insert(format!("__RUBASH_FD_WPATH_{fd}"), src_path);
            }
            // rubash#3997: a symbolic FullDevice write endpoint has
            // no handle for the deliveries above (its
            // standard_backing is None), so the respawned
            // background child was born with the slot unbound and
            // its external children met EBADF where GNU's fork copy
            // writes ENOSPC. Materialize the device itself for the
            // env channel, exactly as the command-spawn delivery
            // does (collect_external_fd_delivery): an independent
            // open of /dev/full is indistinguishable from the
            // descriptor GNU's fork inherits (redir.c:725; the
            // open_spawn_full_device precedent, rubash#3985). A
            // walk save slot's clone is not a live binding and is
            // never delivered (fd_is_walk_save_slot). Where no
            // device backs the spelling the slot stays
            // undelivered, as before.
            if matches!(entry.write.as_ref(), Some(FdWriteEndpoint::FullDevice))
                && !self.fd_is_walk_save_slot(*fd)
            {
                if let Some(file) =
                    crate::executor::external_redirects::open_spawn_full_device("/dev/full", false)
                {
                    #[cfg(unix)]
                    let raw = {
                        use std::os::unix::io::AsRawFd;
                        file.as_raw_fd()
                    };
                    #[cfg(windows)]
                    let raw = {
                        use std::os::windows::io::AsRawHandle;
                        file.as_raw_handle() as crate::fd::HANDLE
                    };
                    if let Ok(dup) = crate::fd::duplicate_handle_inheritable(raw) {
                        extra_handles.push(dup);
                        owned_handles.push(dup);
                        env_map.insert(format!("__RUBASH_FD_WHANDLE_{fd}"), format!("{:#x}", dup));
                        env_map.insert(format!("__RUBASH_FD_WPATH_{fd}"), "/dev/full".to_string());
                    }
                }
            }
        }

        // rubash#3300: hand the proof fd to the whitelist transport
        // (spawn_whitelisted dup2s each extra onto its own number in
        // pre_exec — a no-op for an already-numbered fd — and the
        // parent's copy closes with the other owned handles after the
        // spawn lands). The mint already cleared CLOEXEC, so the fd
        // survives exec at its number.
        #[cfg(unix)]
        if let Some((proof_fd, _)) = respawn_proof {
            extra_handles.push(proof_fd);
            owned_handles.push(proof_fd);
        }

        // rubash#2356: the composed environment itself is a state
        // channel — the fork-context stamp and the attribute lists
        // scale with this shell's function table, and a single entry
        // past MAX_ARG_STRLEN fails the whole execve (E2BIG) even
        // with the `-c` script already on the file transport. Move
        // oversized entries onto the state-file transport before the
        // spawn; the child's import splices them back.
        let argv_bytes =
            exe.as_os_str().len() + spawn_args.iter().map(|arg| arg.len() + 1).sum::<usize>();
        let (env_entries, state_temp) =
            self.spill_respawn_env_entries(env_map.into_iter().collect(), argv_bytes);
        // rubash#3268 (e): when the session cwd no longer resolves,
        // the spec's anchor fd lets the spawner fchdir the child into
        // the session's inode (fd::spawn_whitelisted) instead of
        // leaving it in this process's cwd. Borrowed from the anchor
        // for the duration of the spawn call below.
        #[cfg(unix)]
        let cwd_anchor_fd = self.session_cwd_anchor_fd();
        #[cfg(not(unix))]
        let cwd_anchor_fd: Option<i32> = None;
        let spawn_result = crate::fd::spawn_whitelisted(&crate::fd::WhitelistedSpawn {
            program: exe.clone(),
            args: spawn_args,
            env: env_entries,
            std_handles,
            extra_handles,
            // G-02: the respawned child starts in the session cwd.
            cwd: Some(self.shell_state.cwd.clone()),
            cwd_anchor_fd,
            // rubash#2241: under job control the respawned async child
            // leads its own process group (GNU jobs.c:2390 make_child),
            // matching the pgrp the job table records for it below.
            new_process_group: crate::builtins::set::shell_option_enabled(
                &self.shell_state.env_vars,
                "monitor",
            ),
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
                if let Some(path) = &state_temp {
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
            if crate::executor::support_names::is_shell_name(prefix) {
                return false;
            }
        }
        if crate::executor::builtin_names::is_shell_builtin_name(name)
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
            if crate::executor::support_names::is_shell_name(name) {
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
        // rubash#2585: arm the probe through the RAII guard, not a
        // bare set(true)/set(false) pair — a NESTED probe (a comsub or
        // async gate inside the probed expansion arming its own probe
        // on this same executor) used to run its unconditional
        // set(false) and silently disarm the outer probe, re-admitting
        // the doubled `no match` report and the armed evalerror
        // discard this probe exists to suppress. The guard restores
        // the PRIOR value on drop, the SpeculativeExpansionGuard shape
        // (pipeline_exec.rs).
        let probe = {
            let _probe_guard = ExpansionProbeGuard::new(&self.expansion_probe_active);
            self.expand_command_words(command)
        };
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
            || crate::executor::builtin_names::is_shell_builtin_name(name)
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
        if crate::executor::external_inner::is_enoexec_candidate(&program) {
            return None;
        }
        // rubash#1905: a word naming a synthetic /proc operand needs
        // the spawn-boundary materialization the foreground external
        // path owns (proc_operands); the direct spawn has no operand
        // plumbing, so the shape keeps the wrapper, whose child
        // serves the operand there — the same decline family as
        // process substitutions above.
        if crate::executor::proc_operands::args_name_synthetic_proc(&expanded.words) {
            return None;
        }
        Some((program, expanded.words))
    }

    fn background_command_source(&self, command: &CommandNode) -> String {
        let mut source = self.background_function_prelude();
        source.push_str(&self.background_trap_rearm_source());
        source.push_str(&bash_command_source_text(command));
        source
    }

    /// rubash#2982: re-arm the DEBUG/ERR traps the wrapper child starts
    /// without (reset_traps_for_background_wrapper) — after the function
    /// prelude, so the handler functions exist when the traps first
    /// fire, and before the body, so the body's commands run traced
    /// exactly as GNU's forked async child (execute_cmd.c:1576
    /// execute_in_subshell). Only the traps the parent would have
    /// inherited are re-armed: DEBUG under functrace, ERR under
    /// errtrace (trap.c:1467 reset_or_restore_signal_handlers /
    /// trap.c:1526 reset_signal_handlers). The
    /// re-arm is a single line with no newline, so the prelude
    /// line-offset math (prelude_newlines) is unaffected.
    fn background_trap_rearm_source(&self) -> String {
        let env_vars = &self.shell_state.env_vars;
        let mut rearm = String::new();
        if crate::builtins::set::shell_option_enabled(env_vars, "functrace") {
            if let Some(action) = crate::builtins::trap::get_trap_action(env_vars, "DEBUG")
                .filter(|action| !action.is_empty())
            {
                rearm.push_str(&format!(
                    "trap {} DEBUG; ",
                    crate::executor::parameter_replace::shell_single_quote_assignment_value(
                        &action
                    )
                ));
            }
        }
        if crate::builtins::set::shell_option_enabled(env_vars, "errtrace") {
            if let Some(action) = crate::builtins::trap::get_trap_action(env_vars, "ERR")
                .filter(|action| !action.is_empty())
            {
                rearm.push_str(&format!(
                    "trap {} ERR; ",
                    crate::executor::parameter_replace::shell_single_quote_assignment_value(
                        &action
                    )
                ));
            }
        }
        rearm
    }

    /// rubash#1095: `background_command_source` for an absorbed `( list )`
    /// body — the inner sequence rides the child's `-c` source because the
    /// spawn itself realized the subshell level (GNU execute_cmd.c:701).
    fn background_command_source_for_body(&self, body: &[CommandNode]) -> String {
        let mut source = self.background_function_prelude();
        source.push_str(&self.background_trap_rearm_source());
        source.push_str(&bash_command_sequence_text(body));
        source
    }

    /// Exportable-function prelude shared by both background source forms
    /// (print_cmd-style `name() { ...; }; ` definitions ahead of the body).
    /// rubash#2543: iterates background_prelude_function_entries, which
    /// excludes names whose identical `BASH_FUNC_<name>%%` carrier already
    /// rides this process's environment — every respawn builder seeds the
    /// child's env from that process env, so those functions are defined
    /// exactly once in the child by the startup import (GNU
    /// variables.c:397-446 initialize_shell_variables, the
    /// parse_and_execute SEVAL_FUNCDEF call at :418). GNU's forked async
    /// child defines nothing (make_child copies the table whole), so a
    /// prelude re-definition of an env-imported name is a redefinition —
    /// one that trips the readonly guard (execute_cmd.c:6325-6331) for
    /// readonly-marked functions.
    pub(in crate::executor) fn background_function_prelude(&self) -> String {
        let mut source = String::new();
        for (name, body) in self.background_prelude_function_entries() {
            source.push_str(name);
            source.push_str("() { ");
            let body_text = bash_command_sequence_text(&body.commands);
            source.push_str(&body_text);
            // print_cmd.c:1520-1529 semicolon(): no `;' after a body
            // whose last command printed as `... &` — parse.y:1275/1290
            // only admits newline_list after `&`, and `& ; }` is the
            // syntax error that killed respawned background children
            // (rubash#294). `&\n}` is the valid GNU spelling.
            // Same delimiter-line rule when the body ends with a
            // heredoc body (rubash#2388's closer logic in
            // function_command_source_text): the delimiter must stand
            // alone on its line, so `DELIM; }` leaves the heredoc
            // unterminated and the respawned child dies at parse
            // (rubash#2212 cell 118: bats' `usage()` heredoc broke
            // every async respawn out of bats-main's table).
            if body_text.ends_with('&')
                || crate::executor::command_text::render_ended_with_heredoc()
            {
                source.push_str("\n}; ");
            } else {
                source.push_str("; }; ");
            }
        }
        source
    }

    /// rubash#2543: `BASH_FUNC_<name>%%` carriers present in this
    /// process's environment that carry the SAME definition as the
    /// CURRENT function table. The carrier text and the table's
    /// exportstr need not be byte-identical (GNU's single-line export
    /// form vs rubash's multiline re-render), so both sides go through
    /// the SAME canonical printer: parse the carrier with the
    /// ShellShock-guarded import parser, render it and the table body
    /// through exported_function_env_value, and compare those. A STALE
    /// carrier (the parent redefined the function after importing it)
    /// renders differently and does not skip: the prelude's fresh
    /// rendering wins, exactly the body GNU's fork would carry.
    fn env_carried_function_exports(&self) -> std::collections::HashSet<String> {
        let mut carried = std::collections::HashSet::new();
        for (env_name, value) in crate::executor::env_helpers::process_env_pairs() {
            let Some(name) = imported_function_name(&env_name) else {
                continue;
            };
            let Some(body) = self.shell_state.functions.get(name) else {
                continue;
            };
            let Some((carrier_commands, carrier_redirects)) = parse_exported_function_body(&value)
            else {
                continue;
            };
            let def_redirects = self
                .shell_state
                .function_def_infos
                .get(name)
                .map(|info| info.def_redirects.as_slice())
                .unwrap_or(&[]);
            if exported_function_env_value(&carrier_commands, &carrier_redirects)
                == exported_function_env_value(&body.commands, def_redirects)
            {
                carried.insert(name.to_string());
            }
        }
        carried
    }

    /// The function-table entries background_function_prelude re-renders
    /// into a respawned child: every function with an importable name
    /// (rubash#441: GNU forks inherit the FULL table, not just
    /// `export -f` names), minus the env-carried names above (rubash#2543:
    /// the child's startup import defines those once, variables.c:418).
    /// The FORK_FN_SPANS transport and the prelude-def counts at the
    /// spawn sites iterate exactly this set, so the child's line-span
    /// reseeding and its line-offset math stay coherent with the rendered
    /// source.
    pub(super) fn background_prelude_function_entries(&self) -> Vec<(&String, &FunctionBody)> {
        let env_carried = self.env_carried_function_exports();
        self.shell_state
            .functions
            .iter()
            .filter(|(name, _)| {
                is_exportable_function_name(name) && !env_carried.contains(name.as_str())
            })
            .collect()
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
}
