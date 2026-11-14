//! rubash#3421 — readonly-PWD bind family: after a failed readonly
//! `pushd` bind and directory deletion, `cd .` loses the getcwd
//! diagnostic line.
//!
//! GNU law: `change_to_directory` (builtins/cd.def:522-660) updates
//! `tcwd` (builtins/common.c:589 `get_working_directory`, "the current
//! working directory") at move time, BEFORE `bindpwd` runs
//! (builtins/cd.def:136-175); a readonly bind therefore fails with
//! EXECUTION_FAILURE while the chdir stands and tcwd already names
//! the new directory. When the cwd is then deleted, `resetpwd`
//! (builtins/cd.def:183-191) calls `get_working_directory`, whose
//! getcwd failure prints
//! `cd: error retrieving current directory: getcwd: cannot access
//! parent directories: No such file or directory`
//! (builtins/common.c:589-617). `pushd` delegates its Dir operand to
//! `cd_builtin` (builtins/pushd.def:296-302), so the same law covers
//! stack moves: rotation and swap are moves too.
//!
//! The rubash bug (introduced by the #3386 stack-builtin rework):
//! `execute_stack_builtin` maintained `logical_cwd` only when
//! `status == 0` and took the value from the env PWD — which is
//! stale exactly when the bind failed. After a readonly
//! `pushd a`, `logical_cwd` stayed `$D` while the session sat in
//! `$D/a`; a later `cd .` then probed `$D/.` (which canonicalizes)
//! instead of `$D/a/.` (which does not), so the deleted-dot arm
//! never fired and the getcwd diagnostic was lost, and `pwd -L`
//! read the stale spelling. The sibling face on the `cd` side was
//! `execute_cd` gating `refresh_cwd_anchor()` on `status == 0`, so
//! a readonly `cd a` never warmed the anchor for the entered
//! directory and a later `cd ../b` from the deleted cwd stranded
//! on the dead spelling.
//!
//! The fix (both gates are the MOVE, not the status; the value is
//! the move's logical spelling, not the env PWD):
//! `src/executor/trap_stack_builtins.rs` reads the stack carrier's
//! element 0 (the move's logical spelling, saved by pushd.rs on
//! every move path even on its bind-failure early returns) whenever
//! the physical cwd changed; `src/executor/printf_path_builtins.rs`
//! warms the anchor whenever `cd` moved.
//!
//! Expectations below are byte-derived from the GNU oracle
//! (bash 5.3.20, `~/workspace/rubash-audit/gnu-bash-5.3.20/bin/bash`;
//! probes `~/workspace/fix-3421/probe-3421-gnu*.out`). The prolog
//! file in stderr is the script path (`{path}`); the scratch root
//! is `{r}`.

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const GETCWD_DIAG: &str = "cd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory\n";

struct Tree {
    base: PathBuf,
    real: String,
}

