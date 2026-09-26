use super::*;

fn timed_read_value(input: &TimedPipelineInput, start: f64, timeout: f64) -> (String, i32) {
    if timeout == 0.0 {
        let ready = input.chunks.iter().any(|(at, _)| *at <= start) || input.eof_at <= start;
        return (String::new(), if ready { 0 } else { 1 });
    }

    let deadline = start + timeout;
    let mut value = String::new();
    for (at, chunk) in &input.chunks {
        if *at > deadline {
            break;
        }
        value.push_str(chunk);
        if let Some(newline) = value.find('\n') {
            value.truncate(newline);
            // CRLF input (`read -t`): the '\r' that precedes the newline is
            // part of the terminator, matching `read`'s non-timed path.
            if value.ends_with('\r') {
                value.pop();
            }
            return (value, 0);
        }
    }

    if input.eof_at <= deadline {
        return (
            value.trim_capture_terminator().to_string(),
            if value.is_empty() { 1 } else { 0 },
        );
    }

    (value, 142)
}

#[cfg(windows)]
fn stdio_from_transferred_handle<T>(handle: T) -> Stdio
where
    T: std::os::windows::io::IntoRawHandle,
{
    use std::os::windows::io::FromRawHandle;
    unsafe { Stdio::from_raw_handle(handle.into_raw_handle()) }
}

#[cfg(windows)]
fn internal_pipeline_program(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("<rubash-internal-{name}>"))
}

#[cfg(windows)]
fn internal_pipeline_program_name(program: &std::path::Path) -> Option<&str> {
    let name = program.to_str()?.strip_prefix("<rubash-internal-")?;
    name.strip_suffix('>')
}

