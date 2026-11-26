//! rubash#3828: output redirects that create a file without owner-write
//! permission (umask masking 0200) must not fail with `Permission denied`.
//! GNU opens each output redirect target exactly once per redirect entry
//! (redir.c:237 `do_redirections` -> one `do_redirection_internal`
//! (redir.c:790) -> one `redir_open` (redir.c:949, mode 0666)), so a
//! single create-open succeeds at the kernel level and the data lands.
//! rubash opened the target twice (a validation/probe open, then a second
//! open by path at bind/flush time); the second open fails EACCES once
//! the created mode lacks the owner-write bit, and the data is lost.
//!
//! Regression seed: R134-A (issue #3828 body). GNU bash 5.3.20: every
//! check below passes; rubash base 969230b9 fails the umask-0200 faces.
//! Unix-only: the divergence needs DAC enforcement. The probes run as
//! user `nobody` via setpriv when the test process is root (mirroring
//! the seed); as a non-root caller they run directly. When root with no
//! setpriv/nobody available the checks cannot enforce DAC and skip
//! (vacuous pass, noted on stderr) rather than false-passing.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3828-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin")).expect("scratch bin dir");
    std::fs::create_dir_all(dir.join("home")).expect("scratch home dir");
    std::fs::create_dir_all(dir.join("work")).expect("scratch work dir");
    // The shell binary runs as `nobody` below, so it must live inside
    // the scratch tree with world-readable/executable bits, like the
    // seed's $T/bin/shell copy.
    std::fs::copy(BIN, dir.join("bin").join("shell")).expect("copy shell binary");
    set_mode(&dir.join("bin").join("shell"), 0o755);
    set_mode(&dir, 0o755);
    set_mode(&dir.join("bin"), 0o755);
    set_mode(&dir.join("home"), 0o755);
    set_mode(&dir.join("work"), 0o777);
    std::fs::canonicalize(&dir).expect("canonical root")
}

fn set_mode(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .expect("set scratch permissions");
}

fn euid_is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim() == "0")
        .unwrap_or(false)
}

fn have_nobody_via_setpriv() -> bool {
    let setpriv = Command::new("setpriv").arg("--version").output().is_ok();
    let nobody = Command::new("id")
        .args(["-u", "nobody"])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    setpriv && nobody
}

/// Whether DAC enforcement is reachable for this run: non-root callers
/// enforce it directly; root callers need setpriv + nobody.
fn dac_enforced() -> bool {
    !euid_is_root() || have_nobody_via_setpriv()
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run(root: &Path, body: &str) -> Outcome {
    let work = root.join("work");
    std::fs::write(work.join("driver.sh"), body).expect("driver script");
    let shell = root.join("bin").join("shell");
    let mut cmd = if euid_is_root() {
        // Root bypasses DAC on most filesystems: drop to nobody so the
        // created mode actually gates the second open, as in the seed.
        let mut cmd = Command::new("setpriv");
        cmd.args(["--reuid=nobody", "--regid=nogroup", "--clear-groups"]);
        cmd.arg(&shell);
        cmd
    } else {
        Command::new(&shell)
    };
    if euid_is_root() {
        cmd.env_clear();
        cmd.env("PATH", "/usr/bin:/bin");
        cmd.env("HOME", root.join("home"));
        cmd.env("LC_ALL", "C");
        cmd.env("TZ", "UTC");
    } else {
        cmd.env("LC_ALL", "C");
    }
    let out = cmd
        .arg("driver.sh")
        .current_dir(&work)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path)
        .expect("target file exists")
        .permissions()
        .mode()
        & 0o777
}

fn read_back(path: &Path) -> String {
    // The file may be mode 000: the owner (or root) re-permits it
    // before reading, exactly like the seed's `chmod 644 f; cat f`.
    set_mode(path, 0o644);
    std::fs::read_to_string(path).expect("read target file")
}

fn skip_unless_dac() -> bool {
    if dac_enforced() {
        false
    } else {
        eprintln!("rubash#3828: root without setpriv/nobody; DAC unenforced, skipping");
        true
    }
}

/// 1 (seed check 1): umask 777, create-only redirect succeeds, mode 000.
#[test]
fn create_only_umask_777_succeeds() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("create-777");
    let out = run(&root, "umask 777; : > f1; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=0\n");
    assert_eq!(out.stderr, "");
    assert_eq!(mode_of(&root.join("work").join("f1")), 0);
}

/// 2 (seed check 2): umask 777, written data lands in the file.
#[test]
fn write_umask_777_data_lands() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("write-777");
    let out = run(&root, "umask 777; echo hello > f2; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=0\n");
    assert_eq!(out.stderr, "");
    assert_eq!(mode_of(&root.join("work").join("f2")), 0);
    assert_eq!(read_back(&root.join("work").join("f2")), "hello\n");
}

/// 3 (seed check 3): umask 0222 (mode 444), written data lands.
#[test]
fn write_umask_0222_data_lands() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("write-222");
    let out = run(&root, "umask 0222; echo hi > f3; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=0\n");
    assert_eq!(out.stderr, "");
    assert_eq!(mode_of(&root.join("work").join("f3")), 0o444);
    assert_eq!(read_back(&root.join("work").join("f3")), "hi\n");
}

