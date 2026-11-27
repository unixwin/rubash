//! Issue rubash#382 regression: a pipeline producer that outlives its
//! consumer and takes the Windows broken-pipe hard-kill reported exit
//! status 1 (TerminateProcess), where WSL GNU Bash 5.3.0 reports 141
//! (128 + SIGPIPE).
//!
//! GNU spec: the producer of `yes | head -3` dies of SIGPIPE once head
//! exits; `wait_for` (jobs.c:3064) reaps WIFSIGNALED / WTERMSIG==SIGPIPE
//! and jobs.c:2958 process_exit_status surfaces 128 + 13 = 141 into
//! PIPESTATUS[0] (and `set -o pipefail` propagates it as the pipeline
//! status). The shell never rewrites the status.
//!
//! Rust semantic owner: pipeline_exec.rs
//! wait_for_windows_pipeline_member's kill arm (and the sequential-tail
//! leading-run kill arm) now return lingerer_sigpipe_status() —
//! ExitStatus::from_raw(141), the same 128+signal convention
//! builtins/kill.rs signal_process encodes into TerminateProcess.
//!
//! The lingerer here is a PowerShell loop that swallows write errors:
//! on GNU the equivalent continuous writer dies of SIGPIPE (141, verified
//! by the dual-shell matrices in the lane artifacts); on Windows it keeps
//! running after the consumer exits, so it must take the hard kill. The
//! self-exiting producers (WinuxCmd yes exits 1, Git MSYS yes exits
//! 13<<8=3328) report their own natural codes — those are environment
//! gaps tracked at unixwin/WinuxCmd#1142, not rubash status rewrites.
//!
//! Every run is wrapped in a deadline: the pre-fix failure mode for the
//! interaction shapes is a hang (niubash#158 family), and the lingerer
//! itself never exits without the kill.

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUN_LIMIT: Duration = Duration::from_secs(60);

/// PowerShell loop writing forever (bare LF endings), swallowing write
/// errors: the Windows stand-in for GNU's SIGPIPE-dying producer.
const LINGERER: &str =
    "powershell -NoProfile -Command 'for($i=0;;$i++){try{[Console]::Out.Write(\"L\"+($i%3)+\"`n\")}catch{}}'";

struct RunOutcome {
    timed_out: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue382-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue382 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");

    // Drain while running: the lingerer matrix writes bounded output, but a
    // regression to a hang must not deadlock the test on a full pipe.
    let stdout_pipe = child.stdout.take().expect("piped stdout");
    let stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stdout_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stderr_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break Some(status),
            None => {
                if started.elapsed() >= RUN_LIMIT {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let stdout = stdout_reader.join().expect("join stdout reader");
    let stderr = stderr_reader.join().expect("join stderr reader");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        timed_out,
        code: status.and_then(|status| status.code()),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    }
}

/// rubash#382 core: the hard-killed lingerer reports 141 in PIPESTATUS[0]
/// while the pipeline status itself stays the last member's (0).
#[test]
fn hardkilled_lingerer_reports_sigpipe_shape() {
    let out = run_rubash_script(&format!(
        "{LINGERER} | head -3\necho \"pipe0=${{PIPESTATUS[0]}} rc=$?\"\n"
    ));
    assert!(!out.timed_out, "pipeline hung");
    assert_eq!(
        out.stdout, "L0\nL1\nL2\npipe0=141 rc=0\n",
        "stdout mismatch (stderr: {})",
        out.stderr
    );
    assert_eq!(out.code, Some(0));
}

/// set -o pipefail: the 141 propagates as the pipeline status (GNU matrix
/// L-pf-rc/L-and-rc rows).
#[test]
fn pipefail_propagates_the_sigpipe_shape() {
    let out = run_rubash_script(&format!(
        concat!(
            "set -o pipefail\n",
            "{} | head -3\n",
            "echo \"pf-rc=$? pipe0=${{PIPESTATUS[0]}}\"\n",
            "{} | head -3 && echo AND-OK\n",
            "echo \"and-rc=$?\"\n",
        ),
        LINGERER, LINGERER
    ));
    assert!(!out.timed_out, "pipeline hung");
    assert_eq!(
        out.stdout, "L0\nL1\nL2\npf-rc=141 pipe0=141\nL0\nL1\nL2\nand-rc=141\n",
        "stdout mismatch (stderr: {})",
        out.stderr
    );
    assert_eq!(out.code, Some(0));
}

/// The lingerer as a NON-last member (two hard-kill arms exist: the
/// concurrent wait arm and the sequential-tail leading-run arm; both the
/// mid-member `| cat` and the compound-tail `| while read` routes must
/// report 141 — GNU matrix L-mid/L-tail rows).
#[test]
fn mid_member_and_compound_tail_report_sigpipe_shape() {
    let out = run_rubash_script(&format!(
        concat!(
            "set -o pipefail\n",
            "{} | head -3 | cat\n",
            "echo \"mid-pipe0=${{PIPESTATUS[0]}} rc=$?\"\n",
            "{} | head -3 | while read x; do echo \"got:$x\"; done\n",
            "echo \"tail-pipe0=${{PIPESTATUS[0]}} rc=$?\"\n",
        ),
        LINGERER, LINGERER
    ));
    assert!(!out.timed_out, "pipeline hung");
    assert_eq!(
        out.stdout,
        concat!(
            "L0\nL1\nL2\nmid-pipe0=141 rc=141\n",
            "got:L0\ngot:L1\ngot:L2\ntail-pipe0=141 rc=141\n",
        ),
        "stdout mismatch (stderr: {})",
        out.stderr
    );
    assert_eq!(out.code, Some(0));
}

/// Interaction regression canary (niubash#158 / 2c781657): the patient
/// wait still moves the full payload — no kill-window truncation.
#[test]
fn seq_five_million_wc_runs_full_value_repeatedly() {
    let out = run_rubash_script("seq 5000000 | wc -l\nseq 5000000 | wc -l\nseq 5000000 | wc -l\n");
    assert!(!out.timed_out, "pipeline hung");
    assert_eq!(out.stdout, "5000000\n5000000\n5000000\n");
    assert_eq!(out.code, Some(0));
}

/// Interaction regression canary (e4d84c8b): a comsub pipeline drains
/// concurrently and returns promptly.
#[test]
fn comsub_seq_20000_cat_returns_promptly() {
    let out = run_rubash_script("v=$(seq 20000 | cat | wc -l)\necho \"comsub-wc=$v\"\n");
    assert!(!out.timed_out, "comsub pipeline hung");
    assert_eq!(out.stdout, "comsub-wc=20000\n");
    assert_eq!(out.code, Some(0));
}

/// Canary: `yes | head` (the PATH producer) still terminates and yields
/// the expected bytes; the member status is the producer's own natural
/// exit code (environment-dependent — not asserted here).
#[test]
fn yes_head_canary_still_terminates() {
    let out = run_rubash_script("yes | head -3\necho \"rc=$?\"\n");
    assert!(!out.timed_out, "yes | head hung");
    assert_eq!(out.stdout, "y\ny\ny\nrc=0\n");
    assert_eq!(out.code, Some(0));
}
