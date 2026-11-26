//! rubash#2276: the `/dev/fd/N` SPELLING is an open(), not a dup.
//!
//! GNU (HAVE_DEV_FD): the shell hands the literal path to open(2)
//! (redir.c do_redirections' r_input/r_output arms call redir_open
//! on the target word); the kernel resolves the /dev/fd/N magic
//! symlink — a regular file fresh-opens, a pipe attaches, and a
//! SOCKET fails ENXIO ("No such device or address"). rubash used to
//! serve a socket endpoint through its tolerant dup machinery on
//! every spelling face, streaming bytes GNU never delivers.
//! Descriptor faces (`cat <&3`, `read -u 3`) are dups, not opens,
//! and must keep streaming — the controls at the bottom pin that
//! split. Oracle: probes/p2276.sh against GNU bash 5.3.20.
//!
//! The socket endpoint under test is the shell's own /dev/tcp
//! connection; the test process plays the server.

use std::io::Write;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

/// Bind an ephemeral listener and serve "hello\n" on every accepted
/// connection (each shell face reconnects before use).
fn hello_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        for _ in 0..8 {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.write_all(b"hello\n");
                    let _ = stream.flush();
                }
                Err(_) => break,
            }
        }
    });
    port
}

fn run(script: &str) -> (String, String, i32) {
    let scratch = std::env::temp_dir().join(format!(
        "rubash-issue2276-{}-{}",
        std::process::id(),
        script.len()
    ));
    let _ = std::fs::create_dir_all(&scratch);
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .current_dir(&scratch)
        .env("HOME", &scratch)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn issue2276_operand_cat_dev_fd_socket_enxio() {
    let port = hello_server();
    let script = format!("exec 3<>/dev/tcp/127.0.0.1/{port}\ncat /dev/fd/3\necho rc=$?\n");
    let (stdout, stderr, _) = run(&script);
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: /dev/fd/3: No such device or address"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2276_redirect_input_socket_enxio() {
    let port = hello_server();
    let script = format!("exec 3<>/dev/tcp/127.0.0.1/{port}\ncat < /dev/fd/3\necho rc=$?\n");
    let (stdout, stderr, _) = run(&script);
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(stderr.contains("/dev/fd/3"), "stderr: {stderr}");
    assert!(
        stderr.contains("No such device or address"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2276_output_spelling_socket_enxio() {
    let port = hello_server();
    let script = format!("exec 3<>/dev/tcp/127.0.0.1/{port}\necho x > /dev/fd/3\necho rc=$?\n");
    let (stdout, stderr, _) = run(&script);
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("No such device or address"),
        "stderr: {stderr}"
    );
}

// Controls: descriptor faces are dups and keep streaming the socket.

#[test]
fn issue2276_control_read_u_streams_socket() {
    let port = hello_server();
    let script =
        format!("exec 3<>/dev/tcp/127.0.0.1/{port}\nread -u 3 line\necho \"line=$line\"\n");
    let (stdout, stderr, _) = run(&script);
    assert_eq!(stdout, "line=hello\n", "stderr: {stderr}");
}

#[test]
fn issue2276_control_dup_input_streams_socket() {
    let port = hello_server();
    let script = format!("exec 3<>/dev/tcp/127.0.0.1/{port}\ncat <&3\n");
    let (stdout, stderr, _) = run(&script);
    assert_eq!(stdout, "hello\n", "stderr: {stderr}");
}

// Controls: non-socket spelling-opens keep their #2256 behavior —
// a pipe attaches, a regular file fresh-opens.

#[test]
fn issue2276_control_pipe_spelling_open_attaches() {
    let (stdout, stderr, _) = run("exec 4< <(printf 'PIPE\\n')\ncat /dev/fd/4\n");
    assert_eq!(stdout, "PIPE\n", "stderr: {stderr}");
}

#[test]
fn issue2276_control_regular_file_spelling_open() {
    let (stdout, stderr, _) = run("printf 'REG\\n' > reg\nexec 5<reg\ncat /dev/fd/5\n");
    assert_eq!(stdout, "REG\n", "stderr: {stderr}");
}
