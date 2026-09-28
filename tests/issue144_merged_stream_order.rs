//! Issue niubash#144 regression: an external child's merged-output streams
//! must keep TRUE WRITE ORDER and stay LIVE. The retired routes piped the
//! child's stdout and stderr separately, then replayed stdout-then-stderr at
//! exit — reordering `E1 O2 E3` into `O2 E1 E3` (the two-pipe concatenation
//! order) and withholding both halves until the child exited.
//!
//! GNU baseline: `2>&1` is `dup2(1, 2)` (redir.c:1169-1170 "This is correct.
//! 2>&1 means dup2 (1, 2);") — one shared open description per merge target,
//! so the merged bytes interleave exactly in the order the child wrote them,
//! and each write reaches the shared file/pipe as it happens. The
//! `r_err_and_out` family (redir.c:1030-1037) gives `&>f` / `>&f` /
//! `2>f 1>&2` the same one-open semantics.
//!
//! The child used here (`cmd /c "echo E1 1>&2 & echo O2 & echo E3 1>&2"`)
//! writes E1 (fd 2), O2 (fd 1), E3 (fd 2) in strict sequence, so the
//! byte order in the merged stream discriminates the shared-handle fix
//! (E1 O2 E3) from the retired two-pipe replay (O2 E1 E3) with no sleeps.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// `cmd /c` child writing E1 (stderr), O2 (stdout), E3 (stderr) in order.
const ORDER_CHILD: &str = "echo E1 1>&2 & echo O2 & echo E3 1>&2";

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// A per-test temp directory; removed on drop.
struct Fixture {
    dir: std::path::PathBuf,
}

impl Fixture {
    fn script(&self, body: &str) -> String {
        format!(
            "cd '{}' || exit 1\n{}\n",
            self.dir.to_string_lossy().replace('\\', "/"),
            body
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("rubash-issue144-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue144 fixture dir");
    Fixture { dir }
}

/// Splits shell text into \n-terminated lines with the CR and cmd.exe
/// trailing space trimmed, preserving order.
fn ordered_lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n")
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| line.trim_end().to_string())
        .collect()
}

#[test]
fn merged_file_target_keeps_stderr_first_order() {
    let fixture = fixture("file");
    let (stdout, stderr, code) =
        rubash(&fixture.script(&format!("cmd /c \"{ORDER_CHILD}\" > m.txt 2>&1\ncat m.txt")));
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    // The retired two-pipe replay produced O2, E1, E3 here.
    assert_eq!(ordered_lines(&stdout), vec!["E1", "O2", "E3"]);
    let merged = std::fs::read_to_string(fixture.dir.join("m.txt")).expect("read m.txt");
    assert_eq!(ordered_lines(&merged), vec!["E1", "O2", "E3"]);
}

#[test]
fn merged_dup_2_to_1_then_word_keeps_order() {
    // `2> s.txt 1>&2` (GNU: the fd-1 dup onto fd 2's open description).
    let fixture = fixture("dup");
    let (stdout, stderr, code) = rubash(&fixture.script(&format!(
        "cmd /c \"{ORDER_CHILD}\" 2> s.txt 1>&2\ncat s.txt"
    )));
    assert_eq!(ordered_lines(&stdout), vec!["E1", "O2", "E3"]);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let merged = std::fs::read_to_string(fixture.dir.join("s.txt")).expect("read s.txt");
    assert_eq!(ordered_lines(&merged), vec!["E1", "O2", "E3"]);
}

#[test]
fn combined_output_word_keeps_order() {
    // `>& m.txt`: r_err_and_out opens the word once and dups fd 2 onto it
    // (redir.c:832-838, redir.c:1030-1037).
    let fixture = fixture("ampword");
    let (stdout, stderr, code) =
        rubash(&fixture.script(&format!("cmd /c \"{ORDER_CHILD}\" >& m.txt\ncat m.txt")));
    assert_eq!(ordered_lines(&stdout), vec!["E1", "O2", "E3"]);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let merged = std::fs::read_to_string(fixture.dir.join("m.txt")).expect("read m.txt");
    assert_eq!(ordered_lines(&merged), vec!["E1", "O2", "E3"]);
}

#[test]
fn group_and_subshell_merges_keep_order() {
    let fixture = fixture("group");
    let (stdout, stderr, code) = rubash(&fixture.script(&format!(
        "{{ cmd /c \"{ORDER_CHILD}\"; }} > g1.txt 2>&1\n( cmd /c \"{ORDER_CHILD}\" ) > g2.txt 2>&1\ncat g1.txt g2.txt"
    )));
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(
        ordered_lines(&stdout),
        vec!["E1", "O2", "E3", "E1", "O2", "E3"]
    );
}

