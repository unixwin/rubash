//! rubash#2121 — the output-side twin of #2098 (the #1950 family,
//! output face). GNU redir.c do_redirections applies EVERY redirect's
//! open in list order; a superseded entry's descriptor is replaced by
//! the winner's dup2, but its open already ran. rubash's output walk
//! validates net targets without connecting (#1967) and its flush
//! opens only the FINAL binding, so a losing net OUTPUT entry produced
//! ZERO connections (and a refused loser was silently invisible: the
//! command ran and the winner connected). The loser's open is now
//! settled in the common reject gate (`reject_invalid_redirects`) for
//! exactly the funnels whose apply loses it (buffered builtins,
//! redirect-only commands, pipeline-stage wiring), classified by
//! `superseded_net_output_settlement_applies`; funnels with a real
//! ordered apply (plain external commands, function calls) are NOT
//! settled for — they open every entry themselves.
//!
//! The merged forms are the second face: GNU's `2>&1` is a dup2 of ONE
//! open file description, so `> /dev/tcp/... 2>&1` connects exactly
//! once. The buffered flush opened once per fd (the second fd's empty
//! stream still owed the #1967 pairing open), the pipeline stage
//! stream routing did the same, and an EXTERNAL stage's merged flush
//! opened the net word as a filesystem path (ENOENT, zero connects,
//! stage status 1). Dup-aliased Path bindings now share one flush open
//! (OutputFdState::open_flush_file) and the stage merged flush opens
//! through the registry-classifying append.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! loopback listeners counting connections and bytes, 2026-10-30):
//! every shape below matches the GNU counts byte-for-byte.

use std::io::Read;
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash2121-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg(&case)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Loopback listener counting accepted connections and the total
/// bytes received across them (the output-side #2098 harness shape).
struct Listener {
    port: u16,
    rx: mpsc::Receiver<(usize, usize)>,
}

fn start_listener() -> Listener {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking");
        let mut count = 0usize;
        let mut bytes = 0usize;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    count += 1;
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
                    let mut sink = [0u8; 256];
                    loop {
                        match stream.read(&mut sink) {
                            Ok(0) => break,
                            Ok(n) => bytes += n,
                            Err(_) => break,
                        }
                    }
                    if tx.send((count, bytes)).is_err() {
                        break;
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
        }
    });
    Listener { port, rx }
}

impl Listener {
    /// (connections, total bytes) after a settle window.
    fn stats(&self) -> (usize, usize) {
        std::thread::sleep(Duration::from_millis(600));
        let mut last = (0, 0);
        while let Ok(stats) = self.rx.try_recv() {
            last = stats;
        }
        last
    }

    fn target(&self) -> String {
        format!("/dev/tcp/127.0.0.1/{}", self.port)
    }
}

fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

fn assert_run(tag: &str, script: &str, want_code: i32, want_out: &str) {
    let (stdout, stderr, code) = run_script(tag, script);
    assert_eq!(code, Some(want_code), "{tag}: exit code (stderr: {stderr})");
    assert_eq!(stdout, want_out, "{tag}: stdout");
}

/// Face A core: a superseded net output entry on a buffered builtin
/// connects exactly once with no bytes; the winner connects once and
/// receives the output (GNU do_redirections order semantics).
#[test]
fn issue2121_builtin_output_loser_connects_once() {
    let loser = start_listener();
    let winner = start_listener();
    let (l, w) = (loser.target(), winner.target());
    assert_run("a1", &format!("echo hi > {l} > {w}\n"), 0, "");
    assert_eq!(loser.stats(), (1, 0), "loser plain output");
    assert_eq!(winner.stats(), (1, 3), "winner plain output");
}

#[test]
fn issue2121_append_and_fd2_output_losers_connect_once() {
    let loser = start_listener();
    let winner = start_listener();
    let (l, w) = (loser.target(), winner.target());
    assert_run("a4", &format!("echo hi >> {l} >> {w}\n"), 0, "");
    assert_eq!(loser.stats(), (1, 0), "loser append");
    assert_eq!(winner.stats(), (1, 3), "winner append");

    let loser = start_listener();
    let winner = start_listener();
    let (l, w) = (loser.target(), winner.target());
    // Only fd 2 is redirected here; echo's stdout still reaches the
    // script's stdout ("hi\n"), exactly as under GNU.
    assert_run("a6", &format!("echo hi 2> {l} 2> {w}\n"), 0, "hi\n");
    assert_eq!(loser.stats(), (1, 0), "loser fd 2");
    // The winner's stream is empty (echo writes no stderr) but the
    // binding still connects once — GNU opens it at redirect time.
    assert_eq!(winner.stats(), (1, 0), "winner fd 2");
}

#[test]
fn issue2121_combined_output_loser_connects_once() {
    let loser = start_listener();
    let winner = start_listener();
    let (l, w) = (loser.target(), winner.target());
    assert_run("a5", &format!("echo hi &> {l} &> {w}\n"), 0, "");
    assert_eq!(loser.stats(), (1, 0), "loser combined");
    assert_eq!(winner.stats(), (1, 3), "winner combined");

    let loser = start_listener();
    let winner = start_listener();
    let (l, w) = (loser.target(), winner.target());
    assert_run("a11", &format!("echo hi > {l} &> {w}\n"), 0, "");
    assert_eq!(loser.stats(), (1, 0), "loser before combined winner");
    assert_eq!(winner.stats(), (1, 3), "combined winner");
}

