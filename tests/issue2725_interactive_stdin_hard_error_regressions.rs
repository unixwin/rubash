//! rubash#2725 (B5-01): the interactive stdin driver's completeness
//! grouping could not distinguish an immediately-invalid compound
//! opener from an incomplete one, so after `if then' it kept
//! gathering lines into the dead command and discarded the whole
//! group with a single error. GNU's reader parses incrementally
//! (parse.y read_command): a token that is wrong NOW errors on its
//! own line and the reader continues with the next line, while a
//! genuinely incomplete command keeps gathering (PS2).
use std::{
    env,
    io::Write,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

fn run_interactive(input: &str) -> (String, String, i32) {
    let dir = env::temp_dir().join(format!("rubash-2725-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .current_dir(&dir)
        .env_clear()
        .env("PATH", env::var("PATH").unwrap_or_default())
        .env("HOME", &dir)
        .env("TERM", "dumb")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(input.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let output = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("rubash -i did not exit within the bound")
        .expect("wait for rubash -i");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn issue2725_invalid_if_opener_errors_on_its_own_line() {
    let (stdout, stderr, code) = run_interactive("if then\necho in\nfi\necho after\nexit 4\n");
    assert_eq!(code, 4, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "in\nafter\n");
    assert!(
        stderr.contains("syntax error near unexpected token `then'"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("syntax error near unexpected token `fi'"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2725_invalid_function_opener_errors_immediately() {
    let (stdout, stderr, code) = run_interactive("f( { echo x;}\necho after\nexit 5\n");
    assert_eq!(code, 5, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "after\n");
    assert!(
        stderr.contains("syntax error near unexpected token `{'"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2725_genuinely_incomplete_commands_still_group() {
    let (stdout, stderr, code) = run_interactive("if true; then\necho grouped\nfi\nexit 6\n");
    assert_eq!(code, 6, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "grouped\n");

    let (stdout, stderr, code) = run_interactive("case x in\nx) echo cased;;\nesac\nexit 7\n");
    assert_eq!(code, 7, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "cased\n");

    let (stdout, stderr, code) = run_interactive("echo \"multi\nline\"\nexit 8\n");
    assert_eq!(code, 8, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "multi\nline\n");
}
