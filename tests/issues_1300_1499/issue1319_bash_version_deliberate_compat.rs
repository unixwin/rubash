//! rubash#1319 (W5 residual / characterization pin): `$BASH_VERSION`
//! reports the exact GNU string `5.3.20(1)-release`, byte-identical to the
//! GNU bash 5.3.20 oracle, so version-gated scripts cannot distinguish
//! rubash from genuine GNU bash.
//!
//! Ruling (do not "fix"): this is DELIBERATE compatibility, ruled
//! 2026-10-24 and recorded in the issue's closing comment and in
//! ~/workspace/rubash-fix3/DECISIONS.md ("rubash #1319 $BASH_VERSION →
//! 不改：support_names.rs 注释在案的刻意兼容决定"). The engine seeds the
//! value from `bash_version_value()` in src/executor/support_names.rs,
//! whose comment states the intent: the spec oracle is GNU bash 5.3.20,
//! so version probes must observe the same string GNU would report. If a
//! distinguishable identity is ever wanted, that is a new feature
//! discussion, not a bug fix — this pin exists so the deliberate identity
//! cannot drift silently.

use std::process::Command;

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1319_bash_version_is_exact_gnu_compat_string() {
    let (stdout, stderr, status) = run(r#"printf '<%s>\n' "$BASH_VERSION""#);
    assert_eq!(stdout, "<5.3.20(1)-release>\n", "stderr: {stderr:?}");
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}

#[test]
fn issue1319_bash_versinfo_matches_gnu_5320_shape() {
    // GNU bash 5.3.20: BASH_VERSINFO = (5 3 20 1 release <machtype>).
    // Elements 0..=4 are the compat contract; element 5 is the host
    // MACHTYPE and is pinned by the issue1030 identity pins instead.
    let (stdout, stderr, status) = run(r#"printf '<%s>\n' "${BASH_VERSINFO[*]:0:5}""#);
    assert_eq!(stdout, "<5 3 20 1 release>\n", "stderr: {stderr:?}");
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}
