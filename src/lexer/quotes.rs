use super::ansi::decode_ansi_c_quoted;
use super::dolbrace::{scan_braced_parameter, BraceContext, DolbraceState};
use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};

/// Emitted by quote removal right before a quote boundary that terminates
/// an unbraced `$name` parameter. Quote removal deletes the quote, which
/// would otherwise let the expansion stage read `a$x"b"` as the variable
/// `xb` (bash keeps the name boundary in the word's quote structure,
/// subst.c param_expand). The marker is a non-name character: name
/// collection stops at it, and the expansion walkers drop it from output.
pub(crate) const PARAM_NAME_END_MARKER: char = crate::executor::markers::PARAM_NAME_END_MARKER;

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
pub(crate) const ANSI_C_QUOTE_MARKER: char = crate::executor::markers::ANSI_C_QUOTE_MARKER;
pub(crate) const ANSI_C_DQUOTE_MARKER: char = crate::executor::markers::ANSI_C_DQUOTE_MARKER;

/// `&str` forms for the `.replace(...)` restore sites, whose receivers are
/// already `String` and therefore require `&str` arguments.
pub(crate) const ANSI_C_QUOTE_MARKER_STR: &'static str =
    crate::executor::markers::ANSI_C_QUOTE_MARKER_STR;
pub(crate) const ANSI_C_DQUOTE_MARKER_STR: &'static str =
    crate::executor::markers::ANSI_C_DQUOTE_MARKER_STR;

pub(crate) fn remove_shell_quotes(raw: &str) -> String {
    remove_shell_quotes_with_posix(raw, false)
}

/// Replace every `'` that is NOT inside a substitution unit (`${...}`,
/// `$(...)`, `` `...` ``) with the ANSI-C quote data marker.
///
/// GNU parse.y:3877 parse_matched_pair / parse.y:4451 parse_comsub own the
/// quoting inside a substitution: a quote there belongs to the
/// substitution's own grammar — the pattern quotes of `${x#'foo'}` (POSIX
/// 2.6.2: "quoting characters within the braces shall have this effect" on
/// the pattern characters), the default value of `${x:-'v'}`, the words of
/// an inner command — never to the enclosing double-quoted word. Only a
/// `'` in the word's own text is literal data that must survive downstream
/// quote removal (parse.y skip_double_quoted: `'` is ordinary inside
/// `"..."`). Blanket `.replace('\'', marker)` across a double-quoted body
/// corrupts the substitution's quotes into data markers (rubash#258
/// BUG_PSUBSQUOT: `v="${x#'foo'}"` kept `'foo'` literal and stopped
/// stripping the prefix).
pub(crate) fn mark_data_squotes_around_substitutions(body: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len());
    let mut index = 0usize;
    while index < chars.len() {
        let unit_start = index;
        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            if let Some(close) = skip_braced_unit_quotes(&chars, index + 1, '}') {
                out.extend(&chars[unit_start..=close]);
                index = close + 1;
                continue;
            }
        }
        if chars[index] == '$' && chars.get(index + 1) == Some(&'(') {
            if let Some(close) = skip_braced_unit_quotes(&chars, index + 1, ')') {
                out.extend(&chars[unit_start..=close]);
                index = close + 1;
                continue;
            }
        }
        if chars[index] == '`' {
            index += 1;
            while index < chars.len() && chars[index] != '`' {
                if chars[index] == '\\' && index + 1 < chars.len() {
                    index += 1;
                }
                index += 1;
            }
            if index < chars.len() {
                index += 1;
            }
            out.extend(&chars[unit_start..index]);
            continue;
        }
        if chars[index] == '\'' {
            out.push(crate::executor::markers::ANSI_C_QUOTE_MARKER);
        } else {
            out.push(chars[index]);
        }
        index += 1;
    }
    out
}

