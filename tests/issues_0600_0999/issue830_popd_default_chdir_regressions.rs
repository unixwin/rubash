//! Issue rubash#830 regression: popd's default (top-of-stack) pop must
//! carry the process cwd along, not just the stack and PWD.
//!
//! GNU spec — vendored bash builtins/pushd.def read against WSL GNU bash
//! 5.3.0(1) probes, 2026-10-13:
//!
//! ```text
//! $ pushd -n ./sub; popd; pwd -P; /bin/pwd     # both print <tmp>/sub
//! $ pushd -n ./sub; rmdir sub; popd; echo $?   # 1; "popd: ./sub: No such
//!                                              #  file or directory";
//!                                              #  dirs still shows ./sub
//! $ pushd -n ./sub; popd -n +0; pwd -P         # unchanged: -n never cds
//! $ (2-entry stack) popd -1; pwd -P            # <stack[1]>: -1 == offset
//!                                              #  is the top branch
//! $ pushd sub; popd; echo $OLDPWD              # the pre-popd $PWD
//! $ popd +0          (1-entry stack)           # rc 1, "directory stack
//!                                              #  empty" (pushd_error)
//! ```
//!
//! The symptom was a builtin/real split: rubash's `pwd` and $PWD claimed
//! ./sub while `pwd -P` and every external child stayed in the original
//! directory. Each scenario below therefore checks both witnesses — the
//! `pwd -P` builtin AND an external `cmd /c cd` in the same session.

