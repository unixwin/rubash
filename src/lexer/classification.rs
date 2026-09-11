pub(super) fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "while"
            | "do"
            | "done"
            | "until"
            | "for"
            | "case"
            | "esac"
            | "in"
            | "function"
            | "select"
            | "time"
            | "coproc"
    )
}

pub(super) fn is_assignment(word: &str) -> bool {
    let Some(pos) = word.find('=') else {
        return false;
    };
    let var_name = word[..pos].strip_suffix('+').unwrap_or(&word[..pos]);
    !var_name.is_empty()
        && var_name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && var_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}


pub(super) fn is_brace_expansion(word: &str) -> bool {
    word.starts_with('{')
        && word.ends_with('}')
        && word.len() >= 3
        && !word.chars().any(char::is_whitespace)
        && (word[1..word.len() - 1].contains("..") || word.contains(','))
}

pub(super) fn is_word_delimiter(ch: char) -> bool {
    " \t\n|&;<>(){}".contains(ch)
}

pub(super) fn assignment_value_is_quoted(raw: &str) -> bool {
    let Some((_, value)) = raw.split_once('=') else {
        return false;
    };

    // Quotes inside a `${...}` body belong to the expansion itself (GNU keeps
    // them for the expansion stage), not to the assignment's quoting state.
    let mut in_backtick = false;
    let mut escaped = false;
    let mut expansion_depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' && !in_single {
            escaped = true;
            continue;
        }

        if ch == '`' && !in_single {
            in_backtick = !in_backtick;
            continue;
        }

        if expansion_depth > 0 {
            match ch {
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '$' if !in_single && !in_double && chars.peek() == Some(&'{') => {
                    chars.next();
                    expansion_depth += 1;
                }
                '}' if !in_single && !in_double => {
                    expansion_depth -= 1;
                    if expansion_depth == 0 {
                        in_single = false;
                        in_double = false;
                    }
                }
                _ => {}
            }
            continue;
        }

        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next();
            expansion_depth = 1;
            in_single = false;
            in_double = false;
            continue;
        }

        if !in_backtick && matches!(ch, '"' | '\'') {
            return true;
        }
    }

    false
}

/// True when every character of an assignment word's right-hand side lies
/// inside single quotes (`x='...'`). GNU runs no expansion at all inside a
/// single-quoted span (subst.c copies a single-quoted region verbatim), so
/// such an RHS is literal data and must not be scanned for `$(...)` or
/// backticks by the assignment expander — otherwise `x='$(date)'` stores the
/// substitution output instead of the text `$(date)`.
///
/// Quote-aware tiling: `'a'$x`, `'a'"b"` and `'a'\'b'` are *not* fully single
/// quoted, because a character (or escape) sits outside the single-quoted
/// spans and would be expanded by GNU.
pub(super) fn assignment_rhs_is_fully_single_quoted(raw: &str) -> bool {
    let Some((_, rhs)) = raw.split_once('=') else {
        return false;
    };
    let mut in_single = false;
    let mut saw_span = false;
    for ch in rhs.chars() {
        if ch == '\'' {
            in_single = !in_single;
            saw_span = true;
            continue;
        }
        if !in_single {
            return false;
        }
    }
    saw_span && !in_single
}

/// Rewrite a wholly single-quoted assignment RHS into protected literal data:
/// drop the quote delimiters and carry `$`/backtick as the walker's literal
/// markers (\x1f / \x1a), which the parameter-expansion and storage layers
/// restore on the way out. The `name=` prefix is copied verbatim.
pub(super) fn protect_fully_single_quoted_assignment(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars();
    for ch in chars.by_ref() {
        out.push(ch);
        if ch == '=' {
            break;
        }
    }
    for ch in chars {
        match ch {
            '\'' => {}
            '$' => out.push('\x1f'),
            '`' => out.push('\x1a'),
            _ => out.push(ch),
        }
    }
    out
}

pub(super) fn mark_quoted_assignment_value(raw: &str, value: &str) -> String {
    let Some((name, rhs)) = value.split_once('=') else {
        return value.to_string();
    };
    let raw_rhs = raw.split_once('=').map(|(_, rhs)| rhs).unwrap_or_default();
    let rhs = if raw_rhs.starts_with('\"')
        && raw_rhs.ends_with('\"')
        && !raw_rhs.contains("$(")
        && !raw_rhs.contains('`')
    {
        rhs.replace('\'', "\x16")
    } else {
        rhs.to_string()
    };

    format!("{name}=\x1c{rhs}")
}

pub(super) fn quoted_literal_tilde(raw: &str, value: &str) -> bool {
    value.starts_with('~')
        && ((raw.starts_with('\'') && raw.ends_with('\''))
            || (raw.starts_with('"') && raw.ends_with('"')))
}
