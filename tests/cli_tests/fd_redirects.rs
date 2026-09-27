use std::process::Command;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[test]
fn c_external_cat_reads_raw_bytes_from_persistent_fd() {
    let path = env::temp_dir().join(format!("rubash-external-fd-{}.bin", std::process::id()));
    fs::write(&path, [0xff, b'\n']).expect("write raw fd input");
    let shell_path = path.to_string_lossy().replace('\\', "/");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!("exec 3< '{shell_path}'; cat <&3"))
        .output()
        .expect("run external cat persistent fd probe");
    let _ = fs::remove_file(&path);

    assert!(output.status.success());
    assert_eq!(output.stdout, [0xff, b'\n']);
    assert_eq!(output.stderr, b"");
}

#[test]
fn c_filtered_command_substitution_preserves_raw_bytes() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("x=$(printf 'x\\377\\n' | sed -n '1p'); printf '%s\\n' \"$x\"")
        .output()
        .expect("run filtered command substitution probe");

    assert!(output.status.success());
    assert_eq!(output.stdout, [b'x', 0xff, b'\n']);
    assert_eq!(output.stderr, b"");
}

#[test]
fn c_external_command_redirects_stdout_to_stderr_fd() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = helper_path(&bin_dir, "emitout");
    let literal_fd_path = Path::new("&2");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(literal_fd_path);
    fs::create_dir_all(&bin_dir).unwrap();
    write_helper_script(&script_path, "echo external-fd-copy\n");
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg("emitout >&2")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "");
    assert_eq!(stream_text(&output.stderr), "external-fd-copy\n");
    assert!(!literal_fd_path.exists());
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_external_command_redirects_stderr_to_stdout_fd() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = helper_path(&bin_dir, "emiterr");
    let literal_fd_path = Path::new("&1");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(literal_fd_path);
    fs::create_dir_all(&bin_dir).unwrap();
    write_helper_script(&script_path, "echo external-error >&2\n");
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg("emiterr 2>&1")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "external-error\n");
    assert_eq!(stream_text(&output.stderr), "");
    assert!(!literal_fd_path.exists());
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_external_stderr_fd_copy_keeps_original_stdout_before_redirect() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = helper_path(&bin_dir, "emiterr");
    let output_path = Path::new("target").join("rubash-cli-external-fd-copy-output.txt");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(&output_path);
    fs::create_dir_all(&bin_dir).unwrap();
    write_helper_script(&script_path, "echo external-error >&2\n");
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg(format!(
            "emiterr 2>&1 > {}",
            output_path.to_string_lossy().replace('\\', "/")
        ))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "external-error\n");
    assert_eq!(stream_text(&output.stderr), "");
    assert_eq!(read_text_or_default(&output_path), "");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_external_command_uses_persistent_fd_copied_from_stdout() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = helper_path(&bin_dir, "emitout");
    let literal_fd_path = Path::new("&3");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(literal_fd_path);
    fs::create_dir_all(&bin_dir).unwrap();
    write_helper_script(&script_path, "echo external-via-fd\n");
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg("exec 3>&1; emitout >&3")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "external-via-fd\n");
    assert_eq!(stream_text(&output.stderr), "");
    assert!(!literal_fd_path.exists());
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_external_command_uses_persistent_fd_copied_from_stderr() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = helper_path(&bin_dir, "emitout");
    let literal_fd_path = Path::new("&3");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(literal_fd_path);
    fs::create_dir_all(&bin_dir).unwrap();
    write_helper_script(&script_path, "echo external-via-fd\n");
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg("exec 3>&2; emitout >&3")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "");
    assert_eq!(stream_text(&output.stderr), "external-via-fd\n");
    assert!(!literal_fd_path.exists());
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_external_stderr_uses_persistent_fd_copied_from_stdout() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = helper_path(&bin_dir, "emiterr");
    let literal_fd_path = Path::new("&3");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(literal_fd_path);
    fs::create_dir_all(&bin_dir).unwrap();
    write_helper_script(&script_path, "echo external-error-via-fd >&2\n");
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg("exec 3>&1; emiterr 2>&3")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "external-error-via-fd\n");
    assert_eq!(stream_text(&output.stderr), "");
    assert!(!literal_fd_path.exists());
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_external_command_reports_bad_fd_after_exec_close() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = helper_path(&bin_dir, "emitout");
    let literal_fd_path = Path::new("&3");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(literal_fd_path);
    fs::create_dir_all(&bin_dir).unwrap();
    write_helper_script(&script_path, "echo external-after-close\n");
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg("exec 3>&-; emitout >&3; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "status:1\n");
    assert_eq!(
        stream_text(&output.stderr),
        "bash: line 1: 3: Bad file descriptor\n"
    );
    assert!(!literal_fd_path.exists());
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_external_command_reports_write_error_for_closed_stdout() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = helper_path(&bin_dir, "emitout");
    let _ = fs::remove_dir_all(&bin_dir);
    fs::create_dir_all(&bin_dir).unwrap();
    write_helper_script(&script_path, "echo external-closed-stdout\n");
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg("emitout >&-; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "status:1\n");
    assert_eq!(
        stream_text(&output.stderr),
        "bash: line 1: emitout: write error: Bad file descriptor\n"
    );
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_external_command_reports_ambiguous_stderr_fd_redirect() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = helper_path(&bin_dir, "emiterr");
    let literal_fd_path = Path::new("&bad");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(literal_fd_path);
    fs::create_dir_all(&bin_dir).unwrap();
    write_helper_script(&script_path, "echo external-error >&2\n");
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg("emiterr 2>&bad; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "status:1\n");
    assert_eq!(
        stream_text(&output.stderr),
        "bash: line 1: bad: ambiguous redirect\n"
    );
    assert!(!literal_fd_path.exists());
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_builtin_command_reports_ambiguous_redirect_after_unquoted_expansion() {
    let _ = fs::remove_file("a b");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("target='a b'; echo hi > $target; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "status:1\n");
    assert_eq!(
        stream_text(&output.stderr),
        "bash: line 1: $target: ambiguous redirect\n"
    );
    assert!(!Path::new("a b").exists());
}

#[test]
fn c_exec_reports_ambiguous_redirect_for_invalid_expanded_fd() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("fd=-1; exec <&$fd; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "status:1\n");
    assert_eq!(
        stream_text(&output.stderr),
        "bash: line 1: -1: ambiguous redirect\n"
    );
}

#[test]
fn c_exec_keeps_stdout_redirect_after_invalid_expanded_option() {
    let output_path = Path::new("target").join("rubash-cli-exec-invalid-option.txt");
    let _ = fs::remove_file(&output_path);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!(
            "fd=-1; exec $fd>{}; echo after",
            output_path.to_string_lossy().replace('\\', "/")
        ))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "");
    assert!(stream_text(&output.stderr).contains("exec: -1: invalid option"));
    assert_eq!(read_text(&output_path), "after\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn c_external_combined_redirect_preserves_stderr_first_output() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = helper_path(&bin_dir, "emitboth");
    let output_path = Path::new("target").join("rubash-cli-external-combined-output.txt");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(&output_path);
    fs::create_dir_all(&bin_dir).unwrap();
    write_helper_script(&script_path, "echo external-error >&2\necho external-out\n");
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg(format!(
            "emitboth &> {}",
            output_path.to_string_lossy().replace('\\', "/")
        ))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "");
    assert_eq!(stream_text(&output.stderr), "");
    assert_eq!(read_text(&output_path), "external-error\nexternal-out\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_external_combined_append_preserves_existing_and_both_streams() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = helper_path(&bin_dir, "emitboth");
    let output_path = Path::new("target").join("rubash-cli-external-combined-append.txt");
    let _ = fs::remove_dir_all(&bin_dir);
    fs::write(&output_path, "first\n").unwrap();
    fs::create_dir_all(&bin_dir).unwrap();
    write_helper_script(&script_path, "echo external-error >&2\necho external-out\n");
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg(format!(
            "emitboth &>> {}",
            output_path.to_string_lossy().replace('\\', "/")
        ))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "");
    assert_eq!(stream_text(&output.stderr), "");
    assert_eq!(
        read_text(&output_path),
        "first\nexternal-error\nexternal-out\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_builtin_printf_preserves_left_to_right_stderr_redirects() {
    let output_path = Path::new("target").join("rubash-cli-printf-stderr-order.txt");
    let _ = fs::remove_file(&output_path);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!(
            "printf 'first\\n' >&2 2> {}; printf 'second\\n' >&2 2>> {}; printf 'both\\n' >&2 2>&1",
            output_path.to_string_lossy().replace('\\', "/"),
            output_path.to_string_lossy().replace('\\', "/")
        ))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "");
    assert_eq!(stream_text(&output.stderr), "first\nsecond\nboth\n");
    assert!(output_path.exists());
    assert_eq!(read_text(&output_path), "");
    let _ = fs::remove_file(output_path);
}

fn external_fd_copy_bin_dir() -> std::path::PathBuf {
    Path::new("target").join("rubash-cli-external-fd-copy-bin")
}

fn helper_path(bin_dir: &Path, name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        bin_dir.join(format!("{name}.cmd"))
    }

    #[cfg(not(windows))]
    {
        bin_dir.join(name)
    }
}

