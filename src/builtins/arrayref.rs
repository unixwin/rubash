//! GNU arrayfunc.c:1288 tokenize_array_reference /
//! arrayfunc.c:1348 valid_array_reference — the `name[sub]` validity check
//! shared by `read` (read.def:1037/1090), `printf -v` (printf.def:306),
//! and other builtin variable-name operands.
//!
//! The check is flag-sensitive: SET_VFLAGS (builtins/common.h:279) turns
//! the array_expand_once/assoc_expand_once shopt into VA_NOEXPAND, and only
//! then is the base name's associative-ness consulted (arrayfunc.c:1300).
//! Without VA_NOEXPAND the subscript is scanned with arithmetic rules
//! (subst.c:2086 skip_matched_pair, flags=0), so `a[80's]` — an unbalanced
//! single quote — is not a valid reference (assoc9.sub `read a[$b]`).

use crate::executor::markers::DATA_DOLLAR;
use std::collections::HashMap;

/// In-band W_ARRAYREF carrier (GNU execute_cmd.c:4366 fix_arrayref_words):
/// an operand word of an ARRAYREF_BUILTIN whose pre-expansion text passed
/// valid_array_reference(.., 0) carries this flag through expansion, where
/// SET_VFLAGS (common.h:279) / builtin_arrayref_flags (common.c:1050) turn
/// it into VA_ONEWORD|VA_NOEXPAND for post-expansion validation. That is
/// what distinguishes `read A[$rkey]` (marked pre-expansion, expands to the
/// valid `]` subscript `A[]]`) from literal `read A[]]` and quoted
/// `read "A[$rkey]"` (both unmarked, both invalid) under assoc_expand_once.
/// The byte travels inside the word string so it survives word splitting
/// the same way GNU copies word->flags to each expanded output word; every
/// consumer strips it before use.
pub(crate) const ARRAYREF_FLAG: char = crate::executor::markers::ARRAYREF_FLAG;

/// Split a possibly-marked operand word into (w_arrayref, text).
pub(crate) fn take_arrayref_flag(word: &str) -> (bool, &str) {
    match word.strip_prefix(ARRAYREF_FLAG) {
        Some(rest) => (true, rest),
        None => (false, word),
    }
}

/// GNU builtins/mkbuiltins.c:180 arrayvar_builtins — the builtins whose
/// operand words get W_ARRAYREF in fix_arrayref_words.
pub(crate) fn is_arrayref_builtin(name: &str) -> bool {
    matches!(
        name,
        "declare"
            | "let"
            | "local"
            | "printf"
            | "read"
            | "test"
            | "["
            | "typeset"
            | "unset"
            | "wait"
    )
}

/// Return true when `word` is a well-formed `name[sub]` array reference:
/// a valid identifier base, a non-empty subscript, and the closing `]`
/// as the last byte. `noexpand` is VA_NOEXPAND (SET_VFLAGS maps the
/// array_expand_once shopt; builtin_arrayref_flags also sets it for
/// W_ARRAYREF words); `oneword` is the effective
/// (VA_NOEXPAND|VA_ONEWORD) == both condition — GNU arrayfunc.c:1307-1309
/// then takes `len = strlen(t) - 1`, accepting any `]`-terminated
/// subscript for an assoc base. `base_is_assoc` is assoc_p(base) — GNU
/// only performs the assoc lookup when VA_NOEXPAND is set.
pub(crate) fn valid_array_reference(
    word: &str,
    noexpand: bool,
    base_is_assoc: bool,
    oneword: bool,
) -> bool {
    let Some(open) = word.find('[') else {
        return false;
    };
    if !valid_identifier(&word[..open]) {
        return false;
    }
    let tail = &word.as_bytes()[open..];
    let close = if base_is_assoc && oneword {
        // ONEWORD (arrayfunc.c:1307-1310): len = strlen(t) - 1 — the tail
        // is accepted wholesale as long as it ends with `]`.
        Some(tail.len() - 1)
    } else if noexpand && base_is_assoc {
        // skipsubscript(t, 0, ssflags|1): skip_matched_pair flags&1
        // disables escapes, quote spans, substitutions, and bracket
        // nesting — the first `]` closes the subscript.
        tail.iter().position(|&b| b == b']')
    } else {
        skip_matched_pair_arith(tail)
    };
    // GNU arrayfunc.c:1319: t[len] must be `]`, the subscript must be
    // non-empty (len > 1), and `]` must be the last byte.
    matches!(close, Some(index) if index > 1 && index + 1 == tail.len() && tail[index] == b']')
}

