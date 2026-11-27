//! rubash#3725: after `while ...; done N<<<body`, fd N is not restored
//! to its pre-loop target — the loop-heredoc face of the #3706 hazard.
//!
//! GNU ground truth (bash 5.3): loop redirections are undoable
//! (execute_cmd.c:854 `do_redirections (command->redirects,
//! RX_ACTIVE|RX_UNDOABLE)`), and the undo save IS a kernel duplicate
//! of the displaced descriptor (`fcntl (fd, F_DUPFD, SHELL_FD_BASE)`,
//! redir.c:965/1082/1144, recorded by `add_undo_redirect`), so the
//! pre-loop descriptor stays alive for the loop's duration and
//! `undo_redirections` dups it back. Rubash's loop-heredoc save in
//! `execute_simple_loop` (src/executor/alias_loops.rs) only cloned the
//! userspace entry, so the bind-time retire (retire_read_arm,
//! rubash#3213) closed the very descriptor the restore was about to
//! replay — the restore re-inserted a retired entry, and the next
//! `read <&N` failed (EBADF) or read the wrong target.
//!
//! The cells below are byte-shaped against GNU (expected outputs
//! verified against system bash 5.2.21 as an indicator run; the
//! Bash-5.3 script-file oracle re-run is owed with the cargo legs,
//! routes down). Every cell except the no-heredoc control is
//! fail-first on the unfixed base (predicted: the post-loop read
//! fails rc=1 with an empty value — mechanism-derived, run owed).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3725-{}-{tag}", std::process::id()));
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

/// The primitive: `while ... done 8<<<"LOOPBODY"` must leave fd 8
/// reading the pre-loop file — the loop consumes fd 9, fd 8 is only
/// displaced for the loop's duration.
#[test]
fn loop_heredoc_restores_fd8_file() {
    let (stdout, stderr, rc) = run_driver(
        "fd8",
        "printf 'f8a\\nf8b\\n' > f8.txt\n\
         printf 'i1\\ni2\\n' > f9.txt\n\
         exec 8<f8.txt\n\
         exec 9<f9.txt\n\
         while IFS= read -r l <&9; do echo \"[$l]\"; done 8<<<\"LOOPBODY\"\n\
         IFS= read -r rest <&8; r=$?\n\
         echo \"rc=$r rest=[$rest]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "[i1]\n[i2]\nrc=0 rest=[f8a]\n");
}

/// The defect hits fd 3 exactly like fd 8.
#[test]
fn loop_heredoc_restores_fd3_file() {
    let (stdout, stderr, rc) = run_driver(
        "fd3",
        "printf 'f3a\\nf3b\\n' > f3.txt\n\
         printf 'j1\\n' > f9.txt\n\
         exec 3<f3.txt\n\
         exec 9<f9.txt\n\
         while IFS= read -r l <&9; do echo \"[$l]\"; done 3<<<\"BODY3\"\n\
         IFS= read -r rest <&3; r=$?\n\
         echo \"rc=$r rest=[$rest]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "[j1]\nrc=0 rest=[f3a]\n");
}

/// The bind itself applies (the body reads the heredoc line off fd 8)
/// AND the restore puts the file back afterwards.
#[test]
fn loop_body_reads_heredoc_then_fd8_restored() {
    let (stdout, stderr, rc) = run_driver(
        "bind-and-restore",
        "printf 'f8a\\nf8b\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         i=0\n\
         while [ $i -lt 1 ]; do IFS= read -r h <&8; echo \"h=[$h]\"; i=1; done 8<<<\"HEREDOC-LINE\"\n\
         IFS= read -r rest <&8; r=$?\n\
         echo \"rc=$r rest=[$rest]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "h=[HEREDOC-LINE]\nrc=0 rest=[f8a]\n");
}

/// The `until` twin of the primitive.
#[test]
fn until_loop_heredoc_restores_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "until",
        "printf 'f8a\\nf8b\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         n=0\n\
         until [ $n -ge 2 ]; do echo \"tick $n\"; n=$((n+1)); done 8<<<\"UNTILBODY\"\n\
         IFS= read -r rest <&8; r=$?\n\
         echo \"rc=$r rest=[$rest]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "tick 0\ntick 1\nrc=0 rest=[f8a]\n");
}

/// Two sequential loops each bind fd 8: the shared offset survives
/// both save/restore cycles (the dup names the same open file
/// description), so the reads after each loop advance f8a, then f8b.
#[test]
fn two_sequential_loops_restore_fd8_with_offset() {
    let (stdout, stderr, rc) = run_driver(
        "two-loops",
        "printf 'f8a\\nf8b\\nf8c\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         i=0\n\
         while [ $i -lt 1 ]; do i=1; done 8<<<\"FIRST\"\n\
         IFS= read -r r1 <&8; a=$?\n\
         j=0\n\
         while [ $j -lt 1 ]; do j=1; done 8<<<\"SECOND\"\n\
         IFS= read -r r2 <&8; b=$?\n\
         echo \"a=$a r1=[$r1] b=$b r2=[$r2]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a=0 r1=[f8a] b=0 r2=[f8b]\n");
}

/// Control: a loop with no heredoc leaves fd 8 untouched. Passes on
/// the unfixed base too; guards the fix against breaking the
/// no-heredoc path.
#[test]
fn loop_without_heredoc_leaves_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "control",
        "printf 'f8a\\nf8b\\n' > f8.txt\n\
         printf 'm1\\n' > f9.txt\n\
         exec 8<f8.txt\n\
         exec 9<f9.txt\n\
         while IFS= read -r l <&9; do echo \"[$l]\"; done\n\
         IFS= read -r rest <&8; r=$?\n\
         echo \"rc=$r rest=[$rest]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "[m1]\nrc=0 rest=[f8a]\n");
}
