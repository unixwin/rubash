use crate::executor::markers::{STORAGE_WORD_PREFIX};
mod array;
mod assoc;
mod words;

pub(in crate::builtins) use array::append_array_value;
pub(in crate::builtins) use array::{format_indexed_array_storage, indexed_array_entries};
pub(in crate::builtins) use assoc::append_assoc_value;
pub(in crate::builtins) use assoc::{format_assoc_storage, parse_assoc_words};
pub(super) use assoc::{quote_assoc_key, quote_assoc_storage_value};
pub(super) use words::{
    parse_array_tokens, split_indexed_tagged_token, split_storage_words, unquote_storage_value,
};

pub(super) fn parse_single_element_array(value: &str) -> Option<&str> {
    value.strip_prefix('(')?.strip_suffix(')')
}

pub(super) fn format_array_value(value: &str) -> String {
    if let Some(rendered) = value.strip_prefix(STORAGE_WORD_PREFIX) {
        return rendered.to_string();
    }

    let elements = parse_array_words(value);
    if elements.is_empty() {
        return format!("([0]={})", quote_array_element_value(value));
    }

    elements
        .iter()
        .enumerate()
        .map(|(index, value)| format!("[{index}]={}", quote_array_element_value(value)))
        .collect::<Vec<_>>()
        .join(" ")
        .pipe_parenthesized()
}

pub(super) fn format_assoc_value(value: &str, nbuckets: usize) -> String {
    let entries = parse_assoc_words(value);
    if entries.is_empty() {
        if value == "()" {
            return "()".to_string();
        }
        return format!("([0]={} )", quote_declare_value(value));
    }

    // print_assoc_assignment walks the hash table: bucket order with
    // head-insertion chains (hashlib.c). The general order helper replaces
    // the previous per-test hardcoded key sequences.
    let ordered = crate::executor::bash_assoc_order(&entries, nbuckets);
    let rendered = ordered
        .into_iter()
        .map(|(_, (key, entry_value))| {
            format!(
                "[{}]={}",
                quote_assoc_display_key(&key),
                quote_declare_display_value(&entry_value)
            )
        })
        .collect::<Vec<_>>();
    format!("({} )", rendered.join(" "))
}

pub(super) fn parse_array_words(value: &str) -> Vec<String> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return vec![value.to_string()];
    };
    inner.split_whitespace().map(str::to_string).collect()
}

pub(super) fn is_noassign_bash_array(name: &str) -> bool {
    let name = name.split_once('[').map(|(name, _)| name).unwrap_or(name);
    matches!(
        name,
        "BASH_ARGC" | "BASH_ARGV" | "BASH_LINENO" | "BASH_SOURCE" | "FUNCNAME"
    )
}
pub(super) fn eval_arith_value(value: &str) -> i128 {
    value
        .split('+')
        .map(|part| {
            let part = part.trim();
            // GNU expr.c: hex (0x...), octal (0...), and decimal literals
            // are all valid in array subscripts (arrayfunc.c:753
            // assign_compound_array_list calls evalexp, the full
            // arithmetic evaluator). The previous split-on-'+' parser
            // only handled decimal, so [0x0020] evaluated to 0 and
            // every hex-subscripted element landed at index 0
            // (unicode1.sub C_UTF_8 array).
            if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
                i128::from_str_radix(hex, 16).unwrap_or(0)
            } else if part.len() > 1
                && part.starts_with('0')
                && part.bytes().all(|b| b.is_ascii_digit())
            {
                i128::from_str_radix(part, 8).unwrap_or(0)
            } else {
                part.parse::<i128>().unwrap_or(0)
            }
        })
        .sum()
}

trait Parenthesized {
    fn pipe_parenthesized(self) -> String;
}

impl Parenthesized for String {
    fn pipe_parenthesized(self) -> String {
        format!("({self})")
    }
}

/// Output boundary: stored values arrive as transport text (C0 carriers,
/// PUA sentinels, E400 literal-char escapes). GNU prints the decoded
/// value (subst.c:4807 dequote_string → strtrans.c ansic_quote), so
/// decode to user-visible text before display quoting; the $'...' arm
/// decodes raw-byte markers internally already.
fn display_text(value: &str) -> String {
    crate::locale::decode_to_visible_text(value)
}

pub(super) fn quote_declare_value(value: &str) -> String {
    // setattr.def:528-531 (show_var_attributes):
    //   if (ansic_shouldquote (value_cell (var)))
    //     x = ansic_quote (value_cell (var), 0, (int *)0);
    //   else
    //     x = sh_double_quote (value_cell (var));
    // gnu_ansic_quote is the faithful port of strtrans.c::ansic_quote and
    // already carries the $'...' shell, so there is no separate private
    // renderer here. A previous version had its own quote_ansi_c copy whose
    // named-escape arm pushed the real character instead of the escape
    // spelling ('\n' -> push "\n" rather than "\\n"), so `n=$'a\nb'` was
    // printed with a literal newline where GNU prints $'a\nb'.
    // GNU's ansic_shouldquote tests the dequoted value; stored carrier
    // bytes (CTLESC protection pairs) are transport, not data, so decode
    // to visible text first or a quoted `']'` renders as $'\'\x11]'.
    let visible = display_text(value);
    if gnu_ansic_shouldquote(&visible) {
        return gnu_ansic_quote(&visible);
    }
    format!("\"{}\"", quote_double(&visible))
}

