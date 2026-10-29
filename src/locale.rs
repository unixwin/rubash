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

// Override for the locale the ANSI-C `\u`/`\U` decoder sees (rubash#353).
//
// GNU decodes `$'...'` when the READER pulls the enclosing command list
// (parse.y:5560 ansiexpand inside read_token_word), so the locale that
// gates the decoder (lib/sh/strtrans.c:161-181 -> unicode.c:239 u32cconv,
// non-UTF-8 fallback u32tocesc `\u%04X`/`\U%08X`) is the one in effect at
// that READ time — for a sequential script, the locale after all earlier
// top-level lists executed. Rubash lexes the whole file upfront, so the
// executor re-derives u-bearing words under their line's read-time locale
// (recorded per source line at each top-level command start). During that
// re-derivation this thread-local override carries the recorded locale
// into the otherwise env-derived `locale_name()` the decoder consults;
// outside a re-derivation there is no override and the decoder sees the
// live environment exactly as before.
thread_local! {
    static ANSI_LEX_LOCALE_OVERRIDE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// RAII guard for [`ANSI_LEX_LOCALE_OVERRIDE`]: on drop it restores the
/// value the override held on entry (an outer pin, or none) instead of
/// clearing the slot, so nested pins compose — an inner re-derivation
/// must not erase the outer line's read-time locale.
struct AnsiLexLocaleGuard {
    previous: Option<String>,
}

impl AnsiLexLocaleGuard {
    fn new(locale: &str) -> Self {
        let previous =
            ANSI_LEX_LOCALE_OVERRIDE.with(|cell| cell.borrow_mut().replace(locale.to_string()));
        AnsiLexLocaleGuard { previous }
    }
}

impl Drop for AnsiLexLocaleGuard {
    fn drop(&mut self) {
        let previous = std::mem::replace(&mut self.previous, None);
        ANSI_LEX_LOCALE_OVERRIDE.with(|cell| {
            *cell.borrow_mut() = previous;
        });
    }
}

/// Run `f` with the ANSI-C decoder locale pinned to `locale` (the
/// read-time locale recorded for the word's source line).
pub fn with_ansi_lex_locale<T>(locale: &str, f: impl FnOnce() -> T) -> T {
    let _guard = AnsiLexLocaleGuard::new(locale);
    f()
}

/// The locale name the ANSI-C decoder must use: the re-derivation override
/// when one is active, the live environment otherwise.
pub fn ansi_lex_locale_name() -> String {
    ANSI_LEX_LOCALE_OVERRIDE
        .with(|cell| cell.borrow().clone())
        .unwrap_or_else(locale_name)
}

/// The active encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Ascii,
    Utf8,
}

