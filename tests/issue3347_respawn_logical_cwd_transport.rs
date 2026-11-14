//! rubash#3347 — a fork-shaped respawn child re-derives its logical cwd
//! from the exported $PWD, so a forged $PWD moves `pwd -L` in the child.
//!
//! GNU's fork child inherits the_current_working_directory verbatim
//! (builtins/common.c:589): `&` backgrounds (execute_cmd.c:1576
//! execute_in_subshell), coproc bodies (coproc `make_child`) and
//! process-substitution bodies (subst.c:6362 process_substitute) are
//! forks, so the child's `pwd -L` prints the parent's logical value
//! while `$PWD` keeps the forged variable. A respawned `rubash -c`
//! child re-imports only its process environment, so the parent's
//! `logical_cwd` value must ride the respawn transport set
//! (`__RUBASH_LOGICAL_CWD` in fork_context_transport_entries) and the
//! child must adopt it under the provenance gate instead of re-deriving
//! from $PWD. The adoption check itself stays variables.c:907 set_pwd's
//! same_file judgment (logical_value_names_cwd): a transported value
//! that no longer names the session cwd falls back to the $PWD
//! adoption.
//!
//! The fresh-exec faces are NOT this bug and must keep adopting the
//! exported $PWD: an explicit `bin -c` child and a script-file child
//! are fresh processes in GNU too, and set_pwd adopts any
//! same-directory spelling there — the controls below pin the forged
//! value for exactly those faces.
//!
//! Expectations measured against GNU bash 5.3.20 (script files and
//! respawn spawns, 2026-11-11). Unix-only.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

/// Tree: <base>/real, <base>/link -> real
struct Tree {
    base: PathBuf,
    /// Canonical spelling of base (what physical lookups report).
    real: String,
}

fn make_tree(tag: &str) -> Tree {
    let base = std::env::temp_dir().join(format!("rubash3347-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("real")).expect("create real");
    std::os::unix::fs::symlink("real", base.join("link")).expect("create link");
    let real = std::fs::canonicalize(&base)
        .expect("canonicalize base")
        .to_string_lossy()
        .into_owned();
    Tree { base, real }
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
fn issue3347_background_builtin_keeps_logical_cwd() {
    let tree = make_tree("bg");
    let script = format!(
        "cd \"{r}/real\"; export PWD=\"{r}/link\"; pwd -L & wait; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/real\nPWD={r}/link\n", r = tree.real));
}

#[test]
fn issue3347_coproc_body_keeps_logical_cwd() {
    let tree = make_tree("coproc");
    let script = format!(
        "cd \"{r}/real\"; export PWD=\"{r}/link\"; coproc {{ pwd -L; echo \"PWD=$PWD\"; }}; cat <&${{COPROC[0]}}; wait $COPROC_PID 2>/dev/null\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/real\nPWD={r}/link\n", r = tree.real));
}

#[test]
fn issue3347_procsub_body_keeps_logical_cwd() {
    let tree = make_tree("procsub");
    let script = format!(
        "cd \"{r}/real\"; export PWD=\"{r}/link\"; cat <(pwd -L; echo \"PWD=$PWD\")\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/real\nPWD={r}/link\n", r = tree.real));
}

#[test]
fn issue3347_control_subshell_keeps_logical_cwd() {
    let tree = make_tree("subshell");
    let script = format!(
        "cd \"{r}/real\"; export PWD=\"{r}/link\"; ( pwd -L; echo \"PWD=$PWD\" )\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/real\nPWD={r}/link\n", r = tree.real));
}

#[test]
fn issue3347_control_comsub_keeps_logical_cwd() {
    let tree = make_tree("comsub");
    let script = format!(
        "cd \"{r}/real\"; export PWD=\"{r}/link\"; x=$(pwd -L; echo \"PWD=$PWD\"); printf '%s\\n' \"$x\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/real\nPWD={r}/link\n", r = tree.real));
}

#[test]
fn issue3347_control_explicit_c_child_adopts_forged_pwd() {
    let tree = make_tree("dashc");
    let script = format!(
        "cd \"{r}/real\"; export PWD=\"{r}/link\"; \"{bin}\" -c 'pwd -L; pwd -P; echo \"PWD=$PWD\"'\n",
        r = tree.real,
        bin = env!("CARGO_BIN_EXE_rubash")
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("{r}/link\n{r}/real\nPWD={r}/link\n", r = tree.real)
    );
}

#[test]
fn issue3347_control_script_file_child_adopts_forged_pwd() {
    let tree = make_tree("script");
    let child = tree.base.join("real").join("child.sh");
    std::fs::write(&child, "pwd -L; pwd -P; echo \"PWD=$PWD\"\n").expect("write child fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o755))
            .expect("chmod child fixture");
    }
    let script = format!(
        "cd \"{r}/real\"; export PWD=\"{r}/link\"; ./child.sh\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("{r}/link\n{r}/real\nPWD={r}/link\n", r = tree.real)
    );
}

#[test]
fn issue3347_control_parent_keeps_logical_cwd() {
    let tree = make_tree("parent");
    let script = format!(
        "cd \"{r}/real\"; export PWD=\"{r}/link\"; pwd -L; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/real\nPWD={r}/link\n", r = tree.real));
}
