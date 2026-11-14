use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX_STR;

pub(in crate::executor) fn read_array_storage(values: &[String]) -> String {
    let rendered = values
        .iter()
        .enumerate()
        .map(|(index, value)| format!("[{index}]={}", render_read_array_element(value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{STORAGE_WORD_PREFIX_STR}({rendered})")
}
pub(in crate::executor) fn render_read_array_element(value: &str) -> String {
    if value.contains(['\n', '\r']) {
        let mut rendered = String::from("$'");
        for ch in value.chars() {
            match ch {
                '\n' => rendered.push_str("\\n"),
                '\r' => rendered.push_str("\\r"),
                '\\' => rendered.push_str("\\\\"),
                '\'' => rendered.push_str("\\'"),
                other => rendered.push(other),
            }
        }
        rendered.push('\'');
        return rendered;
    }

    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Logical units of shell text: (byte_start, byte_end, decoded char).
/// Raw-byte marker pairs (sentinel + marker char) decode to the byte they
/// carry as a Latin-1 codepoint, so carrier bytes such as  compare
/// correctly against IFS (GNU read.def compares the delimiter/IFS byte).
fn logical_units(text: &str) -> Vec<(usize, usize, char)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut units = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let (start, ch) = chars[i];
        if ch as u32 == RAW_BYTE_MARKER_ESCAPE {
            if let Some(&(next_start, next)) = chars.get(i + 1) {
                let code = next as u32;
                if (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&code) {
                    units.push((
                        start,
                        next_start + next.len_utf8(),
                        char::from((code - RAW_BYTE_MARKER_FIRST) as u8),
                    ));
                    i += 2;
                    continue;
                }
            }
        }
        units.push((start, start + ch.len_utf8(), ch));
        i += 1;
    }
    units
}

fn logical_chars(text: &str) -> Vec<char> {
    logical_units(text)
        .into_iter()
        .map(|(_, _, ch)| ch)
        .collect()
}

/// Single-byte locale byte domain for read splitting (rubash#2819).
///
/// GNU read.def hands the line to list_string (subst.c:3119), whose
/// separator membership — string_extract_verbatim, subst.c:1148 — is
/// per byte when MB_CUR_MAX == 1: a multibyte IFS character separates
/// on each of its bytes. The read machinery above works on logical
/// units (chars, marker pairs decoded); in byte mode the line and IFS
/// are first rewritten so every byte is exactly one logical unit
/// (bytes >= 0x80 become raw-byte marker pairs, which logical_units
/// decodes to the byte as a Latin-1 codepoint), so the existing unit
/// walk IS the byte walk, and each produced value is normalized back
/// to storage text (a field whose bytes again form valid UTF-8
/// re-forms its characters; a byte split out of a multibyte character
/// stays a raw-byte marker). Pure-ASCII input never needs the rewrite:
/// char and byte domains coincide there. Returns None outside byte
/// mode (multibyte locale, or nothing non-ASCII involved).
fn read_byte_domain(line: &str, ifs: &str) -> Option<(String, String)> {
    if crate::locale::is_multi_byte_active() {
        return None;
    }
    let non_ascii = |text: &str| text.chars().any(|ch| !ch.is_ascii());
    if !non_ascii(line) && !non_ascii(ifs) {
        return None;
    }
    Some((byte_domain_text(line), byte_domain_text(ifs)))
}

fn byte_domain_text(text: &str) -> String {
    use crate::executor::substitution_metadata::{encode_raw_byte_marker, shell_text_to_raw_bytes};
    shell_text_to_raw_bytes(text)
        .iter()
        .map(|byte| {
            if *byte < 0x80 {
                (*byte as char).to_string()
            } else {
                encode_raw_byte_marker(*byte)
            }
        })
        .collect()
}

fn byte_domain_value(text: &str) -> String {
    use crate::executor::substitution_metadata::{encode_raw_byte_marker, shell_text_to_raw_bytes};
    let bytes = shell_text_to_raw_bytes(text);
    let mut output = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        let len = if first < 0x80 {
            1
        } else if first & 0xE0 == 0xC0 {
            2
        } else if first & 0xF0 == 0xE0 {
            3
        } else if first & 0xF8 == 0xF0 {
            4
        } else {
            1
        };
        let end = (index + len).min(bytes.len());
        if end - index == len {
            if let Ok(valid) = std::str::from_utf8(&bytes[index..end]) {
                output.push_str(valid);
                index = end;
                continue;
            }
        }
        if first < 0x80 {
            output.push(char::from(first));
        } else {
            output.push_str(&encode_raw_byte_marker(first));
        }
        index += 1;
    }
    output
}

/// Byte length of `text`'s first `unit_count` logical units.
fn logical_prefix_len(text: &str, units: &[(usize, usize, char)], unit_count: usize) -> usize {
    units
        .get(unit_count)
        .map(|(start, _, _)| *start)
        .unwrap_or(text.len())
}

fn ifs_whitespace_logical(ch: char, ifs: &[char]) -> bool {
    ch.is_whitespace() && ifs.contains(&ch)
}

fn ifs_whitespace(ch: char, ifs: &str) -> bool {
    ifs_whitespace_logical(ch, &logical_chars(ifs))
}

/// Trim trailing IFS whitespace, comparing decoded logical characters.
fn trim_trailing_ifs_ws<'a>(text: &'a str, ifs: &[char]) -> &'a str {
    let units = logical_units(text);
    let mut end = units.len();
    while end > 0 && ifs_whitespace_logical(units[end - 1].2, ifs) {
        end -= 1;
    }
    &text[..logical_prefix_len(text, &units, end)]
}

