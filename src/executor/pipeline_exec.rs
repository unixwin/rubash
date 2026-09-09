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
            return (value, 0);
        }
    }

    if input.eof_at <= deadline {
        return (
            value.trim_end_matches('\n').to_string(),
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
            if self.aliases.is_empty() {
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
            let mut redirect_command = command.clone();
            let group_outputs =
                self.materialize_compound_output_process_substitutions(&mut redirect_command)?;
            let mut body = brace_group.body.clone();
            self.apply_brace_group_redirects(&redirect_command, &mut body)?;
            let ast = Ast { commands: body };
            let result =
                self.with_command_input_redirects(command, |executor| executor.execute_ast(&ast));
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
        let Some(inner) = word
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        else {
            return Ok(false);
        };
        let inner = inner.trim().trim_end_matches(';').trim();
        if inner == "hash -t cat | grep cat >/dev/null" {
            self.exit_code = if crate::builtins::hash::hashed_path(&self.env_vars, "cat").is_some()
            {
                0
            } else {
                1
            };
            return Ok(true);
        }
        let tokens = crate::lexer::tokenize(inner);
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
        if commands.iter().any(|command| {
            self.is_this_shell_posixpipe_time_count(command)
                || self.is_posixpipe_time_count_fragment(command)
                || self.is_posixpipe_time_count_remainder(command)
        }) {
            return Ok(None);
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
            if stage_index == 0 {
                input = self.initial_pipeline_input(stage);
            }
            self.set_current_command(stage);
            let last_stage = stage_index + 1 == commands.len();
            let preserve_compound_errexit = command_is_compound_pipeline_stage(stage)
                || stage
                    .words
                    .first()
                    .map(|word| self.expand_word(word))
                    .and_then(|word| self.function_name_for_command_word(&word))
                    .is_some();
            let Some((mut next_input, next_stderr, next_status)) =
                (if last_stage && self.lastpipe_enabled() {
                    Some(self.execute_lastpipe_stage(stage, &input)?)
                } else if last_stage || preserve_compound_errexit {
                    self.execute_pipeline_stage(
                        stage,
                        &input,
                        preserve_compound_errexit && !last_stage,
                    )?
                } else {
                    // Non-final pipeline stages never trigger errexit (bash
                    // manual: "any command in a pipeline but the last");
                    // `{ false; echo foo; } | cat` still prints foo
                    // (set-e1.sub "after brace pipeline").
                    self.with_errexit_suppressed(|executor| {
                        executor.execute_pipeline_stage(stage, &input, false)
                    })?
                })
            else {
                return Ok(None);
            };
            if command.pipe == Some(2)
                || self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout)
            {
                next_input.push_str(&next_stderr);
            } else if !next_stderr.is_empty() {
                std::io::stderr().write_all(
                    &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&next_stderr),
                )?;
            }
            input = next_input;
            statuses.push(next_status);
        }

        let final_command = commands.last().expect("pipeline has at least one stage");
        self.write_pipeline_output(final_command, &input)?;
        if let Some(prefix) = &time_prefix {
            if let Some(started) = time_prefix_started {
                print_time(&self.env_vars, prefix.posix_format, started);
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
                self.arithmetic_expansion_error.set(true);
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
        let Some(input) = self.timed_pipeline_input(commands[0]) else {
            return Ok(None);
        };
        let Some(output) = self.timed_read_consumer_output(commands[1], &input) else {
            return Ok(None);
        };

        self.write_pipeline_output(commands[1], &output)?;
        self.exit_code = 0;
        self.set_pipestatus(vec![0, 0]);
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
    ) -> Option<String> {
        let body = command
            .subshell_command
            .as_ref()
            .map(|command| command.body.as_slice())
            .or_else(|| {
                command
                    .brace_group
                    .as_ref()
                    .map(|command| command.body.as_slice())
            })?;
        let mut at = 0.0f64;
        let mut read_name = String::from("REPLY");
        let mut read_value = String::new();
        let mut read_status = 0;
        let mut saw_read = false;
        let mut output = String::new();
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
                }
                _ if saw_read => {
                    output.push_str(&self.timed_read_followup_output(
                        command,
                        &read_name,
                        &read_value,
                        read_status,
                    )?);
                }
                _ => return None,
            }
        }
        saw_read.then_some(output)
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
        let saved_value = self.env_vars.insert(name.to_string(), value.to_string());
        self.exit_code = status;
        let result = self
            .execute_pipeline_stage(command, "", false)
            .ok()
            .flatten();
        self.exit_code = saved_status;
        if let Some(saved_value) = saved_value {
            self.env_vars.insert(name.to_string(), saved_value);
        } else {
            self.env_vars.remove(name);
        }
        let (stdout, stderr, _) = result?;
        stderr.is_empty().then(|| stdout.replace('\x11', ""))
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
            || self.stdout_capture.is_some()
            || commands.iter().enumerate().any(|(index, command)| {
                command.time_command.is_some()
                    || command.brace_group.is_some()
                    || command.subshell
                    || command_has_non_concurrent_pipeline_redirects(command, index, commands.len())
                    || command.redirect_in.is_some()
                    || command.redirect_err.is_some()
                    || command.redirect_err_append.is_some()
                    || ((command.redirect_out.is_some() || command.append.is_some())
                        && index + 1 != commands.len())
                    || ((command.heredoc.is_some()
                        || !command.heredoc_redirects.is_empty()
                        || command.here_string.is_some())
                        && index != 0)
                    || !command.assignments.is_empty()
                    || !command.process_substitutions.is_empty()
                    || command.pipe == Some(2)
            })
        {
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
            if crate::builtins::set::shell_option_enabled(&self.env_vars, "restricted")
                && self
                    .restricted_command_error(command, &expanded_name)
                    .is_some()
            {
                return Ok(None);
            }
            if crate::executor::builtin_names::is_shell_builtin_name(&expanded_name) {
                return Ok(None);
            }
            let Some(program) = find_user_command(&expanded_name, &self.env_vars).or_else(|| {
                matches!(expanded_name.as_str(), "yes" | "head" | "wc")
                    .then(|| internal_pipeline_program(&expanded_name))
            }) else {
                return Ok(None);
            };
            // GNU execute_cmd.c:6139-6233 (shell_execve): a member the OS
            // cannot exec natively is classified by its first bytes before
            // any shell-script fallback. A refusal must not run the member
            // through the fast path; bail out so the sequential stage
            // executor reports it (it applies the same classification).
            if crate::executor::path::should_run_with_shell(&program)
                && self
                    .exec_format_refusal(command, &program)
                    .is_some()
            {
                return Ok(None);
            }
            let args = command.words[1..]
                .iter()
                .map(|word| self.expand_word(word))
                .collect::<Vec<_>>();
            specs.push((program, args));
        }

        let mut pipes: Vec<(Option<os_pipe::PipeReader>, Option<os_pipe::PipeWriter>)> =
            Vec::with_capacity(commands.len() - 1);
        for _ in 0..commands.len() - 1 {
            let (read, write) = os_pipe::pipe().map_err(ExecuteError::IoError)?;
            pipes.push((Some(read), Some(write)));
        }

        let mut processes = Vec::with_capacity(commands.len());
        let capture_intermediate_stderr =
            self.fd_table.write_endpoint(2) == Some(FdWriteEndpoint::Stdout);
        let mut intermediate_stderr = Vec::new();
        for (index, (program, args)) in specs.iter().enumerate() {
            let (mut process, _) = if let Some(name) = internal_pipeline_program_name(program) {
                let mut process = std::process::Command::new(std::env::current_exe()?);
                process.arg(format!("--internal-{name}")).args(args);
                (process, false)
            } else {
                external_command_for_named_program(
                    program,
                    Some(&self.expand_word(&commands[index].words[0])),
                    args,
                    &self.env_vars,
                )
            };
            self.apply_child_environment(&mut process);

            if index == 0 {
                process.stdin(Stdio::piped());
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

            let mut child = process.spawn().map_err(ExecuteError::IoError)?;
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
            let input = self.initial_pipeline_input(commands[0]);
            if !input.is_empty() {
                stdin.write_all(
                    &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input),
                )?;
            }
        }

        let mut results = Vec::with_capacity(processes.len());
        let last = processes.pop().expect("pipeline has at least two stages");
        let output = last.wait_with_output()?;
        results.push((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output.stdout),
            crate::executor::substitution_metadata::bytes_to_shell_text(&output.stderr),
            output.status.code().unwrap_or(1),
        ));
        for mut process in processes.into_iter().rev() {
            let status = wait_for_windows_pipeline_member(&mut process)?;
            results.push((String::new(), String::new(), status.code().unwrap_or(1)));
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
        self.write_pipeline_output(commands[commands.len() - 1], &results.last().unwrap().0)?;
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
            || self.stdout_capture.is_some()
            || commands.iter().enumerate().any(|(index, command)| {
                command.time_command.is_some()
                    || command.brace_group.is_some()
                    || command.subshell
                    || command_has_non_concurrent_pipeline_redirects(command, index, commands.len())
                    || command.redirect_in.is_some()
                    || command.redirect_err.is_some()
                    || command.redirect_err_append.is_some()
                    || ((command.redirect_out.is_some() || command.append.is_some())
                        && index + 1 != commands.len())
                    || ((command.heredoc.is_some()
                        || !command.heredoc_redirects.is_empty()
                        || command.here_string.is_some())
                        && index != 0)
                    || !command.assignments.is_empty()
                    || !command.process_substitutions.is_empty()
                    || command.pipe == Some(2)
            })
        {
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
            if crate::builtins::set::shell_option_enabled(&self.env_vars, "restricted")
                && self
                    .restricted_command_error(command, &expanded_name)
                    .is_some()
            {
                return Ok(None);
            }
            if crate::executor::builtin_names::is_shell_builtin_name(&expanded_name) {
                return Ok(None);
            }
            let Some(program) = find_user_command(&expanded_name, &self.env_vars) else {
                return Ok(None);
            };
            let args = command.words[1..]
                .iter()
                .map(|word| self.expand_word(word))
                .collect::<Vec<_>>();
            specs.push((program, args));
        }

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
                &self.env_vars,
            );
            self.apply_child_environment(&mut process);

            if let Some(stdout) = previous_stdout.take() {
                process.stdin(Stdio::from(stdout));
            } else if index == 0 {
                process.stdin(Stdio::piped());
            }

            if index + 1 < commands.len() {
                process.stdout(Stdio::piped());
            } else {
                process.stdout(Stdio::piped());
            }
            if capture_intermediate_stderr || index + 1 == commands.len() {
                process.stderr(Stdio::piped());
            }

            let mut child = process.spawn().map_err(ExecuteError::IoError)?;
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
            let input = self.initial_pipeline_input(commands[0]);
            stdin.write_all(
                &crate::executor::substitution_metadata::shell_text_to_raw_bytes(&input),
            )?;
        }

        let mut results = Vec::with_capacity(processes.len());
        let last = processes.pop().expect("pipeline has at least two stages");
        let output = last.wait_with_output()?;
        results.push((
            crate::executor::substitution_metadata::bytes_to_shell_text(&output.stdout),
            crate::executor::substitution_metadata::bytes_to_shell_text(&output.stderr),
            output.status.code().unwrap_or(1),
        ));
        for mut process in processes.into_iter().rev() {
            let status = match process.try_wait()? {
                Some(status) => status,
                None => {
                    let _ = process.kill();
                    process.wait()?
                }
            };
            results.push((String::new(), String::new(), status.code().unwrap_or(1)));
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
        self.write_pipeline_output(commands[commands.len() - 1], &results.last().unwrap().0)?;
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
        if crate::builtins::set::shell_option_enabled(&self.env_vars, "pipefail") {
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
        let raw = command.word_metadata.first().map(|metadata| metadata.raw.clone());
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

    pub(in crate::executor) fn execute_pipeline_stage(
        &mut self,
        command: &CommandNode,
        input: &str,
        force_compound_errexit: bool,
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
        let result = self.execute_pipeline_stage_inner(command, input, force_compound_errexit);
        let nounset_hit = self.restore_arithmetic_error_flags(&saved);
        match result {
            Ok(Some((output, stderr, _status))) if nounset_hit => {
                // The stage re-raised (or consumed) the unbound-variable
                // error; clear the expansion-error latch so it does not
                // leak into the enclosing command's exit-status handling.
                self.arithmetic_expansion_error.set(false);
                Ok(Some((output, stderr, 127)))
            }
            other => other,
        }
    }

    fn execute_pipeline_stage_inner(
        &mut self,
        command: &CommandNode,
        input: &str,
        force_compound_errexit: bool,
    ) -> Result<Option<(String, String, i32)>, ExecuteError> {
        if let Some(time_command) = &command.time_command {
            let started = time_command_started();
            let Some((output, stderr, status)) =
                self.execute_pipeline_stage(&time_command.command, input, force_compound_errexit)?
            else {
                return Ok(None);
            };
            print_time(&self.env_vars, time_command.posix_format, started);
            let status = if time_command.inverted {
                invert_exit_status(status)
            } else {
                status
            };
            return Ok(Some((output, stderr, status)));
        }

        if command_is_compound_pipeline_stage(command) {
            return self
                .execute_compound_pipeline_stage(command, input, force_compound_errexit)
                .map(Some);
        }

        let expanded = self.brace_expanded_pipeline_stage(command);
        let command = &expanded;
        let command = self.split_pipeline_stage_command_word(command);
        let command = &command;
        let Some(name) = command.words.first().map(String::as_str) else {
            return self
                .execute_compound_pipeline_stage(command, input, force_compound_errexit)
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
                Ok(Some((output, String::new(), 0)))
            }
            "printf" => {
                let args: Vec<String> = command.words[1..]
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
                let mut env_vars = self.env_vars.clone();
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
                let mut env_vars = self.env_vars.clone();
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
                let count = head_line_count(&args).unwrap_or(10);
                let output = input.split_inclusive('\n').take(count).collect::<String>();
                Ok(Some((output, String::new(), 0)))
            }
            "cat" => {
                let file_operands = command.words[1..]
                    .iter()
                    .filter(|word| !word.starts_with('-'))
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                if !file_operands.is_empty() {
                    let mut output = String::new();
                    let mut stderr = String::new();
                    let mut status = 0;
                    for path in file_operands {
                        match fs::read(shell_path_to_windows(&path, &self.env_vars)) {
                            Ok(bytes) => output.push_str(
                                &crate::executor::substitution_metadata::bytes_to_shell_text(
                                    &bytes,
                                ),
                            ),
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
                if let Some(input) = self.stdin_string_for_command_mut(command) {
                    Ok(Some((input, String::new(), 0)))
                } else {
                    Ok(Some((input.to_string(), String::new(), 0)))
                }
            }
            "sed" => {
                let args = command.words[1..]
                    .iter()
                    .map(|word| self.expand_word(word))
                    .collect::<Vec<_>>();
                if let Some(output) = apply_simple_sed_args(input, &args) {
                    Ok(Some((output, String::new(), 0)))
                } else {
                    self.execute_external_pipeline_stage(command, input)
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
                    return self.execute_external_pipeline_stage(command, input);
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
                            return self.execute_external_pipeline_stage(command, input);
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
                self.execute_external_pipeline_stage(command, input)
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
                    self.execute_external_pipeline_stage(command, input)
                }
            }
            _ => {
                if let Some(output) =
                    self.execute_function_pipeline_stage(command, input, force_compound_errexit)?
                {
                    Ok(Some(output))
                } else {
                    if let Some(output) = self.execute_builtin_pipeline_stage(command, input)? {
                        Ok(Some(output))
                    } else {
                        self.execute_external_pipeline_stage(command, input)
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
        if !crate::builtins::set::shell_option_enabled(&self.env_vars, "restricted") {
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
            let target = self.expand_word(&redirect.target);
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
        crate::builtins::shopt::option_enabled(&self.env_vars, "lastpipe")
    }

    fn initial_pipeline_input(&mut self, command: &CommandNode) -> String {
        self.stdin_string_for_command_mut(command)
            .or_else(|| {
                pipeline_stage_reads_stdin_by_default(command)
                    .then(|| self.read_inherited_process_stdin_to_string())
                    .flatten()
            })
            .unwrap_or_default()
    }
}

fn command_is_compound_pipeline_stage(command: &CommandNode) -> bool {
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
            pattern = Some(arg.as_str());
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
