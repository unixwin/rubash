//! redirection module.
//!
//! GNU Bash source ownership:
// - redir.c
// - redir.h

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
enum OutputTarget {
    Stdout,
    Stderr,
    /// GNU dup2 snapshot (rubash#170): an fd_table `Stdout` endpoint on any
    /// fd OTHER than 1 means "the process stdout / active capture object as
    /// bound" — a later rebinding of fd 1 (`exec > f` inside
    /// `( ... ) 2>&1`) must NOT move it, exactly as a dup2'd descriptor
    /// keeps pointing at the original open file description. Resolves at
    /// write time to the active capture or the raw process stdout, never
    /// through the live fd_table[1].
    ProcessStdout,
    /// ProcessStdout bound to the snapshot recorded at dup time
    /// (FdTable::stdout_alias_generation): the capture generation's buffer
    /// while a NESTED command substitution owns the active slot — the pipe
    /// the fd actually holds — or `None` for the real process stdout when
    /// it was bound outside any capture (rubash#223).
    ProcessStdoutAt(Option<usize>),
    /// The stderr twin of ProcessStdout (rubash#279): GNU
    /// redir.c:237 do_redirections applies `3>&2` left to right — the dup2
    /// copies fd 2's open file description AT THAT MOMENT, so a later
    /// `2>/dev/null` in the same list never moves fd 3 (ble.sh's
    /// `... 3>&2 ... &>/dev/null` init guard prints its diagnostic through
    /// exactly this). An fd_table `Stderr` endpoint on any fd OTHER than 2
    /// means "the process stderr object as bound" and must resolve to the
    /// raw stderr channel, never through the live fd_table[2].
    ProcessStderr,
    Null,
    CoprocStdin(u32),
    Path(String),
    /// GNU dup2 semantics: a write endpoint carried by fd_table refers to
    /// the same open file description — writes go through the live handle
    /// at the shared offset, not a fresh append-mode reopen. `N<>file`
    /// writes at offset 0 and `1>&6` shares fd 6's offset.
    SharedFile(Rc<FileFd>),
    Closed,
}

#[derive(Debug, Clone)]
struct OutputFdState {
    fds: HashMap<u32, OutputTarget>,
    saw_output_redirect: bool,
    redirect_failed: bool,
    /// Pipeline-routing mode: `OutputTarget::Stdout` stands for the pipe
    /// channel, not the process stdout. Diagnostics issued while resolving
    /// the element's redirects (a failed `>&N` dup after `2>&1`, say) are
    /// written to fd 2 — whose target may be the pipe — so writes to the
    /// Stdout target are captured here and merged into the routed pipe
    /// stream instead of escaping to real stdout.
    defer_stdout_writes: bool,
    deferred_stdout: std::cell::RefCell<Vec<u8>>,
}

impl Executor {
    /// A redirect injected from an enclosing compound
    /// (GROUP_REDIRECT_INJECTED_MARK) mirrors a redirection GNU applies once
    /// to the compound's real descriptors (redir.c:767-955
    /// do_redirection_internal). When the fd table already carries a File
    /// endpoint for that fd — the group scope's binding — the leaf must
    /// write through that shared open file description, not a fresh
    /// append-mode reopen whose offset starts at the file's end while the
    /// bound description's offset stays behind (or vice versa: a dup of the
    /// bound fd clobbers what append-channel leaves wrote). Returns true
    /// when the injected redirect is already realized by the live binding.
    pub(in crate::executor) fn injected_redirect_fd_is_bound(
        &self,
        redirect: &Redirect,
        fd: u32,
    ) -> bool {
        crate::executor::support_names::is_injected_group_redirect(redirect)
            && matches!(
                self.fd_table.write_endpoint(fd),
                Some(FdWriteEndpoint::File(_))
            )
    }

    /// GNU do_redirections (redir.c:230-260) applies a command's
    /// redirection list left to right and the FIRST failing entry aborts
    /// the command (exit 1) with its diagnostic. This is rubash's
    /// equivalent gate, run before execute_prepared_command: it rejects
    /// (returns true) for the whole error class do_redirection_internal
    /// can produce at APPLY time, so the command never runs — matching
    /// GNU's "command not executed, $? = 1" contract for every command
    /// type (builtin, external, function) alike:
    ///
    /// - AMBIGUOUS_REDIRECT (redir.c:911 / :784-843): a file-redirect word
    ///   that expands to ZERO fields (`> $unset`) or MORE THAN ONE field
    ///   (`> $v` with v="a b"). An EMPTY single field is NOT ambiguous —
    ///   a quoted null (`> ""`, `> "$unsetv"`) anchors one empty field
    ///   (execution_misc::raw_word_has_quoted_span), the open("")
    ///   proceeds and fails ENOENT (`: No such file or directory`,
    ///   redir.c:895-930 open + redirection_error default arm).
    /// - dup/EBADF shapes (redir.c:1115 dup2, redir.c:149-176
    ///   redirection_error): `<&N`/`>&N`/`>&N-` with fd N not open →
    ///   `N: Bad file descriptor`; `>& ""`/`>& "$unset"` (all_digits("")
    ///   vacuously true at general.c:233 but valid_number("") fails at
    ///   general.c:243 → dest -1) → raw-word (redirector 0/1) or
    ///   redirector-number EBADF text.
    /// - move-fd with a bad source (redir.c:1137-1143 add_undo_redirect of
    ///   the closed moved fd fails → sys_error line + `N: Bad file
    ///   descriptor`).
    /// - `<> word` (r_input_output, make_cmd.c:682 O_RDWR|O_CREAT): the
    ///   open CREATES the missing file; only a real open error rejects.
    ///
    /// Diagnostics are order-routed (issue #250 semantics): only the
    /// redirects BEFORE the failing entry have applied, so a preceding
    /// `2>/dev/null` silences and a following one leaks.
    pub(in crate::executor) fn reject_invalid_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        // Semantic identity of a redirect for this scan (the same key the
        // expand_redirect_target memo uses): the ordered `redirects` list
        // plus the legacy mirror fields may carry THE SAME source redirect
        // twice with cosmetic differences — re-validating a duplicate
        // would see the first copy's ordered fd effects (a move marks its
        // source fd closed) and misreport a dup that already succeeded.
        // GNU has exactly one do_redirections list (redir.c:246).
        let semantic_key = |redirect: &Redirect| {
            format!(
                "{:?}\x1f{}\x1f{:?}\x1f{}",
                redirect.kind, redirect.operator, redirect.fd, redirect.target
            )
        };
        let mut redirects = cmd.redirects.iter();
        let mut candidates = Vec::new();
        candidates.extend(redirects.by_ref());
        let mut seen_keys: std::collections::HashSet<String> = candidates
            .iter()
            .map(|redirect| semantic_key(redirect))
            .collect();
        for redirect in [
            cmd.redirect_in.as_ref(),
            cmd.redirect_out.as_ref(),
            cmd.append.as_ref(),
            cmd.redirect_err.as_ref(),
            cmd.redirect_err_append.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if seen_keys.insert(semantic_key(redirect)) {
                candidates.push(redirect);
            }
        }

