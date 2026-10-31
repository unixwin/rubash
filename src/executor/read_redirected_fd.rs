use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;

impl Executor {
    pub(in crate::executor) fn read_redirected_fd(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        // GNU redir.c applies the command's own redirections before the
        // builtin runs, so `read -u 3 x 3< file` reads the file. rubash
        // parks a numbered input redirect in cmd.redirects (only the
        // fd-0 one lands in redirect_in); mapfile already consults both
        // stores (mapfile_fd_is_available), read consulted only
        // redirect_in and reported the fd invalid (rubash#1296). The
        // last matching redirect in source order is the effective one.
        let redirect = cmd
            .redirect_in
            .as_deref()
            .filter(|redirect| redirect.fd == Some(fd))
            .or_else(|| {
                cmd.redirects.iter().rev().find(|redirect| {
                    redirect.fd == Some(fd)
                        && matches!(
                            redirect.kind,
                            crate::parser::RedirectKind::Input
                                | crate::parser::RedirectKind::ReadWrite
                        )
                })
            })?;

        self.read_redirected_fd_entry(
            redirect,
            cmd.line,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        )
    }

    /// Entry-exact file serve (rubash#2314): serve the descriptor
    /// from the input entry AT `index` in the command's own list —
    /// the entry a positioned binding scan resolved as the winner —
    /// instead of the last entry naming the fd.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::executor) fn read_redirected_fd_at(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
        index: usize,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        let redirect = cmd
            .redirects
            .iter()
            .take(index + 1)
            .rev()
            .find(|redirect| {
                redirect.fd == Some(fd)
                    && matches!(
                        redirect.kind,
                        crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite
                    )
            })?;
        self.read_redirected_fd_entry(
            redirect,
            cmd.line,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        )
    }

    fn read_redirected_fd_entry(
        &mut self,
        redirect: &crate::parser::Redirect,
        line: Option<usize>,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        if let Some(source) = redirect
            .target
            .strip_prefix("<(")
            .and_then(|target| target.strip_suffix(')'))
        {
            // rubash#1952: stream first — serve the record from the
            // spawned body's live pipe through a scratch fd (GNU
            // subst.c:6362 process_substitute forks the body; the
            // record reader observes real arrival). Collection below
            // is the spawn-failure fallback.
            if let Some(scratch) = self.allocate_dynamic_fd() {
                if let Some(file_fd) = self.spawn_input_process_substitution_child(source, line) {
                    self.fd_table
                        .open_input(scratch, FdReadEndpoint::File(file_fd), true);
                    let line = self.fd_table.read_text(
                        scratch,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                    self.fd_table.entries.remove(&scratch);
                    return Some(match line {
                        Some(line) => trim_read_input(
                            line,
                            delimiter,
                            char_limit,
                            exact_char_limit,
                            escape_aware,
                            raw,
                        ),
                        None => {
                            if !self.fd_table.read_timed_out {
                                self.read_eof_no_delimiter = true;
                            }
                            String::new()
                        }
                    });
                }
            }
            let output = self.process_substitution_output(source)?;
            self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                &output,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            );
            return Some(trim_read_input(
                output,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ));
        }

        let target = self.expand_redirect_target(redirect);
        if is_closed_redirect_target(&target) {
            return None;
        }
        // rubash#2235: an fd-alias target (/dev/fd/N, /proc/self/fd/N)
        // names the fd table's endpoint at N (the rubash#1963 alias
        // layer) — above all a list-level input procsub the
        // materializer rewrote to exactly this spelling. Serve the
        // record from that endpoint through a scratch fd, mirroring
        // the streamed `<(` arm above; the path slurp below would
        // read the process's unrelated real fd N (or fail on it).
        // rubash#2256 (wave 2): the alias is an open() (GNU redir.c),
        // so a source the resolver names as a regular file is read
        // through a FRESH FileFd — offset 0, the source descriptor's
        // own offset unmoved — instead of a clone of the source's
        // endpoint, whose shared offset made repeated
        // `read < /dev/fd/6` calls walk the file instead of
        // re-reading its first record.
        if let Some(alias_fd) = dev_stdio_redirect_fd(&target) {
            // rubash#2276: a socket endpoint's spelling-open fails
            // ENXIO in GNU, so the alias serve below must not stream
            // it; decline the whole resolution (the redirect's
            // failure is reported by the binding face — falling
            // through to the path slurp would read the process's
            // unrelated real fd instead).
            if self.fd_alias_spelling_open_error(alias_fd).is_some() {
                return None;
            }
            let fresh_endpoint = self
                .fd_alias_reopen_path(alias_fd)
                .and_then(|path| FileFd::open_read(path).ok())
                .map(FdReadEndpoint::File);
            let endpoint = fresh_endpoint.or_else(|| self.fd_table.read_endpoint(alias_fd));
            if let Some(endpoint) = endpoint {
                if let Some(scratch) = self.allocate_dynamic_fd() {
                    self.fd_table.open_input(scratch, endpoint, true);
                    let line = self.fd_table.read_text(
                        scratch,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                    self.fd_table.entries.remove(&scratch);
                    return Some(match line {
                        Some(line) => trim_read_input(
                            line,
                            delimiter,
                            char_limit,
                            exact_char_limit,
                            escape_aware,
                            raw,
                        ),
                        None => {
                            if !self.fd_table.read_timed_out {
                                self.read_eof_no_delimiter = true;
                            }
                            String::new()
                        }
                    });
                }
            }
        }
        // niubash#177: same operand resolution as open_input_redirect — a
        // virtual-root input operand may exist only under its `.exe`
        // spelling on disk.
        let path = self.session_path(crate::executor::path::windows_operand_file_path(
            shell_path_to_windows(&target, &self.shell_state.env_vars),
            &self.shell_state.env_vars,
        ));
        if redirect.append {
            let _ = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path);
        }
        // rubash#1903: the read builtin's own slurp of its redirect
        // target must go through the rendezvous for a WinuxCmd fifo
        // marker — plain-reading the target hands `read` the 7 marker
        // bytes and strands the writer (the #2014 append pin's
        // `read x < g` face).
        #[cfg(windows)]
        if let Some(result) =
            crate::winuxcmd_fifo::slurp_marker_bytes(&path, &self.shell_state.env_vars)
        {
            let input = match result {
                Ok(bytes) => crate::executor::substitution_metadata::bytes_to_shell_text(&bytes),
                Err(_) => return None,
            };
            self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                &input,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            );
            return Some(trim_read_input(
                input,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ));
        }
        // Same byte-preserving contract as read_io.rs for the shell-owned
        // descriptor path (GNU redir.c feeds raw bytes to the read owner).
        let input = match crate::executor::substitution_metadata::read_shell_input_file(&path) {
            Ok(text) => text,
            Err(_) => return None,
        };
        self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
            &input,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        );
        Some(trim_read_input(
            input,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        ))
    }

    pub(in crate::executor) fn read_heredoc_fd_input(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        let redirect = cmd
            .heredoc_redirects
            .iter()
            .rev()
            .find(|redirect| redirect.fd == Some(fd))?;
        self.read_heredoc_fd_input_entry(
            redirect,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        )
    }

    /// Entry-exact heredoc serve (rubash#2314): serve the body of
    /// the heredoc entry AT `index` in the command's own list. The
    /// list entry correlates with `heredoc_redirects` by ordinal
    /// among the same-fd, same-stringness document entries before
    /// it — the correlation materialize_async_stdin_body uses
    /// (compound_exec.rs).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::executor) fn read_heredoc_fd_input_at(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
        index: usize,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        let winner = cmd.redirects.get(index)?;
        let want_string = matches!(winner.kind, crate::parser::RedirectKind::HereString);
        let ordinal = cmd.redirects[..index]
            .iter()
            .filter(|entry| {
                matches!(
                    entry.kind,
                    crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString
                ) && matches!(entry.kind, crate::parser::RedirectKind::HereString) == want_string
                    && entry.fd == Some(fd)
            })
            .count();
        let redirect = cmd
            .heredoc_redirects
            .iter()
            .filter(|entry| entry.fd == Some(fd) && entry.here_string == want_string)
            .nth(ordinal)?;
        self.read_heredoc_fd_input_entry(
            redirect,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        )
    }

    fn read_heredoc_fd_input_entry(
        &mut self,
        redirect: &crate::parser::HereDocRedirect,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        let body = redirect.body.as_deref()?;
        if let Some(carrier) = &redirect.body_carrier {
            if let crate::parser::StdinBody::Preexpanded(text) = carrier {
                let mut input = text.clone();
                input.push('\n');
                self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                    &input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                );
                return Some(trim_read_input(
                    input,
                    delimiter,
                    char_limit,
                    exact_char_limit,
                    escape_aware,
                    raw,
                ));
            }
        }
        if let Some(word) = body.strip_prefix(STORAGE_WORD_PREFIX) {
            let mut input =
                decode_ansi_c_quoted_word(word).unwrap_or_else(|| self.expand_word(word));
            input.push('\n');
            self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
                &input,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            );
            return Some(trim_read_input(
                input,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            ));
        }
        let input = if let Some(pre) = preexpanded_stdin_body(body) {
            pre.to_string()
        } else {
            strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body)).to_string()
        };
        self.read_eof_no_delimiter |= slurped_record_eof_no_delimiter(
            &input,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        );
        Some(trim_read_input(
            input,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        ))
    }
}

