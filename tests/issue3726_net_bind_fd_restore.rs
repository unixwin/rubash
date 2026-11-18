//! rubash#3726: a /dev/tcp|udp bind restores a retired entry (same
//! hazard as #3706, on the network-redirect face).
//!
//! GNU ground truth (bash 5.3.20): an undoable redirect saves the
//! displaced descriptor BEFORE the redirect binds, with
//! `fcntl (fd, F_DUPFD, SHELL_FD_BASE)` (redir.c:965 for input,
//! :1082 for duplicating, :1144 for move; SHELL_FD_BASE is 10,
//! redir.c:86), recorded by `add_undo_redirect` so
//! `undo_redirections` can dup it back. The simple-command funnel's
//! net bind (`bind_simple_command_net_inputs`, rubash#2124) runs the
//! same save/restore pair in-process for builtin/function routes:
//! `bind_net_input_fd` saved only an Rc clone of the displaced
//! entry, the bind's `open_input`/`open_output` retired that
//! descriptor (`retire_read_arm` / `retire_write_arm`, rubash#3213),
//! and `restore_simple_command_net_inputs` re-inserted the retired
//! entry — the next read saw EBADF (or the next spawn's pin pass
//! dup2'd whatever had recycled that kernel number onto the fd).
//!
//! The cells below are byte-shaped from that mechanism: a builtin
//! (`true`) whose numbered / fd-0 / readwrite net redirect displaces
//! a file-backed fd, then a read/write of the pre-bind target. Every
//! cell except the plain-builtin control and the `<>`-close sharing
//! guard is fail-first on the unfixed base (read EBADF / wrong
//! content / wrong /proc target). The listener harness is the #2124
//! shape (loopback listener, greeting, connection count).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let (tx, rx) = mpsc::channel();
    let script = script.to_string();
    let panic_tag = tag.to_string();
    let tag = tag.to_string();
    std::thread::spawn(move || {
        let dir = std::env::temp_dir().join(format!("rubash3726-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let case = dir.join("case.sh");
        std::fs::write(&case, &script).expect("write fixture");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .current_dir(&dir)
            .arg(&case)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run rubash");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = tx.send((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        ));
    });
    rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| panic!("{panic_tag}: rubash run timed out"))
}

/// Loopback listener: greets each accepted connection and reports the
/// running connection count (the #2124 harness shape).
struct Listener {
    port: u16,
    rx: mpsc::Receiver<usize>,
}

fn start_listener() -> Listener {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking");
        let mut count = 0usize;
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    count += 1;
                    let _ = stream.write_all(b"GREETING-LINE\n");
                    let _ = stream.flush();
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
                    let mut sink = [0u8; 64];
                    loop {
                        match stream.read(&mut sink) {
                            Ok(0) => break,
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
            if tx.send(count).is_err() {
                break;
            }
        }
    });
    Listener { port, rx }
}

impl Listener {
    /// Poll until the count reaches `want` (3s deadline) — the
    /// accept thread's reports can lag under parallel test load, so
    /// a positive expectation waits instead of sampling once.
    fn connections_expect(&self, want: usize) -> usize {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut last = 0;
        loop {
            while let Ok(count) = self.rx.try_recv() {
                last = count;
            }
            if last >= want || std::time::Instant::now() >= deadline {
                return last;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn target(&self) -> String {
        format!("/dev/tcp/127.0.0.1/{}", self.port)
    }
}

/// The primitive: `true 8</dev/tcp/...` must leave fd 8 reading the
/// pre-bind file from its saved offset — first line, then second
/// line, both rc=0 — and must connect exactly once.
#[test]
fn net_bind_restores_fd8() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, rc) = run_script(
        "fd8",
        &format!(
            "printf 'l1\\nl2\\nl3\\n' > f8.txt\n\
             exec 8<f8.txt\n\
             true 8<{t}\n\
             IFS= read -r l1 <&8; rc1=$?\n\
             IFS= read -r l2 <&8; rc2=$?\n\
             echo \"rc1=$rc1 l1=[$l1] rc2=$rc2 l2=[$l2]\"\n"
        ),
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc1=0 l1=[l1] rc2=0 l2=[l2]\n");
    assert_eq!(listener.connections_expect(1), 1, "fd8 bind: connections");
}

/// The fd-0 winner face: `true </dev/tcp/...` displaces fd 0 and the
/// restore must hand it back to the pre-bind file.
#[test]
fn net_bind_restores_fd0() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, rc) = run_script(
        "fd0",
        &format!(
            "printf 'w1\\nw2\\n' > f0.txt\n\
             exec 0<f0.txt\n\
             true <{t}\n\
             IFS= read -r l1; rc1=$?\n\
             IFS= read -r l2; rc2=$?\n\
             echo \"rc1=$rc1 l1=[$l1] rc2=$rc2 l2=[$l2]\"\n"
        ),
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc1=0 l1=[w1] rc2=0 l2=[w2]\n");
    assert_eq!(listener.connections_expect(1), 1, "fd0 bind: connections");
}

/// The defect hits fd 3 exactly like fd 8.
#[test]
fn net_bind_restores_fd3() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, rc) = run_script(
        "fd3",
        &format!(
            "printf 'a\\nb\\n' > f3.txt\n\
             exec 3<f3.txt\n\
             true 3<{t}\n\
             IFS= read -r l <&3; r=$?\n\
             echo \"rc=$r l=[$l]\"\n"
        ),
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0 l=[a]\n");
    assert_eq!(listener.connections_expect(1), 1, "fd3 bind: connections");
}

