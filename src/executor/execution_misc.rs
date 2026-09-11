use super::*;

pub(in crate::executor) fn is_arithmetic_command_words(words: &[String]) -> bool {
    matches!(words, [open, _, close] if open == "((" && close == "))")
}

pub(in crate::executor) fn echo_args_without_background_marker(args: &[String]) -> Vec<String> {
    // TODO(parse.y/jobs.c): `&` is a command terminator that launches the
    // preceding command asynchronously. The parser now consumes a source-level
    // `&` (token_actions sets CommandNode::background), so args ending in `&`
    // are field-split expansion data (e.g. echo ${s//?/\\& }) and must be
    // printed. Keep the hook for legacy word shapes that still carry `&`.
    args.to_vec()
}

pub(in crate::executor) fn is_null_device(path: &str) -> bool {
    crate::executor::path::is_shell_null_device(path)
}

pub(in crate::executor) fn is_closed_redirect_target(path: &str) -> bool {
    path == "&-"
}

pub(in crate::executor) fn redirect_target_fd(target: &str) -> Option<u32> {
    redirect_target_fd_and_move(target).and_then(|(fd, move_fd)| (!move_fd).then_some(fd))
}

pub(in crate::executor) fn redirect_target_fd_and_move(target: &str) -> Option<(u32, bool)> {
    let target = target.trim_start_matches(['\x1b', '\x1d']);
    let fd = target.strip_prefix('&')?;
    let fd = fd.trim_matches(|ch| ch == '"' || ch == '\x1d');
    let (fd, move_fd) = fd
        .strip_suffix('-')
        .map(|fd| (fd, true))
        .unwrap_or((fd, false));
    (!fd.is_empty() && fd.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| fd.parse::<u32>().ok().map(|fd| (fd, move_fd)))
        .flatten()
}

pub(in crate::executor) fn redirect_target_is_ambiguous(raw: &str, expanded: &str) -> bool {
    if !expanded.chars().any(char::is_whitespace) {
        return false;
    }

    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let mut has_unquoted_expansion = false;

    for ch in raw.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && !single_quoted {
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !double_quoted => single_quoted = !single_quoted,
            '"' if !single_quoted => double_quoted = !double_quoted,
            '$' if !single_quoted && !double_quoted => {
                has_unquoted_expansion = true;
            }
            _ => {}
        }
    }

    has_unquoted_expansion
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
    body.strip_prefix(crate::lexer::QUOTED_HEREDOC_MARKER)
        .unwrap_or(body)
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
        if !single && !double && ch == '$' && chars.get(index + 1) == Some(&'{') {
            // Bash 5.3 (parser.h FUNSUB_CHAR): a whitespace-led `${ command; }`
            // is a nofork command substitution. Its unquoted output word-splits
            // and vanishes when empty, exactly like $() output
            // (comsub2.tests: `echo ${ printf '%s\n' aa bb; }` -> one line).
            // The `${| command; }` funsub form keeps field integrity and must
            // not trigger splitting here.
            if chars
                .get(index + 2)
                .is_some_and(|next| next.is_whitespace())
            {
                return true;
            }
        }
        index += 1;
    }
    false
}

pub(in crate::executor) fn for_word_has_unquoted_expansion(word: &str, raw: Option<&str>) -> bool {
    if word.starts_with('\x1b') || word.starts_with('\x1d') {
        return false;
    }
    let source = raw.unwrap_or(word);
    word_has_unquoted_parameter_expansion(source) || word_has_unquoted_command_substitution(source)
}

fn word_has_unquoted_parameter_expansion(word: &str) -> bool {
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
        if !single && !double && ch == '$' {
            match chars.get(index + 1).copied() {
                Some('{') => return true,
                Some(next) if is_shell_name_start(next) => return true,
                Some(next)
                    if next.is_ascii_digit()
                        || matches!(next, '@' | '*' | '#' | '?' | '$' | '!' | '-') =>
                {
                    return true;
                }
                _ => {}
            }
        }
        index += 1;
    }
    false
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
    if cfg!(windows) {
        let path = path.strip_prefix("//?/").unwrap_or(path);
        let path = crate::executor::path::shell_path_display_from_windows(path);
        // GNU process_substitute (subst.c) emits forward-slash paths
        // (/dev/fd/N, or sh_mktmpname /-separated names) so the substituted
        // word survives eval/source re-parsing: a literal backslash would be
        // consumed as a shell escape. Normalize Windows backslashes to slashes
        // before the drive-letter check so bytes[2]=='/' fires too.
        let path = path.replace('\\', "/");
        return windows_native_to_slash_drive_display(&path);
    }
    path.to_string()
}