        // GNU applies the list left to right (redir.c:246 do_redirections
        // for-loop); a dup only sees fds opened by redirects BEFORE it
        // (`cmd <&7 7<f` still fails — GNU opens in order). fd_table
        // carries the ambient state; `opened` overlays the fds this
        // command's earlier redirects opened/closed/duped for the scan.
        let mut opened: HashMap<u32, bool> = HashMap::new();

        for (index, redirect) in candidates.iter().enumerate() {
            if matches!(redirect.kind, crate::parser::RedirectKind::HereString) {
                continue;
            }
            let target = self.expand_redirect_target(redirect);
            let raw_word = redirect
                .target_metadata
                .raw
                .strip_prefix('&')
                .unwrap_or(&redirect.target_metadata.raw);
            let expanded_word = target.strip_prefix('&').unwrap_or(&target);
            let file_open_kind = matches!(
                redirect.kind,
                crate::parser::RedirectKind::Input
                    | crate::parser::RedirectKind::ReadWrite
                    | crate::parser::RedirectKind::Output
                    | crate::parser::RedirectKind::Append
                    | crate::parser::RedirectKind::ClobberOutput
                    | crate::parser::RedirectKind::CombinedOutput
                    | crate::parser::RedirectKind::CombinedAppend
            );
            let is_dup_kind = matches!(
                redirect.kind,
                crate::parser::RedirectKind::DuplicateInput
                    | crate::parser::RedirectKind::DuplicateOutput
            );
            // fd_var ({var}<>) redirects allocate dynamically at apply time
            // (redir.c redir_varassign); only their DUP forms carry
            // gate-visible errors — `{v}>&word` (non-digit word) is
            // AMBIGUOUS_REDIRECT reported under the VARIABLE name
            // (redir.c:137-138: the REDIR_VARASSIGN && error<0 branch uses
            // the redirector word), and `{v}>&N` with N closed is the
            // plain EBADF shape. File-open {var} forms need no gate check.
            if redirect.fd_var.is_some() && !is_dup_kind {
                continue;
            }
            let default_redirector = if redirect.kind == crate::parser::RedirectKind::DuplicateInput
            {
                0
            } else {
                1
            };
            let redirector = redirect.fd.unwrap_or(default_redirector);

            // ---- validation FIRST: a redirect never invalidates itself
            // (GNU applies the redirector's own dup2 before the move's
            // close, redir.c:1153-1166), so the ordered fd effects of THIS
            // entry are recorded only after its checks pass. ----
            if !(file_open_kind || is_dup_kind) {
                // Non-file, non-dup kinds (heredocs etc.) carry no fd
                // effects for the scan either.
                continue;
            }

            // ---- dup redirections: [N]<&WORD / [N]>&WORD ----------------
            if is_dup_kind {
                if is_closed_redirect_target(&target) {
                    opened.insert(redirector, false);
                    continue;
                }
                // {var}>&word: the varassign word rules (redir.c:784-843
                // with REDIR_VARASSIGN). A digit target still dups fd N
                // (the allocate-and-dup runs at redir_varassign); a
                // non-digit word never reaches the r_err_and_out arm
                // (redirector is a word, not dest 1) — AMBIGUOUS_REDIRECT
                // under the variable name.
                if let Some(name) = redirect.fd_var.as_deref() {
                    if let Some((fd, _move_fd)) = redirect_target_fd_and_move(&target) {
                        if !dup_source_fd_open(&self.fd_table, fd, &opened) {
                            // move_fd=true shape: the REDIR_VARASSIGN pre-dup
                            // (redir.c:1122-1127 fcntl F_DUPFD) sys_errors
                            // the same two-line diagnostic as the move undo.
                            return self
                                .reject_with_dup_bad_fd(cmd, index, fd, true, raw_word, redirector)
                                .map(|_| true);
                        }
                        continue;
                    }
                    if !expanded_word.chars().all(|ch| ch.is_ascii_digit())
                        || expanded_word
                            .parse::<i64>()
                            .ok()
                            .and_then(|value| u32::try_from(value).ok())
                            .is_none()
                    {
                        return self
                            .reject_redirect_at(cmd, index, &format!("{name}: ambiguous redirect"))
                            .map(|_| true);
                    }
                    continue;
                }
                // A dup-kind target WITHOUT the `&` marker is rubash's own
                // materialized form — command_with_process_substitution_
                // files rewrites an external command's `<&N` operand to the
                // temp file holding fd N's remaining bytes (the rewrite only
                // runs when fd N was open, so validation already happened).
                // GNU never sees such an operand; skip the word rules.
                if !target.starts_with('&') {
                    opened.insert(redirector, true);
                    continue;
                }
                // Digit forms `&N` / `&N-` (possibly via expansion).
                if let Some((fd, move_fd)) = redirect_target_fd_and_move(&target) {
                    if !dup_source_fd_open(&self.fd_table, fd, &opened) {
                        return self
                            .reject_with_dup_bad_fd(cmd, index, fd, move_fd, raw_word, redirector)
                            .map(|_| true);
                    }
                    // The dup itself succeeds (GNU dup2 at redir.c:1153);
                    // the command-scoped application sites own the binding
                    // and the move's close.
                    opened.insert(redirector, true);
                    if move_fd {
                        opened.insert(fd, false);
                    }
                    continue;
                }
                // Word forms (redir.c:784-843 TRANSLATE_REDIRECT).
                // Zero-field expansion → redirection_expand NULL →
                // AMBIGUOUS_REDIRECT reporting the RAW word.
                if expanded_word.is_empty() && !raw_word_has_quoted_span(raw_word) {
                    return self
                        .reject_redirect_at(cmd, index, &format!("{raw_word}: ambiguous redirect"))
                        .map(|_| true);
                }
                if expanded_word == "-" {
                    continue; // `>&$x` with x=- → close (redir.c:798-803)
                }
                if expanded_word.chars().all(|ch| ch.is_ascii_digit()) {
                    // all_digits("") is vacuously true (general.c:233) but
                    // valid_number("") fails (general.c:243) → dest -1 →
                    // dup2(-1) EBADF. A digit word that parses dups that fd.
                    match expanded_word
                        .parse::<i64>()
                        .ok()
                        .and_then(|value| u32::try_from(value).ok())
                    {
                        Some(fd) => {
                            if !dup_source_fd_open(&self.fd_table, fd, &opened) {
                                return self
                                    .reject_with_dup_bad_fd(
                                        cmd, index, fd, false, raw_word, redirector,
                                    )
                                    .map(|_| true);
                            }
                            opened.insert(redirector, true);
                        }
                        None => {
                            // dest -1: EBADF text per redir.c:149-176 — the
                            // raw word for redirector 0/1 dup-WORD
                            // instructions, the redirector number otherwise.
                            let name = dup_bad_fd_name(raw_word, redirector, redirector);
                            return self
                                .reject_redirect_at(
                                    cmd,
                                    index,
                                    &format!("{name}: Bad file descriptor"),
                                )
                                .map(|_| true);
                        }
                    }
                    continue;
                }
                // A source word ending in `-` never reaches the
                // r_err_and_out translation (that arm requires
                // r_duplicating_output_word, redir.c:832; the dash already
                // made make_redirection build r_move_output_word,
                // make_cmd.c:716) — a non-digit expansion is
                // AMBIGUOUS_REDIRECT reporting the expanded word with the
                // dash stripped (make_cmd.c:709-710 mutated the word).
                let move_word = raw_word.ends_with('-');
                // Non-digit, non-empty word: `>&word` with redirector 1 and
                // no varassign is r_err_and_out (redir.c:832-838) — a file
                // open, not a dup. Everything else is AMBIGUOUS_REDIRECT
                // reporting the EXPANDED word (redir.c:839-843 + :186-190).
                let dup_output_err_and_out = !move_word
                    && redirect.kind == crate::parser::RedirectKind::DuplicateOutput
                    && redirector == 1;
                if !dup_output_err_and_out {
                    let diagnostic_word = expanded_word
                        .strip_suffix('-')
                        .filter(|_| move_word)
                        .unwrap_or(expanded_word);
                    return self
                        .reject_redirect_at(
                            cmd,
                            index,
                            &format!("{diagnostic_word}: ambiguous redirect"),
                        )
                        .map(|_| true);
                }
                // r_err_and_out falls through to the file-open class with
                // the stripped word as filename.
            }

            // ---- file-open redirections --------------------------------
            if expanded_word.is_empty() {
                // Multi-field split or zero-field expansion: both are the
                // redirection_expand NULL condition (redir.c:325-333); an
                // anchored quoted-null single empty field is NOT.
                if redirect_target_is_ambiguous(&redirect.target_metadata.raw, &target)
                    || !raw_word_has_quoted_span(raw_word)
                {
                    return self
                        .reject_redirect_at(cmd, index, &format!("{raw_word}: ambiguous redirect"))
                        .map(|_| true);
                }
                // One anchored empty field: the open itself fails —
                // open("") → ENOENT on every flag combination, so the
                // diagnostic is deterministic without touching the disk.
                return self
                    .reject_redirect_at(cmd, index, ": No such file or directory")
                    .map(|_| true);
            }
            if redirect_target_is_ambiguous(&redirect.target_metadata.raw, &target) {
                return self
                    .reject_redirect_at(cmd, index, &format!("{raw_word}: ambiguous redirect"))
                    .map(|_| true);
            }
            // `<> word` opens O_RDWR|O_CREAT (make_cmd.c:682) — creating
            // the missing file is part of the redirection, not a consumer
            // side effect (rubash#264). Other open failures (bad
            // directory, permission) abort like GNU's redir_open.
            if redirect.kind == crate::parser::RedirectKind::ReadWrite
                && !is_null_device(&target)
                && dev_stdio_redirect_fd(&target).is_none()
                && !target.starts_with("<(")
            {
                let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
                if let Err(error) = FileFd::open_readwrite(path) {
                    let message = crate::posix_errors::path_error(&target, error);
                    let text = crate::posix_errors::message(&message);
                    return self
                        .reject_redirect_at(cmd, index, &format!("{target}: {text}"))
                        .map(|_| true);
                }
            }
            // Checks passed — record this entry's ordered fd effect for
            // later dups in the same list: a plain file open binds its
            // redirector, an fd-alias name (redir.c /dev/fd resolution)
            // dups the aliased fd's state.
            if is_closed_redirect_target(&target) {
                opened.insert(redirector, false);
            } else if let Some(source) = dev_stdio_redirect_fd(&target) {
                let open = dup_source_fd_open(&self.fd_table, source, &opened);
                opened.insert(redirector, open);
            } else {
                opened.insert(redirector, true);
            }
        }

