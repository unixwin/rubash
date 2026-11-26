//! rubash#1134 — `printf` invalid-number diagnostic comes after the
//! output value in a merged 2>&1 stream.
//!
//! GNU builtins/printf.def: the conversion error is reported while the
//! format is being processed, BEFORE the converted value (`0`) for that
//! conversion is emitted. In a merged stream the diagnostic therefore
//! precedes the value; captured separately the two streams are the
//! same as they ever were.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` string with
//! stdout and stderr attached to the SAME file): diagnostic line
//! first, then `0`; matched on the stable core (line prefix carries
//! the binary's own path).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1134-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn printf_diagnostic_precedes_value_in_merged_stream() {
    let dir = tempdir("merged");
    let merged_path = dir.join("merged.txt");
    let file = std::fs::File::create(&merged_path).expect("create merged file");
    let file2 = file.try_clone().expect("clone merged file handle");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("printf '%d\\n' abc")
        .stdout(Stdio::from(file))
        .stderr(Stdio::from(file2))
        .output()
        .expect("run rubash");
    assert_eq!(output.status.code(), Some(1));
    let merged = std::fs::read_to_string(&merged_path).expect("read merged file");
    let diag_pos = merged
        .find("printf: abc: invalid number")
        .unwrap_or_else(|| panic!("diagnostic missing from merged stream: {merged:?}"));
    let value_pos = merged
        .find("\n0\n")
        .unwrap_or_else(|| panic!("converted value missing from merged stream: {merged:?}"));
    assert!(
        diag_pos < value_pos,
        "diagnostic must precede the value in the merged stream: {merged:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
