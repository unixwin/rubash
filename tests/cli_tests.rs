use std::io::Write;
use std::process::{Command, Stdio};
use std::{fs, path::Path};

#[test]
fn bash_execution_string_reflects_c_command() {
    let command = "printf '%s\\n' \"$BASH_EXECUTION_STRING\"";
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(command)
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{command}\n")
    );
}

#[test]
fn c_command_uses_command_name_and_positional_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("printf '%s:%s:%s\\n' \"$0\" \"$1\" \"$#\"")
        .arg("arg0")
        .arg("alpha")
        .arg("beta")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "arg0:alpha:2\n");
}

#[test]
fn script_file_uses_script_name_and_positional_arguments() {
    let script_path = Path::new("target").join("rubash-cli-script-args.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(&script_path, "printf '%s:%s:%s\\n' \"$0\" \"$1\" \"$#\"\n").unwrap();
    let script = script_path.to_string_lossy().to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .arg("alpha")
        .arg("beta")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{script}:alpha:2\n")
    );
    let _ = fs::remove_file(script_path);
}

#[test]
fn stdin_script_uses_s_positional_arguments() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .arg("alpha")
        .arg("beta")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run rubash");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"printf '%s:%s:%s\\n' \"$0\" \"$1\" \"$#\"\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "rubash:alpha:2\n");
}
