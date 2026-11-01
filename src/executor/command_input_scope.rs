use super::redirection::{dup_bad_fd_name, dup_source_fd_open};
use super::*;
use crate::executor::fd_table::FdEntry;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};

/// Saved descriptor state for a numbered fd opened by a compound command's
/// input redirection (see open_compound_numbered_input_redirects).
pub(in crate::executor) struct SavedNumberedFd {
    fd: u32,
    entry: Option<FdEntry>,
    fd_stdin: Option<String>,
    fd_stdin_offset: Option<String>,
    fd_dynamic: Option<String>,
    fd_closed: Option<String>,
}

impl SavedNumberedFd {
    pub(in crate::executor) fn fd(&self) -> u32 {
        self.fd
    }

    /// The pre-command fd-table entry this snapshot restores.
    pub(in crate::executor) fn saved_entry(&self) -> Option<&FdEntry> {
        self.entry.as_ref()
    }

    /// The pre-command fd-closed marker this snapshot restores.
    pub(in crate::executor) fn saved_fd_closed(&self) -> Option<&String> {
        self.fd_closed.as_ref()
    }
}

/// Saved fd-0 state for a command-scoped input binding: the fd-table slot
/// snapshot plus the FUNCTION_STDIN text-channel pair it may have replaced
/// (see bind_command_fd0_input).
pub(in crate::executor) struct CommandFd0InputGuard {
    saved_fd0_entry: Option<FdEntry>,
    saved_function_stdin: Option<String>,
    saved_function_stdin_offset: Option<String>,
    /// rubash#1141: production schedule of the FUNCTION_STDIN being
    /// shadowed (the rebound text is heredoc/herestring/loop input with
    /// no production timing of its own).
    saved_timed_stdin: Option<super::timed_input::SharedTimedInput>,
    /// rubash#1902: the pipeline-stage fd-0 pipe fact in force when the
    /// bind began; the mirror branch below may clear it (the rebound
    /// mirror supersedes the inter-stage pipe), every other branch
    /// leaves it, and restore puts the entry value back verbatim.
    saved_stage_pipe_fact: Option<String>,
    /// rubash#2131: the FD0_MIRROR_FACT in force when the bind began;
    /// the mirror branch below may replace it with the staged source's
    /// own fact, every other branch leaves it, and restore puts the
    /// entry value back verbatim.
    saved_mirror_fact: Option<String>,
    /// rubash#1141: when fd 0 was bound to a process-substitution
    /// stream's endpoint, the stream's registered path — restore writes
    /// the endpoint's consumed offset back into the stream.
    procsub_stream_path: Option<std::path::PathBuf>,
}

impl SavedOutputFd {
    pub(in crate::executor) fn fd(&self) -> u32 {
        self.fd
    }

    /// The pre-command fd-table entry this snapshot restores.
    pub(in crate::executor) fn saved_entry(&self) -> Option<&FdEntry> {
        self.entry.as_ref()
    }

    /// The pre-command fd-closed marker this snapshot restores.
    pub(in crate::executor) fn saved_fd_closed(&self) -> Option<&String> {
        self.fd_closed.as_ref()
    }
}

/// Saved descriptor state for an fd rebound by a compound command's
/// output redirection (see open_compound_output_redirects).
pub(in crate::executor) struct SavedOutputFd {
    fd: u32,
    entry: Option<FdEntry>,
    /// dup2 snapshot record (FdTable::stdout_alias_generation) that went
    /// with the saved entry; restored together so an inner scope's
    /// `2>/dev/null` cannot drop a `2>&1` binding's generation
    /// (rubash#223).
    alias_generation: Option<Option<usize>>,
    fd_output: Option<String>,
    fd_procsub: Option<String>,
    fd_closed: Option<String>,
}

/// rubash#2309: the first construct in an unquoted heredoc body whose
/// expansion GNU's expansion layer cannot complete, classified by the
/// layer GNU assigns the failure to. A heredoc body is collected by the
/// delimiter scan and only then expanded as a word, so a pending
/// construct in the body is diagnosed by the EXPANSION layer: a
/// backquote that never closes fails its extraction in
/// expand_word_internal (subst.c:11587), and anything unterminated
/// inside a `$(` — its closer, or a quote within — fails the parse of
/// the extracted body (subst.c:1262 extract_command_subst hands the
/// body to parse_comsub). Constructs the expansion completes, and `$'`
/// text (literal in a heredoc body — quote processing does not run
/// there), are not failures.
enum HeredocBodyFailure {
    /// Unterminated backquote: `bad substitution: no closing "`" in …`
    /// carrying the text from the opening backquote to the end of the
    /// body (subst.c:11587 reports string+t_index, the rest of the word).
    Backquote { text: String },
    /// Unterminated `$(`: parse_comsub EOF naming the innermost pending
    /// construct's closer at the given line.
    Comsub { close: char, line: usize },
}

/// End (byte length) of the backquote span opening at the start of
/// `text`, or None if it never closes. GNU delimits a backquote body
/// atomically (subst.c string_extract, SX_REQMATCH): a backslash quotes
/// the following character, so \` and \\ never close the span.
fn heredoc_backquote_span_end(text: &str) -> Option<usize> {
    let mut escaped = false;
    for (index, ch) in text.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '`' => return Some(index + 1),
            _ => {}
        }
    }
    None
}

/// End (byte length) of the `$(` span opening at the start of `text`,
/// by the comsub collector's own discipline, or None if it never
/// closes.
fn heredoc_comsub_span_end(
    text: &str,
    aliases: &std::collections::HashMap<String, crate::builtins::alias::Alias>,
) -> Option<usize> {
    let mut iter = text[2..].chars().peekable();
    let (_, closed) =
        super::embedded_mutations::collect_command_substitution_source_ex(&mut iter, aliases);
    if closed {
        Some(2 + (text[2..].len() - iter.clone().map(|c| c.len_utf8()).sum::<usize>()))
    } else {
        None
    }
}

/// End (byte length) of the double-quoted span opening at the start of
/// `text` (inside a `${...}` body, where quotes are real), or None if
/// it never closes. Nested substitutions are skipped as spans; a `}`
/// inside the quotes is literal text and cannot close the parameter.
fn heredoc_double_quote_span_end(
    text: &str,
    aliases: &std::collections::HashMap<String, crate::builtins::alias::Alias>,
) -> Option<usize> {
    let mut pos = 1usize;
    while pos < text.len() {
        let rest = &text[pos..];
        let ch = rest.chars().next().expect("pos stays on a char boundary");
        match ch {
            '\\' => {
                pos += 1 + rest[1..].chars().next().map_or(0, |next| next.len_utf8());
            }
            '"' => return Some(pos + 1),
            '`' => pos += heredoc_backquote_span_end(rest)?,
            '$' if rest[1..].starts_with('(') => pos += heredoc_comsub_span_end(rest, aliases)?,
            '$' if rest[1..].starts_with('{') => pos += heredoc_parameter_span_end(rest, aliases)?,
            _ => pos += ch.len_utf8(),
        }
    }
    None
}

/// End (byte length) of the `${` span opening at the start of `text`,
/// or None if it never closes. Quotes and nested substitutions inside
/// the parameter text are skipped as spans so their bytes cannot close
/// the brace early (subst.c string_extract / parse_matched_pair walk
/// the same units).
fn heredoc_parameter_span_end(
    text: &str,
    aliases: &std::collections::HashMap<String, crate::builtins::alias::Alias>,
) -> Option<usize> {
    let mut depth = 1usize;
    let mut pos = 2usize;
    while pos < text.len() {
        let rest = &text[pos..];
        let ch = rest.chars().next().expect("pos stays on a char boundary");
        match ch {
            '\\' => {
                pos += 1 + rest[1..].chars().next().map_or(0, |next| next.len_utf8());
            }
            // Inside `${...}` a single quote is a real quote whose
            // content is literal (a backslash inside it quotes nothing).
            '\'' => match rest[1..].find('\'') {
                Some(offset) => pos += 1 + offset + 1,
                None => return None,
            },
            '"' => pos += heredoc_double_quote_span_end(rest, aliases)?,
            '`' => pos += heredoc_backquote_span_end(rest)?,
            '$' if rest[1..].starts_with('(') => pos += heredoc_comsub_span_end(rest, aliases)?,
            '$' if rest[1..].starts_with('{') => {
                depth += 1;
                pos += 2;
            }
            '}' => {
                depth -= 1;
                pos += 1;
                if depth == 0 {
                    return Some(pos);
                }
            }
            _ => pos += ch.len_utf8(),
        }
    }
    None
}

impl Executor {
    pub(crate) fn with_command_input_redirects<T: Default>(
        &mut self,
        cmd: &CommandNode,
        execute: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        // GNU redir.c do_redirection_internal (redir.c:767-955) applies a
        // compound command's redirections before the command runs, and
        // execute_cmd.c undoes them when the command finishes (RX_UNDOABLE /
        // redir.c:949-955). A numbered input redirection such as
        // `while read -ru3 x; do :; done 3< <(echo x)` (redir10.sub,
        // procsub.tests bug()) must therefore keep fd 3 open for the whole
        // compound command - condition and body alike - and restore the
        // previous descriptor state afterwards.
        // RESTRICTED_SHELL (#804 N1): this binder reaches fd-var entries
        // through execute_dynamic_fd_var_redirect DIRECTLY and opens the
        // compound's output targets below, so the first restricted-write
        // entry refuses here — before anything opens or binds — and the
        // body never runs (status 1, GNU's in-child do_redirections
        // refusal).
        if self.reject_restricted_write_redirect_entry(cmd)? {
            let default_result = T::default();
            return Ok(default_result);
        }
        let dynamic_names = cmd
            .redirects
            .iter()
            .filter_map(|redirect| redirect.fd_var.clone())
            .collect::<Vec<_>>();
        // rubash#2282: running set of the numbered slots preceding each
        // entry in the list — a {var} allocation counts them as occupied
        // (see apply_dynamic_fd_var_redirects).
        let mut occupied_slots: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for redirect in &cmd.redirects {
            if let Some(fd) = redirect.fd {
                occupied_slots.insert(fd);
            }
            if redirect.fd_var.is_some() {
                let _ = self.execute_dynamic_fd_var_redirect(
                    redirect,
                    false,
                    false,
                    &occupied_slots,
                    &std::collections::HashSet::new(),
                    0,
                )?;
            }
        }
        // The dup preflight runs before ANY bind so the abandonment it
        // triggers leaves no half-applied descriptor state, and its
        // left-to-right walk reports the FIRST failing dup in list order
        // like GNU do_redirections (output side rubash#263/#266, input
        // side rubash#2030).
        if let Some((index, name)) = self.compound_dup_bad_fd(cmd) {
            return self.abandon_compound_for_dup_bad_fd(cmd, index, &name);
        }
        let saved_numbered = self.open_compound_numbered_input_redirects(cmd)?;
        let saved_output = self.open_compound_output_redirects(cmd)?;
        let result = self.with_command_input_redirects_inner(cmd, execute);
        self.restore_compound_output_redirects(saved_output);
        self.restore_compound_numbered_input_redirects(saved_numbered);
        for name in dynamic_names {
            self.close_dynamic_fd(&name)?;
        }
        result
    }

