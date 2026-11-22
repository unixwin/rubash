//! rubash#4060: imported bash-managed scalar names (`BASH_ALIASES`,
//! `BASH_ARGC`, `BASH_ARGV`, `BASH_CMDS`, `BASH_LINENO`, `BASH_SOURCE`,
//! `DIRSTACK`, `FUNCNAME`, plus `GROUPS`) are ordinary scalar variables in
//! GNU Bash (`variables.c:511-520` `initialize_shell_variables`,
//! `variables.c:1255-1280` `init_dynamic_array_var` / `init_dynamic_assoc_var`,
//! `variables.c:1827-1841` `init_funcname_var`), so assignment, `local`, and
//! `unset` succeed on them. Unimported dynamic `att_noassign` / `att_nounset`
//! variables (`BASH_ARGC`, `BASH_ARGV`, `BASH_LINENO`, `BASH_SOURCE`,
//! `FUNCNAME`, `GROUPS`) must still reject `local` and `unset` (for
//! `att_nounset`).

use std::process::Command;

fn run_with_env(
    envs: &[(&str, &str)],
    unset_envs: &[&str],
    script: &str,
) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c").arg(script).env_remove("BASH_ENV");
    for key in unset_envs {
        cmd.env_remove(key);
    }
    for (key, val) in envs {
        cmd.env(key, val);
    }
    let output = cmd.output().expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn imported_bash_managed_scalars_allow_assign_local_and_unset() {
    let names = [
        "BASH_ALIASES",
        "BASH_ARGC",
        "BASH_ARGV",
        "BASH_CMDS",
        "BASH_LINENO",
        "BASH_SOURCE",
        "DIRSTACK",
        "FUNCNAME",
        "GROUPS",
    ];
    for name in names {
        let script = format!(
            "echo \"init=[${{{name}}}]\"\n\
             {name}=x\n\
             echo \"assign=$?:[${{{name}}}]\"\n\
             f() {{ local {name}=y; echo \"local=$?:[${{{name}}}]\"; }}\n\
             f\n\
             echo \"after=[${{{name}}}]\"\n\
             unset {name}\n\
             echo \"unset=$?:[${{{name}}}]\"\n"
        );
        let (stdout, stderr, code) = run_with_env(&[(name, "init")], &[], &script);
        assert_eq!(code, Some(0), "name={name} stderr={stderr:?}");
        assert_eq!(stderr, "", "name={name} stdout={stdout:?}");
        assert_eq!(
            stdout, "init=[init]\nassign=0:[x]\nlocal=0:[y]\nafter=[x]\nunset=0:[]\n",
            "name={name} stderr={stderr:?}"
        );
    }
}

#[test]
fn unimported_dynamic_noassign_and_nounset_still_enforced() {
    let noassign_names = [
        "BASH_ARGC",
        "BASH_ARGV",
        "BASH_LINENO",
        "BASH_SOURCE",
        "FUNCNAME",
        "GROUPS",
    ];
    for name in noassign_names {
        let script = format!(
            "{name}=x\n\
             f() {{ local {name}=y; echo \"local_rc=$?\"; }}\n\
             f\n"
        );
        let (stdout, stderr, code) = run_with_env(&[], &[name], &script);
        assert_eq!(code, Some(0), "name={name} stderr={stderr:?}");
        assert!(
            stderr.contains("variable may not be assigned value"),
            "name={name} expected noassign diagnostic, got stderr={stderr:?}"
        );
        assert_eq!(stdout, "local_rc=1\n", "name={name} stderr={stderr:?}");
    }

    for name in ["BASH_ARGC", "BASH_ARGV", "BASH_LINENO", "BASH_SOURCE"] {
        let script = format!("unset {name}; echo \"unset_rc=$?\"");
        let (stdout, stderr, code) = run_with_env(&[], &[name], &script);
        assert_eq!(code, Some(0), "name={name} stderr={stderr:?}");
        assert!(
            stderr.contains("cannot unset"),
            "name={name} expected cannot unset diagnostic, got stderr={stderr:?}"
        );
        assert_eq!(stdout, "unset_rc=1\n", "name={name} stderr={stderr:?}");
    }
}

#[test]
fn unimported_bash_cmds_assign_unset_and_local_match_gnu() {
    let script = "\
        BASH_CMDS=x; echo \"assign=$?:[${BASH_CMDS}]\"\n\
        unset BASH_CMDS; echo \"unset=$?:[${BASH_CMDS}]\"\n\
        f() { local BASH_CMDS=y; echo \"local=$?:[${BASH_CMDS}]\"; }\n\
        f\n\
        echo \"after=[${BASH_CMDS}]\"\n";
    let (stdout, stderr, code) = run_with_env(&[], &["BASH_CMDS"], script);
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
    assert_eq!(stdout, "assign=0:[x]\nunset=0:[]\nlocal=0:[y]\nafter=[]\n");
}

// Captain graft supplement (rubash#4060 graft): the operator and element
// read faces of an imported scalar — `${name:-def}` must treat the name
// as SET (the rubash#4062 face), and `${name[0]}` reads the scalar cell.
#[test]
fn imported_scalar_operator_and_element_faces() {
    let (stdout, stderr, code) = run_with_env(
        &[("BASH_ARGC", "7")],
        &[],
        "echo \"dflt=[${BASH_ARGC:-def}] plus=[${BASH_ARGC+alt}] e0=[${BASH_ARGC[0]}] len=[${#BASH_ARGC}]\"",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
    assert_eq!(stdout, "dflt=[7] plus=[alt] e0=[7] len=[1]\n");

    let (stdout, stderr, code) = run_with_env(
        &[("BASH_SOURCE", "src_scalar")],
        &[],
        "echo \"v=[${BASH_SOURCE}] e0=[${BASH_SOURCE[0]}] dflt=[${BASH_SOURCE:-def}]\"",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
    assert_eq!(stdout, "v=[src_scalar] e0=[src_scalar] dflt=[src_scalar]\n");
}

// Captain graft supplement: an imported name's declaration face is a
// plain scalar (no array/assoc attribute rendering).
#[test]
fn imported_bash_managed_declare_p_shows_plain_scalar() {
    let (stdout, stderr, code) = run_with_env(&[("BASH_ARGC", "7")], &[], "declare -p BASH_ARGC");
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert!(
        stdout.contains("BASH_ARGC=\"7\""),
        "expected scalar rendering, got stdout={stdout:?}"
    );
    assert!(
        !stdout.contains("declare -a") && !stdout.contains("declare -A"),
        "imported BASH_ARGC must not render as an array: stdout={stdout:?}"
    );
}
