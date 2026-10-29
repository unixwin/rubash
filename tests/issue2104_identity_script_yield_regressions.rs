//! Regression pins for rubash#2104: the identity tools (`uname`,
//! `arch`) are hidden in-process faces whose stand-down policy lives
//! in the command registry (ScriptYield/FailClosed). A PATH
//! resolution landing on a script must run that script — GNU has no
//! uname/arch builtin, and nvm's `get_arch` places a mock `uname`
//! first on PATH — while a native-image resolution keeps the
//! persona answer and an unresolvable name stays 127 (the #1545
//! face, pinned in issue1545's file, not repeated here).
//!
//! Unix-only: the shadow shapes are `#!` scripts resolved through a
//! `.` PATH element, the POSIX resolution surface the defect was
//! measured on. The registry values themselves are pinned
//! platform-independently in command_registry's unit tests.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join("rubash")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2104-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run_in_dir(dir: &Path, script: &str) -> String {
    let output = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(dir)
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    let mut text = String::from_utf8_lossy(&output.stdout).replace('\r', "");
    let stderr = String::from_utf8_lossy(&output.stderr).replace('\r', "");
    if !stderr.is_empty() {
        text.push_str("STDERR:");
        text.push_str(&stderr);
    }
    text
}

fn write_script(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write mock");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod mock");
}

#[test]
fn issue2104_uname_script_shadow_runs_the_script() {
    let dir = scratch("uname-script");
    write_script(&dir, "uname", "echo MOCKX");
    let out = run_in_dir(
        &dir,
        "export PATH=\".:${PATH}\"; \
         echo \"plain: $(uname -m)\"; \
         echo \"command: $(command uname -m)\"; \
         x=$(uname -m); echo \"comsub: $x\"",
    );
    assert_eq!(out, "plain: MOCKX\ncommand: MOCKX\ncomsub: MOCKX\n");
}

#[test]
fn issue2104_arch_script_shadow_runs_the_script() {
    let dir = scratch("arch-script");
    write_script(&dir, "arch", "echo ARCHMOCK");
    let out = run_in_dir(&dir, "export PATH=\".:${PATH}\"; arch");
    assert_eq!(out, "ARCHMOCK\n");
}

#[test]
fn issue2104_native_resolution_keeps_the_persona_answer() {
    // A native-image candidate (an ELF copy of /bin/true) first on
    // PATH is the synced body the persona stands in for: the face
    // must NOT yield — running it would print nothing at all, while
    // the persona answers the machine name.
    let dir = scratch("native");
    let true_bin = ["/bin/true", "/usr/bin/true"]
        .iter()
        .map(Path::new)
        .find(|p| p.exists())
        .expect("a system true binary");
    let fake = dir.join("uname");
    fs::copy(true_bin, &fake).expect("copy true as uname");
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod fake uname");
    let out = run_in_dir(
        &dir,
        &format!(
            "export PATH=\"{}:${{PATH}}\"; uname -m; echo rc=$?",
            dir.display()
        ),
    );
    #[cfg(target_arch = "x86_64")]
    assert_eq!(out, "x86_64\nrc=0\n");
    #[cfg(not(target_arch = "x86_64"))]
    assert!(
        out.ends_with("\nrc=0\n") && !out.starts_with("rc="),
        "persona answered: {out:?}"
    );
}

#[test]
fn issue2104_function_still_shadows_the_identity_face() {
    let dir = scratch("function");
    let out = run_in_dir(&dir, "uname() { echo FN; }; uname");
    assert_eq!(out, "FN\n");
}
