use super::*;

/// Apply `ansic_quote` to a word when it contains non-printable characters
/// or raw-byte markers (GNU print_cmd.c xtrace_print_word calls ansic_quote
/// on words needing quoting). Words that are already printable are returned
/// unchanged.
pub(in crate::executor) fn xtrace_quote_word(word: &str) -> String {
    // GNU print_cmd.c:549 xtrace_print_word_list: an empty word prints as
    // `''`, then ansic_shouldquote → `$'...'`, then sh_contains_shell_metas
    // → `'...'`, otherwise the bare word.
    let visible = crate::locale::decode_to_visible_text(word);
    if visible.is_empty() {
        "''".to_string()
    } else if super::execution_misc::word_needs_ansic_quote(word) {
        super::execution_misc::ansic_quote_with_markers(word)
    } else if xtrace_contains_shell_metas(&visible) {
        xtrace_single_quote(&visible)
    } else {
        // xtrace prints user-visible text: decode transport carriers
        // (E400 literal-char escape, C0 carriers) or marker escapes leak.
        visible
    }
}

/// lib/sh/shquote.c sh_contains_shell_metas (376-407): IFS whitespace,
/// quoting chars, shell metacharacters, globbing chars, expansion chars.
fn xtrace_contains_shell_metas(value: &str) -> bool {
    let chars: Vec<char> = value.chars().collect();
    for (index, ch) in chars.iter().enumerate() {
        match ch {
            ' ' | '\t' | '\n' | '\'' | '"' | '\\' | '|' | '&' | ';' | '(' | ')' | '<' | '>'
            | '!' | '{' | '}' | '*' | '[' | '?' | ']' | '^' | '$' | '`' => return true,
            '~' => {
                if index == 0 || chars[index - 1] == '=' || chars[index - 1] == ':' {
                    return true;
                }
            }
            '#' => {
                if index == 0 {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// GNU read_token_word token text for one compound-assignment element:
/// source-verbatim except `$'...'`, which the lexer replaces with
/// `sh_single_quote(ansiexpand(body))` (parse.y:5563-5574).
///
/// Iterated over `chars()`, not `as_bytes()`: widening each payload byte to
/// a char Latin-1-encodes multi-byte elements, so `set -x` traced
/// `arr=(中文)` as `arr=(ä¸­æ–‡)` and the `$'...'` body handed
/// decode_ansi_c_quoted mojibake. The recognized metacharacters (`$`, `'`,
/// `\`) are ASCII, so char iteration keeps the same scan semantics.
fn compound_element_xtrace_text(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && chars.get(i + 1) == Some(&'\'') {
            let mut j = i + 2;
            let mut body = String::new();
            let mut closed = false;
            while j < chars.len() {
                match chars[j] {
                    '\\' if j + 1 < chars.len() => {
                        body.push('\\');
                        body.push(chars[j + 1]);
                        j += 2;
                    }
                    '\'' => {
                        closed = true;
                        j += 1;
                        break;
                    }
                    c => {
                        body.push(c);
                        j += 1;
                    }
                }
            }
            if closed {
                out.push('\'');
                for c in crate::lexer::ansi::decode_ansi_c_quoted(&body).chars() {
                    if c == '\'' {
                        out.push_str("'\\''");
                    } else {
                        out.push(c);
                    }
                }
                out.push('\'');
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// lib/sh/shquote.c sh_single_quote: wrap a string in single quotes,
/// `'` → `'\''`.
fn xtrace_single_quote(word: &str) -> String {
    let mut ret = String::with_capacity(word.len() + 2);
    ret.push('\'');
    for c in word.chars() {
        if c == '\'' {
            ret.push_str("'\\''");
        } else {
            ret.push(c);
        }
    }
    ret.push('\'');
    ret
}

impl Executor {
    pub(in crate::executor) fn indirect_pattern_removal(&self, name: &str) -> Option<String> {
        let (ref_expr, pattern, operation) = parse_indirect_pattern_removal(name)?;
        let ref_name = ref_expr
            .strip_suffix("[@]")
            .or_else(|| ref_expr.strip_suffix("[*]"))
            .unwrap_or(ref_expr);
        if !is_shell_name(ref_name) {
            return None;
        }

        let target_expr = self.shell_state.env_vars.get(ref_name)?;
        let values = self.indirect_target_values(target_expr);
        if values.is_empty() {
            return Some(String::new());
        }

        let pattern = self.expand_embedded_parameters(pattern);
        let values = values
            .into_iter()
            .map(|value| match operation {
                PatternRemoval::ShortestPrefix => remove_matching_prefix(
                    &value,
                    &pattern,
                    MatchLength::Shortest,
                    self.extglob_enabled(),
                ),
                PatternRemoval::LongestPrefix => remove_matching_prefix(
                    &value,
                    &pattern,
                    MatchLength::Longest,
                    self.extglob_enabled(),
                ),
                PatternRemoval::ShortestSuffix => remove_matching_suffix(
                    &value,
                    &pattern,
                    MatchLength::Shortest,
                    self.extglob_enabled(),
                ),
                PatternRemoval::LongestSuffix => remove_matching_suffix(
                    &value,
                    &pattern,
                    MatchLength::Longest,
                    self.extglob_enabled(),
                ),
            })
            .collect::<Vec<_>>();
        Some(self.join_expanded_array_values(values, target_expr))
    }

    pub(in crate::executor) fn indirect_target_values(&self, target_expr: &str) -> Vec<String> {
        if let Some(array_name) = target_expr
            .strip_suffix("[@]")
            .or_else(|| target_expr.strip_suffix("[*]"))
        {
            let resolved = self
                .resolved_variable_name(array_name)
                .unwrap_or_else(|| array_name.to_string());
            return self
                .parameter_array_storage(array_name)
                .map(|value| {
                    if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, &resolved) {
                        // GNU assoc_reference iterates the hash table in
                        // bucket order (hashlib.c), not storage order —
                        // `${!aref}` with aref=`assoc[@]` must match
                        // `${assoc[@]}` (quotearray4.sub).
                        assoc_hash_ordered_values(
                            &value,
                            assoc_nbuckets(&self.shell_state.env_vars, &resolved),
                        )
                    } else {
                        array_values(&value)
                    }
                })
                .unwrap_or_default();
        }

        if let Some(value) = self.array_element_parameter_value(target_expr) {
            return vec![value];
        }

        self.shell_state
            .env_vars
            .get(target_expr)
            .map(|value| {
                if is_array_storage(value)
                    || is_marked_array_var(&self.shell_state.env_vars, target_expr)
                {
                    array_value_at(value, 0).into_iter().collect()
                } else {
                    vec![value.clone()]
                }
            })
            .unwrap_or_default()
    }

    pub(in crate::executor) fn decode_prompt_string(&self, value: &str) -> String {
        let mut output = String::new();
        let mut chars = value.chars().peekable();
        while let Some(ch) = chars.next() {
            // GNU parse.y:6269-6288: in POSIX mode a bare `!' expands to the
            // prompt history number and `!!' is a literal `!'. This runs
            // before the backslash switch, so it covers unescaped `!' too.
            if self.posix_mode_enabled() && ch == '!' {
                if chars.peek() == Some(&'!') {
                    chars.next();
                    output.push('!');
                } else {
                    output.push_str(&self.prompt_history_number().to_string());
                }
                continue;
            }
            if ch != '\\' {
                output.push(ch);
                continue;
            }

            match chars.next() {
                Some('a') => output.push('\x07'),
                Some('e') | Some('E') => output.push(crate::executor::markers::QUOTED_WORD_PREFIX),
                Some('n') => output.push('\n'),
                Some('r') => output.push('\r'),
                Some('t') => output.push_str(&self.prompt_time("%H:%M:%S")),
                Some('T') => output.push_str(&self.prompt_time("%I:%M:%S")),
                Some('@') => output.push_str(&self.prompt_time("%I:%M %p")),
                Some('A') => output.push_str(&self.prompt_time("%H:%M")),
                Some('d') => output.push_str(&self.prompt_time("%a %b %d")),
                Some('D') => output.push_str(&self.decode_prompt_date_escape(&mut chars)),
                Some('u') => output.push_str(&prompt_username(&self.shell_state.env_vars)),
                Some('h') => output.push_str(&prompt_hostname(&self.shell_state.env_vars, false)),
                Some('H') => output.push_str(&prompt_hostname(&self.shell_state.env_vars, true)),
                Some('w') => output.push_str(&self.prompt_working_directory(false)),
                Some('W') => output.push_str(&self.prompt_working_directory(true)),
                Some('l') => output.push_str(&prompt_terminal_basename(&self.shell_state.env_vars)),
                Some('s') => output.push_str("bash"),
                Some('v') => output.push_str(&prompt_short_version(&self.shell_state.env_vars)),
                Some('V') => output.push_str(&prompt_release_version(&self.shell_state.env_vars)),
                Some('j') => output.push_str(&self.prompt_job_count().to_string()),
                // GNU parse.y: the `\!' escape always renders the prompt
                // history number (no POSIX gate here; the POSIX rule governs
                // only a bare `!').
                Some('!') => output.push_str(&self.prompt_history_number().to_string()),
                Some('#') => output.push_str(&self.prompt_command_number().to_string()),
                Some('$') => output.push(prompt_dollar(&self.shell_state.env_vars)),
                Some('\\') => output.push('\\'),
                Some(marker @ ('[' | ']')) => {
                    // GNU parse.y:6609-6622: \[ and \] emit the readline
                    // prompt-ignore markers (RL_PROMPT_START/END_IGNORE) only
                    // when the line editor is active; with no_line_editing (a
                    // script without `set -o emacs`/`vi`) they are dropped
                    // entirely.
                    if crate::builtins::set::shell_option_enabled(
                        &self.shell_state.env_vars,
                        "emacs",
                    ) || crate::builtins::set::shell_option_enabled(
                        &self.shell_state.env_vars,
                        "vi",
                    ) {
                        output.push(if marker == '[' {
                            crate::executor::markers::PROMPT_IGNORE_START
                        } else {
                            crate::executor::markers::PROMPT_IGNORE_END
                        });
                    }
                }
                Some(octal @ '0'..='7') => {
                    push_ansi_c_codepoint(&mut output, read_prompt_octal(octal, &mut chars))
                }
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            }
        }
        // Command substitutions preserve control bytes as owner-tagged
        // private-use code points while they pass through shell variables.
        // Prompt expansion is the byte-oriented output boundary, so restore
        // those markers before reedline or another terminal renderer sees the
        // prompt text.
        let decoded =
            crate::executor::substitution_metadata::decode_raw_byte_markers(output.as_bytes());
        String::from_utf8_lossy(&decoded).into_owned()
    }

    pub(in crate::executor) fn expand_prompt_parameters(&self, word: &str) -> String {
        let mut output = String::new();
        let mut chars = word.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch != '$' {
                output.push(ch);
                continue;
            }

            match chars.peek().copied() {
                Some('{') => {
                    chars.next();
                    let mut name = String::new();
                    for name_ch in chars.by_ref() {
                        if name_ch == '}' {
                            break;
                        }
                        name.push(name_ch);
                    }
                    output.push_str(&self.parameter_error_value(&name).unwrap_or_default());
                }
                Some('(') => {
                    chars.next();
                    let mut depth = 1;
                    let mut source = String::new();
                    while let Some(source_ch) = chars.next() {
                        match source_ch {
                            '(' => {
                                depth += 1;
                                source.push(source_ch);
                            }
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                source.push(source_ch);
                            }
                            _ => source.push(source_ch),
                        }
                    }
                    output.push_str(&self.expand_command_substitution(&source));
                }
                Some(first) if is_shell_name_start(first) => {
                    let mut name = String::new();
                    while let Some(name_ch) = chars.peek().copied() {
                        if !is_shell_name_char(name_ch) {
                            break;
                        }
                        chars.next();
                        name.push(name_ch);
                    }
                    output.push_str(&self.parameter_error_value(&name).unwrap_or_default());
                }
                Some(other) => {
                    chars.next();
                    output.push('$');
                    output.push(other);
                }
                None => output.push('$'),
            }
        }

        output
    }

    pub(in crate::executor) fn prompt_working_directory(&self, basename_only: bool) -> String {
        let pwd = self
            .shell_state
            .env_vars
            .get("PWD")
            .cloned()
            .unwrap_or_default();
        let rendered = if let Some(home) = self.shell_state.env_vars.get("HOME") {
            if pwd == *home {
                "~".to_string()
            } else if let Some(rest) = pwd.strip_prefix(&format!("{home}/")) {
                format!("~/{rest}")
            } else {
                pwd
            }
        } else {
            pwd
        };

        if basename_only {
            // GNU polite_directory_format (parse.y): ROOT_PATH renders as
            // itself; the basename of `/` is not the empty string.
            if rendered == "/" {
                return rendered;
            }
            rendered
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(&rendered)
                .to_string()
        } else {
            rendered
        }
    }

    pub(in crate::executor) fn prompt_job_count(&self) -> usize {
        0
    }

    pub(in crate::executor) fn prompt_history_number(&self) -> usize {
        // GNU parse.y prompt_history_number (parse.y:6209-6216): with no
        // history entries history_number() returns history_base (1) and the
        // value is returned unchanged, which is what a script-driven
        // ${var@P} \! sees (new-exp.tests new-exp10: 1).
        1
    }

    pub(in crate::executor) fn prompt_command_number(&self) -> usize {
        0
    }

    pub(in crate::executor) fn prompt_time(&self, format: &str) -> String {
        crate::builtins::printf::time::format_current_time(format, &self.shell_state.env_vars)
    }

    fn decode_prompt_date_escape<I>(&self, chars: &mut std::iter::Peekable<I>) -> String
    where
        I: Iterator<Item = char>,
    {
        if chars.peek() != Some(&'{') {
            return "\\D".to_string();
        }
        chars.next();

        let mut format = String::new();
        for ch in chars.by_ref() {
            if ch == '}' {
                return self.prompt_time(&format);
            }
            format.push(ch);
        }

        format!("\\D{{{format}")
    }

    pub(in crate::executor) fn expand_assignment_tilde(&self, value: &str) -> String {
        if value.contains('=') {
            return value.to_string();
        }
        tilde_expand::expand_assignment_value(value, &self.shell_state.env_vars)
    }

    pub(in crate::executor) fn home_value(&self) -> String {
        tilde_expand::home_value(&self.shell_state.env_vars)
    }

    pub(in crate::executor) fn shell_option_flags(&self) -> String {
        let mut flags = String::new();
        // Order matches GNU Bash flags.c:165 shell_flags[] so `$-` output
        // agrees (`set -e -h -B` prints `ehB`, set-e3.sub `echo $-`). `i` is
        // the one table letter with no `set -o` name — flags.c:174 maps it
        // to `forced_interactive`, which shell.c:672 turns on for every
        // interactive shell (`-i` forced, or tty-detected at
        // shell.c:540-547) — so it renders from the interactive marker in
        // its table position between `h` and `k`.
        for (flag, option) in [
            ('a', Some("allexport")),
            ('b', Some("notify")),
            ('e', Some("errexit")),
            ('f', Some("noglob")),
            ('h', Some("hashall")),
            ('i', None),
            ('k', Some("keyword")),
            ('n', Some("noexec")),
            ('p', Some("privileged")),
            ('r', Some("restricted")),
            ('t', Some("onecmd")),
            ('u', Some("nounset")),
            ('v', Some("verbose")),
            ('x', Some("xtrace")),
            ('B', Some("braceexpand")),
            ('C', Some("noclobber")),
            ('E', Some("errtrace")),
            ('H', Some("histexpand")),
            ('P', Some("physical")),
            ('T', Some("functrace")),
        ] {
            let on = match option {
                Some(option) => {
                    crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, option)
                }
                None => self
                    .shell_state
                    .env_vars
                    .contains_key("__RUBASH_INTERACTIVE"),
            };
            if on {
                flags.push(flag);
            }
        }
        // flags.c:304-307 which_set_flags appends the invocation-only letters
        // after the option table, in this order.
        // Bash exposes `c` in `$-` while executing a command string passed
        // with `-c`; script-file and stdin execution do not set it.
        if self
            .shell_state
            .env_vars
            .contains_key("BASH_EXECUTION_STRING")
        {
            flags.push('c');
        }
        // `s` shows while the shell reads commands from stdin: explicit `-s`
        // (shell.c:928), a non-interactive shell with no script operand
        // (shell.c:780-786), or an interactive shell with no operand
        // (shell.c:787-790) — GNU's `read_from_stdin` (shell.c:301). The
        // marker is set by the stdin drivers (run_stdin_script,
        // run_interactive_stdin, run_repl) and, like the C global, is not
        // exported to child processes.
        if self
            .shell_state
            .env_vars
            .get(crate::script_driver::READ_STDIN_MARKER)
            .map(String::as_str)
            == Some("1")
        {
            flags.push('s');
        }
        flags
    }

    pub(in crate::executor) fn noexec_enabled(&self) -> bool {
        crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "noexec")
    }

    pub(in crate::executor) fn errexit_enabled(&self) -> bool {
        self.shell_state
            .env_vars
            .get("__RUBASH_ERREXIT")
            .map(String::as_str)
            == Some("1")
            || crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "errexit")
    }

    pub(in crate::executor) fn errexit_is_active(&self) -> bool {
        self.suppress_errexit == 0
    }

    pub(crate) fn with_errexit_suppressed<T>(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<T, ExecuteError>,
    ) -> Result<T, ExecuteError> {
        self.suppress_errexit += 1;
        let result = body(self);
        self.suppress_errexit -= 1;
        result
    }

    pub(in crate::executor) fn xtrace_enabled(&self) -> bool {
        self.shell_state
            .env_vars
            .get("__RUBASH_XTRACE")
            .map(String::as_str)
            == Some("1")
            || crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "xtrace")
    }

    /// Expanded PS4 prefix for `set -x` tracing (Bash prints the expanded
    /// value of PS4 before each traced command). Defaults to `+ ` like Bash.
    /// Write xtrace output to the current trace target. GNU print_cmd.c
    /// xtrace_fd defaults to stderr; `BASH_XTRACEFD=N` retargets it
    /// (variables.c sv_xtracefd) and xtrace_fdchk silently resets to stderr
    /// when that descriptor is closed.
    pub(in crate::executor) fn xtrace_write(&mut self, bytes: &[u8]) {
        // Realign when BASH_XTRACEFD changed through a path that skipped the
        // assignment hook (read/local/unset/direct store).
        let current = self
            .shell_state
            .env_vars
            .get("BASH_XTRACEFD")
            .cloned()
            .unwrap_or_default();
        if current != *self.shell_state.xtrace_fd_source.borrow() {
            self.resolve_xtracefd(false);
        }
        let fd = self.shell_state.xtrace_fd.get();
        if fd >= 0 {
            if !self.fd_table.has_entry(fd as u32) || self.fd_table.is_closed(fd as u32) {
                // xtrace_fdchk (print_cmd.c:436): a closed trace fd silently
                // falls back to stderr until the next BASH_XTRACEFD assign.
                self.shell_state.xtrace_fd.set(-1);
            } else {
                let _ = self.write_fd_endpoint(fd as u32, bytes);
                return;
            }
        }
        let _ = self.write_default_stderr(bytes);
    }

    /// GNU variables.c:6424 sv_xtracefd: resolve BASH_XTRACEFD after an
    /// assignment. Empty/unset resets to stderr; a non-numeric or closed fd
    /// reports `invalid value for trace file descriptor` and keeps the
    /// previous target. `report` controls the diagnostic (skipped for lazy
    /// realignment, which is not an assignment event).
    pub(in crate::executor) fn apply_xtracefd_assignment(&mut self) {
        self.resolve_xtracefd(true);
    }

    fn resolve_xtracefd(&mut self, report: bool) {
        let value = self
            .shell_state
            .env_vars
            .get("BASH_XTRACEFD")
            .cloned()
            .unwrap_or_default();
        if !value.is_empty() {
            // strtol: leading whitespace ok, the entire rest must parse.
            let parsed = value.trim_start().parse::<i64>().ok();
            let mut fd: Option<i32> = None;
            if let Some(n) = parsed.filter(|n| (0..=i32::MAX as i64).contains(n)) {
                // sh_validfd (shvalidfd.c): the descriptor only has to be
                // open — fdopen("w") failure is a separate diagnostic GNU
                // only reaches for exotic states, so open-for-anything
                // suffices here.
                if self.fd_table.has_entry(n as u32) && !self.fd_table.is_closed(n as u32) {
                    fd = Some(n as i32);
                }
            }
            if let Some(n) = fd {
                self.shell_state.xtrace_fd.set(n);
            } else {
                // internal_error: the diagnostic fires but the previous
                // trace target stays bound (sv_xtracefd returns early).
                if report {
                    let line = format!(
                        "{}BASH_XTRACEFD: {value}: invalid value for trace file descriptor\n",
                        self.diagnostic_prefix()
                    );
                    let _ = self.write_default_stderr(line.as_bytes());
                }
            }
        } else {
            self.shell_state.xtrace_fd.set(-1);
        }
        *self.shell_state.xtrace_fd_source.borrow_mut() = value;
    }

    pub(in crate::executor) fn xtrace_prefix(&self) -> String {
        let ps4 = self
            .shell_state
            .env_vars
            .get("PS4")
            .cloned()
            .unwrap_or_else(|| "+ ".to_string());
        self.expand_embedded_parameters(&ps4)
    }

    /// Rendered command text for xtrace: prefix assignments followed by words.
    /// Bash traces both `VAR=x cmd args` and bare `VAR=x` assignments.
    pub(in crate::executor) fn xtrace_command_text(&mut self, cmd: &CommandNode) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.extend(self.xtrace_assignment_text(cmd));
        parts.extend(cmd.words.iter().map(|w| {
            // W_ARRAYREF (in-band ARRAYREF_FLAG) is node metadata in GNU —
            // invisible in xtrace output.
            xtrace_quote_word(crate::builtins::arrayref::take_arrayref_flag(w).1)
        }));
        parts.join(" ")
    }

    /// GNU execute_simple_command traces the assignment prefix on its own
    /// line, separate from the command words: `foo=one echo hi` traces as
    /// `+ foo=one` then `+ echo hi` (assignments are traced as they are
    /// performed, before the command words are dispatched).
    pub(in crate::executor) fn xtrace_assignment_text(&mut self, cmd: &CommandNode) -> Vec<String> {
        let mut parts: Vec<String> = Vec::new();
        for (name, value) in &cmd.assignments {
            // GNU subst.c:3580 xtrace_print_assignment: a compound assignment
            // (assign_list) prints `name=(raw-inner)` — the space-joined
            // element WORD texts, unexpanded (arrayfunc.c
            // extract_array_assignment_list). rubash's marker value already
            // carries the parenthesized interior; rebuild GNU's join by
            // re-splitting on unquoted whitespace and applying the lexer's
            // `$'...'` → `'<decoded>'` token rewrite (parse.y:5563-5574).
            if let Some(raw) =
                value.strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
            {
                let interior = raw
                    .strip_prefix('(')
                    .and_then(|inner| inner.strip_suffix(')'))
                    .unwrap_or(raw);
                let joined = crate::parser::assignment::split_compound_assignment_words(interior)
                    .iter()
                    .map(|element| {
                        crate::locale::decode_to_visible_text(&compound_element_xtrace_text(
                            element,
                        ))
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                parts.push(format!("{name}=({joined})"));
                continue;
            }
            // Scalar: the EXPANDED value is quoted like an xtrace word,
            // except an empty value prints as nothing (GNU print_cmd.c:522).
            let expanded = self.expand_assignment_value(name, value);
            let expanded = expanded
                .strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(&expanded);
            let visible = crate::locale::decode_to_visible_text(expanded);
            let rendered = if visible.is_empty() {
                visible
            } else if super::execution_misc::word_needs_ansic_quote(expanded) {
                super::execution_misc::ansic_quote_with_markers(expanded)
            } else if xtrace_contains_shell_metas(&visible) {
                xtrace_single_quote(&visible)
            } else {
                visible
            };
            parts.push(format!("{name}={rendered}"));
        }
        parts
    }

    /// GNU print_cmd.c:986 `xtrace_print_arith_cmd`: print `(( expr ))` when
    /// `set -x` is on.  Called before evaluating each arithmetic expression
    /// (both `(( ... ))` commands and `for ((init; test; update))` loops).
    /// The expression is the already-expanded string that will be evaluated.
    /// GNU's `expand_arith_string` strips leading whitespace; trailing
    /// whitespace is preserved (e.g. `i++  ` from `for ((...; ...; i++ ))`).
    pub(in crate::executor) fn xtrace_print_arith_cmd(&mut self, expression: &str) {
        if self.xtrace_enabled() {
            let prefix = self.xtrace_prefix();
            self.xtrace_write(format!("{prefix}(( {} ))\n", expression.trim_start()).as_bytes());
        }
    }

    /// Same trace for plain `(( expr ))` commands (GNU execute_cmd.c:3940):
    /// GNU prints the between-parens text untouched on both sides —
    /// `(( n ))` traces as `+ ((  n  ))` — unlike the arith-for sections,
    /// whose parsed word lists drop the leading blanks (xtrace above).
    pub(in crate::executor) fn xtrace_print_arith_cmd_raw(&mut self, expression: &str) {
        if self.xtrace_enabled() {
            let prefix = self.xtrace_prefix();
            self.xtrace_write(format!("{prefix}(( {} ))\n", expression).as_bytes());
        }
    }
}

fn prompt_release_version(env_vars: &HashMap<String, String>) -> String {
    let version = env_vars
        .get("BASH_VERSION")
        .cloned()
        .unwrap_or_else(bash_version_value);
    version
        .split_once('(')
        .map(|(release, _)| release.to_string())
        .unwrap_or(version)
}

fn prompt_short_version(env_vars: &HashMap<String, String>) -> String {
    let release = prompt_release_version(env_vars);
    let mut parts = release.split('.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) => format!("{major}.{minor}"),
        _ => release,
    }
}

fn prompt_terminal_basename(env_vars: &HashMap<String, String>) -> String {
    env_vars
        .get("TTY")
        .or_else(|| env_vars.get("SSH_TTY"))
        .map(|tty| {
            tty.trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(tty)
                .to_string()
        })
        .unwrap_or_default()
}

fn prompt_dollar(env_vars: &HashMap<String, String>) -> char {
    if env_vars.get("EUID").map(String::as_str) == Some("0") {
        '#'
    } else {
        '$'
    }
}

fn read_prompt_octal<I>(first: char, chars: &mut std::iter::Peekable<I>) -> Option<u32>
where
    I: Iterator<Item = char>,
{
    let mut value = first.to_string();
    while value.len() < 3 {
        let Some(next) = chars.peek().copied() else {
            break;
        };
        if next.to_digit(8).is_none() {
            break;
        }
        value.push(next);
        chars.next();
    }
    u32::from_str_radix(&value, 8).ok()
}
