use std::fs;
use std::process::{Command, Output};

fn run_case(tag: &str, script: &str) -> Output {
    let root = std::env::temp_dir().join(format!("rubash-615-copy-{tag}-{}", std::process::id()));
    fs::create_dir(&root).expect("create copy fixture");
    fs::create_dir(root.join("src")).unwrap();
    fs::create_dir(root.join("dst")).unwrap();
    fs::write(root.join("file"), b"original\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&root)
        .args(["-c", script])
        .output()
        .expect("run directory contents copy probe");
    fs::remove_dir_all(&root).expect("remove copy fixture");
    output
}

fn assert_failure(output: Output, diagnostic: &str) {
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(diagnostic),
        "{:?}",
        output.stderr
    );
}

#[test]
fn file_dot_reports_not_a_directory_and_failure() {
    assert_failure(
        run_case("file-dot", "cp -r file/. dst"),
        "cannot stat 'file/.': Not a directory",
    );
}

#[test]
fn missing_dot_preserves_no_such_file_error() {
    assert_failure(
        run_case("missing-dot", "cp -r missing/. dst"),
        "cannot stat 'missing/.': No such file or directory",
    );
}

#[test]
fn directory_contents_without_recursive_option_fails() {
    assert_failure(
        run_case("no-recursion", "cp src/. dst"),
        "-r not specified; omitting directory 'src/.'",
    );
}

#[test]
fn directory_contents_cannot_overwrite_regular_file() {
    assert_failure(
        run_case("overwrite-file", "cp -r src/. file"),
        "cannot overwrite non-directory 'file' with directory 'src/.'",
    );
}
