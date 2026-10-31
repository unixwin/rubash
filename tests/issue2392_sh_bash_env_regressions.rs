//! Issue 2392: a shell invoked as `sh` must NOT read $BASH_ENV.
//!
//! GNU shell.c run_startup_files (shell.c:1214-1220): a
//! non-interactive shell sources $BASH_ENV only when
//! `posixly_correct == 0 && act_like_sh == 0` — invoked as `sh`
//! (act_like_sh, shell.c set_shell_name) or in posix mode, the file
//! is never read, in both the `-c` and the script-file form. A plain
//! bash non-interactive shell DOES read it (control).
//!
//! Oracle: GNU bash 5.3.20, script-file probes (see the issue's
//! Regression seed). The drivers below run under rubash itself; the
//! inner shell is a copy of the test binary named `bash` on PATH,
//! re-invoked through `exec -a sh` exactly like the seed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn rubash_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2392-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, contents: &str) {
    fs::write(dir.join(name), contents).expect("write scratch file");
}

/// Install the test binary into `dir` under the name `bash`, so a
/// bare `bash` word on PATH resolves to this build. A symlink where
/// the platform allows (a debug binary is far too large to copy
/// into a small tmpfs); a real copy on Windows.
fn install_bash_copy(dir: &Path) {
    let name = if cfg!(windows) { "bash.exe" } else { "bash" };
    let target = dir.join(name);
    #[cfg(unix)]
    std::os::unix::fs::symlink(rubash_bin(), &target).expect("link rubash binary as bash");
    #[cfg(not(unix))]
    fs::copy(rubash_bin(), &target).expect("copy rubash binary as bash");
}

fn run_driver(dir: &Path, driver: &str) -> (String, Option<i32>) {
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(rubash_bin())
        .arg(driver)
        .arg(dir)
        .env("PATH", path)
        .env("HOME", dir)
        .output()
        .expect("run driver under rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2392_sh_mode_ignores_bash_env_in_c_and_script_forms() {
    let dir = scratch("sh");
    install_bash_copy(&dir);
    write(&dir, "env11.sh", "SU11=eleven\n");
    write(&dir, "scr11.sh", "echo fileM=${SU11-unset}\n");
    write(
        &dir,
        "driver.sh",
        "BASH_ENV=$1/env11.sh bash -c 'exec -a sh \"$0\" -c \"echo M=\\${SU11-unset}\"' bash\n\
         BASH_ENV=$1/env11.sh bash -c 'exec -a sh \"$0\" scr11.sh' bash\n",
    );
    let (stdout, code) = run_driver(&dir, "driver.sh");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "M=unset\nfileM=unset\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2392_bash_mode_still_reads_bash_env_control() {
    let dir = scratch("bash");
    install_bash_copy(&dir);
    write(&dir, "env11.sh", "SU11=eleven\n");
    write(
        &dir,
        "driver.sh",
        "BASH_ENV=$1/env11.sh bash -c 'exec -a bash \"$0\" -c \"echo M=\\${SU11-unset}\"' bash\n",
    );
    let (stdout, code) = run_driver(&dir, "driver.sh");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "M=eleven\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2392_posix_mode_ignores_bash_env() {
    let dir = scratch("posix");
    write(&dir, "env11.sh", "SU11=eleven\n");
    let output = Command::new(rubash_bin())
        .arg("--posix")
        .arg("-c")
        .arg("echo M=${SU11-unset}")
        .env("BASH_ENV", dir.join("env11.sh"))
        .env("HOME", &dir)
        .output()
        .expect("run rubash --posix -c");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        "M=unset\n"
    );
    let _ = fs::remove_dir_all(&dir);
}