// The locale name of the shell whose builtin is currently executing,
// resolved from its VARIABLE table rather than the process environment.
// GNU's sv_lc* watchers (variables.c:4283-4350) make a plain (unexported)
// `LC_ALL=C` assignment change the shell's own locale semantics at once;
// rubash only syncs the process env on `export`, so executor-side builtin
// dispatch installs this override from env_vars for the duration of the
// builtin (rubash#1488: `LC_ALL=C; echo -e '\u00e9'` must see the C locale
// inside echo even though LC_ALL was never exported).
thread_local! {
    static SHELL_LOCALE_OVERRIDE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// RAII guard installing [`SHELL_LOCALE_OVERRIDE`] from a variable table.
///
/// On drop the guard restores the override it found on entry (an outer
/// guard's value, or none) instead of clearing the slot, so nested
/// builtin dispatch keeps the outer builtin's variable-table locale
/// when the inner builtin returns.
pub struct ShellLocaleGuard {
    previous: Option<String>,
}

impl ShellLocaleGuard {
    pub fn new(env_vars: &std::collections::HashMap<String, String>) -> Self {
        let previous = SHELL_LOCALE_OVERRIDE
            .with(|cell| cell.borrow_mut().replace(locale_name_from_vars(env_vars)));
        ShellLocaleGuard { previous }
    }
}

impl Drop for ShellLocaleGuard {
    fn drop(&mut self) {
        let previous = std::mem::replace(&mut self.previous, None);
        SHELL_LOCALE_OVERRIDE.with(|cell| {
            *cell.borrow_mut() = previous;
        });
    }
}

/// Locale name from a variable table: LC_ALL > LC_CTYPE > LC_MESSAGES >
/// LANG, empty values skipped (same precedence as [`locale_name`]).
pub fn locale_name_from_vars(env_vars: &std::collections::HashMap<String, String>) -> String {
    for name in ["LC_ALL", "LC_CTYPE", "LC_MESSAGES", "LANG"] {
        if let Some(value) = env_vars.get(name) {
            if !value.is_empty() {
                return value.clone();
            }
        }
    }
    String::new()
}

/// The locale name a running builtin should observe: the executing
/// shell's variable-table locale when a guard is installed, the process
/// environment otherwise.
pub fn active_locale_name() -> String {
    SHELL_LOCALE_OVERRIDE
        .with(|cell| cell.borrow().clone())
        .unwrap_or_else(locale_name)
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
/// the locale named by the environment is actually active. The decision
/// itself lives in [`locale_name_selects_utf8`]; this is its form for the
/// live process environment.
pub fn is_utf8() -> bool {
    locale_name_selects_utf8(&locale_name())
}

/// The single locale decision shared by every character-semantics gate:
/// the `${#var}` length face ([`is_utf8`]/[`is_multi_byte`]) and the
/// `\u`/`\U` decoder faces (the ANSI-C lexer in `lexer/ansi.rs`, `echo -e`
/// in `builtins/echo.rs`, printf's format walker) all ask this one
/// question of the locale name in effect on their own read path.
///
/// GNU bash resolves the locale at startup with setlocale(LC_ALL, "")
/// (locale.c set_default_locale, called from shell.c), so with NO locale
/// variable set — or every one of them empty — the C locale is in effect
/// and character semantics are byte semantics: an empty name NEVER
/// selects UTF-8 (rubash#1622 fixed that default on the length face;
/// rubash#1837 removed the opposite default the decoder gates still
/// kept, under which `$'\uE301'` decoded to a 3-byte character while
/// `${#x}` counted those bytes one by one in the same environment).
/// `C`/`POSIX` and any named single-byte locale (ru_RU.CP1251,
/// en_US.ISO-8859-1, ...) likewise select byte semantics. A UTF-8-named
/// locale selects UTF-8 only when the C library can activate it;
/// otherwise GNU bash - and rubash - fall back to the C locale, which is
/// what makes `${#var}` count bytes on a machine that does not have
/// `en_US.UTF-8` compiled in (intl1.sub: 31/30 for a 15-character
/// Cyrillic word, not 16/15).
pub fn locale_name_selects_utf8(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let lower = name.to_lowercase();
    if lower == "c" || lower == "posix" {
        return false;
    }
    if !is_utf8_locale_name(name) {
        return false;
    }
    utf8_locale_active(name)
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

/// Non-ASCII decimal-digit (general category Nd) codepoint ranges.
/// glibc's C.UTF-8 `alpha` class admits Nd characters beyond ASCII
/// (measured against the oracle: Arabic-Indic, Devanagari, Thai and
/// fullwidth digits are all `alpha`), while its `digit`/`xdigit`
/// classes stay ASCII-only, and Rust's `char` predicates cannot
/// express the set (`is_numeric`/`is_alphanumeric` also admit No/Nl
/// characters like '²', which glibc keeps out of `alnum`). Generated
/// from the Unicode data tables; this is the single source both the
/// pattern matcher (predicate form, [`utf8_posix_class_matches`]) and
/// the `=~` class translator (regex-text form,
/// [`non_ascii_nd_class_items`]) draw from (rubash#1621).
const NON_ASCII_ND_RANGES: &[(u32, u32)] = &[
    (0x0660, 0x0669),
    (0x06F0, 0x06F9),
    (0x07C0, 0x07C9),
    (0x0966, 0x096F),
    (0x09E6, 0x09EF),
    (0x0A66, 0x0A6F),
    (0x0AE6, 0x0AEF),
    (0x0B66, 0x0B6F),
    (0x0BE6, 0x0BEF),
    (0x0C66, 0x0C6F),
    (0x0CE6, 0x0CEF),
    (0x0D66, 0x0D6F),
    (0x0DE6, 0x0DEF),
    (0x0E50, 0x0E59),
    (0x0ED0, 0x0ED9),
    (0x0F20, 0x0F29),
    (0x1040, 0x1049),
    (0x1090, 0x1099),
    (0x17E0, 0x17E9),
    (0x1810, 0x1819),
    (0x1946, 0x194F),
    (0x19D0, 0x19D9),
    (0x1A80, 0x1A89),
    (0x1A90, 0x1A99),
    (0x1B50, 0x1B59),
    (0x1BB0, 0x1BB9),
    (0x1C40, 0x1C49),
    (0x1C50, 0x1C59),
    (0xA620, 0xA629),
    (0xA8D0, 0xA8D9),
    (0xA900, 0xA909),
    (0xA9D0, 0xA9D9),
    (0xA9F0, 0xA9F9),
    (0xAA50, 0xAA59),
    (0xABF0, 0xABF9),
    (0xFF10, 0xFF19),
    (0x104A0, 0x104A9),
    (0x10D30, 0x10D39),
    (0x11066, 0x1106F),
    (0x110F0, 0x110F9),
    (0x11136, 0x1113F),
    (0x111D0, 0x111D9),
    (0x112F0, 0x112F9),
    (0x11450, 0x11459),
    (0x114D0, 0x114D9),
    (0x11650, 0x11659),
    (0x116C0, 0x116C9),
    (0x11730, 0x11739),
    (0x118E0, 0x118E9),
    (0x11950, 0x11959),
    (0x11C50, 0x11C59),
    (0x11D50, 0x11D59),
    (0x11DA0, 0x11DA9),
    (0x11F50, 0x11F59),
    (0x16A60, 0x16A69),
    (0x16AC0, 0x16AC9),
    (0x16B50, 0x16B59),
    (0x1D7CE, 0x1D7FF),
    (0x1E140, 0x1E149),
    (0x1E2F0, 0x1E2F9),
    (0x1E4F0, 0x1E4F9),
    (0x1E950, 0x1E959),
    (0x1FBF0, 0x1FBF9),
];

/// Whether `ch` is a decimal digit (category Nd) outside ASCII.
pub(crate) fn is_non_ascii_nd(ch: char) -> bool {
    let cp = ch as u32;
    NON_ASCII_ND_RANGES
        .binary_search_by(|&(lo, hi)| {
            if cp < lo {
                std::cmp::Ordering::Greater
            } else if cp > hi {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// [`NON_ASCII_ND_RANGES`] rendered as regex character-class items,
/// for the `=~` bracket-class translator in executor::conditional.
pub(crate) fn non_ascii_nd_class_items() -> &'static str {
    static ITEMS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ITEMS.get_or_init(|| {
        let mut out = String::new();
        for &(lo, hi) in NON_ASCII_ND_RANGES {
            if lo == hi {
                out.push_str(&format!("\\u{{{lo:x}}}"));
            } else {
                out.push_str(&format!("\\u{{{lo:x}}}-\\u{{{hi:x}}}"));
            }
        }
        out
    })
}

/// glibc C.UTF-8 `space` membership: the Unicode White_Space set MINUS
/// U+00A0 and U+202F, which glibc keeps in `print`/`graph`/`punct`
/// only (measured against the oracle, rubash#1621).
pub(crate) fn is_glibc_space(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{205f}' | '\u{3000}'
    )
}

/// glibc C.UTF-8 `blank` membership: tab, space, and the space-class
/// members in category Zs (U+2028/U+2029 are `space` but not `blank`).
pub(crate) fn is_glibc_blank(ch: char) -> bool {
    matches!(
        ch,
        '\t' | ' ' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{205f}' | '\u{3000}'
    )
}

/// glibc C.UTF-8 `cntrl` membership: category Cc plus U+2028/U+2029,
/// which glibc counts as control (and therefore not `print`); other
/// format characters (Cf, e.g. U+00AD, U+200B) are `print` there.
pub(crate) fn is_glibc_cntrl(ch: char) -> bool {
    ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}')
}

/// POSIX character-class membership under a UTF-8 locale, following
/// glibc's C.UTF-8 tables — GNU resolves classes through
/// wctype/iswctype (lib/glob/smatch.c for the pattern matcher, the
/// regcomp tables for `=~`). Measured divergences from the naive
/// Unicode reading, all pinned by oracle probes (rubash#1621):
/// `digit`/`xdigit` are ASCII-only; `alpha` adds non-ASCII Nd;
/// `alnum` is `alpha` plus every Nd; `word` is `alnum` plus '_';
/// `print` is the complement of the cntrl table above, so unassigned
/// codepoints count as print — the one place the tables cannot be
/// reproduced exactly from Rust predicates.
pub(crate) fn utf8_posix_class_matches(class: &str, ch: char) -> bool {
    fn is_alpha(ch: char) -> bool {
        ch.is_alphabetic() || is_non_ascii_nd(ch)
    }
    fn is_alnum(ch: char) -> bool {
        is_alpha(ch) || ch.is_ascii_digit()
    }
    match class {
        "alpha" => is_alpha(ch),
        "alnum" => is_alnum(ch),
        "upper" => ch.is_uppercase(),
        "lower" => ch.is_lowercase(),
        "digit" => ch.is_ascii_digit(),
        "xdigit" => ch.is_ascii_hexdigit(),
        "ascii" => ch.is_ascii(),
        "cntrl" => is_glibc_cntrl(ch),
        "print" => !is_glibc_cntrl(ch),
        "graph" => !is_glibc_cntrl(ch) && !is_glibc_blank(ch),
        "punct" => !is_glibc_cntrl(ch) && !is_glibc_blank(ch) && !is_alnum(ch),
        "space" => is_glibc_space(ch),
        "blank" => is_glibc_blank(ch),
        "word" => is_alnum(ch) || ch == '_',
        _ => false,
    }
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

/// GNU lib/sh/casemod.c maps each character through the locale's
/// towupper/towlower — the SIMPLE (1:1) Unicode case mapping, never the
/// full mapping: `ß` has no simple uppercase and stays `ß`, the `ﬁ`
/// ligature stays, and U+0130 `İ` lowercases to plain `i` (its simple
/// mapping; the full mapping is `i` + combining dot above). Rust's
/// char::to_uppercase/to_lowercase produce FULL mappings, so keep the
/// result only when it is exactly one character (rubash#1063). In a
/// single-byte locale only ASCII letters map at all (glibc towupper in
/// the C locale), matching is_utf8()'s byte model.
pub fn simple_case_upper(ch: char) -> char {
    if !is_utf8() {
        return if ch.is_ascii_lowercase() {
            ch.to_ascii_uppercase()
        } else {
            ch
        };
    }
    let mut mapped = ch.to_uppercase();
    match (mapped.next(), mapped.next()) {
        (Some(upper), None) => upper,
        _ => ch,
    }
}

/// Lowercase counterpart of [`simple_case_upper`].
pub fn simple_case_lower(ch: char) -> char {
    if !is_utf8() {
        return if ch.is_ascii_uppercase() {
            ch.to_ascii_lowercase()
        } else {
            ch
        };
    }
    // U+0130: glibc towlower gives plain `i` (the simple mapping); the
    // Rust full mapping is `i` + U+0307 (rubash#1063).
    if ch == '\u{130}' {
        return 'i';
    }
    let mut mapped = ch.to_lowercase();
    match (mapped.next(), mapped.next()) {
        (Some(lower), None) => lower,
        _ => ch,
    }
}

/// Byte offsets of the GNU multibyte character boundaries in `bytes`.
///
/// This is the ADVANCE_CHAR walk of `include/shmbutil.h:114-147`: a valid
/// multibyte sequence is one character, and every byte of an invalid
/// sequence is one character (`mbrlen` returns (size_t)-1 / (size_t)-2, so
/// the macro advances the index by one byte). `${#v}` (MB_STRLEN,
/// subst.c:8308 -> utf8.c:167-184) and `${v:off:len}` (mb_substring,
/// subst.c:9022 -> ADVANCE_CHAR) share this walk.
///
/// The result always begins with 0 and ends with `bytes.len()`, so a
/// character count is `offsets.len() - 1` and the `n`-th character spans
/// `offsets[n]..offsets[n + 1]`.
pub fn multibyte_char_offsets(bytes: &[u8]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(bytes.len() + 1);
    offsets.push(0);
    let mut index = 0usize;
    while index < bytes.len() {
        match std::str::from_utf8(&bytes[index..]) {
            Ok(text) => {
                let mut cursor = index;
                for ch in text.chars() {
                    cursor += ch.len_utf8();
                    offsets.push(cursor);
                }
                index = bytes.len();
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    let text = std::str::from_utf8(&bytes[index..index + valid])
                        .expect("valid_up_to prefix is valid UTF-8");
                    let mut cursor = index;
                    for ch in text.chars() {
                        cursor += ch.len_utf8();
                        offsets.push(cursor);
                    }
                    index += valid;
                }
                // utf8.c:177-178 / shmbutil.h:134-138: an invalid byte is
                // one character and advances the walk by one byte.
                index += 1;
                offsets.push(index);
            }
        }
    }
    offsets
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

/// The locale variable family GNU binds to sv_locale (variables.c:5800
/// special_vars table -> variables.c:6272 sv_locale).
pub fn is_locale_family_name(name: &str) -> bool {
    matches!(
        name,
        "LANG" | "LC_ALL" | "LC_COLLATE" | "LC_CTYPE" | "LC_MESSAGES" | "LC_NUMERIC" | "LC_TIME"
    )
}

/// POSIX category precedence (locale.c:343 get_locale_var): LC_ALL wins,
/// then the category's own variable, then LANG, then "" (the
/// environment-default locale).
fn get_locale_var(var: &str, env_vars: &std::collections::HashMap<String, String>) -> String {
    let nonempty = |name: &str| {
        env_vars
            .get(name)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    };
    nonempty("LC_ALL")
        .or_else(|| nonempty(var))
        .or_else(|| nonempty("LANG"))
        .unwrap_or("")
        .to_string()
}

#[cfg(unix)]
fn category_constant(var: &str) -> Option<libc::c_int> {
    Some(match var {
        "LC_ALL" => libc::LC_ALL,
        "LC_COLLATE" => libc::LC_COLLATE,
        "LC_CTYPE" => libc::LC_CTYPE,
        "LC_MESSAGES" => libc::LC_MESSAGES,
        "LC_NUMERIC" => libc::LC_NUMERIC,
        "LC_TIME" => libc::LC_TIME,
        _ => return None,
    })
}

/// Address of the calling thread's errno cell — the storage behind
/// C's `errno` lvalue, which GNU clears before the setlocale call and
/// reads after it (locale.c:191, in set_locale_var). The accessor's
/// spelling is libc-specific: glibc and musl call it
/// `__errno_location`, Darwin and FreeBSD call it `__error`, and
/// Android's bionic calls it `__errno`. The fallthrough keeps the
/// glibc spelling, which the remaining Unix libcs in the libc crate
/// (DragonFly, emscripten, Fuchsia, ...) also use.
#[cfg(all(
    unix,
    not(any(
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos",
        target_os = "freebsd"
    ))
))]
fn errno_location() -> *mut libc::c_int {
    unsafe { libc::__errno_location() }
}

#[cfg(all(
    unix,
    any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos",
        target_os = "freebsd"
    )
))]
fn errno_location() -> *mut libc::c_int {
    unsafe { libc::__error() }
}

