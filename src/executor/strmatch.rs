//! Port of GNU bash `lib/glob/strmatch.c` + `lib/glob/smatch.c` for the
//! loadable-builtin C ABI, covering both `internal_strmatch` (single-byte
//! compile of sm_loop.c) and `internal_wstrmatch` (the wide-char compile
//! smatch.c:585-603 `xstrmatch` selects under a multibyte locale).
//!
//! GNU bash exports `strmatch` (lib/glob/strmatch.c:32-39) so that loadable
//! builtins linked with `-undefined dynamic_lookup` can call back into the
//! shell image; `glob-bracket.tests` builds such a plugin (`strmatch.so`)
//! and crashes when the host does not export the symbol. This module exports
//! `strmatch` with the same signature and semantics.
//!
//! The engine operates on `char` elements: bytes are lifted through the
//! Latin-1 bijection (`char::from(u8)`), which preserves the byte domain's
//! equality and ordering, so `strmatch_bytes` is the single-byte compile
//! (`FOLD` ASCII-only, `ISDIRSEP` = `/`, C-locale collation) while
//! `strmatch_chars` is the wide compile (Unicode `fold`/character classes
//! via the internal FNM_WIDE selector, standing in for GNU's `towlower`/
//! `isw*` wide calls).

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};

// Flag values from lib/glob/strmatch.h:35-50.
pub const FNM_PATHNAME: u32 = 1 << 0;
pub const FNM_NOESCAPE: u32 = 1 << 1;
pub const FNM_PERIOD: u32 = 1 << 2;
pub const FNM_LEADING_DIR: u32 = 1 << 3;
pub const FNM_CASEFOLD: u32 = 1 << 4;
pub const FNM_EXTMATCH: u32 = 1 << 5;
pub const FNM_DOTDOT: u32 = 1 << 7;
const FNM_NOMATCH: c_int = 1;

/// Internal selector, not part of the public flag set (strmatch.h:35-50
/// ends at bit 7): when set, `fold` and `is_cclass` use the wide compile's
/// `towlower`/`isw*` semantics instead of the single-byte ASCII domain.
const FNM_WIDE: u32 = 1 << 8;

/// lib/glob/strmatch.c:32-39 `strmatch()`: exported so loadable builtins can
/// resolve the symbol against the host image (GNU exports it from bash for
/// the same purpose). Arguments follow the C order: pattern, string, flags.
/// Returns 0 on match, FNM_NOMATCH (1) otherwise.
///
/// GNU's exported symbol is a thin wrapper over `xstrmatch` (strmatch.c:33-39),
/// NOT over `internal_strmatch`: a loadable builtin therefore observes the
/// same locale dispatch the shell's own matching does. Routing it to the
/// single-byte compile made `strmatch("u/?.txt", "u/Ä.txt", FNM_PATHNAME)`
/// answer 1 where GNU answers 0 (glob-bracket.tests canonicalizes through
/// this call under a UTF-8 locale).
#[no_mangle]
pub unsafe extern "C" fn strmatch(
    pattern: *const c_char,
    string: *const c_char,
    flags: c_int,
) -> c_int {
    if pattern.is_null() || string.is_null() {
        return FNM_NOMATCH;
    }
    let pattern = CStr::from_ptr(pattern).to_bytes();
    let string = CStr::from_ptr(string).to_bytes();
    if xstrmatch(pattern, string, flags as u32) {
        0
    } else {
        FNM_NOMATCH
    }
}

/// smatch.c:585-603 `xstrmatch()`: the locale dispatch every caller goes
/// through. `MB_CUR_MAX == 1` stays single-byte; otherwise a pattern and
/// name that are both single-byte and whose bracket classes are all
/// POSIX-defined still punt to the single-byte compile, and anything else
/// is converted with `mbsrtowcs` and handed to `internal_wstrmatch`.
/// rubash derives `MB_CUR_MAX > 1` from the shell's locale model
/// (`is_multi_byte_active()`, the same predicate the expansion engines use);
/// an unconvertible byte string falls back to the single-byte compile like
/// xdupmbstowcs' `(size_t)-1`.
pub(crate) fn xstrmatch(pattern: &[u8], string: &[u8], flags: u32) -> bool {
    if !crate::locale::is_multi_byte_active() {
        return strmatch_bytes(pattern, string, flags);
    }
    // mbsmbchar (lib/sh/utf8.c:49-58) looks for a continuation byte; a
    // string without one holds no multibyte character.
    let single_byte = |s: &[u8]| !s.iter().any(|&b| b & 0xc0 == 0x80);
    if single_byte(string) && single_byte(pattern) && posix_cclass_only(pattern) {
        return strmatch_bytes(pattern, string, flags);
    }
    let (Ok(pat), Ok(name)) = (std::str::from_utf8(pattern), std::str::from_utf8(string)) else {
        return strmatch_bytes(pattern, string, flags);
    };
    let pat: Vec<char> = pat.chars().collect();
    let name: Vec<char> = name.chars().collect();
    strmatch_chars(&pat, &name, flags)
}

