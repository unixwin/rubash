//! rubash#2562 — a numbered OUTPUT dup whose source an EARLIER
//! entry of the SAME redirect list bound input-side was wrongly
//! rejected with `9: Bad file descriptor`:
//!
//!     echo hi 9<f9.txt 10>&9        (GNU: passes)
//!     echo hi 9<>f.txt 10>&9        (GNU: passes)
//!     cat 9<f9.txt 10>&9 <&10       (GNU: passes, prints the file)
//!
//! GNU applies a command's redirections as ONE ordered walk in the
//! child (redir.c:237 do_redirections, loop at redir.c:252): the
//! dup arms (do_redirection_internal, redir.c:1139-1275) require
//! only that the source be an OPEN descriptor at the dup's
//! position — descriptor direction is irrelevant to dup2
//! (redir.c:1193), and the description the dup clones is the one
//! the earlier entry bound (`<>` is O_RDWR|O_CREAT, make_cmd.c:682).
//!
//! Root cause in rubash: the apply walk's state is seeded from the
//! AMBIENT fd table only, and the walk skipped Input / ReadWrite /
//! DuplicateInput / CloseInput / HereDoc entries entirely, so a
//! source bound earlier in the same list had no model in the
//! output-side target graph and `fd_target` returned None — EBADF.
//! The ambient form (`exec 9<f; echo hi 10>&9`) passed, which is
//! why only same-list bindings broke. The mirror shape
//! (`9>f 10<&9`) was never affected: an output binding lands in the
//! graph directly. The fix records each input-side binding in the
//! walk at its list position and resolves a dup's source against
//! that positional record (a dup of a read-only description clones
//! the write-less model; a dup of a `<>` description clones the
//! shared read/write file description), and the spawn delivery's
//! numbered-dup walk stamps the whole resolved description (both
//! channels) instead of a single channel.
//!
//! Positionality is load-bearing and pinned both ways: a dup that
//! names its source BEFORE the binding entry still fails, exactly
//! as GNU's `echo hi 10>&9 9<f9.txt` does.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes (cluster workdir probes/: the shape matrix,
//! chain, rw-write, colon, external faces).

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");
const FOREIGN: &str = "/usr/bin/bash";

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2562-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    fs::write(dir.join("f9.txt"), "line1\nline2\n").expect("write f9");
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

#[test]
fn issue2562_dup_of_same_list_input_binding() {
    let dir = scratch("input-dup");
    write_script(&dir, "case.sh", "echo hi 9<f9.txt 10>&9\necho \"rc=$?\"\n");
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn issue2562_dup_of_same_list_readwrite_binding() {
    let dir = scratch("rw-dup");
    write_script(&dir, "case.sh", "echo hi 9<>f9.txt 10>&9\necho \"rc=$?\"\n");
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn issue2562_dup_clones_the_bound_description_for_reading() {
    // The dup must clone the description, not merely pass
    // validation: cat reads the file's bytes through fd 10, the dup
    // of the fd-9 input binding.
    let dir = scratch("clone-reads");
    write_script(&dir, "case.sh", "cat 9<f9.txt 10>&9 <&10\necho \"rc=$?\"\n");
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "line1\nline2\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn issue2562_readwrite_dup_carries_the_write_arm_and_offset() {
    // A dup of a `<>` binding clones the O_RDWR description WITH
    // its shared offset: the write through `1>&10` lands at offset
    // 0 of the same file.
    let dir = scratch("rw-write");
    write_script(
        &dir,
        "case.sh",
        "echo data 9<>f9.txt 10>&9 1>&10\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "f9.txt"), "data\n\nline2\n");
}

#[test]
fn issue2562_dup_chain_through_same_list_bindings() {
    let dir = scratch("chain");
    write_script(
        &dir,
        "case.sh",
        "echo hi 9<f9.txt 10>&9 11>&10\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn issue2562_dup_before_its_binding_still_fails() {
    // Positionality control: the dup precedes the binding entry,
    // so fd 9 is not open at the dup's position — GNU rejects this
    // too, with exactly one diagnostic and status 1.
    let dir = scratch("dup-first");
    write_script(&dir, "case.sh", "echo hi 10>&9 9<f9.txt\necho \"rc=$?\"\n");
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert_eq!(
        stderr, "case.sh: line 1: 9: Bad file descriptor\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2562_mirror_output_binding_dup_control() {
    // Control: the mirror shape (output binding, input dup) was
    // never broken.
    let dir = scratch("mirror");
    write_script(&dir, "case.sh", "echo hi 9>out.txt 10<&9\necho \"rc=$?\"\n");
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "out.txt"), "");
}

#[test]
fn issue2562_no_output_builtin_dup_of_same_list_input() {
    let dir = scratch("colon");
    write_script(&dir, "case.sh", ": 9<f9.txt 10>&9\necho \"rc=$?\"\n");
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn issue2562_external_dup_of_same_list_input() {
    let dir = scratch("external");
    write_script(
        &dir,
        "case.sh",
        &format!("{FOREIGN} -c 'exit 0' 9<f9.txt 10>&9\necho \"rc=$?\"\n"),
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}
