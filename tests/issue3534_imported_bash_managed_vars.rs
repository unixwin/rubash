use std::{env, process::Command};

// rubash#3534: imported bash-managed variable names must retain their
// `export` (`-x`) attribute per GNU `variables.c:506-517
// `initialize_shell_variables` and each variable's initializer (`set_ppid`,
// `uidset`, `INIT_DYNAMIC_VAR`, etc.), rather than being stripped by a blanket
// exclusion list.

#[test]
fn imported_bash_managed_vars_preserve_gnu_export_attributes() {
    let script = r#"
for n in BASH BASHOPTS BASH_VERSION EUID HOSTNAME HOSTTYPE LINENO MACHTYPE OLDPWD OPTARG OPTIND OSTYPE PIPESTATUS PPID RANDOM SECONDS SHELLOPTS SRANDOM UID _; do
  line=$(declare -p "$n" 2>/dev/null)
  read -r _ attrs _ <<< "$line"
  printf '%s=%s\n' "$n" "$attrs"
done
"#;
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .env("BASH", "/x")
        .env("BASHOPTS", "dotglob")
        .env("BASH_VERSION", "a")
        .env("EUID", "5")
        .env("HOSTNAME", "h")
        .env("HOSTTYPE", "h")
        .env("LINENO", "3")
        .env("MACHTYPE", "m")
        .env("OLDPWD", "/tmp")
        .env("OPTARG", "o")
        .env("OPTIND", "3")
        .env("OSTYPE", "o")
        .env("PIPESTATUS", "1")
        .env("PPID", "1")
        .env("RANDOM", "1")
        .env("SECONDS", "1")
        .env("SHELLOPTS", "posix")
        .env("SRANDOM", "1")
        .env("UID", "5")
        .env("_", "x")
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
BASH=-x
BASHOPTS=-rx
BASH_VERSION=-x
EUID=-x
HOSTNAME=-x
HOSTTYPE=-x
LINENO=-x
MACHTYPE=-x
OLDPWD=-x
OPTARG=-x
OPTIND=-ix
OSTYPE=-x
PIPESTATUS=-x
PPID=-ir
RANDOM=-ix
SECONDS=-ix
SHELLOPTS=-rx
SRANDOM=-ix
UID=-x
_=--
";
    assert_eq!(stdout, expected);
}
// DRAFT graft pins for PR #4034 (captain graft, to append to
// tests/issue3534_imported_bash_managed_vars.rs). Expectations to be
// calibrated against the cloud oracle matrix before finalizing.
// Gap A/B from review-4034.md §3: the 10 unpinned names + value face.

use std::fs;

#[test]
fn imported_array_assoc_family_scalars_preserved_with_export() {
    // rubash#3534 gap A: BASH_ALIASES/BASH_ARGC/BASH_ARGV/BASH_CMDS/
    // BASH_LINENO/BASH_SOURCE/DIRSTACK/FUNCNAME were entirely unpinned.
    // GNU (variables.c:1255-1277 init_dynamic_array_var /
    // init_dynamic_assoc_var): an imported cell is returned as-is, so the
    // scalar value AND its -x attribute survive. BASH_VERSINFO is a guard
    // only (GNU rebuilds it readonly in both trees: -ar either way).
    let script = r#"
for n in BASH_ALIASES BASH_ARGC BASH_ARGV BASH_CMDS BASH_LINENO BASH_SOURCE DIRSTACK FUNCNAME; do
  line=$(declare -p "$n" 2>/dev/null)
  read -r _ attrs _ <<< "$line"
  printf '%s=%s\n' "$n" "$attrs"
done
echo "ALIASES=[$BASH_ALIASES] ARGC=[$BASH_ARGC] ARGV=[$BASH_ARGV] CMDS=[$BASH_CMDS] LINENO2=[$BASH_LINENO] SOURCE=[$BASH_SOURCE] DIRSTACK=[$DIRSTACK] FUNCNAME=[$FUNCNAME]"
declare -p BASH_VERSINFO
"#;
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .env("BASH_ALIASES", "aliases_scalar")
        .env("BASH_ARGC", "7")
        .env("BASH_ARGV", "argv_scalar")
        .env("BASH_CMDS", "cmds_scalar")
        .env("BASH_LINENO", "42")
        .env("BASH_SOURCE", "src_scalar")
        .env("BASH_VERSINFO", "vers_scalar")
        .env("DIRSTACK", "dirstack_scalar")
        .env("FUNCNAME", "func_scalar")
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
BASH_ALIASES=-x
BASH_ARGC=-x
BASH_ARGV=-x
BASH_CMDS=-x
BASH_LINENO=-x
BASH_SOURCE=-x
DIRSTACK=-x
FUNCNAME=-x
ALIASES=[aliases_scalar] ARGC=[7] ARGV=[argv_scalar] CMDS=[cmds_scalar] LINENO2=[42] SOURCE=[src_scalar] DIRSTACK=[dirstack_scalar] FUNCNAME=[func_scalar]
";
    assert!(
        stdout.starts_with(expected),
        "stdout mismatch:\n{stdout}\nexpected prefix:\n{expected}"
    );
    // BASH_VERSINFO: attribute + array shape only (element values are the
    // shell's own version, not comparable across shells).
    let vers_line = stdout.lines().nth(9).expect("versinfo line");
    assert!(
        vers_line.starts_with("declare -ar BASH_VERSINFO=([0]="),
        "BASH_VERSINFO must be a readonly array, got: {vers_line}"
    );
}

