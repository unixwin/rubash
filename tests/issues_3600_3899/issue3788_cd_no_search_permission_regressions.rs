//! rubash#3788 — `cd`/`pushd` into a directory without the search
//! (execute) bit report success (rc 0, PWD/DIRSTACK updated) although
//! the directory change never happens; GNU refuses with rc 1
//! `Permission denied` and leaves the cwd unchanged.
//!
//! GNU keeps the move and the verdict together:
//! builtins/cd.def change_to_directory performs a real `chdir(2)`
//! (cd.def:600/641) and only then updates
//! the_current_working_directory (cd.def:605-608); the failure tail
//! reports the chdir errno (cd.def:427). The kernel refuses a target
//! without the search bit with EACCES, while realpath(3) and stat(2)
//! both succeed on such a directory — path resolution only needs
//! search permission on the PARENT components. The session-cwd moves
//! (G-02) never chdir the process, and their probe was canonicalize
//! + is_dir, so the move committed without any check of the target's
//! own search bit.
//!
//! Boundary is exactly the search bit: modes 000/222/444 fail on GNU,
//! modes 100/111/555 succeed on both — as seen by the OWNER class
//! when the shell runs as the directory owner, and by the OTHER class
//! (101/111/555 succeed, 100 fails) when it runs as `nobody`. The
//! probes therefore pick their success modes from the class that
//! applies: the test runs the shell as the test-process user when
//! that user is not root, and as `nobody` via setpriv when the test
//! process is root (root would bypass the permission bits entirely,
//! so a root-run probe is a false green and is never used).
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-11-16). Unix-only: the shapes spell POSIX paths and unix
//! permission bits.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

/// How the shell under test is launched.
struct Ctx {
    /// The shell runs as the test-process user (which owns the
    /// fixture dirs, so the OWNER permission class applies and the
    /// script itself may chmod them).
    direct: bool,
    /// Prefix argv for the shell invocation: empty when direct, the
    /// setpriv form when the test process is root.
    prefix: Vec<String>,
}

fn current_uid() -> Option<u32> {
    let out = Command::new("id").arg("-u").output().ok()?;
    String::from_utf8(out.stdout)
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
}

