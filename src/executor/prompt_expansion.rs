use super::*;

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

        let target_expr = self.env_vars.get(ref_name)?;
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
            return self
                .parameter_array_storage(array_name)
                .map(|value| array_values(&value))
                .unwrap_or_default();
        }

        if let Some(value) = self.array_element_parameter_value(target_expr) {
            return vec![value];
        }

        self.env_vars
            .get(target_expr)
            .map(|value| {
                if is_array_storage(value) || is_marked_array_var(&self.env_vars, target_expr) {
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
                Some('e') | Some('E') => output.push('\x1b'),
                Some('n') => output.push('\n'),
                Some('r') => output.push('\r'),
                Some('t') => output.push_str(&self.prompt_time("%H:%M:%S")),
                Some('T') => output.push_str(&self.prompt_time("%I:%M:%S")),
                Some('@') => output.push_str(&self.prompt_time("%I:%M %p")),
                Some('A') => output.push_str(&self.prompt_time("%H:%M")),
                Some('d') => output.push_str(&self.prompt_time("%a %b %d")),
                Some('D') => output.push_str(&self.decode_prompt_date_escape(&mut chars)),
                Some('u') => output.push_str(&prompt_username(&self.env_vars)),
                Some('h') => output.push_str(&prompt_hostname(&self.env_vars, false)),
                Some('H') => output.push_str(&prompt_hostname(&self.env_vars, true)),
                Some('w') => output.push_str(&self.prompt_working_directory(false)),
                Some('W') => output.push_str(&self.prompt_working_directory(true)),
                Some('l') => output.push_str(&prompt_terminal_basename(&self.env_vars)),
                Some('s') => output.push_str("bash"),
                Some('v') => output.push_str(&prompt_short_version(&self.env_vars)),
                Some('V') => output.push_str(&prompt_release_version(&self.env_vars)),
                Some('j') => output.push_str(&self.prompt_job_count().to_string()),
                // GNU parse.y: the `\!' escape always renders the prompt
                // history number (no POSIX gate here; the POSIX rule governs
                // only a bare `!').
                Some('!') => output.push_str(&self.prompt_history_number().to_string()),
                Some('#') => output.push_str(&self.prompt_command_number().to_string()),
                Some('$') => output.push(prompt_dollar(&self.env_vars)),
                Some('\\') => output.push('\\'),
                Some('[') | Some(']') => {
                    // GNU parse.y:6609-6622: \[ and \] emit the readline
                    // prompt-ignore markers (RL_PROMPT_START/END_IGNORE) only
                    // when the line editor is active; with no_line_editing (a
                    // script without `set -o emacs`/`vi`) they are dropped
                    // entirely. A marker equal to CTLESC (0x01) carries a
                    // CTLESC prefix exactly as GNU does, so the pair dequote
                    // downstream yields the single marker byte.
                    if crate::builtins::set::shell_option_enabled(&self.env_vars, "emacs")
                        || crate::builtins::set::shell_option_enabled(&self.env_vars, "vi")
                    {
                        // The marker bytes pass through the word carrier as
                        //-is when followed by non-marker bytes, matching how
                        // the octal \001 escape already renders through @P.
                        output.push(if ch == '[' { '\x01' } else { '\x02' });
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
        let pwd = self.env_vars.get("PWD").cloned().unwrap_or_default();
        let rendered = if let Some(home) = self.env_vars.get("HOME") {
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
        crate::builtins::printf::time::format_current_time(format, &self.env_vars)
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
        tilde_expand::expand_assignment_value(value, &self.env_vars)
    }

    pub(in crate::executor) fn home_value(&self) -> String {
        tilde_expand::home_value(&self.env_vars)
    }

    pub(in crate::executor) fn shell_option_flags(&self) -> String {
        let mut flags = String::new();
        // Order matches GNU Bash flags.c shell_flags[] so `$-` output agrees
        // (`set -e -h -B` prints `ehB`, set-e3.sub `echo $-`).
        for (flag, option) in [
            ('a', "allexport"),
            ('b', "notify"),
            ('e', "errexit"),
            ('f', "noglob"),
            ('h', "hashall"),
            ('k', "keyword"),
            ('n', "noexec"),
            ('p', "privileged"),
            ('r', "restricted"),
            ('t', "onecmd"),
            ('u', "nounset"),
            ('v', "verbose"),
            ('x', "xtrace"),
            ('B', "braceexpand"),
            ('C', "noclobber"),
            ('E', "errtrace"),
            ('H', "histexpand"),
            ('P', "physical"),
            ('T', "functrace"),
        ] {
            if crate::builtins::set::shell_option_enabled(&self.env_vars, option) {
                flags.push(flag);
            }
        }
        // Bash exposes `c` in `$-` while executing a command string passed
        // with `-c`; script-file and stdin execution do not set it.
        if self.env_vars.contains_key("BASH_EXECUTION_STRING") {
            flags.push('c');
        }
        flags
    }

    pub(in crate::executor) fn noexec_enabled(&self) -> bool {
        crate::builtins::set::shell_option_enabled(&self.env_vars, "noexec")
    }

    pub(in crate::executor) fn errexit_enabled(&self) -> bool {
        self.env_vars.get("__RUBASH_ERREXIT").map(String::as_str) == Some("1")
            || crate::builtins::set::shell_option_enabled(&self.env_vars, "errexit")
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
        self.env_vars.get("__RUBASH_XTRACE").map(String::as_str) == Some("1")
            || crate::builtins::set::shell_option_enabled(&self.env_vars, "xtrace")
    }

    /// Expanded PS4 prefix for `set -x` tracing (Bash prints the expanded
    /// value of PS4 before each traced command). Defaults to `+ ` like Bash.
    pub(in crate::executor) fn xtrace_prefix(&self) -> String {
        let ps4 = self
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
        for (name, value) in &cmd.assignments {
            // `COMPOUND_ASSIGNMENT_MARKER` is an internal carrier for compound
            // array assignments and must never leak into user-visible xtrace.
            let expanded = self.expand_assignment_value(value);
            let expanded = expanded
                .strip_prefix(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(&expanded);
            parts.push(format!("{name}={expanded}"));
        }
        parts.extend(cmd.words.iter().cloned());
        parts.join(" ")
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
