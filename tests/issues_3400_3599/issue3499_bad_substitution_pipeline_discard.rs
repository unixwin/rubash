//! rubash#3499 — a bad substitution in a pipeline element reported
//! one diagnostic per bad word and ran the command anyway, and the
//! verdict latch leaked into the next command (which then died
//! silently, without its own diagnostic).
//!
//! GNU subst.c:10277-10288 (parameter_brace_expand): the FIRST word
//! -expansion failure of a word list reports once
//! (`report_error ("%s: bad substitution")`) and abandons the whole
//! list via expand_word_error -> exp_jump_to_top_level(DISCARD):
//! the element produces no output and dies with status 1, the
//! pipeline's status stays the last element's, and the script
//! continues. The one-diagnostic rule is per element: two bad
//! elements report one each.
//!
//! Expectations are the GNU bash 5.2.21 oracle's, probed from script
//! files (script-mode expansion errors are DISCARD, status 1, and
//! the script continues).
//!
//! Known limitation, NOT covered here: a discarded element's own
//! output redirect (`echo a=${-x} > file | cat`) still creates or
//! truncates `file` in rubash, where GNU creates nothing — the
//! stage redirect preflight opens the target before the word
//! expansion that detects the failure (and an unopenable target
//! swallows the bad-substitution diagnostic outright). Fixing that
//! needs the preflight to move after expansion, a redirect-machinery
//! rework beyond this issue; the driver's routing walk already skips
//! a bad-substitution discard (see `pipeline_stage_bad_substitution_
//! discarded`).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3499-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("t.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("t.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's three shapes: two bad words report once and the
/// command is discarded; the pipeline form prints no `a=`; the
/// script continues to the next line.
#[test]
fn issue_shapes_one_diagnostic_and_discard() {
    let script = concat!(
        "echo ${-x} ${-y}\n",
        "echo a=${-x} b=${-y}\n",
        "echo a=${-x} | cat\n",
        "echo ${-x}\n",
        "echo DONE rc=$?\n",
    );
    let (stdout, stderr, code) = rubash_file("shapes", script);
    assert_eq!(stdout, "DONE rc=1\n");
    assert_eq!(
        stderr,
        concat!(
            "t.sh: line 1: ${-x}: bad substitution\n",
            "t.sh: line 2: a=${-x}: bad substitution\n",
            "t.sh: line 3: a=${-x}: bad substitution\n",
            "t.sh: line 4: ${-x}: bad substitution\n",
        )
    );
    assert_eq!(code, Some(0));
}

/// The latch must not leak: after a pipeline element's failure, the
/// next command still reports its own diagnostic and runs.
#[test]
fn no_latch_leak_into_next_command() {
    let script = concat!(
        "echo a=${-x} | cat\n",
        "echo ${-x}\n",
        "echo AFTER rc=$?\n",
        "echo ${-x} | cat\n",
        "echo ${-y}\n",
        "echo DONE\n",
    );
    let (stdout, stderr, code) = rubash_file("leak", script);
    assert_eq!(stdout, "AFTER rc=1\nDONE\n");
    assert_eq!(
        stderr,
        concat!(
            "t.sh: line 1: a=${-x}: bad substitution\n",
            "t.sh: line 2: ${-x}: bad substitution\n",
            "t.sh: line 4: ${-x}: bad substitution\n",
            "t.sh: line 5: ${-y}: bad substitution\n",
        )
    );
    assert_eq!(code, Some(0));
}

/// Pipeline positions: a bad head word, a bad last stage (pipeline
/// status 1), a bad first external stage (pipeline status stays the
/// last stage's 0), two bad elements (one diagnostic each), and a
/// bad assignment-only element (one diagnostic, status of `cat`).
#[test]
fn pipeline_positions_match_gnu() {
    let script = concat!(
        "${-x} | cat\n",
        "echo \"rcA=$?\"\n",
        "echo hi | cat ${-x}\n",
        "echo \"rcB=$?\"\n",
        "cat ${-x} | cat\n",
        "echo \"rcC=$?\"\n",
        "/bin/echo ${-x} | cat\n",
        "echo \"rcD=$?\"\n",
        "echo ${-x} | echo ${-y}\n",
        "echo \"rcE=$?\"\n",
        "x=${-x} | cat\n",
        "echo \"rcF=$?\"\n",
        "echo DONE\n",
    );
    let (stdout, stderr, code) = rubash_file("positions", script);
    assert_eq!(
        stdout,
        concat!("rcA=0\n", "rcB=1\n", "rcC=0\n", "rcD=0\n", "rcE=1\n", "rcF=0\n", "DONE\n",)
    );
    assert_eq!(
        stderr,
        concat!(
            "t.sh: line 1: ${-x}: bad substitution\n",
            "t.sh: line 3: ${-x}: bad substitution\n",
            "t.sh: line 5: ${-x}: bad substitution\n",
            "t.sh: line 7: ${-x}: bad substitution\n",
            "t.sh: line 9: ${-x}: bad substitution\n",
            "t.sh: line 9: ${-y}: bad substitution\n",
            "t.sh: line 11: ${-x}: bad substitution\n",
        )
    );
    assert_eq!(code, Some(0));
}