fn command_ok(prog: &str, args: &[&str]) -> bool {
    Command::new(prog)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Returns None when the permission probes cannot run as an
/// unprivileged user here: root without setpriv/`nobody` must skip,
/// never probe as root (root bypasses the bits).
fn ctx() -> Option<Ctx> {
    match current_uid() {
        Some(0) => {
            if command_ok("setpriv", &["--version"]) && command_ok("id", &["-u", "nobody"]) {
                Some(Ctx {
                    direct: false,
                    prefix: vec![
                        "setpriv".to_string(),
                        "--reuid=nobody".to_string(),
                        "--regid=nogroup".to_string(),
                        "--clear-groups".to_string(),
                    ],
                })
            } else {
                eprintln!("SKIP: root without setpriv/nobody; permission probes need an unprivileged user");
                None
            }
        }
        Some(_) => Some(Ctx {
            direct: true,
            prefix: Vec::new(),
        }),
        None => {
            eprintln!("SKIP: cannot determine uid");
            None
        }
    }
}

/// Fixture tree under a fresh temp dir (parents chmod 755: the
/// ambient umask may be 007, which would hide the tree from `nobody`).
struct Tree {
    base: PathBuf,
    /// Canonical spelling of the work dir (what `pwd` reports).
    work: String,
    /// The success-mode dir name for the applicable class: `d100`
    /// when the shell runs as the owner, `d101` as `nobody`.
    ok_mode: &'static str,
}

fn make_tree(tag: &str, ctx: &Ctx) -> Tree {
    let base = std::env::temp_dir().join(format!("rubash3788-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let work = base.join("work");
    std::fs::create_dir_all(&work).expect("create work");
    for (name, mode) in [
        ("d000", 0o000),
        ("d222", 0o222),
        ("d444", 0o444),
        ("d100", 0o100),
        ("d101", 0o101),
        ("d111", 0o111),
        ("d555", 0o555),
        ("real", 0o000),
    ] {
        let dir = work.join(name);
        std::fs::create_dir(&dir).expect("create dir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).expect("chmod dir");
    }
    // Nested shape: a searchable parent holding an unsearchable leaf,
    // and a searchable nested pair for the success control.
    std::fs::create_dir_all(work.join("sub/inner")).expect("create sub/inner");
    std::fs::set_permissions(work.join("sub"), std::fs::Permissions::from_mode(0o755))
        .expect("chmod sub");
    std::fs::set_permissions(
        work.join("sub/inner"),
        std::fs::Permissions::from_mode(0o000),
    )
    .expect("chmod sub/inner");
    std::fs::create_dir_all(work.join("sub2/inner2")).expect("create sub2/inner2");
    std::fs::set_permissions(work.join("sub2"), std::fs::Permissions::from_mode(0o755))
        .expect("chmod sub2");
    std::fs::set_permissions(
        work.join("sub2/inner2"),
        std::fs::Permissions::from_mode(0o555),
    )
    .expect("chmod sub2/inner2");
    // CDPATH shapes: an unsearchable first unit and a searchable
    // second unit holding the same name.
    std::fs::create_dir_all(work.join("else1/d000")).expect("create else1/d000");
    std::fs::set_permissions(
        work.join("else1/d000"),
        std::fs::Permissions::from_mode(0o000),
    )
    .expect("chmod else1/d000");
    std::fs::create_dir_all(work.join("else2/d000")).expect("create else2/d000");
    std::fs::set_permissions(
        work.join("else2/d000"),
        std::fs::Permissions::from_mode(0o555),
    )
    .expect("chmod else2/d000");
    std::os::unix::fs::symlink("real", work.join("link")).expect("create link");
    std::fs::write(work.join("afile"), "x").expect("write afile");
    for dir in [&base, &work] {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))
            .expect("chmod parent");
    }
    let work_str = std::fs::canonicalize(&work)
        .expect("canonicalize work")
        .to_string_lossy()
        .into_owned();
    Tree {
        base,
        work: work_str,
        ok_mode: if ctx.direct { "d100" } else { "d101" },
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        // Re-open the tree for removal: the 000 dirs cannot be
        // listed while the bits are off.
        let _ = Command::new("chmod")
            .arg("-R")
            .arg("755")
            .arg(&self.base)
            .status();
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn run_script(ctx: &Ctx, tree: &Tree, script: &str) -> (String, String, Option<i32>) {
    let case = tree.base.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    std::fs::set_permissions(&case, std::fs::Permissions::from_mode(0o644)).expect("chmod fixture");
    let shell = env!("CARGO_BIN_EXE_rubash");
    let mut cmd = Command::new(
        &ctx.prefix
            .first()
            .cloned()
            .unwrap_or_else(|| shell.to_string()),
    );
    if !ctx.prefix.is_empty() {
        cmd.args(&ctx.prefix[1..]).arg(shell);
    }
    let output = cmd
        .current_dir(&tree.work)
        .arg(&case)
        .env("HOME", &tree.work)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&case);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// A failed move leaves everything alone: rc 1, the diagnostic, the
/// cwd and $PWD still the work dir.
fn assert_refused(
    stdout: &str,
    stderr: &str,
    code: Option<i32>,
    work: &str,
    builtin: &str,
    operand: &str,
) {
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("rc=1"), "stdout was: {stdout:?}");
    assert_eq!(lines.next(), Some(work), "stdout was: {stdout:?}");
    assert_eq!(
        lines.next(),
        Some(format!("PWD={work}").as_str()),
        "stdout was: {stdout:?}"
    );
    assert_eq!(code, Some(0), "script rc was: {code:?}");
    assert!(
        stderr.contains("Permission denied"),
        "stderr was: {stderr:?}"
    );
    assert!(
        stderr.contains(&format!("{builtin}: {operand}")),
        "stderr was: {stderr:?}"
    );
}

#[test]
fn issue3788_cd_no_search_permission_refused() {
    let Some(ctx) = ctx() else { return };
    let tree = make_tree("cd", &ctx);
    let work = tree.work.clone();
    let ok = tree.ok_mode;

    // Relative, absolute, nested and symlink shapes, both modes.
    for (operand, label) in [
        ("d000", "cd-000"),
        ("d444", "cd-444"),
        ("d222", "cd-222"),
        ("sub/inner", "cd-nested-000"),
        ("link", "cd-symlink-000"),
    ] {
        let (stdout, stderr, code) = run_script(
            &ctx,
            &tree,
            &format!("cd {operand}; echo \"rc=$?\"; pwd; echo \"PWD=$PWD\"\n"),
        );
        assert_refused(&stdout, &stderr, code, &work, "cd", operand);
        let _ = label;
    }
    let (stdout, stderr, code) = run_script(
        &ctx,
        &tree,
        &format!("cd \"$PWD/d000\"; echo \"rc=$?\"; pwd; echo \"PWD=$PWD\"\n"),
    );
    assert_refused(&stdout, &stderr, code, &work, "cd", &format!("{work}/d000"));
    let (stdout, stderr, code) = run_script(
        &ctx,
        &tree,
        &format!("cd -P d000; echo \"rc=$?\"; pwd; echo \"PWD=$PWD\"\n"),
    );
    assert_refused(&stdout, &stderr, code, &work, "cd", "d000");

    // Success controls: the boundary mode, 111, 555, nested.
    for (operand, label) in [
        (ok, "cd-ok-mode"),
        ("d111", "cd-111"),
        ("d555", "cd-555"),
        ("sub2/inner2", "cd-nested-555"),
    ] {
        let (stdout, stderr, code) = run_script(
            &ctx,
            &tree,
            &format!("cd {operand}; echo \"rc=$?\"; pwd; echo \"PWD=$PWD\"\n"),
        );
        let expect = format!("{work}/{operand}");
        let mut lines = stdout.lines();
        assert_eq!(
            lines.next(),
            Some("rc=0"),
            "{label}: stdout was: {stdout:?}"
        );
        assert_eq!(
            lines.next(),
            Some(expect.as_str()),
            "{label}: stdout was: {stdout:?}"
        );
        assert_eq!(
            lines.next(),
            Some(format!("PWD={expect}").as_str()),
            "{label}: stdout was: {stdout:?}"
        );
        assert_eq!(code, Some(0), "{label}: script rc was: {code:?}");
        assert!(stderr.is_empty(), "{label}: stderr was: {stderr:?}");
    }

    // Adjacent failures stay failures: missing dir, plain file.
    for (operand, want) in [
        ("/no/such/dir", "No such file"),
        ("afile", "Not a directory"),
    ] {
        let (stdout, stderr, code) = run_script(
            &ctx,
            &tree,
            &format!("cd {operand}; echo \"rc=$?\"; pwd; echo \"PWD=$PWD\"\n"),
        );
        let mut lines = stdout.lines();
        assert_eq!(lines.next(), Some("rc=1"), "stdout was: {stdout:?}");
        assert_eq!(lines.next(), Some(work.as_str()), "stdout was: {stdout:?}");
        assert_eq!(code, Some(0), "script rc was: {code:?}");
        assert!(stderr.contains(want), "stderr was: {stderr:?}");
    }
}

#[test]
fn issue3788_pushd_no_search_permission_refused() {
    let Some(ctx) = ctx() else { return };
    let tree = make_tree("pushd", &ctx);
    let work = tree.work.clone();
    let ok = tree.ok_mode;

    for (operand, label) in [
        ("d000", "pushd-000"),
        ("d444", "pushd-444"),
        ("d222", "pushd-222"),
        ("link", "pushd-symlink-000"),
    ] {
        let (stdout, stderr, code) = run_script(
            &ctx,
            &tree,
            &format!("pushd {operand} >/dev/null; echo \"rc=$?\"; dirs; echo \"PWD=$PWD\"\n"),
        );
        // pushd prints no dirs listing on failure: the first line is
        // the rc, the second the unchanged stack (just the work dir,
        // which dirs abbreviates to `~` because HOME is the work dir
        // here — GNU-verified; pwd/$PWD stay absolute).
        let mut lines = stdout.lines();
        assert_eq!(
            lines.next(),
            Some("rc=1"),
            "{label}: stdout was: {stdout:?}"
        );
        assert_eq!(lines.next(), Some("~"), "{label}: stdout was: {stdout:?}");
        assert_eq!(
            lines.next(),
            Some(format!("PWD={work}").as_str()),
            "{label}: stdout was: {stdout:?}"
        );
        assert_eq!(code, Some(0), "{label}: script rc was: {code:?}");
        assert!(
            stderr.contains("Permission denied"),
            "{label}: stderr was: {stderr:?}"
        );
        assert!(
            stderr.contains(&format!("pushd: {operand}")),
            "{label}: stderr was: {stderr:?}"
        );
    }
    let (stdout, stderr, code) = run_script(
        &ctx,
        &tree,
        &format!("pushd \"$PWD/d444\" >/dev/null; echo \"rc=$?\"; dirs; echo \"PWD=$PWD\"\n"),
    );
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("rc=1"), "stdout was: {stdout:?}");
    assert_eq!(lines.next(), Some("~"), "stdout was: {stdout:?}");
    assert_eq!(code, Some(0), "script rc was: {code:?}");
    assert!(
        stderr.contains("Permission denied"),
        "stderr was: {stderr:?}"
    );

    // Success controls.
    for (operand, label) in [(ok, "pushd-ok-mode"), ("d555", "pushd-555")] {
        let (stdout, stderr, code) = run_script(
            &ctx,
            &tree,
            &format!(
                "pushd {operand} >/dev/null; echo \"rc=$?\"; popd >/dev/null; echo \"PWD=$PWD\"\n"
            ),
        );
        let mut lines = stdout.lines();
        assert_eq!(
            lines.next(),
            Some("rc=0"),
            "{label}: stdout was: {stdout:?}"
        );
        assert_eq!(
            lines.next(),
            Some(format!("PWD={work}").as_str()),
            "{label}: stdout was: {stdout:?}"
        );
        assert_eq!(code, Some(0), "{label}: script rc was: {code:?}");
        assert!(stderr.is_empty(), "{label}: stderr was: {stderr:?}");
    }
}

#[test]
fn issue3788_cdpath_skips_unsearchable_candidate() {
    let Some(ctx) = ctx() else { return };
    let tree = make_tree("cdpath", &ctx);
    let work = tree.work.clone();

    // GNU tries each CDPATH unit's change_to_directory in order: the
    // unsearchable first unit fails, the searchable second unit lands.
    let (stdout, stderr, code) = run_script(
        &ctx,
        &tree,
        &format!(
            "CDPATH=\"$PWD/else1:$PWD/else2\"; cd d000; echo \"rc=$?\"; pwd; echo \"PWD=$PWD\"\n"
        ),
    );
    let expect = format!("{work}/else2/d000");
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some(expect.as_str()),
        "stdout was: {stdout:?}"
    );
    assert_eq!(lines.next(), Some("rc=0"), "stdout was: {stdout:?}");
    assert_eq!(
        lines.next(),
        Some(expect.as_str()),
        "stdout was: {stdout:?}"
    );
    assert_eq!(
        lines.next(),
        Some(format!("PWD={expect}").as_str()),
        "stdout was: {stdout:?}"
    );
    assert_eq!(code, Some(0), "script rc was: {code:?}");
    assert!(stderr.is_empty(), "stderr was: {stderr:?}");
}

#[test]
fn issue3788_cdable_vars_tried_after_unsearchable_operand() {
    let Some(ctx) = ctx() else { return };
    let tree = make_tree("cdable", &ctx);
    let work = tree.work.clone();

    // GNU tries the cdable_vars value only after the plain operand's
    // change_to_directory failed (cd.def LCD_DOVARS): the unsearchable
    // `d000` must not block the variable step.
    let (stdout, stderr, code) = run_script(
        &ctx,
        &tree,
        &format!(
            "shopt -s cdable_vars; d000=\"$PWD/d555\"; cd d000; echo \"rc=$?\"; pwd; echo \"PWD=$PWD\"\n"
        ),
    );
    let expect = format!("{work}/d555");
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some(expect.as_str()),
        "stdout was: {stdout:?}"
    );
    assert_eq!(lines.next(), Some("rc=0"), "stdout was: {stdout:?}");
    assert_eq!(
        lines.next(),
        Some(expect.as_str()),
        "stdout was: {stdout:?}"
    );
    assert_eq!(
        lines.next(),
        Some(format!("PWD={expect}").as_str()),
        "stdout was: {stdout:?}"
    );
    assert_eq!(code, Some(0), "script rc was: {code:?}");
    assert!(stderr.is_empty(), "stderr was: {stderr:?}");
}

