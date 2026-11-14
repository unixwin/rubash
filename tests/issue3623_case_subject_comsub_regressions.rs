//! rubash#3623 — a single-command `$(...)` inside a `case` subject (or
//! pattern) must not truncate the OUTER command substitution.
//!
//! `dollar_command_substitution` scans `(`/`)` without touching `depth`
//! while a `case` is open, so `case $(ls) in` scanned the inner comsub
//! inline: `ls` completed as the case subject and the inner `)` hit the
//! case-close arm, cutting the outer substitution at `$(case $(ls`.
//!
//! GNU parse.y:4451 parse_comsub parses every `$(` as a fresh parse —
//! the inner comsub is opaque to the outer case machine. The scanner now
//! skips a nested `$(...)` as a unit when `case_depth > 0` (same shape
//! as the double-quote nested-$( skip), leaving a non-empty word so the
//! case subject still completes at the next boundary.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3623-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(script: &str) -> (String, String) {
    let dir = tempdir("case");
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
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr)
}

#[test]
fn single_command_comsub_in_case_subject_does_not_truncate() {
    // The issue's repro shape: single-command $(ls) / $(echo) / $(id).
    let (out, err) = run_script(
        "x=$(\n    case $(echo star) in\n\t*) echo foobix;;\n    esac\n)\necho \"x=$x\"\n",
    );
    assert_eq!(out, "x=foobix\n", "out={out} err={err}");
    assert_eq!(err, "");
}

#[test]
fn comsub_subject_with_empty_esac_stays_closed() {
    let (out, err) = run_script("y=$(case $(ls) in\n    esac\n)\necho \"empty=$y\"\n");
    assert_eq!(out, "empty=\n", "out={out} err={err}");
    assert_eq!(err, "");
}

#[test]
fn comsub_subject_with_pattern_list_still_matches() {
    let (out, err) = run_script("z=$(case $(echo a) in a|b) echo hit;; esac)\necho \"z=$z\"\n");
    assert_eq!(out, "z=hit\n", "out={out} err={err}");
    assert_eq!(err, "");
}

#[test]
fn case_without_inner_comsub_still_scans() {
    let (out, err) =
        run_script("w=$(case foo in\n    foo) echo two-ok;;\n    esac)\necho \"w=$w\"\n");
    assert_eq!(out, "w=two-ok\n", "out={out} err={err}");
    assert_eq!(err, "");
}
