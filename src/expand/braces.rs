//! Brace expansion for {a,b,c} comma-separated lists and {1..5}/{a..e} sequences.
//!
//! GNU Bash source ownership:
// - braces.c

/// Expand brace patterns in a word, returning multiple words.
/// Handles {a,b,c} comma-separated lists and {1..5}, {a..e} sequences.
/// Returns a single-element vec if no braces found (no expansion needed).
pub fn expand_braces(word: &str) -> Vec<String> {
    brace_expand(word)
}

/// One product of brace expansion, with the one fact about it that the
/// plain text cannot carry: whether an empty product is a word that
/// nevertheless exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BraceProduct {
    pub text: String,
    /// True when this product is empty but is NOT a bare alternative
    /// null: it descends from a character sequence's backslash element.
    /// GNU braces.c emits one word per codepoint in a sequence; at the
    /// backslash position the word's value is empty, yet the word itself
    /// is kept through expansion (braces.tests: `{a..A}` yields 33
    /// words, the backslash position an empty one — the snapshot line
    /// shows `]  [`). A bare alternative's empty member (`{a,}`) is the
    /// opposite kind: an unquoted null that word expansion discards
    /// (rubash#337). Composition rule: a concatenated product carries
    /// the flag only when its whole text is empty and one of its factors
    /// carried it — glued to any real text the element simply
    /// contributes nothing, like GNU's word at that position dequoting
    /// away inside a larger word.
    pub sequence_null: bool,
}

/// Brace expansion that reports, per product, whether an empty product
/// is a sequence null (see BraceProduct::sequence_null). Text results
/// are identical to expand_braces.
pub(crate) fn expand_braces_tagged(word: &str) -> Vec<BraceProduct> {
    brace_expand_tagged(word)
}

fn brace_expand(text: &str) -> Vec<String> {
    brace_expand_tagged(text)
        .into_iter()
        .map(|product| product.text)
        .collect()
}

fn brace_expand_tagged(text: &str) -> Vec<BraceProduct> {
    // GNU braces.c brace_expand recursion
    let Some((open, close, etype)) = find_first_valid_brace(text) else {
        return vec![BraceProduct {
            text: text.to_string(),
            sequence_null: false,
        }];
    };
    let preamble = &text[..open];
    let amble = &text[open + 1..close];
    let postamble = &text[close + 1..];

    let tack: Vec<BraceProduct> = if etype == BRACE_SEQ {
        // valid_seqterm already passed, try expand_seqterm
        if let Some(seq) = expand_range_tagged(amble) {
            seq
        } else {
            // GNU: if sequence expansion fails (overflow etc) and postamble non-empty,
            // treat the whole brace as literal and recurse on postamble.
            // If postamble empty, return literal whole text.
            if !postamble.is_empty() {
                let mut literal = Vec::new();
                literal.push(BraceProduct {
                    text: format!("{{{}}}", amble),
                    sequence_null: false,
                });
                literal
            } else {
                return vec![BraceProduct {
                    text: text.to_string(),
                    sequence_null: false,
                }];
            }
        }
    } else if etype == BRACE_COMMA {
        expand_amble_tagged(amble)
    } else {
        vec![BraceProduct {
            text: format!("{{{}}}", amble),
            sequence_null: false,
        }]
    };

    // array_concat(preamble, tack)
    let mut result: Vec<BraceProduct> = Vec::new();
    if preamble.is_empty() && tack.is_empty() {
        result.push(BraceProduct {
            text: String::new(),
            sequence_null: false,
        });
    } else if preamble.is_empty() {
        result.extend(tack.clone());
    } else if tack.is_empty() {
        result.push(BraceProduct {
            text: preamble.to_string(),
            sequence_null: false,
        });
    } else {
        // preamble is single string, tack is Vec<BraceProduct>
        for t in &tack {
            let text = format!("{}{}", preamble, t.text);
            result.push(BraceProduct {
                sequence_null: text.is_empty() && t.sequence_null,
                text,
            });
        }
    }
    // handle empty tack case where result is just preamble?
    if result.is_empty() {
        result.push(BraceProduct {
            text: preamble.to_string(),
            sequence_null: false,
        });
    }

    if postamble.is_empty() {
        return result;
    }
    let post_expanded = brace_expand_tagged(postamble);
    // array_concat(result, post_expanded)
    let mut final_result = Vec::new();
    for r in &result {
        for p in &post_expanded {
            let text = format!("{}{}", r.text, p.text);
            final_result.push(BraceProduct {
                sequence_null: text.is_empty() && (r.sequence_null || p.sequence_null),
                text,
            });
        }
    }
    // handle cases where result or post_expanded is empty?
    if final_result.is_empty() {
        return result;
    }
    final_result
}

fn expand_amble_tagged(amble: &str) -> Vec<BraceProduct> {
    let parts = split_brace_commas(amble);
    // GNU braces.c expand_amble: every top-level alternative is brace-expanded
    // and the results are appended (array_concat) — a list, not a cross
    // product. Empty alternatives are KEPT as empty elements here; the
    // resulting bare empty word is dropped later by ordinary unquoted
    // empty-word removal. That is what makes
    //   a{,}b  -> [ab][ab]   {a,}x -> [ax][x]   x{a,} -> [xa][x]
    // while
    //   {a,}   -> [a]        (the trailing empty word is discarded)
    // and
    //   {,}    -> one empty word (printf '<%s>' prints <>)
    // A member that is itself a brace group keeps its products' flags:
    // in {{a..A},} the sequence member's null stays a sequence null,
    // distinct from the bare empty alternative beside it.
    let mut result: Vec<BraceProduct> = Vec::new();
    for part in parts {
        result.extend(brace_expand_tagged(part));
    }
    if result.is_empty() {
        result.push(BraceProduct {
            text: String::new(),
            sequence_null: false,
        });
    }
    result
}

const BRACE_COMMA: i32 = 0x01;
const BRACE_SEQ: i32 = 0x02;
// const BRACE_NONE: i32 = 0x04;

