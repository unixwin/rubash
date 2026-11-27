//! rubash#3335 — `cd .` in a deleted cwd: the verdict depends on how
//! the deleted cwd was entered, and on the mode.
//!
//! GNU law: builtins/cd.def change_to_directory (cd.def:522) makes the
//! operand absolute against the_current_working_directory
//! (make_absolute), canonicalizes it (sh_canonpath, PATH_CHECKDOTDOT |
//! PATH_CHECKEXISTS), and chdirs the canonical form — or, when
//! canonicalization fails, the operand VERBATIM: `chdir (nolinks ?
//! newdir : tdir)` first, then the POSIX step-9 verbatim `chdir
//! (newdir)`. A dot-only operand resolves through the cwd inode
//! itself, so the verbatim chdir succeeds even though the directory is
//! unlinked; `resetpwd ("cd")` (cd.def:183) then fails getcwd — the
//! diagnostic, `builtins/common.c:592 get_working_directory` — and
//! `set_working_directory (tdir)` binds the uncanonicalized
//! make_absolute form, so $PWD gains the `/.` suffix. The same verbatim
//! arm is what lets `cd ..` reach the surviving parent from a deleted
//! cwd. Both arms are mode-independent: -P tries the verbatim chdir
//! FIRST.
//!
//! The session side: the_current_working_directory is ONE stored
//! string (common.c:589), maintained by every cd-family mover —
//! pushd.def:296-302 delegates the Dir operand to cd_builtin, so
//! pushd maintains it too — and it is NEVER re-validated against the
//! filesystem: in a deleted cwd nothing canonicalizes, so the stored
//! spelling is the only truth. Rubash's session logical_cwd is that
//! stored value, and the cwd inode anchor (rubash#3198) is the
//! "process sits in the inode" half: it must be warmed by every
//! cd-family mover, not just cd, or the verbatim fallback strands.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-11-11). Unix-only: the shapes spell POSIX paths.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const GETCWD_DIAG: &str = "cd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory\n";

struct Tree {
    base: PathBuf,
    /// Canonical spelling of base (what physical lookups report).
    real: String,
}

