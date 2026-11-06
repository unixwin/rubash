//! rubash#2880 — the `exec N>fifo` pairing deadlock (the run-test
//! `test1.sub` fifo dance). GNU opens a redirect target exactly once
//! (redir.c do_redirection_internal); the exec-with-only-redirects walk
//! validated the target with a throwaway `File::create` whose handle was
//! dropped and then opened the target a second time in
//! `set_fd_output_file`. On a FIFO the dropped first open consumes the
//! rendezvous pairing: the background reader takes EOF in the
//! zero-writer gap and exits, and the second open blocks forever
//! (~20–30% of attempts under load; strace: open #1 O_WRONLY pairs with
//! the reader, the reader exits, open #2 never returns).
//!
//! Expectations verified against GNU bash 5.3.20 (script files).
#![cfg(unix)]

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2880-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

struct BoundedOutput {
    stdout: String,
    code: Option<i32>,
    timed_out: bool,
}

/// Run a script file under the built rubash with a hard deadline: a
/// regression of #2880 is a HANG, so the test must fail on timeout
/// instead of blocking the suite forever.
fn run_bounded(dir: &PathBuf, text: &str, deadline: Duration) -> BoundedOutput {
    let script = dir.join("probe.sh");
    std::fs::write(&script, text).expect("write script");
    let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let mut out_pipe = child.stdout.take().expect("stdout pipe");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let started = Instant::now();
    let mut timed_out = false;
    let code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status.code(),
            None => {
                if started.elapsed() > deadline {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    let stdout_bytes = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    BoundedOutput {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        code,
        timed_out,
    }
}

/// The verbatim tail of GNU's `test1.sub` (third_party/bash/tests):
/// the background reader must pair with the `exec 6>` open, and fd 6
/// must name the fifo afterwards.
#[test]
fn issue2880_test1_sub_verbatim_tail() {
    let dir = tempdir("tail");
    let script = format!(
        "TMPDIR='{}'\n\
         trap 'rm -f ${{TMPDIR}}/pipe' EXIT HUP INT QUIT ABRT TERM\n\
         exec 6>&-\n\
         echo \"t -p /dev/fd/6\"\n\
         test -p /dev/fd/6\n\
         echo $?\n\
         rm -f ${{TMPDIR}}/pipe 2>/dev/null\n\
         mkfifo ${{TMPDIR}}/pipe\n\
         cat < ${{TMPDIR}}/pipe &\n\
         exec 6>&-\n\
         exec 6>${{TMPDIR}}/pipe\n\
         echo \"t -p /dev/fd/6\"\n\
         test -p /dev/fd/6\n\
         echo $?\n\
         exec 2>/dev/null\n\
         kill $! 2>/dev/null\n",
        dir.display()
    );
    let out = run_bounded(&dir, &script, Duration::from_secs(30));
    assert!(
        !out.timed_out,
        "verbatim test1.sub tail hung (pairing deadlock regressed); partial stdout: {:?}",
        out.stdout
    );
    assert_eq!(out.stdout, "t -p /dev/fd/6\n1\nt -p /dev/fd/6\n0\n");
    assert_eq!(out.code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The dance in a loop: at the pre-fix hang rate (~20–30% per attempt),
/// 30 consecutive pairings escape with probability < 0.2%; a single
/// pairing failure hangs the script and trips the deadline.
#[test]
fn issue2880_fifo_dance_loop_pairs_reliably() {
    let dir = tempdir("loop");
    let script = format!(
        "TMPDIR='{}'\n\
         i=0\n\
         while [ $i -lt 30 ]; do\n\
         rm -f $TMPDIR/pipe\n\
         mkfifo $TMPDIR/pipe\n\
         cat < $TMPDIR/pipe &\n\
         exec 6>&-\n\
         exec 6>$TMPDIR/pipe\n\
         test -p /dev/fd/6 || {{ echo \"ROUND $i NOTFIFO\"; exit 1; }}\n\
         exec 6>&-\n\
         kill $! 2>/dev/null\n\
         wait $! 2>/dev/null\n\
         i=$((i+1))\n\
         echo \"round $i ok\"\n\
         done\n\
         echo DANCE-LOOP-DONE\n",
        dir.display()
    );
    let out = run_bounded(&dir, &script, Duration::from_secs(120));
    assert!(
        !out.timed_out,
        "fifo dance loop hung (pairing deadlock regressed); partial stdout: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("round 30 ok\n"),
        "missing final round marker; stdout: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.ends_with("DANCE-LOOP-DONE\n"),
        "missing completion marker; stdout: {:?}",
        out.stdout
    );
    assert_eq!(out.code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The pairing is not just an open rendezvous: bytes written through
/// the exec-bound fd must reach the background reader, and closing the
/// fd must deliver EOF so the reader finishes.
#[test]
fn issue2880_exec_fifo_delivers_bytes_and_eof() {
    let dir = tempdir("data");
    let script = format!(
        "TMPDIR='{}'\n\
         mkfifo $TMPDIR/pipe\n\
         cat < $TMPDIR/pipe > $TMPDIR/out &\n\
         exec 6>$TMPDIR/pipe\n\
         echo hello-fifo >&6\n\
         exec 6>&-\n\
         wait\n\
         echo \"out=[$(cat $TMPDIR/out)]\"\n",
        dir.display()
    );
    let out = run_bounded(&dir, &script, Duration::from_secs(30));
    assert!(
        !out.timed_out,
        "fifo data-flow shape hung; partial stdout: {:?}",
        out.stdout
    );
    assert_eq!(out.stdout, "out=[hello-fifo]\n");
    assert_eq!(out.code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