fn valid_seqterm(amble: &str) -> bool {
    // Port of braces.c valid_seqterm minimal check
    let Some(pos) = amble.find("..") else {
        return false;
    };
    let lhs = &amble[..pos];
    let rhs = &amble[pos + 2..];
    if lhs.is_empty() || rhs.is_empty() {
        return false;
    }
    if lhs.starts_with('.') || rhs.starts_with('}') {
        return false;
    }
    // lhs type
    let lhs_is_int = lhs
        .as_bytes()
        .first()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
        || ((lhs.starts_with('+') || lhs.starts_with('-'))
            && lhs
                .as_bytes()
                .get(1)
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false));
    let lhs_is_char = lhs.len() == 1 && lhs.as_bytes()[0].is_ascii_alphabetic();
    if !lhs_is_int && !lhs_is_char {
        return false;
    }
    // rhs type: need to handle possible increment part ".."
    // Extract first segment of rhs before next ".." if any
    let rhs_first = if let Some(p) = rhs.find("..") {
        &rhs[..p]
    } else {
        rhs
    };
    if rhs_first.is_empty() {
        return false;
    }
    let rhs_is_int = rhs_first
        .as_bytes()
        .first()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
        || ((rhs_first.starts_with('+') || rhs_first.starts_with('-'))
            && rhs_first
                .as_bytes()
                .get(1)
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false));
    let rhs_is_char = rhs_first.len() == 1 && rhs_first.as_bytes()[0].is_ascii_alphabetic();
    if !rhs_is_int && !rhs_is_char {
        return false;
    }
    // Types must match (both int or both char)
    let lhs_is_int_type = lhs_is_int;
    let rhs_is_int_type = rhs_is_int;
    if lhs_is_int_type != rhs_is_int_type {
        return false;
    }
    // Also ensure rhs segment is not empty and not starting with '}'
    true
}

/// UTF-8 bytes of markers.rs ASSIGN_DATA_DQUOTE (U+E302) — the stored form
/// of a `"` in de-quoted assignment text.
const DQUOTE_CARRIER: &[u8] = crate::executor::markers::ASSIGN_DATA_DQUOTE_STR.as_bytes();
/// UTF-8 bytes of markers.rs ASSIGN_DATA_SQUOTE (U+E301) — the stored form
/// of a `'` in de-quoted assignment text.
const SQUOTE_CARRIER: &[u8] = crate::executor::markers::ASSIGN_DATA_SQUOTE_STR.as_bytes();

/// UTF-8 bytes of markers.rs ASSIGN_HOISTED_SQUOTE (U+E307) — the compound
/// -assignment hoisted single-quote DELIMITER: the lexer wraps a `'...'`
/// span's data in a pair of these sentinels, so the pair plays the role
/// GNU braces.c:729-734 gives the two `'` characters — everything between
/// them is opaque quoted data that brace syntax never sees
/// (niubash#1060: `a=('X{16,}Y')` stored the span E307-wrapped and the
/// scanner brace-expanded inside the quotes).
const HOISTED_SQUOTE_CARRIER: &[u8] =
    crate::executor::markers::ASSIGN_HOISTED_SQUOTE_STR.as_bytes();

/// True for a `$` byte OR its cooked-word carrier DATA_DOLLAR (U+001F,
/// markers.rs): quoting already turned this dollar into data, so the brace
/// scanners must give a following `{` the same treatment GNU gives `${` —
/// braces.c:679-680 brace_gobbler "treat ${...} like \{...}" — and never
/// read it as brace syntax. A cooked `'${x,y}'` word arrives here as
/// `\x1f{x,y}`; without this predicate the scanner expands the member list
/// that quoting made literal.
fn is_dollar_or_carrier(byte: u8) -> bool {
    byte == b'$' || byte == DATA_DOLLAR_BYTE
}

/// UTF-8 byte of markers.rs DATA_DOLLAR (U+001F) — single-byte C0 carrier.
const DATA_DOLLAR_BYTE: u8 = 0x1f;

/// Returns the quote carrier starting at `i`, if any — the assignment
/// storage sentinels whose PAIRS delimit the quoted spans GNU braces.c
/// brace_gobbler skips (E301/E302 data quotes, E307 hoisted `'` span).
fn data_quote_carrier_at(bytes: &[u8], i: usize) -> Option<&'static [u8]> {
    if bytes.len() >= i + DQUOTE_CARRIER.len()
        && &bytes[i..i + DQUOTE_CARRIER.len()] == DQUOTE_CARRIER
    {
        Some(DQUOTE_CARRIER)
    } else if bytes.len() >= i + SQUOTE_CARRIER.len()
        && &bytes[i..i + SQUOTE_CARRIER.len()] == SQUOTE_CARRIER
    {
        Some(SQUOTE_CARRIER)
    } else if bytes.len() >= i + HOISTED_SQUOTE_CARRIER.len()
        && &bytes[i..i + HOISTED_SQUOTE_CARRIER.len()] == HOISTED_SQUOTE_CARRIER
    {
        Some(HOISTED_SQUOTE_CARRIER)
    } else {
        None
    }
}

/// Skips a DATA-quote span opened by `carrier`, mirroring GNU braces.c
/// brace_gobbler's quoted-state skip: everything until the same carrier
/// recurs is opaque. An unterminated span runs to the end of the word,
/// after which no valid brace group can follow.
fn skip_data_quote_carrier(bytes: &[u8], i: usize, carrier: &[u8]) -> usize {
    let mut j = i + carrier.len();
    while j + carrier.len() <= bytes.len() {
        if &bytes[j..j + carrier.len()] == carrier {
            return j + carrier.len();
        }
        j += 1;
    }
    bytes.len()
}