#[test]
fn imported_seconds_counts_from_imported_base() {
    // rubash#3534 gap C: SECONDS_OFFSET seeding. GNU init_seconds_var
    // (variables.c:1350-1361) reads the imported value into
    // seconds_value_assigned; $SECONDS then starts at the imported base
    // and ticks. A double-count defect would read 20/21 for SECONDS=10.
    let script = r#"
first=$SECONDS
sleep 1
second=$SECONDS
echo "first=$first second=$second"
declare -p SECONDS
"#;
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .env("SECONDS", "10")
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
    let mut lines = stdout.lines();
    let first_line = lines.next().expect("first line");
    let rest: Vec<&str> = lines.collect();
    assert!(
        rest.iter().any(|l| l.starts_with("declare -ix SECONDS=")),
        "SECONDS must be -ix, got: {stdout}"
    );
    let nums: Vec<i64> = first_line
        .trim_start_matches("first=")
        .split(" second=")
        .map(|s| s.parse::<i64>().expect("numbers"))
        .collect();
    assert_eq!(nums.len(), 2);
    assert!(
        (10..=11).contains(&nums[0]),
        "SECONDS must start at the imported base 10, got {}",
        nums[0]
    );
    assert_eq!(
        nums[1] - nums[0],
        1,
        "SECONDS must tick by exactly 1 after sleep 1, got {nums:?}"
    );
}

#[test]
fn imported_bash_execution_string_kept_in_script_mode_replaced_in_c_mode() {
    // rubash#3534 gap E: BASH_EXECUTION_STRING dual mode. Script mode
    // keeps the imported cell; -c mode replaces it with the -c string
    // via the ordinary bind path.
    let script_path = env::temp_dir()
        .join(format!("issue3534_execstr_{}.sh", std::process::id()));
    fs::write(&script_path, "echo \"ES=[$BASH_EXECUTION_STRING]\"\n")
        .expect("write probe script");
    let script_out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .env("BASH_EXECUTION_STRING", "execstr_imported")
        .arg(&script_path)
        .output()
        .expect("run rubash script");
    assert!(
        script_out.status.success(),
        "rubash script failed: {}",
        String::from_utf8_lossy(&script_out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&script_out.stdout).trim(),
        "ES=[execstr_imported]"
    );
    let c_out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .env("BASH_EXECUTION_STRING", "execstr_imported")
        .arg("-c")
        .arg("echo \"ES=[$BASH_EXECUTION_STRING]\"")
        .output()
        .expect("run rubash -c");
    assert!(
        c_out.status.success(),
        "rubash -c failed: {}",
        String::from_utf8_lossy(&c_out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&c_out.stdout).trim(),
        "ES=[echo \"ES=[$BASH_EXECUTION_STRING]\"]"
    );
    fs::remove_file(&script_path).ok();
}
