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
use std::cell::Cell;

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
    static UTF8_ACTIVE_CACHE: Cell<Option<(String, bool)>> = const { Cell::new(None) };
}

#[cfg(unix)]
fn utf8_locale_active(name: &str) -> bool {
    UTF8_ACTIVE_CACHE.with(|cache| {
        if let Some((seen, value)) = *cache.get() {
            if seen == name {
                return value;
            }
        }
        let value = probe_utf8_locale(name);
        cache.set(Some((name.to_string(), value)));
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

#[cfg(test)]
mod tests {
    use super::is_utf8_locale_name;

    #[test]
    fn utf8_charset_names_are_recognized_case_insensitively() {
        assert!(is_utf8_locale_name("en_US.UTF-8"));
        assert!(is_utf8_locale_name("C.UTF-8"));
        assert!(is_utf8_locale_name("C.utf8"));
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
