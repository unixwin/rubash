//! Issue rubash#1838: an Executor created inside a host process adopted
//! the host's open descriptors >= 3 as fd-table File endpoints over the
//! RAW descriptor, and FileFd's drop closes the handle it holds — so
//! Executor teardown (or a shell-level close of the slot) closed
//! descriptors the executor never opened. When the host-side owner was
//! a std IO-safety type — the UnixListener in executor_tests part_072
//! (`test_conditional_unix_file_unary_checks_paths`) — its own drop
//! then aborted the whole process: "fatal runtime error: IO Safety
//! violation: owned file descriptor already closed". The full
//! executor_tests table could never run to completion.
//!
//! GNU reference point: GNU has no embedding posture; in its own
//! process the inherited descriptors ARE the shell's, and redir.c
//! validates a dup source against the kernel itself (`fcntl
//! (redirector, F_GETFD, 0) != -1`, redir.c:1160). The embedded
//! invariant pinned here is the ownership line: adoption now
//! duplicates the descriptor (`crate::fd::dup_adopted_fd`,
//! F_DUPFD_CLOEXEC), so the table owns only its duplicate — the same
//! open file description, shared offset — while the host's original
//! descriptor is never closed by the executor. The shell-visible
//! #1557 contract is unchanged: `exec N>&-` still closes the shell's
//! fd and later references report Bad file descriptor (also pinned
//! below, in-process).
//!
//! Acceptance shape for the issue itself: `cargo test --test
//! executor_tests` runs the full table to completion (no abort).

#![cfg(unix)]

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash1838-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn fd_is_open(fd: i32) -> bool {
    (unsafe { libc::fcntl(fd, libc::F_GETFD) }) >= 0
}

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script {script:?} failed: {result:?}");
}

/// The part_072 abort shape: a host-owned UnixListener is open while an
/// in-process Executor runs (its path is stat'd by `[[ -S ]]`, output
/// redirected to files). Dropping the Executor must leave the
/// listener's descriptor untouched — on the pre-fix code this test
/// binary ABORTS at the listener drop that ends the test.
#[test]
fn issue1838_executor_teardown_keeps_host_socket_alive() {
    use std::os::unix::net::{UnixListener, UnixStream};

    let dir = tempdir("socket");
    let sock_path = dir.join("kind.sock");
    let out_path = dir.join("out.txt");
    let listener = UnixListener::bind(&sock_path).expect("bind listener");
    let listener_fd = listener.as_raw_fd();
    assert!(fd_is_open(listener_fd));

    {
        let mut executor = Executor::new();
        let script = format!(
            "[[ -S {sock} ]]; echo $? > {out}; [[ -d {dir} ]]; echo $? >> {out}; [[ -f {out} ]]; echo $? >> {out}",
            sock = sock_path.display(),
            out = out_path.display(),
            dir = dir.display(),
        );
        run_script(&mut executor, &script);
        // Executor drops here: teardown must not close the host fd.
    }

    assert_eq!(
        std::fs::read_to_string(&out_path).expect("read output"),
        "0\n0\n0\n"
    );
    assert!(
        fd_is_open(listener_fd),
        "executor teardown closed the host's listener descriptor"
    );
    // The listener must still be fully functional, not merely open.
    let _stream = UnixStream::connect(&sock_path).expect("listener still accepts connections");
    drop(listener); // aborts the process on the pre-fix code
    let _ = std::fs::remove_dir_all(&dir);
}

/// The ownership invariant without any IO-safety owner in play: an
/// Executor that never touches a host descriptor must not close it
/// either — adoption alone is not ownership.
#[test]
fn issue1838_executor_teardown_keeps_host_file_usable() {
    let dir = tempdir("file");
    let path = dir.join("host.txt");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .expect("open host file");
    let fd = file.as_raw_fd();

    {
        let mut executor = Executor::new();
        run_script(&mut executor, "true");
        // Executor drops here.
    }

    assert!(fd_is_open(fd), "executor teardown closed the host's file");
    file.write_all(b"alive\n")
        .expect("host file still writable");
    drop(file);
    assert_eq!(
        std::fs::read_to_string(&path).expect("read host file"),
        "alive\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The #1557 shell-view contract, in-process: `exec N>&-` closes the
/// SHELL's adopted fd (a later write is the ordinary Bad file
/// descriptor, rc=1), while the host's original descriptor — the same
/// open file description the shell adopted a duplicate of — stays
/// open and usable.
#[test]
fn issue1838_shell_close_of_adopted_fd_spares_host_original() {
    let dir = tempdir("close");
    let data_path = dir.join("data.txt");
    let out_path = dir.join("rc.txt");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(&data_path)
        .expect("open data file");
    let fd = file.as_raw_fd();

    {
        let mut executor = Executor::new();
        let script = format!(
            "exec {fd}>&-; echo X >&{fd}; echo rc=$? > {out}",
            out = out_path.display()
        );
        run_script(&mut executor, &script);
        // Executor drops here.
    }

    assert_eq!(
        std::fs::read_to_string(&out_path).expect("read rc file"),
        "rc=1\n",
        "shell view: write to a closed adopted fd must fail like GNU"
    );
    assert!(
        fd_is_open(fd),
        "shell-level close of the adopted fd closed the host's original"
    );
    file.write_all(b"host\n")
        .expect("host original still writable");
    drop(file);
    assert_eq!(
        std::fs::read_to_string(&data_path).expect("read data file"),
        "host\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
