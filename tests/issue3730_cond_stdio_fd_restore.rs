//! rubash#3730: after `[[ ... ]] >file` (or `2>file`, `1>&-`,
//! `2>&1`), fd 1/2 is not restored to its pre-conditional target —
//! the next write is lost (EBADF on the retired descriptor) or
//! lands on the `[[ ]]` redirect's target.
//!
//! GNU ground truth (bash 5.3.20): the conditional command's own
//! redirections are undoable — `execute_cond_command` stamps its
//! line only once its redirections have bound
//! (execute_cmd.c:4138 `SET_LINE_NUMBER (cond_command->line)`), and
//! the undo save IS a kernel duplicate of the displaced descriptor
//! (`fcntl (fd, F_DUPFD, SHELL_FD_BASE)`, redir.c:965/1082/1144,
//! recorded by `add_undo_redirect`), so the pre-`[[ ]]` descriptor
//! stays alive for the test's duration and `undo_redirections` dups
//! it back. Rubash's scope save only cloned the userspace entry
//! (`command_prepare.rs:370-371`), so the bind-time retire
//! (`retire_write_arm`, rubash#3213) closed the very descriptor the
//! restore was about to replay — the same hazard #3706 isolated for
//! group redirects (#3723 fixes it there), on the conditional-command
//! stdio face.
//!
//! The hazard needs fd 1/2 File-backed: at birth fd 1/2 are the
//! symbolic `Stdout`/`Stderr` markers, which no retire touches — so
//! every cell first runs `exec 1>outer.txt` / `exec 2>err.txt` to
//! make the slot File-backed, then a `[[ ]]` redirect displaces it.
//! Every cell except the plain-conditional control is fail-first on
//! the unfixed base (the post-`[[ ]]` write is lost or misdirected).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(
    tag: &str,
    driver: &str,
    files: &[&str],
) -> (String, String, Option<i32>, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("rubash-3730-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let mut file = std::fs::File::create(dir.join("driver.sh")).expect("write driver");
    file.write_all(driver.as_bytes()).expect("write driver");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("driver.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let contents = files
        .iter()
        .map(|name| {
            std::fs::read_to_string(dir.join(name)).unwrap_or_else(|_| "<missing>".to_string())
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        contents,
    )
}

/// The primitive: `[[ -n x ]] >inner.txt` must leave fd 1 on the
/// pre-conditional file — `echo after` lands in outer.txt, and the
/// `[[ ]]` redirect's own file stays empty.
#[test]
fn cond_output_redirect_restores_fd1() {
    let (stdout, stderr, rc, files) = run_driver(
        "fd1",
        "exec 1>outer.txt\n\
         [[ -n x ]] >inner.txt\n\
         echo after\n",
        &["outer.txt", "inner.txt"],
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "");
    assert_eq!(files[0], "after\n");
    assert_eq!(files[1], "");
}

/// The fd 2 face: `[[ -n x ]] 2>inner2.txt` must leave fd 2 on the
/// pre-conditional file — `echo oops >&2` lands in err.txt.
#[test]
fn cond_stderr_redirect_restores_fd2() {
    let (stdout, stderr, rc, files) = run_driver(
        "fd2",
        "exec 2>err.txt\n\
         [[ -n x ]] 2>inner2.txt\n\
         echo oops >&2\n",
        &["err.txt", "inner2.txt"],
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "");
    assert_eq!(files[0], "oops\n");
    assert_eq!(files[1], "");
}

/// The close form: `[[ -n x ]] 1>&-` closes fd 1 for the test only
/// — `echo after` still reaches outer.txt.
#[test]
fn cond_close_form_restores_fd1() {
    let (stdout, stderr, rc, files) = run_driver(
        "close1",
        "exec 1>outer.txt\n\
         [[ -n x ]] 1>&-\n\
         echo after\n",
        &["outer.txt"],
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "");
    assert_eq!(files[0], "after\n");
}

/// The LOOP shape: three `[[ ]]` redirects in a row must each
/// restore fd 1 — no EBADF, no misdirected line.
#[test]
fn cond_redirect_loop_restores_fd1() {
    let (stdout, stderr, rc, files) = run_driver(
        "loop1",
        "exec 1>outer.txt\n\
         for i in 1 2 3; do [[ -n \"$i\" ]] >inner.txt; echo \"line$i\"; done\n",
        &["outer.txt", "inner.txt"],
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "");
    assert_eq!(files[0], "line1\nline2\nline3\n");
    assert_eq!(files[1], "");
}

/// The dup form: `[[ -n x ]] 2>&1` retires fd 2's File endpoint
/// (dup onto fd 1) — `echo oops >&2` must still reach err.txt,
/// not outer.txt.
#[test]
fn cond_dup_form_restores_fd2() {
    let (stdout, stderr, rc, files) = run_driver(
        "dup2",
        "exec 1>outer.txt\n\
         exec 2>err.txt\n\
         [[ -n x ]] 2>&1\n\
         echo oops >&2\n",
        &["err.txt", "outer.txt"],
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "");
    assert_eq!(files[0], "oops\n");
    assert_eq!(files[1], "");
}

/// Both slots displaced by one `[[ ]]`: `>both.txt 2>&1` binds fd 1
/// to the file and fd 2 onto fd 1 — both must come back.
#[test]
fn cond_output_and_dup_redirect_restores_both() {
    let (stdout, stderr, rc, files) = run_driver(
        "both",
        "exec 1>outer.txt\n\
         exec 2>err.txt\n\
         [[ -n x ]] >both.txt 2>&1\n\
         echo after\n\
         echo oops >&2\n",
        &["outer.txt", "err.txt", "both.txt"],
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "");
    assert_eq!(files[0], "after\n");
    assert_eq!(files[1], "oops\n");
    assert_eq!(files[2], "");
}

/// Offset sharing: the undo save duplicates the open file
/// description (shared offset), so `echo second` appends after
/// `echo first` instead of overwriting at offset 0 — a fresh-open
/// "fix" would fail this cell.
#[test]
fn cond_redirect_preserves_offset() {
    let (stdout, stderr, rc, files) = run_driver(
        "offset",
        "exec 1>outer.txt\n\
         echo first\n\
         [[ -n x ]] >inner.txt\n\
         echo second\n",
        &["outer.txt", "inner.txt"],
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "");
    assert_eq!(files[0], "first\nsecond\n");
    assert_eq!(files[1], "");
}

/// The `&&` list shape: the restore happens before the next list
/// element runs.
#[test]
fn cond_redirect_then_and_list_restores_fd1() {
    let (stdout, stderr, rc, files) = run_driver(
        "andlist",
        "exec 1>outer.txt\n\
         [[ -n x ]] >inner.txt && echo chained\n",
        &["outer.txt", "inner.txt"],
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "");
    assert_eq!(files[0], "chained\n");
    assert_eq!(files[1], "");
}

/// Control: a plain `[[ ]]` with no redirect leaves fd 1 alone.
/// Passes on base and head alike; it guards against an
/// over-correction that would disturb unredirected conditionals.
#[test]
fn plain_cond_without_redirect_leaves_fd1_alone() {
    let (stdout, stderr, rc, files) = run_driver(
        "plain",
        "exec 1>outer.txt\n\
         [[ -n x ]]\n\
         echo after\n",
        &["outer.txt"],
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "");
    assert_eq!(files[0], "after\n");
}

/// The /proc face (Linux-only): an external child spawned after the
/// `[[ ]]` redirect must see fd 1 pointing at the pre-conditional
/// file — the spawn pin pass must not dup2 a recycled descriptor
/// onto it.
#[test]
#[cfg(target_os = "linux")]
fn external_child_sees_restored_fd1() {
    let (stdout, stderr, rc, files) = run_driver(
        "proc1",
        "exec 1>outer.txt\n\
         [[ -n x ]] >inner.txt\n\
         ls -l /proc/self/fd/1\n",
        &["outer.txt"],
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "");
    assert!(
        files[0].contains("outer.txt"),
        "fd 1 must point at outer.txt, got: {}",
        files[0]
    );
}
