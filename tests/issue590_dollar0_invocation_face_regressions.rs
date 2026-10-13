//! rubash#590: `$0` reads the invocation face, not a hardcoded product name.
//!
//! GNU anchors: shell.c:1778-1806 set_shell_name binds
//! `dollar_vars[0] = argv[0]` (verbatim, shell.c:1797-1799) and keys the
//! sh/ash (shell.c:733-737) and rbash (shell.c:1258-1299) contracts off its
//! basename. One shim binary serves every installed face, so the invocation
//! name travels in NIU_INVOKED_AS (rubash#504, extended to every shell face
//! by #590); the engine consumes it — ahead of the process argv[0] path,
//! behind an explicit BASH_ARGV0 import (variables.c:1547-1555, which GNU
//! ranks above argv[0]) — and consumes it on read so it never leaks into
//! the shell environment.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// A throwaway install tree: the shim copied under each face name, with the
/// engine as the forward target (NIU_SHELL).
struct ShimTree {
    dir: PathBuf,
}

impl ShimTree {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("issue590-shim-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create shim tree");
        for face in ["sh.exe", "bash.exe", "rbash.exe"] {
            fs::copy(env!("CARGO_BIN_EXE_bash"), dir.join(face))
                .expect("copy shim under face name");
        }
        Self { dir }
    }

    fn run_face(&self, face: &str, script: &str) -> (String, String, Option<i32>) {
        let output = Command::new(self.dir.join(format!("{face}.exe")))
            .env("NIU_SHELL", env!("CARGO_BIN_EXE_rubash"))
            // The dev/CI parent may carry a niu session's BASH_ARGV0 or a
            // stale NIU_INVOKED_AS; the chain under test must see a clean
            // invocation.
            .env_remove("BASH_ARGV0")
            .env_remove("NIU_INVOKED_AS")
            .args(["-c", script])
            .output()
            .expect("run shim face");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for ShimTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// `sh -c 'echo $0'` prints `sh` (rubash#504 acceptance, re-pinned through
/// the #590 forward), and the sh POSIX contract still rides the same name.
#[test]
fn sh_face_reports_sh_as_dollar_zero_with_posix_on() {
    let tree = ShimTree::new();
    let (stdout, stderr, rc) = tree.run_face("sh", "echo $0; set -o | grep -w posix");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "sh\nposix          \ton\n", "stderr: {stderr:?}");
}

/// `bash -c 'echo $0'` prints `bash` — the face GNU set_shell_name
/// (shell.c:1797-1799) reports, not the host product name.
#[test]
fn bash_face_reports_bash_as_dollar_zero() {
    let tree = ShimTree::new();
    let (stdout, stderr, rc) = tree.run_face("bash", "echo $0");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "bash\n", "stderr: {stderr:?}");
}

/// `rbash -c 'echo $0'` prints `rbash` (shell.c:1258-1299 keys the
/// restricted shell off the same basename the forward carries).
#[test]
fn rbash_face_reports_rbash_as_dollar_zero() {
    let tree = ShimTree::new();
    let (stdout, stderr, rc) = tree.run_face("rbash", "echo $0");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "rbash\n", "stderr: {stderr:?}");
}

/// The forwarded name is consumed on read: the shell environment never sees
/// NIU_INVOKED_AS (the #504 isolation rule).
#[test]
fn forwarded_name_does_not_leak_into_the_shell_env() {
    let tree = ShimTree::new();
    let (stdout, stderr, rc) = tree.run_face("sh", "echo [$NIU_INVOKED_AS]");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n", "stderr: {stderr:?}");
}

/// The engine alone consumes NIU_INVOKED_AS as its argv[0] reconstruction:
/// `$0` and the invocation contract (posix for sh) both follow it.
#[test]
fn engine_honors_niu_invoked_as_directly() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("NIU_INVOKED_AS", "sh")
        .env_remove("BASH_ARGV0")
        .args(["-c", "echo $0; set -o | grep -w posix"])
        .output()
        .expect("run engine");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "sh\nposix          \ton\n");
}

/// An explicit BASH_ARGV0 import outranks the forwarded face — GNU
/// variables.c:1547-1555 ranks the environment value above argv[0]
/// ("BASH_ARGV0=this-bash bash -c 'echo $0 $BASH_ARGV0'" prints this-bash
/// twice).
#[test]
fn explicit_bash_argv0_outranks_the_forwarded_face() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("NIU_INVOKED_AS", "sh")
        .env("BASH_ARGV0", "custom-zero")
        .args(["-c", "echo $0"])
        .output()
        .expect("run engine");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "custom-zero\n");
}

/// A `-c` operand still rebinds `$0` over everything (shell.c:523-528: the
/// argv after the command string is $0, $1, ...).
#[test]
fn explicit_c_operand_still_wins() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("NIU_INVOKED_AS", "sh")
        .env_remove("BASH_ARGV0")
        .args(["-c", "echo $0", "operand-zero"])
        .output()
        .expect("run engine");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "operand-zero\n");
}

/// Without a shim face the engine keeps the GNU argv[0]-verbatim contract:
/// a bare name (PATH-style invocation) is reported as-is.
#[test]
fn bare_name_invocation_reports_argv0_verbatim() {
    let dir = std::env::temp_dir().join(format!("issue590-bare-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create bare tree");
    let exe = dir.join("faceprov.exe");
    fs::copy(env!("CARGO_BIN_EXE_rubash"), &exe).expect("copy engine");

    let output = Command::new("faceprov.exe")
        .env("PATH", &dir)
        .env_remove("BASH_ARGV0")
        .env_remove("NIU_INVOKED_AS")
        .args(["-c", "echo $0"])
        .output()
        .expect("run engine under bare name");

    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "faceprov.exe\n");
}

/// The stdin-script driver reports the same invocation face, not the engine
/// path a shim chain erased (shell.c:1613 only rebinds $0 for script FILES;
/// a stdin script keeps set_shell_name's argv[0]).
#[test]
fn stdin_script_keeps_the_forwarded_face_as_dollar_zero() {
    let tree = ShimTree::new();
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(tree.dir.join("sh.exe"))
        .env("NIU_SHELL", env!("CARGO_BIN_EXE_rubash"))
        .env_remove("BASH_ARGV0")
        .env_remove("NIU_INVOKED_AS")
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn shim sh -s");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(b"echo [$0]\n")
        .expect("write stdin script");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait shim sh -s");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "[sh]\n");
}
