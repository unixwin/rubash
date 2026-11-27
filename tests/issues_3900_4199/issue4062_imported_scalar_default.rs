use std::{env, process::Command};

// rubash#4062: import-preserved bash-managed scalars must be treated as set
// (not unset) by parameter-brace expansion operators (`:-`, `-`, `:+`, `+`,
// `:?`, `?`), matching GNU `variables.c:1255-1280` (`init_dynamic_array_var` /
// `init_dynamic_assoc_var`), `variables.c:1827-1841` (`init_funcname_var`),
// and `subst.c:10131` (`parameter_brace_expand` `var_is_set = temp != NULL`).
// Because `init_dynamic_array_var` returns an imported cell unchanged without
// `att_nounset`, `unset` on an imported scalar also succeeds and leaves it
// unset for subsequent expansions.

#[test]
fn imported_bash_managed_scalars_work_with_braced_operators_and_unset() {
    let script = r#"
for v in BASH_ARGC BASH_SOURCE FUNCNAME BASH_LINENO BASH_ARGV BASH_CMDS BASH_ALIASES DIRSTACK; do
  eval "printf '%s: val=[%s] :-=[%s] -=[%s] :+=[%s] +=[%s] #=[%s]\n' \
    \"$v\" \"\$$v\" \"\${$v:-def}\" \"\${$v-def}\" \"\${$v:+alt}\" \"\${$v+alt}\" \"\${#$v}\""
done
echo "--- after unset ---"
for v in BASH_ARGC BASH_SOURCE FUNCNAME BASH_LINENO BASH_ARGV BASH_CMDS BASH_ALIASES DIRSTACK; do
  unset "$v"
  eval "printf '%s: val=[%s] :-=[%s] -=[%s] :+=[%s] +=[%s] #=[%s]\n' \
    \"$v\" \"\$$v\" \"\${$v:-def}\" \"\${$v-def}\" \"\${$v:+alt}\" \"\${$v+alt}\" \"\${#$v}\""
done
"#;
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .env("BASH_ARGC", "7")
        .env("BASH_SOURCE", "src_scalar")
        .env("FUNCNAME", "fn")
        .env("BASH_LINENO", "1")
        .env("BASH_ARGV", "argv_scalar")
        .env("BASH_CMDS", "c")
        .env("BASH_ALIASES", "a")
        .env("DIRSTACK", "d")
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");

    assert!(
        output.status.success(),
        "rubash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "",
        "expected empty stderr"
    );
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let expected = "\
BASH_ARGC: val=[7] :-=[7] -=[7] :+=[alt] +=[alt] #=[1]
BASH_SOURCE: val=[src_scalar] :-=[src_scalar] -=[src_scalar] :+=[alt] +=[alt] #=[10]
FUNCNAME: val=[fn] :-=[fn] -=[fn] :+=[alt] +=[alt] #=[2]
BASH_LINENO: val=[1] :-=[1] -=[1] :+=[alt] +=[alt] #=[1]
BASH_ARGV: val=[argv_scalar] :-=[argv_scalar] -=[argv_scalar] :+=[alt] +=[alt] #=[11]
BASH_CMDS: val=[c] :-=[c] -=[c] :+=[alt] +=[alt] #=[1]
BASH_ALIASES: val=[a] :-=[a] -=[a] :+=[alt] +=[alt] #=[1]
DIRSTACK: val=[d] :-=[d] -=[d] :+=[alt] +=[alt] #=[1]
--- after unset ---
BASH_ARGC: val=[] :-=[def] -=[def] :+=[] +=[] #=[0]
BASH_SOURCE: val=[] :-=[def] -=[def] :+=[] +=[] #=[0]
FUNCNAME: val=[] :-=[def] -=[def] :+=[] +=[] #=[0]
BASH_LINENO: val=[] :-=[def] -=[def] :+=[] +=[] #=[0]
BASH_ARGV: val=[] :-=[def] -=[def] :+=[] +=[] #=[0]
BASH_CMDS: val=[] :-=[def] -=[def] :+=[] +=[] #=[0]
BASH_ALIASES: val=[] :-=[def] -=[def] :+=[] +=[] #=[0]
DIRSTACK: val=[] :-=[def] -=[def] :+=[] +=[] #=[0]
";
    assert_eq!(stdout, expected);
}

#[test]
fn imported_empty_bash_managed_scalars_distinguish_null_from_unset() {
    let script = r#"
for v in BASH_ARGC BASH_SOURCE FUNCNAME BASH_LINENO BASH_ARGV; do
  eval "printf '%s: :-=[%s] -=[%s] :+=[%s] +=[%s] #=[%s]\n' \
    \"$v\" \"\${$v:-def}\" \"\${$v-def}\" \"\${$v:+alt}\" \"\${$v+alt}\" \"\${#$v}\""
done
"#;
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .env("BASH_ARGC", "")
        .env("BASH_SOURCE", "")
        .env("FUNCNAME", "")
        .env("BASH_LINENO", "")
        .env("BASH_ARGV", "")
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");

    assert!(
        output.status.success(),
        "rubash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let expected = "\
BASH_ARGC: :-=[def] -=[] :+=[] +=[alt] #=[0]
BASH_SOURCE: :-=[def] -=[] :+=[] +=[alt] #=[0]
FUNCNAME: :-=[def] -=[] :+=[] +=[alt] #=[0]
BASH_LINENO: :-=[def] -=[] :+=[] +=[alt] #=[0]
BASH_ARGV: :-=[def] -=[] :+=[] +=[alt] #=[0]
";
    assert_eq!(stdout, expected);
}

#[test]
fn non_imported_dynamic_stack_arrays_remain_unset_and_non_unsettable() {
    let script = r#"
printf 'FUNCNAME=[%s] SOURCE=[%s]\n' "${FUNCNAME:-def}" "${BASH_SOURCE-def}"
unset BASH_ARGC 2>&1 || echo "unset_argc_failed"
unset BASH_SOURCE 2>&1 || echo "unset_source_failed"
"#;
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");

    assert!(
        output.status.success(),
        "rubash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert!(
        stdout.starts_with("FUNCNAME=[def] SOURCE=[def]\n"),
        "unexpected stdout: {stdout}"
    );
    assert!(
        stdout.contains("BASH_ARGC: cannot unset")
            && stdout.contains("unset_argc_failed")
            && stdout.contains("BASH_SOURCE: cannot unset")
            && stdout.contains("unset_source_failed"),
        "expected cannot unset failures, got: {stdout}"
    );
}
