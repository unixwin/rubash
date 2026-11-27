//! read / mapfile / input regression pins (backfill wave bf-c).
//!
//! rubash#1299 — `select` on closed stdin (`done <&-`) dropped GNU's
//! read-error diagnostic; GNU prints the menu, then
//! `0: read error: Bad file descriptor`, the body never runs, and the
//! loop's status is 1.
//! rubash#1300 — `read -d '\'` treated the backslash as a delimiter;
//! without `-r` the backslash is the escape introducer and never matches
//! as a delimiter (the whole line is consumed, EOF gives rc=1). With
//! `-r` both shells agree it delimits.
//! rubash#1301 — `read -a -d : arr` (where `-a` swallows `-d` as its
//! array name) emitted a spurious extra `` `-d': not a valid identifier ``;
//! GNU validates only the actual variable positions (`:`) — one line.
//! rubash#1255 — `read -p PROMPT` never displayed the prompt on a tty.
//! TTY-only: its pin lives with the other interactive forms in
//! `tests/issue1016_1072_1073_1254_1255_history_interactive_regressions.rs`
//! (util-linux `script(1)` PTY harness, unix); off-tty both shells
//! suppress the prompt by design.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files
//! and piped probes in this environment (byte-level stdout/stderr/rc).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-bfc-read-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script_stdin(script: &str, stdin_bytes: &[u8]) -> (String, String, Option<i32>) {
    let dir = scratch("script");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(stdin_bytes)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn issue_1299_select_on_closed_stdin_reports_read_error() {
    let (stdout, stderr, code) = run_script_stdin(
        "select c in aa bb; do echo \"c=[$c]\"; break; done <&-\necho \"rc=$?\"\n",
        b"",
    );
    // GNU: select prints a lone newline to stdout, the body never runs,
    // and the loop ends with status 1.
    assert_eq!(stdout, "\nrc=1\n", "stderr: {stderr}");
    assert!(stderr.contains("1) aa"), "menu missing; stderr: {stderr}");
    assert!(stderr.contains("2) bb"), "menu missing; stderr: {stderr}");
    assert!(
        stderr.contains("read error: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1300_read_d_backslash_is_escape_not_delimiter() {
    let (stdout, stderr, code) = run_script_stdin(
        "read -d '\\' x\necho \"rc=$? x=[$x]\"\nread y\necho \"y=[$y]\"\n",
        b"a\\b:c\n",
    );
    // GNU: `\b` collapses to `b`, the backslash never delimits, the
    // whole line lands in x and EOF yields rc=1; nothing remains for y.
    assert_eq!(stdout, "rc=1 x=[ab:c]\ny=[]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1300_read_r_d_backslash_does_delimit() {
    // Control: with -r the backslash is a plain byte and DOES delimit —
    // both shells agree here.
    let (stdout, _stderr, code) =
        run_script_stdin("read -r -d '\\' x\necho \"rc=$? x=[$x]\"\n", b"a\\b:c\n");
    assert_eq!(stdout, "rc=0 x=[a]\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1301_read_a_swallowing_dash_d_validates_only_var_positions() {
    let (stdout, stderr, code) = run_script_stdin("read -a -d : arr\necho \"rc=$?\"\n", b"a b\n");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert_eq!(
        stderr.matches("not a valid identifier").count(),
        1,
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("`:': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("`-d': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
