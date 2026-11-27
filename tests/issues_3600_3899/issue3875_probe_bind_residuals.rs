//! rubash#3875: the probe-then-bind double-open of rubash#3828 survived
//! on four residual faces - the env leg, pipeline stage spawns, the
//! pipeline routing walk (incl. `<>` stages), and plan-decline shapes.
//! GNU opens each redirect entry exactly once (redir.c:237
//! `do_redirections` -> one `do_redirection_internal` (redir.c:790) ->
//! one `redir_open` (redir.c:725)); every walk that needs an entry's
//! open must adopt the one open an earlier walk performed (the pending
//! preopened channel of #3828) instead of re-opening by path, which
//! fails EACCES on a file the umask created without owner access and
//! loses the data.
//!
//! Harness mirrors tests/issue3828_umask_redirect_permission.rs: the
//! probes run as user `nobody` via setpriv when the test process is
//! root; as a non-root caller they run directly. When DAC cannot be
//! enforced the checks skip (vacuous pass, noted on stderr).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3875-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin")).expect("scratch bin dir");
    std::fs::create_dir_all(dir.join("home")).expect("scratch home dir");
    std::fs::create_dir_all(dir.join("work")).expect("scratch work dir");
    std::fs::copy(BIN, dir.join("bin").join("shell")).expect("copy shell binary");
    set_mode(&dir.join("bin").join("shell"), 0o755);
    set_mode(&dir, 0o755);
    set_mode(&dir.join("bin"), 0o755);
    set_mode(&dir.join("home"), 0o755);
    set_mode(&dir.join("work"), 0o777);
    // ENOEXEC subject: an executable script with no shebang, doubling
    // as the plain external command for the pipeline fd>=3 face.
    let ext = dir.join("work").join("ext.sh");
    std::fs::write(&ext, "echo \"$@\"\n").expect("seed ext.sh");
    set_mode(&ext, 0o755);
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
    // Process-substitution carriers live under TMPDIR; keep them in
    // the scratch tree so the nobody shell can create them.
    cmd.env("TMPDIR", root.join("home"));
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
    // The file may be mode 000: re-permit before reading.
    set_mode(path, 0o644);
    std::fs::read_to_string(path).expect("read target file")
}

fn exists_eventually(path: &Path) -> bool {
    // Process-substitution consumers finish asynchronously; poll
    // briefly for the file they create.
    for _ in 0..40 {
        if path.exists() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    path.exists()
}

fn skip_unless_dac() -> bool {
    if dac_enforced() {
        false
    } else {
        eprintln!("rubash#3875: root without setpriv/nobody; DAC unenforced, skipping");
        true
    }
}

fn assert_clean(out: &Outcome) {
    assert_eq!(out.rc, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "rc=0\n", "stderr: {}", out.stderr);
    assert_eq!(out.stderr, "", "stdout: {}", out.stdout);
}

/// Face (a): the env leg had no preflight gate; on a plan decline the
/// plan walk opened the plain target, dropped the handle, and the
/// legacy branch re-opened by path.
#[test]
fn env_leg_decline_procsub() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("env-decline");
    let out = run(
        &root,
        "umask 777; env /bin/echo hello > f 3> >(cat > g); echo rc=$?\n",
    );
    assert_clean(&out);
    assert_eq!(read_back(&root.join("work").join("f")), "hello\n");
    assert!(exists_eventually(&root.join("work").join("g")));
}

/// Face (a): the env leg with a `{var}` fd entry after the plain
/// target - the plan declines on the fd-var shape.
#[test]
fn env_leg_fdvar_after_plain() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("env-fdvar");
    let out = run(
        &root,
        "umask 777; env /bin/echo hello > f {v}> g; echo rc=$?\n",
    );
    assert_clean(&out);
    assert_eq!(read_back(&root.join("work").join("f")), "hello\n");
    assert!(root.join("work").join("g").exists());
}

/// Face (a): the ENOEXEC fallback legs of the external spawn dropped
/// the caller's preopened map and re-opened by path.
#[test]
fn enoexec_fallback_forwards_preopened() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("enoexec");
    let out = run(&root, "umask 777; ./ext.sh hello > f; echo rc=$?\n");
    assert_clean(&out);
    assert_eq!(read_back(&root.join("work").join("f")), "hello\n");
}

/// Face (b): a pipeline stage's plain-file redirect on fd >= 3 was
/// opened by the stage gate and again by the stage spawn.
#[test]
fn pipeline_stage_fd3() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("stage-fd3");
    let out = run(&root, "umask 777; ./ext.sh 3> f | cat > sink; echo rc=$?\n");
    assert_clean(&out);
    assert!(root.join("work").join("f").exists());
    assert_eq!(read_back(&root.join("work").join("sink")), "\n");
}

