use super::*;

#[derive(Debug, Default)]
pub(in crate::executor) struct ProcessSubstitutionFiles {
    inputs: Vec<PathBuf>,
    outputs: Vec<OutputProcessSubstitution>,
}

#[derive(Debug)]
struct OutputProcessSubstitution {
    path: PathBuf,
    source: String,
    /// Line of the command that constructed the substitution, captured
    /// at materialization time. The body runs later (at finish, in a
    /// spawned child), when the shell's current line has moved on, so
    /// the construction line must ride along — GNU's process_substitute
    /// fork (subst.c:6362) inherits the parent's line_number at fork
    /// time (rubash#1184 f4).
    line: Option<usize>,
}

fn exec_keeps_output_process_substitution(words: &[String], redirect: &Redirect) -> bool {
    if !redirect
        .target
        .strip_prefix(">(")
        .is_some_and(|target| target.ends_with(')'))
    {
        return false;
    }
    matches!(words, [command] if command == "exec")
        || matches!(words, [command, fd] if command == "exec" && dynamic_fd_word(fd).is_some())
}

pub(in crate::executor) fn command_needs_process_substitution_materialization(
    cmd: &CommandNode,
) -> bool {
    // Shell-owned `read` must consume virtual descriptors directly. Its
    // ordinary `<&N` redirects are not process substitutions; materializing
    // them into a temporary file would advance or replace the descriptor on
    // every loop iteration.
    if cmd.words.first().map(String::as_str) == Some("read")
        && cmd.process_substitutions.is_empty()
        && !cmd
            .redirects
            .iter()
            .any(|redirect| redirect.target.starts_with("<(") || redirect.target.starts_with(">("))
    {
        return false;
    }
    // Persistent fd-prefixed output substitutions are opened by the exec
    // builtin and must retain their backing file across later commands.
    if cmd.words.first().map(String::as_str) == Some("exec")
        && cmd.redirects.iter().any(|redirect| {
            matches!(
                redirect.kind,
                crate::parser::RedirectKind::Output
                    | crate::parser::RedirectKind::Append
                    | crate::parser::RedirectKind::ClobberOutput
            ) && redirect.target.starts_with(">(")
                && redirect.target.ends_with(')')
        })
    {
        return false;
    }

    if cmd.redirects.is_empty()
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.heredoc.is_none()
        && cmd.heredoc_redirects.is_empty()
        && cmd.here_string.is_none()
        && cmd.process_substitutions.is_empty()
        && !cmd
            .word_metadata
            .iter()
            .any(word_metadata_needs_process_substitution_materialization)
    {
        return cmd.words.iter().enumerate().any(|(index, word)| {
            if cmd.word_metadata.get(index).is_some() {
                return false;
            }
            word.strip_prefix("<(")
                .or_else(|| word.strip_prefix(">("))
                .is_some_and(|word| word.ends_with(')'))
        });
    }

    true
}

fn word_metadata_needs_process_substitution_materialization(metadata: &WordMetadata) -> bool {
    !metadata.process_substitutions.is_empty()
        || crate::parser::raw_word_has_unquoted_process_substitution(&metadata.raw)
}

pub(in crate::executor) fn shared_combined_output_process_substitution(
    first: Option<&Redirect>,
    second: Option<&Redirect>,
) -> Option<String> {
    let first = first?;
    let second = second?;
    if first.target != second.target
        || !matches!(first.operator.as_str(), "&>" | "&>>")
        || first.operator != second.operator
    {
        return None;
    }
    if !redirect_target_span_is_recorded(first) && !redirect_target_span_is_recorded(second) {
        return None;
    }
    first
        .target
        .strip_prefix(">(")
        .and_then(|target| target.strip_suffix(')'))
        .map(str::to_string)
}

/// rubash#1931: whether a redirect target whose VALUE text is one
/// whole process-substitution span is a REAL span — the parser
/// recorded it (unquoted) in the target's own metadata. The
/// materialization arms in `command_with_process_substitution_files`
/// match on the target's quote-stripped value, where a double-quoted
/// `"<(cmd)"` / `">(cmd)"` is indistinguishable from a live span;
/// GNU creates the substitution only for a span the parser saw
/// unquoted (parse.y read_token_word) and treats the quoted form as
/// a literal filename. Admission therefore consults the recorded
/// spans, with the same raw-gated re-derivation the word loop in
/// that function uses when a word's metadata carries none.
pub(in crate::executor) fn redirect_target_span_is_recorded(redirect: &Redirect) -> bool {
    let metadata = &redirect.target_metadata;
    let recorded = if !metadata.process_substitutions.is_empty() {
        metadata.process_substitutions.as_slice()
    } else if crate::parser::raw_word_has_unquoted_process_substitution(&metadata.raw) {
        return crate::parser::WordMetadata::new(
            0,
            redirect.target.clone(),
            redirect.target.clone(),
        )
        .process_substitutions
        .iter()
        .any(|substitution| substitution.target == redirect.target);
    } else {
        return false;
    };
    recorded
        .iter()
        .any(|substitution| substitution.target == redirect.target)
}

fn dynamic_fd_word(word: &str) -> Option<&str> {
    let name = word.strip_prefix('{')?.strip_suffix('}')?;
    (!name.is_empty()
        && name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric()))
    .then_some(name)
}