        Ok(false)
    }

    /// The dup bad-fd rejection. GNU `>&7-`/`<&7-` with fd 7 closed fails
    /// in TWO steps (verified against WSL GNU 5.3.0, rubash#263/#266):
    /// redir.c:1137-1143's add_undo_redirect of the moved fd sys_errors
    /// `script: redirection error: cannot duplicate fd: ...` (error.c
    /// get_name_for_error prolog, no line segment), then
    /// REDIRECTION_ERROR returns EBADF and redirection_error
    /// (redir.c:149-158) prints the fd with the full prefix. Plain dups
    /// (`>&7`, redir.c:1153 dup2) report only the second line.
    fn reject_with_dup_bad_fd(
        &mut self,
        cmd: &CommandNode,
        index: usize,
        fd: u32,
        move_fd: bool,
        raw_word: &str,
        redirector: u32,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        if move_fd {
            writeln!(
                &mut stderr,
                "{}redirection error: cannot duplicate fd: Bad file descriptor",
                self.script_name_prefix()
            )?;
        }
        let name = dup_bad_fd_name(raw_word, redirector, fd);
        writeln!(
            &mut stderr,
            "{}{name}: Bad file descriptor",
            self.diagnostic_prefix()
        )?;
        self.write_redirect_diagnostic_at(cmd, index, &stderr)?;
        self.exit_code = 1;
        Ok(())
    }

    fn reject_redirect_at(
        &mut self,
        cmd: &CommandNode,
        index: usize,
        text: &str,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        writeln!(&mut stderr, "{}{text}", self.diagnostic_prefix())?;
        self.write_redirect_diagnostic_at(cmd, index, &stderr)?;
        self.exit_code = 1;
        Ok(())
    }

    /// Order-routed redirect diagnostic (issue #250 semantics): only the
    /// redirects strictly BEFORE the failing one have applied when GNU
    /// reports, so their fd-2 binding routes this message —
    /// `cmd 2>/dev/null > /missing` is silent, the reversed order leaks.
    fn write_redirect_diagnostic_at(
        &mut self,
        cmd: &CommandNode,
        index: usize,
        message: &[u8],
    ) -> Result<(), ExecuteError> {
        let mut state = self.command_output_fd_state();
        let mut prefix_cmd = cmd.clone();
        prefix_cmd.redirects.truncate(index);
        let _ = self.apply_ordered_output_redirects(&prefix_cmd, &mut state)?;
        state.write_to_fd(self, 2, message)
    }

    pub(in crate::executor) fn command_output_redirect_fails(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        let mut state = self.command_output_fd_state();
        if !self.apply_ordered_output_redirects(cmd, &mut state)? {
            return Ok(false);
        }
        Ok(state.redirect_failed)
    }

    /// GNU redir.c:135 redirection_error, reached from redir.c:260
    /// do_redirections: the redirection list applies left-to-right, and a
    /// failed open reports through the fd 2 binding the redirects BEFORE
    /// the failing one already installed — `cat 2>/dev/null < /missing` is
    /// silent (fd 2 already points at /dev/null) while the reversed
    /// `< /missing 2>/dev/null` leaks (the open fails before the `2>`
    /// applies). `message` is the already-prefixed diagnostic; when its
    /// `{target}: ` head matches an input-side redirect of `cmd`, the fd
    /// state resolves only the redirects preceding it, otherwise the full
    /// ordered state is used (the failing step is not an input open, so
    /// every redirection is applied by then). Issue #250: rubash used to
    /// write these diagnostics to the shell default stderr (eprintln /
    /// write_default_stderr), bypassing an applied `2>/dev/null` /
    /// `2>file` / comsub-captured fd 2.
    pub(in crate::executor) fn write_redirect_diagnostic_routed(
        &mut self,
        cmd: &CommandNode,
        message: &[u8],
    ) -> Result<(), ExecuteError> {
        let failed_index = cmd.redirects.iter().position(|redirect| {
            if !matches!(
                redirect.kind,
                crate::parser::RedirectKind::Input | crate::parser::RedirectKind::DuplicateInput
            ) || redirect.fd.is_some_and(|fd| fd != 0)
            {
                return false;
            }
            let target = self.expand_redirect_target(redirect);
            if super::execution_misc::is_closed_redirect_target(&target)
                || redirect_target_fd(&target).is_some()
            {
                return false;
            }
            let head = format!("{target}: ");
            let prefix = self.diagnostic_prefix();
            std::str::from_utf8(message).is_ok_and(|text| {
                text.strip_prefix(prefix.as_str())
                    .is_some_and(|rest| rest.starts_with(&head))
            })
        });
        let mut state = self.command_output_fd_state();
        match failed_index {
            Some(index) => {
                // Only the redirects strictly before the failing input open
                // have taken effect (GNU do_redirections stops at the error).
                let mut prefix_cmd = cmd.clone();
                prefix_cmd.redirects.truncate(index);
                let _ = self.apply_ordered_output_redirects(&prefix_cmd, &mut state)?;
            }
            None => {
                let _ = self.apply_ordered_output_redirects(cmd, &mut state)?;
            }
        }
        state.write_to_fd(self, 2, message)
    }

    pub(in crate::executor) fn write_ordered_command_output(
        &mut self,
        cmd: &CommandNode,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<bool, ExecuteError> {
        self.last_builtin_write_failed.set(false);
        let mut state = self.command_output_fd_state();
        if !self.apply_ordered_output_redirects(cmd, &mut state)? {
            // A persistent `exec >&N-` closes the shell fd without adding a
            // redirect to the current command. Buffered builtins still need
            // Bash's write diagnostic instead of silently dropping output.
            if !stdout.is_empty()
                && state
                    .fd_target(1)
                    .is_some_and(|target| *target == OutputTarget::Closed)
            {
                if builtin_output_write_is_unchecked(cmd) {
                    return Ok(true);
                }
                self.write_bad_output_fd_diagnostic(cmd, 1)?;
                self.exit_code = 1;
                self.last_builtin_write_failed.set(true);
                return Ok(true);
            }
            if !stderr.is_empty()
                && state
                    .fd_target(2)
                    .is_some_and(|target| *target == OutputTarget::Closed)
            {
                self.write_bad_output_fd_diagnostic(cmd, 2)?;
                self.exit_code = 1;
                self.last_builtin_write_failed.set(true);
                return Ok(true);
            }
            if !stdout.is_empty()
                && matches!(state.fd_target(1), Some(OutputTarget::CoprocStdin(_)))
            {
                self.write_state_output_or_diagnostic(cmd, &state, 1, stdout)?;
                return Ok(true);
            }
            if !stderr.is_empty()
                && matches!(state.fd_target(2), Some(OutputTarget::CoprocStdin(_)))
            {
                self.write_state_output_or_diagnostic(cmd, &state, 2, stderr)?;
                return Ok(true);
            }
            return Ok(false);
        }
        if state.redirect_failed {
            return Ok(true);
        }

        if !stdout.is_empty()
            && state
                .fd_target(1)
                .is_some_and(|target| *target == OutputTarget::Closed)
        {
            if builtin_output_write_is_unchecked(cmd) {
                return Ok(true);
            }
            self.write_bad_output_fd_diagnostic(cmd, 1)?;
            self.exit_code = 1;
            self.last_builtin_write_failed.set(true);
            return Ok(true);
        }
        if !stderr.is_empty()
            && state
                .fd_target(2)
                .is_some_and(|target| *target == OutputTarget::Closed)
        {
            self.write_bad_output_fd_diagnostic(cmd, 2)?;
            self.exit_code = 1;
            self.last_builtin_write_failed.set(true);
            return Ok(true);
        }

        self.write_state_output_or_diagnostic(cmd, &state, 1, stdout)?;
        self.write_state_output_or_diagnostic(cmd, &state, 2, stderr)?;
        Ok(true)
    }

    /// Builtin wrappers that return their status (`Ok(status)` after a
    /// buffered write) must merge the write-failure flag the way GNU's
    /// `sh_chkwrite` folds a flush error into the builtin's return value
    /// (builtins/common.c:320-334). Returns and clears the flag.
    pub(in crate::executor) fn take_builtin_write_failed(&mut self) -> bool {
        self.last_builtin_write_failed.replace(false)
    }

    /// GNU execute_cmd.c execute_simple_command runs `expand_words`
    /// (execute_cmd.c:4617) BEFORE the command's own `do_redirections`
    /// (execute_builtin_or_function at execute_cmd.c:5606, or the forked-child
    /// path at execute_cmd.c:5522). A word-expansion diagnostic
    /// (`${x?word}`, bad substitution) therefore writes to fd 2 as bound by
    /// the *enclosing* context only — a `{ }`/`( )` compound redirect or the
    /// pipeline dup — never to the failing command's own `2>file`/`2>&1`.
    /// The fd state is seeded from the ambient fd table, which already
    /// carries those enclosing bindings.
    pub(in crate::executor) fn write_redirected_command_stderr(
        &mut self,
        cmd: &CommandNode,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        let _ = cmd;
        let state = self.command_output_fd_state();
        state.write_to_fd(self, 2, output)
    }

    fn write_state_output_or_diagnostic(
        &mut self,
        cmd: &CommandNode,
        state: &OutputFdState,
        fd: u32,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        match state.write_to_fd(self, fd, output) {
            Ok(()) => Ok(()),
            Err(error) if is_closed_output_error(&error) => {
                if fd == 1 && builtin_output_write_is_unchecked(cmd) {
                    return Ok(());
                }
                self.write_bad_output_fd_diagnostic(cmd, fd)?;
                self.exit_code = 1;
                self.last_builtin_write_failed.set(true);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(in crate::executor) fn command_needs_ordered_output_capture(
        &self,
        cmd: &CommandNode,
    ) -> bool {
        cmd.redirects.iter().any(|redirect| {
            matches!(
                redirect.kind,
                crate::parser::RedirectKind::DuplicateOutput
                    | crate::parser::RedirectKind::CloseOutput
            )
        })
    }

    fn command_output_fd_state(&self) -> OutputFdState {
        let mut state = OutputFdState {
            fds: HashMap::new(),
            saw_output_redirect: false,
            redirect_failed: false,
            defer_stdout_writes: false,
            deferred_stdout: std::cell::RefCell::new(Vec::new()),
        };
        state.fds.insert(1, OutputTarget::Stdout);
        state.fds.insert(2, OutputTarget::Stderr);

        // FdTable is the semantic source of truth for every output endpoint.
        for (fd, entry) in &self.fd_table.entries {
            let target = if entry.closed || entry.write.is_none() {
                OutputTarget::Closed
            } else {
                match entry.write.as_ref().expect("checked above") {
                    FdWriteEndpoint::Stdout => {
                        // fd 1's live alias stays live (an `exec > f` inside
                        // the body must retarget plain stdout writes); every
                        // OTHER fd's Stdout endpoint is a dup2 snapshot of
                        // the original stdout object (rubash#170) — carrying
                        // the binding recorded at dup time when one exists
                        // (rubash#223).
                        if *fd == 1 {
                            OutputTarget::Stdout
                        } else if let Some(record) = self.fd_table.stdout_alias_generation.get(fd) {
                            OutputTarget::ProcessStdoutAt(*record)
                        } else {
                            OutputTarget::ProcessStdout
                        }
                    }
                    FdWriteEndpoint::Stderr => {
                        // fd 2's live alias stays live (an `exec 2> f` inside
                        // the body must retarget plain stderr writes); every
                        // OTHER fd's Stderr endpoint is a dup2 snapshot of
                        // the original stderr object (rubash#279), mirroring
                        // the Stdout/ProcessStdout split above.
                        if *fd == 2 {
                            OutputTarget::Stderr
                        } else {
                            OutputTarget::ProcessStderr
                        }
                    }
                    FdWriteEndpoint::File(file_fd) => {
                        let path = shell_display_path(&file_fd.path.to_string_lossy());
                        if is_null_device(&path) {
                            OutputTarget::Null
                        } else {
                            OutputTarget::SharedFile(file_fd.clone())
                        }
                    }
                    FdWriteEndpoint::CoprocStdin { pid, .. } => OutputTarget::CoprocStdin(*pid),
                    FdWriteEndpoint::ProcessSubstitution { path, .. } => {
                        OutputTarget::Path(path.to_string_lossy().into_owned())
                    }
                }
            };
            state.fds.insert(*fd, target);
        }

        state
    }

    fn apply_ordered_output_redirects(
        &mut self,
        cmd: &CommandNode,
        state: &mut OutputFdState,
    ) -> Result<bool, ExecuteError> {
        for redirect in &cmd.redirects {
            match redirect.kind {
                crate::parser::RedirectKind::Output
                | crate::parser::RedirectKind::Append
                | crate::parser::RedirectKind::ClobberOutput => {
                    let fd = redirect_fd_or_default(redirect, 1);
                    let target = self.expand_redirect_target(redirect);
                    // See injected_redirect_fd_is_bound: an injected group
                    // redirect is already realized when the seeded fd holds
                    // the group's shared File binding (the seeded state maps
                    // fd-table File endpoints to SharedFile one-to-one).
                    if self.injected_redirect_fd_is_bound(redirect, fd)
                        && matches!(state.fds.get(&fd), Some(OutputTarget::SharedFile(_)))
                    {
                        state.saw_output_redirect = true;
                        continue;
                    }
                    if let Some(source_fd) = redirect_target_fd(&target) {
                        let Some(source_target) = state.fd_target(source_fd).cloned() else {
                            self.write_bad_fd_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        };
                        if source_target == OutputTarget::Closed {
                            self.write_bad_fd_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        }
                        // A dup of the LIVE stderr alias must freeze the
                        // original stderr object (rubash#279): `3>&2
                        // 2>/dev/null` keeps fd 3 on the original stderr.
                        // fd 2 itself keeps the live marker so a plain
                        // `2>&2` stays a no-op alias.
                        let source_target = if fd != 2 && source_target == OutputTarget::Stderr {
                            OutputTarget::ProcessStderr
                        } else {
                            source_target
                        };
                        state.fds.insert(fd, source_target);
                        state.saw_output_redirect = true;
                        continue;
                    }
                    self.open_command_output_target(state, fd, &target, redirect)?;
                }
                crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => {
                    let target = self.expand_redirect_target(redirect);
                    self.open_command_output_target(state, 1, &target, redirect)?;
                    let stdout_target = state.fd_target(1).cloned().unwrap_or(OutputTarget::Stdout);
                    state.fds.insert(2, stdout_target);
                }
                crate::parser::RedirectKind::DuplicateOutput => {
                    let target_fd = redirect_fd_or_default(redirect, 1);
                    let target = self.expand_redirect_target(redirect);
                    if is_closed_redirect_target(&target) {
                        state.fds.insert(target_fd, OutputTarget::Closed);
                        state.saw_output_redirect = true;
                        continue;
                    }
                    // `>&N` and the move form `>&N-` (make_cmd.c:704-718 →
                    // redir.c r_move_output) both resolve fd N's write
                    // target into the redirector. This state map is
                    // per-command transient and GNU's move close is undone
                    // when the command ends (RX_UNDOABLE), so a plain copy
                    // matches the command's observable writes (`exec 5>&1;
                    // echo x >&5-` writes stdout, fd 5 still open after).
                    if let Some((source_fd, _move_source)) = redirect_target_fd_and_move(&target) {
                        let Some(source_target) = state.fd_target(source_fd).cloned() else {
                            self.write_bad_fd_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        };
                        if source_target == OutputTarget::Closed {
                            self.write_bad_fd_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        }
                        // Same live-alias freeze as the file arm above
                        // (rubash#279): `>&2 2>/dev/null` in one command
                        // list keeps the first dup on the original stderr.
                        let source_target =
                            if target_fd != 2 && source_target == OutputTarget::Stderr {
                                OutputTarget::ProcessStderr
                            } else {
                                source_target
                            };
                        state.fds.insert(target_fd, source_target);
                        state.saw_output_redirect = true;
                        continue;
                    }

                    if target_fd == 1 {
                        let path = target.strip_prefix('&').unwrap_or(&target);
                        // Empty `>&word`/`1>&word` expansion is an ambiguous
                        // redirect (redirection_expand returned NULL); the
                        // precheck normally catches it, keep the invariant
                        // here for paths that bypass it.
                        if path.is_empty() {
                            self.write_ambiguous_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        }
                        self.open_command_output_target(state, target_fd, path, redirect)?;
                        // GNU redir.c:832-838: `>&word` and `1>&word` alike
                        // (redirector == 1, no varassign) become
                        // r_err_and_out — stderr follows stdout to word.
                        if redirect.fd_var.is_none() {
                            let stdout_target =
                                state.fd_target(1).cloned().unwrap_or(OutputTarget::Stdout);
                            state.fds.insert(2, stdout_target);
                        }
                    } else {
                        // report_ambiguous_redirect (redir.c:846-870): dup
                        // redirections report the expanded fd word when
                        // non-empty and the raw word when expansion is
                        // empty (see the precheck note above).
                        if target.strip_prefix('&').unwrap_or(&target).is_empty() {
                            self.write_ambiguous_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                        } else {
                            self.write_ambiguous_redirect_diagnostic(state, &target)?;
                        }
                        self.exit_code = 1;
                        state.redirect_failed = true;
                        return Ok(true);
                    }
                }
                crate::parser::RedirectKind::CloseOutput => {
                    state
                        .fds
                        .insert(redirect_fd_or_default(redirect, 1), OutputTarget::Closed);
                    state.saw_output_redirect = true;
                }
                _ => {}
            }
        }

        Ok(state.saw_output_redirect)
    }

    fn open_command_output_target(
        &self,
        state: &mut OutputFdState,
        fd: u32,
        target: &str,
        redirect: &Redirect,
    ) -> Result<(), ExecuteError> {
        if is_closed_redirect_target(target) {
            state.fds.insert(fd, OutputTarget::Closed);
        } else if is_null_device(target) {
            state.fds.insert(fd, OutputTarget::Null);
        } else {
            let target = self.redirect_output_path_target(target);
            if redirect.append {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&target, &self.shell_state.env_vars))?;
            } else {
                self.create_redirect_output(&target, redirect.clobber)?;
            }
            state.fds.insert(fd, OutputTarget::Path(target));
        }
        state.saw_output_redirect = true;
        Ok(())
    }

    fn write_bad_fd_redirect_diagnostic(
        &mut self,
        state: &OutputFdState,
        source_target: &str,
    ) -> Result<(), ExecuteError> {
        let mut stderr = Vec::new();
        let display_target = source_target.strip_prefix('&').unwrap_or(source_target);
        // GNU reports a failed dup2 of `>&N` as EBADF ("Bad file
        // descriptor"), but an unopened `/dev/fd/N` (or /dev/std*) path
        // fails at open() with ENOENT ("No such file or directory") —
        // redir.c redirection_error uses the syscall errno (niubash#118).
        let reason =
            if crate::executor::execution_misc::dev_stdio_redirect_fd(display_target).is_some() {
                "No such file or directory"
            } else {
                "Bad file descriptor"
            };
        writeln!(
            &mut stderr,
            "{}{display_target}: {reason}",
            self.diagnostic_prefix()
        )?;
        state.write_to_fd(self, 2, &stderr)
    }

    fn write_ambiguous_redirect_diagnostic(
        &mut self,
        state: &OutputFdState,
        target: &str,
    ) -> Result<(), ExecuteError> {
        let target = target.strip_prefix('&').unwrap_or(target);
        let mut stderr = Vec::new();
        writeln!(
            &mut stderr,
            "{}{target}: ambiguous redirect",
            self.diagnostic_prefix()
        )?;
        state.write_to_fd(self, 2, &stderr)
    }

    /// GNU execute_cmd.c execute_pipeline + redir.c do_redirection_internal:
    /// inside each pipeline element's subshell the pipe is bound to fd 1
    /// first and the element's own redirections run after it — `cmd >f |
    /// next` sends the element's bytes to f and hands next an empty pipe,
    /// `cmd 1>&2 | next` sends them to stderr, `cmd 2>&1 | next` merges
    /// stderr into the pipe. Sequential stages capture their raw fd-1/fd-2
    /// streams; this resolves the element's output redirects (fd 1 seeded
    /// as the pipe) and routes the captures to the same targets. Compound
    /// stages applied their redirections internally and skip this.
    /// Returns true when the stage carried output redirects to resolve.
    pub(in crate::executor) fn route_pipeline_stage_streams(
        &mut self,
        cmd: &CommandNode,
        stdout: &mut String,
        stderr: &mut String,
        status: &mut i32,
    ) -> Result<bool, ExecuteError> {
        if cmd.redirect_out.is_none()
            && cmd.append.is_none()
            && cmd.redirect_err.is_none()
            && cmd.redirect_err_append.is_none()
            && !cmd
                .redirects
                .iter()
                .any(|redirect| redirect.is_output_side())
        {
            return Ok(false);
        }
        let mut state = self.command_output_fd_state();
        // The element's fd 1 starts bound to the pipe/output channel
        // regardless of the ambient fd 1 binding; fd 2 stays ambient.
        state.fds.insert(1, OutputTarget::Stdout);
        state.defer_stdout_writes = true;
        if !self.apply_ordered_output_redirects(cmd, &mut state)? {
            return Ok(false);
        }
        let deferred_stdout = std::mem::take(&mut *state.deferred_stdout.borrow_mut());
        if state.redirect_failed {
            stdout.clear();
            stderr.clear();
            stdout.push_str(&String::from_utf8_lossy(&deferred_stdout));
            *status = 1;
            return Ok(true);
        }
        // `cmd 2>&1 >f` resolves fd 2 to the pipe while `cmd >f 2>&1`
        // resolves it to the file, so fd 1's stream routes first and an
        // fd2-to-Stdout merge lands in the post-redirect pipe content.
        let stdout_target = state.fds.get(&1).cloned().unwrap_or(OutputTarget::Stdout);
        let stderr_target = state.fds.get(&2).cloned().unwrap_or(OutputTarget::Stderr);
        match &stdout_target {
            OutputTarget::Stdout => {}
            OutputTarget::Stderr => {
                let taken = std::mem::take(stdout);
                stderr.push_str(&taken);
            }
            OutputTarget::ProcessStderr => {
                // dup2 snapshot of the original stderr (rubash#279): route
                // straight to the raw channel, not the (possibly rebound)
                // fd 2 staging.
                let taken = std::mem::take(stdout);
                let bytes = crate::executor::substitution_metadata::shell_text_to_raw_bytes(&taken);
                super::shell_options::write_stderr_bytes(&bytes)?;
            }
            target => {
                let target = target.clone();
                let taken = std::mem::take(stdout);
                self.write_stage_stream_to_target(cmd, 1, &target, &taken, stderr, status)?;
            }
        }
        match &stderr_target {
            OutputTarget::Stderr => {}
            OutputTarget::Stdout => {
                let taken = std::mem::take(stderr);
                stdout.push_str(&taken);
            }
            OutputTarget::ProcessStderr => {
                let taken = std::mem::take(stderr);
                let bytes = crate::executor::substitution_metadata::shell_text_to_raw_bytes(&taken);
                super::shell_options::write_stderr_bytes(&bytes)?;
            }
            target => {
                let target = target.clone();
                let taken = std::mem::take(stderr);
                self.write_stage_stream_to_target(cmd, 2, &target, &taken, stdout, status)?;
            }
        }
        Ok(true)
    }

    /// Writes one captured stage stream to a non-stdio resolved target.
    /// Stdout/Stderr are handled by the caller (pipe merge vs ambient
    /// channel); a Closed target reports the write error the way GNU's
    /// flushed-write check does (builtins/common.c:320 sh_chkwrite), into
    /// `diag` — the opposite stream, which the caller still routes through
    /// the resolved state afterwards.
    fn write_stage_stream_to_target(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
        target: &OutputTarget,
        output: &str,
        diag: &mut String,
        status: &mut i32,
    ) -> Result<(), ExecuteError> {
        let bytes = crate::executor::substitution_metadata::shell_text_to_raw_bytes(output);
        match target {
            OutputTarget::Stdout | OutputTarget::Stderr => {}
            OutputTarget::ProcessStderr => {
                // dup2 snapshot of the original stderr object (rubash#279).
                super::shell_options::write_stderr_bytes(&bytes)?;
            }
            OutputTarget::ProcessStdout => {
                // Snapshot of the original stdout object (rubash#170): the
                // stream goes to the default stdout resolution — active
                // capture or raw process stdout — never a rebound fd 1.
                if crate::executor::shell_options::stdout_capture_active() {
                    crate::executor::shell_options::stdout_capture_write(&bytes)?;
                } else if let Some(capture) = self.stdout_capture.as_mut() {
                    use std::io::Write;
                    capture.write_all(&bytes)?;
                } else {
                    write_stdout_bytes(&bytes)?;
                }
            }
            OutputTarget::ProcessStdoutAt(record) => {
                // dup2 snapshot with the recorded binding (rubash#223):
                // route to that capture generation's buffer, or the real
                // process stdout when bound outside any capture.
                match record {
                    Some(generation) => {
                        let _ = crate::executor::shell_options::write_stdout_capture_at_generation(
                            &bytes,
                            *generation,
                        );
                    }
                    None => {
                        write_stdout_bytes(&bytes)?;
                    }
                }
            }
            OutputTarget::Null => {}
            OutputTarget::Closed => {
                if !output.is_empty() && !(fd == 1 && builtin_output_write_is_unchecked(cmd)) {
                    let command = cmd.words.first().map(String::as_str).unwrap_or("command");
                    diag.push_str(&format!(
                        "{}{command}: write error: Bad file descriptor\n",
                        self.diagnostic_prefix()
                    ));
                    self.exit_code = 1;
                    self.last_builtin_write_failed.set(true);
                    *status = 1;
                }
            }
            OutputTarget::CoprocStdin(coproc_fd) => {
                if let Some(pipe) = self.coproc_write_file(*coproc_fd) {
                    crate::fd::write_all(pipe.handle, &bytes)?;
                }
            }
            OutputTarget::Path(path) => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(path, &self.shell_state.env_vars))?;
                file.write_all(&bytes)?;
            }
            OutputTarget::SharedFile(file) => {
                crate::fd::write_all(file.handle, &bytes)?;
            }
        }
        Ok(())
    }

    fn write_bad_output_fd_diagnostic(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
    ) -> Result<(), ExecuteError> {
        let command = cmd.words.first().map(String::as_str).unwrap_or("command");
        let mut stderr = Vec::new();
        writeln!(
            &mut stderr,
            "{}{command}: write error: Bad file descriptor",
            self.diagnostic_prefix()
        )?;
        if fd == 2 {
            self.write_default_stdout(&stderr)
        } else {
            self.write_default_stderr(&stderr)
        }
    }
}