#[cfg(all(unix, target_os = "android"))]
fn errno_location() -> *mut libc::c_int {
    unsafe { libc::__errno() }
}

/// One real setlocale attempt, mirroring GNU locale.c set_locale_var:
/// errno is cleared first (locale.c:191), and on failure the caller
/// reports `strerror(errno)` only when the call left errno set — glibc
/// sets ENOENT on the first failed lookup of a (category, name) pair
/// and leaves errno clear on repeat lookups it has already cached, so
/// the `: No such file or directory` suffix appears exactly on the
/// first failing bind of each name, as in GNU.
#[cfg(unix)]
fn attempt_setlocale(category: libc::c_int, name: &str) -> Result<(), Option<String>> {
    let Ok(c_name) = std::ffi::CString::new(name) else {
        return Err(None);
    };
    unsafe {
        *errno_location() = 0;
        let result = libc::setlocale(category, c_name.as_ptr());
        if !result.is_null() {
            return Ok(());
        }
        let errno = *errno_location();
        if errno == 0 {
            Err(None)
        } else {
            let message = std::ffi::CStr::from_ptr(libc::strerror(errno));
            Err(Some(message.to_string_lossy().into_owned()))
        }
    }
}

#[cfg(not(unix))]
fn attempt_setlocale(_category: std::ffi::c_int, _name: &str) -> Result<(), Option<String>> {
    // The Windows CRT has no compiled locale archive to reject a name
    // (see utf8_locale_active); GNU's warning has no failing call to
    // mirror there. (std::ffi::c_int keeps the signature identical to
    // the unix form; no caller reaches this on non-unix.)
    Ok(())
}

/// locale.c reset_locale_vars: composite re-apply when LC_ALL is
/// empty — setlocale(LC_ALL, lang) then each category from its
/// precedence value. Never warns (the function has no diagnostic).
#[cfg(unix)]
fn reset_locale_vars_mirror(env_vars: &std::collections::HashMap<String, String>) {
    let lang = env_vars.get("LANG").cloned().unwrap_or_default();
    // locale.c reset_locale_vars returns at once when the composite
    // apply fails; the per-category applies never run.
    if attempt_setlocale(libc::LC_ALL, &lang).is_err() {
        return;
    }
    for category in [
        "LC_CTYPE",
        "LC_COLLATE",
        "LC_MESSAGES",
        "LC_NUMERIC",
        "LC_TIME",
    ] {
        if let Some(constant) = category_constant(category) {
            let _ = attempt_setlocale(constant, &get_locale_var(category, env_vars));
        }
    }
}

