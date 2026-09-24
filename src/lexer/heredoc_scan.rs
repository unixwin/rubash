/// Returns the resume index plus, when a `)` on the heredoc header line
/// closed the enclosing `$(...)` and the delimiter line was found, the
/// `)` position and the header line's `\n` position. GNU parse.y
/// parse_comsub (PST_EOFTOKEN) treats that `)` as the substitution's eof
/// token and gathers the pending body from the following input lines
/// (parse.y:4564 gather_here_documents); print_comsub reprints the word
/// with the body inside the closing `)`.
pub(super) fn skip_heredoc_in_chars_with_closure(
    chars: &[char],
    start: usize,
) -> (usize, Option<(usize, usize)>) {
    let mut index = start + 2;
    let strip_tabs = if chars.get(index) == Some(&'-') {
        index += 1;
        true
    } else {
        false
    };
    while chars.get(index).is_some_and(|ch| matches!(ch, ' ' | '\t')) {
        index += 1;
    }
    let delimiter_start = index;
    // GNU read_token_word: quoting inside the delimiter word makes
    // metacharacters literal — `<< ')'` names `)` as the delimiter, so a
    // quoted `)` (or `;`, `|`, `&`) is delimiter text, not the
    // substitution closer (comsub-posix.tests).
    let mut delimiter_single = false;
    let mut delimiter_double = false;
    while let Some(next) = chars.get(index).copied() {
        match next {
            '\'' if !delimiter_double => delimiter_single = !delimiter_single,
            '"' if !delimiter_single => delimiter_double = !delimiter_double,
            _ if !delimiter_single
                && !delimiter_double
                && (next.is_whitespace() || matches!(next, ';' | '|' | '&' | ')')) =>
            {
                break;
            }
            // A backslash quotes the next delimiter byte (`<<\)` uses a
            // literal `)` delimiter); consume the escape pair as one unit so
            // the quoted `)` is not mistaken for the substitution closer.
            '\\' if !delimiter_single && !delimiter_double && chars.get(index + 1).is_some() => {
                index += 1;
            }
            _ => {}
        }
        index += 1;
    }
    let mut delimiter = chars[delimiter_start..index]
        .iter()
        .collect::<String>()
        .replace(['\'', '"', '\\'], "");
    if strip_tabs {
        delimiter = delimiter.trim_start_matches('\t').to_string();
    }
    if delimiter.is_empty() {
        return (index, None);
    }
    let mut header_close_paren = None;
    while chars.get(index).is_some_and(|ch| *ch != '\n') {
        if chars.get(index) == Some(&')') && header_close_paren.is_none() {
            header_close_paren = Some(index);
        }
        index += 1;
    }
    let header_end = index;
    if chars.get(index) == Some(&'\n') {
        index += 1;
    }

    let mut found_delimiter = false;
    while index < chars.len() {
        let line_start = index;
        while chars.get(index).is_some_and(|ch| *ch != '\n') {
            index += 1;
        }
        let line = chars[line_start..index].iter().collect::<String>();
        let comparable = if strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line.as_str()
        };
        if comparable
            .strip_suffix([')', '`'])
            .is_some_and(|value| value == delimiter)
        {
            let leading_tabs = if strip_tabs {
                line.chars().take_while(|ch| *ch == '\t').count()
            } else {
                0
            };
            index = line_start + leading_tabs + delimiter.chars().count();
            break;
        }
        if comparable == delimiter {
            found_delimiter = true;
            if chars.get(index) == Some(&'\n') {
                index += 1;
            }
            break;
        }
        // GNU make_cmd.c:602-611 (PST_EOFTOKEN): a body line that starts with
        // the delimiter and carries `)` later on ends the heredoc as if it hit
        // EOF; the remainder is pushed back into the parser input, where the
        // `)` then closes the command substitution (`foo=$(cat <<EOF\nhi\nEOF`).
        // Resume at that first `)` so the paren-balance scan sees the closer.
        if comparable.starts_with(delimiter.as_str()) {
            if let Some(paren) = comparable[delimiter.len()..].find(')') {
                index = line_start + delimiter.chars().count() + paren;
                break;
            }
        }
        if chars.get(index) == Some(&'\n') {
            index += 1;
        }
    }

    let closure = if header_close_paren.is_some() && found_delimiter {
        header_close_paren.map(|paren| (paren, header_end))
    } else {
        None
    };
    (index, closure)
}
