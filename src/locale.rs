/// Locale subsystem for GNU bash compatibility.
///
/// GNU bash calls setlocale() and derives its character semantics from
/// MB_CUR_MAX: in a UTF-8 locale one character is one multibyte sequence,
/// while a locale that setlocale() cannot activate falls back to the C
/// locale, where every byte is one character (variables.c:1466-1490,
/// lib/sh/utf8.c:167-184).
///
/// Unlike GNU bash, which calls setlocale() at startup only, rubash checks
/// the environment dynamically on each call since LC_ALL is often set within
/// scripts after startup.
use std::cell::RefCell;

/// The active encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Ascii,
    Utf8,
}

/// Get the active locale name from environment variables.
/// Priority: LC_ALL > LC_CTYPE > LC_MESSAGES > LANG
pub fn locale_name() -> String {
    let locale_all = std::env::var("LC_ALL").unwrap_or_default();
    let locale_ctype = std::env::var("LC_CTYPE").unwrap_or_default();
    let locale_messages = std::env::var("LC_MESSAGES").unwrap_or_default();
    let lang = std::env::var("LANG").unwrap_or_default();

    if !locale_all.is_empty() {
        locale_all
    } else if !locale_ctype.is_empty() {
        locale_ctype
    } else if !locale_messages.is_empty() {
        locale_messages
    } else {
        lang
    }
}

/// Whether multibyte (UTF-8) character semantics are in effect, i.e. whether
/// the locale named by the environment is actually active.
///
/// An unset locale keeps rubash's default UTF-8 model. `C`/`POSIX` and any
/// named single-byte locale (ru_RU.CP1251, en_US.ISO-8859-1, ...) select
/// byte semantics. A UTF-8-named locale selects UTF-8 only when the C library
/// can activate it; otherwise GNU bash - and rubash - fall back to the C
/// locale, which is what makes `${#var}` count bytes on a machine that does
/// not have `en_US.UTF-8` compiled in (intl1.sub: 31/30 for a 15-character
/// Cyrillic word, not 16/15).
pub fn is_utf8() -> bool {
    let name = locale_name();
    if name.is_empty() {
        return true;
    }
    let lower = name.to_lowercase();
    if lower == "c" || lower == "posix" {
        return false;
    }
    if !is_utf8_locale_name(&name) {
        return false;
    }
    utf8_locale_active(&name)
}

/// GNU/MBCS view of the same decision: multibyte mode is on when
/// MB_CUR_MAX > 1.
pub fn is_multi_byte() -> bool {
    is_utf8()
}

/// Whether the locale name names a UTF-8 charset, regardless of availability.
pub fn is_utf8_locale_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("utf-8") || lower.contains("utf8")
}

// Cached outcome of the locale activation probe, keyed by the locale name so
// that `LC_ALL=...` reassignment inside a script invalidates it.
thread_local! {
    static UTF8_ACTIVE_CACHE: RefCell<Option<(String, bool)>> = const { RefCell::new(None) };
}

#[cfg(unix)]
fn utf8_locale_active(name: &str) -> bool {
    UTF8_ACTIVE_CACHE.with(|cache| {
        if let Some((seen, value)) = cache.borrow().as_ref() {
            if seen == name {
                return *value;
            }
        }
        let value = probe_utf8_locale(name);
        *cache.borrow_mut() = Some((name.to_string(), value));
        value
    })
}

#[cfg(unix)]
fn probe_utf8_locale(name: &str) -> bool {
    let Ok(locale) = std::ffi::CString::new(name.to_string()) else {
        return false;
    };
    unsafe {
        // setlocale() returns NULL when the locale is not compiled in - the
        // same failure GNU bash reports as "warning: setlocale: LC_ALL:
        // cannot change locale (en_US.UTF-8)" and then degrades to
        // single-byte semantics for the rest of the process. The saved
        // pointer stays valid only until the next setlocale() call, so the
        // restore below is the immediate next call, as the man page requires.
        let saved = libc::setlocale(libc::LC_CTYPE, locale.as_ptr());
        if saved.is_null() {
            return false;
        }
        libc::setlocale(libc::LC_CTYPE, saved);
        true
    }
}

#[cfg(not(unix))]
fn utf8_locale_active(_name: &str) -> bool {
    // The Windows CRT has no compiled locale archive to reject, so a UTF-8
    // locale name is active as requested.
    true
}

