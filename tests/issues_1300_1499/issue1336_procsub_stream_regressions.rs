//! Regression pins for rubash#1336 — the harvest-nightly HANG watchlist,
//! as narrowed by the maintainer errata (2026-10-23) to the one asset
//! with a REAL engine divergence: microsoft/shell-intune-samples
//! `installIllumioVEN.sh`, whose line 860 idiom
//!   exec > >(tee -a "$log") 2>&1
//! made rubash stop after the first log line: a persistent OUTPUT
//! process substitution bound by bare `exec` never delivered its
//! stream. The deferred body ran only from
//! `close_persistent_output_fd`; rebinding the fd overwrote the
//! endpoint and abandoned the carrier, subshell-region ends and shell
//! exit flushed nothing. GNU runs the substitution as a real child
//! holding the pipe (subst.c process_substitute; redir.c
//! do_redirections close semantics): EOF — and the reader's completion —
//! arrives when the LAST descriptor referring to the pipe closes.
//!
//! The fix cluster verified an 11-form matrix against GNU (simple
//! command, command group, bare exec, cat-body, rebind-to-file, dynamic
//! fd + dup, explicit close, exit flush, dup-then-close-original, input
//! direction, write-after-close). These pins cover the forms that
//! regressed, driving the body with `cat > file` (the matrix's v4 form)
//! so no external `tee` is needed, plus one `tee` leg on unix where
//! coreutils is guaranteed. Every run is deadline-guarded: a HANG
//! relapse fails the pin instead of freezing the suite.
//!
//! (The watchlist's other errata targets did not reproduce as engine
//! bugs — stdin-reading scripts are asset nature — and the residual
//! +7-line diagnostic drift in the same asset is tracked separately;
//! neither is pinned here.)

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-bfd-1336-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run with a hard deadline; a hang is a test failure, never a frozen
/// suite (the class of bug this file pins).
fn rubash_deadline(
    dir: &PathBuf,
    script: &str,
    deadline: Duration,
) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let t0 = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                if t0.elapsed() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash did not finish within {deadline:?} — HANG regression for script: {script}");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_end(&mut out);
    }
    if let Some(mut se) = child.stderr.take() {
        let _ = se.read_to_end(&mut err);
    }
    let status = child.wait().expect("wait");
    (
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
        status.code(),
    )
}

/// The substitution body is a real child process; its file write can
/// land a moment after the shell exits. Poll briefly for the content.
fn wait_file(dir: &PathBuf, name: &str, expected: &str) -> String {
    let path = dir.join(name);
    let t0 = Instant::now();
    loop {
        let got = std::fs::read_to_string(&path).unwrap_or_default();
        if got == expected || t0.elapsed() > Duration::from_secs(5) {
            return got;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn issue1336_bare_exec_output_procsub_delivers_stream() {
    let dir = scratch("bare");
    let (stdout, _e, code) = rubash_deadline(
        &dir,
        "exec > >(cat > log.txt); echo first; echo second",
        Duration::from_secs(30),
    );
    let log = wait_file(&dir, "log.txt", "first\nsecond\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "", "all output went into the substitution");
    assert_eq!(
        log, "first\nsecond\n",
        "bare-exec procsub stream must be delivered in full at exit flush"
    );
}

#[test]
fn issue1336_rebind_to_real_file_flushes_substitution() {
    let dir = scratch("rebind");
    let (_o, _e, code) = rubash_deadline(
        &dir,
        "exec > >(cat > log.txt); echo a; exec > real.txt; echo b",
        Duration::from_secs(30),
    );
    let log = wait_file(&dir, "log.txt", "a\n");
    let real = wait_file(&dir, "real.txt", "b\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(
        log, "a\n",
        "rebinding must flush the replaced substitution, not abandon it"
    );
    assert_eq!(real, "b\n", "post-rebind output lands in the real file");
}

#[test]
fn issue1336_chained_procsub_rebind_flushes_both() {
    let dir = scratch("chain");
    let (_o, _e, code) = rubash_deadline(
        &dir,
        "exec > >(cat > l1.txt); echo a; exec > >(cat > l2.txt); echo b",
        Duration::from_secs(30),
    );
    let l1 = wait_file(&dir, "l1.txt", "a\n");
    let l2 = wait_file(&dir, "l2.txt", "b\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(l1, "a\n", "first substitution in a chain keeps its segment");
    assert_eq!(
        l2, "b\n",
        "second substitution in a chain keeps its segment"
    );
}

#[test]
fn issue1336_subshell_region_flushes_its_substitution() {
    let dir = scratch("subshell");
    let (stdout, _e, code) = rubash_deadline(
        &dir,
        "( exec > >(cat > log.txt); echo inner ); echo outer",
        Duration::from_secs(30),
    );
    let log = wait_file(&dir, "log.txt", "inner\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout, "outer\n",
        "parent output is unaffected by the subshell's redirect"
    );
    assert_eq!(
        log, "inner\n",
        "subshell region end must flush its substitution"
    );
}

#[test]
fn issue1336_exec_tee_idiom_from_the_asset() {
    if cfg!(windows) {
        return; // tee is external coreutils here; the cat-body legs pin the same mechanism.
    }
    let dir = scratch("tee");
    let (_o, _e, code) = rubash_deadline(
        &dir,
        "exec > >(tee -a log.txt) 2>&1; echo first; echo second",
        Duration::from_secs(30),
    );
    let log = wait_file(&dir, "log.txt", "first\nsecond\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(
        log, "first\nsecond\n",
        "the installIllumioVEN.sh idiom delivers its full log"
    );
}