/// Which source a command's OWN redirect list effectively binds on an
/// fd, for the in-process `read -u` serve (rubash#2305). GNU applies
/// the list in order before the builtin runs (redir.c:237
/// do_redirections; redir.c:423 here_document_to_fd materializes a
/// heredoc node's own body), so the LAST input-side entry naming the
/// fd is the descriptor the builtin reads — across kinds, not per
/// kind: `3<file 3<<EOF` reads the body, `3<<EOF 3<file` reads the
/// file, `3<<EOF` under an ambient `3<outer` reads the body. The
/// per-kind serves historically consulted each store in a fixed
/// order (own files, then the ambient table entry, then own heredocs
/// as a last resort), which let an ambient binding — or an earlier
/// own entry of a luckier kind — shadow the effective one.
pub(in crate::executor) enum OwnInputBinding {
    /// Input / ReadWrite entry: served by read_redirected_fd.
    File,
    /// HereDoc / HereString entry: served by read_heredoc_fd_input.
    Heredoc,
    /// DuplicateInput entry: serve the source fd's binding.
    Duplicate(u32),
    /// CloseInput entry: the command closed the fd for itself.
    Close,
}

impl Executor {
    /// The effective own-list binding for `fd`, or None when the
    /// command's own list does not bind it (the ambient descriptor
    /// state then governs, as before). Position-blind wrapper over
    /// own_input_binding_scan for callers that only need the kind.
    pub(in crate::executor) fn own_input_binding(
        &self,
        cmd: &CommandNode,
        fd: u32,
    ) -> Option<OwnInputBinding> {
        self.own_input_binding_scan(cmd, fd, None)
            .map(|(_position, binding)| binding)
    }

