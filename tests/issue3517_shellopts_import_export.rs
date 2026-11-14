//! rubash#3517 — an imported `$SHELLOPTS` / `$BASHOPTS` keeps its export
//! attribute, so the live option state reaches child shells.
//!
//! GNU variables.c:506-517 initialize_shell_variables: every valid-name
//! environment entry — SHELLOPTS and BASHOPTS included (the `ro` branch
//! binds them with ASS_FORCE and then re-asserts att_readonly) — is bound
//! with `att_exported | att_imported`. The later rebuilds (builtins/set.def
//! set_shellopts, builtins/shopt.def set_bashopts) go through
//! bind_variable, which keeps the attributes: an imported SHELLOPTS stays
//! `declare -rx`, and `set -o` / `shopt -s` changes travel to a child
//! shell through the exported value. A shell whose environment carried
//! neither name binds them readonly but NOT exported.
//!
//! The pre-fix tree dropped the export mark on import (both names sat on
//! the bash-managed "never imported" list), so `declare -p` printed `-r`
//! and a grandchild shell fell back to the default option set.
//!
//! Expectations verified against GNU bash 5.3.20 with script files.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3517-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str, env: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("t.sh"), script).expect("write probe");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .current_dir(&dir)
        .arg("t.sh")
        .env_remove("SHELLOPTS")
        .env_remove("BASHOPTS")
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("THIS_SH")
        .env_remove("POSIXLY_CORRECT")
        .env_remove("__RUBASH_POSIX_MODE")
        .env_remove("__RUBASH_SETOPT_posix");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const DEFAULT_BASHOPTS: &str = "checkwinsize:cmdhist:complete_fullquote:extquote:\
force_fignore:globasciiranges:globskipdots:hostcomplete:interactive_comments:\
patsub_replacement:progcomp:promptvars:sourcepath";

#[test]
fn imported_shellopts_stays_readonly_exported() {
    let (stdout, stderr, code) = rubash_file(
        "so-import",
        "declare -p SHELLOPTS\nset -o noglob\ndeclare -p SHELLOPTS\n",
        &[("SHELLOPTS", "posix")],
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -rx SHELLOPTS=\"braceexpand:hashall:interactive-comments:posix\"\n\
         declare -rx SHELLOPTS=\"braceexpand:hashall:interactive-comments:noglob:posix\"\n"
    );
}

#[test]
fn imported_bashopts_stays_readonly_exported() {
    let (stdout, stderr, code) = rubash_file(
        "bo-import",
        "x=$(declare -p BASHOPTS)\necho \"${x:0:12}\"\n",
        &[("BASHOPTS", "dotglob")],
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -rx \n");
}

#[test]
fn imported_shellopts_carries_live_state_to_child_shell() {
    let (stdout, stderr, code) = rubash_file(
        "so-child",
        "set -o noglob\n\"$THIS_SH\" -c 'echo \"child $SHELLOPTS\"; declare -p SHELLOPTS'\n",
        &[("SHELLOPTS", "posix")],
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "child braceexpand:hashall:interactive-comments:noglob:posix\n\
         declare -rx SHELLOPTS=\"braceexpand:hashall:interactive-comments:noglob:posix\"\n"
    );
}

#[test]
fn imported_bashopts_carries_live_state_to_child_shell() {
    let (stdout, stderr, code) = rubash_file(
        "bo-child",
        "shopt -s nullglob\n\"$THIS_SH\" -c 'shopt -p dotglob nullglob'\n",
        &[("BASHOPTS", "dotglob")],
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "shopt -s dotglob\nshopt -s nullglob\n");
}

#[test]
fn shellopts_bashopts_not_imported_stay_unexported() {
    let (stdout, stderr, code) = rubash_file(
        "not-imported",
        "declare -p SHELLOPTS BASHOPTS\n\"$THIS_SH\" -c 'declare -p SHELLOPTS'\n",
        &[],
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        format!(
            "declare -r SHELLOPTS=\"braceexpand:hashall:interactive-comments\"\n\
             declare -r BASHOPTS=\"{DEFAULT_BASHOPTS}\"\n\
             declare -r SHELLOPTS=\"braceexpand:hashall:interactive-comments\"\n"
        )
    );
}