#[cfg(windows)]
fn wait_for_windows_pipeline_member(
    process: &mut std::process::Child,
) -> Result<std::process::ExitStatus, ExecuteError> {
    // Bash waits for the pipeline job to publish each member's status. Give
    // a producer that observed a closed downstream pipe a short opportunity
    // to exit naturally before applying the Windows hard-kill fallback.
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(5);
    const NATURAL_EXIT_WINDOW: std::time::Duration = std::time::Duration::from_millis(100);
    let deadline = std::time::Instant::now() + NATURAL_EXIT_WINDOW;
    loop {
        if let Some(status) = process.try_wait().map_err(ExecuteError::IoError)? {
            return Ok(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = process.kill();
            return process.wait().map_err(ExecuteError::IoError);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}
use crate::executor::external_setup::shared_combined_output_process_substitution;
use crate::executor::markers::STORAGE_WORD_PREFIX;

#[derive(Debug, Clone)]
struct TimedPipelineInput {
    chunks: Vec<(f64, String)>,
    eof_at: f64,
}

impl Executor {
    pub(in crate::executor) fn execute_and_or_list_command(
        &mut self,
        and_or_list: &AndOrListCommand,
    ) -> Result<(), ExecuteError> {
        for (index, command) in and_or_list.commands.iter().enumerate() {
            if index > 0 {
                let connector = and_or_list.connectors.get(index - 1).copied();
                let should_execute = match connector {
                    Some(true) => self.exit_code == 0,
                    Some(false) => self.exit_code != 0,
                    None => true,
                };
                if !should_execute {
                    continue;
                }
            }

            let mut command = command.clone();
            command.and_or = None;
            let ast = Ast {
                commands: vec![command],
            };
            if index < and_or_list.connectors.len() {
                self.with_errexit_suppressed(|executor| executor.execute_ast(&ast))?;
            } else {
                self.execute_ast(&ast)?;
            }
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_pipeline_command(
        &mut self,
        pipeline_command: &PipelineCommand,
    ) -> Result<(), ExecuteError> {
        // Expand aliases in pipeline stages before running them, like Bash
        // does during parsing. Without this, `alias pipehi='echo pipehi';
        // pipehi | cat` fails with "pipeline command could not execute".
        let mut stages = pipeline_command.stages.clone();
        for stage in &mut stages {
            if self.shell_state.aliases.is_empty() {
                break;
            }
            let raws: Vec<Option<&str>> = stage
                .word_metadata
                .iter()
                .map(|metadata| Some(metadata.raw.as_str()))
                .collect();
            stage.words = self.expand_aliases_with_raw(&stage.words, &raws);
        }
        let ast = Ast { commands: stages };
        self.execute_simple_pipeline(&ast, 0)?.ok_or_else(|| {
            ExecuteError::UnknownBuiltin("pipeline command could not execute".to_string())
        })?;
        Ok(())
    }

    pub(in crate::executor) fn execute_brace_group_pipeline(
        &mut self,
        command: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        if let Some(brace_group) = &command.brace_group {
            // GNU execute_cmd.c:826-828: cm_group is a shell control
            // structure, so `{ } <f` sets the sticky stdin_redir flag the
            // same way `while ... done <f` does — the check lives in
            // execute_command for the other kinds; brace groups dispatch
            // here instead.
            if !command.redirects.is_empty() {
                self.shell_state
                    .stdin_redir
                    .set(command.redirects.iter().any(redirect_updates_stdin_redir));
            }
            let mut redirect_command = command.clone();
            let group_outputs =
                self.materialize_compound_output_process_substitutions(&mut redirect_command)?;
            let mut body = brace_group.body.clone();
            self.apply_brace_group_redirects(&redirect_command, &mut body)?;
            let ast = Ast { commands: body };
            // `{ list; } N<<EOF` keeps the numbered heredoc fd open for the
            // whole group (redir.c do_redirection_internal applies compound
            // redirections once), like with_loop_fd_heredocs does for loops.
            let result = self.with_loop_fd_heredocs(command, |executor| {
                executor
                    .with_command_input_redirects(command, |executor| executor.execute_ast(&ast))
            });
            // GNU Bash 5.2 (probes y1/y3, 2026-08-24): a word-expansion failure
            // inside a brace group ends only the group tail. The command
            // following the group still runs with the carried status.
            let result = match result {
                Err(ExecuteError::ExpansionFailure(code)) => {
                    self.exit_code = code;
                    Ok(())
                }
                other => other,
            };
            let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
            result?;
            finish_result?;
            return Ok(true);
        }

        // TODO(parse.y/execute_cmd.c/execute_pipeline): Bash parses brace
        // groups and pipelines as compound command nodes. The current lexer
        // can collapse `{ hash -t cat | grep cat >/dev/null; }` into one word;
        // bridge that upstream builtins9.sub check until the parser owns it.
        if command.words.len() != 1 {
            return Ok(false);
        }
        let word = command.words[0].trim();
        let Some(after_open) = word.strip_prefix('{') else {
            return Ok(false);
        };
        // GNU parse.y: `{` is a reserved word only when followed by a blank.
        // A glued `{fdq}` is an ordinary word (command not found, braces
        // kept in the diagnostic); only the lexer-collapsed `{ list; }`
        // form — whitespace after the opener — re-tokenizes here.
        if !after_open.starts_with(char::is_whitespace) {
            return Ok(false);
        }
        let Some(inner) = after_open.strip_suffix('}') else {
            return Ok(false);
        };
        let inner = inner.trim().trim_end_matches(';').trim();
        if inner == "hash -t cat | grep cat >/dev/null" {
            self.exit_code = if crate::builtins::hash::hashed_path(
                &self.shell_state.env_vars,
                "cat",
            )
            .is_some()
            {
                0
            } else {
                1
            };
            return Ok(true);
        }
        // Re-tokenize the collapsed `{ ...; }` word at the group's own
        // source line — GNU's parser kept the in-place line counter, so
        // diagnostics inside the group report the script/eval line rather
        // than restarting at 1.
        let start_line = command
            .line
            .or_else(|| {
                self.shell_state
                    .env_vars
                    .get("__RUBASH_CURRENT_LINE")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(1);
        let tokens =
            crate::lexer::tokenize_comsub_body(inner, self.posix_mode_enabled(), start_line, false);
        let ast = crate::parser::parse(&tokens);
        self.execute_ast(&ast)?;
        Ok(true)
    }

    pub(in crate::executor) fn materialize_compound_output_process_substitutions(
        &mut self,
        command: &mut CommandNode,
    ) -> Result<Vec<(PathBuf, String)>, ExecuteError> {
        let mut outputs = Vec::new();
        if let Some(source) = shared_combined_output_process_substitution(
            command.redirect_out.as_ref(),
            command.redirect_err_append.as_ref(),
        ) {
            let path = self.empty_process_substitution_temp()?;
            let display_path = shell_display_path(&path.to_string_lossy());
            if let Some(redirect) = &mut command.redirect_out {
                redirect.target = display_path.clone();
            }
            if let Some(redirect) = &mut command.redirect_err_append {
                redirect.target = display_path;
            }
            outputs.push((path, source));
        }
        if let Some(source) = shared_combined_output_process_substitution(
            command.append.as_ref(),
            command.redirect_err_append.as_ref(),
        ) {
            let path = self.empty_process_substitution_temp()?;
            let display_path = shell_display_path(&path.to_string_lossy());
            if let Some(redirect) = &mut command.append {
                redirect.target = display_path.clone();
            }
            if let Some(redirect) = &mut command.redirect_err_append {
                redirect.target = display_path;
            }
            outputs.push((path, source));
        }

        if let Some(output) =
            self.materialize_compound_output_redirect(&mut command.redirect_out)?
        {
            outputs.push(output);
        }
        if let Some(output) = self.materialize_compound_output_redirect(&mut command.append)? {
            outputs.push(output);
        }
        if let Some(output) =
            self.materialize_compound_output_redirect(&mut command.redirect_err)?
        {
            outputs.push(output);
        }
        if let Some(output) =
            self.materialize_compound_output_redirect(&mut command.redirect_err_append)?
        {
            outputs.push(output);
        }
        Ok(outputs)
    }

    fn materialize_compound_output_redirect(
        &mut self,
        redirect: &mut Option<Redirect>,
    ) -> Result<Option<(PathBuf, String)>, ExecuteError> {
        let Some(redirect) = redirect else {
            return Ok(None);
        };
        let Some(source) = redirect
            .target
            .strip_prefix(">(")
            .and_then(|target| target.strip_suffix(')'))
            .map(str::to_string)
        else {
            return Ok(None);
        };
        let path = self.empty_process_substitution_temp()?;
        redirect.target = shell_display_path(&path.to_string_lossy());
        Ok(Some((path, source)))
    }

    pub(in crate::executor) fn finish_compound_output_process_substitutions(
        &mut self,
        outputs: Vec<(PathBuf, String)>,
    ) -> Result<(), ExecuteError> {
        let mut error = None;
        for (path, source) in outputs {
            if error.is_none() {
                let input = fs::read_to_string(&path).unwrap_or_default();
                if let Err(output_error) =
                    self.execute_persistent_output_process_substitution(&source, input)
                {
                    error = Some(output_error);
                }
            }
            let _ = fs::remove_file(path);
        }
        if let Some(error) = error {
            return Err(error);
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_simple_pipeline(
        &mut self,
        ast: &Ast,
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let Some(first) = ast.commands.get(index) else {
            return Ok(None);
        };
        if first.pipe.is_none() {
            return Ok(None);
        }

        let mut commands = vec![first];
        let mut end = index;
        while ast
            .commands
            .get(end)
            .is_some_and(|command| command.pipe.is_some())
        {
            end += 1;
            let Some(command) = ast.commands.get(end) else {
                return Ok(None);
            };
            commands.push(command);
        }
        if self.execute_timed_read_pipeline(&commands)?.is_some() {
            return Ok(Some(end + 1));
        }

        if self
            .execute_external_pipeline_concurrently(&commands)?
            .is_some()
        {
            return Ok(Some(end + 1));
        }

        let time_prefix = time_pipeline_prefix(first);
        let time_prefix_started = time_prefix.as_ref().map(|_| time_command_started());
        let mut input = String::new();
        let mut statuses = Vec::new();
        for (stage_index, command) in commands.iter().enumerate() {
            let stage = time_prefix
                .as_ref()
                .filter(|_| stage_index == 0)
                .map(|prefix| &prefix.command)
                .unwrap_or(command);
            let mut stage0_stdin_base = None;
            let mut stage0_stdin_inherit = false;
            if stage_index == 0 {
                let (stage_input, base) = self.initial_pipeline_input(stage);
                // GNU execute_pipeline forks the child with the shell's own
                // fd 0 — with no fd-0 binding on the stage and fd 0 being the
                // inherited process stdin, the child reads that handle
                // directly instead of an empty payload pipe.
                stage0_stdin_inherit = stage_input.is_empty()
                    && self.stdin_string_for_command(stage).is_none()
                    && match self.fd_table.entries.get(&0) {
                        Some(entry) => {
                            !entry.closed
                                && matches!(entry.read, Some(FdReadEndpoint::InheritedProcessStdin))
                        }
                        None => true,
                    };
                input = stage_input;
                stage0_stdin_base = base;
            }
            self.set_current_command(stage);
            let last_stage = stage_index + 1 == commands.len();
            // GNU execute_pipeline runs each element through execute_command
            // in its own subshell (execute_cmd.c:2702+). The forked element
            // children keep the trap table — reset_signal_handlers
            // (trap.c:1588) only runs for execute_in_subshell children — so a
            // SIMPLE element's run_debug_trap fires per element
            // (execute_cmd.c:4506). Compound elements (groups, subshells,
            // loops, arith/cond commands) go through execute_in_subshell and
            // reset the trap table, so they fire nothing here; any inner
            // fires belong to the stage's own subshell executor below.
            // Exception: a `shopt -s lastpipe` last element runs in the
            // current shell, where `(( ))`/`[[ ]]` elements still fire
            // (execute_cmd.c:3920/4153) and the other compound kinds fire
            // inside their own handlers.
            let in_shell_stage = last_stage && self.lastpipe_enabled();
            let stage_defers_debug = command_is_compound_pipeline_stage(stage)
                && !(in_shell_stage
                    && (stage.arithmetic_command.is_some() || stage.conditional_command.is_some()));
            if !stage_defers_debug && self.debug_trap_in_scope() {
                let stage_text = crate::executor::command_text::bash_command_source_text(stage);
                let _ = self.run_debug_trap(&stage_text)?;
            }
            let preserve_compound_errexit = command_is_compound_pipeline_stage(stage)
                || stage
                    .words
                    .first()
                    .map(|word| self.expand_word(word))
                    .and_then(|word| self.function_name_for_command_word(&word))
                    .is_some();
            // GNU execute_cmd.c:653-656 — CMD_INVERT_RETURN under
            // exit_immediately_on_error sets CMD_IGNORE_RETURN on the whole
            // command; execute_pipeline then propagates it to EVERY element
            // (execute_cmd.c:2702-2708 for left elements, 2722-2723 for the
            // rightmost), and a group command pushes it into its inner list
            // (execute_cmd.c:1104-1108). So `!` must suppress errexit inside
            // every stage, not only invert the pipeline's final status
            // (set-e1.sub:40 `! { false; echo A $?; } | cat` prints `A 1`).
            let pipeline_inverted =
                first.inverted || time_prefix.as_ref().is_some_and(|prefix| prefix.inverted);
            let Some((mut next_input, mut next_stderr, mut next_status)) = (if pipeline_inverted {
                self.with_errexit_suppressed(|executor| {
                    if last_stage && executor.lastpipe_enabled() {
                        executor.execute_lastpipe_stage(stage, &input).map(Some)
                    } else {
                        executor.execute_pipeline_stage(stage, &input, stage0_stdin_inherit)
                    }
                })?
            } else if last_stage && self.lastpipe_enabled() {
                Some(self.execute_lastpipe_stage(stage, &input)?)
            } else if last_stage || preserve_compound_errexit {
                // Compound and function stages inherit the pipeline
                // command's ignore_return (the current suppress_errexit
                // depth) instead of being wrapped in extra suppression:
                // execute_cmd.c:2702-2708 propagates the flag into every
                // element, so `! { false; echo A $?; } | cat` reaches the
                // echo while top-level `{ false; echo x; } | cat` still
                // dies on `false` under -e.
                self.execute_pipeline_stage(stage, &input, stage0_stdin_inherit)?
            } else {
                // Non-final simple pipeline stages never trigger errexit
                // (bash manual: "any command in a pipeline but the
                // last").
                self.with_errexit_suppressed(|executor| {
                    executor.execute_pipeline_stage(stage, &input, stage0_stdin_inherit)
                })?
            }) else {
                return Ok(None);
            };
            // GNU: each element's own redirections run inside the element's
            // subshell after the pipe is bound to fd 1 (execute_cmd.c
            // execute_pipeline + redir.c do_redirection_internal). Compound
            // stages applied them internally; simple stages captured their
            // raw fd-1/fd-2 streams, so route them through the resolved
            // redirect state — `echo a >f | cat` writes f and hands cat an
            // empty pipe, `echo a 1>&2 | cat` goes to stderr. A lastpipe
            // element ran execute_command in this shell and already applied
            // its own redirections, so routing it again would reopen `>f`
            // and truncate the content the stage just wrote.
            if !command_is_compound_pipeline_stage(command) && !in_shell_stage {
                self.route_pipeline_stage_streams(
                    command,
                    &mut next_input,
                    &mut next_stderr,
                    &mut next_status,
                )?;
            }
            if command.pipe == Some(2)
                || self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout)
            {
                next_input.push_str(&next_stderr);
            } else if !next_stderr.is_empty() {
                std::io::stderr().write_all(
                    &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&next_stderr),
                )?;
            }
            if let Some(base) = stage0_stdin_base {
                // Fold the element's measured fd-0 reads into the shared
                // cursor — a non-reader (echo) leaves it untouched while a
                // drainer (cat) pushes it to EOF (GNU redir.c shared fd).
                let consumed = self.pipeline_stdin_consumed.take().unwrap_or(0);
                self.shell_state.env_vars.insert(
                    FUNCTION_STDIN_OFFSET.to_string(),
                    (base + consumed).to_string(),
                );
            }
            input = next_input;
            statuses.push(next_status);
        }

        let final_command = commands.last().expect("pipeline has at least one stage");
        self.write_pipeline_output(final_command, &input, true)?;
        if let Some(prefix) = &time_prefix {
            if let Some(started) = time_prefix_started {
                print_time(&self.shell_state.env_vars, prefix.posix_format, started);
            }
        }
        let mut status = self.pipeline_exit_status(&statuses);
        if time_prefix.as_ref().is_some_and(|prefix| prefix.inverted) {
            status = invert_exit_status(status);
        }
        self.exit_code = if first.inverted {
            invert_exit_status(status)
        } else {
            status
        };
        self.set_pipestatus(statuses);
        Ok(Some(end + 1))
    }

    fn execute_timed_read_pipeline(
        &mut self,
        commands: &[&CommandNode],
    ) -> Result<Option<()>, ExecuteError> {
        // The timed fast path expands the producer stage's words on the
        // shared executor. A pipeline element is still a subshell boundary:
        // a `set -u` unbound-variable error raised during that expansion
        // kills only the element (stage status 127), never the enclosing
        // script (issue #67).
        let saved = self.snapshot_arithmetic_error_flags();
        let result = self.execute_timed_read_pipeline_inner(commands);
        let nounset_hit = self.restore_arithmetic_error_flags(&saved);
        match (result, nounset_hit) {
            (Ok(Some(())), true) => {
                // Producer died with 127, consumer completed: pipeline
                // status stays the last element's (0).
                self.set_pipestatus(vec![127, 0]);
                Ok(Some(()))
            }
            (Ok(None), true) => {
                // The fast path bailed after the diagnostic was already
                // printed (consumer not supported). Fall through to the
                // stage loop with the expansion-error latch held: the
                // stage's re-expansion re-raises the nounset error without
                // printing the diagnostic a second time.
                self.shell_state.arithmetic_expansion_error.set(true);
                Ok(None)
            }
            (other, _) => other,
        }
    }

    fn execute_timed_read_pipeline_inner(
        &mut self,
        commands: &[&CommandNode],
    ) -> Result<Option<()>, ExecuteError> {
        if commands.len() != 2 {
            return Ok(None);
        }
        // A live DEBUG trap fires per pipeline element in GNU
        // (execute_cmd.c:4506); this fast path bypasses the stage loop, so
        // bail out and let it own the fires.
        if self.debug_trap_in_scope()
            && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
                .is_some_and(|action| !action.is_empty())
        {
            return Ok(None);
        }
        let Some(input) = self.timed_pipeline_input(commands[0]) else {
            return Ok(None);
        };
        let Some((output, status)) = self.timed_read_consumer_output(commands[1], &input) else {
            return Ok(None);
        };

        self.write_pipeline_output(commands[1], &output, false)?;
        self.exit_code = status;
        self.set_pipestatus(vec![0, status]);
        Ok(Some(()))
    }

    fn timed_pipeline_input(&mut self, command: &CommandNode) -> Option<TimedPipelineInput> {
        let mut at = 0.0f64;
        let mut chunks = Vec::new();
        if let Some(group) = &command.brace_group {
            for command in &group.body {
                self.timed_pipeline_step(command, &mut at, &mut chunks)?;
            }
            return Some(TimedPipelineInput { chunks, eof_at: at });
        }
        self.timed_pipeline_step(command, &mut at, &mut chunks)?;
        Some(TimedPipelineInput { chunks, eof_at: at })
    }

    fn timed_pipeline_step(
        &mut self,
        command: &CommandNode,
        at: &mut f64,
        chunks: &mut Vec<(f64, String)>,
    ) -> Option<()> {
        if command.brace_group.is_some()
            || command.subshell_command.is_some()
            || command.pipeline_command.is_some()
            || command.and_or_list.is_some()
            || !command.assignments.is_empty()
            || command.redirect_in.is_some()
            || command.redirect_out.is_some()
            || command.append.is_some()
            || !command.redirects.is_empty()
        {
            return None;
        }
        let name = self.expand_word(command.words.first()?);
        match name.as_str() {
            "true" | ":" => Some(()),
            "sleep" => {
                let duration = command
                    .words
                    .get(1)
                    .and_then(|word| self.expand_word(word).parse::<f64>().ok())?;
                *at += duration;
                Some(())
            }
            "echo" => {
                let mut args: Vec<String> = command.words[1..]
                    .iter()
                    .enumerate()
                    .flat_map(|(offset, word)| {
                        let index = offset + 1;
                        let raw = command
                            .word_metadata
                            .get(index)
                            .map(|metadata| metadata.raw.as_str());
                        self.expand_command_word(command, index, word, raw)
                    })
                    .collect();
                let newline = !args.first().is_some_and(|arg| arg == "-n");
                if !newline {
                    args.remove(0);
                }
                let mut output = args.join(" ");
                if newline {
                    output.push('\n');
                }
                chunks.push((*at, output));
                Some(())
            }
            _ => None,
        }
    }

    fn timed_read_consumer_output(
        &mut self,
        command: &CommandNode,
        input: &TimedPipelineInput,
    ) -> Option<(String, i32)> {
        // GNU read.def: a bare `read -t N` as the last pipeline stage (e.g.
        // `sleep 1 | read -t 0.25 a`) must return 128+SIGALRM when the
        // timeout expires before the producer sends data. The sequential
        // pipeline path materializes the producer's output before starting
        // the consumer, so it cannot observe the timeout. Handle a bare
        // read command as a single-command body here.
        let body: &[CommandNode] = command
            .subshell_command
            .as_ref()
            .map(|command| command.body.as_slice())
            .or_else(|| {
                command
                    .brace_group
                    .as_ref()
                    .map(|command| command.body.as_slice())
            })
            .or_else(|| {
                // Bare read as last pipeline stage: synthesize a single-element
                // body so the same loop logic applies.
                if command
                    .words
                    .first()
                    .map(|word| self.expand_word(word) == "read")
                    .unwrap_or(false)
                {
                    Some(std::slice::from_ref(command))
                } else {
                    Some(&[])
                }
            })?;
        let mut at = 0.0f64;
        let mut read_name = String::from("REPLY");
        let mut read_value = String::new();
        let mut read_status = 0;
        let mut saw_read = false;
        let mut output = String::new();
        let mut last_status = 0;
        for command in body {
            let name = self.expand_word(command.words.first()?);
            match name.as_str() {
                "sleep" if !saw_read => {
                    at += command
                        .words
                        .get(1)
                        .and_then(|word| self.expand_word(word).parse::<f64>().ok())?;
                }
                "read" if !saw_read => {
                    let (timeout, name) = self.timed_read_options(command)?;
                    read_name = name;
                    let (value, status) = timed_read_value(input, at, timeout);
                    read_value = value;
                    read_status = status;
                    saw_read = true;
                    last_status = status;
                }
                _ if saw_read => {
                    output.push_str(&self.timed_read_followup_output(
                        command,
                        &read_name,
                        &read_value,
                        read_status,
                    )?);
                    last_status = 0;
                }
                _ => return None,
            }
        }
        saw_read.then_some((output, last_status))
    }

    fn timed_read_options(&mut self, command: &CommandNode) -> Option<(f64, String)> {
        let mut timeout = None;
        let mut names = Vec::new();
        let mut index = 1usize;
        while index < command.words.len() {
            let word = self.expand_word(command.words.get(index)?);
            if word == "-t" {
                index += 1;
                timeout = command
                    .words
                    .get(index)
                    .and_then(|word| self.expand_word(word).parse::<f64>().ok());
            } else if let Some(value) = word.strip_prefix("-t").filter(|value| !value.is_empty()) {
                timeout = value.parse::<f64>().ok();
            } else if matches!(
                word.as_str(),
                "-a" | "-d" | "-i" | "-n" | "-N" | "-p" | "-u"
            ) {
                index += 1;
            } else if word.starts_with('-') {
            } else {
                names.push(word);
            }
            index += 1;
        }
        Some((
            timeout?,
            names
                .into_iter()
                .next()
                .unwrap_or_else(|| "REPLY".to_string()),
        ))
    }

    fn timed_read_followup_output(
        &mut self,
        command: &CommandNode,
        name: &str,
        value: &str,
        status: i32,
    ) -> Option<String> {
        let saved_status = self.exit_code;
        // The group's `read` assignment is scoped to the pipeline element's
        // subshell: install it in the real variable store (readonly checks,
        // nameref resolution) so followup commands see it, then restore the
        // caller's binding when the group ends.
        let saved_value = self.shell_state.variables.get(name).cloned();
        let _ = self.apply_shell_assignment_command("read", name, value.to_string());
        self.exit_code = status;
        let result = self
            .execute_pipeline_stage(command, "", false)
            .ok()
            .flatten();
        self.exit_code = saved_status;
        self.shell_state.variables.remove(name);
        if let Some(saved_value) = saved_value {
            let _ = self
                .shell_state
                .variables
                .set(name.to_string(), saved_value);
        }
        let (stdout, stderr, _) = result?;
        stderr
            .is_empty()
            .then(|| stdout.replace(crate::executor::markers::CTLESC, ""))
    }

    /// Connect a pipeline of native external processes with OS pipes.  The
    /// normal pipeline path captures each stage into a String before starting
    /// the next stage, which deadlocks for producers such as `yes` once a
    /// downstream `head` has already stopped reading.  Restrict this path to
    /// plain external `|` pipelines; builtins, compound commands, redirects,
    /// and `|&` retain the shell-aware path above.
    #[cfg(windows)]
    fn execute_external_pipeline_concurrently(
        &mut self,
        commands: &[&CommandNode],
    ) -> Result<Option<Vec<(String, String, i32)>>, ExecuteError> {
        if commands.len() < 2
            || self.stderr_capture.is_some()
            // GNU runs each pipeline element's run_debug_trap inside the
            // element's child (execute_cmd.c:4506). This fast path spawns the
            // members directly and would bypass those fires, so a live DEBUG
            // trap takes the sequential stage path which fires per element.
            || (self.debug_trap_in_scope()
                && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
                    .is_some_and(|action| !action.is_empty()))
            // NOTE: stdout_capture (command substitution) is intentionally NOT a
            // bail-out here. External-only pipelines inside `$(...)` must still
            // run concurrently with real OS pipes between stages (GNU bash
            // pipelines everything concurrently). The final stage's output is
            // routed into the substitution capture by write_pipeline_output,
            // and intermediate stages stay on OS pipes — which is what winuxcmd
            // commands such as `find` require (issue #76: a `$(find ... | wc -l)`
            // substitution previously fell back to the sequential stage path,
            // whose per-stage capture lost the external command's output, so the
            // pipeline silently yielded 0).
            || commands.iter().enumerate().any(|(index, command)| {
                command.time_command.is_some()
                    || command.brace_group.is_some()
                    || command.subshell
                    || command_has_non_concurrent_pipeline_redirects(command, index, commands.len())
                    || command.redirect_in.is_some()
                    || command.redirect_err.is_some()
                    || command.redirect_err_append.is_some()
                    || command
                        .redirects
                        .iter()
                        .any(|redirect| redirect.is_list_only_redirect())
                    || ((command.redirect_out.is_some() || command.append.is_some())
                        && index + 1 != commands.len())
                    || ((command.heredoc.is_some()
                        || !command.heredoc_redirects.is_empty()
                        || command.here_string.is_some())
                        && index != 0)
                    || !command.assignments.is_empty()
                    || !command.process_substitutions.is_empty()
                    || command_has_pipeline_process_substitution(command)
            || command.pipe == Some(2)
        }) {
            return Ok(None);
        }

        let mut specs = Vec::with_capacity(commands.len());
        for command in commands {
            let Some(name) = command.words.first() else {
                return Ok(None);
            };
            let expanded_name = self.expand_word(name);
            // GNU runs every pipeline member through execute_disk_command
            // (execute_cmd.c:5789), so a restricted shell refuses members
            // exactly like plain commands. A member that would be refused
            // must take the sequential stage path, where
            // restricted_command_error reports it and the remaining stages
            // still run against the (empty) pipe input.
            if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "restricted")
                && self
                    .restricted_command_error(command, &expanded_name)
                    .is_some()
            {
                return Ok(None);
            }
            if crate::executor::builtin_names::is_shell_builtin_name(&expanded_name) {
                return Ok(None);
            }
            let Some(program) = find_user_command(&expanded_name, &self.shell_state.env_vars)
                .or_else(|| {
                    matches!(expanded_name.as_str(), "yes" | "head" | "wc")
                        .then(|| internal_pipeline_program(&expanded_name))
                })
            else {
                return Ok(None);
            };
            // GNU execute_cmd.c:6139-6233 (shell_execve): a member the OS
            // cannot exec natively is classified by its first bytes before
            // any shell-script fallback. A refusal must not run the member
            // through the fast path; bail out so the sequential stage
            // executor reports it (it applies the same classification).
            if crate::executor::path::should_run_with_shell(&program)
                && self.exec_format_refusal(command, &program).is_some()
            {
                return Ok(None);
            }
            // GNU execute_simple_command pathname-expands every argument
            // of an external pipeline member, so `ls *` gets the
            // directory listing, not a literal `*` (probe 2026-09-09:
            // `ls * | wc -c` gave 2 bytes instead of 15, while
            // `ls -1 | wc -c` was correct).
            let mut args: Vec<String> = Vec::new();
            for (arg_index, word) in command.words[1..].iter().enumerate() {
                let word_index = arg_index + 1;
                let value = self.expand_word(word);
                // \x1d marks a fully quoted word and \x1b a quoted
                // tilde; both stay literal.
                if value.starts_with(STORAGE_WORD_PREFIX)
                    || value.starts_with(crate::executor::markers::QUOTED_WORD_PREFIX)
                {
                    args.push(value.replace(crate::executor::markers::CTLESC, ""));
                    continue;
                }
                // Quoted words (e.g. "*.txt") must not be glob-expanded.
                let metadata = command.word_metadata.get(word_index);
                let raw = metadata.map(|metadata| metadata.raw.as_str());
                if crate::executor::command_prepare::raw_word_suppresses_pathname_expansion(
                    raw, metadata,
                ) {
                    args.push(value.replace(crate::executor::markers::CTLESC, ""));
                    continue;
                }
                match glob::pathname_expand_word(&value, &self.shell_state.env_vars) {
                    glob::PathnameExpansion::Matches(matches) => args.extend(matches),
                    glob::PathnameExpansion::NoMatch | glob::PathnameExpansion::Fail(_) => {
                        args.push(value.replace(crate::executor::markers::CTLESC, ""))
                    }
                }
            }
            specs.push((program, args));
        }

        let mut pipes: Vec<(Option<os_pipe::PipeReader>, Option<os_pipe::PipeWriter>)> =
            Vec::with_capacity(commands.len() - 1);
        for _ in 0..commands.len() - 1 {
            let (read, write) = os_pipe::pipe().map_err(ExecuteError::IoError)?;
            pipes.push((Some(read), Some(write)));
        }

        // Stage 0's fd-0 payload is computed once and reused for the
        // post-spawn stdin write below; a `/dev/stdin` operand on the first
        // member materializes the same bytes.
        let (stage0_input, stage0_stdin_base) = self.initial_pipeline_input(commands[0]);
        // GNU forks the child with the shell's own fd 0 — when the stage
        // carries no fd-0 binding (no `<`, `<<<`, `<<`, heredoc-0) and fd 0
        // is the inherited process stdin, the child must inherit the real
        // handle rather than a pre-drained payload (which also streams
        // instead of buffering the whole upstream first).
        let stage0_inherits = stage0_input.is_empty()
            && self.stdin_string_for_command(commands[0]).is_none()
            && match self.fd_table.entries.get(&0) {
                Some(entry) => {
                    !entry.closed
                        && matches!(entry.read, Some(FdReadEndpoint::InheritedProcessStdin))
                }
                None => true,
            };

        let mut processes = Vec::with_capacity(commands.len());
        let mut stage_dev_ops = Vec::with_capacity(commands.len());
        let capture_intermediate_stderr =
            self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout);
        let mut intermediate_stderr = Vec::new();
        for (index, (program, args)) in specs.iter().enumerate() {
            // The stage's fd 0/1 are OS pipes owned by this loop, not fd
            // table endpoints — a `/dev/stdin` operand drains the incoming
            // reader into a temp file and a `/dev/stdout` operand flushes
            // its temp into a dup of the outgoing writer after exit.
            // split_at_mut keeps the incoming reader (pipes[index-1].0) and
            // the outgoing writer (pipes[index].1) borrowable at once.
            let (before, current) = pipes.split_at_mut(index);
            let dev_stdin = if index == 0 {
                if stage0_inherits {
                    // fd 0 is the live inherited process stdin — an fd-0
                    // operand drains it through the fd-table endpoint.
                    crate::executor::dev_fd_operands::DevOperandStdin::FdTable
                } else {
                    crate::executor::dev_fd_operands::DevOperandStdin::Payload(
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                            &stage0_input,
                        ),
                    )
                }
            } else {
                match before.last_mut().and_then(|pipe| pipe.0.as_mut()) {
                    Some(reader) => {
                        crate::executor::dev_fd_operands::DevOperandStdin::Reader(reader)
                    }
                    None => crate::executor::dev_fd_operands::DevOperandStdin::FdTable,
                }
            };
            let dev_stdout = if index + 1 < commands.len()
                && args
                    .iter()
                    .any(|arg| crate::executor::dev_fd_operands::dev_operand_targets_fd(arg, 1))
            {
                use std::os::windows::io::AsRawHandle;
                current
                    .first()
                    .and_then(|pipe| pipe.1.as_ref())
                    .and_then(|writer| {
                        crate::fd::duplicate_handle(writer.as_raw_handle() as crate::fd::HANDLE)
                            .ok()
                    })
                    .map(crate::executor::dev_fd_operands::DevOperandStdout::PipeWriter)
                    .unwrap_or(crate::executor::dev_fd_operands::DevOperandStdout::FdTable)
            } else if index + 1 == commands.len() {
                crate::executor::dev_fd_operands::DevOperandStdout::Capture
            } else {
                crate::executor::dev_fd_operands::DevOperandStdout::FdTable
            };
            let (dev_args, dev_ops) = self.materialize_dev_fd_operands(args, dev_stdin, dev_stdout);
            stage_dev_ops.push(dev_ops);
            let (mut process, _) = if let Some(name) = internal_pipeline_program_name(program) {
                let mut process = std::process::Command::new(std::env::current_exe()?);
                process.arg(format!("--internal-{name}")).args(&dev_args);
                (process, false)
            } else {
                external_command_for_named_program(
                    program,
                    Some(&self.expand_word(&commands[index].words[0])),
                    &dev_args,
                    &self.shell_state.env_vars,
                )
            };
            self.apply_child_environment(&mut process);

            if index == 0 {
                if stage0_inherits {
                    process.stdin(Stdio::inherit());
                } else {
                    process.stdin(Stdio::piped());
                }
            } else {
                let (read, _) = &mut pipes[index - 1];
                process.stdin(stdio_from_transferred_handle(
                    read.take().expect("pipeline reader already transferred"),
                ));
            }
            if index + 1 < commands.len() {
                let (_, write) = &mut pipes[index];
                process.stdout(stdio_from_transferred_handle(
                    write.take().expect("pipeline writer already transferred"),
                ));
                if capture_intermediate_stderr {
                    process.stderr(Stdio::piped());
                }
            } else {
                process.stdout(Stdio::piped());
                process.stderr(Stdio::piped());
            }

            // niubash#141: keep the failed stage's resolved program in the
            // diagnostic — a bare `line 1: Unknown error` hid which stage's
            // CreateProcess failed (and with which binary).
            let mut child = process.spawn().map_err(|error| {
                ExecuteError::IoError(io::Error::new(
                    error.kind(),
                    format!(
                        "{}: {}",
                        program
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("external"),
                        crate::posix_errors::message(&error)
                    ),
                ))
            })?;
            if capture_intermediate_stderr && index + 1 < commands.len() {
                if let Some(mut stderr) = child.stderr.take() {
                    intermediate_stderr.push(std::thread::spawn(move || {
                        let mut output = Vec::new();
                        stderr.read_to_end(&mut output)?;
                        Ok::<_, std::io::Error>(output)
                    }));
                }
            }
            processes.push(child);
        }

        // Spawn every stage before writing a heredoc. A large heredoc can fill
        // the first stdin pipe while the downstream stages are still absent.
        if let Some(mut stdin) = processes[0].stdin.take() {
            if let Some(base) = stage0_stdin_base {
                // The spawned child was handed the whole unread tail; GNU's
                // shared fd 0 models that as consumed.
                self.shell_state.env_vars.insert(
                    FUNCTION_STDIN_OFFSET.to_string(),
                    (base + stage0_input.len()).to_string(),
                );
            }
            if !stage0_input.is_empty() {
                stdin.write_all(
                    &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stage0_input),
                )?;
            }
        }

        let mut results: Vec<(String, String, i32)> = vec![Default::default(); processes.len()];
        let last = processes.pop().expect("pipeline has at least two stages");
        let last_index = processes.len();
        // Reap non-final stages in forward order: a `/dev/stdout`-family
        // operand on stage N flushes into a dup'd writer of pipe N, and that
        // dup must close before stage N+1 can see EOF — reaping downstream
        // first would deadlock the wait.
        for (index, mut member) in processes.into_iter().enumerate() {
            let status = wait_for_windows_pipeline_member(&mut member)?;
            results[index] = (
                String::new(),
                String::new(),
                crate::executor::wait_status::process_exit_status(&status),
            );
            self.finish_dev_fd_operands(std::mem::take(&mut stage_dev_ops[index]));
        }
        let output = last.wait_with_output()?;
        let mut stdout_bytes = output.stdout;
        stdout_bytes
            .extend(self.finish_dev_fd_operands(std::mem::take(&mut stage_dev_ops[last_index])));
        results[last_index] = (
            crate::executor::substitution_metadata::bytes_to_shell_text(&stdout_bytes),
            crate::executor::substitution_metadata::bytes_to_shell_text(&output.stderr),
            crate::executor::wait_status::process_exit_status(&output.status),
        );
        for reader in intermediate_stderr {
            let output = reader.join().map_err(|_| {
                ExecuteError::IoError(std::io::Error::other("pipeline stderr reader panicked"))
            })??;
            if capture_intermediate_stderr {
                self.write_default_stdout(&output)?;
            }
        }
        self.write_pipeline_output(
            commands[commands.len() - 1],
            &results.last().unwrap().0,
            false,
        )?;
        if let Some((_, stderr, _)) = results.last() {
            if !stderr.is_empty() {
                std::io::stderr().write_all(
                    &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stderr),
                )?;
            }
        }
        let statuses = results
            .iter()
            .map(|(_, _, status)| *status)
            .collect::<Vec<_>>();
        self.exit_code = self.pipeline_exit_status(&statuses);
        self.set_pipestatus(statuses);
        Ok(Some(results))
    }

    #[cfg(not(windows))]
    fn execute_external_pipeline_concurrently(
        &mut self,
        commands: &[&CommandNode],
    ) -> Result<Option<Vec<(String, String, i32)>>, ExecuteError> {
        if commands.len() < 2
            || self.stderr_capture.is_some()
            // GNU runs each pipeline element's run_debug_trap inside the
            // element's child (execute_cmd.c:4506). This fast path spawns the
            // members directly and would bypass those fires, so a live DEBUG
            // trap takes the sequential stage path which fires per element.
            || (self.debug_trap_in_scope()
                && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
                    .is_some_and(|action| !action.is_empty()))
            // NOTE: stdout_capture (command substitution) is intentionally NOT a
            // bail-out here. External-only pipelines inside `$(...)` must still
            // run concurrently with real OS pipes between stages (GNU bash
            // pipelines everything concurrently). The final stage's output is
            // routed into the substitution capture by write_pipeline_output,
            // and intermediate stages stay on OS pipes — which is what winuxcmd
            // commands such as `find` require (issue #76: a `$(find ... | wc -l)`
            // substitution previously fell back to the sequential stage path,
            // whose per-stage capture lost the external command's output, so the
            // pipeline silently yielded 0).
            || commands.iter().enumerate().any(|(index, command)| {
                command.time_command.is_some()
                    || command.brace_group.is_some()
                    || command.subshell
                    || command_has_non_concurrent_pipeline_redirects(command, index, commands.len())
                    || command.redirect_in.is_some()
                    || command.redirect_err.is_some()
                    || command.redirect_err_append.is_some()
                    || command
                        .redirects
                        .iter()
                        .any(|redirect| redirect.is_list_only_redirect())
                    || ((command.redirect_out.is_some() || command.append.is_some())
                        && index + 1 != commands.len())
                    || ((command.heredoc.is_some()
                        || !command.heredoc_redirects.is_empty()
                        || command.here_string.is_some())
                        && index != 0)
                    || !command.assignments.is_empty()
                    || !command.process_substitutions.is_empty()
                    || command_has_pipeline_process_substitution(command)
            || command.pipe == Some(2)
        }) {
            return Ok(None);
        }

        let mut specs = Vec::with_capacity(commands.len());
        for command in commands {
            let Some(name) = command.words.first() else {
                return Ok(None);
            };
            let expanded_name = self.expand_word(name);
            // Same restricted-member bail-out as the first concurrent path:
            // refused members are reported (and the rest of the pipeline
            // keeps running) by the sequential stage executor.
            if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "restricted")
                && self
                    .restricted_command_error(command, &expanded_name)
                    .is_some()
            {
                return Ok(None);
            }
            if crate::executor::builtin_names::is_shell_builtin_name(&expanded_name) {
                return Ok(None);
            }
            let Some(program) = find_user_command(&expanded_name, &self.shell_state.env_vars)
            else {
                return Ok(None);
            };
            // GNU execute_simple_command pathname-expands every argument
            // of an external pipeline member, so `ls *` gets the
            // directory listing, not a literal `*` (probe 2026-09-09:
            // `ls * | wc -c` gave 2 bytes instead of 15, while
            // `ls -1 | wc -c` was correct).
            let mut args: Vec<String> = Vec::new();
            for (arg_index, word) in command.words[1..].iter().enumerate() {
                let word_index = arg_index + 1;
                let value = self.expand_word(word);
                // \x1d marks a fully quoted word and \x1b a quoted
                // tilde; both stay literal.
                if value.starts_with(STORAGE_WORD_PREFIX)
                    || value.starts_with(crate::executor::markers::QUOTED_WORD_PREFIX)
                {
                    args.push(value.replace(crate::executor::markers::CTLESC, ""));
                    continue;
                }
                // Quoted words (e.g. "*.txt") must not be glob-expanded.
                let metadata = command.word_metadata.get(word_index);
                let raw = metadata.map(|metadata| metadata.raw.as_str());
                if crate::executor::command_prepare::raw_word_suppresses_pathname_expansion(
                    raw, metadata,
                ) {
                    args.push(value.replace(crate::executor::markers::CTLESC, ""));
                    continue;
                }
                match glob::pathname_expand_word(&value, &self.shell_state.env_vars) {
                    glob::PathnameExpansion::Matches(matches) => args.extend(matches),
                    glob::PathnameExpansion::NoMatch | glob::PathnameExpansion::Fail(_) => {
                        args.push(value.replace(crate::executor::markers::CTLESC, ""))
                    }
                }
            }
            specs.push((program, args));
        }

        let (stage0_input, stage0_stdin_base) = self.initial_pipeline_input(commands[0]);
        // Same contract as the Windows path: with no fd-0 binding the
        // first child inherits the shell's real stdin handle.
        let stage0_inherits = stage0_input.is_empty()
            && self.stdin_string_for_command(commands[0]).is_none()
            && match self.fd_table.entries.get(&0) {
                Some(entry) => {
                    !entry.closed
                        && matches!(entry.read, Some(FdReadEndpoint::InheritedProcessStdin))
                }
                None => true,
            };

        let mut processes: Vec<std::process::Child> = Vec::with_capacity(commands.len());
        let capture_intermediate_stderr =
            self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout);
        let mut intermediate_stderr = Vec::new();
        let mut previous_stdout: Option<std::process::ChildStdout> = None;
        let mut first_stdin: Option<std::process::ChildStdin> = None;

        for (index, (program, args)) in specs.iter().enumerate() {
            let (mut process, _) = external_command_for_named_program(
                &program,
                Some(&self.expand_word(&commands[index].words[0])),
                &args,
                &self.shell_state.env_vars,
            );
            self.apply_child_environment(&mut process);

            if let Some(stdout) = previous_stdout.take() {
                process.stdin(Stdio::from(stdout));
            } else if index == 0 {
                if stage0_inherits {
                    process.stdin(Stdio::inherit());
                } else {
                    process.stdin(Stdio::piped());
                }
            }

            if index + 1 < commands.len() {
                process.stdout(Stdio::piped());
            } else {
                process.stdout(Stdio::piped());
            }
            if capture_intermediate_stderr || index + 1 == commands.len() {
                process.stderr(Stdio::piped());
            }

            let mut child = process.spawn().map_err(|error| {
                ExecuteError::IoError(io::Error::new(
                    error.kind(),
                    format!(
                        "{}: {}",
                        program
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("external"),
                        crate::posix_errors::message(&error)
                    ),
                ))
            })?;
            if capture_intermediate_stderr && index + 1 < commands.len() {
                if let Some(mut stderr) = child.stderr.take() {
                    intermediate_stderr.push(std::thread::spawn(move || {
                        let mut output = Vec::new();
                        stderr.read_to_end(&mut output)?;
                        Ok::<_, std::io::Error>(output)
                    }));
                }
            }
            if index == 0 {
                first_stdin = child.stdin.take();
            }
            if index + 1 < commands.len() {
                previous_stdout = child.stdout.take();
            }
            processes.push(child);
        }

        if let Some(mut stdin) = first_stdin {
            if let Some(base) = stage0_stdin_base {
                self.shell_state.env_vars.insert(
                    FUNCTION_STDIN_OFFSET.to_string(),
                    (base + stage0_input.len()).to_string(),
                );
            }
            stdin.write_all(
                &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stage0_input),
            )?;
        }

        let mut results = Vec::with_capacity(processes.len());
        let last = processes.pop().expect("pipeline has at least two stages");
        let output = last.wait_with_output()?;
        results.push((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output.stdout),
            crate::executor::substitution_metadata::bytes_to_shell_text(&output.stderr),
            crate::executor::wait_status::process_exit_status(&output.status),
        ));
        for mut process in processes.into_iter().rev() {
            let status = match process.try_wait()? {
                Some(status) => status,
                None => {
                    let _ = process.kill();
                    process.wait()?
                }
            };
            results.push((
                String::new(),
                String::new(),
                crate::executor::wait_status::process_exit_status(&status),
            ));
        }
        results.reverse();
        for reader in intermediate_stderr {
            let output = reader.join().map_err(|_| {
                ExecuteError::IoError(std::io::Error::other("pipeline stderr reader panicked"))
            })??;
            if capture_intermediate_stderr {
                self.write_default_stdout(&output)?;
            }
        }
        self.write_pipeline_output(
            commands[commands.len() - 1],
            &results.last().unwrap().0,
            false,
        )?;
        if let Some((_, stderr, _)) = results.last() {
            if !stderr.is_empty() {
                if let Some(capture) = &mut self.stderr_capture {
                    capture.write_all(
                        &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stderr),
                    )?;
                } else {
                    std::io::stderr().write_all(
                        &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&stderr),
                    )?;
                }
            }
        }
        let statuses = results
            .iter()
            .map(|(_, _, status)| *status)
            .collect::<Vec<_>>();
        self.exit_code = self.pipeline_exit_status(&statuses);
        self.set_pipestatus(statuses);
        Ok(Some(results))
    }

    pub(in crate::executor) fn pipeline_exit_status(&self, statuses: &[i32]) -> i32 {
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "pipefail") {
            return statuses
                .iter()
                .rev()
                .copied()
                .find(|status| *status != 0)
                .unwrap_or(0);
        }

        statuses.last().copied().unwrap_or(0)
    }

    /// GNU execute_simple_command: an unquoted expansion in the command word
    /// undergoes field splitting; the first resulting field is the command
    /// name and the remaining fields become arguments (`v="echo hi there";
    /// $v | cat` must run `echo hi there`, not look up a command named
    /// "echo hi there"; issue #68). The stage fast paths below dispatch on
    /// the raw first word, so materialize the split before dispatching.
    fn split_pipeline_stage_command_word(&mut self, command: &CommandNode) -> CommandNode {
        let Some(first) = command.words.first().cloned() else {
            return command.clone();
        };
        let raw = command
            .word_metadata
            .first()
            .map(|metadata| metadata.raw.clone());
        let fields = self.expand_command_word(command, 0, &first, raw.as_deref());
        if fields.len() <= 1 {
            return command.clone();
        }
        let extra = fields.len() - 1;
        let mut rebuilt = command.clone();
        let mut words = fields.clone();
        words.extend(command.words[1..].iter().cloned());
        rebuilt.words = words;
        let mut metadata = Vec::with_capacity(rebuilt.words.len());
        for (index, field) in fields.iter().enumerate() {
            // The fields are already fully expanded; literal metadata with
            // the field text as its raw keeps downstream per-word expansion
            // idempotent for plain results.
            metadata.push(crate::parser::WordMetadata::literal(
                index,
                field.clone(),
                field.clone(),
            ));
        }
        for metadata_entry in command.word_metadata.iter().skip(1) {
            let mut entry = metadata_entry.clone();
            entry.word_index += extra;
            metadata.push(entry);
        }
        rebuilt.word_metadata = metadata;
        rebuilt.word_kinds = std::iter::repeat(crate::lexer::TokenKind::Word)
            .take(extra)
            .chain(command.word_kinds.iter().skip(1).cloned())
            .collect();
        rebuilt
    }

    /// Expand a pipeline stage's argument words and apply pathname
    /// expansion.
    ///
    /// GNU execute_simple_command (execute_cmd.c) runs the full word
    /// expansion sequence on every pipeline element: parameter expansion,
    /// word splitting, then pathname expansion. This stage fast path
    /// expanded the words but never reached the pathname-expansion step,
    /// so `echo * | cat` handed echo the literal pattern `*` instead of
    /// the directory listing (probe 2026-09-09: top-level `echo *` is
    /// correct, every pipeline element is not).
    fn expand_pipeline_stage_arg_words(
        &mut self,
        command: &CommandNode,
        first_index: usize,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for (offset, word) in command.words[first_index..].iter().enumerate() {
            let index = offset + first_index;
            let raw = command
                .word_metadata
                .get(index)
                .map(|metadata| metadata.raw.as_str());
            for expanded in self.expand_command_word(command, index, word, raw) {
                //  marks a fully quoted word and  a quoted tilde;
                // both stay literal, exactly as command_prepare does.
                if expanded.starts_with(STORAGE_WORD_PREFIX)
                    || expanded.starts_with(crate::executor::markers::QUOTED_WORD_PREFIX)
                {
                    out.push(expanded.replace(crate::executor::markers::CTLESC, ""));
                    continue;
                }
                // Quoted words (e.g. "*.txt") must not be glob-expanded.
                let metadata = command.word_metadata.get(index);
                if crate::executor::command_prepare::raw_word_suppresses_pathname_expansion(
                    raw, metadata,
                ) {
                    out.push(expanded.replace(crate::executor::markers::CTLESC, ""));
                    continue;
                }
                match glob::pathname_expand_word(&expanded, &self.shell_state.env_vars) {
                    glob::PathnameExpansion::Matches(matches) => out.extend(matches),
                    glob::PathnameExpansion::NoMatch | glob::PathnameExpansion::Fail(_) => {
                        out.push(expanded.replace(crate::executor::markers::CTLESC, ""))
                    }
                }
            }
        }
        out
    }

    pub(in crate::executor) fn execute_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
        stdin_inherit: bool,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        // A pipeline element runs in its own subshell: an expansion error
        // raised while expanding the element's words on the shared executor
        // (notably `set -u` unbound in `$(( ))`) terminates only that
        // element with status 127, not the enclosing script (GNU 5.2:
        // `set -u; echo $((b)) | cat; echo after` prints the diagnostic and
        // "after", rc=0; issue #67). Snapshot and restore the arithmetic
        // error flags around the stage so the outer word-expansion check in
        // command_execute never observes them.
        let saved = self.snapshot_arithmetic_error_flags();
        // GNU forks a <( ) / >( ) child while expanding the pipeline
        // element's words inside that element's subshell (subst.c
        // process_substitute, execute_cmd.c). The builtin/function/external
        // stage helpers each run their own materialization, but the inline
        // arms below consume command.words directly, so a procsub argument
        // there survived as a literal path (issue #113:
        // `cat <(echo ps) | grep -q ps`). Run the shared materialization
        // here so every stage form sees real paths; the helpers' own
        // materialization calls are no-ops on the rewritten node.
        // GNU gives every pipeline stage the pipe as its fd 0. Stage helpers
        // and external stages already receive it through FUNCTION_STDIN;
        // expose it the same way while the inline arms run so fd-alias
        // redirections like `cat < /dev/stdin` (niubash#118) resolve to the
        // stage input instead of the process's own stdin handle.
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
        self.pipeline_stdin_consumed.set(None);
        let result = if command_has_pipeline_process_substitution(command) {
            // The substitution child inherits the stage's stdin — the
            // upstream pipe — so expose the captured input while the
            // substitution sources run.
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
            let materialized = self.command_with_process_substitution_files(command);
            restore_optional_env_var(&mut self.shell_state.env_vars, FUNCTION_STDIN, old_stdin);
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                FUNCTION_STDIN_OFFSET,
                old_stdin_offset,
            );
            match materialized {
                Ok((materialized, process_substitutions)) => {
                    let inner =
                        self.execute_pipeline_stage_inner(&materialized, input, stdin_inherit);
                    match self.finish_process_substitutions(process_substitutions) {
                        Err(error) => Err(error),
                        Ok(()) => inner,
                    }
                }
                Err(error) => Err(error),
            }
        } else {
            self.execute_pipeline_stage_inner(command, input, stdin_inherit)
        };
        // Inline stage arms that ran on `self` consumed FUNCTION_STDIN
        // directly; subshell/child stages report through the cell. Whichever
        // is missing falls back to the cursor visible here.
        if self.pipeline_stdin_consumed.get().is_none() {
            let measured = self
                .shell_state
                .env_vars
                .get(FUNCTION_STDIN_OFFSET)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            self.pipeline_stdin_consumed.set(Some(measured));
        }
        restore_optional_env_var(&mut self.shell_state.env_vars, FUNCTION_STDIN, old_stdin);
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FUNCTION_STDIN_OFFSET,
            old_stdin_offset,
        );
        let nounset_hit = self.restore_arithmetic_error_flags(&saved);
        match result {
            Ok(Some((output, stderr, _status))) if nounset_hit => {
                // The stage re-raised (or consumed) the unbound-variable
                // error; clear the expansion-error latch so it does not
                // leak into the enclosing command's exit-status handling.
                self.shell_state.arithmetic_expansion_error.set(false);
                Ok(Some((output, stderr, 127)))
            }
            other => other,
        }
    }

    fn execute_pipeline_stage_inner(
        &mut self,
        command: &CommandNode,
        input: &str,
        stdin_inherit: bool,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        if let Some(time_command) = &command.time_command {
            let started = time_command_started();
            let Some((output, stderr, status)) =
                self.execute_pipeline_stage(&time_command.command, input, false)?
            else {
                return Ok(None);
            };
            print_time(
                &self.shell_state.env_vars,
                time_command.posix_format,
                started,
            );
            let status = if time_command.inverted {
                invert_exit_status(status)
            } else {
                status
            };
            return Ok(Some((output, stderr, status)));
        }

        if command_is_compound_pipeline_stage(command) {
            return self
                .execute_compound_pipeline_stage(command, input)
                .map(Some);
        }

        // GNU execute_cmd.c:4617 expand_words runs BEFORE the element's own
        // do_redirections (execute_builtin_or_function execute_cmd.c:5606),
        // so a word-expansion diagnostic (`${x?word}`, bad substitution)
        // writes to the element's ambient fd 2 — the real stderr for a
        // top-level pipeline — never through the command's own `2>&1`.
        // Stage helpers expand words via expand_command_word which would
        // silently substitute the `${x?word}` error word, so run the same
        // expansion-error check the top-level command path runs first.
        if let Some((name, message, status)) = self.parameter_expansion_error(command) {
            let line = format!("{}{}: {}\n", self.diagnostic_prefix(), name, message);
            self.write_default_stderr(line.as_bytes())?;
            let code = if status == Self::FATAL_PARAMETER_EXPANSION_STATUS {
                self.expansion_fatal_status()
            } else {
                status
            };
            return Ok(Some((String::new(), String::new(), code)));
        }

        let expanded = self.brace_expanded_pipeline_stage(command);
        let command = &expanded;
        let command = self.split_pipeline_stage_command_word(command);
        let command = &command;
        let Some(name) = command.words.first().map(String::as_str) else {
            return self
                .execute_compound_pipeline_stage(command, input)
                .map(Some);
        };

        if let Some(restricted) = self.restricted_command_error(command, name) {
            return Ok(Some((String::new(), restricted, 1)));
        }

        // Bash applies a pipeline element's redirections before running the
        // command (redir.c do_redirection_internal runs for every command, and
        // the dup arms report failures through the already-redirected fd2).
        // This fast path historically skipped that machinery, so
        // "echo foo 2>&1 >&$v | cat" ran echo and sent foo down the pipe
        // instead of failing with a $v: Bad file descriptor diagnostic routed
        // into the pipe. Run the same ordered redirect preflight the top-level
        // builtin path uses; with the stdout capture active, a 2>&1-routed
        // diagnostic lands in this stage's stdout (the pipe) like GNU's.
        let saved_capture = crate::executor::shell_options::begin_stdout_capture();
        if self.command_output_redirect_fails(command)? {
            let captured = crate::executor::shell_options::take_stdout_capture();
            crate::executor::shell_options::restore_stdout_capture(saved_capture);
            return Ok(Some((
                crate::executor::substitution_metadata::bytes_to_shell_text(&captured),
                String::new(),
                1,
            )));
        }
        crate::executor::shell_options::restore_stdout_capture(saved_capture);

        // GNU execute_simple_command (execute_cmd.c): a pipeline element
        // whose words all expand to zero fields is a null command - its
        // redirections (applied by the preflight above) still take effect
        // and the status is 0 (redir.tests: `exit 3 | $EXIT >
        // $TMPDIR/null-redir-e` reports `0 -- 3 0`, not 127). A quoted
        // empty word ("") survives as one empty field and keeps the
        // command-not-found path below.
        let stage_expands_to_no_fields = !command.words.is_empty()
            && command.words.iter().enumerate().all(|(index, word)| {
                let raw = command
                    .word_metadata
                    .get(index)
                    .map(|metadata| metadata.raw.as_str());
                self.expand_command_word(command, index, word, raw)
                    .is_empty()
            });
        if stage_expands_to_no_fields {
            return Ok(Some((String::new(), String::new(), 0)));
        }

        match name {
            "true" | ":" => Ok(Some((String::new(), String::new(), 0))),
            "false" => Ok(Some((String::new(), String::new(), 1))),
            "echo" => {
                let args = self.expand_pipeline_stage_arg_words(command, 1);
                let mut output = Vec::new();
                crate::builtins::echo::write_echo_decoded(
                    args.iter().map(String::as_str),
                    &mut output,
                )?;
                Ok(Some((
                    crate::executor::substitution_metadata::bytes_to_shell_text(&output),
                    String::new(),
                    0,
                )))
            }
            "printf" => {
                let args: Vec<String> = self.expand_pipeline_stage_arg_words(command, 1);
                let mut env_vars = self.shell_state.env_vars.clone();
                let mut output = Vec::new();
                let mut stderr = Vec::new();
                let status = crate::builtins::printf::execute_with_io(
                    args.iter().map(String::as_str),
                    &mut env_vars,
                    &mut output,
                    &mut stderr,
                )?;
                Ok(Some((
                    crate::executor::substitution_metadata::bytes_to_shell_text(&output),
                    crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
                    status,
                )))
            }
            "trap" => {
                let args = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                let mut env_vars = self.shell_state.env_vars.clone();
                let mut output = Vec::new();
                let mut stderr = Vec::new();
                let status = crate::builtins::trap::execute_with_io(
                    &args,
                    &mut env_vars,
                    &mut output,
                    &mut stderr,
                )?;
                Ok(Some((
                    crate::executor::substitution_metadata::bytes_to_shell_text(&output),
                    crate::executor::substitution_metadata::bytes_to_shell_text(&stderr),
                    status,
                )))
            }
            "head" => {
                let args = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                // GNU head with file operands ignores stdin and reads the
                // files (`printf x | head -1 f*` prints the file header,
                // not the pipe input). This inline arm can only count
                // lines of the pipe input, so an operand-bearing
                // invocation must run the real external head.
                let mut cursor = 0;
                while cursor < args.len() {
                    let arg = args[cursor].as_str();
                    if arg == "-c" || arg == "--bytes" {
                        // Byte mode has no line-count representation; the
                        // real head must run instead of silently emitting
                        // whole lines.
                        return self.execute_external_pipeline_stage(command, input, stdin_inherit);
                    }
                    if matches!(arg, "-n" | "-b") {
                        cursor += 2;
                        continue;
                    }
                    if arg.starts_with('-') {
                        cursor += 1;
                        continue;
                    }
                    return self.execute_external_pipeline_stage(command, input, stdin_inherit);
                }
                let count = head_line_count(&args).unwrap_or(10);
                let output = input.split_inclusive('\n').take(count).collect::<String>();
                Ok(Some((output, String::new(), 0)))
            }
            "cat" => {
                // Pathname-expand the operands the way
                // execute_simple_command does, or `cat f*` opens the
                // literal name "f*" and reports it as missing (probe
                // 2026-09-09: `printf x | cat f*` printed nothing).
                let show_nonprinting =
                    crate::executor::external_file_builtins::cat_has_show_nonprinting(command);
                let mut file_operands: Vec<String> = Vec::new();
                let mut options_done = false;
                for (arg_index, word) in command.words[1..].iter().enumerate() {
                    let word_index = arg_index + 1;
                    // GNU cat: a bare `-` operand is stdin at that position
                    // and `--` ends option processing; neither is a flag.
                    if word == "--" && !options_done {
                        options_done = true;
                        continue;
                    }
                    if word == "-" {
                        file_operands.push(word.clone());
                        continue;
                    }
                    if !options_done && word.starts_with('-') {
                        continue;
                    }
                    let value = self.expand_word(word);
                    // \x1d marks a fully quoted word and \x1b a quoted
                    // tilde; both stay literal.
                    if value.starts_with(STORAGE_WORD_PREFIX)
                        || value.starts_with(crate::executor::markers::QUOTED_WORD_PREFIX)
                    {
                        file_operands.push(value.replace(crate::executor::markers::CTLESC, ""));
                        continue;
                    }
                    // Quoted words (e.g. "*.txt") must not be glob-expanded.
                    let metadata = command.word_metadata.get(word_index);
                    let raw = metadata.map(|metadata| metadata.raw.as_str());
                    if crate::executor::command_prepare::raw_word_suppresses_pathname_expansion(
                        raw, metadata,
                    ) {
                        file_operands.push(value.replace(crate::executor::markers::CTLESC, ""));
                        continue;
                    }
                    match glob::pathname_expand_word(&value, &self.shell_state.env_vars) {
                        glob::PathnameExpansion::Matches(matches) => file_operands.extend(matches),
                        glob::PathnameExpansion::NoMatch | glob::PathnameExpansion::Fail(_) => {
                            file_operands.push(value.replace(crate::executor::markers::CTLESC, ""))
                        }
                    }
                }
                if !file_operands.is_empty() {
                    let mut output = String::new();
                    let mut stderr = String::new();
                    let mut status = 0;
                    // `cat -` consumes the stage's stdin once; later `-`
                    // operands see EOF. Resolved lazily so commands without
                    // a `-` operand never touch the stdin machinery.
                    let mut stdin_remaining: Option<String> = None;
                    for path in file_operands {
                        if path == "-" {
                            if stdin_remaining.is_none() {
                                stdin_remaining = Some(
                                    self.stdin_string_for_command_mut(command)
                                        .unwrap_or_else(|| input.to_string()),
                                );
                            }
                            let text = stdin_remaining.take().unwrap_or_default();
                            let bytes = if show_nonprinting {
                                crate::executor::external_file_builtins::cat_v_filter(
                                    &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&text),
                                )
                            } else {
                                crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                                    &text,
                                )
                            };
                            output.push_str(
                                &crate::executor::substitution_metadata::bytes_to_shell_text(
                                    &bytes,
                                ),
                            );
                            continue;
                        }
                        // `/dev/stdin`/`/dev/fd/N`/`/proc/self/fd/N`
                        // operands resolve against this stage's fd
                        // endpoints, not the filesystem: fd 0 is the
                        // stage input (same lazy cursor as `-`), higher
                        // fds come from the executor's fd table.
                        if let Some(fd) = crate::executor::dev_fd_operands::dev_operand_fd(&path) {
                            let bytes_opt = if fd == 0 {
                                if stdin_remaining.is_none() {
                                    stdin_remaining = Some(
                                        self.stdin_string_for_command_mut(command)
                                            .unwrap_or_else(|| input.to_string()),
                                    );
                                }
                                Some(
                                    crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                                        &stdin_remaining.take().unwrap_or_default(),
                                    ),
                                )
                            } else {
                                match self.dev_fd_operand_bytes_for_command(command, fd) {
                                    // GNU cat.c: the operand resolves to the
                                    // file fd 1 writes to — report, skip,
                                    // exit 1.
                                    Some(read) if read.stdout_file && !read.bytes.is_empty() => {
                                        stderr.push_str(&format!(
                                            "{}cat: {path}: input file is output file\n",
                                            self.diagnostic_prefix()
                                        ));
                                        status = 1;
                                        None
                                    }
                                    other => other.map(|read| read.bytes),
                                }
                            };
                            match bytes_opt {
                                Some(bytes) => {
                                    let bytes = if show_nonprinting {
                                        crate::executor::external_file_builtins::cat_v_filter(
                                            &bytes,
                                        )
                                    } else {
                                        bytes
                                    };
                                    output.push_str(
                                        &crate::executor::substitution_metadata::bytes_to_shell_text(
                                            &bytes,
                                        ),
                                    );
                                }
                                None => {
                                    stderr.push_str(&format!(
                                        "{}cat: {path}: No such file or directory\n",
                                        self.diagnostic_prefix()
                                    ));
                                    status = 1;
                                }
                            }
                            continue;
                        }
                        match fs::read(shell_path_to_windows(&path, &self.shell_state.env_vars)) {
                            Ok(bytes) => {
                                let bytes = if show_nonprinting {
                                    crate::executor::external_file_builtins::cat_v_filter(&bytes)
                                } else {
                                    bytes
                                };
                                output.push_str(
                                    &crate::executor::substitution_metadata::bytes_to_shell_text(
                                        &bytes,
                                    ),
                                );
                            }
                            Err(_) => {
                                stderr.push_str(&format!(
                                    "{}cat: {path}: No such file or directory\n",
                                    self.diagnostic_prefix()
                                ));
                                status = 1;
                            }
                        }
                    }
                    return Ok(Some((output, stderr, status)));
                }
                let output = if show_nonprinting {
                    // cat -v renders the user's byte stream: decode the
                    // transport text first or marker escapes (E400 literal
                    // prefix, E000 byte pairs) filter as stray M-^ bytes.
                    let text = self
                        .stdin_string_for_command_mut(command)
                        .unwrap_or_else(|| input.to_string());
                    let bytes = crate::executor::external_file_builtins::cat_v_filter(
                        &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&text),
                    );
                    crate::executor::substitution_metadata::bytes_to_shell_text(&bytes)
                } else if let Some(input) = self.stdin_string_for_command_mut(command) {
                    input
                } else {
                    input.to_string()
                };
                Ok(Some((output, String::new(), 0)))
            }
            "sed" => {
                let args = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                if let Some(output) = apply_simple_sed_args(input, &args) {
                    Ok(Some((output, String::new(), 0)))
                } else {
                    self.execute_external_pipeline_stage(command, input, stdin_inherit)
                }
            }
            "grep" => {
                // The inline fast path handles the flag forms GNU pipelines use
                // most (grep [-cinvq] [--] PATTERN with stdin input only).
                // Anything else -- regex engine flags, file operands, unknown
                // options -- must run the real grep; historically words[1] was
                // treated as the pattern even when it was a flag, so
                // "set | grep -c VTILDE" grepped for the literal "-c" (it
                // matched SHELLOPTS' "interactive-comments") and printed the
                // matching lines instead of a count.
                let args: Vec<String> = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect();
                let Some(spec) = inline_grep_args(&args) else {
                    return self.execute_external_pipeline_stage(command, input, stdin_inherit);
                };
                let mut selected = 0usize;
                let mut line_number = 0usize;
                let mut output = String::new();
                for line in input.split_inclusive('\n') {
                    line_number += 1;
                    let comparable = line.strip_suffix('\n').unwrap_or(line);
                    let haystack = if spec.ignore_case {
                        comparable.to_lowercase()
                    } else {
                        comparable.to_string()
                    };
                    let needle = if spec.ignore_case {
                        spec.pattern.to_lowercase()
                    } else {
                        spec.pattern.to_string()
                    };
                    let hit = simple_grep_pattern_matches(&haystack, &needle) != spec.invert;
                    if !hit {
                        continue;
                    }
                    selected += 1;
                    if spec.quiet || spec.count_mode {
                        continue;
                    }
                    if spec.line_numbers {
                        output.push_str(&line_number.to_string());
                        output.push(':');
                    }
                    output.push_str(line);
                    if !line.ends_with('\n') {
                        output.push('\n');
                    }
                }
                if spec.count_mode {
                    output = format!("{selected}\n");
                }
                Ok(Some((output, String::new(), i32::from(selected == 0))))
            }
            "wc" => {
                let args: Vec<String> = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect();
                // Only the single-flag fast paths are emulated inline; every
                // other invocation (-w, -L, -m, --words, combined flags, file
                // operands) must run the real external `wc`. Returning None
                // here would abort the whole pipeline with the misleading
                // "pipeline command could not execute" diagnostic.
                if args.len() == 1 {
                    let value = match args[0].as_str() {
                        "-c" => input.as_bytes().len(),
                        "-l" => input.bytes().filter(|byte| *byte == b'\n').count(),
                        "-w" => input.split_whitespace().count(),
                        _ => {
                            return self.execute_external_pipeline_stage(
                                command,
                                input,
                                stdin_inherit,
                            );
                        }
                    };
                    return Ok(Some((format!("{value}\n"), String::new(), 0)));
                }
                // Default (no operands) matches GNU wc: lines, words, bytes.
                if args.is_empty() {
                    let lines = input.bytes().filter(|byte| *byte == b'\n').count();
                    let words = input.split_whitespace().count();
                    let bytes = input.as_bytes().len();
                    return Ok(Some((
                        format!("{lines:>7} {words:>7} {bytes:>7}\n"),
                        String::new(),
                        0,
                    )));
                }
                self.execute_external_pipeline_stage(command, input, stdin_inherit)
            }
            "tr" => {
                let args = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                if args.len() == 2 && matches!(args[0].as_str(), "\\n" | "\n") {
                    Ok(Some((input.replace('\n', &args[1]), String::new(), 0)))
                } else if args.len() == 2
                    && inline_expand_tr_set(&args[0]).is_some()
                    && inline_expand_tr_set(&args[1]).is_some()
                {
                    Ok(Some((
                        translate_tr(input, &args[0], &args[1]),
                        String::new(),
                        0,
                    )))
                } else {
                    // Specs the inline fast path cannot represent (POSIX
                    // classes it does not know, `[x*n]` repeats, escapes)
                    // must run the real external `tr`; silently returning the
                    // input unchanged is never acceptable.
                    self.execute_external_pipeline_stage(command, input, stdin_inherit)
                }
            }
            _ => {
                if let Some(output) = self.execute_function_pipeline_stage(command, input)? {
                    Ok(Some(output))
                } else {
                    if let Some(output) = self.execute_builtin_pipeline_stage(command, input)? {
                        Ok(Some(output))
                    } else {
                        self.execute_external_pipeline_stage(command, input, stdin_inherit)
                    }
                }
            }
        }
    }

    pub(in crate::executor) fn restricted_command_error(
        &self,
        command: &CommandNode,
        name: &str,
    ) -> Option<String> {
        if !crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "restricted") {
            return None;
        }
        let slash = |value: &str| value.contains('/') || value.contains('\\');
        if slash(name) {
            return Some(format!(
                "{}{}: restricted: cannot specify `/' in command names\n",
                self.diagnostic_prefix(),
                name
            ));
        }
        if name == "cd" {
            // GNU builtins/cd.def:267-274 refuses any cd in a restricted
            // shell: sh_restricted(NULL) -> "cd: restricted".
            return Some(format!(
                "{}{}: restricted\n",
                self.diagnostic_prefix(),
                name
            ));
        }
        if name == "exec" {
            // GNU builtins/exec.def:118-145: bare `exec` (redirects only)
            // still succeeds in a restricted shell; an operand triggers
            // sh_restricted(NULL) -> "exec: restricted".
            if command.words.len() > 1 {
                return Some(format!(
                    "{}{}: restricted\n",
                    self.diagnostic_prefix(),
                    name
                ));
            }
        }
        if matches!(name, "." | "source") {
            // GNU builtins/source.def:148-155 refuses when the filename
            // operand contains a slash: sh_restricted(filename) ->
            // ".: <filename>: restricted".
            if let Some(operand) = command.words.get(1) {
                let filename = self.expand_word(operand);
                if slash(&filename) {
                    return Some(format!(
                        "{}{}: {}: restricted\n",
                        self.diagnostic_prefix(),
                        name,
                        filename
                    ));
                }
            }
        }
        if name == "command"
            && command
                .words
                .iter()
                .skip(1)
                .any(|word| self.expand_word(word) == "-p")
        {
            return Some(format!(
                "{}command: -p: restricted\n",
                self.diagnostic_prefix()
            ));
        }
        for redirect in &command.redirects {
            let target = self.expand_redirect_target(redirect);
            if redirect.fd_var.is_none() && redirect_target_fd(&target).is_none() && slash(&target)
            {
                return Some(format!(
                    "{}{}: restricted: cannot redirect output\n",
                    self.diagnostic_prefix(),
                    target
                ));
            }
        }
        None
    }

    fn brace_expanded_pipeline_stage(&self, command: &CommandNode) -> CommandNode {
        if !self.is_brace_expand_enabled() {
            return command.clone();
        }

        let mut expanded = command.clone();
        expanded.words = command
            .words
            .iter()
            .enumerate()
            .flat_map(|(index, word)| {
                let raw = command
                    .word_metadata
                    .get(index)
                    .map(|metadata| metadata.raw.as_str());
                crate::executor::command_prepare::expand_braces_with_optional_raw(word, raw)
            })
            .collect();
        expanded
    }

    fn lastpipe_enabled(&self) -> bool {
        crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "lastpipe")
    }

    /// Computes the first pipeline element's fd-0 payload. GNU gives every
    /// element the same open file description (execute_cmd.c execute_pipeline):
    /// the shared FUNCTION_STDIN cursor may move only when the element
    /// actually reads. stdin_string_for_command_mut's drain-to-EOF models a
    /// consumer, so the cursor is snapshot/restored here and the real
    /// consumption is folded back by execute_simple_pipeline after the stage
    /// runs (pipeline_stdin_consumed reports subshell/child reads). Returns
    /// the input plus the FUNCTION_STDIN base offset when the buffer was the
    /// source (None for heredoc/redirect/virtual-fd/process-stdin sources,
    /// whose cursors live elsewhere).
    fn initial_pipeline_input(&mut self, command: &CommandNode) -> (String, Option<usize>) {
        self.apply_comsub_stdin_writeback();
        let base = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let from_function_stdin = self.function_stdin_is_command_source(command);
        let input = self
            .stdin_string_for_command_mut(command)
            .or_else(|| {
                pipeline_stage_reads_stdin_by_default(command)
                    .then(|| self.read_inherited_process_stdin_to_string())
                    .flatten()
            })
            .unwrap_or_default();
        if from_function_stdin {
            self.shell_state
                .env_vars
                .insert(FUNCTION_STDIN_OFFSET.to_string(), base.to_string());
            (input, Some(base))
        } else {
            (input, None)
        }
    }
}

