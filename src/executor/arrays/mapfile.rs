pub(in crate::executor) fn split_mapfile_input(
    input: &str,
    delimiter: Option<u8>,
    trim_delimiter: bool,
) -> Vec<String> {
    use crate::executor::substitution_metadata::{bytes_to_shell_text, shell_text_to_raw_bytes};

    // GNU builtins/mapfile.def:177-209 zgetline loop + bind_array_element:
    // raw bytes split on the delimiter, no re-encode.
    let delim = delimiter.unwrap_or(b'\n');
    // perf rubash#680 (input-side): split encoded input when the ASCII
    // delim round-trips as itself; any doubt keeps the decode path (rubash#117).
    if delim != 0
        && delim < 0x80
        && !crate::lexer::ansi::is_assignment_carrier_byte(u32::from(delim))
        && mapfile_input_is_plain(input)
    {
        if input.is_empty() {
            return Vec::new();
        }
        let mut values = Vec::new();
        values.reserve(std::cmp::min(input.len() / 8, 65_536) + 1);
        let mut start = 0;
        for (index, byte) in input.bytes().enumerate() {
            if byte != delim {
                continue;
            }
            let end = if trim_delimiter { index } else { index + 1 };
            values.push(input[start..end].to_string());
            start = index + 1;
        }
        if start < input.len() {
            values.push(input[start..].to_string());
        }
        return values;
    }
    // Decode once, split raw bytes as ranges, strip `-t` in the byte domain,
    // encode each segment once (subst.c marker contract keeps `-d $'\xff'`
    // byte-exact).
    let bytes = shell_text_to_raw_bytes(input);
    if bytes.is_empty() {
        return Vec::new();
    }
    let strip = trim_delimiter || delim == 0;
    let mut values = Vec::new();
    values.reserve(std::cmp::min(bytes.len() / 8, 65_536) + 1);
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte != delim {
            continue;
        }
        let end = if strip { index } else { index + 1 };
        values.push(bytes_to_shell_text(&bytes[start..end]));
        start = index + 1;
    }
    if start < bytes.len() {
        values.push(bytes_to_shell_text(&bytes[start..]));
    }
    values
}

/// Encoded-split whitelist: C0 controls (minus tab/LF/CR) ride marker
/// pairs/carriers; 0xEE leads U+E000..U+EFFF registry markers (markers.rs).
/// NUL stays on the decode path by design.
fn mapfile_input_is_plain(input: &str) -> bool {
    !input
        .bytes()
        .any(|b| matches!(b, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0xEE))
}
