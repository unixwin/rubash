//! rubash#3814: a close inside a subshell must not retire the parent
//! shell's descriptor — `( : ) 8<&-` leaves the parent's fd 8 fully
//! readable.
//!
//! GNU ground truth (bash 5.3.20, script-file mode): the subshell is a
//! fork (execute_cmd.c:1576 execute_in_subshell); the child's close(2)
//! removes only the child's descriptor copy, so the parent's fd 8
//! still reads `a` — `P5min rc=0 l2=[a]`. Rubash emulates the fork with
//! an fd-table snapshot; the snapshot shared the `Rc<FileFd>` kernel
//! handle, so the in-subshell close retired the very descriptor the
//! parent's restored entry named and the follow-up `read` failed
//! EBADF (`P5min rc=1 l2=[]`).
//!
//! Expectations verified byte-for-byte against /usr/local/bin/bash
//! 5.3.20 under WSL, script-file mode (inline `-c` takes a different
//! materialization path and does not reproduce).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3814-{}-{tag}", std::process::id()));
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

/// The issue reproducer verbatim: process-substitution input on fd 8,
/// a subshell that closes it, then a parent-side read. GNU prints
/// `P5min rc=0 l2=[a]`.
#[test]
#[cfg(unix)]
fn subshell_close_does_not_kill_parent_fd() {
    let (stdout, stderr, rc) = run_driver(
        "p5min",
        "exec 8< <(printf 'a\\nb\\n')\n\
         ( : ) 8<&-\n\
         IFS= read -r l2 <&8\n\
         echo \"P5min rc=$? l2=[$l2]\"\n\
         exec 8<&-\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "P5min rc=0 l2=[a]\n");
}

/// File-backed variant: the same leak hit any File endpoint, not just
/// procsub pipes — `( : ) 9<&-` must leave fd 9 reading the file.
#[test]
#[cfg(unix)]
fn subshell_close_file_fd_stays_live() {
    let (stdout, stderr, rc) = run_driver(
        "file9",
        "printf 'f1\\nf2\\n' > f9.txt\n\
         exec 9<f9.txt\n\
         ( : ) 9<&-\n\
         IFS= read -r l <&9\n\
         echo \"rc=$? l=[$l]\"\n\
         exec 9<&-\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0 l=[f1]\n");
}

/// Nested subshell: a close two levels down must still leave every
/// ancestor's descriptor intact.
#[test]
#[cfg(unix)]
fn nested_subshell_close_isolated() {
    let (stdout, stderr, rc) = run_driver(
        "nested",
        "exec 8< <(printf 'x\\n')\n\
         ( ( : ) 8<&- )\n\
         IFS= read -r l <&8\n\
         echo \"rc=$? l=[$l]\"\n\
         exec 8<&-\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0 l=[x]\n");
}

/// Command substitution is the same fork boundary (subst.c:7143
/// command_substitute): a close inside `$( )` must not leak either.
#[test]
#[cfg(unix)]
fn command_substitution_close_isolated() {
    let (stdout, stderr, rc) = run_driver(
        "comsub",
        "exec 8< <(printf 'y\\n')\n\
         x=$(exec 8<&-)\n\
         IFS= read -r l <&8\n\
         echo \"rc=$? l=[$l]\"\n\
         exec 8<&-\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0 l=[y]\n");
}

/// The subshell's own close must still work inside the region: after
/// `( : ) 8<&-`, a SECOND close in the parent is a clean no-op error
/// shape GNU produces (read on the closed fd fails), not a crash.
#[test]
#[cfg(unix)]
fn parent_close_after_subshell_close_still_works() {
    let (stdout, stderr, rc) = run_driver(
        "doubleclose",
        "exec 8< <(printf 'z\\n')\n\
         ( : ) 8<&-\n\
         exec 8<&-\n\
         IFS= read -r l <&8 2>/dev/null\n\
         echo \"rc=$? l=[$l]\"\n",
    );
    assert_eq!(rc, Some(0));
    // GNU: the parent close succeeds; the read then fails EBADF with
    // rc=1 and empty l.
    assert_eq!(stdout, "rc=1 l=[]\n");
    let _ = stderr;
}
