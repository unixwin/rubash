//! Issue unixwin/rubash#3991: a redirect walk's undo save was
//! never materialized as a resolvable descriptor.
//!
//! GNU's do_redirections (redir.c:237) registers an undo save for
//! every entry that rebinds an open descriptor, and the save is a
//! REAL descriptor — a dup taken at the lowest free slot at or
//! above SHELL_FD_BASE (10), add_undo_redirect (redir.c:1296) —
//! so a later entry of the SAME list can dup from the save slot:
//! `echo W10 9>o1.txt 1>&10` after `exec 9>seed.txt` copies the
//! save of fd 9 (at slot 10) into fd 1 and the payload lands in
//! seed.txt. rubash computed the save slots only as an occupied
//! set for `{var}` allocation (fd_walk_undo_save_slots) and no
//! walk could resolve a descriptor there, so the dup failed with
//! `10: Bad file descriptor` on every undoable face (plain
//! builtin, function call, compound, subshell body, comsub,
//! exec's own list). The fork-child walks (pipeline stages,
//! external commands) are RX_ACTIVE in GNU and earn no saves —
//! the same dup fails there on both sides, and these pins hold
//! that boundary too.
//!
//! The fix registers the saves in the one shared walk space
//! (fd_walk_register_saves) and materializes them at their
//! entry's position in every undoable walk: the reject gate, the
//! `{var}` applier, the buffered-output routing walk, the
//! compound input/output binders, and the exec walk (whose saves
//! are disposed when the list stands).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3991-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

/// Run `body` as a script inside `dir`.
fn run_in(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    rubash_stdin(&format!("cd {} || exit 1\n{body}", dir.display()))
}

/// Poll until `path` has exactly `expected` contents or the deadline.
fn wait_file_contents(path: &Path, expected: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if std::fs::read_to_string(path).is_ok_and(|text| text == expected) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::fs::read_to_string(path).is_ok_and(|text| text == expected)
}

fn assert_file(dir: &Path, name: &str, expected: &str) {
    assert!(
        wait_file_contents(&dir.join(name), expected),
        "{name} must hold {expected:?}; got {:?}",
        std::fs::read_to_string(dir.join(name))
    );
}

#[test]
fn issue3991_c15_plain_builtin_dup_from_save() {
    // The issue's minimal reproducer: the save of fd 9 sits at
    // slot 10 and `1>&10` dups it, so the payload lands in the
    // file fd 9 held BEFORE the list rebound it.
    let dir = test_dir("c15");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         echo W10 9>o1.txt 1>&10\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
    assert_file(&dir, "seed.txt", "W10\n");
    assert_file(&dir, "o1.txt", "");
}

#[test]
fn issue3991_c15_two_saves_claim_in_order() {
    // Two rebinds earn two saves at 10 and 11; the dup names the
    // second save (of fd 8), so the payload lands in seed2.txt.
    let dir = test_dir("c15-two");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         exec 8>seed2.txt\n\
         echo W10 9>o1.txt 8>o2.txt 1>&11\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_file(&dir, "seed2.txt", "W10\n");
    assert_file(&dir, "seed.txt", "");
}

#[test]
fn issue3991_c15_close_entry_earns_the_save() {
    // A close entry's undo record is the save itself: `9>&-`
    // saves fd 9 at slot 10 before closing it, and `1>&10` still
    // dups the saved description.
    let dir = test_dir("c15-close");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         echo W10 9>&- 1>&10\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_file(&dir, "seed.txt", "W10\n");
}

#[test]
fn issue3991_c15_save_slot_shadowed_by_real_bind() {
    // A later entry that binds the save slot for real wins over
    // the save (GNU's walk clobbers the save with the bind).
    let dir = test_dir("c15-shadow");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         echo W10 9>o1.txt 10>o10.txt 1>&10\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_file(&dir, "o10.txt", "W10\n");
    assert_file(&dir, "seed.txt", "");
}

#[test]
fn issue3991_c15_no_save_at_unnamed_slot() {
    // Negative control: nothing earned a save at slot 12, so the
    // dup still fails exactly like GNU.
    let dir = test_dir("c15-negative");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         echo W10 9>o1.txt 1>&12\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("12: Bad file descriptor"),
        "stderr must name the bad dup source; got {stderr:?}"
    );
    assert_file(&dir, "seed.txt", "");
}

#[test]
fn issue3991_c15_var_allocation_counts_the_save() {
    // The save claims slot 10, so the `{v}` allocation in the
    // same list takes 11 — and a dup FROM the save slot in the
    // same entry list works (the c15g shape).
    let dir = test_dir("c15-var");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         echo W10 9>o1.txt {v}>&10\n\
         echo v=$v\n\
         echo PAYLOAD >&$v\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    // fd 1 itself is never rebound in this list, so W10 reaches
    // the shell's stdout; the var's descriptor carries the save.
    assert_eq!(stdout, "W10\nv=11\n", "stderr: {stderr}");
    assert_file(&dir, "seed.txt", "PAYLOAD\n");
    assert_file(&dir, "o1.txt", "");
}

