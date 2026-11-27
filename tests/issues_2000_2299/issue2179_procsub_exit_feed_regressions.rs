//! Issue unixwin/rubash#2179: an output process substitution's payload
//! was intermittently LOST when the shell exited right after the
//! command that produced it — the consumer read EOF and printed
//! nothing (empty stdout, status 0).
//!
//! GNU owner: subst.c:6362 process_substitute() forks the consumer at
//! materialization and the producer's own writes fill the substitution
//! pipe during the command, so at any later shell exit the bytes are
//! already in the kernel pipe and the consumer finishes on its own
//! (verified: GNU bash 5.3.20 loses 0/60 on both shapes below under a
//! read-to-EOF capture). Rubash's carrier defers the transfer: a
//! writer thread feeds the temp carrier's bytes to the respawned
//! consumer's stdin at command finish. That thread raced the process's
//! unconditional exit — in failing runs it had never been scheduled
//! before std::process::exit destroyed it, so the payload never
//! entered the pipe. The fix keeps the feed handles on the Executor
//! and completes them (drain_procsub_feeds) at every end of the
//! executor's process lifetime: Drop, the `exec` image replacement,
//! and the nofork tail replacement. The consumer itself is never
//! waited for (see issue1220's pins for the async contract).
//!
//! The repetition counts are the determinism argument: pre-fix each
//! repetition passed only ~55-85% of the time under load, so 30
//! consecutive passes are unreachable by timing luck; post-fix the
//! transfer completes by construction before the shell can exit.

use std::process::Command;

fn rubash_c(script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c probe");
    (output.stdout, output.stderr, output.status.code())
}

fn assert_repeated(script: &str, expected_stdout: &[u8], reps: usize) {
    for rep in 0..reps {
        let (stdout, stderr, code) = rubash_c(script);
        assert_eq!(code, Some(0), "rep {rep}: script {script:?}");
        assert_eq!(
            stdout,
            expected_stdout,
            "rep {rep}: script {script:?} stderr {:?}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(stderr, b"", "rep {rep}: script {script:?}");
    }
}

#[test]
fn issue2179_output_procsub_payload_survives_shell_exit() {
    // The #2179 victim pin's shape (also pinned once in
    // cli_tests::compat_issue_regressions): raw byte through >(od).
    assert_repeated(r"printf '\377' > >(od -An -tx1)", b" ff\n", 30);
}

#[test]
fn issue2179_function_procsub_streams_survive_shell_exit() {
    // The second victim: a function's input and output substitutions
    // in one call.
    assert_repeated(
        r#"f() { read value; printf 'value=%s\n' "$value"; }; f < <(printf 'hello\n') > >(cat)"#,
        b"value=hello\n",
        30,
    );
}

#[test]
#[cfg(unix)]
fn issue2179_output_procsub_feed_completes_before_exec_replacement() {
    // `exec` replaces the process image; the feed must complete before
    // the replacement, or the payload dies with the old image exactly
    // as it did at plain exit.
    assert_repeated("printf 'kept\\n' > >(cat); exec /bin/true", b"kept\n", 10);
}

#[test]
#[cfg(unix)]
fn issue2179_output_procsub_feed_completes_before_tail_replacement() {
    // The nofork tail optimization replaces the shell with the last
    // plain external command of a -c string; same lifetime boundary.
    assert_repeated("printf 'kept\\n' > >(cat); /bin/true", b"kept\n", 10);
}

#[test]
#[cfg(unix)]
fn issue2179_output_procsub_large_payload_delivered_in_full() {
    // 588895 bytes exceed the 64KB pipe buffer many times over: the
    // pre-fix feed was abandoned mid-transfer (a truncated multiple of
    // the buffer is what the consumer counted), the completed feed
    // delivers every byte.
    for rep in 0..3 {
        let (stdout, _stderr, code) = rubash_c("seq 100000 > >(wc -c)");
        assert_eq!(code, Some(0), "rep {rep}");
        assert_eq!(stdout, b"588895\n", "rep {rep}");
    }
}