/// Quote-aware skip from an opener (the `{` of `${` or the `(` of `$(`) to
/// its matching closer at nesting depth 0, honoring `'...'`/`"..."` spans
/// and backslash escapes outside single quotes (the same state machine GNU
/// parse_matched_pair runs; the executor's
/// skip_braced_case_pattern_unit is its twin). Returns the index OF the
/// closing character.
fn skip_braced_unit_quotes(chars: &[char], open: usize, closer: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;
    let mut single = false;
    let mut double = false;
    while index < chars.len() {
        match chars[index] {
            '\\' if !single => {
                index += 2;
                continue;
            }
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '{' if !single && !double && closer == '}' => depth += 1,
            '(' if !single && !double && closer == ')' => depth += 1,
            ')' if !single && !double && closer == ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            '}' if !single && !double && closer == '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Quote removal with the lexer's POSIX mode. Inside double quotes the
/// `${...}` span scan must agree with the tokenizing skip phase: in POSIX
/// mode the Interp 221 big hammer closes the span at the first `}` (single
/// quotes are literal), otherwise the de-quoted value keeps quote structure
/// the expansion stage cannot interpret (posixexp2 case 28).
pub(crate) fn remove_shell_quotes_with_posix(raw: &str, posix: bool) -> String {
    remove_shell_quotes_inner_cursor(raw, posix)
}

/// Assignment-word quote removal: identical to
/// `remove_shell_quotes_with_posix` except that expansion-trigger characters
/// inside `'...'` spans travel as the walker's data carriers.
///
/// GNU parse.y:5366-5398 read_token_word: a backslash-escaped quote
/// consumes two characters and never opens quote state, so the `'` right
/// after `\'` re-enters parse_matched_pair (parse.y:5419-5437) as a fresh
/// single-quoted span. Inside that span every byte is quoted data —
/// subst.c:11882-11886 expand_word_internal case '\'' hands the whole span
/// to add_quoted_string without ever consulting the command-substitution
/// scanner — and dequote_string (subst.c:4807) later removes only the
/// delimiters. A `name=value` word whose RHS mixes spans (`x='a'\''`b`'`)
/// must therefore keep `` `b` `` as literal data, not as a comsub opener the
/// assignment expander (expand_backtick_substitution_typed /
/// expand_mixed_command_substitution_assignment) would execute.
pub(super) fn remove_shell_quotes_assignment(raw: &str, posix: bool) -> String {
    remove_shell_quotes_inner_cursor(raw, posix)
}

fn is_lexer_shell_name_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_lexer_shell_name_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

// ---------------------------------------------------------------------------
// quoterm22: byte-cursor span-copy substrate for the hot walk.
//
// GNU anchor: parse.y:5305 read_token_word assembles the token text once,
// character by character (parse.y:3557 read_token's streaming model); GNU's
// per-character lex walk inserts CTLESC carriers but never memcpy-runs — this
// port's walk BOTH strips quotes and inserts the rubash carrier bytes
// (DATA_DOLLAR/DATA_BACKTICK/DATA_SQUOTE/DATA_BACKSLASH/CTLESC), so the
// per-character `Peekable<Chars>` state machine was the tokenize pass's
// dominant cost (feeder21: word_finish 20-25ms of a 45-50ms tokenize on
// configure, 40,163 words).
//
// The cursor family below is a SUBSTRATE-ONLY conversion of the original
// per-character `Peekable<Chars>` state machines (the main walk it replaced
// was removed after the differential run; the iterator-based subscanners it
// still shares — remove_double_quoted_into and friends, used by the rare
// outside-backticks path — remain verbatim above): every arm, guard, carrier
// choice, and state transition was transcribed arm-for-arm; only the
// iteration mechanics change — a byte
// cursor over `&str` with bulk `push_str` spans between the ASCII trigger
// bytes. Equivalence relies on three facts, each checked against the
// original code:
//   1. every trigger byte the state machines match on is ASCII, so a UTF-8
//      continuation byte (>= 0x80) is never a trigger and can be span-copied
//      verbatim;
//   2. `pending_name` (the only cross-character state in the `_` arms) can
//      only be ARMED at a `$` — itself a trigger byte — and only CONTINUED by
//      [A-Za-z0-9_] (all non-trigger ASCII), so a trigger-free span either
//      extends the pending name to its end or breaks it at its first
//      non-name byte;
//   3. subscanners that consume verbatim report their consumption by
//      returning the remaining slice; the dequoting subscanners (ANSI-C
//      span, double-quote machinery) are cursor transcriptions of their
//      originals.
// A lane-local differential run compared the cursor walk against the
// original character-machine walk over every token of the vendored GNU test
// corpus (737 files, 354,153 tokens x posix on/off), nvm.sh, and bash's own
// configure — zero mismatches; the full 83-suite true-baseline A/B (base
// binary vs lane binary, rb.out/rb.err/rb.rc all byte-identical) is the
// durable proof recorded in docs/PERF-BASELINE.md (quoterm22 round).
// ---------------------------------------------------------------------------

/// Bytes that can make any arm of the cursor walk emit something other than
/// the input verbatim (or change state): the seven de-quote bytes minus `]`,
/// which is only stateful inside an array subscript.
#[inline]
fn q22_walk_trigger(b: u8) -> bool {
    matches!(b, b'\'' | b'"' | b'\\' | b'$' | b'`' | b'[')
}

#[inline]
fn q22_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[inline]
fn q22_name_start_byte(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphabetic()
}

/// The raw->value dequote walk (GNU read_token_word quote removal,
/// parse.y:5305+): `rest` walks the raw word byte-by-byte and runs of
/// non-trigger bytes are pushed as one `&str` span.
fn remove_shell_quotes_inner_cursor(raw: &str, posix: bool) -> String {
    if !raw
        .as_bytes()
        .iter()
        .any(|&b| matches!(b, b'\'' | b'"' | b'\\' | b'$' | b'`' | b'[' | b']'))
    {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    // Array-subscript regions keep `\"` as a bare data quote (see the
    // original walk's comment); `]` only matters while a subscript is open.
    let mut subscript_depth = 0usize;
    // True while the emitted tail is an unbraced `$name` parameter.
    let mut pending_name = false;

    while !rest.is_empty() {
        let first = rest.as_bytes()[0];
        match first {
            b'[' => {
                subscript_depth += 1;
                pending_name = false;
                out.push('[');
                rest = &rest[1..];
            }
            b']' if subscript_depth > 0 => {
                subscript_depth -= 1;
                pending_name = false;
                out.push(']');
                rest = &rest[1..];
            }
            b'$' => {
                pending_name = false;
                match rest.as_bytes().get(1) {
                    Some(&b'(') => {
                        rest = copy_dollar_paren_substitution_cursor(&mut out, rest);
                    }
                    Some(&b'\'') => {
                        rest = decode_ansi_c_span_cursor(&mut out, &rest[2..]);
                    }
                    Some(&b'"') => {
                        rest = &rest[2..];
                        rest = remove_double_quoted_into_cursor(&mut out, rest, false, posix);
                    }
                    Some(&b'{') => {
                        rest = copy_braced_parameter_unquoted_cursor(&mut out, rest);
                    }
                    next => {
                        // `$` with none of the four special followers: the
                        // original falls to the `_` arm, which arms
                        // pending_name iff the next char is a name start.
                        pending_name = next.is_some_and(|&b| q22_name_start_byte(b));
                        out.push('$');
                        rest = &rest[1..];
                    }
                }
            }
            b'\'' => {
                if pending_name {
                    out.push(PARAM_NAME_END_MARKER);
                }
                pending_name = false;
                rest = &rest[1..];
                // Single-quoted span: every byte is literal data; the three
                // expansion-trigger bytes travel as carriers (original
                // `'` arm; GNU subst.c:11882-11886 never consults the
                // comsub scanner inside `'...'`).
                loop {
                    let bytes = rest.as_bytes();
                    let mut idx = 0usize;
                    while idx < bytes.len() && !matches!(bytes[idx], b'\'' | b'$' | b'`' | b'"') {
                        idx += 1;
                    }
                    out.push_str(&rest[..idx]);
                    rest = &rest[idx..];
                    match rest.as_bytes().first() {
                        Some(&b'\'') => {
                            rest = &rest[1..];
                            break;
                        }
                        Some(&b'$') => {
                            out.push(DATA_DOLLAR);
                            rest = &rest[1..];
                        }
                        Some(&b'`') => {
                            out.push(crate::executor::markers::DATA_BACKTICK);
                            rest = &rest[1..];
                        }
                        Some(&b'"') => {
                            out.push(crate::executor::markers::DATA_DQUOTE);
                            rest = &rest[1..];
                        }
                        None => break, // unterminated span: consumed to end
                        _ => unreachable!("span loop stops only at the four bytes"),
                    }
                }
            }
            b'"' => {
                if pending_name {
                    out.push(PARAM_NAME_END_MARKER);
                }
                pending_name = false;
                rest = &rest[1..];
                rest = remove_double_quoted_into_cursor(&mut out, rest, true, posix);
            }
            b'`' => {
                pending_name = false;
                out.push('`');
                rest = copy_backtick_body_cursor(&mut out, &rest[1..]);
            }
            b'\\' => {
                pending_name = false;
                let Some(&escaped) = rest.as_bytes().get(1) else {
                    // Trailing backslash: the original pushes it verbatim.
                    out.push('\\');
                    rest = &rest[1..];
                    continue;
                };
                match escaped {
                    b'$' => {
                        out.push(DATA_DOLLAR);
                        rest = &rest[2..];
                    }
                    b'`' => {
                        out.push(crate::executor::markers::DATA_BACKTICK);
                        rest = &rest[2..];
                    }
                    b'\'' => {
                        out.push(crate::executor::markers::DATA_SQUOTE);
                        rest = &rest[2..];
                    }
                    b'"' => {
                        // Both subscript branches of the original emit the
                        // same DATA_DQUOTE carrier for `\"`.
                        out.push(crate::executor::markers::DATA_DQUOTE);
                        rest = &rest[2..];
                    }
                    b'\\' => {
                        out.push(crate::executor::markers::DATA_BACKSLASH);
                        rest = &rest[2..];
                    }
                    b'*' | b'?' | b'[' | b']' | b'@' | b'+' | b'!' | b'(' | b')' | b'|' | b'/'
                    | b'-' | b'^' | b'.' | b'=' | b':' => {
                        // GNU parse.y:5694-5706 got_escaped_character marks
                        // pattern-significant escaped chars with CTLESC.
                        out.push(crate::executor::markers::CTLESC);
                        out.push(char::from(escaped));
                        rest = &rest[2..];
                    }
                    b if b < 0x80 => {
                        out.push(char::from(b));
                        rest = &rest[2..];
                    }
                    _ => {
                        // Non-ASCII escaped char: push the whole char.
                        let ch = rest[1..].chars().next().unwrap();
                        out.push(ch);
                        rest = &rest[1 + ch.len_utf8()..];
                    }
                }
            }
            _ => {
                // Trigger-free span: bulk push. `]` at subscript depth 0 is
                // data (the guarded arm above missed), so it participates in
                // the span; every other span byte is a non-trigger.
                let bytes = rest.as_bytes();
                let mut idx = 0usize;
                while idx < bytes.len() {
                    let b = bytes[idx];
                    if q22_walk_trigger(b) || (b == b']' && subscript_depth > 0) {
                        break;
                    }
                    idx += 1;
                }
                if pending_name {
                    // pending_name can only be continued by name bytes; the
                    // first non-name byte of the span breaks it silently
                    // (PARAM_NAME_END_MARKER is only emitted at quotes).
                    let name_run = bytes[..idx]
                        .iter()
                        .position(|&b| !q22_name_byte(b))
                        .unwrap_or(idx);
                    if name_run < idx {
                        pending_name = false;
                    }
                }
                out.push_str(&rest[..idx]);
                rest = &rest[idx..];
            }
        }
    }

    out
}

/// Cursor form of `copy_dollar_paren_substitution` (rest at the `$`).
fn copy_dollar_paren_substitution_cursor<'a>(out: &mut String, rest: &'a str) -> &'a str {
    out.push('$');
    let rest = &rest[1..]; // past '$'
    if !rest.starts_with('(') {
        return rest; // unreachable from the dispatcher; keeps parity
    }
    out.push('(');
    copy_dollar_paren_body_raw_cursor(out, &rest[1..])
}

/// Cursor form of `copy_dollar_paren_body_raw` (rest just past the `$(`).
/// Every consumed byte is pushed verbatim; the case-depth tracker and the
/// `rest` lookahead are transcribed unchanged (the lookahead now passes the
/// remaining slice instead of collecting a fresh String).
fn copy_dollar_paren_body_raw_cursor<'a>(out: &mut String, rest: &'a str) -> &'a str {
    let mut depth = 1usize;
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;
    let mut case_in_stage = 0u8;
    let mut case_pattern_region = false;
    // rubash#380: previous significant char fed to the case word machine
    // (the `esac` previous-token witness, parse.y:3181/3183).
    let mut prev_sig: Option<char> = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let ch = rest[idx..].chars().next().unwrap();
        out.push(ch);
        idx += ch.len_utf8();
        if ch == '\\' {
            // A backslash-quoted character is word text, never a reserved
            // word; push the same placeholder the original uses.
            if let Some(escaped) = rest[idx..].chars().next() {
                out.push(escaped);
                idx += escaped.len_utf8();
                word.push('\u{1}');
            }
            continue;
        }
        // `case WORD in' chain tracker lookahead (rubash#276/#284): the
        // machine only needs the char after `ch` (the `;;`/`;&`
        // pattern-list re-arm of parse.y:3710/3759); `idx` already points
        // past `ch`.
        let next = rest[idx..].chars().next();
        super::skip::update_command_substitution_case_depth(
            ch,
            false,
            false,
            &mut word,
            &mut case_depth,
            &mut word_boundary,
            &mut current_word_boundary,
            next,
            &mut case_in_stage,
            &mut case_pattern_region,
            prev_sig,
        );
        if crate::lexer::skip::esac_prev_token_char(ch) {
            prev_sig = Some(ch);
        }
        match ch {
            '$' if rest.as_bytes().get(idx) == Some(&b'\'') => {
                idx += 1;
                out.push('\'');
                idx += copy_ansi_c_single_quoted_raw_cursor(out, &rest[idx..]);
                word.clear();
                word_boundary = false;
            }
            '$' if rest.as_bytes().get(idx) == Some(&b'(') => {
                idx += 1;
                out.push('(');
                depth += 1;
            }
            '\'' => {
                idx += copy_single_quoted_raw_cursor(out, &rest[idx..]);
                word.clear();
                word_boundary = false;
            }
            '"' => {
                idx += copy_double_quoted_raw_cursor(out, &rest[idx..]);
                word.clear();
                word_boundary = false;
            }
            '`' => {
                let rem = copy_backtick_body_cursor(out, &rest[idx..]);
                idx = rest.len() - rem.len();
                word.clear();
                word_boundary = false;
            }
            '(' if case_depth == 0 => depth += 1,
            ')' if case_depth == 0 => {
                // Plain subtraction like the original: an unbalanced `)`
                // underflows/wraps identically (never observed on real
                // scripts; the case_depth tracker owns pattern parens).
                depth -= 1;
                if depth == 0 {
                    return &rest[idx..];
                }
            }
            _ => {}
        }
    }
    &rest[idx..]
}