fn write_helper_script(path: &Path, content: &str) {
    #[cfg(windows)]
    {
        let body = content.replace(" >&2", ">&2").replace('\n', "\r\n");
        fs::write(path, format!("@echo off\r\n{body}")).unwrap();
    }

    #[cfg(not(windows))]
    {
        fs::write(path, content).unwrap();
        make_executable(path);
    }
}

fn stream_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[test]
fn c_dynamic_fd_closed_redirect_preserves_source_token_diagnostic() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"exec {fd}>&1; exec {fd}>&-; echo hi >&$fd; printf 'status:%s\n' "$?""#)
        .output()
        .expect("run closed dynamic fd diagnostic probe");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "status:1\n");
    assert_eq!(
        stream_text(&output.stderr),
        "bash: line 1: $fd: Bad file descriptor\n"
    );
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

fn read_text_or_default(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

#[test]
fn c_dynamic_varredir_close_closes_fd_after_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "shopt -s varredir_close; : {fd}>&1; \
             printf 'allocated=%s\\n' \"$fd\"; \
             echo after >&$fd; printf 'write_status=%s\\n' \"$?\"",
        )
        .output()
        .expect("run varredir_close lifetime probe");

    assert!(output.status.success());
    assert_eq!(
        stream_text(&output.stdout),
        "allocated=10\nwrite_status=1\n"
    );
    assert_eq!(
        stream_text(&output.stderr),
        "bash: line 1: $fd: Bad file descriptor\n"
    );
}

fn path_with_bin_first(bin_dir: &Path) -> std::ffi::OsString {
    let old_path = env::var_os("PATH");
    env::join_paths(
        std::iter::once(bin_dir.to_path_buf())
            .chain(env::split_paths(old_path.as_deref().unwrap_or_default())),
    )
    .unwrap()
}

#[cfg(not(windows))]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}
