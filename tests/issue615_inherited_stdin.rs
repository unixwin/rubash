use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run_with_stdin(script: &str, input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run inherited stdin probe");
    child.stdin.take().unwrap().write_all(input).unwrap();
    child
        .wait_with_output()
        .expect("wait inherited stdin probe")
}

fn external_cat_script() -> String {
    let shell = env!("CARGO_BIN_EXE_rubash").replace('\\', "/");
    format!("exec 3<&0; '{shell}' -c cat <&3")
}

#[test]
fn external_child_duplicates_inherited_stdin() {
    let output = run_with_stdin(&external_cat_script(), b"first\nsecond\n");
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(output.stdout, b"first\nsecond\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn external_child_preserves_raw_stdin_bytes() {
    let input = [0, 0xff, 0x1d, b'\n'];
    let output = run_with_stdin(&external_cat_script(), &input);
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(output.stdout, input);
    assert!(output.stderr.is_empty());
}

#[test]
fn closed_input_descriptor_still_fails() {
    let shell = env!("CARGO_BIN_EXE_rubash").replace('\\', "/");
    let output = run_with_stdin(&format!("exec 3<&-; '{shell}' -c cat <&3"), b"");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Bad file descriptor"));
}