    /// Positioned own-list binding scan (rubash#2314): like
    /// own_input_binding, but the ordered walk stops BEFORE list
    /// position `before` (None = the whole list) and the winner's
    /// own index is returned alongside the binding. GNU's
    /// do_redirections (redir.c:237; the sequential loop at
    /// redir.c:252 walks the list in order) applies each entry
    /// against the descriptor state left by the entries before it,
    /// so a dup copies its source as bound AT THE DUP'S POSITION:
    /// `3<&4 4<inner` copies fd 4's pre-list binding, and in
    /// `3<&4 4<&3` the second dup sees fd 3 already rebound by the
    /// first. A whole-list (last-wins) resolution reads the
    /// source's FINAL binding instead and diverges on both shapes.
    ///
    /// `redirect_in` convention: it mirrors a list entry whose
    /// position is unrecoverable, so it is consulted only when the
    /// bounded ordered scan yields no binding at all (the ordered
    /// list is authoritative when both speak — read_redirected_fd's
    /// dual-store rule), and a binding sourced from it reports
    /// position None. A dup sourced from redirect_in resolves its
    /// source against the FULL list (bound None); every
    /// list-sourced dup reports its index, so chase positions
    /// strictly descend from there and the chase terminates.
    pub(in crate::executor) fn own_input_binding_scan(
        &self,
        cmd: &CommandNode,
        fd: u32,
        before: Option<usize>,
    ) -> Option<(Option<usize>, OwnInputBinding)> {
        let limit = before
            .unwrap_or(cmd.redirects.len())
            .min(cmd.redirects.len());
        let mut binding: Option<(usize, OwnInputBinding)> = None;
        for (index, redirect) in cmd.redirects[..limit].iter().enumerate() {
            if redirect.fd != Some(fd) || redirect.fd_var.is_some() {
                continue;
            }
            binding = match redirect.kind {
                crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite => {
                    Some((index, OwnInputBinding::File))
                }
                crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString => {
                    Some((index, OwnInputBinding::Heredoc))
                }
                crate::parser::RedirectKind::DuplicateInput => {
                    let target = self.expand_redirect_target(redirect);
                    redirect_target_fd_and_move(&target)
                        .map(|(source, _move)| (index, OwnInputBinding::Duplicate(source)))
                }
                crate::parser::RedirectKind::CloseInput => Some((index, OwnInputBinding::Close)),
                _ => continue,
            };
        }
        if let Some((index, binding)) = binding {
            return Some((Some(index), binding));
        }
        if let Some(redirect) = cmd
            .redirect_in
            .as_deref()
            .filter(|redirect| redirect.fd == Some(fd) && redirect.fd_var.is_none())
        {
            let binding = match redirect.kind {
                crate::parser::RedirectKind::Input | crate::parser::RedirectKind::ReadWrite => {
                    Some(OwnInputBinding::File)
                }
                crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString => {
                    Some(OwnInputBinding::Heredoc)
                }
                crate::parser::RedirectKind::DuplicateInput => {
                    let target = self.expand_redirect_target(redirect);
                    redirect_target_fd_and_move(&target)
                        .map(|(source, _move)| OwnInputBinding::Duplicate(source))
                }
                crate::parser::RedirectKind::CloseInput => Some(OwnInputBinding::Close),
                _ => None,
            };
            return binding.map(|binding| (None, binding));
        }
        None
    }