/// smatch.c:505-545 `posix_cclass_only()`: 1 when the pattern contains no
/// `[:class:]` expression, or only POSIX-defined ones (the names
/// `cclass_name[]` accepts). An unterminated `[:...]` ends the scan with 1;
/// an unknown class name makes it 0 (the wide compile may know locale
/// classes the single-byte compile does not).
fn posix_cclass_only(pattern: &[u8]) -> bool {
    const CCLASS_NAMES: [&[u8]; 14] = [
        b"ascii", b"alnum", b"alpha", b"blank", b"cntrl", b"digit", b"graph", b"lower", b"print",
        b"punct", b"space", b"upper", b"word", b"xdigit",
    ];
    let mut i = 0usize;
    while let Some(off) = pattern[i..].iter().position(|&b| b == b'[') {
        let p = i + off;
        if pattern.get(p + 1) != Some(&b':') {
            i = p + 1;
            continue;
        }
        let mut j = p + 2;
        let end = loop {
            if j >= pattern.len() {
                // No closing `:]`: C breaks out of the scan and returns 1.
                return true;
            }
            if pattern[j] == b':' && pattern.get(j + 1) == Some(&b']') {
                break j;
            }
            j += 1;
        };
        let name = &pattern[p + 2..end];
        // `char cc[16]`: a longer name cannot be a valid POSIX class.
        if name.len() >= 16 || !CCLASS_NAMES.contains(&name) {
            return false;
        }
        i = end + 2;
    }
    true
}

/// `internal_strmatch` entry point (smatch.c FCT, single-byte compile):
/// GMATCH over bytes lifted to chars through the Latin-1 bijection.
pub(crate) fn strmatch_bytes(pattern: &[u8], string: &[u8], flags: u32) -> bool {
    let pat: Vec<char> = pattern.iter().map(|&b| b as char).collect();
    let s: Vec<char> = string.iter().map(|&b| b as char).collect();
    gmatch(&pat, &s, flags & !FNM_WIDE)
}

/// `internal_wstrmatch` entry point (smatch.c wide compile): GMATCH over
/// decoded characters. Callers perform the smatch.c:585-603 `xstrmatch`
/// locale check before selecting this path.
pub(crate) fn strmatch_chars(pattern: &[char], string: &[char], flags: u32) -> bool {
    gmatch(pattern, string, flags | FNM_WIDE)
}

#[inline]
fn fold(c: char, flags: u32) -> char {
    // smatch.c:301-304 FOLD: single-byte compile is ASCII TOLOWER only (the
    // UTF8_SINGLEBYTE guard means non-ASCII bytes are never folded); the
    // wide compile folds through `towlower`, which handles non-ASCII.
    if flags & FNM_CASEFOLD == 0 {
        return c;
    }
    if flags & FNM_WIDE != 0 {
        c.to_lowercase().next().unwrap_or(c)
    } else if c.is_ascii_uppercase() {
        ((c as u8) + 32) as char
    } else {
        c
    }
}

/// smatch.c:313 PDOT_OR_DOTDOT: `s` begins with `.`/`..` followed by a path
/// separator or end. `s` here is the slice starting at the checked position.
#[inline]
fn pdot_or_dotdot(s: &[char]) -> bool {
    s.first() == Some(&'.')
        && (s.get(1).is_none()
            || s.get(1) == Some(&'/')
            || (s.get(1) == Some(&'.') && (s.get(2).is_none() || s.get(2) == Some(&'/'))))
}

/// smatch.c:314 SDOT_OR_DOTDOT: `s` is exactly `.` or `..`.
#[inline]
fn sdot_or_dotdot(s: &[char]) -> bool {
    s.first() == Some(&'.')
        && (s.get(1).is_none() || (s.get(1) == Some(&'.') && s.get(2).is_none()))
}