/// Face (b): the last stage's own `> sink`-class target on an earlier
/// stage double-opened the same way.
#[test]
fn pipeline_stage_stdout_file() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("stage-fd1");
    let out = run(
        &root,
        "umask 777; /bin/echo hello > f | cat > sink; echo rc=$?\n",
    );
    assert_clean(&out);
    assert_eq!(read_back(&root.join("work").join("f")), "hello\n");
    assert_eq!(read_back(&root.join("work").join("sink")), "");
}

/// Face (c): a builtin pipeline stage's routing walk re-opened fd 1.
#[test]
fn pipeline_builtin_stage_routing() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("route-builtin");
    let out = run(
        &root,
        "umask 777; echo hello > f | cat > sink; echo rc=$?\n",
    );
    assert_clean(&out);
    assert_eq!(read_back(&root.join("work").join("f")), "hello\n");
}

/// Face (c): a `<>` entry on a builtin pipeline stage - the reject
/// gate's O_RDWR open must be the only one.
#[test]
fn pipeline_readwrite_builtin_stage() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("rw-builtin-stage");
    let out = run(&root, "umask 777; : <> f | cat > sink; echo rc=$?\n");
    assert_clean(&out);
    assert!(root.join("work").join("f").exists());
    assert!(root.join("work").join("sink").exists());
}

/// Face (c): a `<>` entry on an external pipeline stage.
#[test]
fn pipeline_readwrite_external_stage() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("rw-ext-stage");
    let out = run(
        &root,
        "umask 777; /bin/echo <> f | cat > sink; echo rc=$?\n",
    );
    assert_clean(&out);
    assert!(root.join("work").join("f").exists());
}

/// Face (c), broader than filed: a plain external `<>` (no pipeline)
/// also double-opened - the spawn's RW bind never adopted the reject
/// gate's parked handle.
#[test]
fn plain_external_readwrite() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("rw-plain");
    let out = run(&root, "umask 777; /bin/true <> f; echo rc=$?\n");
    assert_clean(&out);
    assert!(root.join("work").join("f").exists());
}

/// Face (d): a procsub entry makes the spawn plan decline after it
/// opened the plain target; the legacy branch must adopt the gate's
/// open instead of re-opening by path.
#[test]
fn plan_decline_procsub() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("decline-procsub");
    let out = run(
        &root,
        "umask 777; /bin/echo hello > f 3> >(cat > g); echo rc=$?\n",
    );
    assert_clean(&out);
    assert_eq!(read_back(&root.join("work").join("f")), "hello\n");
    assert!(exists_eventually(&root.join("work").join("g")));
}

/// Face (d): a `{var}` fd entry makes the plan decline the same way.
#[test]
fn plan_decline_fdvar() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("decline-fdvar");
    let out = run(&root, "umask 777; /bin/echo hello > f {v}> g; echo rc=$?\n");
    assert_clean(&out);
    assert_eq!(read_back(&root.join("work").join("f")), "hello\n");
    assert!(root.join("work").join("g").exists());
}

/// Face (d): combined output plus a procsub entry - the plan declines
/// after opening both plain targets.
#[test]
fn plan_decline_combined_procsub() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("decline-combined");
    let out = run(
        &root,
        "umask 777; /bin/echo hello > f 2> e 3> >(cat > g); echo rc=$?\n",
    );
    assert_clean(&out);
    assert_eq!(read_back(&root.join("work").join("f")), "hello\n");
    assert!(root.join("work").join("e").exists());
    assert!(exists_eventually(&root.join("work").join("g")));
}

/// Face (c): a function pipeline stage - the stage gate's open must
/// survive the transfer into the stage subshell and the function
/// splice's pre-create.
#[test]
fn function_stage_call_redirect() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("func-stage");
    let out = run(
        &root,
        "umask 777; greet() { echo hello; }; greet > f | cat > sink; echo rc=$?\n",
    );
    assert_clean(&out);
    assert_eq!(read_back(&root.join("work").join("f")), "hello\n");
    assert_eq!(read_back(&root.join("work").join("sink")), "");
}

/// The emulated `cat` writer is the bind for its fd 1 mirror: it must
/// adopt the retained preflight open instead of re-opening by path
/// (the shape every `>(cat > g)` consumer body exercises).
#[test]
fn emulated_cat_mirror_adopts() {
    if skip_unless_dac() {
        return;
    }
    let root = scratch("cat-mirror");
    let out = run(&root, "umask 777; cat > g < /dev/null; echo rc=$?\n");
    assert_clean(&out);
    assert!(root.join("work").join("g").exists());
}
