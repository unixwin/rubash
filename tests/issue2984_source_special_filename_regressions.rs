//! rubash#2984 regressions (the junit #2 face): `source`/`.` double-
//! expanded its filename argument — the general simple-command path
//! expands every command once before dispatch (GNU execute_cmd.c:4612
//! expand_words), and `source`'s handler called `expand_command_words`
//! a second time, which re-read already-expanded DATA as SYNTAX: a
//! literal `"`, `'`, or `$` produced by the first expansion was
//! consumed as a quote delimiter / expansion introducer by
//! quote-removal, so `source "$F"` on a file named `a"b.src` reported
//! `ab.src: No such file or directory`. GNU source.def uses the words
//! as given — there is no second expansion. The nested junit run's
//! `.src` for the `xml-escape-"<>'&.bats` fixture never sourced, so
//! the gather died and junit #2 failed on a lines[] it never built.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2984-src-{}-{}",
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

fn check(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "script failed: {script}\nstderr: {stderr}");
    assert_eq!(stderr, "", "script {script} wrote to stderr");
    assert_eq!(stdout, expected_stdout, "script: {script}");
}

#[test]
fn issue2984_source_literal_filename_with_double_quote() {
    check(
        r#"printf 'echo INNER-DQ\n' > 'a"b.src'
source 'a"b.src'
"#,
        "INNER-DQ\n",
    );
}

#[test]
fn issue2984_source_expanded_filename_with_double_quote() {
    check(
        r#"printf 'echo INNER-DQ2\n' > 'a"b.src'
F='a"b.src'
source "$F"
"#,
        "INNER-DQ2\n",
    );
}

#[test]
fn issue2984_source_filename_with_single_quote() {
    check(
        r#"printf 'echo INNER-SQ\n' > "a'b.src"
source "a'b.src"
"#,
        "INNER-SQ\n",
    );
}

#[test]
fn issue2984_source_expanded_filename_with_dollar() {
    check(
        r#"printf 'echo INNER-DOLLAR\n' > 'a$b.src'
F='a$b.src'
source "$F"
"#,
        "INNER-DOLLAR\n",
    );
}

#[test]
fn issue2984_dot_filename_with_double_quote() {
    check(
        r#"printf 'echo INNER-DOT\n' > 'a"b.src'
. 'a"b.src'
"#,
        "INNER-DOT\n",
    );
}

#[test]
fn issue2984_source_missing_filename_still_errors() {
    let (stdout, stderr, code) = rubash_file(
        r#"source 'nonexistent.src'
echo "rc=$?"
"#,
    );
    assert_eq!(code, Some(0), "script failed, stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("nonexistent.src") && stderr.contains("No such file"),
        "missing-file diagnostic lost: {stderr}"
    );
}