impl Tree {
    fn new(tag: &str) -> Tree {
        let base = std::env::temp_dir().join(format!("rubash-3421-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("create scratch dir");
        let real = fs::canonicalize(&base)
            .expect("canonicalize scratch dir")
            .to_string_lossy()
            .into_owned();
        Tree { base, real }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// Run `body` (with `@R@` replaced by the scratch root) as a script
/// file and return the script path, stdout, stderr and exit code.
fn run_case(tag: &str, body: &str) -> (String, String, String, Option<i32>) {
    let tree = Tree::new(tag);
    let script = tree.base.join("probe.sh");
    fs::write(&script, body.replace("@R@", &tree.real)).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .current_dir(&tree.base)
        .output()
        .expect("run rubash");
    (
        script.to_string_lossy().into_owned(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

// ---------------------------------------------------------------------
// Acceptance face: C4-diag — readonly pushd bind fails, directory is
// deleted, `cd .` must print the getcwd diagnostic line (and keep the
// readonly lines, in order, before and after it).
// ---------------------------------------------------------------------

#[test]
fn issue3421_c4_diag_getcwd_line() {
    let (path, stdout, stderr, code) = run_case(
        "c4",
        r#"cd "@R@" || exit 1
mkdir a
readonly PWD OLDPWD
pushd a; echo "pushd-rc=$? PWD=$PWD"
(cd ..; rmdir a) || exit 1
cd .; echo "cd-rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(stdout, format!("pushd-rc=1 PWD={r}\ncd-rc=1 PWD={r}\n"));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: OLDPWD: readonly variable\n\
             {path}: line 4: PWD: readonly variable\n\
             {path}: line 5: OLDPWD: readonly variable\n\
             {path}: line 5: PWD: readonly variable\n\
             {GETCWD_DIAG}\
             {path}: line 6: OLDPWD: readonly variable\n\
             {path}: line 6: PWD: readonly variable\n"
        ),
        "stderr"
    );
}

/// The scratch root of the case that produced `path` (the script is
/// `<root>/probe.sh`).
fn path_r(path: &str) -> String {
    PathBuf::from(path)
        .parent()
        .expect("script parent")
        .to_string_lossy()
        .into_owned()
}

// ---------------------------------------------------------------------
// Live divergence #2: readonly pushd + `cd .` in a live dir —
// `pwd -L` must follow the move to `$D/a`, not the stale `$D`.
// ---------------------------------------------------------------------

#[test]
fn issue3421_readonly_pushd_cd_dot_pwd_l() {
    let (path, stdout, stderr, code) = run_case(
        "s4",
        r#"cd "@R@" || exit 1
mkdir a
readonly PWD
pushd a; echo "pushd-rc=$? PWD=$PWD"
cd .; echo "cd-rc=$? PWD=$PWD"; pwd -L
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(
        stdout,
        format!("pushd-rc=1 PWD={r}\ncd-rc=1 PWD={r}\n{r}/a\n")
    );
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: PWD: readonly variable\n\
             {path}: line 5: PWD: readonly variable\n"
        )
    );
}

// ---------------------------------------------------------------------
// Live divergence #3: readonly pushd + popd (empty stack) —
// `pwd -L` must read `$D/a`.
// ---------------------------------------------------------------------

#[test]
fn issue3421_readonly_pushd_popd_pwd_l() {
    let (path, stdout, stderr, code) = run_case(
        "s5b",
        r#"cd "@R@" || exit 1
mkdir a
readonly PWD OLDPWD
pushd a; echo "pushd-rc=$? PWD=$PWD"
popd; echo "popd-rc=$? PWD=$PWD"; pwd -L
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(
        stdout,
        format!("pushd-rc=1 PWD={r}\npopd-rc=1 PWD={r}\n{r}/a\n")
    );
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: OLDPWD: readonly variable\n\
             {path}: line 4: PWD: readonly variable\n\
             {path}: line 5: popd: directory stack empty\n"
        )
    );
}

// ---------------------------------------------------------------------
// Live divergence #4: readonly popd after a successful pushd —
// `pwd -L` must read `$D` (the popd-top target), not the stale
// `$D/a`, and `dirs` must show the popped stack.
// ---------------------------------------------------------------------

#[test]
fn issue3421_readonly_popd_pwd_l() {
    let (path, stdout, stderr, code) = run_case(
        "s12b",
        r#"cd "@R@" || exit 1
mkdir a
pushd a >/dev/null || exit 1
readonly PWD OLDPWD
popd; echo "popd-rc=$? PWD=$PWD"; pwd -L; dirs
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(stdout, format!("popd-rc=1 PWD={r}/a\n{r}\n{r} {r}\n"));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 5: OLDPWD: readonly variable\n\
             {path}: line 5: PWD: readonly variable\n"
        )
    );
}

// ---------------------------------------------------------------------
// Live divergence #5: readonly `cd`, directory deleted, `cd ../b` —
// the move must stand (tcwd follows to `$D/b`) with no spurious
// `cd:../b: No such file...` diagnostic; only the readonly lines.
// ---------------------------------------------------------------------

#[test]
fn issue3421_readonly_cd_deleted_cd_dotdot_b() {
    let (path, stdout, stderr, code) = run_case(
        "s13",
        r#"cd "@R@" || exit 1
mkdir a b
readonly PWD
cd a; echo "cd-rc=$? PWD=$PWD"
(cd ..; rmdir a) || exit 1
cd ../b; echo "cd2-rc=$? PWD=$PWD"; pwd -L
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(
        stdout,
        format!("cd-rc=1 PWD={r}\ncd2-rc=1 PWD={r}\n{r}/b\n")
    );
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: PWD: readonly variable\n\
             {path}: line 5: PWD: readonly variable\n\
             {path}: line 6: PWD: readonly variable\n"
        )
    );
}

// ---------------------------------------------------------------------
// Rotation / swap under a readonly bind: the move stands, tcwd
// follows the rotated / swapped stack top.
// ---------------------------------------------------------------------

#[test]
fn issue3421_readonly_rotation_pwd_l() {
    let (path, stdout, stderr, code) = run_case(
        "s16",
        r#"cd "@R@" || exit 1
mkdir a b
pushd a >/dev/null || exit 1
pushd ../b >/dev/null || exit 1
readonly PWD
pushd +1; echo "rot-rc=$? PWD=$PWD"; pwd -L; dirs
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(
        stdout,
        format!("rot-rc=1 PWD={r}/b\n{r}/a\n{r}/a {r} {r}/b\n")
    );
    assert_eq!(stderr, format!("{path}: line 6: PWD: readonly variable\n"));
}

#[test]
fn issue3421_readonly_swap_pwd_l() {
    let (path, stdout, stderr, code) = run_case(
        "s17",
        r#"cd "@R@" || exit 1
mkdir a b
pushd a >/dev/null || exit 1
pushd ../b >/dev/null || exit 1
readonly PWD
pushd; echo "swap-rc=$? PWD=$PWD"; pwd -L; dirs
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(
        stdout,
        format!("swap-rc=1 PWD={r}/b\n{r}/a\n{r}/a {r}/b {r}\n")
    );
    assert_eq!(stderr, format!("{path}: line 6: PWD: readonly variable\n"));
}

// ---------------------------------------------------------------------
// Conservation cells — neighbouring readonly-PWD shapes that already
// agreed with GNU on the base and must not move.
// ---------------------------------------------------------------------

#[test]
fn issue3421_c5_spawn_empty() {
    // C5: a spawned `ls` in the deleted cwd after a failed readonly
    // pushd bind must see an empty directory (the anchor twin of C4).
    let (path, stdout, stderr, code) = run_case(
        "s2",
        r#"cd "@R@" || exit 1
touch marker
mkdir a
readonly PWD
pushd a >/dev/null; echo "pushd-rc=$?"
(cd ..; rmdir a) || exit 1
x=$(ls 2>&1); echo "ls=[$x]"
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    assert_eq!(stdout, "pushd-rc=1\nls=[]\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 5: PWD: readonly variable\n\
             {path}: line 6: PWD: readonly variable\n"
        )
    );
}

#[test]
fn issue3421_readonly_pushd_cd_dotdot() {
    // S3: readonly pushd + `cd ..` in a live dir.
    let (path, stdout, stderr, code) = run_case(
        "s3",
        r#"cd "@R@" || exit 1
mkdir a
readonly PWD
pushd a; echo "pushd-rc=$? PWD=$PWD"
cd ..; echo "cd-rc=$? PWD=$PWD"; pwd -L
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(
        stdout,
        format!("pushd-rc=1 PWD={r}\ncd-rc=1 PWD={r}\n{r}\n")
    );
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: PWD: readonly variable\n\
             {path}: line 5: PWD: readonly variable\n"
        )
    );
}

#[test]
fn issue3421_readonly_cd_deleted_cd_dot() {
    // S8: readonly cd, directory deleted, `cd .` — the getcwd
    // diagnostic fires on the cd side already (cd.rs commits the
    // move before the bind check).
    let (path, stdout, stderr, code) = run_case(
        "s8",
        r#"cd "@R@" || exit 1
mkdir a
readonly PWD
cd a; echo "cd-rc=$? PWD=$PWD"
(cd ..; rmdir a) || exit 1
cd .; echo "cd2-rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(stdout, format!("cd-rc=1 PWD={r}\ncd2-rc=1 PWD={r}\n"));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: PWD: readonly variable\n\
             {path}: line 5: PWD: readonly variable\n\
             {GETCWD_DIAG}\
             {path}: line 6: PWD: readonly variable\n"
        )
    );
}

#[test]
fn issue3421_pushd_entry_cd_dot_control() {
    // S9/B2 control: non-readonly pushd entry, directory deleted,
    // `cd .` — rc 0, PWD gains the `/.` suffix, getcwd line only.
    let (path, stdout, stderr, code) = run_case(
        "s9",
        r#"mkdir "@R@/a"
pushd "@R@/a" >/dev/null || exit 1
(cd "@R@"; rmdir a) || exit 1
cd .; echo "rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(stdout, format!("rc=0 PWD={r}/a/.\n"));
    assert_eq!(stderr, GETCWD_DIAG);
}

#[test]
fn issue3421_pwd_only_readonly_c4() {
    // S10: PWD-only readonly variant of the C4 acceptance shape.
    let (path, stdout, stderr, code) = run_case(
        "s10",
        r#"cd "@R@" || exit 1
mkdir a
readonly PWD
pushd a; echo "pushd-rc=$? PWD=$PWD"
(cd ..; rmdir a) || exit 1
cd .; echo "cd-rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(stdout, format!("pushd-rc=1 PWD={r}\ncd-rc=1 PWD={r}\n"));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: PWD: readonly variable\n\
             {path}: line 5: PWD: readonly variable\n\
             {GETCWD_DIAG}\
             {path}: line 6: PWD: readonly variable\n"
        )
    );
}

#[test]
fn issue3421_readonly_pushd_deleted_cd_dotdot_b() {
    // S14: readonly pushd, directory deleted, `cd ../b` — the
    // pushd anchor twin of S13 (already agreed on the base).
    let (path, stdout, stderr, code) = run_case(
        "s14",
        r#"cd "@R@" || exit 1
mkdir a b
readonly PWD
pushd a >/dev/null; echo "pushd-rc=$?"
(cd ..; rmdir a) || exit 1
cd ../b; echo "cd2-rc=$? PWD=$PWD"; pwd -L
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(stdout, format!("pushd-rc=1\ncd2-rc=1 PWD={r}\n{r}/b\n"));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 5: PWD: readonly variable\n\
             {path}: line 6: PWD: readonly variable\n\
             {path}: line 7: PWD: readonly variable\n"
        )
    );
}

#[test]
fn issue3421_readonly_cd_cd_dotdot() {
    // S6: readonly cd + `cd ..` in a live dir.
    let (path, stdout, stderr, code) = run_case(
        "s6",
        r#"cd "@R@" || exit 1
mkdir a
readonly PWD
cd a; echo "cd-rc=$? PWD=$PWD"
cd ..; echo "cd2-rc=$? PWD=$PWD"; pwd -L
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(stdout, format!("cd-rc=1 PWD={r}\ncd2-rc=1 PWD={r}\n{r}\n"));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: PWD: readonly variable\n\
             {path}: line 5: PWD: readonly variable\n"
        )
    );
}

