use super::control_flow::compound_command_has_redirects;
use crate::executor::*;

fn case_pattern_has_extglob(pattern: &str) -> bool {
    // Zero-alloc scan (no `Vec<char>` staging): extglob operators and the
    // backslash escape are single-byte ASCII, so a byte scan is equivalent
    // to the char scan — multibyte continuation bytes never equal them.
    let bytes = pattern.as_bytes();
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if matches!(bytes[index], b'@' | b'*' | b'+' | b'?' | b'!') && bytes[index + 1] == b'(' {
            return true;
        }
        index += 1;
    }
    false
}

/// Static-literal gate for case arms (perf #662): GNU execute_cmd.c:3686
/// re-expands each arm per match attempt, but a pattern whose stored text
/// and raw source carry no expansion trigger expands to itself. Whitelist
/// (default-deny): plain chars only — `$`, backquote, quotes, `\`, plus
/// `~{<>}` (tilde/brace/process-subst act in expand_word_mut), controls,
/// and the markers.rs PUA carriers. Any doubt falls to the slow path.
fn case_arm_pattern_is_static_literal(text: &str, raw: &str) -> bool {
    fn plain(word: &str) -> bool {
        word.chars().all(|c| {
            !matches!(
                c,
                '$' | '`' | '\'' | '"' | '\\' | '~' | '{' | '}' | '<' | '>'
            ) && !c.is_control()
                && !('\u{E000}'..='\u{F8FF}').contains(&c)
        })
    }
    plain(text) && plain(raw)
}

fn case_pattern_raw_has_quotes(raw: &str) -> bool {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        match chars[index] {
            '\'' | '"' => return true,
            '$' if matches!(chars.get(index + 1), Some('\'' | '"')) => return true,
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    false
}

/// Twin of `case_pattern_raw_has_quotes` that ignores quotes inside
/// `<(...)` / `>(...)` bodies (rubash#2764 F3): a quote inside a
/// process-substitution body belongs to the child command, not to
/// the pattern — it disappears with the `<(...)`→`/dev/fd/N` splice,
/// so it must not decide whether the pattern is quoted. Bodies are
/// skipped with the lexer's parenthesized-unit walker (the same one
/// the unquoted branch below uses for `$(...)` units); an
/// unterminated body falls back to char-by-char judging, i.e. the
/// old behavior for that span. Only the bodies the materializer
/// splices are skipped — quotes anywhere else, including inside
/// `$(...)`, are judged exactly as before, so every pattern without
/// a body-internal quote keeps its old branch. A `<(`/`>(` directly
/// preceded by the same operator char (`<<(`/`>>(`) is a heredoc /
/// append redirect, not a substitution, and is not skipped.
fn case_pattern_raw_has_outside_quotes(raw: &str) -> bool {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        if (chars[index] == '<' || chars[index] == '>')
            && chars.get(index + 1) == Some(&'(')
            && index
                .checked_sub(1)
                .map_or(true, |prev| chars[prev] != chars[index])
        {
            if let Some(close) = crate::lexer::skip_parenthesized_unit_corrected(&chars, index + 1)
            {
                index = close.min(chars.len());
                continue;
            }
            // Unterminated body: fall through and judge char by char
            // (the old behavior for this span).
        }
        match chars[index] {
            '\'' | '"' => return true,
            '$' if matches!(chars.get(index + 1), Some('\'' | '"')) => return true,
            '\\' => index += 1,
            _ => {}
        }
        index += 1;
    }
    false
}

/// Word-position twin of `quote_aware_case_pattern` (rubash#153).
///
/// GNU execute_cmd.c:3695-3699 expands the case word with
/// expand_word_leave_quoted and then dequote_string: single-quoted spans
/// are literal data — subst.c:11882-11886 hands the whole '...' body to
/// add_quoted_string without ever consulting the command-substitution
/// scanner — and only unquoted segments reach the full expansion
/// pipeline. Double-quoted spans expand parameters/substitutions but keep
/// a `'` inside literal (parse.y read_token_word's double-quote scanner
/// never opens single-quote state). Unlike the pattern twin this applies
/// no glob-literal escaping: the case word is the *subject* of
/// strmatch(), not a pattern (execute_cmd.c:3727-3730).
/// Expand one cooked double-quoted case-word segment (rubash#3084).
///
/// The walker above consumed the segment's quotes, so the bare cooked
/// fragment has lost its double-quote provenance: the assignment
/// pre-scan (apply_parameter_assignment_expansions_in_word) derives
/// each `${...}` fragment's quote context from the word text itself
/// and reads the fragment as UNQUOTED — so `"${v=a\ b}"` assigned
/// `a b`, eating a backslash GNU keeps (parse.y read_token_word's
/// double-quote scanner makes `\` an escape only before `$`, backtick,
/// `"`, `\` or newline, and subst.c:7663 parameter_brace_expand_word
/// applies that rule inside the `${...}` word too). Restore the
/// provenance with the lexer's own spelling for a wholly-quoted word
/// carrying a `${...}` (lexer/word.rs: STORAGE_WORD_PREFIX): the
/// pre-scan then seeds its quote-state walk inside double quotes and
/// the render takes the DoubleQuoted parameter-word path, exactly as
/// for a quoted word that never went through this walker. Fragments
/// without a `${` have no quote-context-sensitive consumer and keep
/// the plain cooked path (the lexer's own admission rule for the
/// marker is likewise `${`-carrying words only).
fn expand_case_dq_segment(expand_word: &mut impl FnMut(&str) -> String, cooked: &str) -> String {
    if cooked.contains("${") {
        let marked = format!(
            "{}{cooked}",
            crate::executor::markers::STORAGE_WORD_PREFIX_STR
        );
        expand_word(&marked)
    } else {
        expand_word(cooked)
    }
}