impl Executor {
    pub(in crate::executor) fn apply_external_stdin_redirect(
        &mut self,
        cmd: &CommandNode,
        process: &mut Command,
    ) -> Result<(), ExecuteError> {
        // fd-0 stdin source obeys GNU left-to-right redirect order
        // (redir.c do_redirections): the last fd-0 input redirect wins,
        // so `cat <<A <file` must NOT pipe heredoc bytes over the file.
        let stdin_body_wins = match crate::executor::shell_options::fd0_stdin_redirect_winner(cmd) {
            Some(kind) => matches!(
                kind,
                crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString
            ),
            // No ordered redirect info: legacy field checks.
            None => {
                cmd.heredoc.is_some()
                    || cmd.here_string.is_some()
                    || cmd
                        .heredoc_redirects
                        .iter()
                        .any(|redirect| redirect.fd == Some(0))
            }
        };
        if stdin_body_wins {
            // GNU applies every redirection in order (redir.c
            // do_redirections): an earlier fd-0 `< file` still runs its
            // open even though a later `<<` overrides fd 0, so a missing
            // file aborts the command rather than silently yielding the
            // heredoc body. `<> file` runs its O_RDWR|O_CREAT open
            // (make_cmd.c:682) the same way (rubash#264).
            for redirect in cmd.redirects.iter() {
                if redirect.fd.unwrap_or(0) == 0
                    && matches!(
                        redirect.kind,
                        crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite
                    )
                {
                    let target = self.expand_redirect_target(redirect);
                    if redirect_target_fd(&target).is_none() && !is_closed_redirect_target(&target)
                    {
                        if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                            self.probe_input_redirect_readwrite(&target)?;
                        } else {
                            self.probe_input_redirect(&target)?;
                        }
                    }
                }
            }
            process.stdin(Stdio::piped());
        } else if let Some(ref redirect) = cmd.redirect_in {
            let target = self.expand_redirect_target(redirect);
            if redirect.fd.unwrap_or(0) == 0 {
                // `<&N` and the move form `<&N-` (make_cmd.c:704-718) both
                // hand the child a dup of fd N's open file description; the
                // move's close of fd N is scoped to the command in the
                // parent (GNU RX_UNDOABLE) and the child's dup survives
                // independently (redir.c:1153-1166). Only `&`-prefixed
                // operands take this arm — /dev/stdin-style fd-alias NAMES
                // keep their dedicated piped-stdin handling below
                // (niubash#118).
                if let Some((fd, _move_fd)) =
                    redirect_target_fd_and_move(&target).filter(|_| target.starts_with('&'))
                {
                    match self.fd_table.read_endpoint(fd) {
                        Some(FdReadEndpoint::CoprocStdout { fd: pipe, .. }) => {
                            let dup = crate::fd::duplicate_handle_inheritable(pipe.handle)
                                .map_err(|_| {
                                    io::Error::new(
                                        io::ErrorKind::BrokenPipe,
                                        "coprocess output is closed",
                                    )
                                })?;
                            process.stdin(Stdio::from(crate::fd::handle_to_file(dup)));
                            return Ok(());
                        }
                        // `< /dev/fd/N` (and `<&N`) on a file-backed fd dup
                        // the open file into the child's stdin (GNU redir.c
                        // dup2 semantics through the /dev/fd device layer):
                        // DuplicateHandle keeps the shared file offset, so a
                        // partially-read fd feeds the child only the rest.
                        Some(FdReadEndpoint::File(file_fd)) => {
                            let dup = crate::fd::duplicate_handle(file_fd.handle)?;
                            process.stdin(Stdio::from(crate::fd::handle_to_file(dup)));
                            return Ok(());
                        }
                        // Text/process-substitution endpoints materialize to
                        // a temp file; inherited stdin dups the real handle.
                        // The child drains the stream, so consume the
                        // parent's copy too (shared-offset semantics).
                        Some(_) => {
                            let file = self.open_fd_read_endpoint(fd, &target)?;
                            self.fd_table.consume_all_text(fd);
                            process.stdin(Stdio::from(file));
                            return Ok(());
                        }
                        // `cmd <&N` with N closed/absent — GNU redir.c
                        // dup2 fails EBADF; the open never happens.
                        None => {
                            return Err(io::Error::new(
                                io::ErrorKind::Other,
                                format!("{fd}: Bad file descriptor"),
                            )
                            .into());
                        }
                    }
                }
            }
            if is_closed_redirect_target(&target) {
                if redirect.fd.unwrap_or(0) == 0 {
                    process.stdin(closed_stdin());
                }
                return Ok(());
            }
            // `< /dev/stdin` / `<&0` duplicate fd 0 — GNU dup2 keeps the
            // stage's *current* stdin, which here lives in the virtual
            // FUNCTION_STDIN/fd-table channel, not the process's own stdin
            // handle (niubash#118: piping into `cat < /dev/stdin` must feed
            // the pipe, not block on the console).
            if redirect.fd.unwrap_or(0) == 0
                && redirect_target_fd(&target) == Some(0)
                && (self.shell_state.env_vars.contains_key(FUNCTION_STDIN)
                    || self.virtual_fd_stdin_remaining(0).is_some())
            {
                process.stdin(Stdio::piped());
                return Ok(());
            }
            if redirect.fd.unwrap_or(0) == 0 && self.input_fd_redirects_to_process_stdin(&target) {
                return Ok(());
            }
            let file = if redirect.append {
                let path =
                    self.session_path(shell_path_to_windows(&target, &self.shell_state.env_vars));
                // niubash#519: CON has no read+write handle; a child's fd 0
                // only needs the console input half (FileFd::
                // open_readwrite_pair splits the fd-table binds).
                if is_con_device_path(&path) {
                    File::open("CONIN$")?
                } else {
                    OpenOptions::new()
                        .create(true)
                        .read(true)
                        .write(true)
                        .open(&path)?
                }
            } else {
                self.open_input_redirect(&target)?
            };
            if redirect.fd.unwrap_or(0) == 0 {
                process.stdin(Stdio::from(file));
            }
        } else if self.fd_table.is_closed(0)
            || (self.fd_table.has_entry(0) && !self.fd_table.is_open_for_read(0))
        {
            // rubash#1102: an exec-installed closed/write-only fd 0 shadows
            // the ambient stage buffer — the forked stage simply has no
            // readable fd 0 (GNU: `exec <&-` in a group makes its readers
            // fail with EBADF, not see the pipe bytes).
            process.stdin(closed_stdin());
        } else if let Some(
            FdReadEndpoint::File(file_fd) | FdReadEndpoint::CoprocStdout { fd: file_fd, .. },
        ) = self.fd_table.read_endpoint(0)
        {
            // GNU: fd 0 is a real descriptor and the child inherits the open
            // file description, file offset included (`exec 0<f; tr ...`
            // reads f from the current position). DuplicateHandle shares the
            // file pointer just like dup2. rubash#1102/#1103: this arm must
            // win over the FUNCTION_STDIN buffer below — inside a pipeline
            // stage an `exec <file` rebinds fd 0 to the file, and only the
            // readers that ran BEFORE the exec keep the pipe bytes.
            let dup = crate::fd::duplicate_handle(file_fd.handle)?;
            process.stdin(Stdio::from(crate::fd::handle_to_file(dup)));
        } else if self.shell_state.env_vars.contains_key(FUNCTION_STDIN)
            || self.virtual_fd_stdin_remaining(0).is_some()
        {
            process.stdin(Stdio::piped());
        }

