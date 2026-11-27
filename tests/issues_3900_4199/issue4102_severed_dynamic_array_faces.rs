use std::{env, process::Command};

// rubash#4102: imported dynamic arrays (`GROUPS`, `BASH_ARGC`, `BASH_ARGV`,
// `BASH_LINENO`, `BASH_SOURCE`, `FUNCNAME`) are ordinary exported scalars
// (GNU `variables.c:1255` `init_dynamic_array_var` / `1831` `init_funcname_var`
// return an existing imported variable unchanged, without `att_array`,
// `att_noassign`, or `dynamic_value`). Element reads (`${NAME[0]}`) must read
// the scalar cell as `VT_VARIABLE` (`arrayfunc.c:1483`), and element writes
// (`NAME[0]=e`, `(( NAME[0] = 5 ))`, `printf -v`, `mapfile`) must convert the
// scalar to an indexed array (`arrayfunc.c:74` `convert_var_to_array`) rather
// than hitting the live getter or `att_noassign` swallow.

#[test]
fn imported_groups_scalar_element_reads_and_writes_match_gnu() {
    let script = r#"
printf 'init: %s | s=[%s] #=[%s] [0]=[%s] [1]=[%s] #[@]=%s ![%s] slice=(%s) :-0=[%s] :-1=[%s]\n' \
  "$(declare -p GROUPS)" "$GROUPS" "${#GROUPS}" "${GROUPS[0]}" "${GROUPS[1]}" \
  "${#GROUPS[@]}" "${!GROUPS[*]}" "${GROUPS[@]:0:1}" "${GROUPS[0]:-def}" "${GROUPS[1]:-def}"
GROUPS[0]=e
printf 'elem_assign(%d): %s | [0]=[%s]\n' "$?" "$(declare -p GROUPS)" "${GROUPS[0]}"
"#;
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .env("GROUPS", "init")
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");

    assert!(
        output.status.success(),
        "rubash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let expected = "\
init: declare -x GROUPS=\"init\" | s=[init] #=[4] [0]=[init] [1]=[] #[@]=1 ![0] slice=(i) :-0=[init] :-1=[def]
elem_assign(0): declare -ax GROUPS=([0]=\"e\") | [0]=[e]
";
    assert_eq!(stdout, expected);
}

#[test]
fn imported_groups_scalar_arith_printf_read_mapfile_convert_to_array() {
    let script = r#"
(( GROUPS[0] = 5 )); printf 'arith_elem(%d): %s\n' "$?" "$(declare -p GROUPS)"
(( GROUPS = 7 )); printf 'arith_bare(%d): %s\n' "$?" "$(declare -p GROUPS)"
printf -v 'GROUPS[0]' '%s' hello; printf 'printf_v(%d): %s\n' "$?" "$(declare -p GROUPS)"
read GROUPS <<< r; printf 'read(%d): %s\n' "$?" "$(declare -p GROUPS)"
mapfile -t GROUPS <<< a; printf 'mapfile(%d): %s\n' "$?" "$(declare -p GROUPS)"
"#;
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .env("GROUPS", "init")
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");

    assert!(
        output.status.success(),
        "rubash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let expected = "\
arith_elem(0): declare -ax GROUPS=([0]=\"5\")
arith_bare(0): declare -ax GROUPS=([0]=\"7\")
printf_v(0): declare -ax GROUPS=([0]=\"hello\")
read(0): declare -ax GROUPS=([0]=\"r\")
mapfile(0): declare -ax GROUPS=([0]=\"a\")
";
    assert_eq!(stdout, expected);
}

#[test]
fn imported_dynamic_noassign_arrays_allow_element_reads_and_writes() {
    let script = r#"
f() {
  for v in GROUPS BASH_ARGC BASH_ARGV BASH_LINENO BASH_SOURCE FUNCNAME; do
    eval "printf '%s_fn: [0]=[%s] [1]=[%s]\n' \"\$v\" \"\${$v[0]}\" \"\${$v[1]}\""
  done
}
f
for v in GROUPS BASH_ARGC BASH_ARGV BASH_LINENO BASH_SOURCE FUNCNAME; do
  eval "$v[0]=e"
  rc=$?
  eval "printf '%s(%d): %s | [0]=[%s]\n' \"\$v\" \"\$rc\" \"\$(declare -p $v)\" \"\${$v[0]}\""
done
"#;
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .env("GROUPS", "init")
        .env("BASH_ARGC", "init")
        .env("BASH_ARGV", "init")
        .env("BASH_LINENO", "init")
        .env("BASH_SOURCE", "init")
        .env("FUNCNAME", "init")
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");

    assert!(
        output.status.success(),
        "rubash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let expected = "\
GROUPS_fn: [0]=[init] [1]=[]
BASH_ARGC_fn: [0]=[init] [1]=[]
BASH_ARGV_fn: [0]=[init] [1]=[]
BASH_LINENO_fn: [0]=[init] [1]=[]
BASH_SOURCE_fn: [0]=[init] [1]=[]
FUNCNAME_fn: [0]=[init] [1]=[]
GROUPS(0): declare -ax GROUPS=([0]=\"e\") | [0]=[e]
BASH_ARGC(0): declare -ax BASH_ARGC=([0]=\"e\") | [0]=[e]
BASH_ARGV(0): declare -ax BASH_ARGV=([0]=\"e\") | [0]=[e]
BASH_LINENO(0): declare -ax BASH_LINENO=([0]=\"e\") | [0]=[e]
BASH_SOURCE(0): declare -ax BASH_SOURCE=([0]=\"e\") | [0]=[e]
FUNCNAME(0): declare -ax FUNCNAME=([0]=\"e\") | [0]=[e]
";
    assert_eq!(stdout, expected);
}

#[test]
fn non_imported_groups_remains_noassign_until_unset() {
    let script = r#"
orig="${GROUPS[0]}"
GROUPS[0]=e
rc1=$?
after="${GROUPS[0]}"
unset GROUPS
rc_unset=$?
GROUPS=after_unset
GROUPS[0]=e
rc2=$?
printf 'same=%s rc1=%d rc_unset=%d rc2=%d %s\n' \
  "$([ "$orig" = "$after" ] && echo yes || echo no)" \
  "$rc1" "$rc_unset" "$rc2" "$(declare -p GROUPS)"
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
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert_eq!(
        stdout,
        "same=yes rc1=0 rc_unset=0 rc2=0 declare -a GROUPS=([0]=\"e\")\n"
    );
}