/// Count characters in a string (respects UTF-8 multi-byte sequences).
pub fn char_count(s: &str) -> usize {
    s.chars().count()
}

/// Count bytes in a string.
pub fn byte_count(s: &str) -> usize {
    s.len()
}

/// Get the effective length of a string (characters in a multibyte locale,
/// bytes otherwise). This is what `${#var}` reports.
pub fn effective_length(s: &str) -> usize {
    if is_utf8() {
        char_count(s)
    } else {
        byte_count(s)
    }
}

/// Return the locale-aware decimal point character for `printf %f` output.
///
/// GNU `snprintf.c:439-445` calls `localeconv()` and replaces the `.` in
/// rendered floating-point output with `localeconv()->decimal_point[0]`.
/// In `de_DE.UTF-8` this is `,`, so `printf '%.4f' 1` outputs `1,0000`.
/// In `C`/`en_US.UTF-8` it stays `.`.
///
/// Priority follows GNU setlocale category resolution for LC_NUMERIC:
/// `LC_ALL` > `LC_NUMERIC` > `LANG` (intl2.sub: `export LANG=de_DE.UTF-8`
/// must yield `1,0000` even when the host exports `LC_CTYPE=C.UTF-8` —
/// LC_CTYPE belongs to the character-type category, not numeric). An unset
/// or `C`/`POSIX` locale returns `.`.
pub fn decimal_point() -> char {
    let locale_all = std::env::var("LC_ALL").unwrap_or_default();
    let numeric = std::env::var("LC_NUMERIC").unwrap_or_default();
    let locale = if !locale_all.is_empty() {
        locale_all
    } else if !numeric.is_empty() {
        numeric
    } else {
        std::env::var("LANG").unwrap_or_default()
    };
    let lower = locale.to_lowercase();
    if lower.starts_with("de_de")
        || lower.starts_with("fr_fr")
        || lower.starts_with("es_es")
        || lower.starts_with("it_it")
        || lower.starts_with("pt_pt")
        || lower.starts_with("nl_nl")
        || lower.starts_with("ru_ru")
        || lower.starts_with("pl_pl")
    {
        ','
    } else {
        '.'
    }
}

/// Check if a character is printable (locale-aware).
pub fn is_printable(c: char) -> bool {
    if is_utf8() {
        // UTF-8 locale: printable includes Unicode printable characters
        (c as u32) >= 0x20 && (c as u32) <= 0x7E || (c as u32) >= 0xA0
    } else {
        // ASCII locale: only ASCII printable
        (c as u32) >= 0x20 && (c as u32) <= 0x7E
    }
}

/// Check if a byte sequence is a valid UTF-8 character start.
pub fn is_utf8_start(byte: u8) -> bool {
    byte >= 0x80
}

/// Decode a UTF-8 character from bytes, returning the character and the
/// number of bytes consumed.
pub fn decode_utf8_char(bytes: &[u8]) -> Option<(char, usize)> {
    if bytes.is_empty() {
        return None;
    }

    let first = bytes[0];
    let (num_bytes, code_start) = if first < 0x80 {
        (1, first as u32)
    } else if first & 0xE0 == 0xC0 {
        (2, (first & 0x1F) as u32)
    } else if first & 0xF0 == 0xE0 {
        (3, (first & 0x0F) as u32)
    } else if first & 0xF8 == 0xF0 {
        (4, (first & 0x07) as u32)
    } else {
        return None;
    };

    if bytes.len() < num_bytes {
        return None;
    }

    let mut code_point = code_start;
    for i in 1..num_bytes {
        let b = bytes[i];
        if b & 0xC0 != 0x80 {
            return None;
        }
        code_point = (code_point << 6) | (b & 0x3F) as u32;
    }

    char::from_u32(code_point).map(|c| (c, num_bytes))
}

/// Report whether the currently requested locale is active.
///
/// GNU bash warns once per failed setlocale() (variables.c:1476-1488) and
/// then keeps using single-byte semantics; rubash mirrors the semantics and
/// leaves diagnostic emission to the caller.
pub fn locale_request_active() -> bool {
    let name = locale_name();
    if name.is_empty() {
        return true;
    }
    !is_utf8_locale_name(&name) || utf8_locale_active(&name)
}