/// Cursor form of `copy_single_quoted_raw`: copy through the closing `'`.
fn copy_single_quoted_raw_cursor(out: &mut String, rest: &str) -> usize {
    let bytes = rest.as_bytes();
    match bytes.iter().position(|&b| b == b'\'') {
        Some(pos) => {
            out.push_str(&rest[..=pos]);
            pos + 1
        }
        None => {
            out.push_str(rest);
            rest.len()
        }
    }
}

/// Cursor form of `copy_ansi_c_single_quoted_raw`: `\\` pairs are copied
/// through, an unescaped `'` closes.
fn copy_ansi_c_single_quoted_raw_cursor(out: &mut String, rest: &str) -> usize {
    let bytes = rest.as_bytes();
    let mut idx = 0usize;
    let mut escaped = false;
    while idx < bytes.len() {
        let b = bytes[idx];
        if escaped {
            escaped = false;
            idx += 1;
            continue;
        }
        if b == b'\\' {
            escaped = true;
            idx += 1;
            continue;
        }
        if b == b'\'' {
            idx += 1;
            break;
        }
        idx += 1;
    }
    out.push_str(&rest[..idx]);
    idx
}

/// Cursor form of `copy_double_quoted_raw`: copy through the closing `"`,
/// honoring `\\` pairs and nested `$(...)` bodies verbatim.
fn copy_double_quoted_raw_cursor(out: &mut String, rest: &str) -> usize {
    let mut idx = 0usize;
    while idx < rest.len() {
        let ch = rest[idx..].chars().next().unwrap();
        out.push(ch);
        idx += ch.len_utf8();
        match ch {
            '"' => return idx,
            '\\' => {
                if let Some(escaped) = rest[idx..].chars().next() {
                    out.push(escaped);
                    idx += escaped.len_utf8();
                }
            }
            '$' if rest.as_bytes().get(idx) == Some(&b'(') => {
                idx += 1;
                out.push('(');
                let consumed = copy_dollar_paren_body_raw_cursor(out, &rest[idx..]);
                idx = rest.len() - consumed.len();
            }
            _ => {}
        }
    }
    idx
}