fn windows_native_to_slash_drive_display(path: &str) -> String {
    if path.len() >= 3 && path.as_bytes()[1] == b':' && path.as_bytes()[2] == b'/' {
        let drive = (path.as_bytes()[0] as char).to_ascii_lowercase();
        return format!("/{drive}/{}", &path[3..]);
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
    let source = source
        .replace(crate::lexer::QUOTED_HEREDOC_MARKER, "")
        .replace(crate::executor::types::COMPOUND_ASSIGNMENT_MARKER, "")
        .replace('\x1c', "")
        .replace('\x1f', "$")
        .replace('\x17', "'")
        .replace("\u{E002}", "'")
        .replace("\u{E003}", "\"");
    protect_unmatched_double_quoted_backticks(&source)
}

fn protect_unmatched_double_quoted_backticks(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut chars = source.char_indices().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    while let Some((_index, ch)) = chars.next() {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            output.push(ch);
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                output.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                output.push(ch);
            }
            '`' if in_double && !in_single => output.push('\x1a'),
            _ => output.push(ch),
        }
    }
    output
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

const COMMAND_SUBSTITUTION_PAYLOAD_PREFIX: &str = "__RUBASH_CSB1_";

pub(in crate::executor) fn contains_command_substitution_payload(value: &str) -> bool {
    value.contains(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX)
}

pub(in crate::executor) fn command_substitution_value_needs_payload_protection(
    source: &str,
    value: &str,
) -> bool {
    source.contains('$')
        && !source.contains('`')
        && !value.contains(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX)
        && value.chars().any(|ch| ('\x10'..='\x1f').contains(&ch))
}

pub(in crate::executor) fn protect_command_substitution_output(value: &str) -> String {
    let escaped_value = value.replace(
        COMMAND_SUBSTITUTION_PAYLOAD_PREFIX,
        &format!("{COMMAND_SUBSTITUTION_PAYLOAD_PREFIX}{COMMAND_SUBSTITUTION_PAYLOAD_PREFIX}"),
    );
    let mut output = String::with_capacity(escaped_value.len());
    for ch in escaped_value.chars() {
        match ch {
            '\x10'..='\x1f' => output.push_str(&format!(
                "{COMMAND_SUBSTITUTION_PAYLOAD_PREFIX}{:02x};",
                ch as u32
            )),
            '`' => output.push('\x1a'),
            '$' => output.push('\x1f'),
            '\\' => output.push('\x15'),
            _ => output.push(ch),
        }
    }
    output
}

pub(in crate::executor) fn restore_command_substitution_output(value: &str) -> String {
    value
        .replace('\x1a', "`")
        .replace('\x1f', "$")
        .replace('\x15', "\\")
        .replace('\x14', "\\")
}

pub(in crate::executor) fn decode_command_substitution_payload(value: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    while let Some(index) = rest.find(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX) {
        output.push_str(&rest[..index]);
        rest = &rest[index + COMMAND_SUBSTITUTION_PAYLOAD_PREFIX.len()..];
        if let Some(escaped) = rest.strip_prefix(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX) {
            output.push_str(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX);
            rest = escaped;
        } else if rest.len() >= 3 && rest.as_bytes()[2] == b';' {
            if let Ok(byte) = u8::from_str_radix(&rest[..2], 16) {
                output.push(char::from(byte));
                rest = &rest[3..];
            } else {
                output.push_str(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX);
            }
        } else {
            output.push_str(COMMAND_SUBSTITUTION_PAYLOAD_PREFIX);
        }
    }
    output.push_str(rest);
    output
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

#[cfg(test)]
mod command_substitution_payload_tests {
    use super::decode_command_substitution_payload;

    #[test]
    fn decodes_c0_payload_without_utf8_loss() {
        assert_eq!(
            decode_command_substitution_payload("a__RUBASH_CSB1_15;b"),
            "a\x15b"
        );
    }

    #[test]
    fn preserves_escaped_payload_prefix() {
        assert_eq!(
            decode_command_substitution_payload("__RUBASH_CSB1___RUBASH_CSB1_"),
            "__RUBASH_CSB1_"
        );
    }

    #[test]
    fn leaves_malformed_payload_literal() {
        assert_eq!(
            decode_command_substitution_payload("x__RUBASH_CSB1_no_semicolon"),
            "x__RUBASH_CSB1_no_semicolon"
        );
    }
}
