use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[test]
fn c_external_cat_reads_raw_bytes_from_persistent_fd() {
    let path = env::temp_dir().join(format!("rubash-external-fd-{}.bin", std::process::id()));
    fs::write(&path, [0xff, b'\n']).expect("write raw fd input");
    let shell_path = path.to_string_lossy().replace('\\', "/");
    let output = crate::rubash_command()
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
    let output = crate::rubash_command()
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

    let output = crate::rubash_command()
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

    let output = crate::rubash_command()
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

    let output = crate::rubash_command()
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

    let output = crate::rubash_command()
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

    let output = crate::rubash_command()
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

    let output = crate::rubash_command()
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

    let output = crate::rubash_command()
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

    let output = crate::rubash_command()
        .env("PATH", path)
        .arg("-c")
        .arg("emitout >&-; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "status:1\n");
    // rubash#1035: the helper has no `#!`, so the kernel could not exec it
    // and GNU runs it as a shell script in-process (shell_execve ENOEXEC
    // fallback) — the write error surfaces as the script-internal `echo`
    // diagnostic, byte-matching bash. The diagnostic prefix is the path
    // actually opened — for a bare command word that is the PATH-resolved
    // path (GNU findcmd.c records the found path; verified against GNU
    // bash 5.3.20 on both the -c and program faces, 2026-10-31). The
    // earlier bare-name expectation pinned the pre-#2302 deviation.
    // Windows keeps the shell-spawn path, where the parent reports the
    // command-level write error as before.
    #[cfg(unix)]
    assert_eq!(
        stream_text(&output.stderr),
        format!(
            "{}: line 1: echo: write error: Bad file descriptor\n",
            if script_path.is_absolute() {
                script_path.display().to_string()
            } else {
                std::env::current_dir()
                    .unwrap()
                    .join(&script_path)
                    .display()
                    .to_string()
            }
        )
    );
    #[cfg(not(unix))]
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

    let output = crate::rubash_command()
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
    let output = crate::rubash_command()
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
    let output = crate::rubash_command()
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
fn c_exec_invalid_expanded_option_discards_redirect_and_keeps_stdout() {
    // GNU 5.3.20 measured (rubash#2175): `exec $fd>FILE` with fd=-1
    // fails option parsing (`exec: -1: invalid option`), and the whole
    // exec — redirections included — is discarded: the file is created
    // empty by the redirection setup but nothing is written to it, and
    // the shell continues with its original stdout, so `echo after`
    // lands on stdout. (Renamed from
    // c_exec_keeps_stdout_redirect_after_invalid_expanded_option,
    // whose name and expectations asserted the opposite of GNU.)
    let output_path = Path::new("target").join("rubash-cli-exec-invalid-option.txt");
    let _ = fs::remove_file(&output_path);
    let output = crate::rubash_command()
        .arg("-c")
        .arg(format!(
            "fd=-1; exec $fd>{}; echo after",
            output_path.to_string_lossy().replace('\\', "/")
        ))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(stream_text(&output.stdout), "after\n");
    assert!(stream_text(&output.stderr).contains("exec: -1: invalid option"));
    assert_eq!(read_text(&output_path), "");
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

    let output = crate::rubash_command()
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

    let output = crate::rubash_command()
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

    let output = crate::rubash_command()
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
    // Unique per call so parallel tests never share (and delete) one dir:
    // pid + counter is unique across concurrent test binaries and threads.
    // Never format a ThreadId here: its Debug form `ThreadId(N)` puts
    // brackets in a PATH entry and Windows `.cmd` resolution through such
    // a path fails (unresolved command, not a shell regression).
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    Path::new("target").join(format!(
        "rubash-cli-external-fd-copy-bin-{}-{}",
        std::process::id(),
        id
    ))
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
    let output = crate::rubash_command()
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
    let output = crate::rubash_command()
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