/// A `<>` pair survives the net bind on BOTH arms: the saved entry
/// duplicates the shared open file description, so a read after the
/// bind advances the shared offset and a write lands in the file at
/// that offset (rw.txt was "x\n"; the write appends "after\n" behind
/// the consumed line).
#[test]
fn net_bind_readwrite_pair_survives() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, rc) = run_script(
        "rw",
        &format!(
            "printf 'x\\n' > rw.txt\n\
             exec 8<>rw.txt\n\
             true 8<>{t}\n\
             IFS= read -r l <&8; r1=$?\n\
             echo \"after\" >&8; r2=$?\n\
             echo \"r1=$r1 l=[$l] r2=$r2\"\n\
             cat rw.txt\n"
        ),
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "r1=0 l=[x] r2=0\nx\nafter\n");
    // rubash#3798: the `<>` net bind opens exactly one TCP connection
    // (GNU bash 5.3.20: one O_RDWR socket, redir.c do_redirections /
    // r_input_output) — the mirror probe's second connect is fixed.
    assert_eq!(listener.connections_expect(1), 1, "rw bind: connections");
}

/// Two numbered net binds on one command restore both fds.
#[test]
fn two_numbered_net_binds_restore_both() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, rc) = run_script(
        "two",
        &format!(
            "printf 'e8\\n' > f8.txt\n\
             printf 'e9\\n' > f9.txt\n\
             exec 8<f8.txt\n\
             exec 9<f9.txt\n\
             true 8<{t} 9<{t}\n\
             IFS= read -r l8 <&8; r8=$?\n\
             IFS= read -r l9 <&9; r9=$?\n\
             echo \"r8=$r8 l8=[$l8] r9=$r9 l9=[$l9]\"\n"
        ),
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "r8=0 l8=[e8] r9=0 l9=[e9]\n");
    assert_eq!(listener.connections_expect(2), 2, "two binds: connections");
}

/// The /proc face (Linux-only): an external child spawned after the
/// net bind must see fd 8 pointing at the pre-bind file — the spawn
/// pin pass must not dup2 a recycled descriptor onto it.
#[test]
#[cfg(target_os = "linux")]
fn external_child_sees_restored_fd8() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, rc) = run_script(
        "proc8",
        &format!(
            "printf 'F8\\n' > f8.txt\n\
             exec 8<f8.txt\n\
             true 8<{t}\n\
             ls -l /proc/self/fd/8\n"
        ),
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert!(
        stdout.contains("f8.txt"),
        "fd 8 must point at f8.txt, got: {stdout}"
    );
    assert_eq!(listener.connections_expect(1), 1, "proc8 bind: connections");
}

/// Control: a plain builtin with no net redirect leaves fd 8 alone.
/// Passes on base and head alike; it guards against an
/// over-correction that would disturb unredirected dispatches.
#[test]
fn plain_builtin_leaves_fd8_alone() {
    let (stdout, stderr, rc) = run_script(
        "plain",
        "printf 'c1\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         true\n\
         IFS= read -r l <&8; r=$?\n\
         echo \"rc=$r l=[$l]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0 l=[c1]\n");
}

/// Sharing guard (not fail-first for #3726 — it passes on the
/// unfixed base too, where the write fails because the restore is
/// dead): a `<>` pair shares ONE descriptor between its arms, so
/// `exec 8<&-` after the bind must close the whole descriptor — the
/// write must fail, exactly like GNU's close(2) on fd 8. This locks
/// the undo save's sharing preservation: duplicating the two arms
/// independently would leave the write arm live here.
#[test]
fn net_bind_readwrite_close_input_closes_both() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, _stderr, rc) = run_script(
        "rwclose",
        &format!(
            "printf 'x\\n' > rw.txt\n\
             exec 8<>rw.txt\n\
             true 8<>{t}\n\
             exec 8<&-\n\
             echo \"after\" >&8; r=$?\n\
             echo \"rc=$r\"\n"
        ),
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    // rubash#3798: exactly one TCP connection for the `<>` net bind
    // (GNU bash 5.3.20: one O_RDWR socket, redir.c do_redirections /
    // r_input_output) — the mirror probe's second connect is fixed.
    assert_eq!(
        listener.connections_expect(1),
        1,
        "rwclose bind: connections"
    );
}
