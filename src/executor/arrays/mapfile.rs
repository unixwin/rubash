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
