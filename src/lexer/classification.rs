use crate::executor::markers::DATA_DOLLAR;
pub(super) fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "while"
            | "do"
            | "done"
            | "until"
            | "for"
            | "case"
            | "esac"
            | "in"
            | "function"
            | "select"
            | "time"
            | "coproc"
    )
}

pub(crate) fn is_assignment(word: &str) -> bool {
    let Some(pos) = word.find('=') else {
        return false;
    };
    let var_name = word[..pos].strip_suffix('+').unwrap_or(&word[..pos]);
    !var_name.is_empty()
        && var_name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && var_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// GNU general.c:480 `assignment()` on the raw token, bracket arm
/// included — the reader's own classification (parse.y:5789-5796):
/// a word is an assignment word when it is a legal variable name, or
/// `name[subscript]` with the subscript closed by skipsubscript's
/// balance-0 `]`, immediately followed by `=` or `+=`. Unlike
/// [`is_assignment`] this accepts the array-element form
/// (`a[64|3]=A`), which is exactly the form GNU returns
/// ASSIGNMENT_WORD for (parse.y:5846, when the word was also read in
/// assignment-acceptable position). Used for the reader-history
/// classification in scanner.rs record_token; the token-stream kind
/// keeps using is_assignment (element words ride the parser's own
/// array-element channel there).
pub(crate) fn is_assignment_word(word: &str) -> bool {
    let bytes = word.as_bytes();
    // legal_variable_starter on the first character (general.c:499).
    if !matches!(bytes.first(), Some(b) if b.is_ascii_alphabetic() || *b == b'_') {
        return false;
    }
    let mut indx = 0usize;
    while indx < bytes.len() {
        let c = bytes[indx];
        if c == b'=' {
            // general.c:507: `=` at indx (never 0 — the starter check
            // above consumed a name character class member).
            return indx > 0;
        }
        if c == b'[' {
            // general.c:512-521: skipsubscript to the balance-0 `]`,
            // then `=` or `+=` must follow immediately.
            let Some(close) = skip_subscript_close(bytes, indx) else {
                return false;
            };
            let newi = close + 1;
            if bytes.get(newi) == Some(&b'+') && bytes.get(newi + 1) == Some(&b'=') {
                return true;
            }
            return bytes.get(newi) == Some(&b'=');
        }
        // general.c:525: `+=` for the scalar form.
        if c == b'+' && bytes.get(indx + 1) == Some(&b'=') {
            return true;
        }
        // general.c:530: variable names contain only letters, digits
        // and `_`; anything else ends the classification.
        if !(c.is_ascii_alphanumeric() || c == b'_') {
            return false;
        }
        indx += 1;
    }
    false
}

/// Index of the `]` closing the subscript opened at `bytes[open]`,
/// per skipsubscript (subst.c:2186) = skip_matched_pair with
/// FLAGS & 1 == 0 (subst.c:2086-2177): brackets nest, a backslash
/// protects the next byte, and backquote spans, single- and
/// double-quoted spans and `$(...)` / `${...}` constructs are
/// skipped as units — a `]` inside one of them is data. The lexer's
/// own port (the executor keeps its twin in arrays/storage.rs; the
/// layers do not import each other). None when no balance-0 `]`
/// exists.
fn skip_subscript_close(bytes: &[u8], open: usize) -> Option<usize> {
    if bytes.get(open) != Some(&b'[') {
        return None;
    }
    skip_matched_pair_end(bytes, open + 1, b'[', b']').map(|end| end - 1)
}

/// Byte port of skip_matched_pair (subst.c:2086-2177), FLAGS & 1 == 0:
/// starting just past an opening delimiter at depth 1, return the
/// index just past the matching `close`.
fn skip_matched_pair_end(bytes: &[u8], mut index: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'`' => index = skip_backquote_end(bytes, index + 1)?,
            b'\'' => index = skip_single_quoted_end(bytes, index + 1)?,
            b'"' => index = skip_double_quoted_end(bytes, index + 1)?,
            b'$' if matches!(bytes.get(index + 1), Some(b'(') | Some(b'{')) => {
                let (nested_open, nested_close) = if bytes[index + 1] == b'(' {
                    (b'(', b')')
                } else {
                    (b'{', b'}')
                };
                index = skip_matched_pair_end(bytes, index + 2, nested_open, nested_close)?;
            }
            byte if byte == open => {
                depth += 1;
                index += 1;
            }
            byte if byte == close => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index += 1,
        }
    }
    None
}

