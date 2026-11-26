//! Issue rubash#1805: a leading assignment behind a NESTED `time`
//! head was misrouted — `time time x=1` dispatched `x=1` as a
//! command name (127, `x=1: command not found`) and never applied
//! the assignment, although GNU applies it (status 0).
//!
//! GNU: the timespec production is recursive —
//! `pipeline_command: timespec pipeline_command` (parse.y) — so the
//! inner command after the second head is a simple command in
//! pristine command position, and its leading words classify as
//! assignment words exactly as behind a single head (the
//! rubash#1661 single-head case). Verified against GNU bash 5.3.20
//! (self-compiled oracle), script-file mode.
//!
//! Controls: a quoted inner word stays a command name
//! (`time "x=1"` -> 127 in GNU too), and a single head keeps
//! working (rubash#1661 pins cover it separately).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1805-{}-{}",
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

#[test]
fn issue1805_nested_time_applies_a_leading_assignment() {
    let (stdout, _, code) = rubash_file("time time x=1\necho \"X=$x S=$?\"\n");
    assert_eq!(stdout, "X=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_with_posix_option_applies_a_leading_assignment() {
    let (stdout, _, code) = rubash_file("time -p time x=1\necho \"X=$x S=$?\"\n");
    assert_eq!(stdout, "X=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_triple_time_applies_a_leading_assignment() {
    let (stdout, _, code) = rubash_file("time time time x=1\necho \"X=$x S=$?\"\n");
    assert_eq!(stdout, "X=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_applies_multiple_assignments() {
    let (stdout, _, code) = rubash_file("time time x=1 y=2\necho \"X=$x Y=$y S=$?\"\n");
    assert_eq!(stdout, "X=1 Y=2 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_assignment_before_a_command_stays_temporary() {
    // GNU: the assignments prefix the timed command (`env`), they
    // do not persist in the shell.
    let (stdout, _, code) = rubash_file("time time x=1 env | grep '^x='\necho \"X=$x S=$?\"\n");
    assert_eq!(stdout, "x=1\nX= S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_subscript_assignment_applies() {
    let (stdout, _, code) = rubash_file("time time arr[0]=x\necho \"A=${arr[0]} S=$?\"\n");
    assert_eq!(stdout, "A=x S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_echo_control() {
    let (stdout, _, code) = rubash_file("time time echo hi\necho \"S=$?\"\n");
    assert_eq!(stdout, "hi\nS=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_single_time_quoted_assignment_control() {
    // GNU: the quoted word is a command name even behind one head.
    let (stdout, _, code) = rubash_file("time \"x=1\"\necho \"X=$x S=$?\"\n");
    assert_eq!(stdout, "X= S=127\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_compound_assignment_applies() {
    let (stdout, _, code) = rubash_file("time time arr=(1 2)\necho \"A=${#arr[@]} S=$?\"\n");
    assert_eq!(stdout, "A=2 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_in_a_pipeline_applies_assignments() {
    // The timespec recursion applies to the pipeline_command as a
    // whole: both heads strip off the pipeline's first stage, whose
    // assignments prefix its command (here `env`) temporarily.
    let (stdout, _, code) = rubash_file("time time x=1 env | grep '^x='\necho \"X=$x S=$?\"\n");
    assert_eq!(stdout, "x=1\nX= S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_case_command_runs() {
    // The recursion also admits compound inners: a second recorded
    // head in the compound-start path introduces the inner timed
    // command exactly as the first did.
    let (stdout, _, code) = rubash_file("time time case w in w) z=1;; esac\necho \"Z=$z S=$?\"\n");
    assert_eq!(stdout, "Z=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_arithmetic_command_runs() {
    let (stdout, _, code) = rubash_file("time time ((y=1))\necho \"Y=$y S=$?\"\n");
    assert_eq!(stdout, "Y=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_if_command_runs() {
    let (stdout, _, code) = rubash_file("time time if true; then q=1; fi\necho \"Q=$q S=$?\"\n");
    assert_eq!(stdout, "Q=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_brace_group_runs() {
    let (stdout, _, code) = rubash_file("time time { b=1; }\necho \"B=$b S=$?\"\n");
    assert_eq!(stdout, "B=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_chain_prints_one_report() {
    // GNU: a pure time chain delegates its report to the innermost
    // frame — exactly one real/user/sys block for the whole chain
    // (the rubash#1611 rule, now also for parse-folded chains).
    let (_, stderr, code) = rubash_file("time time false\n");
    let reports = stderr
        .lines()
        .filter(|line| line.starts_with("user\t"))
        .count();
    assert_eq!(reports, 1, "stderr: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn issue1805_nested_time_empty_chain() {
    let (stdout, _, code) = rubash_file("time time\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1805_nested_time_inverted_assignment() {
    let (stdout, _, code) = rubash_file("! time time x=1\necho \"X=$x S=$?\"\n");
    assert_eq!(stdout, "X=1 S=1\n");
    assert_eq!(code, Some(0));
}
