//! Issues rubash#844 + rubash#848 regressions: the dirstack cd family's
//! readonly binds and the out-of-range operand wording.
//!
//! GNU spec — vendored bash (bash.git @b4608166, 5.3 patch 15) read against
//! WSL GNU bash 5.3.0(1)-release probes, 2026-10-19:
//!
//! ```text
//! $ readonly OLDPWD; popd      # rc 1, "OLDPWD: readonly variable" (one
//!                              #  diagnostic per variable, OLDPWD first —
//!                              #  cd.def:136-175 bindpwd via err_readonly,
//!                              #  error.c:453-456); the chdir STANDS (cwd
//!                              #  moved) but pushd.def:400-403 returns before
//!                              #  the free: the stack keeps both entries and
//!                              #  nothing is displayed.
//! $ readonly PWD; pushd        # same verdict family for every cd-carrying
//! $ readonly PWD; pushd +1     # pushd path — the swap (pushd.def:203-215)
//! $ readonly PWD; pushd ../x   # and rotation (pushd.def:256-286) land in the
//!                              #  list BEFORE change_to_temp, so the swap/
//!                              #  rotation survives the failure while the
//!                              #  display and rc 0 stay behind it; the Dir
//!                              #  branch (pushd.def:296-316) never adds the
//!                              #  element because cd_builtin failed first.
//! $ readonly OLDPWD PWD; popd -n / pushd -n ../x   # NOCD never cds: rc 0,
//!                              #  no diagnostic.
//! $ popd +5 -n    (2-entry)    # "popd: +5: directory stack index out of
//!                              #  range" — the walk records which_word
//!                              #  (pushd.def:374-377) and pushd_error
//!                              #  (pushd.def:540-546, sh_erange common.c:252)
//!                              #  echoes THE OPERAND, never a trailing
//!                              #  option. pushd.def:242 checks pushd's own
//!                              #  operand inline: `pushd +9 -n` says "+9".
//! $ pushd +9      (fresh)      # "pushd: directory stack empty" — pushd_error
//! $ pushd -9      (fresh)      #  names the empty stack while the offset is 0.
//! ```
//!
//! The symptom was twofold: rubash's popd/pushd cd paths silently skipped
//! the readonly binds (rc 0, stack mutated, no diagnostic — cd.rs already
//! mirrored bindpwd since #537), and the range diagnostic took `args.last()`
//! so `popd +5 -n` blamed "-n".