/// Cursor form of `copy_backtick_body_preserving_syntax` (rest just past the
/// opening backtick). Returns the remaining slice; the closing backtick is
/// pushed. `\`+newline (and `\`+CRLF) inside the body is elided — the one
/// place consumption is NOT verbatim, which is why this function returns the
/// remainder instead of letting callers measure `out`.
fn copy_backtick_body_cursor<'a>(out: &mut String, rest: &'a str) -> &'a str {
    let bytes = rest.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        match bytes[idx] {
            b'`' => {
                out.push('`');
                return &rest[idx + 1..];
            }
            b'\\' => match bytes.get(idx + 1) {
                Some(&b'\n') => {
                    idx += 2;
                }
                Some(&b'\r') if bytes.get(idx + 2) == Some(&b'\n') => {
                    idx += 3;
                }
                _ => {
                    // Backslash + following char (possibly multi-byte),
                    // pushed verbatim like the original.
                    let escaped = rest[idx + 1..].chars().next();
                    match escaped {
                        Some(ch) => {
                            out.push('\\');
                            out.push(ch);
                            idx += 1 + ch.len_utf8();
                        }
                        None => {
                            out.push('\\');
                            idx += 1;
                        }
                    }
                }
            },
            _ => {
                let start = idx;
                while idx < bytes.len() && bytes[idx] != b'`' && bytes[idx] != b'\\' {
                    idx += 1;
                }
                out.push_str(&rest[start..idx]);
            }
        }
    }
    &rest[idx..]
}

/// Cursor form of `decode_ansi_c_span` (rest just past `$'`): collect the
/// escape-honoring span, decode it, push the carrier-escaped decode, and
/// return the remainder after the closing `'`.
fn decode_ansi_c_span_cursor<'a>(out: &mut String, rest: &'a str) -> &'a str {
    let bytes = rest.as_bytes();
    let mut quoted = String::new();
    let mut escaped = false;
    let mut idx = 0usize;
    while idx < bytes.len() {
        let b = bytes[idx];
        if escaped {
            escaped = false;
            quoted.push('\\');
            if b < 0x80 {
                quoted.push(char::from(b));
                idx += 1;
            } else {
                let ch = rest[idx..].chars().next().unwrap();
                quoted.push(ch);
                idx += ch.len_utf8();
            }
            continue;
        }
        if b == b'\\' {
            escaped = true;
            idx += 1;
            continue;
        }
        if b == b'\'' {
            idx += 1;
            break;
        }
        if b < 0x80 {
            quoted.push(char::from(b));
            idx += 1;
        } else {
            let ch = rest[idx..].chars().next().unwrap();
            quoted.push(ch);
            idx += ch.len_utf8();
        }
    }
    if escaped {
        quoted.push('\\');
    }
    out.push_str(&escape_decoded_ansi_c_quotes(&decode_ansi_c_quoted(
        &quoted,
    )));
    &rest[idx..]
}

/// Cursor form of `copy_braced_parameter_unquoted` (rest at the `$`).
fn copy_braced_parameter_unquoted_cursor<'a>(out: &mut String, rest: &'a str) -> &'a str {
    copy_braced_parameter_inner_cursor(out, rest, false, false)
}

/// Cursor form of `copy_braced_parameter_after_dollar` (rest at the `$`).
fn copy_braced_parameter_after_dollar_cursor<'a>(
    out: &mut String,
    rest: &'a str,
    posix: bool,
) -> &'a str {
    copy_braced_parameter_inner_cursor(out, rest, true, posix)
}

/// Cursor form of `copy_braced_parameter_inner` (rest at the `$`). The
/// `${...}` unit is copied verbatim; only the consumption accounting changes
/// (bytes consumed instead of chars counted).
fn copy_braced_parameter_inner_cursor<'a>(
    out: &mut String,
    rest: &'a str,
    outer_double_quote: bool,
    posix: bool,
) -> &'a str {
    out.push('$');
    let rest = &rest[1..]; // past '$'
    if !rest.starts_with('{') {
        return rest; // unreachable from the dispatchers; keeps parity
    }
    let remaining = &rest[1..]; // past '{'
    if braced_body_contains_ansi_c(remaining) {
        return copy_braced_parameter_with_ansi_c_cursor(out, rest);
    }
    let mut wrapped = String::with_capacity(rest.len() + 1);
    wrapped.push('$');
    wrapped.push_str(rest);
    let context = BraceContext {
        outer_double_quote,
        posix,
        replacement_context: false,
        initial_state: DolbraceState::Param,
    };
    if let Some(scan) = scan_braced_parameter(&wrapped, context) {
        // Original counts consumed CHARS of `wrapped[..scan.end]` minus the
        // synthetic '$' and re-pushes them; the same bytes are `rest[..take]`.
        let take = scan.end.saturating_sub(1).min(rest.len());
        out.push_str(&rest[..take]);
        return &rest[take..];
    }
    // Fallback: byte walk with nested `${` tracking, pushing verbatim.
    out.push('{');
    let bytes = rest.as_bytes();
    let mut idx = 1usize; // past '{'
    let mut depth = 1usize;
    while idx < bytes.len() {
        let ch = rest[idx..].chars().next().unwrap();
        out.push(ch);
        idx += ch.len_utf8();
        if ch == '$' && bytes.get(idx) == Some(&b'{') {
            out.push('{');
            idx += 1;
            depth += 1;
            continue;
        }
        if ch == '}' {
            // Plain subtraction like the original fallback loop.
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
    }
    &rest[idx..]
}

