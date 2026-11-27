use std::{env, process::Command};

// rubash#3967: a here-string inside a command substitution inside a
// top-level `{ }` group was misread as a here-document. The brace-group
// heredoc-operator scan tested every `<` position, so the second `<` of
// `<<<` posed as the `<<` operator; the phantom verdict refused the
// group fold and the unfolded body then cascaded into unterminated
// here-document warnings (or syntax errors). GNU bash 5.3.20 runs every
// shape below cleanly. The scan now only opens an operator at a
// run-start `<`, matching read_token's greedy run tokenization.

fn run_c(script: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c probe")
}

fn assert_clean(script: &str, expected_stdout: &str) {
    let output = run_c(script);
    assert!(
        output.status.success(),
        "script {script:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected_stdout);
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn herestring_in_comsub_in_brace_group() {
    assert_clean("{ x=$(cat <<<foo); echo \"$x\"; }", "foo\n");
}

#[test]
fn herestring_on_inner_group_in_comsub_in_brace_group() {
    assert_clean("{ x=$( { cat; } <<<foo ); echo \"$x\"; }", "foo\n");
}

#[test]
fn herestring_spaced_word_in_comsub_in_brace_group() {
    assert_clean("{ x=$(cat <<< in); echo \"got:$x\"; }", "got:in\n");
}

#[test]
fn herestring_in_comsub_in_nested_brace_group() {
    assert_clean("{ { x=$(cat <<<foo); echo \"$x\"; }; }", "foo\n");
}

#[test]
fn herestring_in_comsub_in_brace_group_multiline_script() {
    let script_path = std::env::temp_dir().join("rubash-3967-multiline-group.sh");
    std::fs::write(&script_path, "{\nx=$(cat <<<foo)\necho \"$x\"\n}\n")
        .expect("write multiline group probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run multiline group probe");
    let _ = std::fs::remove_file(&script_path);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "foo\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn real_heredoc_in_comsub_in_brace_group_still_works() {
    // The fold refusal for groups that DO contain a heredoc operator
    // must stay: a terminated heredoc body inside the substitution is
    // gathered, not folded away.
    assert_clean("{ x=$(cat <<EOF2\nfoo\nEOF2\n); echo \"$x\"; }", "foo\n");
    assert_clean("{ cat <<EOF2\nfoo\nEOF2\n}", "foo\n");
}

#[test]
fn unterminated_heredoc_in_comsub_in_brace_group_still_warns() {
    // GNU parity: a heredoc whose body never arrives warns twice and
    // still runs the command (heredoc7.sub shape).
    let output = run_c("{ x=$(cat <<foo); echo \"rc=$?\"; }");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "rc=0\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("warning: command substitution: 1 unterminated here-document"),
        "missing header-close warning: {stderr}"
    );
    assert!(
        stderr.contains("delimited by end-of-file (wanted `foo')"),
        "missing eof warning: {stderr}"
    );
}
