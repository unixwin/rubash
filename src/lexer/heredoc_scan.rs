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
    let (index, closure, _terminator_found) = skip_heredoc_in_chars_decided(chars, start);
    (index, closure)
}

/// perf17 variant of [`skip_heredoc_in_chars_with_closure`]: additionally
/// reports whether the heredoc's terminator line was found inside `chars`.
///
/// GNU anchor (make_cmd.c:512 `make_here_document`, driven by parse.y:3120
/// `gather_here_documents`): the body is read line by line through
/// `read_secondary_line` until a line equals the delimiter word exactly,
/// and the reader then continues PAST that line — a consumed heredoc body
/// is never re-read on later input lines (parse.y:3557 `read_token` streams
/// and never re-scans consumed text). The `terminator_found` flag carries
/// exactly that fact to checkpoint callers: once the terminator line is in
/// the buffer, the skip's resume index is prefix-stable — the terminator
/// search compares only WHOLE lines (the group driver appends complete
/// '\n'-terminated physical lines, and the park machinery guarantees the
/// `<<` itself never straddles a resume boundary), so a longer buffer can
/// only append lines after the ones already compared and cannot move the
/// first match. A `terminator_found == false` answer is the undecided
/// case: the terminator line (or the make_cmd.c:602-611 `EOF`-prefix
/// pushback line) has not arrived yet, and future input decides it.
///
/// The empty-delimiter early return (e.g. `<< ""`) reports `false` by
/// construction: it never scans a body at all, so callers keep their
/// pre-perf17 parking behavior there.
pub(super) fn skip_heredoc_in_chars_decided(
    chars: &[char],
    start: usize,
) -> (usize, Option<(usize, usize)>, bool) {
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
    // substitution closer (comsub-posix.tests). UNQUOTED, the delimiter
    // word ends at ANY shell metacharacter (parse.y:5688 shellbreak within
    // parse.y:5305 read_token_word), so `<<EOF>#c)` declares `EOF' with
    // `>' an operator and `#c)' a comment; the `)' inside that comment
    // never closes the substitution (rubash#305 lexmix family: the old
    // break set swallowed `>#c' INTO the delimiter).
    let mut delimiter_single = false;
    let mut delimiter_double = false;
    while let Some(next) = chars.get(index).copied() {
        match next {
            '\'' if !delimiter_double => delimiter_single = !delimiter_single,
            '"' if !delimiter_single => delimiter_double = !delimiter_double,
            _ if !delimiter_single
                && !delimiter_double
                && (next.is_whitespace()
                    || matches!(next, ';' | '|' | '&' | ')' | '(' | '<' | '>')) =>
            {
                break;
            }
            '#' if !delimiter_single && !delimiter_double && index == delimiter_start => break,
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
        return (index, None, false);
    }
    let mut header_close_paren = None;
    // A word-initial `#' after the delimiter word comments through the end
    // of the header line (parse.y:3630-3643 parse_comment at the token
    // boundary the operator left), so a `)' inside the comment tail does
    // not close the substitution.
    let mut comment_to_eol = false;
    while chars.get(index).is_some_and(|ch| *ch != '\n') {
        if !comment_to_eol {
            let ch = chars[index];
            if ch == '#'
                && (index == 0
                    || chars[index - 1].is_whitespace()
                    || matches!(chars[index - 1], ';' | '|' | '&' | '(' | ')' | '<' | '>'))
            {
                comment_to_eol = true;
            } else if ch == ')' && header_close_paren.is_none() {
                header_close_paren = Some(index);
            }
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
    (index, closure, found_delimiter)
}