/// A combined entry superseded on fd 1 only is a PARTIAL winner: it
/// keeps fd 2, so it is not gate-settled — its own flush performs the
/// one connect (no bytes: nothing was written to fd 2).
#[test]
fn issue2121_combined_partial_winner_connects_once() {
    let partial = start_listener();
    let winner = start_listener();
    let (p, w) = (partial.target(), winner.target());
    assert_run("a10", &format!("echo hi &> {p} > {w}\n"), 0, "");
    assert_eq!(partial.stats(), (1, 0), "partial winner combined");
    assert_eq!(winner.stats(), (1, 3), "winner after combined");
}

/// A loser superseded by a close still connects once; the command
/// then fails writing to the closed fd, exactly like GNU.
#[test]
fn issue2121_output_loser_then_close_connects_once() {
    let loser = start_listener();
    let l = loser.target();
    assert_run("a9", &format!("echo hi > {l} >&-\n"), 1, "");
    assert_eq!(loser.stats(), (1, 0), "loser before close");
}

/// A redirect-only (null) command has no downstream apply at all:
/// both entries are performed by the gate/flush pair, once each.
#[test]
fn issue2121_redirect_only_loser_connects_once() {
    let loser = start_listener();
    let winner = start_listener();
    let (l, w) = (loser.target(), winner.target());
    assert_run("a21", &format!("> {l} > {w}\n"), 0, "");
    assert_eq!(loser.stats(), (1, 0), "loser redirect-only");
    assert_eq!(winner.stats(), (1, 0), "winner redirect-only");
}

/// A REFUSED loser connect fails the command at that entry (GNU
/// aborts do_redirections there): status 1 and the winner never opens.
#[test]
fn issue2121_refused_output_loser_fails_command() {
    let winner = start_listener();
    let w = winner.target();
    let refused = format!("/dev/tcp/127.0.0.1/{}", closed_port());
    assert_run("a18", &format!("echo hi > {refused} > {w}\n"), 1, "");
    assert_eq!(winner.stats(), (0, 0), "winner after refused loser");
}

/// A builtin stage's stdio wiring is the deferred model too: the
/// loser settles once through the stage gate.
#[test]
fn issue2121_pipeline_stage_output_loser_connects_once() {
    let loser = start_listener();
    let winner = start_listener();
    let (l, w) = (loser.target(), winner.target());
    assert_run("a16", &format!("echo hi > {l} > {w} | cat\n"), 0, "");
    assert_eq!(loser.stats(), (1, 0), "loser stage");
    assert_eq!(winner.stats(), (1, 3), "winner stage");
}

/// Guards: funnels with a real ordered apply open every entry
/// themselves — the gate must not settle for them (a second connect).
#[test]
fn issue2121_real_apply_funnels_stay_single_connect() {
    let loser = start_listener();
    let winner = start_listener();
    let (l, w) = (loser.target(), winner.target());
    assert_run("a2", &format!("/usr/bin/echo hi > {l} > {w}\n"), 0, "");
    assert_eq!(loser.stats(), (1, 0), "loser external plain");
    assert_eq!(winner.stats(), (1, 3), "winner external plain");

    let loser = start_listener();
    let winner = start_listener();
    let (l, w) = (loser.target(), winner.target());
    assert_run(
        "a13",
        &format!("f() {{ echo hi; }}\nf > {l} > {w}\n"),
        0,
        "",
    );
    assert_eq!(loser.stats(), (1, 0), "loser function call");
    assert_eq!(winner.stats(), (1, 3), "winner function call");
}

/// Face B: the merged dup form shares ONE open file description in
/// GNU — one connect, bytes delivered once.
#[test]
fn issue2121_merged_dup_form_connects_once() {
    let merged = start_listener();
    let p = merged.target();
    assert_run("b1", &format!("echo hi > {p} 2>&1\n"), 0, "");
    assert_eq!(merged.stats(), (1, 3), "merged dup form");

    let merged = start_listener();
    let p = merged.target();
    assert_run("b2", &format!("echo hi > {p} 2>&1 | cat\n"), 0, "");
    assert_eq!(merged.stats(), (1, 3), "merged dup form in pipeline");
}

/// An external stage's merged form used to open the net word as a
/// filesystem path: zero connects, stage status 1. GNU connects once
/// and delivers the merged bytes with status 0.
#[test]
fn issue2121_merged_external_stage_connects_once() {
    let merged = start_listener();
    let p = merged.target();
    assert_run("b4", &format!("/usr/bin/echo hi > {p} 2>&1 | cat\n"), 0, "");
    assert_eq!(merged.stats(), (1, 3), "merged external stage");
}

/// A single combined entry is one open (regression pin for the
/// gate's supersession scan: the entry's own fd-1/fd-2 mirror fields
/// must not judge it superseded by itself).
#[test]
fn issue2121_combined_single_entry_connects_once() {
    let merged = start_listener();
    let p = merged.target();
    assert_run("v7", &format!("echo hi &> {p}\n"), 0, "");
    assert_eq!(merged.stats(), (1, 3), "single combined entry");
}

/// A group's merged form: the compound apply opens once; the inner
/// commands' materialized bindings (injected group redirects) must
/// not be settled again by the gate. Both streams' bytes arrive over
/// the single connection.
#[test]
fn issue2121_group_merged_form_connects_once() {
    let merged = start_listener();
    let p = merged.target();
    assert_run(
        "b6",
        &format!("{{ echo hi; echo e >&2; }} > {p} 2>&1\n"),
        0,
        "",
    );
    assert_eq!(merged.stats(), (1, 5), "group merged form");
}

/// Over-merge guard: two DISTINCT entries naming the same target are
/// two opens in GNU (do_redirections performs both) — the flush
/// sharing must not collapse them.
#[test]
fn issue2121_two_entries_same_target_connect_twice() {
    let target = start_listener();
    let p = target.target();
    assert_run("b7", &format!("echo hi > {p} 2> {p}\n"), 0, "");
    assert_eq!(target.stats(), (2, 3), "two entries, same target");
}