/// POSIX collating symbols table, generated from lib/glob/collsyms.h.
static POSIX_COLLSYMS: &[(&[u8], u8)] = &[
    (b"NUL", b'\x00'),
    (b"SOH", b'\x01'),
    (b"STX", b'\x02'),
    (b"ETX", b'\x03'),
    (b"EOT", b'\x04'),
    (b"ENQ", b'\x05'),
    (b"ACK", b'\x06'),
    (b"alert", b'\x07'),
    (b"BS", b'\x08'),
    (b"backspace", b'\x08'),
    (b"HT", b'\x09'),
    (b"tab", b'\x09'),
    (b"LF", b'\x0A'),
    (b"newline", b'\x0A'),
    (b"VT", b'\x0B'),
    (b"vertical-tab", b'\x0B'),
    (b"FF", b'\x0C'),
    (b"form-feed", b'\x0C'),
    (b"CR", b'\x0D'),
    (b"carriage-return", b'\x0D'),
    (b"SO", b'\x0E'),
    (b"SI", b'\x0F'),
    (b"DLE", b'\x10'),
    (b"DC1", b'\x11'),
    (b"DC2", b'\x12'),
    (b"DC3", b'\x13'),
    (b"DC4", b'\x14'),
    (b"NAK", b'\x15'),
    (b"SYN", b'\x16'),
    (b"ETB", b'\x17'),
    (b"CAN", b'\x18'),
    (b"EM", b'\x19'),
    (b"SUB", b'\x1A'),
    (b"ESC", b'\x1B'),
    (b"IS4", b'\x1C'),
    (b"FS", b'\x1C'),
    (b"IS3", b'\x1D'),
    (b"GS", b'\x1D'),
    (b"IS2", b'\x1E'),
    (b"RS", b'\x1E'),
    (b"IS1", b'\x1F'),
    (b"US", b'\x1F'),
    (b"space", b' '),
    (b"exclamation-mark", b'!'),
    (b"quotation-mark", b'"'),
    (b"number-sign", b'#'),
    (b"dollar-sign", b'$'),
    (b"percent-sign", b'%'),
    (b"ampersand", b'&'),
    (b"apostrophe", b'\''),
    (b"left-parenthesis", b'('),
    (b"right-parenthesis", b')'),
    (b"asterisk", b'*'),
    (b"plus-sign", b'+'),
    (b"comma", b','),
    (b"hyphen", b'-'),
    (b"hyphen-minus", b'-'),
    (b"minus", b'-'),
    (b"dash", b'-'),
    (b"period", b'.'),
    (b"full-stop", b'.'),
    (b"slash", b'/'),
    (b"solidus", b'/'),
    (b"zero", b'0'),
    (b"one", b'1'),
    (b"two", b'2'),
    (b"three", b'3'),
    (b"four", b'4'),
    (b"five", b'5'),
    (b"six", b'6'),
    (b"seven", b'7'),
    (b"eight", b'8'),
    (b"nine", b'9'),
    (b"colon", b':'),
    (b"semicolon", b';'),
    (b"less-than-sign", b'<'),
    (b"equals-sign", b'='),
    (b"greater-than-sign", b'>'),
    (b"question-mark", b'?'),
    (b"commercial-at", b'@'),
    (b"left-square-bracket", b'['),
    (b"backslash", b'\\'),
    (b"reverse-solidus", b'\\'),
    (b"right-square-bracket", b']'),
    (b"circumflex", b'^'),
    (b"circumflex-accent", b'^'),
    (b"underscore", b'_'),
    (b"grave-accent", b'`'),
    (b"left-brace", b'{'),
    (b"left-curly-bracket", b'{'),
    (b"vertical-line", b'|'),
    (b"right-brace", b'}'),
    (b"right-curly-bracket", b'}'),
    (b"tilde", b'~'),
    (b"DEL", b'\x7F'),
];

/// smatch.c:167-182 collsym(): resolve a `[.sym.]` collating symbol name to a
/// byte; a single-character name is its own symbol. None = INVALID.
fn collsym(name: &[char]) -> Option<char> {
    for &(n, code) in POSIX_COLLSYMS {
        if n.len() == name.len() && n.iter().zip(name.iter()).all(|(&a, &b)| a as char == b) {
            return Some(code as char);
        }
    }
    if name.len() == 1 {
        Some(name[0])
    } else {
        None
    }
}

/// smatch.c:283-297 is_cclass(): test C against a `[:class:]` name.
/// Returns -1 for an invalid class name (sm_loop.c:474-486 then skips the
/// class and keeps scanning), 1/0 for a real test. The single-byte compile
/// tests ASCII only; the wide compile (FNM_WIDE) uses the `isw*` domains.
fn is_cclass(c: char, name: &[char], flags: u32) -> i32 {
    // sm_loop.c:474-486: an unknown class name yields -1 and the caller
    // skips the class and keeps scanning, which is not the same as a
    // class that simply fails to match.
    const KNOWN: &[&str] = &[
        "ascii", "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct",
        "space", "upper", "word", "xdigit",
    ];
    let name: String = name.iter().collect();
    if !KNOWN.contains(&name.as_str()) {
        return -1;
    }
    let matched = if flags & FNM_WIDE != 0 {
        // Wide compile: isw* predicates, aligned to the glibc class
        // semantics rubash already models for the pattern matcher.
        crate::locale::utf8_posix_class_matches(&name, c)
    } else {
        match name.as_str() {
            "ascii" => (c as u32) <= 0x7F,
            "alnum" => c.is_ascii_alphanumeric(),
            "alpha" => c.is_ascii_alphabetic(),
            "blank" => c == ' ' || c == '\t',
            "cntrl" => c.is_ascii_control(),
            "digit" => c.is_ascii_digit(),
            "graph" => (0x21..=0x7Eu32).contains(&(c as u32)),
            "lower" => c.is_ascii_lowercase(),
            "print" => (0x20..=0x7Eu32).contains(&(c as u32)),
            "punct" => c.is_ascii_punctuation(),
            "space" => c.is_ascii_whitespace(),
            "upper" => c.is_ascii_uppercase(),
            "word" => c.is_ascii_alphanumeric() || c == '_',
            "xdigit" => c.is_ascii_hexdigit(),
            _ => unreachable!(),
        }
    };
    matched as i32
}