        Ok(())
    }

    pub(in crate::executor) fn execute_external(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // rubash#1525: record the disk-command dispatch class for
        // the enclosing simple command's PIPESTATUS completion
        // write (see execute_materialized_command).
        self.external_dispatch_ran.set(true);
        // rubash#815 profiler: the external-spawn seam (GNU execute_cmd.c
        // dispatches disk commands through execute_command_internal's
        // simple-command tail; this is the fork face of the attribution
        // three-way split). Drop guard covers every exit form; the off path
        // is one relaxed AtomicBool load.
        let _profiler_external =
            super::profiler::enter_external(cmd.words.first().map(String::as_str).unwrap_or("?"));
        let (mut cmd, mut process_substitutions) =
            self.command_with_process_substitution_files(cmd)?;
        self.apply_default_external_stdin_file(&mut cmd, &mut process_substitutions)?;
        let result = self.execute_external_inner(&cmd);
        // GNU execute_cmd.c: external commands fork before do_redirections
        // — a `{var}` bind happened only in the child, so the parent's
        // variable is untouched and the allocated descriptor dies there.
        self.undo_child_fd_var_redirects();
        self.finish_process_substitutions(process_substitutions)?;
        result
    }

    pub(in crate::executor) fn command_with_process_substitution_files(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(CommandNode, ProcessSubstitutionFiles), ExecuteError> {
        let mut rewritten = cmd.clone();
        let mut files = ProcessSubstitutionFiles::default();
        // rubash#1184 f4: every output substitution materialized here is
        // constructed by THIS command; its line is the body's diagnostic
        // attribution (see OutputProcessSubstitution::line).
        let cmd_line = cmd.line;
        let mut redirect_target_rewrites = Vec::new();
        // GNU subst.c: an unquoted `name=<( )` word is an assignment whose
        // value is the process substitution (make_dev_fd_filename,
        // subst.c:6362+). The first parse splits that word into `name=` and
        // a procsub token when it appears as an eval argument, which turned
        // the materialized temp path into a command word and lost the
        // assignment (procsub.tests:25 `eval f=<(echo test4) "; cat \$f"`).
        // Re-merge the pair so the word materializes into the assignment
        // value the way a direct `name=<( )` statement parses.
        let mut merge_index = 0;
        while merge_index + 1 < rewritten.words.len() {
            let assignment_word = &rewritten.words[merge_index];
            let is_bare_assignment = assignment_word.ends_with('=')
                && !assignment_word.contains(' ')
                && is_shell_name(&assignment_word[..assignment_word.len() - 1]);
            let procsub_word = rewritten.words[merge_index + 1].clone();
            let is_whole_procsub = (procsub_word.starts_with("<(")
                || procsub_word.starts_with(">("))
                && procsub_word.ends_with(')');
            if is_bare_assignment && is_whole_procsub {
                let next_metadata = rewritten.word_metadata.get(merge_index + 1).cloned();
                rewritten.words[merge_index].push_str(&procsub_word);
                // Drop the absorbed procsub word and hand its metadata to
                // the merged word so the loop below materializes the
                // substitution inside the assignment value.
                rewritten.words.remove(merge_index + 1);
                if merge_index + 1 < rewritten.word_metadata.len() {
                    rewritten.word_metadata.remove(merge_index + 1);
                }
                if let Some(metadata) = next_metadata {
                    if merge_index < rewritten.word_metadata.len() {
                        rewritten.word_metadata[merge_index] = metadata;
                    } else {
                        rewritten.word_metadata.push(metadata);
                    }
                }
                continue;
            }
            merge_index += 1;
        }
        for word_index in 0..rewritten.words.len() {
            let metadata = rewritten.word_metadata.get(word_index).cloned();
            let substitutions = metadata
                .as_ref()
                .map(|metadata| metadata.process_substitutions.clone())
                .unwrap_or_default();
            let substitutions = if substitutions.is_empty()
                && metadata.as_ref().is_some_and(|metadata| {
                    crate::parser::raw_word_has_unquoted_process_substitution(&metadata.raw)
                }) {
                WordMetadata::new(
                    word_index,
                    rewritten.words[word_index].clone(),
                    rewritten.words[word_index].clone(),
                )
                .process_substitutions
            } else {
                substitutions
            };
            if substitutions.is_empty() {
                if metadata.is_none() {
                    self.materialize_standalone_process_substitution_word(
                        &mut rewritten.words[word_index],
                        &mut files,
                        cmd_line,
                    )?;
                }
            } else {
                self.materialize_process_substitution_word(
                    &mut rewritten.words[word_index],
                    substitutions,
                    &mut files,
                    cmd_line,
                )?;
            }
        }
        if let Some(redirect) = &mut rewritten.redirect_in {
            if let Some(source) = redirect
                .target
                .strip_prefix("<(")
                .and_then(|target| target.strip_suffix(')'))
                // rubash#1931: only a parser-recorded (unquoted) span
                // materializes; a quoted "<(...)" target is a literal
                // filename in GNU.
                .filter(|_| redirect_target_span_is_recorded(redirect))
            {
                // rubash#1952: stream first — name the spawned body's
                // pipe by its word fd (GNU subst.c:6362 hands the
                // consumer a /dev/fd/N); the downstream redirect
                // machinery resolves the alias to the live endpoint.
                // The collected temp-carrier form below is the
                // spawn-failure fallback.
                if let Some(streamed) = self.process_substitution_child_fd_word(source, cmd_line) {
                    let old_target = redirect.target.clone();
                    redirect.target = streamed;
                    redirect_target_rewrites.push((old_target, redirect.target.clone()));
                } else if let Some((output, body_timing)) =
                    self.process_substitution_output_bytes_timed(source)
                {
                    let path = self.write_process_substitution_temp_bytes(&output)?;
                    // rubash#1141: the temp path is registered as a procsub
                    // stream by the write; attach the body's timing so the
                    // fd-0 bind (and literal-redirect readers) can judge
                    // TMOUT against the recorded arrival schedule.
                    self.procsub_stream_set_timing(&path, body_timing);
                    let old_target = redirect.target.clone();
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    redirect_target_rewrites.push((old_target, redirect.target.clone()));
                    files.inputs.push(path);
                }
            }
            if redirect.fd.unwrap_or(0) == 0 {
                let target = self.expand_redirect_target(redirect);
                if let Some(fd) = redirect_target_fd(&target) {
                    if self.fd_table.is_open_for_read(fd) {
                        if let Some(input) = self.virtual_fd_stdin_remaining_bytes(fd) {
                            let path = self.write_process_substitution_temp_bytes(&input)?;
                            let input_len = self.virtual_fd_stdin_len(fd);
                            self.fd_table.consume_all_text(fd);
                            self.shell_state
                                .env_vars
                                .insert(fd_stdin_offset_key(fd), input_len);
                            redirect.target = shell_display_path(&path.to_string_lossy());
                            files.inputs.push(path);
                        }
                        // File-backed fds keep `&N`: the spawn layer dups
                        // the real handle (shared offset) — a path rewrite
                        // would reopen the file at offset 0.
                    } else if let Some(input) = self.external_fd_heredoc_input(cmd, fd) {
                        let path = self.write_process_substitution_temp(&input)?;
                        redirect.target = shell_display_path(&path.to_string_lossy());
                        files.inputs.push(path);
                    }
                }
            }
        }
        let exec_words = rewritten.words.clone();
        if let Some(source) = shared_combined_output_process_substitution(
            rewritten.redirect_out.as_deref(),
            rewritten.redirect_err_append.as_deref(),
        ) {
            let path = self.empty_process_substitution_temp()?;
            let display_path = shell_display_path(&path.to_string_lossy());
            if let Some(redirect) = &mut rewritten.redirect_out {
                let old_target = redirect.target.clone();
                redirect.target = display_path.clone();
                redirect_target_rewrites.push((old_target, redirect.target.clone()));
            }
            if let Some(redirect) = &mut rewritten.redirect_err_append {
                let old_target = redirect.target.clone();
                redirect.target = display_path;
                redirect_target_rewrites.push((old_target, redirect.target.clone()));
            }
            files.outputs.push(OutputProcessSubstitution {
                path,
                source,
                line: cmd_line,
            });
        }
        if let Some(source) = shared_combined_output_process_substitution(
            rewritten.append.as_deref(),
            rewritten.redirect_err_append.as_deref(),
        ) {
            let path = self.empty_process_substitution_temp()?;
            let display_path = shell_display_path(&path.to_string_lossy());
            if let Some(redirect) = &mut rewritten.append {
                let old_target = redirect.target.clone();
                redirect.target = display_path.clone();
                redirect_target_rewrites.push((old_target, redirect.target.clone()));
            }
            if let Some(redirect) = &mut rewritten.redirect_err_append {
                let old_target = redirect.target.clone();
                redirect.target = display_path;
                redirect_target_rewrites.push((old_target, redirect.target.clone()));
            }
            files.outputs.push(OutputProcessSubstitution {
                path,
                source,
                line: cmd_line,
            });
        }
        if let Some(redirect) = &mut rewritten.redirect_out {
            if !exec_keeps_output_process_substitution(&exec_words, redirect) {
                if let Some(source) = redirect
                    .target
                    .strip_prefix(">(")
                    .and_then(|target| target.strip_suffix(')'))
                    // rubash#1931: recorded-span admission, as above.
                    .filter(|_| redirect_target_span_is_recorded(redirect))
                {
                    let source = source.to_string();
                    let path = self.empty_process_substitution_temp()?;
                    let old_target = redirect.target.clone();
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    redirect_target_rewrites.push((old_target, redirect.target.clone()));
                    files.outputs.push(OutputProcessSubstitution {
                        path,
                        source,
                        line: cmd_line,
                    });
                }
            }
        }
        if let Some(redirect) = &mut rewritten.append {
            if !exec_keeps_output_process_substitution(&exec_words, redirect) {
                if let Some(source) = redirect
                    .target
                    .strip_prefix(">(")
                    .and_then(|target| target.strip_suffix(')'))
                    // rubash#1931: recorded-span admission, as above.
                    .filter(|_| redirect_target_span_is_recorded(redirect))
                {
                    let source = source.to_string();
                    let path = self.empty_process_substitution_temp()?;
                    let old_target = redirect.target.clone();
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    redirect_target_rewrites.push((old_target, redirect.target.clone()));
                    files.outputs.push(OutputProcessSubstitution {
                        path,
                        source,
                        line: cmd_line,
                    });
                }
            }
        }
        if let Some(redirect) = &mut rewritten.redirect_err {
            if !exec_keeps_output_process_substitution(&exec_words, redirect) {
                if let Some(source) = redirect
                    .target
                    .strip_prefix(">(")
                    .and_then(|target| target.strip_suffix(')'))
                    // rubash#1931: recorded-span admission, as above.
                    .filter(|_| redirect_target_span_is_recorded(redirect))
                {
                    let source = source.to_string();
                    let path = self.empty_process_substitution_temp()?;
                    let old_target = redirect.target.clone();
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    redirect_target_rewrites.push((old_target, redirect.target.clone()));
                    files.outputs.push(OutputProcessSubstitution {
                        path,
                        source,
                        line: cmd_line,
                    });
                }
            }
        }
        if let Some(redirect) = &mut rewritten.redirect_err_append {
            if !exec_keeps_output_process_substitution(&exec_words, redirect) {
                if let Some(source) = redirect
                    .target
                    .strip_prefix(">(")
                    .and_then(|target| target.strip_suffix(')'))
                    // rubash#1931: recorded-span admission, as above.
                    .filter(|_| redirect_target_span_is_recorded(redirect))
                {
                    let source = source.to_string();
                    let path = self.empty_process_substitution_temp()?;
                    let old_target = redirect.target.clone();
                    redirect.target = shell_display_path(&path.to_string_lossy());
                    redirect_target_rewrites.push((old_target, redirect.target.clone()));
                    files.outputs.push(OutputProcessSubstitution {
                        path,
                        source,
                        line: cmd_line,
                    });
                }
            }
        }
        for redirect in &mut rewritten.redirects {
            if let Some((_, new_target)) = redirect_target_rewrites
                .iter()
                .find(|(old_target, _)| redirect.target == *old_target)
            {
                redirect.target = new_target.clone();
            }
        }
        // The rewritten command no longer carries process-substitution
        // syntax in its words or redirect targets. Clear the consumed
        // markers so a second pass over the same node (pipeline stage
        // helpers re-enter through their own materialization call) is a
        // no-op instead of re-running every substitution source.
        rewritten.process_substitutions.clear();
        for metadata in &mut rewritten.word_metadata {
            metadata.process_substitutions.clear();
        }
        sync_ordered_redirect_targets(&mut rewritten);
        Ok((rewritten, files))
    }

    fn materialize_standalone_process_substitution_word(
        &mut self,
        word: &mut String,
        files: &mut ProcessSubstitutionFiles,
        line: Option<usize>,
    ) -> Result<(), ExecuteError> {
        if let Some(source) = word
            .strip_prefix("<(")
            .and_then(|word| word.strip_suffix(')'))
        {
            // rubash#1952: stream first — GNU process_substitute
            // (subst.c:6362) forks the body onto a pipe at expansion
            // time, so the consumer starts on the first bytes; the
            // collected form below starves it until the body exits
            // (forever for a non-terminating producer) and remains
            // only as the spawn-failure fallback.
            if let Some(streamed) = self.process_substitution_child_fd_word(source, line) {
                *word = streamed;
                return Ok(());
            }
            let Some(output) = self.process_substitution_output_bytes(source) else {
                return Ok(());
            };
            let path = self.write_process_substitution_temp_bytes(&output)?;
            *word = self.process_substitution_fd_word(&output);
            files.inputs.push(path);
        } else if let Some(source) = word
            .strip_prefix(">(")
            .and_then(|word| word.strip_suffix(')'))
        {
            let source = source.to_string();
            let path = self.empty_process_substitution_temp()?;
            *word = self.process_substitution_output_fd_word(&path);
            files.outputs.push(OutputProcessSubstitution {
                path,
                source,
                line: line,
            });
        }
        Ok(())
    }

    fn materialize_process_substitution_word(
        &mut self,
        word: &mut String,
        substitutions: Vec<crate::parser::ProcessSubstitution>,
        files: &mut ProcessSubstitutionFiles,
        line: Option<usize>,
    ) -> Result<(), ExecuteError> {
        for substitution in substitutions {
            if substitution.output {
                let path = self.empty_process_substitution_temp()?;
                files.outputs.push(OutputProcessSubstitution {
                    path: path.clone(),
                    source: substitution.source,
                    line,
                });
                let display_path = self.process_substitution_output_fd_word(&path);
                *word = word.replacen(&substitution.target, &display_path, 1);
            } else {
                // rubash#1952: stream first (see
                // materialize_standalone_process_substitution_word);
                // the collected form below is the spawn-failure
                // fallback.
                if let Some(streamed) =
                    self.process_substitution_child_fd_word(&substitution.source, line)
                {
                    *word = word.replacen(&substitution.target, &streamed, 1);
                    continue;
                }
                let Some((output, body_timing)) =
                    self.process_substitution_output_bytes_timed(&substitution.source)
                else {
                    continue;
                };
                let path = self.write_process_substitution_temp_bytes(&output)?;
                files.inputs.push(path.clone());
                // rubash#1141: the same production schedule serves both
                // carriers of the substitution's bytes — the fd-table
                // endpoint (word form) and the draining stream (temp
                // path form) share one virtual clock.
                self.procsub_stream_set_timing(&path, body_timing.clone());
                let display_path = self.process_substitution_fd_word_timed(&output, body_timing);
                *word = word.replacen(&substitution.target, &display_path, 1);
            };
        }
        Ok(())
    }

    /// Materialize the process substitutions of an assignment RHS from its
    /// VERBATIM raw text.
    ///
    /// GNU subst.c:11349-11378 (`expand_word_internal`, cases '<'/'>'): an
    /// unquoted `<(`/`>(` in a word is extracted by `extract_process_subst`
    /// (subst.c:1311, via `xparse_dolparen` — the body keeps its own quoting
    /// because the span is pulled out of the word string before any quote
    /// removal) and executed by `process_substitute` (subst.c:6362), whose
    /// `/dev/fd/N` result replaces the span in the expanding word. This runs
    /// in assignment position exactly like argument position (W_NOPROCSUB is
    /// never set for assignment words).
    ///
    /// Rubash's lexer de-quotes the whole `name=<(...)` token at once
    /// (`finish_word_token`), so the executor's assignment value arrives with
    /// the substitution body's quotes already stripped; the parser keeps the
    /// verbatim RHS in `CommandNode::assignment_raws`. Running the body from
    /// the de-quoted text corrupts it (`x=<(printf '\377')` would execute
    /// `printf \377` and print `377`); this entry re-derives the body from
    /// the raw RHS so the substitution sees the same text GNU extracts.
    /// Everything outside the span keeps its raw quoting, which the normal
    /// assignment-expansion chain then processes as usual.
    /// Splice range for one assignment process substitution (rubash#1561).
    /// The metadata target is a token-join RE-PRINT that need not occur
    /// verbatim in the word (`>&2` re-prints as `>& 2`), so after the
    /// exact search the span is located VERBATIM — GNU subst.c:1311
    /// extract_process_subst pulls the span out of the word string —
    /// and verified against the metadata source re-print. The cursor
    /// search keeps identical spans in left-to-right order; the
    /// from-zero retry covers metadata order differing from word order
    /// (a token-shape span listed before an embedded span that sits
    /// earlier in the word).
    fn assignment_procsub_splice_range(
        word: &str,
        cursor: usize,
        target: &str,
        operator: &str,
        source: &str,
    ) -> Option<(usize, usize)> {
        let exact = |from: usize| {
            word[from..]
                .find(target)
                .map(|offset| (from + offset, from + offset + target.len()))
        };
        let verbatim = |from: usize| {
            crate::parser::verbatim_process_substitution_span(word, from, operator, source)
        };
        exact(cursor)
            .or_else(|| verbatim(cursor))
            .or_else(|| exact(0))
            .or_else(|| verbatim(0))
    }

    pub(in crate::executor) fn materialize_assignment_process_substitutions_from_raw(
        &mut self,
        raw: &str,
    ) -> Result<String, ExecuteError> {
        let substitutions = crate::parser::WordMetadata::new(0, raw.to_string(), raw.to_string())
            .process_substitutions;

        let mut word = raw.to_string();
        let mut cursor = 0usize;
        for substitution in substitutions {
            // rubash#1523: the metadata target is the RE-PRINTED span
            // (token join — `>&3` comes back as `>& 3`), which need
            // not occur verbatim in the raw word. Splicing by that
            // string then silently misses, the span survives into
            // expand_assignment_value_inner, and its cooked-pass
            // materialize registers the SAME substitution a second
            // time — the finish drained both carriers and the body
            // ran twice. Register only when the splice will land;
            // a missed span stays in the word for the cooked pass,
            // which materializes it exactly once from the re-printed
            // text its own scan produced.
            //
            // rubash#1561: the splice search now falls back to the
            // verbatim span (see assignment_procsub_splice_range), so
            // a re-print-divergent body (`>&2`, pipes, `;`) is spliced
            // here exactly once instead of surviving into the stored
            // value as literal text.
            let Some((start, end)) = Self::assignment_procsub_splice_range(
                &word,
                cursor,
                &substitution.target,
                &substitution.operator,
                &substitution.source,
            ) else {
                continue;
            };
            if substitution.output {
                let path = self.empty_process_substitution_temp()?;
                let word_form = self.process_substitution_output_fd_word(&path);
                self.insert_assignment_output_process_substitution(
                    word_form.clone(),
                    substitution.source,
                );
                word.replace_range(start..end, &word_form);
                cursor = start + word_form.len();
            } else {
                // rubash#1952: stream first (see
                // materialize_process_substitution_word); the collected
                // form below is the spawn-failure fallback.
                if let Some(word_form) =
                    self.process_substitution_child_fd_word(&substitution.source, None)
                {
                    word.replace_range(start..end, &word_form);
                    cursor = start + word_form.len();
                    continue;
                }
                let Some(output) = self.process_substitution_output_bytes(&substitution.source)
                else {
                    continue;
                };
                // rubash#1691: keep the carrier path — the per-command
                // finish removes it (see the field docs); discarding it
                // here leaked the file in $TMPDIR for good.
                let carrier = self.write_process_substitution_temp_bytes(&output)?;
                self.assignment_input_process_substitution_files
                    .push(carrier);
                let word_form = self.process_substitution_fd_word(&output);
                word.replace_range(start..end, &word_form);
                cursor = start + word_form.len();
            }
        }
        Ok(word)
    }

    pub(in crate::executor) fn materialize_assignment_process_substitutions(
        &mut self,
        value: &str,
    ) -> Result<String, ExecuteError> {
        if !value.contains("<(") && !value.contains(">(") {
            return Ok(value.to_string());
        }

        let mut word = value.to_string();
        let substitutions =
            crate::parser::WordMetadata::new(0, value.to_string(), value.to_string())
                .process_substitutions;

        let mut cursor = 0usize;
        for substitution in substitutions {
            // rubash#1561: same splice search as the raw channel — the
            // cooked word still carries the span verbatim while the
            // metadata target is the re-print, so a plain replacen
            // missed and the literal span survived into the stored
            // value. Registration still happens on a total miss (the
            // historical cooked-pass behavior); only the splice is
            // conditional on locating the span.
            let range = Self::assignment_procsub_splice_range(
                &word,
                cursor,
                &substitution.target,
                &substitution.operator,
                &substitution.source,
            );
            if substitution.output {
                let path = self.empty_process_substitution_temp()?;
                let word_form = self.process_substitution_output_fd_word(&path);
                self.insert_assignment_output_process_substitution(
                    word_form.clone(),
                    substitution.source,
                );
                if let Some((start, end)) = range {
                    word.replace_range(start..end, &word_form);
                    cursor = start + word_form.len();
                }
            } else {
                // rubash#1952: stream first (see
                // materialize_process_substitution_word); the
                // registration happens even when the splice misses,
                // mirroring the collected form below.
                if let Some(word_form) =
                    self.process_substitution_child_fd_word(&substitution.source, None)
                {
                    if let Some((start, end)) = range {
                        word.replace_range(start..end, &word_form);
                        cursor = start + word_form.len();
                    }
                    continue;
                }
                let Some(output) = self.process_substitution_output_bytes(&substitution.source)
                else {
                    continue;
                };
                // rubash#1691: same carrier bookkeeping as the raw
                // channel above.
                let carrier = self.write_process_substitution_temp_bytes(&output)?;
                self.assignment_input_process_substitution_files
                    .push(carrier);
                let word_form = self.process_substitution_fd_word(&output);
                if let Some((start, end)) = range {
                    word.replace_range(start..end, &word_form);
                    cursor = start + word_form.len();
                }
            }
        }
        Ok(word)
    }

    fn apply_default_external_stdin_file(
        &mut self,
        rewritten: &mut CommandNode,
        files: &mut ProcessSubstitutionFiles,
    ) -> Result<(), ExecuteError> {
        if rewritten.redirect_in.is_some()
            || rewritten.heredoc.is_some()
            || rewritten.here_string.is_some()
            || self.virtual_fd_stdin_remaining_bytes(0).is_none()
        {
            return Ok(());
        }

        let input = self.virtual_fd_stdin_remaining_bytes(0).unwrap_or_default();
        let path = self.write_process_substitution_temp_bytes(&input)?;
        let input_len = self.virtual_fd_stdin_len(0);
        self.fd_table.consume_all_text(0);
        self.shell_state
            .env_vars
            .insert(fd_stdin_offset_key(0), input_len);
        let target = shell_display_path(&path.to_string_lossy());
        rewritten.redirect_in = Some(Box::new(Redirect {
            fd: Some(0),
            fd_var: None,
            operator: "<".to_string(),
            operator_metadata: Box::new(crate::parser::WordMetadata::new(
                0,
                "<".to_string(),
                "<".to_string(),
            )),
            kind: crate::parser::RedirectKind::Input,
            target_metadata: Box::new(crate::parser::WordMetadata::new(
                0,
                target.clone(),
                target.clone(),
            )),
            target,
            append: false,
            clobber: false,
        }));
        files.inputs.push(path);
        Ok(())
    }

    pub(in crate::executor) fn finish_process_substitutions(
        &mut self,
        files: ProcessSubstitutionFiles,
    ) -> Result<(), ExecuteError> {
        let mut error = None;
        for output in &files.outputs {
            if error.is_none() {
                if let Err(output_error) = self.execute_output_process_substitution(output) {
                    error = Some(output_error);
                }
            }
        }
        self.cleanup_process_substitution_files(files);
        // rubash#1691: ad-hoc carriers written while this command's
        // redirects and fd endpoints were served (see
        // ShellState::procsub_adhoc_carriers) die at the same finish —
        // their consumers (a spawned child, an in-shell read) are done
        // by now. The opened File handles stay valid on unix even
        // after the unlink.
        let adhoc = std::mem::take(&mut *self.shell_state.procsub_adhoc_carriers.borrow_mut());
        for path in adhoc {
            let _ = fs::remove_file(path);
        }
        if let Some(error) = error {
            return Err(error);
        }
        Ok(())
    }

    /// rubash#545 (cell 3): run the `>(...)` bodies queued by `[[ ]]`
    /// operand realization — each reads its temp carrier as stdin, the way
    /// the simple-command finish path executes output process
    /// substitutions.
    pub(in crate::executor) fn finish_conditional_output_procsubs(
        &mut self,
    ) -> Result<(), ExecuteError> {
        for (path, source, line) in std::mem::take(&mut self.conditional_output_procsubs) {
            self.execute_output_process_substitution(&OutputProcessSubstitution {
                path,
                source,
                line,
            })?;
        }
        Ok(())
    }

    /// Register one materialized assignment-RHS output process
    /// substitution for its finish (rubash#1523: the queue is in
    /// materialization order; re-registering a word form replaces
    /// its body instead of queueing a second run).
    fn insert_assignment_output_process_substitution(&mut self, word_form: String, source: String) {
        if let Some(entry) = self
            .assignment_output_process_substitutions
            .iter_mut()
            .find(|(key, _)| *key == word_form)
        {
            entry.1 = source;
        } else {
            self.assignment_output_process_substitutions
                .push((word_form, source));
        }
    }

    pub(in crate::executor) fn finish_assignment_output_process_substitutions_for_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // rubash#1691: assignment-RHS INPUT procsub carriers die with
        // this command — GNU's substitution fd is closed when the
        // command finishes, so the stored /dev/fd/N word is dead from
        // the next command on (verified oracle); the payload bytes
        // stay readable through the in-memory stream registry for
        // in-shell consumers. This drain must run even when the
        // output queue below is empty (the common input-only case).
        if !self.assignment_input_process_substitution_files.is_empty() {
            let carriers = std::mem::take(&mut self.assignment_input_process_substitution_files);
            self.cleanup_process_substitution_files(ProcessSubstitutionFiles {
                inputs: carriers,
                outputs: Vec::new(),
            });
        }
        if self.assignment_output_process_substitutions.is_empty() {
            return Ok(());
        }

        // rubash#1523: the bodies are child processes in GNU
        // (subst.c process_substitute forks them at
        // materialization) — what they write is observable, their
        // statuses are not: running one must not move this
        // command's exit status (the drain previously overwrote
        // the enclosing command's $? with the body's).
        let saved_exit_code = self.exit_code;
        let targets = [
            cmd.redirect_out.as_deref(),
            cmd.append.as_deref(),
            cmd.redirect_err.as_deref(),
            cmd.redirect_err_append.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter_map(|redirect| {
            let target = self.expand_redirect_target(redirect);
            self.assignment_output_process_substitutions
                .iter()
                .any(|(key, _)| key == &target)
                .then_some(target)
        })
        .collect::<Vec<_>>();

        for target in targets {
            self.finish_assignment_output_process_substitution_target(&target)?;
        }

        let remaining_targets = self
            .assignment_output_process_substitutions
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for target in remaining_targets {
            self.finish_assignment_output_process_substitution_target(&target)?;
        }

        self.exit_code = saved_exit_code;
        Ok(())
    }

    fn finish_assignment_output_process_substitution_target(
        &mut self,
        target: &str,
    ) -> Result<(), ExecuteError> {
        let Some(position) = self
            .assignment_output_process_substitutions
            .iter()
            .position(|(key, _)| key == target)
        else {
            return Ok(());
        };
        let (_, source) = self
            .assignment_output_process_substitutions
            .remove(position);
        // rubash#355: the recorded key is the `/dev/fd/N` word form; the
        // bytes live in the fd's ProcessSubstitution write carrier (its
        // temp path), not at the word itself.
        let word_fd = target
            .strip_prefix("/dev/fd/")
            .and_then(|fd| fd.parse::<u32>().ok());
        let path = if let Some(fd) = word_fd {
            match self.fd_table.output_endpoint(fd) {
                Some(FdWriteEndpoint::ProcessSubstitution { path, .. }) => path,
                _ => return Ok(()),
            }
        } else {
            shell_path_to_windows(target, &self.shell_state.env_vars)
        };
        let input = fs::read_to_string(&path).unwrap_or_default();
        // rubash#1523: a failing body — including one that runs
        // `exit` — is the child process's business, not this
        // command's error; propagating it aborted the enclosing
        // script with the body's status. Diagnostics the body
        // itself prints still go out on its own stderr.
        let _ = self.execute_persistent_output_process_substitution(&source, input, None);
        let _ = fs::remove_file(path);
        // rubash#1871: the fd dies with the assignment command, exactly
        // like the input side's command-scoped close (#545 cell 2/3,
        // #1691): GNU closes a command's process-substitution fds when
        // the command finishes, so the stored `/dev/fd/N` word dangles
        // from the next command on and an open by that name fails
        // ENOENT. Leaving the fd-table entry behind let a later
        // `printf ... > "$p"` resolve in-shell to the dead carrier and
        // silently swallow the write (the carrier file recreated by
        // the flush open, the consumer never re-run).
        if let Some(fd) = word_fd {
            self.fd_table.entries.remove(&fd);
        }
        Ok(())
    }

    pub(in crate::executor) fn cleanup_process_substitution_files(
        &self,
        files: ProcessSubstitutionFiles,
    ) {
        for path in files.inputs {
            let _ = fs::remove_file(path);
        }
        for output in files.outputs {
            let _ = fs::remove_file(output.path);
        }
    }

    fn execute_output_process_substitution(
        &mut self,
        output: &OutputProcessSubstitution,
    ) -> Result<(), ExecuteError> {
        let input = fs::read(&output.path).unwrap_or_default();
        // GNU subst.c:6362 process_substitute runs the `>(...)` consumer as a
        // FORKED async child (FORK_ASYNC|FORK_PROCSUB): it reads the
        // substitution pipe as its fd 0 concurrently with the commands that
        // follow, and the parent never blocks on it — the child is reaped
        // only by `wait`/SIGCHLD (jobs.c procsub_add). Rubash's carrier is a
        // temp file rather than a live pipe, so the body runs as a detached
        // `rubash -c` child fed by an OS pipe carrying the captured bytes: a
        // slow or looping consumer (`sleep`, `while read`) can no longer
        // stall the parent's next command (rubash#1220), and the child's own
        // `exit`/traps/variables stay child-local by construction (subst.c
        // 6421 restore_original_signals, :6628 exit).
        self.spawn_output_process_substitution_child(&output.source, input, output.line)
    }

    /// Spawn the `>(body)` consumer detached, the stand-in for GNU's
    /// process_substitute fork. `input` is the bytes the owning command
    /// wrote into the temp carrier; they stream to the child's fd 0 over an
    /// OS pipe from a writer thread, so a slow reader paces the writer
    /// exactly like GNU's pipe. Registration matches `cmd &` minus the job
    /// listing (GNU parks procsub children on a separate list — jobs.c
    /// procsub_add keeps them off `jobs`): the handle lands in
    /// `background_children` so `wait $!` and a bare `wait` reap it, and $!
    /// names the real child pid (subst.c:6478 procsub_add -> last_procsub_pid
    /// -> last_made_pid via execute_cmd.c).
    fn spawn_output_process_substitution_child(
        &mut self,
        body: &str,
        input: Vec<u8>,
        line: Option<usize>,
    ) -> Result<(), ExecuteError> {
        let exe = std::env::var_os("CARGO_BIN_EXE_rubash")
            .map(std::path::PathBuf::from)
            .or_else(test_rubash_binary_from_current_exe)
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| "rubash".into());
        // The forked child keeps the parent's functions
        // (background_function_prelude serializes them ahead of the body),
        // so `>(myfunc)` resolves the same definition GNU's fork would see.
        let mut source = self.background_function_prelude();
        // The body's first line sits this many lines into the `-c`
        // source; the line-offset seed below compensates for it.
        let prelude_newlines = source.matches('\n').count();
        source.push_str(body);
        // Same environment transport as the `cmd &` respawn
        // (compound_exec.rs execute_background_ast_command): the process env
        // merged with the shell's env_vars, `__RUBASH_*` keys crossing only
        // through the spawn-inherited whitelist, and the parent's caught
        // traps cancelled for the child (subst.c:6421).
        let mut env_map: std::collections::BTreeMap<String, String> = std::env::vars().collect();
        for (key, value) in &self.shell_state.env_vars {
            if !key.starts_with("__RUBASH_") || rubash_spawn_inherited_state(key) {
                env_map.insert(key.clone(), value.clone());
            }
        }
        env_map.retain(|key, _| {
            !(key.starts_with("__RUBASH_TRAP") && key != crate::builtins::trap::TRAP_ORIG_IGNORES)
        });
        env_map.insert("__RUBASH_SHELL_PID".to_string(), self.shell_pid.to_string());
        // subst.c:6609 subshell_level++ — the procsub child runs one level
        // deeper than the parent for $BASH_SUBSHELL/$BASHPID.
        env_map.insert(
            "__RUBASH_SUBSHELL_DEPTH".to_string(),
            (self.shell_state.subshell_depth.get() + 1).to_string(),
        );
        env_map.insert(
            "__RUBASH_BG_BASHPID_BASE".to_string(),
            (self.shell_state.subshell_depth.get() + 1).to_string(),
        );
        // Script identity for the body's `script: line N:` diagnostics —
        // the same markers the `&` respawn forwards.
        if let Some(script_name) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
            env_map.insert("__RUBASH_SCRIPT_NAME".to_string(), script_name.clone());
            env_map.insert(
                "__RUBASH_CURRENT_LINE".to_string(),
                self.shell_state
                    .env_vars
                    .get("__RUBASH_CURRENT_LINE")
                    .cloned()
                    .unwrap_or_else(|| "1".to_string()),
            );
        }
        // rubash#1184 f4: when the construction line was captured at
        // materialization, it overrides the finish-time current line —
        // GNU's fork inherits line_number when process_substitute runs
        // (subst.c:6362), not when the consumer is reaped. The offset
        // follows the `&` respawn's protocol (line - 1), less the
        // prelude's newlines, so the body's first line reports at the
        // construction line in the child's own parse.
        if let Some(line) = line {
            env_map.insert("__RUBASH_CURRENT_LINE".to_string(), line.to_string());
            let line_offset = line.saturating_sub(1).saturating_sub(prelude_newlines);
            env_map.insert("__RUBASH_LINE_OFFSET".to_string(), line_offset.to_string());
        }
        // GNU setup_async_signals (subst.c:6424): the async child ignores
        // SIGINT/SIGQUIT while job control is off — the entry-ignore marker
        // the `&` respawn stamps.
        #[cfg(unix)]
        if !crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "monitor") {
            env_map.insert(
                crate::builtins::kill::ASYNC_CHILD_ENV_KEY.to_string(),
                "1".to_string(),
            );
        }
        let mut spawn_args = vec!["-c".to_string(), source.clone()];
        if respawn_source_needs_file(&exe, &source) {
            if let Ok(path) = self.write_respawn_command_file(&source) {
                env_map.insert(
                    "__RUBASH_CMD_FILE".to_string(),
                    path.to_string_lossy().into_owned(),
                );
                spawn_args[1] = String::new();
            }
        }
        let mut process = std::process::Command::new(&exe);
        process
            .args(&spawn_args)
            .envs(&env_map)
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        process.current_dir(&self.shell_state.cwd);
        suppress_orphan_console(&mut process);
        let mut child = process.spawn()?;
        let pid = child.id();
        adopt_orphan_console(pid);
        if let Some(mut stdin) = child.stdin.take() {
            std::thread::spawn(move || {
                use std::io::Write as _;
                // A child that exits early leaves a broken pipe; the
                // feed must end as EPIPE, not kill the shell (SIGPIPE is
                // SIG_DFL process-wide since rubash#1333).
                #[cfg(unix)]
                crate::builtins::kill::block_sigpipe_on_this_thread();
                let _ = stdin.write_all(&input);
            });
        }
        self.background_children
            .insert(pid, crate::fd::SpawnedChild::from(child));
        self.shell_state.last_background_pid = Some(pid);
        Ok(())
    }

    /// GNU subst.c:6362 process_substitute(): the substituted word is
    /// `/dev/fd/N` — the parent's end of the substitution pipe, moved to a
    /// high fd below 64 (move_to_high_fd, the same band the coproc fds
    /// 63/60 come from). Windows has no /dev/fd filesystem, but the
    /// shell's virtual fd table can carry the stream: every `/dev/fd/N`
    /// consumer already resolves through it — redirects via
    /// execution_misc::dev_stdio_redirect_fd -> open_fd_read_endpoint
    /// (FdReadEndpoint::ProcessSubstitution), external argv via
    /// dev_fd_operands::materialize_dev_fd_operands ->
    /// dev_fd_table_operand, in-shell reads via the endpoint itself
    /// (rubash#355: `case <(echo c) in /dev/fd/*` and `echo <(cmd)` are
    /// script-visible and GNU prints the /dev/fd form).
    ///
    /// The temp file stays the backing store for Windows child processes
    /// (registered in the stream table under its path and deleted by
    /// ProcessSubstitutionFiles::drop); the fd-table endpoint owns its own
    /// copy so the shared-offset drain semantics (procsub.tests
    /// count_lines 1,0,0,0,0) survive word-form changes. The fd stays
    /// registered for the shell's lifetime — GNU marks the real fd
    /// close-on-exec but keeps it in the parent, so a variable carrying
    /// `/dev/fd/N` remains resolvable later.
    fn register_process_substitution_fd(&mut self, output: &[u8]) -> u32 {
        let fd = self.allocate_process_substitution_word_fd();
        self.fd_table.open_input(
            fd,
            FdReadEndpoint::process_substitution_bytes(output.to_vec()),
            true,
        );
        fd
    }

    /// The fd a word-form input process substitution binds.
    /// move_to_high_fd (general.c:681) is dup2-backed in GNU — it scans
    /// downward from maxfd-1 via fcntl F_GETFD and dup2()s into the
    /// first free slot — a different mechanism from the fcntl F_DUPFD
    /// that backs {var} allocation (redir.c:942). The limit-aware
    /// fallback below is bounded by the same RLIMIT_NOFILE soft limit
    /// as every other dynamic allocation (rubash#773); its low-first
    /// pick order is a pre-existing simplification of GNU's high-side
    /// scan.
    fn allocate_process_substitution_word_fd(&mut self) -> u32 {
        let mut fd = 63u32;
        while fd > 3 && !self.fd_table_fd_free(fd) {
            fd -= 1;
        }
        if fd <= 3 {
            return self.allocate_dynamic_fd().unwrap_or(63);
        }
        fd
    }

    /// rubash#1952: the streaming registration behind a word-form
    /// input process substitution — spawn the body as a child shell
    /// on a real pipe (GNU subst.c:6362 process_substitute) and bind
    /// the pipe read end at the word fd, so the consumer starts on
    /// the body's first bytes instead of after its exit. The fd is
    /// command-scoped exactly like the collected form's (see
    /// process_substitution_fd_word). Returns None when the spawn
    /// could not be set up; the caller then collects synchronously.
    fn register_process_substitution_child(
        &mut self,
        source: &str,
        line: Option<usize>,
    ) -> Option<u32> {
        let file_fd = self.spawn_input_process_substitution_child(source, line)?;
        let fd = self.allocate_process_substitution_word_fd();
        self.fd_table
            .open_input(fd, FdReadEndpoint::File(file_fd), true);
        self.command_scoped_procsub_input_fds.push(fd);
        Some(fd)
    }

    /// The word form of a streamed input process substitution:
    /// `/dev/fd/N` per GNU subst.c:6362, naming the pipe read end
    /// bound by register_process_substitution_child.
    pub(in crate::executor) fn process_substitution_child_fd_word(
        &mut self,
        source: &str,
        line: Option<usize>,
    ) -> Option<String> {
        self.register_process_substitution_child(source, line)
            .map(|fd| format!("/dev/fd/{fd}"))
    }

    fn fd_table_fd_free(&self, fd: u32) -> bool {
        match self.fd_table.entries.get(&fd) {
            // No entry = free (GNU move_to_high_fd skips only fds the
            // shell actually holds open).
            None => true,
            Some(entry) => entry.closed || (entry.read.is_none() && entry.write.is_none()),
        }
    }

    /// The word form of a materialized input process substitution:
    /// `/dev/fd/N` per GNU subst.c:6362, with the payload registered under
    /// that fd (see register_process_substitution_fd). rubash#545 (cell
    /// 2): the fd is command-scoped — execute_materialized_command closes
    /// every fd recorded here when the creating command finishes, matching
    /// GNU's reap (`a=(<(echo p9)); cat "${a[0]}"` reports `No such file`).
    pub(in crate::executor) fn process_substitution_fd_word(&mut self, output: &[u8]) -> String {
        self.process_substitution_fd_word_timed(output, None)
    }

    /// rubash#1141: `process_substitution_fd_word` with the substitution
    /// body's production schedule attached to the registered endpoint,
    /// so reads through the fd observe GNU's pipe arrival times.
    pub(in crate::executor) fn process_substitution_fd_word_timed(
        &mut self,
        output: &[u8],
        timing: Option<super::timed_input::SharedTimedInput>,
    ) -> String {
        let fd = self.register_process_substitution_fd(output);
        if let Some(timing) = timing {
            self.fd_table.set_input_timing(fd, timing);
        }
        self.command_scoped_procsub_input_fds.push(fd);
        format!("/dev/fd/{fd}")
    }

    /// rubash#545 (cell 2): close the input-procsub word fds the command
    /// created (every fd recorded at or after `base`). GNU closes a
    /// command's process-substitution fds and reaps its processes when the
    /// command ends (subst.c process_substitute bookkeeping), so a stored
    /// `/dev/fd/N` string names a dead path afterwards.
    pub(in crate::executor) fn close_command_scoped_procsub_input_fds_from(&mut self, base: usize) {
        for fd in self.command_scoped_procsub_input_fds.drain(base..) {
            self.fd_table.entries.remove(&fd);
        }
    }

    /// The word form of a materialized output process substitution
    /// (`>(cmd)`): `/dev/fd/N` naming an fd whose write side feeds the
    /// substitution's temp carrier (the finish path reads the temp by
    /// path and runs the body with it as stdin).
    pub(in crate::executor) fn process_substitution_output_fd_word(
        &mut self,
        temp: &PathBuf,
    ) -> String {
        let mut fd = 63u32;
        while fd > 3 && !self.fd_table_fd_free(fd) {
            fd -= 1;
        }
        if fd <= 3 {
            return shell_display_path(&temp.to_string_lossy());
        }
        self.fd_table.open_output(
            fd,
            FdWriteEndpoint::ProcessSubstitution {
                path: temp.to_path_buf(),
                command: String::new(),
                procsub_pid: None,
            },
            true,
        );
        format!("/dev/fd/{fd}")
    }

    pub(in crate::executor) fn empty_process_substitution_temp(
        &self,
    ) -> Result<PathBuf, ExecuteError> {
        let path = self.process_substitution_temp_path()?;
        File::create(&path)?;
        Ok(path)
    }

    pub(in crate::executor) fn process_substitution_temp_path(
        &self,
    ) -> Result<PathBuf, ExecuteError> {
        self.shell_temp_path("rubash-process-subst", ".tmp")
    }

    /// Unique path in the shell's temp dir for the niubash#362 out-of-band
    /// `-c` transport: a serialized async-child script too large for the OS
    /// command line / a single argv entry. The child removes the file after
    /// reading it (rubash::invocation::materialize_command_file).
    pub(in crate::executor) fn respawn_command_temp_path(&self) -> Result<PathBuf, ExecuteError> {
        self.shell_temp_path("rubash-respawn-cmd", ".sh")
    }

    /// Resolve `$TMPDIR` (or the platform default) into a unique,
    /// Windows-form path in an existing directory. Shared by the process
    /// substitution materialization and the niubash#362 respawn transport so
    /// the TMPDIR resolution and existence handling live in one place.
    ///
    /// Writability probe (not just existence): a present-but-unwritable
    /// directory (Android's /tmp) must fall through to the next candidate.
    /// Probe files carry pid + counter so parallel spawns never share one.
    fn dir_usable_for_scratch(dir: &std::path::Path) -> bool {
        use std::sync::atomic::{AtomicU64, Ordering};
        static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
        let probe = dir.join(format!(
            ".rubash-write-probe-{}-{}",
            std::process::id(),
            PROBE_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let usable = std::fs::write(&probe, b"").is_ok();
        let _ = std::fs::remove_file(&probe);
        usable
    }
    fn shell_temp_path(&self, prefix: &str, suffix: &str) -> Result<PathBuf, ExecuteError> {
        let dir_value = self
            .shell_state
            .env_vars
            .get("TMPDIR")
            .cloned()
            .unwrap_or_else(safe_temp_dir_string);
        let mut dir = shell_path_to_windows(&dir_value, &self.shell_state.env_vars);
        if !Self::dir_usable_for_scratch(&dir) {
            // The configured/platform directory may exist but be
            // unwritable (Android ships an unwritable /tmp) or
            // uncreatable: fall back to the process temp dir instead of
            // failing the transport (whose only fallback is an inline
            // -c string that dies E2BIG past MAX_ARG_STRLEN —
            // background_command_with_large_function_table_runs).
            // std::env::temp_dir honors TMPDIR/TEMP/TMP first, so an
            // explicitly configured value still wins. As a last resort
            // use the shell cwd (uniquely named file, removed after the
            // child reads it) — an airtight test env may leave no temp
            // variable set at all.
            let fallback = std::env::temp_dir();
            if Self::dir_usable_for_scratch(&fallback) {
                dir = fallback;
            } else {
                // G-02: "the shell cwd" (the pre-G-02 code's last-resort
                // process cwd) is the session cwd field.
                dir = self.shell_state.cwd.clone();
            }
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        dir.push(format!("{prefix}-{}-{nanos}{suffix}", std::process::id()));
        Ok(dir)
    }

    pub(in crate::executor) fn write_process_substitution_temp(
        &self,
        output: &str,
    ) -> Result<PathBuf, ExecuteError> {
        self.write_process_substitution_temp_bytes(output.as_bytes())
    }

    /// Fork the rubash#1022 `$!` placeholder: a child that exits at once
    /// with the already-recorded procsub status. Only async-signal-safe
    /// work happens post-fork (`_exit`), so forking a multithreaded shell
    /// is safe — the child never touches locks, allocators, or Rust
    /// destructors (the process ends via `_exit`, not return).
    #[cfg(unix)]
    pub(in crate::executor) fn fork_procsub_placeholder(status: i32) -> Option<u32> {
        // SAFETY: the child calls only `_exit`, which is async-signal-safe;
        // no heap or lock state is consulted between fork and exit.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return None;
        }
        if pid == 0 {
            unsafe { libc::_exit(status) };
        }
        Some(pid as u32)
    }

    /// GNU subst.c:6362 process_substitute forks an asynchronous child for
    /// every process substitution; jobs.c adds that fork to the job list so
    /// `$!` expands to its pid (execute_cmd.c) and `wait $!` reports the
    /// child's exit status (procsub.tests: `cat <(exit 123); wait $!` ->
    /// 123). Rubash evaluates `<(...)` substitutions inline while
    /// materializing the containing command; the already-finished status is
    /// recorded under a forked placeholder pid (rubash#1022).
    ///
    /// The placeholder is a real child that exits at once with the recorded
    /// status: it is born a zombie, so `kill -0` and signal delivery behave
    /// exactly like GNU's just-exited procsub, while the recorded status
    /// (not the placeholder's own, though they agree) stays the source of
    /// truth for `wait` — immune to the fork/exit race. An explicit `wait`
    /// (or the bare `wait`) reaps it; output-form placeholders otherwise
    /// stay signallable across commands the way GNU's pipe-held procsub
    /// children outlive the creating command.
    pub(in crate::executor) fn register_process_substitution_status(&mut self, status: i32) -> u32 {
        const PROCSUB_SEQ_KEY: &str = "RUBASH_INTERNAL_PROCSUB_SEQ";
        let seq = self
            .shell_state
            .env_vars
            .get(PROCSUB_SEQ_KEY)
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        self.shell_state
            .env_vars
            .insert(PROCSUB_SEQ_KEY.to_string(), seq.wrapping_add(1).to_string());
        #[cfg(unix)]
        let pid = match Self::fork_procsub_placeholder(status) {
            Some(pid) => {
                self.shell_state.job_table.procsub_placeholders.insert(pid);
                pid
            }
            None => self.shell_pid.wrapping_add(0x4000_0000).wrapping_add(seq),
        };
        // Without fork (non-unix) or when the fork fails, keep the previous
        // synthetic pid in the dedicated high range that cannot collide with
        // a real OS child pid; it carries the status for `wait` exactly as
        // before, only without signal reachability.
        #[cfg(not(unix))]
        let pid = self.shell_pid.wrapping_add(0x4000_0000).wrapping_add(seq);
        self.shell_state
            .job_table
            .completed_statuses
            .insert(pid, status);
        self.shell_state.last_background_pid = Some(pid);
        pid
    }

    pub(in crate::executor) fn write_process_substitution_temp_bytes(
        &self,
        output: &[u8],
    ) -> Result<PathBuf, ExecuteError> {
        let path = self.process_substitution_temp_path()?;
        fs::write(&path, output)?;
        // subst.c:7143 command_substitute — the `<(cmd)` word names a
        // pipe-like stream: every open shares one offset that drains on
        // read. The temp file is only the argv carrier for external
        // children; in-shell opens must consult the registered stream
        // instead (procsub.tests count_lines: `wc -l < $1` x5 ->
        // 1,0,0,0,0).
        self.shell_state.procsub_streams.borrow_mut().insert(
            path.clone(),
            Rc::new(RefCell::new(crate::shell::state::ProcSubStream {
                data: output.to_vec(),
                offset: 0,
                timing: None,
            })),
        );
        Ok(path)
    }

    /// Serve the remaining bytes of a process-substitution stream opened
    /// through its temp-path carrier, advancing the shared offset to EOF
    /// (GNU: the word is a `/dev/fd/N` pipe dup — redir.c:1183
    /// open_redir_file hands every consumer the same draining stream).
    /// Returns `None` when `path` is not a registered substitution.
    fn procsub_stream_key(&self, path: &std::path::Path) -> Option<PathBuf> {
        let streams = self.shell_state.procsub_streams.borrow();
        if streams.contains_key(path) {
            return Some(path.to_path_buf());
        }
        let canon = path.canonicalize().ok()?;
        streams
            .keys()
            .find(|key| key.canonicalize().ok().as_deref() == Some(canon.as_path()))
            .cloned()
    }

    pub(in crate::executor) fn procsub_stream_take(
        &self,
        path: &std::path::Path,
    ) -> Option<Vec<u8>> {
        self.procsub_stream_take_timed(path)
            .map(|(rest, _timing, _base)| rest)
    }

    /// rubash#1141: `procsub_stream_take` plus the stream's production
    /// schedule and the offset the returned bytes start at, so a timed
    /// consumer can judge the records it serves from the drained bytes.
    pub(in crate::executor) fn procsub_stream_take_timed(
        &self,
        path: &std::path::Path,
    ) -> Option<(Vec<u8>, Option<super::timed_input::SharedTimedInput>, usize)> {
        let key = self.procsub_stream_key(path)?;
        let stream = self.shell_state.procsub_streams.borrow().get(&key)?.clone();
        let mut stream = stream.borrow_mut();
        let base = stream.offset;
        let rest = stream.data[stream.offset..].to_vec();
        stream.offset = stream.data.len();
        Some((rest, stream.timing.clone(), base))
    }

    /// rubash#1141: snapshot of a registered stream (bytes, shared
    /// offset, production schedule) without draining it — the fd-0
    /// binding for a compound `< <(...)` redirect serves the stream
    /// through an fd-table endpoint and writes the consumed offset
    /// back on restore.
    pub(in crate::executor) fn procsub_stream_lookup(
        &self,
        path: &std::path::Path,
    ) -> Option<(Vec<u8>, usize, Option<super::timed_input::SharedTimedInput>)> {
        let key = self.procsub_stream_key(path)?;
        let stream = self.shell_state.procsub_streams.borrow().get(&key)?.clone();
        let stream = stream.borrow();
        Some((stream.data.clone(), stream.offset, stream.timing.clone()))
    }

    /// rubash#1141: write back a stream's shared offset (fd-0 binding
    /// restore — see procsub_stream_lookup).
    pub(in crate::executor) fn procsub_stream_set_offset(
        &self,
        path: &std::path::Path,
        offset: usize,
    ) {
        if let Some(key) = self.procsub_stream_key(path) {
            if let Some(stream) = self.shell_state.procsub_streams.borrow().get(&key) {
                stream.borrow_mut().offset = offset;
            }
        }
    }

    /// rubash#1141: attach a substitution body's production schedule to
    /// its registered stream (the temp file is written before the
    /// schedule is attached — see materialize_process_substitution_word).
    pub(in crate::executor) fn procsub_stream_set_timing(
        &self,
        path: &std::path::Path,
        timing: Option<super::timed_input::SharedTimedInput>,
    ) {
        if let Some(key) = self.procsub_stream_key(path) {
            if let Some(stream) = self.shell_state.procsub_streams.borrow().get(&key) {
                stream.borrow_mut().timing = timing;
            }
        }
    }

    pub(in crate::executor) fn virtual_fd_stdin_remaining_bytes(&self, fd: u32) -> Option<Vec<u8>> {
        match self.fd_table.materialize_for_child().remove(&fd)?.read? {
            MaterializedRead::Bytes(input) => Some(input),
            MaterializedRead::InheritedProcessStdin => None,
            _ => None,
        }
    }

    pub(in crate::executor) fn virtual_fd_stdin_remaining(&self, fd: u32) -> Option<String> {
        self.virtual_fd_stdin_remaining_bytes(fd)
            .map(|bytes| crate::executor::substitution_metadata::bytes_to_shell_text(&bytes))
    }

    fn virtual_fd_stdin_len(&self, fd: u32) -> String {
        if let Some((input, _)) = self.fd_table.input_snapshot_bytes(fd) {
            return input.len().to_string();
        }
        "0".to_string()
    }

    pub(in crate::executor) fn external_fd_heredoc_input(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
    ) -> Option<String> {
        let redirect = cmd
            .heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd == Some(fd))?;
        if redirect.body_carrier.is_none() {
            redirect.body.as_ref()?;
        }
        Some(self.expand_heredoc_redirect_text(redirect))
    }
}

