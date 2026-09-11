pub(super) fn skip_heredoc_in_chars_with_closure(chars: &[char], start: usize) -> (usize, bool) {
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
    while chars
        .get(index)
        .is_some_and(|ch| !ch.is_whitespace() && !matches!(ch, ';' | '|' | '&' | ')'))
    {
        // A backslash quotes the next delimiter byte (`<<\)` uses a literal
        // `)` delimiter); consume the escape pair as one unit so the quoted
        // `)` is not mistaken for the substitution closer.
        if chars.get(index) == Some(&'\\') && chars.get(index + 1).is_some() {
            index += 1;
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
        return (index, false);
    }
    let mut header_closes_command_substitution = false;
    while chars.get(index).is_some_and(|ch| *ch != '\n') {
        if chars.get(index) == Some(&')') {
            header_closes_command_substitution = true;
        }
        index += 1;
    }
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

    (index, header_closes_command_substitution && found_delimiter)
}