/// A bad word abandons the whole word list: a command substitution
/// after it never runs, and a second bad word in a non-first stage
/// still reports exactly once.
#[test]
fn later_words_never_expand_after_first_failure() {
    let script = concat!(
        "echo ${-x} $(echo SIDE) | cat\n",
        "echo \"rc1=$?\"\n",
        "echo ${-x} ${-y} | cat\n",
        "echo \"rc2=$?\"\n",
        "printf '%s\\n' ${-x} | cat\n",
        "echo \"rc3=$?\"\n",
        "echo hi | echo ${-x}\n",
        "echo \"rc4=$?\"\n",
        "echo DONE\n",
    );
    let (stdout, stderr, code) = rubash_file("later-words", script);
    assert_eq!(stdout, "rc1=0\nrc2=0\nrc3=0\nrc4=1\nDONE\n");
    assert_eq!(
        stderr,
        concat!(
            "t.sh: line 1: ${-x}: bad substitution\n",
            "t.sh: line 3: ${-x}: bad substitution\n",
            "t.sh: line 5: ${-x}: bad substitution\n",
            "t.sh: line 7: ${-x}: bad substitution\n",
        )
    );
    assert_eq!(code, Some(0));
}

/// The failure discards the ELEMENT, not the pipeline's failure
/// adjudication: a 0-status pipeline (bad first stage, healthy last)
/// fires neither the ERR trap nor errexit, but a 1-status pipeline
/// (bad last stage) fires both, like any failed pipeline.
#[test]
fn pipeline_failure_still_fires_err_trap_and_errexit() {
    let script = concat!(
        "trap 'echo TRAPPED' ERR\n",
        "echo a=${-x} | cat\n",
        "echo \"rcF=$?\"\n",
        "echo hi | echo ${-x}\n",
        "echo \"rcG=$?\"\n",
        "trap - ERR\n",
        "set -e\n",
        "echo a=${-x} | cat\n",
        "echo AFTER\n",
        "set +e\n",
        "( set -e; echo hi | echo ${-x}; echo \"SUB-AFTER\" )\n",
        "echo \"rcH=$?\"\n",
        "echo DONE\n",
    );
    let (stdout, stderr, code) = rubash_file("traps", script);
    assert_eq!(stdout, "rcF=0\nTRAPPED\nrcG=1\nAFTER\nrcH=1\nDONE\n");
    assert_eq!(
        stderr,
        concat!(
            "t.sh: line 2: a=${-x}: bad substitution\n",
            "t.sh: line 4: ${-x}: bad substitution\n",
            "t.sh: line 8: a=${-x}: bad substitution\n",
            "t.sh: line 11: ${-x}: bad substitution\n",
        )
    );
    assert_eq!(code, Some(0));
}

/// POSIX mode: the element is a forked child, so the jump is
/// contained there (execute_cmd.c:1811) and the element exits 1
/// while the script continues — no fatal abort of the script.
#[test]
fn posix_mode_element_stays_contained() {
    let script = concat!(
        "set -o posix\n",
        "echo a=${-x} | cat\n",
        "echo \"rcE=$?\"\n",
        "echo AFTER\n",
    );
    let (stdout, stderr, code) = rubash_file("posix", script);
    assert_eq!(stdout, "rcE=0\nAFTER\n");
    assert_eq!(stderr, "t.sh: line 2: a=${-x}: bad substitution\n");
    assert_eq!(code, Some(0));
}

/// A discarded element never runs, so a pre-failure argument's side
/// effect never happens: `rm marker ${-x}` must not delete the
/// marker, and a bad substitution inside a command substitution
/// still reports exactly once (through the ambient fd 2) while the
/// substitution captures nothing.
#[test]
fn discarded_element_never_runs_its_side_effects() {
    let dir = tempdir("side-effects");
    std::fs::write(dir.join("marker"), "x\n").expect("write marker");
    std::fs::write(
        dir.join("t.sh"),
        concat!(
            "rm marker ${-x} | cat\n",
            "echo \"rc=$?\"\n",
            "test -f marker && echo SURVIVED || echo GONE\n",
            "x=$(echo ${-x} | cat); echo \"x=[$x]\"\n",
        ),
    )
    .expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("t.sh")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "rc=0\nSURVIVED\nx=[]\n");
    assert_eq!(
        stderr,
        concat!(
            "t.sh: line 1: ${-x}: bad substitution\n",
            "t.sh: line 4: ${-x}: bad substitution\n",
        )
    );
    assert_eq!(output.status.code(), Some(0));
}
