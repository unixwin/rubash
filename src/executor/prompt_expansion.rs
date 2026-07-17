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
                PatternRemoval::ShortestPrefix => {
                    remove_matching_prefix(&value, &pattern, MatchLength::Shortest)
                }
                PatternRemoval::LongestPrefix => {
                    remove_matching_prefix(&value, &pattern, MatchLength::Longest)
                }
                PatternRemoval::ShortestSuffix => {
                    remove_matching_suffix(&value, &pattern, MatchLength::Shortest)
                }
                PatternRemoval::LongestSuffix => {
                    remove_matching_suffix(&value, &pattern, MatchLength::Longest)
                }
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
                .env_vars
                .get(array_name)
                .map(|value| array_values(value))
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
                Some('!') => output.push_str(&self.prompt_history_number().to_string()),
                Some('#') => output.push_str(&self.prompt_command_number().to_string()),
                Some('$') => output.push(prompt_dollar(&self.env_vars)),
                Some('\\') => output.push('\\'),
                Some('[') | Some(']') => {}
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
        output
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
        0
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
        if crate::builtins::set::shell_option_enabled(&self.env_vars, "hashall") {
            flags.push('h');
        }
        for (flag, option) in [
            ('a', "allexport"),
            ('b', "notify"),
            ('B', "braceexpand"),
            ('E', "errtrace"),
            ('H', "histexpand"),
            ('k', "keyword"),
            ('P', "physical"),
            ('p', "privileged"),
            ('t', "onecmd"),
            ('T', "functrace"),
            ('v', "verbose"),
        ] {
            if crate::builtins::set::shell_option_enabled(&self.env_vars, option) {
                flags.push(flag);
            }
        }
        if self.errexit_enabled() {
            flags.push('e');
        }
        if self.xtrace_enabled() {
            flags.push('x');
        }
        if crate::builtins::set::shell_option_enabled(&self.env_vars, "nounset") {
            flags.push('u');
        }
        if self.noexec_enabled() {
            flags.push('n');
        }
        if crate::builtins::set::shell_option_enabled(&self.env_vars, "noclobber") {
            flags.push('C');
        }
        if crate::builtins::set::shell_option_enabled(&self.env_vars, "noglob") {
            flags.push('f');
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
