//! rubash#2316: the #2276 ENXIO gate must judge the endpoint the
//! ordered walk would open AT the spelling's list position.
//!
//! GNU runs ONE ordered walk (redir.c:237 do_redirections): when a
//! `/dev/fd/N` spelling is reached, fd N carries the binding the
//! list's earlier entries gave it — a dup of a socket makes the
//! spelling's target a socket (ENXIO), and a file bind in the list
//! shadows an ambient socket (the spelling opens the file). The
//! pre-fix gate judged only the ambient table (plus a side scan for
//! direct net binds), so a dup-bound socket streamed and an
//! ambient socket shadowed by a list file bind was refused.
//! Oracle: probes/shape{A..E}.sh against GNU bash 5.3.20.
//!
//! The socket endpoint under test is the shell's own /dev/tcp
//! connection; the test process plays the server (the #2276 fixture).
//!
//! Face note: the gate is face-independent (it runs before dispatch),
//! and the refuse shapes pin it on the plain command face. The mirror
//! ("must not refuse") shapes are pinned on the exec face, whose
//! applier walks the same list sequentially end to end: on the plain
//! command face a spelling of an in-list binding is resolved by the
//! spawn plan's own channel, a pre-existing gap independent of this
//! gate (it fails identically with no socket anywhere in the shape:
//! `cat 9<data.txt 0</dev/fd/9` -> ENOENT against GNU's file-body).
//! The exec-face shapes below carry the identical gate inputs —
//! same list, same binds, same spelling position — and were verified
//! byte-identical against GNU 5.3.20.

use std::io::Write;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

/// Bind an ephemeral listener and serve "hello\n" on every accepted
/// connection (each shell face reconnects before use).
fn hello_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        for _ in 0..16 {
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

fn run(test: &str, script: &str) -> (String, String, i32) {
    let scratch =
        std::env::temp_dir().join(format!("rubash-issue2316-{}-{}", std::process::id(), test));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("create scratch");
    std::fs::write(scratch.join("data.txt"), "file-body\n").expect("write data file");
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
fn issue2316_dup_bound_socket_spelling_refused() {
    let port = hello_server();
    let script = format!("exec 8<>/dev/tcp/127.0.0.1/{port}\ncat 9<&8 0</dev/fd/9\necho rc=$?\n");
    let (stdout, stderr, _) = run("dup_bound_socket", &script);
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(stderr.contains("/dev/fd/9"), "stderr: {stderr}");
    assert!(
        stderr.contains("No such device or address"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2316_dup_chain_socket_spelling_refused() {
    let port = hello_server();
    let script =
        format!("exec 8<>/dev/tcp/127.0.0.1/{port}\ncat 9<&8 10<&9 0</dev/fd/10\necho rc=$?\n");
    let (stdout, stderr, _) = run("dup_chain_socket", &script);
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(stderr.contains("/dev/fd/10"), "stderr: {stderr}");
    assert!(
        stderr.contains("No such device or address"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2316_ambient_socket_rebound_to_file_spelling_opens() {
    let port = hello_server();
    let script = format!(
        "exec 9<>/dev/tcp/127.0.0.1/{port}\nexec 9<data.txt 0</dev/fd/9\ncat\necho rc=$?\n"
    );
    let (stdout, stderr, _) = run("ambient_rebound_file", &script);
    assert_eq!(stdout, "file-body\nrc=0\n", "stderr: {stderr}");
}

#[test]
fn issue2316_net_bind_then_file_rebind_spelling_opens() {
    let port = hello_server();
    let script =
        format!("exec 9<>/dev/tcp/127.0.0.1/{port} 9<data.txt 0</dev/fd/9\ncat\necho rc=$?\n");
    let (stdout, stderr, _) = run("net_then_file", &script);
    assert_eq!(stdout, "file-body\nrc=0\n", "stderr: {stderr}");
}

#[test]
fn issue2316_dup_bound_file_spelling_opens() {
    let script = "exec 9<data.txt 10<&9 0</dev/fd/10\ncat\necho rc=$?\n";
    let (stdout, stderr, _) = run("dup_bound_file", script);
    assert_eq!(stdout, "file-body\nrc=0\n", "stderr: {stderr}");
}

#[test]
fn issue2316_exec_list_dup_socket_spelling_refused() {
    // The issue's primary shape inside a single list: dup binds the
    // socket at position 1, the spelling at position 2 must refuse.
    // A failed exec redirect ends the non-interactive shell, so the
    // pin reads the process status, not an rc line.
    let port = hello_server();
    let script = format!("exec 8<>/dev/tcp/127.0.0.1/{port} 9<&8 0</dev/fd/9\n");
    let (stdout, stderr, status) = run("exec_list_dup_socket", &script);
    assert_eq!(status, 1, "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(stderr.contains("/dev/fd/9"), "stderr: {stderr}");
    assert!(
        stderr.contains("No such device or address"),
        "stderr: {stderr}"
    );
}

// Controls: the ambient judgment is unchanged when the list does not
// rebind the source, and descriptor faces (dups, not spellings) keep
// streaming a socket the list bound by dup.

#[test]
fn issue2316_control_ambient_socket_spelling_still_refused() {
    let port = hello_server();
    let script = format!("exec 8<>/dev/tcp/127.0.0.1/{port}\ncat 0</dev/fd/8\necho rc=$?\n");
    let (stdout, stderr, _) = run("ambient_control", &script);
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("No such device or address"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2316_control_dup_face_streams_socket() {
    let port = hello_server();
    let script = format!("exec 8<>/dev/tcp/127.0.0.1/{port}\ncat 9<&8 0<&9\necho rc=$?\n");
    let (stdout, stderr, _) = run("dup_face_control", &script);
    assert_eq!(stdout, "hello\nrc=0\n", "stderr: {stderr}");
}