/// glob.c:431-447 udequote_pathname(): remove backslash quoting in place
/// (the C version is unconditional — no FNM_NOESCAPE check).
fn dequote_pathname(s: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == '\\' {
            i += 1;
        }
        if i >= s.len() {
            break;
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// sm_loop.c:382-394 PARSE_SUBBRACKET: `pat` starts AT the type char
/// (`:`/`=`/`.`). Returns the index of the matching `type]` terminator's
/// type char, or None. A `.` type does not stop at a bare `]` (collating
/// symbols may contain it); `/` ends the scan under FNM_PATHNAME.
fn parse_subbracket(pat: &[char], flags: u32) -> Option<usize> {
    let ty = *pat.first()?;
    let mut i = 1;
    while i < pat.len() {
        let c = pat[i];
        if c == '\0' || (c == '/' && flags & FNM_PATHNAME != 0) || (ty != '.' && c == ']') {
            return None;
        }
        if c == ty && pat.get(i + 1) == Some(&']') {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// sm_loop.c:675-757 PATSCAN: scan `pat` for DELIM (or the matching `)` when
/// DELIM is 0) respecting `(...)` and `[...]` nesting and backslash escapes.
/// Returns the index just past the delimiter, or None on empty/ill-formed.
fn patscan(pat: &[char], delim: char, flags: u32) -> Option<usize> {
    if pat.is_empty() {
        return None;
    }
    let (mut pnest, mut bnest, mut skip) = (0i32, 0i32, false);
    let mut bfirst: Option<usize> = None;
    let mut s = 0usize;
    while s < pat.len() {
        let c = pat[s];
        if c == '\0' {
            return None;
        }
        if skip {
            skip = false;
            s += 1;
            continue;
        }
        match c {
            '\\' => {
                if flags & FNM_NOESCAPE == 0 {
                    skip = true;
                }
            }
            '[' => {
                if bnest == 0 {
                    let mut bf = s + 1;
                    if matches!(pat.get(bf), Some('!') | Some('^')) {
                        bf += 1;
                    }
                    bfirst = Some(bf);
                    bnest += 1;
                } else if matches!(pat.get(s + 1), Some(':') | Some('.') | Some('=')) {
                    // POSIX special bracket expression: skip past `type]`.
                    if let Some(t) = parse_subbracket(&pat[s + 1..], flags) {
                        s = s + 1 + t + 1; // t + 2 - 1 cancels the s += 1 below
                    }
                }
            }
            ']' => {
                if bnest != 0 && Some(s) != bfirst {
                    bnest -= 1;
                    bfirst = None;
                }
            }
            '(' => {
                if bnest == 0 {
                    pnest += 1;
                }
            }
            ')' => {
                if bnest == 0 {
                    if pnest <= 0 {
                        return Some(s + 1);
                    }
                    pnest -= 1;
                }
            }
            '|' => {
                if bnest == 0 && pnest == 0 && delim == '|' {
                    return Some(s + 1);
                }
            }
            _ => {}
        }
        s += 1;
    }
    None
}

/// Result of BRACKMATCH (sm_loop.c:398-652): the C function returns a pattern
/// pointer — past `]` on match, 0 on no-match, `savep` (the entry pointer)
/// when the `[` should be treated as a literal.
enum Brack {
    /// Matched; pattern index just past the closing `]` (relative to the
    /// slice that started after the `[`).
    Match(usize),
    /// No match.
    NoMatch,
    /// Unterminated/`/`-interrupted class: the `[` is a literal character
    /// (C returns savep, which equals NOMATCH unless the tested character is
    /// itself `[`).
    LiteralOpen,
}

/// sm_loop.c:398-652 BRACKMATCH: `pat` starts just after the `[`. `test` is
/// the single name character being matched.
fn brackmatch(pat: &[char], test: char, flags: u32) -> Brack {
    let orig_test = test;
    let test = fold(test, flags);
    // Every "literal [`" exit in C checks `test == '['` (folded); `[` is
    // unaffected by folding.
    let lit = |_: usize| -> Brack {
        if test == '[' {
            Brack::LiteralOpen
        } else {
            Brack::NoMatch
        }
    };

    let mut p = 0usize;
    let not = matches!(pat.first(), Some('!') | Some('^'));
    if not {
        p += 1;
    }

    let mut c: char = pat.get(p).copied().unwrap_or('\0');
    p += 1;
    loop {
        let mut cstart = c;
        let mut cend = c;
        let mut forcecoll = false;
        let mut isrange = false;

        // [=x=] POSIX.2 equivalence class (sm_loop.c:431-457).
        if c == '[' && pat.get(p) == Some(&'=') {
            if let Some(close) = parse_subbracket(&pat[p..], flags) {
                p += 1;
                // COLLSYM(p, close - p): name ends at the terminator.
                let pc = collsym(&pat[p..p + close - 1])
                    .map(|v| fold(v, flags))
                    .unwrap_or('\u{FF}');
                p += close + 1; // C: p = close + 2 relative to the type char
                if pc == test {
                    p += 1;
                    return match_class_tail(pat, p, not, flags, lit);
                }
                c = pat.get(p).copied().unwrap_or('\0');
                p += 1;
                if c == '\0' || (c == '/' && flags & FNM_PATHNAME != 0) {
                    return lit(0);
                } else if c == ']' {
                    break;
                }
                c = fold(c, flags);
                continue;
            }
        }

        // [:class:] POSIX.2 character class (sm_loop.c:460-513).
        if c == '[' && pat.get(p) == Some(&':') {
            if let Some(close) = parse_subbracket(&pat[p..], flags) {
                p += 1;
                let ccname = dequote_pathname(&pat[p..p + close - 1]);
                let pc = is_cclass(orig_test, &ccname, flags);
                p += close + 1;
                if pc != 0 && pc != -1 {
                    p += 1;
                    return match_class_tail(pat, p, not, flags, lit);
                }
                c = pat.get(p).copied().unwrap_or('\0');
                p += 1;
                if c == '\0' || (c == '/' && flags & FNM_PATHNAME != 0) {
                    return lit(0);
                } else if c == ']' {
                    break;
                }
                c = fold(c, flags);
                continue;
            }
        }

        // [.x.] POSIX.2 collating symbol (sm_loop.c:519-525).
        if c == '[' && pat.get(p) == Some(&'.') {
            if let Some(close) = parse_subbracket(&pat[p..], flags) {
                p += 1;
                cstart = collsym(&pat[p..p + close - 1]).unwrap_or('\u{FF}');
                cend = cstart;
                p += close + 1;
                forcecoll = true;
            }
        }

        // Backslash-quoted class char (sm_loop.c:527-534).
        if flags & FNM_NOESCAPE == 0 && c == '\\' {
            match pat.get(p) {
                None | Some('\0') => return lit(0),
                Some(&'/') if flags & FNM_PATHNAME != 0 => return lit(0),
                Some(&q) => {
                    cstart = q;
                    cend = q;
                    p += 1;
                }
            }
        }

        cstart = fold(cstart, flags);
        cend = fold(cend, flags);

        if c == '\0' || (c == '/' && flags & FNM_PATHNAME != 0) {
            return lit(0);
        }

        c = fold(pat.get(p).copied().unwrap_or('\0'), flags);
        p += 1;
        if c == '\0' || (c == '/' && flags & FNM_PATHNAME != 0) {
            return lit(0);
        }

        // Range `cstart-cend`, unless `-` is the last class char
        // (sm_loop.c:568-601).
        if c == '-' && pat.get(p) != Some(&']') {
            cend = pat.get(p).copied().unwrap_or('\0');
            p += 1;
            if flags & FNM_NOESCAPE == 0 && cend == '\\' {
                cend = pat.get(p).copied().unwrap_or('\0');
                p += 1;
            }
            if cend == '\0' || (cend == '/' && flags & FNM_PATHNAME != 0) {
                return lit(0);
            }
            if cend == '[' && pat.get(p) == Some(&'.') {
                if let Some(close) = parse_subbracket(&pat[p..], flags) {
                    p += 1;
                    cend = collsym(&pat[p..p + close - 1]).unwrap_or('\u{FF}');
                    p += close + 1;
                    forcecoll = true;
                }
            }
            cend = fold(cend, flags);
            c = pat.get(p).copied().unwrap_or('\0');
            p += 1;
            // Invalid range (end < start): skip it, keep scanning
            // (sm_loop.c:593-599). RANGECMP is a byte compare in the C
            // locale regardless of forcecoll.
            if (cstart as i32) - (cend as i32) > 0 {
                if c == ']' {
                    break;
                }
                c = fold(c, flags);
                continue;
            }
            isrange = true;
        }

        if !isrange && test == cstart {
            return match_class_tail(pat, p, not, flags, lit);
        }
        if isrange && (test as i32) - (cstart as i32) >= 0 && (test as i32) - (cend as i32) <= 0 {
            return match_class_tail(pat, p, not, flags, lit);
        }
        if c == ']' {
            break;
        }
    }
    // No match: `return (!not ? 0 : p)` — a negated class that matched
    // nothing succeeds and resumes after the `]`.
    if !not {
        Brack::NoMatch
    } else {
        Brack::Match(p)
    }
}

/// sm_loop.c:614-651 `matched:` — a class item matched; skip the rest of the
/// `[...]` and return the pattern index past `]`. `p` enters pointing one
/// char past the resume point (the C code does `c = *--p` on entry).
fn match_class_tail(
    pat: &[char],
    mut p: usize,
    not: bool,
    flags: u32,
    lit: impl Fn(usize) -> Brack,
) -> Brack {
    let mut c = pat.get(p - 1).copied().unwrap_or('\0');
    p -= 1;
    loop {
        if c == '\0' || (c == '/' && flags & FNM_PATHNAME != 0) {
            return lit(0);
        }
        c = pat.get(p).copied().unwrap_or('\0');
        p += 1;
        if c == '[' && matches!(pat.get(p), Some('=') | Some(':') | Some('.')) {
            if let Some(close) = parse_subbracket(&pat[p..], flags) {
                // C: p = close + 2 (past the `]`).
                p += close + 2;
            }
        } else if c == ']' {
            break;
        } else if flags & FNM_NOESCAPE == 0 && c == '\\' {
            match pat.get(p) {
                None | Some('\0') => return lit(0),
                Some(&'/') if flags & FNM_PATHNAME != 0 => return lit(0),
                _ => p += 1,
            }
        }
    }
    if not {
        Brack::NoMatch
    } else {
        Brack::Match(p)
    }
}

/// sm_loop.c:761-793 STRCOMPARE: literal compare of the rest of the pattern
/// (operator char included) against the rest of the string — used when an
/// extglob group is ill-formed.
fn strcompare(p: &[char], s: &[char]) -> bool {
    p == s
}

/// sm_loop.c:799-922 EXTMATCH. `op_pat` is the pattern starting at the
/// operator char (`?`, `*`, `+`, `@`, `!`); `op_pat[1]` is normally `(`.
fn extmatch(op_pat: &[char], s: &[char], flags: u32) -> bool {
    let xc = op_pat[0];
    // prest = PATSCAN(p + (*p == '('), pe, 0, flags): scan from just after
    // the `(` for the group's close; the result points just past `)`.
    let group_open = 1 + usize::from(op_pat.get(1) == Some(&'('));
    let Some(prest) = patscan(&op_pat[group_open..], '\0', flags).map(|i| group_open + i) else {
        return strcompare(op_pat, s);
    };

    let alternatives = |psub: usize| -> (usize, &[char]) {
        // pnext = PATSCAN(psub, pe, '|', flags): lands just past `|` or the
        // group's closing `)` (== prest).
        let pnext = patscan(&op_pat[psub..], '|', flags)
            .map(|i| psub + i)
            .unwrap_or(prest);
        (pnext, &op_pat[psub..pnext - 1])
    };
    let xflags = |srest: usize| {
        if srest > 0 {
            flags & !(FNM_PERIOD | FNM_DOTDOT)
        } else {
            flags
        }
    };

    match xc {
        '+' | '*' => {
            // sm_loop.c:830: `*()` tries zero occurrences first.
            if xc == '*' && gmatch(&op_pat[prest..], s, flags) {
                return true;
            }
            let mut psub = 2;
            loop {
                let (pnext, alt) = alternatives(psub);
                for srest in 0..=s.len() {
                    if gmatch(alt, &s[..srest], flags)
                        && (gmatch(&op_pat[prest..], &s[srest..], xflags(srest))
                            || (srest != 0 && gmatch(op_pat, &s[srest..], xflags(srest))))
                    {
                        return true;
                    }
                }
                if pnext == prest {
                    break;
                }
                psub = pnext;
            }
            false
        }
        '?' | '@' => {
            // sm_loop.c:867: `?()` tries zero occurrences first.
            if xc == '?' && gmatch(&op_pat[prest..], s, flags) {
                return true;
            }
            // sm_loop.c:876: a group ending the pattern must match the whole
            // remainder, so srest starts at the end.
            let start = if prest >= op_pat.len() { s.len() } else { 0 };
            let mut psub = 2;
            loop {
                let (pnext, alt) = alternatives(psub);
                for srest in start..=s.len() {
                    if gmatch(alt, &s[..srest], flags)
                        && gmatch(&op_pat[prest..], &s[srest..], xflags(srest))
                    {
                        return true;
                    }
                }
                if pnext == prest {
                    break;
                }
                psub = pnext;
            }
            false
        }
        '!' => {
            for srest in 0..=s.len() {
                let mut psub = 2;
                let m1;
                loop {
                    let (pnext, alt) = alternatives(psub);
                    if gmatch(alt, &s[..srest], flags) {
                        m1 = true;
                        break;
                    }
                    if pnext == prest {
                        m1 = false;
                        break;
                    }
                    psub = pnext;
                }
                if !m1 {
                    // sm_loop.c:907-911: the negation must not match a string
                    // whose first char needs explicit matching.
                    if flags & FNM_PERIOD != 0 && s.first() == Some(&'.') {
                        return false;
                    }
                    if flags & FNM_DOTDOT != 0 && sdot_or_dotdot(s) {
                        return false;
                    }
                    if gmatch(&op_pat[prest..], &s[srest..], xflags(srest)) {
                        return true;
                    }
                }
            }
            false
        }
        _ => false,
    }
}

/// sm_loop.c:GMATCH (smatch.c single-byte compile). `pat`/`s` are the
/// remaining pattern and string; position 0 of `s` is the C call's `string`.
fn gmatch(pat: &[char], s: &[char], flags: u32) -> bool {
    let mut p = 0usize;
    let mut n = 0usize;
    while p < pat.len() {
        let c = pat[p];
        p += 1;
        let c = fold(c, flags);
        let sc = s.get(n).copied().unwrap_or('\0');

        // sm_loop.c:85-93 EXTMATCH dispatch.
        if flags & FNM_EXTMATCH != 0
            && pat.get(p) == Some(&'(')
            && matches!(c, '+' | '*' | '?' | '@' | '!')
        {
            let lflags = if n == 0 {
                flags
            } else {
                flags & !(FNM_PERIOD | FNM_DOTDOT)
            };
            return extmatch(&pat[p - 1..], &s[n..], lflags);
        }

        match c {
            '?' => {
                if sc == '\0' {
                    return false;
                }
                if flags & FNM_PATHNAME != 0 && sc == '/' {
                    return false;
                }
                if flags & FNM_PERIOD != 0
                    && sc == '.'
                    && (n == 0 || (flags & FNM_PATHNAME != 0 && s[n - 1] == '/'))
                {
                    return false;
                }
                if flags & FNM_DOTDOT != 0
                    && ((n == 0 && sdot_or_dotdot(&s[n..]))
                        || (flags & FNM_PATHNAME != 0
                            && n > 0
                            && s[n - 1] == '/'
                            && pdot_or_dotdot(&s[n..])))
                {
                    return false;
                }
                n += 1;
            }
            '\\' => {
                // sm_loop.c:121-138: trailing `\` matches a trailing `\`.
                if p == pat.len() && sc == '\\' && n + 1 == s.len() {
                    n += 1;
                    continue;
                }
                if p == pat.len() {
                    return false;
                }
                if flags & FNM_NOESCAPE == 0 {
                    let e = fold(pat[p], flags);
                    p += 1;
                    if fold(sc, flags) != e {
                        return false;
                    }
                } else if fold(sc, flags) != c {
                    return false;
                }
                n += 1;
            }
            '*' => {
                if flags & FNM_PERIOD != 0
                    && sc == '.'
                    && (n == 0 || (flags & FNM_PATHNAME != 0 && s[n - 1] == '/'))
                {
                    return false;
                }
                if flags & FNM_DOTDOT != 0
                    && ((n == 0 && sdot_or_dotdot(&s[n..]))
                        || (flags & FNM_PATHNAME != 0
                            && n > 0
                            && s[n - 1] == '/'
                            && pdot_or_dotdot(&s[n..])))
                {
                    return false;
                }
                if p == pat.len() {
                    return true;
                }

                // sm_loop.c:168-228: collapse the following `?`/`*` run.
                let mut c = pat[p];
                p += 1;
                while c == '?' || c == '*' {
                    let sc = s.get(n).copied().unwrap_or('\0');
                    if flags & FNM_PATHNAME != 0 && sc == '/' {
                        return false;
                    }
                    if flags & FNM_EXTMATCH != 0 && c == '?' && pat.get(p) == Some(&'(') {
                        // `?(` inside a run: try the group; on failure skip
                        // it and continue the run (sm_loop.c:176-191).
                        if extmatch(&pat[p - 1..], &s[n..], flags) {
                            return true;
                        }
                        p = patscan(&pat[p + 1..], '\0', flags)
                            .map(|i| p + 1 + i)
                            .unwrap_or(pat.len());
                    } else if c == '?' {
                        if sc == '\0' {
                            return false;
                        }
                        n += 1;
                    }
                    if flags & FNM_EXTMATCH != 0 && c == '*' && pat.get(p) == Some(&'(') {
                        // `*(` inside a run: try the group at every position
                        // (sm_loop.c:206-224); on failure skip it.
                        for newn in n..s.len() {
                            if extmatch(&pat[p - 1..], &s[newn..], flags) {
                                return true;
                            }
                        }
                        p = patscan(&pat[p + 1..], '\0', flags)
                            .map(|i| p + 1 + i)
                            .unwrap_or(pat.len());
                    }
                    if p == pat.len() {
                        break;
                    }
                    c = pat[p];
                    p += 1;
                }

                // sm_loop.c:249-250: the run reached the end of the pattern.
                if p == pat.len() && (c == '?' || c == '*') {
                    return true;
                }

                // sm_loop.c:255-261: string exhausted with `!`/`?` group
                // next — the negation inverts the result.
                if n == s.len()
                    && flags & FNM_EXTMATCH != 0
                    && (c == '!' || c == '?')
                    && pat.get(p) == Some(&'(')
                {
                    let m = extmatch(&pat[p - 1..], &s[n..], flags);
                    return if c == '!' { !m } else { m };
                }

                // sm_loop.c:268-275: `*/...` — consume the name to its next
                // slash, then match the rest past it.
                if c == '/' && flags & FNM_PATHNAME != 0 {
                    while n < s.len() && s[n] != '/' {
                        n += 1;
                    }
                    if n < s.len() && s[n] == '/' && gmatch(&pat[p..], &s[n + 1..], flags) {
                        return true;
                    }
                    return false;
                }

                // sm_loop.c:277-325: general case — try the rest of the
                // pattern at each position up to the next `/` (or end).
                // The `ends` backtracking cursor is elided: it does not
                // change the accepted set, only the retry order.
                let c1 = if flags & FNM_NOESCAPE == 0 && c == '\\' {
                    fold(pat.get(p).copied().unwrap_or('\0'), flags)
                } else {
                    c
                };
                let endp = if flags & FNM_PATHNAME != 0 {
                    s[n..]
                        .iter()
                        .position(|&b| b == '/')
                        .map(|i| n + i)
                        .unwrap_or(s.len())
                } else {
                    s.len()
                };
                for k in n..endp {
                    if flags & FNM_EXTMATCH == 0 && c != '[' && fold(s[k], flags) != c1 {
                        continue;
                    }
                    if flags & FNM_EXTMATCH != 0
                        && pat.get(p) != Some(&'(')
                        && !matches!(c, '?' | '*' | '+' | '@' | '!')
                        && c != '['
                        && fold(s[k], flags) != c1
                    {
                        continue;
                    }
                    if gmatch(&pat[p - 1..], &s[k..], flags & !(FNM_PERIOD | FNM_DOTDOT)) {
                        return true;
                    }
                }
                return false;
            }
            '[' => {
                if sc == '\0' || n == s.len() {
                    return false;
                }
                if flags & FNM_PERIOD != 0
                    && sc == '.'
                    && (n == 0 || (flags & FNM_PATHNAME != 0 && s[n - 1] == '/'))
                {
                    return false;
                }
                if sc == '/' && flags & FNM_PATHNAME != 0 {
                    return false;
                }
                if flags & FNM_DOTDOT != 0
                    && ((n == 0 && sdot_or_dotdot(&s[n..]))
                        || (flags & FNM_PATHNAME != 0
                            && n > 0
                            && s[n - 1] == '/'
                            && pdot_or_dotdot(&s[n..])))
                {
                    return false;
                }
                match brackmatch(&pat[p..], sc, flags) {
                    Brack::Match(off) => {
                        p += off;
                        n += 1;
                    }
                    Brack::LiteralOpen => {
                        // `[` consumed as a literal (sc == '['); p stays just
                        // after the `[` (C returns savep).
                        n += 1;
                    }
                    Brack::NoMatch => return false,
                }
            }
            _ => {
                if c != fold(sc, flags) {
                    return false;
                }
                n += 1;
            }
        }
    }

    if n == s.len() {
        return true;
    }
    // sm_loop.c:369-371: FNM_LEADING_DIR accepts `foo*` == `foo/bar`.
    if flags & FNM_LEADING_DIR != 0 && s[n] == '/' {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::posix_cclass_only;

    /// smatch.c:505-545: no class expression, or only POSIX names, punts to
    /// the single-byte compile; an unknown `[:name:]` keeps the wide compile.
    #[test]
    fn posix_cclass_only_matches_the_c_scan() {
        assert!(posix_cclass_only(b"u/?.txt"));
        assert!(posix_cclass_only(b"u/[[:alpha:]].txt"));
        assert!(posix_cclass_only(b"[a-z]"));
        assert!(posix_cclass_only(b"u/[[:unterminated.txt"));
        // A bracket that is not a `[:class:]` is skipped by the scan.
        assert!(posix_cclass_only(b"[[.a.]]"));
        assert!(!posix_cclass_only(b"u/[[:bogus:]].txt"));
        assert!(!posix_cclass_only(b"[[:alpha:]][[:bogus:]]"));
    }
}
