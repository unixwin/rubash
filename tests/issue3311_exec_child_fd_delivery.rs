//! rubash#3311 / rubash#3312 / rubash#3313 — `exec` with a command
//! did not hand the exec'd image the descriptor set GNU's image
//! inherits.
//!
//! GNU execute_builtin_or_function runs do_redirections on the
//! shell's own table (execute_cmd.c:5606) and keeps the result for
//! exec (execute_cmd.c:5624); exec_builtin then calls shell_execve
//! (builtins/exec.def:231), so the image starts with exactly the
//! descriptors the walk left: every fd >= 3 the command opened
//! (redir.c:949 redir_open) or an earlier `exec N>f` bound, the
//! opened `/dev/full` device itself (redir_open's special-name list
//! does not include it, redir.c:725), and a `<&-` slot closed
//! (redir.c:1239 r_close_this).
//!
//! rubash's exec spawn simulation built its child Command with only
//! the three std slots: fd >= 3 bindings never reached the child
//! (#3311), a `/dev/full` std target was the #1898 stand-in sink
//! (#3312), and a closed fd 0 was replaced by the null device
//! (#3313). Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes; the cross-platform cells use rubash itself as
//! the child (`$THIS_SH`), the device / foreign-child cells are unix
//! only (`/bin/sh`, `/bin/echo`, `/bin/cat`, `/dev/full`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3311-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `body` as a script FILE under rubash with cwd = dir.
/// Returns (stdout, stderr, status).
fn rubash_script(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    let script = dir.join("case.sh");
    fs::write(&script, body).expect("write script fixture");
    let output = Command::new(RUBASH)
        .arg(&script)
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

#[test]
fn issue3311_exec_own_fd3_reaches_child() {
    let dir = scratch("own3");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "( exec \"$THIS_SH\" -c 'echo x >&3' 3>f.txt ); echo \"rc=$? f=[$(cat f.txt)]\"\n",
    );
    assert_eq!(stdout, "rc=0 f=[x]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3311_exec_prefix_fd4_reaches_child() {
    let dir = scratch("prefix4");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "( exec 4>f.txt \"$THIS_SH\" -c 'echo y >&4' ); echo \"rc=$? f=[$(cat f.txt)]\"\n",
    );
    assert_eq!(stdout, "rc=0 f=[y]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3311_exec_ambient_fd_and_dups_reach_child() {
    let dir = scratch("ambient");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "exec 3>amb.txt\n",
            "( exec \"$THIS_SH\" -c 'echo amb >&3' ); echo \"amb rc=$? f=[$(cat amb.txt)]\"\n",
            "( exec \"$THIS_SH\" -c 'echo own >&3' 3>own.txt 1>&3 ); echo \"own rc=$? f=[$(cat own.txt)]\"\n",
            "( exec \"$THIS_SH\" -c 'echo o5 >&5' 5>&3 ); echo \"dup rc=$?\"\n",
            "cat amb.txt\n",
        ),
    );
    assert_eq!(
        stdout,
        "amb rc=0 f=[amb]\nown rc=0 f=[own]\ndup rc=0\namb\no5\n"
    );
    assert_eq!(stderr, "");
}

/// The exec walk opens the fd >= 3 target at its own list position;
/// the delivery must hand the child THAT description, not open the
/// target again (a second noclobber open of the file the walk just
/// created would fail).
#[test]
fn issue3311_exec_own_fd3_under_noclobber_opens_once() {
    let dir = scratch("noclobber");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "set -C\n( exec \"$THIS_SH\" -c 'echo nc >&3' 3>nc.txt ); echo \"rc=$? f=[$(cat nc.txt)]\"\n",
    );
    assert_eq!(stdout, "rc=0 f=[nc]\n");
    assert_eq!(stderr, "");
}

#[cfg(unix)]
#[test]
fn issue3311_exec_fd3_reaches_foreign_child() {
    let dir = scratch("foreign");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "( exec /bin/sh -c 'echo x >&3' 3>f.txt ); echo \"rc=$? f=[$(cat f.txt)]\"\n",
            "exec 3>amb.txt\n",
            "( exec /bin/sh -c 'echo amb >&3' ); echo \"amb rc=$? f=[$(cat amb.txt)]\"\n",
            "( exec /bin/sh -c 'echo c >&3' 3>&- ) 2>/dev/null; echo \"close rc=$?\"\n",
        ),
    );
    assert_eq!(stdout, "rc=0 f=[x]\namb rc=0 f=[amb]\nclose rc=2\n");
    assert_eq!(stderr, "");
}

#[cfg(unix)]
#[test]
fn issue3311_exec_fd3_reaches_replaced_image() {
    // Top level, no subshell: the true image replacement path
    // (rubash#1175) must carry the pins too.
    let dir = scratch("image");
    let (stdout, stderr, status) = rubash_script(&dir, "exec /bin/sh -c 'echo x >&3' 3>f.txt\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
    assert_eq!(fs::read_to_string(dir.join("f.txt")).unwrap(), "x\n");
}

#[cfg(unix)]
#[test]
fn issue3312_exec_dev_full_write_fails_in_child() {
    let dir = scratch("full");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "( exec /bin/echo hi >/dev/full ); echo \"full rc=$?\"\n",
            "( exec /bin/echo hi >>/dev/full ); echo \"append rc=$?\"\n",
            "( exec /bin/sh -c 'echo f3 >&3' 3>/dev/full ) 2>/dev/null; echo \"fd3 rc=$?\"\n",
        ),
    );
    assert_eq!(stdout, "full rc=1\nappend rc=1\nfd3 rc=1\n");
    assert_eq!(
        stderr,
        "/bin/echo: write error: No space left on device\n\
         /bin/echo: write error: No space left on device\n"
    );
}

#[cfg(unix)]
#[test]
fn issue3312_exec_dev_full_top_level() {
    let dir = scratch("full-top");
    let (stdout, stderr, status) = rubash_script(&dir, "exec /bin/echo hi >/dev/full\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "/bin/echo: write error: No space left on device\n");
    assert_eq!(status, Some(1));
}

#[cfg(unix)]
#[test]
fn issue3313_exec_close_stdin_stays_closed() {
    let dir = scratch("close0");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "( exec /bin/cat <&- ); echo \"cat rc=$?\"\n",
            "( exec 0<&-; exec /bin/cat ); echo \"ambient rc=$?\"\n",
        ),
    );
    assert_eq!(stdout, "cat rc=1\nambient rc=1\n");
    assert_eq!(
        stderr,
        "/bin/cat: -: Bad file descriptor\n\
         /bin/cat: closing standard input: Bad file descriptor\n\
         /bin/cat: -: Bad file descriptor\n\
         /bin/cat: closing standard input: Bad file descriptor\n"
    );
}

#[cfg(unix)]
#[test]
fn issue3313_exec_close_stdin_top_level() {
    let dir = scratch("close0-top");
    let (stdout, stderr, status) = rubash_script(&dir, "exec /bin/cat <&-\n");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "/bin/cat: -: Bad file descriptor\n/bin/cat: closing standard input: Bad file descriptor\n"
    );
    assert_eq!(status, Some(1));
}
