//! rubash#1037: an executable text file without a `#!` line is run by
//! the shell itself (the ENOEXEC fallback), not handed to a foreign
//! /bin/sh.
//!
//! GNU anchor: `execute_cmd.c` (`shell_execve`): when execve fails
//! with ENOEXEC the shell re-executes itself on the file, so shell
//! syntax (arrays, `[[ ]]`) works and `$0` is the path as invoked.
//! rubash delegated to /system/bin/sh, where bashisms died with a
//! foreign shell's syntax errors.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn scratch_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1037-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1037 scratch dir");
    dir
}

fn write_executable(dir: &std::path::Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod script");
}

fn run_parent(dir: &std::path::Path, parent_body: &str) -> (Option<i32>, String, String) {
    let parent = dir.join("parent.sh");
    std::fs::write(&parent, parent_body).expect("write parent script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&parent)
        .current_dir(dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash parent script");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The reported case: no shebang, bash-only syntax inside. GNU runs
/// it as a shell script: `runner=./nosheb.sh`, `len=2`, rc 0.
#[test]
fn shebangless_script_runs_in_the_shell_itself() {
    let dir = scratch_dir();
    write_executable(
        &dir,
        "nosheb.sh",
        "echo \"runner=$0\"\narr=(a b)\necho \"len=${#arr[@]}\"\n",
    );
    let (code, stdout, _) = run_parent(&dir, "./nosheb.sh; echo \"rc=$?\"\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "runner=./nosheb.sh\nlen=2\nrc=0\n");
}

/// Boundary that already agreed: a script WITH a `#!/bin/sh` line is
/// still handed to that interpreter.
#[test]
fn shebanged_script_uses_its_interpreter() {
    let dir = scratch_dir();
    write_executable(&dir, "withsheb.sh", "#!/bin/sh\necho \"via-sh\"\n");
    let (code, stdout, _) = run_parent(&dir, "./withsheb.sh; echo \"rc=$?\"\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "via-sh\nrc=0\n");
}
