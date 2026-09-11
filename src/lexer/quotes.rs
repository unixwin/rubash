use super::ansi::decode_ansi_c_quoted;
use super::dolbrace::{scan_braced_parameter, BraceContext, DolbraceState};

/// Emitted by quote removal right before a quote boundary that terminates
/// an unbraced `$name` parameter. Quote removal deletes the quote, which
/// would otherwise let the expansion stage read `a$x"b"` as the variable
/// `xb` (bash keeps the name boundary in the word's quote structure,
/// subst.c param_expand). The marker is a non-name character: name
/// collection stops at it, and the expansion walkers drop it from output.
pub(crate) const PARAM_NAME_END_MARKER: char = '\u{13}';

/// Quote markers emitted by `escape_decoded_ansi_c_quotes` for quote
/// characters produced by ANSI-C decoding of a `$'...'` word. They travel
/// through assignment storage and are restored to `'` / `"` by the
/// consumers that end the assignment path.
///
/// The C0 range and the first three private-use code points are all taken:
/// U+0011/14/17/1F carry the walker's protection markers, U+0013 is
/// PARAM_NAME_END_MARKER, and U+E000/E001 are the assignment data-quote
/// sentinels while U+E002 is QUOTED_NULL_MARKER (embedded_mutations.rs).
/// Using U+E002 here is what made `recho "\a"` print garbage in the
/// builtins/quote/tilde2/trap suites: `recho` (tests/recho.c) renders every
/// byte below 0x20 as `^X`, and U+E002 is a three-byte UTF-8 sequence
/// (EE 80 82) whose bytes all pass the `>= ' '` check, so it printed raw
/// instead of as the `^W` that U+0017 produces. Start at U+E010 to keep a
/// margin between the two marker families.
pub(crate) const ANSI_C_QUOTE_MARKER: char = '\u{E010}';
pub(crate) const ANSI_C_DQUOTE_MARKER: char = '\u{E011}';

/// `&str` forms for the `.replace(...)` restore sites, whose receivers are
/// already `String` and therefore require `&str` arguments.
pub(crate) const ANSI_C_QUOTE_MARKER_STR: &str = "\u{E010}";
pub(crate) const ANSI_C_DQUOTE_MARKER_STR: &str = "\u{E011}";

pub(crate) fn remove_shell_quotes(raw: &str) -> String {
    remove_shell_quotes_with_posix(raw, false)
}

