pub(in crate::executor) fn case_pattern_matches(pattern: &str, word: &str) -> bool {
    // The byte-spelling gate runs BEFORE the pattern-only fast paths:
    // literal_pattern_equality and simple_glob_matches verdict on the raw
    // chars and cannot see marker spellings — a `?` must match a carried
    // byte, not fail on its marker spelling, and a pattern carrying a
    // comsub payload sentinel (`[[ $v == *$(printf '\001')* ]]`,
    // rubash#729 rework) would be verdicted against the word's raw
    // spelling and never reach the normalization that decodes the payload.
    // rubash#1533: the byte-locale units gate runs with the spelling
    // gate — in a non-UTF-8 locale a multibyte char on either side
    // matches as its raw bytes, one unit per byte.
    let byte_locale = sides_need_byte_locale_units(pattern, word);
    if byte_locale || sides_need_byte_char_matching(pattern, word) {
        let pattern: Vec<char> = stage_pattern_match_units(pattern, byte_locale);
        let word: Vec<char> = stage_word_match_units(word, byte_locale);
        return case_pattern_matches_at_with_case(&pattern, 0, &word, 0, false);
    }
    case_pattern_matches_prechecked(pattern, word, false)
}

/// Matcher body below the byte-spelling / byte-locale gates, for callers
/// that evaluate the gates ONCE for a whole (pattern, value) pair and
/// then match many char-boundary slices of that value — the `${v#pat}` /
/// `${v%pat}` removal walks (parameter_decode.rs) and the anchored /
/// scanning substitution finders (parameter_replace.rs). Both gates are
/// containment predicates over the two sides (a byte spelling or a
/// non-ASCII text char on either side), so a gate-clean pair stays clean
/// for every char-boundary slice of the value: slicing cannot
/// manufacture a spelling the whole value does not carry, and the
/// pattern side is invariant across candidates. GNU's match_upattern
/// (subst.c:5382) likewise runs strmatch directly against the live
/// string — the C matcher does no per-candidate staging or
/// re-inspection either. Callers must only use this entry when the full
/// pair is gate-clean; the verdict for any single candidate is then
/// exactly what [`case_pattern_matches`] / its nocase twin would return
/// for that candidate alone.
pub(in crate::executor) fn case_pattern_matches_prechecked(
    pattern: &str,
    word: &str,
    nocase: bool,
) -> bool {
    if let Some(verdict) = literal_pattern_equality(pattern, word, nocase) {
        return verdict;
    }
    if let Some(verdict) = simple_glob_matches(pattern, word, nocase) {
        return verdict;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let word: Vec<char> = word.chars().collect();
    case_pattern_matches_at_with_case(&pattern, 0, &word, 0, nocase)
}

pub(in crate::executor) fn case_pattern_matches_nocase(pattern: &str, word: &str) -> bool {
    // Byte-spelling gate runs BEFORE the pattern-only fast paths (see above).
    // rubash#1533: byte-locale units gate joins it.
    let byte_locale = sides_need_byte_locale_units(pattern, word);
    if byte_locale || sides_need_byte_char_matching(pattern, word) {
        let pattern: Vec<char> = stage_pattern_match_units(pattern, byte_locale);
        let word: Vec<char> = stage_word_match_units(word, byte_locale);
        return case_pattern_matches_at_with_case(&pattern, 0, &word, 0, true);
    }
    case_pattern_matches_prechecked(pattern, word, true)
}

/// Word-hoisted twin of [`case_pattern_matches`]: the `case` subject word is
/// identical for every arm, so `execute_case_command`
/// (GNU `execute_cmd.c:3644 execute_case_command`, word built once via
/// `expand_word_leave_quoted` + `dequote_string` at 3695-3699, then matched
/// per arm with `strmatch` at 3736) stages its `Vec<char>` once outside the
/// arm loop and reuses it here. The pattern side still stages per arm — it
/// differs per candidate — while the literal / simple-glob fast paths stay
/// borrow-only.
///
/// `word_chars` is a LAZY cell: the literal / simple-glob fast paths never
/// consult it, and GNU's `strmatch` for a literal arm does no char staging
/// either, so the common 20-literal-arm loop must not pay a per-execution
/// `Vec<char>` build for a cache that is never read. The caller seeds it on
/// the first arm that actually reaches the staged matcher. The raw-byte-marker
/// path is rare; it delegates to the allocating form rather than duplicating
/// the flatten logic.
pub(in crate::executor) fn case_pattern_matches_with_word_chars(
    pattern: &str,
    word: &str,
    word_chars: &mut Option<Vec<char>>,
) -> bool {
    // Byte-spelling gate runs BEFORE the pattern-only fast paths (see
    // above): the fast paths verdict on raw chars and cannot see marker
    // spellings, so a marked word must go through the flattening base
    // function instead of the staged raw-char path below.
    if sides_need_byte_char_matching(pattern, word) || sides_need_byte_locale_units(pattern, word) {
        return case_pattern_matches(pattern, word);
    }
    if let Some(verdict) = literal_pattern_equality(pattern, word, false) {
        return verdict;
    }
    if let Some(verdict) = simple_glob_matches(pattern, word, false) {
        return verdict;
    }
    let word_chars = word_chars.get_or_insert_with(|| word.chars().collect());
    let pattern_chars: Vec<char> = pattern.chars().collect();
    case_pattern_matches_at_with_case(&pattern_chars, 0, word_chars, 0, false)
}

/// Nocase twin of [`case_pattern_matches_with_word_chars`].
pub(in crate::executor) fn case_pattern_matches_nocase_with_word_chars(
    pattern: &str,
    word: &str,
    word_chars: &mut Option<Vec<char>>,
) -> bool {
    // Byte-spelling gate first (see base function above).
    if sides_need_byte_char_matching(pattern, word) || sides_need_byte_locale_units(pattern, word) {
        return case_pattern_matches_nocase(pattern, word);
    }
    if let Some(verdict) = literal_pattern_equality(pattern, word, true) {
        return verdict;
    }
    if let Some(verdict) = simple_glob_matches(pattern, word, true) {
        return verdict;
    }
    let word_chars = word_chars.get_or_insert_with(|| word.chars().collect());
    let pattern_chars: Vec<char> = pattern.chars().collect();
    case_pattern_matches_at_with_case(&pattern_chars, 0, word_chars, 0, true)
}

/// Direct char-wise match for the simple-glob class: a pattern whose every
/// byte is printable ASCII (or space) with no byte in `[ ] \ + @ ! ( )` —
/// only literals, `*` and `?` remain. GNU gnulib strmatch.c `gmatch` walks
/// exactly this class with a single saved backtrack point (pattern and word
/// positions at the most recent `*`) and no buffer staging; the Vec<char>
/// staging this port paid per call is charged once per CANDIDATE BOUNDARY
/// by the `${v#pat}`/`${v%pat}` removal loops (parameter_decode.rs), which
/// is the hot per-fragment cost of expansion-heavy scripts.
///
/// The single-backtrack-point algorithm is correct for this class: a
/// mismatch resumes from the MOST RECENT `*` with the word advanced by one
/// char; an earlier star's continuation is always subsumed by the later
/// star matching fewer chars (`*` matches any sequence, and nothing else in
/// the class can consume across a star). Returns None when the pattern is
/// outside the class (caller falls through to the staged matcher, which
/// owns brackets, extglob groups and raw-byte markers).
fn simple_glob_matches(pattern: &str, word: &str, nocase: bool) -> Option<bool> {
    if !pattern.bytes().all(|b| {
        (b.is_ascii_graphic() || b == b' ')
            && !matches!(b, b'[' | b']' | b'\\' | b'+' | b'@' | b'!' | b'(' | b')')
    }) {
        return None;
    }
    let matches_char = |pat: char, target: char| {
        pat == '?'
            || (if nocase {
                pat.to_ascii_lowercase() == target.to_ascii_lowercase()
            } else {
                pat == target
            })
    };
    let mut p = pattern.chars();
    let mut w = word.chars();
    let mut star_p: Option<std::str::Chars<'_>> = None;
    let mut star_w: Option<std::str::Chars<'_>> = None;
    // Resume from the most recent `*` with the word advanced by one char;
    // an exhausted word at the backtrack point means the star could not be
    // extended, and no earlier star exists — the match fails.
    macro_rules! backtrack {
        () => {
            match (star_p.take(), star_w.take()) {
                (Some(saved_p), Some(saved_w)) => {
                    let mut next_w = saved_w;
                    if next_w.next().is_none() {
                        // The word is exhausted at the star: the star has
                        // absorbed everything. The match succeeds exactly
                        // when the pattern tail after the star is empty
                        // (literals/`?` left over would need more chars).
                        return Some(saved_p.clone().next().is_none());
                    }
                    p = saved_p;
                    w = next_w.clone();
                    star_p = Some(p.clone());
                    star_w = Some(next_w);
                }
                _ => return Some(false),
            }
        };
    }
    loop {
        match p.clone().next() {
            None => {
                if w.clone().next().is_none() {
                    return Some(true);
                }
                // Pattern exhausted but word has chars left: only a pending
                // star can absorb them.
                backtrack!();
            }
            Some('*') => {
                // Collapse consecutive stars: the backtrack point after a
                // run of `*` is the position past all of them.
                while p.clone().next() == Some('*') {
                    p.next();
                }
                star_p = Some(p.clone());
                star_w = Some(w.clone());
            }
            Some(pat) => match w.clone().next() {
                Some(target) if matches_char(pat, target) => {
                    p.next();
                    w.next();
                }
                _ => backtrack!(),
            },
        }
    }
}

/// rubash#281 (rubash#117 whitelist): a pattern whose every byte is
/// printable ASCII and carries NO glob syntax byte — no `* ? [ ] \` and
/// none of the extglob operator bytes `+ @ ! (` — matches by plain
/// equality: every remaining byte is a literal compared against the word
/// (GNU strmatch walks the pattern character by character; nothing but the
/// metacharacters can consume or fold input). Returning the compare
/// directly skips the two `Vec<char>` stagings per call, which the
/// prefix/suffix removal loop pays once per candidate boundary. `nocase`
/// folds with `eq_ignore_ascii_case`, the ASCII-only fold GNU's C-locale
/// strncasecmp performs; non-ASCII bytes never admit here.
fn literal_pattern_equality(pattern: &str, word: &str, nocase: bool) -> Option<bool> {
    pattern_is_plain_literal(pattern).then(|| {
        if nocase {
            word.eq_ignore_ascii_case(pattern)
        } else {
            word == pattern
        }
    })
}

/// Whether `pattern` belongs to the plain-literal class gated on in
/// [`literal_pattern_equality`]: every byte is printable ASCII (or a
/// space) and none carries glob or extglob operator meaning, so the
/// only verdict the matcher can return for it is string equality. The
/// `${v#pat}` / `${v%pat}` removal loops (parameter_decode.rs) answer
/// such a pattern with a single candidate instead of one matcher call
/// per candidate boundary — the literal case of the matches-nowhere
/// short-circuit GNU documents for match_upattern (subst.c) —
/// rubash#608.
pub(in crate::executor) fn pattern_is_plain_literal(pattern: &str) -> bool {
    pattern.bytes().all(|b| {
        (b.is_ascii_graphic() || b == b' ')
            && !matches!(
                b,
                b'*' | b'?' | b'[' | b']' | b'\\' | b'+' | b'@' | b'!' | b'(' | b')'
            )
    })
}

/// Normalization gate for byte-level matching (rubash#729). Subsumes the
/// narrower raw-byte-marker check landed by #983: the 0xEE lead byte below
/// already covers the whole PUA marker plane including E000.
///
/// GNU matches patterns and words as BYTE strings (subst.c pat_substr /
/// sm_loop.c gmatch walk `unsigned char` buffers). Rubash transport has
/// SEVERAL spellings for the same byte: the raw-byte marker pair
/// (E000+E0xx, the entry encoding), a CTLESC-quoted pair (a quoted span
/// quote-protects each pair char), a literal C0 char (a
/// `substitution_result_visible_text` decode that re-materialized the
/// byte), and the `U+E319<hex><hex>;` payload (protect_command_substitution_
/// output). Two sides carrying DIFFERENT spellings of the same byte must
/// still match — `[[ $v == *"$(printf '\001')"* ]]` failed because the word
/// held the pair while the quoted command-substitution pattern held the
/// decoded C0 char.
///
/// The gate fires when EITHER side names a byte-level spelling: a raw-byte
/// marker (0xEE is the UTF-8 lead byte of the whole PUA marker plane), a
/// command-substitution payload sentinel, or a literal C0 char outside
/// \t/\n (0x01..=0x1f, the carrier-collision block; \t/\n words are the
/// common case and match un-flattened today, so they keep the fast path).
/// Both sides are then flattened, so CTLESC quote marks dequote
/// identically and a quoted word (`[[ "a*b" == a* ]]`) still matches.
///
/// #735 safety: a user `$'\uE4xx'` char arrives E400-prefixed
/// (push_literal_char); normalization never consumes an E400 escape (it
/// has no E000/E0xx-pair or sentinel-payload shape), so escaped user data
/// flattens identically on both sides and can never be read as a marker.
pub(in crate::executor) fn sides_need_byte_char_matching(pattern: &str, word: &str) -> bool {
    str_carries_byte_spelling(pattern) || str_carries_byte_spelling(word)
}

/// Staged-word twin of [`sides_need_byte_char_matching`] for the
/// `*_with_word_chars` matcher entries, which hold the word as `&[char]`.
pub(in crate::executor) fn sides_need_byte_char_matching_staged(
    pattern: &str,
    word: &[char],
) -> bool {
    str_carries_byte_spelling(pattern) || chars_carry_byte_spelling(word)
}

fn str_carries_byte_spelling(text: &str) -> bool {
    text.contains(crate::executor::markers::COMSUB_PAYLOAD_SENTINEL)
        || text
            .bytes()
            .any(|b| b == 0xEE || (b < 0x20 && b != b'\t' && b != b'\n'))
}

/// The `carries` probe for a char slice: a `0xEE` UTF-8 lead byte names
/// exactly U+E000..=U+EFFF, so the codepoint range is the same question.
fn chars_carry_byte_spelling(chars: &[char]) -> bool {
    chars.iter().any(|&c| {
        c == crate::executor::markers::COMSUB_PAYLOAD_SENTINEL
            || (0xE000..=0xEFFF).contains(&(c as u32))
            || ((c as u32) < 0x20 && !matches!(c, '\t' | '\n'))
    })
}

/// U+E100+byte byte-char → the byte it carries, or None for any other char.
/// rubash#1533: in a single-byte locale (LC_CTYPE is not UTF-8) GNU's
/// matchers walk BYTE strings — gmatch/sm_loop advance one byte per `?`,
/// and a multibyte character on either side is simply its raw bytes
/// (subst.c pat_substr / lib/glob sm_loop.c walk `unsigned char` buffers;
/// there is no decoding step when the locale is not multibyte). Rubash
/// stages chars, so under a byte locale any ordinary non-ASCII char must
/// be re-staged as one byte-char unit per UTF-8 byte before matching,
/// reusing the byte units the spelling gate above already feeds the
/// engine. The PUA plane (E000–EFFF: byte-chars, raw-byte markers, the
/// E400 literal escape, the other carriers) is transport, not text, and
/// keeps its existing units.
pub(in crate::executor) fn char_is_text_non_ascii(ch: char) -> bool {
    (ch as u32) >= 0x80 && !matches!(ch as u32, 0xE000..=0xEFFF)
}

pub(in crate::executor) fn sides_need_byte_locale_units(pattern: &str, word: &str) -> bool {
    !crate::locale::is_utf8()
        && (pattern.chars().any(char_is_text_non_ascii) || word.chars().any(char_is_text_non_ascii))
}

pub(in crate::executor) fn sides_need_byte_locale_units_staged(
    pattern: &str,
    word_chars: &[char],
) -> bool {
    !crate::locale::is_utf8()
        && (pattern.chars().any(char_is_text_non_ascii)
            || word_chars.iter().copied().any(char_is_text_non_ascii))
}

/// Split every ordinary non-ASCII char in an already-staged unit buffer
/// into one byte-char per UTF-8 byte (#1533). ASCII, byte-chars and the
/// PUA carrier plane pass through untouched.
pub(in crate::executor) fn split_text_chars_into_byte_units(chars: Vec<char>) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len());
    for ch in chars {
        if char_is_text_non_ascii(ch) {
            let mut buf = [0u8; 4];
            for byte in ch.encode_utf8(&mut buf).as_bytes() {
                out.push(char::from_u32(BYTE_CHAR_BASE + u32::from(*byte)).unwrap());
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Stage a pattern for matching: the #729 spelling flattening, plus the
/// #1533 byte-locale text split when `byte_locale` holds.
pub(in crate::executor) fn stage_pattern_match_units(
    pattern: &str,
    byte_locale: bool,
) -> Vec<char> {
    let staged = flatten_pattern_to_byte_chars(pattern);
    if byte_locale {
        split_text_chars_into_byte_units(staged)
    } else {
        staged
    }
}

/// Word twin of [`stage_pattern_match_units`].
pub(in crate::executor) fn stage_word_match_units(word: &str, byte_locale: bool) -> Vec<char> {
    let staged = flatten_word_to_byte_chars(word);
    if byte_locale {
        split_text_chars_into_byte_units(staged)
    } else {
        staged
    }
}

/// Raw bytes of a unit-space string (the inverse direction of the
/// staging above, for assembled substitution output): a byte-char
/// contributes the byte it carries, every other char contributes its
/// UTF-8 encoding — so an E400 literal-escape pair re-encodes as the
/// pair's own UTF-8 bytes and survives the caller's
/// bytes_to_shell_text round trip verbatim (rubash#1533).
pub(in crate::executor) fn match_units_to_raw_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut buf = [0u8; 4];
    for ch in text.chars() {
        match byte_char_value(ch) {
            Some(byte) => out.push(byte),
            None => out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes()),
        }
    }
    out
}

fn byte_char_value(ch: char) -> Option<u8> {
    let code = ch as u32;
    if (BYTE_CHAR_BASE..BYTE_CHAR_BASE + 0x100).contains(&code) {
        Some((code - BYTE_CHAR_BASE) as u8)
    } else {
        None
    }
}

/// Base for byte-char representation: U+E100 + byte value. This PUA range is
/// disjoint from the raw-byte marker range (U+E000-U+E100) and from ASCII
/// pattern syntax chars (*, ?, \), so byte-chars never collide with syntax.
pub(crate) use crate::executor::markers::BYTE_CHAR_BASE;

/// Flatten a pattern to byte-chars for byte-level matching. Pattern syntax
/// chars (*, ?, \, [, ], CTLESC) are kept as-is; raw-byte markers become
/// byte-chars; the literal-backslash marker becomes the backslash
/// byte-char; all other chars stay verbatim. A CTLESC-quoted raw-byte
/// marker pair (`CTLESC E000 [CTLESC] E0xx`, the quoted-span spelling of
/// the pair) resolves to the single byte-char the pair carries.
pub(in crate::executor) fn flatten_pattern_to_byte_chars(pattern: &str) -> Vec<char> {
    use crate::executor::substitution_metadata::{
        RAW_BYTE_MARKER_ESCAPE, RAW_BYTE_MARKER_FIRST, RAW_BYTE_MARKER_LAST,
    };
    let pattern = decode_payload_sentinels_to_pairs(pattern);
    let mut output = Vec::new();
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch as u32 == RAW_BYTE_MARKER_ESCAPE {
            match chars.peek().copied() {
                Some(next)
                    if (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&(next as u32)) =>
                {
                    chars.next();
                    output.push(
                        char::from_u32(BYTE_CHAR_BASE + (next as u32 - RAW_BYTE_MARKER_FIRST))
                            .expect("byte-char is valid"),
                    );
                }
                _ => output.push(ch),
            }
        } else if ch == crate::executor::markers::LITERAL_CHAR_ESCAPE {
            // rubash#1635: a literal-char escape pair is ONE data
            // character (its payload), exactly as a raw-byte pair is
            // one byte-char above — staging the halves as two units
            // made `?` unable to match the character, so ${v#?} on a
            // lone escaped char removed nothing. Greedy pairing,
            // mirroring shell_text_to_raw_bytes.
            match chars.next() {
                Some(payload) => output.push(payload),
                None => output.push(ch),
            }
        } else if ch == crate::executor::markers::CTLESC {
            // CTLESC quotes the next pattern char (parse.y:5694-5706
            // got_escaped_character). When the quoted unit is a raw-byte
            // marker pair — the char after CTLESC is the E000 introducer
            // and the one after that the payload (itself possibly
            // CTLESC'd) — the pair collapses to its byte-char. Any other
            // CTLESC stays: case_pattern_atom_matches owns the escape arm.
            match try_consume_ctlesc_quoted_pair(&mut chars) {
                Some(byte_char) => output.push(byte_char),
                None => output.push(ch),
            }
        } else if ch == crate::executor::markers::PATTERN_LITERAL_BACKSLASH {
            // A literal-backslash marker names `\` data — the matcher owns
            // this invariant (case_pattern_atom_matches matches the marker
            // against a literal backslash). Normalize to the backslash
            // byte-char so it meets `\` on the word side. The generic C0
            // arm below must not claim it: mapping the marker to byte 0x18
            // matches nothing and turned `${red//\\[\\e/}` (exp.tests
            // exp5.sub) into a silent no-op.
            output.push(char::from_u32(BYTE_CHAR_BASE + 0x5C).expect("byte-char"));
        } else if (ch as u32) < 0x20 {
            // A literal C0 char (0x01..=0x1f) is a re-materialized byte
            // (substitution_result_visible_text / printf octal escapes):
            // normalize to its byte-char so it meets the pair spelling of
            // the same byte on the other side.
            output.push(char::from_u32(BYTE_CHAR_BASE + ch as u32).expect("byte-char"));
        } else {
            // Verbatim: syntax chars (* ? [ ] \) stay for the matcher's
            // operator arms, ASCII members stay so bracket class names
            // ([[:cntrl:]]) recognize, multibyte chars stay as single
            // chars (the word side keeps the same literals — a carried
            // INVALID byte meets them through the chars_match byte map).
            output.push(ch);
        }
    }
    output
}