    fn with_command_input_redirects_inner<T: Default>(
        &mut self,
        cmd: &CommandNode,
        execute: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        // GNU redir.c do_redirection_internal (redir.c:767-955): a compound
        // command's fd-0 input redirection is opened BEFORE the command
        // runs; an open failure reports the diagnostic and terminates the
        // whole compound with the redirection's status (1) without
        // executing it — `while read -r l; do :; done < noexist` prints
        // "noexist: No such file or directory" and leaves $? = 1
        // (rubash#137; rubash used to run the loop against inherited stdin
        // silently and report 0).
        if let Some(message) = self.compound_stdin_open_failure(cmd) {
            let mut stderr = Vec::new();
            writeln!(&mut stderr, "{message}")?;
            self.write_redirected_command_stderr(cmd, &stderr)?;
            self.exit_code = 1;
            // The compound command is terminated without running: its
            // result is the default (unit for loop/group bodies, None for
            // the source-test `if` scanner, which then consumes the frame).
            return Ok(T::default());
        }
        // GNU do_redirection_internal opens `< file` as a REAL descriptor
        // on fd 0: the shell reads it record-wise off the shared file
        // offset (read.def zreadc + zsyncfd), and a child spawned in the
        // body inherits a dup of the same open file description — a child
        // that never reads leaves the parent's cursor untouched, so
        // `while read -r l; do expr ...; done < f` iterates over every
        // line (rubash#260: the FUNCTION_STDIN text mirror eagerly fed the
        // whole remaining buffer to every child and marked it consumed,
        // ending the loop after one iteration). For a stream source
        // (regular file, FIFO, device — compound_fd0_stream_source) the
        // fd table already has the whole GNU mechanism — byte-accurate
        // record reads (FdTable::read_file_bytes) and DuplicateHandle-based
        // child inheritance — so bind fd 0 to the handle instead of
        // slurping the text mirror. Text-production sources (procsubs,
        // heredocs, here-strings, /dev-aliases) keep the mirror below.
        let guard = match self.bind_command_fd0_input(cmd) {
            Ok(guard) => guard,
            Err(error) => {
                // rubash#2124: the bind's net arm reports a refused
                // /dev/tcp|udp connect as an io error — a compound
                // abandoned by a failed fd-0 open takes the same
                // shape as the open-failure channel above (diagnostic
                // through the command's fd-2 routing, status 1, body
                // never runs; GNU redir.c redirection_error). Other
                // bind errors keep their propagation.
                let net_fd0 = cmd.redirect_in.as_deref().is_some_and(|redirect| {
                    redirect.fd.unwrap_or(0) == 0
                        && crate::special_paths::is_net_redirect_target(
                            &self.expand_redirect_target(redirect),
                        )
                });
                if !net_fd0 {
                    return Err(error);
                }
                let message = match &error {
                    ExecuteError::IoError(io_error) => crate::posix_errors::message(io_error),
                    other => other.to_string(),
                };
                let mut stderr = Vec::new();
                let _ = writeln!(&mut stderr, "{}{}", self.error_prefix(), message.trim_end());
                self.write_redirected_command_stderr(cmd, &stderr)?;
                self.exit_code = 1;
                return Ok(T::default());
            }
        };
        let Some(guard) = guard else {
            return execute(self);
        };
        let result = execute(self);
        self.restore_command_fd0_input(guard);
        result
    }