/// 4 (seed check 4): umask 0266 (mode 400), create-only succeeds.
#[test]
fn create_only_umask_0266_succeeds() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("create-266");
    let out = run(&root, "umask 0266; : > f4; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=0\n");
    assert_eq!(out.stderr, "");
    assert_eq!(mode_of(&root.join("work").join("f4")), 0o400);
}

/// 5 (seed check 5): umask 0644 (mode 022, group+other write only),
/// data lands.
#[test]
fn write_umask_0644_data_lands() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("write-644");
    let out = run(&root, "umask 0644; echo hello > f5; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=0\n");
    assert_eq!(out.stderr, "");
    assert_eq!(mode_of(&root.join("work").join("f5")), 0o022);
    assert_eq!(read_back(&root.join("work").join("f5")), "hello\n");
}

/// 6 (seed check 6): umask 0600 (mode 066, owner-write stripped),
/// data lands.
#[test]
fn write_umask_0600_data_lands() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("write-600");
    let out = run(&root, "umask 0600; echo hello > f6; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=0\n");
    assert_eq!(out.stderr, "");
    assert_eq!(mode_of(&root.join("work").join("f6")), 0o066);
    assert_eq!(read_back(&root.join("work").join("f6")), "hello\n");
}

/// 7: append under umask 777 lands both writes in one file.
#[test]
fn append_umask_777_data_lands() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("append-777");
    // GNU bash 5.3.20 (verified): the second `>>` is a fresh open of
    // the mode-000 file and fails EACCES exactly like `>` — only the
    // first append's data lands, rc of that command is 1, and the
    // script's last `echo rc=$?` still exits 0.
    let out = run(&root, "umask 777; echo a >> f7; echo b >> f7; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=1\n");
    assert!(
        out.stderr.contains("Permission denied"),
        "stderr lacks Permission denied: {:?}",
        out.stderr
    );
    assert_eq!(mode_of(&root.join("work").join("f7")), 0);
    assert_eq!(read_back(&root.join("work").join("f7")), "a\n");
}

/// 8: a persistent `exec 3> f` under umask 777 binds once; a later
/// write through fd 3 lands.
#[test]
fn exec_persistent_redirect_umask_777() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("exec-777");
    let out = run(&root, "umask 777; exec 3> f8; echo hi >&3; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=0\n");
    assert_eq!(out.stderr, "");
    assert_eq!(mode_of(&root.join("work").join("f8")), 0);
    assert_eq!(read_back(&root.join("work").join("f8")), "hi\n");
}

/// 9: a persistent `exec 2> f` under umask 777 captures stderr.
#[test]
fn exec_stderr_redirect_umask_777() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("exec2-777");
    let out = run(&root, "umask 777; exec 2> f9; echo oops >&2; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=0\n");
    assert_eq!(out.stderr, "");
    assert_eq!(mode_of(&root.join("work").join("f9")), 0);
    assert_eq!(read_back(&root.join("work").join("f9")), "oops\n");
}

/// 10: a group's stderr redirect under umask 777 delivers the data.
#[test]
fn group_stderr_redirect_umask_777() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("group-777");
    let out = run(&root, "umask 777; { echo oops >&2; } 2> f10; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=0\n");
    assert_eq!(out.stderr, "");
    assert_eq!(mode_of(&root.join("work").join("f10")), 0);
    assert_eq!(read_back(&root.join("work").join("f10")), "oops\n");
}

/// 11: an external command's redirect under umask 777 lands its output.
#[test]
fn external_command_redirect_umask_777() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("ext-777");
    let out = run(&root, "umask 777; /bin/echo hello > f11; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=0\n");
    assert_eq!(out.stderr, "");
    assert_eq!(mode_of(&root.join("work").join("f11")), 0);
    assert_eq!(read_back(&root.join("work").join("f11")), "hello\n");
}

/// 12: `<>` under umask 777 creates the file, rc 0.
#[test]
fn readwrite_create_only_umask_777() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("rw-777");
    let out = run(&root, "umask 777; : <> f12; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=0\n");
    assert_eq!(out.stderr, "");
    assert_eq!(mode_of(&root.join("work").join("f12")), 0);
}

/// 13 (control, seed check 9): truncating a PRE-EXISTING 444 file
/// still fails rc 1 with `Permission denied` on both shells.
#[test]
fn preexisting_unwritable_file_still_fails() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("control-444");
    // GNU bash 5.3.20 (verified): the redirect fails with status 1,
    // but the script's exit status is the FINAL command's — `echo
    // rc=$?` prints rc=1 and itself exits 0.
    let out = run(
        &root,
        "printf 'x' > f13; chmod 444 f13; echo hi > f13; echo rc=$?\n",
    );
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=1\n");
    assert!(
        out.stderr.contains("Permission denied"),
        "stderr lacks Permission denied: {:?}",
        out.stderr
    );
    assert_eq!(read_back(&root.join("work").join("f13")), "x");
}