#[test]
fn split_redirects_stay_split() {
    // The #144 fix must not disturb plain `> o 2> e`: stdout half in o,
    // stderr half in e (GNU do_redirections opens each fd separately).
    let fixture = fixture("split");
    let (stdout, stderr, code) = rubash(&fixture.script(&format!(
        "cmd /c \"{ORDER_CHILD}\" > o.txt 2> e.txt\necho O:; cat o.txt\necho E:; cat e.txt"
    )));
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(ordered_lines(&stdout), vec!["O:", "O2", "E:", "E1", "E3"]);
}

#[test]
fn same_file_double_open_matches_gnu_shape() {
    // `> c.txt 2> c.txt`: two independent opens both at offset 0; the
    // later stdout write overwrites E1, so GNU leaves O2 then E3
    // (verified against WSL GNU bash 5.3.0; gnu-order.out shape C).
    let fixture = fixture("samefile");
    let (_stdout, stderr, code) = rubash(&fixture.script(&format!(
        "cmd /c \"{ORDER_CHILD}\" > c.txt 2> c.txt\ncat c.txt"
    )));
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let merged = std::fs::read_to_string(fixture.dir.join("c.txt")).expect("read c.txt");
    assert_eq!(ordered_lines(&merged), vec!["O2", "E3"]);
}

#[test]
fn command_substitution_merged_capture_keeps_order() {
    // Inside a command substitution the merge target is the capture: GNU's
    // child holds the substitution pipe on fd 1 and dup2's fd 2 onto it
    // (subst.c:7143 command_substitute), so the captured bytes keep write
    // order (rubash#223 capture generations preserved).
    let fixture = fixture("comsub");
    let (stdout, stderr, code) = rubash(&fixture.script(&format!(
        "x=$(cmd /c \"{ORDER_CHILD}\" 2>&1)\nprintf '[%s]' \"$x\""
    )));
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(
        ordered_lines(&stdout.replace('[', "").replace(']', "")),
        vec!["E1", "O2", "E3"]
    );
}

/// Runs a rubash script with piped stdout and returns (bytes, wall time to
/// first byte, exit code). Panics if no byte arrives within `first_byte_bound`.
fn streamed_output(script: &str, first_byte_bound: Duration) -> (Vec<u8>, Duration, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rubash");
    let start = Instant::now();
    let mut stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    bytes.extend_from_slice(&chunk[..n]);
                    let _ = tx.send(Instant::now());
                }
            }
        }
        bytes
    });
    let first_byte = rx
        .recv_timeout(first_byte_bound)
        .expect("merged stderr arrived live (was withheld until exit)");
    let bytes = reader.join().expect("reader thread");
    let status = child.wait().expect("wait rubash");
    (bytes, first_byte.duration_since(start), status.code())
}

#[test]
fn merged_stream_arrives_live_not_withheld_to_exit() {
    // Streaming canary (bounded): the child writes E1 to the merged stream,
    // sleeps ~2s (ping -n 3), writes O2, exits. E1 must arrive while the
    // child is still running — the retired capture-replay withheld BOTH
    // halves until exit. Bound: first byte within 1.5s of a >=2s child.
    let fixture = fixture("canary");
    let (bytes, ttfb, code) = streamed_output(
        &fixture.script("cmd /c \"echo E1 1>&2 & ping -n 3 127.0.0.1 >nul & echo O2\" 2>&1"),
        Duration::from_millis(1500),
    );
    assert_eq!(code, Some(0));
    assert!(ttfb < Duration::from_millis(1500), "ttfb was {ttfb:?}");
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(ordered_lines(&text), vec!["E1", "O2"]);
}

#[test]
fn pipeline_merged_stage_keeps_order_and_drains() {
    // `2>&1 | cat` shares ONE pipe between the producer's fd 1 and fd 2
    // (GNU binds the element's fd 1 to the pipe, then the dup lands fd 2 on
    // the same pipe). Also guards the drain deadlock fixed by dropping the
    // std::process::Command (which retains the pipe write ends) before
    // read_to_end: a regression hangs, and the 30s bound below fails the
    // test instead of the runner.
    let fixture = fixture("pipe");
    let (bytes, ttfb, code) = streamed_output(
        &fixture.script(&format!("cmd /c \"{ORDER_CHILD}\" 2>&1 | cat")),
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0));
    assert!(ttfb < Duration::from_secs(30));
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(ordered_lines(&text), vec!["E1", "O2", "E3"]);
}

#[test]
fn pipestatus_after_merged_element() {
    let fixture = fixture("pipestatus");
    let (stdout, _, code) = rubash(&fixture.script(&format!(
        "cmd /c \"{ORDER_CHILD} & exit 5\" 2>&1 | cat > /dev/null\necho ${{PIPESTATUS[*]}}"
    )));
    assert_eq!(code, Some(0));
    assert_eq!(ordered_lines(&stdout), vec!["5 0"]);
}
