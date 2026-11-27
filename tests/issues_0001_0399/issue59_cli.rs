use std::fs;
use std::path::Path;
use std::process::Command;

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn noexec_flag_skips_command_string() {
    let output_path = Path::new("target").join("issue59-noexec-command.txt");
    let _ = fs::remove_file(&output_path);
    let script = format!("printf should-not-run > {}", shell_test_path(&output_path));

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-n", "-c", &script])
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(!output_path.exists());
}

#[test]
fn noexec_flag_skips_script_file() {
    let script_path = Path::new("target").join("issue59-noexec-script.sh");
    let output_path = Path::new("target").join("issue59-noexec-script-output.txt");
    let _ = fs::remove_file(&output_path);
    fs::write(
        &script_path,
        format!(
            "printf should-not-run > {}\n",
            shell_test_path(&output_path)
        ),
    )
    .expect("write noexec script");

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-n"])
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(&script_path);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(!output_path.exists());
}
