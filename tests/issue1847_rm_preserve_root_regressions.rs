//! Regression pins for rubash#1847: the retained rm emulation had
//! no preserve-root failsafe — `--preserve-root`/`--no-preserve-root`
//! parsed as no-ops and a recursive removal of the root directory
//! would have proceeded where GNU refuses by default. The emulation
//! now tracks the flag (coreutils rm.c: `--preserve-root` is the
//! default and its only accepted argument is `all`) and refuses a
//! recursive operand that resolves to the root directory, with
//! coreutils remove.c's two-line diagnostic; other operands still
//! run and the status is 1.
//!
//! Expected behavior was probed against the local oracle (GNU
//! coreutils 9.4) inside a disposable chroot jail:
//! - `/` refuses; `//` refuses with the operand spelled `//` and a
//!   ` (same as '/')` suffix; `///` is reported as plain `/` (the
//!   walked path collapses a leading run of 3+ slashes);
//! - `/.`, `/..`, `/tmp/..` get the pre-existing dot/dotdot refusal
//!   instead (GNU's check order, already pinned by the #534 family);
//! - a slashed symlink to the root (`rootlink/`) refuses with the
//!   suffix; a bare symlink operand is NOT the root — GNU unlinks
//!   the link itself (which exposed that the emulation's symlink
//!   branch used remove_dir on unix, an ENOTDIR that -f swallowed:
//!   `rm -rf dirlink` was a silent no-op; the branch now unlinks on
//!   unix and keeps remove_dir only on Windows, where a directory
//!   symlink needs RemoveDirectory);
//! - `--no-preserve-root` lifts the failsafe; `--preserve-root=all`
//!   is accepted; `--preserve-root=bogus` is its own one-line
//!   diagnostic; `--no-preserve-root=x` is a getopt argument error.
//!
//! The root-operand pins cannot run against the host's real `/` —
//! if the failsafe regressed, the test would wipe the machine — so
//! they run the built shell inside a chroot jail whose `/` is a
//! scratch tree with marker files and a fake `rm` script on the
//! jail PATH (a declined invocation shows up as a FAKEBIN marker).
//! Every jail script first checks a canary file that only exists
//! inside the jail and exits 99 otherwise, so a jail that silently
//! failed to engage can never reach the dangerous operand. When
//! the jail cannot be built (not root, no chroot, no ldd, static
//! binary), the jail pins return early — the same precedent as the
//! #1636 pty pin. The non-recursive root controls and the symlink
//! pins are host-safe and always run.

#[cfg(unix)]
use std::collections::BTreeSet;
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Command, Stdio};

#[cfg(unix)]
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
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

#[cfg(unix)]
struct Output {
    status: i32,
    stdout: String,
    stderr: String,
}

#[cfg(unix)]
fn collect(mut child: std::process::Child) -> Output {
    let _ = child.stdin.take();
    let output = child.wait_with_output().expect("wait for rubash");
    Output {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
    }
}