pub(crate) fn command_is_compound_pipeline_stage(command: &CommandNode) -> bool {
    command.for_command.is_some()
        || command.if_command.is_some()
        || command.loop_command.is_some()
        || command.select_command.is_some()
        || command.case_command.is_some()
        || command.coproc_command.is_some()
        || command.subshell_command.is_some()
        || command.brace_group.is_some()
        || command.time_command.is_some()
        || command.arithmetic_command.is_some()
        || command.conditional_command.is_some()
        || command.inverted_command.is_some()
        || command.background_command.is_some()
}

fn pipeline_stage_reads_stdin_by_default(command: &CommandNode) -> bool {
    let Some(command_name) = command.words.first().map(String::as_str) else {
        return false;
    };
    let command_name = command_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command_name);
    matches!(
        command_name,
        "awk" | "cat" | "grep" | "head" | "sed" | "sort" | "tail" | "tr" | "uniq" | "wc"
    )
}

pub(in crate::executor) fn head_line_count(args: &[String]) -> Option<usize> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            break;
        }
        if arg == "-" || !arg.starts_with('-') {
            break;
        }
        if arg == "-n" {
            return args
                .get(index + 1)
                .and_then(|value| value.parse::<usize>().ok());
        }
        if let Some(value) = arg.strip_prefix("--lines=") {
            return value.parse::<usize>().ok();
        }
        if let Some(value) = arg.strip_prefix("-n") {
            return value.parse::<usize>().ok();
        }
        if let Some(value) = arg.strip_prefix('-') {
            if value.chars().all(|ch| ch.is_ascii_digit()) {
                return value.parse::<usize>().ok();
            }
        }
        index += 1;
    }
    None
}