/// Cursor form of `copy_braced_parameter_with_ansi_c` (rest at the `{`).
fn copy_braced_parameter_with_ansi_c_cursor<'a>(out: &mut String, rest: &'a str) -> &'a str {
    out.push('{');
    let bytes = rest.as_bytes();
    let mut idx = 1usize; // past '{'
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    while idx < bytes.len() {
        let ch = rest[idx..].chars().next().unwrap();
        out.push(ch);
        idx += ch.len_utf8();
        // $'...' ANSI-C quoting: consume to the closing ' (honoring \').
        if ch == '$' && bytes.get(idx) == Some(&b'\'') && !single && !double {
            out.push('\'');
            idx += 1;
            let mut escaped = false;
            while idx < bytes.len() {
                let sub = rest[idx..].chars().next().unwrap();
                out.push(sub);
                idx += sub.len_utf8();
                if escaped {
                    escaped = false;
                    continue;
                }
                if sub == '\\' {
                    escaped = true;
                    continue;
                }
                if sub == '\'' {
                    break;
                }
            }
            continue;
        }
        // Backslash escaping (not in single quotes)
        if ch == '\\' && !single {
            if let Some(escaped) = rest[idx..].chars().next() {
                out.push(escaped);
                idx += escaped.len_utf8();
            }
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            continue;
        }
        if ch == '$' && bytes.get(idx) == Some(&b'{') && !single && !double {
            out.push('{');
            idx += 1;
            depth += 1;
            continue;
        }
        if ch == '}' && !single && !double {
            // Plain subtraction like the original.
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
    }
    &rest[idx..]
}

