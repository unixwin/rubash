//! rubash#2029 — a pipeline element 0's PARTIAL consumption of a
//! materialized fd-alias payload must fold the shared descriptor offset.
//!
//! GNU execute_pipeline (execute_cmd.c:2620) forks every element with the
//! shell's own descriptors, and the element applies its own `<&N` inside
//! the fork as a dup2 (redir.c:1138 do_redirection_internal, case
//! r_duplicating_input -> dup2 at :1193), so the element's reads advance
//! the SHARED open-file offset that a later `cat <&3` continues from.
//! Coreutils head additionally seeks a regular input back to the last
//! printed byte (head.c), so `head -1 <&3 | wc -l; cat <&3` prints the
//! remainder (lines 2-5) after GNU bash 5.3.20.
//!
//! rubash materializes the aliased descriptor as a buffered endpoint
//! (#2003) and the inline stage arms consume the payload COPY; before the
//! fix nothing reported that consumption back, so the follow-up reader
//! replayed the whole payload. The fix routes the arms' measured
//! consumption through pipeline_stdin_consumed and the driver folds it
//! into the materialized endpoint (prefix semantics for a seekable file
//! source, block-drain semantics for a pipe-like source — a procsub
//! stream or coproc read end cannot be seeked back).
//!
//! Every expectation below was cross-checked byte-for-byte against WSL
//! GNU bash 5.3.20 (issue body's oracle).

use std::process::Command;

fn run_script(body: &str) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!("issue2029_{}.sh", std::process::id()));
    std::fs::write(&path, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&std::env::temp_dir())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const FIVE: &str = "\
exec 3<five.txt
";

fn five_fixture(dir: &std::path::Path) -> String {
    let path = dir.join("five.txt");
    std::fs::write(&path, b"l1\nl2\nl3\nl4\nl5\n").expect("write fixture");
    dir.to_string_lossy().replace('\\', "/")
}

/// The issue repro: the head arm prints line 1 and the follow-up `cat <&3`
/// must continue after it (GNU: `1` then lines 2-5).
#[test]
fn head_pipe_fold_leaves_the_remainder() {
    let dir = std::env::temp_dir();
    let base = five_fixture(&dir);
    let out = run_script(&format!(
        "{base}\n{FIVE}head -1 <&3 | wc -l\ncat <&3\nexec 3<&-\n"
    ));
    assert_eq!(out, "1\nl2\nl3\nl4\nl5\n");
}

/// `head -2` consumes lines 1-2; `read x <&3` continues at line 3.
#[test]
fn head_two_lines_then_read_continues() {
    let dir = std::env::temp_dir();
    let base = five_fixture(&dir);
    let out = run_script(&format!(
        "{base}\n{FIVE}head -2 <&3 | wc -l\nread x <&3; echo next=[$x]\nexec 3<&-\n"
    ));
    assert_eq!(out, "2\nnext=[l3]\n");
}

/// `head -0` prints nothing and must consume nothing (GNU head does not
/// read for a zero count).
#[test]
fn head_zero_leaves_the_payload_intact() {
    let dir = std::env::temp_dir();
    let base = five_fixture(&dir);
    let out = run_script(&format!(
        "{base}\n{FIVE}head -0 <&3 | wc -l\ncat <&3\nexec 3<&-\n"
    ));
    assert_eq!(out, "0\nl1\nl2\nl3\nl4\nl5\n");
}

/// The GNU negative form: `head -n -1` prints all but the last line and
/// consumes exactly that much.
#[test]
fn head_all_but_last_folds_the_prefix() {
    let dir = std::env::temp_dir();
    let base = five_fixture(&dir);
    let out = run_script(&format!(
        "{base}\n{FIVE}head -n -1 <&3 | wc -l\ncat <&3\nexec 3<&-\n"
    ));
    assert_eq!(out, "4\nl5\n");
}

/// A pipe-like source (process substitution) is unseekable: a reading
/// stage drains the stream (GNU head's first read(2) takes the whole
/// procsub-sized pipe), a zero-count stage leaves it untouched.
#[test]
fn procsub_source_reading_head_drains() {
    let dir = std::env::temp_dir();
    let base = five_fixture(&dir);
    let out = run_script(&format!(
        "{base}\nexec 3< <(printf 'p1\\np2\\np3\\n')\nhead -1 <&3 | wc -l\ncat <&3\nexec 3<&-\n"
    ));
    assert_eq!(out, "1\n");
}

#[test]
fn procsub_source_zero_head_keeps_the_stream() {
    let dir = std::env::temp_dir();
    let base = five_fixture(&dir);
    let out = run_script(&format!(
        "{base}\nexec 3< <(printf 'p1\\np2\\np3\\n')\nhead -0 <&3 | wc -l\ncat <&3\nexec 3<&-\n"
    ));
    assert_eq!(out, "0\np1\np2\np3\n");
}

/// Full-consumer inline arms (wc) fold the endpoint to EOF: a follow-up
/// reader sees nothing (GNU wc reads the dup to EOF without seek-back).
#[test]
fn full_consumer_wc_drains_the_endpoint() {
    let dir = std::env::temp_dir();
    let base = five_fixture(&dir);
    let out = run_script(&format!(
        "{base}\n{FIVE}wc -l <&3 | wc -l\ncat <&3\nexec 3<&-\n"
    ));
    assert_eq!(out, "1\n");
}

/// grep scans the whole payload copy — the fold follows it to EOF.
#[test]
fn grep_stage_drains_the_endpoint() {
    let dir = std::env::temp_dir();
    let base = five_fixture(&dir);
    let out = run_script(&format!(
        "{base}\n{FIVE}grep l3 <&3 | wc -l\ncat <&3\nexec 3<&-\n"
    ));
    assert_eq!(out, "1\n");
}

/// A compound stage's read loop consumes through the shared endpoint; the
/// follow-up `cat <&3` continues after the lines the body read.
#[test]
fn compound_read_stage_folds_the_remainder() {
    let dir = std::env::temp_dir();
    let base = five_fixture(&dir);
    let out = run_script(&format!(
        "{base}\n{FIVE}{{ read x; echo \"got=[$x]\"; }} <&3 | wc -l\ncat <&3\nexec 3<&-\n"
    ));
    assert_eq!(out, "1\nl2\nl3\nl4\nl5\n");
}

/// The #2003 drain face is preserved: a cat stage still hands the next
/// reader EOF, and the pipeline's own count stays exact.
#[test]
fn cat_stage_still_drains_once() {
    let dir = std::env::temp_dir();
    let base = five_fixture(&dir);
    let out = run_script(&format!(
        "{base}\n{FIVE}cat <&3 | wc -l\ncat <&3\nexec 3<&-\n"
    ));
    assert_eq!(out, "5\n");
}
