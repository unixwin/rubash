//! niubash#993 §6 — `pwd -W` (the MSYS/Git Bash extension) prints the
//! win32 spelling of the physical cwd.
//!
//! Git Bash 5.3.15 probes (Windows, junction `jlink -> real`):
//!   cd /c/tmp        -> pwd -W `C:/tmp`
//!   cd /c            -> pwd -W `C:/`
//!   cd /c/tmp/jlink  -> pwd `/c/tmp/jlink`, pwd -W `C:/tmp/real`,
//!                     pwd -P `/c/tmp/real`   (physical resolution)
//!   pwd -LW / -PW    -> `C:/tmp`   (last option wins)
//!   pwd -WL / -WP    -> `/c/tmp`   (L/P override W)
//!   pwd -x           -> `pwd: usage: pwd [-LPW]`, status 2
//!   pwd --help       -> `pwd [-LPW]` synopsis + `-W` doc line
//!
//! rubash maps the resolved POSIX display through
//! `shell_path_to_windows` (src/executor/path.rs) — the same funnel
//! every shell-path consumer uses — rendered forward-slashed. The
//! `__RUBASH_PHYSICAL_PWD` bridge state maps the bridged name (`cd /`
//! -> the configured shell root's native path).

#![cfg(windows)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_script(tag: &str, script: &str) -> (String, String, i32) {
    let dir = std::env::temp_dir().join(format!("rubash-993s6-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path: PathBuf = dir.join("case.sh");
    std::fs::write(&path, script).expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&path)
        .current_dir(&dir)
        .env("HOME", &dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "SystemRoot",
        "SystemDrive",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "OS",
        "TEMP",
        "TMP",
    ] {
        if let Ok(value) = std::env::var(name) {
            cmd.env(name, value);
        }
    }
    let out = cmd.output().expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn pwd_w_prints_win32_spelling_of_cwd() {
    // run_script's cwd is rubash-993s6-main-<pid>; -W must print its
    // win32 drive form (Git Bash: `C:/<path>`).
    let fixture = std::env::temp_dir().join(format!("rubash-993s6-main-{}", std::process::id()));
    std::fs::create_dir_all(&fixture).expect("fixture dir");
    let expected_dir = fixture.canonicalize().expect("canonicalize fixture");
    let (stdout, stderr, code) = run_script("main", "pwd -W\npwd\n");
    assert_eq!(code, 0, "stderr: {stderr:?}");
    let win32 = expected_dir
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout:?}");
    assert_eq!(lines[0], win32, "win32 spelling: {stdout:?}");
    assert!(lines[1].starts_with('/'), "posix pwd: {stdout:?}");
}

#[test]
fn pwd_w_and_lp_share_one_option_slot() {
    let (stdout, _stderr, _code) = run_script("slots", "pwd -WP\npwd -WL\npwd -LW\npwd -PW\n");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 4, "stdout: {stdout:?}");
    assert!(lines[0].starts_with('/'), "-WP -> posix: {lines:?}");
    assert!(lines[1].starts_with('/'), "-WL -> posix: {lines:?}");
    assert!(lines[2].contains(":/"), "-LW -> win32: {lines:?}");
    assert!(lines[3].contains(":/"), "-PW -> win32: {lines:?}");
}

#[test]
fn invalid_option_reports_lpw_usage() {
    let (_stdout, stderr, code) = run_script("usage", "pwd -x\n");
    assert_eq!(code, 2, "stderr: {stderr:?}");
    assert!(
        stderr.contains("pwd: usage: pwd [-LPW]"),
        "stderr: {stderr:?}"
    );
}

/// `cd /` under a configured shell root: the bridged logical name maps
/// to the root's native path — the niubash equivalent of Git Bash's
/// `C:/Program Files/Git`.
#[test]
fn pwd_w_of_bridged_root_prints_shell_root() {
    let root = std::env::temp_dir().join(format!("rubash-993s6-root-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("root fixture");
    let root_native = root.to_string_lossy().replace('\\', "/");
    let root_drive = root_native.replace('/', "\\").replace("\\\\", "\\");
    let script = format!("export __RUBASH_SHELL_ROOT=\"{root_drive}\"\ncd / && pwd && pwd -W\n");
    let (stdout, stderr, code) = run_script("root", &script);
    assert_eq!(code, 0, "stderr: {stderr:?}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "/", "bridged logical pwd: {stdout:?}");
    assert_eq!(lines[1], root_native, "-W of bridged /: {stdout:?}");
}