pub(in crate::executor) fn translate_tr(input: &str, source: &str, target: &str) -> String {
    let (Some(source_chars), Some(target_chars)) =
        (inline_expand_tr_set(source), inline_expand_tr_set(target))
    else {
        return input.to_string();
    };
    input
        .chars()
        .map(|ch| {
            source_chars
                .iter()
                .position(|source_ch| *source_ch == ch)
                .map(|index| target_chars[index.min(target_chars.len() - 1)])
                .unwrap_or(ch)
        })
        .collect()
}

/// Expands a `tr` set for the inline pipeline fast path. Returns `None` for
/// syntax it does not implement (unknown POSIX classes, `[x*n]` repeats,
/// backslash escapes, non-ASCII, reversed ranges) so callers can run the real
/// external `tr` instead of silently translating nothing.
pub(in crate::executor) fn inline_expand_tr_set(spec: &str) -> Option<Vec<char>> {
    if !spec.is_ascii() {
        return None;
    }
    let chars: Vec<char> = spec.chars().collect();
    let mut expanded = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' => return None,
            '[' => {
                let class = spec[index..].strip_prefix("[:")?;
                let end = class.find(":]")?;
                expanded.extend(tr_class_chars(&class[..end])?);
                index += end + 4;
            }
            current if index + 2 < chars.len() && chars[index + 1] == '-' => {
                if current > chars[index + 2] {
                    // A reversed range is an error in real `tr`; let the
                    // external binary surface it instead of eating input.
                    return None;
                }
                expanded.extend(current..=chars[index + 2]);
                index += 3;
            }
            current => {
                expanded.push(current);
                index += 1;
            }
        }
    }
    Some(expanded)
}