/// Quote removal with the lexer's POSIX mode. Inside double quotes the
/// `${...}` span scan must agree with the tokenizing skip phase: in POSIX
/// mode the Interp 221 big hammer closes the span at the first `}` (single
/// quotes are literal), otherwise the de-quoted value keeps quote structure
/// the expansion stage cannot interpret (posixexp2 case 28).
pub(crate) fn remove_shell_quotes_with_posix(raw: &str, posix: bool) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    // Array-subscript regions keep `\"` as a bare data quote: the subscript
    // parser owns quote semantics there and the word-value contract expects
    // the de-escaped form. Outside subscripts `\"` must survive expansion as
    // data, so it travels as the walker's data-double-quote marker.
    let mut subscript_depth = 0usize;
    // True while the emitted tail is an unbraced `$name` parameter; a quote
    // boundary at that point must carry PARAM_NAME_END_MARKER (see above).
    let mut pending_name = false;

    while let Some(ch) = chars.next() {
        match ch {
            '[' => {
                subscript_depth += 1;
                pending_name = false;
                out.push(ch);
            }
            ']' if subscript_depth > 0 => {
                subscript_depth -= 1;
                pending_name = false;
                out.push(ch);
            }
            '$' if chars.peek() == Some(&'(') => {
                pending_name = false;
                copy_dollar_paren_substitution(&mut out, &mut chars);
            }
            '$' if chars.peek() == Some(&'\'') => {
                pending_name = false;
                chars.next();
                let mut quoted = String::new();
                let mut escaped = false;
                for quoted_ch in chars.by_ref() {
                    if escaped {
                        quoted.push('\\');
                        quoted.push(quoted_ch);
                        escaped = false;
                        continue;
                    }
                    if quoted_ch == '\\' {
                        escaped = true;
                        continue;
                    }
                    if quoted_ch == '\'' {
                        break;
                    }
                    quoted.push(quoted_ch);
                }
                if escaped {
                    quoted.push('\\');
                }
                // GNU subst.c CTLESC-quotes every byte an ANSI-C string decodes
                // to, so quote characters in the decoded value are data for
                // every later pass (posixexp7: the escaped-quote ANSI-C word
                // kept its literal quote; unescaped, the expansion walker
                // consumed the bare quote as a delimiter). Rubash's walker
                // expresses that with backslash escapes, so escape the decode
                // output with that convention, keeping backslash-prefixed
                // runs verbatim.
                out.push_str(&escape_decoded_ansi_c_quotes(&decode_ansi_c_quoted(
                    &quoted,
                )));
            }
            '$' if chars.peek() == Some(&'"') => {
                pending_name = false;
                chars.next();
                remove_double_quoted_into(&mut out, &mut chars, false, posix);
            }
            '$' if chars.peek() == Some(&'{') => {
                pending_name = false;
                copy_braced_parameter_unquoted(&mut out, &mut chars);
            }
            '\'' => {
                if pending_name {
                    out.push(PARAM_NAME_END_MARKER);
                }
                pending_name = false;
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    if quoted == '$' {
                        // Preserve the existing protected-dollar contract used by
                        // downstream expansion, but do not protect literal globs.
                        out.push('\x1f');
                    } else {
                        out.push(quoted);
                    }
                }
            }
            '"' => {
                if pending_name {
                    out.push(PARAM_NAME_END_MARKER);
                }
                pending_name = false;
                // GNU subst.c string_extract_double_quoted: a backtick inside
                // double quotes enters backquote mode and the body is copied
                // verbatim (subst.c:925-932) - the body's quotes are delimiters
                // of the inner command, never literal data of this word
                // (quote.tests:47 `echo "`echo 'foo bar'`"` must not leak the
                // single quotes). Nested $()/\\${} spans inside the body stay
                // owned by copy_backtick_body_preserving_syntax.
                remove_double_quoted_into(&mut out, &mut chars, true, posix);
            }
            '`' => {
                pending_name = false;
                out.push(ch);
                copy_backtick_body_preserving_syntax(&mut out, &mut chars);
            }
            '\\' => {
                pending_name = false;
                let Some(escaped) = chars.next() else {
                    out.push(ch);
                    continue;
                };
                if escaped == '$' {
                    out.push('\x1f');
                } else if escaped == '`' {
                    out.push('\x1a');
                } else if escaped == '\'' {
                    out.push('\x17');
                } else if escaped == '"' {
                    if subscript_depth > 0 {
                        // Inside a subscript the de-escaped quote is data for
                        // the subscript parser (`a[\" \"]=15` keeps `a[" "]=15`).
                        out.push('"');
                    } else {
                        // `\"` outside quotes is a literal double quote that
                        // must survive as data: downstream expansion scanners
                        // toggle quote state on bare quotes, which would
                        // swallow it (posixexp2 case 8, `echo \"`). \x18 is
                        // the walker's data-double-quote marker, restored on
                        // output.
                        out.push('\x18');
                    }
                } else if escaped == '\\' {
                    // Keep a literal backslash distinct from the protected
                    // double-quote marker used by expansion internals.
                    out.push('\x14');
                } else if matches!(escaped, '*' | '?' | '[' | '@' | '+' | '!') {
                    out.push('\x11');
                    out.push(escaped);
                } else {
                    out.push(escaped);
                }
            }
            _ => {
                if ch == '$' {
                    pending_name = chars
                        .peek()
                        .is_some_and(|next| is_lexer_shell_name_start(*next));
                } else if !(pending_name && is_lexer_shell_name_char(ch)) {
                    pending_name = false;
                }
                out.push(ch);
            }
        }
    }

    out
}

fn is_lexer_shell_name_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_lexer_shell_name_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

pub(super) fn remove_shell_quotes_outside_backticks(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    let mut pending_name = false;

    while let Some(ch) = chars.next() {
        match ch {
            '`' => {
                pending_name = false;
                out.push(ch);
                copy_backtick_body_preserving_syntax(&mut out, &mut chars);
            }
            '$' if chars.peek() == Some(&'"') => {
                pending_name = false;
                chars.next();
                remove_double_quoted_into(&mut out, &mut chars, true, false);
            }
            '$' if chars.peek() == Some(&'{') => {
                pending_name = false;
                copy_braced_parameter_unquoted(&mut out, &mut chars);
            }
            '\'' => {
                if pending_name {
                    out.push(PARAM_NAME_END_MARKER);
                }
                pending_name = false;
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    out.push(quoted);
                }
            }
            '"' => {
                if pending_name {
                    out.push(PARAM_NAME_END_MARKER);
                }
                pending_name = false;
                remove_double_quoted_into(&mut out, &mut chars, true, false);
            }
            '\\' => {
                pending_name = false;
                let Some(escaped) = chars.next() else {
                    out.push(ch);
                    continue;
                };
                if escaped == '\'' {
                    out.push('\x17');
                } else if escaped == '`' {
                    out.push('\x1a');
                } else {
                    out.push(escaped);
                }
            }
            _ => {
                if ch == '$' {
                    pending_name = chars
                        .peek()
                        .is_some_and(|next| is_lexer_shell_name_start(*next));
                } else if !(pending_name && is_lexer_shell_name_char(ch)) {
                    pending_name = false;
                }
                out.push(ch);
            }
        }
    }

    out
}