/// GNU subst.c:2086 skip_matched_pair(string, 0, '[', ']', 0): scan a
/// bracketed subscript under arithmetic rules — backslash escapes,
/// backquote spans, single/double quoted spans skipped wholesale, `[`/`]`
/// nesting counted, and `$(`/`${` extracted as balanced units. Returns
/// Some(index) of the closing `]` at depth 0, or None when the scan ran
/// off the end (an unterminated quote or unmatched bracket makes the
/// reference invalid).
fn skip_matched_pair_arith(tail: &[u8]) -> Option<usize> {
    debug_assert_eq!(tail.first(), Some(&b'['));
    let mut index = 1usize;
    let mut depth = 1usize;
    while index < tail.len() {
        match tail[index] {
            b'\\' => index += 2,
            b'`' => {
                index += 1;
                while index < tail.len() {
                    match tail[index] {
                        b'\\' => index += 2,
                        b'`' => {
                            index += 1;
                            break;
                        }
                        _ => index += 1,
                    }
                }
            }
            b'[' => {
                depth += 1;
                index += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            b'\'' => {
                index += 1;
                while index < tail.len() && tail[index] != b'\'' {
                    index += 1;
                }
                index += 1;
            }
            b'"' => {
                index += 1;
                while index < tail.len() {
                    match tail[index] {
                        b'\\' => index += 2,
                        b'"' => {
                            index += 1;
                            break;
                        }
                        _ => index += 1,
                    }
                }
            }
            b'$' if matches!(tail.get(index + 1), Some(b'(') | Some(b'{')) => {
                index = skip_balanced_substitution(tail, index);
            }
            _ => index += 1,
        }
    }
    None
}

/// GNU subst.c extract_delimited_string / extract_dollar_brace_string:
/// skip a `$(...)`, `$((...))`, or `${...}` unit starting at `start`
/// (`text[start] == b'$'`) with quote, escape, and nesting awareness.
/// Returns the index just past the unit (or past EOS if unterminated).
fn skip_balanced_substitution(text: &[u8], start: usize) -> usize {
    let (open, close) = if text.get(start + 1) == Some(&b'{') {
        (b'{', b'}')
    } else {
        (b'(', b')')
    };
    let mut index = start + 2;
    let mut depth = 1usize;
    while index < text.len() {
        match text[index] {
            b'\\' => index += 2,
            b'\'' => {
                index += 1;
                while index < text.len() && text[index] != b'\'' {
                    index += 1;
                }
                index += 1;
            }
            b'"' => {
                index += 1;
                while index < text.len() {
                    match text[index] {
                        b'\\' => index += 2,
                        b'"' => {
                            index += 1;
                            break;
                        }
                        _ => index += 1,
                    }
                }
            }
            c if c == open => {
                depth += 1;
                index += 1;
            }
            c if c == close => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => index += 1,
        }
    }
    index
}

/// Convenience for builtin operands: resolve the inputs GNU derives from
/// live shell state — the array_expand_once shopt and assoc_p(base) — then
/// run valid_array_reference. `w_arrayref` is the operand word's in-band
/// W_ARRAYREF flag; `one_word_gated` mirrors SET_VFLAGS (common.h:279),
/// which only lets W_ARRAYREF set VA_ONEWORD when array_expand_once is on —
/// read.def:404/1036/1089, printf.def:305, wait.def:156 all use it.
/// unset (set.def:887) instead uses builtin_arrayref_flags (common.c:1050),
/// which applies VA_ONEWORD|VA_NOEXPAND unconditionally; that caller passes
/// `one_word_gated: false` and pre-ORs its own noexpand.
pub(crate) fn valid_array_reference_for_env(
    word: &str,
    env_vars: &HashMap<String, String>,
    w_arrayref: bool,
) -> bool {
    let Some(open) = word.find('[') else {
        return false;
    };
    // SET_VFLAGS (common.h:279): vflags = expand_once ? VA_NOEXPAND : 0;
    // if (expand_once && W_ARRAYREF) vflags |= VA_ONEWORD|VA_NOEXPAND.
    // Without array_expand_once a marked word still gets the arithmetic
    // subscript scan (`read A[$rkey]` -> `A[]]` is then invalid).
    let expand_once = crate::builtins::shopt::option_enabled(env_vars, "array_expand_once");
    let oneword = w_arrayref && expand_once;
    let base_is_assoc = expand_once && is_marked_assoc(env_vars, &word[..open]);
    valid_array_reference(word, expand_once, base_is_assoc, oneword)
}

/// GNU common.c builtin_arrayref_flags semantics for unset (set.def:887):
/// VA_ONEWORD|VA_NOEXPAND is applied whenever the operand carried
/// W_ARRAYREF — no array_expand_once gate.
pub(crate) fn valid_array_reference_for_unset(
    word: &str,
    env_vars: &HashMap<String, String>,
    w_arrayref: bool,
) -> bool {
    let Some(open) = word.find('[') else {
        return false;
    };
    let expand_once = crate::builtins::shopt::option_enabled(env_vars, "array_expand_once");
    let noexpand = expand_once || w_arrayref;
    let base_is_assoc = noexpand && is_marked_assoc(env_vars, &word[..open]);
    valid_array_reference(word, noexpand, base_is_assoc, w_arrayref)
}

/// GNU general.c legal_identifier: a shell variable name — leading
/// alpha/underscore, then alphanumeric/underscore.
fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_marked_assoc(env_vars: &HashMap<String, String>, base: &str) -> bool {
    env_vars
        .get(crate::executor::types::ASSOC_VARS)
        .map(|marked| marked.split(DATA_DOLLAR).any(|entry| entry == base))
        .unwrap_or(false)
}
