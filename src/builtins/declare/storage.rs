mod array;
mod assoc;
mod glob;
mod words;

pub(super) use array::{append_array_value, format_indexed_array_storage, indexed_array_entries};
pub(super) use assoc::{append_assoc_value, format_assoc_storage, parse_assoc_words};
pub(super) use words::{parse_array_tokens, split_storage_words, unquote_storage_value};

pub(super) fn parse_single_element_array(value: &str) -> Option<&str> {
    value.strip_prefix('(')?.strip_suffix(')')
}

pub(super) fn format_array_value(value: &str) -> String {
    if let Some(rendered) = value.strip_prefix('\x1d') {
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

pub(super) fn format_assoc_value(value: &str) -> String {
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
    let ordered = crate::executor::bash_assoc_order(&entries);
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
        .map(|part| part.trim().parse::<i128>().unwrap_or(0))
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

pub(super) fn quote_declare_value(value: &str) -> String {
    if value.contains(['\n', '\r', '\'']) {
        return format!("$'{}'", quote_ansi_c(value));
    }
    format!("\"{}\"", quote_double(value))
}


/// array.c array_to_assign element rule (964-968, same pair in
/// array_to_kvpair 911-915): $'...' for values holding non-printing
/// characters (ansic_shouldquote), sh_double_quote otherwise. Indexed
/// array element renders follow array.c, not the scalar setattr.def rule.
pub(super) fn quote_array_element_value(value: &str) -> String {
    if gnu_ansic_shouldquote(value) {
        return gnu_ansic_quote(value);
    }
    format!("\"{}\"", quote_double(value))
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
    String::from_utf8_lossy(&out).into_owned()
}

fn quote_ansi_c(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\'', "\\'")
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

/// strtrans.c ansic_shouldquote: `$'...'` quoting is needed when the string
/// contains a non-printing character. With a UTF-8 locale, printable
/// non-ASCII characters stay literal (ansic_wshouldquote passes them).
fn gnu_ansic_shouldquote(value: &str) -> bool {
    value.chars().any(|ch| ch.is_control())
}

/// strtrans.c ansic_quote: render the `$'...'` form. Named escapes for the
/// C specials, `\\` and `\'` verbatim, other non-printing characters as
/// three-digit octal escapes.
fn gnu_ansic_quote(value: &str) -> String {
    let mut out = String::from("$'");
    for ch in value.chars() {
        match ch {
            '\u{1b}' => out.push_str("\\E"),
            '\u{7}' => out.push_str("\\a"),
            '\u{b}' => out.push_str("\\v"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            c if !c.is_control() => out.push(c),
            c => out.push_str(&format!("\\{:03o}", c as u32)),
        }
    }
    out.push('\'');
    out
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
    if gnu_ansic_shouldquote(key) {
        return gnu_ansic_quote(key);
    }
    if gnu_sh_contains_shell_metas(key) {
        return format!("\"{}\"", quote_double(key));
    }
    if key.len() == 1 && matches!(key, "*" | "@") {
        return format!("\"{key}\"");
    }
    key.to_string()
}

/// assoc.c assoc_to_assign value rule (setattr.def:528 uses the same pair
/// for scalars): `$'...'` when the value has non-printing characters,
/// otherwise always double quotes.
fn quote_declare_display_value(value: &str) -> String {
    if gnu_ansic_shouldquote(value) {
        return gnu_ansic_quote(value);
    }
    format!("\"{}\"", quote_double(value))
}
