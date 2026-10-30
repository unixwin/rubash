//! Redirection / fd / process-substitution regression pins (backfill bf-c).
//!
//! rubash#1025 — `set -C` (noclobber) wrongly rejected compound-command
//! redirects to NON-existent files; GNU only refuses to overwrite a file
//! that exists (simple, brace-group, and subshell forms alike).
//! rubash#1026 — `/dev/tcp` / `/dev/udp` network redirections: a refused
//! TCP open reports `Connection refused` with status 1 (non-fatal), a
//! UDP open succeeds silently, and a live TCP endpoint round-trips.
//! rubash#1068 — `>f 2>f` on the same file opens TWO independent file
//! descriptions (own offsets); the later writer overwrites the head of
//! the earlier one's bytes instead of appending after them.
//! rubash#1103 — a bare internal `cat` fast path ignored stdin redirects:
//! `cat </dev/null` and `cat <&-` must not read the inherited pipe.
//! rubash#1359 — an auto-fd `{f}>` write redirect on a non-`exec` command
//! swallowed or reordered the command's own stdout (file target) or lost
//! it entirely (process-substitution target).
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files
//! and `-c` probes in this environment (byte-level stdout/stderr/rc and
//! resulting file bytes).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-bfc-redir-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_c_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_c_stdin(script: &str, stdin_bytes: &[u8]) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(stdin_bytes)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue_1025_noclobber_compound_redirect_to_new_file_succeeds() {
    let dir = scratch("noclobber");
    let script = "set -C\n{ echo hello; } > zzz.txt\necho rc1=$?\ncat zzz.txt\n{ echo again; } > zzz.txt\necho rc2=$?\ncat zzz.txt\n( echo sub ) > zzz2.txt\necho rc3=$?\ncat zzz2.txt\necho simple > zzz.txt\necho rc4=$?\n";
    let (stdout, stderr, code) = run_c_in(&dir, script);
    assert_eq!(
        stdout, "rc1=0\nhello\nrc2=1\nhello\nrc3=0\nsub\nrc4=1\n",
        "stderr: {stderr}"
    );
    // Both the compound form (rc2) and the simple form (rc4) refuse to
    // overwrite the existing file, with GNU's diagnostic.
    assert_eq!(
        stderr.matches("cannot overwrite existing file").count(),
        2,
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue_1026_dev_tcp_refused_reports_connection_refused() {
    // Bind then drop to obtain a port nothing listens on.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    let (stdout, stderr, _code) = run_c(&format!("exec 3<>/dev/tcp/127.0.0.1/{port}; echo rc=$?"));
    assert_eq!(stdout, "rc=1\n");
    assert!(stderr.contains("Connection refused"), "stderr: {stderr}");
}

#[test]
fn issue_1026_dev_udp_open_succeeds_silently() {
    let (stdout, stderr, _code) = run_c("exec 3<>/dev/udp/127.0.0.1/9; echo rc=$?");
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue_1026_dev_tcp_live_endpoint_round_trips() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind server");
    let port = listener.local_addr().expect("local addr").port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 5];
        stream.read_exact(&mut buf).expect("read ping");
        assert_eq!(&buf, b"ping\n");
        stream.write_all(b"pong\n").expect("write pong");
    });
    let (stdout, stderr, code) = run_c(&format!(
        "exec 3<>/dev/tcp/127.0.0.1/{port}; echo ping >&3; read -u 3 line; echo \"got=[$line]\""
    ));
    assert_eq!(stdout, "got=[pong]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    server.join().expect("server thread");
}