/// Consume an optional CTLESC-quoted raw-byte marker pair starting at the
/// cursor (which sits just past a leading CTLESC). Returns the byte-char
/// when the shape `E000 [CTLESC] payload` (payload in E001..=E100) is
/// present; otherwise the cursor is untouched and None is returned.
fn try_consume_ctlesc_quoted_pair(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Option<char> {
    use crate::executor::substitution_metadata::{
        RAW_BYTE_MARKER_ESCAPE, RAW_BYTE_MARKER_FIRST, RAW_BYTE_MARKER_LAST,
    };
    let mut probe = chars.clone();
    let introducer = probe.next()?;
    if introducer as u32 != RAW_BYTE_MARKER_ESCAPE {
        return None;
    }
    // The payload char of a quoted pair can itself be CTLESC-protected
    // (quote_string protects every char of the quoted span).
    let payload = match probe.next()? {
        crate::executor::markers::CTLESC => probe.next()?,
        direct => direct,
    };
    if !(RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&(payload as u32)) {
        return None;
    }
    *chars = probe;
    Some(
        char::from_u32(BYTE_CHAR_BASE + (payload as u32 - RAW_BYTE_MARKER_FIRST))
            .expect("byte-char is valid"),
    )
}

/// Rewrite the `U+E319<hex><hex>;` payload spelling
/// (protect_command_substitution_output) into the raw-byte marker pair the
/// byte-char walk understands, so a command-substitution byte meets the
/// same byte in any other spelling (rubash#729). Same grammar as
/// execution_misc::decode_command_substitution_payload: an E400-escaped
/// sentinel is user data (rubash#840) and the pair rides through verbatim;
/// a sentinel without the full `<hex><hex>;` tail is malformed protocol
/// and re-emits verbatim.
fn decode_payload_sentinels_to_pairs(text: &str) -> String {
    use crate::executor::markers::{COMSUB_PAYLOAD_SENTINEL, LITERAL_CHAR_ESCAPE};
    use crate::executor::substitution_metadata::{RAW_BYTE_MARKER_ESCAPE, RAW_BYTE_MARKER_FIRST};
    if !text.contains(COMSUB_PAYLOAD_SENTINEL) {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == LITERAL_CHAR_ESCAPE {
            output.push(ch);
            if let Some(next) = chars.next() {
                output.push(next);
            }
            continue;
        }
        if ch != COMSUB_PAYLOAD_SENTINEL {
            output.push(ch);
            continue;
        }
        let mut probe = chars.clone();
        let byte = match (probe.next(), probe.next(), probe.next()) {
            (Some(high), Some(low), Some(';'))
                if high.is_ascii_hexdigit() && low.is_ascii_hexdigit() =>
            {
                u8::from_str_radix(&format!("{high}{low}"), 16).ok()
            }
            _ => None,
        };
        match byte {
            Some(byte) => {
                output.push(char::from_u32(RAW_BYTE_MARKER_ESCAPE).expect("sentinel is valid"));
                output.push(
                    char::from_u32(RAW_BYTE_MARKER_FIRST + byte as u32)
                        .expect("marker char is valid"),
                );
                chars.next();
                chars.next();
                chars.next();
            }
            None => output.push(ch),
        }
    }
    output
}

/// Flatten a word to byte-chars for byte-level matching. Raw-byte markers
/// become byte-chars; Unicode chars become their UTF-8 byte-chars. A
/// CTLESC-quoted pair (the quoted-span spelling of a carried byte) resolves
/// to its byte-char, symmetric with the pattern side.
pub(in crate::executor) fn flatten_word_to_byte_chars(word: &str) -> Vec<char> {
    use crate::executor::substitution_metadata::{
        RAW_BYTE_MARKER_ESCAPE, RAW_BYTE_MARKER_FIRST, RAW_BYTE_MARKER_LAST,
    };
    let word = decode_payload_sentinels_to_pairs(word);
    let mut output = Vec::new();
    let mut chars = word.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch as u32 == RAW_BYTE_MARKER_ESCAPE {
            match chars.peek().copied() {
                Some(next)
                    if (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&(next as u32)) =>
                {
                    chars.next();
                    output.push(
                        char::from_u32(BYTE_CHAR_BASE + (next as u32 - RAW_BYTE_MARKER_FIRST))
                            .expect("byte-char is valid"),
                    );
                }
                _ => output.push(ch),
            }
        } else if ch == crate::executor::markers::LITERAL_CHAR_ESCAPE {
            // rubash#1635: word twin of the pattern-side pair collapse
            // (one data character per literal-escape pair).
            match chars.next() {
                Some(payload) => output.push(payload),
                None => output.push(ch),
            }
        } else if ch == crate::executor::markers::CTLESC {
            match try_consume_ctlesc_quoted_pair(&mut chars) {
                Some(byte_char) => output.push(byte_char),
                // A CTLESC quoting an ordinary char is word-text quoting
                // state (the CTLESC port); the quoted char is the data.
                None => {
                    if let Some(quoted) = chars.next() {
                        output.push(quoted);
                    }
                }
            }
        } else if (ch as u32) < 0x20 {
            // Literal C0 char = re-materialized byte (see the pattern side).
            output.push(char::from_u32(BYTE_CHAR_BASE + ch as u32).expect("byte-char"));
        } else {
            // Verbatim literal (see the pattern side for the arity note).
            output.push(ch);
        }
    }
    output
}