/// Index just past the closing backquote of a span whose text starts
/// at `index`; a backslash protects the next byte
/// (skip_matched_pair's backq state, subst.c:2113-2120).
fn skip_backquote_end(bytes: &[u8], mut index: usize) -> Option<usize> {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'`' => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

/// Index just past the closing `'` of a span whose text starts at
/// `index`; single quotes admit no escapes (skip_single_quoted,
/// subst.c:1952).
fn skip_single_quoted_end(bytes: &[u8], mut index: usize) -> Option<usize> {
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

/// Index just past the closing `"` of a span whose text starts at
/// `index`. Inside double quotes a backslash protects the next byte
/// and backquote / `$(...)` / `${...}` constructs nest as units
/// (skip_double_quoted, subst.c:1981).
fn skip_double_quoted_end(bytes: &[u8], mut index: usize) -> Option<usize> {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'`' => index = skip_backquote_end(bytes, index + 1)?,
            b'"' => return Some(index + 1),
            b'$' if matches!(bytes.get(index + 1), Some(b'(') | Some(b'{')) => {
                let (nested_open, nested_close) = if bytes[index + 1] == b'(' {
                    (b'(', b')')
                } else {
                    (b'{', b'}')
                };
                index = skip_matched_pair_end(bytes, index + 2, nested_open, nested_close)?;
            }
            _ => index += 1,
        }
    }
    None
}

/// True when the assignment `=` is immediately followed by an unquoted `(` —
/// GNU parse.y:5648-5657 read_token_word peeks the character right after the
/// `=` that ends the assignment name (an `+=` consumes one extra `=`) and
/// only `peek_char == '('` starts parse_compound_assignment. An `=(` inside
/// a quoted RHS (`a='x=(1)'`, `a="x=(1)"`) is data and must not select the
/// verbatim compound path.
pub(super) fn assignment_rhs_opens_compound(word: &str) -> bool {
    let Some((_, rhs)) = word.split_once('=') else {
        return false;
    };
    let rhs = rhs.strip_prefix('=').unwrap_or(rhs);
    rhs.starts_with('(')
}

pub(super) fn is_brace_expansion(word: &str) -> bool {
    word.starts_with('{')
        && word.ends_with('}')
        && word.len() >= 3
        && !word.chars().any(char::is_whitespace)
        && (word[1..word.len() - 1].contains("..") || word.contains(','))
}

/// GNU syntax.h:29-30: `shell_meta_chars "()<>;&|"` and
/// `shell_break_chars "()<>;&| \t\n"` — `{` and `}` are NOT shell
/// metacharacters or word-break characters, so read_token_word (parse.y:5305)
/// collects them as ordinary word text. A brace only leaves the word stream
/// when the WHOLE token is the reserved `{`/`}` (CHECK_FOR_RESERVED_WORD,
/// parse.y:3168, exact STREQ in command position), which next_token's `{`/`}`
/// dispatch decides before any word scanning starts. Keeping `{}` out of this
/// set is what makes `$(echo a){b}`, `` `echo a`{b} `` and `ble/x:{s}/d` single
/// words like GNU (rubash#244).
pub(super) fn is_word_delimiter(ch: char) -> bool {
    " \t\r\n|&;<>()".contains(ch)
}

pub(super) fn assignment_value_is_quoted(raw: &str) -> bool {
    let Some((_, value)) = raw.split_once('=') else {
        return false;
    };

    // GNU parse.y:7104 parse_compound_assignment: a compound array assignment
    // (`x=(...)`) is NOT a quoted RHS. The `(` begins the compound body and
    // every quote inside it is an element-level quote, not the assignment's
    // quoting state. Returning true here would mark the whole compound body
    // as quoted, which suppresses the data-quote hoisting the compound
    // element tokenizer needs (unicode1.sub `[0x0022]=\"`).
    if value.starts_with('(') {
        return false;
    }

    // Quotes inside a `${...}` body belong to the expansion itself (GNU keeps
    // them for the expansion stage), not to the assignment's quoting state.
    let mut in_backtick = false;
    let mut escaped = false;
    let mut expansion_depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' && !in_single {
            escaped = true;
            continue;
        }

        if ch == '`' && !in_single {
            in_backtick = !in_backtick;
            continue;
        }

        if expansion_depth > 0 {
            match ch {
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '$' if !in_single && !in_double && chars.peek() == Some(&'{') => {
                    chars.next();
                    expansion_depth += 1;
                }
                '}' if !in_single && !in_double => {
                    expansion_depth -= 1;
                    if expansion_depth == 0 {
                        in_single = false;
                        in_double = false;
                    }
                }
                _ => {}
            }
            continue;
        }

        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next();
            expansion_depth = 1;
            in_single = false;
            in_double = false;
            continue;
        }

        if !in_backtick && matches!(ch, '"' | '\'') {
            return true;
        }
    }

    false
}

/// True when every character of an assignment word's right-hand side lies
/// inside single quotes (`x='...'`). GNU runs no expansion at all inside a
/// single-quoted span (subst.c copies a single-quoted region verbatim), so
/// such an RHS is literal data and must not be scanned for `$(...)` or
/// backticks by the assignment expander — otherwise `x='$(date)'` stores the
/// substitution output instead of the text `$(date)`.
///
/// Quote-aware tiling: `'a'$x`, `'a'"b"` and `'a'\'b'` are *not* fully single
/// quoted, because a character (or escape) sits outside the single-quoted
/// spans and would be expanded by GNU.
pub(super) fn assignment_rhs_is_fully_single_quoted(raw: &str) -> bool {
    let Some((_, rhs)) = raw.split_once('=') else {
        return false;
    };
    let mut in_single = false;
    let mut saw_span = false;
    for ch in rhs.chars() {
        if ch == '\'' {
            in_single = !in_single;
            saw_span = true;
            continue;
        }
        if !in_single {
            return false;
        }
    }
    saw_span && !in_single
}