fn quote_aware_case_word(raw: &str, mut expand_word: impl FnMut(&str) -> String) -> String {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')) {
            let quote = chars[index + 1];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 2, quote, quote == '\'') {
                let body = chars[index + 2..end].iter().collect::<String>();
                if quote == '\'' {
                    output.push_str(&decode_ansi_c_escapes(&body));
                } else {
                    // Same DQ-context rule as quote_aware_glob_rhs
                    // (rubash#2981): cook the body to the lexer's carrier
                    // spelling before expanding, so `\"`/`\$`/`\\`/`` \` ``
                    // resolve as double-quote escapes and `'` travels as
                    // DATA_SQUOTE data.
                    let cooked = crate::lexer::quotes::cook_double_quoted_segment(&body);
                    output.push_str(&expand_case_dq_segment(&mut expand_word, &cooked));
                }
                index = end + 1;
                continue;
            } else {
                // No closer: an unclosed quote is a parse error before
                // execution in GNU; never spin the walker on it.
                output.push(chars[index]);
                index += 1;
                continue;
            }
        }

        if matches!(chars[index], '\'' | '"') {
            let quote = chars[index];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 1, quote, false) {
                let body = chars[index + 1..end].iter().collect::<String>();
                if quote == '\'' {
                    // GNU subst.c:11882-11886: every byte of a '...' span is
                    // quoted data — a backtick here must never reach the
                    // comsub scanner (rubash#153: `case 'a'\''`b`'` ran `b`).
                    output.push_str(&body);
                } else {
                    // Same DQ-context rule as quote_aware_glob_rhs
                    // (rubash#2981): cook the body to the lexer's carrier
                    // spelling before expanding, so `\"`/`\$`/`\\`/`` \` ``
                    // resolve as double-quote escapes and `'` travels as
                    // DATA_SQUOTE data. Quotes inside a `${...}`/`$(...)`/
                    // backtick unit belong to the substitution's own
                    // grammar (parse.y:3877 parse_matched_pair) and stay
                    // raw through the cooker (rubash#258 BUG_PSUBSQUOT).
                    let cooked = crate::lexer::quotes::cook_double_quoted_segment(&body);
                    output.push_str(&expand_case_dq_segment(&mut expand_word, &cooked));
                }
                index = end + 1;
                continue;
            } else {
                output.push(chars[index]);
                index += 1;
                continue;
            }
        }

        let mut pending_start = index;
        while index < chars.len()
            && chars[index] != '\''
            && chars[index] != '"'
            && !(chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')))
        {
            // Quoting inside a substitution belongs to the substitution's
            // own input (GNU parse_matched_pair), so skip the unit whole.
            if chars[index] == '$' && chars.get(index + 1) == Some(&'(') {
                if let Some(close) =
                    crate::lexer::skip_parenthesized_unit_corrected(&chars, index + 1)
                {
                    index = close.min(chars.len());
                    continue;
                }
            }
            if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
                if let Some(close) = skip_braced_case_pattern_unit(&chars, index + 1) {
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
                continue;
            }
            if chars[index] == '\\' && index + 1 < chars.len() {
                // GNU parse.y:5366-5398 read_token_word: an unquoted
                // backslash quotes exactly one character, which survives
                // as a quoted literal (got_escaped_character,
                // parse.y:5694-5706). Flush the pending expandable run,
                // then emit the escaped character as data — `case \x in`
                // must expand the word to `x` and match pattern `x`
                // (case1.sub ok 9).
                if pending_start < index {
                    let segment = chars[pending_start..index].iter().collect::<String>();
                    output.push_str(&expand_word(&segment));
                }
                output.push(chars[index + 1]);
                index += 2;
                pending_start = index;
                continue;
            }
            index += 1;
        }
        if pending_start < index {
            let segment = chars[pending_start..index].iter().collect::<String>();
            output.push_str(&expand_word(&segment));
        }
    }

    output
}

/// The first `${name[@]}` unit in `text`: (byte offset of the `$`, byte
/// offset just past the closing `}`, array name). Braced and
/// parenthesized units that are not exactly `${name[@]}` are skipped
/// whole, so a `[@]` nested inside another expansion is not mistaken
/// for a top-level one.
fn find_case_pattern_array_at(text: &str) -> Option<(usize, usize, String)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let plain: Vec<char> = chars.iter().map(|(_, c)| *c).collect();
    let mut index = 0usize;
    while index < plain.len() {
        if plain[index] == '$' && plain.get(index + 1) == Some(&'(') {
            if let Some(close) = crate::lexer::skip_parenthesized_unit_corrected(&plain, index + 1)
            {
                index = close.min(plain.len());
                continue;
            }
        }
        if plain[index] == '$' && plain.get(index + 1) == Some(&'{') {
            if let Some(close) = skip_braced_case_pattern_unit(&plain, index + 1) {
                let unit: String = plain[index..=close].iter().collect();
                let name = unit
                    .strip_prefix("${")
                    .and_then(|unit| unit.strip_suffix("[@]}"))
                    .filter(|name| {
                        !name.is_empty()
                            && name
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    });
                if let Some(name) = name {
                    let start = chars[index].0;
                    let end = if close + 1 < chars.len() {
                        chars[close + 1].0
                    } else {
                        text.len()
                    };
                    return Some((start, end, name.to_string()));
                }
                index = close + 1;
                continue;
            }
        }
        index += 1;
    }
    None
}

enum CasePatternSegment {
    /// The segment's literal contribution; later segments still glue
    /// onto the same pattern word.
    Continues(String),
    /// The pattern word is complete: a multi-element `${name[@]}`
    /// produced a second field, so everything after it (the rest of
    /// this segment and all later segments) is discarded.
    Finished(String),
}

/// Expand one double-quoted case-pattern segment under GNU's field rule
/// (rubash#1393): the pattern is a single word, but a quoted
/// `"${name[@]}"` expands to a field LIST and only the first field
/// belongs to the pattern — text before the expansion glues onto that
/// field, text after it belongs to later fields. A one-element
/// expansion glues whole; an empty one contributes nothing.
/// Cook one double-quoted case-pattern segment chunk to the lexer's carrier
/// spelling before expanding it (same DQ-context rule as
/// quote_aware_glob_rhs, rubash#2981): `\"`/`\$`/`\\`/`` \` `` resolve as
/// double-quote escapes and `'` travels as DATA_SQUOTE data, instead of the
/// raw fragment being re-read without its quoting context.
fn cook_dquoted_case_segment(chunk: &str) -> String {
    crate::lexer::quotes::cook_double_quoted_segment(chunk)
}

fn case_pattern_quoted_segment(
    body: &str,
    expand_word: &mut impl FnMut(&str) -> String,
    array_at: &std::collections::HashMap<String, (String, usize)>,
) -> CasePatternSegment {
    let mut literal = String::new();
    let mut rest = body;
    loop {
        let Some((start, end, name)) = find_case_pattern_array_at(rest) else {
            literal.push_str(&expand_word(&cook_dquoted_case_segment(rest)));
            return CasePatternSegment::Continues(literal);
        };
        let (first, count) = array_at.get(&name).cloned().unwrap_or_default();
        literal.push_str(&expand_word(&cook_dquoted_case_segment(&rest[..start])));
        if count == 0 {
            rest = &rest[end..];
            continue;
        }
        literal.push_str(&first);
        if count >= 2 {
            return CasePatternSegment::Finished(literal);
        }
        rest = &rest[end..];
    }
}

