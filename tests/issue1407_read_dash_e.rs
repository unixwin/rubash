//! Issue rubash#1407: `read -E` rejected as invalid option (GNU accepts).
//!
//! GNU bash 5.3 read.def:308 optstring "Eersa:d:i:n:p:t:u:N:" includes `-E`
//! (disable readline, inverse of `-e`). Rubash rejected it.
//!
//! Fix: src/executor/read_builtin.rs accepts `-E` (no-op, rubash has no
//! readline editor for read).

use std::process::Command;

fn rubash_with_stdin(script: &str, stdin: &str) -> (String, String, Option<i32>) {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let output = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn read_dash_e_accepted() {
    let (stdout, stderr, code) = rubash_with_stdin(r#"read -E x; echo "rc=$? x=[$x]""#, "hello\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty());
    assert!(stdout.contains("rc=0 x=[hello]"));
}