/// Rewrite a wholly single-quoted assignment RHS into protected literal data:
/// drop the quote delimiters and carry `$`/backtick/`"`/`\` as the walker's
/// literal markers (\x1f / \x1a / \x18 / \x14), which the parameter-expansion
/// and storage layers restore on the way out. The `name=` prefix is copied
/// verbatim.
///
/// GNU parse.y:5305 read_token_word treats every byte inside a single-quoted
/// span as literal data and subst.c:4807 dequote_string removes only the
/// quote delimiters, so a `"` there is data. The `name=` prefix sits outside
/// the quotes, so the word is never fully single-quoted: a bare `"` would be
/// re-read as a quote delimiter by the expansion walker and silently dropped
/// (`echo a='x"y'` printed `a=xy`; GNU prints `a=x"y`). It therefore travels
/// as DATA_DQUOTE, the same carrier the normal dequote path emits for `"`
/// inside single quotes when the word has content outside them
/// (lexer/quotes.rs saw_outside_single arm).
///
/// A `\` needs the same carrier (rubash#301): GNU subst.c:11881-11886
/// (expand_word_internal case '\'') hands the whole span to add_quoted_string,
/// whose quote_string (subst.c:4773) CTLESC-protects every character, so the
/// span's backslashes are data — `alias path='echo -e ${PATH//:/\\n}'`
/// (mathiasbynens .aliases:148) stores `\\n` byte-for-byte. Rubash erases the
/// delimiters here, and a raw `\` left in the token is re-read as source-word
/// escape syntax by the embedded walker (embedded_mutations.rs `\\` arm
/// collapses the pair to one `\`), so `NAME='...'` argument words lost one
/// backslash of every `\\`. DATA_BACKSLASH is restored by the walker's tail
/// chain and materialize_expanded_command_word before argv/glob see the word.
pub(super) fn protect_fully_single_quoted_assignment(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars();
    for ch in chars.by_ref() {
        out.push(ch);
        if ch == '=' {
            break;
        }
    }
    for ch in chars {
        match ch {
            '\'' => {}
            '$' => out.push(DATA_DOLLAR),
            '`' => out.push(crate::executor::markers::DATA_BACKTICK),
            '"' => out.push(crate::executor::markers::DATA_DQUOTE),
            '\\' => out.push(crate::executor::markers::DATA_BACKSLASH),
            _ => out.push(ch),
        }
    }
    out
}

pub(super) fn mark_quoted_assignment_value(raw: &str, value: &str) -> String {
    let Some((name, rhs)) = value.split_once('=') else {
        return value.to_string();
    };
    let raw_rhs = raw.split_once('=').map(|(_, rhs)| rhs).unwrap_or_default();
    let rhs = if raw_rhs.starts_with('\"')
        && raw_rhs.ends_with('\"')
        && !raw_rhs.contains("$(")
        && !raw_rhs.contains('`')
    {
        // A `'` in the RHS's own text is data inside the double quotes, but
        // quotes inside a `${...}` unit belong to the substitution's own
        // grammar (`${x#'foo'}` pattern quoting, POSIX 2.6.2 /
        // parse.y:3877 parse_matched_pair) and must stay raw for the
        // parameter engine — mark only the word's own quotes (rubash#258
        // BUG_PSUBSQUOT: `v="${x#'foo'}"` stopped stripping the prefix).
        super::quotes::mark_data_squotes_around_substitutions(rhs).replace(
            crate::executor::markers::ANSI_C_QUOTE_MARKER_STR,
            crate::executor::markers::PROTECTED_ESCAPED_SQUOTE_STR,
        )
    } else {
        rhs.to_string()
    };

    format!(
        "{name}={}{rhs}",
        crate::executor::markers::QUOTED_WORD_VALUE_PREFIX_STR
    )
}

pub(super) fn quoted_literal_tilde(raw: &str, value: &str) -> bool {
    value.starts_with('~')
        && ((raw.starts_with('\'') && raw.ends_with('\''))
            || (raw.starts_with('"') && raw.ends_with('"'))
            // A backslash-quoted tilde is quoted exactly like '~'/"~":
            // GNU parse.y keeps the escaped `~` CTLESC-protected, so no
            // tilde pass may touch it (rubash#1416: `echo \~/x` printed
            // $HOME/x because dequoting erased the only evidence).
            || raw.starts_with("\\~"))
}

/// True when an assignment's raw RHS begins with a backslash-quoted tilde
/// (at the value start or right after a `:` segment boundary). Such an RHS
/// takes the quoted-value marker so the assignment tilde pass leaves it
/// literal, mirroring `assignment_value_is_quoted` for quote characters
/// (rubash#1416: `v=\~/x` must stay `\~/x` -> `~/x`).
pub(super) fn assignment_value_has_escaped_tilde(raw: &str) -> bool {
    let Some((_, value)) = raw.split_once('=') else {
        return false;
    };
    !value.starts_with('(') && (value.starts_with("\\~") || value.contains(":\\~"))
}