/// GNU builtins route their exit status through `sh_chkwrite`
/// (builtins/common.c:320), which reports `write error` and fails the
/// builtin when stdout is closed — but only on code paths that call it.
/// `help` with no topic operands returns `EXECUTION_SUCCESS` directly
/// (builtins/help.def:114-118: `if (list == 0) { ...; return
/// (EXECUTION_SUCCESS); }`), so `help >&-` is silent with status 0 while
/// `help topic >&-` reports the write error. Mirror that per-path split.
fn builtin_output_write_is_unchecked(cmd: &CommandNode) -> bool {
    if cmd.words.first().map(String::as_str) != Some("help") {
        return false;
    }
    // After internal_getopt consumes `-dms` options, `list == 0' means no
    // topic operands remain (options themselves still count as args here,
    // so reject any word that is not a recognized option cluster).
    // internal_getopt stops at `--' or the first non-option word; only
    // `-dms' clusters optionally closed by a trailing `--' leave `list == 0'.
    let mut args = cmd.words[1..].iter();
    loop {
        match args.next() {
            None => return true,
            Some(word) if word == "--" => return args.next().is_none(),
            Some(word)
                if word.starts_with('-')
                    && word.len() > 1
                    && word[1..].chars().all(|ch| matches!(ch, 'd' | 'm' | 's')) =>
            {
                continue
            }
            _ => return false,
        }
    }
}

