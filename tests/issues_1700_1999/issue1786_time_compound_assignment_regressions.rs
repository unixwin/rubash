//! Issue rubash#1786: a compound assignment behind a `time` head was
//! rejected with `syntax error near unexpected token `('`, although
//! GNU accepts it — `time arr=(1 2)`, `time declare a=(1 2)`,
//! `time -p arr=(1 2)` all parse and apply.
//!
//! GNU: the timespec derives a full pipeline_command (parse.y:1416
//! `timespec pipeline_command`), so the timed simple command starts
//! in pristine command position and its assignment-statement
//! acceptance (read_command's PST_ASSIGNOK, parse.y:5791-5810) is
//! judged on its own words — the recorded `time` head and its
//! operand window are not among them. Verified against GNU bash
//! 5.3.20 (self-compiled oracle), script-file mode.
//!
//! Controls that must KEEP their verdicts: an assignment statement
//! is still rejected after an ordinary command word
//! (`echo a=(1)`), after a QUOTED `time` head (`'time' arr=(1 2)` —
//! the head is a command name, not the keyword), and when an
//! assignment prefix precedes the head (`x=1 time arr=(1 2)` —
//! rubash#1355: `time` is a command name there).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1786-{}-{}",
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
fn issue1786_time_accepts_a_compound_assignment() {
    let (stdout, _, code) = rubash_file("time arr=(1 2)\necho \"A=${#arr[@]} S=$?\"\n");
    assert_eq!(stdout, "A=2 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1786_time_accepts_a_declare_compound_argument() {
    let (stdout, _, code) = rubash_file("time declare a=(1 2)\necho \"B=${#a[@]} S=$?\"\n");
    assert_eq!(stdout, "B=2 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1786_time_posix_option_still_accepts_a_compound_assignment() {
    let (stdout, _, code) = rubash_file("time -p arr=(1 2)\necho \"A=${#arr[@]} S=$?\"\n");
    assert_eq!(stdout, "A=2 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1786_time_accepts_an_export_compound_argument() {
    let (stdout, _, code) = rubash_file("time export b=(3 4)\necho \"B=${#b[@]} S=$?\"\n");
    assert_eq!(stdout, "B=2 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1786_time_accepts_declare_with_an_option_word() {
    // The untimed form accepts the option word; the timed form must
    // reach the identical verdict on the timed command's own words.
    let (stdout, _, code) = rubash_file("time declare -a a=(1 2)\necho \"A=${#a[@]} S=$?\"\n");
    assert_eq!(stdout, "A=2 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1786_time_accepts_compound_after_a_scalar_prefix() {
    // Both assignment words belong to the timed simple command; the
    // compound one is judged in command position of that command.
    let (stdout, _, code) = rubash_file("time x=1 arr=(1 2)\necho \"A=${#arr[@]} X=$x S=$?\"\n");
    assert_eq!(stdout, "A=2 X=1 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1786_nested_time_accepts_a_compound_assignment() {
    let (stdout, _, code) = rubash_file("time time arr=(1 2)\necho \"A=${#arr[@]} S=$?\"\n");
    assert_eq!(stdout, "A=2 S=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1786_compound_argument_after_an_ordinary_word_stays_rejected() {
    let (stdout, _, code) = rubash_file("echo a=(1)\n");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
}

#[test]
fn issue1786_quoted_time_head_stays_rejected() {
    // A quoted head is a plain command name; the `(` after it is an
    // unexpected token in GNU too.
    let (stdout, _, code) = rubash_file("'time' arr=(1 2)\n");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
}

#[test]
fn issue1786_assignment_prefix_before_time_stays_rejected() {
    // rubash#1355: after an assignment prefix `time` is a command
    // name, so the compound word behind it is not an assignment
    // statement (GNU: syntax error).
    let (stdout, _, code) = rubash_file("x=1 time arr=(1 2)\n");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
}

#[test]
fn issue1786_untimed_compound_assignment_control() {
    let (stdout, _, code) = rubash_file("arr=(1 2)\necho \"A=${#arr[@]}\"\n");
    assert_eq!(stdout, "A=2\n");
    assert_eq!(code, Some(0));
}
