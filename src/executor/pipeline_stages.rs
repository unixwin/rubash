use super::redirection::OutputTarget;
use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;
use crate::executor::pipeline_exec::command_is_compound_pipeline_stage;

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
        let (thread_captured, result) =
            crate::executor::shell_options::capture_stdout(|| self.execute_command(command));
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
    ) -> Result<(String, String, i32), ExecuteError> {
        let saved_dir = env::current_dir().ok();
        let mut subshell = self.command_substitution_executor();
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

        subshell.stdout_capture = Some(Vec::new());
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
        let saved_capture = crate::executor::shell_options::begin_stdout_capture();

        let result = if command.brace_group.is_some() {
            subshell.execute_brace_group_pipeline(command).map(|_| ())
        } else {
            subshell.execute_command(command)
        };
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
        let mut thread_output = crate::executor::shell_options::take_stdout_capture();
        let mut output = subshell.stdout_capture.take().unwrap_or_default();
        let stderr = subshell.stderr_capture.take().unwrap_or_default();

        crate::executor::shell_options::restore_stdout_capture(saved_capture);
        output.append(&mut thread_output);

        if let Some(saved_dir) = saved_dir {
            let _ = env::set_current_dir(saved_dir);
        }
        let status = match (result, trap_result) {
            (Ok(()), Ok(trap_status)) => trap_status,
            (Err(ExecuteError::ExitCode(code)), Ok(_)) => code,
            (Err(ExecuteError::ExpansionFailure(code)), Ok(_)) => code,
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
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        let Some(name) = command.words.first() else {
            return Ok(Some((String::new(), String::new(), 0)));
        };
        let expanded_name = self.expand_word(name);
        let Some(function_name) = self.function_name_for_command_word(&expanded_name) else {
            return Ok(None);
        };
        // GNU subst.c process_substitute: a <( ) word in the command's
        // argument list materializes before the command runs
        // (procsub.tests: count_lines <(date) | ... must pass the
        // substitution result as \$1, not the raw <(date) text). The
        // top-level function path does this through
        // command_with_process_substitution_files; pipeline stages skipped
        // it, leaving $1 as the literal <(date) string.
        let (materialized_command, procsub_files) =
            self.command_with_process_substitution_files(command)?;
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

        let saved_dir = env::current_dir().ok();
        let mut subshell = self.command_substitution_executor();
        // Function pipeline stages are compound command bodies for errexit
        // (execute_cmd.c execute_function: the body inherits the element's
        // CMD_IGNORE_RETURN), so the subshell keeps the parent's
        // suppress_errexit like the compound-stage path above.
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input.to_string());
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());

        subshell.stdout_capture = Some(Vec::new());
        subshell.stderr_capture = Some(Vec::new());
        // Direct-stdout builtins in the function consult the thread-local
        // capture, which belongs to an enclosing capture when this stage
        // runs inside one; give the call its own capture.
        let (thread_captured, result) = crate::executor::shell_options::capture_stdout(|| {
            subshell.execute_function(&function_name, &args, &call)
        });
        let mut output = subshell.stdout_capture.take().unwrap_or_default();
        output.extend_from_slice(&thread_captured);
        let stderr = subshell.stderr_capture.take().unwrap_or_default();
        let status = subshell.last_exit_code();

        if let Some(saved_dir) = saved_dir {
            let _ = env::set_current_dir(saved_dir);
        }
        let status = match result {
            Ok(()) => status,
            Err(ExecuteError::ExitCode(code)) | Err(ExecuteError::ExpansionFailure(code)) => code,
            Err(error) => return Err(error),
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
        if !crate::executor::builtin_names::is_shell_builtin_name(expanded_name) {
            return Ok(None);
        }

        let mut call = command.clone();
        call.redirect_out = None;
        call.append = None;

        let saved_capture = crate::executor::shell_options::begin_stdout_capture();

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
        if self.command_output_redirect_fails(&call)? {
            let captured = crate::executor::shell_options::take_stdout_capture();
            crate::executor::shell_options::restore_stdout_capture(saved_capture);
            return Ok(Some((
                crate::executor::substitution_metadata::bytes_to_shell_text(&captured),
                String::new(),
                1,
            )));
        }

        let mut subshell = self.command_substitution_executor();
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input.to_string());
        subshell
            .shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());

        subshell.stderr_capture = Some(Vec::new());
        let result = subshell.execute_command(&call);
        let output = crate::executor::shell_options::take_stdout_capture();
        let stderr = subshell.stderr_capture.take().unwrap_or_default();
        let status = subshell.last_exit_code();

        crate::executor::shell_options::restore_stdout_capture(saved_capture);
        let status = match result {
            Ok(()) => status,
            Err(ExecuteError::ExitCode(code)) | Err(ExecuteError::ExpansionFailure(code)) => code,
            Err(error) => return Err(error),
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
                match glob::pathname_expand_word(&value, &self.shell_state.env_vars) {
                    glob::PathnameExpansion::Matches(matches) => args.extend(matches),
                    glob::PathnameExpansion::NoMatch | glob::PathnameExpansion::Fail(_) => {
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
        if let Some(output) = self.invoke_host_external_command(&host_cmd) {
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
                        ": command not found".to_string(),
                        127,
                    ),
                };
            let diagnostic = format!("{}{}{}\n", self.diagnostic_prefix(), word, message);
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
                    return Ok(Some((String::new(), String::new(), 127)));
                }
            } else if let Some(redirect) = &command.redirect_err_append {
                let target = self.expand_redirect_target(redirect);
                if !is_closed_redirect_target(&target) && redirect_target_fd(&target).is_none() {
                    if let Ok(mut file) = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(shell_path_to_windows(&target, &self.shell_state.env_vars))
                    {
                        let _ = file.write_all(diagnostic.as_bytes());
                    }
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
        );

        self.apply_child_environment(&mut process);
        let raws_aligned = command.assignment_raws.len() == command.assignments.len();
        for (index, (var_name, var_value)) in command.assignments.iter().enumerate() {
            let (base_name, _) = assignment_name_and_append(var_name);
            let raw = raws_aligned
                .then(|| command.assignment_raws.get(index))
                .flatten()
                .map(String::as_str);
            let expanded_value = self.expand_assignment_value_with_raw(var_name, var_value, raw);
            if is_valid_process_env(base_name, &expanded_value) {
                process.env(base_name, expanded_value);
            }
        }
        if stdin_inherit {
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
            process.stdout(Stdio::piped());
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
                            if let Ok(file) = OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(shell_path_to_windows(&target, &self.shell_state.env_vars))
                            {
                                process.stderr(Stdio::from(file));
                            }
                        }
                    }
                }
            }
        }

        let mut child = process.spawn()?;
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
            self.pipeline_stdin_consumed.set(Some(input.len()));
            std::thread::spawn(move || {
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
                        let mut file = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(shell_path_to_windows(&path, &self.shell_state.env_vars))
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

        let start_line = commands.get(index + 1).and_then(|command| command.line);
        let mut next_index = index + 1;
        while next_index < commands.len()
            && commands[next_index].line == start_line
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
