//! rubash#2690 — `exec >/dev/stdout` (and the `/dev/fd/N`,
//! `/dev/stderr` spellings) bound a DUP of the current endpoint
//! instead of REOPENING the underlying file: after `exec >inner.txt`
//! and two lines, `exec >/dev/stdout` must truncate inner.txt and
//! restart its offset, so only the post-reopen line survives.
//!
//! GNU treats an fd-alias PATH spelling as an open, not a dup:
//! redir.c:725 redir_open performs a plain open() of the magic link
//! (which resolves to the file the descriptor names, with the
//! redirect's own truncation), while the RF_DEVFD dup special case
//! (redir.c:603-608) compiles only without HAVE_DEV_FD; `exec >&1`
//! (an fd-spelled dup target) stays a dup. The fix diverts the exec
//! redirects-only walk's Output arm to FileFd::open_write over the
//! resolver's reopen path whenever the target is not a dup spelling.
//!
//! Every expectation below was cross-checked byte-for-byte against
//! GNU bash 5.3.20 (script-file probes, stdout captured through a
//! pipe — with a regular-file stdout the pre-fix build was already
//! byte-identical, so the pipe is the discriminator).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2690_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

fn read_file(run: &Run, name: &str) -> String {
    std::fs::read_to_string(run.dir.join(name)).unwrap_or_default()
}

#[test]
fn exec_dev_stdout_reopens_and_truncates() {
    let run = run_script(
        "( exec >inner.txt; echo first; echo second; exec >/dev/stdout; echo third )\necho \"file:[$(cat inner.txt)]\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        read_file(&run, "inner.txt"),
        "third\n",
        "the reopen must truncate: only the post-reopen line survives"
    );
    assert!(
        run.stdout.contains("file:[third]"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn exec_dev_fd_spelling_reopens_the_same_way() {
    let run = run_script(
        "( exec >inner.txt; echo first; echo second; exec >/dev/fd/1; echo third )\necho \"file:[$(cat inner.txt)]\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(read_file(&run, "inner.txt"), "third\n");
}

#[test]
fn exec_dup_spelling_stays_a_dup() {
    // `exec >&1` shares the endpoint (no truncation): both lines land.
    let run = run_script(
        "( exec >inner.txt; echo first; exec >&1; echo second )\necho \"file:[$(cat inner.txt)]\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(read_file(&run, "inner.txt"), "first\nsecond\n");
}

#[test]
fn exec_dev_stderr_reopens() {
    let run = run_script(
        "( exec 2>inner.txt; echo e1 >&2; exec 2>/dev/stderr; echo e2 >&2 ) 2>outer.txt\necho \"inner:[$(cat inner.txt)] outer:[$(cat outer.txt)]\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(read_file(&run, "inner.txt"), "e2\n");
    assert_eq!(read_file(&run, "outer.txt"), "");
    assert!(
        run.stdout.contains("inner:[e2] outer:[]"),
        "stdout: {}",
        run.stdout
    );
}