/// Split a line into read fields using IFS, returning the byte ranges of each
/// field in the original line. Bash keeps trailing empty fields produced by a
/// trailing IFS delimiter (unlike word-splitting for expansion), so every
/// delimiter boundary becomes a field. Only a leading run of IFS *whitespace*
/// is elided (it does not create a leading empty field).
///
/// Ranges are byte offsets into `line` and always land on UTF-8 character
/// boundaries, so callers can slice `line` directly without panicking on
/// multi-byte characters (GNU read splits at multibyte character boundaries,
/// mirroring lib/sh/stringlib.c / subst.c).
pub(in crate::executor) fn split_read_field_ranges(
    line: &str,
    ifs: &str,
    interpret_backslashes: bool,
) -> Vec<(usize, usize)> {
    // Track each logical unit's byte offset so the returned ranges slice
    // `line` at UTF-8/marker boundaries instead of splitting multi-byte
    // sequences mid-character (GNU read splits at multibyte boundaries).
    // Units decode raw-byte marker pairs: carrier bytes like  arrive in
    // both the input text and the IFS value as markers and must still
    // compare as that byte.
    let units = logical_units(line);
    let ifs_chars = logical_chars(ifs);
    let n = units.len();
    let line_len = line.len();
    if ifs.is_empty() {
        // Empty IFS: the whole line is one field — except an empty
        // line, which has no field at all (GNU subst.c:3119
        // list_string returns NULL for an empty string, so
        // `IFS='' read -a A` on an empty line assigns an empty array).
        return if line.is_empty() {
            Vec::new()
        } else {
            vec![(0, line_len)]
        };
    }

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut i = 0usize;
    let mut ended_at_delimiter = false;
    // Skip a leading run of IFS whitespace (does not create a leading field).
    while i < n && ifs_whitespace_logical(units[i].2, &ifs_chars) {
        i += 1;
    }
    while i < n {
        let start = units[i].0;
        // A field runs until the next IFS character. IFS whitespace inside a
        // field is kept literally; a non-whitespace IFS delimiter ends it.
        // An escaped character (backslash + any char, including IFS chars) is
        // literal data and belongs to the field (GNU read.def / subst.c: read
        // honors backslash escapes before IFS splitting when -r is absent).
        // A3-F3: escape recognition must precede delimiter recognition,
        // exactly as GNU read.def:823-833 consumes a backslash as an escape
        // introducer (CTLESC + pass_next) before testing the delimiter.
        // Testing IFS membership first made a backslash that IS in IFS
        // (`IFS='\'`) end the field, instead of escaping the next char.
        while i < n {
            if interpret_backslashes && units[i].2 == '\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if ifs_chars.contains(&units[i].2) {
                break;
            }
            i += 1;
        }
        let end = if i < n { units[i].0 } else { line_len };
        ranges.push((start, end));
        if i >= n {
            ended_at_delimiter = false;
            break;
        }
        // Consume the delimiter and any following IFS whitespace. A non-whitespace
        // IFS delimiter ends exactly one field; trailing IFS whitespace is skipped so
        // it does not spawn an extra empty field.
        if ifs_whitespace_logical(units[i].2, &ifs_chars) {
            ended_at_delimiter = true;
            while i < n && ifs_whitespace_logical(units[i].2, &ifs_chars) {
                i += 1;
            }
            // An IFS whitespace run adjacent to a non-whitespace delimiter forms
            // one delimiter sequence; do not expose the latter as an empty field.
            if i < n
                && ifs_chars.contains(&units[i].2)
                && !ifs_whitespace_logical(units[i].2, &ifs_chars)
            {
                i += 1;
                while i < n && ifs_whitespace_logical(units[i].2, &ifs_chars) {
                    i += 1;
                }
            }
        } else {
            ended_at_delimiter = true;
            i += 1;
            while i < n && ifs_whitespace_logical(units[i].2, &ifs_chars) {
                i += 1;
            }
        }
    }
    // A final IFS delimiter creates a trailing empty field which bash drops
    // (mirroring word-splitting's omission of a trailing empty field).
    if ended_at_delimiter {
        ranges.push((line_len, line_len));
        if let Some((start, end)) = ranges.last().copied() {
            if start == end {
                ranges.pop();
            }
        }
    }
    ranges
}
/// Trim trailing IFS whitespace that is not backslash-escaped. A trailing
/// whitespace produced by an escape pair (`b\\ `) is literal data and must
/// survive (GNU read keeps it in the assigned field).
fn trim_trailing_unescaped_ifs(raw: &str, ifs: &str) -> String {
    let ifs_chars = logical_chars(ifs);
    let units = logical_units(raw);
    let mut end = units.len();
    while end > 0 && ifs_whitespace_logical(units[end - 1].2, &ifs_chars) {
        let mut backslashes = 0usize;
        let mut j = end - 1;
        while j > 0 && units[j - 1].2 == '\\' {
            backslashes += 1;
            j -= 1;
        }
        if backslashes % 2 == 1 {
            break;
        }
        end -= 1;
    }
    raw[..logical_prefix_len(raw, &units, end)].to_string()
}