pub(in crate::executor) fn case_pattern_matches_at_with_case(
    pattern: &[char],
    p_index: usize,
    word: &[char],
    w_index: usize,
    nocase: bool,
) -> bool {
    // Keep the wildcard backtracking iterative. A recursive `*` matcher uses
    // one stack frame per input character; ordinary Windows PATH values are
    // long enough to exhaust the process stack during a valid `case` test.
    let mut pattern_index = p_index;
    let mut word_index = w_index;
    let mut star_pattern_index = None;
    let mut star_word_index = word_index;

    while word_index < word.len() {
        if pattern_index < pattern.len() {
            if pattern[pattern_index] == '*' {
                star_pattern_index = Some(pattern_index);
                star_word_index = word_index;
                pattern_index += 1;
                continue;
            }

            let (matched, next_pattern_index) = case_pattern_atom_matches(
                pattern,
                pattern_index,
                word[word_index],
                word_index + 1 == word.len(),
                nocase,
            );
            if matched {
                pattern_index = next_pattern_index;
                word_index += 1;
                continue;
            }
        }

        let Some(star_index) = star_pattern_index else {
            return false;
        };
        star_word_index += 1;
        word_index = star_word_index;
        pattern_index = star_index + 1;
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
}

fn case_pattern_atom_matches(
    pattern: &[char],
    pattern_index: usize,
    candidate: char,
    is_last_word_char: bool,
    nocase: bool,
) -> (bool, usize) {
    match pattern[pattern_index] {
        crate::executor::markers::PATTERN_LITERAL_BACKSLASH => {
            (candidate == '\\', pattern_index + 1)
        }
        crate::executor::markers::CTLESC if pattern_index + 1 < pattern.len() => (
            chars_match(pattern[pattern_index + 1], candidate, nocase),
            pattern_index + 2,
        ),
        '?' => (true, pattern_index + 1),
        '[' => {
            if let Some((matched, next_index)) = case_bracket_expression_matches_with_case(
                pattern,
                pattern_index,
                Some(candidate),
                nocase,
            ) {
                (matched, next_index)
            } else {
                (chars_match('[', candidate, nocase), pattern_index + 1)
            }
        }
        '\\' if pattern_index + 1 < pattern.len() => (
            chars_match(pattern[pattern_index + 1], candidate, nocase),
            pattern_index + 2,
        ),
        // GNU sm_loop.c:121-135: a trailing unquoted `\` at the end of the
        // pattern "cannot match" — except the special case where the string
        // char is `\` AND it is the last character of the string (p == pe &&
        // sc == '\\' && n+1 == se breaks out as a match).
        '\\' => (candidate == '\\' && is_last_word_char, pattern_index + 1),
        literal => (chars_match(literal, candidate, nocase), pattern_index + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::{case_pattern_matches, case_pattern_matches_nocase};

    #[test]
    fn wildcard_matching_handles_long_words_without_recursion() {
        let word = "x".repeat(32 * 1024);

        assert!(case_pattern_matches("*x*", &word));
        assert!(!case_pattern_matches("*missing*", &word));
    }

    #[test]
    fn wildcard_matching_backtracks_after_a_partial_literal_match() {
        assert!(case_pattern_matches("*ab", "aab"));
        assert!(case_pattern_matches_nocase("*foo*", "PATH_FOO_VALUE"));
    }

    /// exp.tests exp5.sub pins `${red//\\[\\e/}` -> `[0;31m\]` and the
    /// follow-up `${foo//[0;31m\\/}` -> `]`. Both patterns carry the
    /// literal-backslash marker (\x18), which trips the byte-char gate, so
    /// these exercise the flattened path end to end. The #729 rewrite
    /// mapped the marker through the generic C0 arm to byte 0x18 (which
    /// matches nothing) and both substitutions went silent; the marker
    /// must normalize to the backslash byte.
    #[test]
    fn flattened_literal_backslash_marker_matches_backslash() {
        assert!(case_pattern_matches("\u{18}[\u{18}e", "\\[\\e"));
        assert!(!case_pattern_matches("\u{18}[\u{18}e", "\\[\\E"));
        assert!(case_pattern_matches("[0;31m\u{18}", "[0;31m\\"));
        assert!(!case_pattern_matches("[0;31m\u{18}", "[0;31m]"));
    }

    /// GNU matches bytes, not chars (subst.c glob_pattern_patscan walks
    /// unsigned char buffers): a word carrying a raw-byte marker pair for
    /// a single byte must match a single-char pattern. Regression test
    /// for the word-side blind spot in the byte-spelling gate: previously
    /// only the pattern side was checked, so `[[ $x == ? ]]` with a
    /// carried 0x11 byte fell through to the plain char-wise matcher and
    /// wrongly failed (the two-char marker spelling never equals one `?`).
    #[test]
    fn carried_byte_matches_single_char_pattern() {
        // \u{e000}\u{e012} is the stored spelling of byte 0x11
        // (RAW_BYTE_MARKER_ESCAPE + FIRST + byte).
        let carried = "\u{e000}\u{e012}";
        assert!(case_pattern_matches("?", carried));
        assert!(case_pattern_matches_nocase("?", carried));
        assert!(case_pattern_matches("*", carried));
        assert!(!case_pattern_matches("??", carried));
        assert!(!case_pattern_matches("x", carried));
        // Plain words without marker spellings keep the old behavior.
        assert!(case_pattern_matches("?", "q"));
        assert!(!case_pattern_matches("??", "q"));
        assert!(case_pattern_matches("a", "a"));
        assert!(!case_pattern_matches("a", "b"));
    }
}

pub(in crate::executor) fn extglob_match_literal(
    pattern: &[char],
    p: usize,
    word: &[char],
    w: usize,
) -> bool {
    extglob_match_literal_with_case(pattern, p, word, w, false)
}

pub(in crate::executor) fn extglob_match_literal_nocase(
    pattern: &[char],
    p: usize,
    word: &[char],
    w: usize,
) -> bool {
    extglob_match_literal_with_case(pattern, p, word, w, true)
}

fn extglob_match_literal_with_case(
    pattern: &[char],
    p: usize,
    word: &[char],
    w: usize,
    nocase: bool,
) -> bool {
    if p == pattern.len() {
        return w == word.len();
    }
    match pattern[p] {
        '*' => {
            extglob_matches_at_with_case(pattern, p + 1, word, w, nocase)
                || (w < word.len() && extglob_matches_at_with_case(pattern, p, word, w + 1, nocase))
        }
        '?' => w < word.len() && extglob_matches_at_with_case(pattern, p + 1, word, w + 1, nocase),
        '[' => {
            if let Some((matched, next)) =
                case_bracket_expression_matches_with_case(pattern, p, word.get(w).copied(), nocase)
            {
                matched && extglob_matches_at_with_case(pattern, next, word, w + 1, nocase)
            } else {
                w < word.len()
                    && chars_match(pattern[p], word[w], nocase)
                    && extglob_matches_at_with_case(pattern, p + 1, word, w + 1, nocase)
            }
        }
        '\\' if p + 1 < pattern.len() => {
            w < word.len()
                && chars_match(pattern[p + 1], word[w], nocase)
                && extglob_matches_at_with_case(pattern, p + 2, word, w + 1, nocase)
        }
        c => {
            w < word.len()
                && chars_match(c, word[w], nocase)
                && extglob_matches_at_with_case(pattern, p + 1, word, w + 1, nocase)
        }
    }
}

pub(in crate::executor) fn case_bracket_expression_matches_with_case(
    pattern: &[char],
    start: usize,
    candidate: Option<char>,
    nocase: bool,
) -> Option<(bool, usize)> {
    let mut index = start + 1;
    if index >= pattern.len() {
        return None;
    }

    let negated = matches!(pattern[index], '!' | '^');
    if negated {
        index += 1;
    }

    let mut matched = false;
    let mut saw_member = false;
    let candidate = candidate?;
    let candidate_cmp = comparable_char(candidate, nocase);
    while index < pattern.len() {
        if pattern[index] == ']' && saw_member {
            return Some((if negated { !matched } else { matched }, index + 1));
        }

        // lib/glob/glob.c parse_bracket: a backslash-escaped `]` is a literal
        // bracket member, not the closing bracket. `[[:alpha:]\]` therefore
        // has an unterminated bracket (the `]` after `\` is a member), so the
        // pattern never closes and cannot match (posixpat.tests ok 21).
        // `\x18` is a legacy protected-literal-backslash marker that may still
        // reach this matcher; treat it exactly like a real backslash here.
        if matches!(
            pattern[index],
            '\\' | crate::executor::markers::PATTERN_LITERAL_BACKSLASH
                | crate::executor::markers::CTLESC
        ) && index + 1 < pattern.len()
        {
            let lit = pattern[index + 1];
            if chars_match(lit, candidate, nocase) {
                matched = true;
            }
            saw_member = true;
            // sm_loop.c BRACKET:527-534+568: the escaped member may anchor a
            // range (`[\a-z]` is the range a-z); the range end may itself be
            // escaped (`[\a-\z]`).
            if index + 3 < pattern.len() && pattern[index + 2] == '-' && pattern[index + 3] != ']' {
                let mut end_index = index + 3;
                if matches!(
                    pattern[end_index],
                    '\\' | crate::executor::markers::PATTERN_LITERAL_BACKSLASH
                        | crate::executor::markers::CTLESC
                ) && end_index + 1 < pattern.len()
                {
                    end_index += 1;
                }
                let end = pattern[end_index];
                let start_cmp = comparable_char(lit, nocase);
                let end_cmp = comparable_char(end, nocase);
                if start_cmp <= candidate_cmp && candidate_cmp <= end_cmp {
                    matched = true;
                }
                index = end_index + 1;
                continue;
            }
            index += 2;
            continue;
        }

        // POSIX bracket prefixes: [.sym.] collating symbols and [=c=]
        // equivalence classes (lib/glob/sm_loop.c). A valid symbol collapses
        // to one member character and may anchor a range; an unknown symbol
        // degenerates to its literal characters (posixpat.tests collating
        // section).
        if pattern[index] == '[' {
            if let Some((members, next_index)) = parse_collating_or_equivalence(pattern, index) {
                for member in &members {
                    if chars_match(*member, candidate, nocase) {
                        matched = true;
                    }
                }
                saw_member = true;
                if members.len() == 1 {
                    if let Some((end_char, after)) = collating_range_end(pattern, next_index) {
                        let start_cmp = comparable_char(members[0], nocase);
                        let end_cmp = comparable_char(end_char, nocase);
                        if start_cmp <= candidate_cmp && candidate_cmp <= end_cmp {
                            matched = true;
                        }
                        saw_member = true;
                        index = after;
                        continue;
                    }
                }
                index = next_index;
                continue;
            }
        }

        let current = pattern[index];
        if let Some((class_matched, next_index)) =
            bracket_posix_class_matches(pattern, index, candidate)
        {
            if class_matched {
                matched = true;
            }
            saw_member = true;
            index = next_index;
        } else if index + 2 < pattern.len()
            && pattern[index + 1] == '-'
            && pattern[index + 2] != ']'
        {
            // sm_loop.c BRACKET:568-572: the range end may be an escaped
            // character (`[a-\z]` is the range a-z).
            let mut end_index = index + 2;
            if matches!(
                pattern[end_index],
                '\\' | crate::executor::markers::PATTERN_LITERAL_BACKSLASH
                    | crate::executor::markers::CTLESC
            ) && end_index + 1 < pattern.len()
            {
                end_index += 1;
            }
            let end = pattern[end_index];
            let current_cmp = comparable_char(current, nocase);
            let end_cmp = comparable_char(end, nocase);
            if current_cmp <= candidate_cmp && candidate_cmp <= end_cmp {
                matched = true;
            }
            saw_member = true;
            index = end_index + 1;
        } else {
            if chars_match(current, candidate, nocase) {
                matched = true;
            }
            saw_member = true;
            index += 1;
        }
    }

    None
}

/// Parses a `[.sym.]` collating symbol or `[=c=]` equivalence class at
/// index (which must point at the opening `[`). Returns the member
/// characters and the index just past the closing `.]` / `=]`. A valid
/// single-character symbol yields that character; a recognized multi-
/// character name resolves through the POSIX portable character set table;
/// an unrecognized symbol degenerates to its literal characters, matching
/// bash's sm_loop.c fallback for undefined collating symbols.
fn parse_collating_or_equivalence(pattern: &[char], index: usize) -> Option<(Vec<char>, usize)> {
    let open = *pattern.get(index + 1)?;
    if open != '.' && open != '=' {
        return None;
    }
    let mut scan = index + 2;
    while scan + 1 < pattern.len() {
        if pattern[scan] == open && pattern[scan + 1] == ']' {
            let body: Vec<char> = pattern[index + 2..scan].to_vec();
            if body.is_empty() {
                return None;
            }
            let members = if body.len() == 1 {
                body
            } else {
                let name: String = body.iter().collect();
                match named_collating_symbol(&name) {
                    Some(c) => vec![c],
                    None => body,
                }
            };
            return Some((members, scan + 2));
        }
        scan += 1;
    }
    None
}

/// Resolves a range whose start is a single-character collating symbol:
/// dash_index points at the `-`; the endpoint may be another `[.sym.]` /
/// `[=c=]` span, an escaped character, or a plain character. Returns the
/// endpoint character and the index just past it.
fn collating_range_end(pattern: &[char], dash_index: usize) -> Option<(char, usize)> {
    if pattern.get(dash_index) != Some(&'-') {
        return None;
    }
    let end_index = dash_index + 1;
    let end = *pattern.get(end_index)?;
    if end == ']' {
        return None;
    }
    if end == '[' {
        let (members, next) = parse_collating_or_equivalence(pattern, end_index)?;
        if members.len() == 1 {
            return Some((members[0], next));
        }
        return None;
    }
    if matches!(
        end,
        '\\' | crate::executor::markers::PATTERN_LITERAL_BACKSLASH
    ) {
        let after = *pattern.get(end_index + 1)?;
        return Some((after, end_index + 2));
    }
    Some((end, end_index + 1))
}

/// POSIX portable character set collating-symbol names (the set glibc's C
/// locale recognizes), mapped to their characters.
fn named_collating_symbol(name: &str) -> Option<char> {
    Some(match name {
        "NUL" => '\0',
        "space" => ' ',
        "tab" => '\t',
        "newline" => '\n',
        "vertical-tab" => '\u{000b}',
        "form-feed" => '\u{000c}',
        "carriage-return" => '\r',
        "exclamation-mark" => '!',
        "quotation-mark" => '"',
        "number-sign" => '#',
        "dollar-sign" => '$',
        "percent-sign" => '%',
        "ampersand" => '&',
        "apostrophe" => '\'',
        "left-parenthesis" => '(',
        "right-parenthesis" => ')',
        "asterisk" => '*',
        "plus-sign" => '+',
        "comma" => ',',
        "hyphen" => '-',
        "full-stop" => '.',
        "slash" => '/',
        "colon" => ':',
        "semicolon" => ';',
        "less-than-sign" => '<',
        "equals-sign" => '=',
        "greater-than-sign" => '>',
        "question-mark" => '?',
        "commercial-at" => '@',
        "left-square-bracket" => '[',
        "backslash" => '\\',
        "right-square-bracket" => ']',
        "circumflex-accent" => '^',
        "low-line" => '_',
        "grave-accent" => '`',
        "left-curly-bracket" => '{',
        "vertical-line" => '|',
        "right-curly-bracket" => '}',
        "tilde" => '~',
        _ => return None,
    })
}

fn chars_match(pattern: char, candidate: char, nocase: bool) -> bool {
    // Byte-chars (U+E100+byte) compare by the byte they carry: a pattern
    // byte-char meets a word byte-char as equal bytes, and a byte-char meets
    // a literal char only when the literal IS that byte (rubash#729 — GNU
    // matches bytes, subst.c/sm_loop.c walk unsigned char buffers). PUA
    // marker chars (E200-E4xx) never equal a byte: their codepoints exceed
    // 0xFF, so the `as u32` compare below stays false — #735 user data
    // (E400-escaped) can never be claimed as a byte value.
    if let (Some(p), Some(c)) = (byte_char_value(pattern), byte_char_value(candidate)) {
        return p == c;
    }
    if let Some(p) = byte_char_value(pattern) {
        return p as u32 == candidate as u32;
    }
    if let Some(c) = byte_char_value(candidate) {
        // Only an ASCII literal can name the same byte (GNU compares
        // bytes; a multibyte literal like U+00E9 is bytes C3 A9 in GNU and
        // never equals a lone carried 0xE9).
        return pattern.is_ascii() && pattern as u32 == c as u32;
    }
    if nocase {
        pattern.eq_ignore_ascii_case(&candidate)
    } else {
        pattern == candidate
    }
}

fn comparable_char(ch: char, nocase: bool) -> char {
    // Bracket ranges compare in byte order (sm_loop.c U_STRING_PRECOMPARE /
    // unsigned char ranges): a byte-char participates as its byte value so
    // `[$'\001'-$'\005']` covers the carried bytes, and a byte-char never
    // falls inside a literal ASCII range it does not carry.
    if let Some(byte) = byte_char_value(ch) {
        return char::from(byte);
    }
    if nocase {
        ch.to_ascii_lowercase()
    } else {
        ch
    }
}

fn bracket_posix_class_matches(
    pattern: &[char],
    start: usize,
    candidate: char,
) -> Option<(bool, usize)> {
    if pattern.get(start) != Some(&'[') || pattern.get(start + 1) != Some(&':') {
        return None;
    }

    let mut end = start + 2;
    while end + 1 < pattern.len() {
        if pattern[end] == ':' && pattern[end + 1] == ']' {
            let class: String = pattern[start + 2..end].iter().collect();
            return Some((posix_class_matches(&class, candidate), end + 2));
        }
        end += 1;
    }
    None
}

fn posix_class_matches(class: &str, candidate: char) -> bool {
    // A byte-char (U+E100+byte, the carried raw byte) classifies as its
    // byte: [[:cntrl:]] must match a carried 0x01 exactly like GNU's
    // iscntrl(byte) does (rubash#729 — subst.c/sm_loop.c classify raw
    // bytes, glibc C-locale <ctype.h> tables).
    let candidate = match byte_char_value(candidate) {
        Some(byte) => char::from(byte),
        None => candidate,
    };
    // Under a UTF-8 locale GNU resolves classes through wctype/iswctype
    // (lib/glob/smatch.c), so multibyte characters classify by the
    // locale's Unicode-derived tables (rubash#1621) — É is `upper`
    // there. The ASCII table below is the single-byte-locale semantics
    // and stays byte-exact for it (rubash#1533). Which table applies is
    // locale::is_utf8's decision alone; no other site re-decides it.
    if crate::locale::is_utf8() {
        return crate::locale::utf8_posix_class_matches(class, candidate);
    }
    match class {
        "alnum" => candidate.is_ascii_alphanumeric(),
        "alpha" => candidate.is_ascii_alphabetic(),
        "ascii" => candidate.is_ascii(),
        "blank" => matches!(candidate, ' ' | '\t'),
        "cntrl" => candidate.is_ascii_control(),
        "digit" => candidate.is_ascii_digit(),
        "graph" => candidate.is_ascii_graphic(),
        "lower" => candidate.is_ascii_lowercase(),
        "print" => candidate.is_ascii_graphic() || candidate == ' ',
        "punct" => candidate.is_ascii_punctuation(),
        // glibc C locale is_space: space, \t, \n, \v, \f, \r. Rust's
        // is_ascii_whitespace excludes \v (0x0b) — GNU matches it.
        "space" => matches!(candidate, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r'),
        "upper" => candidate.is_ascii_uppercase(),
        "word" => candidate.is_ascii_alphanumeric() || candidate == '_',
        "xdigit" => candidate.is_ascii_hexdigit(),
        _ => false,
    }
}
use super::extglob::extglob_matches_at_with_case;