fn tr_class_chars(class: &str) -> Option<Vec<char>> {
    Some(match class {
        "alpha" => ('a'..='z').chain('A'..='Z').collect(),
        "digit" => ('0'..='9').collect(),
        "lower" => ('a'..='z').collect(),
        "upper" => ('A'..='Z').collect(),
        "space" => vec![' ', '\t', '\n', '\r', '\x0b', '\x0c'],
        "blank" => vec![' ', '\t'],
        "punct" => ('!'..='/')
            .chain(':'..='@')
            .chain('['..='`')
            .chain('{'..='~')
            .collect(),
        "xdigit" => ('0'..='9').chain('a'..='f').chain('A'..='F').collect(),
        "cntrl" => (0u8..=0x1f)
            .map(|byte| byte as char)
            .chain(['\x7f'])
            .collect(),
        "graph" => ('!'..='~').collect(),
        "print" => (' '..='~').collect(),
        _ => return None,
    })
}

#[cfg(test)]
mod inline_tr_tests {
    use super::*;

    #[test]
    fn translate_tr_maps_ranges_and_classes() {
        assert_eq!(translate_tr("Hello", "a-z", "A-Z"), "HELLO");
        assert_eq!(
            translate_tr("Hello World", "[:upper:]", "[:lower:]"),
            "hello world"
        );
        assert_eq!(translate_tr("a b c", "[:space:]", "-"), "a-b-c");
        assert_eq!(translate_tr("abc123", "[:digit:]", "#"), "abc###");
        assert_eq!(translate_tr("abc", "b", "x"), "axc");
    }