    /// The binding half of with_command_input_redirects_inner: a
    /// stream-source `< file` (regular file, FIFO, character/block
    /// device) opens a real FileFd on fd slot 0 (GNU's shared-offset
    /// descriptor, redir.c do_redirection_internal), any other input
    /// source — here-string, heredoc, procsub text, /dev/fd alias —
    /// stages its expanded text on FUNCTION_STDIN with the slot
    /// snapshotted. Returns None when the command carries no fd-0 input
    /// binding at all (nothing to restore). Used by the compound wrapper
    /// and by the exec builtin, whose spawn must see the node's input
    /// redirections already dup2'd onto fd 0 (GNU redir.c applies them
    /// before execve — rubash#575).
    pub(in crate::executor) fn bind_command_fd0_input(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<CommandFd0InputGuard>, ExecuteError> {
        // rubash#1902: snapshot the stage-pipe fact before any branch
        // can touch it; the guard restores it verbatim (the mirror
        // branch is the one branch that may clear it, below).
        let saved_stage_pipe_fact = self.shell_state.env_vars.get(FD0_STAGE_PIPE).cloned();
        // rubash#2131: same snapshot discipline for the mirror fact;
        // the mirror branch is the one branch that may replace it.
        let saved_mirror_fact = self.shell_state.env_vars.get(FD0_MIRROR_FACT).cloned();
        // rubash#1141: compound commands (brace group, select, loops)
        // dispatch BEFORE the simple-command materialization gate, so
        // their `< <(...)` redirect_in can still carry the raw
        // substitution form here and the registered-stream branch below
        // never sees it — the node fell through to command_input_redirect,
        // whose loop_redirect_input slurps the body UNTIMED into
        // FUNCTION_STDIN and clears timed_stdin, so TMOUT could never
        // fire for the compound's reads/selects. Materialize the body
        // with its production schedule (the same timed variant the
        // prepare path runs for simple commands) and bind fd 0 to the
        // timed endpoint directly; the endpoint is private to this
        // compound scope, so no stream registration or offset write-back
        // is needed. A body that fails to materialize falls through to
        // the existing paths unchanged.
        // NOTE (rubash#1931 residual, resolved by rubash#2024): this arm —
        // like the fd walk in stdin_string_for_command and
        // loop_redirect_input — used to match the target's quote-stripped
        // value, so a QUOTED "<(...)" compound input target was spawned
        // here even though GNU treats it as a literal filename. Admission
        // is now the same recorded-span check the simple-command path runs
        // (external_setup's redirect_target_span_is_recorded): the spawn
        // happens only for a span the parser saw unquoted. A quoted form
        // never reaches the arm through the compound wrapper — the
        // literal-target error channel (compound_stdin_open_failure)
        // reports `<(echo hi): No such file or directory` and abandons the
        // compound before the bind (GNU redir.c:916 redir_open →
        // redirection_error, compound terminated without running) — the
        // gate keeps the arm itself from materializing a literal by value
        // on the faces that reach it with the channel not yet run.
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0
                && redirect.fd_var.is_none()
                && redirect.kind == crate::parser::RedirectKind::Input
                && super::external_setup::redirect_target_span_is_recorded(redirect)
            {
                if let Some(source) = redirect
                    .target
                    .strip_prefix("<(")
                    .and_then(|target| target.strip_suffix(')'))
                    .map(str::to_string)
                {
                    // rubash#1952: stream first — GNU process_substitute
                    // (subst.c:6362) binds the forked body's pipe, so
                    // the compound's reads observe real arrival times
                    // and a non-terminating producer no longer starves
                    // the compound forever. The timed collection below
                    // is the spawn-failure fallback (its recorded
                    // schedule served #1141 before a live pipe could).
                    if let Some(file_fd) =
                        self.spawn_input_process_substitution_child(&source, cmd.line)
                    {
                        let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
                        let saved_function_stdin =
                            self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
                        let saved_function_stdin_offset = self
                            .shell_state
                            .env_vars
                            .get(FUNCTION_STDIN_OFFSET)
                            .cloned();
                        self.fd_table
                            .open_input(0, FdReadEndpoint::File(file_fd), false);
                        return Ok(Some(CommandFd0InputGuard {
                            saved_fd0_entry,
                            saved_function_stdin,
                            saved_function_stdin_offset,
                            // The text channel is untouched in this
                            // branch, so its timing stays valid; restore
                            // writes the same value back.
                            saved_timed_stdin: self.timed_stdin.clone(),
                            saved_stage_pipe_fact: saved_stage_pipe_fact.clone(),
                            saved_mirror_fact: saved_mirror_fact.clone(),
                            procsub_stream_path: None,
                        }));
                    }
                    if let Some((output, body_timing)) =
                        self.process_substitution_output_bytes_timed(&source)
                    {
                        let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
                        let saved_function_stdin =
                            self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
                        let saved_function_stdin_offset = self
                            .shell_state
                            .env_vars
                            .get(FUNCTION_STDIN_OFFSET)
                            .cloned();
                        self.fd_table.open_input(
                            0,
                            FdReadEndpoint::process_substitution_bytes(output),
                            false,
                        );
                        self.fd_table.set_input_offset(0, 0);
                        if let Some(timing) = body_timing {
                            self.fd_table.set_input_timing(0, timing);
                        }
                        return Ok(Some(CommandFd0InputGuard {
                            saved_fd0_entry,
                            saved_function_stdin,
                            saved_function_stdin_offset,
                            // The text channel is untouched in this
                            // branch, so its timing stays valid; restore
                            // writes the same value back.
                            saved_timed_stdin: self.timed_stdin.clone(),
                            saved_stage_pipe_fact: saved_stage_pipe_fact.clone(),
                            saved_mirror_fact: saved_mirror_fact.clone(),
                            procsub_stream_path: None,
                        }));
                    }
                }
            }
        }
        // rubash#1141: a `< <(...)` redirect whose target materialized to
        // a registered process-substitution stream binds fd 0 to the
        // stream's fd-table endpoint — not to a FileFd on the temp
        // carrier — so the body's production schedule judges the
        // compound's reads/selects (GNU binds the pipe itself). The
        // endpoint starts at the stream's shared offset; restore writes
        // the consumed offset back.
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0
                && redirect.fd_var.is_none()
                && redirect.kind == crate::parser::RedirectKind::Input
                && !redirect.target.starts_with("<(")
            {
                let target = self.expand_redirect_target(redirect);
                let path = self.session_path(super::path::windows_operand_file_path(
                    super::path::shell_path_to_windows(&target, &self.shell_state.env_vars),
                    &self.shell_state.env_vars,
                ));
                if let Some((data, offset, timing)) = self.procsub_stream_lookup(&path) {
                    let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
                    let saved_function_stdin =
                        self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
                    let saved_function_stdin_offset = self
                        .shell_state
                        .env_vars
                        .get(FUNCTION_STDIN_OFFSET)
                        .cloned();
                    self.fd_table.open_input(
                        0,
                        FdReadEndpoint::process_substitution_bytes(data),
                        false,
                    );
                    self.fd_table.set_input_offset(0, offset);
                    if let Some(timing) = timing {
                        self.fd_table.set_input_timing(0, timing);
                    }
                    return Ok(Some(CommandFd0InputGuard {
                        saved_fd0_entry,
                        saved_function_stdin,
                        saved_function_stdin_offset,
                        saved_timed_stdin: self.timed_stdin.clone(),
                        saved_stage_pipe_fact: saved_stage_pipe_fact.clone(),
                        saved_mirror_fact: saved_mirror_fact.clone(),
                        procsub_stream_path: Some(path),
                    }));
                }
            }
        }
        // rubash#2124: a winning /dev/tcp|udp fd-0 input on a compound
        // never reached a bind: compound_fd0_stream_file classifies
        // the target by filesystem metadata (a socket target is not a
        // regular file, so it returns None) and the text fallback
        // staged an empty string. GNU redir.c do_redirections opens
        // it — redir_open -> netopen connects — before the body runs.
        // Connect once here and bind the socket on fd 0 for the
        // body's duration; a failed connect returns the path-shaped
        // io error open_net_redirect already carries (the connect
        // pre-line was emitted by connect_net_stream).
        if let Some(redirect) = &cmd.redirect_in {
            if redirect.fd.unwrap_or(0) == 0
                && redirect.fd_var.is_none()
                && matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite
                )
            {
                let target = self.expand_redirect_target(redirect);
                if crate::special_paths::is_net_redirect_target(&target) {
                    let file = self.open_net_redirect(&target)?;
                    let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
                    let saved_function_stdin =
                        self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
                    let saved_function_stdin_offset = self
                        .shell_state
                        .env_vars
                        .get(FUNCTION_STDIN_OFFSET)
                        .cloned();
                    let file_fd = FileFd::from_file(file, target.into());
                    self.fd_table
                        .open_input(0, FdReadEndpoint::File(file_fd), false);
                    return Ok(Some(CommandFd0InputGuard {
                        saved_fd0_entry,
                        saved_function_stdin,
                        saved_function_stdin_offset,
                        saved_timed_stdin: self.timed_stdin.clone(),
                        // This branch (from the #2124 merge) does not touch
                        // FD0_STAGE_PIPE, so the guard restores exactly the
                        // fact it found — same shape as the compound-stream
                        // branch below.
                        saved_stage_pipe_fact: saved_stage_pipe_fact.clone(),
                        saved_mirror_fact: saved_mirror_fact.clone(),
                        procsub_stream_path: None,
                    }));
                }
            }
        }
        if let Some(file) = self.compound_fd0_stream_file(cmd) {
            // Snapshot the FUNCTION_STDIN pair too: restore_command_fd0_input
            // is unconditional, and this branch never rewrites the text
            // channel — the guard must put back exactly what it found.
            let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
            let saved_function_stdin = self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
            let saved_function_stdin_offset = self
                .shell_state
                .env_vars
                .get(FUNCTION_STDIN_OFFSET)
                .cloned();
            self.fd_table
                .open_input(0, FdReadEndpoint::File(file), false);
            return Ok(Some(CommandFd0InputGuard {
                saved_fd0_entry,
                saved_function_stdin,
                saved_function_stdin_offset,
                // The text channel is untouched in this branch, so its
                // timing stays valid; restore writes the same value back.
                saved_timed_stdin: self.timed_stdin.clone(),
                saved_stage_pipe_fact: saved_stage_pipe_fact.clone(),
                saved_mirror_fact: saved_mirror_fact.clone(),
                procsub_stream_path: None,
            }));
        }
        let Some(input) = self.command_input_redirect(cmd) else {
            return Ok(None);
        };
        let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
        let saved_function_stdin = self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
        let saved_function_stdin_offset = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .cloned();
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input);
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());
        // rubash#1141: the rebound FUNCTION_STDIN is locally produced
        // text (heredoc / here-string / loop input) — the enclosing
        // stream's production schedule does not describe it.
        let saved_timed_stdin = self.timed_stdin.take();
        // rubash#1902: the rebound mirror supersedes whatever fd 0 the
        // enclosing scope had — including a pipeline stage's pipe, whose
        // fact (FD0_STAGE_PIPE) the stage entry installed. The fact
        // survives only when this binding's own source is a pipe in GNU
        // (heredoc / here-string / process substitution); a file or
        // device text source clears it for the scope's duration, and
        // the guard restores the stage's fact at scope end. GNU's
        // ordering is do_piping first, then the command's own
        // redirections (execute_cmd.c:2620, redir.c
        // do_redirection_internal).
        if !super::shell_options::fd0_mirror_binding_keeps_pipe_fact(cmd) {
            self.shell_state.env_vars.remove(FD0_STAGE_PIPE);
        }
        // rubash#2131: the mirror installs no fd-table entry, so the
        // staged source's own fd-0 fact travels beside it in
        // FD0_MIRROR_FACT for the fd-alias sync (the evaluator's
        // /dev/stdin predicates must answer from the rebound source,
        // not the ambient binding). A node with no fd-0 winner of its
        // own staged the enclosing channel's remainder — the key is
        // left as the enclosing scope set it.
        if let Some(fact) = self.fd0_mirror_fact_value(cmd) {
            self.shell_state
                .env_vars
                .insert(FD0_MIRROR_FACT.to_string(), fact);
        }
        // rubash#2170: the staged mirror IS fd 0's binding for the
        // scope — GNU do_redirection_internal (redir.c:767-955)
        // installs the command's own redirection on fd 0, superseding
        // whatever the enclosing scope bound there. Every descriptor
        // branch of this bind replaces the fd-table entry; the mirror
        // branch left it in place, so a persistent File binding
        // (`exec 0<file`) kept governing the body's readers — the
        // builtin read's fd-table arm, the external-child stdin
        // wiring and the fd-alias fact sync all answered from the
        // superseded file while the staged channel claimed the
        // mirror's source. Displace the entry for the scope's
        // duration: the guard snapshotted it above and restores it
        // verbatim at scope end, and with the slot empty every
        // consumer sees the state the mirror's design always assumed
        // (the #2131 fact pass already treats an absent slot as
        // mirror-governed).
        self.fd_table.entries.remove(&0);
        Ok(Some(CommandFd0InputGuard {
            saved_fd0_entry,
            saved_function_stdin,
            saved_function_stdin_offset,
            saved_timed_stdin,
            saved_stage_pipe_fact,
            saved_mirror_fact,
            procsub_stream_path: None,
        }))
    }

    /// The unwind half of bind_command_fd0_input. GNU redir.c
    /// do_redirection_internal (RX_UNDOABLE) restores fd 0 at scope end —
    /// even undoing a permanent `exec 0<g` inside the body.
    pub(in crate::executor) fn restore_command_fd0_input(&mut self, guard: CommandFd0InputGuard) {
        // rubash#1141: fold the fd-0 endpoint's consumption back into
        // the process-substitution stream's shared offset before the
        // descriptor binding is unwound.
        if let Some(path) = &guard.procsub_stream_path {
            if let Some(offset) = self.fd_table.input_offset(0) {
                self.procsub_stream_set_offset(path, offset);
            }
        }
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FUNCTION_STDIN,
            guard.saved_function_stdin,
        );
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FD0_STAGE_PIPE,
            guard.saved_stage_pipe_fact,
        );
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FD0_MIRROR_FACT,
            guard.saved_mirror_fact,
        );
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FUNCTION_STDIN_OFFSET,
            guard.saved_function_stdin_offset,
        );
        self.timed_stdin = guard.saved_timed_stdin;
        match guard.saved_fd0_entry {
            Some(entry) => {
                self.fd_table.entries.insert(0, entry);
            }
            None => {
                self.fd_table.entries.remove(&0);
            }
        }
    }

    /// Opens the numbered input redirections of a compound command in the
    /// fd table for the command's whole duration. GNU applies compound
    /// redirections left to right before the command runs (redir.c
    /// do_redirection_internal:767); procsub targets feed
    /// FdReadEndpoint-backed text exactly like the persistent fd-var path
    /// (trap_exec.rs execute_dynamic_fd_var_redirect).
    pub(in crate::executor) fn open_compound_numbered_input_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Vec<SavedNumberedFd>, ExecuteError> {
        let mut saved: Vec<SavedNumberedFd> = Vec::new();
        self.open_compound_numbered_input_redirects_in(cmd, 0, cmd.redirects.len(), &mut saved)?;
        Ok(saved)
    }

    /// The numbered-input binding loop over the redirect range
    /// `[start, end)`, accumulating saved slots into `saved` (the
    /// first snapshot of a slot wins, so segmented calls restore the
    /// pre-command entry exactly like one whole-list call).
    /// rubash#2310: the same-exe script face binds its inputs in
    /// segments split at the command's `{var}` entries so each such
    /// entry's effect can be applied at its own list position; every
    /// other caller binds the whole list in one call, unchanged.
    pub(in crate::executor) fn open_compound_numbered_input_redirects_in(
        &mut self,
        cmd: &CommandNode,
        start: usize,
        end: usize,
        saved: &mut Vec<SavedNumberedFd>,
    ) -> Result<(), ExecuteError> {
        for redirect in &cmd.redirects[start..end] {
            // The fd before the operator defaults like GNU: input-side
            // redirects without an explicit number target fd 0
            // (`{ list; } <&3` dups fd 3 onto the group's stdin).
            let fd = redirect.fd.unwrap_or(0);
            if redirect.fd_var.is_some() {
                continue;
            }
            if !matches!(
                redirect.kind,
                crate::parser::RedirectKind::Input
                    | crate::parser::RedirectKind::ReadWrite
                    | crate::parser::RedirectKind::DuplicateInput
                    | crate::parser::RedirectKind::CloseInput
            ) {
                continue;
            }
            // fd 0 `<file` stays on the FUNCTION_STDIN text channel for
            // external-children compatibility, but `0<&N`/`0<&-` are real
            // descriptor operations — dup_entry gives the group's fd 0 the
            // source's open file description (shared offset) for both
            // builtins and spawned children.
            if fd == 0
                && !matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::DuplicateInput
                        | crate::parser::RedirectKind::CloseInput
                )
            {
                continue;
            }
            let target = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: the diagnostic already printed
            // at the expansion; abort the compound before any fd binding.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
            if !saved.iter().any(|saved| saved.fd == fd) {
                saved.push(SavedNumberedFd {
                    fd,
                    entry: self.fd_table.entries.get(&fd).cloned(),
                    fd_stdin: self.shell_state.env_vars.get(&fd_stdin_key(fd)).cloned(),
                    fd_stdin_offset: self
                        .shell_state
                        .env_vars
                        .get(&fd_stdin_offset_key(fd))
                        .cloned(),
                    fd_dynamic: self
                        .shell_state
                        .env_vars
                        .get(&fd_dynamic_input_key(fd))
                        .cloned(),
                    fd_closed: self.shell_state.env_vars.get(&fd_closed_key(fd)).cloned(),
                });
            }
            match redirect.kind {
                // `{ cmd; } N<&-` / `N>&-` — scoped close (redir.c
                // do_redirection_internal r_close_input).
                crate::parser::RedirectKind::CloseInput => {
                    self.fd_table.close_input(fd);
                    self.shell_state
                        .env_vars
                        .insert(fd_closed_key(fd), "1".to_string());
                    continue;
                }
                // `{ cmd; } N<&M` — dup2 the source descriptor into the
                // scoped slot (e.g. `4<&0` saves stdin for the group).
                crate::parser::RedirectKind::DuplicateInput => {
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        let _ = self.fd_table.dup_input(fd, source_fd);
                        self.shell_state.env_vars.remove(&fd_closed_key(fd));
                        // Self-move: GNU's `redir_fd != redirector`
                        // guard makes it a no-op (rubash#1236).
                        if move_source && source_fd != fd {
                            self.fd_table.close(source_fd);
                        }
                    }
                    continue;
                }
                _ => {}
            }
            if is_closed_redirect_target(&target) {
                continue;
            }
            if is_null_device(&target) {
                self.set_fd_input_bytes(fd, Vec::new(), true);
                if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                    self.set_fd_readwrite_file(fd, &target, true)
                        .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                }
                continue;
            }
            if let Some(source) = target
                .strip_prefix("<(")
                .and_then(|target| target.strip_suffix(')'))
            {
                // rubash#1952: stream first — GNU process_substitute
                // (subst.c:6362) binds the forked body's pipe read end
                // on the target fd; the collected form below is the
                // spawn-failure fallback.
                if let Some(file_fd) = self.spawn_input_process_substitution_child(source, cmd.line)
                {
                    self.set_fd_input_file(fd, file_fd, true);
                    if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                        self.set_fd_readwrite_file(fd, &target, true)
                            .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                    }
                    continue;
                }
                let Some(output) = self.process_substitution_output(source) else {
                    continue;
                };
                self.set_fd_input_bytes(
                    fd,
                    crate::executor::substitution_metadata::shell_text_to_raw_bytes(&output),
                    true,
                );
                if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                    self.set_fd_readwrite_file(fd, &target, true)
                        .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                }
                continue;
            }
            // rubash#2235: an fd-alias target (/dev/fd/N,
            // /proc/self/fd/N, /dev/std*) names the fd table's
            // current endpoint at N — GNU resolves the alias through
            // the descriptor layer (the rubash#1963 alias service).
            // Bind from that endpoint, never by opening the spelling
            // as a filesystem path: the process's real fd N is an
            // unrelated descriptor, so the plain open below reports
            // a spurious ENOENT (or worse, opens an unrelated file).
            // Which bind the endpoint gets — fresh open or dup — is
            // the endpoint-kind dispatch at the arm itself (rubash#2256).
            // A list-level input procsub materialized for the
            // external composition rewrites its entry to exactly
            // this spelling, so this arm is what lets the in-process
            // binders consume the same materialized node. An alias
            // whose endpoint is absent falls through: the plain
            // open's ENOENT is GNU's dangling-alias error.
            if let Some(alias_fd) = super::execution_misc::dev_stdio_redirect_fd(&target) {
                if self.fd_table.read_endpoint(alias_fd).is_some() {
                    // rubash#2276: a socket endpoint's spelling-open
                    // fails ENXIO (see fd_alias_spelling_open_error) —
                    // the dup fallback below is the pipe rule, not the
                    // socket rule.
                    if let Some(error) = self.fd_alias_spelling_open_error(alias_fd) {
                        return Err(crate::posix_errors::path_error(&target, error).into());
                    }
                    // rubash#2256: the alias spelling is an open, not a
                    // dup instruction — a regular-file endpoint binds a
                    // FRESH open of its target (new description, offset
                    // 0; the source fd does not move), the ReadWrite
                    // form re-binds through set_fd_readwrite_file whose
                    // own alias arm owns the same dispatch for the pair,
                    // and every other endpoint kind binds by dup. A
                    // fresh open that fails falls back to the dup (an
                    // unlinked-but-open target stays readable through
                    // the descriptor, as GNU's magic-link open does).
                    if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                        self.set_fd_readwrite_file(fd, &target, true)
                            .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                    } else if let Some(path) = self.fd_alias_reopen_path(alias_fd) {
                        match FileFd::open_read(path) {
                            Ok(fresh) => self.set_fd_input_file(fd, fresh, true),
                            Err(_) => {
                                let _ = self.fd_table.dup_input(fd, alias_fd);
                            }
                        }
                    } else {
                        let _ = self.fd_table.dup_input(fd, alias_fd);
                    }
                    continue;
                }
            }
            // Real handle on the slot: [N]<> is a single O_RDWR open file
            // description (redir.c r_input_output), otherwise O_RDONLY.
            if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                self.set_fd_readwrite_file(fd, &target, true)
                    .map_err(|e| crate::posix_errors::path_error(&target, e))?;
            } else if crate::special_paths::is_zero_device(&target)
                || crate::special_paths::is_full_device(&target)
            {
                // rubash#1898: the zero/full devices read as an
                // unbounded zero stream — bind the pump-fed source.
                let file = Self::zero_device_file(&target)
                    .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                let file_fd = FileFd::from_file(file, target.clone().into());
                self.set_fd_input_file(fd, file_fd, true);
            } else if crate::special_paths::is_random_device(&target) {
                // niubash#758: the entropy devices read as an
                // unbounded CSPRNG stream — bind the pump-fed
                // random source.
                let file = Self::random_device_file(&target)
                    .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                let file_fd = FileFd::from_file(file, target.clone().into());
                self.set_fd_input_file(fd, file_fd, true);
            } else if let Some(bytes) = crate::special_paths::synthetic_proc_content(&target) {
                // Registry first (rubash#1917, same ladder-skip family
                // as the exec-fd face): synthetic /proc content has no
                // filesystem file behind the mapped path on Windows;
                // serve the snapshot bytes through the fd byte channel.
                self.set_fd_input_bytes(fd, bytes, true);
            } else if let Some(link) = {
                let (cwd_forms, root_forms) = self.proc_link_forms();
                crate::special_paths::synthetic_link_target(
                    &target,
                    Some(&cwd_forms),
                    Some(&root_forms),
                )
            } {
                // SyntheticLink (rubash#1900): open the resolved
                // native target (session cwd answers for self/cwd).
                // rubash#1937: a directory target binds the directory
                // endpoint (GNU's bind succeeds; reads fail EISDIR) —
                // see the exec-fd face in trap_exec.rs.
                if crate::proc_vfs::proc_unary_facts(&target)
                    .is_some_and(|facts| facts.is_link && facts.link_target_is_dir)
                {
                    self.set_fd_input_directory(fd, true);
                } else {
                    let file = FileFd::open_read(std::path::PathBuf::from(&link.native))
                        .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                    self.set_fd_input_file(fd, file, true);
                }
            } else if crate::special_paths::is_net_redirect_target(&target) {
                // rubash#2124: a /dev/tcp|udp target on a numbered fd
                // of a compound or function call — GNU redir.c
                // do_redirections connects it (redir_open ->
                // netopen); the plain-file open below reported it
                // ENOENT. The raw form is used because this ladder
                // shapes its own errors with path_error.
                let file = self
                    .open_net_redirect_raw(&target)
                    .map_err(|e| crate::posix_errors::path_error(&target, e))?;
                self.set_fd_input_file(fd, FileFd::from_file(file, target.clone().into()), true);
            } else {
                // niubash#177: same operand resolution as
                // open_input_redirect — a virtual-root input operand may
                // exist only under its `.exe` spelling on disk.
                let path = self.session_path(crate::executor::path::windows_operand_file_path(
                    shell_path_to_windows(&target, &self.shell_state.env_vars),
                    &self.shell_state.env_vars,
                ));
                // rubash#1903: a WinuxCmd fifo marker bound on a
                // numbered fd takes the rendezvous read open, not a
                // plain open of the marker file (same family as the
                // fd-0 bind in compound_fd0_stream_file).
                #[cfg(windows)]
                if crate::winuxcmd_fifo::path_is_winuxcmd_fifo(&path) {
                    let file = crate::winuxcmd_fifo::open_winuxcmd_fifo_read(&path, &|| {
                        crate::winuxcmd_fifo::fifo_wait_interrupted(&self.shell_state.env_vars)
                    })
                    .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                    self.set_fd_input_file(fd, FileFd::from_file(file, path), true);
                    continue;
                }
                let file = FileFd::open_read(path)
                    .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                self.set_fd_input_file(fd, file, true);
            }
        }
        Ok(())
    }

    pub(in crate::executor) fn restore_compound_numbered_input_redirects(
        &mut self,
        saved: Vec<SavedNumberedFd>,
    ) {
        for saved in saved {
            match saved.entry {
                Some(entry) => {
                    self.fd_table.entries.insert(saved.fd, entry);
                }
                None => {
                    self.fd_table.entries.remove(&saved.fd);
                }
            }
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_stdin_key(saved.fd),
                saved.fd_stdin,
            );
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_stdin_offset_key(saved.fd),
                saved.fd_stdin_offset,
            );
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_dynamic_input_key(saved.fd),
                saved.fd_dynamic,
            );
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_closed_key(saved.fd),
                saved.fd_closed,
            );
        }
    }

    /// GNU do_redirection_internal (redir.c:767-955) applies a compound
    /// command's output redirections to real descriptors — in parse order —
    /// before the body runs, and execute_cmd.c undoes them when the command
    /// finishes. Bind them on the fd table so a body's `1>&3` resolves fd 3
    /// to the *group's* binding (e.g. `>/dev/null 3>&1` leaves fd 3 on the
    /// null device) instead of re-dup'ing the leaf's own fd 1. Returns the
    /// touched slots for restore_compound_output_redirects; on a mid-list
    /// open failure the already-applied slots are unwound before the error
    /// propagates, like GNU abandoning the compound.
    pub(in crate::executor) fn open_compound_output_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Vec<SavedOutputFd>, ExecuteError> {
        let mut saved: Vec<SavedOutputFd> = Vec::new();
        if let Err(error) =
            self.open_compound_output_redirects_in(cmd, 0, cmd.redirects.len(), &mut saved)
        {
            self.restore_compound_output_redirects(saved);
            return Err(error);
        }
        Ok(saved)
    }

    fn save_compound_output_fd(&mut self, saved: &mut Vec<SavedOutputFd>, fd: u32) {
        if saved.iter().any(|entry| entry.fd == fd) {
            return;
        }
        saved.push(SavedOutputFd {
            fd,
            entry: self.fd_table.entries.get(&fd).cloned(),
            alias_generation: self.fd_table.stdout_alias_generation.get(&fd).cloned(),
            fd_output: self.shell_state.env_vars.get(&fd_output_key(fd)).cloned(),
            fd_procsub: self
                .shell_state
                .env_vars
                .get(&fd_output_process_substitution_key(fd))
                .cloned(),
            fd_closed: self.shell_state.env_vars.get(&fd_closed_key(fd)).cloned(),
        });
    }

    /// Range-scoped twin of open_compound_output_redirects (the
    /// full-range call above): binds only the numbered output
    /// entries in `cmd.redirects[start..end]`. rubash#2322: the
    /// same-exe script face's segmented walk binds outputs segment
    /// by segment so an output-side `{var}` entry observes the
    /// command's own earlier output binds at its list position.
    pub(in crate::executor) fn open_compound_output_redirects_in(
        &mut self,
        cmd: &CommandNode,
        start: usize,
        end: usize,
        saved: &mut Vec<SavedOutputFd>,
    ) -> Result<(), ExecuteError> {
        for redirect in &cmd.redirects[start..end] {
            // `{var}`-targeted redirections allocate a persistent dynamic fd
            // (redir.c: the variable keeps the descriptor after the command)
            // and are handled by execute_dynamic_fd_var_redirect above.
            if redirect.fd_var.is_some() {
                continue;
            }
            let fd = match redirect.kind {
                crate::parser::RedirectKind::Output
                | crate::parser::RedirectKind::Append
                | crate::parser::RedirectKind::ClobberOutput
                | crate::parser::RedirectKind::DuplicateOutput
                | crate::parser::RedirectKind::CloseOutput => redirect.fd.unwrap_or(1),
                crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => 1,
                _ => continue,
            };
            let target = self.expand_redirect_target(redirect);
            // subst.c:12663-12668 failglob: the diagnostic already printed
            // at the expansion; abort the compound before any fd binding.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }

            // See injected_redirect_fd_is_bound (redirection.rs): a redirect
            // injected from an enclosing compound is already realized by the
            // group's live fd-table binding — re-opening would re-truncate
            // the file and fork a second offset for the nested compound.
            if self.injected_redirect_fd_is_bound(redirect, fd) {
                continue;
            }

            match redirect.kind {
                crate::parser::RedirectKind::CloseOutput => {
                    self.save_compound_output_fd(saved, fd);
                    self.fd_table.close_output(fd);
                    self.shell_state
                        .env_vars
                        .insert(fd_closed_key(fd), "1".to_string());
                }
                crate::parser::RedirectKind::DuplicateOutput => {
                    self.save_compound_output_fd(saved, fd);
                    if is_closed_redirect_target(&target) {
                        self.fd_table.close_output(fd);
                        self.shell_state
                            .env_vars
                            .insert(fd_closed_key(fd), "1".to_string());
                        continue;
                    }
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        if move_source {
                            self.save_compound_output_fd(saved, source_fd);
                        }
                        // GNU dup2 copies the whole descriptor — the group's
                        // fd takes the source's open file description, shared
                        // offset included (fd_table dup_entry).
                        if self.fd_table.dup_output(fd, source_fd).is_err()
                            && source_fd <= 2
                            && !self.fd_table.entries.contains_key(&source_fd)
                        {
                            // The source is an IMPLICIT std fd (no fd-table
                            // entry — the shell's own stdout/stderr). GNU
                            // redir.c dup2 still succeeds: the duplicate
                            // snapshots the process stdio AT ENTRY, so a
                            // later `exec > f` inside the body rebinds fd 1
                            // only and writes to fd 2 keep going to the
                            // original stdout (rubash#170 nest4:
                            // `( exec > f; echo x >&2 ) 2>&1` must print x,
                            // not write f). The FdWriteEndpoint::Stdout /
                            // ::Stderr markers carry exactly that meaning —
                            // they resolve to the process stdio (or the
                            // active capture), never to a later fd-1
                            // rebinding.
                            let endpoint = if source_fd == 1 {
                                FdWriteEndpoint::Stdout
                            } else {
                                FdWriteEndpoint::Stderr
                            };
                            self.fd_table.open_output(fd, endpoint, false);
                            self.shell_state.env_vars.remove(&fd_closed_key(fd));
                        }
                        self.record_output_fd_ledger(fd);
                        // Self-move: no-op, never closes (rubash#1236).
                        if move_source && source_fd != fd {
                            self.fd_table.close(source_fd);
                            self.shell_state
                                .env_vars
                                .insert(fd_closed_key(source_fd), "1".to_string());
                        }
                        continue;
                    }
                    // GNU redir.c:832-838: `>&word`/`1>&word` with a
                    // non-numeric word is r_err_and_out — one open, fd 2
                    // shares fd 1's description. Other redirectors report
                    // AMBIGUOUS_REDIRECT; the propagation/diagnostic paths
                    // already report it, so leave the fd table alone.
                    if fd == 1 {
                        let path = target.strip_prefix('&').unwrap_or(&target);
                        if redirect.append {
                            self.set_fd_output_file(1, path.to_string(), false, true)
                        } else {
                            // rubash#1868: probe form — on a FIFO the
                            // physical create open would consume the
                            // rendezvous pairing set_fd_output_file's
                            // bind open waits for (see
                            // probe_output_redirect).
                            crate::executor::shell_options::probe_output_redirect(
                                self,
                                path,
                                redirect.clobber,
                            )?;
                            self.set_fd_output_file(1, path.to_string(), false, false)
                        }
                        .map_err(|error| crate::posix_errors::path_error(path, error))?;
                        self.save_compound_output_fd(saved, 2);
                        self.share_fd_output_file(1, 2, path, false);
                    }
                }
                crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => {
                    if is_closed_redirect_target(&target) {
                        continue;
                    }
                    // `&> >(...)` / `&>> >(...)`: materialize_compound_output_
                    // process_substitutions (rubash#394) has already rewritten
                    // this list entry's `>(` text to the substitution's temp
                    // carrier, so the walk below binds fd 1 and fd 2 to the
                    // carrier exactly like GNU's r_err_and_out dup2 onto the
                    // pipe (redir.c:832-838) — the compound's own live
                    // writes (select prompt, loop diagnostics) follow it,
                    // and the body's leaf writes arrive through the per-leaf
                    // splice on the same carrier.
                    self.save_compound_output_fd(saved, 1);
                    self.save_compound_output_fd(saved, 2);
                    let append = redirect.kind == crate::parser::RedirectKind::CombinedAppend;
                    // rubash#1025: same prepare/bind pair as the Output
                    // arm below — consume the prepare phase's record.
                    // rubash#1868: probe form of the create (FIFO targets
                    // are access-checked, not opened — the bind open in
                    // set_fd_output_file is the single pairing open).
                    if !append && !self.take_noclobber_prepared_target(&target) {
                        crate::executor::shell_options::probe_output_redirect(
                            self,
                            &target,
                            redirect.clobber,
                        )?;
                    }
                    self.set_fd_output_file(1, target.clone(), false, append)
                        .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                    self.share_fd_output_file(1, 2, &target, false);
                }
                // Output | Append | ClobberOutput
                _ => {
                    self.save_compound_output_fd(saved, fd);
                    if is_closed_redirect_target(&target) {
                        self.fd_table.close_output(fd);
                        self.shell_state
                            .env_vars
                            .insert(fd_closed_key(fd), "1".to_string());
                        continue;
                    }
                    // GNU redir.c:767-955 do_redirection_internal opens
                    // /dev/std*, /dev/fd/N and /proc/self/fd/N through the
                    // OS fd-alias layer: the kernel resolves the open() to a
                    // dup of fd N's CURRENT open file description
                    // (execution_misc::dev_stdio_redirect_fd documents the
                    // same for the input side, niubash#118). Opening the
                    // name as a literal Windows path instead bound the
                    // group's fd to the CONOUT$ console device, whose bytes
                    // bypass the real — possibly redirected or captured —
                    // stderr stream: a body's output vanished under pipes,
                    // and an external child re-opening the name failed with
                    // Permission denied (rubash#216). Dup the target fd's
                    // live binding here, with the implicit-std fallback the
                    // DuplicateOutput arm uses for a source with no
                    // fd-table entry (rubash#170 nest4).
                    if let Some(source_fd) = super::execution_misc::dev_stdio_redirect_fd(&target) {
                        // rubash#2276: a socket endpoint's
                        // spelling-open fails ENXIO (see
                        // fd_alias_spelling_open_error) — the dup
                        // below is the pipe rule, not the socket rule.
                        if let Some(error) = self.fd_alias_spelling_open_error(source_fd) {
                            return Err(ExecuteError::from(crate::posix_errors::path_error(
                                &target, error,
                            )));
                        }
                        // rubash#2256: the alias spelling is an open —
                        // a regular-file endpoint (either direction: a
                        // read-bound fd's file truncates/appends exactly
                        // like a write-bound one's) binds a FRESH open
                        // of its target, under the same noclobber
                        // verdict a plain target gets (GNU refuses
                        // `set -C; echo x > /dev/fd/6` naming the
                        // spelling). The fresh open's failure is the
                        // redirect's failure, as GNU's open is. Every
                        // other endpoint kind keeps the dup below.
                        if let Some(path) = self.fd_alias_reopen_path(source_fd) {
                            let append = matches!(
                                redirect.kind,
                                crate::parser::RedirectKind::Append
                                    | crate::parser::RedirectKind::CombinedAppend
                            );
                            if !append
                                && !redirect.clobber
                                && crate::builtins::set::shell_option_enabled(
                                    &self.shell_state.env_vars,
                                    "noclobber",
                                )
                                && !Self::noclobber_target_is_device_like(&path)
                            {
                                // The io payload is the bare reason:
                                // path_error adds the `{target}: ` head.
                                // Pre-formatting the head into the
                                // payload as well printed the spelling
                                // twice (rubash#2359) — the sibling
                                // faces (the routing walk's gate, the
                                // fresh-open serves) carry the
                                // pre-formatted shape only where the
                                // renderer writes the error verbatim.
                                return Err(ExecuteError::from(crate::posix_errors::path_error(
                                    &target,
                                    std::io::Error::new(
                                        std::io::ErrorKind::AlreadyExists,
                                        "cannot overwrite existing file",
                                    ),
                                )));
                            }
                            let fresh = FileFd::open_write(path, append, false)
                                .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                            self.fd_table
                                .open_output(fd, FdWriteEndpoint::File(fresh), true);
                            self.shell_state.env_vars.remove(&fd_closed_key(fd));
                            self.record_output_fd_ledger(fd);
                            continue;
                        }
                        // rubash#2385: the source is a pipe read end
                        // with no write endpoint to dup — GNU's open
                        // attaches to the pipe's other end, so bind
                        // the complementary write endpoint the
                        // resolver mints instead of the dup's empty
                        // write side.
                        if self.fd_table.output_endpoint(source_fd).is_none() {
                            if let Some(endpoint) =
                                self.fd_alias_complementary_write_endpoint(source_fd)
                            {
                                self.fd_table.open_output(fd, endpoint, false);
                                self.shell_state.env_vars.remove(&fd_closed_key(fd));
                                self.record_output_fd_ledger(fd);
                                continue;
                            }
                        }
                        if self.fd_table.dup_output(fd, source_fd).is_ok() {
                            self.record_output_fd_ledger(fd);
                            continue;
                        }
                        if source_fd <= 2 && !self.fd_table.entries.contains_key(&source_fd) {
                            let endpoint = if source_fd == 1 {
                                FdWriteEndpoint::Stdout
                            } else {
                                FdWriteEndpoint::Stderr
                            };
                            self.fd_table.open_output(fd, endpoint, false);
                            self.shell_state.env_vars.remove(&fd_closed_key(fd));
                            self.record_output_fd_ledger(fd);
                            continue;
                        }
                        // Source closed or unbound: fall through to the
                        // literal open, whose failure reports like GNU's
                        // open() on the alias would.
                    }
                    // rubash#1025: a noclobber target this compound's
                    // prepare phase already created (O_EXCL) must not be
                    // O_EXCL-opened again here — the record is consumed
                    // exactly once, so pre-existing files still fail in
                    // the prepare phase and bind-only scopes keep the
                    // plain check. rubash#1868: probe form of the create
                    // (FIFO targets are access-checked, not opened — the
                    // bind open in set_fd_output_file is the single
                    // pairing open).
                    if !redirect.append && !self.take_noclobber_prepared_target(&target) {
                        crate::executor::shell_options::probe_output_redirect(
                            self,
                            &target,
                            redirect.clobber,
                        )?;
                    }
                    // GNU redir.c do_redirection_internal: every `>file`
                    // redirection performs its OWN open() — `>f 2>f` binds
                    // two independent open file descriptions with
                    // independent offsets, so the fd-2 write overwrites the
                    // fd-1 write at offset 0 instead of concatenating
                    // (rubash#1068). A shared offset exists only where a dup
                    // created it (`2>&1`, `&>`, `>&word` — the dup arms
                    // above and share_fd_output_file). The earlier
                    // fd==2-forces-append model (rubash#826) made every
                    // compound fd-2 binding write at EOF, which is exactly
                    // the concatenation #1068 files; diagnostics routed
                    // through a plain `2>f` binding take the same
                    // independent-offset semantics as GNU's second open.
                    let append = redirect.append;
                    self.set_fd_output_file(fd, target.clone(), fd >= 10, append)
                        .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                }
            }
        }
        Ok(())
    }

    /// Scoped fd-table binding for a builtin that reparses its body in
    /// place (eval / fc / source / trap bodies): GNU applies the command's
    /// redirections to real descriptors for the command's whole duration
    /// (redir.c do_redirection_internal + execute_cmd.c undo on exit), so
    /// `eval 'echo x 1>&3' 3>&1` resolves fd 3 to the binding in force when
    /// eval ran.
    pub(crate) fn with_compound_output_redirects<T>(
        &mut self,
        cmd: &CommandNode,
        execute: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        let saved = self.open_compound_output_redirects(cmd)?;
        let result = execute(self);
        self.restore_compound_output_redirects(saved);
        result
    }

    pub(in crate::executor) fn restore_compound_output_redirects(
        &mut self,
        saved: Vec<SavedOutputFd>,
    ) {
        for saved in saved {
            match saved.entry {
                Some(entry) => {
                    self.fd_table.entries.insert(saved.fd, entry);
                }
                None => {
                    self.fd_table.entries.remove(&saved.fd);
                }
            }
            // Restore the dup2 snapshot record with the entry (rubash#223):
            // an inner scope may have cleared it (e.g. `2>/dev/null`
            // between the `2>&1` binding and the write).
            match saved.alias_generation {
                Some(record) => {
                    self.fd_table
                        .stdout_alias_generation
                        .insert(saved.fd, record);
                }
                None => {
                    self.fd_table.stdout_alias_generation.remove(&saved.fd);
                }
            }
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_output_key(saved.fd),
                saved.fd_output,
            );
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_output_process_substitution_key(saved.fd),
                saved.fd_procsub,
            );
            restore_optional_env_var(
                &mut self.shell_state.env_vars,
                &fd_closed_key(saved.fd),
                saved.fd_closed,
            );
        }
    }

    pub(in crate::executor) fn command_input_redirect(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<String> {
        if let Some(input) = self.loop_redirect_input(cmd) {
            return Some(input);
        }

        if cmd.here_string_carrier.is_some() {
            // GNU redir.c here_document_to_fd: a here-string's word is
            // written to the pipe with a trailing newline — the compound's
            // staged stdin must end with it, like the direct-read path in
            // stdin_string_for_command_mut (rubash#270: without it,
            // `while read v; done <<< alpha` stages "alpha" and the read
            // ends at EOF-without-delimiter).
            let mut input = self.expand_here_string_mut_from_carrier(&cmd.here_string_carrier);
            input.push('\n');
            return Some(input);
        }
        if let Some(here_string) = cmd.here_string.clone() {
            // Here-string content already had quote removal applied by the
            // parser; expand only substitutions with quotes-as-data semantics.
            let mut input = self.expand_here_string_mut(&here_string);
            input.push('\n');
            return Some(input);
        }

        if cmd.heredoc_body.is_some() {
            return Some(self.expand_heredoc_body_mut_from_carrier(&cmd.heredoc_body));
        }
        if let Some(heredoc) = cmd.heredoc.clone() {
            return Some(self.expand_heredoc_body_mut(&heredoc));
        }

        cmd.heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd.is_none())
            .map(|redirect| {
                if redirect.body_carrier.is_some() {
                    if redirect.here_string {
                        self.expand_here_string_mut_from_carrier(&redirect.body_carrier)
                    } else {
                        self.expand_heredoc_body_mut_from_carrier(&redirect.body_carrier)
                    }
                } else {
                    redirect
                        .body
                        .as_deref()
                        .map(|body| self.expand_heredoc_body_mut(body))
                        .unwrap_or_default()
                }
            })
    }

    /// The canonical text of one heredoc/herestring redirect entry
    /// (redir.c here_document_to_fd). The form the parser stored decides
    /// the expansion ladder: a typed carrier expands through its
    /// snapshot; a here-string's STORAGE_WORD_PREFIX storage word is
    /// ANSI-C-decoded when it has that form, else word-expanded; a plain
    /// here-string body takes the here-string expansion; a heredoc body
    /// takes the heredoc expansion. A here-string's bytes always end
    /// with a newline — that terminator is what lets a final `read`
    /// still return the last line. Every staging consumer (compound fd
    /// binds, dynamic-fd allocation, /dev/fd materialization) must use
    /// this ladder: feeding the raw stored body to the heredoc
    /// expansion leaks the storage prefix into the delivered stream and
    /// drops the trailing newline (rubash#1867).
    /// Expand one STORAGE_WORD_PREFIX here-string word (the parser's
    /// stored token value) to its final data bytes: ANSI-C decode when
    /// the word kept its textual `$'...'` form, else word expansion.
    /// The result is a byte stream (redir.c here_document_to_fd), so
    /// the field-split transport guard on lexer-decoded `$'...'` blanks
    /// (ANSI_C_IFS_GUARD) must not survive into it — the same
    /// value-boundary strip the redirect-filename path applies
    /// (expand_word.rs, rubash#379); the strip is E400-aware, so a
    /// user-typed literal U+E401 stays data (rubash#735, rubash#1867).
    pub(in crate::executor) fn expand_stored_here_string_word(&mut self, word: &str) -> String {
        let input = decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
        crate::executor::markers::strip_ansi_c_ifs_guard(&input)
    }

    pub(in crate::executor) fn expand_heredoc_redirect_text(
        &mut self,
        redirect: &crate::parser::HereDocRedirect,
    ) -> String {
        if redirect.body_carrier.is_some() {
            let mut input = if redirect.here_string {
                self.expand_here_string_mut_from_carrier(&redirect.body_carrier)
            } else {
                self.expand_heredoc_body_mut_from_carrier(&redirect.body_carrier)
            };
            if redirect.here_string {
                input.push('\n');
            }
            return input;
        }
        let body = redirect.body.as_deref().unwrap_or("");
        if redirect.here_string {
            let mut input = if let Some(word) = body.strip_prefix(STORAGE_WORD_PREFIX) {
                self.expand_stored_here_string_word(word)
            } else {
                self.expand_here_string_mut(body)
            };
            input.push('\n');
            input
        } else {
            self.expand_heredoc_body_mut(body)
        }
    }

    /// rubash#2309: classifies `body` (an unquoted heredoc body about to
    /// be expanded) for the first expansion failure in scan order,
    /// mirroring GNU's left-to-right expansion scan of the body word.
    /// On a failure this prints the owning layer's diagnostic, latches
    /// the expansion failure (the redirect is abandoned and the command
    /// fails with status 1 — GNU's expand_word error return propagating
    /// through do_redirections), and returns true; the caller then
    /// substitutes the empty body.
    fn report_heredoc_body_failure(&self, body: &str) -> bool {
        let Some(failure) = self.heredoc_body_first_failure(body) else {
            return false;
        };
        match &failure {
            HeredocBodyFailure::Backquote { text } => {
                // GNU report_error (error.c) line: the line of the
                // command being executed — the heredoc's command line,
                // regardless of which body line the backquote sits on.
                let line = self
                    .shell_state
                    .env_vars
                    .get("__RUBASH_CURRENT_LINE")
                    .and_then(|line| line.parse::<usize>().ok())
                    .unwrap_or(1);
                eprintln!(
                    "{}bad substitution: no closing \"`\" in {}",
                    self.error_prefix_for_line(line),
                    text
                );
            }
            HeredocBodyFailure::Comsub { close, line } => {
                eprintln!(
                    "{}unexpected EOF while looking for matching `{close}'",
                    self.comsub_eof_error_diagnostic(*line)
                );
            }
        }
        self.shell_state.arithmetic_fatal_error.set(true);
        self.shell_state.arithmetic_expansion_error.set(true);
        true
    }

    /// The first construct in a heredoc body whose expansion cannot
    /// complete, in the body's own scan order. Only the constructs the
    /// expansion scan itself delimits are judged: a backquote span
    /// (delimited atomically), a `$(` span (delimited by the comsub
    /// collector, then judged by the close-char verdict scan — the
    /// parse_comsub stand-in), and a `${` span (skipped when closed;
    /// when unterminated it is the parameter layer's own failure and is
    /// NOT claimed here). `$'` opens nothing in a heredoc body — quote
    /// processing does not run there — so it stays literal text, as do
    /// bare quotes.
    fn heredoc_body_first_failure(&self, body: &str) -> Option<HeredocBodyFailure> {
        let current_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .unwrap_or(1);
        // GNU seeds the command-substitution stream of a heredoc body at
        // the body's first line: EOF inside a `$(` reports that base
        // plus the lines consumed WITHIN the extracted substitution
        // text — not the opener's own body line, not the delimiter line
        // (oracle probes: a `$(` on body line 3 of 3 reports the same
        // line as one on body line 1 whose text spans all three).
        let body_first_line = current_line + 1;
        let posix = self
            .shell_state
            .env_vars
            .get("__RUBASH_POSIX_MODE")
            .map(|mode| mode == "1")
            .unwrap_or(false);
        let mut pos = 0usize;
        while pos < body.len() {
            let rest = &body[pos..];
            let ch = rest.chars().next().expect("pos stays on a char boundary");
            match ch {
                // A backslash quotes the next character in expansion
                // text: it cannot open a construct.
                '\\' => {
                    pos += 1 + rest[1..].chars().next().map_or(0, |next| next.len_utf8());
                }
                '`' => match heredoc_backquote_span_end(rest) {
                    Some(len) => pos += len,
                    None => {
                        return Some(HeredocBodyFailure::Backquote {
                            text: rest.to_string(),
                        });
                    }
                },
                '$' if rest[1..].starts_with('(') => {
                    let mut iter = rest[2..].chars().peekable();
                    let (source, closed) =
                        super::embedded_mutations::collect_command_substitution_source_ex(
                            &mut iter,
                            &self.shell_state.aliases,
                        );
                    if closed {
                        pos += 2
                            + (rest[2..].len() - iter.clone().map(|c| c.len_utf8()).sum::<usize>());
                        continue;
                    }
                    // The extracted body runs out: parse_comsub owns the
                    // verdict — the innermost pending construct in the
                    // extracted text names the missing delimiter, at its
                    // own line within that text (the verdict scan's
                    // report_open) or at the text's EOF line.
                    let (close, line) =
                        match crate::lexer::unclosed_input_close_char_posix(&source, posix) {
                            Some(verdict) => {
                                let verdict_line = if verdict.report_open {
                                    verdict.open_line
                                } else {
                                    verdict.eof_line
                                };
                                (verdict.close, body_first_line + verdict_line - 1)
                            }
                            None => (')', body_first_line + source.matches('\n').count()),
                        };
                    return Some(HeredocBodyFailure::Comsub { close, line });
                }
                '$' if rest[1..].starts_with('{') => {
                    match heredoc_parameter_span_end(rest, &self.shell_state.aliases) {
                        Some(len) => pos += len,
                        // An unterminated `${` fails in the parameter
                        // layer (subst.c string_extract, bad
                        // substitution), which diagnoses it downstream;
                        // this scan does not claim it.
                        None => return None,
                    }
                }
                _ => pos += ch.len_utf8(),
            }
        }
        None
    }

    /// Expands an unquoted heredoc body like Bash: parameter, command and
    /// arithmetic expansions run in the context of the receiving command.
    /// A dedicated quoted-heredoc marker keeps the body literal without sharing
    /// the compound-assignment transport protocol.
    pub(in crate::executor) fn expand_heredoc_body_mut(&mut self, body: &str) -> String {
        if let Some(pre) = preexpanded_stdin_body(body) {
            return decode_stdin_body_enq(pre);
        }
        let quoted = body.starts_with(crate::lexer::QUOTED_HEREDOC_MARKER);
        let body = strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body));
        if quoted {
            return decode_stdin_body_enq(body);
        }
        // comsub-eof6 + rubash#2309: a heredoc body is expanded as a word,
        // so a construct the expansion cannot complete fails HERE, in the
        // expansion layer, with that layer's diagnostic — classified by
        // report_heredoc_body_failure in scan order. (The genuine eof6
        // shape: body `$(seq 10` must not expand to `1`; GNU reports
        // `command substitution: … matching `)'`, leaves foo empty, and
        // the command fails.)
        if self.report_heredoc_body_failure(body) {
            return String::new();
        }
        let prepared = prepare_unquoted_heredoc_expansion(body);
        let expanded = self.expand_embedded_parameters_mut_with_context(
            &prepared,
            SubstitutionQuoteContext::HereDocument,
        );
        // Same decode boundary as the &self expand_heredoc_body below: the
        // mutable walker's substitution splices re-protect their output, so
        // C0 carriers and payload escapes must restore here or they leak as
        // raw bytes into the heredoc text.
        decode_stdin_body_enq(&decode_command_substitution_payload(
            &restore_command_substitution_output(&expanded),
        ))
    }

    /// Expands a heredoc body from a typed carrier. Returns the body verbatim
    /// if preexpanded; otherwise performs expansion.
    pub(in crate::executor) fn expand_heredoc_body_mut_from_carrier(
        &mut self,
        carrier: &Option<crate::parser::StdinBody>,
    ) -> String {
        match carrier {
            Some(crate::parser::StdinBody::Preexpanded(text)) => decode_stdin_body_enq(text),
            Some(crate::parser::StdinBody::NeedsExpansion(body)) => {
                self.expand_heredoc_body_mut(body)
            }
            None => String::new(),
        }
    }

    pub(in crate::executor) fn expand_heredoc_body_readback(
        &self,
        body: &str,
    ) -> SubstitutionOutput {
        let expanded = self.expand_heredoc_body(body);
        // expand_heredoc_body returns shell text; readback takes raw bytes,
        // so decode marker pairs once instead of re-encoding them.
        SubstitutionOutput::readback(
            crate::executor::substitution_metadata::shell_text_to_raw_bytes(&expanded),
            0,
            SubstitutionQuoteContext::HereDocument,
        )
    }

    /// Readback variant of `expand_heredoc_body_mut_from_carrier`: a
    /// `Preexpanded` carrier is already expanded text (preexpand_command_stdin
    /// ran at the GNU do_redirections point), so it returns verbatim —
    /// expanding again would re-run embedded substitutions and collapse
    /// backslash sequences a second time (e.g. `c\\`<newline>`d` -> `cd`).
    pub(in crate::executor) fn expand_heredoc_body_readback_from_carrier(
        &self,
        carrier: &Option<crate::parser::StdinBody>,
        fallback_body: Option<&str>,
    ) -> SubstitutionOutput {
        match carrier {
            Some(crate::parser::StdinBody::Preexpanded(text)) => {
                return SubstitutionOutput::readback(
                    crate::executor::substitution_metadata::shell_text_to_raw_bytes(
                        &decode_stdin_body_enq(text),
                    ),
                    0,
                    SubstitutionQuoteContext::HereDocument,
                );
            }
            Some(crate::parser::StdinBody::NeedsExpansion(body)) => {
                return self.expand_heredoc_body_readback(body);
            }
            None => {}
        }
        self.expand_heredoc_body_readback(fallback_body.unwrap_or_default())
    }

    pub(in crate::executor) fn expand_heredoc_body(&self, body: &str) -> String {
        if let Some(pre) = preexpanded_stdin_body(body) {
            return decode_stdin_body_enq(pre);
        }
        let quoted = body.starts_with(crate::lexer::QUOTED_HEREDOC_MARKER);
        let body = strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body));
        if quoted {
            return decode_stdin_body_enq(body);
        }
        // Same classification as the mut variant above (rubash#2309).
        if self.report_heredoc_body_failure(body) {
            return String::new();
        }
        let expanded =
            self.expand_embedded_parameters_for_heredoc(&prepare_unquoted_heredoc_expansion(body));
        decode_stdin_body_enq(&decode_command_substitution_payload(
            &restore_command_substitution_output(&expanded),
        ))
    }
}

