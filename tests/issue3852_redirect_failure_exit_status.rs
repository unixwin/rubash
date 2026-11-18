//! rubash#3852: a failed output redirect must set the command's exit
//! status to 1 (GNU sets `$?` to 1); rubash printed the diagnostic but
//! left `$?` at 0 on the audit's #3835-WIP tree.
//!
//! GNU anchor: redir.c `do_redirections` (redir.c:237) calls
//! `do_redirection_internal` (redir.c:790) per entry; on failure it
//! calls `redirection_error` (redir.c:135), which sets
//! EXECUTION_FAILURE (redir.c:129) as the command's status (cf.
//! execute_cmd.c status propagation). The redirect error is fatal to
//! the command: the builtin never runs and `$?` is 1 afterwards.
//!
//! Expectations below are byte-verified against GNU bash 5.3.20
//! (oracle probe, same scripts, run as `nobody`): every form prints
//! `rc=1` and the diagnostic, the target file keeps its old content,
//! and the script itself exits 0 (its last command is `echo rc=$?`).
//! Unix-only: the divergence needs DAC enforcement. The probes run as
//! user `nobody` via setpriv when the test process is root (mirroring
//! the #3828 seed); as a non-root caller they run directly. When root
//! with no setpriv/nobody available the checks cannot enforce DAC and
//! skip (vacuous pass, noted on stderr) rather than false-passing.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3852-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin")).expect("scratch bin dir");
    std::fs::create_dir_all(dir.join("home")).expect("scratch home dir");
    std::fs::create_dir_all(dir.join("work")).expect("scratch work dir");
    // The shell binary runs as `nobody` below, so it must live inside
    // the scratch tree with world-readable/executable bits, like the
    // #3828 seed's $T/bin/shell copy.
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
        // 444 mode actually gates the open, as in the #3828 seed.
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

fn read_back(path: &Path) -> String {
    // The file is mode 444: the owner (or root) re-permits it before
    // reading, exactly like the #3828 seed's `chmod 644 f; cat f`.
    set_mode(path, 0o644);
    std::fs::read_to_string(path).expect("read target file")
}

fn skip_unless_dac() -> bool {
    if dac_enforced() {
        false
    } else {
        eprintln!("rubash#3852: root without setpriv/nobody; DAC unenforced, skipping");
        true
    }
}

/// Assert the shared shape: the script exits 0, prints `rc=1`, the
/// diagnostic names the failure, and the pre-existing file is intact.
fn assert_failed_redirect(out: &Outcome, root: &Path, needle: &str) {
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=1\n");
    assert!(
        out.stderr.contains(needle),
        "stderr lacks {needle:?}: {:?}",
        out.stderr
    );
    assert_eq!(read_back(&root.join("work").join("f13")), "x");
}

/// 1 (issue repro): `echo hi > f13` on a pre-existing 444 file sets
/// `$?` to 1 (GNU redir.c:790 failure path -> EXECUTION_FAILURE).
#[test]
fn echo_truncating_preexisting_444_sets_status_1() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("echo-444");
    let out = run(
        &root,
        "printf 'x' > f13; chmod 444 f13; echo hi > f13; echo rc=$?\n",
    );
    assert_failed_redirect(&out, &root, "Permission denied");
}

/// 2: the no-output builtin `:` with the same failing redirect.
#[test]
fn colon_no_output_builtin_sets_status_1() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("colon-444");
    let out = run(
        &root,
        "printf 'x' > f13; chmod 444 f13; : > f13; echo rc=$?\n",
    );
    assert_failed_redirect(&out, &root, "Permission denied");
}

/// 3: the no-output builtin `true` with the same failing redirect.
#[test]
fn true_no_output_builtin_sets_status_1() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("true-444");
    let out = run(
        &root,
        "printf 'x' > f13; chmod 444 f13; true > f13; echo rc=$?\n",
    );
    assert_failed_redirect(&out, &root, "Permission denied");
}

/// 4: the buffered-output builtin `printf` with the same failing
/// redirect (a different output-builtin face than `echo`).
#[test]
fn printf_buffered_builtin_sets_status_1() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("printf-444");
    let out = run(
        &root,
        "printf 'x' > f13; chmod 444 f13; printf 'y' > f13; echo rc=$?\n",
    );
    assert_failed_redirect(&out, &root, "Permission denied");
}

/// 5: the append form `>>` on the pre-existing 444 file.
#[test]
fn append_preexisting_444_sets_status_1() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("append-444");
    let out = run(
        &root,
        "printf 'x' > f13; chmod 444 f13; echo hi >> f13; echo rc=$?\n",
    );
    assert_failed_redirect(&out, &root, "Permission denied");
}

/// 6: the stderr form `2>` on the pre-existing 444 file.
#[test]
fn stderr_redirect_preexisting_444_sets_status_1() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("stderr-444");
    let out = run(
        &root,
        "printf 'x' > f13; chmod 444 f13; echo hi 2> f13; echo rc=$?\n",
    );
    assert_failed_redirect(&out, &root, "Permission denied");
}

/// 7: a different failure cause — missing directory (ENOENT) — also
/// sets `$?` to 1.
#[test]
fn missing_directory_enoent_sets_status_1() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("enoent");
    let out = run(&root, "echo hi > /no-such-dir/f; echo rc=$?\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=1\n");
    assert!(
        out.stderr.contains("No such file or directory"),
        "stderr lacks No such file or directory: {:?}",
        out.stderr
    );
}

/// 8: the wordless null command `> f13` with the failing redirect.
#[test]
fn wordless_redirect_preexisting_444_sets_status_1() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("wordless-444");
    let out = run(
        &root,
        "printf 'x' > f13; chmod 444 f13; > f13; echo rc=$?\n",
    );
    assert_failed_redirect(&out, &root, "Permission denied");
}

/// 9: a brace group's redirect `{ echo hi; } > f13` with the failing
/// target.
#[test]
fn brace_group_redirect_preexisting_444_sets_status_1() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("group-444");
    let out = run(
        &root,
        "printf 'x' > f13; chmod 444 f13; { echo hi; } > f13; echo rc=$?\n",
    );
    assert_failed_redirect(&out, &root, "Permission denied");
}

/// 10: an assignment-only command `x=1 > f13`: GNU sets `$?` to 1 and
/// the assignment still binds.
#[test]
fn assignment_only_redirect_preexisting_444_sets_status_1() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("assign-444");
    let out = run(
        &root,
        "printf 'x' > f13; chmod 444 f13; x=1 > f13; echo rc=$?-x=$x\n",
    );
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "rc=1-x=1\n");
    assert!(
        out.stderr.contains("Permission denied"),
        "stderr lacks Permission denied: {:?}",
        out.stderr
    );
    assert_eq!(read_back(&root.join("work").join("f13")), "x");
}