fn find_first_valid_brace(text: &str) -> Option<(usize, usize, i32)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut escaped = false;

    while i < bytes.len() {
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if bytes[i] == b'\\' {
            escaped = true;
            i += 1;
            continue;
        }
        // Assignment-storage DATA quotes ride as the PUA sentinels
        // E301 (') / E302 (") because the lexer already de-quoted the word
        // (markers.rs ASSIGN_DATA_SQUOTE / ASSIGN_DATA_DQUOTE). GNU
        // braces.c:189-193 brace_gobbler opens quoted state on those
        // characters in the RAW word and skips everything until the same
        // character closes it — a comma or brace inside a quoted brace
        // member never splits (`a=({","})` stays one literal element). The
        // sentinels carry exactly that quote through rubash's stored form,
        // so the scanner must honor them as quote openers the same way.
        if let Some(carrier) = data_quote_carrier_at(bytes, i) {
            i = skip_data_quote_carrier(bytes, i, carrier);
            continue;
        }
        // Skip single-quoted strings
        if bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        // Skip double-quoted strings
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        // GNU bash runs brace expansion on the word itself; the text of a
        // command substitution is a separate parse unit and the outer shell
        // must not split braces inside it (parse.y read_token_word hands the
        // $()/backtick body to parse_comsub, never to brace_expand). Skip
        // the whole substitution body here.
        if bytes[i] == b'`' {
            i = skip_backtick_body(bytes, i).unwrap_or(bytes.len());
            continue;
        }
        if (bytes[i] == b'<' || bytes[i] == b'>') && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            i = skip_dollar_paren_body(bytes, i);
            continue;
        }
        if is_dollar_or_carrier(bytes[i]) {
            if i + 1 < bytes.len() && bytes[i + 1] == b'(' {
                i = skip_dollar_paren_body(bytes, i);
                continue;
            }
            if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                // $'...' ANSI-C quoting: the single quote opens a quoted
                // unit, not a quoting state toggle for later bytes.
                i = skip_single_quoted(bytes, i + 1);
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                // Skip the whole ${...} parameter body: its closing brace is
                // part of the expansion (subst.c extract_dollar_brace_string),
                // never the closing brace of an enclosing brace group, and a
                // comma inside it (foo{bar,${var}.}) does not split. The
                // DATA_DOLLAR carrier form (cooked word) is the same body.
                i = skip_dollar_brace_body(bytes, i);
                continue;
            }
        }
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        // Bash ignores a brace opener at a word boundary when it is followed
        // by whitespace or a closing brace; this keeps `{ a,b}` literal.
        let preceded_by_whitespace = i == 0 || bytes[i - 1].is_ascii_whitespace();
        let followed_by_whitespace_or_close =
            i + 1 >= bytes.len() || bytes[i + 1].is_ascii_whitespace() || bytes[i + 1] == b'}';
        if preceded_by_whitespace && followed_by_whitespace_or_close {
            i += 1;
            continue;
        }
        // Skip ${...} parameter expansions (and the DATA_DOLLAR carrier
        // form of the same opener in cooked words).
        if i > 0 && is_dollar_or_carrier(bytes[i - 1]) {
            i += 1;
            continue;
        }

        let inner_start = i + 1;
        let mut depth = 1u32;
        let mut j = inner_start;
        let mut has_comma = false;
        let mut has_double_dot = false;
        let mut j_escaped = false;
        let mut j_single = false;
        let mut j_double = false;

        while j < bytes.len() && depth > 0 {
            if j_escaped {
                j_escaped = false;
                j += 1;
                continue;
            }
            if bytes[j] == b'\'' && !j_double {
                j_single = !j_single;
                j += 1;
                continue;
            }
            if bytes[j] == b'"' && !j_single {
                j_double = !j_double;
                j += 1;
                continue;
            }
            if let Some(carrier) = data_quote_carrier_at(bytes, j) {
                // GNU braces.c brace_gobbler: inside quote state only the
                // same quote character closes it; the other quote kind is
                // data and never opens nested state. The hoisted E307
                // single-quote delimiter toggles the same `'` state.
                if carrier == DQUOTE_CARRIER && !j_single {
                    j_double = !j_double;
                } else if (carrier == SQUOTE_CARRIER || carrier == HOISTED_SQUOTE_CARRIER)
                    && !j_double
                {
                    j_single = !j_single;
                }
                j += carrier.len();
                continue;
            }
            if j_single || j_double {
                j += 1;
                continue;
            }
            match bytes[j] {
                b'\\' => {
                    j_escaped = true;
                    j += 1;
                    continue;
                }
                c if is_dollar_or_carrier(c) && j + 1 < bytes.len() && bytes[j + 1] == b'{' => {
                    j = skip_dollar_brace_body(bytes, j);
                    continue;
                }
                c if is_dollar_or_carrier(c) && j + 1 < bytes.len() && bytes[j + 1] == b'(' => {
                    j = skip_dollar_paren_body(bytes, j);
                    continue;
                }
                b'<' | b'>' if j + 1 < bytes.len() && bytes[j + 1] == b'(' => {
                    j = skip_dollar_paren_body(bytes, j);
                    continue;
                }
                b'`' => {
                    j = skip_backtick_body(bytes, j).unwrap_or(bytes.len());
                    continue;
                }
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                b',' if depth == 1 => has_comma = true,
                b'.' if depth == 1 && j + 1 < bytes.len() && bytes[j + 1] == b'.' => {
                    has_double_dot = true;
                }
                _ => {}
            }
            j += 1;
        }

        if depth != 0 {
            i += 1;
            continue;
        }

        let inner = &text[inner_start..j];
        let etype = if has_comma {
            BRACE_COMMA
        } else if has_double_dot {
            BRACE_SEQ
        } else {
            0 // BRACE_NONE
        };
        if etype == 0 {
            i += 1;
            continue;
        }
        if etype == BRACE_SEQ && !valid_seqterm(inner) {
            // GNU valid_seqterm check: skip invalid sequence brace
            i += 1;
            continue;
        }
        return Some((i, j, etype));
    }
    None
}

/// Index just past the matching close brace of the ${ at `start` (which
/// points at the $). Tracks brace nesting while staying inside quoted
/// spans, mirroring subst.c extract_dollar_brace_string's search.
fn skip_dollar_brace_body(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 2;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\\' && !single {
            i += 2;
            continue;
        }
        if ch == b'\'' && !double {
            single = !single;
        } else if ch == b'"' && !single {
            double = !double;
        } else if !single && !double {
            if ch == b'{' {
                depth += 1;
            } else if ch == b'}' {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
        }
        i += 1;
    }
    bytes.len()
}

