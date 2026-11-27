//! rubash#1862 pins: a background compound command's redirects are
//! applied by the async child (GNU execute_cmd.c:1761-1763
//! execute_in_subshell runs do_redirections after the fork), so an
//! open with rendezvous semantics — a FIFO target — must NOT be
//! performed synchronously by the parent executor while it resolves
//! the child's stdio. Before the fix, `( printf; sleep ) > fifo &`
//! deadlocked the shell's own execution thread inside
//! openat(O_WRONLY|O_CREAT|O_TRUNC) in resolve_background_stdio: the
//! reader was a later command the parent never reached. A regular
//! file control was unaffected.
//!
//! Every cell asserts the spawn returns at once ("spawned" prints
//! before any peer exists) and the payload then arrives through the
//! fifo once a reader shows up. GNU oracle (5.3.20): all cells
//! complete in ~0.5s.
//!
//! Unix-only: the shapes need mkfifo.

#![cfg(unix)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

struct Out {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

/// Run one script under the built rubash with a hard deadline: the
/// pre-fix failure mode is a hang, so the deadline IS the assertion
/// mechanism, backed by the content assertions in each cell.
fn run(script: &str) -> Out {
    let shell = env!("CARGO_BIN_EXE_rubash");
    let mut child = Command::new(shell)
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let mut stdout = child.stdout.take().expect("stdout pipe");
    let mut stderr = child.stderr.take().expect("stderr pipe");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut so = String::new();
        let mut se = String::new();
        let _ = stdout.read_to_string(&mut so);
        let _ = stderr.read_to_string(&mut se);
        let _ = tx.send((so, se));
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok((stdout, stderr)) => Out {
            stdout,
            stderr,
            code: child.wait().ok().and_then(|status| status.code()),
        },
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("shell hung past the 10s deadline (background FIFO open in the parent?)");
        }
    }
}

