//! Shared GNU-style parameter-brace scanning primitives.
//!
//! This module owns structural scanning only. It deliberately does not remove
//! shell quotes or expand parameter words.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DolbraceState {
    Param,
    Op,
    Word,
    Quote,
    Quote2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BraceContext {
    pub(crate) outer_double_quote: bool,
    pub(crate) posix: bool,
    pub(crate) replacement_context: bool,
    pub(crate) initial_state: DolbraceState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuoteEventKind {
    Single,
    Double,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QuoteEvent {
    pub(crate) offset: usize,
    pub(crate) kind: QuoteEventKind,
    pub(crate) state: DolbraceState,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct BracedScan {
    pub(crate) end: usize,
    pub(crate) final_state: DolbraceState,
    pub(crate) quote_events: Vec<QuoteEvent>,
}

pub(crate) fn scan_braced_parameter_body(input: &str, options: BraceContext) -> Option<BracedScan> {
    let mut wrapped = String::from("${");
    wrapped.push_str(input);
    let mut scan = scan_braced_parameter(&wrapped, options)?;
    scan.end = scan.end.saturating_sub(2);
    for event in &mut scan.quote_events {
        event.offset = event.offset.saturating_sub(2);
    }
    Some(scan)
}

/// Zero-allocation body scan over a char slice that starts *after* the
/// `${` prefix (the whole-buffer continuation scanner's hot path,
/// rubash#185: the &str API above would re-collect the rest of a
/// multi-kilobyte accumulated script buffer for every `${`).
/// `end`/`offsets` are CHAR indices into `chars`, matching the caller's
/// slice indexing.
pub(crate) fn scan_braced_parameter_body_chars(
    chars: &[char],
    options: BraceContext,
) -> Option<BracedScan> {
    let mut scan = scan_braced_chars_from(chars, 0, options)?;
    for event in &mut scan.quote_events {
        event.offset = event.offset.saturating_sub(2);
    }
    scan.end = scan.end.saturating_sub(2);
    Some(scan)
}

pub(crate) fn scan_braced_parameter(input: &str, options: BraceContext) -> Option<BracedScan> {
    if !input.starts_with("${") {
        return None;
    }
    let chars: Vec<char> = input.chars().collect();
    let scan = scan_braced_chars_from(&chars, 0, options)?;
    // Translate char indices to byte offsets for the &str API contract.
    let mut char_to_byte = Vec::with_capacity(scan.end + 1);
    let mut max_event = scan.end;
    for event in &scan.quote_events {
        max_event = max_event.max(event.offset);
    }
    // char_indices() yields BYTE offsets; enumerate() numbers CHARS. The
    // break must compare the CHAR ordinal against the char-indexed
    // max_event — comparing the byte offset instead truncates the table
    // whenever the body contains a multi-byte character, and translate()'s
    // input.len() fallback then reports the whole remainder as the span
    // (`x="${V:-<U+2714>}"` swallowed the closing quote and the rest of
    // the script, rubash#251 omb-prompt-base.sh:196). GNU parse.y
    // parse_matched_pair scans characters, not bytes.
    for (char_index, (byte, _)) in input.char_indices().enumerate() {
        if char_index > max_event {
            break;
        }
        char_to_byte.push(byte);
    }
    let translate =
        |index: usize| -> usize { char_to_byte.get(index).copied().unwrap_or(input.len()) };
    let mut scan = scan;
    scan.end = translate(scan.end);
    for event in &mut scan.quote_events {
        event.offset = translate(event.offset);
    }
    Some(scan)
}

/// The state machine shared by both entry points: GNU-style scanning of a
/// `${...}` span whose opening `${` sits at `start` of `chars`. Returns
/// CHAR indices (the closing-`}` position plus one, quote-event offsets).
fn scan_braced_chars_from(
    chars: &[char],
    start: usize,
    options: BraceContext,
) -> Option<BracedScan> {
    if chars.get(start) != Some(&'$') || chars.get(start + 1) != Some(&'{') {
        return None;
    }
    let mut cursor = start + 2usize;
    let mut depth = 1usize;
    let mut state = options.initial_state;
    let mut states = Vec::new();
    let mut single = false;
    let mut double = false;
    // GNU parse.y runs a fresh quote state machine for each nested ${...}.
    // Save and reset the outer quote state on entry so a `"` in the outer
    // pattern cannot keep an inner expansion's `}` from closing (e.g.
    // ${v%"${v#?}"}), and restore it when the inner expansion closes.
    let mut quote_stack: Vec<(bool, bool)> = Vec::new();
    let mut quote_events = Vec::new();
    while cursor < chars.len() {
        let ch = chars[cursor];
        cursor += 1;
        if ch == '\\' && !single {
            cursor = cursor.saturating_add(1);
            continue;
        }
        if ch == '`' && !single {
            cursor = skip_backtick(chars, cursor);
            continue;
        }
        if (ch == '<' || ch == '>') && chars.get(cursor).is_some_and(|next| *next == '(') && !single
        {
            cursor = skip_parenthesized(chars, cursor + 1);
            continue;
        }
        if ch == '$' && chars.get(cursor).is_some_and(|next| *next == '(') && !single {
            let open = if chars.get(cursor + 1).is_some_and(|next| *next == '(') {
                cursor + 1
            } else {
                cursor
            };
            cursor = skip_parenthesized(chars, open + 1);
            continue;
        }
        if ch == '$' && chars.get(cursor).is_some_and(|next| *next == '{') && !single {
            cursor += 1;
            states.push(state);
            quote_stack.push((double, single));
            double = false;
            single = false;
            depth += 1;
            state = match state {
                DolbraceState::Word | DolbraceState::Quote | DolbraceState::Quote2 => {
                    DolbraceState::Param
                }
                other => other,
            };
            continue;
        }
        // $'...' ANSI-C quoting: skip the entire string (handling \' escapes)
        // so the closing ' is not mistaken for a single-quote toggle, which
        // would prevent the real closing } from being found (nquote2.sub).
        if ch == '$' && chars.get(cursor).is_some_and(|next| *next == '\'') && !single && !double {
            cursor += 1; // skip the '
            while cursor < chars.len() {
                let quoted_ch = chars[cursor];
                cursor += 1;
                if quoted_ch == '\\' {
                    cursor = cursor.saturating_add(1); // skip escaped char
                    continue;
                }
                if quoted_ch == '\'' {
                    break;
                }
            }
            continue;
        }
        if ch == '}' && (options.replacement_context || (!single && !double)) {
            depth -= 1;
            if depth == 0 {
                return Some(BracedScan {
                    end: cursor,
                    final_state: state,
                    quote_events,
                });
            }
            state = states.pop().unwrap_or(DolbraceState::Param);
            (double, single) = quote_stack.pop().unwrap_or((false, false));
            continue;
        }
        if ch == '\'' && !double {
            quote_events.push(QuoteEvent {
                offset: cursor - 1,
                kind: QuoteEventKind::Single,
                state,
            });
            // GNU parse.y (Austin Group Interp 221): single quotes inside
            // `${...}` open a nested quoted string everywhere except in POSIX
            // mode inside double quotes while scanning the parameter,
            // operator, or word, where they are literal characters.
            let literal = options.outer_double_quote
                && options.posix
                && !matches!(state, DolbraceState::Quote | DolbraceState::Quote2);
            if !literal {
                single = !single;
            }
            continue;
        }
        if ch == '"' && !single {
            quote_events.push(QuoteEvent {
                offset: cursor - 1,
                kind: QuoteEventKind::Double,
                state,
            });
            double = !double;
            continue;
        }
        let operator = matches!(
            ch,
            '#' | '%' | '^' | ',' | '~' | ':' | '-' | '=' | '?' | '+' | '/'
        );
        match state {
            DolbraceState::Param if operator => {
                state = if matches!(ch, '%' | '#' | '^' | ',') {
                    DolbraceState::Quote
                } else if ch == '/' {
                    DolbraceState::Quote2
                } else {
                    DolbraceState::Op
                }
            }
            DolbraceState::Op if !operator => state = DolbraceState::Word,
            _ => {}
        }
    }
    None
}

fn skip_backtick(chars: &[char], mut cursor: usize) -> usize {
    while cursor < chars.len() {
        if chars[cursor] == '\\' {
            cursor = cursor.saturating_add(2);
        } else if chars[cursor] == '`' {
            return cursor + 1;
        } else {
            cursor += 1;
        }
    }
    chars.len()
}

fn skip_parenthesized(chars: &[char], mut cursor: usize) -> usize {
    let mut depth = 1usize;
    let mut quote = None;
    while cursor < chars.len() {
        let ch = chars[cursor];
        cursor += 1;
        if ch == '\\' {
            cursor = cursor.saturating_add(1);
            continue;
        }
        if let Some(active) = quote {
            // A single quote is literal while inside double quotes (and
            // vice versa); only the active quote closes this nested command.
            if ch == active {
                quote = None;
            }
            continue;
        }
        if ch == char::from_u32(39).unwrap() || ch == char::from_u32(34).unwrap() {
            quote = Some(ch);
        } else if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                return cursor;
            }
        }
    }
    chars.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    const PLAIN: BraceContext = BraceContext {
        outer_double_quote: false,
        posix: false,
        replacement_context: false,
        initial_state: DolbraceState::Param,
    };
    const OUTER_DOUBLE_DEFAULT: BraceContext = BraceContext {
        outer_double_quote: true,
        posix: false,
        replacement_context: false,
        initial_state: DolbraceState::Param,
    };
    const OUTER_DOUBLE_POSIX: BraceContext = BraceContext {
        outer_double_quote: true,
        posix: true,
        replacement_context: false,
        initial_state: DolbraceState::Param,
    };
    const PARAMETER_WORD: BraceContext = BraceContext {
        outer_double_quote: false,
        posix: false,
        replacement_context: false,
        initial_state: DolbraceState::Word,
    };
    const POSIX_DOUBLE_QUOTE_WORD: BraceContext = BraceContext {
        outer_double_quote: true,
        posix: true,
        replacement_context: false,
        initial_state: DolbraceState::Quote,
    };

    #[test]
    fn posix_dquote_interleaved_quotes_close_at_first_unquoted_brace() {
        // posixexp2 test 28 body: in POSIX mode inside double quotes the
        // big-hammer makes `'` literal, so the body closes at the FIRST `}`
        // after `x'` (GNU probe: the word ends before `}'x"'}"...`).
        let word = r#"${IFS+"'"x ~ x'}'x"'}"x}" #'"#;
        let scan = scan_braced_parameter(word, OUTER_DOUBLE_POSIX).unwrap();
        assert_eq!(&word[..scan.end], "${IFS+\"'\"x ~ x'}");
    }

    #[test]
    fn initial_parameter_word_state_is_preserved() {
        let scan = scan_braced_parameter("${name}", PARAMETER_WORD).unwrap();
        assert_eq!(scan.final_state, DolbraceState::Word);
    }

    #[test]
    fn initial_posix_double_quote_state_records_operator_quotes() {
        let scan = scan_braced_parameter("${IFS+'}'z}", POSIX_DOUBLE_QUOTE_WORD).unwrap();
        assert_eq!(scan.end, "${IFS+'}'z}".len());
        assert_eq!(scan.quote_events.len(), 2);
    }

    #[test]
    fn scans_nested_parameter_and_restores_outer_state() {
        let input = "${A[${i}]}";
        let scan = scan_braced_parameter(input, PLAIN).unwrap();
        assert_eq!(scan.end, input.len());
        assert_eq!(scan.final_state, DolbraceState::Param);
    }
    #[test]
    fn escaped_closing_brace_does_not_close() {
        let input = "${x:-\\}}";
        let scan = scan_braced_parameter(input, PLAIN).unwrap();
        assert_eq!(scan.end, input.len());
    }
    #[test]
    fn closing_brace_inside_bracket_pattern_closes_expression() {
        // GNU parse.y uses P_FIRSTCLOSE for ${...}: the first unquoted '}'
        // closes the expression, even inside a bracket pattern. The
        // remaining `]}` is literal text outside the expansion.
        let input = "${o%[}]}";
        let scan = scan_braced_parameter(input, PLAIN).unwrap();
        assert_eq!(scan.end, 6);
    }

    #[test]
    fn skips_opaque_command_and_arithmetic_substitutions() {
        let input = "${x:-$(printf '}') $((1 + 2)) `printf '}'`}";
        let scan = scan_braced_parameter(input, PLAIN).unwrap();
        assert_eq!(scan.end, input.len());
    }

    #[test]
    fn posix_double_quote_literalizes_operator_quotes() {
        // GNU parse.y (Interp 221): in POSIX mode inside double quotes,
        // single quotes in PARAM/OP/WORD state are literal, so the first
        // unquoted `}` closes; outside POSIX mode they open nested quotes,
        // so the same input is unterminated.
        let input = "${x:-'}";
        assert!(scan_braced_parameter(input, OUTER_DOUBLE_DEFAULT).is_none());
        assert!(scan_braced_parameter(input, OUTER_DOUBLE_POSIX).is_some());
    }

    #[test]
    fn records_operator_word_quote_metadata() {
        let input = "${IFS+'}'z}";
        let scan = scan_braced_parameter(input, OUTER_DOUBLE_POSIX).unwrap();
        // POSIX+dquote Op state: the quote is literal and the expansion
        // closes at the first `}`.
        assert_eq!(scan.end, "${IFS+'}".len());
        assert_eq!(scan.quote_events.len(), 1);
        assert_eq!(scan.quote_events[0].kind, QuoteEventKind::Single);
        assert_eq!(scan.quote_events[0].state, DolbraceState::Op);
    }

    #[test]
    fn nested_pattern_quote_does_not_block_inner_brace() {
        // Outer pattern quote `"` must not keep the inner ${v#?} `}` from
        // closing: ${v%"${v#?}"} has a fresh quote scope per nested ${...}.
        let input = "${v%\"${v#?}\"}";
        let scan = scan_braced_parameter(input, OUTER_DOUBLE_DEFAULT).unwrap();
        assert_eq!(scan.end, input.len());
    }

    #[test]
    fn adjacent_expansions_close_at_their_own_brace() {
        let input = "IFS+'a'bc}\n${IFS+'}'z}";
        let scan = scan_braced_parameter_body(input, PLAIN).unwrap();
        assert_eq!(scan.end, "IFS+'a'bc}".len());
        let scan = scan_braced_parameter_body("IFS+'}'z}", PLAIN).unwrap();
        assert_eq!(scan.end, "IFS+'}'z}".len());
    }
}
