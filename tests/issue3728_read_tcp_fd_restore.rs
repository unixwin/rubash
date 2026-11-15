//! rubash#3728: `read` from /dev/tcp restores a retired fd 0 entry
//! (same hazard as #3706).
//!
//! GNU ground truth (bash 5.3, redir.c:965/1082/1144): an undoable
//! redirect saves the displaced descriptor with a kernel duplicate
//! (`fcntl (fd, F_DUPFD, SHELL_FD_BASE)`), so the pre-redirect
//! descriptor stays alive while the redirect binds. Rubash's
//! transient fd-0 binds saved only an Rc clone of the userspace
//! entry: the bind retired the displaced descriptor
//! (retire_read_arm/retire_write_arm, rubash#3213) and the restore
//! re-inserted the retired entry, so the next read from the restored
//! fd failed (EBADF) or read whatever recycled the number.
//!
//! The four cells below cover every fixed site:
//! - `read x < /dev/tcp/...` (fd-0 funnel face, builtin_redirects.rs
//!   `bind_net_input_fd` — the live path for a plain `read` from the
//!   network: every in-process builtin dispatch binds its net inputs
//!   there before the body runs, and the read builtin then serves the
//!   already-bound endpoint);
//! - the same shape with `exec 0<>file`, whose saved entry's two arms
//!   share one Rc (the `<>` sharing path of `dup_file_endpoints`);
//! - `read -u 3 x 3</dev/tcp/...` (the funnel's numbered-fd arm);
//! - `read x < /dev/null` (the device arm in read_io.rs, which binds
//!   a non-regular input transiently as fd 0).
//! The Windows-only fifo arm (read_io.rs, winuxcmd fifo) has no Linux
//! pin; it is fixed by the identical mechanism.
//!
//! Every cell is fail-first on the unfixed base: after the transient
//! read, the following read from the restored fd fails instead of
//! serving the next record.

use std::io::Write;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const SOCK_LINE: &str = "sockline";

/// Loopback listener serving one line to the first connection, then
/// closing. Returns the port; the serving thread is joined by the
/// caller after the run (non-blocking accept with a deadline, so a
/// shell that never connects cannot hang the test).
fn start_listener() -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    listener.set_nonblocking(true).expect("nonblocking");
    let handle = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.write_all(format!("{SOCK_LINE}\n").as_bytes());
                    let _ = stream.flush();
                    return;
                }
                Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() > deadline {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });
    (port, handle)
}

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let (tx, rx) = mpsc::channel();
    let driver = driver.to_string();
    let tag = tag.to_string();
    // The `move` closure below takes `tag`; keep a clone for the
    // timeout message, otherwise E0382 (borrow of moved value).
    let tag_for_timeout = tag.clone();
    std::thread::spawn(move || {
        let dir = std::env::temp_dir().join(format!("rubash-3728-{tag}-{}", std::process::id()));
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
        let _ = tx.send((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        ));
    });
    rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| panic!("{tag_for_timeout}: rubash run timed out"))
}

/// Run a driver that reads once from /dev/tcp: start the listener,
/// substitute its port, run, and join the serving thread.
fn run_tcp_driver(tag: &str, driver_template: &str) -> (String, String, Option<i32>) {
    let (port, handle) = start_listener();
    let driver = driver_template.replace("{PORT}", &port.to_string());
    let result = run_driver(tag, &driver);
    let _ = handle.join();
    result
}

/// The primitive: `read a < /dev/tcp/...` must leave fd 0 reading the
/// pre-read file from its saved offset — the socket line, then the
/// file's first and second lines, all rc=0.
#[test]
fn read_from_dev_tcp_restores_stdin_file() {
    let (stdout, stderr, rc) = run_tcp_driver(
        "tcp-stdin",
        "printf 'l1\\nl2\\nl3\\n' > in.txt\n\
         exec 0<in.txt\n\
         read a < /dev/tcp/127.0.0.1/{PORT}; r0=$?\n\
         IFS= read -r b; r1=$?\n\
         IFS= read -r c; r2=$?\n\
         echo \"r0=$r0 a=[$a] r1=$r1 b=[$b] r2=$r2 c=[$c]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "r0=0 a=[sockline] r1=0 b=[l1] r2=0 c=[l2]\n");
}

/// The `<>` sharing face: with `exec 0<>file` the saved entry's two
/// arms share one Rc, and the dup snapshot must preserve that sharing
/// (closing one direction still retires the shared descriptor) while
/// keeping the file readable after the network read.
#[test]
fn read_from_dev_tcp_restores_readwrite_stdin_file() {
    let (stdout, stderr, rc) = run_tcp_driver(
        "tcp-stdin-rw",
        "printf 'l1\\nl2\\n' > in.txt\n\
         exec 0<>in.txt\n\
         read a < /dev/tcp/127.0.0.1/{PORT}; r0=$?\n\
         IFS= read -r b; r1=$?\n\
         echo \"r0=$r0 a=[$a] r1=$r1 b=[$b]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "r0=0 a=[sockline] r1=0 b=[l1]\n");
}

/// The funnel's numbered-fd arm: `read -u 3 a 3</dev/tcp/...` must
/// leave fd 3 reading its pre-read file.
#[test]
fn read_from_dev_tcp_numbered_fd_restores_fd3() {
    let (stdout, stderr, rc) = run_tcp_driver(
        "tcp-fd3",
        "printf 'f3a\\nf3b\\n' > f3.txt\n\
         exec 3<f3.txt\n\
         read -u 3 a 3</dev/tcp/127.0.0.1/{PORT}; r0=$?\n\
         IFS= read -r b <&3; r1=$?\n\
         echo \"r0=$r0 a=[$a] r1=$r1 b=[$b]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "r0=0 a=[sockline] r1=0 b=[f3a]\n");
}

/// The device arm: `read a < /dev/null` binds a non-regular input
/// transiently as fd 0, and must leave fd 0 reading the pre-read file
/// (the /dev/null read itself fails with rc 1, as on GNU).
#[test]
fn read_from_dev_null_restores_stdin_file() {
    let (stdout, stderr, rc) = run_driver(
        "devnull-stdin",
        "printf 'l1\\nl2\\n' > in.txt\n\
         exec 0<in.txt\n\
         read a < /dev/null; r0=$?\n\
         IFS= read -r b; r1=$?\n\
         IFS= read -r c; r2=$?\n\
         echo \"r0=$r0 a=[$a] r1=$r1 b=[$b] r2=$r2 c=[$c]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "r0=1 a=[] r1=0 b=[l1] r2=0 c=[l2]\n");
}
