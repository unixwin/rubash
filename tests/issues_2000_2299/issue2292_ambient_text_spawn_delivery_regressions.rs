//! rubash#2292 — heredoc Text endpoints were excluded from spawn
//! delivery on every face.
//!
//! Two halves of one mechanism — a Text (heredoc / here-string)
//! binding was invisible to the machinery that hands descriptors
//! to a child:
//!
//! 1. AMBIENT bindings. A heredoc bound by an enclosing face (a
//!    brace group, a function call, a script's own redirects, a
//!    persistent `exec`) lives in the fd table as a Text endpoint.
//!    The ambient spawn delivery (the `__RUBASH_FD_*` stamp and
//!    `ambient_fd_pins_for`) only carries endpoints backed by a
//!    real handle, so a foreign grandchild spawned under the
//!    compound inherited nothing at the slot and read EBADF. GNU's
//!    forked child inherits the descriptor here_document_to_fd
//!    created (redir.c:423) because do_redirections (redir.c:237)
//!    runs in every fork. The fix materializes ambient Text
//!    endpoints into carrier files in the shared delivery segment
//!    (the #2272 {var} trick), skipping every slot the command's
//!    own list, its own heredoc_redirects, or a {var} binding owns.
//!
//! 2. OWN-LIST bindings (Discovery B). A command's own heredoc
//!    entry recorded no ordered fd effect in the reject gate
//!    (`reject_invalid_redirects`) and was kind-skipped by the fd-0
//!    dup replay (`own_input_binding_before`), so a later `<&N`
//!    dup of the heredoc's fd in the SAME list was rejected with
//!    `N: Bad file descriptor` (`cat 3<<EOF <<EOF2 <&3`). GNU's
//!    single ordered walk binds the heredoc first (redir.c:423,
//!    dup2 at redir.c:1065-1072) and the dup copies it.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with the
//! probe cells in the cluster workdir (probes/p*.sh, probes/b*.sh,
//! probes/d*.sh, probes/s*.sh): every cell was run with a foreign
//! observer (python3) reading the descriptor it actually
//! received. The pins below use a nested rubash as the observer —
//! a real spawned process, never an ENOEXEC script child (which
//! would share the parent's fd table and pass vacuously).
//!
//! NOT pinned (documented in the cluster REPORT.md): the
//! compound-NESTED overlap (`{ cmd 3<<INNER; } 3<<OUTER`, and its
//! different-fd and in-process variants) delivers the OUTER body
//! on current master for a separate, pre-existing reason —
//! heredoc body correlation under nesting fails on in-process
//! faces too (a nested group's `read -u 3` also reads OUTER), so
//! it is not this issue's spawn-delivery mechanism. The overlap
//! pins below use `exec`-installed ambient bindings, where the
//! no-clobber property of THIS fix is observable.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2292-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    fs::write(dir.join("seed.txt"), "seed-line\n").expect("write seed");
    fs::write(dir.join("inner.txt"), "inner-file-line\n").expect("write inner");
    fs::write(dir.join("outer.txt"), "outer-file-line\n").expect("write outer");
    dir
}

fn write(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, body).expect("write fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if name.ends_with(".sh") {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fixture");
        }
    }
}

/// Run `script` as a `-c` string in `dir` with stdin closed (null).
fn rubash_in(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
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
/// reads fd 3 — a real spawn between the binding and the read, so
/// nothing passes through the parent's in-process fd table.
fn obs3() -> String {
    format!("\"{RUBASH}\" -c 'cat <&3'")
}

/// Foreign observer reading its stdin (fd 0).
fn obs0() -> String {
    format!("\"{RUBASH}\" -c 'cat'")
}

// ---- (a) compound faces: ambient Text materialization ----------

#[test]
fn issue2292_group_heredoc_reaches_spawned_grandchild() {
    let dir = scratch("group");
    let script = format!("{{ {}; }} 3<<EOF\ngroup-heredoc-body\nEOF\n", obs3());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "group-heredoc-body\n");
}

#[test]
fn issue2292_function_heredoc_reaches_spawned_grandchild() {
    let dir = scratch("func");
    let script = format!("f() {{ {}; }}\nf 3<<EOF\nfunc-heredoc-body\nEOF\n", obs3());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "func-heredoc-body\n");
}

#[test]
fn issue2292_script_heredoc_reaches_spawned_grandchild() {
    let dir = scratch("script");
    // The script child is the SUBJECT (rubash executes it
    // in-process — the face under test); the observer inside it is
    // the nested rubash, a real spawn.
    write(&dir, "child.sh", &format!("{}\n", obs3()));
    let script = "./child.sh 3<<EOF\nscript-heredoc-body\nEOF\n".to_string();
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "script-heredoc-body\n");
}

#[test]
fn issue2292_exec_heredoc_reaches_spawned_child() {
    let dir = scratch("exec");
    let script = format!("exec 3<<EOF\nexec-heredoc-body\nEOF\n{}\n", obs3());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "exec-heredoc-body\n");
}

#[test]
fn issue2292_group_herestring_reaches_spawned_grandchild() {
    let dir = scratch("herestring-group");
    let script = format!("{{ {}; }} 3<<<compound-herestring-body\n", obs3());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "compound-herestring-body\n");
}

#[test]
fn issue2292_group_heredoc_reaches_pipeline_stage_grandchild() {
    let dir = scratch("stage");
    let script = format!("{{ {}; }} 3<<EOF | cat\ngroup-heredoc-body\nEOF\n", obs3());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "group-heredoc-body\n");
}

