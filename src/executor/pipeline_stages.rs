use super::redirection::OutputTarget;
use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;
use crate::executor::pipeline_exec::command_is_compound_pipeline_stage;

/// niubash#345: adopt a duplicated host handle as a child's `Stdio`. The
/// caller hand-duplicates the pipeline write end so the child owns a private
/// copy (the parent keeps its own until the producer stage returns).
fn stdio_from_fd_handle(handle: crate::fd::HANDLE) -> Stdio {
    #[cfg(windows)]
    {
        use std::os::windows::io::FromRawHandle;
        unsafe { Stdio::from_raw_handle(handle as *mut std::ffi::c_void) }
    }
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        unsafe { Stdio::from_raw_fd(handle) }
    }
}

impl Executor {
    pub(in crate::executor) fn execute_lastpipe_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
    ) -> Result<(String, String, i32), ExecuteError> {
        let old_stdin = self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
        let old_stdin_offset = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .cloned();
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input.to_string());
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());

        let saved_stdout_capture = self.stdout_capture.take();
        let saved_stderr_capture = self.stderr_capture.take();
        self.stdout_capture = Some(Vec::new());
        self.stderr_capture = Some(Vec::new());

        // The element's own output redirections stay on the command: the
        // lastpipe element runs in the current shell, and its `>file` /
        // `>/dev/null` must rebind fd 1 for the body the way GNU's
        // do_redirection_internal does inside the element (see
        // execute_compound_pipeline_stage).
        // Direct-stdout builtins in the stage consult the thread-local
        // capture, which belongs to an enclosing capture when this pipeline
        // runs inside one; give the stage its own capture.
        let (thread_captured, result) = crate::executor::shell_options::capture_stdout(|| {
            // GNU execute_pipeline hands the lastpipe element to
            // execute_command in the current shell, where cm_group
            // dispatches to execute_group_command (execute_cmd.c:1104).
            // execute_command's compound dispatch has no brace-group arm —
            // the AST loop peels groups into execute_brace_group_pipeline
            // first — so a typed group node here fell to
            // execute_empty_words_command and dropped the body
            // (rubash#1320). Dispatch the group to that port the way
            // execute_compound_pipeline_stage does.
            if command.brace_group.is_some() {
                self.execute_brace_group_pipeline(command).map(|_| ())
            } else {
                self.execute_command(command)
            }
        });
        let mut output = self.stdout_capture.take().unwrap_or_default();
        output.extend_from_slice(&thread_captured);
        let stderr = self.stderr_capture.take().unwrap_or_default();
        self.stdout_capture = saved_stdout_capture;
        self.stderr_capture = saved_stderr_capture;
        // In-shell stage: the cursor visible on self is the element's real
        // fd-0 consumption within `input` (execute_cmd.c:2758 lastpipe).
        let consumed = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        self.pipeline_stdin_consumed.set(Some(consumed));
        restore_optional_env_var(&mut self.shell_state.env_vars, FUNCTION_STDIN, old_stdin);
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FUNCTION_STDIN_OFFSET,
            old_stdin_offset,
        );
        // GNU execute_cmd.c:2758: the lastpipe stage runs in the current
        // shell.  `exit N` must therefore exit the current shell, not just
        // set the pipeline's exit status.  Convert ExitCode to LastpipeExit
        // so it propagates past the pipeline-status match in execute_ast_inner.
        match result {
            Ok(()) => {}
            Err(ExecuteError::ExitCode(code)) => {
                self.exit_code = code;
                return Err(ExecuteError::LastpipeExit(code));
            }
            // niubash#345: the lastpipe element is a shell-side producer too;
            // a broken pipe terminates it with the SIGPIPE status (141).
            Err(ExecuteError::BrokenPipe) => {
                self.exit_code = 141;
            }
            Err(error) => return Err(error),
        }

        Ok((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output),
            crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
            self.last_exit_code(),
        ))
    }

    pub(in crate::executor) fn execute_compound_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
        streaming_stdin: bool,
        live_stdin: bool,
    ) -> Result<(String, String, i32), ExecuteError> {
        // G-02: no process-cwd save/restore — the stage subshell is a clone
        // with its own session cwd field.
        let mut subshell = self.command_substitution_executor();
        // GNU execute_cmd.c:6373 do_piping: an element that is not the
        // rightmost is forked with dup2(pipe_out, 1) — the pipe REPLACES
        // whatever fd 1 the enclosing context bound (a group/function
        // call's `>file` redirect included). The stage clone inherits
        // the context's fd-1 entry verbatim, so without this rebind the
        // body's writes follow the enclosing binding and never reach
        // the stage capture — the same rebind the command-substitution
        // child performs (command_substitution.rs, subst.c:7320). The
        // rightmost element gets NO_PIPE: its fd 1 stays the context
        // binding, which is the pipeline's own stdout (rubash#1807).
        if !self.pipeline_final_stage.get() {
            subshell.fd_table.entries.insert(
                1,
                crate::executor::fd_table::FdEntry {
                    read: None,
                    write: Some(FdWriteEndpoint::Stdout),
                    closed: false,
                    dynamic: false,
                },
            );
        }
        // A compound pipeline element passes execute_in_subshell, which
        // resets loop_level to 0 (execute_cmd.c:1642, Posix interp 842):
        // break/continue inside `{ break; } | cat` are "only meaningful"
        // there even when the pipeline sits in a loop — unlike a SIMPLE
        // element, which inherits the depth (rubash#1064).
        subshell.shell_state.inherited_loop_depth = 0;
        // A pipeline member runs in a subshell: caught signal traps reset to
        // the inherited disposition (execute_cmd.c subshell trap reset), so
        // only traps set inside the member run at its exit.
        crate::builtins::trap::reset_for_subshell(&mut subshell.shell_state.env_vars);
        // GNU execute_cmd.c execute_pipeline (2702-2708 left elements,
        // 2722-2723 rightmost) propagates the pipeline command's
        // CMD_IGNORE_RETURN into EVERY element, and a group command pushes
        // it into its inner list (1104-1108). The stage subshell therefore
        // inherits the parent's errexit suppression verbatim: at top level
        // (ignore_return clear) `{ false; echo x; } | cat` dies on `false`,
        // while under `!`/if/while/&&/||/comsub suppression the same group
        // runs to completion (set-e1.sub:40 prints `A 1`).
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input.to_string());
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        // rubash#761: a streaming tail's body reads fd 0 (the real OS pipe
        // bound by `execute_external_prefix_streaming_tail`). Leave
        // FUNCTION_STDIN ABSENT so `read`/`mapfile` inside the body pull the
        // live pipe instead of short-circuiting to an empty mirror
        // (read_io.rs:361-363); the subshell inherits the fd-0 file endpoint
        // through the cloned fd table.
        // rubash#1873: the same absence when the element is stage 0 and
        // its fd 0 is the shell's live fd 0 (live_stdin) — the body's
        // readers pull the inherited handle / file endpoint the clone's
        // fd table carries.
        if streaming_stdin || live_stdin {
            subshell.shell_state.env_vars.remove(FUNCTION_STDIN);
            subshell.shell_state.env_vars.remove(FUNCTION_STDIN_OFFSET);
        }
        // rubash#2003: when the element's own fd-0 winner is an fd alias
        // (`<&N`, `< /dev/fd/N`), GNU binds the element's fd 0 to that
        // descriptor (execute_cmd.c:2620 execute_pipeline forks the
        // element with the shell's descriptors; redir.c
        // do_redirection_internal dups the source onto fd 0) — the
        // descriptor is the element's ONLY stdin channel. Arming the
        // FUNCTION_STDIN mirror with the resolved payload as well makes
        // mirror readers drain the mirror and then fall through to the
        // endpoint, serving the same bytes twice (a stage-0 read loop
        // iterated its input twice), so bind the clone's fd 0 to the
        // source endpoint — dup shares the open file description — and
        // drop the mirror, the live_stdin treatment above. A failed dup
        // (unbound source) keeps the mirror fallback.
        if let Some(alias_fd) = self.stage_fd_alias_source(command) {
            if subshell.fd_table.dup_input(0, alias_fd).is_ok() {
                subshell.shell_state.env_vars.remove(FUNCTION_STDIN);
                subshell.shell_state.env_vars.remove(FUNCTION_STDIN_OFFSET);
            }
        }
        let raws_aligned = command.assignment_raws.len() == command.assignments.len();
        for (index, (name, value)) in command.assignments.iter().enumerate() {
            let (base_name, _) = assignment_name_and_append(name);
            let raw = raws_aligned
                .then(|| command.assignment_raws.get(index))
                .flatten()
                .map(String::as_str);
            let expanded_value = subshell.expand_assignment_value_with_raw(name, value, raw);
            subshell
                .shell_state
                .env_vars
                .insert(base_name.to_string(), expanded_value);
        }

        // rubash#1605 (residual of #1454): when this stage is the
        // pipeline's final member and its fd 1 IS the shell's own stdout
        // (the final_member_stdout_streams_to_host predicate: no output
        // redirect on the element, fd 1 open on the Stdout endpoint, no
        // enclosing capture), GNU execute_cmd.c execute_pipeline runs the
        // element with the inherited stdout — `echo hi | { readlink
        // /proc/self/fd/1; }` names the shell's stdout target. An fd-1
        // capture here would resolve the body's external children to the
        // capture pipe instead (a fresh pipe), so skip both capture
        // layers and let the body write straight to the live endpoint;
        // the stage's returned stdout stays empty. The stderr capture is
        // unaffected.
        let stream_final_stdout = self.pipeline_final_stage.get()
            && self.pipeline_stdout_stream.is_none()
            && self.final_member_stdout_streams_to_host(command);
        if !stream_final_stdout {
            subshell.stdout_capture = Some(Vec::new());
        }
        subshell.stderr_capture = Some(Vec::new());

        // GNU execute_cmd.c execute_pipeline + redir.c do_redirection_internal:
        // inside the element's subshell the pipe is bound to fd 1 first and
        // the element's own redirections run after it — `( inner ) >/dev/null`
        // rebinds fd 1 for the whole body, so an inner `3>&1` duplicates the
        // null device, not the pipe. Keep redirect_out/append on the stage
        // command so the compound-redirect propagation applies them inside.
        // Builtins that write the process stdout directly (notably the exec
        // builtin's printenv/external fallback via io::stdout()) bypass the
        // subshell's stdout_capture field, so their output would leak past a
        // downstream pipe element. Wrap the stage in the same thread-local
        // stdout capture execute_builtin_pipeline_stage uses and merge both
        // capture sources.
        let saved_capture = if stream_final_stdout {
            None
        } else {
            let saved = crate::executor::shell_options::begin_stdout_capture();
            // niubash#345: this element is the shell-side producer when the
            // driver armed `pipeline_stdout_stream`; give its capture frame the
            // pipe target so the body streams (nested frames still capture).
            crate::executor::shell_options::set_capture_stream(self.pipeline_stdout_stream);
            Some(saved)
        };

        let result = if command.brace_group.is_some() {
            subshell.execute_brace_group_pipeline(command).map(|_| ())
        } else {
            subshell.execute_command(command)
        };
        // A failglob-aborted redirect expansion must escape the stage
        // boundary (GNU subst.c:12663-12668 DISCARD): the forked child owns
        // the element's do_redirections, so the parent never re-applies
        // them. Converting the abort into a stage status here would let the
        // parent's routing phase re-expand the target, print `no match' a
        // second time, and open the literal — propagate instead.
        if subshell.shell_state.redirect_failglob_seen.replace(false) {
            if let Err(error) = result {
                // rubash#846: the thread-local stdout capture above is
                // active — restore it before propagating so later stdout
                // writes in the script are not swallowed into the
                // orphaned buffer.
                if let Some(saved) = saved_capture {
                    crate::executor::shell_options::restore_stdout_capture(saved);
                }
                return Err(error);
            }
        }
        // GNU execute_cmd.c:1761-1763 (execute_in_subshell): a pipeline
        // element is a forked subshell whose exit — implicit after the body
        // as much as via `exit` — runs the subshell's EXIT trap
        // (subshell_exit -> run_exit_trap; trap4.sub's group without a
        // trailing `exit` still prints its "in trap EXIT"). The trap table
        // was reset at entry (reset_for_subshell above), so only traps the
        // body itself installed can fire; the explicit-`exit` case already
        // consumed the action through the exit builtin, and take_exit_trap
        // leaves nothing for this second run.
        let body_status = subshell.last_exit_code();
        let trap_result = subshell.run_exit_trap_for_status(body_status);
        // rubash#1605: a streamed final stage installed no thread capture
        // (saved_capture is None); its bytes already went to the live
        // stdout, so there is nothing to take or restore.
        let mut thread_output = if saved_capture.is_some() {
            crate::executor::shell_options::take_stdout_capture()
        } else {
            Vec::new()
        };
        let mut output = subshell.stdout_capture.take().unwrap_or_default();
        let stderr = subshell.stderr_capture.take().unwrap_or_default();

        if let Some(saved) = saved_capture {
            crate::executor::shell_options::restore_stdout_capture(saved);
        }
        // rubash#1141: the stage's output assembles field-sink bytes
        // first, thread-sink bytes after — snapshot the prefix totals so
        // the timing recorder can place thread arrivals behind them.
        crate::executor::shell_options::capture_timing_set_prefix(&output);
        output.append(&mut thread_output);

        let status = match (result, trap_result) {
            (Ok(()), Ok(trap_status)) => trap_status,
            (Err(ExecuteError::ExitCode(code)), Ok(_)) => code,
            (Err(ExecuteError::ExpansionFailure(code)), Ok(_)) => code,
            // rubash#1423: a pipeline element runs in a subshell, so a
            // `return` raised by the element body ends the ELEMENT with
            // that status — it must not unwind the enclosing function
            // (GNU: `{ return 3; } | cat` inside f leaves f running).
            (Err(ExecuteError::Return(code)), Ok(_)) => code,
            // niubash#345: the stage's fd-1 write hit a broken pipe (the
            // downstream consumer exited): the element is SIGPIPE-killed, so
            // report 141 rather than unwinding the whole pipeline.
            (Err(ExecuteError::BrokenPipe), Ok(_)) => 141,
            (_, Err(error)) | (Err(error), _) => return Err(error),
        };
        // The subshell's FUNCTION_STDIN cursor is the element's fd-0
        // consumption within `input`; report it for the driver writeback.
        let consumed = subshell
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        self.pipeline_stdin_consumed.set(Some(consumed));

        Ok((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output),
            crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
            status,
        ))
    }

    pub(in crate::executor) fn execute_function_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
        live_stdin: bool,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        let Some(name) = command.words.first() else {
            return Ok(Some((String::new(), String::new(), 0)));
        };
        let expanded_name = self.expand_word(name);
        let Some(function_name) = self
            .function_name_for_command_word(&expanded_name)
            // Bash posix-mode notes (item 19): POSIX mode never executes a
            // function whose name contains a slash; the stage falls through
            // to the external command search.
            .filter(|function_name| !(self.posix_mode_enabled() && function_name.contains('/')))
        else {
            return Ok(None);
        };
        // GNU subst.c process_substitute: a <( ) word in the command's
        // argument list materializes before the command runs
        // (procsub.tests: count_lines <(date) | ... must pass the
        // substitution result as \$1, not the raw <(date) text). The
        // top-level function path does this through
        // command_with_process_substitution_files; pipeline stages skipped
        // it, leaving $1 as the literal <(date) string.
        // rubash#2003: the materialization below rewrites an fd-0 alias
        // redirect (`<&N` on a buffered source) into a temp-file path
        // for the external-spawn layer and CONSUMES the source endpoint
        // (command_with_process_substitution_files' redirect snapshot).
        // A function element is never spawned: its body resolves the
        // alias itself, so the consumption must not stand — snapshot
        // the alias source's offset (and its per-fd mirror-offset key)
        // and restore both after the call.
        let alias_snapshot = self.stage_fd_alias_source(command).and_then(|alias_fd| {
            self.fd_table
                .input_snapshot_bytes(alias_fd)
                .map(|(_, offset)| (alias_fd, offset))
        });
        let alias_offset_key = alias_snapshot.map(|(alias_fd, _)| {
            (
                alias_fd,
                self.shell_state
                    .env_vars
                    .get(&fd_stdin_offset_key(alias_fd))
                    .cloned(),
            )
        });
        let (mut materialized_command, procsub_files) =
            self.command_with_process_substitution_files(command)?;
        if let Some((alias_fd, offset)) = alias_snapshot {
            self.fd_table.set_input_offset(alias_fd, offset);
            if let Some((key_fd, saved)) = &alias_offset_key {
                restore_optional_env_var(
                    &mut self.shell_state.env_vars,
                    &fd_stdin_offset_key(*key_fd),
                    saved.clone(),
                );
            }
            // Undo the snapshot's redirect rewrite too: the temp path
            // would stage a second text channel inside the body (a read
            // loop then iterates the alias bytes twice — once staged,
            // once through the fd-0 dup below). Only the alias spelling
            // is restored; a `<( )` target's rewrite is a real
            // materialization the body depends on.
            if let (Some(original), Some(rewritten)) = (
                command.redirect_in.as_ref(),
                materialized_command.redirect_in.as_mut(),
            ) {
                if redirect_target_fd(&original.target) == Some(alias_fd)
                    && rewritten.target != original.target
                {
                    rewritten.target = original.target.clone();
                }
            }
        }
        // GNU expands a pipeline element's words through the same word-list
        // machinery as any simple command (execute_cmd.c:624
        // execute_command_internal -> execute_simple_command ->
        // subst.c:13219 expand_word_list_internal): "$@" becomes a LIST of
        // individually quoted words (subst.c:10691 case '@'), unquoted $*/$@
        // field-split, and unquoted patterns pathname-expand — none of that
        // depends on pipeline membership. The old per-word `expand_word`
        // joined "$@" into ONE space-joined argument (rubash#330: FFmpeg's
        // `map ... "$@" | awk` lost every per-arg word), left `g*.g`
        // unexpanded, and produced an empty word for an empty "$@".
        // expand_stage_tail_words is the same argv builder the external and
        // builtin stages use (field split + marker restore + glob).
        let args = self.expand_stage_tail_words(&materialized_command);
        let mut call = materialized_command.clone();
        call.words = std::iter::once(function_name.clone())
            .chain(args.iter().cloned())
            .collect();
        call.redirect_out = None;
        call.append = None;

        // G-02: no process-cwd save/restore (stage subshell clone, as above).
        let mut subshell = self.command_substitution_executor();
        // GNU execute_cmd.c:6373 do_piping: an element that is not the
        // rightmost is forked with dup2(pipe_out, 1) — the pipe REPLACES
        // whatever fd 1 the enclosing context bound (a function/group
        // call's `>file` redirect included). The stage clone inherits
        // the context's fd-1 entry verbatim, so without this rebind the
        // body's writes follow the enclosing binding and never reach
        // the stage capture — the same rebind the command-substitution
        // child performs (command_substitution.rs, subst.c:7320). The
        // rightmost element gets NO_PIPE: its fd 1 stays the context
        // binding, which is the pipeline's own stdout (rubash#1807).
        if !self.pipeline_final_stage.get() {
            subshell.fd_table.entries.insert(
                1,
                crate::executor::fd_table::FdEntry {
                    read: None,
                    write: Some(FdWriteEndpoint::Stdout),
                    closed: false,
                    dynamic: false,
                },
            );
        }
        // A pipeline member runs in a subshell: caught signal traps reset to
        // the inherited disposition (execute_cmd.c subshell trap reset), so
        // only traps the function body itself installs run at its exit.
        crate::builtins::trap::reset_for_subshell(&mut subshell.shell_state.env_vars);
        // Function pipeline stages are compound command bodies for errexit
        // (execute_cmd.c execute_function: the body inherits the element's
        // CMD_IGNORE_RETURN), so the subshell keeps the parent's
        // suppress_errexit like the compound-stage path above.
        // rubash#1873: when the element is stage 0 and its fd 0 is the
        // shell's live fd 0 (live_stdin), the mirror stays ABSENT — the
        // body's readers pull the inherited handle / file endpoint the
        // clone's fd table carries (the #761 streaming treatment). Any
        // ambient mirror the clone inherited is removed with it: in
        // this state the live descriptor, not the buffer, is fd 0.
        // rubash#2003: when the element's own fd-0 winner is an fd alias
        // (`<&N`, `< /dev/fd/N`), the descriptor is the element's only
        // stdin channel (execute_cmd.c:2620 execute_pipeline; redir.c
        // do_redirection_internal dups the source onto the element's
        // fd 0). Bind the clone's fd 0 to the source endpoint — dup
        // shares the open file description — and keep the mirror absent
        // like the live_stdin treatment: a body command reading the
        // descriptor channel (`cat`, an external child) otherwise sees
        // an unbound fd 0 and reads EOF while only mirror readers
        // (`read`) are served. A failed dup (unbound source) keeps the
        // mirror fallback.
        let alias_bound = self
            .stage_fd_alias_source(command)
            .is_some_and(|alias_fd| subshell.fd_table.dup_input(0, alias_fd).is_ok());
        if live_stdin || alias_bound {
            subshell.shell_state.env_vars.remove(FUNCTION_STDIN);
            subshell.shell_state.env_vars.remove(FUNCTION_STDIN_OFFSET);
        } else {
            subshell
                .shell_state
                .env_vars
                .insert(FUNCTION_STDIN.to_string(), input.to_string());
            subshell
                .shell_state
                .env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        }

        // rubash#1605: same final-member rule as the compound stage
        // above — a final function element whose fd 1 IS the shell's own
        // stdout runs uncaptured, so external children in the body
        // inherit the real fd 1 instead of a fresh capture pipe.
        let stream_final_stdout = self.pipeline_final_stage.get()
            && self.pipeline_stdout_stream.is_none()
            && self.final_member_stdout_streams_to_host(command);
        if !stream_final_stdout {
            subshell.stdout_capture = Some(Vec::new());
        }
        subshell.stderr_capture = Some(Vec::new());
        // GNU: a pipeline element's forked child runs the element's simple
        // command itself, so the command's assignment words are processed
        // in the child — subst.c:13153 expand_word_list_internal picks
        // assign_in_env (variables.c:3536) to build the temporary_env, and
        // execute_function (execute_cmd.c:5181) receives it as the
        // function's variable context via push_context
        // (execute_cmd.c:5233/5252). The non-pipeline invocation path
        // (execute_function_command_invocation) performs exactly this
        // apply on the executing executor; the stage clone IS the forked
        // child, so the apply belongs on the clone, before the body runs
        // (rubash#1765: the stage used to drop the prefix assignments
        // entirely and the body saw the outer values).
        let assignment_raws = (materialized_command.assignment_raws.len()
            == materialized_command.assignments.len())
        .then(|| materialized_command.assignment_raws.as_slice());
        let (temporary_assignments, assignments_failed) = subshell
            .apply_temporary_assignments(&materialized_command.assignments, assignment_raws);
        // GNU variables.c push_context: the call tempenv becomes the
        // function's variable context — recorded so a posix special
        // builtin's merged tempenv inside the body can tell a context
        // binding from an ordinary command tempenv binding (mirrors
        // execute_function_command_invocation).
        subshell.function_tempenv_names.push(
            materialized_command
                .assignments
                .iter()
                .map(|(name, _)| assignment_name_and_append(name).0.to_string())
                .collect(),
        );
        // Direct-stdout builtins in the function consult the thread-local
        // capture, which belongs to an enclosing capture when this stage
        // runs inside one; give the call its own capture.
        // The failure classes mirror execute_function_command_invocation:
        // a posix-fatal tempenv failure or a DISCARD-class expansion error
        // skips the body with status 1 (in the child both end the element);
        // any other failure (default-mode readonly) only diagnosed — the
        // window stays partially applied and the body still runs.
        // rubash#1605: short of that fatal skip, a final stage whose fd 1
        // IS the shell's own stdout runs the body uncaptured (no
        // thread-local capture frame), exactly as in the compound stage.
        let (thread_captured, result) = if assignments_failed
            && (subshell.posix_assignment_failure_is_fatal() || subshell.evalerror_pending.get())
        {
            subshell.exit_code = 1;
            (Vec::new(), Err(ExecuteError::ExpansionFailure(1)))
        } else if stream_final_stdout {
            (
                Vec::new(),
                subshell.execute_function(&function_name, &args, &call),
            )
        } else {
            crate::executor::shell_options::capture_stdout_streamed(
                self.pipeline_stdout_stream,
                || subshell.execute_function(&function_name, &args, &call),
            )
        };
        // The window dies with the command, before the element's EXIT
        // trap runs (the trap is subshell teardown, after the command
        // completed and its temp vars were popped). The clone is dropped
        // after the stage, so this restore only keeps the clone's own
        // bookkeeping balanced for the trap run below.
        subshell.restore_temporary_assignments(temporary_assignments);
        subshell.function_tempenv_names.pop();
        // GNU execute_cmd.c:1761-1763 (execute_in_subshell): a pipeline
        // element is a forked subshell whose exit — implicit after the body
        // as much as via `exit` — runs the subshell's EXIT trap. The trap
        // table was reset at entry above, so only traps the body installed
        // can fire (rubash#1104: `f | cat` with `trap ... EXIT` inside f
        // carries the trap output down the pipe). Run before harvesting the
        // captures so both the field buffer and the thread-local capture
        // (still open) receive the trap output.
        // (No trampoline fold here, unlike the builtin stage: a no_args
        // DISCARD raised inside a function element's body keeps the
        // pre-set EX_BADUSAGE 2 — GNU 5.3.20 probe: `f() { exit 1 2; };
        // f | cat` reports PIPESTATUS 2. Only a builtin that IS the
        // element folds to 1.)
        let body_status = subshell.last_exit_code();
        let (trap_thread_captured, trap_result) = if stream_final_stdout {
            (Vec::new(), subshell.run_exit_trap_for_status(body_status))
        } else {
            crate::executor::shell_options::capture_stdout_streamed(
                self.pipeline_stdout_stream,
                || subshell.run_exit_trap_for_status(body_status),
            )
        };
        let mut output = subshell.stdout_capture.take().unwrap_or_default();
        // rubash#1141: field-sink bytes precede the thread-sink bytes in
        // the assembled output — snapshot the prefix for the recorder.
        crate::executor::shell_options::capture_timing_set_prefix(&output);
        output.extend_from_slice(&thread_captured);
        output.extend_from_slice(&trap_thread_captured);
        // The call's ORDERED redirect list (call.redirects) still carries
        // the stage's output redirects, and execute_function's redirect
        // walk applied them onto the subshell's fd table before the body
        // ran -- GNU execute_cmd.c semantics: an element's redirections
        // run inside the element after the pipe binds fd 1, so the body's
        // writes already landed on the opened targets. Tell the pipeline
        // loop this stage owns its stream placement; the routing walk
        // would re-open `>f` and TRUNCATE what the body just wrote
        // (rubash#412: `printf x | f > OUT` lost pwd's line and the
        // piped payload). Brace-group stages get the same skip via
        // command_is_compound_pipeline_stage; external stages that wire
        // both fds onto one open file set this flag themselves.
        if command_has_output_redirects(command) {
            self.pipeline_stage_fds_pre_wired.set(true);
        }
        let stderr = subshell.stderr_capture.take().unwrap_or_default();

        let status = match (result, trap_result) {
            (Ok(()), Ok(trap_status)) => trap_status,
            (Err(ExecuteError::ExitCode(code)), Ok(_)) => code,
            // rubash#1542: an expansion-phase DISCARD harvested from the
            // element clone keeps its pre-set class status (EX_BADUSAGE
            // 2 for a failed `${var:=word}` bind) — except when this
            // pipeline itself runs inside a command-substitution body,
            // where the body's parse_and_execute catch
            // (builtins/evalstring.c:423-426) folds it to
            // EXECUTION_FAILURE like every DISCARD in that frame.
            (Err(ExecuteError::ExpansionFailure(code)), Ok(_)) => {
                if code == 2 && self.discard_fold_frame() {
                    1
                } else {
                    code
                }
            }
            // rubash#1423: the element runs in a subshell — a `return`
            // raised by the element body ends the element with that
            // status instead of unwinding the enclosing function.
            (Err(ExecuteError::Return(code)), Ok(_)) => code,
            // niubash#345: a function pipeline element writing past the
            // consumer's exit is SIGPIPE-killed (status 141).
            (Err(ExecuteError::BrokenPipe), Ok(_)) => 141,
            (_, Err(error)) | (Err(error), _) => return Err(error),
        };
        let consumed = subshell
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        self.pipeline_stdin_consumed.set(Some(consumed));
        // Drain output substitutions (none expected for <( ) arguments) and
        // delete the materialized input temp files after the call.
        self.finish_process_substitutions(procsub_files)?;
        Ok(Some((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output),
            crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
            status,
        )))
    }

    /// Runs a builtin command inside a subshell for a pipeline stage and
    /// captures its stdout/stderr/status. Bash runs every pipeline element in
    /// a subshell, so special builtins like `set`, `export`, `type`, `trap`
    /// work as pipeline stages without leaking state to the parent.
    pub(in crate::executor) fn execute_builtin_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
        streaming_stdin: bool,
        live_stdin: bool,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        let Some(name) = command.words.first() else {
            return Ok(None);
        };
        // GNU bash field-splits the expanded first word before command
        // dispatch, so `v="echo hi"; $v | cat` runs the builtin echo in this
        // stage. expand_word alone would classify the joined "echo hi" string
        // as a non-builtin name and fall through to the external stage.
        let first_raw = command
            .word_metadata
            .first()
            .map(|metadata| metadata.raw.as_str());
        let expanded_first = self.expand_command_word(command, 0, name, first_raw);
        let Some(expanded_name) = expanded_first.first() else {
            return Ok(None);
        };
        if !self.routes_as_builtin(expanded_name) {
            return Ok(None);
        }

        let mut call = command.clone();
        call.redirect_out = None;
        call.append = None;

        let saved_capture = crate::executor::shell_options::begin_stdout_capture();
        // niubash#345: this builtin element is the shell-side producer when
        // the driver armed `pipeline_stdout_stream`; bind its capture frame
        // to the pipe so an unbounded writer streams.
        crate::executor::shell_options::set_capture_stream(self.pipeline_stdout_stream);

        // Bash applies a pipeline element's redirections before running the
        // command (redir.c do_redirection_internal runs for every command). A
        // failing dup must abort the command with its diagnostic routed through
        // the already-redirected fd2 -- which, after a 2>&1 dup, is this
        // stage's pipe. The stripped clone below never reached that machinery,
        // so "echo foo 2>&1 >&$v | cat" silently ran echo instead of failing
        // with a $v: Bad file descriptor diagnostic down the pipe. Run the same
        // ordered redirect preflight the top-level builtin path uses, while the
        // stdout capture is active so a 2>&1-routed diagnostic lands in this
        // stage's stdout (the pipe) exactly like GNU's.
        // rubash#846: the preflight can return Err (unopenable `>file`).
        // Restore the active thread-local capture before propagating or
        // every later stdout write is swallowed into the orphaned buffer.
        let builtin_stage_output_redirect_failed = match self.command_output_redirect_fails(&call) {
            Ok(failed) => failed,
            Err(error) => {
                crate::executor::shell_options::restore_stdout_capture(saved_capture);
                return Err(error);
            }
        };
        if builtin_stage_output_redirect_failed {
            // rubash#1844: same pre-wired contract as the stage
            // preflight in pipeline_exec.rs — the element died at
            // do_redirections; the driver must not re-apply its list.
            self.pipeline_stage_fds_pre_wired.set(true);
            let captured = crate::executor::shell_options::take_stdout_capture();
            crate::executor::shell_options::restore_stdout_capture(saved_capture);
            return Ok(Some((
                crate::executor::substitution_metadata::bytes_to_shell_text(&captured),
                String::new(),
                1,
            )));
        }

        // rubash#1835: the stderr file redirects join redirect_out/append
        // under the driver's sole ownership. The clone used to keep them,
        // so the builtin's immediate-channel diagnostics were written to
        // the file DURING the stage — and the driver's
        // route_pipeline_stage_streams then re-applied the same redirect,
        // its truncating setup open erasing the bytes it was about to
        // route (the captured stderr was empty: it had never entered the
        // capture). With the clone stripped, diagnostics land in the
        // stage's stderr capture exactly like a stage without its own
        // `2>`, and the driver applies the redirect once (GNU applies a
        // pipeline element's redirections once, in the element's
        // subshell: execute_cmd.c:2620 execute_pipeline). The preflight
        // above already validated the unstripped list (failing dups keep
        // their in-pipe diagnostics). Fd-2 dup entries (`2>&1`) are
        // stripped too: applied in-clone they merge the diagnostics
        // into the stdout capture before the driver can resolve the
        // dup against the ordered list — `2>&1 >f` then wrongly lands
        // in f, and `>f 2>&1` loses the text between the clone's file
        // write and the driver's truncating setup. The driver's walk
        // resolves the dup in list order against the original command,
        // which is exactly GNU's ordering.
        call.redirect_err = None;
        call.redirect_err_append = None;
        call.redirects.retain(|redirect| {
            redirect.fd_var.is_some()
                || !(matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::CombinedOutput
                        | crate::parser::RedirectKind::CombinedAppend
                ) || (matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::Output
                        | crate::parser::RedirectKind::Append
                        | crate::parser::RedirectKind::ClobberOutput
                        | crate::parser::RedirectKind::DuplicateOutput
                        | crate::parser::RedirectKind::DuplicateInput
                ) && redirect.fd == Some(2)))
        });

        let mut subshell = self.command_substitution_executor();
        // A pipeline member runs in a subshell: caught signal traps reset to
        // the inherited disposition (execute_cmd.c subshell trap reset), so
        // only traps the member itself installs run at its exit.
        // rubash#761: a streaming tail's reader pulls fd 0 (the real OS pipe
        // bound by `execute_external_prefix_streaming_tail`); leave
        // FUNCTION_STDIN ABSENT so `read` does not short-circuit to the
        // buffered mirror and report EOF (read_io.rs:361-363). The subshell
        // inherits the fd-0 file endpoint through the cloned fd table.
        // rubash#1873: the same absence when the element is stage 0 and
        // its fd 0 is the shell's live fd 0 (live_stdin).
        if streaming_stdin || live_stdin {
            subshell.shell_state.env_vars.remove(FUNCTION_STDIN);
            subshell.shell_state.env_vars.remove(FUNCTION_STDIN_OFFSET);
        } else {
            subshell
                .shell_state
                .env_vars
                .insert(FUNCTION_STDIN.to_string(), input.to_string());
            subshell
                .shell_state
                .env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        }

        subshell.stderr_capture = Some(Vec::new());
        // rubash#1342: the pipeline driver already emitted this stage's
        // xtrace line; mark the clone so its dispatch re-trace is
        // suppressed once (nested commands still trace normally).
        subshell.pipeline_stage_pre_traced.set(true);
        // rubash#815 profiler: this stage re-dispatches through
        // execute_command, whose own prepared-command seam confirms the
        // element — cancel the stage-level tentative frame so the element is
        // not recorded twice.
        super::profiler::cancel_pending_dispatch();
        let result = subshell.execute_command(&call);
        // GNU execute_cmd.c:1761-1763 (execute_in_subshell): the element's
        // exit runs the subshell's EXIT trap (rubash#1104 family: `trap
        // 'echo X' EXIT | cat` carries X down the pipe). The table was reset
        // at entry, so only traps the member installed can fire. Run before
        // harvesting the captures so the trap output joins the pipe.
        // rubash#1540: a no_args DISCARD raised by the element builtin
        // itself (exit/return with too many arguments) is caught by the
        // execute_subshell_builtin_or_function trampoline
        // (execute_cmd.c) and converted to subshell_exit(1) — the
        // element reports 1 (and its EXIT trap runs with 1), never the
        // pre-set EX_BADUSAGE 2 the clone's reader-level adjudication
        // leaves behind. Expansion-phase failures keep their class
        // status (see the match below): the trampoline only ever sees
        // jumps raised while the builtin runs.
        let body_status = if subshell.evalerror_exit_toomany_pending.get() {
            1
        } else {
            subshell.last_exit_code()
        };
        let trap_result = subshell.run_exit_trap_for_status(body_status);
        let output = crate::executor::shell_options::take_stdout_capture();
        let stderr = subshell.stderr_capture.take().unwrap_or_default();

        crate::executor::shell_options::restore_stdout_capture(saved_capture);
        let status = match (result, trap_result) {
            (Ok(()), Ok(trap_status)) => trap_status,
            (Err(ExecuteError::ExitCode(code)), Ok(_)) => code,
            // rubash#1542: an expansion-phase DISCARD harvested from the
            // element clone keeps its pre-set class status (EX_BADUSAGE
            // 2 for a failed `${var:=word}` bind) — except when this
            // pipeline itself runs inside a command-substitution body,
            // where the body's parse_and_execute catch
            // (builtins/evalstring.c:423-426) folds it to
            // EXECUTION_FAILURE like every DISCARD in that frame.
            (Err(ExecuteError::ExpansionFailure(code)), Ok(_)) => {
                if code == 2 && self.discard_fold_frame() {
                    1
                } else {
                    code
                }
            }
            // rubash#1423: the element runs in a subshell — a `return`
            // raised by the element body ends the element with that
            // status instead of unwinding the enclosing function.
            (Err(ExecuteError::Return(code)), Ok(_)) => code,
            // niubash#345: a builtin pipeline element writing past the
            // consumer's exit is SIGPIPE-killed (status 141).
            (Err(ExecuteError::BrokenPipe), Ok(_)) => 141,
            (_, Err(error)) | (Err(error), _) => return Err(error),
        };
        let consumed = subshell
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        self.pipeline_stdin_consumed.set(Some(consumed));
        Ok(Some((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output),
            crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
            status,
        )))
    }

    pub(in crate::executor) fn execute_external_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
        stdin_inherit: bool,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        let (command, process_substitution_files) =
            self.command_with_process_substitution_files(command)?;
        // rubash#815 profiler: external pipeline elements spawn here without
        // passing execute_external, so the spawn face needs its own frame.
        // Builtin elements need none — the stage body funnels through
        // execute_command, whose dispatch seam confirms them already.
        let _profiler_external = super::profiler::enter_external(
            command.words.first().map(String::as_str).unwrap_or("?"),
        );
        let result = self.execute_external_pipeline_stage_inner(&command, input, stdin_inherit);
        let finish_result = self.finish_process_substitutions(process_substitution_files);
        let output = result?;
        finish_result?;
        Ok(output)
    }

    /// Expands a pipeline stage's words into its final argv:
    /// (program name, argument words). Field-splits the expanded first word
    /// like GNU bash (`v="echo -n hi there"; $v | cat` dispatches on the
    /// first field "echo" and passes the rest as leading arguments), restores
    /// path-escape carriers, and pathname-expands unquoted argument words
    /// (execute_cmd.c execute_simple_command -> expand_words). Computed once
    /// per stage so substitution side effects in arguments do not run twice.
    fn expand_stage_argv(&mut self, command: &CommandNode) -> (String, Vec<String>) {
        let mut first_fields = self.expand_stage_first_word(command).into_iter().peekable();
        let expanded_name = first_fields.next().unwrap_or_default();
        let mut args: Vec<String> = first_fields.collect();
        args.extend(self.expand_stage_tail_words(command));
        (expanded_name, args)
    }

    /// The stage's expanded first word, field-split like GNU
    /// execute_simple_command's dispatch word (`v="echo hi"; $v | cat` runs
    /// two fields). Every field after the first becomes a leading argument.
    /// Shared by the sequential stage executor and the concurrent native
    /// pipeline path so both produce byte-identical argv: the text-layer
    /// `expand_word` re-reads already-dequoted quote characters as syntax
    /// and strips embedded `"` data (rubash#205 — GNU never re-parses argv
    /// text between pipeline elements; execute_cmd.c:2620 execute_pipeline
    /// forks each element with words expanded once from the parsed WORD
    /// structs, and subst.c:4807 dequote_string only removes CTLESC
    /// escapes, leaving bare `"` data characters for execve verbatim).
    pub(in crate::executor) fn expand_stage_first_word(
        &mut self,
        command: &CommandNode,
    ) -> Vec<String> {
        let Some(name) = command.words.first() else {
            return Vec::new();
        };
        let first_raw = command
            .word_metadata
            .first()
            .map(|metadata| metadata.raw.as_str());
        self.expand_command_word(command, 0, name, first_raw)
            .into_iter()
            .map(|word| {
                crate::executor::command_prepare::restore_pathname_escape_markers(
                    &word
                        .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
                        .replace(crate::executor::markers::DATA_BACKSLASH, "\\"),
                )
            })
            .collect()
    }

    /// The stage's argument words (words[1..]), expanded with the same
    /// per-word machinery as the sequential stage executor: marker restore,
    /// quoted-word glob suppression, and pathname expansion for unquoted
    /// words (execute_cmd.c execute_simple_command -> expand_words).
    pub(in crate::executor) fn expand_stage_tail_words(
        &mut self,
        command: &CommandNode,
    ) -> Vec<String> {
        // GNU execute_simple_command runs pathname expansion on every
        // argument of an external command in a pipeline element, so
        // `ls *` hands ls the directory listing rather than a literal
        // `*`. Without this the pattern reached the host binary verbatim
        // (probe 2026-09-09: `ls * | wc -c` gave 2 bytes instead of 15,
        // while `ls -1 | wc -c` was correct).
        let mut args: Vec<String> = Vec::new();
        for (offset, word) in command.words[1..].iter().enumerate() {
            let index = offset + 1;
            let raw = command
                .word_metadata
                .get(index)
                .map(|metadata| metadata.raw.as_str());
            for expanded in self.expand_command_word(command, index, word, raw) {
                let value = crate::executor::command_prepare::restore_pathname_escape_markers(
                    &expanded
                        .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
                        .replace(crate::executor::markers::DATA_BACKSLASH, "\\"),
                )
                .to_string();
                // \x1d marks a fully quoted word and \x1b a quoted tilde;
                // both stay literal, as command_prepare does.
                if expanded.starts_with(STORAGE_WORD_PREFIX)
                    || expanded.starts_with(crate::executor::markers::QUOTED_WORD_PREFIX)
                {
                    args.push(value);
                    continue;
                }
                // Quoted words (e.g. "*.txt") must not be glob-expanded.
                let metadata = command.word_metadata.get(index);
                let suppress =
                    crate::executor::command_prepare::raw_word_suppresses_pathname_expansion(
                        raw, metadata,
                    );
                if suppress {
                    args.push(value);
                    continue;
                }
                match glob::pathname_expand_word(
                    &value,
                    &self.shell_state.env_vars,
                    &self.shell_state.cwd,
                ) {
                    glob::PathnameExpansion::Matches(matches) => args.extend(matches),
                    glob::PathnameExpansion::NoMatch => args.push(value),
                    // rubash#1540: same failglob funnel as the stage
                    // arg-words path — report once and latch; the
                    // caller's stage wrapper (sequential path) or the
                    // spec latch net (concurrent path) adjudicates the
                    // failure as the element's own (empty, 1) result
                    // instead of running the literal pattern.
                    glob::PathnameExpansion::Fail(pattern) => {
                        if !self.evalerror_failglob_pending.replace(true) {
                            self.report_failglob(&pattern);
                        }
                        args.push(value)
                    }
                }
            }
        }
        args
    }

    fn execute_external_pipeline_stage_inner(
        &mut self,
        command: &CommandNode,
        input: &str,
        stdin_inherit: bool,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        let Some(name) = command.words.first() else {
            return Ok(Some((String::new(), String::new(), 0)));
        };
        // Expand the argv once up front. The host external handler receives
        // the same contract as the top-level dispatch (execute_cmd.c
        // expand_words before execute_disk_command): raw word text would leak
        // lexer carriers (\x1a) to the child argv — a single-quoted sed
        // script 'a`q`c' reached the host sed as "a\x1aq\x1ac" and its output
        // lost the literal backticks (rubash#177).
        let (expanded_name, args) = self.expand_stage_argv(command);
        let mut host_cmd = command.clone();
        host_cmd.words = std::iter::once(expanded_name.clone())
            .chain(args.iter().cloned())
            // expand_command_word keeps the lexer's C0 data carriers; the
            // host external handler contract (top-level dispatch) receives
            // visible argv text, so decode the carriers here.
            .map(|word| super::execution_misc::restore_command_substitution_output(&word))
            .collect();
        // Pipeline-stage path: the stage's assignments were never applied
        // to the table (they live only on the cloned command node), so the
        // host channel must compose appends itself (rubash#1766).
        if let Some(output) = self.invoke_host_external_command(&host_cmd, false) {
            return Ok(Some((
                crate::executor::substitution_metadata::bytes_to_shell_text(&output.stdout),
                crate::executor::substitution_metadata::bytes_to_shell_text(&output.stderr),
                output.status,
            )));
        }
        let _ = name;
        let Some(program) = find_user_command(&expanded_name, &self.shell_state.env_vars) else {
            // GNU findcmd.c:385-386: a slash-containing name skips the PATH
            // search and its failure is classified by execve's errno
            // (execute_cmd.c:6126-6159) — "No such file or directory" /
            // "Is a directory" / "Permission denied" — not the PATH-miss
            // "command not found" (rubash#173).
            let not_found_word = super::execution_misc::printable_filename(&expanded_name);
            let (word, message, status) =
                match super::external_inner::slash_command_execve_error_message(
                    &expanded_name,
                    &self.shell_state.env_vars,
                ) {
                    Some((message, status)) => (
                        format!("{not_found_word}: {message}"),
                        String::new(),
                        status,
                    ),
                    None => (
                        not_found_word.clone(),
                        // With PATH unset the name is never searched, so
                        // GNU reports the execve ENOENT text (rubash#1145).
                        if crate::executor::path::path_is_unset(&self.shell_state.env_vars) {
                            ": No such file or directory".to_string()
                        } else {
                            ": command not found".to_string()
                        },
                        127,
                    ),
                };
            // slash-classified execve error or PATH-miss "command not
            // found" (execute_cmd.c internal_error via error_prolog) honors
            // gnu_errfmt (rubash#1038).
            let diagnostic = format!("{}{}{}\n", self.error_prefix(), word, message);
            // Bash applies a pipeline element's redirections before the
            // command lookup fails (redir.c do_redirection_internal runs for
            // every command): `2>/dev/null` discards the diagnostic,
            // `2>file` writes it there, and `2>&1` sends it down this
            // stage's pipe (issue #70's `git ... 2>/dev/null | sed` idiom).
            if let Some(redirect) = &command.redirect_err {
                let target = self.expand_redirect_target(redirect);
                if redirect_target_fd(&target) == Some(1) {
                    return Ok(Some((diagnostic, String::new(), 127)));
                }
                if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                    if let Ok(mut file) = self.create_redirect_output(&target, redirect.clobber) {
                        let _ = file.write_all(diagnostic.as_bytes());
                    }
                    // rubash#1571: the diagnostic was just delivered
                    // through the element's own open of the target. The
                    // driver's routing walk would re-run the ordered
                    // redirects for this stage and its probe-open
                    // (open_command_output_target) truncates the file
                    // again before writing the stage's (empty) captured
                    // stderr — the delivered diagnostic died for every
                    // non-first stage. The stage owns its stream
                    // placement exactly like the shared-file wiring
                    // below and the function stage's rubash#412 case:
                    // mark it pre-wired so the driver skips routing.
                    self.pipeline_stage_fds_pre_wired.set(true);
                    return Ok(Some((String::new(), String::new(), 127)));
                }
            } else if let Some(redirect) = &command.redirect_err_append {
                let target = self.expand_redirect_target(redirect);
                if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                    if let Ok(mut file) =
                        open_redirect_append(&self.session_path(shell_path_to_windows(
                            &target,
                            &self.shell_state.env_vars,
                        )))
                    {
                        let _ = file.write_all(diagnostic.as_bytes());
                    }
                    // rubash#1571: same direct delivery as above.
                    self.pipeline_stage_fds_pre_wired.set(true);
                    return Ok(Some((String::new(), String::new(), 127)));
                }
            }
            return Ok(Some((String::new(), diagnostic, 127)));
        };

        let mut args: Vec<String> = args
            .into_iter()
            // The spawned child receives visible argv text; the lexer's C0
            // data carriers (\x1a for a quoted backtick) would reach the
            // external program as control bytes (rubash#177 sequential
            // pipeline stages). The concurrent path already decodes them
            // through expand_word.
            .map(|word| super::execution_misc::restore_command_substitution_output(&word))
            .collect();
        // GNU execute_cmd.c:6139-6233 (shell_execve): a file the OS cannot
        // exec natively is classified by its first bytes before the
        // shell-script fallback: an unresolvable #! interpreter is refused
        // ("bad interpreter", EX_NOEXEC) and a binary first line is refused
        // ("cannot execute binary file", EX_BINARY_FILE), both with status
        // 126. Plain text falls through to the shell-script execution.
        if crate::executor::path::should_run_with_shell(&program) {
            if let Some((diagnostic, status)) = self.exec_format_refusal(command, &program) {
                return Ok(Some((String::new(), format!("{diagnostic}\n"), status)));
            }
        }

        // The stage's fd 0 is the buffered `input` payload fed through a
        // pipe below (or the live inherited stdin for an unbound stage 0)
        // and fd 1 is captured by the caller — /dev/fd operands
        // materialize against those endpoints, not the fd table.
        let dev_stdin = if stdin_inherit {
            crate::executor::dev_fd_operands::DevOperandStdin::FdTable
        } else {
            crate::executor::dev_fd_operands::DevOperandStdin::Payload(
                crate::executor::substitution_metadata::shell_text_to_raw_bytes(input),
            )
        };
        let (dev_args, dev_ops) = self.materialize_dev_fd_operands(
            &args,
            dev_stdin,
            crate::executor::dev_fd_operands::DevOperandStdout::Capture,
        );
        let (mut process, _) = external_command_for_named_program(
            &program,
            Some(&expanded_name),
            &dev_args,
            &self.shell_state.env_vars,
            self.bash_argv0_env_supplied(&[]),
        );

        self.apply_child_environment(&mut process);
        let raws_aligned = command.assignment_raws.len() == command.assignments.len();
        for (index, (var_name, var_value)) in command.assignments.iter().enumerate() {
            let (base_name, append) = assignment_name_and_append(var_name);
            let raw = raws_aligned
                .then(|| command.assignment_raws.get(index))
                .flatten()
                .map(String::as_str);
            let expanded_value = self.expand_assignment_value_with_raw(var_name, var_value, raw);
            // Marker hygiene (rubash#1620): a compound `name=(...)`
            // tempenv word binds as the literal list text; the internal
            // COMPOUND_ASSIGNMENT_MARKER must not reach the child.
            let expanded_value = expanded_value
                .strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(&expanded_value);
            if append {
                // A pipeline stage's assignments are NOT pre-applied to
                // the variable table (the direct external path's child
                // inherits the window binding via apply_child_environment;
                // a stage's table still holds the outer value), so the
                // spawner composes the window value itself, mirroring
                // assign_in_env with ASS_APPEND: the target's scalar view
                // plus the literal RHS text (rubash#1620 — re-applying the
                // raw RHS lost the base and leaked the marker).
                let window = format!("{}{}", self.tempenv_scalar_view(base_name), expanded_value);
                if is_valid_process_env(base_name, &window) {
                    process.env(base_name, window);
                }
                continue;
            }
            if is_valid_process_env(base_name, expanded_value) {
                process.env(base_name, expanded_value);
            }
        }
        // GNU execute_cmd.c:4617 execute_simple_command: the forked pipeline
        // element applies ITS OWN redirections after the pipeline binds its
        // fds (redir.c do_redirection_internal), so a stage's `< file` /
        // `<> file` fd-0 redirect REPLACES the inherited shell stdin (an
        // unbound stage 0) or the upstream pipe payload. This spawner only
        // modeled inherit-or-payload: a stage-0 `< /dev/null` left the child
        // reading the live console/pipe forever (rubash#429 — the bash-it
        // iterate theme's per-prompt `$BASH --norc -i < /dev/null | sed`
        // wedged the whole ConPTY session), and `echo hi | cat < /dev/null`
        // fed the upstream payload where GNU feeds the empty file.
        let fd0_file_redirect =
            match crate::executor::shell_options::fd0_stdin_redirect_winner(command) {
                Some(crate::parser::RedirectKind::Input)
                | Some(crate::parser::RedirectKind::ReadWrite) => true,
                // A heredoc/here-string winner is the payload already fed
                // through the piped arm below (initial_pipeline_input).
                Some(_) => false,
                None => command.redirect_in.is_some(),
            };
        if fd0_file_redirect {
            self.apply_external_stdin_redirect(command, &mut process)?;
        } else if stdin_inherit {
            process.stdin(Stdio::inherit());
        } else {
            process.stdin(Stdio::piped());
        }

        // Bash applies a pipeline element's redirections before running the
        // command (redir.c do_redirection_internal), with the pipe bound to
        // fd 1 first (execute_cmd.c execute_pipeline): the element's FINAL
        // fd-1/fd-2 targets decide the child's wiring.
        //
        // niubash#144: when both targets are the stage pipe (`2>&1`,
        // `2>&3 3>&1`, `|&`), the child gets ONE shared pipe for both
        // streams — GNU's dup2 (redir.c:1169-1170) leaves both descriptors
        // on the pipe's write end, so the merged bytes interleave in true
        // write order; two pipes concatenated stdout-first would reorder
        // and withhold the stderr half. `|&` arrives as a trailing `2>&1`
        // redirect (normalize_pipeerr_stage, parse.y:1470-1487), so the
        // ordered walk — not a pipe == Some(2) special case — decides.
        //
        // When BOTH fds resolve onto one FILE description (`>f 2>&1`,
        // `>f |&` — the dup follows the file open), GNU holds both
        // descriptors on the SAME open file: the child writes both streams
        // live through it. Wire both ends of one append-open directly and
        // mark the stage pre-wired — the post-hoc routing pass would
        // otherwise re-truncate the file the child already wrote (its walk
        // re-opens `>f`). The walk above already applied the redirect's
        // create/truncate/noclobber semantics once.
        let (fd1_target, fd2_target) = self
            .pipeline_stage_stdio_targets(command)?
            .unwrap_or((OutputTarget::Stdout, OutputTarget::Stderr));
        let stderr_merges_into_stdout = matches!(fd1_target, OutputTarget::Stdout)
            && matches!(fd2_target, OutputTarget::Stdout);
        let stage_fds_share_one_file = match (&fd1_target, &fd2_target) {
            (OutputTarget::Path(a), OutputTarget::Path(b)) => a == b,
            (OutputTarget::SharedFile(a), OutputTarget::SharedFile(b)) => std::rc::Rc::ptr_eq(a, b),
            _ => false,
        };
        let mut merged_stage_reader = None;
        let mut merged_shared_file: Option<MergedSharedFile> = None;
        if stderr_merges_into_stdout {
            let (reader, writer) = os_pipe::pipe().map_err(ExecuteError::IoError)?;
            let writer_clone = writer.try_clone().map_err(ExecuteError::IoError)?;
            process.stdout(Stdio::from(writer_clone));
            process.stderr(Stdio::from(writer));
            merged_stage_reader = Some(reader);
        } else if stage_fds_share_one_file {
            // Both fds share ONE open file description in GNU — model it
            // with ONE merged pipe (true write order) and append the
            // drained bytes to the shared file after exit. The stage pipe
            // payload stays empty (GNU hands the downstream stage nothing).
            // Directly wiring the child's fds onto the file does not work
            // for every child here (msys children lose direct stage-wired
            // file handles), so the child only ever sees pipes.
            let (reader, writer) = os_pipe::pipe().map_err(ExecuteError::IoError)?;
            let writer_clone = writer.try_clone().map_err(ExecuteError::IoError)?;
            process.stdout(Stdio::from(writer_clone));
            process.stderr(Stdio::from(writer));
            merged_stage_reader = Some(reader);
            merged_shared_file = Some(match &fd1_target {
                OutputTarget::Path(path) => MergedSharedFile::Path(path.clone()),
                OutputTarget::SharedFile(file) => MergedSharedFile::Handle(file.handle),
                _ => unreachable!("guarded above"),
            });
            self.pipeline_stage_fds_pre_wired.set(true);
        } else {
            // niubash#345: a shell-writer pipeline producer wires the child's
            // fd 1 onto the inter-member pipe (the caller's `os_pipe` write
            // end) instead of a parent capture — GNU binds the stage pipe to
            // the element's fd 1 (execute_cmd.c:2702) and the element writes
            // it directly, dying of SIGPIPE when the consumer exits. The
            // parent keeps its own copy of the write end open until the stage
            // returns, then closes it so a finite producer's EOF reaches the
            // consumer.
            let streamed_stdout = if matches!(&fd1_target, OutputTarget::Stdout) {
                self.pipeline_stdout_stream
            } else {
                None
            };
            match streamed_stdout {
                Some(stream) => {
                    let duplicated = crate::fd::duplicate_handle_inheritable(stream)
                        .map_err(ExecuteError::IoError)?;
                    process.stdout(stdio_from_fd_handle(duplicated));
                }
                None => {
                    // rubash#1454: for the FINAL stage, fd 1's payload
                    // lands on the shell's own stdout (write_pipeline_output
                    // is the only consumer), so when that resolution is the
                    // host stdout itself, hand the child the real fd
                    // (GNU execute_cmd.c:2620) instead of a capture pipe
                    // the shell forwards after exit — the member then
                    // sees the terminal/file directly (pagers page).
                    // wait_with_output yields an empty stdout for the
                    // inherited end, so the returned payload stays empty.
                    if matches!(&fd1_target, OutputTarget::Stdout)
                        && self.pipeline_final_stage.get()
                        && self.final_member_stdout_streams_to_host(command)
                    {
                        process.stdout(Stdio::inherit());
                    } else {
                        process.stdout(Stdio::piped());
                    }
                }
            }
            match &fd2_target {
                // fd 2 still on the stage pipe (`2>&1 >f`: the dup ran
                // before fd 1 left for the file): pipe it and let the
                // routing pass place the payload on the pipe.
                OutputTarget::Stdout => {
                    process.stderr(Stdio::piped());
                }
                // fd 2 on the walk's file (`2>err`, `>f 2>&1`): pipe it and
                // let the routing pass place the payload in the file (its
                // walk applies the redirect's create/truncate semantics
                // again, then appends the payload — the file holds exactly
                // the stream's bytes in write order). Direct stage-side
                // file wiring is not reliable for every child (msys
                // children lose it), so the child only ever sees pipes.
                OutputTarget::Path(_) | OutputTarget::SharedFile(_) => {
                    process.stderr(Stdio::piped());
                }
                OutputTarget::Null => {
                    process.stderr(Stdio::null());
                }
                // Closed fd 2: pipe the payload so the routing pass reports
                // `write error: Bad file descriptor` with status 1 the way
                // GNU's EBADF write does.
                OutputTarget::Closed => {
                    process.stderr(Stdio::piped());
                }
                // Ambient stderr, dup snapshots the walk cannot express, or
                // a failed walk: keep the per-redirect legacy handling.
                _ => {
                    if let Some(redirect) = &command.redirect_err {
                        let target = self.expand_redirect_target(redirect);
                        if !is_closed_redirect_target(&target)
                            && redirect_target_fd(&target).is_none()
                        {
                            if let Ok(file) = self.create_redirect_output(&target, redirect.clobber)
                            {
                                process.stderr(Stdio::from(file));
                            }
                        }
                    } else if let Some(redirect) = &command.redirect_err_append {
                        let target = self.expand_redirect_target(redirect);
                        if !is_closed_redirect_target(&target)
                            && redirect_target_fd(&target).is_none()
                        {
                            if let Ok(file) = open_redirect_append(&self.session_path(
                                shell_path_to_windows(&target, &self.shell_state.env_vars),
                            )) {
                                process.stderr(Stdio::from(file));
                            }
                        }
                    }
                }
            }
        }

        // rubash#1688: this sequential stage spawner builds its child by
        // hand like the concurrent pipeline loops — pin the ambient
        // descriptors, or a member of a mixed (builtin-headed) pipeline
        // sees EBADF on every `exec` fd the concurrent path delivers.
        self.attach_ambient_fd_pins(command, &mut process);
        // rubash#438: pin the buffered process-substitution pipe read ends at
        // their /dev/fd/N numbers in the child (fork-exec dup2), so the
        // literal argv word resolves through the OS /dev/fd layer.
        #[cfg(unix)]
        crate::executor::dev_fd_operands::attach_unix_dev_fd_operands(&mut process, &dev_ops);
        process.current_dir(&self.shell_state.cwd);
        suppress_orphan_console(&mut process);
        let mut child = process.spawn()?;
        adopt_orphan_console(child.id());
        // Drop the Command NOW: std retains the `Stdio::from(PipeWriter)`
        // values set above, and a live Command keeps the merged pipe's
        // write end open in the parent — the drain below would never see
        // EOF even after the child exits. The child holds its own
        // duplicated inheritable handles (the fork-and-dup of GNU's
        // execute_pipeline element setup).
        drop(process);
        // niubash#158: feed the stdin payload from a writer thread. Writing
        // synchronously deadlocks when the payload exceeds the stdin pipe's
        // capacity and the child is a pass-through that already filled its
        // own (still undrained) stdout pipe: the shell blocks in write_all,
        // the child stops reading, and the drain below never starts
        // (`cat bigfile | cat`). GNU never mediates this write — the
        // upstream element writes the inter-member pipe directly
        // (execute_cmd.c:2645-2711) and dies on SIGPIPE when the reader
        // goes away — so a broken-pipe result here (the child exited
        // early, e.g. `head -1`) is the child's choice and is swallowed
        // like SIGPIPE's silence, not propagated as a stage error.
        let stdin_writer = child.stdin.take().map(|mut stdin| {
            let payload = crate::executor::substitution_metadata::shell_text_to_raw_bytes(input);
            // The child was handed the whole payload; model it as consumed
            // (same approximation as comsub_stdin_writeback — a spawned
            // process's read() calls are not observable here).
            //
            // `input` is transport text and this delta is folded into
            // FUNCTION_STDIN_OFFSET, a transport byte index into the same
            // buffer (`function_stdin_remaining`/`read_function_stdin` slice
            // `input[offset..]`). Advance by the TRANSPORT length, never the
            // raw payload length: a raw delta under-advances by the carrier
            // overhead (x6 per control byte, x2 per high byte) and replays
            // bytes the child already consumed (niubash#383 follow-up).
            self.pipeline_stdin_consumed.set(Some(input.len()));
            std::thread::spawn(move || {
                // The payload write must survive a broken pipe as a plain
                // EPIPE (see the spawn comment): SIGPIPE is SIG_DFL for
                // the shell process since rubash#1333, so block it here.
                #[cfg(unix)]
                crate::builtins::kill::block_sigpipe_on_this_thread();
                let _ = stdin.write_all(&payload);
            })
        });
        if let Some(mut reader) = merged_stage_reader {
            // One pipe carries the merged stream in true write order; drain
            // it to EOF while the child runs, then reap.
            use std::io::Read;
            let mut merged = Vec::new();
            reader
                .read_to_end(&mut merged)
                .map_err(ExecuteError::IoError)?;
            self.note_foreground_wait();
            let status = child.wait().map_err(ExecuteError::IoError)?;
            // The child is gone; its stdin pipe is broken, so the payload
            // writer thread (spawned above) has finished or is about to.
            let _ = stdin_writer.map(|writer| writer.join());
            merged.extend(self.finish_dev_fd_operands(dev_ops));
            if let Some(target) = merged_shared_file {
                // The merged pipe stood for BOTH fds on one shared file
                // (`>f 2>&1`, `>f |&`): the ordered walk already applied
                // the redirect's open/truncate once, so append the drained
                // bytes in true write order and hand the downstream stage
                // an empty pipe (GNU's next element reads nothing).
                match target {
                    MergedSharedFile::Path(path) => {
                        let mut file = open_redirect_append(&self.session_path(
                            shell_path_to_windows(&path, &self.shell_state.env_vars),
                        ))
                        .map_err(ExecuteError::IoError)?;
                        file.write_all(&merged)?;
                    }
                    MergedSharedFile::Handle(handle) => {
                        crate::fd::write_all(handle, &merged).map_err(ExecuteError::IoError)?;
                    }
                }
                return Ok(Some((
                    String::new(),
                    String::new(),
                    crate::executor::wait_status::process_exit_status(&status),
                )));
            }
            return Ok(Some((
                crate::executor::substitution_metadata::bytes_to_shell_text(&merged),
                String::new(),
                crate::executor::wait_status::process_exit_status(&status),
            )));
        }
        self.note_foreground_wait();
        let output = child.wait_with_output()?;
        // Same reaping boundary as the merged branch above: the exited child
        // broke the stdin pipe, the payload writer thread ends on its
        // broken-pipe write, and joining here keeps the thread's lifetime
        // inside the stage call.
        let _ = stdin_writer.map(|writer| writer.join());

        let mut stdout_bytes = output.stdout;
        stdout_bytes.extend(self.finish_dev_fd_operands(dev_ops));
        // Both-fds-on-the-pipe shapes took the merged-pipe branch above and
        // returned early; here the streams stay separate and the driver's
        // route_pipeline_stage_streams places each payload (fd 2 on the
        // pipe = `2>&1 >f` lands on stdout's pipe content, a file target
        // lands in the file).
        let stderr_bytes = output.stderr;

        Ok(Some((
            crate::executor::substitution_metadata::bytes_to_shell_text(&stdout_bytes),
            crate::executor::substitution_metadata::bytes_to_shell_text(&stderr_bytes),
            crate::executor::wait_status::process_exit_status(&output.status),
        )))
    }

    pub(in crate::executor) fn write_pipeline_output(
        &mut self,
        command: &CommandNode,
        output: &str,
        already_routed: bool,
    ) -> Result<(), ExecuteError> {
        // Final pipeline output is a byte boundary: decode owner-tagged
        // raw-byte markers exactly once before reaching files, captures,
        // or stdout.
        // Compound stages propagated their own output redirections into the
        // body already (execute_compound_pipeline_stage keeps redirect_out so
        // inner `3>&1` resolves the redirected fd 1), so only what escaped to
        // fd 1 remains here. Simple stages still carry theirs — resolve them
        // like the intermediate-stage routing does, covering `>f`, `1>&2`,
        // `>&-`, and numbered fds the way do_redirection_internal would (the
        // old mirror-only path once wrote `cat 1>&2`'s payload to a literal
        // file named `&2`). Callers that already ran
        // route_pipeline_stage_streams for the stage (the sequential stage
        // loop, and lastpipe/compound stages whose redirections were applied
        // inside) pass already_routed — a second pass would reopen `>f` and
        // truncate what the first pass wrote.
        if !already_routed && !command_is_compound_pipeline_stage(command) {
            let mut stdout = output.to_string();
            let mut stderr = String::new();
            let mut status = 0;
            if self.route_pipeline_stage_streams(command, &mut stdout, &mut stderr, &mut status)? {
                if status != 0 {
                    self.exit_code = status;
                }
                if !stdout.is_empty() {
                    let payload =
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stdout);
                    // One fd-1 stream: the thread-local capture is the
                    // innermost live fd 1 while a stage body runs, so route
                    // through write_default_stdout — writing the field
                    // capture first would reorder against thread-local
                    // writes from spawned children in the same body
                    // (procsub.tests: `x <(date) | cat` emitted the inner
                    // pipeline's wc output before the earlier spawned wc).
                    self.write_default_stdout(&payload)?;
                }
                if !stderr.is_empty() {
                    let payload =
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stderr);
                    self.write_default_stderr(&payload)?;
                }
                return Ok(());
            }
        }
        let payload = crate::executor::substitution_metadata::shell_text_to_raw_bytes(output);
        // Same single-fd-1 ordering: the live thread-local capture (if any)
        // is the stage's fd 1, ahead of self.stdout_capture.
        self.write_default_stdout(&payload)?;
        Ok(())
    }

    pub(in crate::executor) fn skip_and_or_rhs(
        &self,
        commands: &[CommandNode],
        index: usize,
    ) -> Option<usize> {
        // TODO(parse.y/execute_cmd.c): Bash executes AND_AND/OR_OR lists from
        // the grammar, not by scanning flattened commands. This narrow bridge
        // keeps `cmd || { echo ...; exit 1; }` failure handlers from running
        // after a successful command in upstream source8.sub.
        let connector = commands.get(index)?.and_or()?;
        let should_skip = (connector && self.exit_code != 0) || (!connector && self.exit_code == 0);
        if !should_skip {
            return None;
        }

        if commands
            .get(index)
            .is_some_and(|command| is_arithmetic_command_words(&command.words))
        {
            return Some((index + 2).min(commands.len()));
        }

        // The skip is bounded by the LOGICAL line (GNU executes the
        // and_or list as one unit regardless of line breaks): commands
        // joined by backslash-newline continuations share the boundary
        // even though their physical lines differ (rubash#411).
        let start_line = commands
            .get(index + 1)
            .and_then(|command| command.logical_line.or(command.line));
        let mut next_index = index + 1;
        while next_index < commands.len()
            && commands[next_index]
                .logical_line
                .or(commands[next_index].line)
                == start_line
            && commands[next_index].and_or().is_none()
        {
            next_index += 1;
        }
        Some(next_index.max(index + 1))
    }

    pub(in crate::executor) fn execute_alias_escaped_pipe(
        &mut self,
        commands: &[CommandNode],
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        // TODO(parse.y/alias.c): Bash pushes alias text back to the parser, so
        // an alias ending with backslash can quote the next input character.
        // This covers alias4.sub's `alias a='printf "<%s>\n" \'` followed by
        // `a|cat`, which should pass literal `|cat` to printf.
        let Some(command) = commands.get(index) else {
            return Ok(None);
        };
        if command.pipe.is_none() || command.words.len() != 1 {
            return Ok(None);
        }

        let Some(alias) = self.shell_state.aliases.get(&command.words[0]) else {
            return Ok(None);
        };
        if !alias.value.ends_with('\\') {
            return Ok(None);
        }

        let Some(next_command) = commands.get(index + 1) else {
            return Ok(None);
        };
        let mut source = alias.value.trim_end_matches('\\').trim_end().to_string();
        source.push_str(" \\|");
        source.push_str(&next_command.words.join(" "));

        let tokens = crate::lexer::tokenize(&source);
        let reparsed = crate::parser::parse(&tokens);
        self.execute_ast(&reparsed)?;
        Ok(Some(index + 2))
    }
}

/// The file target a stage's merged pipe stood in for (both fds on one
/// open file description — `>f 2>&1`, `>f |&`): the drained merged bytes
/// append here after the child exits.
enum MergedSharedFile {
    Path(String),
    Handle(crate::fd::HANDLE),
}
