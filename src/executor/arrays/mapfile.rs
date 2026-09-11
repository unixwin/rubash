pub(in crate::executor) fn split_mapfile_input(
    input: &str,
    delimiter: Option<u8>,
    trim_delimiter: bool,
) -> Vec<String> {
    use crate::executor::substitution_metadata::{bytes_to_shell_text, shell_text_to_raw_bytes};

    // GNU mapfile.c: delim is an unsigned char (single byte). Even delimiter
    // bytes >= 0x80 travel as U+E000 marker pairs in Rubash's Rust String
    // carrier; byte-exact split must operate on the decoded raw bytes and
    // re-encode each segment only once at the boundary (subst.c raw-byte
    // marker contract). This fixes mapfile -d $'\xff' where the previous
    // char-wise split compared the sentinel U+E000 alone and leaked the
    // payload marker as stray PUA data (mapfile.tests s=$'a\xffb\xffc\xff').
    let delim = delimiter.unwrap_or(b'\n');
    let bytes = shell_text_to_raw_bytes(input);
    let mut values = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    for &byte in &bytes {
        current.push(byte);
        if byte == delim {
            if trim_delimiter || delim == 0 {
                current.pop();
            }
            values.push(bytes_to_shell_text(&current));
            current.clear();
        }
    }
    if !current.is_empty() {
        values.push(bytes_to_shell_text(&current));
    }
    values
}