fn quote_aware_case_pattern(
    raw: &str,
    mut expand_word: impl FnMut(&str) -> String,
    array_at: &std::collections::HashMap<String, (String, usize)>,
) -> String {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')) {
            let quote = chars[index + 1];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 2, quote, quote == '\'') {
                let body = chars[index + 2..end].iter().collect::<String>();
                if quote == '"' {
                    match case_pattern_quoted_segment(&body, &mut expand_word, array_at) {
                        CasePatternSegment::Continues(literal) => {
                            output.push_str(&escape_case_pattern_literal(&literal));
                            index = end + 1;
                            continue;
                        }
                        CasePatternSegment::Finished(literal) => {
                            output.push_str(&escape_case_pattern_literal(&literal));
                            return output;
                        }
                    }
                }
                let literal = decode_ansi_c_escapes(&body);
                output.push_str(&escape_case_pattern_literal(&literal));
                index = end + 1;
                continue;
            } else {
                // Unclosed unit: parse error before execution in GNU;
                // advance as data rather than spin.
                output.push(chars[index]);
                index += 1;
                continue;
            }
        }

        if matches!(chars[index], '\'' | '"') {
            let quote = chars[index];
            if let Some(end) = quoted_case_pattern_end(&chars, index + 1, quote, false) {
                let body = chars[index + 1..end].iter().collect::<String>();
                if quote == '"' {
                    // parse.y read_token_word's double-quote scanner never
                    // opens single-quote state: a `'` inside the body is
                    // literal data (GNU case patterns keep it, so
                    // `case $s in "''")` matches the two quote characters).
                    // The segment expander cooks each chunk to the lexer's
                    // carrier spelling before expanding (same DQ-context
                    // rule as quote_aware_glob_rhs, rubash#2981), so the
                    // `'` travels as DATA_SQUOTE data and `\"`/`\$`/`\\`/
                    // `` \` `` resolve as double-quote escapes. Quotes
                    // inside a `${...}`/`$(...)`/backtick unit stay raw
                    // (parse.y:3877 parse_matched_pair owns them;
                    // rubash#258 BUG_PSUBSQUOT).
                    match case_pattern_quoted_segment(&body, &mut expand_word, array_at) {
                        CasePatternSegment::Continues(literal) => {
                            output.push_str(&escape_case_pattern_literal(&literal));
                            index = end + 1;
                            continue;
                        }
                        CasePatternSegment::Finished(literal) => {
                            output.push_str(&escape_case_pattern_literal(&literal));
                            return output;
                        }
                    }
                }
                output.push_str(&escape_case_pattern_literal(&body));
                index = end + 1;
                continue;
            } else {
                output.push(chars[index]);
                index += 1;
                continue;
            }
        }

        let start = index;
        while index < chars.len()
            && chars[index] != '\''
            && chars[index] != '"'
            && !(chars[index] == '$' && matches!(chars.get(index + 1), Some('\'' | '"')))
        {
            // GNU read_token_word: quoting inside a substitution belongs to
            // the substitution's own input, not to the pattern — `$( echo
            // "$bar")` must not split the segment at the inner `"`, or the
            // `$(` looks unclosed and the expansion reports EOF
            // (comsub-posix6.sub case-pattern substitution).
            if chars[index] == '$' && chars.get(index + 1) == Some(&'(') {
                // Returns the index just past the closing `)`.
                if let Some(close) =
                    crate::lexer::skip_parenthesized_unit_corrected(&chars, index + 1)
                {
                    index = close.min(chars.len());
                    continue;
                }
            }
            if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
                if let Some(close) = skip_braced_case_pattern_unit(&chars, index + 1) {
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
                continue;
            }
            if chars[index] == '\\' && index + 1 < chars.len() {
                index += 2;
            } else {
                index += 1;
            }
        }
        let segment = chars[start..index].iter().collect::<String>();
        let expanded = expand_word(&segment);
        // The segment text between the source quotes is expanded, and
        // the decode runs in expansion-result mode: quote characters
        // the expansion produced are data, not pattern quoting, and
        // are not stripped from the edges either (rubash#2428).
        output.push_str(&decode_case_pattern_expansion(&expanded));
    }

    output
}

