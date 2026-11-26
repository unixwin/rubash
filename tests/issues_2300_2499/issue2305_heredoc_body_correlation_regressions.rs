//! rubash#2305 — heredoc bodies correlated to the wrong redirect
//! under compound commands.
//!
//! Three layers of one defect family: a heredoc body did not stay
//! attached to its own redirect node.
//!
//! 1. PARSE-TIME PAIRING. GNU gathers bodies in the order the `<<`
//!    operators were read (parse.y gather_here_documents) and each
//!    body belongs to its own REDIRECT node from creation
//!    (make_cmd.c make_here_document; redir.c:423
//!    here_document_to_fd materializes that node's redirectee).
//!    rubash filled the first still-pending slot in a scan that
//!    visited a compound node's OWN heredoc entries before its
//!    members — but a compound's own `<<` is written after its
//!    closing delimiter, so it is LAST in operator order while its
//!    body arrives after every member's. The outer entry stole the
//!    first body and the members' bodies shifted by one (crossed).
//!    The fill walk now claims slots in operator order: nested
//!    commands first, the node's own entries last. The
//!    trailing-redirection collector no longer fills bodies at
//!    all — when it runs, the earliest pending operator may belong
//!    to an earlier sibling subtree (two groups on one line), so
//!    it defers every body to the token loop's global pairing.
//!
//! 2. IN-PROCESS SERVE. GNU applies a command's own redirect list
//!    over the ambient descriptor state before the builtin runs
//!    (redir.c:237 do_redirections; scoped lists restore via the
//!    undo list, execute_cmd.c:526 undo_partial_redirects). The
//!    `read -u N` serve consulted each store in a fixed per-kind
//!    order — own files, coproc, the AMBIENT table entry, own
//!    heredocs last — so an ambient same-fd binding shadowed the
//!    command's own heredoc/dup, and a mixed own list served an
//!    earlier entry of a luckier kind. The serve now resolves the
//!    own list's effective binding (last input-side entry for the
//!    fd, across kinds) first: heredocs and dups serve before the
//!    ambient transports, and an own close makes the fd invalid
//!    even over an ambient entry.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes (cluster workdir probes/: headline, q1–q7,
//! w2–w4, s1–s3, m1–m6, n1–n9). Spawn faces use a nested rubash
//! observer whose external `cat` reads fd 3 — a real spawn between
//! the binding and the read, never an ENOEXEC script child.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2305-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    fs::write(dir.join("inner.txt"), "inner-body\n").expect("write inner");
    fs::write(dir.join("outer2.txt"), "outer-line1\nouter-line2\n").expect("write outer2");
    dir
}

fn write_script(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, body).expect("write fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fixture");
    }
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

/// Foreign observer: a nested rubash whose own external `cat`
/// reads fd 3 — a real spawned process, so nothing passes through
/// the parent's in-process fd table.
fn obs3() -> String {
    format!("\"{RUBASH}\" -c 'cat <&3'")
}

// ---- (1) parse-time pairing under compounds ---------------------

#[test]
fn issue2305_nested_group_in_process_reads_inner_body() {
    let dir = scratch("nested-inproc");
    write_script(
        &dir,
        "case.sh",
        "{ { IFS= read -r -u 3 line; echo \"got: $line\"; } 3<<INNER; } 3<<OUTER\ninner-body\nINNER\nouter-body\nOUTER\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "got: inner-body\n");
}

#[test]
fn issue2305_nested_group_spawn_reads_inner_body() {
    let dir = scratch("nested-spawn");
    let script = format!(
        "{{ {{ {}; }} 3<<INNER; }} 3<<OUTER\ninner-body\nINNER\nouter-body\nOUTER\n",
        obs3()
    );
    write_script(&dir, "case.sh", &script);
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "inner-body\n");
}

#[test]
fn issue2305_group_sibling_heredocs_do_not_cross() {
    let dir = scratch("siblings");
    // First command's own heredoc, second command redirect-less:
    // GNU serves INNER then OUTER (the group's ambient binding,
    // restored intact after the sibling's scoped bind — redir.c:237
    // RX_UNDOABLE / execute_cmd.c:526). Pre-fix the two bodies
    // crossed: OUTER then INNER.
    let script = format!(
        "{{ {} 3<<INNER; {}; }} 3<<OUTER\ninner-body\nINNER\nouter-body\nOUTER\n",
        obs3(),
        obs3()
    );
    write_script(&dir, "case.sh", &script);
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "inner-body\nouter-body\n");
}

#[test]
fn issue2305_group_sibling_discarded_first_second_reads_outer() {
    let dir = scratch("sibling-discarded");
    let script = format!(
        "{{ {} 3<<INNER >/dev/null; {}; }} 3<<OUTER\ninner-body\nINNER\nouter-body\nOUTER\n",
        obs3(),
        obs3()
    );
    write_script(&dir, "case.sh", &script);
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "outer-body\n");
}

#[test]
fn issue2305_two_groups_on_one_line_pair_their_own_bodies() {
    let dir = scratch("two-groups");
    // Four operators on one line (AAA, BBB, CCC, DDD in stream
    // order): each member must receive its own group's member body.
    // Pre-fix the trailing-redirection collector filled the SECOND
    // group's subtree with the FIRST line's bodies.
    let script = format!(
        "{{ {} 3<<AAA; }} 3<<BBB; {{ {} 3<<CCC; }} 3<<DDD\naaa-body\nAAA\nbbb-body\nBBB\nccc-body\nCCC\nddd-body\nDDD\n",
        obs3(),
        obs3()
    );
    write_script(&dir, "case.sh", &script);
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "aaa-body\nccc-body\n");
}