#[cfg(unix)]
fn run_in_dir(dir: &Path, script: &str, path_env: &str) -> Output {
    let child = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(dir)
        .env("PATH", path_env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    collect(child)
}

#[cfg(unix)]
fn scratch(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("rubash-preserve-root-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[cfg(unix)]
const SYSTEM_PATH: &str = "/usr/bin:/bin";

/// Every jail script starts with this: the canary exists only
/// inside the jail, so on any accidental host run the script exits
/// 99 before an rm operand is ever reached.
#[cfg(unix)]
const CANARY: &str = "test -f /JAIL-CANARY || exit 99; ";

#[cfg(unix)]
fn chroot_bin() -> Option<PathBuf> {
    for candidate in ["/usr/sbin/chroot", "/sbin/chroot", "/usr/bin/chroot"] {
        if Path::new(candidate).exists() {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

#[cfg(unix)]
fn copy_into_jail(jail: &Path, host_path: &Path) -> Option<()> {
    let rel = host_path.strip_prefix("/").ok()?;
    let dest = jail.join(rel);
    std::fs::create_dir_all(dest.parent()?).ok()?;
    std::fs::copy(host_path, &dest).ok()?;
    Some(())
}

/// Build a chroot jail under `base/jail` holding the shell binary,
/// /bin/sh and /bin/cat (with their ldd libraries), a fake `rm`
/// script, marker files, and root symlinks. Returns the jail path,
/// or None when the environment cannot host one (the caller then
/// skips the pin).
#[cfg(unix)]
fn build_jail(base: &Path) -> Option<PathBuf> {
    let jail = base.join("jail");
    for d in ["bin", "sub", "dev"] {
        std::fs::create_dir_all(jail.join(d)).ok()?;
    }
    let mut ldd_inputs = Vec::new();
    for (host, name) in [
        (shell_bin(), "rubash"),
        (PathBuf::from("/bin/sh"), "sh"),
        (PathBuf::from("/usr/bin/cat"), "cat"),
    ] {
        std::fs::copy(&host, jail.join("bin").join(name)).ok()?;
        ldd_inputs.push(host);
    }
    std::fs::write(jail.join("bin/rm"), "#!/bin/sh\necho FAKEBIN-rm\nexit 42\n").ok()?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(jail.join("bin/rm"), std::fs::Permissions::from_mode(0o755)).ok()?;
    let mut libs = BTreeSet::new();
    for bin in &ldd_inputs {
        let out = Command::new("ldd").arg(bin).output().ok()?;
        if !out.status.success() {
            return None;
        }
        for token in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            if token.starts_with('/') {
                libs.insert(PathBuf::from(token));
            }
        }
    }
    if libs.is_empty() {
        return None;
    }
    for lib in &libs {
        copy_into_jail(&jail, lib)?;
    }
    std::fs::write(jail.join("JAIL-CANARY"), b"jail\n").ok()?;
    std::fs::write(jail.join("KEEPFILE"), b"keep\n").ok()?;
    std::fs::write(jail.join("sub/MARK"), b"mark\n").ok()?;
    std::fs::write(jail.join("dev/null"), b"").ok()?;
    std::os::unix::fs::symlink("/", jail.join("rootlink")).ok()?;
    std::os::unix::fs::symlink("/sub", jail.join("tmplink")).ok()?;
    // Smoke: the shell must answer from inside the jail before any
    // pin trusts it with a root operand.
    let smoke = run_jail(&jail, "echo JAIL-OK")?;
    if smoke.stdout != "JAIL-OK\n" {
        return None;
    }
    Some(jail)
}

#[cfg(unix)]
fn run_jail(jail: &Path, script: &str) -> Option<Output> {
    let chroot = chroot_bin()?;
    let child = Command::new(chroot)
        .arg(jail)
        .arg("/bin/rubash")
        .args(["-c", script])
        .env("PATH", "/bin")
        .env("HOME", "/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    Some(collect(child))
}

#[cfg(unix)]
fn jail_pin(tag: &str) -> Option<PathBuf> {
    let base = scratch(tag);
    match build_jail(&base) {
        Some(jail) => Some(jail),
        None => {
            let _ = std::fs::remove_dir_all(&base);
            None
        }
    }
}

// --------------------------------------------------------------------
// The failsafe itself (chroot jail).
// --------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn issue1847_recursive_root_operand_is_refused() {
    let Some(jail) = jail_pin("root") else { return };
    let out = run_jail(
        &jail,
        &format!("{CANARY}rm -rf / 2>err.txt; echo rc=$?; cat err.txt"),
    )
    .expect("jail run");
    let base = jail.parent().expect("jail parent").to_path_buf();
    let kept = jail.join("KEEPFILE").exists() && jail.join("sub/MARK").exists();
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(
        out.stdout,
        "rc=1\nrm: it is dangerous to operate recursively on '/'\nrm: use --no-preserve-root to override this failsafe\n",
        "stderr={:?}",
        out.stderr
    );
    assert!(!out.stdout.contains("FAKEBIN"), "the emulation must answer");
    assert!(
        !out.stderr.contains("dangerous"),
        "the diagnostic belongs to the command's 2>, stderr={:?}",
        out.stderr
    );
    assert!(kept, "nothing under the root may be removed");
}

#[cfg(unix)]
#[test]
fn issue1847_double_slash_root_carries_the_same_as_suffix() {
    let Some(jail) = jail_pin("doubleslash") else {
        return;
    };
    let out = run_jail(
        &jail,
        &format!("{CANARY}rm -rf // 2>err.txt; echo rc=$?; cat err.txt"),
    )
    .expect("jail run");
    let base = jail.parent().expect("jail parent").to_path_buf();
    let kept = jail.join("KEEPFILE").exists();
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(
        out.stdout,
        "rc=1\nrm: it is dangerous to operate recursively on '//' (same as '/')\nrm: use --no-preserve-root to override this failsafe\n",
        "stderr={:?}",
        out.stderr
    );
    assert!(kept);
}

#[cfg(unix)]
#[test]
fn issue1847_triple_slash_root_is_reported_as_plain_root() {
    let Some(jail) = jail_pin("tripleslash") else {
        return;
    };
    let out = run_jail(
        &jail,
        &format!("{CANARY}rm -rf /// 2>err.txt; echo rc=$?; cat err.txt"),
    )
    .expect("jail run");
    let base = jail.parent().expect("jail parent").to_path_buf();
    let kept = jail.join("KEEPFILE").exists();
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(
        out.stdout,
        "rc=1\nrm: it is dangerous to operate recursively on '/'\nrm: use --no-preserve-root to override this failsafe\n",
        "stderr={:?}",
        out.stderr
    );
    assert!(kept);
}

#[cfg(unix)]
#[test]
fn issue1847_slashed_symlink_to_root_is_refused() {
    let Some(jail) = jail_pin("rootlink-slash") else {
        return;
    };
    let out = run_jail(
        &jail,
        &format!("{CANARY}rm -rf rootlink/ 2>err.txt; echo rc=$?; cat err.txt"),
    )
    .expect("jail run");
    let base = jail.parent().expect("jail parent").to_path_buf();
    let kept = jail.join("KEEPFILE").exists() && jail.join("rootlink").exists();
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(
        out.stdout,
        "rc=1\nrm: it is dangerous to operate recursively on 'rootlink/' (same as '/')\nrm: use --no-preserve-root to override this failsafe\n",
        "stderr={:?}",
        out.stderr
    );
    assert!(kept, "neither the root contents nor the link may go");
}

#[cfg(unix)]
#[test]
fn issue1847_bare_symlink_to_root_removes_only_the_link() {
    let Some(jail) = jail_pin("rootlink-bare") else {
        return;
    };
    let out = run_jail(&jail, &format!("{CANARY}rm -rf rootlink; echo rc=$?")).expect("jail run");
    let base = jail.parent().expect("jail parent").to_path_buf();
    let link_gone = !jail.join("rootlink").exists();
    let kept = jail.join("KEEPFILE").exists() && jail.join("sub/MARK").exists();
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(out.stdout, "rc=0\n", "stderr={:?}", out.stderr);
    assert!(link_gone, "a bare symlink operand unlinks the link");
    assert!(kept, "the link's target is never touched");
}

#[cfg(unix)]
#[test]
fn issue1847_refusal_does_not_stop_the_other_operands() {
    let Some(jail) = jail_pin("mixed") else {
        return;
    };
    let out = run_jail(
        &jail,
        &format!("{CANARY}rm -rf /sub / 2>err.txt; echo rc=$?; cat err.txt"),
    )
    .expect("jail run");
    let base = jail.parent().expect("jail parent").to_path_buf();
    let sub_gone = !jail.join("sub").exists();
    let kept = jail.join("KEEPFILE").exists();
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(
        out.stdout,
        "rc=1\nrm: it is dangerous to operate recursively on '/'\nrm: use --no-preserve-root to override this failsafe\n",
        "stderr={:?}",
        out.stderr
    );
    assert!(sub_gone, "the ordinary operand is still processed");
    assert!(kept);
}

#[cfg(unix)]
#[test]
fn issue1847_preserve_root_flag_forms() {
    let Some(jail) = jail_pin("flag-forms") else {
        return;
    };
    // `--preserve-root=all` is the one accepted argument; the bare
    // flag is the default anyway. Both refuse.
    for script in [
        "rm --preserve-root=all -rf / 2>err.txt; echo rc=$?; cat err.txt",
        "rm --preserve-root -rf / 2>err.txt; echo rc=$?; cat err.txt",
    ] {
        let out = run_jail(&jail, &format!("{CANARY}{script}")).expect("jail run");
        assert_eq!(
            out.stdout,
            "rc=1\nrm: it is dangerous to operate recursively on '/'\nrm: use --no-preserve-root to override this failsafe\n",
            "{script}: stderr={:?}",
            out.stderr
        );
    }
    // Any other attached value is its own one-line diagnostic.
    let out = run_jail(
        &jail,
        &format!("{CANARY}rm --preserve-root=bogus -rf / 2>err.txt; echo rc=$?; cat err.txt"),
    )
    .expect("jail run");
    assert_eq!(
        out.stdout, "rc=1\nrm: unrecognized --preserve-root argument: 'bogus'\n",
        "stderr={:?}",
        out.stderr
    );
    // --no-preserve-root takes no argument at all.
    let out = run_jail(
        &jail,
        &format!("{CANARY}rm --no-preserve-root=x -rf /sub 2>err.txt; echo rc=$?; cat err.txt"),
    )
    .expect("jail run");
    let base = jail.parent().expect("jail parent").to_path_buf();
    let kept = jail.join("KEEPFILE").exists() && jail.join("sub/MARK").exists();
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(
        out.stdout,
        "rc=1\nrm: option '--no-preserve-root' doesn't allow an argument\nTry 'rm --help' for more information.\n",
        "stderr={:?}",
        out.stderr
    );
    assert!(kept);
}

#[cfg(unix)]
#[test]
fn issue1847_no_preserve_root_lifts_the_failsafe() {
    let Some(jail) = jail_pin("lifted") else {
        return;
    };
    let out = run_jail(
        &jail,
        &format!("{CANARY}rm -rf --no-preserve-root /sub; echo rc=$?"),
    )
    .expect("jail run");
    let sub_gone = !jail.join("sub").exists();
    assert_eq!(out.stdout, "rc=0\n", "stderr={:?}", out.stderr);
    assert!(sub_gone);
    // On the root itself the removal now proceeds (the jail is
    // disposable): everything goes, and the root directory's own
    // removal reports the busy error, exactly like the oracle.
    let out = run_jail(
        &jail,
        &format!("{CANARY}rm -rf --no-preserve-root /; echo rc=$?"),
    )
    .expect("jail run");
    let base = jail.parent().expect("jail parent").to_path_buf();
    let wiped = !jail.join("KEEPFILE").exists() && !jail.join("sub").exists();
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(out.stdout, "rc=1\n", "stderr={:?}", out.stderr);
    assert!(
        out.stderr
            .contains("rm: cannot remove '/': Device or resource busy"),
        "stderr={:?}",
        out.stderr
    );
    assert!(wiped, "--no-preserve-root must actually lift the guard");
}

// --------------------------------------------------------------------
// Host-safe controls: non-recursive root operands keep their plain
// errors (the failsafe is recursive-only), and directory symlinks
// unlink like GNU says.
// --------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn issue1847_nonrecursive_root_operands_keep_their_plain_errors() {
    let dir = scratch("root-controls");
    let out = run_in_dir(&dir, "rm / 2>err.txt; echo rc=$?; cat err.txt", SYSTEM_PATH);
    assert_eq!(
        out.stdout, "rc=1\nrm: cannot remove '/': Is a directory\n",
        "stderr={:?}",
        out.stderr
    );
    // gnulib remove.c judges a -d failure by the directory's
    // contents; a bare rmdir on the root reports EBUSY instead.
    let out = run_in_dir(
        &dir,
        "rm -d / 2>err.txt; echo rc=$?; cat err.txt",
        SYSTEM_PATH,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout, "rc=1\nrm: cannot remove '/': Directory not empty\n",
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.stderr, "");
}

#[cfg(unix)]
#[test]
fn issue1847_directory_symlink_operands_unlink_the_link() {
    let dir = scratch("dirlink");
    std::fs::create_dir_all(dir.join("realdir")).expect("fixture dir");
    std::fs::write(dir.join("realdir/inner"), b"i").expect("fixture inner");
    std::os::unix::fs::symlink("realdir", dir.join("dirlink")).expect("fixture link");
    std::os::unix::fs::symlink("realdir", dir.join("dirlink2")).expect("fixture link 2");
    // Pre-fix the symlink branch called remove_dir on the link path
    // (ENOTDIR on unix), so -rf silently kept the link and the bare
    // form reported `Not a directory`. GNU unlinks the link itself.
    let out = run_in_dir(&dir, "rm -rf dirlink; echo rc=$?", SYSTEM_PATH);
    assert_eq!(out.stdout, "rc=0\n", "stderr={:?}", out.stderr);
    assert!(!dir.join("dirlink").exists(), "the link must be gone");
    let out = run_in_dir(&dir, "rm dirlink2; echo rc=$?", SYSTEM_PATH);
    let link2_gone = !dir.join("dirlink2").exists();
    let inner_kept = dir.join("realdir/inner").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.stdout, "rc=0\n", "stderr={:?}", out.stderr);
    assert!(link2_gone);
    assert!(inner_kept, "the link's target is never touched");
}
