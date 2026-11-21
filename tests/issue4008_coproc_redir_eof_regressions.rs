//! rubash#4008 — a coproc output redirect must not leave the
//! `${COPROC[0]}` read end open past the coproc's death.
//!
//! GNU binds a trailing redirect list to the coproc's INNER command
//! (parse.y:1130/1153 `COPROC shell_command redirection_list`
//! appends it to $2->redirects), so the displaced fd-1 pipe writer's
//! lifetime is the inner command's own redirect scope:
//!
//! - `{ }` group / compound body: apply undoably (execute_cmd.c:854
//!   do_redirections RX_ACTIVE|RX_UNDOABLE -> redir.c
//!   add_undo_redirect's fcntl F_DUPFD to a save slot >=
//!   SHELL_FD_BASE). The write end lives until the body finishes —
//!   the coproc's death. Verified on GNU 5.3.20: `coproc { sleep 3;
//!   } >/dev/null` leaves the saved end on the child's fd 10 and
//!   `cat <&${COPROC[0]}` blocks for the full 3 s.
//! - `( )` subshell body or simple command: apply without the undo
//!   list (execute_cmd.c:1761/5522 RX_ACTIVE), so the displaced end
//!   closes at apply time and the read end sees EOF immediately
//!   (same oracle: `coproc sleep 2 >/dev/null` and `coproc ( sleep
//!   2; ) >/dev/null` EOF in ~4 ms).
//!
//! Rubash's parent-held displaced writer used to be dropped only at
//! coproc RETIREMENT, which is deferred to a disposal point — past
//! the `cat <&${COPROC[0]}` / `read -u` that was blocking on the
//! pipe's EOF, so every one of these shapes hung forever:
//!
//!   coproc { :; } >/dev/null; cat <&${COPROC[0]}
//!   coproc { :; } >/dev/null; read -r -u ${COPROC[0]} l
//!   coproc whoami.exe >file; cat <&${COPROC[0]}
//!   coproc { echo E >&2; } 2>&1 >/dev/null; cat <&${COPROC[0]}
//!
//! The writer now dies with the child: on Unix it rides a save-slot
//! fd into the child (pre_exec) for hold-until-exit bodies; on hosts
//! without fd injection a detached waiter drops it when the child
//! exits. Bodies GNU applies non-undoably (simple/`( )`) drop the
//! writer at spawn for an immediate EOF.
//!
//! Every pin runs through the shared wait_bounded harness so a
//! regression reports a timed_out failure instead of hanging the
//! suite. Expectations verified against GNU bash 5.3.20 on WSL.

mod common;

use std::process::{Command, Stdio};
use std::time::Duration;