#![cfg(windows)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    /// Unique temp root with a `sub` directory inside; the shell runs with
    /// this root as its process cwd.
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "rubash-830-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create sandbox root");
        fs::create_dir_all(root.join("sub")).expect("create sandbox sub");
        Self { root }
    }

    fn run(&self, script: &str) -> (String, String, Option<i32>) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_remove("BASH_ENV")
            .env_remove("WINUXSH_ROOT")
            .spawn()
            .expect("spawn rubash");
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(script.as_bytes())
            .expect("write script to stdin");
        let output = child.wait_with_output().expect("wait for rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }

    fn sub_canonical(&self) -> PathBuf {
        self.root
            .join("sub")
            .canonicalize()
            .expect("canonicalize sub")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn lines(stdout: &str) -> Vec<&str> {
    stdout.lines().map(str::trim_end).collect()
}

/// The ticket scenario: `pushd -n ./sub; popd` — the builtin `pwd -P` and
/// the external `cmd /c cd` must both report the sub directory afterwards.
#[test]
fn default_popd_moves_process_cwd() {
    let sandbox = Sandbox::new("move");
    let (stdout, stderr, code) =
        sandbox.run("pwd -P\npushd -n ./sub >/dev/null\npopd >/dev/null\npwd -P\ncmd /c cd\n");
    assert_eq!(code, Some(0), "rc; stderr: {stderr:?}");
    let out = lines(&stdout);
    assert_eq!(out.len(), 3, "pwd -P, pwd -P, cmd /c cd: {stdout:?}");

    // The physical pwd moved off the original directory...
    assert_ne!(out[0], out[1], "pwd -P must change across popd");
    assert!(out[1].ends_with("/sub"), "pwd -P after popd: {:?}", out[1]);

    // ...and the external child agrees (the ticket's real split).
    let external = Path::new(out[2]).canonicalize().expect("cmd /c cd path");
    assert_eq!(external, sandbox.sub_canonical());
}

/// Command-substitution form of the same witness: `$(cmd /c cd)` runs the
/// external in the post-popd process cwd.
#[test]
fn command_substitution_sees_post_popd_cwd() {
    let sandbox = Sandbox::new("comsub");
    let (stdout, stderr, code) =
        sandbox.run("pushd -n ./sub >/dev/null\npopd >/dev/null\necho [$(cmd /c cd)]\n");
    assert_eq!(code, Some(0), "rc; stderr: {stderr:?}");
    let out = lines(&stdout);
    assert_eq!(out.len(), 1, "{stdout:?}");
    let external = Path::new(out[0].trim_start_matches('[').trim_end_matches(']'))
        .canonicalize()
        .expect("comsub cwd path");
    assert_eq!(external, sandbox.sub_canonical());
}

/// pushd.def:394-403: cd_to_string runs BEFORE the element is freed, so a
/// failed chdir leaves the stack, PWD and the display untouched — only the
/// cd_builtin-shaped diagnostic and EXECUTION_FAILURE come out.
#[test]
fn popd_chdir_failure_keeps_stack_and_cwd() {
    let sandbox = Sandbox::new("fail");
    let (stdout, stderr, code) =
        sandbox.run("pushd -n ./sub >/dev/null\nrmdir sub\npopd\necho rc=$?\ndirs\npwd -P\n");
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(
        stderr.contains("popd: ./sub: No such file or directory"),
        "diagnostic: {stderr:?}"
    );
    let out = lines(&stdout);
    assert_eq!(out.len(), 3, "rc, dirs, pwd -P: {stdout:?}");
    // popd itself printed no stack (GNU returns before dirs_builtin).
    assert_eq!(out[0], "rc=1");
    // The stack was NOT popped: still `<cwd> ./sub`.
    assert_eq!(out[1].split_whitespace().count(), 2, "dirs: {:?}", out[1]);
    assert!(out[1].ends_with("./sub"), "dirs: {:?}", out[1]);
    // The cwd never moved.
    assert!(!out[2].ends_with("/sub"), "pwd -P: {:?}", out[2]);
}

/// `popd -n` skips cd_to_string entirely (pushd.def:397): stack shrinks,
/// nothing else moves. Covers both the bare top form and the `+0` spelling.
#[test]
fn popd_no_cd_never_moves_the_process() {
    for operand in ["-n", "-n +0"] {
        let sandbox = Sandbox::new("nocd");
        let (stdout, stderr, code) = sandbox.run(&format!(
            "pwd -P\npushd -n ./sub >/dev/null\npopd {operand} >/dev/null\npwd -P\ndirs\n"
        ));
        assert_eq!(code, Some(0), "rc for popd {operand}; stderr: {stderr:?}");
        let out = lines(&stdout);
        assert_eq!(out.len(), 3, "popd {operand}: {stdout:?}");
        assert_eq!(out[0], out[1], "popd {operand} must not move the cwd");
        // The pushed word is gone from the display; only the cwd remains.
        assert_eq!(
            out[2].split_whitespace().count(),
            1,
            "dirs after popd {operand}: {:?}",
            out[2]
        );
    }
}

/// GNU R1 probe: on a two-entry display, `popd -1` names the bottom slot,
/// which IS the top branch (`which == directory_list_offset`) — it cds.
#[test]
fn popd_minus_one_on_two_entries_is_the_top_branch() {
    let sandbox = Sandbox::new("minus1");
    let (stdout, stderr, code) =
        sandbox.run("pushd -n ./sub >/dev/null\npopd -1 >/dev/null\npwd -P\n");
    assert_eq!(code, Some(0), "rc; stderr: {stderr:?}");
    let out = lines(&stdout);
    assert_eq!(out.len(), 1, "{stdout:?}");
    assert!(
        out[0].ends_with("/sub"),
        "pwd -P after popd -1: {:?}",
        out[0]
    );
}

/// Non-top pops (pushd.def:404-421) only remove the element: no chdir, and
/// $PWD stays exactly as it was.
#[test]
fn non_top_popd_leaves_cwd_and_pwd_alone() {
    let sandbox = Sandbox::new("nontop");
    let (stdout, stderr, code) = sandbox.run(
        "pwd -P\npushd -n ./sub >/dev/null\npushd -n ./sub >/dev/null\npopd +1 >/dev/null\npwd -P\necho $PWD\ndirs\n",
    );
    assert_eq!(code, Some(0), "rc; stderr: {stderr:?}");
    let out = lines(&stdout);
    assert_eq!(out.len(), 4, "{stdout:?}");
    assert_eq!(out[0], out[1], "popd +1 must not move the cwd");
    assert_eq!(out[1], out[2], "popd +1 must not rewrite $PWD");
    assert_eq!(out[3].split_whitespace().count(), 2, "dirs: {:?}", out[3]);
}

/// cd_builtin's bindpwd runs under cd_to_string, so the default pop refreshes
/// OLDPWD to the pre-popd PWD (GNU probe 5: `pushd sub; popd`).
#[test]
fn default_popd_refreshes_oldpwd() {
    let sandbox = Sandbox::new("oldpwd");
    let (stdout, stderr, code) = sandbox
        .run("pre=$PWD\npushd -n ./sub >/dev/null\npopd >/dev/null\necho [$OLDPWD]\necho [$pre]\n");
    assert_eq!(code, Some(0), "rc; stderr: {stderr:?}");
    let out = lines(&stdout);
    assert_eq!(out.len(), 2, "{stdout:?}");
    assert_eq!(out[0], out[1], "OLDPWD must equal the pre-popd PWD");
}

/// dirs -c (CLEARSTAK, pushd.def:481-485) only clears the array: no chdir,
/// no PWD write, and the next display falls back to the working directory —
/// the same directory popd's chdir left the process in.
#[test]
fn dirs_c_clears_without_touching_the_cwd() {
    let sandbox = Sandbox::new("dirsc");
    let (stdout, stderr, code) =
        sandbox.run("pushd -n ./sub >/dev/null\npopd >/dev/null\npwd -P\ndirs -c\npwd -P\ndirs\n");
    assert_eq!(code, Some(0), "rc; stderr: {stderr:?}");
    let out = lines(&stdout);
    assert_eq!(out.len(), 3, "{stdout:?}");
    assert!(
        out[0].ends_with("/sub"),
        "pwd -P before dirs -c: {:?}",
        out[0]
    );
    assert_eq!(out[1], out[0], "dirs -c must not move the cwd");
    assert_eq!(
        out[2].split_whitespace().count(),
        1,
        "dirs after -c falls back to the cwd: {:?}",
        out[2]
    );
}

/// Range verdicts on a spent stack: pushd_error (pushd.def:540-546) says
/// "directory stack empty" while the offset is 0 — for `popd`, `popd +0` and
/// `popd +1` alike — and nothing is removed.
#[test]
fn empty_stack_range_verdicts() {
    for operand in ["", "+0", "+1"] {
        let sandbox = Sandbox::new("empty");
        let script = format!("popd {operand}\necho rc=$?\ndirs\n");
        let (stdout, stderr, code) = sandbox.run(&script);
        assert_eq!(code, Some(0), "script rc for popd {operand:?}");
        assert!(
            stderr.contains("popd: directory stack empty"),
            "diagnostic for popd {operand:?}: {stderr:?}"
        );
        let out = lines(&stdout);
        assert_eq!(out.len(), 2, "popd {operand:?}: {stdout:?}");
        assert_eq!(out[0], "rc=1", "popd {operand:?}");
        // Exactly the cwd entry survives, and popd printed no stack.
        assert_eq!(out[1].split_whitespace().count(), 1, "dirs: {:?}", out[1]);
    }
}