#[test]
fn issue3788_comsub_cd_pwd_fast_path_refused() {
    let Some(ctx) = ctx() else { return };
    let tree = make_tree("comsub", &ctx);
    let work = tree.work.clone();

    // The `$(cd X && pwd)` fast path computes the destination without
    // running the builtin: a refusal there must carry the same
    // verdict (rc 1, `Permission denied`, empty capture).
    let (stdout, stderr, code) = run_script(
        &ctx,
        &tree,
        "x=$(cd d000 && pwd); echo \"rc=$?\"; echo \"x=[$x]\"; pwd; echo \"PWD=$PWD\"\n",
    );
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("rc=1"), "stdout was: {stdout:?}");
    assert_eq!(lines.next(), Some("x=[]"), "stdout was: {stdout:?}");
    assert_eq!(lines.next(), Some(work.as_str()), "stdout was: {stdout:?}");
    assert_eq!(
        lines.next(),
        Some(format!("PWD={work}").as_str()),
        "stdout was: {stdout:?}"
    );
    assert_eq!(code, Some(0), "script rc was: {code:?}");
    assert!(
        stderr.contains("Permission denied"),
        "stderr was: {stderr:?}"
    );
    assert!(stderr.contains("cd: d000"), "stderr was: {stderr:?}");
}

