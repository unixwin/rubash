//! rubash#1861 pins: a compound command's `< fifo` binds a real
//! descriptor and streams on demand, exactly like GNU bash 5.3.20 —
//! the bind must NOT slurp the fifo to EOF first (which stalled the
//! body until the writer closed, discarded the over-read bytes with
//! the FUNCTION_STDIN mirror, and left a later opener facing a
//! writer that was already gone).
//!
//! Each cell runs against a writer thread (this test process) that
//! writes its lines, then HOLDS the fifo open: under the slurp the
//! shell's first output could only appear after the writer closed,
//! so an early-arrival deadline far below the hold time separates the
//! two mechanisms without tight timing assumptions. GNU oracle
//! (5.3.20, script-file probes): both groups finish at +0.05s with a
//! 5s hold; the while loop prints its first lines at +0.07s with the
//! writer still producing; an external child inside the group
//! continues the same stream.
//!
//! Unix-only: the shapes need mkfifo.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Timed {
    lines: Vec<(Duration, String)>,
    code: Option<i32>,
    stderr: String,
}

/// Spawn `rubash -c script`, streaming stdout lines with arrival
/// times; kill the child if it outlives `deadline`.
fn run_timed(script: &str, deadline: Duration) -> Timed {
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
    let stdout = child.stdout.take().expect("stdout pipe");
    let stderr = child.stderr.take().expect("stderr pipe");
    let (tx, rx) = mpsc::channel::<(Duration, String)>();
    let t0 = Instant::now();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let _ = tx.send((t0.elapsed(), line.trim_end().to_string()));
                }
            }
        }
    });
    let (etx, erx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut reader = stderr;
        let mut buf = String::new();
        let _ = reader.read_to_string(&mut buf);
        let _ = etx.send(buf);
    });
    let mut lines = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(item) => lines.push(item),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if t0.elapsed() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("shell outlived deadline {deadline:?}; lines so far: {lines:?}");
        }
        if let Ok(Some(_)) = child.try_wait() {
            // Drain any trailing lines the reader already queued.
            while let Ok(item) = rx.try_recv() {
                lines.push(item);
            }
            break;
        }
    }
    let code = child.wait().ok().and_then(|status| status.code());
    let stderr = erx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    Timed {
        lines,
        code,
        stderr,
    }
}

/// Unique scratch dir per test invocation (pid + nanos): a fixed
/// /tmp path collides across runs when a killed earlier run leaked
/// its fifo and the pid got reused — the shell's mkfifo then fails
/// and the cell silently runs against a stranger's fifo. The fifo
/// itself is created HERE, before any writer thread starts: the
/// writer's File::create would otherwise win the race and create a
/// regular file at the path (the shell's mkfifo then fails with
/// EEXIST and the groups read a file — offset restart and all).
fn scratch_fifo(tag: &str) -> (String, String) {
    let dir = format!(
        "/tmp/rubash1861-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let fifo = format!("{dir}/f.fifo");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("spawn mkfifo");
    assert!(status.success(), "mkfifo {fifo}");
    (dir, fifo)
}

/// Writer thread: open the fifo for writing (blocks until the shell's
/// bind opens it), write `payload`, keep holding for `hold`, close.
fn spawn_writer(path: &str, payload: &'static str, hold: Duration) {
    let path = path.to_string();
    std::thread::spawn(move || {
        let mut file = std::fs::File::create(&path).expect("writer open fifo");
        file.write_all(payload.as_bytes()).expect("writer write");
        file.flush().expect("writer flush");
        std::thread::sleep(hold);
    });
}

fn arrival(lines: &[(Duration, String)], want: &str) -> Option<Duration> {
    lines
        .iter()
        .find(|(_, line)| line == want)
        .map(|(at, _)| *at)
}

#[test]
fn issue1861_two_groups_stream_and_second_open_survives() {
    let (dir, fifo) = scratch_fifo("two");
    let script = format!(
        "F={fifo}; \
         {{ read a; }} < \"$F\"; echo \"a=$a\"; \
         {{ read b; read c; }} < \"$F\"; echo \"b=$b c=$c\"; \
         rm -f \"$F\"; echo DONE"
    );
    spawn_writer(&fifo, "l1\nl2\nl3\nl4\n", Duration::from_secs(4));
    let out = run_timed(&script, Duration::from_secs(9));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.stderr, "", "{:?}", out.lines);
    // Both groups must complete while the writer still holds the
    // fifo (4s): the slurp could not produce `a=l1` before +4s and
    // deadlocked the second group outright.
    let a_at = arrival(&out.lines, "a=l1").expect("a=l1 line");
    assert!(a_at < Duration::from_secs(2), "a=l1 arrived at {a_at:?}");
    let bc_at = arrival(&out.lines, "b=l2 c=l3").expect("b/c line");
    assert!(bc_at < Duration::from_secs(2), "b/c arrived at {bc_at:?}");
    assert!(arrival(&out.lines, "DONE").is_some(), "{:?}", out.lines);
    assert_eq!(out.code, Some(0));
}