/// Mirror one locale-family bind the way GNU sv_locale does
/// (variables.c:6272 -> locale.c:185 set_locale_var / locale.c:313
/// set_lang), performing the same real setlocale call GNU performs
/// and returning the warning text (without the caller's diagnostic
/// prefix) when GNU would print one:
///
/// * `LC_ALL` non-empty: one setlocale(LC_ALL, value) attempt; failure
///   warns `setlocale: LC_ALL: cannot change locale (<value>)`.
/// * `LC_ALL` empty/unset: the silent reset path (no warning even
///   when a category value is bad).
/// * A category (LC_COLLATE/LC_CTYPE/LC_MESSAGES/LC_NUMERIC/LC_TIME):
///   attempted only while LC_ALL is empty, with the precedence value
///   from get_locale_var — so `LANG=bad` followed by `LC_CTYPE=` warns
///   about LC_CTYPE with LANG's value, exactly as locale.c:301 does.
/// * `LANG`: never warns (set_lang is silent); still runs the reset
///   so the process locale follows, like GNU.
///
/// `env_vars` must already hold the just-committed values (sv_locale
/// reads the stored binding via get_string_value).
pub fn locale_assignment_warning(
    var: &str,
    env_vars: &std::collections::HashMap<String, String>,
) -> Option<String> {
    if !is_locale_family_name(var) {
        return None;
    }
    // The Windows CRT has no compiled locale archive to reject a name,
    // so GNU's failing-setlocale warning has no counterpart there.
    #[cfg(not(unix))]
    {
        return None;
    }
    #[cfg(unix)]
    {
        let lc_all = env_vars.get("LC_ALL").cloned().unwrap_or_default();
        if var == "LC_ALL" {
            if lc_all.is_empty() {
                reset_locale_vars_mirror(env_vars);
                return None;
            }
            return match attempt_setlocale(libc::LC_ALL, &lc_all) {
                Ok(()) => None,
                Err(suffix) => Some(format!(
                    "setlocale: LC_ALL: cannot change locale ({lc_all}){}",
                    suffix.map(|text| format!(": {text}")).unwrap_or_default()
                )),
            };
        }
        if var == "LANG" {
            if lc_all.is_empty() {
                reset_locale_vars_mirror(env_vars);
            }
            return None;
        }
        // A category variable.
        if !lc_all.is_empty() {
            return None;
        }
        let applied = get_locale_var(var, env_vars);
        let constant = category_constant(var)?;
        match attempt_setlocale(constant, &applied) {
            Ok(()) => None,
            Err(suffix) => Some(format!(
                "setlocale: {var}: cannot change locale ({applied}){}",
                suffix.map(|text| format!(": {text}")).unwrap_or_default()
            )),
        }
    }
}

