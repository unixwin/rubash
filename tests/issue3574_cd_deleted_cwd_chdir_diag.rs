//! rubash#3574 — in the poisoned-cwd state (a failed `pwd -P` went
//! through resetpwd and left the_current_working_directory NULL,
//! rubash#3349), GNU's `cd` entry re-derivation emits one `chdir:`
//! diagnostic per `change_to_directory` call
//! (builtins/cd.def:529-534 — `get_working_directory ("chdir")`,
//! builtins/common.c:592-624), and the move still stands; rubash
//! emitted none. The count is per attempt, not per invocation:
//! the CDPATH candidates in order, then the plain operand, then the
//! cdable_vars value (oracle p2/p2b/p14/p15 — 1 failed CDPATH unit
//! + hit = 2 lines; 2 failed units + plain fail = 3; cdable_vars
//! success = plain + cdable_vars = 2). Early returns that never
//! reach change_to_directory (too many arguments, null directory,
//! unset HOME/OLDPWD) owe no line (oracle p8/p9).
//!
//! Companion correction in the same state: for a dot-only operand
//! GNU's make_absolute base is NULL, so canon cannot fail on a
//! deleted component and the verbatim chdir's resetpwd arm (the
//! `cd:` line rubash#3335's stayed_deleted_dot owes) never runs —
//! the `chdir:` line is the only diagnostic (oracle p5/p12).
//!
//! Deliberately NOT pinned here: the bound $PWD value after a
//! RELATIVE cd from the poisoned state. GNU binds the make_absolute
//! form against the NULL base (`.` / `..` as spelled), while rubash
//! binds the stale-base logical destination (`$R/gone/.` / `$R`) —
//! a separate PWD-value residual, recorded in the lane REPORT for
//! its own issue. Those cells assert only the status and the
//! diagnostics.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-11-16), stdout and stderr separately. Unix-only.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

/// Tree: <base>/gone (the directory deleted under the shell) plus
/// <base>/dest (a surviving target for CDPATH cells).
struct Tree {
    base: PathBuf,
    /// Canonical spelling of base (what physical lookups report).
    real: String,
}

