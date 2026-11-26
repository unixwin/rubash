//! rubash#1059: `BASH_REMATCH` keeps one slot per capture group —
//! a group that did not participate is bound EMPTY, never dropped.
//!
//! GNU anchor: `subst.c` builds the rematch array from the regexec
//! result vector with nsub+1 entries; non-participating groups get
//! the empty string, so `${#BASH_REMATCH[@]}` counts them and
//! `${BASH_REMATCH[i]+set}` reports them set. rubash built the
//! array from the matched substrings only, shrinking the array and
//! leaving trailing groups UNSET.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::{Command, Stdio};

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!("rubash-issue1059-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1059 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The reported case: an optional middle group that matched empty
/// still occupies its slot — count 4, `[2]` set-but-empty, and
/// `declare -p` shows `[2]=""`.
#[test]
fn unmatched_optional_group_is_set_but_empty() {
    let (code, stdout, _) = run_script(
        "[[ ab =~ (a)(x)?(b) ]]; echo \"n=${#BASH_REMATCH[@]}\"\n\
         echo \"idx2=[${BASH_REMATCH[2]}] set2=${BASH_REMATCH[2]+SET}\"\n\
         declare -p BASH_REMATCH\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "n=4\n\
         idx2=[] set2=SET\n\
         declare -a BASH_REMATCH=([0]=\"ab\" [1]=\"a\" [2]=\"\" [3]=\"b\")\n"
    );
}

/// The alternation form (Linux re-measurement on the issue, 2026-10):
/// the branch that did not run is set-but-empty and the array keeps
/// all three slots.
#[test]
fn unmatched_alternation_branch_is_set_but_empty() {
    let (code, stdout, _) = run_script(
        "[[ ab =~ (a)|(x) ]]; echo \"alt-n=${#BASH_REMATCH[@]}\"\n\
         echo \"alt1=[${BASH_REMATCH[1]}] altset1=${BASH_REMATCH[1]+SET} alt2=[${BASH_REMATCH[2]}] altset2=${BASH_REMATCH[2]+SET}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "alt-n=3\nalt1=[a] altset1=SET alt2=[] altset2=SET\n"
    );
}

/// Control that already agreed: when every group participates the
/// array is the plain matched list.
#[test]
fn fully_matched_groups_stay_plain() {
    let (code, stdout, _) = run_script(
        "[[ ab =~ (a)(b) ]]; echo \"n=${#BASH_REMATCH[@]} ${BASH_REMATCH[1]} ${BASH_REMATCH[2]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=3 a b\n");
}