// ---- (2) in-process serve: own list beats ambient ---------------

#[test]
fn issue2305_in_process_own_heredoc_shadows_ambient() {
    let dir = scratch("inproc-shadow");
    write_script(
        &dir,
        "case.sh",
        "{ read -u 3 a 3<<INNER; echo \"A:$a\"; read -u 3 b; echo \"B:$b\"; } 3<<OUTER\ninner-body\nINNER\nouter-body\nOUTER\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "A:inner-body\nB:outer-body\n");
}

#[test]
fn issue2305_in_process_own_heredoc_leaves_ambient_file_untouched() {
    let dir = scratch("inproc-ambient-file");
    // The ambient binding is a FILE with two lines: if the own
    // heredoc is wrongly ignored, the first read consumes line 1
    // and the second read gets line 2. GNU: the own heredoc serves
    // the first read and the ambient file is still at line 1 for
    // the second.
    write_script(
        &dir,
        "case.sh",
        "{ read -u 3 a 3<<INNER; echo \"A:$a\"; read -u 3 b; echo \"B:$b\"; } 3<outer2.txt\ninner-body\nINNER\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "A:inner-body\nB:outer-line1\n");
}

#[test]
fn issue2305_in_process_own_dup_shadows_ambient() {
    let dir = scratch("inproc-dup");
    write_script(
        &dir,
        "case.sh",
        "exec 4<inner.txt\n{ read -u 3 a 3<&4; echo \"A:$a\"; read -u 3 b; echo \"B:$b\"; } 3<<OUTER\nouter-body\nOUTER\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "A:inner-body\nB:outer-body\n");
}

#[test]
fn issue2305_in_process_mixed_own_list_last_entry_wins() {
    let dir = scratch("inproc-mixed");
    // `3<file 3<<EOF`: the heredoc is the LAST input-side entry
    // for fd 3, so it is the descriptor read serves (redir.c:237
    // ordered walk). The per-kind serve used to find the file
    // first regardless of list order.
    write_script(
        &dir,
        "case.sh",
        "read -u 3 a 3<inner.txt 3<<EOF; echo \"A:$a\"\neof-body\nEOF\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "A:eof-body\n");
}

#[test]
fn issue2305_in_process_own_close_invalidates_fd_over_ambient() {
    let dir = scratch("inproc-close");
    // The command closes fd 3 for itself: GNU read.def:377-381
    // sh_validfd judges the REDIRECTED descriptor — invalid even
    // though the ambient binding exists — and the ambient binding
    // is untouched for the next command.
    write_script(
        &dir,
        "case.sh",
        "{ read -u 3 a 3<&-; echo \"rc=$? A:[$a]\"; read -u 3 b; echo \"B:$b\"; } 3<<OUTER\nouter-body\nOUTER\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("read: 3: invalid file descriptor: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "rc=1 A:[]\nB:outer-body\n");
}

#[test]
fn issue2305_mapfile_own_heredoc_shadows_ambient() {
    let dir = scratch("mapfile-shadow");
    // mapfile runs the same per-kind serve order as read did: its
    // own heredoc lost to the ambient table entry (and drained
    // it). GNU: mapfile consumes its OWN body; the following read
    // still sees the ambient OUTER body intact.
    write_script(
        &dir,
        "case.sh",
        "{ mapfile -u 3 -t arr 3<<INNER; echo \"M:${arr[0]}\"; read -u 3 b; echo \"B:$b\"; } 3<<OUTER\ninner-body\nINNER\nouter-body\nOUTER\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "M:inner-body\nB:outer-body\n");
}

#[test]
fn issue2305_mapfile_own_dup_reads_source() {
    let dir = scratch("mapfile-dup");
    write_script(
        &dir,
        "case.sh",
        "exec 4<inner.txt\nmapfile -u 3 -t arr 3<&4; echo \"M:${arr[0]}\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "M:inner-body\n");
}

#[test]
fn issue2305_mapfile_own_close_invalidates_fd_over_ambient() {
    let dir = scratch("mapfile-close");
    write_script(
        &dir,
        "case.sh",
        "{ mapfile -u 3 -t arr 3<&-; echo \"rc=$? M:[${arr[0]:-unset}]\"; read -u 3 b; echo \"B:$b\"; } 3<<OUTER\nouter-body\nOUTER\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("mapfile: 3: invalid file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "rc=1 M:[unset]\nB:outer-body\n");
}

// ---- controls ----------------------------------------------------

#[test]
fn issue2305_simple_command_control() {
    let dir = scratch("control-simple");
    let script = format!("{} 3<<EOF\nsolo-body\nEOF\n", obs3());
    write_script(&dir, "case.sh", &script);
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "solo-body\n");
}

#[test]
fn issue2305_function_control() {
    let dir = scratch("control-func");
    write_script(
        &dir,
        "case.sh",
        "f() { read -u 3 l; echo \"F:$l\"; }\n{ f 3<<INNER; read -u 3 m; echo \"M:$m\"; } 3<<OUTER\ninner-body\nINNER\nouter-body\nOUTER\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "F:inner-body\nM:outer-body\n");
}

#[test]
fn issue2305_file_bind_control() {
    let dir = scratch("control-file");
    // Sibling binds fd 3 to a File instead of a heredoc: the
    // redirect-less second command reads the group's OUTER body.
    // This face was already correct pre-fix and must stay correct.
    let script = format!(
        "{{ {} 3<inner.txt >/dev/null; {}; }} 3<<OUTER\nouter-body\nOUTER\n",
        obs3(),
        obs3()
    );
    write_script(&dir, "case.sh", &script);
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "outer-body\n");
}