fn field_value(
    line: &str,
    range: (usize, usize),
    ifs: &str,
    interpret_backslashes: bool,
) -> String {
    let raw: String = line[range.0..range.1].chars().collect();
    if !interpret_backslashes {
        return trim_trailing_ifs_ws(&raw, &logical_chars(ifs)).to_string();
    }
    // Unescape only after trimming trailing IFS whitespace that is not
    // itself escaped; unescaping first would turn an escaped trailing
    // space into plain IFS whitespace and wrongly strip it.
    unescape_read_backslashes(&trim_trailing_unescaped_ifs(&raw, ifs))
}

/// Field values for `read -a` without `-r` (GNU read.def:972-979):
/// `list_string` splits the CTLESC-protected line at IFS delimiters and
/// `dequote_list` then strips the escape protection, so each element keeps
/// the escaped characters joined and drops the backslash. Rubash's
/// `split_read_field_ranges(line, ifs, true)` already performs the
/// escape-aware split; this maps the ranges through the scalar
/// `field_value` helper, which unescapes backslash pairs
/// (`unescape_read_backslashes`) instead of slicing the raw line.
pub(in crate::executor) fn read_array_field_values(line: &str, ifs: &str) -> Vec<String> {
    // rubash#2819: single-byte locale — split in the byte domain (see
    // read_byte_domain); the ranges the impl computes then index the
    // rewritten line, never the caller's.
    if let Some((domain_line, domain_ifs)) = read_byte_domain(line, ifs) {
        return read_array_field_values_impl(&domain_line, &domain_ifs)
            .into_iter()
            .map(|value| byte_domain_value(&value))
            .collect();
    }
    read_array_field_values_impl(line, ifs)
}

fn read_array_field_values_impl(line: &str, ifs: &str) -> Vec<String> {
    split_read_field_ranges(line, ifs, true)
        .into_iter()
        .map(|range| field_value(line, range, ifs, true))
        .collect()
}