/// Sync the locale-category variables into the PROCESS environment so the
/// dynamic `locale_name()` readers (is_utf8, decimal_point, ...) observe
/// assignments made inside the script. GNU variables.c installs sv_lang /
/// sv_lc* watchers that call setlocale(3) on every LC_ALL/LC_CTYPE/LANG
/// assignment (variables.c:4283-4350); the process env is rubash's
/// equivalent single source of truth for those readers. Unset names are
/// removed so the next-priority category applies.
pub fn sync_process_locale(env_vars: &std::collections::HashMap<String, String>) {
    // rubash#1531: the full watcher family (GNU variables.c special_vars
    // binds every LC_* category plus LANG to sv_locale), not only the
    // collate/ctype subset the #1512 fix needed.
    // G-01: the locale process-env sync is a process-global write —
    // serialize it. It is deliberately NOT journaled (G-03): locale
    // state is process-wide by C-library nature, and per-session
    // locale isolation is WP-3 territory (see ms-stopbleed REPORT).
    crate::executor::local_helpers::with_process_state_lock(|| {
        for name in [
            "LC_ALL",
            "LC_CTYPE",
            "LC_COLLATE",
            "LC_MESSAGES",
            "LC_NUMERIC",
            "LC_TIME",
            "LANG",
        ] {
            match env_vars.get(name) {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    });
}

/// Initialize locale state at shell startup, mirroring GNU shell.c's
/// startup sequence: set_default_locale (locale.c:85) applies
/// setlocale(LC_ALL, "") — the environment composite, silently — then
/// set_default_lang (locale.c set_default_lang) runs
/// set_locale_var("LC_ALL", <env value>), which is the one startup
/// bind that can warn (`<shell_name>: warning: setlocale: LC_ALL:
/// cannot change locale (<value>)`, no line number, and never the
/// strerror suffix: the priming composite attempt already consumed
/// the first-failure errno for the name), and set_lang applies LANG
/// silently. Invalid LANG or LC_* values in the startup environment
/// warn about nothing (rubash#1692).
pub fn init_locale() {
    #[cfg(unix)]
    {
        let _ = attempt_setlocale(libc::LC_ALL, "");
        let env: std::collections::HashMap<String, String> = std::env::vars().collect();
        if let Some(warning) = locale_assignment_warning("LC_ALL", &env) {
            let shell_name = std::env::args()
                .next()
                .unwrap_or_else(|| "rubash".to_string());
            eprintln!("{shell_name}: warning: {warning}");
        }
        let _ = locale_assignment_warning("LANG", &env);
    }
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
/// | `U+E31A` | COMPOUND_ASSIGNMENT_MARKER (compound assignment tag;
/// whitelisted carrier — the interactive line-editing boundary delivers
/// it inside pre-execution AST words, #802) | nothing |
/// | `__RUBASH_CA1__`/`__RUBASH_HD1__`/`__RUBASH_CSB1_` | the retired
/// (pre-#1634) ASCII compound-assignment prefix, QUOTED_HEREDOC_MARKER
/// and the retired (pre-#840) ASCII comsub payload prefix — the live
/// introducers are the U+E31A and U+E319 sentinels | passes through
/// verbatim — plain typeable ASCII with no entry-side encoding; HD1's
/// live tag is consumed at its execution boundary before anything is
/// displayed, and CA1/CSB1 have no live producer at all, so an
/// occurrence here is user data (#824, #840, #1634) |
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
    use crate::executor::substitution_metadata::{
        RAW_BYTE_MARKER_ESCAPE, RAW_BYTE_MARKER_FIRST, RAW_BYTE_MARKER_LAST,
    };
    use crate::executor::types::DEFERRED_COMPOUND_BODY;
    use crate::lexer::{ANSI_C_DQUOTE_MARKER, ANSI_C_QUOTE_MARKER, PARAM_NAME_END_MARKER};

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
        // #802 entry-inventory whitelist: the compound assignment carrier
        // is a legal resident of transport text on the interactive
        // line-editing boundary. The parser pushes it into
        // compound-assignment words (parser/token_actions.rs) and the
        // assignment boundary consumes it before a value is stored
        // (assignment_dispatch.rs / command_words.rs / execution_misc.rs
        // strip it on every post-expansion display path), but hosts
        // inspect AST words BETWEEN parse and execution — the interactive
        // REPL decodes each command word for its easter-egg routing and
        // this-shell dispatch (niubash shell.rs decoded_words). That text
        // legitimately still carries the marker, so the decoder consumes
        // it instead of rejecting it. Since the #1634 protocol revision
        // the carrier is the single registry-zone char U+E31A (the former
        // ASCII spelling `__RUBASH_CA1__` was typeable user text and is
        // now plain data here, like every other retired named-string
        // carrier — #824, #840); the drop arm sits in the match below.
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
            c if c as u32 == RAW_BYTE_MARKER_ESCAPE => match chars.clone().next() {
                Some(next) if next as u32 == RAW_BYTE_MARKER_ESCAPE => {
                    chars.next();
                    out.push(c);
                }
                Some(next)
                    if (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&(next as u32)) =>
                {
                    chars.next();
                    out.push(byte_char(next as u32 - RAW_BYTE_MARKER_FIRST));
                }
                // E000 + <non-payload char> is user data, not a marker:
                // a literal U+E000 that reached the boundary unescaped
                // (rubash#1060). Emit it; `next` decodes on its own.
                Some(_) => out.push(c),
                None => out.push(c),
            },
            DEFERRED_COMPOUND_BODY
            | PARAM_NAME_END_MARKER
            | QUOTED_NULL_MARKER
            | crate::executor::markers::COMPOUND_ASSIGNMENT_MARKER_CHAR
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
    // #824 entry-inventory audit (the #729 carrier-consumer methodology
    // applied to this decoder's OUTPUT side): a registered marker codepoint
    // in `out` is not evidence of a decode failure, so there are no
    // output-side "leaked" assertions here. Every family has a legitimate
    // user-data path to the output:
    //   - C0 carriers (0x11..=0x1f): the user's own bytes ride raw-byte
    //     marker pairs at every entry (registry `user_reachable` contract)
    //     and the E000+E0xx arm restores them as literal chars —
    //     `x=$'\x11'; set -x; echo "$x"` traces byte 0x11, and GNU 5.3.0
    //     prints it verbatim in the xtrace output too.
    //   - PUA markers (the registry zone): user chars are E400
    //     literal-char-escaped at entry (push_literal_char, rubash#735) and
    //     the E400 arm restores them verbatim — `x=$'\uE314Z'; declare -p x`
    //     must print U+E314 (GNU oracle: `declare -- x="\uE314Z"` raw). A
    //     bare guard sentinel cannot survive the scan (the guard arm above
    //     consumes each guard with its next char), so a guard codepoint in
    //     `out` is restored user data, never a live guard.
    //   - Named ASCII markers: `__RUBASH_HD1__` / `__RUBASH_CSB1_` /
    //     `__RUBASH_CA1__` are simply typeable text with no entry-side
    //     encoding to tell them apart; HD1's live producer is consumed at
    //     the heredoc-body execution boundary, CSB1 is the retired
    //     (pre-#840) payload prefix whose introducer moved to the U+E319
    //     sentinel, and CA1 is the retired (pre-#1634) compound prefix
    //     whose introducer moved to the U+E31A sentinel — so an
    //     occurrence on this boundary is user data by contract — GNU
    //     prints it verbatim, and so do the scan's fallthrough arms.
    // Output-side debug_asserts over these codepoints could therefore only
    // ever fire on well-formed user data — each had a two-to-three-line
    // user-literal repro through `declare -p` or xtrace in a debug build
    // (#824; e.g. `x=a__RUBASH_HD1__b; declare -p x`,
    // `set -x; v=$'\uE314Z'; echo "$v"`). Leak detection stays where
    // protocol and data can be told apart: in the scan arms above (a bare
    // marker is always consumed or decoded in-pass) and at the integration
    // boundary on marker-free inputs (tests/marker_leak_golden.rs).
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

/// Map an environment locale name to the name the host collation call
/// wants: on Windows the BCP-47 base ("en_US.UTF-8" -> "en-US"); on POSIX
/// the name VERBATIM, because setlocale(3) resolves the full
/// "lang_TERRITORY.codeset[@modifier]" form while the codeset-stripped
/// base FAILS (returns NULL) whenever the locale was compiled under its
/// full name only — the usual localedef outcome — which silently reduced
/// every collation on such a system to byte order (rubash#1512).
/// Returns None for the C/POSIX locale (and every `C.<codeset>` /
/// `POSIX.<codeset>` form), where collation is plain strcmp — glibc defines
/// the C.UTF-8 family as code-point order (verified against WSL GNU Bash
/// 5.3.0: with `LANG=C.UTF-8`, `[[ ".a" < "a" ]]` and `[[ "B" < "a" ]]`
/// are both true and `sort` yields pure byte order). Without this mapping
/// every C.UTF-8 comparison took the host-collation call; on Windows that
/// is CompareStringEx with NORM_IGNORESYMBOLS, measured at 55ms to sort a
/// single 1500-entry glob result (98% of the glob expansion cost,
/// rubash#157).
fn host_collate_name(name: &str) -> Option<String> {
    // C-family detection without the to_lowercase allocation: ASCII case
    // fold is all POSIX locale names use.
    let bytes = name.as_bytes();
    let is_c = |b: &[u8]| b.eq_ignore_ascii_case(b"c");
    let is_posix = |b: &[u8]| b.eq_ignore_ascii_case(b"posix");
    let c_family = is_c(bytes)
        || is_posix(bytes)
        || (bytes.len() > 1 && (bytes[0] == b'c' || bytes[0] == b'C') && bytes[1] == b'.')
        || (bytes.len() > 6 && name[..6].eq_ignore_ascii_case("posix."));
    if name.is_empty() || c_family {
        return None;
    }
    #[cfg(windows)]
    {
        // glibc "en_US" -> BCP-47 "en-US"; a bare language ("en") is kept
        // as a neutral name and resolved by CompareStringEx.
        let base = name.split(['.', '@']).next().unwrap_or(name);
        Some(base.replace('_', "-"))
    }
    #[cfg(not(windows))]
    {
        // POSIX setlocale wants the full "lang_TERRITORY.codeset[@modifier]"
        // form exactly as named (see the doc comment above, rubash#1512).
        Some(name.to_string())
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

/// Whether the active collation is the C/POSIX family, where strcoll is
/// plain byte order (glibc defines C.UTF-8 as code-point order; verified
/// against WSL GNU Bash 5.3.0 — see host_collate_name). Lets sort call
/// sites resolve the locale ONCE per sort instead of paying
/// collate_locale_name's three env lookups + name mapping per COMPARISON
/// inside strcoll_posixcmp. Result-identical: under C collation
/// strcoll_posixcmp(a, b) == a.cmp(b) exactly, because strcoll never ties
/// distinct strings there so the first-byte/strcmp tie-break never fires
/// (rubash#681 slice b).
pub fn collation_is_c() -> bool {
    host_collate_name(&collate_locale_name()).is_none()
}

/// Sort with byte order under the C collation, host collation otherwise —
/// one locale resolution per sort (see collation_is_c).
pub fn sort_collated(names: &mut [String]) {
    if collation_is_c() {
        names.sort_by(|a, b| a.cmp(b));
    } else {
        names.sort_by(|a, b| strcoll_posixcmp(a, b));
    }
}

/// Reverse variant of sort_collated.
pub fn sort_collated_reverse(names: &mut [String]) {
    if collation_is_c() {
        names.sort_by(|a, b| b.cmp(a));
    } else {
        names.sort_by(|a, b| strcoll_posixcmp(b, a));
    }
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

/// -D / --dump-strings / --dump-po-strings (rubash#1977): collect every
/// locale string ($"...") in `input` without executing anything, the way
/// GNU bash's dump options do. GNU wires the flag as
/// read_but_dont_execute (shell.c:506-510) and prints each string from
/// locale_expand as the word lexer recognizes it (locale.c:550-565);
/// this is the library entry for that face — the rubash binary's main
/// routes the option here, and hosts call it too. It began as niubash's
/// host implementation (the only implementation either binary had);
/// the audit's two-way break (rubash binary missing, niubash shadow) is
/// closed by owning it here.
///
/// GNU recognizes a locale string only where the word lexer reads `$`
/// followed by `"` while scanning a word (parse.y read_token_word's
/// dollar-quote branch). Comment text and here-doc bodies are never
/// lexed as words, so they never dump; single-quoted text, double-quoted
/// spans and backtick bodies are skipped as units; word-embedded,
/// quoted and arithmetic-embedded command substitutions are re-lexed,
/// so locale strings inside them do dump (parse.y:4100 processes `$(`
/// units encountered inside a matched pair). Two recognition classes
/// the original host pass skipped are covered, per GNU 5.3.20 probes:
/// `${...}` bodies (default values, patterns, subscripts are lexed as
/// words: `${x:-$"d"}`, `${x##$"pat"}`, `${arr[$"i"]}` all dump) and
/// bare `$"..."` in arithmetic text (`$((1 + $"a"))`, `((y = $"b"))`,
/// `$[1 + $"c"]` all dump).
///
/// This pass therefore walks the token stream — which already excludes
/// the comment and here-doc-body classes structurally, since the lexer
/// never yields word-shaped tokens from them — and applies the
/// word-level quote rules to the raw spelling of word-shaped tokens.
///
/// Collects `(line, raw body)` pairs into `out`, in source order.
/// `base_line` is the line the token stream's own numbering starts
/// from: top-level tokens carry real script lines in `token.position`,
/// while a re-lexed substitution body restarts at 1, so nested strings
/// are mapped back with `base_line + position - 1`. GNU reports the
/// physical line of each nested string; the two agree whenever the
/// substitution body starts on its token's start line (the
/// overwhelmingly common single-line word).
///
/// Known residuals of the standalone-pass shape (shared with the host
/// implementation it replaces): a syntax error later in the input does
/// not stop collection or change the exit code the way GNU's reader
/// does (GNU dumps the strings read before the error and exits 2), and
/// a nested `$"..."` inside a command substitution inside a dumped
/// body is reported verbatim where GNU rewrites it to `"..."` (needs
/// the byte-exact body serializer of the host-semantic-layer plan, C1
/// residual).
pub fn collect_locale_strings(input: &str, base_line: usize, out: &mut Vec<(usize, String)>) {
    for token in crate::lexer::tokenize(input) {
        let line = base_line + token.position.saturating_sub(1);
        match token.kind {
            crate::lexer::TokenKind::Word
            | crate::lexer::TokenKind::Assignment
            | crate::lexer::TokenKind::BraceExpand => scan_locale_words(&token.raw, line, out),
            // A standalone `${...}` expansion is a Variable token, not
            // a Word (lexer/scanner.rs, the `$` + `{` arm); its raw is
            // the expansion spelling itself, and `$name` raws contain
            // no `$"` by construction, so the word scan is a no-op for
            // them and a body scan for the braced form.
            crate::lexer::TokenKind::Variable => scan_locale_words(&token.raw, line, out),
            crate::lexer::TokenKind::CommandSubst => match substitution_span(&token.raw) {
                SubstitutionSpan::Command(body) => collect_locale_strings(&body, line, out),
                SubstitutionSpan::Arithmetic(body) => scan_arithmetic_text(&body, line, out),
                SubstitutionSpan::None => {}
            },
            _ => {}
        }
    }
}

/// Word-level scan of one token's raw spelling, mirroring where GNU's word
/// lexer recognizes `$"`: outside single quotes, double-quoted spans,
/// backtick bodies and `$'...'` units. `\"` at word level escapes the
/// next character, so `\$"x"` is not a locale string introducer.
fn scan_locale_words(raw: &str, line: usize, out: &mut Vec<(usize, String)>) {
    let bytes = raw.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => i = skip_single_quoted(bytes, i + 1),
            b'`' => i = skip_backquoted(bytes, i + 1),
            b'"' => i = scan_double_quoted(raw, i + 1, line, out),
            b'$' => match bytes.get(i + 1) {
                Some(b'"') => {
                    let close = locale_body_end(raw, i + 2, line, out);
                    out.push((line, raw[i + 2..close].to_string()));
                    i = close + 1;
                }
                Some(b'\'') => i = skip_ansi_c_quoted(bytes, i + 2),
                Some(b'{') => i = scan_dollar_brace_body(raw, i + 2, line, out),
                Some(b'(') => i = scan_substitution_unit(raw, i + 1, line, out),
                _ => i += 1,
            },
            b'\\' => i = (i + 2).min(bytes.len()),
            _ => i += 1,
        }
    }
}

/// `${...}` spans are delimited by skip_dollar_brace, but their bodies
/// are word text to GNU's lexer — default values, patterns and
/// subscripts are lexed as words, so locale strings inside them dump.
/// Scan the body with the word scanner instead of skipping the unit
/// whole; the returned index is the same one the skip would give.
fn scan_dollar_brace_body(
    raw: &str,
    start: usize,
    line: usize,
    out: &mut Vec<(usize, String)>,
) -> usize {
    let end = skip_dollar_brace(raw.as_bytes(), start);
    let body_end = if end > start && raw.as_bytes()[end - 1] == b'}' {
        end - 1
    } else {
        end
    };
    scan_locale_words(&raw[start..body_end], line, out);
    end
}

/// Byte index of the closing `"` of a locale string body whose text starts at
/// `start` (just past the opening quote). Nested `${...}`/`` `...` ``/`$(...)`
/// units are skipped the way GNU parse_matched_pair skips them while it
/// extracts the pair, and command-substitution units are re-lexed so their
/// own locale strings dump first (GNU order: inner before outer). The
/// surrounding body is reported verbatim: GNU additionally rewrites nested
/// `$"..."` units to `"..."` inside the body it dumps, which needs a
/// byte-exact body serializer rubash does not expose (host-semantic-layer
/// plan, C1 residual).
fn locale_body_end(raw: &str, start: usize, line: usize, out: &mut Vec<(usize, String)>) -> usize {
    let bytes = raw.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return i,
            b'\\' => i = (i + 2).min(bytes.len()),
            b'`' => i = skip_backquoted(bytes, i + 1),
            b'$' => match bytes.get(i + 1) {
                Some(b'{') => i = skip_dollar_brace(bytes, i + 2),
                Some(b'(') => i = scan_substitution_unit(raw, i + 1, line, out),
                _ => i += 1,
            },
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Walks a double-quoted span. `$` followed by `"` is literal data here (GNU
/// dumps nothing for `echo "$"dqp" tail"`), while `$(...)` units are re-lexed
/// (parse.y:4100) and their locale strings dump, and `${...}` units carry
/// word text whose locale strings dump (GNU probe: `"${x:-$"d"}"` dumps).
fn scan_double_quoted(
    raw: &str,
    start: usize,
    line: usize,
    out: &mut Vec<(usize, String)>,
) -> usize {
    let bytes = raw.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return i + 1,
            b'\\' => i = (i + 2).min(bytes.len()),
            b'`' => i = skip_backquoted(bytes, i + 1),
            b'$' => match bytes.get(i + 1) {
                Some(b'{') => i = scan_dollar_brace_body(raw, i + 2, line, out),
                Some(b'(') => i = scan_substitution_unit(raw, i + 1, line, out),
                _ => i += 1,
            },
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Extracts the span of a `$(...)` / `$((...))` unit whose `(` sits at `open`
/// and dispatches it: command-substitution bodies are re-lexed (GNU dumps
/// their locale strings, unquoted, word-embedded or double-quoted alike) and
/// arithmetic bodies are scanned as arithmetic text.
fn scan_substitution_unit(
    raw: &str,
    open: usize,
    line: usize,
    out: &mut Vec<(usize, String)>,
) -> usize {
    let bytes = raw.as_bytes();
    let Some(close) = paren_close(bytes, open) else {
        return bytes.len();
    };
    if bytes.get(open + 1) == Some(&b'(') {
        scan_arithmetic_text(&raw[open + 2..close - 1], line, out);
    } else {
        collect_locale_strings(&raw[open + 1..close], line, out);
    }
    close + 1
}

/// Arithmetic text (`$(( ... ))` inner span, `(( ... ))` bodies,
/// `$[ ... ]`): a bare `$"..."` dumps here — GNU's arithmetic is lexed
/// by the same word machinery (GNU 5.3.20 probes above) — and quoted
/// command substitutions are parsed by GNU and their locale strings
/// dump (GNU 5.3.0: `$(( "$(echo $"x")" + 1 ))` dumps `"x"`).
fn scan_arithmetic_text(text: &str, line: usize, out: &mut Vec<(usize, String)>) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => i = scan_double_quoted(text, i + 1, line, out),
            b'`' => i = skip_backquoted(bytes, i + 1),
            b'$' if bytes.get(i + 1) == Some(&b'"') => {
                let close = locale_body_end(text, i + 2, line, out);
                out.push((line, text[i + 2..close].to_string()));
                i = close + 1;
            }
            b'$' if bytes.get(i + 1) == Some(&b'(') => {
                if bytes.get(i + 2) == Some(&b'(') {
                    // Nested arithmetic span: scanned as arithmetic text
                    // by this same walk (its own `$"..."` still dumps).
                    i += 3;
                } else {
                    i = scan_substitution_unit(text, i + 1, line, out);
                }
            }
            b'\\' => i = (i + 2).min(bytes.len()),
            _ => i += 1,
        }
    }
}

enum SubstitutionSpan {
    Command(String),
    Arithmetic(String),
    None,
}

/// Classifies a CommandSubst token's raw spelling: a `` `...` `` token shares
/// the kind but never starts with `$(`, and its body must not be re-lexed
/// (GNU keeps backtick bodies verbatim at parse time, so `echo `echo $"x"``
/// dumps nothing). `$((...))` yields its arithmetic inner span.
fn substitution_span(raw: &str) -> SubstitutionSpan {
    let bytes = raw.as_bytes();
    if bytes.first() != Some(&b'$') || bytes.get(1) != Some(&b'(') {
        return SubstitutionSpan::None;
    }
    let Some(close) = paren_close(bytes, 1) else {
        return SubstitutionSpan::None;
    };
    if bytes.get(2) == Some(&b'(') {
        SubstitutionSpan::Arithmetic(raw[3..close - 1].to_string())
    } else {
        SubstitutionSpan::Command(raw[2..close].to_string())
    }
}

/// Byte index of the `)` matching the `(` at `open`, honoring quoting the way
/// GNU parse_matched_pair does while it extracts a substitution span. Returns
/// None when the span never closes; callers then treat the rest of the text
/// as the unit, which keeps the scan total on malformed input.
fn paren_close(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Some(i - 1);
                }
            }
            b'\'' => i = skip_single_quoted(bytes, i + 1),
            b'"' => i = skip_double_span(bytes, i + 1),
            b'`' => i = skip_backquoted(bytes, i + 1),
            b'\\' => i = (i + 2).min(bytes.len()),
            _ => i += 1,
        }
    }
    None
}