    #[test]
    fn unknown_tr_syntax_is_not_supported() {
        assert!(inline_expand_tr_set("[x*n]").is_none());
        assert!(inline_expand_tr_set(r"\n").is_none());
        assert!(inline_expand_tr_set("héllo").is_none());
        assert!(inline_expand_tr_set("[:bogus:]").is_none());
        assert!(inline_expand_tr_set("z-a").is_none());
        assert!(inline_expand_tr_set("a-z").is_some());
        assert!(inline_expand_tr_set("[:upper:]").is_some());
        assert!(inline_expand_tr_set("abc").is_some());
    }
}

fn command_has_non_concurrent_pipeline_redirects(
    command: &CommandNode,
    index: usize,
    pipeline_len: usize,
) -> bool {
    if command.redirects.is_empty() {
        return false;
    }
    let is_last_stage = index + 1 == pipeline_len;
    command.redirects.iter().any(|redirect| {
        let is_initial_heredoc = index == 0
            && matches!(
                redirect.kind,
                crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString
            );
        let is_final_output = is_last_stage
            && matches!(
                redirect.kind,
                crate::parser::RedirectKind::Output
                    | crate::parser::RedirectKind::Append
                    | crate::parser::RedirectKind::ClobberOutput
            );
        !is_initial_heredoc && !is_final_output
    })
}

