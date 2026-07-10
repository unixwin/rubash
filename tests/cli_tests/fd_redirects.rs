use std::process::Command;
use std::{env, fs, path::Path};

#[test]
fn c_external_command_redirects_stdout_to_stderr_fd() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = bin_dir.join("emitout");
    let literal_fd_path = Path::new("&2");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(literal_fd_path);
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(&script_path, "echo external-fd-copy\n").unwrap();
    make_executable(&script_path);
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg("emitout >&2")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "external-fd-copy\n"
    );
    assert!(!literal_fd_path.exists());
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_external_command_redirects_stderr_to_stdout_fd() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = bin_dir.join("emiterr");
    let literal_fd_path = Path::new("&1");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(literal_fd_path);
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(&script_path, "echo external-error >&2\n").unwrap();
    make_executable(&script_path);
    let path = path_with_bin_first(&bin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("PATH", path)
        .arg("-c")
        .arg("emiterr 2>&1")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "external-error\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert!(!literal_fd_path.exists());
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn c_external_stderr_fd_copy_keeps_original_stdout_before_redirect() {
    let bin_dir = external_fd_copy_bin_dir();
    let script_path = bin_dir.join("emiterr");
    let output_path = Path::new("target").join("rubash-cli-external-fd-copy-output.txt");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(&output_path);
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(&script_path, "echo external-error >&2\n").unwrap();
    make_executable(&script_path);
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "external-error\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert_eq!(fs::read_to_string(&output_path).unwrap_or_default(), "");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(bin_dir);
}

fn external_fd_copy_bin_dir() -> std::path::PathBuf {
    Path::new("target").join("rubash-cli-external-fd-copy-bin")
}

fn path_with_bin_first(bin_dir: &Path) -> std::ffi::OsString {
    let old_path = env::var_os("PATH");
    env::join_paths(
        std::iter::once(bin_dir.to_path_buf())
            .chain(env::split_paths(old_path.as_deref().unwrap_or_default())),
    )
    .unwrap()
}

fn make_executable(path: &Path) {
    #[cfg(not(unix))]
    let _ = path;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}
