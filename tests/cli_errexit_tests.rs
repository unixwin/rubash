use std::process::Command;
use std::{fs, path::Path};

#[test]
fn script_errexit_allows_common_conditional_failures() {
    let script_path = Path::new("target").join("rubash-cli-errexit-common.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(
        &script_path,
        "set -e\n\
         if false; then echo bad-if; fi\n\
         false || echo recovered\n\
         ! false\n\
         echo after-invert\n\
         while false; do echo bad-while; done\n\
         echo before-final-failure\n\
         true && false\n\
         echo bad-after\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "recovered\nafter-invert\nbefore-final-failure\n"
    );
}

#[test]
fn script_errexit_exits_on_assignment_command_substitution_failure() {
    let script_path = Path::new("target").join("rubash-cli-errexit-assignment-comsub.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(&script_path, "set -e\nv=$(false)\necho bad\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
}

#[test]
fn script_assignment_command_substitution_success_keeps_running() {
    let script_path = Path::new("target").join("rubash-cli-assignment-comsub-success.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(
        &script_path,
        "set -e\nv=$(printf ok)\nprintf '%s\\n' \"$v\"\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\n");
}