fn scratch(tag: &str) -> String {
    // pid + nanos: a pid-only path collides across runs when a
    // killed earlier run leaked its directory and the pid got reused.
    format!(
        "/tmp/rubash1862-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
}

#[test]
fn issue1862_subshell_output_fifo_does_not_block_spawn() {
    let dir = scratch("sub");
    let script = format!(
        "D={dir}; mkdir -p \"$D\"; F=\"$D/f\"; mkfifo \"$F\"; \
         ( printf 'hi\\n'; sleep 0.3 ) > \"$F\" & echo spawned; \
         cat \"$F\" > \"$D/got\"; wait; \
         echo \"got=$(cat \"$D/got\")\"; rm -rf \"$D\"; echo DONE"
    );
    let out = run(&script);
    assert_eq!(out.stderr, "", "{}", out.stdout);
    assert_eq!(out.stdout, "spawned\ngot=hi\nDONE\n");
    assert_eq!(out.code, Some(0));
}

#[test]
fn issue1862_brace_group_output_fifo_does_not_block_spawn() {
    let dir = scratch("brace");
    let script = format!(
        "D={dir}; mkdir -p \"$D\"; F=\"$D/f\"; mkfifo \"$F\"; \
         {{ printf 'yo\\n'; sleep 0.3; }} > \"$F\" & echo spawned; \
         cat \"$F\" > \"$D/got\"; wait; \
         echo \"got=$(cat \"$D/got\")\"; rm -rf \"$D\"; echo DONE"
    );
    let out = run(&script);
    assert_eq!(out.stderr, "", "{}", out.stdout);
    assert_eq!(out.stdout, "spawned\ngot=yo\nDONE\n");
    assert_eq!(out.code, Some(0));
}

#[test]
fn issue1862_combined_redirect_and_dup_replay_in_child() {
    let dir = scratch("comb");
    // `&>` to a fifo: one deferred open feeds both fds in the child.
    // The `> fifo 2>&1` spelling must land stderr on the fifo too —
    // the dup's source fd is deferred, so the dup replays in the
    // child after the fifo open.
    let script = format!(
        "D={dir}; mkdir -p \"$D\"; F=\"$D/f\"; G=\"$D/g\"; \
         mkfifo \"$F\"; mkfifo \"$G\"; \
         ( printf 'o1\\n'; printf 'e1\\n' >&2 ) &> \"$F\" & echo s1; \
         cat \"$F\" > \"$D/gotf\"; wait; \
         ( printf 'o2\\n'; printf 'e2\\n' >&2 ) > \"$G\" 2>&1 & echo s2; \
         cat \"$G\" > \"$D/gotg\"; wait; \
         echo \"f=$(sort \"$D/gotf\" | tr '\\n' '+')\"; \
         echo \"g=$(sort \"$D/gotg\" | tr '\\n' '+')\"; \
         rm -rf \"$D\"; echo DONE"
    );
    let out = run(&script);
    assert_eq!(out.stderr, "", "{}", out.stdout);
    assert_eq!(out.stdout, "s1\ns2\nf=e1+o1+\ng=e2+o2+\nDONE\n");
    assert_eq!(out.code, Some(0));
}

#[test]
fn issue1862_input_side_fifo_background_read() {
    let dir = scratch("in");
    // The reader is backgrounded first; its `< fifo` open must wait
    // in the child, not in the parent — the writer only arrives a
    // second later.
    let script = format!(
        "D={dir}; mkdir -p \"$D\"; F=\"$D/f\"; mkfifo \"$F\"; \
         ( read x; echo \"x=$x\" > \"$D/got\" ) < \"$F\" & echo spawned; \
         ( sleep 1; printf 'yo\\n' ) > \"$F\" & \
         wait; echo \"got=$(cat \"$D/got\")\"; rm -rf \"$D\"; echo DONE"
    );
    let out = run(&script);
    assert_eq!(out.stderr, "", "{}", out.stdout);
    assert_eq!(out.stdout, "spawned\ngot=x=yo\nDONE\n");
    assert_eq!(out.code, Some(0));
}

#[test]
fn issue1862_deferral_rolls_back_earlier_same_fd_redirect() {
    let dir = scratch("roll");
    // fd 1 first takes a regular file (consumed in the parent today),
    // then the fifo: deferring fd 1 rolls the regular redirect back
    // too, and the child replays both in order — the payload lands in
    // the fifo, the regular file is created/truncated but stays
    // empty, exactly like GNU's child-side sequential opens.
    let script = format!(
        "D={dir}; mkdir -p \"$D\"; F=\"$D/f\"; R=\"$D/r\"; mkfifo \"$F\"; \
         ( printf 'pay\\n' ) > \"$R\" > \"$F\" & echo spawned; \
         cat \"$F\" > \"$D/got\"; wait; \
         echo \"got=$(cat \"$D/got\")\"; \
         echo \"rsize=$(wc -c < \"$D/r\" | tr -d ' ')\"; \
         rm -rf \"$D\"; echo DONE"
    );
    let out = run(&script);
    assert_eq!(out.stderr, "", "{}", out.stdout);
    assert_eq!(out.stdout, "spawned\ngot=pay\nrsize=0\nDONE\n");
    assert_eq!(out.code, Some(0));
}

#[test]
fn issue1862_other_fds_stay_parent_resolved_when_one_fd_defers() {
    let dir = scratch("mix");
    // fd 2 goes to a regular file (parent-resolved as before) while
    // fd 1 defers to the fifo: the split must not leak stderr into
    // the fifo or lose the regular capture.
    let script = format!(
        "D={dir}; mkdir -p \"$D\"; F=\"$D/f\"; R=\"$D/r\"; mkfifo \"$F\"; \
         ( printf 'out\\n'; printf 'err\\n' >&2 ) 2> \"$R\" > \"$F\" & echo spawned; \
         cat \"$F\" > \"$D/got\"; wait; \
         echo \"got=$(cat \"$D/got\")\"; \
         echo \"err=$(cat \"$D/r\")\"; rm -rf \"$D\"; echo DONE"
    );
    let out = run(&script);
    assert_eq!(out.stderr, "", "{}", out.stdout);
    assert_eq!(out.stdout, "spawned\ngot=out\nerr=err\nDONE\n");
    assert_eq!(out.code, Some(0));
}
