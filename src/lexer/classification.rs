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

pub(super) fn has_unquoted_assignment_equal(raw: &str) -> bool {
    let mut chars = raw.chars();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                chars.next();
            }
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '=' if !in_single && !in_double => return true,
            _ => {}
        }
    }
    false
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

    let mut in_backtick = false;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' {
            escaped = true;
            continue;
        }

        if ch == '`' {
            in_backtick = !in_backtick;
            continue;
        }

        if !in_backtick && matches!(ch, '"' | '\'') {
            return true;
        }
    }

    false
}

pub(super) fn mark_quoted_assignment_value(value: &str) -> String {
    let Some((name, rhs)) = value.split_once('=') else {
        return value.to_string();
    };

    format!("{name}=\x1c{rhs}")
}

pub(super) fn quoted_literal_tilde(raw: &str, value: &str) -> bool {
    value == "~"
        && ((raw.starts_with('\'') && raw.ends_with('\''))
            || (raw.starts_with('"') && raw.ends_with('"')))
}
