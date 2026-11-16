//! rubash#3706: after `{ ...; } N<redir`, fd N is not restored to its
//! pre-group target — it lands on fd 0's target (for `N</dev/null`) or
//! fd 1's target (for `N<&-`).
//!
//! GNU ground truth (bash 5.3.20, verified by the #2902 round-3 lane,
//! REPORT ~/workspace/eco-2902-closing3/REPORT.md §4): a compound
//! group's redirections are undoable (execute_cmd.c:854
//! `do_redirections (command->redirects, RX_ACTIVE|RX_UNDOABLE)`), and
//! the undo save IS a kernel duplicate of the displaced descriptor
//! (`fcntl (fd, F_DUPFD, SHELL_FD_BASE)`, redir.c:965/1082/1144,
//! recorded by `add_undo_redirect`), so the pre-group descriptor stays
//! alive for the group's duration and `undo_redirections` dups it back.
//! Rubash's scope save only cloned the userspace entry, so the
//! bind-time retire (retire_read_arm/retire_write_arm, rubash#3213)
//! closed the very descriptor the restore was about to replay — the
//! restore re-inserted a retired entry, and the next spawn's pin pass
//! dup2'd whatever kernel descriptor had recycled that number.
//!
//! The cells below are the REPORT §4 repros, byte-shaped: §4.2 single
//! group (§4.3 mapping table), §4.1 LOOP form, fd 3 / fd 9 / nested /
//! fd 0 / the `N<file` bind form / the external-child /proc face /
//! the `<>` pair's write arm. Every cell except the plain-group
//! control and the `<>`-close sharing guard is fail-first on the
//! unfixed base (read EBADF / wrong content / wrong /proc target).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3706-{}-{tag}", std::process::id()));
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
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// §4.2 (the primitive): `{ echo hi; } 8</dev/null` must leave fd 8
/// reading the pre-group file from its saved offset — first line,
/// then second line, both rc=0.
#[test]
fn single_group_dev_null_restores_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "single8",
        "printf 'l1\\nl2\\nl3\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { echo hi; } 8</dev/null\n\
         IFS= read -r l1 <&8; rc1=$?\n\
         IFS= read -r l2 <&8; rc2=$?\n\
         echo \"rc1=$rc1 l1=[$l1] rc2=$rc2 l2=[$l2]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hi\nrc1=0 l1=[l1] rc2=0 l2=[l2]\n");
}

/// §4.1 (the suite's LOOP shape): three iterations, no EBADF.
#[test]
fn loop_form_reads_three_iterations() {
    let (stdout, stderr, rc) = run_driver(
        "loop",
        "exec 8< <(printf 'a\\nb\\nc\\n')\n\
         while IFS= read -r l <&8; do { echo \"[$l]\"; } 8<&-; done\n\
         echo \"loop-done rc=$?\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "[a]\n[b]\n[c]\nloop-done rc=0\n");
}

/// §4.3 T2: the defect hits fd 3 exactly like fd 8.
#[test]
fn single_group_dev_null_restores_fd3() {
    let (stdout, stderr, rc) = run_driver(
        "single3",
        "printf 'x\\ny\\n' > f3.txt\n\
         exec 3<f3.txt\n\
         { echo g; } 3</dev/null\n\
         IFS= read -r l <&3; r=$?\n\
         echo \"rc=$r l=[$l]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "g\nrc=0 l=[x]\n");
}

/// §4.3 T4: a group redirecting fd 9 restores fd 9 and leaves fd 8
/// (bound to its own file) untouched.
#[test]
fn single_group_dev_null_restores_fd9_and_leaves_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "single9",
        "printf 'f8a\\nf8b\\n' > f8.txt\n\
         printf 'f9a\\nf9b\\n' > f9.txt\n\
         exec 8<f8.txt\n\
         exec 9<f9.txt\n\
         { echo g; } 9</dev/null\n\
         IFS= read -r l9 <&9; r9=$?\n\
         IFS= read -r l8 <&8; r8=$?\n\
         echo \"r9=$r9 l9=[$l9] r8=$r8 l8=[$l8]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "g\nr9=0 l9=[f9a] r8=0 l8=[f8a]\n");
}

/// §4.3 T3: the close form `{ echo g; } 8<&-` restores fd 8 too.
#[test]
fn close_form_restores_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "close8",
        "printf 'l1\\nl2\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { echo g; } 8<&-\n\
         IFS= read -r l <&8; r=$?\n\
         echo \"rc=$r l=[$l]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "g\nrc=0 l=[l1]\n");
}

