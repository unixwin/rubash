//! rubash#2905 regressions: the 16 pinned mvdan/sh ecosuite fixtures
//! (`tests/fixtures/ecosuite/mvdan-n/`) must keep their GNU parse/exec
//! verdicts under the built rubash binary.
//!
//! Background: these fixtures were extracted from the mvdan/sh corpus
//! (559 snippets) because they once diverged on `-n` exit code (the
//! #220/#221/#222 family). The eco-2906 execution differential re-verified
//! them end-to-end (parse + exec, stdout bytes + exit code; stderr is
//! advisory only — it carries timing/PID/order artifacts, and one known
//! wording divergence tracked separately in rubash#3031). Until this pin,
//! no test or CI job referenced the fixtures at all (full-repo search hit
//! only docs and the fixtures' own MANIFEST), so the verification existed
//! only as manual double-runs; this pin makes it an in-repo gate.
//!
//! The neighbouring chain — the bash-completion pytest behaviour suite
//! driven through a pexpect bridge — is an on-demand eco chain
//! (`scripts/run-bash-completion-chain.py`), not part of this pin; whether
//! either chain is wired into CI is the open decision in rubash#3033.
//!
//! Expectations are the GNU Bash 5.3.20 oracle's own results (self-compiled
//! oracle, isolated env, stdin /dev/null), re-measured 2026-11-08 on
//! master 88047f50: all 16 fixtures produce empty stdout on both shells.

use std::process::{Command, Stdio};

/// (fixture, `bash -n` exit code, exec exit code) per the GNU oracle.
/// Exec stdout is empty for all 16 on both shells.
const CASES: &[(&str, i32, i32)] = &[
    ("t0026.sh", 0, 1),
    ("t0068.sh", 2, 2),
    ("t0142.sh", 2, 2),
    ("t0143.sh", 2, 2),
    ("t0144.sh", 2, 2),
    ("t0164.sh", 0, 127),
    ("t0169.sh", 2, 2),
    ("t0173.sh", 2, 2),
    ("t0286.sh", 0, 0),
    ("t0382.sh", 2, 2),
    ("t0383.sh", 0, 0),
    ("t0411.sh", 0, 1),
    ("t0415.sh", 2, 2),
    ("t0459.sh", 1, 1),
    ("t0525.sh", 2, 2),
    ("t0526.sh", 2, 2),
];

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("ecosuite")
        .join("mvdan-n")
}

fn run_rubash(args: &[&str], fixture: &std::path::Path) -> (String, Option<i32>) {
    let scratch = std::env::temp_dir().join(format!(
        "rubash-issue2905-{}-{}",
        std::process::id(),
        fixture
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&scratch).expect("scratch dir");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .arg(fixture)
        .current_dir(&scratch)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &scratch)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash on fixture");
    let _ = std::fs::remove_dir_all(&scratch);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

#[test]
fn mvdan_pinned_fixtures_keep_gnu_verdicts() {
    let dir = fixture_dir();
    let mut checked = 0;
    for (name, parse_rc, exec_rc) in CASES {
        let fixture = dir.join(name);
        assert!(
            fixture.is_file(),
            "pinned fixture missing: {}",
            fixture.display()
        );

        let (_, got_parse_rc) = run_rubash(&["-n"], &fixture);
        assert_eq!(
            got_parse_rc,
            Some(*parse_rc),
            "{name}: parse (-n) exit code diverged from GNU oracle"
        );

        let (stdout, got_exec_rc) = run_rubash(&[], &fixture);
        assert_eq!(
            got_exec_rc,
            Some(*exec_rc),
            "{name}: exec exit code diverged from GNU oracle"
        );
        assert_eq!(
            stdout, "",
            "{name}: exec stdout diverged from GNU oracle (expected empty)"
        );
        checked += 1;
    }
    assert_eq!(checked, 16, "all 16 pinned fixtures must be exercised");
}