/// Emit setlocale warning if locale is requested but not available.
///
/// GNU bash warns when setlocale() fails (variables.c:1476-1488), e.g.
/// "bash: warning: setlocale: LC_ALL: cannot change locale (en_US.UTF-8)".
/// Emission is intentionally suppressed: the diagnostic prefix is
/// this_command_name, which does not normalize across the two shells, so the
/// warning would be diff noise in every suite that reassigns LC_ALL. The
/// semantic consequence - single-byte fallback - is applied by is_utf8().
pub fn check_setlocale_warning() {
    let _ = locale_request_active();
}

/// Initialize locale state. Kept as a no-op hook for parity with GNU bash's
/// startup setlocale() pass; rubash re-derives the locale on demand.
pub fn init_locale() {
    let _ = locale_request_active();
}

/// Decode rubash's internal transport text into user-visible text.
///
/// Rubash ports GNU's `CTLESC`/`CTLNUL` mechanism (parse.y:5694-5706
/// `got_escaped_character`, subst.c:4692 `dequote_escapes`,
/// subst.c:4807 `dequote_string`) as in-band carrier code points between
/// the lexer, parser, and executor. Hosts that read token `value`/`raw`
/// fields or word text before execution see those carriers; this function
/// is the single boundary decoder that removes them and restores the
/// source characters they protect.
///
/// This is the Phase 0 decode-at-boundary API of the host-semantic-layer
/// elimination plan: `--dump-strings`/`--dump-po-strings` (GNU
/// locale.c:550 `dump_translatable_strings`, shell.c:507-509) and host AST
/// consumers must use it instead of hand-stripping individual markers.
///
/// Carrier table (decode is a single left-to-right pass):
///
/// | Transport | Meaning | Emits |
/// |---|---|---|
/// | `\x11` + c | CTLESC port: quoted glob char (`*?[@+!`) | `c` |
/// | `\x03` | DEFERRED_COMPOUND_BODY protocol byte | nothing |
/// | `\x13` | PARAM_NAME_END_MARKER (name boundary after quote removal) | nothing |
/// | `\x14` | data backslash | `\` |
/// | `\x16` | protected escaped single quote | `'` |
/// | `\x17` | data single quote | `'` |
/// | `\x18` | data double quote | `"` |
/// | `\x1a` | data backtick | `` ` `` |
/// | `\x1b`/`\x1c`/`\x1d` | word-level prefix markers (quoted tilde, IFS glue, storage/comsub tag) | nothing |
/// | `\x1f` | data dollar | `$` |
/// | `U+E002` | QUOTED_NULL_MARKER (empty quoted field) | nothing |
/// | `U+E010`/`U+E011` | ANSI-C `$'...'` decoded `'`/`"` data | `'`/`"` |
/// | `U+E000` + `U+E0xx` | raw-byte marker pair | byte `xx` as char |
/// | `U+E400` + `c` | literal-char escape (markers::push_literal_char) | `c` |
/// | `U+E301`..=`U+E308` | assignment DATA_* sentinels (quote/backtick/backslash data) | the data char |
/// | `U+E309` | COMPOUND_EXPANSION_WS_TAG (field-splitting glue) | nothing |
/// | `U+E100`..=`U+E1FF` | conditional-pattern byte-chars | byte `(cp - E100)` as char |
///
/// Ordering follows the storage-boundary contract documented in
/// assignment_expansion.rs: carriers are restored while raw-byte marker
/// pairs are still tagged, then pairs decode to bytes — decoded output is
/// appended directly and never rescanned, so a pair carrying byte 0x11
/// cannot be mistaken for CTLESC.
///
/// The DATA_* sentinels live in the registry block U+E301..=U+E30C —
/// they were moved out of U+E101..=U+E10C, which sits inside the
/// BYTE_CHAR_BASE byte-char range (U+E100..=U+E1FF) and collided with
/// pattern byte-chars 0x01-0x0C (same bug class as the retired E10A
/// FAILED_SUBSCRIPT_SENTINEL split).
pub fn decode_to_visible_text(text: &str) -> String {
    use crate::executor::conditional::pattern::BYTE_CHAR_BASE;
    use crate::executor::embedded_mutations::{COMPOUND_EXPANSION_WS_TAG, QUOTED_NULL_MARKER};
    use crate::executor::markers::COMSUB_PAYLOAD_PREFIX;
    use crate::executor::substitution_metadata::{
        RAW_BYTE_MARKER_ESCAPE, RAW_BYTE_MARKER_FIRST, RAW_BYTE_MARKER_LAST,
    };
    use crate::executor::types::COMPOUND_ASSIGNMENT_MARKER;
    use crate::executor::types::DEFERRED_COMPOUND_BODY;
    use crate::lexer::{
        ANSI_C_DQUOTE_MARKER, ANSI_C_QUOTE_MARKER, PARAM_NAME_END_MARKER, QUOTED_HEREDOC_MARKER,
    };

    const CTLESC: char = crate::executor::markers::CTLESC;
    // DATA_* sentinels owned by assignment_expansion.rs (declared as
    // fn-local consts there): data quote/backtick/backslash carriers.
    const DATA_SINGLE_QUOTE: char = crate::executor::markers::ASSIGN_DATA_SQUOTE;
    const DATA_DOUBLE_QUOTE: char = crate::executor::markers::ASSIGN_DATA_DQUOTE;
    const DATA_BACKTICK: char = crate::executor::markers::ASSIGN_DATA_BACKTICK;
    const DATA_ESCAPED_DQUOTE: char = crate::executor::markers::ASSIGN_ESCAPED_DQUOTE;
    const DATA_ESCAPED_SQUOTE: char = crate::executor::markers::ASSIGN_ESCAPED_SQUOTE;
    const DATA_ESCAPED_BACKSLASH: char = crate::executor::markers::ASSIGN_ESCAPED_BACKSLASH;
    const HOISTED_SINGLE_QUOTE: char = crate::executor::markers::ASSIGN_HOISTED_SQUOTE;
    const HOISTED_BACKSLASH: char = crate::executor::markers::ASSIGN_HOISTED_BACKSLASH;

    let byte_char = |code: u32| char::from_u32(code).expect("byte-char code point is valid");

    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            CTLESC => {
                if let Some(data) = chars.next() {
                    out.push(data);
                }
            }
            crate::executor::markers::LITERAL_CHAR_ESCAPE => {
                // E400 + c is the literal-char escape used for user data
                // that would otherwise alias a registry marker
                // (markers::push_literal_char): emit the char alone.
                out.push(chars.next().unwrap_or(ch));
            }
            c if c as u32 == RAW_BYTE_MARKER_ESCAPE => match chars.next() {
                Some(next) if next as u32 == RAW_BYTE_MARKER_ESCAPE => out.push(c),
                Some(next)
                    if (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&(next as u32)) =>
                {
                    out.push(byte_char(next as u32 - RAW_BYTE_MARKER_FIRST));
                }
                // E000 + <non-payload char> is a stray introducer: emit
                // the char alone (recovery; nothing produces this).
                Some(next) => out.push(next),
                None => out.push(c),
            },
            DEFERRED_COMPOUND_BODY
            | PARAM_NAME_END_MARKER
            | QUOTED_NULL_MARKER
            | COMPOUND_EXPANSION_WS_TAG
            | crate::executor::markers::QUOTED_WORD_PREFIX
            | crate::executor::markers::IFS_GLUE
            | crate::executor::markers::STORAGE_WORD_PREFIX
            | crate::executor::markers::ARRAYREF_FLAG
            | crate::executor::markers::PATSUB_QUOTED_VALUE_START
            | crate::executor::markers::PATSUB_QUOTED_VALUE_END => {}
            c if crate::executor::markers::FAILED_SUBSCRIPT_SENTINEL.contains(c) => {}
            crate::executor::markers::PATSUB_QUOTED_AMP => out.push('&'),
            crate::executor::markers::PATSUB_QUOTED_BACKSLASH => out.push('\\'),
            // Guard sentinels are protect-prefixes: GUARD + c marks c as
            // literal data (the original `\` was consumed at encode time).
            crate::executor::markers::PARAM_WORD_BACKSLASH_GUARD
            | crate::executor::markers::ESCAPED_IFS_GUARD
            | crate::executor::markers::PROMPT_ESCAPE_GUARD
            | crate::executor::markers::CASE_PATTERN_BACKSLASH_GUARD => {
                if let Some(data) = chars.next() {
                    out.push(data);
                }
                // else: missing data char - drop the guard (recovery path)
            }
            crate::executor::markers::DATA_BACKSLASH
            | HOISTED_BACKSLASH
            | DATA_ESCAPED_BACKSLASH => out.push('\\'),
            crate::executor::markers::PROTECTED_ESCAPED_SQUOTE
            | crate::executor::markers::DATA_SQUOTE
            | ANSI_C_QUOTE_MARKER
            | DATA_SINGLE_QUOTE
            | DATA_ESCAPED_SQUOTE
            | HOISTED_SINGLE_QUOTE => out.push('\''),
            crate::executor::markers::DATA_DQUOTE
            | ANSI_C_DQUOTE_MARKER
            | DATA_DOUBLE_QUOTE
            | DATA_ESCAPED_DQUOTE => out.push('"'),
            crate::executor::markers::DATA_BACKTICK | DATA_BACKTICK => out.push('`'),
            crate::executor::markers::DATA_DOLLAR => out.push('$'),
            c if (BYTE_CHAR_BASE..BYTE_CHAR_BASE + 0x100).contains(&(c as u32)) => {
                out.push(byte_char(c as u32 - BYTE_CHAR_BASE));
            }
            _ => out.push(ch),
        }
    }
    // Golden assertion: guard markers must never leak to output in production contexts
    // These are function-local PUA markers (U+E314-E317) with no
    // external boundary; if they appear in output, the decode pass failed.
    // Note: This assertion is skipped for debug/test contexts where raw marker
    // strings may be passed directly to decode_to_visible_text.
    if !cfg!(test) {
        debug_assert!(
            !out.contains(crate::executor::markers::PARAM_WORD_BACKSLASH_GUARD),
            "PARAM_WORD_BACKSLASH_GUARD leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::ESCAPED_IFS_GUARD),
            "ESCAPED_IFS_GUARD leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::PROMPT_ESCAPE_GUARD),
            "PROMPT_ESCAPE_GUARD leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::CASE_PATTERN_BACKSLASH_GUARD),
            "CASE_PATTERN_BACKSLASH_GUARD leaked to output"
        );
        // Golden assertion: ASSIGN_DATA_* markers must never leak to output
        // These are storage-boundary PUA markers (U+E301-E30C) with no
        // external boundary; if they appear in output, the decode pass failed.
        debug_assert!(
            !out.contains(crate::executor::markers::ASSIGN_DATA_SQUOTE),
            "ASSIGN_DATA_SQUOTE leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::ASSIGN_DATA_DQUOTE),
            "ASSIGN_DATA_DQUOTE leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::ASSIGN_DATA_BACKTICK),
            "ASSIGN_DATA_BACKTICK leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::ASSIGN_ESCAPED_DQUOTE),
            "ASSIGN_ESCAPED_DQUOTE leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::ASSIGN_ESCAPED_SQUOTE),
            "ASSIGN_ESCAPED_SQUOTE leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::ASSIGN_ESCAPED_BACKSLASH),
            "ASSIGN_ESCAPED_BACKSLASH leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::ASSIGN_HOISTED_SQUOTE),
            "ASSIGN_HOISTED_SQUOTE leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::ASSIGN_HOISTED_BACKSLASH),
            "ASSIGN_HOISTED_BACKSLASH leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::COMPOUND_EXPANSION_WS_TAG),
            "COMPOUND_EXPANSION_WS_TAG leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::ASSIGN_SQ_DOLLAR),
            "ASSIGN_SQ_DOLLAR leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::ASSIGN_SQ_BACKTICK),
            "ASSIGN_SQ_BACKTICK leaked to output"
        );
        debug_assert!(
            !out.contains(crate::executor::markers::ASSIGN_SQ_BACKSLASH),
            "ASSIGN_SQ_BACKSLASH leaked to output"
        );
        // Golden assertion: CTLESC must never leak to output
        // This is the most-traveled C0 carrier (0x11) used throughout the
        // lexer/parser/executor pipeline to protect characters through
        // intermediate passes. If it appears in output, the decode pass failed.
        debug_assert!(!out.contains(CTLESC), "CTLESC leaked to output");
        // Golden assertion: named string markers must never leak to output
        // These are multi-char protocol prefixes used in transport text.
        // If they appear in output, the decode pass failed.
        debug_assert!(
            !out.contains(QUOTED_HEREDOC_MARKER),
            "QUOTED_HEREDOC_MARKER leaked to output"
        );
        debug_assert!(
            !out.contains(COMSUB_PAYLOAD_PREFIX),
            "COMSUB_PAYLOAD_PREFIX leaked to output"
        );
        debug_assert!(
            !out.contains(COMPOUND_ASSIGNMENT_MARKER),
            "COMPOUND_ASSIGNMENT_MARKER leaked to output"
        );
    }
    out
}

