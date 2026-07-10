use std::process::Command;
use std::{env, fs, path::Path};

#[test]
fn c_external_command_redirects_stdout_to_stderr_fd() {
    let bin_dir = Path::new("target").join("rubash-cli-external-fd-copy-bin");
    let script_path = bin_dir.join("emitout");
    let literal_fd_path = Path::new("&2");
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(literal_fd_path);
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(&script_path, "echo external-fd-copy\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
    }
    let old_path = env::var_os("PATH");
    let path = env::join_paths(
        std::iter::once(bin_dir.as_path().to_path_buf())
            .chain(env::split_paths(old_path.as_deref().unwrap_or_default())),
    )
    .unwrap();

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