/// Index just past the backquote closing the `` ` `` at `start`
/// (string_extract's `` ` `` terminator honoring only `\x` escapes,
/// subst.c:791-844), or None when no closing backquote exists. The
/// Option distinguishes a pair whose close is the last byte from an
/// unclosed open — expand_word_internal's bare-backquote compatibility
/// test (`sindex - 1 == t_index`, subst.c:11574-11576) needs that.
fn skip_backtick_body(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

/// Index just past the matching close paren of the $( at `start` (which
/// points at the $). Tracks parenthesis nesting while staying inside
/// single- and double-quoted spans, mirroring parse.y parse_comsub's
/// matching-paren search. A backquote group is its own quote state
/// (parse.y:3877 parse_matched_pair's grouping branch does not parse
/// constructs inside a backquote body, so a quote inside it is ordinary
/// text): track it like `skip_compound_subscript_span` does, so an
/// unbalanced `"` inside `` `...` `` cannot leak into the paren count
/// (rubash#3435: `{$(echo `echo "q`),x}` must still brace-expand).
fn skip_dollar_paren_body(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 2;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut in_backtick = false;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\\' && !single {
            i += 2;
            continue;
        }
        if ch == b'\'' && !double && !in_backtick {
            single = !single;
        } else if ch == b'"' && !single && !in_backtick {
            double = !double;
        } else if ch == b'`' && !single {
            in_backtick = !in_backtick;
        } else if !single && !double && !in_backtick {
            if ch == b'(' {
                depth += 1;
            } else if ch == b')' {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
        }
        i += 1;
    }
    bytes.len()
}

/// Index just past the closing single quote starting at `start`.
fn skip_single_quoted(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'\'' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// Index just past the `}` closing the `${` at `start` (which points at
/// the `$`), or end when unclosed — the backquote-aware twin of
/// `skip_dollar_brace_body`: GNU delimits the parameter body through
/// extract_dollar_brace_string -> extract_delimited_string
/// (subst.c:1361), which passes `` `...` `` bodies, `'...'` and `"..."`
/// spans through verbatim (subst.c:1469-1486), so a `}` or `{` inside
/// any of those units is data. A backquote group is its own quote
/// state: a quote inside it is ordinary text.
fn skip_dollar_brace_body_expansion(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 2;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut in_backtick = false;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\\' && !single {
            i += 2;
            continue;
        }
        if ch == b'\'' && !double && !in_backtick {
            single = !single;
        } else if ch == b'"' && !single && !in_backtick {
            double = !double;
        } else if ch == b'`' && !single {
            in_backtick = !in_backtick;
        } else if !single && !double && !in_backtick {
            if ch == b'{' {
                depth += 1;
            } else if ch == b'}' {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
        }
        i += 1;
    }
    bytes.len()
}

/// Byte index in `text` of the backquote GNU's expansion scan would
/// reject, if any — the product-level port of `expand_word_internal`'s
/// `` ` `` case (subst.c:11568-11593). brace_expand_word_list hands
/// every brace product to expansion as a fresh word (subst.c:12750-
/// 12763 -> make_word_flags, make_cmd.c:101); the expansion scan then
/// string_extracts from a raw backquote to its closing backquote
/// honoring ONLY backslash escapes (string_extract, subst.c:791 — a
/// quote byte inside a backquote body is ordinary text). SX_REQMATCH
/// turns an unclosed backquote into the expansion failure
/// `bad substitution: no closing "`" in …` — expand_word_error, which
/// abandons the whole word list (subst.c:13093-13110
/// exp_jump_to_top_level(DISCARD)) — UNLESS the backquote is the word's
/// last character, which passes through as literal data for backwards
/// compatibility ("bare instances of `", subst.c:11574-11576).
///
/// The scan mirrors expand_word_internal's top-level cases for every
/// byte that changes where a backquote is seen:
/// - `\x` quotes x — never opens a construct.
/// - `'...'` is one quoted unit; an unclosed `'` swallows the rest of
///   the word as quoted data (string_extract_single_quoted), so a
///   backquote inside single quotes is never reached.
/// - `"..."` keeps backquotes live: string_extract_double_quoted hands
///   the quoted content to a recursive expand_word_internal
///   (subst.c:11692-11724) where the same `` ` `` case applies, and a
///   `"` cannot close while a backquote span is open. An unclosed
///   inner backquote therefore reports the same tail-to-end-of-word
///   the recursive call's string+t_index shows.
/// - `$(...)`, `${...}`, `$'...'` and `<(...)`/`>(...)` are extracted
///   atomically by their own cases (extract_delimited_string passes
///   `` ` `` bodies through verbatim, subst.c:1469-1474); a backquote
///   inside belongs to that body's own parse, never this scan.
///
/// Data-carrier bytes (DATA_DOLLAR, the PUA quote sentinels) are
/// literal text here — none of them opens a construct for this scan.
pub(crate) fn unclosed_backquote_index(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\'' => {
                // Plain single quote: everything to the next `'` (or to
                // the end of the word) is quoted data.
                match bytes[i + 1..].iter().position(|&b| b == b'\'') {
                    Some(offset) => i += offset + 2,
                    None => return None,
                }
            }
            b'"' => match double_quoted_unclosed_backquote(bytes, i + 1) {
                DoubleQuotedScan::Closed(end) => i = end,
                DoubleQuotedScan::Unclosed => return None,
                DoubleQuotedScan::Backquote(at) => return Some(at),
            },
            b'`' => match skip_backtick_body(bytes, i) {
                Some(close) => i = close,
                // Bare trailing backquote: literal data (subst.c:11574).
                None if i + 1 == bytes.len() => i += 1,
                None => return Some(i),
            },
            b'$' if i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                i = skip_dollar_paren_body(bytes, i);
            }
            b'$' if i + 1 < bytes.len() && bytes[i + 1] == b'{' => {
                i = skip_dollar_brace_body_expansion(bytes, i);
            }
            b'$' if i + 1 < bytes.len() && bytes[i + 1] == b'\'' => {
                i = skip_single_quoted(bytes, i + 1);
            }
            b'<' | b'>' if i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                i = skip_dollar_paren_body(bytes, i);
            }
            _ => i += 1,
        }
    }
    None
}

