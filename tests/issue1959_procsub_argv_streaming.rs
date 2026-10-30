//! rubash#1959: a process substitution delivered as an argv WORD to an
//! external child streamed nowhere on Windows — the parent drained the
//! producer to EOF into a temp file before spawn
//! (dev_fd_operands::dev_fd_table_operand, the `"<("` File-endpoint arm),
//! so the consumer started only after the producer exited. GNU
//! process_substitute (subst.c:6362) hands the child a live pipe under
//! /dev/fd/N: consumers observe the producer's first bytes while it still
//! runs, a non-terminating producer never starves the consumer, and the
//! shell moves on without waiting an abandoned producer.
//!
//! The fix delivers the live stream through the #1903 FIFO-marker
//! rendezvous: the argv word carries an on-disk `!<fifo>` marker path the
//! external child's own runtime bridges to a named pipe; a pump thread
//! copies the procsub read end into it (writer client, POSIX blocking-open
//! semantics). The in-process faces (redirect, read, exec-bound) were
//! already streamed by rubash#1952 and stay pinned in regression.rs
//! (mod issue_1952); this file pins the external WORD face.
//!
//! Oracle: GNU bash 5.3.20 (WSL /usr/bin, 2026-10-29) byte-for-byte on
//! every shape below — first-line/first-bytes arrival before the
//! producer's flag file exists, full delivery + EOF, producer-crash
//! partial delivery, and the pipeline shape where GNU itself waits for
//! the producer (`cat <(…) | head -1` answers `late` there).
//!
//! The external consumers must speak the marker rendezvous (on Windows:
//! WinuxCmd; on Unix: any real head/cat over a real fifo). Each pin runs
//! only when a round-trip gate proves the resolved tools are
//! marker/pipe-aware (WinuxCmd#1442 tracks the one head gap this file
//! sidesteps: line-mode operands there are pinned with `head -c`, not
//! `head -n`). Everywhere else the pins self-skip rather than fail on a
//! toolset that cannot see the rendezvous.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-1959-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

/// The directory list a candidate tool PATH is built from: the ambient
/// PATH probed entry by entry (the marker-aware toolchain is an install,
/// not a fixture), plus the Unix baseline on other platforms.
fn candidate_path_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect()
    } else {
        vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
    }
}

fn path_string(dirs: &[PathBuf]) -> String {
    std::env::join_paths(dirs)
        .expect("join candidate path")
        .to_string_lossy()
        .into_owned()
}

/// Spawn rubash on `script` inside `dir` with `path` as its whole PATH.
/// Bounded: a hung run is killed and fails the pin instead of stalling
/// the suite (the #1820 watchdog precedent).
fn run_rubash(
    dir: &Path,
    path: &str,
    script: &str,
    limit: Duration,
) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", path)
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cfg!(windows) {
        for name in [
            "SystemRoot",
            "SystemDrive",
            "COMSPEC",
            "PATHEXT",
            "OS",
            "TEMP",
            "TMP",
        ] {
            if let Ok(value) = std::env::var(name) {
                cmd.env(name, value);
            }
        }
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => {
                let mut out = child.stdout.take().unwrap();
                let mut err = child.stderr.take().unwrap();
                use std::io::Read;
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let _ = out.read_to_end(&mut stdout);
                let _ = err.read_to_end(&mut stderr);
                return (
                    String::from_utf8_lossy(&stdout).into_owned(),
                    String::from_utf8_lossy(&stderr).into_owned(),
                    status.code(),
                );
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "rubash did not finish within {limit:?}; the rendezvous pinned the shell or a consumer"
                );
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Prove the resolved tools see the rendezvous (or, on Unix, real fifos):
/// `mkfifo f; printf 'zz\n' | tee f & head -1 f` must answer `zz`. A
/// toolset without the marker protocol reads the 7 marker bytes and
/// fails this gate, and every pin below skips.
fn rendezvous_capable_path() -> Option<String> {
    let mut base = Vec::new();
    if cfg!(windows) {
        base.push(PathBuf::from(r"C:\Windows\System32"));
    }
    for dir in candidate_path_dirs() {
        let head = if cfg!(windows) {
            dir.join("head.exe")
        } else {
            dir.join("head")
        };
        if !head.exists() {
            continue;
        }
        let mut dirs = base.clone();
        dirs.push(dir.clone());
        let path = path_string(&dirs);
        let shell =
            format!("mkfifo f 2>/dev/null; printf 'zz\\n' | tee f >/dev/null & head -1 f\n");
        let dir = scratch("gate");
        let (stdout, _stderr, code) = run_rubash(&dir.0, &path, &shell, Duration::from_secs(20));
        drop(dir);
        if code == Some(0) && stdout.contains("zz") {
            return Some(path);
        }
    }
    None
}

/// The pins share one gate result.
fn gated_path() -> Option<String> {
    use std::sync::OnceLock;
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(rendezvous_capable_path).clone()
}

/// rubash#1959 headline shape (the #1952 ignored pin's face, in its
/// Windows-green byte-mode form): the pipeline stage's external consumer
/// reads its first bytes while the producer still runs — GNU answers
/// `first` + `early` (flag not yet created), the drain answered `late`.
#[test]
fn issue1959_pipeline_stage_word_streams_first_bytes() {
    let Some(path) = gated_path() else {
        return;
    };
    let dir = scratch("stage-word");
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        &path,
        "rm -f pd\n\
         head -c 7 <(printf 'first\\nse'; sleep 2; touch pd; printf 'x' 2>/dev/null)\n\
         if [ -e pd ]; then echo late; else echo early; fi\n",
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    // GNU 5.3.20: "first\ns" (exactly 7 bytes, -c honored through the
    // rendezvous) then `early` — the flag check ran before the
    // producer's touch.
    assert_eq!(stdout, "first\nsearly\n", "stderr:\n{stderr}");
}

/// The shell must not wait an abandoned producer at the command
/// boundary: GNU returns at once after the consumer exits (the producer
/// is left running/blocked, its writes later dying on the broken pipe).
/// Pre-fix the face drained the producer to EOF before spawn (the whole
/// sleep inside the command); the post-fix regression spent the whole
/// producer lifetime in a finish-time seek serializing against the
/// pump's parked read — this pin catches both.
#[test]
fn issue1959_abandoned_producer_not_waited_at_boundary() {
    let Some(path) = gated_path() else {
        return;
    };
    let dir = scratch("boundary");
    let started = Instant::now();
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        &path,
        "head -c 7 <(printf 'first\\nse'; sleep 10)\n\
         echo done\n",
        Duration::from_secs(30),
    );
    let elapsed = started.elapsed();
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "first\nsdone\n", "stderr:\n{stderr}");
    assert!(
        elapsed < Duration::from_secs(5),
        "the shell blocked {:?} on the abandoned producer (GNU returns at once)",
        elapsed
    );
}

