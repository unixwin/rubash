//! rubash#2662 — `read -d` with a high-byte (>= 0x80) delimiter read
//! nothing over the buffered (FUNCTION_STDIN) transport: the record
//! scan tested only the delimiter's multibyte needle (the marker pairs
//! of the delimiter char's UTF-8 encoding) and the literal char, never
//! the single-byte marker form the raw stream byte actually travels
//! as — so the delimiter never matched, the first read reported EOF,
//! and the loop body never ran.
//!
//! GNU bash 5.3.20 oracle (builtins/read.def: the delimiter is one
//! byte/character compared against the raw input stream; in a byte
//! locale `-d $'\200'` is the single byte 0x80). Expectations below
//! were measured against the oracle with script files under LC_ALL=C
//! and are byte-exact on stdout and exit status.
//!
//! Red on the base tree: every cell whose record is served from text
//! (pipeline mirror, herestring slurp) with a U+0080..=U+00FF
//! delimiter. Green controls on the base tree: the NUL-delimiter loop
//! (delimiter excluded by the readers), the regular-file redirect
//! (byte-wise reader stops at the byte itself), and the 0x1b carrier
//! delimiter (its tagged marker form was already a tested form).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash-2662-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .env("LC_ALL", "C")
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
fn read_d_high_byte_delimiter_loop_over_pipe() {
    // The issue's own shape (read1.sub): records split at raw 0x80,
    // invalid-UTF-8 payload bytes survive, trailing partial record is
    // still delivered.
    let (stdout, _, code) = run_script(
        "printf '%b\\200' winter 'spring\\0375' summer automn | \
         while IFS= read -rd $'\\200' season; do printf '<%q>\\n' \"$season\"; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<winter>\n<$'spring\\375'>\n<summer>\n<automn>\n");
}

#[test]
fn read_d_high_byte_delimiter_single_read_in_group() {
    let (stdout, _, code) = run_script(
        "printf 'a\\200b\\200' | { IFS= read -rd $'\\200' x; \
         printf 'single=<%q> rc=%s\\n' \"$x\" \"$?\"; }\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "single=<a> rc=0\n");
}

#[test]
fn read_d_high_byte_delimiter_raw_mode_loop() {
    let (stdout, _, code) = run_script(
        "printf 'a\\200b\\200' | \
         while IFS= read -r -d $'\\200' w; do printf '<%q>\\n' \"$w\"; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<a>\n<b>\n");
}

#[test]
fn read_d_high_byte_delimiter_ff_loop() {
    let (stdout, _, code) = run_script(
        "printf 'a\\377b\\377' | \
         while IFS= read -rd $'\\377' w; do printf '<%q>\\n' \"$w\"; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<a>\n<b>\n");
}

#[test]
fn read_d_high_byte_delimiter_array_over_pipe() {
    let (stdout, _, code) = run_script(
        "printf 'a\\200b\\200' | { IFS= read -rd $'\\200' -a arr; \
         printf 'arr=<%q|%q> rc=%s\\n' \"${arr[0]}\" \"${arr[1]}\" \"$?\"; }\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "arr=<a|''> rc=0\n");
}

#[test]
fn read_d_high_byte_delimiter_herestring() {
    // Slurp transport: the record split and the EOF accounting must
    // both see the single-byte form, or the value keeps the delimiter
    // and the read reports EOF.
    let (stdout, _, code) = run_script(
        "IFS= read -rd $'\\200' x <<< $'a\\200b'; \
         printf 'hs=<%q> rc=%s\\n' \"$x\" \"$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hs=<a> rc=0\n");
}

#[test]
fn read_d_high_byte_delimiter_char_limit() {
    // -n budget: the delimiter ends the record before the budget is
    // spent and is not itself consumed as a budget unit of data.
    let (stdout, _, code) = run_script(
        "printf 'ab\\200cd\\200' | { IFS= read -n 3 -rd $'\\200' x; \
         printf 'n=<%q> rc=%s\\n' \"$x\" \"$?\"; }\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=<ab> rc=0\n");
}

#[test]
fn read_d_first_byte_of_multibyte_arg_in_byte_locale() {
    // Under LC_ALL=C the -d argument's raw bytes are C3 A9; GNU's
    // delimiter is the first byte 0xC3 alone, so the A9 byte is data.
    let (stdout, _, code) = run_script(
        "printf 'a\\303\\251b\\303\\251' | \
         while IFS= read -rd $'\\303\\251' w; do printf '<%q>\\n' \"$w\"; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<a>\n<$'\\251b'>\n");
}

#[test]
fn read_d_escaped_high_byte_is_data() {
    // Without -r, a backslash-escaped delimiter byte is record data,
    // never the terminator (read.def escape state, rubash#1300's
    // discipline applied to the single-byte form).
    let (stdout, _, code) = run_script(
        "printf 'a\\\\\\200b\\200' | { IFS= read -d $'\\200' x; \
         printf 'esc=<%q> rc=%s\\n' \"$x\" \"$?\"; }\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "esc=<$'a\\200b'> rc=0\n");
}

#[test]
fn read_d_nul_delimiter_loop_control() {
    let (stdout, _, code) = run_script(
        "printf 'A\\0B\\315\\0C\\226\\0' | \
         while IFS= read -rd '' f; do printf '<%q>\\n' \"$f\"; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<A>\n<$'B\\315'>\n<$'C\\226'>\n");
}

#[test]
fn read_d_high_byte_delimiter_file_redirect_control() {
    // The byte-wise reader stops at the delimiter byte itself; this
    // transport was already correct on the base tree.
    let (stdout, _, code) = run_script(
        "printf 'a\\200b\\200' > \"$TMPDIR/delim.bin\"\n\
         { IFS= read -rd $'\\200' y; printf 'file=<%q> rc=%s\\n' \"$y\" \"$?\"; } < \"$TMPDIR/delim.bin\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "file=<a> rc=0\n");
}

#[test]
fn read_d_carrier_byte_delimiter_control() {
    // 0x1b travels as a tagged carrier pair; that form was already
    // matched on the base tree (rubash#1300 family).
    let (stdout, _, code) = run_script(
        "printf 'a\\033b\\033' | \
         while IFS= read -rd $'\\033' w; do printf '<%q>\\n' \"$w\"; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<a>\n<b>\n");
}
