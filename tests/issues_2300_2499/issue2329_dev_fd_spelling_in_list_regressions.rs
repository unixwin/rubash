//! rubash#2329: a `/dev/fd/N` PATH spelling of an fd bound by an
//! EARLIER ENTRY OF THE SAME redirect list must resolve against
//! that in-list binding, not the ambient fd table.
//!
//! GNU runs ONE ordered walk (redir.c:237 do_redirections): when
//! the spelling is reached, fd N carries the binding the list's
//! earlier entries gave it, and the spelling performs an open of
//! that descriptor (the RF_DEVFD spelling class, redir.c:603; the
//! non-HAVE_DEV_FD dup special case, redir.c:629) — for a regular
//! file that open is FRESH (a new description, offset 0, or a
//! truncating write open for an output spelling). The pre-fix
//! appliers resolved the spelling against the ambient table only,
//! so a plain-face `cat 9<data.txt 0</dev/fd/9` failed ENOENT
//! (fd 9 unbound ambiently) where GNU prints the file body.
//! Oracle: probes A–J against GNU bash 5.3.20 (script files).
//!
//! FD-ENVIRONMENT SENSITIVITY: these pins spawn the shell as a
//! child process and use fds 8/9/10 in the scripts. An inherited
//! ambient descriptor on one of those numbers in the test harness
//! (e.g. a build-slot lock fd) can mask or fake a result — the
//! collector re-runs this file under a scrubbed fd environment
//! (only fds 0/1/2 open). The socket shapes use the shell's own
//! /dev/tcp client against a test-process listener (the #2276
//! fixture pattern).

use std::io::Write;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

/// Bind an ephemeral listener and serve "hello\n" on every accepted
/// connection.
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

fn run(test: &str, script: &str) -> (String, String, i32, String) {
    let scratch =
        std::env::temp_dir().join(format!("rubash-issue2329-{}-{}", std::process::id(), test));
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
    let data_after =
        std::fs::read_to_string(scratch.join("data.txt")).expect("read data file after");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
        data_after,
    )
}

#[test]
fn issue2329_plain_external_input_spelling_reads_in_list_file() {
    // Probe A: GNU prints the body, rc 0.
    let (stdout, stderr, code, _) = run("a", "cat 9<data.txt 0</dev/fd/9");
    assert_eq!(stdout, "file-body\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2329_builtin_read_input_spelling_reads_in_list_file() {
    // Probe D: the builtin face resolves the spelling in-list too.
    let (stdout, stderr, code, _) = run("d", "read line 9<data.txt 0</dev/fd/9\necho \"[$line]\"");
    assert_eq!(stdout, "[file-body]\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2329_builtin_output_spelling_truncates_in_list_file() {
    // Probe E: `2>/dev/fd/9` with 9 in-list-bound to data.txt is a
    // FRESH truncating open of data.txt (GNU), so the file is empty
    // afterwards even though echo wrote nothing to stderr.
    let (stdout, stderr, code, data_after) = run(
        "e",
        "echo hello 9<data.txt 2>/dev/fd/9\necho marker\ncat data.txt\necho end",
    );
    assert_eq!(stdout, "hello\nmarker\nend\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(data_after, "", "the fresh output open must truncate");
}

#[test]
fn issue2329_external_stdout_spelling_truncates_in_list_file() {
    // Probe F: `1>/dev/fd/9` after `9<data.txt` — cat's stdout is a
    // fresh (truncating) open of data.txt; cat reads /dev/null, so
    // the file ends empty and nothing reaches the real stdout.
    let (stdout, stderr, code, data_after) =
        run("f", "cat 9<data.txt 1>/dev/fd/9 </dev/null\necho done");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(data_after, "", "the fresh output open must truncate");
}

#[test]
fn issue2329_dup_chain_spelling_resolves_through_in_list_dup() {
    // Probe I: `10<&9` binds 10 to 9's in-list file binding; the
    // spelling of 10 opens that file fresh.
    let (stdout, stderr, code, _) = run("i", "cat 9<data.txt 10<&9 0</dev/fd/10");
    assert_eq!(stdout, "file-body\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2329_exec_face_controls_stay_green() {
    // Probes H/J: the exec face already walked the list in order;
    // the fix must not move it.
    let (stdout, stderr, code, _) = run("h", "exec 9<data.txt 0</dev/fd/9\ncat");
    assert_eq!(stdout, "file-body\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
    let (stdout, stderr, code, _) = run("j", "exec 9<data.txt 10<&9 0</dev/fd/10\ncat");
    assert_eq!(stdout, "file-body\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2329_ambient_spelling_across_commands_stays_green() {
    // Probe B control: the binding comes from an EARLIER command
    // (ambient table) — the NotBound path must keep serving it.
    let (stdout, stderr, code, _) = run("b", "exec 9<data.txt\ncat 0</dev/fd/9");
    assert_eq!(stdout, "file-body\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2329_in_list_file_bind_shadows_ambient_socket() {
    // The #2316 battery shape on the plain face: fd 8 is a socket
    // ambiently, but THIS list rebinds it to a file before the
    // spelling — GNU opens the file.
    let port = hello_server();
    let script = format!("exec 8<>/dev/tcp/127.0.0.1/{port}\ncat 8<data.txt 0</dev/fd/8");
    let (stdout, stderr, code, _) = run("shadow", &script);
    assert_eq!(stdout, "file-body\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2329_in_list_socket_spelling_still_refused() {
    // The converse: an in-list SOCKET binding's spelling open is
    // ENXIO in GNU (redir.c:629's class judgment at the spelling's
    // position) — the command is refused, nothing streams.
    let port = hello_server();
    let script = format!("cat 8<>/dev/tcp/127.0.0.1/{port} 0</dev/fd/8\necho rc=$?");
    let (stdout, stderr, _, _) = run("sock", &script);
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(stderr.contains("/dev/fd/8"), "stderr: {stderr}");
}
