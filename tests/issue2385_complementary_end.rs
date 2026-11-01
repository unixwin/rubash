//! rubash#2385 — complementary-end opens of pipe endpoints: a
//! WRITE-mode open of a pipe read end's fd alias spelling
//! (`/dev/fd/N`, `/proc/self/fd/N`, `/dev/stdin`) must materialize
//! the pipe's OTHER end — the bytes join the very stream the read
//! end serves.
//!
//! GNU contract (bash 5.3 C source, read first): the spelling open
//! is redir.c `do_redirections`' plain open() of the magic link
//! (redir_open, redir.c:725) — on a HAVE_DEV_FD build the kernel
//! attaches the name to the descriptor's stream, and for a pipe
//! that attachment IS the pipe's other end; bash carries no
//! pipe-specific code here. A DUP is not an open: `>&N` of a read
//! end keeps failing at the write through sh_chkwrite
//! (builtins/common.c:320-334). `set -C`'s noclobber_open
//! (redir.c:676) refuses only regular files, never a pipe link.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-11-01; probe corpus in the wave workdir). Where GNU leaves
//! producer/append interleave to a scheduling race (a concurrent
//! producer), the materialized model deterministically serves the
//! producer's bytes first — a valid sample of the GNU contract.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    run_script_stdin(tag, script, None)
}

