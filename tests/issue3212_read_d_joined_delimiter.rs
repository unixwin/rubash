//! rubash#3212 — `read -d` where the delimiter byte is joined into a
//! valid UTF-8 sequence with following data bytes in the transport
//! text: the canonical storage encoder (`bytes_to_shell_text`) stores
//! a whole valid chunk as one literal character, so stream bytes
//! 0xC3 0xA9 arrive as the single character 'é' and the delimiter byte
//! 0xC3 exists in NO testable form (needle / single-byte marker /
//! literal) — every text serve (pipeline mirror, herestring and file
//! slurps, heredoc) read to EOF and the record was lost.
//!
//! GNU bash 5.3.20 oracle: the delimiter is the FIRST BYTE of the `-d`
//! argument (builtins/read.def:385 `delim = (unsigned char)*list_optarg`)
//! compared byte-by-byte against the raw stream (read.def:834
//! `(unsigned char)c == delim`). Expectations below were measured
//! against the oracle with script files under LC_ALL=C in BOTH the C
//! and C.UTF-8 locales (identical for every cell here) and are
//! byte-exact on stdout and exit status.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash-3212-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .env("LC_ALL", "C")
        .env("TMPDIR", &dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn read_d_joined_delimiter_pipe_loop() {
    // The issue's own shape: -d $'\303\251' (delimiter byte 0xC3),
    // stream a<C3><A9>b<C3><A9>. GNU splits at the 0xC3 byte inside
    // each 'é': records "a" and "\251b" (the leftover continuation
    // byte leads the second record).
    let (stdout, _, code) = run_script(
        "printf 'a\\303\\251b\\303\\251' | \
         while IFS= read -rd $'\\303\\251' w; do printf '<%q>\\n' \"$w\"; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<a>\n<$'\\251b'>\n");
}

#[test]
fn read_d_joined_delimiter_pipe_single_read() {
    let (stdout, _, code) = run_script(
        "printf 'a\\303\\251b\\303\\251' | { IFS= read -rd $'\\303\\251' x; \
         printf 'single=<%q> rc=%s\\n' \"$x\" \"$?\"; }\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "single=<a> rc=0\n");
}

#[test]
fn read_d_joined_delimiter_raw_mode_loop() {
    let (stdout, _, code) = run_script(
        "printf 'a\\303\\251b\\303\\251' | \
         while IFS= read -r -d $'\\303\\251' w; do printf '<%q>\\n' \"$w\"; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<a>\n<$'\\251b'>\n");
}

#[test]
fn read_d_joined_continuation_byte_delimiter_loop() {
    // Delimiter is the CONTINUATION byte 0xA9 itself: in a byte locale
    // GNU splits inside 'é', leaving the lead byte as record data.
    let (stdout, _, code) = run_script(
        "printf 'a\\303\\251b\\303\\251' | \
         while IFS= read -rd $'\\251' w; do printf '<%q>\\n' \"$w\"; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<$'a\\303'>\n<$'b\\303'>\n");
}

#[test]
fn read_d_joined_delimiter_herestring() {
    let (stdout, _, code) = run_script(
        "IFS= read -rd $'\\303\\251' x <<< $'a\\303\\251b'; \
         printf 'hs=<%q> rc=%s\\n' \"$x\" \"$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hs=<a> rc=0\n");
}

#[test]
fn read_d_joined_delimiter_heredoc() {
    let (stdout, _, code) = run_script(
        "IFS= read -rd $'\\303\\251' x <<EOF\na\u{e9}b\u{e9}\nEOF\n\
         printf 'hd=<%q> rc=%s\\n' \"$x\" \"$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hd=<a> rc=0\n");
}

#[test]
fn read_d_joined_delimiter_file_loop() {
    let (stdout, _, code) = run_script(
        "printf 'a\\303\\251b\\303\\251' > \"$TMPDIR/delim.bin\"\n\
         while IFS= read -rd $'\\303\\251' w; do printf '<%q>\\n' \"$w\"; done < \"$TMPDIR/delim.bin\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<a>\n<$'\\251b'>\n");
}

#[test]
fn read_d_joined_char_counts_bytes_toward_n_limit() {
    // -n budget in a byte locale is a BYTE budget (read.def: the
    // single-byte-locale count is per byte): 'a' plus the lead byte
    // 0xC3 of 'é' spends the 2-byte budget, cutting the character in
    // half — the dangling lead byte stays as a raw byte.
    let (stdout, _, code) = run_script(
        "printf 'a\\303\\251b' | { IFS= read -n 2 -rd $'\\377' x; \
         printf 'n=<%q> rc=%s\\n' \"$x\" \"$?\"; }\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=<$'a\\303'> rc=0\n");
}

#[test]
fn read_d_joined_escaped_delimiter_byte_is_data() {
    // Without -r, a backslash-escaped delimiter byte is record data:
    // the first 0xC3 is escaped, the second terminates the record.
    let (stdout, _, code) = run_script(
        "printf 'a\\\\\\303\\251b\\303\\251' | { IFS= read -d $'\\303\\251' x; \
         printf 'esc=<%q> rc=%s\\n' \"$x\" \"$?\"; }\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "esc=<$'a\\303\\251b'> rc=0\n");
}

#[test]
fn read_d_joined_char_undivided_when_delimiter_absent_control() {
    // Control: no delimiter byte anywhere — the joined character is
    // served whole as record data and the read reports EOF.
    let (stdout, _, code) = run_script(
        "printf 'a\\303\\251b' | { IFS= read -rd $'\\377' x; \
         printf 'nd=<%q> rc=%s\\n' \"$x\" \"$?\"; }\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "nd=<$'a\\303\\251b'> rc=1\n");
}