/// The issue's comsub form: an external cat receives the word inside
/// $() — full delivery through the rendezvous with GNU's EOF semantics
/// (the flag proves the bytes arrived while the producer still ran; the
/// trailing `late` is GNU's own answer: cat consumes to EOF, so the
/// assignment completes only after the producer exits).
#[test]
fn issue1959_comsub_external_cat_full_delivery_and_eof() {
    let Some(path) = gated_path() else {
        return;
    };
    let dir = scratch("comsub");
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        &path,
        "rm -f pd\n\
         x=$(cat <(printf 'first\\n'; sleep 2; touch pd; printf 'second\\n'))\n\
         if [ -e pd ]; then echo late; else echo early; fi\n\
         echo \"comsub:[$x]\"\n",
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(
        stdout, "late\ncomsub:[first\nsecond]\n",
        "stderr:\n{stderr}"
    );
}

/// The producer observes the external consumer's output mid-flight: cat
/// must have written line 1 into out.txt before the producer's post-sleep
/// check — the strongest streaming proof (the in-process twin is
/// issue1952_cat_argument_streams_first_line).
#[test]
fn issue1959_producer_sees_consumer_output_midflight() {
    let Some(path) = gated_path() else {
        return;
    };
    let dir = scratch("midflight");
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        &path,
        "rm -f pd out.txt\n\
         cat <(printf 'first\\n'; sleep 2; if [ -s out.txt ]; then touch pd; fi; \
           printf 'second\\n') > out.txt\n\
         if [ -e pd ]; then echo streamed; else echo starved; fi\n\
         cat out.txt\n",
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "streamed\nfirst\nsecond\n", "stderr:\n{stderr}");
}

/// Producer crash mid-stream: the consumer keeps the bytes written so
/// far, sees EOF (not a hang), and the command's status is the
/// consumer's — GNU's shape for a producer dying at exit 3.
#[test]
fn issue1959_producer_crash_partial_delivery_then_eof() {
    let Some(path) = gated_path() else {
        return;
    };
    let dir = scratch("crash");
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        &path,
        "cat <(printf 'x\\n'; exit 3) > crash-out.txt\n\
         echo \"status:$?\"\n\
         cat crash-out.txt\n",
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "status:0\nx\n", "stderr:\n{stderr}");
}

/// The pipeline word form with an early-exiting second stage. GNU itself
/// answers `late` here (bash waits the producer out once the pipeline
/// completes — oracle-verified 2026-10-29), so this pin guards delivery
/// plus GNU-faithful producer-wait semantics, not streaming.
#[test]
fn issue1959_pipeline_cat_word_head_stage_gnu_wait_shape() {
    let Some(path) = gated_path() else {
        return;
    };
    let dir = scratch("cat-stage");
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        &path,
        "rm -f pd\n\
         cat <(printf 'first\\n'; sleep 2; touch pd; printf 'second\\n' 2>/dev/null) | head -1\n\
         if [ -e pd ]; then echo late; else echo early; fi\n",
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "first\nlate\n", "stderr:\n{stderr}");
}
