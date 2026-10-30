//! Regression pins for rubash#2062 — assignment-list superlinearity in
//! release builds (2000 assignments → 253 ms, 8000 → 1411 ms on the
//! reporter's host). Per the perf-pin conventions (see
//! issue1119_1264_perf_scale_regressions.rs), these pins assert FUNCTION
//! plus a scaling SHAPE: no absolute-millisecond expectations, but a
//! growth bound far above the fixed implementation's ~linear cost and
//! far below the quadratic signature (~16x per 4x size).
//!
//! Three distinct Θ(N²) terms were attributed and fixed:
//!   1. The per-command trap reconcile fast-path check scanned every
//!      variable key for a `__RUBASH_TRAP_SIG` prefix — Θ(#vars) at
//!      every command boundary, so Θ(N²) over a script creating N
//!      variables. It now probes the fixed managed-key set instead.
//!   2. The alias-introduced `select` probe re-scanned the remaining
//!      command list for the first word-bearing command at every
//!      index. The scan is a pure function of the list and is now
//!      memoized per list walk.
//!   3. The alias-introduced `for` probe scanned for a following `do`
//!      command before checking that a `for`/`echo` marker was even
//!      present. The scan now runs only after a marker is confirmed.
//!
//! GNU expectation for the shapes: GNU bash 5.3.20 runs the 8000-line
//! assignment script in ~68 ms and the growth is linear; the pins
//! assert the assignments land and the select/trap behavior matches.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn rubash_file(script: &str, stdin: Option<&str>) -> (String, String, Option<i32>, Duration) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "rubash-issue2062-{}-{}.sh",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, script).expect("write probe script");
    let t0 = Instant::now();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&path);
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    if let Some(input) = stdin {
        child
            .stdin
            .as_mut()
            .expect("stdin piped")
            .write_all(input.as_bytes())
            .expect("feed stdin");
    }
    let out = child.wait_with_output().expect("wait rubash");
    let elapsed = t0.elapsed();
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
        elapsed,
    )
}

fn assignment_script(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("V{i}=value{i}\n"));
    }
    s.push_str("echo \"last=$V");
    s.push_str(&(n - 1).to_string());
    s.push_str("\"\n");
    s
}

#[test]
fn issue2062_assignment_list_scales_near_linearly() {
    let (stdout, _e, code, t_small) = rubash_file(&assignment_script(4_000), None);
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "last=value3999");
    let (stdout, _e, code, t_large) = rubash_file(&assignment_script(16_000), None);
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "last=value15999");
    assert!(
        t_large < t_small * 10 + Duration::from_secs(2),
        "scaling shape: 4x the assignments must cost well under 16x \
         (quadratic signature); got {t_small:?} -> {t_large:?}"
    );
    assert!(
        t_large < Duration::from_secs(120),
        "magnitude sentinel: 16000 assignments took {t_large:?}"
    );
}

#[test]
fn issue2062_many_vars_then_many_commands_scales_near_linearly() {
    // The trap-reconcile term bit this shape: N assignments followed by
    // N cheap commands pays the boundary cost at every command.
    let shape = |n: usize| {
        let mut s = String::new();
        for i in 0..n {
            s.push_str(&format!("W{i}=w{i}\n"));
        }
        for _ in 0..n {
            s.push_str(":\n");
        }
        s.push_str("echo \"w=$W");
        s.push_str(&(n - 1).to_string());
        s.push_str("\"\n");
        s
    };
    let (stdout, _e, code, t_small) = rubash_file(&shape(2_000), None);
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "w=w1999");
    let (stdout, _e, code, t_large) = rubash_file(&shape(8_000), None);
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "w=w7999");
    assert!(
        t_large < t_small * 10 + Duration::from_secs(2),
        "scaling shape: 4x vars+commands must cost well under 16x; \
         got {t_small:?} -> {t_large:?}"
    );
}

#[test]
fn issue2062_select_after_many_assignments_still_fires() {
    // The memoized select probe must still stitch a select that
    // follows a long wordless run, and the assignments must all land.
    let mut s = String::new();
    for i in 0..2_000 {
        s.push_str(&format!("S{i}=s{i}\n"));
    }
    s.push_str("select x in one two; do echo \"sel=$x last=$S1999\"; break; done\n");
    let (stdout, _e, code, _t) = rubash_file(&s, Some("2\n"));
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("sel=two last=s1999"),
        "select after assignments: got {stdout:?}"
    );
}

#[test]
fn issue2062_alias_introduced_select_still_fires() {
    // The probe's real shape (executor_command_chaining part_077):
    // the alias definition must be EXECUTED before the use is read
    // (newline separation), and expand_aliases must be on (script mode
    // enables it; set it explicitly anyway).
    let s = "shopt -s expand_aliases\nalias s=select\n\
             s item in alpha beta; do echo chosen:$item; break; done <<< '2'\n";
    let (stdout, _e, code, _t) = rubash_file(s, None);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("chosen:beta"), "got {stdout:?}");
}

#[cfg(unix)]
#[test]
fn issue2062_trap_delivery_with_many_vars() {
    // Fix 1 must not change trap semantics: an armed trap still fires
    // at the command boundary after thousands of variable writes, and
    // a forged __RUBASH_TRAP_SIG* variable creates no trap.
    let mut s = String::new();
    for i in 0..4_000 {
        s.push_str(&format!("T{i}=t{i}\n"));
    }
    s.push_str("trap 'echo fired' USR1\nkill -USR1 $$\necho \"done $T3999\"\n");
    let (stdout, _e, code, _t) = rubash_file(&s, None);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("fired"), "got {stdout:?}");
    assert!(stdout.contains("done t3999"), "got {stdout:?}");

    // The forged key must not arm a trap: the shell is terminated by
    // the TERM it sends itself. Whether the process then exits 143
    // or dies by the signal depends on whether the mailbox handler
    // was installed by an earlier slow-path reconcile — both are the
    // GNU outcome for an untrapped TERM (nothing runs after it), so
    // assert exactly that, not the representation.
    let (stdout, _e, code, _t) = rubash_file(
        "__RUBASH_TRAP_SIGFOO=bar\nkill -TERM $$\necho unreachable\n",
        None,
    );
    assert!(
        !stdout.contains("unreachable"),
        "forged trap key must not arm a trap; got {stdout:?}"
    );
    assert_ne!(
        code,
        Some(0),
        "shell must be terminated by the untrapped TERM"
    );
}
