//! Issue rubash#2779 (A2-11 residual): a pipeline brace group's
//! staged input starves the next reader after `mapfile -n`.
//!
//! GNU: a pipeline element's fd 0 is the real inter-stage pipe
//! (execute_cmd.c do_piping dup2s the pipe onto fd 0), shared by
//! every command in the brace group. `mapfile` deliberately avoids
//! over-reading such a descriptor: for a pipe it switches to
//! unbuffered reads (builtins/mapfile.def:162-166 — `lseek` fails
//! with ESPIPE, so reads pull record-by-record via zgetline), and
//! for a seekable file it rewinds the descriptor past its buffered
//! surplus when done (mapfile.def post-read lseek). A bounded
//! `mapfile -n 1` therefore leaves l2/l3 for the next reader.
//!
//! Rubash stages a pipeline element's input as an executor snapshot
//! (command_input_scope / the FUNCTION_STDIN mirror): taking the
//! snapshot consumes the shared remainder wholesale, so after a
//! bounded mapfile the following `cat` sees EOF. PR #2670 fixed the
//! real-handle faces of A2-11; this staged face remains. All
//! expectations below were measured byte-for-byte against the GNU
//! bash 5.3.20 oracle with the same scripts.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2779-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

#[test]
fn issue2779_mapfile_n1_then_cat_gets_the_tail() {
    let dir = tempdir("core");
    let run = rubash_c(
        "printf \"l1\\nl2\\nl3\\n\" | { mapfile -n 1 -t arr; echo \"arr=[${arr[*]}]\"; cat; }",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "arr=[l1]\nl2\nl3\n", "stderr: {}", run.stderr);
}

#[test]
fn issue2779_mapfile_n2_then_cat_gets_the_tail() {
    let dir = tempdir("n2");
    let run = rubash_c(
        "printf \"l1\\nl2\\nl3\\n\" | { mapfile -n 2 -t arr; cat; }",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "l3\n", "stderr: {}", run.stderr);
}

#[test]
fn issue2779_mapfile_n1_then_read_chain_gets_l2_and_l3() {
    let dir = tempdir("readchain");
    let run = rubash_c(
        "printf \"l1\\nl2\\nl3\\n\" | { mapfile -n 1 -t a; read -r x; echo \"next=[$x]\"; read -r y; echo \"next2=[$y]\"; }",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "next=[l2]\nnext2=[l3]\n",
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn issue2779_second_mapfile_resumes_where_the_first_stopped() {
    let dir = tempdir("twomapfile");
    let run = rubash_c(
        "printf \"l1\\nl2\\nl3\\n\" | { mapfile -n 1 -t a; mapfile -n 1 -t b; echo \"a=[${a[*]}] b=[${b[*]}]\"; cat; }",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "a=[l1] b=[l2]\nl3\n", "stderr: {}", run.stderr);
}

#[test]
fn issue2779_mapfile_all_then_cat_gets_nothing() {
    // Unbounded mapfile legitimately consumes everything; cat must
    // see EOF (guards against a fix that replays the payload).
    let dir = tempdir("all");
    let run = rubash_c(
        "printf \"l1\\nl2\\nl3\\n\" | { mapfile -t arr; cat; echo \"all=[${arr[*]}]\"; }",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "all=[l1 l2 l3]\n", "stderr: {}", run.stderr);
}

#[test]
fn issue2779_brace_group_file_redirect_mapfile_n1_then_cat() {
    // Boundary: the brace group's stdin is a real file rather than a
    // pipeline stage. GNU still leaves the tail (mapfile rewinds its
    // buffered surplus on seekable descriptors).
    let dir = tempdir("file");
    std::fs::write(dir.join("in.txt"), "l1\nl2\nl3\n").expect("seed in.txt");
    let run = rubash_c(
        "{ mapfile -n 1 -t arr; echo \"arr=[${arr[*]}]\"; cat; } < in.txt",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "arr=[l1]\nl2\nl3\n", "stderr: {}", run.stderr);
}

#[test]
fn issue2779_subshell_pipeline_stage_mapfile_n1_then_cat() {
    let dir = tempdir("subshell");
    let run = rubash_c(
        "printf \"l1\\nl2\\nl3\\n\" | ( mapfile -n 1 -t arr; echo \"arr=[${arr[*]}]\"; cat )",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "arr=[l1]\nl2\nl3\n", "stderr: {}", run.stderr);
}

#[test]
fn issue2779_chain_mapfile_read_cat_three_readers_in_sequence() {
    // Chain: three different readers consume the staged stream in
    // sequence, each resuming at the previous reader's cursor.
    let dir = tempdir("chain");
    let run = rubash_c(
        "printf \"l1\\nl2\\nl3\\n\" | { mapfile -n 1 -t a; read -r x; echo \"a=[${a[*]}] next=[$x]\"; cat; }",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "a=[l1] next=[l2]\nl3\n",
        "stderr: {}",
        run.stderr
    );
}
