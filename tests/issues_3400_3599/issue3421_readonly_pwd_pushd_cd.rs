//! rubash#3421 — readonly-PWD bind family: `pushd` + `cd .` regression tests.
//!
//! GNU Bash source ownership:
//! - `builtins/cd.def:262` `cd_builtin`
//! - `builtins/cd.def:522` `change_to_directory` (updates `the_current_working_directory`
//!   at `:600-619` before `bindpwd` runs)
//! - `builtins/cd.def:136` `bindpwd` (fails with `EXECUTION_FAILURE` when `PWD`/`OLDPWD`
//!   is `readonly`, leaving the chdir and `the_current_working_directory` standing)
//! - `builtins/pushd.def:298` `pushd_builtin` / `builtins/pushd.def:562` `cd_to_string`
//!   (delegates to `cd_builtin`)

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Tree {
    base: PathBuf,
    real: String,
}

impl Tree {
    fn new(tag: &str) -> Tree {
        let base =
            std::env::temp_dir().join(format!("rubash-3421-pc-{tag}-{}", std::process::id()));
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

fn posix_display_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let stripped = normalized.strip_prefix("//?/").unwrap_or(&normalized);
    if let Some(rest) = stripped.strip_prefix("//") {
        format!("/{rest}")
    } else if let Some((drive, tail)) = stripped.split_once(":/") {
        if drive.len() == 1 {
            format!("/{}/{}", drive.to_ascii_lowercase(), tail)
        } else {
            stripped.to_string()
        }
    } else {
        stripped.to_string()
    }
}

fn run_case(tag: &str, body: &str) -> (String, String, String, Option<i32>) {
    let tree = Tree::new(tag);
    let script = tree.base.join("probe.sh");
    let root = posix_display_path(&tree.real);
    fs::write(&script, body.replace("@R@", &root)).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .current_dir(&tree.base)
        .output()
        .expect("run rubash");
    (
        posix_display_path(&script.to_string_lossy()),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn readonly_pwd_pushd_then_cd_dot_tracks_logical_cwd() {
    let (path, stdout, stderr, code) = run_case(
        "live",
        r#"cd "@R@" || exit 1
mkdir a
readonly PWD
pushd a
echo "pushd-rc=$? PWD=$PWD"
cd .
echo "cd-rc=$? PWD=$PWD"
pwd -L
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = posix_display_path(
        &PathBuf::from(&path)
            .parent()
            .expect("script parent")
            .to_string_lossy(),
    );
    assert_eq!(
        stdout,
        format!("pushd-rc=1 PWD={r}\ncd-rc=1 PWD={r}\n{r}/a\n")
    );
    assert!(
        stderr.contains("line 4: PWD: readonly variable")
            && stderr.contains("line 6: PWD: readonly variable"),
        "unexpected stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn readonly_pwd_pushd_deleted_cd_dot_emits_getcwd_diag() {
    let (path, stdout, stderr, code) = run_case(
        "deleted",
        r#"cd "@R@" || exit 1
mkdir a
readonly PWD
pushd a
echo "pushd-rc=$? PWD=$PWD"
(cd ..; rmdir a) || exit 1
cd .
echo "cd-rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = PathBuf::from(&path)
        .parent()
        .expect("script parent")
        .to_string_lossy()
        .into_owned();
    assert_eq!(stdout, format!("pushd-rc=1 PWD={r}\ncd-rc=1 PWD={r}\n"));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: PWD: readonly variable\n\
             {path}: line 6: PWD: readonly variable\n\
             cd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory\n\
             {path}: line 7: PWD: readonly variable\n"
        )
    );
}

#[cfg(unix)]
#[test]
fn readonly_pwd_pushd_symlink_same_physical_updates_logical_cwd() {
    let (path, stdout, stderr, code) = run_case(
        "symlink-same-phys",
        r#"cd "@R@" || exit 1
mkdir real
ln -s real link
cd real
readonly PWD
pushd ../link
echo "pushd-rc=$? PWD=$PWD pwd-L=$(pwd -L) pwd-P=$(pwd -P)"
"#,
    );
    assert_eq!(code, Some(0), "script rc: {stderr}");
    let r = PathBuf::from(&path)
        .parent()
        .expect("script parent")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        stdout,
        format!("pushd-rc=1 PWD={r}/real pwd-L={r}/link pwd-P={r}/real\n")
    );
    assert_eq!(stderr, format!("{path}: line 6: PWD: readonly variable\n"));
}