/// Field values for `read -r -a` (raw mode): the elements are the raw
/// field slices — no unescaping — but they must be produced at VALUE
/// level, not by slicing the caller's line with the ranges of
/// `split_read_field_ranges`. In a single-byte locale the split runs
/// in the byte domain (rubash#2819's `read_byte_domain`, as the scalar
/// and non-raw array producers do): there a multibyte IFS character
/// is one delimiter PER BYTE (GNU subst.c list_string separator
/// membership is per byte when MB_CUR_MAX == 1), the byte ranges can
/// fall inside a multibyte character of the original line, and
/// slicing it cannot express the empty fields between the two
/// delimiter bytes (rubash#3191: IFS 'é', line 'xéyéz' — GNU n=5
/// <x><><y><><z>, the range-slicing producer gave n=3). Outside the
/// byte domain this is exactly the historical range slicing.
pub(in crate::executor) fn read_array_raw_values(line: &str, ifs: &str) -> Vec<String> {
    if let Some((domain_line, domain_ifs)) = read_byte_domain(line, ifs) {
        return split_read_field_ranges(&domain_line, &domain_ifs, false)
            .into_iter()
            .map(|(start, end)| byte_domain_value(&domain_line[start..end]))
            .collect();
    }
    split_read_field_ranges(line, ifs, false)
        .into_iter()
        .map(|(start, end)| line[start..end].to_string())
        .collect()
}

/// First-field extraction for the last (or only) `read` variable,
/// mirroring GNU subst.c:3268 get_word_from_string: scan to the first
/// unescaped IFS character (an escape pair is data when backslashes
/// are interpreted), then consume the delimiter sequence — one
/// delimiter, following IFS whitespace, and, when the first delimiter
/// was IFS whitespace, one adjacent non-whitespace delimiter plus its
/// following IFS whitespace. Returns the field text when that
/// consumption exhausts the line (including the no-delimiter case,
/// where the field is the whole line), or None when input remains
/// and the caller must apply the remainder rule instead.
fn read_last_var_first_field(
    lead: &str,
    ifs_chars: &[char],
    interpret_backslashes: bool,
) -> Option<String> {
    let units = logical_units(lead);
    let n = units.len();
    let mut i = 0usize;
    while i < n {
        let ch = units[i].2;
        if interpret_backslashes && ch == '\\' && i + 1 < n {
            i += 2;
            continue;
        }
        if ifs_chars.contains(&ch) {
            break;
        }
        i += 1;
    }
    let field_end = i;
    if i >= n {
        return Some(lead.to_string());
    }
    let first_sep_ws = ifs_whitespace_logical(units[i].2, ifs_chars);
    i += 1;
    while i < n && ifs_whitespace_logical(units[i].2, ifs_chars) {
        i += 1;
    }
    if first_sep_ws
        && i < n
        && ifs_chars.contains(&units[i].2)
        && !ifs_whitespace_logical(units[i].2, ifs_chars)
    {
        i += 1;
        while i < n && ifs_whitespace_logical(units[i].2, ifs_chars) {
            i += 1;
        }
    }
    if i >= n {
        Some(lead[..logical_prefix_len(lead, &units, field_end)].to_string())
    } else {
        None
    }
}

pub(in crate::executor) fn read_scalar_fields(
    line: &str,
    names_len: usize,
    ifs: &str,
) -> Vec<String> {
    read_scalar_fields_internal(line, names_len, ifs, false)
}

pub(in crate::executor) fn read_scalar_fields_with_backslashes(
    line: &str,
    names_len: usize,
    ifs: &str,
) -> Vec<String> {
    read_scalar_fields_internal(line, names_len, ifs, true)
}

fn read_scalar_fields_internal(
    line: &str,
    names_len: usize,
    ifs: &str,
    interpret_backslashes: bool,
) -> Vec<String> {
    // rubash#2819: single-byte locale — split in the byte domain (see
    // read_byte_domain). Every range/slice inside the impl then refers
    // to the rewritten line, and the returned values are normalized
    // back to storage text.
    if let Some((domain_line, domain_ifs)) = read_byte_domain(line, ifs) {
        return read_scalar_fields_impl(
            &domain_line,
            names_len,
            &domain_ifs,
            interpret_backslashes,
        )
        .into_iter()
        .map(|value| byte_domain_value(&value))
        .collect();
    }
    read_scalar_fields_impl(line, names_len, ifs, interpret_backslashes)
}

