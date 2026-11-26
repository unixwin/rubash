//! rubash#3560 — a command's own numbered `N>&-` / `N<&-` must revoke
//! fd N for THAT child: GNU services the close with r_close_this
//! inside the ordered do_redirections walk (redir.c:237; the close
//! itself at redir.c:1239 close_buffered_fd), which runs in the
//! forked child, so the parent's binding survives untouched.
//!
//! rubash's external delivery stamped every ambient fd>=3 into the
//! child's __RUBASH_FD_* env keys (and unix pins) BEFORE the redirect
//! walk, and the walk's close arm only `continue`d — the stamps
//! survived, so the child still saw fd N (`echo c >&3` succeeded and
//! wrote into the parent's file). The fix routes the numbered close
//! through the same revocation a {var} pending close uses
//! (apply_pending_close_at): env stamps removed, pins dropped,
//! carriers retired, walk bindings cleared, and the table step
//! snapshots for undo_child_fd_var_redirects so the parent's entry
//! is restored at command end.
//!
//! Expectations byte-verified against GNU bash 5.3.20
//! (/usr/local/bin/bash, script-file probes in
//! target/scratch-i3/gnu-3560*.sh). The cross-platform cells use
//! rubash itself as the child (`$THIS_SH`): the env channel is
//! shared, so the bug was not Windows-only for rubash children —
//! the Windows handle whitelist just made it total there.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3560-{tag}-{}", std::process::id()));
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

/// The issue's reference shape: ambient fd 3, command-local `3>&-`,
/// child attempts `>&3` — GNU reports bad fd (rc=1) and the parent's
/// file receives only the legitimate later write.
#[test]
fn issue3560_numbered_close_revokes_ambient_fd() {
    let dir = scratch("amb");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "exec 3>amb.txt\n",
            "\"$THIS_SH\" -c 'echo c >&3' 3>&- 2>/dev/null; echo \"A rc=$?\"\n",
            "( \"$THIS_SH\" -c 'echo c >&3' 3>&- ) 2>/dev/null; echo \"B rc=$?\"\n",
            "\"$THIS_SH\" -c 'echo c >&3' 2>/dev/null; echo \"C rc=$?\"\n",
            "echo \"amb bytes: $(wc -c < amb.txt)\"\n",
        ),
    );
    assert_eq!(stdout, "A rc=1\nB rc=1\nC rc=0\namb bytes: 2\n");
    assert_eq!(stderr, "");
}

/// `exec CMD 3>&-` — the exec spawn path shares the same delivery
/// walk (collect_external_fd_delivery with `preopened`), so the
/// close must revoke there too.
#[test]
fn issue3560_exec_child_close_revokes_ambient_fd() {
    let dir = scratch("exec");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "exec 3>amb.txt\n",
            "( exec \"$THIS_SH\" -c 'echo c >&3' 3>&- ) 2>/dev/null; echo \"exec-close rc=$?\"\n",
            "( exec \"$THIS_SH\" -c 'echo ok >&3' ); echo \"exec-keep rc=$?\"\n",
            "echo \"amb=[$(cat amb.txt)]\"\n",
        ),
    );
    assert_eq!(stdout, "exec-close rc=1\nexec-keep rc=0\namb=[ok]\n");
    assert_eq!(stderr, "");
}

/// GNU's ordered walk is last-wins in both directions:
/// `3>f 3>&-` closes (f created, child writes nothing),
/// `3>&- 3>g` rebinds (child writes g).
#[test]
fn issue3560_close_position_orders_against_later_entries() {
    let dir = scratch("order");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "\"$THIS_SH\" -c 'echo x >&3' 3>f.txt 3>&- 2>/dev/null; echo \"own-then-close rc=$? f=[$(cat f.txt)]\"\n",
            "\"$THIS_SH\" -c 'echo y >&3' 3>&- 3>g.txt 2>/dev/null; echo \"close-then-own rc=$? g=[$(cat g.txt)]\"\n",
        ),
    );
    assert_eq!(
        stdout,
        "own-then-close rc=1 f=[]\nclose-then-own rc=0 g=[y]\n"
    );
    assert_eq!(stderr, "");
}

/// A dup whose SOURCE slot the same list closed must fail: GNU
/// services `4>&3` against the fd table as bound at that position —
/// fd 3 is closed, so the redirect itself is a bad-fd error (rc=1).
#[test]
fn issue3560_dup_from_closed_slot_fails() {
    let dir = scratch("dup");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "\"$THIS_SH\" -c 'echo z >&4' 3>&- 4>&3; echo \"close-dup rc=$?\"\n",
    );
    assert_eq!(stdout, "close-dup rc=1\n");
    assert!(stderr.contains("Bad file descriptor"), "stderr={stderr:?}");
}

/// The input-direction close `N<&-` revokes the same way, and the
/// parent's fd survives the command-local close either direction.
#[test]
fn issue3560_input_close_and_parent_survival() {
    let dir = scratch("inclose");
    fs::write(dir.join("in.txt"), "hello\n").expect("write in.txt");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "exec 5<in.txt\n",
            "\"$THIS_SH\" -c 'cat <&5' 5<&- 2>/dev/null; echo \"in-close rc=$?\"\n",
            "\"$THIS_SH\" -c 'cat <&5' 2>/dev/null; echo \"in-ambient rc=$?\"\n",
            "exec 3>amb.txt\n",
            "\"$THIS_SH\" -c 'echo c >&3' 3>&- 2>/dev/null; echo \"amb-close rc=$?\"\n",
            "echo parent >&3; echo \"parent rc=$? amb=[$(cat amb.txt)]\"\n",
        ),
    );
    assert_eq!(
        stdout,
        "in-close rc=1\nhello\nin-ambient rc=0\namb-close rc=1\nparent rc=0 amb=[parent]\n"
    );
    assert_eq!(stderr, "");
}

/// Foreign (non-rubash) children never saw the env stamps anyway —
/// the unix pin pass already skipped command-named slots — but the
/// cell locks the GNU shape end-to-end on unix.
#[cfg(unix)]
#[test]
fn issue3560_foreign_child_close_stays_closed() {
    let dir = scratch("foreign");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "exec 3>amb.txt\n",
            "( exec /bin/sh -c 'echo c >&3' 3>&- ) 2>/dev/null; echo \"sh-close rc=$?\"\n",
            "( exec /bin/sh -c 'echo ok >&3' ); echo \"sh-keep rc=$?\"\n",
            "echo \"amb=[$(cat amb.txt)]\"\n",
        ),
    );
    assert_eq!(stdout, "sh-close rc=2\nsh-keep rc=0\namb=[ok]\n");
    assert_eq!(stderr, "");
}
