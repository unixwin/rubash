use super::*;

pub(in crate::executor) fn is_arithmetic_command_words(words: &[String]) -> bool {
    matches!(words, [open, _, close] if open == "((" && close == "))")
}

pub(in crate::executor) fn echo_args_without_background_marker(args: &[String]) -> Vec<String> {
    // TODO(parse.y/jobs.c): `&` is a command terminator that launches the
    // preceding command asynchronously. Until the parser represents it that
    // way, keep source6.sub's `echo ... > fifo &` from writing a literal ampersand.
    let mut args = args.to_vec();
    if args.last().map(String::as_str) == Some("&") {
        args.pop();
    }
    args
}

pub(in crate::executor) fn is_null_device(path: &str) -> bool {
    matches!(path, "/dev/null" | "NUL")
}

pub(in crate::executor) fn is_closed_redirect_target(path: &str) -> bool {
    path == "&-"
}

pub(in crate::executor) fn redirect_target_fd(target: &str) -> Option<u32> {
    let fd = target.strip_prefix('&')?;
    (!fd.is_empty() && fd.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| fd.parse::<u32>().ok())
        .flatten()
}

pub(in crate::executor) fn stdio_output_target(fd: u32) -> Option<&'static str> {
    match fd {
        1 => Some(FD_STDOUT_TARGET),
        2 => Some(FD_STDERR_TARGET),
        _ => None,
    }
}

pub(in crate::executor) fn command_has_unterminated_heredoc(cmd: &CommandNode) -> bool {
    cmd.heredoc
        .as_deref()
        .is_some_and(|body| strip_quoted_heredoc_marker(body).starts_with('\x1f'))
}

pub(in crate::executor) fn strip_unterminated_heredoc_marker(body: &str) -> &str {
    let Some(stripped) = body.strip_prefix('\x1f') else {
        return body;
    };
    stripped
}

pub(in crate::executor) fn strip_quoted_heredoc_marker(body: &str) -> &str {
    body.strip_prefix('\x1e').unwrap_or(body)
}

pub(in crate::executor) fn unterminated_heredoc_body_line_count(body: &str) -> usize {
    let body = strip_unterminated_heredoc_marker(strip_quoted_heredoc_marker(body));
    body.lines().count()
}

pub(in crate::executor) fn copy_command_substitution_heredoc(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    source: &mut String,
) {
    source.push('<');
    source.push('<');
    chars.next();

    let strip_tabs = if chars.peek().copied() == Some('-') {
        source.push('-');
        chars.next();
        true
    } else {
        false
    };

    while chars.peek().is_some_and(|ch| matches!(ch, ' ' | '\t')) {
        let ch = chars.next().unwrap();
        source.push(ch);
    }

    let mut raw_delimiter = String::new();
    while chars
        .peek()
        .is_some_and(|ch| !ch.is_whitespace() && !matches!(ch, ';' | '|' | '&' | ')'))
    {
        let ch = chars.next().unwrap();
        raw_delimiter.push(ch);
        source.push(ch);
    }
    let mut delimiter = raw_delimiter.replace(['\'', '"', '\\'], "");
    if strip_tabs {
        delimiter = delimiter.trim_start_matches('\t').to_string();
    }
    if delimiter.is_empty() {
        return;
    }

    while let Some(ch) = chars.next() {
        source.push(ch);
        if ch == '\n' {
            break;
        }
    }

    loop {
        let mut line = String::new();
        while let Some(ch) = chars.peek().copied() {
            let comparable = if strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line.as_str()
            };
            if comparable == delimiter && ch == ')' {
                source.push('\x1c');
                return;
            }
            if ch == '\n' {
                break;
            }
            chars.next();
            line.push(ch);
            source.push(ch);
        }

        let comparable = if strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line.as_str()
        };
        if comparable == delimiter {
            if chars.peek().copied() == Some('\n') {
                source.push('\n');
                chars.next();
            }
            return;
        }

        match chars.next() {
            Some('\n') => source.push('\n'),
            Some(ch) => source.push(ch),
            None => return,
        }
    }
}

pub(in crate::executor) fn contains_windows_forbidden_posix_filename_char(path: &str) -> bool {
    path.chars()
        .any(|ch| matches!(ch, '*' | '?' | '<' | '>' | '|'))
}

pub(in crate::executor) fn word_has_unquoted_command_substitution(word: &str) -> bool {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let chars = word.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if !single && !double && ch == '`' {
            return true;
        }
        if !single && !double && ch == '$' && chars.get(index + 1) == Some(&'(') {
            return true;
        }
        index += 1;
    }
    false
}

pub(in crate::executor) fn for_word_has_unquoted_expansion(word: &str) -> bool {
    if word.starts_with('\x1b') || word.starts_with('\x1d') {
        return false;
    }
    word.starts_with('$') || word_has_unquoted_command_substitution(word)
}

pub(in crate::executor) fn bash_aliases_assignment_name(word: &str) -> Option<String> {
    // TODO(variables.c/alias.c): BASH_ALIASES is a dynamic associative array
    // backed by the alias table. This narrow path reports invalid alias names
    // for upstream alias.tests.
    let rest = word.strip_prefix("BASH_ALIASES[")?;
    let (name, _) = rest.split_once("]=")?;
    Some(name.trim_matches('\'').to_string())
}

pub(in crate::executor) fn valid_alias_assignment_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(|ch| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '/' | '$' | '`' | '"' | '\'' | '\\' | '(' | ')' | '<' | '>' | '&' | '|'
                )
        })
}

pub(in crate::executor) fn shell_display_path(path: &str) -> String {
    if cfg!(windows) && path.len() >= 3 && path.as_bytes()[1] == b':' && path.as_bytes()[2] == b'/'
    {
        let drive = path.as_bytes()[0] as char;
        return format!("/{}{}", drive.to_ascii_lowercase(), &path[2..]);
    }
    path.to_string()
}

pub(in crate::executor) fn current_epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub(in crate::executor) fn current_epoch_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_micros() as i64)
        .unwrap_or(0)
}

pub(in crate::executor) fn eval_source_for_reparse(source: &str) -> String {
    source
        .replace('\x1e', "")
        .replace('\x1d', "")
        .replace('\x1f', "$")
        .replace('\x1a', "`")
        .replace('\x17', "'")
}

pub(in crate::executor) fn next_random_from_state(state: &Cell<u32>) -> u32 {
    let next = state.get().wrapping_mul(1_103_515_245).wrapping_add(12_345);
    state.set(next);
    (next / 65_536) % 32_768
}

pub(in crate::executor) fn next_srandom_from_state(state: &Cell<u32>) -> u32 {
    let high = next_random_from_state(state);
    let low = next_random_from_state(state);
    (high << 17) ^ (low << 2) ^ (current_epoch_micros() as u32)
}

pub(in crate::executor) fn strip_shebang(source: &str) -> &str {
    source
        .strip_prefix("#!")
        .and_then(|rest| rest.split_once('\n').map(|(_, body)| body))
        .unwrap_or(source)
}

pub(in crate::executor) fn command_substitution_word_split(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(in crate::executor) fn protect_command_substitution_output(value: &str) -> String {
    value.replace('`', "\x1a").replace('$', "\x1f")
}

pub(in crate::executor) fn unescape_storage_command_substitution_source(source: &str) -> String {
    let mut output = String::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek().copied() {
                Some('"') | Some('\\') => {
                    output.push(chars.next().unwrap());
                }
                _ => output.push(ch),
            }
        } else {
            output.push(ch);
        }
    }
    output
}