#[test]
fn issue_1068_same_file_double_open_uses_independent_offsets() {
    let dir = scratch("offsets");
    let script = "{ echo LONGAAAA; echo B >&2; } >f 2>f\n{ echo AAAA; echo BBBB >&2; } >g 2>g\n: > h\n{ echo LONGAAAA; echo B >&2; } >>h 2>h\n";
    let (_stdout, _stderr, code) = run_c_in(&dir, script);
    assert_eq!(code, Some(0));
    // stderr's fd (opened second, offset 0) overwrites the head of
    // stdout's longer write; the tail survives. GNU byte results:
    assert_eq!(
        std::fs::read(dir.join("f")).expect("read f"),
        b"B\nNGAAAA\n"
    );
    assert_eq!(std::fs::read(dir.join("g")).expect("read g"), b"BBBB\n");
    // Append-mode stdout + truncating stderr: same overwrite geometry.
    assert_eq!(
        std::fs::read(dir.join("h")).expect("read h"),
        b"B\nNGAAAA\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue_1103_bare_cat_honors_stdin_redirects() {
    // `</dev/null`: the internal cat must see EOF, not the pipe bytes.
    let (stdout, _stderr, code) = run_c_stdin("cat </dev/null; echo rc=$?", b"hi\n");
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(code, Some(0));
    // `<&-`: stdin closed — cat must not print the pipe bytes either.
    let (stdout, _stderr, _code) = run_c_stdin("cat <&-", b"hi\n");
    assert_eq!(stdout, "", "closed-stdin cat printed: {stdout:?}");
    // Control: without a redirect the same pipe IS cat's stdin.
    let (stdout, _stderr, code) = run_c_stdin("cat", b"hi\n");
    assert_eq!(stdout, "hi\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1359_autofd_write_redirect_keeps_stdout_intact_and_ordered() {
    let dir = scratch("autofd");
    // File target: the command's stdout reaches the terminal in order,
    // and the file receives only what is written to the auto-fd (nothing).
    let (stdout, stderr, code) = run_c_in(&dir, "echo one {f}> file1; echo rc=$?");
    assert_eq!(stdout, "one\nrc=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(std::fs::read(dir.join("file1")).expect("file1"), b"");
    // Process-substitution target: stdout must not be swallowed.
    let (stdout, _stderr, _code) = run_c_in(&dir, "echo one {f}> >(cat); echo rcps=$?");
    assert!(stdout.contains("one"), "stdout: {stdout}");
    assert!(stdout.contains("rcps=0"), "stdout: {stdout}");
    assert!(
        stdout.find("one") < stdout.find("rcps=0"),
        "stdout order: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue_1897_udp_bind_family_follows_the_resolved_address() {
    use std::net::ToSocketAddrs as _;
    use std::time::Duration;
    // Environment gate (per the issue's own seed): a v6-only host
    // name and an IPv6 loopback must both exist here, otherwise this
    // face cannot be exercised and the pin stands down instead of
    // passing on v4 evidence. `ip6-localhost` is the v6-only name in
    // the stock hosts file.
    let v6_only = ("ip6-localhost", 0u16)
        .to_socket_addrs()
        .map_or(false, |iter| {
            let addrs: Vec<_> = iter.collect();
            !addrs.is_empty() && addrs.iter().all(|addr| addr.is_ipv6())
        });
    let listener = std::net::UdpSocket::bind("[::1]:0");
    let (Ok(listener), true) = (listener, v6_only) else {
        return;
    };
    let port = listener.local_addr().expect("local addr").port();
    listener
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    // GNU _netopen6 binds/connects the getaddrinfo result itself, so
    // the datagram leaves from ::1 even though the host spelling
    // contains no ':' (oracle-verified against GNU bash 5.3.20).
    let (stdout, stderr, code) = run_c(&format!(
        "echo hello > /dev/udp/ip6-localhost/{port}; echo rc=$?"
    ));
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let mut buf = [0u8; 64];
    let (n, from) = listener.recv_from(&mut buf).expect("datagram arrives");
    assert_eq!(&buf[..n], b"hello\n");
    assert!(from.is_ipv6(), "datagram source: {from}");
}

#[test]
fn issue_1894_unresolvable_host_reports_name_line_then_invalid_argument() {
    // GNU _netopen6 (lib/sh/netopen.c:205) reports a getaddrinfo
    // failure through internal_error BEFORE the redirection_error
    // line: `{host}: Name or service not known`, then the target line
    // carries EINVAL (netopen.c:227). A 300-char host name fails
    // resolution locally on every resolver (no DNS traffic), which
    // keeps the pin deterministic offline; oracle-verified against
    // GNU bash 5.3.20.
    let host = "a".repeat(300);
    let (stdout, stderr, _code) = run_c(&format!("( exec 3<>/dev/tcp/{host}/80; echo rc=$? )"));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains(&format!("{host}: Name or service not known")),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains(&format!("/dev/tcp/{host}/80: Invalid argument")),
        "stderr: {stderr}"
    );
    // The empty host is the same EAI_NONAME shape with an empty name.
    let (stdout, stderr, _code) = run_c("( exec 3<>/dev/tcp//59998; echo rc=$? )");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains(": Name or service not known"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("/dev/tcp//59998: Invalid argument"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1895_multi_slash_target_splits_at_first_slash() {
    // GNU netopen splits at the FIRST slash (lib/sh/netopen.c:291
    // strchr): host `a`, service `b/80` — the service word fails, so
    // the diagnostic names `b/80`, not `b`. Oracle-verified against
    // GNU bash 5.3.20 for both protocols.
    let (stdout, stderr, _code) = run_c("( exec 3<>/dev/tcp/a/b/80; echo rc=$? )");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("b/80: Servname not supported for ai_socktype"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("/dev/tcp/a/b/80: Invalid argument"),
        "stderr: {stderr}"
    );
    let (stdout, stderr, _code) = run_c("( exec 3<>/dev/udp/a/b/53; echo rc=$? )");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("b/53: Servname not supported for ai_socktype"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("/dev/udp/a/b/53: Invalid argument"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1896_service_alias_resolves_like_getservbyname() {
    // `www` is http's alias in the services database on unix
    // (/etc/services) and on Windows (drivers\etc\services) alike.
    // Nothing listens on port 80 in the test environment, so a
    // RESOLVED alias surfaces as Connection refused; an unresolved
    // one would surface as the Servname diagnostic instead (GNU
    // netopen's service lookup is getservbyname, which walks the
    // alias column — lib/sh/netopen.c:135).
    let (stdout, stderr, _code) = run_c("( exec 3<>/dev/tcp/127.0.0.1/www; echo rc=$? )");
    assert_eq!(stdout, "rc=1\n");
    assert!(stderr.contains("Connection refused"), "stderr: {stderr}");
    assert!(!stderr.contains("Servname"), "stderr: {stderr}");
}

#[test]
fn issue_1359_autofd_write_redirect_from_function_and_group() {
    let dir = scratch("autofd2");
    // Function form: writing to $f inside the function lands in the file.
    let (stdout, stderr, code) =
        run_c_in(&dir, "g() { echo one >&$f; }; g {f}> file2; echo rcg=$?");
    assert_eq!(stdout, "rcg=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(std::fs::read(dir.join("file2")).expect("file2"), b"one\n");
    // Brace-group form: same, no `ambiguous redirect`.
    let (_stdout, stderr, code) = run_c_in(&dir, "{ echo one >&$f; } {f}> file3; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(!stderr.contains("ambiguous"), "stderr: {stderr}");
    assert_eq!(std::fs::read(dir.join("file3")).expect("file3"), b"one\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Start a counting greeting server for the single-open pins
/// (rubash#1950/#1967): every accepted connection is counted, sent
/// the greeting, and half-closed (write side) so a client reading
/// to EOF finishes at once; whatever the client sends on that
/// connection is recorded. Returns the port, the shared log, a stop
/// flag, and the accept thread.
fn start_counting_server() -> (
    u16,
    std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::thread::JoinHandle<()>,
) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind counting server");
    let port = listener.local_addr().expect("local addr").port();
    listener.set_nonblocking(true).expect("nonblocking");
    let log: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let thread_log = Arc::clone(&log);
    let thread_stop = Arc::clone(&stop);
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !thread_stop.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.write_all(b"GREETING\n");
                    let _ = stream.shutdown(std::net::Shutdown::Write);
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(3)));
                    let mut received = Vec::new();
                    let mut buf = [0u8; 1024];
                    loop {
                        match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => received.extend_from_slice(&buf[..n]),
                            Err(_) => break,
                        }
                    }
                    thread_log.lock().expect("log lock").push(received);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });
    (port, log, stop, server)
}

fn counting_server_result(
    log: &std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    server: std::thread::JoinHandle<()>,
) -> Vec<Vec<u8>> {
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    server.join().expect("counting server thread");
    log.lock().expect("log lock").clone()
}

#[test]
fn issue_1950_cat_input_redirect_opens_exactly_one_connection() {
    // GNU netopen runs once per redirection (redir.c
    // do_redirections): a multi-accept peer sees ONE connection and
    // the reader gets its greeting. The probe/setup walks must not
    // connect ahead of the bind's open.
    let (port, log, stop, server) = start_counting_server();
    let (stdout, stderr, code) = run_c(&format!("cat < /dev/tcp/127.0.0.1/{port}"));
    let connections = counting_server_result(&log, &stop, server);
    assert_eq!(stdout, "GREETING\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(connections.len(), 1, "connections: {connections:?}");
}

#[test]
fn issue_1950_comsub_cat_opens_exactly_one_connection() {
    let (port, log, stop, server) = start_counting_server();
    let (stdout, stderr, code) = run_c(&format!(
        "x=$(cat < /dev/tcp/127.0.0.1/{port}); echo \"x=[$x]\""
    ));
    let connections = counting_server_result(&log, &stop, server);
    assert_eq!(stdout, "x=[GREETING]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(connections.len(), 1, "connections: {connections:?}");
}
#[test]
fn issue_1967_null_command_output_redirect_opens_exactly_one_connection() {
    // `: > /dev/tcp/...` performs GNU's one open (the connection
    // itself is the side effect) and succeeds; a setup probe must
    // not add a second, phantom connection.
    let (port, log, stop, server) = start_counting_server();
    let (stdout, stderr, code) = run_c(&format!(": > /dev/tcp/127.0.0.1/{port}; echo rc=$?"));
    let connections = counting_server_result(&log, &stop, server);
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(connections.len(), 1, "connections: {connections:?}");
    assert_eq!(connections[0], Vec::<u8>::new());
}

#[test]
fn issue_1967_echo_output_redirect_opens_one_connection_and_delivers() {
    let (port, log, stop, server) = start_counting_server();
    let (stdout, stderr, code) = run_c(&format!("echo hi > /dev/tcp/127.0.0.1/{port}"));
    let connections = counting_server_result(&log, &stop, server);
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(connections.len(), 1, "connections: {connections:?}");
    assert_eq!(connections[0], b"hi\n".to_vec());
}
