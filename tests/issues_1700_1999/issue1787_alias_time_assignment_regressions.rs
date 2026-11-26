//! Issue rubash#1787: a `time` head INTRODUCED BY ALIAS EXPANSION
//! lost the token classification the direct spelling carries, so a
//! leading assignment behind it was dispatched as a command name
//! (127) instead of being classified as an assignment word.
//!
//! GNU expands aliases in the reader: the substituted text is
//! re-read in place, so `time tt x=1` with `tt` aliased to `time`
//! is indistinguishable from the literal `time time x=1` — the
//! inner simple command starts in pristine command position and
//! its leading words are assignment words (parse.y: the timespec
//! production is recursive, `pipeline_command: timespec
//! pipeline_command`). Verified against GNU bash 5.3.20
//! (self-compiled oracle), script-file mode with
//! `shopt -s expand_aliases`.
//!
//! Controls: the direct alias spelling at command position
//! (`tt x=1`) already classified correctly and must keep working,
//! and a non-assignment timed command through the same alias
//! (`time tt echo hi`) is unaffected.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1787-{}-{}",
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

const PRE: &str = "shopt -s expand_aliases\nalias tt='time'\n";

#[test]
fn issue1787_time_alias_head_applies_a_leading_assignment() {
    let (stdout, _, code) = rubash_file(&format!("{PRE}time tt x=1\necho \"X=$x S=$?\"\n"));
    assert_eq!(stdout, "X=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1787_time_alias_head_applies_multiple_assignments() {
    let (stdout, _, code) =
        rubash_file(&format!("{PRE}time tt x=1 y=2\necho \"X=$x Y=$y S=$?\"\n"));
    assert_eq!(stdout, "X=1 Y=2 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1787_time_alias_with_own_option_applies_a_leading_assignment() {
    let (stdout, _, code) = rubash_file(
        "shopt -s expand_aliases\nalias tp='time -p'\ntime tp x=1\necho \"X=$x S=$?\"\n",
    );
    assert_eq!(stdout, "X=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1787_alias_chain_to_time_applies_a_leading_assignment() {
    let (stdout, _, code) = rubash_file(&format!(
        "{PRE}alias t2='tt'\ntime t2 x=1\necho \"X=$x S=$?\"\n"
    ));
    assert_eq!(stdout, "X=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1787_time_alias_assignment_before_a_command_stays_temporary() {
    let (stdout, _, code) = rubash_file(&format!(
        "{PRE}time tt x=1 env | grep '^x='\necho \"S=$?\"\n"
    ));
    assert_eq!(stdout, "x=1\nS=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1787_alias_compound_assignment_is_accepted() {
    // The alias-expanded head is the keyword: the compound word
    // behind it is an assignment statement (the rubash#1786
    // position rule judged on the expanded text).
    let (stdout, _, code) = rubash_file(&format!("{PRE}tt a=(1 2)\necho \"A=${{#a[@]}} S=$?\"\n"));
    assert_eq!(stdout, "A=2 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1787_time_alias_compound_assignment_is_accepted() {
    let (stdout, _, code) = rubash_file(&format!(
        "{PRE}time tt arr=(1 2)\necho \"A=${{#arr[@]}} S=$?\"\n"
    ));
    assert_eq!(stdout, "A=2 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1787_direct_alias_head_control() {
    let (stdout, _, code) = rubash_file(&format!("{PRE}tt x=1\necho \"X=$x S=$?\"\n"));
    assert_eq!(stdout, "X=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1787_time_alias_echo_control() {
    let (stdout, _, code) = rubash_file(&format!("{PRE}time tt echo hi\necho \"S=$?\"\n"));
    assert_eq!(stdout, "hi\nS=0\n");
    assert_eq!(code, Some(0));
}