fn make_tree(tag: &str) -> Tree {
    let base = std::env::temp_dir().join(format!("rubash3574-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("gone")).expect("create gone");
    std::fs::create_dir_all(base.join("dest")).expect("create dest");
    let real = std::fs::canonicalize(&base)
        .expect("canonicalize base")
        .to_string_lossy()
        .into_owned();
    Tree { base, real }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn run_script_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .arg(&case)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&case);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const GETCWD_DIAGNOSTIC: &str =
    "pwd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory\n";

const CHDIR_DIAGNOSTIC: &str =
    "chdir: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory\n";

/// `cd "<gone>"; rmdir ../gone; pwd -P; echo "rc=$?"; pwd; echo "rc=$?"` —
/// the poison, shared by the cells that start from it.
fn poison_script(real: &str) -> String {
    format!(
        "cd \"{real}/gone\"; rmdir ../gone; pwd -P; echo \"rc=$?\"; pwd; echo \"rc=$?\"; ",
        real = real
    )
}

#[test]
fn issue3574_cd_absolute_after_poison_emits_one_chdir_line() {
    // The filed face (the #3349 cell5 shape): two pwd diagnostics,
    // then exactly one `chdir:` line from the cd entry, and the move
    // stands — the poison clears and pwd works again.
    let tree = make_tree("cell1");
    let script = format!(
        "{}cd \"{r}\"; pwd; echo \"rc=$?\"\n",
        poison_script(&tree.real),
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, format!("rc=1\nrc=1\n{r}\nrc=0\n", r = tree.real));
    assert_eq!(
        stderr,
        format!("{}{}", GETCWD_DIAGNOSTIC.repeat(2), CHDIR_DIAGNOSTIC)
    );
}

#[test]
fn issue3574_cd_absolute_without_poison_emits_nothing() {
    // Control: no prior physical failure, so the cached logical value
    // stands and the cd entry has nothing to re-derive — GNU is
    // silent here (oracle case2), and so is rubash.
    let tree = make_tree("ctrl1");
    let script = format!(
        "cd \"{r}/gone\"; rmdir ../gone; cd \"{r}\"; pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, format!("{r}\nrc=0\n", r = tree.real));
    assert_eq!(stderr, "");
}

#[test]
fn issue3574_cd_failing_after_poison_emits_chdir_then_failure() {
    // A failing cd still owes the entry line (change_to_directory
    // re-derives before it chdirs), then the failure diagnostic,
    // then the trailing poisoned pwd re-derives once more.
    let tree = make_tree("cell3");
    let script = format!(
        "{}cd /nonexistent-xyz-3574; echo \"cdrc=$?\"; pwd; echo \"rc=$?\"\n",
        poison_script(&tree.real)
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nrc=1\ncdrc=1\nrc=1\n");
    assert!(stderr.starts_with(&format!(
        "{}{}",
        GETCWD_DIAGNOSTIC.repeat(2),
        CHDIR_DIAGNOSTIC
    )));
    assert!(stderr.contains("cd: /nonexistent-xyz-3574: No such file or directory\n"));
    // Three pwd faces: the poison's `pwd -P` and `pwd`, then the
    // trailing `pwd` after the failed cd re-derives once more
    // (GNU 5.3.20 oracle, cell3).
    assert_eq!(stderr.matches(GETCWD_DIAGNOSTIC).count(), 3);
    assert_eq!(stderr.matches(CHDIR_DIAGNOSTIC).count(), 1);
}

#[test]
fn issue3574_cdpath_failed_unit_then_hit_emits_two_chdir_lines() {
    // One line per change_to_directory call: the failed /nonexistent
    // unit's attempt, then the hit unit's attempt. The CDPATH
    // announcement still prints the found directory.
    let tree = make_tree("cell4");
    let script = format!(
        "{}CDPATH=\"/nonexistent:{r}\"; cd dest; echo \"cdrc=$?\"; pwd; echo \"rc=$?\"\n",
        poison_script(&tree.real),
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        format!(
            "rc=1\nrc=1\n{r}/dest\ncdrc=0\n{r}/dest\nrc=0\n",
            r = tree.real
        )
    );
    assert_eq!(
        stderr,
        format!(
            "{}{}",
            GETCWD_DIAGNOSTIC.repeat(2),
            CHDIR_DIAGNOSTIC.repeat(2)
        )
    );
}

#[test]
fn issue3574_cd_dot_after_poison_emits_only_chdir_line() {
    // Dot-only operand in the poisoned state: the entry line is the
    // ONLY diagnostic — GNU's canon cannot fail on a deleted
    // component against the NULL base, so the resetpwd `cd:` line
    // rubash#3335 owes in the unpoisoned state is not owed here.
    let tree = make_tree("cell5");
    let script = format!("{}cd .; echo \"cdrc=$?\"\n", poison_script(&tree.real));
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nrc=1\ncdrc=0\n");
    assert_eq!(
        stderr,
        format!("{}{}", GETCWD_DIAGNOSTIC.repeat(2), CHDIR_DIAGNOSTIC)
    );
}

#[test]
fn issue3574_cd_dotdot_after_poison_emits_one_chdir_line() {
    // Relative operand, same entry rule. The bound $PWD value is
    // deliberately not asserted (see the module doc: GNU binds "..",
    // rubash binds the stale-base destination — a separate residual).
    let tree = make_tree("cell6");
    let script = format!("{}cd ..; echo \"cdrc=$?\"\n", poison_script(&tree.real));
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nrc=1\ncdrc=0\n");
    assert_eq!(
        stderr,
        format!("{}{}", GETCWD_DIAGNOSTIC.repeat(2), CHDIR_DIAGNOSTIC)
    );
}

#[test]
fn issue3574_cd_no_arg_after_poison_emits_one_chdir_line() {
    // `cd` with no operand (cd $HOME) goes through
    // change_to_directory too, so it owes the same one line.
    let tree = make_tree("cell7");
    let script = format!(
        "{}HOME=\"{r}\"; cd; echo \"cdrc=$?\"; pwd; echo \"rc=$?\"\n",
        poison_script(&tree.real),
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        format!("rc=1\nrc=1\ncdrc=0\n{r}\nrc=0\n", r = tree.real)
    );
    assert_eq!(
        stderr,
        format!("{}{}", GETCWD_DIAGNOSTIC.repeat(2), CHDIR_DIAGNOSTIC)
    );
}

#[test]
fn issue3574_cd_too_many_after_poison_emits_no_chdir_line() {
    // The too-many-arguments refusal returns before
    // change_to_directory (cd.def:329-330), so no entry line is owed.
    // GNU stderr: the two poison pwd diagnostics, then the refusal
    // line (`…: cd: too many arguments`, EX_USAGE) — zero `chdir:`
    // lines (oracle p8, GNU bash 5.3.20).
    let tree = make_tree("cell8");
    let script = format!("{}cd a b; echo \"cdrc=$?\"\n", poison_script(&tree.real));
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nrc=1\ncdrc=2\n");
    assert_eq!(stderr.matches(GETCWD_DIAGNOSTIC).count(), 2);
    assert_eq!(stderr.matches(CHDIR_DIAGNOSTIC).count(), 0);
    assert!(
        stderr.contains("cd: too many arguments"),
        "stderr missing the refusal line: {stderr}"
    );
}

#[test]
fn issue3574_cd_null_after_poison_emits_no_chdir_line() {
    // The null-directory refusal (cd.def:331-336) likewise returns
    // before change_to_directory. GNU stderr: the two poison pwd
    // diagnostics, then the refusal line (`…: cd: null directory`) —
    // zero `chdir:` lines (oracle p9, GNU bash 5.3.20).
    let tree = make_tree("cell9");
    let script = format!("{}cd \"\"; echo \"cdrc=$?\"\n", poison_script(&tree.real));
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nrc=1\ncdrc=1\n");
    assert_eq!(stderr.matches(GETCWD_DIAGNOSTIC).count(), 2);
    assert_eq!(stderr.matches(CHDIR_DIAGNOSTIC).count(), 0);
    assert!(
        stderr.contains("cd: null directory"),
        "stderr missing the refusal line: {stderr}"
    );
}

#[test]
fn issue3574_cdable_vars_after_poison_emits_two_chdir_lines() {
    // cdable_vars is a second change_to_directory call after the
    // plain operand's failed attempt: plain + variable value = 2
    // lines. The value is echoed verbatim on success.
    let tree = make_tree("cell10");
    let script = format!(
        "{}shopt -s cdable_vars; v=\"{r}\"; cd v; echo \"cdrc=$?\"; pwd; echo \"rc=$?\"\n",
        poison_script(&tree.real),
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        format!("rc=1\nrc=1\n{r}\ncdrc=0\n{r}\nrc=0\n", r = tree.real)
    );
    assert_eq!(
        stderr,
        format!(
            "{}{}",
            GETCWD_DIAGNOSTIC.repeat(2),
            CHDIR_DIAGNOSTIC.repeat(2)
        )
    );
}
