//! Regression guard for rubash#2820: a bad `-<letter>` first word
//! followed by a directory operand is a numeric-option error
//! (`-z: invalid number` + usage, rc 2), never an arity error.
//!
//! GNU: pushd's option walk validates a ±-shaped word inline
//! (builtins/pushd.def:232-244: valid_number(word + 1) else
//! sh_invalidnum + usage + EX_USAGE) before the operand count is
//! ever judged; the `too many arguments` verdict belongs to
//! cd_builtin's argc check on a genuine multi-word operand list
//! (cd.def:326-330).
//!
//! Adjudication note: on the master this pin was written against,
//! the defect no longer reproduces — the faithful ISOPTION walk
//! landed with the stage-2 ④ Phase 2 triple migration (#2941,
//! commit 500769ef), after the issue was filed against 71ccb966
//! (whose walk broke out of the loop on a bad dash word and let the
//! arity check fire). This pin passes on that base by design: it
//! guards the #2820 shapes, which no earlier pin covered
//! (issue2937 pins a bad word AFTER a +N index, not a bad first
//! word with a trailing directory operand).

#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join("rubash")
}

struct Output {
    stdout: String,
    #[allow(dead_code)]
    stderr: String,
}

fn run_script(tag: &str, script: &str) -> Output {
    let dir = std::env::temp_dir().join(format!("rubash-2820-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).expect("scratch dir");
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(shell_bin())
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Output {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
    }
}

#[test]
fn bad_dash_word_with_dir_operand_is_invalid_number() {
    let out = run_script(
        "badword",
        "pushd -z a 2>&1; echo rc=$?\npushd -x a 2>&1; echo rc=$?\npushd -q a 2>&1; echo rc=$?\n",
    );
    for word in ["-z", "-x", "-q"] {
        assert!(
            out.stdout
                .contains(&format!("pushd: {word}: invalid number")),
            "stdout: {}",
            out.stdout
        );
    }
    assert!(
        out.stdout
            .contains("pushd: usage: pushd [-n] [+N | -N | dir]"),
        "stdout: {}",
        out.stdout
    );
    assert_eq!(out.stdout.matches("rc=2").count(), 3, "{}", out.stdout);
    assert!(!out.stdout.contains("too many arguments"), "{}", out.stdout);
}

#[test]
fn bad_dash_word_after_valid_option_still_invalid_number() {
    let out = run_script(
        "afteropt",
        "pushd -z -n a 2>&1; echo rc=$?\npushd -n -z a 2>&1; echo rc=$?\n",
    );
    assert_eq!(
        out.stdout.matches("pushd: -z: invalid number").count(),
        2,
        "{}",
        out.stdout
    );
    assert_eq!(out.stdout.matches("rc=2").count(), 2, "{}", out.stdout);
}

#[test]
fn two_directory_operands_stay_too_many_arguments() {
    // Positive control: the arity verdict still owns a genuine
    // two-directory operand list.
    let out = run_script("arity", "pushd a b 2>&1; echo rc=$?\n");
    assert!(
        out.stdout.contains("pushd: too many arguments"),
        "stdout: {}",
        out.stdout
    );
    assert!(out.stdout.contains("rc=1"), "{}", out.stdout);
}

#[test]
fn popd_bad_dash_word_is_invalid_number() {
    let out = run_script("popd", "popd -z a 2>&1; echo rc=$?\n");
    assert!(
        out.stdout.contains("popd: -z: invalid number"),
        "stdout: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("popd: usage: popd [-n] [+N | -N]"),
        "stdout: {}",
        out.stdout
    );
    assert!(out.stdout.contains("rc=2"), "{}", out.stdout);
}

#[test]
fn digit_letter_word_is_invalid_number() {
    let out = run_script("diglet", "pushd -1a 2>&1; echo rc=$?\n");
    assert!(
        out.stdout.contains("pushd: -1a: invalid number"),
        "stdout: {}",
        out.stdout
    );
    assert!(out.stdout.contains("rc=2"), "{}", out.stdout);
}
