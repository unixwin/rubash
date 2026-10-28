use super::*;
use crate::shell::var_table::VarTable;

/// array.c:516 array_insert over the sparse PIPESTATUS entry list:
/// an element already living at `index` is replaced in place (the
/// element count does not change); otherwise the entry is inserted
/// keeping the list sorted by index (rubash#1364 R3).
fn pipestatus_insert_or_replace(entries: &mut Vec<(i64, i32)>, index: i64, status: i32) {
    match entries.binary_search_by_key(&index, |(entry_index, _)| *entry_index) {
        Ok(position) => entries[position].1 = status,
        Err(position) => entries.insert(position, (index, status)),
    }
}

/// Stamp a line-number env key without allocating when the value is
/// unchanged. Byte-identical inserts are idempotent, so the equality gate
/// is behavior-neutral; it exists because a loop body re-stamps the same
/// line once per command (GNU's line_number is a C int — execute_cmd.c
/// SET_LINE_NUMBER — and costs nothing to re-assign).
fn stamp_env_usize(env_vars: &mut VarTable, key: &str, line: usize) -> bool {
    let mut buffer = [0u8; 20];
    let mut value = line;
    let mut position = buffer.len();
    loop {
        position -= 1;
        buffer[position] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let rendered = std::str::from_utf8(&buffer[position..]).unwrap_or("0");
    if env_vars.get(key).map(String::as_str) != Some(rendered) {
        env_vars.insert(key.to_string(), rendered.to_string());
        true
    } else {
        false
    }
}

impl Executor {
    pub fn last_exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn set_last_exit_code(&mut self, exit_code: i32) {
        self.set_exit_code(exit_code);
    }

    pub fn expand_prompt_string(&self, value: &str) -> String {
        // rubash#431: the ignore markers that `\[`/`\]` decode to are
        // width-accounting delimiters, never terminal payload (see
        // strip_prompt_ignore_markers) — every render channel gets the
        // stripped form, not just the piped `-i` driver.
        let decoded = self.decode_prompt_string(value);
        // parse.y:6683-6698: the expansion pass is promptvars/posix-gated.
        let decoded = if self.prompt_expansion_enabled() {
            self.expand_embedded_parameters_double_quoted(&decoded)
        } else {
            decoded
        };
        super::prompt_expansion::strip_prompt_ignore_markers(&decoded)
    }

    pub fn expand_prompt_string_mut(&mut self, value: &str) -> String {
        // GNU parse.y:6158-6160 prompt_again feeds PS1/PS2/PS0 through
        // decode_prompt_string as a READ-side render of the reader's next
        // prompt; it never writes the reader's status: WSL GNU Bash 5.3.0
        // piped-`-i` probes — PS1='$(false)' / PS0='$(false)' render before
        // the next read and the following `echo $?` still prints the
        // PRE-render value (issue rubash#604 probes gnu604.sh/gnu604ps0.sh).
        // The embedded-expansion engine underneath is the ASSIGNMENT engine:
        // its command-substitution boundary publishes the body's status for
        // the POSIX assignment exit rule (`v=$(false)` -> $? = 1), so the
        // render channel snapshots and restores both status words here.
        let saved_exit_code = self.exit_code;
        let saved_pipestatus = self.shell_state.pipestatus.clone();
        // The same parse.y:6684-6697 bracket covers last_command_subst_status
        // (:6687/:6697): the embedded engine's substitution boundary leaves
        // the rendered body's reaped status in the comsub-status cell, and a
        // residue here could be published by the next command's
        // substitution-publication sites. Restore it with the status words.
        let saved_comsub_status = self.last_command_substitution_status.replace(None);
        let decoded = self.decode_prompt_string(value);
        // Same promptvars/posix gate as the transform channel (parse.y:6683):
        // `shopt -u promptvars` stops the expansion pass entirely. The pass
        // runs in the double-quoted word context (parse.y:6689
        // expand_prompt_string(result, Q_DOUBLE_QUOTES, 0)) — the same
        // contract the `@P` transform channel gets from
        // expand_embedded_parameters_double_quoted.
        let rendered = if self.prompt_expansion_enabled() {
            // subst.c:4392-4418 containment: a verdict inside the prompt
            // word keeps its diagnostic and falls back to the decoded
            // string; the reader's status words are restored below either
            // way (rubash#604 probes).
            let bad_sub_before = self.shell_state.parameter_bad_substitution.replace(false);
            let arith_err_before = self.shell_state.arithmetic_expansion_error.replace(false);
            let arith_fatal_before = self.shell_state.arithmetic_fatal_error.replace(false);
            let arith_nonfatal_before = self.shell_state.arithmetic_nonfatal_error.replace(false);
            let expanded = self.expand_embedded_parameters_mut_with_context(
                &decoded,
                SubstitutionQuoteContext::DoubleQuoted,
            );
            let failed = self
                .shell_state
                .parameter_bad_substitution
                .replace(bad_sub_before)
                | self
                    .shell_state
                    .arithmetic_expansion_error
                    .replace(arith_err_before)
                | self
                    .shell_state
                    .arithmetic_fatal_error
                    .replace(arith_fatal_before)
                | self
                    .shell_state
                    .arithmetic_nonfatal_error
                    .replace(arith_nonfatal_before);
            if failed {
                decoded
            } else {
                expanded
            }
        } else {
            decoded
        };
        let rendered = super::prompt_expansion::strip_prompt_ignore_markers(&rendered);
        self.exit_code = saved_exit_code;
        self.shell_state.pipestatus = saved_pipestatus;
        self.last_command_substitution_status
            .set(saved_comsub_status);
        rendered
    }

    pub fn mark_parse_error(&mut self) {
        self.parse_error_occurred = true;
    }

    /// Latch a command-substitution body parse error. When the failing
    /// body is an old-style backtick substitution (ambient
    /// `backtick_body_diagnostic`), the companion backtick latch records
    /// the origin: GNU parses a backtick body in the substituted child at
    /// execution time (subst.c string_locale -> parse_and_execute), so the
    /// error kills only the child — substitution empty, status 2, script
    /// continues (issue #515) — while a `$(`-body error is a read-time
    /// failure of the enclosing command and stays fatal.
    pub(in crate::executor) fn latch_command_substitution_parse_error(&self) {
        self.last_command_substitution_parse_error.set(true);
        if self.backtick_body_diagnostic.get() {
            self.last_command_substitution_parse_error_backtick
                .set(true);
        }
    }

    /// GNU parse.y:6865/6886 (report_syntax_error): the offending-input-line
    /// echo (`print_offending_line', parse.y:6814) is suppressed only for
    /// the interactive shell's OWN input stream — a line read from a FILE
    /// echoes in both modes (verified vs the GNU 5.3.20 oracle:
    /// `printf 'if then\n' | bash -i` prints no `` `if then' `` line, but
    /// `bash -i -c 'source badrc'` and `--rcfile badrc -i` both print
    /// `bash: badrc: line 1: \`if then'`; the earlier `bash -i < err.sh`
    /// verification covered only the stdin-stream case). Every
    /// `__RUBASH_PARSE_SOURCE__' echo site must consult this gate.
    pub(in crate::executor) fn offending_line_echo_enabled(&self) -> bool {
        !self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_INTERACTIVE")
            || self.in_source_context()
    }

    /// GNU error.c:324-327 (parser_error): when `exit_immediately_on_error`
    /// is live at diagnostic time the first line of a syntax-error report is
    /// followed by an immediate exit_shell(2). The dynamic state mirrors
    /// GNU's: errexit enabled and not suspended (execute_cmd.c:4998-5016
    /// clears exit_immediately_on_error around eval/source under
    /// CMD_IGNORE_RETURN; rubash's with_errexit_suppressed marks the same
    /// contexts). Records the exit so the eval containment propagates the
    /// ExitCode instead of converting it into eval's rc=2.
    pub(in crate::executor) fn errexit_active_at_diagnostic(&self) -> bool {
        let active = self.errexit_enabled() && self.errexit_is_active();
        if active {
            self.parser_error_errexited.set(true);
        }
        active
    }

    /// Read-only form of GNU's live `exit_immediately_on_error` for callers
    /// outside crate::executor (the script drivers' parser_error sites):
    /// true when errexit is enabled and not suspended. GNU error.c:324-327
    /// forces exit status 2 at a parser_error under this state (rubash#306).
    pub(crate) fn errexit_live_at_diagnostic(&self) -> bool {
        self.errexit_enabled() && self.errexit_is_active()
    }

    /// Emit the stored `__RUBASH_PARSE_ERROR__`/`__RUBASH_PARSE_SOURCE__`
    /// diagnostic for a command node — the same text the parse-error arm in
    /// execute_command prints. Shared so a command-substitution body that
    /// parsed to an error node can report GNU's `syntax error near ...`
    /// lines without executing the broken ast.
    ///
    /// Every line goes through write_diagnostic_fd2, never raw eprintln:
    /// GNU parser_error (error.c) writes to fd 2 AS CURRENTLY BOUND, so a
    /// parse error inside an eval/source body honors the caller's
    /// redirections (`eval '...' 2>&1` routes the diagnostic to stdout;
    /// rubash#1609, the #167 family).
    pub(in crate::executor) fn report_command_parse_error(&self, cmd: &crate::parser::CommandNode) {
        // The compound-EOF shape (parse.y yyerror EOF path, printed by the
        // execute_command arm for top-level nodes): a node carrying the
        // EOF_COMPOUND marker reports `syntax error: unexpected end of
        // file from `X' command on line N' — the plain-marker arm below
        // would misrender it as the near shape. Under the ambient backtick
        // window the prefix is GNU's parse_and_execute child name (issue
        // #515); otherwise the enclosing script's.
        if let Some(spec) = cmd.get_assignment("__RUBASH_PARSE_ERROR_EOF_COMPOUND__") {
            let mut fields = spec.split(crate::executor::markers::PARSE_ERROR_FIELD_SEP);
            let compound_name = fields.next().unwrap_or("(");
            let _open_line = fields
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1);
            let eof_line = fields
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1);
            let prefix = if self.backtick_body_diagnostic.get() {
                self.comsub_eof_error_diagnostic(eof_line)
            } else {
                self.parser_error_prefix_for_line(eof_line)
            };
            self.write_diagnostic_fd2(
                format!(
                    "{prefix}syntax error: unexpected end of file from `{compound_name}' command on line {_open_line}\n"
                )
                .as_bytes(),
            );
            return;
        }
        let message = cmd
            .get_assignment("__RUBASH_PARSE_ERROR__")
            .map(String::as_str)
            .unwrap_or("unexpected token");
        // GNU parser_error (parse.y:6839 via error.c:300-316) cites the
        // parser's line_number AT the error — the offending token's line,
        // which the error node records (push_unexpected_token_error and the
        // subtree propagation both stamp it). A multi-line script's ambient
        // CURRENT_LINE is the LAST line read, not the failure line
        // (rubash#390: `while :; do case x in\nesac) ...` reports the
        // `esac)' line, not the `done' line).
        // A BACKTICK body's report renders under GNU's parse_and_execute
        // child name (subst.c:7413 passes "command substitution" as whom;
        // error.c:300-316 prefixes `name: command substitution: line N: `)
        // — the enclosing script's prefix is the `$(`-body shape, where the
        // outer parse owns the diagnostic (issue #515).
        let prefix = match cmd.line {
            Some(line) if self.backtick_body_diagnostic.get() => {
                self.comsub_eof_error_diagnostic(line)
            }
            Some(line) => self.parser_error_prefix_for_line(line),
            None => self.parser_error_prefix(),
        };
        if message.starts_with("syntax error:") || message.starts_with("arithmetic syntax error:") {
            self.write_diagnostic_fd2(format!("{prefix}{message}\n").as_bytes());
            // GNU error.c:324-327 (parser_error): with
            // exit_immediately_on_error set, the first diagnostic line is
            // followed by an immediate exit_shell(2) — the second line of
            // the report never prints and no enclosing eval/source
            // containment applies (rubash#306).
            if self.errexit_active_at_diagnostic() {
                return;
            }
            // parse.y:6865: the offending-line echo is non-interactive only.
            if self.offending_line_echo_enabled() {
                if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                    self.write_diagnostic_fd2(
                        format!("{prefix}syntax error: `{source}'\n").as_bytes(),
                    );
                }
            }
        } else {
            let message = super::command_execute::bash_style_unexpected_token_message(message);
            self.write_diagnostic_fd2(format!("{prefix}syntax error near {message}\n").as_bytes());
            if self.errexit_active_at_diagnostic() {
                return;
            }
            // GNU parse.y:6813-6826 print_offending_line echoes the current
            // shell_input_line verbatim — only trailing newlines are
            // stripped; leading whitespace and the original spacing
            // (`if :;then ...') stay as read (rubash#285: the old
            // trim/`;then' display rewrite corrupted the physical line).
            // parse.y:6865: the echo is non-interactive only.
            if self.offending_line_echo_enabled() {
                if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                    self.write_diagnostic_fd2(format!("{prefix}`{source}'\n").as_bytes());
                }
            }
        }
    }

    /// Pretty-print driver entry (`bash --pretty-print`, eval.c:215-253
    /// pretty_print_loop): a command node carrying a parse-error marker
    /// makes read_command() longjmp — yyerror's stderr diagnostic is the
    /// ONLY output for that command (nothing reaches stdout) and the loop
    /// returns EXECUTION_FAILURE. The internal `__RUBASH_PARSE_*` marker
    /// assignments must never pretty-print as command text.
    pub fn report_pretty_print_parse_error(&self, cmd: &crate::parser::CommandNode) {
        if let Some(spec) = cmd.get_assignment("__RUBASH_PARSE_ERROR_NEAR__") {
            let mut fields = spec.split(crate::executor::markers::PARSE_ERROR_FIELD_SEP);
            let token = fields.next().unwrap_or_default();
            let line = fields
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or_else(|| cmd.line.unwrap_or(1));
            let prefix = self.parser_error_prefix_for_line(line);
            self.write_diagnostic_fd2(
                format!("{prefix}syntax error near unexpected token `{token}'\n").as_bytes(),
            );
            // parse.y:6865: the offending-line echo is non-interactive only.
            if self.offending_line_echo_enabled() {
                if let Some(source) = cmd.get_assignment("__RUBASH_PARSE_SOURCE__") {
                    self.write_diagnostic_fd2(format!("{prefix}`{source}'\n").as_bytes());
                }
            }
            return;
        }
        self.report_command_parse_error(cmd);
    }

    pub fn take_parse_error(&mut self) -> bool {
        std::mem::take(&mut self.parse_error_occurred)
    }

    /// rubash#1662: take-style read of the signal-discard mark (see
    /// Executor::signal_discard_pending).
    pub fn take_signal_discard_pending(&mut self) -> bool {
        std::mem::take(&mut self.signal_discard_pending)
    }

    /// Tier A signal face (Session API RFC section 6.4; PLAN row
    /// 11, M-3): queue `signal` for this process through the
    /// engine's own delivery machinery — the same queue the
    /// `kill` builtin's self-deliveries use. The delivery is
    /// serviced at the next command boundary by the boundary
    /// poll: a trapped signal runs its trap, an ignored one is
    /// dropped, an untrapped fatal one ends the run with
    /// 128 + signal. This replaces the `builtins::kill` console
    /// hooks as the surface hosts consume; those hooks are the
    /// engine's internal plumbing from here on.
    pub fn inject_signal(&mut self, signal: i32) {
        crate::builtins::kill::queue_self_signal(signal);
    }

    /// Tier A cooperative cancellation (RFC section 6.4): ask
    /// the running execution to end as if SIGINT had arrived —
    /// an injected SIGINT, serviced at the next command
    /// boundary with traps honored and the conventional 130
    /// status for an untrapped interrupt.
    pub fn interrupt(&mut self) {
        self.inject_signal(2);
    }

    /// GNU execute_cmd.c:652-656: `!` gives the command CMD_IGNORE_RETURN
    /// under errexit — the grouped drivers' post-group `status != 0 &&
    /// errexit` check reads this to honor the exemption.
    pub fn last_command_inverted(&self) -> bool {
        self.last_command_inverted.get()
    }

    /// GNU jump_to_top_level(EXITPROG/ERREXIT/FORCE_EOF): the group's
    /// execute_ast ended by unwinding rather than by an ordinary command
    /// status — the grouped drivers must stop reading. Take-style: each
    /// check clears the flag.
    pub fn take_exit_jump_pending(&mut self) -> bool {
        self.exit_jump_pending.replace(false)
    }

    pub fn shell_state(&self) -> &crate::shell::ShellState {
        &self.shell_state
    }

    /// rubash#547: the startup `-o`/`+o` list form (`bash -o` with no
    /// option argument). GNU shell.c routes the argument-less case to the
    /// same listing `set -o` / `set +o` prints (options.c
    /// print_all_shell_options): `-o` renders the `name\tstate` table,
    /// `+o` the `set +o name` / `set -o name` recreate list.
    pub fn print_shell_options_listing<W: std::io::Write>(
        &self,
        recreate: bool,
        stdout: &mut W,
    ) -> std::io::Result<()> {
        crate::builtins::set::print_shell_options(
            &self.shell_state.env_vars,
            recreate,
            crate::builtins::set::SET_O_PRINT_WIDTH,
            stdout,
        )
    }

    /// rubash#780: the startup `-O`/`+O` list form (`bash -O` with no option
    /// argument). GNU shell.c routes the argument-less case to the shopt
    /// table, so the bytes are the builtin's: `-O` renders `name<TAB>on|off`
    /// (the bare `shopt` listing), `+O` the `shopt -s|-u name` recreate list
    /// (the `shopt -p` listing).
    pub fn print_shopt_options_listing<W: std::io::Write>(
        &self,
        recreate: bool,
        stdout: &mut W,
    ) -> std::io::Result<()> {
        crate::builtins::shopt::print_shopt_listing(&self.shell_state.env_vars, recreate, stdout)
    }

    pub fn shell_state_mut(&mut self) -> &mut crate::shell::ShellState {
        &mut self.shell_state
    }

    pub(crate) fn set_exit_code(&mut self, exit_code: i32) {
        self.exit_code = exit_code;
    }

    pub fn set_history_provider(&mut self, provider: crate::history::SharedHistoryProvider) {
        self.history_provider = Some(provider);
    }

    /// Whether a host HistoryProvider owns the interactive history list.
    /// builtins/history.def reads the shell's ONE list; an installed
    /// provider IS that list (job_builtins execute_history's provider
    /// precedence encodes it), so grouped-reader scratch sessions must key
    /// their install on this and never shadow it (unixwin/niubash#508).
    pub(crate) fn has_history_provider(&self) -> bool {
        self.history_provider.is_some()
    }

    /// Snapshot of the host-installed history provider's list (oldest to
    /// newest), for interactive history expansion. `None` when no provider
    /// is installed — the engine's session history is the list then.
    pub(crate) fn history_provider_snapshot(&mut self) -> Option<Vec<String>> {
        let provider = self.history_provider.as_ref()?.clone();
        let entries = provider.borrow_mut().entries().unwrap_or_default();
        Some(entries)
    }

    /// Rewrite the provider's most recent entry while it is still the raw
    /// line the host's editor recorded before execution — the interactive
    /// pre_process_line counterpart of bashhist.c maybe_add_history:
    /// `Some(text)` records the expansion in the raw line's place (GNU
    /// records the expanded line), `None` removes it (a failed expansion
    /// records nothing). No-op when no provider is installed or the last
    /// entry is no longer `raw`.
    pub(crate) fn history_provider_replace_last(&mut self, raw: &str, replacement: Option<String>) {
        let Some(provider) = self.history_provider.as_ref().cloned() else {
            return;
        };
        let Ok(mut entries) = provider.borrow_mut().entries() else {
            return;
        };
        if entries.last().map(String::as_str) != Some(raw) {
            return;
        }
        match replacement {
            Some(text) => {
                let last = entries.len() - 1;
                entries[last] = text;
            }
            None => {
                entries.pop();
            }
        }
        let _ = provider.borrow_mut().replace(entries);
    }

    /// Remove the provider's most recent entry unconditionally (the
    /// interactive recording veto — see
    /// script_driver::interactive_history_veto_recording). No-op when the
    /// list is empty or no provider is installed.
    pub(crate) fn history_provider_drop_last(&mut self) {
        let Some(provider) = self.history_provider.as_ref().cloned() else {
            return;
        };
        let Ok(mut entries) = provider.borrow_mut().entries() else {
            return;
        };
        entries.pop();
        let _ = provider.borrow_mut().replace(entries);
    }

    /// Host completion hook. Given the in-progress command `line` and the
    /// `cursor` position, return completion candidates for the word under the
    /// cursor, honoring the compspec registered for the command (if any) and
    /// falling back to command completion (first word) or file completion.
    ///
    /// Candidate generation reuses the same GNU programmable-completion engine
    /// as the `compgen` builtin (see
    /// `crate::builtins::complete::complete_line_candidates`). The host (niubash)
    /// wires this into its reedline completer so the completion *engine* lives in
    /// rubash while the *UI* stays in the host — the same split as
    /// `HistoryProvider` keeps storage on the host.
    ///
    /// Dynamic compspec actions `-C` (external command) and `-F` (shell function
    /// filling `COMPREPLY`) are resolved by the executor before candidate
    /// generation (`completion_dynamic_for_line` runs the governing
    /// compspec's hook with the live COMP_* context bound) and merged by
    /// `complete_line_candidates_with_dynamic` (niubash#520).
    pub fn complete_line(&mut self, line: &str, cursor: usize) -> Vec<String> {
        let function_names: Vec<String> = self.shell_state.functions.keys().cloned().collect();
        // Job completion names are the command's first word only
        // (rubash#1052; same rule as the compgen path in job_builtins).
        let job_names: Vec<String> = self
            .shell_state
            .job_table
            .jobs
            .values()
            .filter(|job| job.background)
            .map(|job| {
                job.command
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        let dynamic = self.completion_dynamic_for_line(line, cursor);
        crate::builtins::complete::complete_line_candidates_with_dynamic(
            line,
            cursor,
            &self.shell_state.completion_specs,
            &self.shell_state.env_vars,
            &self.shell_state.aliases,
            &function_names,
            &job_names,
            dynamic,
        )
    }

    /// The canonical list of shell builtin command names (mirrors
    /// `BUILTIN_NAMES`). Host completion uses this instead of hardcoding the
    /// list, so the two never drift apart when builtins are added or removed.
    pub fn builtin_command_names() -> &'static [&'static str] {
        crate::executor::builtin_names::builtin_names()
    }

    /// Whether a compspec is registered for `command` (via the `complete`
    /// builtin). Hosts use this as a cheap gate before calling
    /// [`Executor::complete_line`], so they only pay for GNU-engine candidate
    /// generation when a compspec actually applies. The `-D` default and
    /// `-E` empty-line compspecs govern lines whose command has no own
    /// registration (niubash#520), so their presence also opens the gate.
    pub fn has_compspec(&self, command: &str) -> bool {
        self.shell_state.completion_specs.get(command).is_some()
            || self.shell_state.completion_specs.get("-D").is_some()
            || self.shell_state.completion_specs.get("-E").is_some()
    }

    pub fn set_external_file_builtins_enabled(&mut self, enabled: bool) {
        self.external_file_builtins_enabled = enabled;
    }

    /// Configure the native directory that backs the shell-visible `/` root.
    ///
    /// This is a path spelling adapter, not a POSIX runtime: the configured
    /// directory itself contains the real `bin`, `usr/bin`, `etc`, and `tmp`
    /// directories used by the shell.
    pub fn set_shell_root(&mut self, root: impl AsRef<std::path::Path>) {
        let value = root.as_ref().to_string_lossy().into_owned();
        self.shell_state
            .env_vars
            .insert("__RUBASH_SHELL_ROOT".to_string(), value.clone());
        // deprecated: niu bridge, remove after niu stops reading
        self.shell_state
            .env_vars
            .insert("WINUXSH_ROOT".to_string(), value);
        self.mark_exported("WINUXSH_ROOT");
    }

    /// Configure the native WinuxCmd dispatcher used for shell commands that
    /// are not backed by a file in the configured installation root.
    pub fn set_winuxcmd_path(&mut self, path: impl AsRef<std::path::Path>) {
        let path = path.as_ref();
        self.shell_state.env_vars.insert(
            "WINUXCMD_PATH".to_string(),
            path.to_string_lossy().into_owned(),
        );
        self.mark_exported("WINUXCMD_PATH");
        let installation_root = crate::executor::path::winuxcmd_installation_root_from_path(path);
        self.shell_state.env_vars.insert(
            "WINUXCMD_HOME".to_string(),
            installation_root.to_string_lossy().into_owned(),
        );
        self.mark_exported("WINUXCMD_HOME");
    }

    /// Configure an explicit external Bash-compatible shell for text-script
    /// fallbacks. Rubash does not probe sh or bash on Windows by default;
    /// host layers must opt in deliberately when they want that compatibility.
    pub fn set_compatible_shell_path(&mut self, path: impl AsRef<std::path::Path>) {
        self.shell_state.env_vars.insert(
            crate::executor::path::COMPATIBLE_SHELL_PATH_ENV.to_string(),
            path.as_ref().to_string_lossy().into_owned(),
        );
    }

    pub fn clear_compatible_shell_path(&mut self) {
        self.shell_state
            .env_vars
            .remove(crate::executor::path::COMPATIBLE_SHELL_PATH_ENV);
    }

    /// Resolve a shell-visible path using the executor's current namespace.
    pub fn resolve_shell_path(&self, path: &str) -> std::path::PathBuf {
        Self::resolve_shell_path_from_env(path, &self.shell_state.env_vars)
    }

    /// Display-domain form of a script identity path ($0 / BASH_SOURCE's
    /// bottom frame). GNU shell.c:1572-1601 open_shell_script binds
    /// dollar_vars[0] to the script name exactly as the caller spelled it;
    /// rubash's native launch cannot see that spelling (an MSYS-style
    /// caller's `/d/...` argv arrives converted as `D:/...`), so map a
    /// drive-form name back into the domain PWD displays in
    /// (shell_pwd_display_path) — $0, BASH_SOURCE and $PWD must compose in
    /// ONE path domain or dirname/parameter-chops build mixed-domain paths
    /// (rubash#224: bats-core's `${BATS_TEST_FILENAME##*/}` on a `D:/`
    /// BASH_SOURCE produced `1-D:\...src`).
    pub fn script_identity_display_path(&self, path: &str) -> String {
        crate::executor::path::shell_pwd_display_path(path)
    }

    /// Resolve a shell-visible path using an executor environment snapshot.
    ///
    /// Host layers use this when they have an environment map but do not own
    /// the executor instance, such as process-plugin working-directory setup.
    pub fn resolve_shell_path_from_env(
        path: &str,
        env_vars: &std::collections::HashMap<String, String>,
    ) -> std::path::PathBuf {
        crate::executor::path::resolve_shell_path_from_env(path, env_vars)
    }

    /// Resolve one shell PATH entry into native directories for a Windows
    /// child. Logical command directories resolve to their real backing
    /// directory below the configured shell root.
    pub fn resolve_shell_path_process_entries_from_env(
        path: &str,
        env_vars: &std::collections::HashMap<String, String>,
    ) -> Vec<std::path::PathBuf> {
        crate::executor::path::shell_path_process_entries(path, env_vars)
    }

    pub fn set_host_external_command_handler<F>(&mut self, handler: F)
    where
        F: FnMut(&[String], &HashMap<String, String>) -> Option<HostExternalCommandOutput>
            + 'static,
    {
        self.host_external_command_handler = Some(HostExternalCommandHandler(Box::new(handler)));
    }

    #[cfg(windows)]
    pub fn set_elevation_handler<F>(&mut self, handler: F)
    where
        F: FnMut(ElevationRequest) -> Result<ElevationOutput, String> + 'static,
    {
        self.elevation_handler = Some(ElevationHandler(Box::new(handler)));
    }

    #[cfg(windows)]
    pub fn clear_elevation_handler(&mut self) {
        self.elevation_handler = None;
    }

    pub fn set_env(&mut self, name: &str, value: &str) {
        let value = if name == "TMPDIR" && value.contains('\0') {
            safe_temp_dir_string()
        } else {
            value.to_string()
        };
        self.shell_state
            .env_vars
            .insert(name.to_string(), value.clone());
        if is_valid_process_env(name, &value) {
            // G-03: a host-directed write is this lineage's mirror write
            // and must be journaled exactly like remove_env's — set_env
            // is called between runs, when no journal is on the stack,
            // so without the push the write escaped the journal and
            // leaked past Drop into every later Executor in the process
            // (the __RUBASH_IS_C / __RUBASH_INTERACTIVE protocol markers
            // are the observed case: a sibling session's markers became
            // a later session's invocation context at Executor::new's
            // process-env import).
            let _journal = push_env_journal(&self.env_write_journal);
            set_process_env(name, &value);
        }
        // Keep the shell-variable view in sync with the environment mirror.
        // `shell_state.variables` is seeded from `env_vars` at `Executor::new`
        // (init.rs `from_environment`), so every exported env var also exists
        // as a scalar shell variable. `set_env` only updated `env_vars`, which
        // made `$VAR` (shell_variable_value reads shell_state first) return a
        // stale value while tilde/`home_value` (reads env_vars) returned the
        // new one — the `$HOME` vs `~` inconsistency (niubash issue #90) and the
        // same class of bug that PWD/OPTIND/export already paper over with
        // manual syncs. Update the scalar in place so attributes (readonly,
        // integer, exported, case-mod) are preserved; arrays/assoc and absent
        // variables are left untouched, matching the existing sync sites.
        if let Some(variable) = self.shell_state.variables.get_mut(name) {
            if let crate::shell::ShellValue::Scalar(current) = &mut variable.value {
                *current = value.clone();
            }
        }
        if name == "__RUBASH_IS_C" {
            // GNU: a `-c` command string has no script file, so BASH_SOURCE
            // starts as a set-but-EMPTY array; only function entry pushes
            // the $0 operand (via current_bash_source's fallback). The
            // __RUBASH_SCRIPT_NAME installed for the same invocation
            // carries that $0 operand and seeded the stack when it was set
            // (the name is set before IS_C at every startup site —
            // invocation.rs / main.rs); the STORED array is emptied here
            // (rubash#1306). The stack seed itself must SURVIVE: the
            // BASH_SOURCE view pops the bottom seed when there is no main
            // frame, and function entry reads the seed through
            // current_bash_source — clearing the stack made the first
            // function's own frame the bottom entry, so the view popped
            // it and in-function BASH_SOURCE read empty (integration
            // fixup; cli_tests function_call_stack_* encode the GNU
            // shape). The SCRIPT_NAME branch below guards the reverse
            // seeding order.
            store_indexed_array_state(&mut self.shell_state, "BASH_SOURCE", Vec::<String>::new());
        }
        if name == "__RUBASH_SCRIPT_NAME" {
            let source_value = if self
                .shell_state
                .env_vars
                .get("__RUBASH_TOP_LEVEL_NAME")
                .is_some_and(|name| name.rsplit(['/', '\\']).next() == Some("bashdb-generated"))
                && !value.starts_with('/')
            {
                std::fs::canonicalize(self.resolve_shell_path(&value))
                    .ok()
                    .map(|path| Self::debugger_source_path(&path))
                    .unwrap_or_else(|| value.clone())
            } else {
                value.clone()
            };
            // GNU evalfile.c:253 pushes the sourced file onto BASH_SOURCE
            // instead of resetting the stack: inside `source file`, frames for
            // the sourcing function and the main script stay visible
            // (dbg-support.sub traces "FUNCNAME[1]: source called from
            // ./dbg-support.tests"), so only a top-level script-name change
            // (startup, -c, bashdb launcher) rebinds BASH_SOURCE.
            if self.shell_state.env_vars.contains_key("__RUBASH_IN_SOURCE") {
                let values = self.shell_state.bash_source_stack.clone();
                store_indexed_array_state(&mut self.shell_state, "BASH_SOURCE", values);
                return;
            }
            if self.shell_state.env_vars.contains_key("__RUBASH_IS_C") {
                // `-c` invocation (host seeded IS_C before the name):
                // BASH_SOURCE stays the set-but-empty array (rubash#1306),
                // but the stack still takes the $0 seed (see the IS_C
                // branch above for why the seed must survive).
                self.shell_state.bash_source_stack = vec![source_value.clone()];
                store_indexed_array_state(
                    &mut self.shell_state,
                    "BASH_SOURCE",
                    Vec::<String>::new(),
                );
                return;
            }
            self.shell_state.bash_source_stack = vec![source_value.clone()];
            store_indexed_array_state(&mut self.shell_state, "BASH_SOURCE", vec![source_value]);
        }
    }

    pub fn export_env(&mut self, name: &str, value: &str) {
        self.set_env(name, value);
        self.mark_exported(name);
    }

    pub fn unset_env(&mut self, name: &str) {
        self.remove_env(name);
    }

    /// Enable or disable a dispatch builtin without executing shell source.
    /// This allows embedding hosts to apply policy defaults before startup files.
    pub fn set_builtin_disabled(&mut self, name: &str, disabled: bool) {
        crate::builtins::enable::set_disabled(&mut self.shell_state.env_vars, name, disabled);
    }

    pub fn remove_env(&mut self, name: &str) {
        self.shell_state.env_vars.remove(name);
        // G-03: a host-directed removal is still this lineage's
        // mirror write — journal it so Drop treats it like any other.
        let _journal = push_env_journal(&self.env_write_journal);
        mirror_env_remove(name);
    }

    pub fn get_env(&self, name: &str) -> Option<&str> {
        self.shell_state.env_vars.get(name).map(|s| s.as_str())
    }

    /// Attach the session history list used by scripts that enable history.
    pub fn set_session_history(
        &mut self,
        session: Option<std::rc::Rc<std::cell::RefCell<crate::history::SessionHistory>>>,
    ) {
        self.shell_state.session_history = session;
    }

    /// Returns a clone of the session history handle, if any.
    pub fn get_session_history(
        &self,
    ) -> Option<std::rc::Rc<std::cell::RefCell<crate::history::SessionHistory>>> {
        self.shell_state.session_history.clone()
    }

    /// Returns the session history list, creating and installing an
    /// empty one on demand when the shell has none. This is the history
    /// builtin's creation rule (builtins/history.def): the list exists
    /// for history's consumers even in shells that never installed one
    /// (the non-interactive stdin driver), but an installed host
    /// provider IS the history list, so an auto-created empty session
    /// must not shadow it — with a provider and no session this
    /// returns None (rubash#1668).
    pub fn ensure_session_history(
        &mut self,
    ) -> Option<std::rc::Rc<std::cell::RefCell<crate::history::SessionHistory>>> {
        if self.shell_state.session_history.is_none() && !self.has_history_provider() {
            let session = std::rc::Rc::new(std::cell::RefCell::new(
                crate::history::SessionHistory::new(),
            ));
            self.set_session_history(Some(session));
        }
        self.get_session_history()
    }

    /// Run `body` with user trap delivery (DEBUG/RETURN/ERR) suspended.
    ///
    /// GNU has no notion of host housekeeping commands: when an embedding
    /// host runs internal commands through `execute_ast` — startup or
    /// shutdown hook probes, rc cleanup such as `unset HISTFILE`, plugin
    /// existence checks — those commands must not fire the script's
    /// traps. Without this boundary, commands a host issues after the
    /// script reached EOF emit extra DEBUG/RETURN trap lines that GNU
    /// Bash never produces (engine sinking list S2: the one-line
    /// dbg-support/dbg-support2 product-layer diffs).
    pub fn with_traps_suspended<R>(&mut self, body: impl FnOnce(&mut Self) -> R) -> R {
        self.host_internal_depth
            .set(self.host_internal_depth.get() + 1);
        let result = body(self);
        self.host_internal_depth
            .set(self.host_internal_depth.get().saturating_sub(1));
        result
    }

    pub(crate) fn push_bash_source(&mut self, source: String) {
        let source = if self
            .shell_state
            .env_vars
            .get("__RUBASH_TOP_LEVEL_NAME")
            .is_some_and(|name| name.rsplit(['/', '\\']).next() == Some("bashdb-generated"))
        {
            if !source.starts_with('/') {
                std::fs::canonicalize(self.resolve_shell_path(&source))
                    .ok()
                    .map(|path| Self::debugger_source_path(&path))
                    .unwrap_or(source.clone())
            } else {
                source.clone()
            }
        } else {
            source
        };
        self.shell_state.bash_source_stack.insert(0, source);
        let values = self.shell_state.bash_source_stack.clone();
        store_indexed_array_state(&mut self.shell_state, "BASH_SOURCE", values);
    }

    fn debugger_source_path(path: &std::path::Path) -> String {
        let display = path.to_string_lossy().replace('\\', "/");
        let display = display.strip_prefix("//?/").unwrap_or(&display);
        if let Some((drive, rest)) = display.split_once(":") {
            if drive.len() == 1 && !rest.is_empty() {
                return format!("/{}{}", drive.to_lowercase(), rest);
            }
        }
        display.to_string()
    }

    pub(crate) fn pop_bash_source(&mut self) {
        if !self.shell_state.bash_source_stack.is_empty() {
            self.shell_state.bash_source_stack.remove(0);
        }
        let values = self.shell_state.bash_source_stack.clone();
        store_indexed_array_state(&mut self.shell_state, "BASH_SOURCE", values);
    }

    /// Pushes the synthetic "source" call frame for a `source`/`.` of a
    /// file: GNU executes a sourced file with FUNCNAME[0]="source" and
    /// BASH_LINENO[0]=the source command's line (dbg-support.tests: the
    /// DEBUG trap inside dbg-support.sub reports FUNCNAME[1]="source", and
    /// dbg-support.sub's stack trace shows a "source" frame between
    /// sourced_fn and the sourcing function).
    pub(crate) fn push_source_call_frame(&mut self, call_line: String) {
        self.shell_state
            .function_name_stack
            .insert(0, "source".to_string());
        self.shell_state.bash_lineno_stack.insert(0, call_line);
        let values = self.shell_state.bash_lineno_stack.clone();
        store_indexed_array_state(&mut self.shell_state, "BASH_LINENO", values);
    }

    pub(crate) fn pop_source_call_frame(&mut self) {
        if self
            .shell_state
            .function_name_stack
            .first()
            .map(String::as_str)
            == Some("source")
        {
            self.shell_state.function_name_stack.remove(0);
        }
        if !self.shell_state.bash_lineno_stack.is_empty() {
            self.shell_state.bash_lineno_stack.remove(0);
        }
        let values = self.shell_state.bash_lineno_stack.clone();
        store_indexed_array_state(&mut self.shell_state, "BASH_LINENO", values);
    }

    /// Returns whether a shell function is currently defined in this executor.
    pub fn has_function(&self, name: &str) -> bool {
        self.function_name_for_command_word(name).is_some()
    }

    /// Invokes a defined shell function directly, bypassing builtin and PATH lookup.
    ///
    /// Returns the function body's final shell status. If the function is not
    /// defined, returns `ExecuteError::FunctionNotFound`.
    pub fn call_function<I, S>(&mut self, name: &str, args: I) -> Result<i32, ExecuteError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.call_function_with_env(name, args, std::iter::empty::<(&str, &str)>())
    }

    /// Invokes a shell function with temporary environment variables.
    ///
    /// Each provided temporary variable is restored to its previous value after
    /// the call, while unrelated function side effects remain in the executor.
    pub fn call_function_with_env<I, S, E, K, V>(
        &mut self,
        name: &str,
        args: I,
        temporary_env: E,
    ) -> Result<i32, ExecuteError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
        E: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_string())
            .collect::<Vec<_>>();
        let temporary_env = temporary_env
            .into_iter()
            .map(|(name, value)| (name.as_ref().to_string(), value.as_ref().to_string()))
            .collect::<Vec<_>>();
        self.call_function_owned(name, &args, &temporary_env)
    }

    fn call_function_owned(
        &mut self,
        name: &str,
        args: &[String],
        temporary_env: &[(String, String)],
    ) -> Result<i32, ExecuteError> {
        if EXECUTION_LOCK_DEPTH.with(|depth| depth.get() > 0) {
            return self.call_function_inner(name, args, temporary_env);
        }

        // G-01: no whole-call lock hold anymore (see execute_ast).
        // G-02: the pre-call cwd restore now targets the session cwd field
        // (formerly the process cwd). The $PWD variable is deliberately
        // NOT restored, preserving the pre-G-02 observable shape.
        let original_cwd = self.shell_state.cwd.clone();
        EXECUTION_LOCK_DEPTH.with(|depth| depth.set(1));
        // G-03: journal this call's mirror writes to this executor.
        let _env_journal_guard = push_env_journal(&self.env_write_journal);
        let result = self.call_function_inner(name, args, temporary_env);
        EXECUTION_LOCK_DEPTH.with(|depth| depth.set(0));
        self.shell_state.cwd = original_cwd;
        result
    }

    fn call_function_inner(
        &mut self,
        name: &str,
        args: &[String],
        temporary_env: &[(String, String)],
    ) -> Result<i32, ExecuteError> {
        let Some(function_name) = self.function_name_for_command_word(name) else {
            return Err(ExecuteError::FunctionNotFound(name.to_string()));
        };
        let saved_env = temporary_env
            .iter()
            .map(|(name, _)| (name.clone(), self.shell_state.env_vars.get(name).cloned()))
            .collect::<Vec<_>>();
        for (name, value) in temporary_env {
            self.set_env(name, value);
        }

        let call_cmd = CommandNode::new();
        let result = self.execute_function(&function_name, args, &call_cmd);
        let status = self.exit_code;

        for (name, value) in saved_env.into_iter().rev() {
            match value {
                Some(value) => self.set_env(&name, &value),
                None => self.remove_env(&name),
            }
        }

        result.map(|_| status)
    }

    pub fn set_shell_option(&mut self, name: &str, enabled: bool) {
        crate::builtins::set::set_shell_option(&mut self.shell_state.env_vars, name, enabled);
        // Startup-facing entry (CLI --posix/-o, invocation apply, sh
        // argv0): GNU initialize_bashopts (shopt.def) snapshots $BASHOPTS
        // AFTER startup option processing, so a startup posix flip lands
        // in the snapshot. The runtime `set` builtin uses the free
        // function directly and must NOT re-snapshot (rubash#1055:
        // posix_initialize bypasses set_bashopts at runtime).
        if name == "posix" {
            let bashopts = crate::builtins::shopt::bashopts_value(&self.shell_state.env_vars);
            self.shell_state
                .env_vars
                .insert("BASHOPTS".to_string(), bashopts);
        }
    }

    /// GNU shell.c:1587-1601 (open_shell_script): a script filename that is
    /// not found as given and carries no path separator is searched in
    /// $PATH (find_path_file, findcmd.c:258) before the shell gives up.
    pub fn find_script_on_path(&self, name: &str) -> Option<std::path::PathBuf> {
        crate::executor::path::find_user_command(name, &self.shell_state.env_vars)
    }

    pub fn is_shell_option(&self, name: &str) -> bool {
        crate::builtins::set::is_shell_option(name)
    }

    pub fn set_shopt_option(&mut self, name: &str, enabled: bool) -> bool {
        if !crate::builtins::shopt::is_supported_option(name) {
            return false;
        }
        let _ = crate::builtins::shopt::set_option(&mut self.shell_state.env_vars, name, enabled);
        self.sync_shopt_rebound_var();
        true
    }

    /// Mark this executor as a login shell (GNU `LOGIN_SHELL`): records
    /// the `__RUBASH_LOGIN_SHELL` carrier the startup/profile/logout
    /// paths read, and reports the `login_shell` shopt as on (`shopt -q
    /// login_shell`). Every login marking (flag, argv[0] `-` mark, lib
    /// API) funnels through here so the shopt can never drift from the
    /// carrier.
    pub fn mark_login_shell(&mut self) {
        self.set_env("__RUBASH_LOGIN_SHELL", "1");
        self.set_shopt_option("login_shell", true);
    }

    pub(in crate::executor) fn restore_shell_env(
        &mut self,
        saved_env: crate::shell::var_table::VarTable,
    ) {
        let old_names: Vec<String> = self.shell_state.env_vars.keys().cloned().collect();
        for name in old_names {
            if !saved_env.contains_key(&name) {
                // G-03: journal the removal as this lineage's write.
                mirror_env_remove(&name);
            }
        }

        for (name, value) in &saved_env {
            // rubash#237 perf: every map mutation that reaches the process
            // environment went through set_process_env/sync (they update
            // both stores), so a variable whose current map value already
            // equals the saved value never changed in the process env
            // either — skip the setenv. A flat-subshell exit used to
            // rewrite the whole environ (dozens of locked libc setenv
            // calls) per pipeline stage, hundreds of thousands of times
            // across ifs-posix.
            if self.shell_state.env_vars.get(name) == Some(value) {
                continue;
            }
            if is_valid_process_env(name, value) {
                set_process_env(name, value);
            } else {
                mirror_env_remove(name);
            }
        }

        self.shell_state.env_vars = saved_env;
    }

    /// GNU execute_cmd.c:1576 execute_in_subshell: a `( )` subshell's parent
    /// state is the pre-entry ShellState clone — restore it wholesale so
    /// every semantic field (aliases, functions, scopes, positional params,
    /// env-carried traps, job bookkeeping) returns to the parent value.
    /// The process env is a shared in-place resource, so it resyncs
    /// separately: restore_shell_env must run before the swap because it
    /// diffs the child's env_vars against the saved map. The cwd needs no
    /// separate resync since G-02: it is a ShellState field and rides the
    /// wholesale swap.
    pub(in crate::executor) fn restore_flat_subshell(
        &mut self,
        saved_state: crate::shell::ShellState,
    ) {
        // Every caller is a fork boundary (flat `( )` regions and
        // command-substitution children): the region's descriptor copies
        // close here, so run the deferred bodies of the output process
        // substitutions the region opened before the parent's env (which
        // carries their registrations) is swapped back (rubash#1336).
        self.flush_region_owned_output_process_substitutions(&saved_state.env_vars);
        self.restore_shell_env(saved_state.env_vars.clone());
        // rubash#1907: the region ran against this same PROCESS, so a
        // `umask` inside it re-masked the process itself; GNU's fork
        // discards that with the child. Re-apply the mask the save
        // point snapshotted — LAST, so the deferred procsub bodies
        // flushed above still ran in the region's context. Idempotent:
        // callers that restore the same snapshot more than once (or a
        // snapshot whose region never touched the mask) re-apply the
        // same value.
        let fork_saved_process_umask = saved_state.fork_saved_process_umask;
        self.shell_state = saved_state;
        if let Some(mask) = fork_saved_process_umask {
            crate::builtins::umask::apply_process_umask(mask);
        }
    }

    pub fn aliases_snapshot(&self) -> HashMap<String, String> {
        self.shell_state
            .aliases
            .iter()
            .map(|(name, alias)| (name.clone(), alias.value.clone()))
            .collect()
    }

    pub fn functions_snapshot(&self) -> Vec<String> {
        let mut names = self
            .shell_state
            .functions
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    pub fn env_vars_snapshot(&self) -> HashMap<String, String> {
        // Materialize marker strings so snapshots keep the historical
        // shape (child-env bootstrapping reads them); the live table
        // itself stays write-behind (#916).
        let mut snapshot = self.shell_state.env_vars.values_map().clone();
        for key in crate::shell::var_table::ATTR_KEYS {
            match self.shell_state.env_vars.rendered_attr_list(key) {
                Some(rendered) => {
                    snapshot.insert(key.to_string(), rendered);
                }
                None => {
                    snapshot.remove(key);
                }
            }
        }
        snapshot
    }

    /// G-02: the session's current working directory (physical form).
    /// cd/pushd/popd move this field only; the process cwd is never a
    /// session-semantics source.
    pub fn session_cwd(&self) -> &std::path::Path {
        &self.shell_state.cwd
    }

    pub fn env_vars(&self) -> &HashMap<String, String> {
        self.shell_state.env_vars.values_map()
    }

    /// Test/diagnostic visibility into the typed indexed-array dirty flag
    /// (`VarTable::array_cells_dirty`): true after an unsynced mirror write
    /// (arithmetic element stores) until the executor re-syncs the
    /// structured cell. Integration tests (`tests/executor_tests.rs`)
    /// cannot reach the `pub(crate)` shell state, so this accessor exposes
    /// the bit.
    pub fn array_cells_dirty(&self) -> bool {
        self.shell_state.env_vars.array_cells_dirty()
    }

    /// Re-sync one array's structured cell from its mirror and clear the
    /// dirty flag (same two steps the executor sync points in
    /// `temporary_assignments.rs` / `compound_exec.rs` perform, scoped to
    /// `name`). Test/diagnostic support for the dirty-flag unit tests.
    pub fn sync_array_cells_from_mirror(&mut self, name: &str) {
        if let Some(storage) = self.shell_state.env_vars.get(name).cloned() {
            crate::executor::arrays::sync_typed_indexed_storage(
                &mut self.shell_state.variables,
                name,
                &storage,
            );
        }
        self.shell_state.env_vars.clear_array_cells_dirty();
    }

    /// GNU execute_cmd.c:5080 — the source builtin holds sourcenest for its
    /// whole execution (sourced body plus the evalfile.c:395 RETURN trap
    /// action). The depth feeds the trap bare-exit context check
    /// (rubash#524).
    pub(crate) fn push_source_nest(&mut self) {
        self.source_nest_depth += 1;
    }

    pub(crate) fn pop_source_nest(&mut self) {
        self.source_nest_depth -= 1;
    }

    /// The full variable table (values + structured attributes) for
    /// in-crate consumers that query attributes.
    pub(crate) fn env_vars_table(&self) -> &crate::shell::var_table::VarTable {
        &self.shell_state.env_vars
    }

    pub(crate) fn positional_params(&self) -> Vec<String> {
        self.shell_state.positional_window_cloned()
    }

    pub fn set_positional_params(&mut self, positional_params: Vec<String>) {
        self.shell_state.positional_params = positional_params;
        self.shell_state.positional_offset = 0;
    }

    /// Live `$1..$#` window (offset-aware; see
    /// `ShellState::positional_window`).
    pub(crate) fn positional_window(&self) -> &[String] {
        self.shell_state.positional_window()
    }

    /// `$#` over the live window.
    pub(crate) fn positional_len(&self) -> usize {
        self.shell_state.positional_len()
    }

    pub fn inherit_process_stdin(&mut self) {
        self.shell_state
            .env_vars
            .insert(INHERIT_PROCESS_STDIN.to_string(), "1".to_string());
    }

    /// Fetch the next command-source line when fd 0 has been retargeted.
    ///
    /// GNU input.c (bash_input) reads commands through fd 0, so a permanent
    /// `exec 0<file` redirection (redir.c do_redirections under
    /// REDIR_PERSIST via builtins/exec.def) moves the script reader onto the
    /// new input — redir1.sub:4-6 relies on the next commands coming from
    /// the redirected file. Returns `None` while fd 0 still designates the
    /// process's real stdin (caller reads it byte-wise), `Some(n)` for a
    /// buffered fd-0 line, and `Some(0)` at EOF or on a closed/unreadable
    /// fd 0.
    pub fn script_fd0_line(&mut self, output: &mut String) -> Option<usize> {
        match self.fd_table.read_endpoint(0) {
            Some(FdReadEndpoint::InheritedProcessStdin) => None,
            Some(FdReadEndpoint::Text(_))
            | Some(FdReadEndpoint::ProcessSubstitution(_))
            | Some(FdReadEndpoint::File(_))
            | Some(FdReadEndpoint::CoprocStdout { .. }) => {
                let line = self
                    .fd_table
                    .take_buffered_input_line(0)
                    .unwrap_or_default();
                if line.is_empty() {
                    return Some(0);
                }
                output.push_str(&bytes_to_shell_text(&line));
                Some(line.len())
            }
            // Closed or otherwise unreadable fd 0 means end of input for the
            // command reader (`exec 0<&-` retires the stream).
            _ => Some(0),
        }
    }

    /// Single writer for `__RUBASH_CURRENT_LINE` (wt34/perf2). GNU's
    /// line_number is one process-local C int (execute_cmd.c SET_LINE_NUMBER
    /// sites) written by plain assignment; the env-map encoding paid an
    /// unconditional key+value String allocation at every loop-restore site
    /// (for-arith test/update restore twice per iteration where GNU's
    /// execute_cmd.c:3236 restore is an int store; word-list for restores
    /// once per value). Every writer now funnels through this ONE
    /// equality-gated entry point — the env entry (what builtin
    /// diagnostics and child environments consume) keeps exactly one
    /// author, and a byte-identical re-stamp skips the insert
    /// (idempotence-preserving; the perf11 stamp gate extended to the
    /// restore brackets).
    pub(crate) fn set_current_line_value(&mut self, line: usize) {
        stamp_env_usize(
            &mut self.shell_state.env_vars,
            "__RUBASH_CURRENT_LINE",
            line,
        );
    }

    /// rubash#817: push a fresh read-unit generation for a nested read —
    /// a `source`d file (builtins/source/execution.rs) or an `eval` string
    /// (trap_exec.rs execute_eval_source). GNU gives each such unit its own
    /// input stream (evalfile.c:253-257 push_stream; evalstring.c
    /// parse_and_execute's pushed string), and the executor-level alias
    /// same-read guard (arithmetic_aliases.rs alias_defined_on_current_line)
    /// compares (generation, line) so a definition recorded inside the unit
    /// can never collide with the sourcer's own later physical lines.
    /// Returns the caller's generation for exit_read_unit; the ceiling never
    /// decreases, so sibling units never share a value.
    pub(crate) fn enter_read_unit(&mut self) -> usize {
        let saved = self.shell_state.read_generation;
        self.shell_state.read_generation_ceiling =
            self.shell_state.read_generation_ceiling.wrapping_add(1);
        self.shell_state.read_generation = self.shell_state.read_generation_ceiling;
        saved
    }

    /// Restore the caller's read-unit generation saved by enter_read_unit.
    /// GNU's run_unwind_frame pops the pushed stream — restoring
    /// line_number — before source_file runs the RETURN trap
    /// (evalfile.c:395), so the trap action and every later sourcer command
    /// are read by the sourcer's own unit again.
    pub(crate) fn exit_read_unit(&mut self, saved: usize) {
        self.shell_state.read_generation = saved;
    }

    pub(in crate::executor) fn set_current_line(&mut self, cmd: &CommandNode) {
        // GNU execute_cmd.c: only some command kinds stamp `line_number`
        // from their own `->line` (cm_simple :936, cm_subshell :696,
        // cm_for :3001, cm_select :3513, cm_case :3653, cm_arith :3904,
        // cm_cond :4138, cm_arith_for :3236). while/until/if/group and the
        // connection/pipeline/&/! wrappers run under the ambient line —
        // the enclosing body's frozen line or, at top level, this
        // command's own parse-end line (see CommandNode::end_line).
        let line = if command_sets_own_line(cmd) {
            // cm_simple stamps Simple->line, which parse.y:5851 captured at
            // the END of the command's first word — a multi-line first word
            // (assignment RHS, quoted word) stamps its last line, and that
            // is the base every command substitution inside it is numbered
            // from (rubash#201: evalstring.c:345-346 keeps the enclosing
            // command's line for the string's first canonical line).
            simple_command_first_word_end_line(cmd).or(cmd.line)
        } else {
            self.ambient_line.get().or(cmd.end_line).or(cmd.line)
        };
        // The command's START line (first token) for the comsub here-document
        // paths: GNU gathers a comsub's here-document during the outer
        // script parse (parse.y parse_comsub yyparse ->
        // gather_here_documents), so those warnings carry the physical
        // `cat <<EOF` line, not the execution-time `line_number`
        // (comsub-eof0.sub: `foo=$(cat <<EOF ... EOF)` spanning lines 2-4
        // warns "line 4: ... at line 2" — physical, unaffected by
        // Simple->line).
        let start_line = if command_sets_own_line(cmd) {
            cmd.line
        } else {
            self.ambient_line.get().or(cmd.end_line).or(cmd.line)
        };
        if let Some(start_line) = start_line {
            stamp_env_usize(
                &mut self.shell_state.env_vars,
                "__RUBASH_CMD_START_LINE",
                start_line,
            );
        }
        if let Some(line) = line {
            // GNU's line_number is a C int (execute_cmd.c SET_LINE_NUMBER
            // sites) — restamping it per command costs nothing. The env
            // encoding pays a HashMap insert per stamp, and every command
            // is stamped twice (the reader loop stamps, then
            // execute_command stamps again); a body command inside a loop
            // re-stamps the SAME line thousands of times. Insert only when
            // the rendered value differs from what is already stored —
            // byte-identical inserts are idempotent, so skipping them
            // changes nothing observable.
            self.set_current_line_value(line);
            // GNU's line_number is a process-internal C int; the Windows
            // process-environment mirror only needs the OS write when the
            // stamped value differs from the last one WE wrote — a loop
            // body re-stamps the same line thousands of times. The gate
            // compares against the last OS write, not the map value:
            // ambient-line restores write the map directly, so a
            // map-equality gate left the OS env stale (alias/declare
            // diagnostics reported an earlier line).
            if command_needs_process_line_env(cmd) {
                let rendered = line.to_string();
                if self.line_env_os_value.borrow().as_deref() != Some(rendered.as_str()) {
                    set_process_env("__RUBASH_CURRENT_LINE", &rendered);
                    *self.line_env_os_value.borrow_mut() = Some(rendered);
                }
            }
        }
    }

    /// Runs `f` with GNU's ambient `line_number` pinned to `line` for its
    /// duration — the enclosing line-setting command's `->line` in force
    /// while its body executes (execute_cmd.c SET_LINE_NUMBER sites).
    /// Non-line-setting commands inside the body report diagnostics
    /// against this line rather than their own keyword line.
    pub(in crate::executor) fn with_ambient_line<T>(
        &mut self,
        line: Option<usize>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let saved = self.ambient_line.replace(line);
        let result = f(self);
        self.ambient_line.set(saved);
        result
    }

    pub(in crate::executor) fn set_current_command(&mut self, cmd: &CommandNode) {
        // GNU the_printed_command_except_trap is only refreshed when no trap
        // action is running — every print site guards with
        // `signal_in_progress (DEBUG_TRAP) == 0 && running_trap == 0`
        // (execute_cmd.c:4499-4501 and the compound heads), so commands
        // executed inside a DEBUG/ERR/RETURN/signal trap action never replace
        // the trapped command's text that BASH_COMMAND exposes
        // (variables.c:1558 get_bash_command).
        if self.debug_trap_running
            || self.error_trap_running
            || self.return_trap_running
            || self.signal_trap_running
        {
            return;
        }
        // GNU set -v (verbose): every input line is echoed to stderr as it
        // is READ (flags.c:284 echo_input_at_read = verbose_flag; the
        // reader prints shell_input_line, y.tab.c:5071). The echo happens
        // after the command that turned the flag on has run (its own line
        // was already read), and trap execution suspends it (trap.c:1285-
        // 1290 saves and clears echo_input_at_read around the action).
        // This preamble is the read/execute boundary for command lines, so
        // the verbose echo lives here (rubash#348); the rendered command
        // text is the physical line for simple and pipeline commands
        // (compound bodies re-enter through their per-command stages).
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "verbose") {
            let echo_text = bash_command_source_text(cmd);
            let _ = writeln!(std::io::stderr(), "{echo_text}");
        }
        // GNU the_printed_command is produced by print_command
        // (print_cmd.c), which covers every command form — a `[[ ]]`
        // node must record `[[ -n $unset ]]`, not a word-list dump
        // (`'[[' '-n' '$unset' ']]'`); pipelines record the full
        // `a | b` text. bash_command_source_text is that renderer;
        // bash_command_text only knows simple commands.
        // GNU `case` is the exception: execute_cmd.c:3655 prints only the
        // HEAD (`print_case_command_head` -> `case WORD in `), never the
        // clause list, so `$BASH_COMMAND` during the case word expansion is
        // the head. Rendering all N arms per execution here was also the
        // dominant per-arm cost of a literal-arm `case` loop.
        //
        // Literal fast render (same whitelist as the expansion fast
        // paths): an all-literal command's text is just its words joined
        // with single spaces — no renderer walk, no per-word String allocs.
        // Anything else falls through to the full renderer below, so the
        // stored text is byte-identical either way (the contract test pins
        // fast == slow on dispatch-verb and quoting shapes).
        let command = if let Some(case_command) = &cmd.case_command {
            crate::executor::command_text::case_command_head_source_text(case_command)
        } else if let Some(literal) = crate::executor::command_text::try_render_literal_command(cmd)
        {
            literal
        } else if let Some(joined) = crate::executor::command_text::try_render_raw_join_command(cmd)
        {
            // Raw-join twin of the literal render above (same whitelist
            // discipline): words carrying unquoted `$name` expansions
            // (`export BK_V$i=x`) still print as their raw source text —
            // the full renderer would clone each raw and re-scan it for
            // `$(` spans. Anything doubtful keeps the slow path below.
            joined
        } else {
            bash_command_source_text(cmd)
        };
        // Same equality gate as the line stamps: GNU keeps
        // the_printed_command in a C global, so recording it is a pointer
        // store; inserting a byte-identical value into both env keys for
        // every repeated command in a loop is pure churn.
        if self
            .shell_state
            .env_vars
            .get("__RUBASH_LAST_COMMAND")
            .map(String::as_str)
            != Some(command.as_str())
        {
            self.shell_state
                .env_vars
                .insert("__RUBASH_LAST_COMMAND".to_string(), command.clone());
            self.shell_state
                .env_vars
                .insert("__RUBASH_CURRENT_COMMAND".to_string(), command);
        }
    }

    pub(in crate::executor) fn set_pipestatus<I>(&mut self, statuses: I)
    where
        I: IntoIterator<Item = i32>,
    {
        // GNU variables.c:6291 set_pipestatus_array stores into the
        // PIPESTATUS ARRAY object directly and never consults
        // att_readonly: the update lands iff the cell currently holds
        // an array object. A scalar cell (`PIPESTATUS=99`) or a
        // valueless att_invisible cell (`readonly PIPESTATUS` /
        // `declare -a PIPESTATUS` before materialization — array_cell
        // is NULL and array_insert fails silently) freezes the vector;
        // an absent cell is recreated as a fresh array
        // (make_new_array_variable), which is also how an `unset
        // PIPESTATUS` is undone by the very next command (rubash#1363,
        // rubash#1364).
        match self.shell_state.pipestatus_shape {
            crate::shell::state::PipestatusShape::Absent => {
                self.shell_state.pipestatus_shape = crate::shell::state::PipestatusShape::LiveArray;
                unmark_env_name(
                    &mut self.shell_state.env_vars,
                    crate::executor::UNSET_DYNAMIC_VARS,
                    "PIPESTATUS",
                );
                // make_new_array_variable creates an EMPTY array
                // object; entries left over from the unbound cell are
                // gone with it.
                self.shell_state.pipestatus.clear();
            }
            crate::shell::state::PipestatusShape::LiveArray => {}
            crate::shell::state::PipestatusShape::Scalar
            | crate::shell::state::PipestatusShape::Valueless
            | crate::shell::state::PipestatusShape::ValuelessArray => return,
        }
        // The write itself is variables.c:6291 set_pipestatus_array
        // over the sparse ARRAY object (rubash#1364 R3): an empty
        // array takes the statuses at indices 0..n; otherwise the
        // existing elements are replaced IN INDEX ORDER (which also
        // covers the count==1 fast case) and any remaining statuses
        // are written AT THEIR INDEX via array_insert (array.c:516)
        // — an insert landing on an occupied index replaces it, so a
        // sparse cell keeps its indices and its element count can
        // stay below the status count. Only when the cell holds MORE
        // elements than statuses does GNU delete: array_flush plus a
        // dense rebuild at 0..n, indices NOT preserved.
        let statuses: Vec<i32> = statuses.into_iter().collect();
        let statuses: &[i32] = if statuses.is_empty() { &[0] } else { &statuses };
        let entries = &mut self.shell_state.pipestatus;
        let count = entries.len();
        if count == 0 || count > statuses.len() {
            entries.clear();
            entries.extend(
                statuses
                    .iter()
                    .enumerate()
                    .map(|(index, status)| (index as i64, *status)),
            );
        } else {
            for (position, entry) in entries.iter_mut().enumerate() {
                entry.1 = statuses[position];
            }
            for index in count..statuses.len() {
                pipestatus_insert_or_replace(entries, index as i64, statuses[index]);
            }
        }
    }

    /// Shape bookkeeping after a successful PIPESTATUS bind through
    /// the ordinary assignment path (variables.c bind_variable /
    /// arrayfunc.c assign_array_var_from_string). The stored result
    /// decides: an array-storage result means a compound bind (or a
    /// scalar bind onto an array cell, which GNU routes to element 0
    /// keeping the array) — the cell is a live array and the status
    /// vector syncs from the stored entries until the next command
    /// completion rewrites it. A scalar result creates a scalar cell
    /// out of an absent or valueless one (which set_pipestatus then
    /// leaves frozen, rubash#1364); on a live array the shape stays
    /// LiveArray. Either way the bind revives an `unset` dynamic cell
    /// as an ordinary variable, so the UNSET_DYNAMIC mark clears
    /// (variables.c:3839's unbind is undone by the new cell).
    pub(in crate::executor) fn note_pipestatus_assignment(&mut self) {
        let stored = self
            .shell_state
            .env_vars
            .get("PIPESTATUS")
            .cloned()
            .unwrap_or_default();
        // The shape derives from the STORED FORM plus the prior shape
        // only: the static ARRAY_VARS mark init applies to PIPESTATUS
        // names its nominal type, not this cell's shape — consulting
        // it here forced every scalar bind (`PIPESTATUS=99` as the
        // first command) into a live array GNU keeps scalar/frozen.
        if is_array_storage(&stored) {
            self.shell_state.pipestatus_shape = crate::shell::state::PipestatusShape::LiveArray;
            unmark_env_name(
                &mut self.shell_state.env_vars,
                crate::executor::UNSET_DYNAMIC_VARS,
                "PIPESTATUS",
            );
            // The stored form IS the array object here (bind_variable
            // wrote it), so sync index-for-index: a sparse bind stays
            // sparse in the status state (rubash#1364 R3) instead of
            // collapsing into a dense vector with zero-filled holes.
            self.shell_state.pipestatus = indexed_array_entries(&stored)
                .into_iter()
                .map(|(index, element)| (index as i64, element.trim().parse::<i32>().unwrap_or(0)))
                .collect();
        } else if self.shell_state.pipestatus_shape
            != crate::shell::state::PipestatusShape::LiveArray
        {
            self.shell_state.pipestatus_shape = crate::shell::state::PipestatusShape::Scalar;
            unmark_env_name(
                &mut self.shell_state.env_vars,
                crate::executor::UNSET_DYNAMIC_VARS,
                "PIPESTATUS",
            );
        }
    }

    /// PIPESTATUS cell-shape transitions for the attribute builtins
    /// (declare/typeset/local/export/readonly), applied after the
    /// builtin ran on the env table (rubash#1363/#1364). GNU creates
    /// the dynamic variable's cell at shell start with att_invisible
    /// and NO array object; naming it in an attribute builtin before
    /// it materializes leaves exactly that valueless cell behind —
    /// set_pipestatus_array's array_insert then fails silently
    /// forever (variables.c:6291 via array.c:516), which rubash models
    /// as the Valueless shape. An assignment operand instead binds a
    /// value, so it defers to note_pipestatus_assignment's post-state
    /// rule. `declare -a PIPESTATUS` on a scalar cell converts it to
    /// an array holding the scalar at [0] (variables.c
    /// convert_var_to_array), reviving the writer. Print mode creates
    /// nothing, so callers pass print_requested to stand down.
    pub(in crate::executor) fn note_pipestatus_attribute_operands(
        &mut self,
        args: &[String],
        print_requested: bool,
    ) {
        if print_requested {
            return;
        }
        // Function mode (`readonly -f`, `export -f`, `declare -f`):
        // the operands name functions, never the variable cell — GNU's
        // internal_getopt permutes options ahead of operands, so an
        // -f bundle anywhere before `--` decides the mode for all of
        // them. Stand down entirely.
        for arg in args {
            if arg == "--" {
                break;
            }
            if arg.len() > 1
                && (arg.starts_with('-') || arg.starts_with('+'))
                && arg[1..].chars().all(|c| c.is_ascii_alphabetic())
                && arg[1..].contains('f')
            {
                return;
            }
        }
        let mut array_flag = false;
        for arg in args {
            if arg.len() > 1
                && (arg.starts_with('-') || arg.starts_with('+'))
                && arg[1..].chars().all(|c| c.is_ascii_alphabetic())
            {
                if arg[1..].contains('a') {
                    array_flag = arg.starts_with('-');
                }
                continue;
            }
            if arg == "--" {
                continue;
            }
            let (raw_name, has_value) = match arg.split_once('=') {
                Some((name, _)) => (name, true),
                None => (arg.as_str(), false),
            };
            let base = raw_name.strip_suffix('+').unwrap_or(raw_name);
            let base = base.split('[').next().unwrap_or(base);
            if base != "PIPESTATUS" {
                continue;
            }
            if has_value {
                self.note_pipestatus_assignment();
                // rubash#1525: inside a function, an attribute
                // builtin's value operand binds a NEW frame cell
                // (make_local_variable) — a scalar result is a
                // scalar local shadowing any outer array whole, not
                // the permanent bind's element-0 write into the
                // array. note_pipestatus_assignment's post-state
                // rule keeps LiveArray for a scalar store, which is
                // only reachable here through such a fresh cell
                // (an existing array cell stores array form under
                // every other path), so note the scalar shape.
                if self.shell_state.function_depth > 0 {
                    if let Some(stored) = self.shell_state.env_vars.get("PIPESTATUS") {
                        if !is_array_storage(stored) {
                            self.shell_state.pipestatus_shape =
                                crate::shell::state::PipestatusShape::Scalar;
                        }
                    }
                }
                continue;
            }
            // rubash#1525: a bare operand inside a function creates
            // a valueless frame local (att_invisible) even over a
            // materialized outer array — the frame saved the name
            // (the snapshot key rode along) and no value is bound.
            // At top level the same operand is a no-op on an
            // existing cell, so the shape stands.
            if self.shell_state.function_depth > 0
                && !self.shell_state.env_vars.contains_key("PIPESTATUS")
                && self
                    .shell_state
                    .local_var_scopes
                    .last()
                    .is_some_and(|scope| {
                        scope.contains_key(crate::shell::state::PIPESTATUS_STATE_SNAPSHOT_KEY)
                    })
            {
                self.shell_state.pipestatus_shape = if array_flag {
                    crate::shell::state::PipestatusShape::ValuelessArray
                } else {
                    crate::shell::state::PipestatusShape::Valueless
                };
                unmark_env_name(
                    &mut self.shell_state.env_vars,
                    crate::executor::UNSET_DYNAMIC_VARS,
                    "PIPESTATUS",
                );
                continue;
            }
            match self.shell_state.pipestatus_shape {
                crate::shell::state::PipestatusShape::Scalar if array_flag => {
                    let scalar = self
                        .shell_state
                        .env_vars
                        .get("PIPESTATUS")
                        .cloned()
                        .unwrap_or_default();
                    self.shell_state.pipestatus =
                        vec![(0, scalar.trim().parse::<i32>().unwrap_or(0))];
                    self.shell_state.pipestatus_shape =
                        crate::shell::state::PipestatusShape::LiveArray;
                }
                crate::shell::state::PipestatusShape::Absent => {
                    // A bare operand before materialization leaves the
                    // att_invisible cell; `-a` on the operand gives that
                    // cell att_array, which `declare -p` renders even
                    // though the writer still cannot fill it.
                    self.shell_state.pipestatus_shape = if array_flag {
                        crate::shell::state::PipestatusShape::ValuelessArray
                    } else {
                        crate::shell::state::PipestatusShape::Valueless
                    };
                    unmark_env_name(
                        &mut self.shell_state.env_vars,
                        crate::executor::UNSET_DYNAMIC_VARS,
                        "PIPESTATUS",
                    );
                }
                crate::shell::state::PipestatusShape::Valueless if array_flag => {
                    self.shell_state.pipestatus_shape =
                        crate::shell::state::PipestatusShape::ValuelessArray;
                }
                _ => {}
            }
        }
    }

    /// `export -p` / `readonly -p` render from the env table alone, so
    /// a live PIPESTATUS array — its value lives in the shape/vector
    /// state, not the table — shows as a valueless `declare -x
    /// PIPESTATUS` line. GNU's print_var_list reads the same cell
    /// print_var does and emits the array form with its attributes
    /// (`declare -ax PIPESTATUS=([0]="0")`), so swap any valueless
    /// PIPESTATUS line in the buffered output for the dynamic view
    /// (rubash#1363). Only print modes emit such a line, so callers
    /// may invoke this unconditionally.
    pub(in crate::executor) fn substitute_pipestatus_print_line(&self, stdout: &mut Vec<u8>) {
        let Some(view) = self.dynamic_declaration_view("PIPESTATUS") else {
            return;
        };
        let Ok(text) = std::str::from_utf8(stdout) else {
            return;
        };
        // A live array's table line is only ever the valueless form.
        // Any other shape (scalar / valueless) renders through the
        // table with the static array mark baked in — `declare -ax
        // PIPESTATUS=([0]="99")` for a scalar cell — so every
        // PIPESTATUS line is the dynamic view's to replace.
        let live =
            self.shell_state.pipestatus_shape == crate::shell::state::PipestatusShape::LiveArray;
        let valueless_pipestatus_line = |line: &str| {
            if !line.starts_with("declare -") {
                return false;
            }
            if live {
                line.ends_with(" PIPESTATUS") && !line.contains('=')
            } else {
                line.ends_with(" PIPESTATUS") || line.contains(" PIPESTATUS=")
            }
        };
        if !text.lines().any(valueless_pipestatus_line) {
            return;
        }
        let mut replaced = text
            .lines()
            .map(|line| {
                if valueless_pipestatus_line(line) {
                    view.as_str()
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.ends_with('\n') {
            replaced.push('\n');
        }
        *stdout = replaced.into_bytes();
    }

    pub(in crate::executor) fn pipestatus_values(&self) -> Vec<String> {
        // Shape-aware view of the PIPESTATUS cell (variables.c:6291):
        // a scalar cell reads as its single value, a valueless or
        // absent cell as an empty array, a live array as the vector.
        match self.shell_state.pipestatus_shape {
            crate::shell::state::PipestatusShape::Scalar => vec![self
                .shell_state
                .env_vars
                .get("PIPESTATUS")
                .cloned()
                .unwrap_or_default()],
            crate::shell::state::PipestatusShape::Valueless
            | crate::shell::state::PipestatusShape::ValuelessArray
            | crate::shell::state::PipestatusShape::Absent => Vec::new(),
            crate::shell::state::PipestatusShape::LiveArray => self
                .shell_state
                .pipestatus
                .iter()
                .map(|(_, status)| status.to_string())
                .collect(),
        }
    }

    /// Index-preserving sibling of `pipestatus_values`: the cell's
    /// elements as (index, value) pairs in ascending index order. A
    /// scalar cell is its value at index 0; valueless/absent cells
    /// have no elements (rubash#1364 R3).
    pub(in crate::executor) fn pipestatus_entries(&self) -> Vec<(i64, String)> {
        match self.shell_state.pipestatus_shape {
            crate::shell::state::PipestatusShape::Scalar => vec![(
                0,
                self.shell_state
                    .env_vars
                    .get("PIPESTATUS")
                    .cloned()
                    .unwrap_or_default(),
            )],
            crate::shell::state::PipestatusShape::Valueless
            | crate::shell::state::PipestatusShape::ValuelessArray
            | crate::shell::state::PipestatusShape::Absent => Vec::new(),
            crate::shell::state::PipestatusShape::LiveArray => self
                .shell_state
                .pipestatus
                .iter()
                .map(|(index, status)| (*index, status.to_string()))
                .collect(),
        }
    }

    /// GNU arrayfunc.c array_reference over the PIPESTATUS cell: a
    /// non-negative subscript names an INDEX (a sparse hole reads as
    /// unset, silently); a negative subscript resolves against the
    /// MAXIMUM index (max_index + 1 + subscript) and also reads as
    /// unset when the resolution lands on a hole or below zero — the
    /// caller decides whether an unresolvable negative subscript is
    /// the diagnostic-bearing case (see pipestatus_subscript_resolves).
    /// Non-array shapes follow the VT_VARIABLE arm of
    /// array_value_internal (arrayfunc.c:1571-1586, 1627-1628): the
    /// rebase is gated on array_p(var), so a scalar cell never rebases
    /// a negative subscript — element 0 is the value, every other
    /// subscript reads as unset, and a negative subscript is the
    /// INDEX_ERROR case (rubash#1555; the dense single-value view the
    /// generic paths used before the sparse rework wrongly rebased
    /// -1 onto the scalar value).
    pub(in crate::executor) fn pipestatus_element(&self, index: i128) -> Option<String> {
        if self.shell_state.pipestatus_shape != crate::shell::state::PipestatusShape::LiveArray {
            if self.shell_state.pipestatus_shape == crate::shell::state::PipestatusShape::Scalar
                && index == 0
            {
                return self.shell_state.env_vars.get("PIPESTATUS").cloned();
            }
            return None;
        }
        let entries = &self.shell_state.pipestatus;
        let resolved: i128 = if index < 0 {
            let max_index = entries.last().map(|(index, _)| *index as i128)?;
            max_index + 1 + index
        } else {
            index
        };
        if resolved < 0 {
            return None;
        }
        entries
            .iter()
            .find(|(entry_index, _)| *entry_index as i128 == resolved)
            .map(|(_, status)| status.to_string())
    }

    /// Whether a subscript on the PIPESTATUS cell resolves at all:
    /// only a negative subscript whose max-index resolution falls
    /// below zero is unresolvable — GNU's INDEX_ERROR posture in
    /// array_value_internal (arrayfunc.c:1580-1586), which the
    /// subscript-fetch sites render as `bad array subscript`. Missing
    /// elements at a resolvable index read as unset with no
    /// diagnostic. The rebase exists only for the array shape: a
    /// scalar/valueless/absent cell is VT_VARIABLE, where ANY
    /// negative subscript is unresolvable (rubash#1555 — the old
    /// dense-length formula let -1 "resolve" onto a scalar cell).
    pub(in crate::executor) fn pipestatus_subscript_resolves(&self, index: i128) -> bool {
        if index >= 0 {
            return true;
        }
        if self.shell_state.pipestatus_shape == crate::shell::state::PipestatusShape::LiveArray {
            return self
                .shell_state
                .pipestatus
                .last()
                .is_some_and(|(max_index, _)| *max_index as i128 + 1 + index >= 0);
        }
        false
    }

    /// GNU error.c get_name_for_error's interactive branch: `base_pathname
    /// (shell_name)` — the basename of the name the shell was invoked under
    /// ("/usr/local/bin/bash" -> "bash"), never the full path. shell_name
    /// comes from argv[0] (shell.c set_shell_name) unless BASH_ARGV0
    /// replaced it (variables.c:1547 set_argv0), which is why
    /// __RUBASH_SHELL_NAME (argv[0], main.rs) is consulted before the
    /// BASH_ARGV0 fallback. rubash#162 N6: this prolog used to print the
    /// full executable path.
    pub(in crate::executor) fn interactive_shell_basename(
        env_vars: &HashMap<String, String>,
    ) -> String {
        let name = env_vars
            .get("__RUBASH_SHELL_NAME")
            .or_else(|| env_vars.get("BASH_ARGV0"))
            .map(String::as_str)
            .unwrap_or("bash");
        // base_pathname: text after the last path separator (both separators,
        // since a Windows argv[0] may use either).
        name.rsplit(['/', '\\']).next().unwrap_or(name).to_string()
    }

    pub fn diagnostic_prefix(&self) -> String {
        // GNU builtins/common.c:83-110 (builtin_error_prolog): builtin
        // errors (`cd`, `jobs`, `compgen: warning`, ...) print
        // `name: line N: cmd: ...` UNCONDITIONALLY — the gnu_errfmt gate
        // lives only in error.c error_prolog/parser_error, never here.
        // This is the builtin (ungated) shape; error_prolog-family
        // diagnostics (command not found, arithmetic, redirection,
        // completion-function lookup) use error_prefix() instead.
        // GNU error.c:75-86 (report_prolog): runtime errors (command not
        // found, file not found, etc.) use only get_name_for_error() —
        // BASH_SOURCE[0] or dollar_vars[0] — as the prolog name, with no
        // input-stream segment. The "-c:" segment is exclusive to
        // parser_error (error.c:300-316) which appends yy_input_name().
        // Interactive mode (shell reading input from a terminal) omits the
        // line segment entirely (error.c:88-120 get_name_for_error returns
        // only base_pathname(shell_name), no line number).
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_INTERACTIVE")
        {
            // Interactive mode: report only the shell name, no line segment.
            // GNU error.c:88-120 (get_name_for_error) for interactive shells
            // returns base_pathname(shell_name) with no line number.
            return format!(
                "{}: ",
                Self::interactive_shell_basename(&self.shell_state.env_vars)
            );
        }

        // Script/-c mode: line segment present
        let line = self.shell_state.env_vars.get("__RUBASH_CURRENT_LINE");
        let script = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME");
        match (script, line) {
            (Some(script), Some(line)) => {
                // Script mode: "script: line N:"
                format!("{script}: line {line}: ")
            }
            (None, Some(line)) => {
                // -c mode: "bash: line N:"
                format!("bash: line {line}: ")
            }
            _ => {
                // GNU error.c:88-120 (get_name_for_error): without a script/$0
                // context the prolog falls back to base_pathname(shell_name), i.e.
                // the canonical shell name. Rubash reports as "bash"; the upstream
                // suites normalize the baseline's invoked path to the same name.
                "bash: ".to_string()
            }
        }
    }

    /// Whether `shopt -s gnu_errfmt` is active (GNU error.c:71
    /// `gnu_error_format`, bound in builtins/shopt.def:226). It gates the
    /// ` line N` segment of the error_prolog/parser_error families
    /// (report_error, internal_error, internal_warning, parser_error) to
    /// the bare `:N` form. The builtin_error family
    /// (builtins/common.c:83-110 builtin_error_prolog) never gates.
    pub(crate) fn gnu_error_format_enabled(&self) -> bool {
        crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "gnu_errfmt")
    }

    /// Gated `head: line N: ` vs `head:N: ` shared by the
    /// error_prolog/parser_error families (GNU error.c:83
    /// `gnu_error_format ? "" : _(" line ")`).
    fn gnu_error_line(&self, head: &str, line: usize) -> String {
        if self.gnu_error_format_enabled() {
            format!("{head}:{line}: ")
        } else {
            format!("{head}: line {line}: ")
        }
    }

    /// GNU error.c:75-86 error_prolog (report_error, internal_error,
    /// internal_warning): same branches as diagnostic_prefix, but the line
    /// segment honors gnu_errfmt (`script:2: `, `bash:1: `). Callers are the
    /// error_prolog family only — command-not-found, arithmetic evalerror,
    /// redirection_error, completion-function lookup, wait's pid error.
    /// Builtin errors (builtin_error_prolog, no gate) keep
    /// diagnostic_prefix().
    pub fn error_prefix(&self) -> String {
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_INTERACTIVE")
        {
            // Interactive: no line segment in either family (error.c:80).
            return format!(
                "{}: ",
                Self::interactive_shell_basename(&self.shell_state.env_vars)
            );
        }

        let line = self.shell_state.env_vars.get("__RUBASH_CURRENT_LINE");
        let script = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME");
        match (script, line) {
            (Some(script), Some(line)) => {
                let line: usize = line.parse().unwrap_or(1);
                self.gnu_error_line(script, line)
            }
            (None, Some(line)) => {
                let line: usize = line.parse().unwrap_or(1);
                self.gnu_error_line("bash", line)
            }
            _ => "bash: ".to_string(),
        }
    }

    /// Parser/syntax-error diagnostic prefix. GNU parser_error (error.c:300-316)
    /// appends yy_input_name() — the input stream name — when it differs from
    /// get_name_for_error(). For `bash -c` the stream name is "-c", producing
    /// "bash: -c: line N:". For script files the stream name equals the script
    /// name, so no extra segment appears.
    /// rubash#1161: whether the input currently being parsed/executed is
    /// a sourced FILE (the `.` builtin's dynamic extent) rather than the
    /// invocation's own stream. `__RUBASH_IN_SOURCE` is the engine's
    /// sourcelevel marker (set by execute_source_with_args);
    /// source_nest_depth is its counter twin.
    pub(in crate::executor) fn in_source_context(&self) -> bool {
        self.source_nest_depth > 0
            || self
                .shell_state
                .env_vars
                .get("__RUBASH_IN_SOURCE")
                .map(String::as_str)
                == Some("1")
    }

    pub(crate) fn parser_diagnostic_prefix(&self) -> String {
        let is_c = self.shell_state.env_vars.contains_key("__RUBASH_IS_C");
        if let (Some(script), Some(line)) = (
            self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME"),
            self.shell_state.env_vars.get("__RUBASH_CURRENT_LINE"),
        ) {
            if self
                .shell_state
                .env_vars
                .contains_key("__RUBASH_EVAL_CONTEXT")
            {
                return format!("{script}: eval: line {line}: ");
            }
            // rubash#1161: while a FILE is being sourced, GNU's
            // yy_input_name() is that file (error.c:300-316) — the `-c`
            // stream name belongs only to the command string itself.
            // A sourced file under a `-c` invocation (every startup
            // rc/profile under `niu -i -c`, or `. file` inside `-c`)
            // therefore reports `{file}: line N:`, never
            // `{file}: -c: line N:`.
            if is_c && !self.in_source_context() {
                return format!("{script}: -c: line {line}: ");
            }
            if self.in_source_context()
                && self
                    .shell_state
                    .env_vars
                    .contains_key("__RUBASH_INTERACTIVE")
            {
                // Interactive + sourced file: shell name prolog plus the
                // file as input name (see parser_diagnostic_prefix_for_line).
                return format!(
                    "{}: {script}: line {line}: ",
                    Self::interactive_shell_basename(&self.shell_state.env_vars)
                );
            }
            return format!("{script}: line {line}: ");
        }
        if is_c {
            if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
                return format!("{script}: -c: line 1: ");
            }
            return "bash: -c: line 1: ".to_string();
        }

        "bash: ".to_string()
    }

    /// GNU error.c:300-316 parser_error gated twin of
    /// parser_diagnostic_prefix: the ` line N` segment honors gnu_errfmt
    /// (`script:2: `, `script: -c:2: `, `script: eval:2: `).
    pub(crate) fn parser_error_prefix(&self) -> String {
        let is_c = self.shell_state.env_vars.contains_key("__RUBASH_IS_C");
        if let (Some(script), Some(line)) = (
            self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME"),
            self.shell_state.env_vars.get("__RUBASH_CURRENT_LINE"),
        ) {
            let line: usize = line.parse().unwrap_or(1);
            if self
                .shell_state
                .env_vars
                .contains_key("__RUBASH_EVAL_CONTEXT")
            {
                return self.gnu_error_line(&format!("{script}: eval"), line);
            }
            // rubash#1161: sourced-file input suppresses the `-c`
            // segment, as in parser_diagnostic_prefix.
            if is_c && !self.in_source_context() {
                return self.gnu_error_line(&format!("{script}: -c"), line);
            }
            if self.in_source_context()
                && self
                    .shell_state
                    .env_vars
                    .contains_key("__RUBASH_INTERACTIVE")
            {
                return self.gnu_error_line(
                    &format!(
                        "{}: {script}",
                        Self::interactive_shell_basename(&self.shell_state.env_vars)
                    ),
                    line,
                );
            }
            return self.gnu_error_line(script, line);
        }
        if is_c {
            if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
                return self.gnu_error_line(&format!("{script}: -c"), 1);
            }
            return self.gnu_error_line("bash: -c", 1);
        }

        "bash: ".to_string()
    }

    /// GNU parser_error (error.c:300-316) with yy_input_name() ==
    /// "command substitution" (parse_and_execute's `whom`, subst.c:7413):
    /// `script: command substitution: line N: `. The comsub parse inherits
    /// the outer line_number (evalstring.c push_stream(0)), so `line` is the
    /// script line where the substitution's input ran out.
    /// GNU error.c sys_error prolog: get_name_for_error() with NO line
    /// segment — `script: ` in script mode, the shell basename when
    /// interactive, `bash: ` otherwise. redir.c:1291 add_undo_redirect's
    /// "redirection error: cannot duplicate fd" sys_error reports through
    /// this shape (verified vs WSL GNU 5.3.0: `script: redirection error:
    /// cannot duplicate fd: Bad file descriptor`).
    pub(in crate::executor) fn script_name_prefix(&self) -> String {
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_INTERACTIVE")
        {
            return format!(
                "{}: ",
                Self::interactive_shell_basename(&self.shell_state.env_vars)
            );
        }
        let name = self
            .shell_state
            .env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .cloned()
            .unwrap_or_else(|| "bash".to_string());
        format!("{name}: ")
    }

    pub(in crate::executor) fn comsub_eof_diagnostic(&self, line: usize) -> String {
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_INTERACTIVE")
        {
            // parser_error's interactive branch prints only the shell name
            // (base_pathname(shell_name), same rule as get_name_for_error).
            return format!(
                "{}: ",
                Self::interactive_shell_basename(&self.shell_state.env_vars)
            );
        }
        let name = self
            .shell_state
            .env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .cloned()
            .unwrap_or_else(|| "bash".to_string());
        format!("{name}: command substitution: line {line}: ")
    }

    /// GNU error.c:300-316 parser_error gated twin of comsub_eof_diagnostic:
    /// `script: command substitution:2: ` under gnu_errfmt.
    pub(in crate::executor) fn comsub_eof_error_diagnostic(&self, line: usize) -> String {
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_INTERACTIVE")
        {
            return format!(
                "{}: ",
                Self::interactive_shell_basename(&self.shell_state.env_vars)
            );
        }
        let name = self
            .shell_state
            .env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .cloned()
            .unwrap_or_else(|| "bash".to_string());
        self.gnu_error_line(&format!("{name}: command substitution"), line)
    }

    pub fn diagnostic_prefix_for_line(&self, line: usize) -> String {
        if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
            return format!("{script}: line {line}: ");
        }

        "bash: ".to_string()
    }

    /// error_prolog-family (report_error/internal_error/internal_warning)
    /// twin of diagnostic_prefix_for_line: `script:2: ` under gnu_errfmt.
    pub fn error_prefix_for_line(&self, line: usize) -> String {
        if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
            return self.gnu_error_line(script, line);
        }

        "bash: ".to_string()
    }

    /// Parser/syntax-error diagnostic prefix for a specific line. Like
    /// parser_diagnostic_prefix but for a caller-supplied line number.
    pub fn parser_diagnostic_prefix_for_line(&self, line: usize) -> String {
        let is_c = self.shell_state.env_vars.contains_key("__RUBASH_IS_C");
        // error.c yy_input_name: inside `eval` the input stream name is
        // "eval" and overrides the -c tag (`bash: eval: line N:`).
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_EVAL_CONTEXT")
        {
            if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
                return format!("{script}: eval: line {line}: ");
            }
            return format!("bash: eval: line {line}: ");
        }
        if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
            // rubash#1161: sourced file input suppresses the `-c`
            // segment, exactly as in parser_diagnostic_prefix.
            if is_c && !self.in_source_context() {
                return format!("{script}: -c: line {line}: ");
            }
            if self.in_source_context()
                && self
                    .shell_state
                    .env_vars
                    .contains_key("__RUBASH_INTERACTIVE")
            {
                // GNU error.c parser_error in an interactive shell: the
                // prolog name is the shell name (get_name_for_error) and
                // the sourced file rides as yy_input_name —
                // `bash: FILE: line N:` for startup rc/profile errors.
                return format!(
                    "{}: {script}: line {line}: ",
                    Self::interactive_shell_basename(&self.shell_state.env_vars)
                );
            }
            return format!("{script}: line {line}: ");
        }
        if is_c {
            return format!("bash: -c: line {line}: ");
        }

        "bash: ".to_string()
    }

    /// GNU error.c:300-316 parser_error gated twin of
    /// parser_diagnostic_prefix_for_line (`script:2: `,
    /// `script: eval:2: `, `bash: -c:2: ` under gnu_errfmt).
    pub fn parser_error_prefix_for_line(&self, line: usize) -> String {
        let is_c = self.shell_state.env_vars.contains_key("__RUBASH_IS_C");
        // GNU trap.c:496: a trap action parses under the input name
        // "trap" ("interrupt trap" for SIGINT), which parser_error
        // (error.c:300-316) appends like the eval name below
        // (rubash#1351).
        if let Some(context) = self.shell_state.env_vars.get("__RUBASH_TRAP_CONTEXT") {
            if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
                return self.gnu_error_line(&format!("{script}: {context}"), line);
            }
            return self.gnu_error_line(&format!("bash: {context}"), line);
        }
        if self
            .shell_state
            .env_vars
            .contains_key("__RUBASH_EVAL_CONTEXT")
        {
            if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
                return self.gnu_error_line(&format!("{script}: eval"), line);
            }
            return self.gnu_error_line("bash: eval", line);
        }
        if let Some(script) = self.shell_state.env_vars.get("__RUBASH_SCRIPT_NAME") {
            // rubash#1161: same yy_input_name rule as
            // parser_diagnostic_prefix_for_line — a sourced file's input
            // name replaces the `-c` stream tag, and in an interactive
            // shell the file rides behind the shell-name prolog.
            if is_c && !self.in_source_context() {
                return self.gnu_error_line(&format!("{script}: -c"), line);
            }
            if self.in_source_context()
                && self
                    .shell_state
                    .env_vars
                    .contains_key("__RUBASH_INTERACTIVE")
            {
                return self.gnu_error_line(
                    &format!(
                        "{}: {script}",
                        Self::interactive_shell_basename(&self.shell_state.env_vars)
                    ),
                    line,
                );
            }
            return self.gnu_error_line(script, line);
        }
        if is_c {
            return self.gnu_error_line("bash: -c", line);
        }

        "bash: ".to_string()
    }

    pub(in crate::executor) fn report_unterminated_heredoc(&self, cmd: &CommandNode) {
        // GNU parse.y:3130/make_cmd.c:627: `lineno` is the parser's
        // line_number when gather_here_documents ran make_here_document —
        // the physical line where the command's logical line ended (advanced
        // by any earlier heredoc bodies of the same command), not the `<<`
        // line. internal_warning's prefix uses line_number after the body
        // scan. Prefer the lexer's physical end line: with an unquoted
        // delimiter GNU joins `\<newline>` pairs while collecting
        // (parse.y read_a_line via read_secondary_line), so the stored body
        // can hold fewer newlines than the physical lines consumed
        // (rubash#1028); only fall back to gather + stored lines.
        let gather_line = cmd.heredoc_gather_line.or(cmd.line).unwrap_or(1);
        let body_lines = cmd
            .heredoc
            .as_deref()
            .map(unterminated_heredoc_body_line_count)
            .unwrap_or(0);
        let warning_line = cmd
            .heredoc_end_line
            .unwrap_or_else(|| gather_line + body_lines);
        let delimiter = cmd.heredoc_delimiter.as_deref().unwrap_or("");
        self.write_diagnostic_fd2(
            format!(
                "{}warning: here-document at line {gather_line} delimited by end-of-file (wanted `{delimiter}')\n",
                self.error_prefix_for_line(warning_line)
            )
            .as_bytes(),
        );
    }

    /// Report a here-document whose delimiter was found but not on a line by
    /// itself (e.g. `EOF)` inside a command substitution).  GNU make_cmd.c:606-627
    /// sets `full_line = 0` via the PST_EOFTOKEN backwards-compat path and
    /// issues the same warning as a truly unterminated heredoc.  The prefix
    /// line is the delimiter line (start_line + body_lines + 1), and "at line
    /// N" is the heredoc start line.
    pub(in crate::executor) fn report_warned_heredoc(&self, cmd: &CommandNode) {
        let gather_line = cmd.heredoc_gather_line.or(cmd.line).unwrap_or(1);
        let body_lines = cmd
            .heredoc
            .as_deref()
            .map(unterminated_heredoc_body_line_count)
            .unwrap_or(0);
        // The physical end line is the delimiter line itself (see
        // report_unterminated_heredoc for the `\<newline>`-join rationale,
        // rubash#1028).
        let delimiter_line = cmd
            .heredoc_end_line
            .unwrap_or_else(|| gather_line + body_lines + 1);
        let delimiter = cmd.heredoc_delimiter.as_deref().unwrap_or("");
        self.write_diagnostic_fd2(
            format!(
                "{}warning: here-document at line {gather_line} delimited by end-of-file (wanted `{delimiter}')\n",
                self.error_prefix_for_line(delimiter_line)
            )
            .as_bytes(),
        );
    }

    /// Report an unclosed `$(` that ran to end of input. GNU make_cmd.c
    /// gather_here_documents scans heredoc headers inside the comsub text
    /// first (each unterminated body warns at the last input line, with
    /// "at line N" = the header's script line); parse.y:6883-6896 then issues
    /// `unexpected EOF while looking for matching ')'` at the parser's
    /// line_number — the line after the last physical input line.
    /// `raw` is the token text starting at `start_line` that contains the
    /// unclosed `$(` and everything it swallowed.
    pub(in crate::executor) fn report_unclosed_comsub_eof(&self, raw: &str, start_line: usize) {
        // Locate the last `$(` whose tail never closes — its text is the
        // comsub body GNU's nested parse consumed.
        let bytes = raw.as_bytes();
        let mut innermost: Option<(usize, &str)> = None;
        for (idx, window) in bytes.windows(2).enumerate() {
            if window == b"$("
                && (idx == 0 || bytes[idx - 1] != b'\\')
                && crate::lexer::has_unclosed_command_substitution(&raw[idx..])
            {
                innermost = Some((idx, &raw[idx + 2..]));
            }
        }
        let last_line = start_line + raw.matches('\n').count();
        if let Some((idx, content)) = innermost {
            let content_line = start_line + raw[..idx].matches('\n').count();
            // Replay the gather: track pending headers and consumed
            // delimiter lines; whatever is still open at EOF warns.
            let mut pending: Vec<(String, usize)> = Vec::new();
            let mut line_no = content_line;
            for line in content.split('\n') {
                if let Some((delim, _)) = pending.first().cloned() {
                    if line == delim {
                        pending.remove(0);
                    }
                } else {
                    let (headers, _) = crate::lexer::scan_line_for_comsub_heredoc_headers(line);
                    for header in headers {
                        pending.push((header.delimiter, line_no));
                    }
                }
                line_no += 1;
            }
            for (delim, at_line) in pending {
                self.write_diagnostic_fd2(
                    format!(
                        "{}warning: here-document at line {at_line} delimited by end-of-file (wanted `{delim}')\n",
                        self.error_prefix_for_line(last_line)
                    )
                    .as_bytes(),
                );
            }
        }
        self.write_diagnostic_fd2(
            format!(
                "{}unexpected EOF while looking for matching `)'\n",
                self.parser_error_prefix_for_line(last_line + 1)
            )
            .as_bytes(),
        );
    }

    pub(in crate::executor) fn report_unterminated_subshell_heredoc(&self, cmd: &CommandNode) {
        self.report_unterminated_heredoc(cmd);
        let start_line = cmd.line.unwrap_or(1);
        let body_lines = cmd
            .heredoc
            .as_deref()
            .map(unterminated_heredoc_body_line_count)
            .unwrap_or(0);
        let warning_line = cmd
            .heredoc_end_line
            .unwrap_or_else(|| cmd.heredoc_gather_line.or(cmd.line).unwrap_or(1) + body_lines);
        let syntax_line = warning_line + 1;
        self.write_diagnostic_fd2(
            format!(
                "{}syntax error: unexpected end of file from `(' command on line {start_line}\n",
                self.parser_error_prefix_for_line(syntax_line)
            )
            .as_bytes(),
        );
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        // startup21: the process-exit executor's process dies with this
        // drop, so its marker removal is skipped entirely: deliver()'s
        // dead-pid path (kill.rs deliver_rubash_signal -> process_exists
        // -> unregister_signal_mailbox) self-heals stale markers on the
        // next kill attempt at a recycled pid, and the registration
        // thread opportunistically prunes dead-pid markers. The two
        // remove_file calls (~0.4ms of failing/succeeding deletes under
        // real-time scanning) only buy directory hygiene, which those
        // paths already provide. Embedded/child executors keep the full
        // unregister.
        if self.owns_signal_mailbox && !self.is_process_exit_executor {
            // Wait for the async registration thread first — unregistering
            // before the marker write completes would leak the marker
            // (write lands after the remove).
            crate::builtins::kill::join_signal_mailbox_registration();
            crate::builtins::kill::unregister_signal_mailbox(std::process::id());
        }

        // startup21: the process-env restore models an in-process child
        // leaving its parent's env untouched (rubash#182). The process-exit
        // executor's process dies with this drop — nothing can observe its
        // environment afterward — so the restore is skipped there.
        if self.is_process_exit_executor {
            return;
        }
        // G-03: the restore is scoped to this executor lineage's own
        // process-env mirror writes (env_write_journal). The pre-G-03
        // restore rolled EVERY name back to the birth snapshot — including
        // names a still-live sibling session wrote after this executor was
        // born, so the first session to Drop silently un-wrote the other
        // sessions' variables. A journaled key returns to its snapshot
        // state only when the current process value still equals the
        // journal's final written state (no other writer has touched it
        // since); otherwise the live owner's value stands. A fork-time
        // snapshot restore is not an assignment mirror, so the #916
        // special-name gate in set_process_env must not apply here
        // (rubash#917). TMPDIR keeps its pre-#916 exclusion (never
        // mirrored, so leave it alone rather than restoring or removing).
        let journal = self.env_write_journal.borrow();
        // G-01: the whole compare-and-restore sequence is one
        // process-state critical section — a sibling's funnel write
        // must not interleave between the compare and the restore.
        let _process_state = crate::executor::local_helpers::acquire_process_state();
        for (name, final_state) in journal.iter() {
            if name == "TMPDIR" {
                continue;
            }
            let current = env::var_os(name).and_then(|value| value.into_string().ok());
            if current.as_deref() != final_state.as_deref() {
                continue;
            }
            match self.process_env_snapshot.get(name) {
                Some(value) => {
                    if is_valid_process_env(name, value) {
                        env::set_var(name, value);
                    } else {
                        env::remove_var(name);
                    }
                }
                None => env::remove_var(name),
            }
        }
    }
}