/// Result of scanning the inside of a `"..."` span for the
/// `unclosed_backquote_index` port.
enum DoubleQuotedScan {
    /// Index just past the closing `"`.
    Closed(usize),
    /// The `"` never closed; the rest of the word is quoted content and
    /// no unclosed backquote was seen inside it.
    Unclosed,
    /// Byte index of a backquote the recursive expansion scan of the
    /// quoted content would reject.
    Backquote(usize),
}

/// Scan the inside of a `"..."` span (starting just past the opening
/// `"` at `open`) for `unclosed_backquote_index`. Mirrors
/// string_extract_double_quoted's discipline (subst.c:846+): `\x` skips
/// x, a `"` outside an open `` ` `` span closes, and `` ` `` bodies are
/// their own unit — inside the double-quoted content a single quote is
/// ordinary data (subst.c:11882-11886) but `$`/`(`, `${` and backquote
/// expansions are live.
fn double_quoted_unclosed_backquote(bytes: &[u8], open: usize) -> DoubleQuotedScan {
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return DoubleQuotedScan::Closed(i + 1),
            b'`' => match skip_backtick_body(bytes, i) {
                Some(close) => i = close,
                None if i + 1 == bytes.len() => i += 1,
                None => return DoubleQuotedScan::Backquote(i),
            },
            b'$' if i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                i = skip_dollar_paren_body(bytes, i);
            }
            b'$' if i + 1 < bytes.len() && bytes[i + 1] == b'{' => {
                i = skip_dollar_brace_body_expansion(bytes, i);
            }
            _ => i += 1,
        }
    }
    DoubleQuotedScan::Unclosed
}

fn split_brace_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0u32;
    let mut start = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut escaped = false;
    // GNU braces.c:638 brace_gobbler searches for splitting commas outside
    // quoted spans (`'...'`, `"..."`, `` `...` ``) and command/parameter
    // substitution units (`$(...)`, `<(...)`, `>(...)`, `${...}`, `$'...'`),
    // so a comma or unmatched quote inside those units does not split or
    // poison the outer brace group.
    let mut single = false;
    let mut double = false;
    let mut backtick = false;
    while i < bytes.len() {
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if backtick {
            if bytes[i] == b'\\' {
                escaped = true;
            } else if bytes[i] == b'`' {
                backtick = false;
            }
            i += 1;
            continue;
        }
        match bytes[i] {
            b'\\' => {
                escaped = true;
                i += 1;
                continue;
            }
            b'\'' if !double => single = !single,
            b'"' if !single => double = !double,
            b'`' if !single && !double => backtick = true,
            // Data-quote sentinels (markers.rs E301/E302) and the hoisted
            // single-quote delimiter (E307) are the stored form of the
            // quotes GNU braces.c skips while splitting commas
            // (braces.c:712-734 quoted-state handling in brace_gobbler).
            // Only the same carrier kind toggles its state; the other kind
            // is data inside the active quote.
            b'\xEE' if data_quote_carrier_at(bytes, i).is_some() => {
                match data_quote_carrier_at(bytes, i) {
                    Some(DQUOTE_CARRIER) if !single => double = !double,
                    Some(SQUOTE_CARRIER) | Some(HOISTED_SQUOTE_CARRIER) if !double => {
                        single = !single
                    }
                    _ => {}
                }
                i += DQUOTE_CARRIER.len();
                continue;
            }
            c if !single
                && !double
                && is_dollar_or_carrier(c)
                && i + 1 < bytes.len()
                && bytes[i + 1] == b'(' =>
            {
                i = skip_dollar_paren_body(bytes, i);
                continue;
            }
            b'<' | b'>' if !single && !double && i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                i = skip_dollar_paren_body(bytes, i);
                continue;
            }
            c if !single
                && !double
                && is_dollar_or_carrier(c)
                && i + 1 < bytes.len()
                && bytes[i + 1] == b'{' =>
            {
                i = skip_dollar_brace_body(bytes, i);
                continue;
            }
            b'$' if !single && !double && i + 1 < bytes.len() && bytes[i + 1] == b'\'' => {
                i = skip_single_quoted(bytes, i + 1);
                continue;
            }
            b'{' if !single && !double => depth += 1,
            b'}' if !single && !double => depth = depth.saturating_sub(1),
            b',' if depth == 0 && !single && !double => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&s[start..]);
    parts
}

fn expand_range(s: &str) -> Option<Vec<String>> {
    expand_range_tagged(s)
        .map(|products| products.into_iter().map(|product| product.text).collect())
}