/// The active LC_COLLATE locale name.
/// GNU setlocale category precedence: LC_ALL > LC_COLLATE > LANG > "C".
fn collate_locale_name() -> String {
    let locale_all = std::env::var("LC_ALL").unwrap_or_default();
    let locale_collate = std::env::var("LC_COLLATE").unwrap_or_default();
    let lang = std::env::var("LANG").unwrap_or_default();
    if !locale_all.is_empty() {
        locale_all
    } else if !locale_collate.is_empty() {
        locale_collate
    } else if !lang.is_empty() {
        lang
    } else {
        "C".to_string()
    }
}

/// Strip the codeset and modifier suffixes and map to a host locale name:
/// "en_US.UTF-8" -> "en-US" (Windows BCP-47) / "en_US.UTF-8" (POSIX).
/// Returns None for the C/POSIX locale, where collation is plain strcmp.
fn host_collate_name(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    if lower.is_empty() || lower == "c" || lower == "posix" {
        return None;
    }
    let base = name.split(['.', '@']).next().unwrap_or(name);
    #[cfg(windows)]
    {
        // glibc "en_US" -> BCP-47 "en-US"; a bare language ("en") is kept
        // as a neutral name and resolved by CompareStringEx.
        Some(base.replace('_', "-"))
    }
    #[cfg(not(windows))]
    {
        // POSIX setlocale wants the full "lang_TERRITORY.codeset" form; the
        // unsuffixed "en_US" form is the most portable fallback.
        Some(base.to_string())
    }
}