/// `stdin_bytes`: when Some, the shell's stdin is a REAL pipe the
/// harness pre-fills with those bytes (the u01/x06/x07 shapes —
/// stdin itself is the pipe whose complementary end is opened).
fn run_script_stdin(
    tag: &str,
    script: &str,
    stdin_bytes: Option<&[u8]>,
) -> (String, String, Option<i32>) {
    let (tx, rx) = mpsc::channel();
    let script = script.to_string();
    let stdin_bytes = stdin_bytes.map(<[u8]>::to_vec);
    let panic_tag = tag.to_string();
    let tag = tag.to_string();
    std::thread::spawn(move || {
        let dir = std::env::temp_dir().join(format!("rubash2385-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let case = dir.join("case.sh");
        std::fs::write(&case, &script).expect("write fixture");
        let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
        command
            .current_dir(&dir)
            .arg(&case)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = if let Some(bytes) = &stdin_bytes {
            command.stdin(Stdio::piped());
            let mut child = command.spawn().expect("spawn rubash");
            child
                .stdin
                .as_mut()
                .expect("stdin pipe")
                .write_all(bytes)
                .expect("prefill stdin pipe");
            // Close the harness end: the pre-filled bytes are in the
            // pipe; the shell's own complementary writes still land
            // behind them, and readers see EOF after both.
            drop(child.stdin.take());
            child
        } else {
            command.stdin(Stdio::null());
            command.spawn().expect("spawn rubash")
        };
        let output = child.wait_with_output().expect("run rubash");
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

/// Lines of `s` as a sorted set — the producer/append interleave
/// on a kernel pipe is a scheduling race in GNU itself (the
/// producer child and the complementary writer share one pipe), so
/// stream-content pins compare as sets; the producer's own bytes
/// keep their relative order inside any single write, which the
/// set comparison does not need to assume.
fn sorted_lines(s: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = s.lines().collect();
    lines.sort_unstable();
    lines
}

/// s02: the plain shape — a transient write-mode spelling open of a
/// procsub read end appends into the stream.
#[test]
fn issue2385_s02_write_open_of_procsub_read_end_appends() {
    let (out, err, _rc) = run_script(
        "s02",
        "exec 3< <(echo hi)\necho x > /dev/fd/3\necho \"rc:$?\"\ncat <&3\n",
    );
    assert_eq!(err, "");
    let (rc_line, stream) = out.split_once('\n').expect("rc line");
    assert_eq!(rc_line, "rc:0");
    assert_eq!(sorted_lines(stream), ["hi", "x"]);
}

/// u02: producer bytes and appended bytes share one FIFO stream.
#[test]
fn issue2385_u02_appended_bytes_follow_producer_bytes() {
    let (out, err, _rc) = run_script(
        "u02",
        "exec 3< <(printf 'p1\\np2\\n')\necho mine > /dev/fd/3\ncat <&3\n",
    );
    assert_eq!(err, "");
    assert_eq!(sorted_lines(&out), ["mine", "p1", "p2"]);
}

/// The `/proc/self/fd/N` spelling is the same open (x08).
#[test]
fn issue2385_x08_proc_self_fd_spelling_appends() {
    let (out, err, _rc) = run_script(
        "x08",
        "exec 3< <(echo hi)\necho x > /proc/self/fd/3\ncat <&3\n",
    );
    assert_eq!(err, "");
    assert_eq!(sorted_lines(&out), ["hi", "x"]);
}

/// x01: the persistent form — `exec 4> /dev/fd/3` binds the
/// complementary end; later writes through fd 4 append. The read
/// runs after fd 4 closes: while it stays open it is a live write
/// end of the pipe and GNU's reader would block for more input,
/// exactly as here.
#[test]
fn issue2385_x01_exec_bound_complementary_end_writes() {
    let (out, err, _rc) = run_script(
        "x01",
        "exec 3< <(echo hi)\nexec 4> /dev/fd/3\necho \"bind:$?\"\necho x >&4\nexec 4>&-\ncat <&3\n",
    );
    assert_eq!(err, "");
    let (bind_line, stream) = out.split_once('\n').expect("bind line");
    assert_eq!(bind_line, "bind:0");
    assert_eq!(sorted_lines(stream), ["hi", "x"]);
}

/// x02: the append form is the same write — a pipe has no offset
/// for `>` to truncate or `>>` to seek past.
#[test]
fn issue2385_x02_append_form_is_the_same_write() {
    let (out, err, _rc) = run_script("x02", "exec 3< <(echo hi)\necho x >> /dev/fd/3\ncat <&3\n");
    assert_eq!(err, "");
    assert_eq!(sorted_lines(&out), ["hi", "x"]);
}

/// x03: `exec 4<> /dev/fd/3` binds ONE read-write description of
/// the pipe: writes through fd 4 land in the stream fd 4 reads.
/// (Line reads, not a cat to EOF: fd 4 itself is a live write end,
/// so EOF waits for a close that never comes — GNU identical.)
#[test]
fn issue2385_x03_readwrite_bind_serves_both_directions() {
    let (out, err, _rc) = run_script(
        "x03",
        "exec 3< <(echo hi)\nexec 4<> /dev/fd/3\necho \"bind:$?\"\necho x >&4\nread -u 4 a\nread -u 4 b\necho \"got:$a,$b\"\n",
    );
    assert_eq!(err, "");
    let (bind_line, got) = out.split_once('\n').expect("bind line");
    assert_eq!(bind_line, "bind:0");
    let got = got.trim_end().strip_prefix("got:").expect("got line");
    let mut lines: Vec<&str> = got.split(',').collect();
    lines.sort_unstable();
    assert_eq!(lines, ["hi", "x"]);
}

/// x09: a herestring's materialized stream is a channel too.
#[test]
fn issue2385_x09_herestring_stream_appends() {
    let (out, err, _rc) = run_script("x09", "exec 3<<< \"seed\"\necho x > /dev/fd/3\ncat <&3\n");
    assert_eq!(err, "");
    assert_eq!(out, "seed\nx\n");
}

/// y04: repeated complementary opens accumulate in one stream.
#[test]
fn issue2385_y04_repeated_opens_accumulate() {
    let (out, err, _rc) = run_script(
        "y04",
        "exec 3< <(echo hi)\necho a > /dev/fd/3\necho b >> /dev/fd/3\ncat <&3\n",
    );
    assert_eq!(err, "");
    assert_eq!(sorted_lines(&out), ["a", "b", "hi"]);
}

/// y05: appends land at the tail regardless of the read cursor —
/// a partially consumed stream serves its remainder first.
#[test]
fn issue2385_y05_append_ignores_read_cursor() {
    let (out, err, _rc) = run_script(
        "y05",
        "exec 3< <(printf 'l1\\nl2\\nl3\\n')\nread -u 3 first\necho \"first:$first\"\necho tail > /dev/fd/3\ncat <&3\n",
    );
    assert_eq!(err, "");
    assert_eq!(out, "first:l1\nl2\nl3\ntail\n");
}

/// y01: `set -C` never refuses a pipe link — noclobber gates
/// regular files only.
#[test]
fn issue2385_y01_noclobber_does_not_refuse_a_pipe_link() {
    let (out, err, _rc) = run_script(
        "y01",
        "set -C\nexec 3< <(echo hi)\necho x > /dev/fd/3\necho \"rc:$?\"\ncat <&3\n",
    );
    assert_eq!(err, "");
    let (rc_line, stream) = out.split_once('\n').expect("rc line");
    assert_eq!(rc_line, "rc:0");
    assert_eq!(sorted_lines(stream), ["hi", "x"]);
}

/// x05 GUARD: a dup is not an open — `>&3` of a read end still
/// fails at the write (sh_chkwrite), and the stream is untouched.
#[test]
fn issue2385_x05_dup_of_read_end_stays_ebadf() {
    let (out, err, _rc) = run_script(
        "x05",
        "exec 3< <(echo hi)\necho x >&3\necho \"rc:$?\"\ncat <&3\n",
    );
    assert!(
        err.contains("write error: Bad file descriptor"),
        "stderr: {err}"
    );
    assert_eq!(out, "rc:1\nhi\n");
}

/// y02 GUARD: a closed source fd's spelling open fails ENOENT at
/// redirect time, exactly like GNU's dangling magic link.
#[test]
fn issue2385_y02_closed_source_spelling_open_fails_enoent() {
    let (out, err, _rc) = run_script("y02", "echo x > /dev/fd/9\necho \"rc:$?\"\n");
    assert!(err.contains("No such file or directory"), "stderr: {err}");
    assert_eq!(out, "rc:1\n");
}

/// The pipeline-stage face: a stage's spelling redirect appends to
/// the channel instead of the stage pipe.
#[test]
fn issue2385_stage_spelling_redirect_appends() {
    let (out, err, _rc) = run_script(
        "stage",
        "exec 3< <(echo hi)\necho x > /dev/fd/3 | cat\ncat <&3\n",
    );
    assert_eq!(err, "");
    assert_eq!(sorted_lines(&out), ["hi", "x"]);
}

/// u01 (unix): stdin is itself a pipe — `> /dev/stdin` writes into
/// the very pipe the shell then reads.
#[cfg(unix)]
#[test]
fn issue2385_u01_stdin_pipe_complementary_write() {
    let (out, err, _rc) = run_script_stdin(
        "u01",
        "echo into-pipe > /dev/stdin\necho \"rc:$?\"\nread line\necho \"got:$line\"\n",
        Some(b""),
    );
    assert_eq!(err, "");
    assert_eq!(out, "rc:0\ngot:into-pipe\n");
}

/// x06 (unix): bytes already in the stdin pipe keep FIFO priority
/// over the complementary append.
#[cfg(unix)]
#[test]
fn issue2385_x06_prefilled_stdin_pipe_keeps_fifo_order() {
    let (out, err, _rc) = run_script_stdin(
        "x06",
        "echo added > /dev/stdin\ncat\n",
        Some(b"seed-line\n"),
    );
    assert_eq!(err, "");
    assert_eq!(out, "seed-line\nadded\n");
}

/// x07 (unix): an external child reading stdin sees the builtin's
/// complementary write — the stream is a real kernel pipe.
#[cfg(unix)]
#[test]
fn issue2385_x07_external_reader_sees_complementary_write() {
    let (out, err, _rc) = run_script_stdin(
        "x07",
        "echo builtin-line > /dev/stdin\n/bin/cat\n",
        Some(b""),
    );
    assert_eq!(err, "");
    assert_eq!(out, "builtin-line\n");
}

/// x10 (unix): a dup of stdin carries stdin's endpoint — the dup's
/// spelling resolves to the same pipe (endpoint-keyed, never
/// number-keyed).
#[cfg(unix)]
#[test]
fn issue2385_x10_stdin_dup_spelling_resolves_to_the_pipe() {
    let (out, err, _rc) = run_script_stdin(
        "x10",
        "exec 3<&0\necho via-dup > /dev/fd/3\nread -u 3 line\necho \"got:$line\"\n",
        Some(b""),
    );
    assert_eq!(err, "");
    assert_eq!(out, "got:via-dup\n");
}

/// x04 (unix): an external stage's own complementary write lands
/// in the stream — the child cannot hold the in-memory queue, so
/// its bytes ride a capture the parent's routing appends.
#[cfg(unix)]
#[test]
fn issue2385_x04_external_stage_complementary_write_lands() {
    let (out, err, _rc) = run_script(
        "x04",
        "exec 3< <(echo hi)\n/bin/echo x > /dev/fd/3\necho \"rc:$?\"\ncat <&3\n",
    );
    assert_eq!(err, "");
    let (rc_line, stream) = out.split_once('\n').expect("rc line");
    assert_eq!(rc_line, "rc:0");
    assert_eq!(sorted_lines(stream), ["hi", "x"]);
}

/// y06 (unix): a coproc's read end takes a complementary write —
/// injected bytes join the shell's coproc read stream.
#[cfg(unix)]
#[test]
fn issue2385_y06_coproc_read_end_takes_complementary_write() {
    let (out, err, _rc) = run_script(
        "y06",
        "coproc cat\nexec 9> /dev/fd/${COPROC[0]}\necho \"bind:$?\"\necho injected >&9\necho via-cat >&${COPROC[1]}\nread -t 5 -u ${COPROC[0]} a\nread -t 5 -u ${COPROC[0]} b\necho \"got:$a,$b\"\n",
    );
    assert_eq!(err, "");
    // Two writers share one kernel pipe (the injected write end and
    // cat's stdout), so the interleave is a scheduling race in GNU
    // too — the pin is the SET of delivered lines, plus the bind.
    assert!(out.starts_with("bind:0\ngot:"), "stdout: {out}");
    let got = out.trim_end().rsplit("got:").next().unwrap_or("");
    let mut lines: Vec<&str> = got.split(',').collect();
    lines.sort_unstable();
    assert_eq!(lines, ["injected", "via-cat"], "stdout: {out}");
}
