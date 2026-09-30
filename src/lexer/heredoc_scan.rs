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
    let (index, closure, terminator_found, _delimiter_empty, _substitution_on_header) =
        skip_heredoc_in_chars_core(chars, start, true);
    (index, closure, terminator_found)
}

/// perf19: TOP-LEVEL (non-command-substitution) variant of the heredoc
/// skip for the residual scanners. GNU make_cmd.c:512 `make_here_document`
/// reads the body as RAW lines — the parser's quoting state
/// (parse.y:5305 `read_token_word`, driven by parse.y:3557 `read_token`)
/// never processes a body character, so the group-completeness scanners
/// must treat the whole `<<delim ... <terminator line>` span as opaque
/// whenever no `$(`/backtick is open. Two comsub-only behaviors are
/// DISABLED here (GNU gates both on `PST_EOFTOKEN`, set only by
/// parse.y:4513 `parse_comsub` for the substitution's eof token):
///
/// - the make_cmd.c:602-611 backwards-compatibility pushback (a body line
///   that starts with the delimiter and contains the eof token later ends
///   the heredoc, resuming at that token): at top level such a line
///   (`EOF)x`) is plain BODY text — only a line equal to the delimiter
///   exactly terminates (make_cmd.c:571-574 `STREQN ... && line[redir_len]
///   == '\n'`).
/// - the `EOF)` / ``EOF` `` suffix match of the comsub skip (same
///   pushback family): disabled for the same reason.
///
/// Returns `None` — callers keep their pre-perf19 fall-through — for two
/// undecided-for-this-scan shapes:
///
/// - the delimiter word is empty (`<< ""`, `<<` at the buffer tail):
///   GNU's terminator is the first empty line, which this scan does not
///   search for (no new park is introduced);
/// - the header line carries an OPEN substitution introducer (`$(`,
///   `${`, or a backtick) after the `<<` word: GNU gathers the body at
///   the newline that ends the COMMAND, and with an open `$(`/backtick on
///   the header line that newline is INSIDE the substitution — the
///   following lines belong to its body, not to the heredoc (upstream
///   heredoc7.sub: `cat <<EOF && grep $(` reads ` foobar`/`EOF` as
///   substitution text and warns the heredoc unterminated at EOF). The
///   scan does not model that interleaving, so it refuses and the caller
///   falls back to its pre-perf19 char-by-char path (the comsub machinery
///   then owns the lines, exactly as before).
///
/// Otherwise returns `(resume_index, terminator_found)` with the perf17
/// prefix-stability contract: `resume_index` sits just past the
/// terminator line's '\n' once the terminator has arrived, and a
/// `terminator_found == false` answer means future input decides.
pub(super) fn skip_heredoc_top_level(chars: &[char], start: usize) -> Option<(usize, bool)> {
    let (index, _closure, terminator_found, delimiter_empty, substitution_on_header) =
        skip_heredoc_in_chars_core(chars, start, false);
    if delimiter_empty || substitution_on_header {
        return None;
    }
    Some((index, terminator_found))
}

/// Shared body of the heredoc skips. `comsub_context` enables the two
/// PST_EOFTOKEN-gated behaviors (see [`skip_heredoc_top_level`]); the
/// fourth component reports the empty-delimiter early return and the
/// fifth an open substitution introducer on the header line, so the
/// top-level variant can refuse to decide those shapes.
#[allow(clippy::type_complexity)]
fn skip_heredoc_in_chars_core(
    chars: &[char],
    start: usize,
    comsub_context: bool,
) -> (usize, Option<(usize, usize)>, bool, bool, bool) {
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
        return (index, None, false, true, false);
    }
    let mut header_close_paren = None;
    // perf19: an open substitution introducer on the header line (`$(`,
    // `${`, backtick) — see skip_heredoc_top_level's doc for why the
    // top-level scan refuses that shape.
    let mut substitution_on_header = false;
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
            if ch == '`' || (ch == '$' && matches!(chars.get(index + 1), Some('(') | Some('{'))) {
                substitution_on_header = true;
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
        // perf19: the two eof-token (PST_EOFTOKEN) matches below are
        // comsub-context only (make_cmd.c:600-611 requires `parser_state &
        // PST_EOFTOKEN && shell_eof_token`); at top level only the exact
        // whole-line match (make_cmd.c:571-574) terminates the body.
        if comsub_context
            && comparable
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
        if comsub_context && comparable.starts_with(delimiter.as_str()) {
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
    (
        index,
        closure,
        found_delimiter,
        false,
        substitution_on_header,
    )
}