/// Cursor form of `remove_double_quoted_into` (rest just inside the opening
/// `"`; returns the remainder just past the closing `"` or, if unterminated,
/// an empty slice). Trigger bytes inside double quotes: `"`, `$`, `\`, `'`,
/// the glob set `* ? [ @ + !`, and `` ` `` when backtick bodies must be
/// preserved.
fn remove_double_quoted_into_cursor<'a>(
    out: &mut String,
    rest: &'a str,
    preserve_backticks: bool,
    posix: bool,
) -> &'a str {
    let mut rest = rest;
    let mut pending_name = false;
    while !rest.is_empty() {
        let first = rest.as_bytes()[0];
        if first == b'$' {
            match rest.as_bytes().get(1) {
                Some(&b'(') => {
                    pending_name = false;
                    rest = copy_dollar_paren_substitution_cursor(&mut *out, rest);
                    continue;
                }
                Some(&b'{') => {
                    pending_name = false;
                    rest = copy_braced_parameter_after_dollar_cursor(&mut *out, rest, posix);
                    continue;
                }
                Some(&b @ (b'?' | b'$' | b'!' | b'#' | b'-' | b'@' | b'*' | b'0'..=b'9')) => {
                    pending_name = false;
                    out.push('$');
                    out.push(char::from(b));
                    rest = &rest[2..];
                    continue;
                }
                _ => {}
            }
            // `$` with a name-start follower arms pending_name and pushes the
            // `$` raw; any other follower is a literal dollar that travels as
            // DATA_DOLLAR (GNU parse.y skip_double_quoted: `$` not starting
            // an expansion is literal).
            if rest
                .as_bytes()
                .get(1)
                .is_some_and(|&b| q22_name_start_byte(b))
            {
                pending_name = true;
                out.push('$');
                rest = &rest[1..];
            } else {
                pending_name = false;
                out.push(DATA_DOLLAR);
                rest = &rest[1..];
            }
            continue;
        }
        match first {
            b'"' => {
                if pending_name {
                    out.push(PARAM_NAME_END_MARKER);
                }
                return &rest[1..];
            }
            b'`' if preserve_backticks => {
                pending_name = false;
                out.push('`');
                rest = copy_backtick_body_cursor(&mut *out, &rest[1..]);
            }
            b'\\' => {
                pending_name = false;
                match rest.as_bytes().get(1) {
                    Some(&b @ (b'\\' | b'"' | b'$' | b'`' | b'\n')) => {
                        rest = &rest[2..];
                        if b != b'\n' {
                            match b {
                                b'$' => out.push(DATA_DOLLAR),
                                b'`' => out.push(crate::executor::markers::DATA_BACKTICK),
                                b'\\' => out.push(crate::executor::markers::DATA_BACKSLASH),
                                // De-escaped `"` travels as the data-double-
                                // quote marker (see the original's comment).
                                _ => out.push(crate::executor::markers::DATA_DQUOTE),
                            }
                        }
                    }
                    _ => {
                        out.push('\\');
                        rest = &rest[1..];
                    }
                }
            }
            b'\'' => {
                pending_name = false;
                out.push(crate::executor::markers::DATA_SQUOTE);
                rest = &rest[1..];
            }
            b @ (b'*' | b'?' | b'[' | b'@' | b'+' | b'!') => {
                pending_name = false;
                out.push(crate::executor::markers::CTLESC);
                out.push(char::from(b));
                rest = &rest[1..];
            }
            _ => {
                // Trigger-free span inside the quotes. `pending_name` can
                // only be armed at the `$` handled above; a span continues it
                // through name bytes and breaks it at the first other byte.
                let bytes = rest.as_bytes();
                let mut idx = 0usize;
                while idx < bytes.len() {
                    let b = bytes[idx];
                    if matches!(
                        b,
                        b'"' | b'$' | b'\\' | b'\'' | b'*' | b'?' | b'[' | b'@' | b'+' | b'!'
                    ) || (preserve_backticks && b == b'`')
                    {
                        break;
                    }
                    idx += 1;
                }
                if pending_name {
                    let name_run = bytes[..idx]
                        .iter()
                        .position(|&b| !q22_name_byte(b))
                        .unwrap_or(idx);
                    if name_run < idx {
                        pending_name = false;
                    }
                }
                out.push_str(&rest[..idx]);
                rest = &rest[idx..];
            }
        }
    }
    rest
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
            '$' if chars.peek() == Some(&'\'') => {
                pending_name = false;
                chars.next();
                // GNU parse.y:5546-5558 read_token_word: `$'` hands the span
                // to parse_matched_pair (parse.y:3877) with P_ALLOWESC, so
                // `\'` does not close the string and a backtick inside the
                // span is an ordinary character — never a command-substitution
                // opener. Without this arm the `'` after `$` fell into the
                // plain single-quote case below, which does not honor the
                // escape: `\'` closed the span early, the surviving raw
                // backtick then tripped the executor's unclosed-`$(` gate
                // ("unexpected EOF while looking for matching `)' —
                // rubash#215) and, without a backtick, left a live `$` that
                // the assignment expander read as a parameter name
                // (`x=$'a`b'` stored `b`).
                out.push_str(&decode_ansi_c_span(&mut chars));
            }
            '\'' => {
                if pending_name {
                    out.push(PARAM_NAME_END_MARKER);
                }
                pending_name = false;
                // GNU parse.y:5366-5398 read_token_word: `\'` consumes the
                // escaped quote without opening quote state, so the next `'`
                // starts a fresh single-quoted span (parse.y:5419-5437), and
                // inside that span subst.c:11882-11886 never consults the
                // command-substitution scanner — every byte is quoted data.
                // Since this function already strips the `'` delimiters, the
                // expansion-trigger bytes must travel as the walker's data
                // carriers (the protect_fully_single_quoted_assignment
                // convention, classification.rs) or the assignment expander
                // re-reads `x='a'\''`b`'` as `a'` plus a live `` `b` ``
                // substitution and executes `b` (rubash#144).
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    match quoted {
                        '$' => out.push(DATA_DOLLAR),
                        '`' => out.push(crate::executor::markers::DATA_BACKTICK),
                        '"' => out.push(crate::executor::markers::DATA_DQUOTE),
                        _ => out.push(quoted),
                    }
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
                    out.push(crate::executor::markers::DATA_SQUOTE);
                } else if escaped == '`' {
                    out.push(crate::executor::markers::DATA_BACKTICK);
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
                            '$' => out.push(DATA_DOLLAR),
                            '`' => out.push(crate::executor::markers::DATA_BACKTICK),
                            '\\' => out.push(crate::executor::markers::DATA_BACKSLASH),
                            // A de-escaped `"` must travel as the walker's
                            // data-double-quote marker (same as `\"` outside
                            // quotes): downstream expansion scanners toggle
                            // quote state on a bare quote and would swallow
                            // it (`echo "a\"b"` printed `ab`, bash prints
                            // `a"b`).
                            '"' => out.push(crate::executor::markers::DATA_DQUOTE),
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
                out.push(crate::executor::markers::DATA_SQUOTE);
            }
            _ if matches!(quoted, '*' | '?' | '[' | '@' | '+' | '!') => {
                pending_name = false;
                out.push(crate::executor::markers::CTLESC);
                out.push(quoted);
            }
            _ => {
                if quoted == '$' {
                    if chars
                        .peek()
                        .is_some_and(|next| is_lexer_shell_name_start(*next))
                    {
                        pending_name = true;
                    } else {
                        // GNU parse.y skip_double_quoted: `$` not followed
                        // by a valid expansion start is a literal dollar
                        // (e.g. `"hello, $"world""` → `hello, $world`).
                        // Mark it as protected so the expansion walker
                        // does not re-interpret the following text as a
                        // variable reference.
                        pending_name = false;
                        out.push(DATA_DOLLAR);
                        continue;
                    }
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

/// Gather and decode one `$'...'` ANSI-C span. `chars` is positioned just
/// after the opening `'`; the closing `'` is consumed. GNU
/// parse.y:5546-5558 read_token_word → parse_matched_pair (parse.y:3877)
/// with P_ALLOWESC: a `\` escapes the next byte (so `\'` never closes the
/// string) and everything inside — including backticks, which the grouping
/// arms of parse_matched_pair only nest when `open != close` — is string
/// data, never substitution syntax. The decoded bytes leave here through
/// `escape_decoded_ansi_c_quotes`, i.e. tagged with the walker's data
/// carriers so no later pass re-reads a quote, dollar, or backtick in the
/// value as live syntax (the CTLESC equivalent of GNU's ansiexpand +
/// sh_single_quote re-encoding at parse.y:5560-5575).
pub(super) fn decode_ansi_c_span(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
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
    // GNU subst.c CTLESC-quotes every byte an ANSI-C string decodes to, so
    // quote characters in the decoded value are data for every later pass
    // (posixexp7: the escaped-quote ANSI-C word kept its literal quote;
    // unescaped, the expansion walker consumed the bare quote as a
    // delimiter). Rubash's walker expresses that with backslash escapes, so
    // escape the decode output with that convention, keeping
    // backslash-prefixed runs verbatim.
    escape_decoded_ansi_c_quotes(&decode_ansi_c_quoted(&quoted))
}

/// Mark quote characters in a decoded ANSI-C string with the walker's
/// data-quote markers so later expansion passes treat them as data (the
/// CTLESC equivalent; see the call site). The markers are the same ones the
/// lexer emits for backslash-escaped quotes in source words, so every
/// consumer already restores them. A decoded backtick additionally travels
/// as DATA_BACKTICK — the same carrier the single-quote arm uses — so
/// re-scans of assignment shell text (the executor's unclosed-`$(` gate,
/// has_unclosed_command_substitution) cannot mistake it for a live
/// command-substitution opener (rubash#215: `x=$'a`b$(echo z)'`).
pub(crate) fn escape_decoded_ansi_c_quotes(decoded: &str) -> String {
    let chars: Vec<char> = decoded.chars().collect();
    let mut out = String::with_capacity(decoded.len());
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        match ch {
            '\'' => out.push_str(ANSI_C_QUOTE_MARKER_STR),
            '"' => out.push_str(ANSI_C_DQUOTE_MARKER_STR),
            '$' => out.push_str(DATA_DOLLAR_STR),
            '`' => out.push_str(crate::executor::markers::DATA_BACKTICK_STR),
            // The same CTLESC-every-decoded-byte invariant (parse.y:5560-
            // 5575) makes a decoded blank LITERAL WORD DATA, never a field-
            // split separator (rubash#379): `A$((1+1))B$'\n'C$((2+2))D` is
            // one argument in GNU because read_token_word hands the decoded
            // bytes to the word as quoted characters and subst.c field
            // splitting only ever runs on EXPANSION RESULTS, never on
            // source-word bytes. The guard is a PUA codepoint outside
            // every dynamic marker range (see markers.rs) rather than the
            // C0 IFS_GLUE, so the comsub payload protection (which encodes
            // every C0 byte of `$(`/backtick-bearing words) cannot capture
            // it mid-transport; field-split call sites convert it to
            // IFS_GLUE and boundary strips drop it.
            ' ' | '\t' | '\n' => {
                out.push_str(crate::executor::markers::ANSI_C_IFS_GUARD_STR);
                out.push(ch);
            }
            // Decoded glob metacharacters are quoted data too: pathexp.c:57
            // unquoted_glob_pattern_p never sees them (CTLESC-protected in
            // GNU), so `echo $'a*b'` prints the literal text even when a
            // match exists. CTLESC (x11) is the glob engine's own
            // skip-pair carrier (glob.rs contains_glob_or_extglob), the
            // assignment boundary's dequote_ctlesc_pairs removes the pair
            // before storage, and materialize strips it at argv.
            '*' | '?' | '[' => {
                out.push_str(crate::executor::markers::CTLESC_STR);
                out.push(ch);
            }
            // Extglob introducers as decoded data must not open a group.
            '@' | '+' | '!' if chars.get(index + 1) == Some(&'(') => {
                out.push_str(crate::executor::markers::CTLESC_STR);
                out.push(ch);
            }
            _ => out.push(ch),
        }
        index += 1;
    }
    out
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
    // GNU parse.y:4451 parse_comsub parses the `$(...)` body with the real
    // grammar (yyparse), so a case clause inside the body owns its pattern
    // `)` — the substitution closes at the LAST `)`, not the pattern's
    // (parse.y:3465-3468 special_case_tokens: `)` in a case pattern is the
    // pattern-list delimiter, never the comsub eof token). Mirror the same
    // word-boundary case-depth tracking skip_cmd_subst uses (skip.rs
    // update_command_substitution_case_depth) so the verbatim copy spans the
    // whole `case ... esac` (rubash#276: `w=$(case z in z) printf 'z\n' ;;
    // esac)` cooked its value as `... printf z\n ;; esac)` — the body was
    // cut at the pattern `)` and the following `'z\n'` went through ordinary
    // quote removal, so the re-parse swallowed the backslash and stored
    // `zn` where GNU stores `z`).
    let mut depth = 1usize;
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;
    // `case WORD in' chain tracker (GNU special_case_tokens,
    // parse.y:3369-3386 + 3433-3441) — see skip.rs
    // update_command_substitution_case_depth (rubash#284).
    let mut case_in_stage = 0u8;
    let mut case_pattern_region = false;
    // rubash#380: previous significant char fed to the case word machine
    // (the `esac` previous-token witness, parse.y:3181/3183).
    let mut prev_sig: Option<char> = None;
    while let Some(ch) = chars.next() {
        out.push(ch);
        if ch == '\\' {
            // A backslash-quoted character is word text, never a reserved
            // word (`c\ase` is not `case`); push the same placeholder
            // skip_cmd_subst uses so the tracker does not see the raw bytes.
            if let Some(escaped) = chars.next() {
                out.push(escaped);
                word.push('\u{1}');
            }
            continue;
        }
        // The machine only needs the char after `ch` (the `;;`/`;&`
        // pattern-list re-arm of parse.y:3710/3759); the peekable cursor
        // already points at it.
        let next = chars.peek().copied();
        super::skip::update_command_substitution_case_depth(
            ch,
            false,
            false,
            &mut word,
            &mut case_depth,
            &mut word_boundary,
            &mut current_word_boundary,
            next,
            &mut case_in_stage,
            &mut case_pattern_region,
            prev_sig,
        );
        if crate::lexer::skip::esac_prev_token_char(ch) {
            prev_sig = Some(ch);
        }
        match ch {
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                out.push('\'');
                copy_ansi_c_single_quoted_raw(out, chars);
                word.clear();
                word_boundary = false;
            }
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                out.push('(');
                depth += 1;
            }
            '\'' => {
                copy_single_quoted_raw(out, chars);
                word.clear();
                word_boundary = false;
            }
            '"' => {
                copy_double_quoted_raw(out, chars);
                word.clear();
                word_boundary = false;
            }
            '`' => {
                copy_backtick_raw(out, chars);
                word.clear();
                word_boundary = false;
            }
            '(' if case_depth == 0 => depth += 1,
            ')' if case_depth == 0 => {
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
        assert_eq!(
            remove_shell_quotes("\"a:'b' c\""),
            format!(
                "{}{}{}{}{}",
                "a:",
                crate::executor::markers::DATA_SQUOTE_STR,
                "b",
                crate::executor::markers::DATA_SQUOTE_STR,
                " c"
            )
        );
    }
}

// Source-mapped to subst.c::extract_dollar_brace_string: quote removal
// receives explicit outer-quote and POSIX context instead of conflating them.
pub(super) fn copy_braced_parameter_after_dollar(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    posix: bool,
) {
    copy_braced_parameter_inner(out, chars, true, posix);
}

// Unquoted `${...}` units keep their body verbatim (GNU quote removal never
// strips quotes inside a parameter expansion; the expansion stage owns the
// quote state). Whole-word `${...}` tokens already bypass quote removal via
// the lexer's Variable path; this gives embedded occurrences the same shape.
fn copy_braced_parameter_unquoted(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    copy_braced_parameter_inner(out, chars, false, false);
}

/// Copy a `${...}` braced parameter verbatim for the expansion stage.
///
/// Normally `scan_braced_parameter` (dolbrace.rs) determines the span, but it
/// does not recognize `$'...'` ANSI-C quoting inside `${...}`: it treats `'`
/// as a regular single quote, so `\'` inside `$'\x5c\''` corrupts the brace
/// scan and extends the parameter past its real closing `}` (nquote2.sub
/// `${v/x/$'\x5c\''}`).  When the body contains `$'`, fall back to a local
/// scan that handles ANSI-C quoting with the same `\'` escape tracking.
fn copy_braced_parameter_inner(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    outer_double_quote: bool,
    posix: bool,
) {
    out.push('$');
    if chars.peek() != Some(&'{') {
        return;
    }
    let remaining: String = chars.clone().collect();
    // If the braced parameter body contains `$'` (ANSI-C quoting),
    // scan_braced_parameter would mishandle it: it treats `'` as a regular
    // single quote, so `\'` inside `$'\x5c\''` corrupts the brace scan and
    // extends the parameter past its real closing `}` (nquote2.sub).  Use a
    // local scan that tracks `\'` escapes inside `$'...'` instead.  Only
    // check within the brace body (up to the first `}` at depth 0) so `$'`
    // in later script text does not trigger the fallback.
    if braced_body_contains_ansi_c(&remaining) {
        copy_braced_parameter_with_ansi_c(out, chars, outer_double_quote, posix);
        return;
    }
    let mut wrapped = String::from("$");
    wrapped.push_str(&remaining);
    let context = BraceContext {
        outer_double_quote,
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
    // Fallback: copy character by character, tracking ${...} nesting.
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

/// Check whether the body of a `${...}` braced parameter contains `$'`
/// (ANSI-C quoting) before its real closing `}`.  Only scans up to the
/// first `}` at depth 0 so `$'` in later script text does not trigger the
/// fallback.
fn braced_body_contains_ansi_c(remaining: &str) -> bool {
    let chars: Vec<char> = remaining.chars().collect();
    let mut index = 0usize;
    if chars.first() != Some(&'{') {
        return false;
    }
    index += 1;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '$' && index + 1 < chars.len() && chars[index + 1] == '\'' && !single && !double {
            return true;
        }
        if ch == '\\' && !single && index + 1 < chars.len() {
            index += 2;
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
        if ch == '$' && index + 1 < chars.len() && chars[index + 1] == '{' && !single && !double {
            depth += 1;
            index += 2;
            continue;
        }
        if ch == '}' && !single && !double {
            depth -= 1;
            if depth == 0 {
                return false;
            }
            index += 1;
            continue;
        }
        index += 1;
    }
    false
}

/// Local `${...}` scanner that handles `$'...'` ANSI-C quoting inside the
/// body.  Tracks `\'` escapes so the closing `'` of the ANSI-C string is not
/// mistaken for a single-quote toggle, and the real closing `}` of the
/// braced parameter is found (nquote2.sub).
fn copy_braced_parameter_with_ansi_c(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    _outer_double_quote: bool,
    _posix: bool,
) {
    out.push(chars.next().unwrap()); // '{'
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    while let Some(ch) = chars.next() {
        out.push(ch);
        // $'...' ANSI-C quoting: consume to the closing ' (handling \' escapes)
        if ch == '$' && chars.peek() == Some(&'\'') && !single && !double {
            chars.next();
            out.push('\'');
            let mut escaped = false;
            for quoted_ch in chars.by_ref() {
                out.push(quoted_ch);
                if escaped {
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
            }
            continue;
        }
        // Backslash escaping (not in single quotes)
        if ch == '\\' && !single {
            if let Some(escaped) = chars.next() {
                out.push(escaped);
            }
            continue;
        }
        // Single quote handling
        if ch == '\'' && !double {
            single = !single;
            continue;
        }
        // Double quote handling
        if ch == '"' && !single {
            double = !double;
            continue;
        }
        // Nested ${...}
        if ch == '$' && chars.peek() == Some(&'{') && !single && !double {
            chars.next();
            out.push('{');
            depth += 1;
            continue;
        }
        // Closing brace
        if ch == '}' && !single && !double {
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
    }
}

/// True when `raw` contains a glob metacharacter (`*`, `?`, `[`, `]`)
/// OUTSIDE every quoted region and outside `${...}` / `$(...)` / backtick
/// units. Such a word is provably not fully quoted and must keep pathname
/// expansion eligible: `"$DIR"/*"${empty}"` has the bare `*` between the
/// quoted segments (rubash#316 residue; GNU subst.c expand_word_internal
/// globs by per-character quote flags, never by first/last characters).
///
/// Conservative by construction: anything inside `${...}` bodies is
/// operator/pattern text of the expansion itself (`${x#*}`'s `*` is not a
/// pathname glob), and the body end is found by brace nesting alone — when
/// that scan runs off the end the remainder is treated as body text, so
/// the worst case keeps the previous (sentinel) behavior rather than
/// changing it. `$(...)` output and backtick output are never re-globbed
/// by the parent word either, so their bodies are opaque here too.
pub(crate) fn raw_word_has_unquoted_glob_char(raw: &str) -> bool {
    let chars: Vec<char> = raw.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        match ch {
            '\\' => index += 2,
            '\'' => {
                let mut scan = index + 1;
                while scan < chars.len() && chars[scan] != '\'' {
                    scan += 1;
                }
                index = scan + 1;
            }
            '"' => {
                let mut scan = index + 1;
                while scan < chars.len() {
                    match chars[scan] {
                        '\\' => scan += 2,
                        '$' if matches!(chars.get(scan + 1), Some('(')) => {
                            scan = skip_glob_opaque_unit(&chars, scan + 2, ')');
                        }
                        '$' if matches!(chars.get(scan + 1), Some('{')) => {
                            scan = skip_glob_braced_body(&chars, scan);
                        }
                        '`' => {
                            scan = skip_glob_opaque_unit(&chars, scan + 1, '`');
                        }
                        '"' => break,
                        _ => scan += 1,
                    }
                }
                index = scan + 1;
            }
            '$' if matches!(chars.get(index + 1), Some('\'')) => {
                // ANSI-C `$'...'`: escape-aware, no glob semantics.
                let mut scan = index + 2;
                while scan < chars.len() {
                    if chars[scan] == '\\' {
                        scan += 2;
                        continue;
                    }
                    if chars[scan] == '\'' {
                        break;
                    }
                    scan += 1;
                }
                index = scan + 1;
            }
            '$' if matches!(chars.get(index + 1), Some('(')) => {
                index = skip_glob_opaque_unit(&chars, index + 2, ')');
            }
            '$' if matches!(chars.get(index + 1), Some('{')) => {
                index = skip_glob_braced_body(&chars, index);
            }
            '`' => {
                index = skip_glob_opaque_unit(&chars, index + 1, '`');
            }
            '*' | '?' | '[' | ']' => return true,
            _ => index += 1,
        }
    }
    false
}

/// Skip a `$(...)` or backtick unit starting at `open` (just past the
/// opener); returns the index just past its closer, or the slice end when
/// the unit never closes (conservative: treat the rest as opaque).
fn skip_glob_opaque_unit(chars: &[char], open: usize, closer: char) -> usize {
    let mut index = open;
    let mut depth = 1usize;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 2,
            '\'' => {
                index += 1;
                while index < chars.len() && chars[index] != '\'' {
                    index += 1;
                }
            }
            '"' => {
                index += 1;
                while index < chars.len() {
                    if chars[index] == '\\' {
                        index += 2;
                        continue;
                    }
                    if chars[index] == '"' {
                        break;
                    }
                    index += 1;
                }
            }
            '$' if matches!(chars.get(index + 1), Some('(')) => {
                depth += 1;
                index += 2;
            }
            c if c == closer => {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    chars.len()
}

/// Skip a `${...}` unit starting at the `$` (`open`): brace bodies are
/// opaque (operator patterns, not pathname globs). The body end is found
/// by `${`/`}` nesting alone; when it never closes, the remainder counts
/// as body (conservative).
fn skip_glob_braced_body(chars: &[char], open: usize) -> usize {
    let mut index = open + 2;
    let mut depth = 1usize;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 2,
            '$' if matches!(chars.get(index + 1), Some('{')) => {
                depth += 1;
                index += 2;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    chars.len()
}