#[test]
fn issue3788_pushd_rotation_onto_unsearchable_entry_refused() {
    let Some(ctx) = ctx() else { return };
    // The rotation target must lose its search bit AFTER being pushed,
    // which the script itself can only do when it runs as the fixture
    // owner — the setpriv/nobody form cannot chmod root-owned dirs.
    if !ctx.direct {
        eprintln!("SKIP: rotation case needs the shell to own the fixture dirs");
        return;
    }
    let tree = make_tree("rotate", &ctx);
    let work = tree.work.clone();

    // Stack after the pushes: [d111, d555, work]; `pushd +1` rotates
    // d555 to the top and chdirs there — GNU refuses once d555 lost
    // its search bit, leaving PWD on d111 and the list rotated.
    // (The chmod runs from inside d111, so the target is ../d555.)
    let (stdout, stderr, code) = run_script(
        &ctx,
        &tree,
        "pushd d555 >/dev/null; pushd ../d111 >/dev/null; chmod 000 ../d555; pushd +1; echo \"rc=$?\"; pwd; echo \"PWD=$PWD\"; dirs\n",
    );
    let d111 = format!("{work}/d111");
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("rc=1"), "stdout was: {stdout:?}");
    assert_eq!(lines.next(), Some(d111.as_str()), "stdout was: {stdout:?}");
    assert_eq!(
        lines.next(),
        Some(format!("PWD={d111}").as_str()),
        "stdout was: {stdout:?}"
    );
    assert_eq!(code, Some(0), "script rc was: {code:?}");
    assert!(
        stderr.contains("Permission denied"),
        "stderr was: {stderr:?}"
    );

    // The bare swap takes the same road: after the pushes, chmod the
    // second entry away and swap onto it.
    let (stdout, stderr, code) = run_script(
        &ctx,
        &tree,
        "chmod 555 d555; pushd d555 >/dev/null; pushd ../d111 >/dev/null; chmod 000 ../d555; pushd; echo \"rc=$?\"; pwd; echo \"PWD=$PWD\"\n",
    );
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("rc=1"), "stdout was: {stdout:?}");
    assert_eq!(lines.next(), Some(d111.as_str()), "stdout was: {stdout:?}");
    assert_eq!(
        lines.next(),
        Some(format!("PWD={d111}").as_str()),
        "stdout was: {stdout:?}"
    );
    assert_eq!(code, Some(0), "script rc was: {code:?}");
    assert!(
        stderr.contains("Permission denied"),
        "stderr was: {stderr:?}"
    );
}