pub(super) fn normalize_backtick_command_substitution(raw: &str) -> String {
    let mut chars = raw.chars().peekable();
    if chars.next() != Some('`') {
        return raw.to_string();
    }
    let mut out = String::from("`");
    copy_backtick_body_preserving_syntax(&mut out, &mut chars);
    out.extend(chars);
    out
}

fn remove_double_quoted_into(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    preserve_backticks: bool,
    posix: bool,
) {
    // Unbraced `$name` inside double quotes keeps collecting name characters
    // up to the closing quote (bash: `"$xb"` reads the variable `xb`); the
    // closing quote must therefore emit PARAM_NAME_END_MARKER so the
    // expansion stage stops the name there instead of swallowing the
    // following literal text.
    let mut pending_name = false;
    while let Some(quoted) = chars.next() {
        if quoted == '$' && chars.peek() == Some(&'(') {
            pending_name = false;
            copy_dollar_paren_substitution(out, chars);
            continue;
        }
        if quoted == '$' && chars.peek() == Some(&'{') {
            pending_name = false;
            copy_braced_parameter_after_dollar(out, chars, posix);
            continue;
        }
        if quoted == '$'
            && matches!(
                chars.peek().copied(),
                Some('?' | '$' | '!' | '#' | '-' | '@' | '*' | '0'..='9')
            )
        {
            pending_name = false;
            out.push('$');
            if let Some(param) = chars.next() {
                out.push(param);
            }
            continue;
        }
        match quoted {
            '"' => {
                if pending_name {
                    out.push(PARAM_NAME_END_MARKER);
                }
                break;
            }
            '`' if preserve_backticks => {
                pending_name = false;
                out.push(quoted);
                copy_backtick_body_preserving_syntax(out, chars);
            }
            '\\' => {
                pending_name = false;
                if let Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) = chars.peek().copied() {
                    chars.next();
                    if escaped != '\n' {
                        match escaped {
                            '$' => out.push('\x1f'),
                            '`' => out.push('\x1a'),
                            '\\' => out.push('\x14'),
                            _ => out.push(escaped),
                        }
                    }
                } else {
                    out.push('\\');
                }
            }
            '\'' => {
                // GNU parse.y skip_double_quoted: only ", \, $ and ` are
                // special inside double quotes; a single quote is an ordinary
                // literal character. Carry it with the same protected-literal
                // marker as \' so the expansion stage keeps it as data instead
                // of re-reading it as a single-quote delimiter (which would
                // also suppress parameter expansion across the pseudo span).
                pending_name = false;
                out.push('\x17');
            }
            _ if matches!(quoted, '*' | '?' | '[' | '@' | '+' | '!') => {
                pending_name = false;
                out.push('\x11');
                out.push(quoted);
            }
            _ => {
                if quoted == '$' {
                    pending_name = chars
                        .peek()
                        .is_some_and(|next| is_lexer_shell_name_start(*next));
                } else if !(pending_name && is_lexer_shell_name_char(quoted)) {
                    pending_name = false;
                }
                out.push(quoted);
            }
        }
    }
}

fn copy_dollar_paren_substitution(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    out.push('$');
    if chars.next() != Some('(') {
        return;
    }
    out.push('(');
    copy_dollar_paren_body_raw(out, chars);
}

fn copy_single_quoted_raw(out: &mut String, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for ch in chars.by_ref() {
        out.push(ch);
        if ch == '\'' {
            break;
        }
    }
}

fn copy_ansi_c_single_quoted_raw(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    let mut escaped = false;
    for ch in chars.by_ref() {
        out.push(ch);
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '\'' {
            break;
        }
    }
}

/// Mark quote characters in a decoded ANSI-C string with the walker's
/// data-quote markers so later expansion passes treat them as data (the
/// CTLESC equivalent; see the call site). The markers are the same ones the
/// lexer emits for backslash-escaped quotes in source words, so every
/// consumer already restores them.
fn escape_decoded_ansi_c_quotes(decoded: &str) -> String {
    decoded
        .replace('\'', &ANSI_C_QUOTE_MARKER.to_string())
        .replace('"', &ANSI_C_DQUOTE_MARKER.to_string())
}