fn redirect_fd_or_default(redirect: &Redirect, default_fd: u32) -> u32 {
    redirect.fd.unwrap_or_else(|| {
        redirect
            .operator
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(default_fd)
    })
}

impl OutputFdState {
    fn fd_target(&self, fd: u32) -> Option<&OutputTarget> {
        self.fds.get(&fd).or_else(|| match fd {
            1 => self.fds.get(&1),
            2 => self.fds.get(&2),
            _ => None,
        })
    }

    fn write_to_fd(
        &self,
        executor: &mut Executor,
        fd: u32,
        output: &[u8],
    ) -> Result<(), ExecuteError> {
        if output.is_empty() {
            return Ok(());
        }

        match self.fd_target(fd).cloned().unwrap_or(match fd {
            2 => OutputTarget::Stderr,
            _ => OutputTarget::Stdout,
        }) {
            OutputTarget::Stdout => {
                if self.defer_stdout_writes {
                    self.deferred_stdout.borrow_mut().extend_from_slice(output);
                    Ok(())
                } else {
                    executor.write_default_stdout(output)
                }
            }
            OutputTarget::ProcessStdout => {
                // Snapshot of the original stdout object (see the variant):
                // resolve exactly like write_fd_endpoint's Stdout arm —
                // active thread capture first, then the executor's field
                // capture, then the raw process stdout — but NEVER through
                // the live fd_table[1], which `exec > f` may have rebound.
                if self.defer_stdout_writes {
                    self.deferred_stdout.borrow_mut().extend_from_slice(output);
                    Ok(())
                } else if crate::executor::shell_options::stdout_capture_active() {
                    crate::executor::shell_options::stdout_capture_write(output)?;
                    Ok(())
                } else if let Some(capture) = executor.stdout_capture.as_mut() {
                    use std::io::Write;
                    capture.write_all(output)?;
                    Ok(())
                } else {
                    write_stdout_bytes(output)?;
                    Ok(())
                }
            }
            OutputTarget::ProcessStdoutAt(record) => {
                // dup2 snapshot with the recorded binding (rubash#223):
                // route to that capture generation's buffer, or the real
                // process stdout when bound outside any capture.
                match record {
                    Some(generation) => {
                        let _ = crate::executor::shell_options::write_stdout_capture_at_generation(
                            output, generation,
                        );
                    }
                    None => {
                        write_stdout_bytes(output)?;
                    }
                }
                Ok(())
            }
            OutputTarget::Stderr => executor.write_default_stderr(output),
            OutputTarget::ProcessStderr => {
                // dup2 snapshot of the original stderr object (rubash#279):
                // the raw stderr channel, never the live fd_table[2].
                super::shell_options::write_stderr_bytes(output)?;
                Ok(())
            }
            OutputTarget::Null | OutputTarget::Closed => Ok(()),
            OutputTarget::CoprocStdin(fd) => {
                if let Some(pipe) = executor.coproc_write_file(fd) {
                    crate::fd::write_all(pipe.handle, output)?;
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "coprocess input is closed",
                    )
                    .into());
                }
                Ok(())
            }
            OutputTarget::Path(path) => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(shell_path_to_windows(&path, &executor.shell_state.env_vars))?;
                file.write_all(output)?;
                Ok(())
            }
            OutputTarget::SharedFile(file) => {
                crate::fd::write_all(file.handle, output)?;
                Ok(())
            }
        }
    }
}

