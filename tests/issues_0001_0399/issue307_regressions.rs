//! Issue rubash#307 regressions (wt13/fixpack lane).
//!
//! `set -u` + unbound variable inside a command substitution killed the
//! WHOLE script. GNU runs `$( ... )` bodies in a forked child
//! (subst.c:7143 command_substitute -> the child's parse_and_execute
//! performs the nounset check), so the error ends only the substitution:
//! the parent word keeps expanding with empty output, an assignment takes
//! the substitution's exit status, and the script continues.
//!
//! Root cause: the parent-side nounset pre-scan
//! (Executor::nounset_unbound_parameter) entered the `$(` span — its
//! `Some('(')` arm skipped a single `(' character, so `x=$(echo $U)' made
//! the PARENT report `$U' and abort the script. The arm now skips the
//! whole balanced, quote-aware `$( ... )' span; a new backtick arm skips
//! `` ` ... ` `` bodies for the same reason (command_substitute owns both
//! spellings). `$(( ... ))` arithmetic stays parent-scoped: GNU evaluates
//! it without forking, and an unbound name there is still a fatal,
//! script-ending error (expr.c expr_streval; probe p3).
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28; the 15-case matrix
//! lives under target/issue-suites/results/fixpack307/).

use std::process::Command;

/// Run a script FILE in its own scratch directory (relative name `case.sh`
/// keeps the diagnostic prefix stable) and return (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i307-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Core repro: `x=$(echo $U)' — the forked child reports the unbound
/// variable, the assignment takes status 1, and the script continues.
#[test]
fn unbound_in_command_substitution_assignment_continues() {
    let (stdout, stderr, code) =
        rubash_file("set -u\necho start\nx=$(echo $UNBOUND_VAR)\necho \"rc=$?\"\necho end\n");
    assert_eq!(stdout, "start\nrc=1\nend\n");
    assert_eq!(stderr, "case.sh: line 3: UNBOUND_VAR: unbound variable\n");
    assert_eq!(code, Some(0));
}

/// Argument position: the child's failure yields an empty substitution,
/// the echo itself succeeds (rc=0), script continues.
#[test]
fn unbound_in_command_substitution_argument_position() {
    let (stdout, stderr, code) =
        rubash_file("set -u\necho pre $(echo $UNBOUND) post\necho \"rc=$?\"\necho end\n");
    assert_eq!(stdout, "pre post\nrc=0\nend\n");
    assert_eq!(stderr, "case.sh: line 2: UNBOUND: unbound variable\n");
    assert_eq!(code, Some(0));
}

/// Inside a function body the containment is the same.
#[test]
fn unbound_in_command_substitution_inside_function() {
    let (stdout, stderr, code) =
        rubash_file("set -u\nf() { echo $(echo $UNBOUND_F); }\nf\necho \"rc=$?\"\necho end\n");
    assert_eq!(stdout, "\nrc=0\nend\n");
    assert_eq!(stderr, "case.sh: line 2: UNBOUND_F: unbound variable\n");
    assert_eq!(code, Some(0));
}

/// Backtick spelling: same forked-child boundary (subst.c:7143 owns both).
#[test]
fn unbound_in_backtick_substitution_is_contained() {
    let (stdout, stderr, code) =
        rubash_file("set -u\necho `echo $UNBOUND_BT`\necho \"rc=$?\"\necho end\n");
    assert_eq!(stdout, "\nrc=0\nend\n");
    assert_eq!(stderr, "case.sh: line 2: UNBOUND_BT: unbound variable\n");
    assert_eq!(code, Some(0));
}

/// Nested substitution: only the innermost fork dies; the outer `echo'
/// succeeds with an empty argument, so the assignment takes status 0.
#[test]
fn unbound_in_nested_command_substitution_only_kills_inner() {
    let (stdout, stderr, code) =
        rubash_file("set -u\nx=$(echo $(echo $U_NESTED))\necho \"rc=$?\"\necho end\n");
    assert_eq!(stdout, "rc=0\nend\n");
    assert_eq!(stderr, "case.sh: line 2: U_NESTED: unbound variable\n");
    assert_eq!(code, Some(0));
}

/// Quoted comsub body: still the child's nounset check (status 1 for the
/// assignment).
#[test]
fn unbound_in_double_quoted_command_substitution() {
    let (stdout, stderr, code) =
        rubash_file("set -u\nx=\"$(echo \"$U_DQ\")\"\necho \"rc=$?\"\necho end\n");
    assert_eq!(stdout, "rc=1\nend\n");
    assert_eq!(stderr, "case.sh: line 2: U_DQ: unbound variable\n");
    assert_eq!(code, Some(0));
}

/// `$(( ... ))` INSIDE a comsub: the arithmetic runs in the child, so the
/// failure ends only the substitution (assignment status 1, continue).
#[test]
fn arithmetic_unbound_inside_command_substitution_is_contained() {
    let (stdout, stderr, code) =
        rubash_file("set -u\nx=$(echo $(( $U_AR_IN_CS )))\necho \"rc=$?\"\necho end\n");
    assert_eq!(stdout, "rc=1\nend\n");
    assert_eq!(stderr, "case.sh: line 2: U_AR_IN_CS: unbound variable\n");
    assert_eq!(code, Some(0));
}

/// Parent-side unbound (`echo $U') remains fatal to the script — the
/// containment must not swallow direct parameter references.
#[test]
fn direct_unbound_parameter_still_exits_script() {
    let (stdout, stderr, code) = rubash_file("set -u\necho $UNBOUND\necho after\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "case.sh: line 2: UNBOUND: unbound variable\n");
    assert_eq!(code, Some(1));
}

/// Parent-side arithmetic (`$(( $U ))' without a comsub) also remains
/// fatal: GNU evaluates it without forking (expr.c expr_streval).
#[test]
fn direct_arithmetic_unbound_still_exits_script() {
    let (stdout, stderr, code) =
        rubash_file("set -u\nx=$(($UNBOUND_ARITH))\necho \"rc=$?\"\necho end\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "case.sh: line 2: UNBOUND_ARITH: unbound variable\n");
    assert_eq!(code, Some(1));
}
