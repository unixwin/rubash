use super::*;
use crate::executor::fd_table::FdEntry;
use crate::executor::markers::DATA_DOLLAR;

/// Saved descriptor state for a numbered fd opened by a compound command's
/// input redirection (see open_compound_numbered_input_redirects).
struct SavedNumberedFd {
    fd: u32,
    entry: Option<FdEntry>,
    fd_stdin: Option<String>,
    fd_stdin_offset: Option<String>,
    fd_dynamic: Option<String>,
    fd_closed: Option<String>,
}

/// Saved descriptor state for an fd rebound by a compound command's
/// output redirection (see open_compound_output_redirects).
pub(in crate::executor) struct SavedOutputFd {
    fd: u32,
    entry: Option<FdEntry>,
    fd_output: Option<String>,
    fd_procsub: Option<String>,
    fd_closed: Option<String>,
}

impl Executor {
    pub(crate) fn with_command_input_redirects<T>(
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
        let dynamic_names = cmd
            .redirects
            .iter()
            .filter_map(|redirect| redirect.fd_var.clone())
            .collect::<Vec<_>>();
        for redirect in &cmd.redirects {
            if redirect.fd_var.is_some() {
                let _ = self.execute_dynamic_fd_var_redirect(redirect, false)?;
            }
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

    fn with_command_input_redirects_inner<T>(
        &mut self,
        cmd: &CommandNode,
        execute: impl FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        let Some(input) = self.command_input_redirect(cmd) else {
            return execute(self);
        };

        let old_function_stdin = self.shell_state.env_vars.get(FUNCTION_STDIN).cloned();
        let old_function_stdin_offset = self
            .shell_state
            .env_vars
            .get(FUNCTION_STDIN_OFFSET)
            .cloned();
        // GNU do_redirection_internal (redir.c:767-955, RX_UNDOABLE) saves
        // fd 0 before a compound's `<` and restores it at scope end — even
        // undoing a permanent `exec 0<g` inside the body. Snapshot the slot
        // so async-spawn stdin materialization (and any fd-0 mutation the
        // body performs) unwinds with the compound.
        let saved_fd0_entry = self.fd_table.entries.get(&0).cloned();
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN.to_string(), input);
        self.shell_state
            .env_vars
            .insert(FUNCTION_STDIN_OFFSET.to_string(), "0".to_string());

        let result = execute(self);
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FUNCTION_STDIN,
            old_function_stdin,
        );
        restore_optional_env_var(
            &mut self.shell_state.env_vars,
            FUNCTION_STDIN_OFFSET,
            old_function_stdin_offset,
        );
        match saved_fd0_entry {
            Some(entry) => {
                self.fd_table.entries.insert(0, entry);
            }
            None => {
                self.fd_table.entries.remove(&0);
            }
        }
        result
    }

    /// Opens the numbered input redirections of a compound command in the
    /// fd table for the command's whole duration. GNU applies compound
    /// redirections left to right before the command runs (redir.c
    /// do_redirection_internal:767); procsub targets feed
    /// FdReadEndpoint-backed text exactly like the persistent fd-var path
    /// (trap_exec.rs execute_dynamic_fd_var_redirect).
    fn open_compound_numbered_input_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Vec<SavedNumberedFd>, ExecuteError> {
        let mut saved: Vec<SavedNumberedFd> = Vec::new();
        for redirect in &cmd.redirects {
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
                        if move_source {
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
            // Real handle on the slot: [N]<> is a single O_RDWR open file
            // description (redir.c r_input_output), otherwise O_RDONLY.
            if redirect.kind == crate::parser::RedirectKind::ReadWrite {
                self.set_fd_readwrite_file(fd, &target, true)
                    .map_err(|e| crate::posix_errors::path_error(&target, e))?;
            } else {
                let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
                let file = FileFd::open_read(path)
                    .map_err(|error| crate::posix_errors::path_error(&target, error))?;
                self.set_fd_input_file(fd, file, true);
            }
        }
        Ok(saved)
    }

    fn restore_compound_numbered_input_redirects(&mut self, saved: Vec<SavedNumberedFd>) {
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
        if let Err(error) = self.open_compound_output_redirects_inner(cmd, &mut saved) {
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
            fd_output: self.shell_state.env_vars.get(&fd_output_key(fd)).cloned(),
            fd_procsub: self
                .shell_state
                .env_vars
                .get(&fd_output_process_substitution_key(fd))
                .cloned(),
            fd_closed: self.shell_state.env_vars.get(&fd_closed_key(fd)).cloned(),
        });
    }

    fn open_compound_output_redirects_inner(
        &mut self,
        cmd: &CommandNode,
        saved: &mut Vec<SavedOutputFd>,
    ) -> Result<(), ExecuteError> {
        for redirect in &cmd.redirects {
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
                        let _ = self.fd_table.dup_output(fd, source_fd);
                        self.record_output_fd_ledger(fd);
                        if move_source {
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
                            self.create_redirect_output(path, redirect.clobber)?;
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
                    self.save_compound_output_fd(saved, 1);
                    self.save_compound_output_fd(saved, 2);
                    let append = redirect.kind == crate::parser::RedirectKind::CombinedAppend;
                    if !append {
                        self.create_redirect_output(&target, redirect.clobber)?;
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
                    if !redirect.append {
                        self.create_redirect_output(&target, redirect.clobber)?;
                    }
                    self.set_fd_output_file(fd, target.clone(), fd >= 10, redirect.append)
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
            return Some(self.expand_here_string_mut_from_carrier(&cmd.here_string_carrier));
        }
        if let Some(here_string) = cmd.here_string.clone() {
            // Here-string content already had quote removal applied by the
            // parser; expand only substitutions with quotes-as-data semantics.
            return Some(self.expand_here_string_mut(&here_string));
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
        // comsub-eof6: `read foo <<EOF` with body `$(seq 10` (missing `)`)
        // must not expand to `1`; GNU reports `command substitution:
        // unexpected EOF while looking for matching `)'` and leaves foo empty.
        // Detect unclosed `$(` in the raw body before expansion.
        if crate::lexer::has_unclosed_command_substitution(body) {
            // GNU error.c:300 parser_error with yy_input_name()=="command
            // substitution": `script: command substitution: line N:` — N is
            // the inherited script line when the comsub string runs out
            // (evalstring.c push_stream(0) keeps line_number), i.e. the
            // delimiter line: command line + body lines + 1.
            let eof_line = self
                .shell_state
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .and_then(|line| line.parse::<usize>().ok())
                .unwrap_or(1)
                + body.lines().count()
                + 1;
            eprintln!(
                "{}unexpected EOF while looking for matching `)'",
                self.comsub_eof_diagnostic(eof_line)
            );
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
        if crate::lexer::has_unclosed_command_substitution(body) {
            // Same GNU parser_error shape as the mut variant above: the
            // reported line is the heredoc delimiter line (command line +
            // body lines + 1), since the comsub inherits the outer
            // line_number (evalstring.c push_stream(0)).
            let eof_line = self
                .shell_state
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .and_then(|line| line.parse::<usize>().ok())
                .unwrap_or(1)
                + body.lines().count()
                + 1;
            eprintln!(
                "{}unexpected EOF while looking for matching `)'",
                self.comsub_eof_diagnostic(eof_line)
            );
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
