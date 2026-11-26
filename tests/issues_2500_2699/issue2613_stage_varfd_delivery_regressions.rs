//! Issue unixwin/rubash#2613: a pipeline stage whose redirect list
//! references, by number, the descriptor an earlier `{var}` entry in
//! the SAME list allocated died with `10: Bad file descriptor` and
//! never ran.
//!
//! GNU runs ONE do_redirections walk per pipeline element
//! (execute_cmd.c:2620 execute_pipeline -> redir.c:237
//! do_redirections), and the `{var}` entries allocate their
//! descriptor inside that walk (redir.c:1470 redir_varassign), so a
//! later entry may reference the allocated descriptor by number:
//! `{v}>&9 1>&10` is an ordinary list. rubash's stage faces judged
//! the list in three different descriptor spaces — the stage reject
//! gate's table-plus-numbered-overlay, the output preflight's seeded
//! state, and the post-execution routing walk's seeded state — and
//! none of them contained the applier's allocation, because the
//! stage executors undo the dynamic-fd applier before those walks
//! run (or, for the gates, run before it). The plain-command funnel
//! runs the applier ahead of the same faces, which is why the same
//! lists worked outside pipelines.
//!
//! The fix brackets each stage auxiliary walk with the SAME
//! applier the funnel uses (apply -> judge -> undo), so every face
//! sees the one bound descriptor space. These pins assert the
//! GNU-expected behavior for the fd-mechanism audit's W3 shapes
//! S02 / S04 / S12 in both pipeline positions, the audit's C1
//! reproducer verbatim, and the builtin / function stage faces of
//! the same class.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2613-{tag}-{}", std::process::id()));
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

/// Assert the S02 side conditions: the stage ran, its payload
/// reached seed.txt through the `{var}` slot, the numbered `9>o1.txt`
/// rebind created o1.txt empty, and no diagnostic leaked.
fn assert_s02(dir: &Path, stdout: &str, stderr: &str, code: Option<i32>) {
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "PS=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
    assert!(
        wait_file_contents(&dir.join("seed.txt"), "W10\n"),
        "seed.txt must hold the stage payload routed through the {{var}} slot; got {:?}",
        std::fs::read_to_string(dir.join("seed.txt"))
    );
    assert!(
        wait_file_contents(&dir.join("o1.txt"), ""),
        "o1.txt must exist (the 9>o1.txt open) and stay empty"
    );
}

#[test]
fn issue2613_s02_pipe_left_var_dup_then_numbered_rebind_and_reference() {
    // Audit shape S02, stage on the left: `{v}>&9` allocates the
    // var's descriptor, `9>o1.txt` rebinds the source, and `1>&10`
    // references the allocation by number. GNU runs the stage and
    // the payload lands in seed.txt.
    let dir = test_dir("s02-left");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         sh -c 'echo W10' {v}>&9 9>o1.txt 1>&10 | cat >/dev/null\n\
         echo \"PS=${PIPESTATUS[0]}\"\n",
    );
    assert_s02(&dir, &stdout, &stderr, code);
}

#[test]
fn issue2613_s02_pipe_right_var_dup_then_numbered_rebind_and_reference() {
    // Audit shape S02, stage on the right of the pipe.
    let dir = test_dir("s02-right");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         cat /dev/null | sh -c 'echo W10' {v}>&9 9>o1.txt 1>&10\n\
         echo \"PS=${PIPESTATUS[1]}\"\n",
    );
    assert_s02(&dir, &stdout, &stderr, code);
}

#[test]
fn issue2613_s04_pipe_left_var_dup_then_close_source_and_reference() {
    // Audit shape S04, stage on the left: `{v}>&9 9>&- 1>&10` — the
    // close of the source fd is the element's own (GNU runs the list
    // in the fork), so the parent shell's fd 9 survives: the POST
    // write through the parent's 9 must still land in seed.txt.
    let dir = test_dir("s04-left");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         sh -c 'echo W10' {v}>&9 9>&- 1>&10 | cat >/dev/null\n\
         echo \"PS=${PIPESTATUS[0]}\"\n\
         echo POST >&9\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "PS=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
    assert!(
        wait_file_contents(&dir.join("seed.txt"), "W10\nPOST\n"),
        "seed.txt must hold the payload plus the parent-side POST; got {:?}",
        std::fs::read_to_string(dir.join("seed.txt"))
    );
}