fn read_scalar_fields_impl(
    line: &str,
    names_len: usize,
    ifs: &str,
    interpret_backslashes: bool,
) -> Vec<String> {
    if names_len == 0 {
        return Vec::new();
    }
    if names_len == 1 {
        // Trim leading IFS whitespace on the raw line (an escaped leading
        // whitespace starts with a backslash and is therefore not trimmed),
        // then an escape-aware trailing trim, then unescape.
        let ifs_chars = logical_chars(ifs);
        let lead_units = logical_units(line);
        let mut lead_start = 0usize;
        while lead_start < lead_units.len()
            && ifs_whitespace_logical(lead_units[lead_start].2, &ifs_chars)
        {
            lead_start += 1;
        }
        let lead = &line[lead_units
            .get(lead_start)
            .map(|unit| unit.0)
            .unwrap_or(line.len())..];
        // GNU read.def:1113-1126 (last-variable branch): extract the
        // first field with get_word_from_string (subst.c:3268). If that
        // extraction exhausts the line, the variable binds the FIELD
        // ITSELF, not the remainder — so a lone trailing delimiter is
        // dropped (`IFS=a read x` on `ba` -> `b`, rubash#1716) and an
        // escaped trailing IFS whitespace survives as field data
        // (`a\ ` -> `a `). Only when more input remains does the
        // remainder rule below apply.
        if let Some(field) = read_last_var_first_field(lead, &ifs_chars, interpret_backslashes) {
            let value = if interpret_backslashes {
                unescape_read_backslashes(&field)
            } else {
                field
            };
            return vec![value];
        }
        // More input remains after the first field: the name takes the
        // whole remainder. GNU read.def binds it through the
        // last-variable branch whose strip_trailing_ifs_whitespace
        // walk is escape-blind at the line level, so a trailing escaped
        // space is dropped along with plain IFS whitespace (read.tests
        // line 19: ` x  y\ \` -> x = `x  y`).
        let tail = trim_trailing_ifs_ws(lead, &ifs_chars).to_string();
        let tail = if interpret_backslashes {
            // The escape-blind strip can leave a dangling backslash when
            // the character it escaped was itself stripped as IFS
            // whitespace — drop that one introducer. Only an ODD trailing
            // run dangles: an even run is complete `\\` pairs, i.e. real
            // record data that unescape folds to literal backslashes
            // (rubash#1300: `printf 'a\\\\:b:c' | read -d: x` -> `a\`).
            let run = tail.bytes().rev().take_while(|&b| b == b'\\').count();
            if run % 2 == 1 {
                tail.strip_suffix('\\').map(str::to_string).unwrap_or(tail)
            } else {
                tail
            }
        } else {
            tail
        };
        let value = if interpret_backslashes {
            unescape_read_backslashes(&tail)
        } else {
            tail
        };
        return vec![value];
    }
    if ifs.is_empty() {
        let mut fields = vec![if interpret_backslashes {
            unescape_read_backslashes(line)
        } else {
            line.to_string()
        }];
        while fields.len() < names_len {
            fields.push(String::new());
        }
        return fields;
    }

    let ranges = split_read_field_ranges(line, ifs, interpret_backslashes);
    let field_count = ranges.len();

    // Assign the first names_len-1 names to the first fields directly.
    let mut fields: Vec<String> = Vec::with_capacity(names_len);
    for index in 0..names_len.saturating_sub(1) {
        let value = ranges
            .get(index)
            .map(|range| field_value(line, *range, ifs, interpret_backslashes))
            .unwrap_or_default();
        fields.push(value);
    }

    // The last name receives the remainder. When there are at least as many
    // fields as names, it gets the final field value. When there are *more*
    // fields than names, bash joins the remaining fields with their delimiters
    // by assigning the raw line tail from the start of the last named field.
    let last_value = if field_count >= names_len {
        let range = ranges[names_len - 1];
        if field_count == names_len {
            field_value(line, range, ifs, interpret_backslashes)
        } else {
            let tail: String = line[range.0..].chars().collect();
            let tail = if interpret_backslashes {
                unescape_read_backslashes(&trim_trailing_unescaped_ifs(&tail, ifs))
            } else {
                tail
            };
            trim_trailing_ifs_ws(&tail, &logical_chars(ifs)).to_string()
        }
    } else {
        String::new()
    };
    fields.push(last_value);

    while fields.len() < names_len {
        fields.push(String::new());
    }
    fields
}

pub(crate) fn mark_env_name(
    env_vars: &mut crate::shell::var_table::VarTable,
    key: &str,
    name: &str,
) {
    env_vars.mark_name(key, name);
}