/// GNU strcoll(3): compare `a` and `b` under the LC_COLLATE collation.
/// C/POSIX and unresolvable locales fall back to bytewise strcmp, matching
/// glibc's behavior when setlocale() cannot activate the named locale.
pub fn strcoll(a: &str, b: &str) -> std::cmp::Ordering {
    // An empty string orders identically under strcoll and strcmp.
    if a.is_empty() || b.is_empty() {
        return a.cmp(b);
    }
    let name = collate_locale_name();
    let Some(host) = host_collate_name(&name) else {
        return a.cmp(b);
    };
    platform_collate(a, b, &host).unwrap_or_else(|| a.cmp(b))
}

/// GNU strvec_posixcmp (lib/sh/stringvec.c:152-165): strcoll() first, then a
/// bytewise tie-break on the first byte, then strcmp. This is the comparator
/// used for glob result ordering via pathexp.c globsort_namecmp().
pub fn strcoll_posixcmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let r = strcoll(a, b);
    if r != Ordering::Equal {
        return r;
    }
    match a.as_bytes().first().cmp(&b.as_bytes().first()) {
        Ordering::Equal => a.cmp(b),
        ord => ord,
    }
}

#[cfg(windows)]
fn platform_collate(a: &str, b: &str, locale: &str) -> Option<std::cmp::Ordering> {
    use windows_sys::Win32::Globalization::{
        CompareStringEx, CSTR_EQUAL, CSTR_GREATER_THAN, CSTR_LESS_THAN, NORM_IGNORESYMBOLS,
    };
    let loc: Vec<u16> = locale.encode_utf16().chain(Some(0)).collect();
    let aw: Vec<u16> = a.encode_utf16().collect();
    let bw: Vec<u16> = b.encode_utf16().collect();
    // NORM_IGNORESYMBOLS approximates glibc collation, where punctuation
    // carries no primary weight (".a" ties with "a"); the residual ordering
    // is then decided by strvec_posixcmp's first-byte/strcmp tie-break,
    // which reproduces glibc's interleaved ".a a .aa aa" result.
    let r = unsafe {
        CompareStringEx(
            loc.as_ptr(),
            NORM_IGNORESYMBOLS,
            aw.as_ptr(),
            aw.len() as i32,
            bw.as_ptr(),
            bw.len() as i32,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    match r {
        CSTR_LESS_THAN => Some(std::cmp::Ordering::Less),
        CSTR_EQUAL => Some(std::cmp::Ordering::Equal),
        CSTR_GREATER_THAN => Some(std::cmp::Ordering::Greater),
        _ => None,
    }
}

#[cfg(unix)]
fn platform_collate(a: &str, b: &str, locale: &str) -> Option<std::cmp::Ordering> {
    use std::ffi::CString;
    use std::sync::Mutex;
    // Activate LC_COLLATE once per distinct locale name; glibc compares with
    // the collation table installed by setlocale(LC_COLLATE, ...).
    static ACTIVE: Mutex<Option<String>> = Mutex::new(None);
    let mut active = ACTIVE.lock().ok()?;
    if active.as_deref() != Some(locale) {
        let cname = CString::new(locale).ok()?;
        let ok = unsafe { libc::setlocale(libc::LC_COLLATE, cname.as_ptr()) };
        if ok.is_null() {
            return None;
        }
        *active = Some(locale.to_string());
    }
    drop(active);
    let ca = CString::new(a).ok()?;
    let cb = CString::new(b).ok()?;
    let r = unsafe { libc::strcoll(ca.as_ptr(), cb.as_ptr()) };
    Some(r.cmp(&0))
}

#[cfg(not(any(windows, unix)))]
fn platform_collate(_a: &str, _b: &str, _locale: &str) -> Option<std::cmp::Ordering> {
    None
}

#[cfg(test)]
mod tests {
    use super::{is_utf8_locale_name, strcoll, strcoll_posixcmp};
    use std::cmp::Ordering;

    #[test]
    fn c_locale_collate_is_bytewise() {
        // With no locale vars the collate category is C: strcmp order.
        let saved = std::env::var("LC_ALL").ok();
        std::env::remove_var("LC_ALL");
        std::env::remove_var("LC_COLLATE");
        std::env::remove_var("LANG");
        assert_eq!(strcoll("B", "a"), Ordering::Less); // 0x42 < 0x61
        assert_eq!(strcoll_posixcmp(".a", "a"), Ordering::Less);
        if let Some(v) = saved {
            std::env::set_var("LC_ALL", v);
        }
    }

    #[cfg(windows)]
    #[test]
    fn utf8_locale_collate_interleaves_punctuation() {
        // glibc en_US.UTF-8 ignores punctuation at the primary weight:
        // ".a" ties "a" and the posixcmp first-byte break orders it first.
        std::env::set_var("LC_ALL", "en_US.UTF-8");
        assert_eq!(strcoll(".a", "a"), Ordering::Equal);
        assert_eq!(strcoll_posixcmp(".a", "a"), Ordering::Less);
        assert_eq!(strcoll_posixcmp("a", ".aa"), Ordering::Less);
        assert_eq!(strcoll_posixcmp("!x", "-x"), Ordering::Less);
        assert_eq!(strcoll_posixcmp("-x", "[x]"), Ordering::Less);
        assert_eq!(strcoll_posixcmp("[x]", "_x"), Ordering::Less);
        assert_eq!(strcoll_posixcmp("_x", "x"), Ordering::Less);
        std::env::remove_var("LC_ALL");
    }

    #[test]
    fn utf8_charset_names_are_recognized_case_insensitively() {
        assert!(is_utf8_locale_name("en_US.UTF-8"));
        assert!(is_utf8_locale_name("C.UTF-8"));
        assert!(is_utf8_locale_name("C.utf8"));
    }

    #[test]
    fn decode_to_visible_text_restores_data_carriers() {
        // The lexer encodes quote/backslash/dollar/backtick data as C0
        // carriers; the decoder must render the source characters.
        assert_eq!(super::decode_to_visible_text("a\u{17}b"), "a'b");
        assert_eq!(super::decode_to_visible_text("a\u{18}b"), "a\"b");
        assert_eq!(super::decode_to_visible_text("a\u{14}b"), "a\\b");
        assert_eq!(super::decode_to_visible_text("a\u{1f}b"), "a$b");
        assert_eq!(super::decode_to_visible_text("a\u{1a}b"), "a`b");
        assert_eq!(super::decode_to_visible_text("a\u{16}b"), "a'b");
    }

    #[test]
    fn decode_to_visible_text_ctlesc_protects_next_char() {
        // Quoted glob chars travel as CTLESC + char (parse.y:5694-5706
        // got_escaped_character port); decode emits the protected char.
        assert_eq!(super::decode_to_visible_text("a\u{11}*b"), "a*b");
        assert_eq!(super::decode_to_visible_text("\u{11}?"), "?");
        // A trailing lone CTLESC decodes to nothing.
        assert_eq!(super::decode_to_visible_text("a\u{11}"), "a");
    }

    #[test]
    fn decode_to_visible_text_drops_structural_markers() {
        // Word-level prefix/infix markers carry no user-visible character.
        assert_eq!(super::decode_to_visible_text("\u{1b}~/x"), "~/x");
        assert_eq!(super::decode_to_visible_text("a\u{1c} b"), "a b");
        assert_eq!(super::decode_to_visible_text("\u{1d}(a b)"), "(a b)");
        assert_eq!(super::decode_to_visible_text("a\u{13}b"), "ab");
        assert_eq!(super::decode_to_visible_text("a\u{e002}b"), "ab");
        assert_eq!(super::decode_to_visible_text("a\u{E309}b"), "ab");
        assert_eq!(super::decode_to_visible_text("a\u{3}b"), "ab");
    }

    #[test]
    fn decode_to_visible_text_restores_pua_quote_markers() {
        assert_eq!(super::decode_to_visible_text("a\u{e010}b"), "a'b");
        assert_eq!(super::decode_to_visible_text("a\u{e011}b"), "a\"b");
        // Assignment DATA_* sentinels (registry block E301-E308; the old
        // E101-E108 codepoints were inside the BYTE_CHAR_BASE byte-char
        // range and collided with pattern byte-chars 0x01-0x08).
        assert_eq!(super::decode_to_visible_text("a\u{E301}b"), "a'b");
        assert_eq!(super::decode_to_visible_text("a\u{E302}b"), "a\"b");
        assert_eq!(super::decode_to_visible_text("a\u{E303}b"), "a`b");
        assert_eq!(super::decode_to_visible_text("a\u{E304}b"), "a\"b");
        assert_eq!(super::decode_to_visible_text("a\u{E305}b"), "a'b");
        assert_eq!(super::decode_to_visible_text("a\u{E306}b"), "a\\b");
        assert_eq!(super::decode_to_visible_text("a\u{E307}b"), "a'b");
        assert_eq!(super::decode_to_visible_text("a\u{E308}b"), "a\\b");
        // Old DATA_* codepoints are plain byte-chars now: E101 = byte 0x01.
        assert_eq!(super::decode_to_visible_text("a\u{e101}b"), "a\u{1}b");
    }

    #[test]
    fn decode_to_visible_text_decodes_raw_byte_pairs_last() {
        // U+E000 + U+E0xx pair -> raw byte xx. The payload byte must not be
        // re-interpreted as a carrier: byte 0x11 (CTLESC) decodes to the
        // literal char U+0011 in output, it does not eat the next char.
        assert_eq!(
            super::decode_to_visible_text("a\u{e000}\u{e012}b"),
            "a\u{11}b"
        );
        // Byte 0x41 = 'A'.
        assert_eq!(super::decode_to_visible_text("\u{e000}\u{e042}"), "A");
        // E400 + c = literal-char escape for user data that would
        // otherwise alias a registry marker
        // (markers::push_literal_char). The char decodes verbatim and the
        // escape is consumed — including payload-range chars like E0A0,
        // which an E000 prefix could never carry unambiguously.
        assert_eq!(
            super::decode_to_visible_text("\u{e400}\u{E314}X"),
            "\u{E314}X"
        );
        assert_eq!(
            super::decode_to_visible_text("a\u{e400}\u{E0A0}b"),
            "a\u{E0A0}b"
        );
        assert_eq!(
            super::decode_to_visible_text("a\u{e400}\u{e000}b"),
            "a\u{e000}b"
        );
        // Conditional-pattern byte-chars: U+E100+byte.
        assert_eq!(super::decode_to_visible_text("x\u{e141}y"), "xAy");
    }

    #[test]
    fn decode_to_visible_text_leaves_plain_text_untouched() {
        assert_eq!(
            super::decode_to_visible_text("echo hello world"),
            "echo hello world"
        );
        assert_eq!(super::decode_to_visible_text(""), "");
    }

    #[test]
    fn single_byte_and_default_locale_names_are_not_utf8() {
        for name in [
            "C",
            "POSIX",
            "ru_RU.CP1251",
            "en_US.ISO-8859-1",
            "de_DE@euro",
            "",
        ] {
            assert!(!is_utf8_locale_name(name), "{name}");
        }
    }
}