#[test]
fn issue2613_s04_pipe_right_var_dup_then_close_source_and_reference() {
    // Audit shape S04, stage on the right of the pipe.
    let dir = test_dir("s04-right");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         cat /dev/null | sh -c 'echo W10' {v}>&9 9>&- 1>&10\n\
         echo \"PS=${PIPESTATUS[1]}\"\n\
         echo POST >&9\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "PS=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
    assert!(
        wait_file_contents(&dir.join("seed.txt"), "W10\nPOST\n"),
        "seed.txt must hold the payload plus the parent-side POST; got {:?}",
        std::fs::read_to_string(dir.join("seed.txt"))
    );
}

#[test]
fn issue2613_s12_pipe_left_move_form_reference_parent_keeps_source() {
    // Audit shape S12, stage on the left: the move form `{v}>&9-`
    // allocates the var's descriptor from fd 9 and closes 9 IN THE
    // ELEMENT only — GNU's parent shell keeps its fd 9 open after
    // the pipeline, so POST still lands in seed.txt.
    let dir = test_dir("s12-left");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         sh -c 'echo W10' {v}>&9- 1>&10 | cat >/dev/null\n\
         echo \"PS=${PIPESTATUS[0]}\"\n\
         echo POST >&9\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "PS=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
    assert!(
        wait_file_contents(&dir.join("seed.txt"), "W10\nPOST\n"),
        "seed.txt must hold the payload plus the parent-side POST; got {:?}",
        std::fs::read_to_string(dir.join("seed.txt"))
    );
}

#[test]
fn issue2613_s12_pipe_right_move_form_reference_parent_keeps_source() {
    // Audit shape S12, stage on the right of the pipe.
    let dir = test_dir("s12-right");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         cat /dev/null | sh -c 'echo W10' {v}>&9- 1>&10\n\
         echo \"PS=${PIPESTATUS[1]}\"\n\
         echo POST >&9\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "PS=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
    assert!(
        wait_file_contents(&dir.join("seed.txt"), "W10\nPOST\n"),
        "seed.txt must hold the payload plus the parent-side POST; got {:?}",
        std::fs::read_to_string(dir.join("seed.txt"))
    );
}

#[test]
#[cfg(unix)]
fn issue2613_headline_c1_ebadf_stage_varfd_delivery() {
    // The audit's C1 reproducer verbatim (fd-mechanism audit,
    // reproducers/C1-pipe-stage.sh): on master this died with
    // `10: Bad file descriptor`, the stage never ran, and seed.txt
    // stayed empty. GNU runs it and seed.txt receives the payload.
    let dir = test_dir("c1");
    std::fs::write(dir.join("w1.sh"), "#!/bin/sh\necho W10\n").expect("write w1.sh");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("w1.sh"), std::fs::Permissions::from_mode(0o755))
            .expect("chmod w1.sh");
    }
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         ./w1.sh {v}>&9 9>o1.txt 1>&10 | cat >/dev/null\n\
         echo \"PS=${PIPESTATUS[0]}\"\n",
    );
    assert!(
        !stderr.contains("Bad file descriptor"),
        "the EBADF headline diagnostic must be gone; stderr: {stderr}"
    );
    assert_s02(&dir, &stdout, &stderr, code);
}

#[test]
fn issue2613_builtin_stage_var_dup_reference() {
    // The builtin stage face of the same class: a builtin element's
    // own preflight judged the list without the applier's bindings.
    let dir = test_dir("builtin");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         echo W10 {v}>&9 1>&10 | cat >/dev/null\n\
         echo \"PS=${PIPESTATUS[0]}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "PS=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
    assert!(
        wait_file_contents(&dir.join("seed.txt"), "W10\n"),
        "seed.txt must hold the builtin's payload; got {:?}",
        std::fs::read_to_string(dir.join("seed.txt"))
    );
}

#[test]
fn issue2613_function_stage_var_dup_reference() {
    // The function stage face of the same class.
    let dir = test_dir("function");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>seed.txt\n\
         f() { echo W10; }\n\
         f {v}>&9 1>&10 | cat >/dev/null\n\
         echo \"PS=${PIPESTATUS[0]}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "PS=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
    assert!(
        wait_file_contents(&dir.join("seed.txt"), "W10\n"),
        "seed.txt must hold the function's payload; got {:?}",
        std::fs::read_to_string(dir.join("seed.txt"))
    );
}
