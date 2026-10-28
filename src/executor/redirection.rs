//! redirection module.
//!
//! GNU Bash source ownership:
// - redir.c
// - redir.h

use super::*;

/// How a redirect-list rejection classifies, decided by the walker and
/// consumed by the dispatch sites (rubash#535).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::executor) enum RedirectRejection {
    /// A dup/ambiguous/open failure: the command is abandoned with status 1
    /// (GNU redirection_error), the script keeps running.
    Command,
    /// An unbound variable in a redirect word: a fatal word expansion error
    /// whose containment follows the target dispatch — the main shell for
    /// builtin/function targets (the noninteractive script ends with the
    /// mode's fatal status), a forked child for external targets (the
    /// script keeps running). The diagnostic is already reported and the
    /// status already set when the walker returns this.
    FatalNounset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::executor) enum OutputTarget {
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
    /// True only for the buffered-builtin route (a builtin running in
    /// THIS shell): GNU redir.c add_undo_redirect registers a move's
    /// source close for undo only when the target fd was open at apply
    /// time; when the target was closed the close is NOT undone, so
    /// `echo x 5>&3-` with fd 5 closed leaves fd 3 closed after the
    /// builtin (rubash#1235). Probe/diagnostic walks and pipeline-stage
    /// routing leave this false — their state is discarded or belongs to
    /// a child shell, so nothing may persist from them.
    persist_move_closes: bool,
    /// Speculative admission probes (pipeline_stage_stdio_targets)
    /// walk the same list only to resolve fd targets; the state is
    /// discarded and the executing path owns the failure report, so
    /// diagnostic writes through this state are muted — an unmuted
    /// probe reported a stage's failed open once speculatively and
    /// the real execution then reported it again (rubash#1844).
    diagnostics_muted: bool,
    /// Buffered-builtin route mode: a failed open is recorded in
    /// first_open_error and handed back through the ExecuteError::
    /// IoError channel (whose top-level handler renders and routes
    /// it — the route's long-standing behaviour) instead of being
    /// written through this walk state (rubash#1844).
    open_failure_via_error: bool,
    first_open_error: std::cell::RefCell<Option<(std::io::ErrorKind, String)>>,
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

    /// RESTRICTED_SHELL scan for a COMPOUND command's own redirects (#804).
    /// GNU applies a compound's redirects before its body runs
    /// (execute_cmd.c:5606 for the builtin/function class,
    /// do_redirection_internal for the group class), so a restricted shell
    /// refuses before anything is created or spliced. Returns the full
    /// diagnostic (prefix included) for the FIRST restricted write form in
    /// parse order; the caller writes it, sets status 1 and skips the body.
    /// The ordered `redirects` list is the authority; the stdio mirror
    /// slots duplicate list entries and are deduped away. Message shapes
    /// mirror redirection_error: a `{var}>` redirector names the VARIABLE
    /// (redir.c:137-138), a command/process-substitution target reports
    /// the LITERAL word (redir.c:186-190 re-expands with
    /// W_NOCOMSUB|W_NOPROCSUB), everything else the expanded filename
    /// (redir.c:210).
    pub(in crate::executor) fn restricted_compound_redirect_error(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<Vec<u8>>, ExecuteError> {
        if !crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "restricted") {
            return Ok(None);
        }
        let semantic_key = |redirect: &Redirect| {
            format!(
                "{:?}\x1f{}\x1f{:?}\x1f{}",
                redirect.kind, redirect.operator, redirect.fd, redirect.target
            )
        };
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut candidates: Vec<&Redirect> = Vec::new();
        for redirect in cmd.redirects.iter() {
            if seen.insert(semantic_key(redirect)) {
                candidates.push(redirect);
            }
        }
        for redirect in [
            cmd.redirect_out.as_deref(),
            cmd.append.as_deref(),
            cmd.redirect_err.as_deref(),
            cmd.redirect_err_append.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if seen.insert(semantic_key(redirect)) {
                candidates.push(redirect);
            }
        }
        for redirect in candidates {
            if !is_restricted_write_redirect(&redirect.kind)
                && !is_raw_err_and_out_redirect(redirect)
            {
                continue;
            }
            // The per-entry order lives in one place: ambiguity resolves
            // before the restricted refusal, with redirection_error's word
            // rules (redir.c:911-919 + :137-138 + :186-190).
            return Ok(Some(self.restricted_write_entry_diagnostic(redirect)?));
        }
        Ok(None)
    }

    /// GNU redir.c:911-919 for ONE restricted-write entry: ambiguity
    /// resolves BEFORE the restricted refusal — redirection_expand yields
    /// zero fields without an anchored quoted span, or more than one
    /// field, and the entry reports `{raw}: ambiguous redirect` (a
    /// varassign entry reports under the VARIABLE name, redir.c:137-138).
    /// The expansion side effects GNU would have run stay run. Returns the
    /// full diagnostic (prefix included) for the caller to route and to
    /// skip the command with.
    pub(in crate::executor) fn restricted_write_entry_diagnostic(
        &mut self,
        redirect: &Redirect,
    ) -> Result<Vec<u8>, ExecuteError> {
        let var_name = redirect.fd_var.as_deref().unwrap_or_default();
        let target = self.expand_redirect_target(redirect);
        let raw_word = redirect
            .target_metadata
            .raw
            .strip_prefix('&')
            .unwrap_or(&redirect.target_metadata.raw);
        let expanded_word = target.strip_prefix('&').unwrap_or(&target);
        let mut stderr = Vec::new();
        // redir.c:911-914: the ambiguity class, worded like
        // redirection_error — the RAW word for a plain entry, the variable
        // for a varassign entry.
        if expanded_word.is_empty() {
            if self.redirect_operand_ambiguous(redirect, &redirect.target_metadata.raw, &target)
                || !raw_word_has_quoted_span(raw_word)
            {
                writeln!(
                    &mut stderr,
                    "{}{}: ambiguous redirect",
                    self.error_prefix(),
                    if redirect.fd_var.is_some() {
                        var_name
                    } else {
                        raw_word
                    }
                )?;
                return Ok(stderr);
            }
            // One anchored quoted-empty field is NOT ambiguous: GNU's
            // restricted check (redir.c:919) still fires before the
            // open("") ENOENT, and the message names the (empty) filename.
            write!(
                &mut stderr,
                "{}: restricted: cannot redirect output",
                self.error_prefix()
            )?;
            return Ok(stderr);
        }
        if self.redirect_operand_ambiguous(redirect, &redirect.target_metadata.raw, &target) {
            writeln!(
                &mut stderr,
                "{}{}: ambiguous redirect",
                self.error_prefix(),
                if redirect.fd_var.is_some() {
                    var_name
                } else {
                    raw_word
                }
            )?;
            return Ok(stderr);
        }
        // redir.c:210 with redirection_error's word rules: a varassign
        // names the variable, a command/process-substitution target the
        // LITERAL word (redir.c:186-190), everything else the expanded
        // filename.
        if redirect.fd_var.is_some() {
            writeln!(
                &mut stderr,
                "{}{var_name}: restricted: cannot redirect output",
                self.error_prefix()
            )?;
        } else {
            let message_word: String =
                if raw_word.contains("$(") || raw_word.contains("<(") || raw_word.contains(">(") {
                    raw_word.to_owned()
                } else {
                    expanded_word.to_owned()
                };
            writeln!(
                &mut stderr,
                "{}{message_word}: restricted: cannot redirect output",
                self.error_prefix()
            )?;
        }
        Ok(stderr)
    }

    pub(in crate::executor) fn reject_restricted_write_redirect_entry(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<bool, ExecuteError> {
        if !crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "restricted") {
            return Ok(false);
        }
        let Some((index, redirect)) = cmd
            .redirects
            .iter()
            .enumerate()
            .find(|(_, redirect)| is_restricted_write_redirect(&redirect.kind))
        else {
            return Ok(false);
        };
        let stderr = self.restricted_write_entry_diagnostic(redirect)?;
        self.write_redirect_diagnostic_at(cmd, index, &stderr)?;
        self.exit_code = 1;
        Ok(true)
    }

    pub(in crate::executor) fn reject_invalid_redirects(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<RedirectRejection>, ExecuteError> {
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
            cmd.redirect_in.as_deref(),
            cmd.redirect_out.as_deref(),
            cmd.append.as_deref(),
            cmd.redirect_err.as_deref(),
            cmd.redirect_err_append.as_deref(),
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
            // GNU redir.c:298 redirection_expand -> expand_words_no_vars:
            // the redirect word's expansion carries the nounset check, and
            // it fires when THAT redirect applies — before its own open and
            // before any later redirect (so the diagnostic routes through
            // the fd 2 bindings applied so far, like issue #250). The
            // report names the BARE variable (`nope: unbound variable`,
            // err_unboundvar without the sigil). The error is a fatal word
            // expansion error whose containment follows where
            // do_redirections runs — the main shell for builtin/function
            // targets, a forked child for external ones — so the walker
            // reports here and hands the fatal class back to the caller
            // (rubash#535).
            if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "nounset")
                && !matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::HereDoc | crate::parser::RedirectKind::HereString
                )
            {
                // rubash#1915: the scan judges the raw target text, but
                // a recorded process-substitution span inside it is
                // child text (subst.c:6362 process_substitute) — mask
                // the command's recorded spans before judging, exactly
                // like the simple-command pre-scan does.
                // The command node's own span records are consumed by
                // materialization before redirects apply, but the
                // target word's metadata keeps its recordings.
                let masked_target = Self::mask_recorded_process_substitutions(
                    &redirect.target_metadata.raw,
                    &redirect.target_metadata.process_substitutions,
                );
                if let Some(name) = self.nounset_unbound_parameter(&masked_target) {
                    let mut stderr = Vec::new();
                    writeln!(
                        &mut stderr,
                        "{}{name}: unbound variable",
                        self.error_prefix()
                    )?;
                    self.write_redirect_diagnostic_at(cmd, index, &stderr)?;
                    self.exit_code = 1;
                    return Ok(Some(RedirectRejection::FatalNounset));
                }
            }
            let target = self.expand_redirect_target(redirect);
            // GNU subst.c:12663-12668: failglob inside redirection_expand
            // prints `no match: WORD' (done, at the expansion) and DISCARDs
            // the command list with status 1 — the command never runs and
            // nothing opens. This gate is the FIRST expansion site of a
            // simple command's redirect list, so it owns the abort.
            if self.redirect_failglob_aborted() {
                return Err(ExecuteError::ExpansionFailure(1));
            }
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
            // plain EBADF shape. File-open {var} forms need no gate check —
            // except the RESTRICTED_SHELL one: `{v}> f` is still
            // r_output_direction for WRITE_REDIRECT (command.h:57 takes the
            // direction, not the redirector spelling), so it is refused
            // before the allocation like any other write form.
            if redirect.fd_var.is_some() && !is_dup_kind {
                if is_restricted_write_redirect(&redirect.kind)
                    && crate::builtins::set::shell_option_enabled(
                        &self.shell_state.env_vars,
                        "restricted",
                    )
                {
                    // One per-entry order for every refusal site: ambiguity
                    // (under the variable name, redir.c:137-138) resolves
                    // before the restricted refusal (redir.c:911-919).
                    let stderr = self.restricted_write_entry_diagnostic(redirect)?;
                    self.write_redirect_diagnostic_at(cmd, index, &stderr)?;
                    self.exit_code = 1;
                    return Ok(Some(RedirectRejection::Command));
                }
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
            // rubash#1396: `>&-` / `<&-` (CloseOutput/CloseInput) close the
            // fd. Mark it closed in `opened` so a later `2>&1` sees the
            // closed fd and fails with EBADF (GNU redir.c:798-803).
            if matches!(
                redirect.kind,
                crate::parser::RedirectKind::CloseOutput | crate::parser::RedirectKind::CloseInput
            ) {
                opened.insert(redirector, false);
                continue;
            }
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
                                .map(|_| Some(RedirectRejection::Command));
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
                            .map(|_| Some(RedirectRejection::Command));
                    }
                    continue;
                }
                // GNU redir.c:784-843 (do_redirection_internal,
                // TRANSLATE_REDIRECT): the dup-word operand of [N]<&WORD /
                // [N]>&WORD expands through redirection_expand
                // (redir.c:298) — a process-substitution word expands to
                // its substitution filename, which is a non-digit word, and
                // the final `else` returns AMBIGUOUS_REDIRECT; redirection_
                // error (redir.c:186-190) re-expands the word with
                // W_NOCOMSUB|W_NOPROCSUB for the message, so the diagnostic
                // carries the LITERAL `<(echo x)` text (rubash#340:
                // `cat <& <(echo x)` -> `<(echo x): ambiguous redirect`,
                // rc=1). The one exception is `>&WORD` with redirector 1
                // (r_err_and_out, redir.c:832-838): the expanded filename
                // opens for stdout+stderr and the command runs (GNU probe:
                // `cat >& <(echo x)` is rc=0, silent).
                if crate::parser::raw_word_has_unquoted_process_substitution(raw_word)
                    && !(redirect.kind == crate::parser::RedirectKind::DuplicateOutput
                        && redirector == 1
                        && !raw_word.ends_with('-'))
                {
                    return self
                        .reject_redirect_at(cmd, index, &format!("{raw_word}: ambiguous redirect"))
                        .map(|_| Some(RedirectRejection::Command));
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
                    // GNU redir.c do_redirection_internal guards the
                    // whole dup/move apply with `redir_fd != redirector`:
                    // a self-dup/self-move never reaches the dup2, so it
                    // succeeds even when the fd is closed, and a self-move
                    // does not close anything (rubash#1236). It also
                    // changes nothing, so the opened bookkeeping is left
                    // exactly as it was (a closed fd stays closed for a
                    // later dup in the same list).
                    if fd == redirector {
                        continue;
                    }
                    if !dup_source_fd_open(&self.fd_table, fd, &opened) {
                        return self
                            .reject_with_dup_bad_fd(cmd, index, fd, move_fd, raw_word, redirector)
                            .map(|_| Some(RedirectRejection::Command));
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
                        .map(|_| Some(RedirectRejection::Command));
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
                            // Self-dup/self-move: never rejected and the
                            // opened state is unchanged (see the digit-form
                            // branch above; redir.c's
                            // `redir_fd != redirector` guard).
                            if fd == redirector {
                                continue;
                            }
                            if !dup_source_fd_open(&self.fd_table, fd, &opened) {
                                return self
                                    .reject_with_dup_bad_fd(
                                        cmd, index, fd, false, raw_word, redirector,
                                    )
                                    .map(|_| Some(RedirectRejection::Command));
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
                                .map(|_| Some(RedirectRejection::Command));
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
                        .map(|_| Some(RedirectRejection::Command));
                }
                // RESTRICTED_SHELL (redir.c:919-924): the translated
                // r_err_and_out IS a WRITE_REDIRECT direction, so a
                // restricted shell refuses it here with the expanded
                // filename (redir.c:210) — `echo hi >& out.txt` used to slip
                // through because the raw kind is still DuplicateOutput.
                if crate::builtins::set::shell_option_enabled(
                    &self.shell_state.env_vars,
                    "restricted",
                ) {
                    return self
                        .reject_redirect_at(
                            cmd,
                            index,
                            &format!("{expanded_word}: restricted: cannot redirect output"),
                        )
                        .map(|_| Some(RedirectRejection::Command));
                }
                // r_err_and_out falls through to the file-open class with
                // the stripped word as filename.
            }

            // ---- file-open redirections --------------------------------
            if expanded_word.is_empty() {
                // Multi-field split or zero-field expansion: both are the
                // redirection_expand NULL condition (redir.c:325-333); an
                // anchored quoted-null single empty field is NOT.
                if self.redirect_operand_ambiguous(redirect, &redirect.target_metadata.raw, &target)
                    || !raw_word_has_quoted_span(raw_word)
                {
                    return self
                        .reject_redirect_at(cmd, index, &format!("{raw_word}: ambiguous redirect"))
                        .map(|_| Some(RedirectRejection::Command));
                }
                // One anchored empty field: the open itself fails —
                // open("") → ENOENT on every flag combination, so the
                // diagnostic is deterministic without touching the disk.
                return self
                    .reject_redirect_at(cmd, index, ": No such file or directory")
                    .map(|_| Some(RedirectRejection::Command));
            }
            if self.redirect_operand_ambiguous(redirect, &redirect.target_metadata.raw, &target) {
                return self
                    .reject_redirect_at(cmd, index, &format!("{raw_word}: ambiguous redirect"))
                    .map(|_| Some(RedirectRejection::Command));
            }
            // RESTRICTED_SHELL (vendored redir.c:919-924): after the
            // redirectee expands and passes the ambiguity checks, a
            // restricted shell refuses every WRITE_REDIRECT direction
            // (command.h:57 — `>` `>>` `<>` `&>` `&>>` `>|`, on ANY target,
            // slash-free ones and /dev/null included) with
            // "<filename>: restricted: cannot redirect output"
            // (redir.c:210) and skips the open — `<>` never creates, `>`
            // never truncates. Read-only input is NOT in WRITE_REDIRECT and
            // stays allowed (#804 gap 11/12: the old slash-in-target scan
            // inverted both cells).
            if is_restricted_write_redirect(&redirect.kind)
                && crate::builtins::set::shell_option_enabled(
                    &self.shell_state.env_vars,
                    "restricted",
                )
            {
                // redirection_error (redir.c:186-190) re-expands the word
                // with W_NOCOMSUB|W_NOPROCSUB for the message: a
                // command/process-substitution target reports the LITERAL
                // `$(echo f)` text (GNU probe), a plain `$v` target reports
                // the expansion.
                let message_word = if raw_word.contains("$(")
                    || raw_word.contains("<(")
                    || raw_word.contains(">(")
                {
                    raw_word
                } else {
                    &*target
                };
                return self
                    .reject_redirect_at(
                        cmd,
                        index,
                        &format!("{message_word}: restricted: cannot redirect output"),
                    )
                    .map(|_| Some(RedirectRejection::Command));
            }
            // `<> word` opens O_RDWR|O_CREAT (make_cmd.c:682) — creating
            // the missing file is part of the redirection, not a consumer
            // side effect (rubash#264). Other open failures (bad
            // directory, permission) abort like GNU's redir_open.
            if redirect.kind == crate::parser::RedirectKind::ReadWrite
                && !is_null_device(&target)
                && dev_stdio_redirect_fd(&target).is_none()
                && !crate::special_paths::is_net_redirect_target(&target)
                && !target.starts_with("<(")
                // Registry classes (rubash#1917/#1900): a synthetic
                // /proc spelling's mapped path is not the file — the
                // class's own open face owns its outcome, so this
                // raw-path probe must not judge it (same skip class as
                // the Input probe below).
                && crate::special_paths::synthetic_proc_content(&target).is_none()
                && !crate::special_paths::is_synthetic_link_path(&target)
            {
                let path = shell_path_to_windows(&target, &self.shell_state.env_vars);
                if let Err(error) = FileFd::open_readwrite_pair(path) {
                    // posix_errors::path_error already renders the
                    // `{target}: {errno text}` shape — pass it through
                    // like the Input probe below instead of prefixing the
                    // target a second time (rubash#1237).
                    let message = crate::posix_errors::path_error(&target, error);
                    let text = crate::posix_errors::message(&message);
                    return self
                        .reject_redirect_at(cmd, index, &text)
                        .map(|_| Some(RedirectRejection::Command));
                }
            }
            // GNU applies `< word`'s open BEFORE the command runs
            // (redir.c do_redirections from execute_simple_command): a
            // failed open reports `{target}: {errno text}` through the
            // already-applied fd 2 and the command never executes —
            // in-process stdin resolution must not fall back to another
            // source on the unopenable target (rubash#525: a pipeline
            // member's `{ cat < /nonexistent; }` used to read the upstream
            // pipe and exit 0 silently). Plain Input opens have no side
            // effects, so the probe is validation-pure.
            if redirect.kind == crate::parser::RedirectKind::Input
                && !is_null_device(&target)
                && dev_stdio_redirect_fd(&target).is_none()
                && !crate::special_paths::is_net_redirect_target(&target)
                && !target.starts_with("<(")
                && redirect_target_fd(&target).is_none()
                // Registry classes (rubash#1917/#1900): synthetic /proc
                // content and links are validated by the open ladder
                // that serves them (probe_input_redirect on the shell
                // spelling). Their mapped Windows path never exists,
                // so this raw-path probe condemned every live /proc
                // target with a spurious ENOENT before the command
                // ran — the reject-gate face of the ladder-skip family
                // (cf. rubash#1904's comsub face).
                && crate::special_paths::synthetic_proc_content(&target).is_none()
                && !crate::special_paths::is_synthetic_link_path(&target)
            {
                let path = self.session_path(crate::executor::path::windows_operand_file_path(
                    shell_path_to_windows(&target, &self.shell_state.env_vars),
                    &self.shell_state.env_vars,
                ));
                // rubash#1820: an O_RDONLY probe open is NOT side-effect
                // free on a FIFO — it blocks until a writer appears and
                // consumes the rendezvous pairing, after which the real
                // open blocks forever (GNU opens the target exactly once,
                // redir.c do_redirections). An existing FIFO validates
                // through access(2) instead: ENOENT / EACCES surface with
                // the same errno the open would have produced.
                if let Err(error) = super::shell_options::probe_input_redirect_gate(&path) {
                    // posix_errors::path_error already renders the
                    // `{target}: {errno text}` shape (same convention the
                    // ReadWrite probe above consumes).
                    let message = crate::posix_errors::path_error(&target, error);
                    let text = crate::posix_errors::message(&message);
                    return self
                        .reject_redirect_at(cmd, index, &text)
                        .map(|_| Some(RedirectRejection::Command));
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

        Ok(None)
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
            self.error_prefix()
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
        writeln!(&mut stderr, "{}{text}", self.error_prefix())?;
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

    /// niubash#144: the ordered final fd-1/fd-2 targets of a pipeline
    /// element, with the element's fd 1 seeded as the stage pipe (GNU
    /// execute_cmd.c execute_pipeline binds the pipe first, then
    /// do_redirections runs — redir.c:1169-1170 dup2 order). Drives the
    /// sequential stage executor's child wiring: both targets Stdout means
    /// the element's bytes flow through ONE shared pipe so they interleave
    /// in true write order (never two pipes concatenated stdout-first);
    /// fd 2 on a file or fd 1/fd 2 sharing one file description tell the
    /// spawner which streams to wire directly. Returns None when the walk
    /// fails (the caller keeps its legacy per-redirect handling).
    pub(in crate::executor) fn pipeline_stage_stdio_targets(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<Option<(OutputTarget, OutputTarget)>, ExecuteError> {
        let mut state = self.command_output_fd_state();
        state.fds.insert(1, OutputTarget::Stdout);
        // Same defer as route_pipeline_stage_streams: a diagnostic the walk
        // emits against the seeded Stdout must not leak to the real stdout.
        state.defer_stdout_writes = true;
        // This walk is an admission probe — it only resolves targets
        // and its state is discarded. The executing path reports a
        // failed open itself; a report here double-prints (rubash#1844).
        state.diagnostics_muted = true;
        if !self.apply_ordered_output_redirects(cmd, &mut state)? {
            return Ok(None);
        }
        // A stage whose own output open failed is not routable from the
        // resolved targets (the failed fd never bound): decline so the
        // sequential stage executor reports the failure per element
        // (rubash#1844).
        if state.redirect_failed {
            return Ok(None);
        }
        let stdout = state.fd_target(1).cloned().unwrap_or(OutputTarget::Stdout);
        let stderr = state.fd_target(2).cloned().unwrap_or(OutputTarget::Stderr);
        Ok(Some((stdout, stderr)))
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

    /// The redirect in `cmd`'s parse-order list whose open(2) failure
    /// produced `message` — GNU redirection_error (redir.c:149-176)
    /// reports the redirectee's word, so the diagnostic head `{target}: `
    /// names the failing entry. Output-side opens match exactly like the
    /// input side (rubash#940: `cmd 2>cap.txt > /missing` reports through
    /// the fd-2 binding `2>cap.txt` already established). Pure fd-dup
    /// (`>&N`), closed (`>&-`) and fd-var entries cannot be the failing
    /// open and are skipped.
    pub(in crate::executor) fn failing_open_redirect_index(
        &mut self,
        cmd: &CommandNode,
        message: &[u8],
    ) -> Option<usize> {
        use crate::parser::RedirectKind;
        cmd.redirects.iter().position(|redirect| {
            let opens_path = match redirect.kind {
                RedirectKind::Input | RedirectKind::DuplicateInput => {
                    redirect.fd.is_none_or(|fd| fd == 0)
                }
                RedirectKind::ReadWrite
                | RedirectKind::Output
                | RedirectKind::Append
                | RedirectKind::ClobberOutput
                | RedirectKind::CombinedOutput
                | RedirectKind::CombinedAppend
                | RedirectKind::DuplicateOutput => true,
                _ => false,
            };
            if !opens_path {
                return false;
            }
            let target = self.expand_redirect_target(redirect);
            if super::execution_misc::is_closed_redirect_target(&target)
                || redirect_target_fd(&target).is_some()
            {
                return false;
            }
            let head = format!("{}: ", target.strip_prefix('&').unwrap_or(&target));
            let prefix = self.error_prefix();
            std::str::from_utf8(message).is_ok_and(|text| {
                text.strip_prefix(prefix.as_str())
                    .is_some_and(|rest| rest.starts_with(&head))
            })
        })
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
        let failed_index = self.failing_open_redirect_index(cmd, message);
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

    /// Routes one command's buffered stream pair through the command's
    /// ordered redirect state. Owner: BUILTIN buffered output (echo/printf
    /// style — rubash buffers a builtin's writes and flushes them at
    /// completion) and the closed-output external shapes (see
    /// write_external_captured_streams). GNU model: a builtin's
    /// do_redirections run in the main shell (execute_cmd.c:5606
    /// execute_builtin_or_function) and its writes go straight through the
    /// redirected descriptors, with builtins/common.c:320-334 (sh_chkwrite)
    /// folding a failed flush into `write error: ...` and the builtin's
    /// status; routing the buffered bytes through the same ordered walk
    /// reproduces the observable targets, the per-write order (each builtin
    /// command's flush happens at its own completion, in execution order),
    /// and the failure statuses.
    pub(in crate::executor) fn route_builtin_buffered_output(
        &mut self,
        cmd: &CommandNode,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<bool, ExecuteError> {
        self.route_builtin_buffered_output_ordered(cmd, stdout, stderr, false)
    }

    /// `stderr_first` swaps the final ordered walk: printf's GNU model
    /// emits builtin_error diagnostics unbuffered while its converted
    /// output flushes only when the builtin returns, so in a merged
    /// stream the diagnostics precede the output (rubash#1134).
    pub(in crate::executor) fn route_builtin_buffered_output_ordered(
        &mut self,
        cmd: &CommandNode,
        stdout: &[u8],
        stderr: &[u8],
        stderr_first: bool,
    ) -> Result<bool, ExecuteError> {
        self.last_builtin_write_failed.set(false);
        let mut state = self.command_output_fd_state();
        // This route serves builtins executing in the current shell, so a
        // move-close whose target was closed persists past the command
        // (GNU redir.c add_undo_redirect; rubash#1235).
        state.persist_move_closes = true;
        // A failed open in THIS route keeps its historical channel:
        // the error returns as ExecuteError::IoError and the top-level
        // handler renders/routes it (rubash#1844 keeps the walk's
        // per-element report for the pipeline consumers only).
        state.open_failure_via_error = true;
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
                self.write_bad_output_fd_diagnostic(cmd, &state)?;
                self.exit_code = 1;
                self.last_builtin_write_failed.set(true);
                return Ok(true);
            }
            if !stderr.is_empty()
                && state
                    .fd_target(2)
                    .is_some_and(|target| *target == OutputTarget::Closed)
            {
                // A closed fd 2 drops diagnostics silently and leaves the
                // command's status alone (GNU builtin_error writes to fd 2;
                // with fd 2 closed the write is lost and sh_chkwrite folds
                // only stdout's ferror — rubash#699 R1.3). Route stdout
                // through the state first so it is not lost with stderr.
                self.write_state_output_or_diagnostic(cmd, &state, 1, stdout)?;
                let _ = self.write_state_output_or_diagnostic(cmd, &state, 2, stderr)?;
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
        if let Some((kind, message)) = state.first_open_error.borrow_mut().take() {
            return Err(ExecuteError::IoError(std::io::Error::new(kind, message)));
        }
        if state.redirect_failed {
            // rubash#1871: a failed redirection fails the command in
            // GNU (redir.c do_redirections error → the builtin never
            // runs, status 1). The walk already printed the diagnostic
            // and set exit_code, but wrappers whose dispatch assigns
            // the builtin's own return value afterwards (printf, pwd)
            // would overwrite it with the nominal status — fold the
            // failure into the same merge flag the sh_chkwrite flush
            // failure uses (take_builtin_write_failed) so those
            // wrappers return 1 too.
            self.last_builtin_write_failed.set(true);
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
            self.write_bad_output_fd_diagnostic(cmd, &state)?;
            self.exit_code = 1;
            self.last_builtin_write_failed.set(true);
            return Ok(true);
        }
        // A closed fd 2 (rubash#699 R1.3) is not special-cased here: the
        // ordered walk below writes stdout, then re-visits fd 2 where the
        // Closed target drops the bytes silently and leaves the status
        // untouched. GNU builtins/common.c:320 sh_chkwrite folds only
        // stdout's ferror into the builtin status; builtin_error's failed
        // fd-2 write is lost.
        if stderr_first {
            self.write_state_output_or_diagnostic(cmd, &state, 2, stderr)?;
            self.write_state_output_or_diagnostic(cmd, &state, 1, stdout)?;
        } else {
            self.write_state_output_or_diagnostic(cmd, &state, 1, stdout)?;
            self.write_state_output_or_diagnostic(cmd, &state, 2, stderr)?;
        }
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
                if fd == 2 {
                    // A failed fd-2 write (closed descriptor / broken stderr
                    // pipe) is lost like GNU's builtin_error: no diagnostic,
                    // no status change (rubash#699 R1.3).
                    return Ok(());
                }
                if builtin_output_write_is_unchecked(cmd) {
                    return Ok(());
                }
                self.write_bad_output_fd_diagnostic(cmd, state)?;
                self.exit_code = 1;
                self.last_builtin_write_failed.set(true);
                Ok(())
            }
            Err(error) => {
                // rubash#1496: a NON-closed write failure on the stdout
                // stream (ENOSPC from a /dev/full redirect target is the
                // canonical case) is GNU's sh_chkwrite arm
                // (builtins/common.c:320-334): the command reports
                // `<name>: write error: <strerror>` through builtin_error
                // and fails with status 1 — the io error must not escape
                // to the command boundary, which renders it bare as
                // `<script>: line N: <strerror>` under the shell's name.
                if fd == 1 {
                    if let ExecuteError::IoError(io_error) = &error {
                        if !builtin_output_write_is_unchecked(cmd) {
                            let command =
                                cmd.words.first().map(String::as_str).unwrap_or("command");
                            let text = format!(
                                "{}{command}: write error: {}\n",
                                self.diagnostic_prefix(),
                                crate::posix_errors::message(io_error)
                            );
                            // The diagnostic rides fd 2 as THIS command's
                            // redirects bound it; a failed delivery (fd 2
                            // is the same full device) is lost like GNU's.
                            let _ = state.write_to_fd(self, 2, text.as_bytes());
                            self.exit_code = 1;
                            self.last_builtin_write_failed.set(true);
                        }
                        return Ok(());
                    }
                }
                Err(error)
            }
        }
    }

    fn command_output_fd_state(&self) -> OutputFdState {
        let mut state = OutputFdState {
            fds: HashMap::new(),
            saw_output_redirect: false,
            redirect_failed: false,
            defer_stdout_writes: false,
            deferred_stdout: std::cell::RefCell::new(Vec::new()),
            persist_move_closes: false,
            diagnostics_muted: false,
            open_failure_via_error: false,
            first_open_error: std::cell::RefCell::new(None),
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
            // rubash#1359: a `{var}` redirection is a redir_varassign
            // (GNU redir.c:1133-1166) — it allocates a fresh descriptor
            // >= 10 and is applied exactly once by
            // apply_dynamic_fd_var_redirects; it NEVER binds the
            // command's own fd 0/1/2. Walking it here defaulted its fd
            // to 1 and rerouted the command's stdout into the var-fd
            // target (a literal file named `>(cat)` for a procsub
            // target; a delayed flush for a plain file target). The
            // async-stdio walk in compound_exec.rs skips fd_var entries
            // for the same reason.
            if redirect.fd_var.is_some() {
                continue;
            }
            match redirect.kind {
                crate::parser::RedirectKind::Output
                | crate::parser::RedirectKind::Append
                | crate::parser::RedirectKind::ClobberOutput => {
                    // RESTRICTED_SHELL (#804 round 3): this routing walk can
                    // run BEFORE the ordered gate on pipeline stdio-wiring
                    // probes; it must never speak for (or open) an entry the
                    // restricted shell refuses — skip it silently and let
                    // the ordered gate / materialization pre-check refuse
                    // exactly once, with the entry's own ambiguity resolved
                    // first (redir.c:911-919).
                    if crate::builtins::set::shell_option_enabled(
                        &self.shell_state.env_vars,
                        "restricted",
                    ) && is_restricted_write_redirect(&redirect.kind)
                    {
                        continue;
                    }
                    let fd = redirect_fd_or_default(redirect, 1);
                    let target = self.expand_redirect_target(redirect);
                    // redir.c:298-333 redirection_expand: a zero-field
                    // unquoted expansion (unset `>$v`) or a multi-field one
                    // is AMBIGUOUS_REDIRECT reporting the RAW word; an
                    // anchored quoted null (`> ""`) is one empty field and
                    // fails the open itself. This arm owns the check for the
                    // apply-time paths (pipeline stage stream routing runs
                    // the ordered walk after the preflight; issue #514 —
                    // without it the empty target fell through to
                    // open("") ENOENT and the stage failure also unwound
                    // the whole pipeline instead of dying alone).
                    if target.is_empty() && !raw_word_has_quoted_span(&redirect.target_metadata.raw)
                    {
                        self.write_ambiguous_redirect_diagnostic(
                            state,
                            &redirect.target_metadata.raw,
                        )?;
                        self.exit_code = 1;
                        state.redirect_failed = true;
                        return Ok(true);
                    }
                    if redirect_target_is_ambiguous(
                        &redirect.target_metadata.raw,
                        &target,
                        crate::builtins::shopt::option_enabled(
                            &self.shell_state.env_vars,
                            "extglob",
                        ),
                    ) && !self.redirect_target_resolved_single_glob_match(redirect)
                    {
                        self.write_ambiguous_redirect_diagnostic(
                            state,
                            &redirect.target_metadata.raw,
                        )?;
                        self.exit_code = 1;
                        state.redirect_failed = true;
                        return Ok(true);
                    }
                    // subst.c:12663-12668 failglob: the diagnostic already
                    // printed at the expansion; this routing pass must not
                    // open the failed target either — follow the established
                    // redirect_failed protocol (no write, status 1).
                    if self.redirect_failglob_aborted() {
                        self.exit_code = 1;
                        state.redirect_failed = true;
                        return Ok(true);
                    }
                    // See injected_redirect_fd_is_bound: an injected group
                    // redirect is already realized when the seeded fd holds
                    // the group's shared File binding (the seeded state maps
                    // fd-table File endpoints to SharedFile one-to-one).
                    // The pipeline routing walks (route_pipeline_stage_streams,
                    // pipeline_stage_stdio_targets) re-seed fd 1 as the stage
                    // pipe (OutputTarget::Stdout, defer_stdout_writes) BEFORE
                    // the ordered walk — the binding is still the group's
                    // File in the fd table, so the skip must hold there too:
                    // falling through to open_command_output_target replaced
                    // the shared description with a Path target whose writes
                    // reopen in append mode on a second offset, and the
                    // body's later writes through the bound handle overwrote
                    // the pipeline's contribution (niubash#532:
                    // `( echo A; printf B | cat; echo C ) > f` lost B).
                    if self.injected_redirect_fd_is_bound(redirect, fd)
                        && (matches!(state.fds.get(&fd), Some(OutputTarget::SharedFile(_)))
                            || (state.defer_stdout_writes
                                && matches!(state.fds.get(&fd), Some(OutputTarget::Stdout))))
                    {
                        state.saw_output_redirect = true;
                        continue;
                    }
                    // The dup shortcut below is for the `>&N` SPELLING
                    // and for virtual source bindings (pipes, captures,
                    // pipeline staging). A /dev/fd/N (or /dev/std*,
                    // /proc/self/fd/N) target naming a DISK FILE source
                    // is a FILENAME open in GNU (r_output_direction),
                    // never a dup — it must reach
                    // open_command_output_target, which owns the reopen
                    // semantics (rubash#1373).
                    // rubash#1871: a path-spelled target whose fd has
                    // NO binding at all (the walk state is seeded from
                    // the whole fd table, so absence means the fd is
                    // dead — e.g. a procsub fd closed with its owning
                    // command) is likewise a filename open: GNU fails
                    // it ENOENT at open(). The dup shortcut would
                    // report it with the raw word and the dup2 EBADF
                    // reason instead.
                    let dead_path_spelled_fd =
                        crate::executor::execution_misc::dev_stdio_redirect_fd(&target).is_some()
                            && redirect_target_fd(&target)
                                .is_some_and(|source_fd| state.fd_target(source_fd).is_none());
                    if let Some(source_fd) = redirect_target_fd(&target).filter(|_| {
                        !self.dev_stdio_target_reopens_disk(&target, state) && !dead_path_spelled_fd
                    }) {
                        let Some(source_target) = state.fd_target(source_fd).cloned() else {
                            self.write_bad_fd_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        };
                        if source_target == OutputTarget::Closed
                            && !self.dup_source_fd_still_open(source_fd)
                        {
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
                    // GNU do_redirections stops at the first failing
                    // redirect (redir.c:246): later targets must not
                    // open once one failed (rubash#1844).
                    if state.redirect_failed {
                        return Ok(true);
                    }
                }
                crate::parser::RedirectKind::CombinedOutput
                | crate::parser::RedirectKind::CombinedAppend => {
                    let target = self.expand_redirect_target(redirect);
                    // subst.c:12663-12668 failglob — see the file arm above.
                    if self.redirect_failglob_aborted() {
                        self.exit_code = 1;
                        state.redirect_failed = true;
                        return Ok(true);
                    }
                    self.open_command_output_target(state, 1, &target, redirect)?;
                    if state.redirect_failed {
                        return Ok(true);
                    }
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
                    // target into the redirector.
                    if let Some((source_fd, move_source)) = redirect_target_fd_and_move(&target) {
                        // GNU redir.c do_redirection_internal: the whole
                        // r_duplicating/r_move apply is guarded by
                        // `redir_fd != redirector`, so a self-dup or
                        // self-move is a total no-op — it succeeds even
                        // when the fd is closed (`echo hi 99>&99`), and a
                        // self-move leaves the fd open (rubash#1236).
                        if source_fd == target_fd {
                            state.saw_output_redirect = true;
                            continue;
                        }
                        let Some(source_target) = state.fd_target(source_fd).cloned() else {
                            self.write_bad_fd_redirect_diagnostic(
                                state,
                                &redirect.target_metadata.raw,
                            )?;
                            self.exit_code = 1;
                            state.redirect_failed = true;
                            return Ok(true);
                        };
                        if source_target == OutputTarget::Closed
                            && !self.dup_source_fd_still_open(source_fd)
                        {
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
                        // GNU add_undo_redirect (redir.c) for r_move: the
                        // source fd is saved for undo ONLY when the target
                        // was open at apply time (fcntl(redirector) != -1);
                        // a closed target means the move's close survives
                        // the command. Judge openness from the state as it
                        // stands BEFORE this redirect binds the target.
                        let target_was_open = state
                            .fd_target(target_fd)
                            .is_some_and(|target| *target != OutputTarget::Closed);
                        state.fds.insert(target_fd, source_target);
                        state.saw_output_redirect = true;
                        if move_source {
                            state.fds.insert(source_fd, OutputTarget::Closed);
                            if state.persist_move_closes && !target_was_open {
                                self.close_persistent_output_fd(source_fd)?;
                                self.shell_state
                                    .env_vars
                                    .insert(fd_closed_key(source_fd), "1".to_string());
                            }
                        }
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
                        if state.redirect_failed {
                            return Ok(true);
                        }
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

    /// rubash#1373 gate for the ordered output walk: a dev-stdio target
    /// (`/dev/fd/N`, `/dev/stdout`, `/proc/self/fd/N`) reopens only when
    /// its source fd is bound to a regular disk file in the fd table.
    /// Pipeline staging (deferred writes) and virtual bindings (pipes,
    /// captures) keep the dup semantics GNU gets from opening a pipe's
    /// /proc/self/fd entry.
    fn dev_stdio_target_reopens_disk(&self, target: &str, state: &OutputFdState) -> bool {
        if state.defer_stdout_writes {
            return false;
        }
        let Some(source_fd) = crate::executor::execution_misc::dev_stdio_redirect_fd(target) else {
            return false;
        };
        match self.fd_table.output_endpoint(source_fd) {
            Some(FdWriteEndpoint::File(file)) => std::fs::metadata(&file.path)
                .map(|meta| meta.is_file())
                .unwrap_or(false),
            Some(FdWriteEndpoint::Stdout) => {
                crate::fd::disk_file_path(crate::fd::process_std_handle(1)).is_some()
            }
            Some(FdWriteEndpoint::Stderr) => {
                crate::fd::disk_file_path(crate::fd::process_std_handle(2)).is_some()
            }
            _ => false,
        }
    }

    fn open_command_output_target(
        &mut self,
        state: &mut OutputFdState,
        fd: u32,
        target: &str,
        redirect: &Redirect,
    ) -> Result<(), ExecuteError> {
        if is_closed_redirect_target(target) {
            state.fds.insert(fd, OutputTarget::Closed);
        } else if is_null_device(target) {
            state.fds.insert(fd, OutputTarget::Null);
        } else if let Some(source_fd) =
            crate::executor::execution_misc::dev_stdio_redirect_fd(target)
        {
            // rubash#355: a `/dev/fd/N` (or /dev/std*) output target is the
            // OS fd-alias device in GNU — open() dups fd N's open file
            // description. Windows has no /dev/fd filesystem; resolve
            // through the virtual fd table exactly like the input side
            // (execution_misc::dev_stdio_redirect_fd ->
            // open_fd_read_endpoint). A ProcessSubstitution write endpoint
            // names its temp carrier (writes land there for the
            // substitution's finish path); a File endpoint shares the real
            // handle. An fd with no write side is GNU's ENOENT
            // ("No such file or directory", see
            // write_bad_fd_redirect_diagnostic).
            // rubash#1373: when the source fd's write endpoint is a regular
            // disk file, GNU's open("/dev/fd/N") is a FRESH open of the
            // underlying file — a new open-file description with its own
            // offset (truncated at open unless the redirect appends) —
            // not a dup of the source descriptor. Model it as a newly
            // opened FileFd, whose writes go through its own OS handle,
            // instead of the source's shared handle. Pipe/capture/procsub
            // endpoints keep the shared-description match below (the
            // cases that already agreed with GNU).
            let reopened = match self.fd_table.output_endpoint(source_fd) {
                Some(FdWriteEndpoint::File(file_fd)) => Some(file_fd.path.clone()),
                Some(FdWriteEndpoint::Stdout) => {
                    crate::fd::disk_file_path(crate::fd::process_std_handle(1))
                }
                Some(FdWriteEndpoint::Stderr) => {
                    crate::fd::disk_file_path(crate::fd::process_std_handle(2))
                }
                _ => None,
            }
            .and_then(|path| {
                crate::executor::fd_table::FileFd::open_write(path, redirect.append, false).ok()
            });
            if let Some(file_fd) = reopened {
                state.fds.insert(fd, OutputTarget::SharedFile(file_fd));
                state.saw_output_redirect = true;
                return Ok(());
            }
            match self.fd_table.output_endpoint(source_fd) {
                Some(FdWriteEndpoint::ProcessSubstitution { path, .. }) => {
                    state.fds.insert(
                        fd,
                        OutputTarget::Path(shell_display_path(&path.to_string_lossy())),
                    );
                }
                Some(FdWriteEndpoint::File(file_fd)) => {
                    state
                        .fds
                        .insert(fd, OutputTarget::SharedFile(file_fd.clone()));
                }
                _ => {
                    // rubash#1871: the diagnostic names the EXPANDED
                    // target — this arm is an open() failure on the
                    // /dev/fd/N filename (GNU redirection_error prints
                    // the expanded word with the open errno, ENOENT;
                    // niubash#118), not a dup2 failure on the raw
                    // spelling. Passing target_metadata.raw printed
                    // the unexpanded `"$p"` and picked the EBADF
                    // reason for a /dev/fd target.
                    self.write_bad_fd_redirect_diagnostic(state, target)?;
                    self.exit_code = 1;
                    state.redirect_failed = true;
                    return Ok(());
                }
            }
        } else {
            let target = self.redirect_output_path_target(target);
            // rubash#1868: this walk's open is a create/truncate
            // VALIDATION whose File is dropped on the floor — buffered
            // builtin output lands later through a SECOND open (the
            // open_redirect_append flush in write_stage_stream_to_target
            // / route_builtin_buffered_output). On a FIFO the dropped
            // validation open consumes the rendezvous pairing and
            // closes without writing: a reader whose first read starts
            // in the zero-writer gap before the flush open takes a
            // spurious EOF (its shell exits 0 with an empty line), and
            // the flush open then blocks forever with no reader left —
            // the intermittent #1820-pin hang under load. GNU redir.c
            // do_redirections performs ONE open per redirection.
            // Mirror the #1820 probe carve-out (probe_output_redirect):
            // a FIFO target is validated by access only, so the flush
            // open is the single pairing open.
            #[cfg(unix)]
            let fifo_validation: Option<Result<(), std::io::Error>> = {
                let path =
                    self.session_path(shell_path_to_windows(&target, &self.shell_state.env_vars));
                crate::executor::shell_options::redirect_path_is_fifo(&path).then(|| {
                    match crate::executor::shell_options::redirect_fifo_access_error(&path, true) {
                        Some(error) => Err(crate::posix_errors::path_error(&target, error)),
                        None => Ok(()),
                    }
                })
            };
            #[cfg(not(unix))]
            let fifo_validation: Option<Result<(), std::io::Error>> = None;
            let open_result: Result<(), std::io::Error> =
                match fifo_validation {
                    Some(result) => result,
                    None if redirect.append => {
                        // GNU redir.c redirection_error (redir.c:149-176) carries the
                        // failed open's target word (`$FIX/b: No such file or
                        // directory`, issue #518); a bare OS error renders
                        // message-only and drops the word.
                        open_redirect_append(&self.session_path(shell_path_to_windows(
                            &target,
                            &self.shell_state.env_vars,
                        )))
                        .map(|_| ())
                        .map_err(|e| crate::posix_errors::path_error(&target, e))
                    }
                    None => self
                        .create_redirect_output(&target, redirect.clobber)
                        .map(|_| ()),
                };
            if let Err(error) = open_result {
                // GNU redir.c:149-176 redirection_error, reached from
                // do_redirections (redir.c:246): a failed open reports
                // through the fd 2 binding in force and fails only THIS
                // command — the walk's redirect_failed protocol the
                // sibling arms (ambiguous, failglob, bad-fd) already
                // follow. Propagating the io error as an executor error
                // instead unwound the WHOLE pipeline around the element:
                // later stages never ran, earlier stages' captured
                // output was dropped, the pipeline status collapsed to
                // the error's 1 and PIPESTATUS was never written
                // (rubash#1844). The io error's Display already carries
                // `{target}: {reason}` (path_error / the noclobber
                // wording inside create_redirect_output).
                if state.open_failure_via_error {
                    let mut slot = state.first_open_error.borrow_mut();
                    if slot.is_none() {
                        *slot = Some((error.kind(), error.to_string()));
                    }
                } else {
                    let mut stderr = Vec::new();
                    writeln!(&mut stderr, "{}{error}", self.error_prefix())?;
                    state.write_to_fd(self, 2, &stderr)?;
                }
                self.exit_code = 1;
                state.redirect_failed = true;
                return Ok(());
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
            self.error_prefix()
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
            self.error_prefix()
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
                    crate::executor::shell_options::stdout_capture_write_mapped(&bytes)?;
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
                let mut file = open_redirect_append(
                    &self.session_path(shell_path_to_windows(path, &self.shell_state.env_vars)),
                )?;
                // GNU builtins/common.c:320-334 sh_chkwrite: a builtin's
                // failed flush is reported by the BUILTIN as
                // `<name>: write error: <strerror>` (builtin_error via
                // sh_wrerror) and fails the builtin — it must not escape
                // as a bare io error, which the command boundary renders
                // as `<script>: line N: <strerror>` with no command name
                // (rubash#1496: `echo hi > /dev/full`, `printf .. >
                // /dev/full`, and the internally-emulated /bin/echo all
                // took that bare channel). Mirror the Closed arm: diag
                // into the opposite stream, status + write-failed flag.
                if let Err(error) = file.write_all(&bytes) {
                    if !bytes.is_empty() {
                        let command = cmd.words.first().map(String::as_str).unwrap_or("command");
                        diag.push_str(&format!(
                            "{}{command}: write error: {}\n",
                            self.diagnostic_prefix(),
                            crate::posix_errors::message(&error)
                        ));
                        self.exit_code = 1;
                        self.last_builtin_write_failed.set(true);
                        *status = 1;
                    }
                }
            }
            OutputTarget::SharedFile(file) => {
                crate::fd::write_all(file.handle, &bytes)?;
            }
        }
        Ok(())
    }

    /// Whether the virtual fd table still holds an OPEN entry for `fd`,
    /// even one with no write endpoint. GNU's dup2 (redir.c:1153) succeeds
    /// for any open descriptor — `exec 3< f; echo x >&3` dups fine and the
    /// write fails later at sh_chkwrite (`echo: write error: Bad file
    /// descriptor`, #551 C8) — while a closed/unopened fd fails the dup at
    /// setup (`3: Bad file descriptor`).
    fn dup_source_fd_still_open(&self, fd: u32) -> bool {
        self.fd_table
            .entries
            .get(&fd)
            .is_some_and(|entry| !entry.closed)
    }

    fn write_bad_output_fd_diagnostic(
        &mut self,
        cmd: &CommandNode,
        state: &OutputFdState,
    ) -> Result<(), ExecuteError> {
        let command = cmd.words.first().map(String::as_str).unwrap_or("command");
        let mut stderr = Vec::new();
        writeln!(
            &mut stderr,
            "{}{command}: write error: Bad file descriptor",
            self.diagnostic_prefix()
        )?;
        // GNU builtins/common.c:320-334 sh_chkwrite -> builtin_error: the
        // write failure is reported to fd 2 *as this command's own redirects
        // bound it* (redir.c do_redirections runs before the builtin), so
        // `echo X >&4 2>err` puts the diagnostic in err (rubash#699 R1.2).
        // A closed fd-2 target drops it silently — the other half of the
        // same rule that makes `ls /no+ 2>&-` quiet (rubash#699 R1.3).
        let _ = state.write_to_fd(self, 2, &stderr);
        Ok(())
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
        if self.diagnostics_muted {
            return Ok(());
        }
        if output.is_empty() {
            // rubash#1868: an empty stream still owes a FIFO target its
            // pairing open. GNU redir.c opens at redirection time,
            // before the builtin runs, so `printf "" > fifo` blocks
            // for a reader and hands it EOF — the open is not skipped
            // for writing nothing. The ordered walk access-probes FIFO
            // targets instead of opening them (the dropped validation
            // open would consume the rendezvous and race the reader to
            // a spurious EOF), so for an empty stream the single
            // pairing open lives here, at the flush site; a non-empty
            // stream reaches it through the Path arm below.
            if let Some(OutputTarget::Path(path)) = self.fd_target(fd).cloned() {
                let resolved = executor
                    .session_path(shell_path_to_windows(&path, &executor.shell_state.env_vars));
                if crate::executor::shell_options::redirect_path_is_fifo(&resolved) {
                    let _pairing = open_redirect_append(&resolved)?;
                }
            }
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
                    crate::executor::shell_options::stdout_capture_write_mapped(output)?;
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
                // process stdout when bound outside any capture. The `None`
                // record names the REAL stdout object the dup snapshotted
                // (redir.c:1170) — writing it through write_stdout_bytes
                // would re-resolve the active command-substitution capture
                // and capture bytes GNU sends outside the substitution
                // (rubash#368).
                match record {
                    Some(generation) => {
                        let _ = crate::executor::shell_options::write_stdout_capture_at_generation(
                            output, generation,
                        );
                    }
                    None => {
                        crate::executor::shell_options::write_real_stdout_uncaptured(output)?;
                    }
                }
                Ok(())
            }
            OutputTarget::Stderr => executor.write_default_stderr(output),
            OutputTarget::ProcessStderr => {
                // dup2 snapshot of the original stderr object (rubash#279):
                // the raw stderr channel, never the live fd_table[2]. The
                // snapshot names the stderr channel AS BOUND at dup time;
                // while an executor stderr capture is active (a pipeline
                // stage's transport for that very channel), the snapshot's
                // bytes belong to the capture — writing them raw flushes
                // them in real time while the stage's live fd-2 writes
                // wait in the capture until stage end, splitting one
                // logical stream across two flush times (rubash#1237
                // item 4: `( ... ) 1>&2 | cat` printed the fd-2 write
                // before both fd-1 writes). The capture field does not
                // follow later fd-2 rebindings, so the snapshot still
                // never tracks the live fd_table[2]; with no capture
                // active the raw channel is used exactly as before.
                if let Some(capture) = executor.stderr_capture.as_mut() {
                    use std::io::Write;
                    capture.write_all(output)?;
                } else {
                    super::shell_options::write_stderr_bytes(output)?;
                }
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
                // G-02: the recorded target is the raw redirect word —
                // anchor it at the session cwd, never the process cwd
                // (this append-open is where buffered builtin output
                // actually lands; the setup open in
                // open_command_output_target only creates/truncates).
                let mut file = open_redirect_append(
                    &executor
                        .session_path(shell_path_to_windows(&path, &executor.shell_state.env_vars)),
                )?;
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
pub(in crate::executor) fn dup_source_fd_open(
    table: &FdTable,
    fd: u32,
    opened: &HashMap<u32, bool>,
) -> bool {
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
pub(in crate::executor) fn dup_bad_fd_name(
    raw_word: &str,
    redirector: u32,
    fallback: u32,
) -> String {
    let base = raw_word.strip_suffix('-').unwrap_or(raw_word);
    if !base.is_empty() && base.chars().all(|ch| ch.is_ascii_digit()) && raw_word == base {
        fallback.to_string()
    } else if redirector == 0 || redirector == 1 {
        base.to_string()
    } else {
        redirector.to_string()
    }
}