#[test]
fn issue3421_readonly_cd_deleted_cd_dotdot() {
    // S7: readonly cd, directory deleted, `cd ..` — resolves via
    // the tcwd string, no anchor needed.
    let (path, stdout, stderr, code) = run_case(
        "s7",
        r#"cd "@R@" || exit 1
mkdir a
readonly PWD
cd a; echo "cd-rc=$? PWD=$PWD"
(cd ..; rmdir a) || exit 1
cd ..; echo "cd2-rc=$? PWD=$PWD"; pwd -L
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(stdout, format!("cd-rc=1 PWD={r}\ncd2-rc=1 PWD={r}\n{r}\n"));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: PWD: readonly variable\n\
             {path}: line 5: PWD: readonly variable\n\
             {path}: line 6: PWD: readonly variable\n"
        )
    );
}

#[test]
fn issue3421_abs_pushd_c4() {
    // S11: absolute-operand readonly pushd variant of C4.
    let (path, stdout, stderr, code) = run_case(
        "s11",
        r#"cd "@R@" || exit 1
mkdir a
readonly PWD OLDPWD
pushd "@R@/a"; echo "pushd-rc=$? PWD=$PWD"
(cd ..; rmdir a) || exit 1
cd .; echo "cd-rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(stdout, format!("pushd-rc=1 PWD={r}\ncd-rc=1 PWD={r}\n"));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: OLDPWD: readonly variable\n\
             {path}: line 4: PWD: readonly variable\n\
             {path}: line 5: OLDPWD: readonly variable\n\
             {path}: line 5: PWD: readonly variable\n\
             {GETCWD_DIAG}\
             {path}: line 6: OLDPWD: readonly variable\n\
             {path}: line 6: PWD: readonly variable\n"
        )
    );
}

#[test]
fn issue3421_rotation_control() {
    // S18: non-readonly rotation control — pushd prints the
    // rotated stack, then the echo/dirs agree.
    let (path, stdout, stderr, code) = run_case(
        "s18",
        r#"cd "@R@" || exit 1
mkdir a b
pushd a >/dev/null || exit 1
pushd ../b >/dev/null || exit 1
pushd +1; echo "rot-rc=$? PWD=$PWD"; pwd -L; dirs
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = path_r(&path);
    assert_eq!(
        stdout,
        format!("{r}/a {r} {r}/b\nrot-rc=0 PWD={r}/a\n{r}/a\n{r}/a {r} {r}/b\n")
    );
    assert_eq!(stderr, "");
}