fn copy_double_quoted_raw(out: &mut String, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(ch) = chars.next() {
        out.push(ch);
        match ch {
            '"' => break,
            '\\' => {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            }
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                out.push('(');
                copy_dollar_paren_body_raw(out, chars);
            }
            _ => {}
        }
    }
}

fn copy_dollar_paren_body_raw(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    let mut depth = 1usize;
    while let Some(ch) = chars.next() {
        out.push(ch);
        match ch {
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                out.push('\'');
                copy_ansi_c_single_quoted_raw(out, chars);
            }
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                out.push('(');
                depth += 1;
            }
            '\'' => copy_single_quoted_raw(out, chars),
            '"' => copy_double_quoted_raw(out, chars),
            '`' => copy_backtick_raw(out, chars),
            '\\' => {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            }
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
}

fn copy_backtick_raw(out: &mut String, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    copy_backtick_body_preserving_syntax(out, chars);
}

fn copy_backtick_body_preserving_syntax(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    while let Some(ch) = chars.next() {
        if ch == '`' {
            out.push(ch);
            break;
        }
        if ch == '\\' {
            match chars.next() {
                Some('\n') => {}
                Some('\r') if chars.peek().copied() == Some('\n') => {
                    chars.next();
                }
                Some(escaped) => {
                    out.push(ch);
                    out.push(escaped);
                }
                None => out.push(ch),
            }
            continue;
        }
        out.push(ch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_single_quotes_inside_double_quoted_parameter_word() {
        assert_eq!(remove_shell_quotes("\"${IFS+'}'z}\""), "${IFS+'}'z}");
    }

    #[test]
    fn double_quoted_single_quote_becomes_protected_literal_data() {
        // GNU parse.y skip_double_quoted: a single quote inside double
        // quotes is ordinary data, carried with the same protected marker
        // as an escaped quote so expansion never re-reads it as a
        // single-quote delimiter.
        assert_eq!(remove_shell_quotes("\"a:'b' c\""), "a:\x17b\x17 c");
    }
}

// Source-mapped to subst.c::extract_dollar_brace_string: quote removal
// receives explicit outer-quote and POSIX context instead of conflating them.
pub(super) fn copy_braced_parameter_after_dollar(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    posix: bool,
) {
    out.push('$');
    if chars.peek() != Some(&'{') {
        return;
    }
    let remaining: String = chars.clone().collect();
    let mut wrapped = String::from("$");
    wrapped.push_str(&remaining);
    let context = BraceContext {
        outer_double_quote: true,
        // POSIX mode comes from the lexer: parse.y's matched-pair scan runs
        // the Interp 221 big hammer there (single quotes inside the
        // double-quoted span are literal, first `}` closes).
        posix,
        replacement_context: false,
        initial_state: DolbraceState::Param,
    };
    if let Some(scan) = scan_braced_parameter(&wrapped, context) {
        let consumed = wrapped[..scan.end].chars().count().saturating_sub(1);
        for _ in 0..consumed {
            if let Some(ch) = chars.next() {
                out.push(ch);
            }
        }
        return;
    }
    out.push(chars.next().unwrap());
    let mut depth = 1usize;
    while let Some(ch) = chars.next() {
        out.push(ch);
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next();
            out.push('{');
            depth += 1;
            continue;
        }
        if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
    }
}

// Unquoted `${...}` units keep their body verbatim (GNU quote removal never
// strips quotes inside a parameter expansion; the expansion stage owns the
// quote state). Whole-word `${...}` tokens already bypass quote removal via
// the lexer's Variable path; this gives embedded occurrences the same shape.
fn copy_braced_parameter_unquoted(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    out.push('$');
    if chars.peek() != Some(&'{') {
        return;
    }
    let remaining: String = chars.clone().collect();
    let mut wrapped = String::from("$");
    wrapped.push_str(&remaining);
    let context = BraceContext {
        outer_double_quote: false,
        posix: false,
        replacement_context: false,
        initial_state: DolbraceState::Param,
    };
    if let Some(scan) = scan_braced_parameter(&wrapped, context) {
        let consumed = wrapped[..scan.end].chars().count().saturating_sub(1);
        for _ in 0..consumed {
            if let Some(ch) = chars.next() {
                out.push(ch);
            }
        }
        return;
    }
    out.push(chars.next().unwrap());
    let mut depth = 1usize;
    while let Some(ch) = chars.next() {
        out.push(ch);
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next();
            out.push('{');
            depth += 1;
            continue;
        }
        if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
    }
}

#[cfg(test)]
mod probe_tests {
    #[test]
    fn probe_escaped_quote_value() {
        let out = super::remove_shell_quotes("a[\\\" \\\"]=15");
        eprintln!("PROBE-OUT={out:?}");
    }
}