fn expand_range_tagged(s: &str) -> Option<Vec<BraceProduct>> {
    let parts = s.split("..").collect::<Vec<_>>();
    let ([left, right] | [left, right, _]) = parts.as_slice() else {
        return None;
    };
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let step = match parts.as_slice() {
        [_, _] => 1,
        [_, _, step] => step.parse::<i64>().ok()?.abs().max(1),
        _ => return None,
    };

    // Numeric range
    if let (Ok(start), Ok(end)) = (left.parse::<i64>(), right.parse::<i64>()) {
        let width = numeric_range_width(left, right);
        // Port of GNU braces.c mkseq (bash 5.3 braces.c:373-472): precompute
        // the element count with checked arithmetic (any overflow -> literal
        // fallback, same as GNU returning NULL from mkseq), then emit
        // exactly `count` values. The increment only happens BETWEEN
        // elements, never after the last one — incrementing past `end`
        // would overflow i64 for end == i64::MAX ({MAX-2..MAX}).
        let abs_incr = step;
        let incr: i64 = if start <= end { abs_incr } else { -abs_incr };
        let prevn: i64 = if start < end {
            match end.checked_sub(start) {
                Some(v) => v,
                None => return None,
            }
        } else {
            match start.checked_sub(end) {
                Some(v) => v,
                None => return None,
            }
        };
        let count: usize = match usize::try_from(prevn / abs_incr) {
            Ok(q) => match q.checked_add(1) {
                Some(c) => c,
                None => return None,
            },
            Err(_) => return None,
        };
        let mut result = Vec::with_capacity(count);
        let mut n = start;
        for i in 0..count {
            result.push(BraceProduct {
                text: format_numeric_range_value(n, width),
                sequence_null: false,
            });
            if i + 1 < count {
                // Invariant: n stays within [min(start,end), max(start,end)],
                // so this cannot overflow (same invariant GNU relies on).
                n += incr;
            }
        }
        return Some(result);
    }
    // Alpha range
    let start = left.as_bytes()[0];
    let end = right.as_bytes()[0];
    if left.len() == 1
        && right.len() == 1
        && start.is_ascii_alphabetic()
        && end.is_ascii_alphabetic()
    {
        // GNU braces.c mkseq ST_CHAR path: same count-based loop as the
        // numeric range (count = |end-start|/incr + 1). count > 1 implies
        // step <= 57, so the `step as i32` cast below cannot truncate.
        let diff = (start as i32 - end as i32).unsigned_abs() as usize;
        let step_u = usize::try_from(step).unwrap_or(usize::MAX);
        let count = diff / step_u + 1;
        let dir: i32 = if start <= end { 1 } else { -1 };
        let mut result = Vec::with_capacity(count);
        let mut cur = start as i32;
        // GNU braces.c emits one word per codepoint in the sequence; at
        // the backslash position the word's value is empty (it is the
        // quoting character), but the word itself survives expansion:
        // {a..Z} yields ... ] "" [ ... so the baseline shows "]  [".
        // The element therefore carries empty text flagged as a
        // sequence null (BraceProduct::sequence_null), so consumers
        // that assemble fields can keep the word instead of discarding
        // it as an unquoted null.
        for i in 0..count {
            let byte = cur as u8;
            if byte == b'\\' {
                result.push(BraceProduct {
                    text: String::new(),
                    sequence_null: true,
                });
            } else {
                result.push(BraceProduct {
                    text: (byte as char).to_string(),
                    sequence_null: false,
                });
            }
            if i + 1 < count {
                cur += dir * (step as i32);
            }
        }
        return Some(result);
    }
    None
}

fn numeric_range_width(left: &str, right: &str) -> Option<usize> {
    let left_digits = left.trim_start_matches('-');
    let right_digits = right.trim_start_matches('-');
    let padded = [left_digits, right_digits]
        .iter()
        .any(|value| value.len() > 1 && value.starts_with('0'));
    padded.then(|| left.len().max(right.len()))
}

fn format_numeric_range_value(value: i64, width: Option<usize>) -> String {
    let Some(width) = width else {
        return value.to_string();
    };
    if value < 0 {
        format!(
            "-{:0width$}",
            value.unsigned_abs(),
            width = width.saturating_sub(1)
        )
    } else {
        format!("{value:0width$}")
    }
}

#[cfg(test)]
mod dollar_probe_tests {
    use super::expand_braces;

