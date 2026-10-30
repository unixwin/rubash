//! rubash#2220 — the `$(cd X && pwd)` fast path rendered the
//! CANONICAL (physical) path for a bare `pwd`, where GNU prints the
//! logical $PWD the cd builtin maintains (builtins/cd.def bindpwd;
//! pwd_builtin echoes $PWD verbatim): symlink components and a
//! leading // survive in the logical rendering.
//!
//! The fast path now renders bare `pwd` through the builtin's own
//! logical chain (validated logical cwd + operand spelling, via
//! builtins::cd::logical_destination_display_from); `pwd -P` keeps
//! the physical rendering. The physical probe still gates existence.
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! probes in the cluster workdir: probe-gnu-2220.sh, probe-extra.sh
//! M rows). Relative-operand rows run with the process launched
//! inside the fixture so the session cwd and the launch cwd agree;
//! rows that first move the SESSION elsewhere and then use a
//! relative operand additionally depend on the comsub anchor fix
//! (rubash#2211, PR #2218) and are pinned there.

use std::path::PathBuf;
use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    rubash_in(None, script)
}

fn rubash_in(dir: Option<&PathBuf>, script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c").arg(script);
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let output = cmd.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2220_double_slash_absolute_operand_keeps_double_slash() {
    let (stdout, stderr, _code) = rubash("x=$(cd //etc && pwd); echo \"x=[$x] rc=$?\"");
    assert_eq!(stdout, "x=[//etc] rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2220_control_plain_absolute_operand() {
    let (stdout, _stderr, _code) = rubash("x=$(cd /tmp && pwd); echo \"x=[$x] rc=$?\"");
    assert_eq!(stdout, "x=[/tmp] rc=0\n");
}

#[test]
fn issue2220_control_missing_target_still_fails() {
    let (stdout, stderr, _code) = rubash("x=$(cd /no/such/dir-xyz && pwd); echo \"x=[$x] rc=$?\"");
    assert_eq!(stdout, "x=[] rc=1\n");
    assert!(
        stderr.contains("cd: /no/such/dir-xyz: No such file or directory"),
        "stderr: {stderr}"
    );
}

#[cfg(unix)]
mod unix_fixture {
    use super::*;

    fn fixture() -> PathBuf {
        let base = std::env::temp_dir().join(format!("rubash-issue2220-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("real/sub")).expect("fixture dirs");
        std::os::unix::fs::symlink("real", base.join("link")).expect("fixture symlink");
        std::fs::canonicalize(&base).expect("canonical fixture base")
    }

    #[test]
    fn issue2220_symlink_operand_renders_the_logical_spelling() {
        let base = fixture();
        let script = format!(
            "x=$(cd {} && pwd); echo \"x=[$x] rc=$?\"",
            base.join("link").display()
        );
        let (stdout, stderr, _code) = rubash(&script);
        assert_eq!(
            stdout,
            format!("x=[{}] rc=0\n", base.join("link").display())
        );
        assert_eq!(stderr, "");
    }

    #[test]
    fn issue2220_symlink_operand_with_subdir_renders_logical_chain() {
        let base = fixture();
        let script = format!(
            "x=$(cd {} && pwd); echo \"x=[$x] rc=$?\"",
            base.join("link/sub").display()
        );
        let (stdout, _stderr, _code) = rubash(&script);
        assert_eq!(
            stdout,
            format!("x=[{}] rc=0\n", base.join("link/sub").display())
        );
    }

    #[test]
    fn issue2220_symlink_operand_pwd_physical_stays_physical() {
        let base = fixture();
        let script = format!(
            "x=$(cd {} && pwd -P); echo \"x=[$x] rc=$?\"",
            base.join("link").display()
        );
        let (stdout, _stderr, _code) = rubash(&script);
        assert_eq!(
            stdout,
            format!("x=[{}] rc=0\n", base.join("real").display())
        );
    }

    #[test]
    fn issue2220_relative_symlink_operand_renders_logical_spelling() {
        let base = fixture();
        let (stdout, _stderr, _code) =
            rubash_in(Some(&base), "x=$(cd link && pwd); echo \"x=[$x] rc=$?\"");
        assert_eq!(
            stdout,
            format!("x=[{}] rc=0\n", base.join("link").display())
        );
    }

    #[test]
    fn issue2220_semicolon_form_renders_logical_spelling() {
        let base = fixture();
        let (stdout, _stderr, _code) =
            rubash_in(Some(&base), "x=$(cd link; pwd); echo \"x=[$x] rc=$?\"");
        assert_eq!(
            stdout,
            format!("x=[{}] rc=0\n", base.join("link").display())
        );
    }

    #[test]
    fn issue2220_subshell_form_agrees_with_fast_path() {
        // Control: the real-executor (subshell) form already rendered
        // the logical spelling; the fast path must agree with it.
        let base = fixture();
        let script = format!(
            "x=$( (cd {} && pwd) ); echo \"x=[$x] rc=$?\"",
            base.join("link").display()
        );
        let (stdout, _stderr, _code) = rubash(&script);
        assert_eq!(
            stdout,
            format!("x=[{}] rc=0\n", base.join("link").display())
        );
    }
}
