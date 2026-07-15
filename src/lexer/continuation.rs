use super::heredoc_scan::skip_heredoc_in_chars;

pub(super) fn ends_with_unquoted_backslash(input: &str) -> bool {
    let mut single = false;
    let mut escaped = false;
    for ch in input.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !single => escaped = true,
            '\'' if !escaped => single = !single,
            _ => {}
        }
    }

    let trailing_backslashes = input.chars().rev().take_while(|ch| *ch == '\\').count();
    !single && trailing_backslashes % 2 == 1
}

pub(super) fn has_unclosed_quotes(input: &str) -> bool {
    // TODO(parse.y): Bash reads parser input with full quoting state,
    // continuations, command substitutions, arithmetic contexts, and here-doc
    // deferral. This tracks only enough single/double quote state to keep a
    // multi-line alias definition as one parser unit.
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut comment_start = true;
    let mut in_comment = false;
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                comment_start = true;
            }
            index += 1;
            continue;
        }

        if escaped {
            escaped = false;
            comment_start = false;
            index += 1;
            continue;
        }

        if ch == '\n' && !single && !double && !ansi_single {
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == '#' && !single && !double && !ansi_single && comment_start {
            in_comment = true;
            index += 1;
            continue;
        }

        if ch.is_whitespace() && !single && !double && !ansi_single {
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == '\\' && (!single || ansi_single) {
            escaped = true;
            comment_start = false;
            index += 1;
            continue;
        }

        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            comment_start = false;
            index += 2;
            continue;
        }

        match ch {
            '\'' if ansi_single => {
                ansi_single = false;
                comment_start = false;
            }
            '\'' if !double && !ansi_single => {
                single = !single;
                comment_start = false;
            }
            '"' if !single && !ansi_single => {
                double = !double;
                comment_start = false;
            }
            _ => {
                if !single && !double && !ansi_single {
                    comment_start = false;
                }
            }
        }
        index += 1;
    }

    single || double || ansi_single
}

pub(super) fn has_unclosed_command_substitution(input: &str) -> bool {
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut backtick = false;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut comment_start = true;
    let mut in_comment = false;

    while index < chars.len() {
        let ch = chars[index];
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                comment_start = true;
            }
            index += 1;
            continue;
        }
        if escaped {
            escaped = false;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\n' && !single && !double && !ansi_single && !backtick && depth == 0 {
            comment_start = true;
            index += 1;
            continue;
        }
        if ch == '#'
            && !single
            && !double
            && !ansi_single
            && !backtick
            && depth == 0
            && comment_start
        {
            in_comment = true;
            index += 1;
            continue;
        }
        if ch.is_whitespace() && !single && !double && !ansi_single && !backtick && depth == 0 {
            comment_start = true;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            comment_start = false;
            index += 2;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            comment_start = false;
            index += 1;
            continue;
        }
        if single {
            index += 1;
            continue;
        }
        if ch == '`' && depth == 0 {
            backtick = !backtick;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '$' && chars.get(index + 1) == Some(&'(') {
            depth += 1;
            comment_start = false;
            index += 2;
            continue;
        }
        if depth > 0
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }
        if depth > 0 && ch == '<' && chars.get(index + 1) == Some(&'<') {
            index = skip_heredoc_in_chars(&chars, index);
            continue;
        }
        if backtick
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }
        if backtick && ch == '<' && chars.get(index + 1) == Some(&'<') {
            index = skip_heredoc_in_chars(&chars, index);
            continue;
        }
        if depth > 0 && !ansi_single && ch == '(' {
            depth += 1;
        } else if depth > 0 && !ansi_single && ch == ')' {
            depth -= 1;
        }
        if !single && !double && !ansi_single && !backtick && depth == 0 {
            comment_start = false;
        }
        index += 1;
    }

    depth > 0 || backtick || ansi_single
}