#[test]
fn issue2292_group_second_command_reads_ambient_heredoc() {
    let dir = scratch("second");
    // The first body command binds its own fd 3 to a file (its
    // read is discarded); the second has no redirects and must
    // inherit the group's ambient binding, intact after the
    // sibling's scoped bind/restore. On the pre-fix tree the
    // second command read EBADF.
    //
    // The sibling bind is a FILE on purpose. The heredoc variant
    // (`{ cmd 3<<INNER >/dev/null; cmd; } 3<<OUTER`) is NOT this
    // issue's shape: heredoc body correlation under a compound
    // crosses the two bodies on current master — the first
    // command is served OUTER, the second INNER, and the
    // in-process face loses the ambient body too. That is the
    // pre-existing nested-heredoc defect tracked as rubash#2305
    // (cluster probes p9/p9d/p9f; adjudication in the cluster
    // REPORT-ADDENDUM.md), not #2292's spawn-delivery mechanism:
    // GNU scopes each list's redirects with an undo list
    // (redir.c:237 do_redirections, RX_UNDOABLE; the body
    // travels with its own redirect node, redir.c:423
    // here_document_to_fd), so both pins' expectations are
    // GNU's — only the mechanism under test differs.
    let script = format!(
        "{{ {} 3<inner.txt >/dev/null; {}; }} 3<<OUTER\nouter-body\nOUTER\n",
        obs3(),
        obs3()
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "outer-body\n");
}

// ---- controls: faces that already delivered --------------------

#[test]
fn issue2292_plain_heredoc_control() {
    let dir = scratch("plain");
    let script = format!("{} 3<<EOF\nplain-numonly-body\nEOF\n", obs3());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "plain-numonly-body\n");
}

#[test]
fn issue2292_plain_heredoc_with_fd0_heredoc_control() {
    let dir = scratch("plain-both");
    let script = format!(
        "{} 3<<EOF <<EOF2\nplain-fd3-body\nEOF\nplain-fd0-body\nEOF2\n",
        obs3()
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "plain-fd3-body\n");
}

#[test]
fn issue2292_plain_herestring_control() {
    let dir = scratch("herestring-plain");
    let script = format!("{} 3<<<plain-herestring-body\n", obs3());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "plain-herestring-body\n");
}

#[test]
fn issue2292_group_file_control() {
    let dir = scratch("file-control");
    let script = format!("{{ {}; }} 3<seed.txt\n", obs3());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line\n");
}

// ---- (b) Discovery B: own-list heredoc + later dup -------------

#[test]
fn issue2292_own_heredoc_dup_to_stdin() {
    let dir = scratch("dup-stdin");
    // `cat 3<<EOF <<EOF2 <&3` shape (observer = nested cat on
    // fd 0): the dup copies the fd-3 heredoc body.
    let script = format!(
        "{} 3<<EOF <<EOF2 <&3\nfd3-body\nEOF\nfd0-body\nEOF2\n",
        obs0()
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd3-body\n");
}

#[test]
fn issue2292_own_heredoc_dup_superseded_by_later_heredoc() {
    let dir = scratch("dup-superseded");
    // The dup applies (and must not be rejected) even though a
    // later fd-0 heredoc wins the slot: GNU applies the whole
    // list in order.
    let script = format!(
        "{} 3<<EOF <&3 <<EOF2\nfd3-body\nEOF\nfd0-body\nEOF2\n",
        obs0()
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fd0-body\n");
}

#[test]
fn issue2292_dup_before_heredoc_bind_still_ebadf() {
    let dir = scratch("dup-before-bind");
    // Negative control, GNU-matching: the dup precedes the
    // binding in the list, so it fails exactly as GNU's does —
    // the gate fix records the heredoc's effect at its own list
    // position, not before it.
    let script = format!("{} <&3 3<<EOF\nfd3-body\nEOF\n", obs0());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(1), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("3: Bad file descriptor"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2292_own_heredoc_dup_builtin_face() {
    let dir = scratch("dup-builtin");
    // The reject gate is the shared funnel's, so the builtin face
    // is unblocked by the same recording: `read` dup's the fd-3
    // heredoc body onto its stdin.
    let script =
        "IFS= read -r line 3<<EOF <&3\nbuiltin-fd3-body\nEOF\necho \"read: $line\"\n".to_string();
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "read: builtin-fd3-body\n");
}

// ---- (d) overlap: own binding owns the slot --------------------

#[test]
fn issue2292_overlap_ambient_text_vs_own_file() {
    let dir = scratch("overlap-text-file");
    // Ambient Text at fd 3 (exec heredoc); the command's own file
    // binding owns the slot — the ambient pass must not clobber
    // it with the outer body.
    let script = format!("exec 3<<OUTER\nouter-body\nOUTER\n{} 3<inner.txt\n", obs3());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "inner-file-line\n");
}

#[test]
fn issue2292_overlap_ambient_file_vs_own_heredoc() {
    let dir = scratch("overlap-file-heredoc");
    let script = format!("exec 3<outer.txt\n{} 3<<INNER\ninner-body\nINNER\n", obs3());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "inner-body\n");
}

#[test]
fn issue2292_overlap_ambient_text_vs_own_heredoc() {
    let dir = scratch("overlap-text-heredoc");
    // The direct no-clobber cell for the ambient pass: without
    // its walk-ownership skip, the outer body would be stamped
    // at fd 3 alongside (or instead of) the command's own
    // heredoc.
    let script = format!(
        "exec 3<<OUTER\nouter-body\nOUTER\n{} 3<<INNER\ninner-body\nINNER\n",
        obs3()
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "inner-body\n");
}