#[test]
fn issue1861_while_loop_reads_lines_as_they_arrive() {
    let (dir, fifo) = scratch_fifo("loop");
    let script = format!(
        "F={fifo}; \
         while read l; do echo \"L=$l\"; done < \"$F\"; \
         echo loopdone; rm -f \"$F\"; echo DONE"
    );
    // Writer: two lines now, a third after 2.5s, then EOF.
    let path = fifo.clone();
    std::thread::spawn(move || {
        let mut file = std::fs::File::create(&path).expect("writer open fifo");
        file.write_all(b"w1\nw2\n").expect("writer write 1");
        file.flush().expect("writer flush 1");
        std::thread::sleep(Duration::from_millis(2500));
        file.write_all(b"w3\n").expect("writer write 2");
        file.flush().expect("writer flush 2");
    });
    let out = run_timed(&script, Duration::from_secs(9));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.stderr, "", "{:?}", out.lines);
    let w1_at = arrival(&out.lines, "L=w1").expect("L=w1 line");
    let w2_at = arrival(&out.lines, "L=w2").expect("L=w2 line");
    assert!(w1_at < Duration::from_millis(1250), "w1 at {w1_at:?}");
    assert!(w2_at < Duration::from_millis(1250), "w2 at {w2_at:?}");
    assert!(arrival(&out.lines, "L=w3").is_some(), "{:?}", out.lines);
    assert!(arrival(&out.lines, "loopdone").is_some(), "{:?}", out.lines);
    assert_eq!(out.code, Some(0));
}

#[test]
fn issue1861_external_child_continues_the_same_stream() {
    let (dir, fifo) = scratch_fifo("child");
    let script = format!(
        "F={fifo}; \
         {{ read a; echo \"a=$a\"; head -n 1; }} < \"$F\"; \
         echo groupdone; rm -f \"$F\"; echo DONE"
    );
    spawn_writer(&fifo, "l1\nl2\nl3\n", Duration::from_secs(3));
    let out = run_timed(&script, Duration::from_secs(8));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.stderr, "", "{:?}", out.lines);
    let a_at = arrival(&out.lines, "a=l1").expect("a=l1 line");
    assert!(a_at < Duration::from_millis(1500), "a=l1 at {a_at:?}");
    // head (an external child) inherits the group's descriptor and
    // prints the NEXT line off the same stream, still while the
    // writer holds the fifo open.
    let head_at = arrival(&out.lines, "l2").expect("head l2 line");
    assert!(head_at < Duration::from_millis(1500), "l2 at {head_at:?}");
    assert!(
        arrival(&out.lines, "groupdone").is_some(),
        "{:?}",
        out.lines
    );
    assert_eq!(out.code, Some(0));
}