fn rubash_script_bounded(script: &str, limit: Duration) -> (String, String, Option<i32>, bool) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-4008-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");
    let outcome = common::wait_bounded(&mut child, limit);
    let result = (
        String::from_utf8_lossy(&outcome.stdout).into_owned(),
        String::from_utf8_lossy(&outcome.stderr).into_owned(),
        outcome.code,
        outcome.timed_out,
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The issue's primary repro: a brace-group coproc whose stdout is
/// redirected away must not hang `cat` on the read end — GNU holds
/// the displaced writer in the child's save slot, so EOF arrives at
/// the child's exit (near-instant for `:;`).
#[test]
fn issue4008_group_stdout_redirect_cat_gets_eof() {
    let script = "coproc { :; } >/dev/null\ncat <&${COPROC[0]}\necho \"cat_rc=$?\"\n";
    let (stdout, stderr, code, timed_out) = rubash_script_bounded(script, Duration::from_secs(20));
    assert!(
        !timed_out,
        "cat hung on ${{COPROC[0]}}: {stdout:?} / {stderr:?}"
    );
    assert_eq!(stdout, "cat_rc=0\n");
    assert_eq!(code, Some(0));
}

/// Same repro through the `read -u` builtin: EOF at the child's
/// death means `read` reports EOF (rc 1, empty var), not a block.
#[test]
fn issue4008_group_stdout_redirect_read_gets_eof() {
    let script =
        "coproc { :; } >/dev/null\nread -r -u ${COPROC[0]} l\necho \"read_rc=$? l=[$l]\"\n";
    let (stdout, _, code, timed_out) = rubash_script_bounded(script, Duration::from_secs(20));
    assert!(!timed_out, "read hung on ${{COPROC[0]}}: {stdout:?}");
    assert_eq!(stdout, "read_rc=1 l=[]\n");
    assert_eq!(code, Some(0));
}

/// The close form `>&-` takes the same displaced-writer path.
#[test]
fn issue4008_group_stdout_close_cat_gets_eof() {
    let script = "coproc { :; } >&-\ncat <&${COPROC[0]}\necho \"cat_rc=$?\"\n";
    let (stdout, _, code, timed_out) = rubash_script_bounded(script, Duration::from_secs(20));
    assert!(!timed_out, "cat hung after >&-: {stdout:?}");
    assert_eq!(stdout, "cat_rc=0\n");
    assert_eq!(code, Some(0));
}

/// GNU keeps the group body's save slot for the body's whole
/// duration: `coproc { sleep N; } >/dev/null` must still delay EOF
/// until the child dies — a drop-at-spawn would EOF instantly. The
/// block is timed INSIDE the script (`date +%s%N` around `cat`):
/// process-elapsed timing cannot see it because the respawned coproc
/// child inherits rubash's stdout/stderr handles and holds the
/// test's pipes open for its whole life.
#[test]
fn issue4008_group_redirect_eof_at_child_death_not_spawn() {
    let script = "coproc { sleep 1; } >/dev/null\ns=$(date +%s%N)\ncat <&${COPROC[0]}\ne=$(date +%s%N)\necho \"cat_rc=$? ms=$(( (e-s)/1000000 ))\"\n";
    let (stdout, _, code, timed_out) = rubash_script_bounded(script, Duration::from_secs(20));
    assert!(!timed_out, "cat hung on ${{COPROC[0]}}: {stdout:?}");
    assert_eq!(code, Some(0));
    let (rc, ms) = parse_cat_timing(&stdout);
    assert_eq!(rc, 0, "cat rc: {stdout:?}");
    assert!(
        ms >= 800,
        "EOF arrived {ms} ms into `cat` — the displaced writer died at \
         spawn instead of with the group body's child (GNU blocks the \
         full sleep)"
    );
}

/// A simple-command body applies the redirect without a save slot
/// (GNU's execute_simple_command non-undoable path), so the read end
/// must see EOF immediately — long before `sleep` returns.
#[test]
fn issue4008_simple_redirect_eof_at_apply() {
    let script = "coproc sleep 3 >/dev/null\ns=$(date +%s%N)\ncat <&${COPROC[0]}\ne=$(date +%s%N)\necho \"cat_rc=$? ms=$(( (e-s)/1000000 ))\"\n";
    let (stdout, _, code, timed_out) = rubash_script_bounded(script, Duration::from_secs(20));
    assert!(!timed_out, "cat hung on ${{COPROC[0]}}: {stdout:?}");
    assert_eq!(code, Some(0));
    let (rc, ms) = parse_cat_timing(&stdout);
    assert_eq!(rc, 0, "cat rc: {stdout:?}");
    assert!(
        ms < 2500,
        "EOF arrived {ms} ms into `cat` — a simple-command body's \
         displaced writer must close at apply time like GNU's, not at \
         the coproc's death"
    );
}

/// A `( )` body is the subshell path — non-undoable redirects, so
/// EOF is immediate like the simple command's.
#[test]
fn issue4008_subshell_redirect_eof_at_apply() {
    let script = "coproc ( sleep 3; ) >/dev/null\ns=$(date +%s%N)\ncat <&${COPROC[0]}\ne=$(date +%s%N)\necho \"cat_rc=$? ms=$(( (e-s)/1000000 ))\"\n";
    let (stdout, _, code, timed_out) = rubash_script_bounded(script, Duration::from_secs(20));
    assert!(!timed_out, "cat hung on ${{COPROC[0]}}: {stdout:?}");
    assert_eq!(code, Some(0));
    let (rc, ms) = parse_cat_timing(&stdout);
    assert_eq!(rc, 0, "cat rc: {stdout:?}");
    assert!(
        ms < 2500,
        "EOF arrived {ms} ms into `cat` — a `( )` body's \
         displaced writer must close at apply time like GNU's"
    );
}

/// Parse `cat_rc=N ms=M` emitted by the timing scripts above.
fn parse_cat_timing(stdout: &str) -> (i32, i64) {
    let line = stdout.trim();
    let mut rc = None;
    let mut ms = None;
    for part in line.split_whitespace() {
        if let Some(v) = part.strip_prefix("cat_rc=") {
            rc = v.parse().ok();
        } else if let Some(v) = part.strip_prefix("ms=") {
            ms = v.parse().ok();
        }
    }
    (
        rc.unwrap_or_else(|| panic!("no cat_rc in {stdout:?}")),
        ms.unwrap_or_else(|| panic!("no ms in {stdout:?}")),
    )
}

/// `2>&1 >/dev/null` on a group body: the fd-2 dup keeps a pipe
/// write end live in the child, so `cat` still reads what stderr
/// wrote and then EOFs at the child's death — the displaced fd-1
/// writer must not outlive it.
#[test]
fn issue4008_stderr_dup_then_null_order_eof() {
    let script =
        "coproc { echo E >&2; :; } 2>&1 >/dev/null\ncat <&${COPROC[0]}\necho \"cat_rc=$?\"\n";
    let (stdout, _, code, timed_out) = rubash_script_bounded(script, Duration::from_secs(20));
    assert!(!timed_out, "cat hung after 2>&1 >/dev/null: {stdout:?}");
    assert_eq!(stdout, "E\ncat_rc=0\n");
    assert_eq!(code, Some(0));
}

/// The issue's `coproc whoami.exe >file` shape — a direct (foreign
/// program) coproc child. GNU's simple-command semantics close the
/// displaced end at apply time; `cat` must not block. `bash` is used
/// for the same portable-external reason the rubash#1644 pins use it.
#[test]
fn issue4008_direct_coproc_redirect_cat_gets_eof() {
    let script = "coproc bash -c \"echo O; echo E >&2\" 2>&1 > f.txt\ncat <&${COPROC[0]}\nwait $COPROC_PID 2>/dev/null\necho \"file:\"\ncat f.txt\n";
    let (stdout, _, code, timed_out) = rubash_script_bounded(script, Duration::from_secs(20));
    assert!(!timed_out, "cat hung after direct coproc >file: {stdout:?}");
    assert_eq!(stdout, "E\nfile:\nO\n");
    assert_eq!(code, Some(0));
}
