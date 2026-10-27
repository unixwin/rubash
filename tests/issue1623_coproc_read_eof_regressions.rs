//! rubash#1623 — `read -u ${COPROC[0]}` reported EOF (rc 1) after a
//! complete, delimiter-terminated record served from a coprocess.
//!
//! Root cause: the `-u N` serve in read_io.rs re-derived the
//! read.def:949 EOF flag from the TRIMMED record text
//! (`!record_contains_delimiter(output, ...)`), but trim_read_input
//! has already removed the delimiter, so every successful record —
//! and every `-n`/`-N` limit stop — scored as "EOF without
//! delimiter". read_coproc_stdout owns that accounting itself (it
//! sets the flag only when the pipe ends before the delimiter
//! arrives); the caller's re-derivation is gone.
//!
//! Expectations verified against GNU bash 5.3.20 script-file probes.

use std::process::{Command, Stdio};

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1623-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn coproc_read_full_line_succeeds() {
    let (stdout, stderr, code) = rubash_script(
        "coproc MYC { cat; }\n\
         echo hello >&${MYC[1]}\n\
         read -u ${MYC[0]} line; echo \"read rc=$? line=[$line]\"\n\
         exec {MYC[0]}>&- {MYC[1]}>&-\n",
    );
    assert_eq!(stdout, "read rc=0 line=[hello]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_read_char_limit_stop_is_not_eof() {
    // A `-n` limit stop mid-record is a success in GNU (read.def):
    // the record was not exhausted, so no EOF may be reported.
    let (stdout, stderr, code) = rubash_script(
        "coproc MYC { cat; }\n\
         echo world >&${MYC[1]}\n\
         read -n 3 -u ${MYC[0]} part; echo \"n3 rc=$? part=[$part]\"\n\
         exec {MYC[0]}>&- {MYC[1]}>&-\n",
    );
    assert_eq!(stdout, "n3 rc=0 part=[wor]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}