/// array.c array_to_assign element rule (964-968, same pair in
/// array_to_kvpair 911-915): $'...' for values holding non-printing
/// characters (ansic_shouldquote), sh_double_quote otherwise. Indexed
/// array element renders follow array.c, not the scalar setattr.def rule.
pub(super) fn quote_array_element_value(value: &str) -> String {
    let visible = display_text(value);
    if gnu_ansic_shouldquote(&visible) {
        return gnu_ansic_quote(&visible);
    }
    format!("\"{}\"", quote_double(&visible))
}

/// Storage roundtrip for element values: decode the full escape set
/// ansic_quote emits (named C escapes, escaped backslash/quote and
/// three-digit octal); other storage forms pass through
/// unquote_storage_value.
pub(super) fn decode_ansic_storage_value(value: &str) -> String {
    if let Some(inner) = value
        .strip_prefix("$'")
        .and_then(|value| value.strip_suffix('\''))
    {
        return decode_ansic_escapes(inner);
    }
    unquote_storage_value(value)
}

fn decode_ansic_escapes(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 1 < bytes.len() {
            let escape = bytes[index + 1];
            index += 2;
            match escape {
                b'E' => out.push(0x1b),
                b'a' => out.push(0x07),
                b'b' => out.push(0x08),
                b't' => out.push(0x09),
                b'n' => out.push(0x0a),
                b'v' => out.push(0x0b),
                b'f' => out.push(0x0c),
                b'r' => out.push(0x0d),
                b'\\' => out.push(b'\\'),
                b'\'' => out.push(b'\''),
                b'0'..=b'7' => {
                    let mut decoded = (escape - b'0') as u32;
                    let mut digits = 1;
                    while digits < 3 && index < bytes.len() && matches!(bytes[index], b'0'..=b'7') {
                        decoded = decoded * 8 + (bytes[index] - b'0') as u32;
                        digits += 1;
                        index += 1;
                    }
                    out.push(decoded as u8);
                }
                other => {
                    out.push(b'\\');
                    out.push(other);
                }
            }
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    // Carrier-range bytes (0x14..=0x1f) must reach readers pair-encoded like
    // scalar assignment storage (bytes_to_assignment_shell_text); a raw
    // control byte would alias IFS_GLUE/DATA_DQUOTE/QUOTED_WORD_PREFIX and
    // compare unequal to the same byte held in a scalar (unicode1.sub).
    crate::executor::substitution_metadata::bytes_to_assignment_shell_text(&out)
}

pub(super) fn quote_double(value: &str) -> String {
    let mut quoted = String::new();
    for ch in value.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            _ => quoted.push(ch),
        }
    }
    quoted
}

// ---- GNU declare -p display quoting (assoc.c assoc_to_assign) -------------

/// strtrans.c ansic_shouldquote (341-361): `$'...'` quoting is needed
/// when the string holds a non-printing byte. Delegates to the executor's
/// byte-stream implementation so raw-byte markers decode before the test
/// (a stored 0xA2 is a PUA wide char that `char::is_control` would pass).
fn gnu_ansic_shouldquote(value: &str) -> bool {
    crate::executor::ansic_shouldquote(value)
}

/// strtrans.c ansic_quote (230-308): the `$'...'` form over the raw byte
/// stream — named escapes, `\\`/`\'` escaped, printable bytes literal,
/// every other byte a three-digit octal escape.
fn gnu_ansic_quote(value: &str) -> String {
    crate::executor::ansic_quote(value)
}

/// shquote.c sh_contains_shell_metas: shell metacharacters force quoting of
/// a bare assoc key. `~` is special only at the start or after `=`/`:` and
/// `#` only at the start of the key.
fn gnu_sh_contains_shell_metas(value: &str) -> bool {
    let chars: Vec<char> = value.chars().collect();
    for (index, ch) in chars.iter().enumerate() {
        match ch {
            ' ' | '\t' | '\n' | '\'' | '"' | '\\' | '|' | '&' | ';' | '(' | ')' | '<' | '>'
            | '!' | '{' | '}' | '*' | '[' | '?' | ']' | '^' | '$' | '`' => return true,
            '~' => {
                if index == 0 || chars[index - 1] == '=' || chars[index - 1] == ':' {
                    return true;
                }
            }
            '#' => {
                if index == 0 {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// assoc.c assoc_to_assign key rule: `$'...'` for non-printing keys,
/// double quotes for keys with shell metas, double quotes for a bare `*`
/// or `@` key (ALL_ELEMENT_SUB), otherwise the bare key.
fn quote_assoc_display_key(key: &str) -> String {
    let key = display_text(key);
    if gnu_ansic_shouldquote(&key) {
        return gnu_ansic_quote(&key);
    }
    if gnu_sh_contains_shell_metas(&key) {
        return format!("\"{}\"", quote_double(&key));
    }
    if key.len() == 1 && matches!(key.as_str(), "*" | "@") {
        return format!("\"{key}\"");
    }
    key
}

/// assoc.c assoc_to_assign value rule (setattr.def:528 uses the same pair
/// for scalars): `$'...'` when the value has non-printing characters,
/// otherwise always double quotes.
fn quote_declare_display_value(value: &str) -> String {
    let visible = display_text(value);
    if gnu_ansic_shouldquote(&visible) {
        return gnu_ansic_quote(&visible);
    }
    format!("\"{}\"", quote_double(&visible))
}
