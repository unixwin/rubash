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
fn double_dash_allows_script_file_after_options() {
    let script_path = Path::new("target").join("rubash-cli-double-dash-script.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(&script_path, "printf '%s:%s\\n' \"$0\" \"$1\"\n").unwrap();
    let script = script_path.to_string_lossy().to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--")
        .arg(&script)
        .arg("alpha")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{script}:alpha\n")
    );
    let _ = fs::remove_file(script_path);
}

#[test]
fn posix_long_option_enables_posix_mode() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--posix")
        .arg("-c")
        .arg("type break")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "break is a special shell builtin\n"
    );
}

#[test]
fn profile_startup_options_are_accepted_before_command_string() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--noprofile")
        .arg("--norc")
        .arg("-c")
        .arg("printf '%s\\n' ok")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\n");
}

#[test]
fn cli_shell_flags_apply_before_command_string() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-u")
        .arg("-c")
        .arg("printf '%s\\n' \"$-\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .contains('u'));
}

#[test]
fn cli_plus_shell_flags_disable_previous_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-u")
        .arg("+u")
        .arg("-c")
        .arg("printf '%s\\n' \"$-\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .contains('u'));
}

#[test]
fn cli_shopt_options_apply_before_command_string() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-O")
        .arg("nullglob")
        .arg("-c")
        .arg("shopt -q nullglob; echo $?")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0\n");
}

#[test]
fn cli_plus_shopt_options_disable_previous_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-O")
        .arg("nullglob")
        .arg("+O")
        .arg("nullglob")
        .arg("-c")
        .arg("shopt -q nullglob; echo $?")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n");
}

#[test]
fn invalid_cli_shopt_option_fails_before_command_string() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-O")
        .arg("no_such_shopt")
        .arg("-c")
        .arg("echo should-not-run")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid shell option name"));
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