fn make_tree(tag: &str) -> Tree {
    let base = std::env::temp_dir().join(format!("rubash3335-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("a")).expect("create a");
    let real = std::fs::canonicalize(&base)
        .expect("canonicalize base")
        .to_string_lossy()
        .into_owned();
    Tree { base, real }
}

fn make_link_tree(tag: &str) -> Tree {
    let tree = make_tree(tag);
    std::fs::create_dir_all(tree.base.join("a/real")).expect("create a/real");
    std::os::unix::fs::symlink("a/real", tree.base.join("link")).expect("create link");
    tree
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn run_script_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .arg(&case)
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

#[test]
fn issue3335_cd_entry_cd_dot_seed_shape() {
    // The issue's seed_r88d, verbatim: cd entry, rc 0, the getcwd
    // diagnostic, PWD gains /. — and pwd -L agrees with the bound value.
    let tree = make_tree("cd-dot");
    let script = format!(
        "cd \"{r}/a\"; (cd ..; rmdir a); cd .; echo \"rc=$?\"; echo \"PWD=$PWD\"; pwd -L; echo \"rc2=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(
        stdout,
        format!("rc=0\nPWD={r}/a/.\n{r}/a/.\nrc2=0\n", r = tree.real)
    );
}

#[test]
fn issue3335_pushd_entry_cd_dot_seed_shape() {
    // Morph mC4: the pushd entry form of the same shape.
    let tree = make_tree("pushd-dot");
    let script = format!(
        "pushd \"{r}/a\" >/dev/null; (cd \"{r}\"; rmdir a); cd .; echo \"rc=$?\"; echo \"PWD=$PWD\"; dirs\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(
        stdout,
        format!("rc=0\nPWD={r}/a/.\n{r}/a/. {r}\n", r = tree.real)
    );
}

#[test]
fn issue3335_cd_entry_cd_dot_slash() {
    let tree = make_tree("dot-slash");
    let script = format!(
        "cd \"{r}/a\"; (cd ..; rmdir a); cd ./; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(stdout, format!("rc=0\nPWD={r}/a/./\n", r = tree.real));
}

#[test]
fn issue3335_cd_entry_cd_dot_slash_dot() {
    let tree = make_tree("dot-slash-dot");
    let script = format!(
        "cd \"{r}/a\"; (cd ..; rmdir a); cd ./.; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(stdout, format!("rc=0\nPWD={r}/a/./.\n", r = tree.real));
}

#[test]
fn issue3335_cd_entry_cd_dash_l_dot() {
    let tree = make_tree("dash-l");
    let script = format!(
        "cd \"{r}/a\"; (cd ..; rmdir a); cd -L .; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(stdout, format!("rc=0\nPWD={r}/a/.\n", r = tree.real));
}

#[test]
fn issue3335_cd_entry_cd_dash_p_dot() {
    // GNU tries the verbatim chdir FIRST in -P (cd.def:522 `chdir
    // (nolinks ? newdir : tdir)`), so -P is the same success with the
    // same diagnostic and the same /. suffix — not a silent physical
    // rebind.
    let tree = make_tree("dash-p");
    let script = format!(
        "cd \"{r}/a\"; (cd ..; rmdir a); cd -P .; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(stdout, format!("rc=0\nPWD={r}/a/.\n", r = tree.real));
}

#[test]
fn issue3335_cd_entry_cd_dash_p_dot_slash() {
    let tree = make_tree("dash-p-slash");
    let script = format!(
        "cd \"{r}/a\"; (cd ..; rmdir a); cd -P ./; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(stdout, format!("rc=0\nPWD={r}/a/./\n", r = tree.real));
}

#[test]
fn issue3335_cd_entry_symlink_cd_dot_keeps_logical_spelling() {
    // The make_absolute base is the stored LOGICAL spelling (tcwd),
    // never the physical cwd: entered through the link, $PWD binds
    // "$R/link/." even though the physical directory is "$R/a/real".
    let tree = make_link_tree("link-dot");
    let script = format!(
        "cd \"{r}/link\"; (cd \"{r}\"; rm -rf a); cd .; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(stdout, format!("rc=0\nPWD={r}/link/.\n", r = tree.real));
}

#[test]
fn issue3335_cd_entry_symlink_cd_dash_p_dot_keeps_physical_spelling() {
    // The -P twin: entered with -P the stored value IS the physical
    // spelling, so the base is "$R/a/real".
    let tree = make_link_tree("link-dash-p");
    let script = format!(
        "cd -P \"{r}/link\"; (cd \"{r}\"; rm -rf a); cd -P .; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(stdout, format!("rc=0\nPWD={r}/a/real/.\n", r = tree.real));
}

#[test]
fn issue3335_pushd_entry_symlink_cd_dot_keeps_logical_spelling() {
    // The pushd-entry twin of the logical-spelling cell: pushd is a
    // cd-family mover (pushd.def:296-302 delegates to cd_builtin), so
    // the stored logical value after pushd is the link spelling.
    let tree = make_link_tree("pushd-link-dot");
    let script = format!(
        "pushd \"{r}/link\" >/dev/null; (cd \"{r}\"; rm -rf a); cd .; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(stdout, format!("rc=0\nPWD={r}/link/.\n", r = tree.real));
}

#[test]
fn issue3335_pushd_entry_cd_dotdot_reaches_parent() {
    // The verbatim arm for `..` needs the cwd inode anchor warmed by
    // the entry mover: pushd must warm it exactly as cd does
    // (rubash#3198), or the parent walk strands on the dead spelling.
    let tree = make_tree("pushd-dotdot");
    let script = format!(
        "pushd \"{r}/a\" >/dev/null; (cd \"{r}\"; rmdir a); cd ..; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("rc=0\nPWD={r}\n", r = tree.real));
}

#[test]
fn issue3335_pushd_entry_cd_dash_p_dotdot_reaches_parent() {
    let tree = make_tree("pushd-p-dotdot");
    let script = format!(
        "pushd \"{r}/a\" >/dev/null; (cd \"{r}\"; rmdir a); cd -P ..; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("rc=0\nPWD={r}\n", r = tree.real));
}

#[test]
fn issue3335_pushd_entry_cd_dash_p_dot() {
    // The pushd-entry -P twin: without the anchor the cd fails
    // outright (rc 1); with it, -P is the same dot-only success.
    let tree = make_tree("pushd-dash-p");
    let script = format!(
        "pushd \"{r}/a\" >/dev/null; (cd \"{r}\"; rmdir a); cd -P .; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(stdout, format!("rc=0\nPWD={r}/a/.\n", r = tree.real));
}

#[test]
fn issue3335_cd_entry_cd_dash_p_dotdot_reaches_parent() {
    // Control: the cd-entry -P `..` already works (the anchor is warm
    // from the entry cd) — the lock for the pushd-entry twin above.
    let tree = make_tree("cd-p-dotdot");
    let script = format!(
        "cd \"{r}/a\"; (cd ..; rmdir a); cd -P ..; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("rc=0\nPWD={r}\n", r = tree.real));
}

#[test]
fn issue3335_controls_in_deleted_cwd() {
    // The issue's agreeing controls: pwd -L and plain pwd echo the
    // stored spelling, cd "$PWD" is ENOENT rc 1, and pwd -P works
    // again once the directory is recreated.
    let tree = make_tree("controls");
    // The cd "$PWD" diagnostic carries the script prolog (shell name
    // + line), so the pin matches its tail via grep instead of an
    // exact line. The diagnostic file itself takes an ABSOLUTE path:
    // a relative redirect target in a deleted cwd is a separate
    // divergence (redirect creation against the dead spelling), not
    // this issue's cd face.
    let script = format!(
        "cd \"{r}/a\"; (cd ..; rmdir a); pwd -L; echo \"rcL=$?\"; pwd; echo \"rcP=$?\"; cd \"$PWD\" 2>\"{r}/err.txt\"; echo \"rcCD=$?\"; if grep -q \"cd: {r}/a: No such file or directory\" \"{r}/err.txt\"; then echo diag-ok; else echo diag-bad; fi; mkdir \"{r}/a\"; pwd -P; echo \"rcPP=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!(
            "{r}/a\nrcL=0\n{r}/a\nrcP=0\nrcCD=1\ndiag-ok\n{r}/a\nrcPP=0\n",
            r = tree.real
        )
    );
}
