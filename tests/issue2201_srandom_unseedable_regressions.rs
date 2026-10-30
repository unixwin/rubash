//! rubash#2201: SRANDOM has no seeded sequence in GNU — the census
//! repro compared two single samples of two unseeded streams.
//!
//! GNU's contract (doc/bash.1, SRANDOM): "Each time it is referenced,
//! this variable expands to a 32-bit pseudo-random number. The random
//! number generator is not linear on systems that support
//! /dev/urandom or arc4random(3)... The random number generator
//! cannot be seeded, so assignments to this variable have no effect."
//! In the C source: variables.c:1859 registers SRANDOM with
//! get_urandom and a NULL assign function (RANDOM at :1857 gets
//! assign_random), and get_urandom (variables.c:1441-1447) returns
//! get_urandom32() — lib/sh/random.c:219-240, whose primary path is
//! the OS entropy source (getrandom(2) / /dev/urandom), returning the
//! raw 32-bit value; the Park-Miller brand32 stream (random.c:140-146)
//! is only the no-entropy fallback. Oracle-verified 2026-10-30 with
//! GNU bash 5.3.20: three runs of the seed script below produced
//! three different S segments while the R segment stayed constant.
//!
//! rubash's pre-fix shape ran the brand32 fallback unconditionally:
//! values capped below 2^31 by the fallback mask and successive draws
//! linearly related. The pins below encode the observable contract:
//! full 32-bit range, and no reproducibility across processes even
//! with an assignment in both.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2201-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(tag: &str, script: &str) -> String {
    let dir = scratch(tag);
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let outcome = wait_bounded(&mut child, Duration::from_secs(30));
    assert!(
        !outcome.timed_out,
        "child {} did not exit within the bound; partial stderr: {}",
        outcome.pid,
        String::from_utf8_lossy(&outcome.stderr)
    );
    assert_eq!(
        outcome.code,
        Some(0),
        "script failed; stderr: {}",
        String::from_utf8_lossy(&outcome.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&outcome.stdout).into_owned()
}

/// The issue's regression seed, run in a fresh process: parse the
/// `base R=<r1> <r2> <r3> S=<s1> <s2>` line into its two segments.
fn seeded_run(tag: &str) -> (Vec<u64>, Vec<u64>) {
    let out = run_script(
        tag,
        "RANDOM=4242\nSRANDOM=777\necho \"base R=$RANDOM $RANDOM $RANDOM S=$SRANDOM $SRANDOM\"\n",
    );
    let line = out.trim();
    let (r_part, s_part) = line
        .split_once(" S=")
        .unwrap_or_else(|| panic!("unexpected output shape: {line}"));
    let r: Vec<u64> = r_part
        .trim_start_matches("base R=")
        .split_whitespace()
        .map(|t| t.parse().expect("numeric R draw"))
        .collect();
    let s: Vec<u64> = s_part
        .split_whitespace()
        .map(|t| t.parse().expect("numeric S draw"))
        .collect();
    assert_eq!(r.len(), 3, "R segment shape: {line}");
    assert_eq!(s.len(), 2, "S segment shape: {line}");
    (r, s)
}

/// SRANDOM draws span the full 32-bit range: in 64 draws at least
/// one value must reach >= 2^31 (the pre-fix fallback mask made that
/// impossible — probability 0 — while under uniform 32-bit draws
/// the failure probability is 2^-64). Every value must also stay
/// within the documented 0..=4294967295.
#[test]
fn issue2201_srandom_draws_span_full_32bit_range() {
    let out = run_script(
        "range",
        "i=0\nwhile (( i < 64 )); do echo $SRANDOM; (( i += 1 )); done\n",
    );
    let values: Vec<u64> = out
        .lines()
        .map(|l| l.trim().parse().expect("numeric SRANDOM draw"))
        .collect();
    assert_eq!(values.len(), 64, "draw count: {out}");
    assert!(
        values.iter().all(|&v| v <= 4_294_967_295),
        "SRANDOM draw out of documented range: {values:?}"
    );
    assert!(
        values.iter().any(|&v| v >= 2_147_483_648),
        "64 SRANDOM draws all below 2^31 — the stream is capped at 31 bits: {values:?}"
    );
}

/// Two processes running the seed script (with `SRANDOM=777` in
/// both) must produce the SAME seeded RANDOM segment — the control,
/// byte-identical to GNU 5.3.20's — and DIFFERENT SRANDOM segments:
/// the assignment is inert (variables.c:1859, NULL assign function),
/// so there is no seeded SRANDOM sequence to reproduce. A future
/// change that seeds SRANDOM from its assignment (the shape the
/// issue's title feared) turns the S segments identical and fails
/// here deterministically; under unseeded draws a collision needs
/// both 32-bit draws to repeat (probability 2^-64).
#[test]
fn issue2201_srandom_assignment_seeds_nothing_across_processes() {
    let (r1, s1) = seeded_run("proc-a");
    let (r2, s2) = seeded_run("proc-b");
    assert_eq!(
        r1,
        vec![25857, 21059, 5065],
        "seeded RANDOM control diverged from the GNU 5.3.20 values"
    );
    assert_eq!(r1, r2, "seeded RANDOM must reproduce across processes");
    assert!(
        s1.iter().chain(s2.iter()).all(|&v| v <= 4_294_967_295),
        "SRANDOM draw out of documented range: {s1:?} {s2:?}"
    );
    assert_ne!(
        s1, s2,
        "SRANDOM segments reproduced across processes despite SRANDOM=777 \
         in both — the assignment seeded the generator (GNU never seeds SRANDOM)"
    );
}