#![cfg(windows)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    /// Unique temp root with `sub` and `sub2` inside; the shell runs with
    /// this root as its process cwd.
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "rubash-844-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("duration")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create sandbox root");
        fs::create_dir_all(root.join("sub")).expect("create sandbox sub");
        fs::create_dir_all(root.join("sub2")).expect("create sandbox sub2");
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

    fn sub2_canonical(&self) -> PathBuf {
        self.root
            .join("sub2")
            .canonicalize()
            .expect("canonicalize sub2")
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

/// Exact per-variable readonly diagnostic check: "OLDPWD: readonly variable"
/// both contains and ends with "PWD: readonly variable", so the match must
/// anchor on the ": " boundary the error prolog leaves before the name.
fn has_var_diagnostic(stderr: &str, var: &str) -> bool {
    stderr
        .lines()
        .any(|line| line.contains(&format!(": {var}: readonly variable")))
}

/// Two-entry stack with real cd faces: cwd=sub2, stack=[sub2, sub],
/// $OLDPWD=sub, $PWD=sub2 — the probe geometry the WSL oracle ran.
fn two_entry_stack() -> String {
    "cd ./sub\npushd ../sub2 >/dev/null\n".to_string()
}

fn ends_with_dir(line: &str, dir: &Path) -> bool {
    Path::new(line)
        .canonicalize()
        .map(|p| p == dir)
        .unwrap_or(false)
        || line
            .replace('\\', "/")
            .ends_with(&format!("/{}", dir.file_name().unwrap().to_string_lossy()))
}

// ---------------------------------------------------------------- #844 ----

/// `readonly OLDPWD; popd`: rc 1 with the bindpwd diagnostic, the chdir
/// stands (pwd -P moved), but the stack keeps both entries and the failed
/// popd displays nothing.
#[test]
fn readonly_oldpwd_popd_moves_cwd_but_keeps_the_stack() {
    let sandbox = Sandbox::new("po-old");
    let script = format!(
        "{}readonly OLDPWD\npopd\necho rc=$?\npwd -P\ndirs -p\n",
        two_entry_stack()
    );
    let (stdout, stderr, code) = sandbox.run(&script);
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(
        has_var_diagnostic(&stderr, "OLDPWD"),
        "diagnostic: {stderr:?}"
    );
    assert!(!has_var_diagnostic(&stderr, "PWD"), "stderr: {stderr:?}");
    let out = lines(&stdout);
    // rc, pwd -P, two dirs -p lines: the failed popd itself printed nothing.
    assert_eq!(out.len(), 4, "stdout: {stdout:?}");
    assert_eq!(out[0], "rc=1");
    assert!(
        ends_with_dir(out[1], &sandbox.sub_canonical()),
        "pwd -P after popd: {:?}",
        out[1]
    );
    // The stack was NOT popped: both entries survive, the cwd slot moved.
    assert!(
        ends_with_dir(out[2], &sandbox.sub_canonical()),
        "dirs[0]: {:?}",
        out[2]
    );
    assert!(
        ends_with_dir(out[3], &sandbox.sub_canonical()),
        "dirs[1]: {:?}",
        out[3]
    );
}

/// `readonly PWD; popd`: the PWD bind fails (variable stays at sub2) while
/// OLDPWD still refreshes to the pre-popd PWD and the cwd moves.
#[test]
fn readonly_pwd_popd_keeps_pwd_var_and_stack() {
    let sandbox = Sandbox::new("po-pwd");
    let script = format!(
        "{}readonly PWD\npopd\necho rc=$?\npwd -P\ndirs -p\necho o=[$OLDPWD]\necho p=[$PWD]\n",
        two_entry_stack()
    );
    let (stdout, stderr, code) = sandbox.run(&script);
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(has_var_diagnostic(&stderr, "PWD"), "diagnostic: {stderr:?}");
    assert!(!has_var_diagnostic(&stderr, "OLDPWD"), "stderr: {stderr:?}");
    let out = lines(&stdout);
    assert_eq!(out.len(), 6, "stdout: {stdout:?}");
    assert_eq!(out[0], "rc=1");
    assert!(
        ends_with_dir(out[1], &sandbox.sub_canonical()),
        "pwd -P: {:?}",
        out[1]
    );
    assert!(
        ends_with_dir(out[2], &sandbox.sub_canonical()),
        "dirs[0]: {:?}",
        out[2]
    );
    assert!(
        ends_with_dir(out[3], &sandbox.sub_canonical()),
        "dirs[1]: {:?}",
        out[3]
    );
    // bindpwd: OLDPWD took the pre-popd $PWD (sub2); the readonly PWD kept it.
    assert!(
        ends_with_dir(
            out[4].trim_start_matches("o=[").trim_end_matches(']'),
            &sandbox.sub2_canonical()
        ),
        "OLDPWD: {:?}",
        out[4]
    );
    assert!(
        ends_with_dir(
            out[5].trim_start_matches("p=[").trim_end_matches(']'),
            &sandbox.sub2_canonical()
        ),
        "PWD stuck: {:?}",
        out[5]
    );
}

/// `readonly OLDPWD; pushd ../sub` (Dir operand): cd_builtin fails before
/// add_dirstack_element (pushd.def:296-316) — no push, no display, rc 1,
/// while the chdir verdict moves the display's cwd slot to the operand.
#[test]
fn readonly_oldpwd_pushd_dir_never_adds_the_element() {
    let sandbox = Sandbox::new("pd-old");
    let script = format!(
        "{}readonly OLDPWD\npushd ../sub\necho rc=$?\ndirs -p\npwd -P\n",
        two_entry_stack()
    );
    let (stdout, stderr, code) = sandbox.run(&script);
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(
        stderr.contains("OLDPWD: readonly variable"),
        "diagnostic: {stderr:?}"
    );
    let out = lines(&stdout);
    assert_eq!(
        out.len(),
        4,
        "the failed pushd printed no stack: {stdout:?}"
    );
    assert_eq!(out[0], "rc=1");
    // Still exactly the two setup entries — the operand never joined.
    assert!(
        ends_with_dir(out[1], &sandbox.sub_canonical()),
        "dirs[0]: {:?}",
        out[1]
    );
    assert!(
        ends_with_dir(out[2], &sandbox.sub_canonical()),
        "dirs[1]: {:?}",
        out[2]
    );
    assert!(
        ends_with_dir(out[3], &sandbox.sub_canonical()),
        "pwd -P: {:?}",
        out[3]
    );
}

/// `readonly PWD; pushd` (no-arg swap): the swap lands before change_to_temp
/// (pushd.def:203-215), so dirs shows the swapped order — but the display
/// and rc 0 stay behind its EXECUTION_SUCCESS gate.
#[test]
fn readonly_pwd_pushd_swap_keeps_swap_but_fails() {
    let sandbox = Sandbox::new("sw-pwd");
    let script = format!(
        "{}readonly PWD\npushd\necho rc=$?\ndirs -p\necho p=[$PWD]\n",
        two_entry_stack()
    );
    let (stdout, stderr, code) = sandbox.run(&script);
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(has_var_diagnostic(&stderr, "PWD"), "diagnostic: {stderr:?}");
    let out = lines(&stdout);
    assert_eq!(out.len(), 4, "the failed swap printed no stack: {stdout:?}");
    assert_eq!(out[0], "rc=1");
    // Swapped in the display: sub (old cwd) on top of sub2.
    assert!(
        ends_with_dir(out[1], &sandbox.sub_canonical()),
        "dirs[0]: {:?}",
        out[1]
    );
    assert!(
        ends_with_dir(out[2], &sandbox.sub2_canonical()),
        "dirs[1]: {:?}",
        out[2]
    );
    // The readonly PWD variable kept its pre-swap value.
    assert!(
        ends_with_dir(
            out[3].trim_start_matches("p=[").trim_end_matches(']'),
            &sandbox.sub2_canonical()
        ),
        "PWD stuck: {:?}",
        out[3]
    );
}

/// `readonly OLDPWD; pushd +1` (rotation): the rotation survives the failed
/// bind like the swap does (pushd.def:256-286), rc 1, no display.
#[test]
fn readonly_oldpwd_pushd_rotate_keeps_rotation_but_fails() {
    let sandbox = Sandbox::new("ro-old");
    let script = format!(
        "{}readonly OLDPWD\npushd +1\necho rc=$?\ndirs -p\npwd -P\n",
        two_entry_stack()
    );
    let (stdout, stderr, code) = sandbox.run(&script);
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(
        has_var_diagnostic(&stderr, "OLDPWD"),
        "diagnostic: {stderr:?}"
    );
    let out = lines(&stdout);
    assert_eq!(
        out.len(),
        4,
        "the failed rotate printed no stack: {stdout:?}"
    );
    assert_eq!(out[0], "rc=1");
    assert!(
        ends_with_dir(out[1], &sandbox.sub_canonical()),
        "dirs[0]: {:?}",
        out[1]
    );
    assert!(
        ends_with_dir(out[2], &sandbox.sub2_canonical()),
        "dirs[1]: {:?}",
        out[2]
    );
    assert!(
        ends_with_dir(out[3], &sandbox.sub_canonical()),
        "pwd -P: {:?}",
        out[3]
    );
}

/// The -n forms never cd, so readonly variables are never consulted:
/// rc 0, no diagnostics, stack operations proceed (pushd.def:397/:400).
#[test]
fn no_cd_forms_ignore_readonly_vars() {
    let sandbox = Sandbox::new("nocd");
    let script = "\
readonly OLDPWD PWD
pushd -n ../sub2 >/dev/null
echo rc1=$?
popd -n >/dev/null
echo rc2=$?
dirs -p
";
    let (stdout, stderr, code) = sandbox.run(script);
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(
        !stderr.contains("readonly variable"),
        "no diagnostic expected: {stderr:?}"
    );
    let out = lines(&stdout);
    assert_eq!(out.len(), 3, "stdout: {stdout:?}");
    assert_eq!(out[0], "rc1=0");
    assert_eq!(out[1], "rc2=0");
    // push then pop of the same -n word: back to the lone cwd entry.
    assert_eq!(out[2].split_whitespace().count(), 1, "dirs: {:?}", out[2]);
}

// ---------------------------------------------------------------- #848 ----

/// `popd +5 -n`: the range diagnostic echoes the OUT-OF-RANGE OPERAND, not
/// the trailing option (pushd.def:374-377 which_word -> :382 pushd_error).
#[test]
fn popd_range_error_names_the_operand_not_the_trailing_option() {
    let sandbox = Sandbox::new("msg-po-n");
    let script = format!("{}popd +5 -n\necho rc=$?\ndirs -p\n", two_entry_stack());
    let (stdout, stderr, code) = sandbox.run(&script);
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(
        stderr.contains("popd: +5: directory stack index out of range"),
        "diagnostic: {stderr:?}"
    );
    assert!(
        !stderr.contains("-n: directory stack"),
        "must not blame the option: {stderr:?}"
    );
    let out = lines(&stdout);
    assert_eq!(out.len(), 3, "stdout: {stdout:?}");
    assert_eq!(out[0], "rc=1");
    // Untouched stack: both entries still there.
    assert!(
        ends_with_dir(out[1], &sandbox.sub2_canonical()),
        "dirs[0]: {:?}",
        out[1]
    );
    assert!(
        ends_with_dir(out[2], &sandbox.sub_canonical()),
        "dirs[1]: {:?}",
        out[2]
    );
}

/// `popd -n +5`: option-first spelling reports the same operand (guard
/// against a regression to args.last(), which only agreed by accident).
#[test]
fn popd_range_error_option_first_still_names_the_operand() {
    let sandbox = Sandbox::new("msg-po-first");
    let script = format!("{}popd -n +5\necho rc=$?\n", two_entry_stack());
    let (stdout, stderr, code) = sandbox.run(&script);
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(
        stderr.contains("popd: +5: directory stack index out of range"),
        "diagnostic: {stderr:?}"
    );
    assert_eq!(lines(&stdout).first().copied(), Some("rc=1"));
}

/// `pushd +9 -n`: pushd's walk range-checks the operand INLINE
/// (pushd.def:235-244) and hands the word itself to pushd_error, so the
/// "-n" that the walk never reaches must not be blamed.
#[test]
fn pushd_range_error_names_the_operand_not_the_trailing_option() {
    let sandbox = Sandbox::new("msg-pd-n");
    let script = format!("{}pushd +9 -n\necho rc=$?\ndirs -p\n", two_entry_stack());
    let (stdout, stderr, code) = sandbox.run(&script);
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(
        stderr.contains("pushd: +9: directory stack index out of range"),
        "diagnostic: {stderr:?}"
    );
    assert!(
        !stderr.contains("-n: directory stack"),
        "must not blame the option: {stderr:?}"
    );
    let out = lines(&stdout);
    assert_eq!(out.len(), 3, "stdout: {stdout:?}");
    assert_eq!(out[0], "rc=1");
}

/// `pushd +9` on a live two-entry stack keeps sh_erange's wording.
#[test]
fn pushd_range_error_on_live_stack_names_the_operand() {
    let sandbox = Sandbox::new("msg-pd-live");
    let script = format!("{}pushd +9\necho rc=$?\ndirs -p\n", two_entry_stack());
    let (stdout, stderr, code) = sandbox.run(&script);
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(
        stderr.contains("pushd: +9: directory stack index out of range"),
        "diagnostic: {stderr:?}"
    );
    let out = lines(&stdout);
    assert_eq!(out.len(), 3, "stdout: {stdout:?}");
    assert_eq!(out[0], "rc=1");
}

/// `pushd +9` / `pushd -9` on a spent stack: pushd_error's offset==0 branch
/// says "directory stack empty" (WSL GNU 5.3.0 probe, 2026-10-19) — the
/// offset wording is reserved for a live stack.
#[test]
fn pushd_range_error_on_empty_stack_says_empty() {
    for operand in ["+9", "-9"] {
        let sandbox = Sandbox::new("msg-pd-empty");
        let script = format!("pushd {operand}\necho rc=$?\ndirs -p\n");
        let (stdout, stderr, code) = sandbox.run(&script);
        assert_eq!(code, Some(0), "script rc for pushd {operand}");
        assert!(
            stderr.contains("pushd: directory stack empty"),
            "diagnostic for pushd {operand}: {stderr:?}"
        );
        assert!(
            !stderr.contains("index out of range"),
            "pushd {operand}: {stderr:?}"
        );
        let out = lines(&stdout);
        assert_eq!(out.len(), 2, "pushd {operand}: {stdout:?}");
        assert_eq!(out[0], "rc=1", "pushd {operand}");
        assert_eq!(
            out[1].split_whitespace().count(),
            1,
            "stack untouched: dirs {:?}",
            out[1]
        );
    }
}

/// `popd +5 -n` on a spent stack: the empty verdict wins over the operand
/// wording for popd too (pushd_error's offset==0 branch, via :382).
#[test]
fn popd_range_error_on_empty_stack_says_empty() {
    let sandbox = Sandbox::new("msg-po-empty");
    let (stdout, stderr, code) = sandbox.run("popd +5 -n\necho rc=$?\n");
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert!(
        stderr.contains("popd: directory stack empty"),
        "diagnostic: {stderr:?}"
    );
    assert!(!stderr.contains("index out of range"), "stderr: {stderr:?}");
    assert_eq!(lines(&stdout).first().copied(), Some("rc=1"));
}

// ---------------------------------------------------------------- #859 ----

/// `pushd -n` with no operand is a silent no-op rc 0 in GNU
/// (pushd.def:290-291 — the option walk consumes `-n`, leaves `list == 0`,
/// and returns EXECUTION_SUCCESS). It must NOT fall into the bare-`pushd`
/// swap arm, which errors "no other directory" when the stack is short.
/// rubash#859.
#[test]
fn pushd_no_cd_with_no_operand_is_a_silent_noop() {
    let sandbox = Sandbox::new("pd-n-noop");
    let (stdout, stderr, code) = sandbox.run("pushd -n\necho rc=$?\n");
    assert_eq!(code, Some(0), "script rc; stderr: {stderr:?}");
    assert_eq!(lines(&stdout).first().copied(), Some("rc=0"));
    assert!(
        !stderr.contains("no other directory"),
        "no swap diagnostic on -n no-op: {stderr:?}"
    );
}
