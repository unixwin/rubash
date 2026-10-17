pub(in crate::executor) fn split_mapfile_input(
    input: &str,
    delimiter: Option<u8>,
    trim_delimiter: bool,
) -> Vec<String> {
    use crate::executor::substitution_metadata::{bytes_to_shell_text, shell_text_to_raw_bytes};

    // GNU builtins/mapfile.def:177-209: the zgetline loop splits raw input
    // bytes on the delimiter and bind_array_element stores each line, with
    // no per-line re-encode. Decode once, split the raw bytes as ranges
    // (no per-byte push buffer), strip `-t` in the byte domain, and encode
    // each segment exactly once (subst.c raw-byte marker contract keeps
    // `-d $'\xff'` byte-exact).
    let delim = delimiter.unwrap_or(b'\n');
    // Fast path (rubash#680 slice 2): when the input is pure ASCII with
    // no carrier bytes and the delimiter is a plain ASCII byte, the
    // decode is the identity copy (no LITERAL/RAW_BYTE escapes exist in
    // ASCII) and the per-segment re-encode is the identity too (no
    // marker-zone chars exist in ASCII; carrier bytes {0x0c,0x11,0x13,
    // 0x14..=0x1f} are excluded above, and only they make
    // bytes_to_shell_text differ from its input). Splitting the text
    // directly then yields byte-identical values with one alloc per
    // segment instead of a whole-input encode plus a per-segment decode
    // scan. Anything else keeps the decode path below (rubash#117);
    // NUL-delimited input stays there by contract (delim != 0).
    if delim != 0
        && delim.is_ascii()
        && input
            .bytes()
            .all(|b| b.is_ascii() && !matches!(b, 0x0c | 0x11 | 0x13 | 0x14..=0x1f))
    {
        let strip = trim_delimiter;
        let mut values = Vec::new();
        values.reserve(std::cmp::min(input.len() / 8, 65_536) + 1);
        let mut start = 0;
        for (index, &byte) in input.as_bytes().iter().enumerate() {
            if byte != delim {
                continue;
            }
            let end = if strip { index } else { index + 1 };
            values.push(input[start..end].to_string());
            start = index + 1;
        }
        if start < input.len() {
            values.push(input[start..].to_string());
        }
        return values;
    }
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