fn prepare_unquoted_heredoc_expansion(body: &str) -> String {
    let mut output = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    let mut command_depth = 0usize;
    let mut single = false;
    let mut double = false;
    while let Some(ch) = chars.next() {
        if ch == '$' && !single && chars.peek() == Some(&'(') {
            chars.next();
            output.push('$');
            output.push('(');
            command_depth += 1;
            continue;
        }

        if command_depth > 0 {
            match ch {
                '\'' if !double => single = !single,
                '"' if !single => double = !double,
                '(' if !single && !double => command_depth += 1,
                ')' if !single && !double => command_depth = command_depth.saturating_sub(1),
                _ => {}
            }
        }

        if ch != '\\' {
            output.push(ch);
            continue;
        }

        if command_depth > 0 && single {
            output.push('\\');
            continue;
        }

        let mut slash_count = 1usize;
        while chars.peek() == Some(&'\\') {
            chars.next();
            slash_count += 1;
        }

        if matches!(chars.peek(), Some('\n') | Some('\r')) {
            for _ in 0..(slash_count / 2) {
                output.push('\\');
            }
            if slash_count % 2 == 1 {
                if chars.peek() == Some(&'\r') {
                    chars.next();
                }
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            continue;
        }

        for _ in 0..(slash_count / 2) {
            output.push(crate::executor::markers::DATA_BACKSLASH);
        }
        if slash_count % 2 == 0 {
            continue;
        }

        match chars.peek().copied() {
            Some('$') => {
                chars.next();
                output.push(DATA_DOLLAR);
            }
            Some('`') => {
                chars.next();
                output.push(crate::executor::markers::DATA_BACKTICK);
            }
            Some('\\') => unreachable!("backslash runs are consumed above"),
            _ => output.push('\\'),
        }
    }
    output
}

#[allow(dead_code)]
fn strip_heredoc_body(body: &str) -> String {
    strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string()
}

impl Executor {
    /// The compound's fd-0 `< file` source when it is a real stream
    /// object on disk — a regular file, a FIFO, or a character/block
    /// device: the sources whose kernel reads are an on-demand stream
    /// over one shared open file description, exactly GNU's
    /// shared-offset descriptor semantics (rubash#260; see
    /// with_command_input_redirects_inner). Everything else — process
    /// substitutions, /dev-fd aliases, /proc synthetic files,
    /// /dev/null, missing files, directories, `<>` read-write opens —
    /// returns None and keeps the FUNCTION_STDIN text-mirror transport.
    /// The distinction is load-bearing for FIFOs (rubash#1861): the
    /// mirror's fs::read_to_string (loop_redirect_input) reads a FIFO
    /// to EOF before the body runs — the bind stalls until the writer
    /// closes, the over-read bytes die with the mirror, and a later
    /// opener misses the writer entirely — while the FileFd endpoint
    /// reads byte-wise on demand (FdTable::read_file_bytes; only
    /// seekable disk files take the buffered + rewind path), so the
    /// stream is consumed exactly as GNU's unbuffered pipe reads are
    /// (read.def keeps pipes unbuffered for the same reason).
    fn compound_fd0_stream_file(&mut self, cmd: &CommandNode) -> Option<Rc<FileFd>> {
        if std::env::var_os("RB_721").is_some() {
            eprintln!(
                "[721] compound_stdin_open_failure redirect_in={:?} redirects={:?}",
                cmd.redirect_in
                    .as_deref()
                    .map(|r| format!("{} {:?}", r.target, r.kind)),
                cmd.redirects
                    .iter()
                    .map(|r| format!("{} {:?}", r.target, r.kind))
                    .collect::<Vec<_>>()
            );
        }
        let redirect = cmd.redirect_in.as_deref()?;
        if redirect.fd.unwrap_or(0) != 0 || redirect.fd_var.is_some() {
            return None;
        }
        if redirect.kind != crate::parser::RedirectKind::Input {
            return None;
        }
        if redirect.target.starts_with("<(") {
            return None;
        }
        let target = self.expand_redirect_target(redirect);
        if super::execution_misc::is_closed_redirect_target(&target)
            || super::execution_misc::dev_stdio_redirect_fd(&target).is_some()
            || is_null_device(&target)
        {
            return None;
        }
        if crate::special_paths::synthetic_proc_content(&target).is_some() {
            return None;
        }
        if crate::special_paths::is_synthetic_link_path(&target) {
            return None;
        }
        // niubash#177: same operand resolution as open_input_redirect so
        // this regular-file probe agrees with the read that follows.
        let path = self.session_path(super::path::windows_operand_file_path(
            super::path::shell_path_to_windows(&target, &self.shell_state.env_vars),
            &self.shell_state.env_vars,
        ));
        // rubash#1903: a WinuxCmd fifo marker is a regular file on disk,
        // so the metadata class below admits it and FileFd::open_read
        // would bind a plain handle on the marker bytes — the body's
        // reads would consume the marker while the writer waits for a
        // rendezvous server that never exists (the #2014 CI hang's
        // compound face). The fd-0 bind performs GNU's single open
        // (redir.c do_redirection_internal) — for a marker that open
        // is the read side of the named-pipe rendezvous, and the slot
        // binds the live pipe handle.
        #[cfg(windows)]
        if crate::winuxcmd_fifo::path_is_winuxcmd_fifo(&path) {
            let file = crate::winuxcmd_fifo::open_winuxcmd_fifo_read(&path, &|| {
                crate::winuxcmd_fifo::fifo_wait_interrupted(&self.shell_state.env_vars)
            })
            .ok()?;
            return Some(FileFd::from_file(file, path));
        }
        if !std::fs::metadata(&path)
            .map(|meta| Self::compound_fd0_stream_source(&meta))
            .unwrap_or(false)
        {
            return None;
        }
        FileFd::open_read(path).ok()
    }

    /// The descriptor-bindable source class for a compound's fd-0
    /// `< file` (see compound_fd0_stream_file): regular files plus —
    /// on unix — FIFOs and character/block devices, the filesystem
    /// objects that yield a live kernel stream from open(2). Windows
    /// has no filesystem FIFOs or device nodes of this kind, so the
    /// class is regular files only there.
    ///
    /// rubash#2101: directories join the class on every platform.
    /// GNU opens a directory for a compound's stdin exactly like a
    /// file (redir.c redir_open) and binds the descriptor; the body
    /// observes the read failure itself (the read builtin's EISDIR
    /// diagnostic, an external reader's own error). Excluding them
    /// dropped the binding to the text mirror, which swallowed the
    /// error at bind time — `{ cat; } < dir` exited 0 silently. The
    /// Windows open succeeds through the fd layer's directory retry
    /// (open_file_read's FILE_FLAG_BACKUP_SEMANTICS arm).
    fn compound_fd0_stream_source(meta: &std::fs::Metadata) -> bool {
        if meta.is_file() || meta.is_dir() {
            return true;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            let file_type = meta.file_type();
            file_type.is_fifo() || file_type.is_char_device() || file_type.is_block_device()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    /// GNU redir.c do_redirection_internal (redir.c:767-955) opens a
    /// compound command's fd-0 input redirection before the command runs;
    /// open failure aborts the compound with the diagnostic and status 1
    /// (rubash#137). Mirrors loop_redirect_input's target resolution:
    /// closed targets (`<&-`), process substitutions (`<(`), /dev/fd
    /// aliases, /proc synthetic files and append-creates never fail here.
    ///
    /// rubash#2024: the `<(`-shaped skip is recorded-span admission, not a
    /// raw-prefix blacklist — GNU records a process substitution only when
    /// the parser saw the `<(` unquoted (parse.y read_token_word; the
    /// executor-facing metadata is target_metadata.process_substitutions),
    /// and a QUOTED `"<(echo hi)"` target is a literal filename that
    /// redir.c:916 redir_open fails ENOENT:
    /// `<(echo hi): No such file or directory`, compound abandoned, body
    /// never runs. This probe IS the bind layer's literal-target error
    /// channel: without it, declining a quoted span's spawn in
    /// bind_command_fd0_input / loop_redirect_input falls through to the
    /// ambient stdin and the compound hangs instead of failing.
    pub(in crate::executor) fn compound_stdin_open_failure(
        &mut self,
        cmd: &CommandNode,
    ) -> Option<String> {
        let redirect = cmd.redirect_in.as_deref()?;
        if redirect.fd.unwrap_or(0) != 0 {
            return None;
        }
        if redirect.target.starts_with("<(")
            && super::external_setup::redirect_target_span_is_recorded(redirect)
        {
            return None;
        }
        let target = self.expand_redirect_target(redirect);
        if super::execution_misc::is_closed_redirect_target(&target) {
            return None;
        }
        // rubash#2124: a /dev/tcp|udp target is not a filesystem
        // object — the plain File::open probe below reports it
        // ENOENT, which is spurious. The compound bind
        // (bind_command_fd0_input) connects the net target and owns
        // its outcome, success or failure, like GNU redir.c
        // do_redirections.
        if crate::special_paths::is_net_redirect_target(&target) {
            return None;
        }
        if let Some(source) = super::execution_misc::dev_stdio_redirect_fd(&target) {
            // rubash#721: `/dev/fd/N` names descriptor N. GNU's open of the
            // alias fails ENOENT when N is closed (redir.c
            // redirection_error over the dangling /proc/self/fd magic
            // symlink), and the compound is abandoned with that diagnostic.
            // Treating every alias as available staged an empty mirror, so
            // `while read ... done < /dev/fd/6` ran zero iterations and
            // reported 0.
            //
            // This must run BEFORE the `&N` probe below:
            // redirect_target_fd_and_move falls back to dev_stdio_redirect_fd
            // for a word without `&`, so it answers Some for `/dev/fd/N` too
            // and would short-circuit this check.
            if !super::redirection::dup_source_fd_open(
                &self.fd_table,
                source,
                &std::collections::HashMap::new(),
            ) {
                return Some(format!(
                    "{}{target}: No such file or directory",
                    self.error_prefix()
                ));
            }
            return None;
        }
        // `<&N` / `<&N-` dups are descriptor operations, not path opens —
        // their source validity is enforced by validate_builtin_dup_source
        // (builtins) and open_compound_numbered_input_redirects's dup_entry
        // (compounds); probing "&N" as a file here would fail every dup
        // form with a bogus ENOENT (rubash#575: `exec cat <&0` inside a
        // command substitution must run, not abort).
        if super::execution_misc::redirect_target_fd_and_move(&target).is_some() {
            return None;
        }
        if redirect.append {
            return None;
        }
        if crate::special_paths::synthetic_proc_content(&target).is_some() {
            return None;
        }
        if crate::special_paths::is_synthetic_link_path(&target) {
            return None;
        }
        // An empty expanded target is a literal name no file can match:
        // GNU redir_open (redir.c:916) opens it and fails ENOENT — the
        // outcome the simple-command face already reports (`echo hi < ""`
        // → `: No such file or directory`). The session-path conversion
        // below would instead resolve "" to the session directory, the
        // probe open would succeed, and the compound would silently fall
        // back to the ambient stdin — the failure the rubash#2024 nounset
        // pin guards (`done < "$NOPE"` with NOPE unbound expands to "").
        // Here-strings and heredocs carry their payload AS the target,
        // so only the path-open kinds (Input / ReadWrite) take this arm.
        if target.is_empty()
            && matches!(
                redirect.kind,
                crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite
            )
        {
            return Some(format!(
                "{}{}",
                self.error_prefix(),
                crate::posix_errors::path_error(
                    &target,
                    std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory",),
                )
            ));
        }
        // niubash#177: same operand resolution as open_input_redirect —
        // a virtual-root input operand may exist only under its `.exe`
        // spelling on disk.
        let path = self.session_path(super::path::windows_operand_file_path(
            super::path::shell_path_to_windows(&target, &self.shell_state.env_vars),
            &self.shell_state.env_vars,
        ));
        // `<> word` (r_input_output) opens O_RDWR|O_CREAT (make_cmd.c:682),
        // so a missing name is CREATED by the open and never fails here
        // (rubash#264); a genuine open error (bad directory, permission)
        // still aborts the compound.
        // rubash#1820: the plain-input probe must not open a FIFO — the
        // throwaway O_RDONLY open consumes the rendezvous pairing the
        // real acquisition (the FUNCTION_STDIN slurp, or a live fd-0
        // bind) needs, and blocks until a writer appears in its own
        // right. An existing FIFO validates through access(2) instead
        // (probe_input_redirect_gate), with the same errno surface.
        let probe_error = if redirect.kind == crate::parser::RedirectKind::ReadWrite {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(&path)
                .err()
        } else if super::shell_options::redirect_path_is_fifo(&path) {
            super::shell_options::probe_input_redirect_gate(&path).err()
        } else {
            std::fs::File::open(&path).err()
        };
        match probe_error {
            None => None,
            // rubash#2024: path_error shapes the GNU redirection failure
            // text (`<target>: <strerror>`) and maps the
            // ERROR_INVALID_NAME a `<`/`>`-bearing literal target
            // (`"<(echo hi)"`) provokes on Windows to GNU's ENOENT —
            // redirection_error (redir.c:135) prints the strerror text.
            Some(error) => Some(format!(
                "{}{}",
                self.error_prefix(),
                crate::posix_errors::path_error(&target, error)
            )),
        }
    }
}

impl Executor {
    /// Bind ONLY the numbered (>2) input and output redirects of `cmd` on
    /// the fd table for `body`'s duration — the narrow subset of
    /// with_command_input_redirects that a function-definition redirect
    /// list needs (GNU execute_cmd.c:854 applies tc->redirects when the
    /// stored body command runs). The full wrapper is wrong here: its
    /// fd-0/FUNCTION_STDIN text-channel bookkeeping belongs to compounds
    /// with stdin redirects, and saving/restoring it around a function
    /// body corrupted the driver's pending-stdin state.
    pub(in crate::executor) fn with_numbered_fd_bindings<T: Default>(
        &mut self,
        cmd: &CommandNode,
        body: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        // Same preflight-first discipline as with_command_input_redirects:
        // the walk is pure, and the abandonment it triggers must not race
        // half-applied binds.
        if let Some((index, name)) = self.compound_dup_bad_fd(cmd) {
            return self.abandon_compound_for_dup_bad_fd(cmd, index, &name);
        }
        let saved_numbered = self.open_compound_numbered_input_redirects(cmd)?;
        let saved_output = self.open_compound_output_redirects(cmd)?;
        // rubash#1867: the numbered heredoc/herestring entries stage
        // through the same wrap as every other compound — a definition
        // carrying `3<<<word` binds the text for the body's duration.
        let result = self.with_loop_fd_heredocs(cmd, body);
        self.restore_compound_output_redirects(saved_output);
        self.restore_compound_numbered_input_redirects(saved_numbered);
        result
    }

    /// rubash#1867: bind a function CALL's numbered input redirections
    /// and heredoc/herestring entries for the body's duration (GNU
    /// execute_cmd.c:5606 do_redirections over the call, undone at
    /// return). The call machinery already covers fd 0
    /// (function_call_stdin) and the outputs
    /// (with_compound_output_redirects); the numbered inputs were
    /// never opened at all, so `f 3<file` / `f 3<<<word` left the body
    /// reading a bad fd.
    pub(crate) fn with_call_numbered_input_redirects<T>(
        &mut self,
        cmd: &CommandNode,
        body: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        let saved_numbered = self.open_compound_numbered_input_redirects(cmd)?;
        let result = self.with_loop_fd_heredocs(cmd, body);
        self.restore_compound_numbered_input_redirects(saved_numbered);
        result
    }

    /// GNU redir.c:1169-1171 (`else if (dup2 (redir_fd, redirector) < 0)
    /// return (errno)`) with redir.c:149-176 redirection_error: a dup whose
    /// source fd is not open fails EBADF, reports `<fd>: Bad file
    /// descriptor`, and ABANDONS the compound without running the body —
    /// the element's status is 1 (WSL 5.3.0 probe: `{ printf X >&2; } 2>&3
    /// 3>&1 | cat` prints the diagnostic once, the pipe stays empty,
    /// PIPESTATUS[0] = 1). The open walks themselves
    /// (open_compound_numbered_input_redirects /
    /// open_compound_output_redirects_inner → fd_table dup) cannot express
    /// the abandonment, so the callers preflight the list
    /// (`compound_dup_bad_fd`) and route the failure here. The diagnostic
    /// goes through the fd 2 binding only the redirects BEFORE the failing
    /// dup installed (`{ :; } 2>/dev/null 2>&3` is silent), because
    /// do_redirections stops at the failing step.
    pub(in crate::executor) fn abandon_compound_for_dup_bad_fd<T: Default>(
        &mut self,
        cmd: &CommandNode,
        index: usize,
        name: &str,
    ) -> Result<T, ExecuteError> {
        let mut stderr = Vec::new();
        writeln!(
            stderr,
            "{}{name}: Bad file descriptor",
            self.diagnostic_prefix()
        )?;
        let mut prefix_cmd = cmd.clone();
        prefix_cmd.redirects.truncate(index);
        let _ = self.write_redirect_diagnostic_routed(&prefix_cmd, &stderr)?;
        self.exit_code = 1;
        // The compound command is terminated without running: its result is
        // the default (the same shape as the stdin open failure path in
        // with_command_input_redirects_inner).
        Ok(T::default())
    }

    /// Ordered preflight of a compound's dup redirects — both sides in ONE
    /// left-to-right walk, because GNU do_redirections applies the list in
    /// order and stops at the first failing step (redir.c:255-260): the
    /// first `M>&N`/`M<&N` (or `M>&N-`/`M<&N-` move) whose source fd N is
    /// not open — judged with the same opened/closed tracking as the
    /// simple-command preflight (`command_output_redirect_fails` /
    /// `dup_source_fd_open`) — returns `(index, diagnostic name)`. Pure:
    /// opens nothing, dups nothing.
    ///
    /// rubash#2030 (input side): a compound-level `<&N` whose source is
    /// unbound used to dup silently (`let _ = dup_input`) and the level
    /// fell back to the ambient descriptor — `{ while read x; …; done
    /// <&3; }` read the pipeline's stdin instead of dying. GNU redir.c
    /// do_redirection_internal (case r_duplicating_input, redir.c:1179
    /// `dup2 (redir_fd, redirector) < 0 → return (errno)`) fails EBADF and
    /// redirection_error (redir.c:149-158) names the SOURCE fd — WSL 5.3
    /// probe: `done <&3` and `done 5<&3` with fd 3 closed both print
    /// `3: Bad file descriptor`, the loop never runs, status 1.
    pub(in crate::executor) fn compound_dup_bad_fd(
        &self,
        cmd: &CommandNode,
    ) -> Option<(usize, String)> {
        let mut opened: HashMap<u32, bool> = HashMap::new();
        for (index, redirect) in cmd.redirects.iter().enumerate() {
            if redirect.fd_var.is_some() {
                continue;
            }
            let fd = match redirect.kind {
                crate::parser::RedirectKind::Output
                | crate::parser::RedirectKind::Append
                | crate::parser::RedirectKind::ClobberOutput
                | crate::parser::RedirectKind::DuplicateOutput
                | crate::parser::RedirectKind::CloseOutput => redirect.fd.unwrap_or(1),
                crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => 1,
                crate::parser::RedirectKind::Input
                | crate::parser::RedirectKind::ReadWrite
                | crate::parser::RedirectKind::DuplicateInput
                | crate::parser::RedirectKind::CloseInput
                | crate::parser::RedirectKind::HereDoc
                | crate::parser::RedirectKind::HereString => redirect.fd.unwrap_or(0),
                crate::parser::RedirectKind::Unknown => continue,
            };
            let target = self.expand_redirect_target(redirect);
            match redirect.kind {
                crate::parser::RedirectKind::CloseOutput
                | crate::parser::RedirectKind::CloseInput => {
                    opened.insert(fd, false);
                }
                crate::parser::RedirectKind::DuplicateInput
                | crate::parser::RedirectKind::DuplicateOutput => {
                    if is_closed_redirect_target(&target) {
                        opened.insert(fd, false);
                        continue;
                    }
                    if let Some((source_fd, _move_source)) = redirect_target_fd_and_move(&target) {
                        // Self-dup/self-move is a no-op that succeeds even
                        // for a closed fd (redir.c `redir_fd !=
                        // redirector` guard; rubash#1236) and leaves the
                        // opened bookkeeping unchanged.
                        if source_fd == fd {
                            continue;
                        }
                        if !dup_source_fd_open(&self.fd_table, source_fd, &opened) {
                            let raw_word = redirect
                                .target_metadata
                                .raw
                                .trim_start_matches('&')
                                .trim_end_matches('-');
                            return Some((index, dup_bad_fd_name(raw_word, fd, source_fd)));
                        }
                        let open = dup_source_fd_open(&self.fd_table, source_fd, &opened);
                        opened.insert(fd, open);
                        continue;
                    }
                    // `>&word` / `1>&word` (r_err_and_out) opens the word —
                    // the open walk reports its failures.
                    opened.insert(fd, true);
                }
                crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => {
                    opened.insert(1, true);
                    opened.insert(2, true);
                }
                // Output | Append | ClobberOutput: a file open binds its
                // redirector; an fd-alias name (/dev/fd/N) dups the alias.
                // Input | ReadWrite | HereDoc | HereString: the same — a
                // file/alias open or staged stdin-body text binds the
                // redirector (the open walk reports its own failures), so
                // it counts as open for a later dup's source check.
                _ => {
                    if is_closed_redirect_target(&target) {
                        opened.insert(fd, false);
                    } else if let Some(source) =
                        super::execution_misc::dev_stdio_redirect_fd(&target)
                    {
                        let open = dup_source_fd_open(&self.fd_table, source, &opened);
                        opened.insert(fd, open);
                    } else {
                        opened.insert(fd, true);
                    }
                }
            }
        }
        None
    }
}
