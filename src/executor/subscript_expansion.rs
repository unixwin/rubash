//! `subst.c expand_subscript_string` for associative-array subscripts.
//!
//! GNU reaches it from two places and both must produce the same key:
//!
//!   * the assignment path -- `assign_array_element_internal`
//!     (`arrayfunc.c:392-398`) after `general.c:480 assignment()` split the
//!     SYNTACTIC word into name / subscript / value;
//!   * the arithmetic path -- `array_variable_part` -> `array_value_internal`
//!     (`arrayfunc.c:1483`) reached from `expr_streval` (`expr.c:1150`).
//!
//! The subscript is word-expanded exactly once, with `W_NOSPLIT2 |
//! W_NOPROCSUB`: parameter, command, arithmetic and tilde expansion plus
//! quote removal, and no field splitting, no pathname expansion and no
//! process substitution. The result is the key verbatim and is never
//! re-expanded.

use super::*;

/// Legacy single-byte data markers shared with the lexer and the
/// embedded-parameter walker (see `executor/parameter_errors.rs`).
const LITERAL_BACKSLASH: char = '\x14';
const LITERAL_SINGLE_QUOTE: char = '\x17';
const LITERAL_DOUBLE_QUOTE: char = '\x18';
const LITERAL_BACKTICK: char = '\x1a';
const LITERAL_DOLLAR: char = '\x1f';

impl Executor {
    /// One `expand_subscript_string` pass over the raw subscript text.
    pub(in crate::executor) fn expand_subscript_string(&self, raw: &str) -> String {
        // Nothing expands inside a single-quoted span, so a subscript covered
        // entirely by single quotes is literal data: `A['$v']` keys on `$v`
        // and `A['a\b']` keeps the backslash.
        if let Some(literal) = wholly_single_quoted_literal(raw) {
            return literal;
        }
        // Quote removal belongs to the LEXER token -- `\X` loses its backslash
        // there -- while the parameter/command/arithmetic walker below only
        // reads data. Resolve the escapes first so the walker never mistakes a
        // quoted `$`, `` ` `` or quote for an expansion (GNU subst.c
        // `expand_word_internal` sees the same already-dequoted characters).
        let masked = mask_subscript_escapes(raw);
        let expanded = self.expand_embedded_parameters(&masked);
        // A leading unquoted `~` tilde-expands; `x~` and `a:~` stay literal
        // and `"~"` never reaches here (its first character is the quote).
        if raw.starts_with('~') {
            return tilde_expand::expand_word_prefix(&expanded, &self.env_vars).unwrap_or(expanded);
        }
        expanded
    }
}

/// Resolve the subscript token's quoting the way the lexer does, leaving the
/// walker a string whose remaining `$`, `` ` `` and quote characters are all
/// live syntax.
///
/// A backslash quotes the following character. In unquoted context the pair
/// collapses to the character alone; inside double quotes a backslash is
/// special before `$`, `` ` ``, `"` and `\` only (subst.c
/// `string_extract_double_quoted`) and stays literal before anything else.
fn mask_subscript_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(ch) = chars.next() {
        if in_single {
            out.push(ch);
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        if ch == '"' {
            in_double = !in_double;
            out.push(ch);
            continue;
        }
        if ch != '\\' {
            if ch == '\'' && !in_double {
                in_single = true;
            }
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            // A trailing backslash is literal.
            out.push('\\');
            break;
        };
        if next == '\n' {
            // Line continuation: the backslash and the newline both vanish.
            continue;
        }
        match next {
            '\\' => out.push(LITERAL_BACKSLASH),
            '$' => out.push(LITERAL_DOLLAR),
            '`' => out.push(LITERAL_BACKTICK),
            '\'' if !in_double => out.push(LITERAL_SINGLE_QUOTE),
            '"' => out.push(LITERAL_DOUBLE_QUOTE),
            _ if in_double => {
                out.push('\\');
                out.push(next);
            }
            _ => out.push(next),
        }
    }
    out
}

/// The concatenated contents of `text` when it is covered entirely by
/// single-quoted spans (`'a b'`, `'a''b'`); `None` when any character sits
/// outside a single-quoted span, in which case the subscript still has to be
/// expanded.
pub(in crate::executor) fn wholly_single_quoted_literal(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    let mut saw_span = false;
    while !rest.is_empty() {
        let inner = rest.strip_prefix('\'')?;
        let end = inner.find('\'')?;
        out.push_str(&inner[..end]);
        rest = &inner[end + 1..];
        saw_span = true;
    }
    saw_span.then_some(out)
}
