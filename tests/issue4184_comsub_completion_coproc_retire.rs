//! Issue rubash#4184 — command-substitution completion is a
//! coproc disposal point.
//!
//! GNU's `command_substitute` (subst.c:7143) waits for the
//! substitution child in the parent (`wait_for (pid, ...)` at
//! subst.c:7449), and coproc_reap runs on wait_for's return path
//! (jobs.c:3494): a coproc death recorded before an `x=$(...)`
//! statement is retired when the substitution completes, so the
//! coproc variables are unset before the next command expands.
//! Rubash's comsub completion recorded the child's wait status
//! but carried no disposal mark, so the recorded death stayed
//! pending until some later foreground-wait face and the vars
//! survived the substitution.
//!
//! The fix is parent-side bookkeeping only (the emulated
//! foreground-wait mark at the completion point in
//! expand_command_substitution_with_context). The substitution
//! child's coproc endpoints are NOT severed: unlike the `( )`
//! fork, command substitution is not a coproc_closeall path in
//! GNU, and a comsub write to a live coproc succeeds on both
//! shells (rubash#4181 q3).
//!
//! GNU side verified against the cloud GNU Bash 5.3.20 oracle:
//! the q5 matrix recorded for rubash#4181, re-run live for this
//! issue (unnamed and named forms: vars unset after the
//! substitution, byte-identical stdout to the fixed build;
//! q3: comsub write works, identical on all three sides).
//! Pristine `161837d0` keeps the vars set in the same shapes.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash4184-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` as probe.sh in a fresh tempdir; return
/// (stdout, stderr, exit code, terminating signal).
fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>, Option<i32>) {
    use std::os::unix::process::ExitStatusExt;
    let dir = tempdir(tag);
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let code = output.status.code();
    let signal = output.status.signal();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, code, signal)
}

// Prelude for the death-first shapes: run the coproc to
// read-EOF, then let the death actually land in the reap
// records with a timed blocking read. Rubash's coproc read-EOF
// can precede the child's exit under load, and a death that is
// not yet recorded cannot retire at any disposal point (GNU's
// direct pipe has no such window). The blocking builtin read
// is the one wait that is NOT a disposal point (rubash#2053:
// it publishes the SIGCHLD window's records, disposal never
// follows a builtin), so the sync cannot retire the coproc by
// itself: on pristine the records publish, the vars survive,
// and the discriminating pins stay deterministically red.
const READ_TO_EOF: &str = "coproc true\nwhile IFS= read -r -u \"${COPROC[0]}\" _line; do :; done\nread -t 5 -r _sync < <(sleep 0.3)\necho \"after-read\"\n";

// The discriminating pins carry a second substitution command
// before the observation. Rubash's disposal cadence is the
// family's established one — the completion records the mark,
// the NEXT boundary refresh flushes the retirement (the same
// beat the rubash#4181 subshell pins observe). GNU retires
// synchronously inside the first substitution already; the
// observable end state after the substitutions is the pinned
// face.

#[test]
fn issue4184_comsub_completion_retires_dead_coproc_vars() {
    let script = format!(
        "{READ_TO_EOF}x=$(echo captured)\ny=$(echo again)\necho \"x=$x y=$y\"\necho \"vars: r=${{COPROC[0]:-unset}} w=${{COPROC[1]:-unset}} pid=${{COPROC_PID:-unset}}\"\necho \"alive\"\n"
    );
    let (stdout, stderr, code, _) = run_script("q5", &script);
    assert_eq!(
        stdout,
        "after-read\nx=captured y=again\nvars: r=unset w=unset pid=unset\nalive\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue4184_comsub_completion_retires_named_coproc_vars() {
    let script = "coproc myco { true; }\nwhile IFS= read -r -u \"${myco[0]}\" _line; do :; done\nread -t 5 -r _sync < <(sleep 0.3)\necho \"after-read\"\nx=$(echo captured)\ny=$(echo again)\necho \"x=$x y=$y\"\necho \"vars: r=${myco[0]:-unset} w=${myco[1]:-unset} pid=${myco_PID:-unset}\"\necho \"alive\"\n";
    let (stdout, stderr, code, _) = run_script("q5n", script);
    assert_eq!(
        stdout,
        "after-read\nx=captured y=again\nvars: r=unset w=unset pid=unset\nalive\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Invariance guard: command substitution is not a
/// coproc_closeall path — the comsub child keeps the endpoints
/// and a write to a live coproc still lands (rubash#4181 q3).
#[test]
fn issue4184_comsub_write_to_live_coproc_still_works() {
    let script = "coproc cat\nx=$(echo hi >&\"${COPROC[1]}\"; echo \"in_comsub_rc=$?\")\necho \"comsub_out=$x\"\nread -r -u \"${COPROC[0]}\" line\necho \"read_rc=$? got=$line\"\nexec {COPROC[1]}>&-\n";
    let (stdout, stderr, code, _) = run_script("q3", script);
    assert_eq!(stdout, "comsub_out=in_comsub_rc=0\nread_rc=0 got=hi\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Invariance guard: when the subshell disposal point
/// (rubash#4181 q2) already retired the coproc, a following
/// comsub completion must not retire twice — vars stay unset,
/// no new diagnostics, and a fresh coproc still round-trips.
#[test]
fn issue4184_comsub_after_subshell_disposal_does_not_double_retire() {
    let script = "coproc true\nwhile IFS= read -r -u \"${COPROC[0]}\" _line; do :; done\nread -t 5 -r _sync < <(sleep 0.3)\n( echo hi >&\"${COPROC[1]}\" )\necho \"subshell_rc=$?\"\necho \"vars-after-sub: r=${COPROC[0]:-unset} pid=${COPROC_PID:-unset}\"\nx=$(echo captured)\necho \"x=$x\"\necho \"vars-after-comsub: r=${COPROC[0]:-unset} pid=${COPROC_PID:-unset}\"\ncoproc cat\necho hello >&\"${COPROC[1]}\"\nread -r -u \"${COPROC[0]}\" line\necho \"roundtrip: $line\"\nexec {COPROC[1]}>&-\necho \"alive\"\n";
    let (stdout, stderr, code, _) = run_script("q2g", script);
    assert_eq!(
        stdout,
        "subshell_rc=1\nvars-after-sub: r=unset pid=unset\nx=captured\nvars-after-comsub: r=unset pid=unset\nroundtrip: hello\nalive\n"
    );
    assert_eq!(
        stderr,
        "probe.sh: line 4: \"${COPROC[1]}\": Bad file descriptor\n"
    );
    assert_eq!(code, Some(0));
}