/// The `N<file` bind form: after `{ echo g; } 8<other`, fd 8 reads
/// the PRE-group file, not the group's file.
#[test]
fn group_redirect_to_file_restores_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "file8",
        "printf 'A1\\nA2\\n' > f8.txt\n\
         printf 'B1\\nB2\\n' > f2.txt\n\
         exec 8<f8.txt\n\
         { echo g; } 8<f2.txt\n\
         IFS= read -r l1 <&8; r1=$?\n\
         IFS= read -r l2 <&8; r2=$?\n\
         echo \"r1=$r1 l1=[$l1] r2=$r2 l2=[$l2]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "g\nr1=0 l1=[A1] r2=0 l2=[A2]\n");
}

/// §4.3 T6: nested groups, both redirecting fd 8, restore it.
#[test]
fn nested_groups_restore_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "nested8",
        "printf 'l1\\nl2\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { { echo g; } 8</dev/null; } 8</dev/null\n\
         IFS= read -r l <&8; r=$?\n\
         echo \"rc=$r l=[$l]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "g\nrc=0 l=[l1]\n");
}

/// min-scope3 P4: a group redirecting fd 0 restores it as well.
#[test]
fn group_redirect_restores_fd0() {
    let (stdout, stderr, rc) = run_driver(
        "fd0",
        "printf 'p\\nq\\n' > f0.txt\n\
         exec 0<f0.txt\n\
         { echo g; } 0</dev/null\n\
         IFS= read -r l; r=$?\n\
         echo \"rc=$r l=[$l]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "g\nrc=0 l=[p]\n");
}

/// §4.3 (the /proc face, Linux-only): an external child spawned
/// after the group must see fd 8 pointing at the pre-group file —
/// the spawn pin pass must not dup2 a recycled descriptor onto it.
#[test]
#[cfg(target_os = "linux")]
fn external_child_sees_restored_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "proc8",
        "printf 'F8\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { echo g; } 8</dev/null\n\
         ls -l /proc/self/fd/8\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert!(
        stdout.contains("f8.txt"),
        "fd 8 must point at f8.txt, got: {stdout}"
    );
}

/// §4.3 T5 (control): a plain group with no redirect leaves fd 8
/// alone. Passes on base and head alike; it guards against an
/// over-correction that would disturb unredirected groups.
#[test]
fn plain_group_leaves_fd8_alone() {
    let (stdout, stderr, rc) = run_driver(
        "plain8",
        "printf 'l1\\nl2\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { echo g; }\n\
         IFS= read -r l <&8; r=$?\n\
         echo \"rc=$r l=[$l]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "g\nrc=0 l=[l1]\n");
}

/// A `<>` pair's write arm survives the group restore too: the
/// saved entry duplicates the shared open file description, so a
/// write after the group lands in the file.
#[test]
fn readwrite_pair_write_survives_group() {
    let (stdout, stderr, rc) = run_driver(
        "rw8",
        "printf 'x\\n' > rw.txt\n\
         exec 8<>rw.txt\n\
         { echo g; } 8</dev/null\n\
         echo \"after\" >&8; r=$?\n\
         echo \"rc=$r\"\n\
         cat rw.txt\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "g\nrc=0\nafter\n");
}

/// Sharing guard (not fail-first for #3706 — it passes on the
/// unfixed base too, where the write fails because the restore is
/// dead): a `<>` pair shares ONE descriptor between its arms, so
/// `exec 8<&-` after the group must close the whole descriptor —
/// the write must fail, exactly like GNU's close(2) on fd 8. This
/// locks the undo save's sharing preservation: duplicating the two
/// arms independently would leave the write arm live here.
#[test]
fn readwrite_pair_close_input_closes_both() {
    let (stdout, _stderr, rc) = run_driver(
        "rwclose8",
        "printf 'x\\n' > rw.txt\n\
         exec 8<>rw.txt\n\
         { echo g; } 8</dev/null\n\
         exec 8<&-\n\
         echo \"after\" >&8; r=$?\n\
         echo \"rc=$r\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "g\nrc=1\n");
}