/// Balanced `${...}` skip for the case-pattern segment scanner — braces,
/// quotes, and escapes inside the expansion are its own (GNU
/// parse_matched_pair). Returns the index of the closing `}`.
pub(in crate::executor) fn skip_braced_case_pattern_unit(
    chars: &[char],
    open: usize,
) -> Option<usize> {
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
            '{' if !single && !double => depth += 1,
            '}' if !single && !double => {
                depth -= 1;
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

pub(in crate::executor) fn quoted_case_pattern_end(
    chars: &[char],
    start: usize,
    quote: char,
    ansi_c: bool,
) -> Option<usize> {
    let mut index = start;
    let mut escaped = false;
    while index < chars.len() {
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        // GNU parse.y:3992-3996 parse_matched_pair: inside a `$'...'`
        // ANSI-C unit (P_ALLOWESC, set by read_token_word at
        // parse.y:5559) a backslash sets LEX_PASSNEXT — the next
        // character, including the closing `'`, is consumed as escaped
        // data and never ends the span. A plain `'...'` span honors no
        // escapes, so `ansi_c` is only true for the `$'` caller.
        if (quote == '"' || ansi_c) && chars[index] == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if quote == '"' {
            // GNU parse.y skip_double_quoted (via parse_matched_pair,
            // parse.y:5416-5432): inside `"..."` a substitution unit owns
            // its own quoting — `$(` takes parse_comsub/parse_matched_pair,
            // `${` takes parse_matched_pair, and a backquote opens backquote
            // mode — so an inner `"` never ends the span. Stopping at the
            // first inner quote truncated `$(<"file")` to `$(<` and made
            // the comsub look unclosed (rubash#259: bats-preprocess
            // `<<<"$(<"$test_file")"$'\n'` lost the whole here-string).
            match chars[index] {
                '`' => {
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
                    continue;
                }
                '$' if chars.get(index + 1) == Some(&'(') => {
                    if let Some(close) =
                        crate::lexer::skip_parenthesized_unit_corrected(chars, index + 1)
                    {
                        index = close.min(chars.len());
                        continue;
                    }
                }
                '$' if chars.get(index + 1) == Some(&'{') => {
                    if let Some(close) = skip_braced_case_pattern_unit(chars, index + 1) {
                        index = close + 1;
                        continue;
                    }
                }
                _ => {}
            }
        }
        if chars[index] == quote {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn escape_case_pattern_literal(value: &str) -> String {
    // GNU subst.c add_quoted_string, reached from the case-pattern
    // expansion path (execute_cmd.c:3670 -> expand_word_leave_quoted):
    // every character of a quoted literal is CTLESC-protected, and the
    // pattern matcher (sm_loop.c) then treats it as plain data. In
    // particular a quoted `]' inside a bracket pattern is a MEMBER, never
    // the closer — `case c in ( *["ab]cd"]* )' matches (modernish
    // FTL_BRACSQBR). Backslash-escaping instead carried posixpat member
    // semantics (posixpat ok 21 keeps `[\]` open) and left `]' entirely
    // unprotected, so the bracket closed early and the pattern could
    // never match.
    //
    // One rubash carrier rides as TWO chars: a byte the text pipeline
    // cannot hold is stored as a raw-byte marker pair (markers.rs
    // RAW_BYTE_MARKER_ESCAPE + RAW_BYTE_MARKER_FIRST+byte; owner
    // substitution_metadata.rs) — the pair IS one character unit, the
    // carrier for one byte. GNU's per-character CTLESC therefore guards
    // the pair once, as a unit: wrapping each half separately (the
    // d3e8affa form) read the introducer as a literal U+E000 and
    // destroyed the pair, so byte-flattening
    // (conditional/pattern.rs flatten_pattern_to_byte_chars) decoded
    // garbage and every quoted pattern holding such a byte stopped
    // matching — `z=$'\v\f\a\b'; case "$z" in "$z")` and the
    // `$'\v\f\a\b')` clause of nquote.tests line 79 printed `bad'
    // (nquote snapshot red gate, regression window beb1eca6..d3e8affa).
    //
    // The literal-char escape pair is the same shape in the other
    // registry family: a typed registry-zone char rides transport as
    // LITERAL_CHAR_ESCAPE + the char (markers.rs push_literal_char) —
    // ONE user character, so it likewise takes one guard as a unit.
    // Guarding the halves separately interposed a CTLESC inside the
    // pair; the matcher's flattening reads a literal-escape pair
    // greedily as one data char, so it consumed E400+CTLESC as a pair
    // whose payload is CTLESC itself and the real char (a typed
    // `$'\uE002'`) could never match (rubash#3014).
    use crate::executor::markers::{
        CTLESC, LITERAL_CHAR_ESCAPE, RAW_BYTE_MARKER_ESCAPE, RAW_BYTE_MARKER_FIRST,
        RAW_BYTE_MARKER_LAST,
    };
    let mut escaped = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        escaped.push(CTLESC);
        escaped.push(ch);
        if (ch as u32) == RAW_BYTE_MARKER_ESCAPE {
            if let Some(&payload) = chars.peek() {
                if (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&(payload as u32)) {
                    escaped.push(chars.next().expect("peeked raw-byte payload"));
                }
            }
        }
        if ch == LITERAL_CHAR_ESCAPE {
            // Greedy pairing, mirroring flatten_pattern_to_byte_chars:
            // the payload is part of the same character unit. A lone
            // trailing escape has no payload and stays a single unit.
            if let Some(payload) = chars.next() {
                escaped.push(payload);
            }
        }
    }
    escaped
}

#[cfg(test)]
mod case_arm_static_tests {
    //! Perf #662: the static-literal short-circuit must agree with the
    //! slow path per pattern shape — GNU execute_cmd.c:3686/3712
    //! re-expands dynamic arms every attempt (match.c/cond.c), so only
    //! proven-static text may borrow.
    use super::case_arm_pattern_is_static_literal;
    use crate::executor::Executor;
    use crate::parser::CasePattern;
    use std::borrow::Cow;

    #[test]
    fn static_shapes_borrow_stored_text() {
        for shape in [
            "foo",
            "*.sh",
            "[a-z]",
            "a?c",
            "a b",
            "caf\u{e9}",
            "+x%/=:;!#&@()|",
        ] {
            let pattern = CasePattern::new(shape.to_string(), 0, 0);
            assert!(
                case_arm_pattern_is_static_literal(&pattern.text, &pattern.raw_text),
                "static {shape:?} rejected"
            );
            match Executor::new().expand_case_pattern(&pattern) {
                Cow::Borrowed(text) => assert_eq!(text, shape),
                Cow::Owned(_) => panic!("static {shape:?} took the slow path"),
            }
        }
    }

    #[test]
    fn dynamic_shapes_take_slow_path() {
        for (text, raw) in [
            ("$v", "$v"),
            ("`id`", "`id`"),
            ("*", "\"*\""),
            ("a", "'a'"),
            ("\\a", "\\a"),
            ("~/x", "~/x"),
            ("{a,b}", "{a,b}"),
            ("<(x)", "<(x)"),
            ("a\u{e317}b", "a\u{e317}b"),
            ("a\u{11}b", "a\u{11}b"),
        ] {
            assert!(
                !case_arm_pattern_is_static_literal(text, raw),
                "dynamic {raw:?} admitted"
            );
        }
        let mut executor = Executor::new();
        executor.set_env("V", "foo");
        match executor.expand_case_pattern(&CasePattern::new("$V".to_string(), 0, 0)) {
            Cow::Owned(text) => assert_eq!(text, "foo"),
            Cow::Borrowed(_) => panic!("dynamic $V borrowed"),
        }
    }

    #[test]
    fn escquote_2421_shapes_expand_like_gnu() {
        // rubash#2421: an escape pair in a quote-free case pattern must
        // reach the matcher intact (GNU execute_cmd.c:3712-3736 +
        // pathexp.c:212 quote_string_for_globbing): the quoted character
        // is literal, an unquoted `*` after it stays a live glob. Before
        // the fix the guard scheme detached the `\`, the expander ate
        // the bare `"`/`$`, and `\"*` collapsed to `\*` (literal star).
        for arm in [
            "\\\"*", "\\$*", "\\a*", "\\\\*", "\\\"", "a\\\"b", "\\\"a", "*\\\"", "\\*", "\\[*",
            "\\`*",
        ] {
            let pattern = CasePattern::new(arm.to_string(), 0, 0);
            let mut executor = Executor::new();
            assert_eq!(
                executor.expand_case_pattern(&pattern).into_owned(),
                arm,
                "arm {arm:?} must survive expansion verbatim"
            );
        }
        // The lexer spells `\"` / `\'` in word text as the data-quote
        // carriers \x18 / \x17 (the backslash is consumed at read time);
        // both must decode to the matcher's escape spelling.
        for (text, expected) in [
            ("\u{18}*", "\\\"*"),
            ("a\u{18}b", "a\\\"b"),
            ("\u{18}", "\\\""),
            ("*\u{18}x", "*\\\"x"),
            ("\u{17}*", "\\'*"),
            ("a\u{17}b", "a\\'b"),
        ] {
            let pattern = CasePattern::new(text.to_string(), 0, 0);
            let mut executor = Executor::new();
            assert_eq!(
                executor.expand_case_pattern(&pattern).into_owned(),
                expected,
                "carrier arm {text:?} must decode to {expected:?}"
            );
        }
    }

    #[test]
    fn arms_match_like_gnu_per_shape() {
        for (script, code) in [
            ("case foo in foo) true;; *) false;; esac", 0),
            ("case foo in bar) true;; *) false;; esac", 1),
            ("case foo in f*) true;; *) false;; esac", 0),
            ("case 'a b' in a\\ b) true;; *) false;; esac", 0),
            ("v=foo; case foo in $v) true;; *) false;; esac", 0),
            ("v=bar; case foo in $v) true;; *) false;; esac", 1),
            ("case foo in $(echo foo)) true;; *) false;; esac", 0),
            ("v='f*'; case foo in \"$v\") true;; *) false;; esac", 1),
            ("case a in \\a) true;; *) false;; esac", 0),
        ] {
            let tokens = crate::lexer::tokenize(script);
            let ast = crate::parser::parse(&tokens);
            let mut executor = Executor::new();
            executor.execute_ast(&ast).ok();
            assert_eq!(executor.last_exit_code(), code, "{script:?}");
        }
    }
}

impl Executor {
    pub(in crate::executor) fn execute_case_command(
        &mut self,
        case_command: &CaseCommand,
    ) -> Result<(), ExecuteError> {
        // TODO(parse.y/execute_cmd.c/pathexp.c): Bash case execution uses the
        // full pattern matcher, fall-through operators, expansion flags, and
        // compound-list control flow. This handles the common shell glob
        // operators used by simple `case` clauses.
        // GNU execute_cmd.c:3679-3680 traces the case head (with the raw
        // unexpanded word, print_cmd.c:742) before pattern expansion.
        // xtrace_print_case_command_head (print_cmd.c:742-749) prints
        // case_command->word->word — the WORD_DESC text, which in GNU still
        // carries the source quote characters ("$x" stays "$x"). rubash's
        // equivalent text is word_metadata.raw; the lexer `.word` value is
        // the quote-stripped transport form whose PARAM_NAME_END_MARKER
        // boundary byte (0x13, where a deleted closing quote terminated an
        // unbraced $name) must never reach the trace output.
        if self.xtrace_enabled() {
            let prefix = self.xtrace_prefix();
            let raw_word = if case_command.word_metadata.raw.is_empty() {
                case_command.word.as_str()
            } else {
                case_command.word_metadata.raw.as_str()
            };
            let visible = crate::locale::decode_to_visible_text(raw_word);
            self.xtrace_write(format!("{prefix}case {visible} in\n").as_bytes());
        }
        // GNU execute_cmd.c:3660-3668: the case head is printed
        // (print_case_command_head, print_cmd.c:731 -> `case WORD in `) and
        // run_debug_trap fires before the word is expanded, so the trap sees
        // the raw unexpanded word — `case "$v" in ` keeps the quotes.
        // GNU's run_debug_trap (trap.c) is a no-op when no DEBUG action is
        // armed; gate the head-text `format!` on that presence test so the
        // common no-trap loop does not allocate a String per `case` (same
        // folded admission as the ast_exec per-command DEBUG preamble,
        // rubash#186).
        if !self.debug_trap_running
            && ((self.debug_trap_in_scope()
                && crate::builtins::trap::has_active_trap_action(
                    &self.shell_state.env_vars,
                    "DEBUG",
                    "__RUBASH_TRAP_DEBUG",
                ))
                || self.debugger_session_here())
        {
            let raw_word = if case_command.word_metadata.raw.is_empty() {
                case_command.word.as_str()
            } else {
                case_command.word_metadata.raw.as_str()
            };
            // execute_cmd.c:3672-3676: under extdebug a non-zero DEBUG status
            // skips the whole case — the word is never expanded, no clause
            // runs, and the case command returns EXECUTION_SUCCESS (probe
            // k9/m-issue: `R=0` after the skipped head).
            if self
                .run_debug_trap(
                    crate::debugger::SeamKind::CaseHead,
                    &format!("case {raw_word} in "),
                )?
                .is_some()
            {
                self.exit_code = 0;
                return Ok(());
            }
        }
        // GNU execute_cmd.c:3695: the case subject goes through
        // expand_word_leave_quoted, so a nounset unbound variable (or
        // another fatal expansion error) in it raises the FORCE_EOF
        // class before any clause is tested (rubash#1130) — the
        // simple-command pre-scan never sees the subject.
        {
            let scan_text = if case_command.word_metadata.raw.is_empty() {
                case_command.word.as_str()
            } else {
                case_command.word_metadata.raw.as_str()
            };
            if let Some(error) =
                self.parameter_expansion_error_in_word_isolated_with_raw(scan_text, Some(scan_text))
            {
                return self.raise_word_expansion_error(error);
            }
        }
        // GNU execute_cmd.c:3695-3699: the case word is expanded by
        // expand_word_leave_quoted and then passed through dequote_string —
        // every single-quoted span is literal data (subst.c:11882-11886
        // never consults the command-substitution scanner inside '...'), and
        // only unquoted segments go through the full expansion pipeline.
        // The raw token text carries those span boundaries; the lexer's word
        // value has them stripped, which made `case 'a'\''`b`'` re-execute
        // `b` as a live substitution (rubash#153). Walk the raw text like
        // the pattern side (quote_aware_case_pattern) but without the
        // glob-literal escaping, which belongs to the pattern domain only.
        //
        // rubash#4007: materialize the word's process substitutions from the
        // PARSER's quote-aware scan of the raw word
        // (word_metadata.process_substitutions), splicing each produced
        // `/dev/fd/N` into the text BEFORE expansion — the same vehicle the
        // case-pattern side and the for/select list words use
        // (materialize_for_list_process_substitutions, rubash#2390). GNU
        // expand_word_internal's '<'/'>' arms (subst.c:11365-11391) fire
        // only while scanning the parser word itself, never on an expansion
        // RESULT: a `<(`/`>(` carried in a variable's value is data. The
        // cooked-text rescan expand_case_word used to run after expansion
        // executed `v='x >(touch PWNED) y'; case "$v" in ...` — the value's
        // literal text — and likewise materialized `"<(...)"` spans that
        // were quoted data at the source level.
        let word_substitutions = &case_command.word_metadata.process_substitutions;
        let word = if case_command.word_metadata.raw.is_empty()
            || case_command.word_metadata.raw == case_command.word
        {
            if word_substitutions.is_empty() {
                self.expand_case_word(&case_command.word)
            } else {
                let materialized = self
                    .materialize_for_list_process_substitutions(
                        &case_command.word,
                        word_substitutions,
                    )
                    .unwrap_or_else(|_| case_command.word.clone());
                self.expand_case_word(&materialized)
            }
        } else {
            let raw = if word_substitutions.is_empty() {
                case_command.word_metadata.raw.clone()
            } else {
                self.materialize_for_list_process_substitutions(
                    &case_command.word_metadata.raw,
                    word_substitutions,
                )
                .unwrap_or_else(|_| case_command.word_metadata.raw.clone())
            };
            quote_aware_case_word(&raw, |segment| self.expand_case_word(segment))
        };
        let word = tilde_expand::strip_assignment_quote_marker(&word);
        // GNU: the case word's command substitution recorded its status in
        // last_command_exit_value (subst.c command_substitute), and the case
        // command itself commits no status until a clause body finishes — so
        // $? inside the first body command still sees the substitution's
        // status (issue #485: `case $(false) in "") echo $?;; esac` prints 1).
        // Commit the word-expansion overlay to the real status here; the
        // clause body's execute_ast_inner clears the overlay at its node
        // boundary, which would otherwise lose it.
        if let Some(status) = self.word_expansion_comsub_exit.take() {
            self.exit_code = status;
        }
        self.abandon_on_arithmetic_expansion_error()?;
        let nocasematch =
            crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "nocasematch");
        let mut fall_through = false;
        let mut matched_any = false;
        // GNU execute_cmd.c:3644 execute_case_command builds `word` once
        // (expand_word_leave_quoted + dequote_string, 3695-3699) and reuses
        // it for every arm's strmatch at 3736. Stage its `Vec<char>` once
        // here so the N-arm loop below does not re-collect it per candidate;
        // the pattern side still stages per arm (it differs per arm) while
        // the literal / simple-glob fast paths stay borrow-only. The cell is
        // LAZY: a loop of static literal arms (GNU strmatch's no-staging
        // path) never builds it.
        let mut word_chars: Option<Vec<char>> = None;
        let mut index = 0;
        while let Some(clause) = case_command.clauses.get(index) {
            let mut pattern_error: Option<(String, String, i32)> = None;
            let matched = fall_through
                || clause.pattern_nodes.iter().any(|pattern| {
                    // rubash#2701 — GNU re-expands every arm's pattern
                    // through the same word pipeline per match attempt
                    // (execute_cmd.c:3712 expand_word_leave_quoted ->
                    // subst.c:11229 expand_word_internal): a `:?` in an
                    // evaluated arm is the same FORCE_EOF class as in a
                    // command word (subst.c:10347 case '?', 10414/10418)
                    // and `:=` binds at the evaluation point. GNU
                    // consults arms in order and stops at the first
                    // match, so the verdict latch preserves ordering —
                    // an unreached arm can neither bind nor error.
                    if pattern_error.is_some() {
                        return true;
                    }
                    if pattern.text.contains('$') || pattern.text.contains('`') {
                        let raw = (!pattern.raw_text.is_empty()).then_some(pattern.raw_text.as_str());
                        self.apply_parameter_assignment_expansions_in_word(&pattern.text);
                        pattern_error = self
                            .parameter_expansion_error_in_word_isolated_with_raw(&pattern.text, raw);
                        if pattern_error.is_some() {
                            return true;
                        }
                    }
                    let stripped = self.expand_case_pattern(pattern);
                    if self.shell_state.arithmetic_fatal_error.get()
                        || self.shell_state.arithmetic_nounset_error.get()
                    {
                        return false;
                    }
                    // rubash#3224: the extglob route is taken only while
                    // the LIVE gate is on — GNU execute_cmd.c:3736 matches
                    // case patterns through strmatch with FNMATCH_EXTFLAG
                    // evaluated from the live `extended_glob` (pathexp.h),
                    // so with the gate off (e.g. a same-unit `shopt -u`
                    // already executed) the operators are literal text
                    // and the plain glob matcher below is the correct
                    // route.
                    if case_pattern_has_extglob(&stripped) && self.extglob_enabled() {
                        // Extglob staging needs the word chars immediately
                        // (no literal fast path in extglob_matches_at); seed
                        // the lazy cell here.
                        let word_chars =
                            word_chars.get_or_insert_with(|| word.chars().collect());
                        if nocasematch {
                            crate::executor::conditional::extglob_case_pattern_matches_nocase_with_word_chars(
                                &stripped, word_chars,
                            )
                        } else {
                            crate::executor::conditional::extglob_case_pattern_matches_with_word_chars(
                                &stripped, word_chars,
                            )
                        }
                    } else if nocasematch {
                        case_pattern_matches_nocase_with_word_chars(
                            &stripped,
                            &word,
                            &mut word_chars,
                        )
                    } else {
                        case_pattern_matches_with_word_chars(&stripped, &word, &mut word_chars)
                    }
                });
            if let Some(error) = pattern_error.take() {
                return self.raise_word_expansion_error(error);
            }
            self.abandon_on_arithmetic_expansion_error()?;
            if matched {
                matched_any = true;
                if clause.body.is_empty() {
                    self.exit_code = 0;
                } else {
                    // GNU execute_case_command (execute_cmd.c:5200 family)
                    // walks the clause body by POINTER; no clone is taken.
                    self.execute_ast_inner(&clause.body)?;
                }
                match clause.terminator {
                    CaseTerminator::Break => return Ok(()),
                    CaseTerminator::FallThrough => {
                        fall_through = true;
                    }
                    CaseTerminator::TestNext => {
                        fall_through = false;
                    }
                }
            }
            index += 1;
        }

        if !matched_any {
            self.exit_code = 0;
        }
        Ok(())
    }

    fn expand_case_pattern<'a>(
        &mut self,
        pattern: &'a crate::parser::CasePattern,
    ) -> std::borrow::Cow<'a, str> {
        // GNU execute_cmd.c:3686/3712 re-expands every arm per match attempt
        // (expand_word_leave_quoted, matched via match.c/cond.c): dynamic
        // re-expansion is semantics and stays. A static literal expands to
        // itself, so borrow the stored text: zero alloc, skip the pipeline.
        if case_arm_pattern_is_static_literal(&pattern.text, &pattern.raw_text) {
            return std::borrow::Cow::Borrowed(pattern.text.as_str());
        }
        // rubash#2764: a case pattern word carrying an UNQUOTED
        // `<(...)` / `>(...)` executes it — GNU expands every pattern
        // with the full word pipeline (execute_cmd.c:3712
        // expand_word_leave_quoted, per pattern attempt) and the
        // `<`/`>` arms of expand_word_internal (subst.c:11365-11385)
        // extract the body verbatim (extract_process_subst,
        // subst.c:1311) and run process_substitute (subst.c:6362),
        // splicing the `/dev/fd/N` result into the expanding word.
        // Rubash's word pipeline materializes process substitution
        // from the parser's quote-aware scan of the RAW word, not
        // from the cooked text (materialize_for_list_process_substitutions,
        // rubash#2390: a quoted `"<(...)"` stays literal, a body keeps
        // its backslashes, and `<(...)` text that only appears in an
        // expansion result is data). Reuse that exact vehicle here,
        // BEFORE either expansion branch runs: splicing the fd path
        // in up front keeps the body out of the pattern expander (a
        // `$v` / `$(...)` inside the body belongs to the child, not
        // to the pattern) and keeps body quotes from mis-splitting
        // the quote-aware walker's segments. Quoted spans are never
        // in the derived list, so they stay literal; the splice is
        // per match attempt (this fn runs once per attempt), so a
        // side-effecting body re-runs per arm like GNU's
        // re-expansion. The static-literal gate above already
        // excludes `<`/`>` from the fast path, so this only fires
        // for patterns that can carry a substitution.
        let pattern_substitutions = if pattern.raw_text.contains("<(")
            || pattern.raw_text.contains(">(")
        {
            crate::parser::WordMetadata::new(0, pattern.raw_text.clone(), pattern.raw_text.clone())
                .process_substitutions
        } else {
            Vec::new()
        };
        // rubash#2764 (F2 rework, F3 fix): materialize ONCE per
        // attempt, on the branch that consumes it. The two expansion
        // branches below consume disjoint strings (the quote-free
        // branch expands `pattern.text`, the quoted branch expands
        // `pattern.raw_text`). Materializing both strings eagerly
        // spawned/registered every body twice (input bodies ran
        // twice — no per-source dedup in
        // spawn_input_process_substitution_child; output bodies
        // registered two different fd-words and both finished);
        // GNU runs each body once per attempt (subst.c:6362
        // process_substitute, per pattern attempt). The branch
        // decision is therefore made ONCE, and on quotes OUTSIDE
        // `<(...)` / `>(...)` bodies: a quote inside a body belongs
        // to the child, not to the pattern, and disappears with the
        // `<(...)`→`/dev/fd/N` splice — deciding on the raw text's
        // body-internal quotes (F3) sent such a pattern down the
        // quoted arm's materialization while the downstream gate
        // re-judged the spliced string and took the unquoted arm,
        // which then consumed the UN-materialized text. Quotes
        // outside bodies survive the splice, so one outside-quote
        // decision serves both the materialization choice and the
        // branch gate below.
        let quoted = if pattern_substitutions.is_empty() {
            case_pattern_raw_has_quotes(&pattern.raw_text)
        } else {
            case_pattern_raw_has_outside_quotes(&pattern.raw_text)
        };
        let materialized_text;
        let materialized_raw;
        let pattern_text: &str;
        let pattern_raw: &str;
        if pattern_substitutions.is_empty() {
            pattern_text = &pattern.text;
            pattern_raw = &pattern.raw_text;
        } else if quoted {
            materialized_raw = self
                .materialize_for_list_process_substitutions(
                    &pattern.raw_text,
                    &pattern_substitutions,
                )
                .unwrap_or_else(|_| pattern.raw_text.clone());
            pattern_text = &pattern.text;
            pattern_raw = &materialized_raw;
        } else {
            materialized_text = self
                .materialize_for_list_process_substitutions(&pattern.text, &pattern_substitutions)
                .unwrap_or_else(|_| pattern.text.clone());
            pattern_text = &materialized_text;
            pattern_raw = &pattern.raw_text;
        };
        const PROTECTED_CASE_PATTERN_BACKSLASH: char =
            crate::executor::markers::CASE_PATTERN_BACKSLASH_GUARD;

        // rubash#2764 (F3): the gate reuses the single `quoted`
        // decision above — it must agree with the materialization
        // choice, because each branch consumes the string its arm
        // materialized. Re-judging here on the (sometimes spliced)
        // `pattern_raw` is what sent body-quoted patterns down the
        // unquoted arm with the un-materialized text.
        if !quoted {
            // Backslashes in a case pattern are escape characters, not
            // quote-removal subjects: GNU performs no quote removal on the
            // pattern (execute_cmd.c:3733-3736) because read_token_word
            // consumes `\X` as ONE quoted unit at read time
            // (parse.y:5366-5398: the backslash branch sets
            // pass_next_character; parse.y:5358-5362 routes the next char
            // to got_escaped_character, which CTLESC-marks it), and
            // quote_string_for_globbing (pathexp.c:212) re-emits that
            // quoting for strmatch after expand_word_leave_quoted
            // (execute_cmd.c:3712-3725).
            //
            // The escape pair must therefore never be split before the
            // expander. Protecting only the `\` (the former guard scheme)
            // left the quoted character bare, and the expander re-read it
            // as syntax: `\"` opened a quote span that quote removal then
            // deleted (the glob after it collapsed to a literal), `\$`
            // started a parameter expansion that ate the `$`, and a `\`
            // inside a `${...}` default was rewritten before the unit's
            // own grammar could see it (rubash#2421). Walk the text the
            // way quote_aware_glob_rhs walks a `[[ == ]]` RHS instead:
            // `\X` pairs outside nested expansion units are emitted
            // verbatim for the matcher, the runs between them go through
            // expansion, and a `\` INSIDE a `$(...)`/`${...}`/backtick
            // unit belongs to that unit's own grammar
            // (parse.y:3877 parse_matched_pair), so units stay whole
            // inside their run. The lexer's data-quote carriers
            // (DATA_DQUOTE / DATA_SQUOTE — its spelling of `\"` / `\'`
            // in word text) are decoded below to the matcher's escape
            // spelling for the literal quote.
            //
            // Use the mutable expander: case pattern expansion is not an
            // isolated sub-expression — `$((x=1))` in a pattern must keep
            // its assignment side effects (bash execute_cmd.c evaluates
            // patterns with the current variable state; case.tests `;&`
            // fall-through).
            let chars: Vec<char> = pattern_text.chars().collect();
            let mut output = String::new();
            let mut run_start = 0usize;
            let mut index = 0usize;
            while index < chars.len() {
                // Nested expansion units ride inside the current run.
                if chars[index] == '$' && chars.get(index + 1) == Some(&'(') {
                    if let Some(close) =
                        crate::lexer::skip_parenthesized_unit_corrected(&chars, index + 1)
                    {
                        index = close.min(chars.len());
                        continue;
                    }
                }
                if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
                    if let Some(close) = skip_braced_case_pattern_unit(&chars, index + 1) {
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
                    continue;
                }
                if chars[index] == '\\' {
                    if run_start < index {
                        let run: String = chars[run_start..index].iter().collect();
                        output.push_str(&self.expand_case_pattern_run(&run));
                    }
                    // Emit the pair verbatim; a trailing lone `\` emits
                    // alone (the matcher reads it as a literal backslash).
                    output.push('\\');
                    if index + 1 < chars.len() {
                        output.push(chars[index + 1]);
                        index += 2;
                    } else {
                        index += 1;
                    }
                    run_start = index;
                    continue;
                }
                if chars[index] == crate::executor::markers::DATA_DQUOTE
                    || chars[index] == crate::executor::markers::DATA_SQUOTE
                {
                    // The lexer spells an escaped quote in word text as
                    // the data-quote carrier (markers.rs DATA_DQUOTE /
                    // DATA_SQUOTE): the backslash was consumed at read
                    // time and the quote is DATA — exactly GNU's
                    // CTLESC-marked char (parse.y:5358-5362
                    // got_escaped_character). Emit the matcher's escape
                    // spelling for the literal quote. The former scheme
                    // guarded this carrier as if it were a backslash,
                    // which collapsed `\"*` to `\*` (rubash#2421). The
                    // pattern-domain PATTERN_LITERAL_BACKSLASH reading
                    // of \x18 belongs to strings the parameter machinery
                    // hands the matcher directly, not to lexer word text
                    // passing through expansion here.
                    if run_start < index {
                        let run: String = chars[run_start..index].iter().collect();
                        output.push_str(&self.expand_case_pattern_run(&run));
                    }
                    output.push('\\');
                    output.push(if chars[index] == crate::executor::markers::DATA_DQUOTE {
                        '"'
                    } else {
                        '\''
                    });
                    index += 1;
                    run_start = index;
                    continue;
                }
                index += 1;
            }
            if run_start < chars.len() {
                let run: String = chars[run_start..].iter().collect();
                output.push_str(&self.expand_case_pattern_run(&run));
            }
            let restored = output.replace(PROTECTED_CASE_PATTERN_BACKSLASH, "\\");
            // No surrounding-quote strip: quotes at the edges of the
            // expanded text can only have arrived from an expansion
            // result, where they are data (rubash#2428).
            return std::borrow::Cow::Owned(restored);
        }

        let array_at = self.case_pattern_array_at_map(pattern_raw);
        std::borrow::Cow::Owned(quote_aware_case_pattern(
            pattern_raw,
            |word| self.expand_word_mut(word),
            &array_at,
        ))
    }

    /// One unquoted run of a quote-free case pattern (the text between
    /// two escape pairs): full mutable expansion plus the pattern quote
    /// decode, exactly the pipeline the former whole-string guard scheme
    /// applied to the protected text (rubash#2421). The decode runs in
    /// expansion-result mode: quote characters the expansion produced
    /// are data, not pattern quoting (rubash#2428).
    fn expand_case_pattern_run(&mut self, run: &str) -> String {
        let expanded = self.expand_word_mut(run);
        decode_case_pattern_expansion(&expanded)
    }

    /// First element + element count for every `${name[@]}` unit in a
    /// case pattern's raw text, in the engine's own `[@]` enumeration
    /// order (hash-slot order for associative arrays, index order for
    /// indexed ones — the order join_array_parameter_values joins in).
    /// Precomputed because quote_aware_case_pattern walks the pattern
    /// with only an expansion callback (rubash#1393).
    fn case_pattern_array_at_map(
        &self,
        raw: &str,
    ) -> std::collections::HashMap<String, (String, usize)> {
        let mut map = std::collections::HashMap::new();
        let mut rest = raw;
        while let Some((_, end, name)) = find_case_pattern_array_at(rest) {
            if !map.contains_key(&name) {
                let entry = self.case_pattern_array_at_first(&name);
                map.insert(name, entry);
            }
            rest = &rest[end..];
        }
        map
    }

    fn case_pattern_array_at_first(&self, name: &str) -> (String, usize) {
        let Some(value) = self.shell_state.env_vars.get(name) else {
            return (String::new(), 0);
        };
        if is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name) {
            let values = crate::executor::assignment_helpers::assoc_hash_ordered_values(
                value,
                crate::executor::assignment_helpers::assoc_nbuckets(
                    &self.shell_state.env_vars,
                    name,
                ),
            );
            let count = values.len();
            (values.into_iter().next().unwrap_or_default(), count)
        } else {
            let entries = crate::executor::indexed_array_entries_cached(value);
            let count = entries.len();
            (entries.values().next().cloned().unwrap_or_default(), count)
        }
    }

    pub(in crate::executor) fn execute_case_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        case_command: &CaseCommand,
    ) -> Result<(), ExecuteError> {
        // rubash#619: same pointer-walk fast path as if — GNU execute_cmd.c
        // execute_case_command walks the stored clauses by pointer and applies
        // redirections only when the statement carries them. Without redirect
        // fields the cmd + case_command deep clones and the no-op redirect
        // application only add per-execution churn; keep the ambient line
        // binding (line_number = case_command->line) semantics intact.
        // RESTRICTED_SHELL (#804): refuse before any splice or creation.
        let has_redirects = compound_command_has_redirects(cmd);
        if has_redirects {
            if let Some(message) = self.restricted_compound_redirect_error(cmd)? {
                self.write_default_stderr(&message)?;
                self.exit_code = 1;
                return Ok(());
            }
        }
        if !has_redirects {
            let case_line = cmd.line;
            let result = self.with_ambient_line(case_line, |executor| {
                executor.execute_case_command(case_command)
            });
            // rubash#2764 (rubash#1523 same rule): a case command owns
            // the `>(...)` bodies its word / patterns materialized —
            // finish them at the case's own completion (GNU forks them
            // at materialization, subst.c process_substitute), so a
            // case that is the last command still runs its output
            // bodies instead of leaving them for the next simple
            // command's drain — or never. Input fds already die here
            // via the per-command wrapper (command_execute.rs).
            let _ = self.finish_assignment_output_process_substitutions_for_command(cmd);
            return result;
        }

        // rubash#3454: same `set -u` unbound-target gate as `if` —
        // `case ... esac > $nope` aborts in GNU's compound
        // do_redirections before any expansion side effect.
        if self.compound_redirect_nounset_fatal(cmd)? {
            return Err(ExecuteError::ExitCode(1));
        }

        // rubash#2246: same borrow conversion as the if wrapper — the
        // cmd + case_command deep clones existed only for the procsub
        // materialization (gated on a `>(` mirror target) and the
        // per-clause splice, which prepare_case_command_redirects
        // replaces with the entry side effects alone.
        let mut redirect_owned;
        let group_outputs;
        let redirect_cmd: &CommandNode =
            if crate::executor::pipeline_exec::compound_redirect_needs_procsub_rewrite(cmd) {
                redirect_owned = cmd.clone();
                group_outputs =
                    self.materialize_compound_output_process_substitutions(&mut redirect_owned)?;
                &redirect_owned
            } else {
                group_outputs = Vec::new();
                cmd
            };
        let result = self.prepare_case_command_redirects(redirect_cmd);
        let status = self.exit_code;
        if let Err(error) = result {
            // rubash#846 (GNU execute_cmd.c:854-883): the case's OWN
            // redirect failure — mark it for the `!` flip sites.
            if matches!(error, ExecuteError::IoError(_)) {
                self.note_compound_redirect_failure();
            }
            let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
            self.exit_code = status;
            finish_result?;
            return Err(error);
        }
        // GNU execute_cmd.c:3653 `line_number = case_command->line`: the
        // `case` keyword's line is the ambient while a clause body runs.
        let case_line = cmd.line;
        let result = self.with_loop_fd_heredocs(cmd, |executor| {
            executor.with_command_input_redirects(cmd, |executor| {
                executor.with_ambient_line(case_line, |executor| {
                    executor.execute_case_command(case_command)
                })
            })
        });
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        let _ = self.finish_assignment_output_process_substitutions_for_command(cmd);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    /// rubash#2246: the entry half of the case command's redirect arms
    /// with the per-clause splice removed — the same arm admissions
    /// (is_stdout_splice_redirect / is_stdout_write_redirect /
    /// is_stderr_splice_redirect, out/err else-if exclusivity), the
    /// same expansions in the same order, and the same
    /// create_redirect_output entry creates. The clause bodies run
    /// against the fd table bound by with_command_input_redirects;
    /// GNU binds a case command's redirections once at entry like
    /// every compound (execute_cmd.c:854-883).
    fn prepare_case_command_redirects(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        if let Some(redirect) = cmd
            .redirect_out
            .as_deref()
            .filter(|r| is_stdout_splice_redirect(cmd, r))
        {
            let target = self.expand_redirect_target(redirect);
            if redirect_target_fd(&target).is_none() {
                self.create_redirect_output_for(Some(redirect), &target, redirect.clobber)?;
            }
        } else if let Some(redirect) = cmd
            .append
            .as_deref()
            .filter(|r| is_stdout_write_redirect(r))
        {
            let _target = self.expand_redirect_target(redirect);
        }

        if let Some(redirect) = cmd
            .redirect_err
            .as_deref()
            .filter(|r| is_stderr_splice_redirect(cmd, r))
        {
            let target = self.expand_redirect_target(redirect);
            if redirect_target_fd(&target).is_none() && !is_null_device(&target) {
                self.create_redirect_output_for(Some(redirect), &target, redirect.clobber)?;
            }
        } else if let Some(redirect) = cmd
            .redirect_err_append
            .as_deref()
            .filter(|r| is_stderr_splice_redirect(cmd, r))
        {
            let _target = self.expand_redirect_target(redirect);
        }

        Ok(())
    }
}