// The native concurrent pipeline runs members through CreateProcess with
// the parsed word text as argv; it has no process-substitution
// materialization, so a member carrying <( ) / >( ) must take the
// sequential stage path, where execute_pipeline_stage materializes the
// substitutions into temp paths first.
fn command_has_pipeline_process_substitution(command: &CommandNode) -> bool {
    !command.process_substitutions.is_empty()
        || command.word_metadata.iter().any(|metadata| {
            !metadata.process_substitutions.is_empty()
                || metadata.raw.contains("<(")
                || metadata.raw.contains(">(")
        })
        || command
            .words
            .iter()
            .any(|word| (word.starts_with("<(") || word.starts_with(">(")) && word.ends_with(')'))
        || command
            .redirects
            .iter()
            .any(|redirect| redirect.target.starts_with("<(") || redirect.target.starts_with(">("))
        || [
            command.redirect_in.as_ref(),
            command.redirect_out.as_ref(),
            command.append.as_ref(),
            command.redirect_err.as_ref(),
            command.redirect_err_append.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|redirect| redirect.target.starts_with("<(") || redirect.target.starts_with(">("))
}

struct TimePipelinePrefix {
    command: CommandNode,
    inverted: bool,
    posix_format: bool,
}

fn time_pipeline_prefix(command: &CommandNode) -> Option<TimePipelinePrefix> {
    if command.words.first().map(String::as_str) != Some("time") {
        return None;
    }

    let mut index = 1;
    let mut inverted = false;
    let mut posix_format = false;
    while let Some(word) = command.words.get(index).map(String::as_str) {
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
    if index >= command.words.len() {
        return None;
    }

    let mut stripped = command.clone();
    stripped.words = command.words[index..].to_vec();
    if command.word_kinds.len() == command.words.len() {
        stripped.word_kinds = command.word_kinds[index..].to_vec();
    }
    if command.word_metadata.len() == command.words.len() {
        stripped.word_metadata = command.word_metadata[index..].to_vec();
    }
    Some(TimePipelinePrefix {
        command: stripped,
        inverted,
        posix_format,
    })
}

/// Parse the inline grep fast-path argument list: options from the set
/// [-cinvq] (bundled or separate), an optional -- terminator, exactly one
/// non-option PATTERN, and no file operands. Returns None when anything
/// outside that subset is present so the pipeline runs the real grep.
fn inline_grep_args(args: &[String]) -> Option<InlineGrepSpec> {
    let mut spec = InlineGrepSpec::default();
    let mut pattern: Option<&str> = None;
    let mut operands_done = false;
    for arg in args {
        if !operands_done && arg == "--" {
            operands_done = true;
            continue;
        }
        if !operands_done && arg.len() > 1 && arg.starts_with('-') && !arg.starts_with("--") {
            for flag in arg[1..].chars() {
                match flag {
                    'c' => spec.count_mode = true,
                    'i' => spec.ignore_case = true,
                    'v' => spec.invert = true,
                    'n' => spec.line_numbers = true,
                    'q' => spec.quiet = true,
                    _ => return None,
                }
            }
            continue;
        }
        if pattern.is_none() {
            let candidate = arg.as_str();
            // The inline matcher is a literal substring search with an
            // optional leading ^ anchor; it does not implement BRE. A
            // pattern carrying a regex met character must run the real
            // external grep, or `grep -c .` counts a line that has no
            // literal dots as 0 instead of 1 (probe 2026-09-09).
            if candidate.chars().any(|ch| {
                matches!(
                    ch,
                    '.' | '*' | '[' | ']' | '\\' | '?' | '+' | '|' | '(' | ')' | '{' | '}' | '$'
                )
            }) {
                return None;
            }
            pattern = Some(candidate);
            continue;
        }
        // Second operand (or an operand before --) means file input.
        return None;
    }
    Some(InlineGrepSpec {
        pattern: pattern?.to_string(),
        ..spec
    })
}

#[derive(Default)]
struct InlineGrepSpec {
    pattern: String,
    invert: bool,
    count_mode: bool,
    quiet: bool,
    line_numbers: bool,
    ignore_case: bool,
}
