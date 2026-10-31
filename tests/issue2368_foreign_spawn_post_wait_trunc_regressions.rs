//! rubash#2368 — foreign-spawn face: a redirect list holding BOTH a
//! `{var}` dup AND a pending close of the dup's source
//! (`9>o1.txt {w}>&9 {v}>&-`, v=9) made the parent re-open the dup's
//! target with O_TRUNC *after* the child exited, destroying the bytes
//! the child wrote through the descriptor (residual D1b of the
//! rubash#2322 cluster).
//!
//! GNU applies a command's redirections as ONE ordered walk in the
//! child (redir.c:237 do_redirections, loop at redir.c:252): the dup
//! copies the descriptor as bound at its position and nothing ever
//! re-opens — let alone truncates — a target after the child ran.
//!
//! Root cause in rubash: the list's output close routed the spawn
//! through the parent-drained capture path, whose post-wait routing
//! walk (write_external_captured_streams ->
//! route_builtin_buffered_output) re-ran the full ordered walk with
//! plain apply semantics, so the numbered `9>o1.txt` entry performed
//! its create/truncate validation open a second time, on the file
//! the child had just filled. The pipeline face's routing walk
//! already resolves numbered entries without opening them
//! (OutputFdState::resolve_only_numbered, rubash#2255); the fix runs
//! this post-external walk in the same mode — the child's spawn
//! delivery realized every numbered binding as a real descriptor,
//! so the walk only routes the drained fd-1/fd-2 bytes.
//!
//! Every test drives a REAL foreign child (/usr/bin/bash) so no
//! rubash respawn/env channel can mask the result. Expectations
//! byte-verified against GNU bash 5.3.20 with script-file probes
//! (cluster workdir probes/: probe, dup_only, unrelated_close,
//! comsub, comsub_duponly, errdup).

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");
const FOREIGN: &str = "/usr/bin/bash";

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2368-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    // Foreign child: writes W10 through the {var}-dup descriptor
    // (fd 10, the first free slot past the command's own 9) and W9
    // through fd 9 itself, reporting which writes the descriptor
    // state allowed.
    fs::write(
        dir.join("reader.sh"),
        "if echo W10 >&10; then echo \"w10=OK\"; else echo \"w10=FAIL\"; fi\n\
         if echo W9 >&9; then echo \"w9=OK\"; else echo \"w9=FAIL\"; fi\n",
    )
    .expect("write reader");
    dir
}

fn write_script(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).expect("write fixture");
}

/// Run a script FILE in `dir` with stdin closed (null).
fn run_script(dir: &Path, name: &str) -> (String, String, Option<i32>) {
    let output = Command::new(RUBASH)
        .arg(dir.join(name))
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn file_contents(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).unwrap_or_default()
}

fn parent_script(child_args: &str) -> String {
    format!(
        "exec 9>seedout.txt\nv=9\n{FOREIGN} reader.sh {child_args}\n\
         echo \"rc=$? w=[$w]\"\necho \"o1=[$(cat o1.txt)]\"\n"
    )
}

#[test]
fn issue2368_dup_bytes_survive_close_of_dup_source() {
    let dir = scratch("headline");
    write_script(&dir, "case.sh", &parent_script("9>o1.txt {w}>&9 {v}>&-"));
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    // w9=FAIL: fd 9 was closed in the child by the {v}>&- entry.
    // o1 keeps exactly the dup descriptor's bytes.
    assert_eq!(
        stdout, "w10=OK\nw9=FAIL\nrc=0 w=[]\no1=[W10]\n",
        "stderr: {stderr}"
    );
    assert_eq!(file_contents(&dir, "o1.txt"), "W10\n");
}

#[test]
fn issue2368_dup_only_control_keeps_both_writes() {
    let dir = scratch("dup-only");
    write_script(&dir, "case.sh", &parent_script("9>o1.txt {w}>&9"));
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "w10=OK\nw9=OK\nrc=0 w=[]\no1=[W10\nW9]\n",
        "stderr: {stderr}"
    );
    assert_eq!(file_contents(&dir, "o1.txt"), "W10\nW9\n");
}

#[test]
fn issue2368_unrelated_close_control_keeps_both_writes() {
    let dir = scratch("unrelated-close");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seedout.txt\nexec 8>other.txt\nv=8\n\
         /usr/bin/bash reader.sh 9>o1.txt {w}>&9 {v}>&-\n\
         echo \"rc=$?\"\necho \"o1=[$(cat o1.txt)]\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "w10=OK\nw9=OK\nrc=0\no1=[W10\nW9]\n",
        "stderr: {stderr}"
    );
    assert_eq!(file_contents(&dir, "o1.txt"), "W10\nW9\n");
}

#[test]
fn issue2368_each_command_in_sequence_keeps_its_bytes() {
    let dir = scratch("sequence");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seedout.txt\nv=9\n\
         /usr/bin/bash reader.sh 9>o1.txt {w}>&9 {v}>&-\n\
         /usr/bin/bash reader.sh 9>o2.txt {w}>&9 {v}>&-\n\
         echo \"o1=[$(cat o1.txt)]\"\necho \"o2=[$(cat o2.txt)]\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "w10=OK\nw9=FAIL\nw10=OK\nw9=FAIL\no1=[W10]\no2=[W10]\n",
        "stderr: {stderr}"
    );
    assert_eq!(file_contents(&dir, "o1.txt"), "W10\n");
    assert_eq!(file_contents(&dir, "o2.txt"), "W10\n");
}

#[test]
fn issue2368_comsub_dup_bytes_survive_close_of_dup_source() {
    let dir = scratch("comsub");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seedout.txt\nv=9\n\
         x=$(/usr/bin/bash reader.sh 9>o1.txt {w}>&9 {v}>&-)\n\
         echo \"x=[$x]\"\necho \"o1=[$(cat o1.txt)]\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "x=[w10=OK\nw9=FAIL]\no1=[W10]\n",
        "stderr: {stderr}"
    );
    assert_eq!(file_contents(&dir, "o1.txt"), "W10\n");
}

#[test]
fn issue2368_drained_stderr_still_routes_through_dup_target() {
    // The routing walk's real job must survive the fix: a drained
    // fd-2 stream whose target is a dup of the numbered file still
    // lands in that file, appended after the child's own descriptor
    // writes — it is the re-OPEN that was spurious, not the routing.
    let dir = scratch("errdup");
    fs::write(dir.join("reader_err.sh"), "echo W10 >&10\necho E2 >&2\n").expect("write reader_err");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seedout.txt\nv=9\n\
         /usr/bin/bash reader_err.sh 9>o1.txt {w}>&9 2>&9 {v}>&-\n\
         echo \"rc=$?\"\necho \"o1=[$(cat o1.txt)]\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\no1=[W10\nE2]\n", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "o1.txt"), "W10\nE2\n");
}