    #[test]
    fn dollar_body_splits_like_gnu() {
        assert_eq!(expand_braces("foo{bar,${a}.}"), vec!["foobar", "foo${a}."]);
        assert_eq!(expand_braces("${a}{x,y}"), vec!["${a}x", "${a}y"]);
        assert_eq!(expand_braces("{a,${a},b}"), vec!["a", "${a}", "b"]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comma_brace() {
        assert_eq!(expand_braces("{a,b,c}"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_nested_comma() {
        assert_eq!(expand_braces("x{a,b}y"), vec!["xay", "xby"]);
    }

    #[test]
    fn test_range_numeric() {
        assert_eq!(expand_braces("{1..3}"), vec!["1", "2", "3"]);
    }

    #[test]
    fn sequence_backslash_element_is_a_flagged_null() {
        // {a..A} spans codepoints 97..=65; index 5 is the backslash
        // position (92). GNU keeps the word with an empty value
        // (braces.tests: 33 words), so the product is an empty text
        // flagged sequence_null — distinct from a bare alternative null.
        let products = expand_braces_tagged("{a..A}");
        assert_eq!(products.len(), 33);
        assert_eq!(products[5].text, "");
        assert!(products[5].sequence_null);
        assert!(products
            .iter()
            .enumerate()
            .all(|(i, p)| i == 5 || !p.sequence_null));
        // Ascending ranges crossing the same codepoint flag it too.
        let ascending = expand_braces_tagged("{A..a}");
        assert_eq!(ascending.len(), 33);
        assert!(ascending[27].sequence_null);
        // Glued to real text the element contributes nothing and the
        // flag does not leak into the (non-empty) product.
        let glued = expand_braces_tagged("x{a..A}y");
        assert_eq!(glued.len(), 33);
        assert_eq!(glued[5].text, "xy");
        assert!(glued.iter().all(|p| !p.sequence_null));
        // Glued only to an empty alternative the composed word is still
        // the sequence's null: {a..A}{,} doubles every element, the
        // backslash position yielding two flagged nulls.
        let doubled = expand_braces_tagged("{a..A}{,}");
        assert_eq!(doubled.len(), 66);
        assert!(doubled[10].sequence_null && doubled[11].sequence_null);
        // A bare alternative null is never flagged.
        let alt = expand_braces_tagged("{a,}");
        assert_eq!(alt.len(), 2);
        assert_eq!(alt[1].text, "");
        assert!(!alt[1].sequence_null);
        // A nested sequence member keeps its flag next to a bare
        // alternative: {{a..A},} yields the sequence's 33 products
        // (one flagged null) plus one unflagged bare null.
        let nested = expand_braces_tagged("{{a..A},}");
        assert_eq!(nested.len(), 34);
        assert_eq!(nested.iter().filter(|p| p.sequence_null).count(), 1);
        assert_eq!(
            nested
                .iter()
                .filter(|p| p.text.is_empty() && !p.sequence_null)
                .count(),
            1
        );
    }

    /// rubash#416/#417: the DATA_DOLLAR carrier (U+001F) is the cooked-word
    /// transport of a quoted `$`; a brace body it opens is `${...}`
    /// parameter text and must be skipped exactly like a literal `${`
    /// (GNU braces.c:679-680 brace_gobbler "treat ${...} like \{...}").
    /// Before this invariant the scanner expanded the member list that
    /// quoting had made literal and downstream remove_shell_quotes ate the
    /// word's data backslashes (`'a\nb${VAR}c'` -> `anb${VAR}c`).
    #[test]
    fn data_dollar_carrier_opens_a_parameter_body_not_a_brace_group() {
        let cooked_single_quoted = "a\\nb\u{1f}{VAR}c";
        assert_eq!(
            expand_braces(cooked_single_quoted),
            vec![cooked_single_quoted]
        );
        // A comma inside the carrier-opened body is parameter text, not a
        // member separator: `'${x,y}'` stays literal.
        let cooked_comma_body = "\u{1f}{x,y}";
        assert_eq!(expand_braces(cooked_comma_body), vec![cooked_comma_body]);
        // The carrier form of `$(` (command substitution) is skipped the
        // same way.
        let cooked_comsub = "\u{1f}(echo {1,2})";
        assert_eq!(expand_braces(cooked_comsub), vec![cooked_comsub]);
        // A REAL brace group in the same word still expands; the carrier
        // body's closing brace is never consumed as the group's.
        assert_eq!(
            expand_braces("x\u{1f}{V}r{a,b}"),
            vec!["x\u{1f}{V}ra", "x\u{1f}{V}rb"]
        );
        // Literal `${` keeps its GNU skip.
        assert_eq!(expand_braces("${V}r{a,b}"), vec!["${V}ra", "${V}rb"]);
    }

    /// unixwin/niubash#1060: a compound-assignment `'...'` element rides to
    /// this scanner wrapped in the ASSIGN_HOISTED_SQUOTE (U+E307) carrier
    /// pair. That pair is the stored form of the `'` delimiters GNU
    /// brace_gobbler opens its quoted state on (braces.c:712-734), so the
    /// braces inside the span are data while a brace group OUTSIDE the
    /// span still expands — `a=('X{16,}Y')` is one literal element and
    /// `a=('a'{1,2})` is two.
    #[test]
    fn hoisted_squote_carrier_span_is_opaque_to_braces() {
        const SQ: char = crate::executor::markers::ASSIGN_HOISTED_SQUOTE;
        let quoted = format!("{SQ}X{{16,}}Y{SQ}");
        assert_eq!(expand_braces(&quoted), vec![quoted]);
        let quoted_seq = format!("{SQ}a{{1..3}}b{SQ}");
        assert_eq!(expand_braces(&quoted_seq), vec![quoted_seq]);
        // Mixed quoting: the unquoted group expands around the quoted
        // data, carriers preserved for the downstream dequote.
        assert_eq!(
            expand_braces(&format!("{SQ}a{SQ}{{1,2}}")),
            vec![format!("{SQ}a{SQ}1"), format!("{SQ}a{SQ}2"),]
        );
        assert_eq!(
            expand_braces(&format!("{SQ}pre{SQ}{{1,2}}{SQ}post{SQ}")),
            vec![
                format!("{SQ}pre{SQ}1{SQ}post{SQ}"),
                format!("{SQ}pre{SQ}2{SQ}post{SQ}"),
            ]
        );
        // A comma inside the carrier span is member text of the quote,
        // not a separator: {'a,b',c} -> 'a,b' c in GNU's member walk.
        assert_eq!(
            expand_braces(&format!("{{{SQ}a,b{SQ},c}}")),
            vec![format!("{SQ}a,b{SQ}"), "c".to_string()]
        );
        // A sequence-looking body inside the span is quoted data too.
        assert_eq!(
            expand_braces(&format!("{{{SQ}a..b{SQ},c}}")),
            vec![format!("{SQ}a..b{SQ}"), "c".to_string()]
        );
    }

    #[test]
    fn test_seq_near_i64_max_terminates() {
        // GNU: {9223372036854775805..9223372036854775807} -> exactly 3 values.
        // rubash hung >5s here via the binary; pin down which layer.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = expand_range("9223372036854775805..9223372036854775807");
            let _ = tx.send(r);
        });
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Some(r)) => assert_eq!(
                r,
                vec![
                    "9223372036854775805".to_string(),
                    "9223372036854775806".to_string(),
                    "9223372036854775807".to_string()
                ]
            ),
            Ok(None) => panic!("expand_range returned None on i64 boundary"),
            Err(_) => panic!("expand_range HUNG on i64 boundary"),
        }

        let (tx2, rx2) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = brace_expand("{9223372036854775805..9223372036854775807}");
            let _ = tx2.send(r);
        });
        match rx2.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(r) => assert_eq!(r.len(), 3),
            Err(_) => panic!("brace_expand HUNG on i64 boundary"),
        }
    }

    #[test]
    fn test_range_numeric_step_and_padding() {
        assert_eq!(expand_braces("{1..5..2}"), vec!["1", "3", "5"]);
        assert_eq!(expand_braces("{1..6..4}"), vec!["1", "5"]);
        assert_eq!(expand_braces("{5..1..2}"), vec!["5", "3", "1"]);
        assert_eq!(expand_braces("{01..03}"), vec!["01", "02", "03"]);
        assert_eq!(expand_braces("{-03..01..2}"), vec!["-03", "-01", "001"]);
        assert_eq!(
            expand_braces("{-003..001..2}"),
            vec!["-003", "-001", "0001"]
        );
    }

    #[test]
    fn test_range_alpha() {
        assert_eq!(expand_braces("{a..c}"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_range_alpha_step() {
        assert_eq!(expand_braces("{a..e..2}"), vec!["a", "c", "e"]);
        assert_eq!(expand_braces("{e..a..2}"), vec!["e", "c", "a"]);
    }

    #[test]
    fn test_nested_escaped_brace_preserves_literal_suffix() {
        assert_eq!(
            expand_braces(r"{x,y,\{a,b,c}}"),
            vec![r"x}", r"y}", r"\{a}", r"b}", r"c}"],
        );
    }

    #[test]
    fn test_no_brace() {
        assert_eq!(expand_braces("hello"), vec!["hello"]);
    }

    #[test]
    fn test_escaped_commas_do_not_split_brace_items() {
        assert_eq!(expand_braces(r"a{b\,c,d}"), vec![r"ab\,c", "ad"]);
        assert_eq!(expand_braces(r"{x\,y,z}"), vec![r"x\,y", "z"]);
    }

    #[test]
    fn test_escaped_braces_do_not_start_or_end_nested_groups() {
        assert_eq!(expand_braces(r"{a\{b,c}"), vec![r"a\{b", "c"]);
        assert_eq!(expand_braces(r"{a\}b,c}"), vec![r"a\}b", "c"]);
    }

    #[test]
    fn test_adjacent_brace_groups() {
        assert_eq!(expand_braces("{a,b}{1..2}"), vec!["a1", "a2", "b1", "b2"]);
    }

    #[test]
    fn test_escaped_brace_group_adjacent_to_real_group() {
        // Escaped braces stay literal (backslashes stripped later by quote
        // removal in command_prepare) while unescaped groups still expand.
        assert_eq!(expand_braces(r"\{a,b}{1,2}"), vec![r"\{a,b}1", r"\{a,b}2"]);
        assert_eq!(
            expand_braces(r"a\{b,c}d{e,f}g"),
            vec![r"a\{b,c}deg", r"a\{b,c}dfg"]
        );
    }

    #[test]
    fn test_invalid_nested_sequences_expand_only_nested_commas() {
        // Expectations below are taken from GNU bash 5.3.0 directly
        // (braces.tests lines 136-142), not from the pre-rewrite sketch:
        //   echo {{1,2,3}..4}   -> {1..4} {2..4} {3..4}
        //   echo {6..{7,8,9}}   -> {6..7} {6..8} {6..9}
        //   echo {{a..c}..{1..3}} -> 9 words {a..1} ... {c..3}
        // Note the nested sequence braces are RETAINED in the output: the
        // invalid outer {..} never expands, it just carries the endpoint text.
        assert_eq!(
            expand_braces("{{1,2,3}..4}"),
            vec!["{1..4}", "{2..4}", "{3..4}"],
        );
        assert_eq!(
            expand_braces("{6..{7,8,9}}"),
            vec!["{6..7}", "{6..8}", "{6..9}"],
        );
        // GNU cross-products the two nested sequences even though neither is a
        // comma group in the valid_seqterm sense.
        assert_eq!(
            expand_braces("{{a..c}..{1..3}}"),
            vec![
                "{a..1}", "{a..2}", "{a..3}", "{b..1}", "{b..2}", "{b..3}", "{c..1}", "{c..2}",
                "{c..3}",
            ],
        );
        assert_eq!(
            expand_braces("{{a..c}..{1,10}}"),
            vec!["{a..1}", "{a..10}", "{b..1}", "{b..10}", "{c..1}", "{c..10}"],
        );
    }

    #[test]
    fn test_debug_adjacent_braces() {
        let result = expand_braces("{a,b}{1,2}");
        println!("Debug: expand_braces = {:?}", result);
        assert_eq!(result, vec!["a1", "a2", "b1", "b2"]);
    }

    #[test]
    fn test_split_brace_commas_brace_gobbler_units() {
        assert_eq!(
            expand_braces("{$(echo `echo \"q`),x}y"),
            vec!["$(echo `echo \"q`)y", "xy"]
        );
        assert_eq!(
            expand_braces("{`echo \"a,b\"`,x}"),
            vec!["`echo \"a,b\"`", "x"]
        );
        assert_eq!(expand_braces("{<(echo a,b),x}"), vec!["<(echo a,b)", "x"]);
        assert_eq!(expand_braces("{>(echo a,b),x}"), vec![">(echo a,b)", "x"]);
        assert_eq!(expand_braces("<(echo {a,b})"), vec!["<(echo {a,b})"]);
    }

    /// rubash#3969: the product-level port of expand_word_internal's
    /// `` ` `` case (subst.c:11568-11593) — string_extract honors only
    /// `\x` escapes inside a backquote body, the bare trailing backquote
    /// is literal data (subst.c:11574-11576), and a backquote pair whose
    /// close is the word's last byte is CLOSED, not unclosed.
    #[test]
    fn unclosed_backquote_index_matches_gnu_rescan() {
        // Unclosed opens report the open's index.
        assert_eq!(unclosed_backquote_index("`x"), Some(0));
        assert_eq!(unclosed_backquote_index("a`b"), Some(1));
        assert_eq!(unclosed_backquote_index("`$x"), Some(0));
        assert_eq!(unclosed_backquote_index("`'x'"), Some(0));
        // The lone `` ` `` product and any word ending in a bare
        // backquote keep the compatibility exception.
        assert_eq!(unclosed_backquote_index("`"), None);
        assert_eq!(unclosed_backquote_index("x`"), None);
        // A pair that closes on the last byte is a real substitution,
        // not an unclosed open (the `close < len` ambiguity).
        assert_eq!(unclosed_backquote_index("a`echo hi`"), None);
        assert_eq!(unclosed_backquote_index("`x`"), None);
        // An escaped backquote neither opens nor closes.
        assert_eq!(unclosed_backquote_index("\\`x"), None);
        assert_eq!(unclosed_backquote_index("`x\\`"), Some(0));
        // Single quotes are opaque to the scan; an unclosed `'` swallows
        // the rest of the word as quoted data.
        assert_eq!(unclosed_backquote_index("'`x'"), None);
        assert_eq!(unclosed_backquote_index("'`x"), None);
        // Inside `"..."` the scan recurses: a closed pair is fine, a
        // `"` inside an open backquote cannot close the quote (the
        // dquote's content then runs to the end of the word and the
        // backquote is unclosed there).
        assert_eq!(unclosed_backquote_index("\"a`b`c\""), None);
        assert_eq!(unclosed_backquote_index("\"a`b"), Some(2));
        assert_eq!(unclosed_backquote_index("\"a`\""), Some(2));
        assert_eq!(unclosed_backquote_index("\"a`"), None);
        // $()/${}/$'..'/<(/> (bodies are extracted atomically; a
        // backquote inside belongs to that body's own parse.
        assert_eq!(unclosed_backquote_index("$(`x"), None);
        assert_eq!(unclosed_backquote_index("${a`b}"), None);
        assert_eq!(unclosed_backquote_index("$'`'"), None);
        // A closed pair followed by a second bare backquote stays
        // literal (`{a..A}`echo Q``'s `` ` `` element product).
        assert_eq!(unclosed_backquote_index("``echo Q`"), None);
        assert_eq!(unclosed_backquote_index("`x`y`"), None);
    }
}
