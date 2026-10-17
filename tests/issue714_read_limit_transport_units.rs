//! rubash#714: `read -n N` / `-N N` limit accounting must count decoded
//! units, not transport characters.
//!
//! A carrier pair (`U+E000` + `U+E001..=U+E100`) is rubash's in-band
//! encoding of one raw stream byte outside the transport set (ESC `0x1b`
//! travels as `U+E000`+`U+E01C`; see `substitution_metadata.rs` /
//! `markers.rs`). The read builtin limited the record by *transport*
//! characters, so a pair was counted as two: a limit that landed inside the
//! pair emitted the bare introducer (`ee 80 80`) instead of the byte, and a
//! limit one past the pair dropped the byte after it. GNU builtins/read.def
//! counts one per decoded character — read.def:884 `nr++`, read.def:886
//! `if (nchars > 0 && nr >= nchars) break;`, with read.def:874 `read_mbchar`
//! consuming a multibyte character whole — so a pair is one unit and is
//! never split.
//!
//! Byte-for-byte baseline: WSL GNU bash 5.3.0, scripts run from a file
//! (`wsl.exe bash <file>`). The cover exercises the FUNCTION_STDIN carrier
//! mirror (the inner-pipeline shape where the split reproduced) plus the
//! outer-pipe shape from the issue, and the plain-ASCII / multibyte guards
//! for the unchanged path.
//!
//! The child runs in a throwaway scratch directory so no relative probe file
//! can land in the repository root (same discipline as #763/#765).

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static SCRATCH_SEQ: AtomicUsize = AtomicUsize::new(0);

/// A unique, per-call scratch directory for the child's CWD.
fn scratch_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-714-{}-{}",
        std::process::id(),
        SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `rubash -c <script>` with empty stdin in a sandboxed CWD, returning
/// raw stdout bytes, stderr and the exit status.
fn run_c(script: &str) -> (Vec<u8>, String, Option<i32>) {
    let sandbox = scratch_dir();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(&sandbox)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&sandbox);
    (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Run `rubash -c <script>` with `stdin` piped in, sandboxed CWD.
fn run_stdin(script: &str, stdin: &[u8]) -> (Vec<u8>, String, Option<i32>) {
    let sandbox = scratch_dir();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(&sandbox)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_dir_all(&sandbox);
    (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `printf '<producer>' | { read <read_args> c; printf %s "$c"; }` — the
/// second stage's `read` is served from the upstream stage's FUNCTION_STDIN
/// carrier mirror. `producer` keeps printf's `\0NN` octal escapes verbatim.
fn piped_read(producer: &str, read_args: &str) -> Vec<u8> {
    let script = format!("printf '{producer}' | {{ read {read_args} c; printf %s \"$c\"; }}");
    let (stdout, stderr, rc) = run_c(&script);
    assert_eq!(rc, Some(0), "rc for {script:?}; stderr: {stderr:?}");
    assert_eq!(stderr, "", "stderr for {script:?}");
    stdout
}

/// `read -n 3` over `AB\033C`: ESC is one carrier pair and must consume one
/// unit, so the record is exactly `41 42 1b` (GNU), not `41 42 ee 80 80`.
#[test]
fn read_n_counts_carrier_pair_as_one_unit() {
    assert_eq!(piped_read(r"AB\033C", "-n3"), vec![0x41, 0x42, 0x1b]);
}

/// A pair landing exactly on the limit boundary is emitted whole, and a pair
/// just past the budget is excluded entirely (no half-pair).
#[test]
fn read_n_cut_at_pair_boundary_stays_whole() {
    // Pair is the last included unit.
    assert_eq!(piped_read(r"A\033B", "-n2"), vec![0x41, 0x1b]);
    // Pair is the next unit but the budget stops before it.
    assert_eq!(piped_read(r"AB\033C", "-n2"), vec![0x41, 0x42]);
    // Pair is the first unit.
    assert_eq!(piped_read(r"\033A", "-n1"), vec![0x1b]);
}

/// A pair must not eat two units of the budget: `-n 4` keeps the byte after
/// the pair (`41 42 1b 43`).
#[test]
fn read_n_after_pair_keeps_following_byte() {
    assert_eq!(piped_read(r"AB\033C", "-n4"), vec![0x41, 0x42, 0x1b, 0x43]);
    assert_eq!(piped_read(r"A\033BC", "-n3"), vec![0x41, 0x1b, 0x42]);
}

/// `-N` is the same decoded-unit budget with the delimiter ignored.
#[test]
fn read_upper_n_exact_limit_counts_pair_once() {
    assert_eq!(piped_read(r"AB\033C", "-N3"), vec![0x41, 0x42, 0x1b]);
    assert_eq!(piped_read(r"A\033BC", "-N4"), vec![0x41, 0x1b, 0x42, 0x43]);
    assert_eq!(piped_read(r"\033\033", "-N2"), vec![0x1b, 0x1b]);
}

/// Several carrier pairs in a row each count once and stay whole.
#[test]
fn read_n_multiple_carriers_in_payload() {
    assert_eq!(piped_read(r"\033\014\021X", "-n3"), vec![0x1b, 0x0c, 0x11]);
    assert_eq!(
        piped_read(r"\033\014\021X", "-n4"),
        vec![0x1b, 0x0c, 0x11, 0x58]
    );
    assert_eq!(piped_read(r"\033\033", "-n3"), vec![0x1b, 0x1b]);
}

/// Plain ASCII counting is unchanged (`-n 3` on `abcdef` is `61 62 63`).
#[test]
fn read_n_plain_ascii_count_unchanged() {
    assert_eq!(piped_read("abcdef", "-n3"), vec![0x61, 0x62, 0x63]);
    assert_eq!(piped_read("abcdef", "-N3"), vec![0x61, 0x62, 0x63]);
    assert_eq!(piped_read(r"ab\ncdef", "-n5"), vec![0x61, 0x62]);
}

/// A true multibyte character is still one unit: `é` (`c3 a9`) + `Z`, `-n 2`.
#[test]
fn read_n_multibyte_char_count_unchanged() {
    assert_eq!(piped_read(r"\303\251Z", "-n2"), vec![0xc3, 0xa9, 0x5a]);
    assert_eq!(piped_read(r"\303\251Z", "-n1"), vec![0xc3, 0xa9]);
}

/// The issue's outer-pipe shape: the shell's own fd 0 is the pipe.
#[test]
fn read_n_issue_repro_over_outer_pipe() {
    let (stdout, stderr, rc) = run_stdin(r#"{ read -n3 c; printf %s "$c"; }"#, b"AB\x1bC");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, vec![0x41, 0x42, 0x1b]);
}