fn is_closed_output_error(error: &ExecuteError) -> bool {
    matches!(
        error,
        ExecuteError::IoError(error)
            if error.kind() == std::io::ErrorKind::BrokenPipe
                || error.raw_os_error() == Some(232)
    )
}

/// Whether fd `fd` is dup2-able in the state the scan has reached: the
/// overlay decides first (fds this command's earlier redirects opened or
/// closed), then the fd table; an absent entry for fds 0-2 is the
/// process's implicit stdio — always open, exactly the fallback
/// open_compound_output_redirects documents for its own dups.
fn dup_source_fd_open(table: &FdTable, fd: u32, opened: &HashMap<u32, bool>) -> bool {
    if let Some(is_open) = opened.get(&fd) {
        return *is_open;
    }
    if table.has_entry(fd) {
        !table.is_closed(fd)
    } else {
        fd <= 2
    }
}

/// GNU redir.c:149-176 redirection_error EBADF filename: a pure-digit
/// source operand came through the NUMBER grammar (`>&7` →
/// r_duplicating_output → itos(redirectee.dest)); every other source is a
/// dup-WORD instruction — redirectors 0/1 report the operand word as
/// written (make_redirection's dash strip at make_cmd.c:709-710 mutates
/// `7-` to `7` first), other redirectors report itos(redirector).
/// `fallback` stands in for the dest when no fd parsed (dest -1).
fn dup_bad_fd_name(raw_word: &str, redirector: u32, fallback: u32) -> String {
    let base = raw_word.strip_suffix('-').unwrap_or(raw_word);
    if !base.is_empty() && base.chars().all(|ch| ch.is_ascii_digit()) && raw_word == base {
        fallback.to_string()
    } else if redirector == 0 || redirector == 1 {
        base.to_string()
    } else {
        redirector.to_string()
    }
}