fn skip_single_quoted(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

fn skip_ansi_c_quoted(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => return i + 1,
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn skip_backquoted(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'`' => return i + 1,
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn skip_double_span(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return i + 1,
            b'\\' => i = (i + 2).min(bytes.len()),
            b'`' => i = skip_backquoted(bytes, i + 1),
            b'$' if bytes.get(i + 1) == Some(&b'{') => i = skip_dollar_brace(bytes, i + 2),
            _ => i += 1,
        }
    }
    bytes.len()
}

fn skip_dollar_brace(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 1usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return i;
                }
            }
            b'\'' => i = skip_single_quoted(bytes, i + 1),
            b'"' => i = skip_double_span(bytes, i + 1),
            b'`' => i = skip_backquoted(bytes, i + 1),
            b'\\' => i = (i + 2).min(bytes.len()),
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Renders collected locale strings exactly the way GNU bash prints them
/// (locale.c locale_expand): plain mode is `"body"` with the raw body text
/// verbatim (escapes stay escaped, embedded newlines split the output line),
/// and PO mode is the mk_msgstr form anchored by `#: name:lineno`.
pub fn render_locale_string_dump(
    strings: &[(usize, String)],
    po: bool,
    source_name: &str,
) -> String {
    let mut out = String::new();
    for (line, body) in strings {
        if po {
            let mut escaped = String::new();
            let mut multiline = false;
            for ch in body.chars() {
                match ch {
                    '\n' => {
                        escaped.push_str("\\n\"\n\"");
                        multiline = true;
                    }
                    '"' | '\\' => {
                        escaped.push('\\');
                        escaped.push(ch);
                    }
                    _ => escaped.push(ch),
                }
            }
            if multiline {
                out.push_str(&format!(
                    "#: {source_name}:{line}\nmsgid \"\"\n\"{escaped}\"\nmsgstr \"\"\n"
                ));
            } else {
                out.push_str(&format!(
                    "#: {source_name}:{line}\nmsgid \"{escaped}\"\nmsgstr \"\"\n"
                ));
            }
        } else {
            out.push('"');
            out.push_str(body);
            out.push_str("\"\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        active_locale_name, ansi_lex_locale_name, is_utf8_locale_name, locale_name, strcoll,
        strcoll_posixcmp, with_ansi_lex_locale, ShellLocaleGuard,
    };
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

    #[test]
    fn ansi_lex_locale_override_restores_outer_pin() {
        // Unpinned: the decoder locale is the live environment's.
        assert_eq!(ansi_lex_locale_name(), locale_name());
        with_ansi_lex_locale("C", || {
            assert_eq!(ansi_lex_locale_name(), "C");
            with_ansi_lex_locale("en_US.UTF-8", || {
                assert_eq!(ansi_lex_locale_name(), "en_US.UTF-8");
            });
            // The inner pin's exit restores the outer pin, not a clear.
            assert_eq!(ansi_lex_locale_name(), "C");
        });
        assert_eq!(ansi_lex_locale_name(), locale_name());
    }

    #[test]
    fn shell_locale_guard_restores_outer_guard() {
        let mut outer_vars = std::collections::HashMap::new();
        outer_vars.insert("LC_ALL".to_string(), "C".to_string());
        let mut inner_vars = std::collections::HashMap::new();
        inner_vars.insert("LC_ALL".to_string(), "en_US.UTF-8".to_string());
        assert_eq!(active_locale_name(), locale_name());
        {
            let _outer = ShellLocaleGuard::new(&outer_vars);
            assert_eq!(active_locale_name(), "C");
            {
                let _inner = ShellLocaleGuard::new(&inner_vars);
                assert_eq!(active_locale_name(), "en_US.UTF-8");
            }
            // The inner guard's exit restores the outer guard's locale.
            assert_eq!(active_locale_name(), "C");
        }
        assert_eq!(active_locale_name(), locale_name());
    }

    /// rubash#681 slice b: the hoisted sort must match the per-comparison
    /// comparator exactly (byte order under C collation, host collation
    /// otherwise) — forward and reverse, on collation-tricky data.
    #[test]
    fn sort_collated_matches_per_comparison_sort() {
        let data = vec![
            ".a".to_string(),
            "a".to_string(),
            ".aa".to_string(),
            "aa".to_string(),
            "B".to_string(),
            "a".to_string(),
            "".to_string(),
            "z".to_string(),
        ];
        let mut hoisted = data.clone();
        super::sort_collated(&mut hoisted);
        let mut reference = data.clone();
        reference.sort_by(|a, b| strcoll_posixcmp(a, b));
        assert_eq!(hoisted, reference);
        let mut hoisted_rev = data.clone();
        super::sort_collated_reverse(&mut hoisted_rev);
        let mut reference_rev = data;
        reference_rev.sort_by(|a, b| strcoll_posixcmp(b, a));
        assert_eq!(hoisted_rev, reference_rev);
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

    #[test]
    fn multibyte_char_offsets_walk_invalid_bytes_one_at_a_time() {
        // Empty and plain ASCII: one offset per byte.
        assert_eq!(super::multibyte_char_offsets(b""), vec![0]);
        assert_eq!(super::multibyte_char_offsets(b"abc"), vec![0, 1, 2, 3]);
        // A valid multibyte sequence is a single character.
        assert_eq!(super::multibyte_char_offsets(b"\xc3\xa9"), vec![0, 2]);
        assert_eq!(super::multibyte_char_offsets(b"\xe2\x82\xac"), vec![0, 3]);
        // An invalid byte is one character (ADVANCE_CHAR's mbrlen -1 path).
        assert_eq!(super::multibyte_char_offsets(&[0xff]), vec![0, 1]);
        assert_eq!(
            super::multibyte_char_offsets(&[0x61, 0xff, 0x62]),
            vec![0, 1, 2, 3]
        );
        // Truncated lead bytes are each their own invalid character.
        assert_eq!(super::multibyte_char_offsets(&[0xe2, 0x82]), vec![0, 1, 2]);
        assert_eq!(
            super::multibyte_char_offsets(&[0xe2, 0x82, 0xac, 0xff, 0x41]),
            vec![0, 3, 4, 5]
        );
    }
}

#[cfg(test)]
mod dump_strings_tests {
    // rubash#1977: the collector behind -D / --dump-strings /
    // --dump-po-strings. Expectations measured against GNU bash
    // 5.3.20 running the same inputs (the recognition matrix was
    // first probed case-by-case for the niubash host implementation
    // this code descends from; the ${...} and arithmetic classes are
    // the widenings the GNU probes forced).
    fn dumped(input: &str) -> Vec<(usize, String)> {
        let mut strings = Vec::new();
        super::collect_locale_strings(input, 1, &mut strings);
        strings
    }

    fn bodies(input: &str) -> Vec<String> {
        dumped(input).into_iter().map(|(_, body)| body).collect()
    }

    #[test]
    fn recognition_matches_gnu_word_lexer() {
        assert_eq!(bodies("echo $\"plain\""), vec!["plain"]);
        assert_eq!(bodies("echo a$\"mid\"dle"), vec!["mid"]);
        assert_eq!(bodies("x=$\"assign.rhs\""), vec!["assign.rhs"]);
        assert_eq!(bodies("echo $\"one\" $\"two\""), vec!["one", "two"]);
        assert_eq!(bodies("echo $\"a\"$\"b\""), vec!["a", "b"]);
        assert!(bodies("# comment with $\"in.comment\" text").is_empty());
        assert!(bodies("echo '$\"single.quoted\" not locale'").is_empty());
        assert!(bodies("echo \"$\"dqp\" tail\"").is_empty());
        assert!(bodies("echo \"a$\"b\"c\"").is_empty());
        assert!(bodies("echo `echo $\"in.backtick\"`").is_empty());
        assert!(bodies("echo \\$\"escaped.dollar\"").is_empty());
        assert_eq!(
            bodies("cat <<EOF\nheredoc body with $\"in.heredoc\"\nEOF\necho $\"after\""),
            vec!["after"]
        );
    }

    #[test]
    fn escapes_stay_raw_and_newlines_stay_in_the_body() {
        // locale.c locale_expand printf("\"%s\"\n", temp) with temp a
        // verbatim copy of the body.
        assert_eq!(
            bodies("echo $\"esc \\\"q1\\\" q2\""),
            vec!["esc \\\"q1\\\" q2"]
        );
        assert_eq!(
            bodies("echo $\"tail.backslash\\\\\""),
            vec!["tail.backslash\\\\"]
        );
        assert_eq!(bodies("echo $\"multi\nline\""), vec!["multi\nline"]);
    }

    #[test]
    fn command_substitutions_are_relexed() {
        assert_eq!(bodies("echo $(echo $\"in.comsub\")"), vec!["in.comsub"]);
        assert_eq!(
            bodies("echo pre$(echo $\"midcomsub\")post"),
            vec!["midcomsub"]
        );
        assert_eq!(
            bodies("echo \"$(echo $\"quotedcomsub\")\""),
            vec!["quotedcomsub"]
        );
        assert_eq!(
            bodies("echo $(( $(echo $\"arithcomsub\") + 1 ))"),
            vec!["arithcomsub"]
        );
    }

    #[test]
    fn dollar_brace_bodies_dump() {
        // GNU 5.3.20: expansion words inside ${...} are lexed as
        // words, so their locale strings dump.
        assert_eq!(
            bodies("echo ${x:-$\"brace.default\"}"),
            vec!["brace.default"]
        );
        assert_eq!(bodies("echo ${x-$\"brace.dash\"}"), vec!["brace.dash"]);
        assert_eq!(
            bodies("echo \"${x:-$\"quoted.default\"}\""),
            vec!["quoted.default"]
        );
        assert_eq!(bodies("echo ${x##$\"pattern\"}"), vec!["pattern"]);
        assert_eq!(bodies("echo ${arr[$\"subscript\"]}"), vec!["subscript"]);
        // The unit is still skipped as a span: text after it scans on.
        assert_eq!(bodies("echo ${x} $\"after\""), vec!["after"]);
    }

    #[test]
    fn arithmetic_text_dumps() {
        // GNU 5.3.20: $((1 + $"a")), ((y = $"b")) and $[1 + $"c"]
        // all dump the bare locale string.
        assert_eq!(bodies("echo $((1 + $\"arith\"))"), vec!["arith"]);
        assert_eq!(bodies("echo $[1 + $\"oldstyle\"]"), vec!["oldstyle"]);
        // A ${...} default word inside arithmetic text is lexed as a
        // word too: the walk reaches its $"..." and dumps it.
        assert_eq!(
            bodies("echo $(( ${x:-$\"arith.brace\"} + 1 ))"),
            vec!["arith.brace"]
        );
    }

    #[test]
    fn po_render_matches_gnu_format() {
        let render = |input: &str| {
            let strings = dumped(input);
            super::render_locale_string_dump(&strings, true, "probe.sh")
        };
        assert_eq!(
            render("echo $\"plain\""),
            "#: probe.sh:1\nmsgid \"plain\"\nmsgstr \"\"\n"
        );
        assert_eq!(
            render("echo $\"esc \\\"q1\\\" q2\""),
            "#: probe.sh:1\nmsgid \"esc \\\\\\\"q1\\\\\\\" q2\"\nmsgstr \"\"\n"
        );
        assert_eq!(
            render("echo $\"multi\nline\""),
            "#: probe.sh:1\nmsgid \"\"\n\"multi\\n\"\n\"line\"\nmsgstr \"\"\n"
        );
        // One entry per string, anchored on its own line.
        assert_eq!(
            render("echo $\"one\" $\"two\"\necho $\"three\""),
            "#: probe.sh:1\nmsgid \"one\"\nmsgstr \"\"\n\
             #: probe.sh:1\nmsgid \"two\"\nmsgstr \"\"\n\
             #: probe.sh:2\nmsgid \"three\"\nmsgstr \"\"\n"
        );
        assert_eq!(
            render("echo $\"\""),
            "#: probe.sh:1\nmsgid \"\"\nmsgstr \"\"\n"
        );
    }

    #[test]
    fn plain_render_is_the_verbatim_body_in_quotes() {
        let strings = dumped("echo $\"a\"\necho $\"b c\"");
        assert_eq!(
            super::render_locale_string_dump(&strings, false, "probe.sh"),
            "\"a\"\n\"b c\"\n"
        );
    }
}