#[test]
fn issue3991_f15_function_call_dup_from_save() {
    // A function call's list walks undoable in the shell
    // (execute_cmd.c:5606), so its saves are live for the list
    // itself.
    let dir = test_dir("f15");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         f() { echo W10; }\n\
         f 9>o1.txt 1>&10\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_file(&dir, "seed.txt", "W10\n");
    assert_file(&dir, "o1.txt", "");
}

#[test]
fn issue3991_f15n_function_call_input_dup_from_save() {
    // The input face of the function-call list: the save of fd 9
    // (holding in.txt) feeds the call's fd 0 through `0<&10`.
    let dir = test_dir("f15n");
    std::fs::write(dir.join("in.txt"), "hello\n").expect("seed in.txt");
    std::fs::write(dir.join("o1.txt"), "").expect("seed o1.txt");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         f() { read x; echo \"[$x]\"; }\n\
         f 9<o1.txt 0<&10\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "[hello]\nrc=0\n", "stderr: {stderr}");
}

#[test]
fn issue3991_c15n_group_input_dup_from_save() {
    // The input face on a brace group's list.
    let dir = test_dir("c15n-group");
    std::fs::write(dir.join("in.txt"), "hello\n").expect("seed in.txt");
    std::fs::write(dir.join("o1.txt"), "").expect("seed o1.txt");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         { read x; echo \"[$x]\"; } 9<o1.txt 0<&10\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "[hello]\nrc=0\n", "stderr: {stderr}");
}

#[test]
fn issue3991_c15_group_output_dup_from_save() {
    let dir = test_dir("c15-group");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         { echo W10; } 9>o1.txt 1>&10\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_file(&dir, "seed.txt", "W10\n");
}

#[test]
fn issue3991_c15_subshell_body_dup_from_save() {
    // A subshell BODY's commands walk undoable in the child (the
    // subshell's own list is the active one; the body's lists are
    // ordinary in-child walks).
    let dir = test_dir("c15-subshell");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         ( echo W10 9>o1.txt 1>&10 )\n\
         echo rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_file(&dir, "seed.txt", "W10\n");
}

#[test]
fn issue3991_c15_comsub_dup_from_save() {
    let dir = test_dir("c15-comsub");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         x=$(echo W10 9>o1.txt 1>&10)\n\
         echo \"[$x] rc=$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "[] rc=0\n", "stderr: {stderr}");
    assert_file(&dir, "seed.txt", "W10\n");
}

#[test]
fn issue3991_e15_exec_list_dup_from_save_and_no_leak() {
    // exec's own list is undoable while it applies
    // (execute_cmd.c:5619): `1>&10` dups the save of fd 9. The
    // saves are disposed when the list stands, so a later `>&10`
    // finds no descriptor — the save must not leak.
    let dir = test_dir("e15");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         exec 3>&1\n\
         exec 9>o1.txt 1>&10\n\
         echo W10\n\
         ( echo x >&10 ) 2>/dev/null\n\
         echo leak_rc=$? >&3\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "leak_rc=1\n", "stderr: {stderr}");
    assert_file(&dir, "seed.txt", "W10\n");
    assert_file(&dir, "o1.txt", "");
}

#[test]
fn issue3991_stage_walk_is_active_no_save() {
    // Boundary pin: a pipeline element's walk is the fork
    // child's RX_ACTIVE pass — no save exists there, so the
    // stage's dup from the would-be save slot fails in GNU and
    // must fail here too (the payload must NOT reach seed.txt).
    let dir = test_dir("stage-active");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         echo W10 9>o1.txt 1>&10 | cat\n\
         echo rc=${PIPESTATUS[0]}\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("10: Bad file descriptor"),
        "stderr must name the bad dup source; got {stderr:?}"
    );
    assert_file(&dir, "seed.txt", "");
}

#[test]
fn issue3991_stage_var_allocation_counts_no_save() {
    // Boundary pin, allocation side: in a pipeline element the
    // `{var}` allocation takes slot 10 itself (no save claims
    // it), and a body dup from 10 reads the var's binding.
    let dir = test_dir("stage-var");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         { echo \"v=$v\"; echo W10 1>&10; } 9>o1.txt {v}>&9 | cat\n\
         echo rc=${PIPESTATUS[0]}\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "v=10\nrc=0\n", "stderr: {stderr}");
    assert_file(&dir, "o1.txt", "W10\n");
    assert_file(&dir, "seed.txt", "");
}