    /// Whether `read -u fd` can be served at all under the command's
    /// own list: a Close makes the descriptor invalid (GNU
    /// read.def:377-381 sh_validfd on the REDIRECTED state), a
    /// Duplicate defers to its source's availability, other bindings
    /// are available by construction. None (no own binding) defers to
    /// the caller's ambient checks.
    pub(in crate::executor) fn own_input_binding_availability(
        &self,
        cmd: &CommandNode,
        fd: u32,
    ) -> Option<bool> {
        match self.own_input_binding_scan(cmd, fd, None) {
            Some((_position, OwnInputBinding::Close)) => Some(false),
            Some((dup_position, OwnInputBinding::Duplicate(source))) => {
                // rubash#2314: the availability chase is positioned
                // like the serve chase — a cyclic dup pair
                // (`3<&4 4<&3`) is NOT a loop in GNU, because each
                // dup resolves its source at its own position and
                // the walk ends at the pre-list ambient state.
                let mut current = source;
                let mut bound = dup_position;
                let mut seen = std::collections::HashSet::new();
                loop {
                    if !seen.insert((current, bound)) {
                        return Some(false);
                    }
                    match self.own_input_binding_scan(cmd, current, bound) {
                        Some((_position, OwnInputBinding::Close)) => return Some(false),
                        Some((position, OwnInputBinding::Duplicate(next))) => {
                            current = next;
                            bound = position;
                        }
                        Some((_position, OwnInputBinding::File))
                        | Some((_position, OwnInputBinding::Heredoc)) => return Some(true),
                        None => return Some(self.fd_table.is_open_for_read(current)),
                    }
                }
            }
            Some((_position, OwnInputBinding::File))
            | Some((_position, OwnInputBinding::Heredoc)) => Some(true),
            None => None,
        }
    }

    /// Serve one record for a Duplicate own binding (`read -u 3 x
    /// 3<&4`): the dup copies the source's binding as of the dup's
    /// position in the list, so follow the source through the own
    /// list first and only then to the ambient descriptor state —
    /// never the target fd's own ambient entry (rubash#2305).
    /// rubash#2314: the chase is positioned — `dup_position` is the
    /// dup entry's own list index (None when sourced from
    /// redirect_in), each step resolves `current` against the
    /// entries BEFORE the current bound, and a list-sourced winner
    /// is served entry-exactly (the `_at` serves), because GNU's
    /// ordered walk (redir.c:237 do_redirections) has already
    /// fixed the descriptor the dup copied by the time any later
    /// entry rebinds the source.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::executor) fn read_dup_fd_input(
        &mut self,
        cmd: &CommandNode,
        source: u32,
        dup_position: Option<usize>,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
        escape_aware: bool,
        raw: bool,
    ) -> Option<String> {
        let mut current = source;
        let mut bound = dup_position;
        let mut seen = std::collections::HashSet::new();
        loop {
            if !seen.insert((current, bound)) {
                return None;
            }
            match self.own_input_binding_scan(cmd, current, bound) {
                Some((Some(index), OwnInputBinding::File)) => {
                    return self.read_redirected_fd_at(
                        cmd,
                        current,
                        index,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                }
                Some((None, OwnInputBinding::File)) => {
                    return self.read_redirected_fd(
                        cmd,
                        current,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                }
                Some((Some(index), OwnInputBinding::Heredoc)) => {
                    return self.read_heredoc_fd_input_at(
                        cmd,
                        current,
                        index,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                }
                Some((None, OwnInputBinding::Heredoc)) => {
                    return self.read_heredoc_fd_input(
                        cmd,
                        current,
                        delimiter,
                        char_limit,
                        exact_char_limit,
                        escape_aware,
                        raw,
                    );
                }
                Some((position, OwnInputBinding::Duplicate(next))) => {
                    current = next;
                    bound = position;
                }
                Some((_position, OwnInputBinding::Close)) => return None,
                None => break,
            }
        }
        if let Some(line) = self.read_virtual_fd_stdin(
            current,
            false,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        ) {
            return Some(line);
        }
        self.read_coproc_stdout(
            current,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        )
        .map(|output| {
            trim_read_input(
                output,
                delimiter,
                char_limit,
                exact_char_limit,
                escape_aware,
                raw,
            )
        })
    }
}