/// stdin for a child whose fd 0 is closed (rubash#1102): GNU leaves the
/// descriptor closed in the forked child, so its reads fail EBADF instead
/// of reporting EOF (coreutils: `cat: -: Bad file descriptor`, exit 1).
/// A write-only /dev/null reproduces that observable contract on Unix —
/// read(2) on a write-only descriptor fails EBADF. Elsewhere fall back to
/// the plain null device (EOF).
pub(in crate::executor) fn closed_stdin() -> Stdio {
    #[cfg(unix)]
    {
        if let Ok(file) = OpenOptions::new().write(true).open("/dev/null") {
            return Stdio::from(file);
        }
    }
    Stdio::null()
}

fn sync_ordered_redirect_targets(command: &mut CommandNode) {
    for redirect in &mut command.redirects {
        if !redirect.target.starts_with(">(") {
            continue;
        }
        let source = if redirect.fd.unwrap_or(1) == 2 {
            command
                .redirect_err_append
                .as_deref()
                .or(command.redirect_err.as_deref())
        } else {
            command
                .redirect_out
                .as_deref()
                .or(command.append.as_deref())
        };
        if let Some(source) = source {
            redirect.target = source.target.clone();
            redirect.target_metadata = source.target_metadata.clone();
        }
    }
}

// The "raw word carries an unquoted process substitution" scan lives
// solely in the parser (crate::parser::
// raw_word_has_unquoted_process_substitution); this file used to carry a
// verbatim copy of the same DFA.
