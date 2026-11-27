//! rubash#3535 — history comment-char arming is sticky in GNU.
//!
//! GNU model (vendored bash 5.3.20 sources):
//! - `variables.c:6168-6177` sv_histtimefmt: when HISTTIMEFORMAT exists,
//!   `if (history_comment_char == 0) history_comment_char = '#';` — and
//!   the unset arm only clears `history_write_timestamps`. The comment
//!   char is NEVER reset, so a bind followed by `unset HISTTIMEFORMAT`
//!   leaves timestamp recognition armed for later reads.
//! - `lib/readline/histfile.c:375-383`: a file starting with `#<digit>`
//!   only borrows the comment char for that one read (reset at :466).
//! - `lib/readline/history.c:322-333` hist_inittime: an entry read while
//!   the comment char is unarmed carries a NUL-char stamp, rendered `??`
//!   once a format appears; read while armed it carries the read time.
//! - `lib/readline/histfile.c:384`: multiline regrouping is gated on
//!   `history_write_timestamps` (the variable existing at READ time),
//!   NOT on the sticky comment char — stickiness must not widen it.
//!
//! Expectations measured with GNU bash 5.3.20 (script files, TZ=UTC).

use std::path::PathBuf;
use std::process::{Command, Stdio};

const PLAIN: &str = "printf 'cmd-one\\n#1700000000\\ncmd-two\\n' > plain.txt\nexport TZ=UTC\n";

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3535-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_rubash(tag: &str, script: &str) -> (String, String) {
    let dir = tempdir(tag);
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The armed two-entry shape: cmd-two carries its file stamp (2023),
/// cmd-one carries the read-time stamp (its year is the run year, so
/// only the shape is pinned), and no `??` appears.
fn assert_armed_two_entries(stdout: &str, stderr: &str) {
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout} stderr: {stderr}");
    assert!(
        lines[0].starts_with("    1  [") && lines[0].ends_with("] cmd-one"),
        "stdout: {stdout}"
    );
    assert_eq!(lines[1], "    2  [2023] cmd-two", "stdout: {stdout}");
}

#[test]
fn read_after_unset_stays_armed() {
    let (stdout, stderr) = run_rubash(
        "repro",
        &format!(
            "{PLAIN}HISTTIMEFORMAT='[%Y] '\nunset HISTTIMEFORMAT\nhistory -r plain.txt\nHISTTIMEFORMAT='[%Y] '\nhistory\n"
        ),
    );
    assert_armed_two_entries(&stdout, &stderr);
}

#[test]
fn never_armed_read_treats_stamp_line_as_literal() {
    let (stdout, stderr) = run_rubash(
        "control",
        &format!("{PLAIN}history -r plain.txt\nHISTTIMEFORMAT='[%Y] '\nhistory\n"),
    );
    assert_eq!(
        stdout, "    1  ??cmd-one\n    2  ??#1700000000\n    3  ??cmd-two\n",
        "stderr: {stderr}"
    );
}

#[test]
fn temporary_prefix_bind_arms_stickily() {
    let (stdout, stderr) = run_rubash(
        "tempprefix",
        &format!(
            "{PLAIN}HISTTIMEFORMAT='[%Y] ' true\nhistory -r plain.txt\nHISTTIMEFORMAT='[%Y] '\nhistory\n"
        ),
    );
    assert_armed_two_entries(&stdout, &stderr);
}

#[test]
fn set_but_empty_bind_arms_stickily() {
    let (stdout, stderr) = run_rubash(
        "empty",
        &format!(
            "{PLAIN}HISTTIMEFORMAT=\nunset HISTTIMEFORMAT\nhistory -r plain.txt\nHISTTIMEFORMAT='[%Y] '\nhistory\n"
        ),
    );
    assert_armed_two_entries(&stdout, &stderr);
}

#[test]
fn read_new_after_unset_stays_armed() {
    let (stdout, stderr) = run_rubash(
        "readnew",
        &format!(
            "{PLAIN}HISTTIMEFORMAT='[%Y] '\nunset HISTTIMEFORMAT\nhistory -n plain.txt\nHISTTIMEFORMAT='[%Y] '\nhistory\n"
        ),
    );
    assert_armed_two_entries(&stdout, &stderr);
}

#[test]
fn sticky_arming_does_not_widen_multiline_regrouping() {
    let (stdout, stderr) = run_rubash(
        "multigate",
        "printf '#86400\\necho a\\nb\\n#1234567890\\necho c\\n' > h1\nexport TZ=UTC\nHISTTIMEFORMAT=\nunset HISTTIMEFORMAT\nhistory -r h1\nhistory\n",
    );
    assert_eq!(
        stdout, "    1  echo a\n    2  b\n    3  echo c\n",
        "stderr: {stderr}"
    );
}
