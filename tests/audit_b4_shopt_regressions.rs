//! Deep-audit lane B4 (shopt / set -o / POSIX-mode matrix) regression pins.
//!
//! Every expectation below was captured from the GNU Bash 5.3.20 oracle
//! (`~/workspace/rubash-audit/gnu-bash-5.3.20/bin/bash`, built from the
//! vendored 5.3 sources) with script files, twice, before the fix.
//! Finding IDs refer to ~/workspace/deep-audit/b4/REPORT.md.

use std::io::Write;
use std::process::{Command, Stdio};

fn shell_bin() -> std::path::PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return std::path::PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

fn run_c(script: &str) -> (i32, String, String) {
    run_c_env(script, &[])
}

fn run_c_env(script: &str, extra_env: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = Command::new(shell_bin());
    // Airtight environment (the tests/regression.rs principle): listing
    // probes must not depend on the test runner's ambient variables.
    cmd.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", "/tmp")
        .env("TERM", "dumb")
        .args(["--noprofile", "--norc", "-c", script]);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("run shell");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn run_file(script: &str) -> (i32, String, String) {
    let dir = tempfile_dir("file");
    let path = dir.join("case.sh");
    std::fs::write(&path, script).expect("write case");
    let output = Command::new(shell_bin())
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", "/tmp")
        .env("TERM", "dumb")
        .args(["--noprofile", "--norc"])
        .arg(&path)
        .output()
        .expect("run shell file");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-audit-b4-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir temp");
    dir
}

// ---------------------------------------------------------------- F01
// GNU variables.c:5719-5736 push_source: with bash_source_fullpath the
// BASH_SOURCE entry is sh_realpath(filename); without it, the name as
// given.
#[test]
fn audit_b4_f01_bash_source_fullpath() {
    let dir = tempfile_dir("f01");
    std::fs::create_dir_all(dir.join("sub")).expect("mkdir sub");
    std::fs::write(dir.join("sub/s.sh"), "echo \"src=[${BASH_SOURCE[0]}]\"\n")
        .expect("write sourced");
    let main = dir.join("main.sh");
    let canon = std::fs::canonicalize(dir.join("sub/s.sh"))
        .expect("canon")
        .to_string_lossy()
        .into_owned();

    std::fs::write(&main, "shopt -s bash_source_fullpath\n. sub/s.sh\n").expect("write main");
    let output = Command::new(shell_bin())
        .args(["--noprofile", "--norc"])
        .arg(&main)
        .current_dir(&dir)
        .output()
        .expect("run on");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        format!("src=[{canon}]\n"),
        "fullpath on must push the realpath"
    );

    // The shell's own `cd` moves the base for the realpath too: rubash
    // tracks the working directory as the logical PWD (no process
    // chdir), so the canonicalization must join onto PWD — a build
    // that resolves against the process cwd only passes the
    // launch-directory shape above and fails this one.
    std::fs::write(&main, "shopt -s bash_source_fullpath\ncd sub\n. s.sh\n")
        .expect("write main cd");
    let cd_canon = std::fs::canonicalize(dir.join("sub/s.sh"))
        .expect("canon cd")
        .to_string_lossy()
        .into_owned();
    let output = Command::new(shell_bin())
        .args(["--noprofile", "--norc"])
        .arg(&main)
        .current_dir(&dir)
        .output()
        .expect("run cd");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        format!("src=[{cd_canon}]\n"),
        "fullpath on must resolve against the shell's post-cd directory"
    );

    std::fs::write(&main, ". sub/s.sh\n").expect("write main off");
    let output = Command::new(shell_bin())
        .args(["--noprofile", "--norc"])
        .arg(&main)
        .current_dir(&dir)
        .output()
        .expect("run off");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        "src=[sub/s.sh]\n",
        "fullpath off must push the name as given"
    );
}

// ---------------------------------------------------------------- F03
// GNU shopt.def minus-o forms: SET/UNSET with an invalid name print the
// diagnostic but return 0; only query/list forms return 1.
#[test]
fn audit_b4_f03_shopt_o_invalid_name_status() {
    let (_, out, _) = run_c("shopt -s -o nosuchopt 2>/dev/null; echo rc=$?");
    assert_eq!(out, "rc=0\n");
    let (_, out, _) = run_c("shopt -u -o nosuchopt 2>/dev/null; echo rc=$?");
    assert_eq!(out, "rc=0\n");
    let (_, out, _) = run_c("shopt -s -o -p nosuchopt 2>/dev/null; echo rc=$?");
    assert_eq!(out, "rc=0\n");
    let (_, out, _) = run_c("shopt -o nosuchopt 2>/dev/null; echo rc=$?");
    assert_eq!(out, "rc=1\n");
    let (_, out, _) = run_c("shopt -p -o nosuchopt 2>/dev/null; echo rc=$?");
    assert_eq!(out, "rc=1\n");
    // The diagnostic is still printed, and valid names in the same
    // invocation are still applied.
    let (_, out, err) = run_c("shopt -s -o posix nosuchopt; echo rc=$?; shopt -o posix");
    assert_eq!(out, "rc=0\nposix               \ton\n");
    assert!(err.contains("nosuchopt: invalid option name"), "err: {err}");
    // The non -o form keeps failing on a bad name.
    let (_, out, _) = run_c("shopt -s nosuchopt 2>/dev/null; echo rc=$?");
    assert_eq!(out, "rc=1\n");
}

// ---------------------------------------------------------------- F06
// GNU bashhist.c lithist: lines of a multi-line command are saved with
// their newlines instead of the delimiting characters — including after
// a reserved word (`do`), where cmdhist alone joins with a space.
#[test]
fn audit_b4_f06_lithist_preserves_newlines() {
    let dir = tempfile_dir("f06");
    let histfile = dir.join("hist");
    let mut child = Command::new(shell_bin())
        .args(["--noprofile", "--norc", "-i"])
        .env("HISTFILE", &histfile)
        .env("HISTFILESIZE", "100")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn interactive");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"shopt -s cmdhist lithist\nfor i in 1 2\ndo\necho val$i\ndone\nexit\n")
        .expect("feed");
    child.wait().expect("wait");
    let hist = std::fs::read_to_string(&histfile).expect("read histfile");
    assert!(
        hist.contains("for i in 1 2\ndo\necho val$i\ndone"),
        "lithist entry must keep every newline, got: {hist:?}"
    );
}

// ---------------------------------------------------------------- F07
// GNU execute_cmd.c:1641-1642 (execute_in_subshell): the loop_level
// reset is compat-gated — at compat44 and below a subshell inherits the
// loop depth, so break/continue there are meaningful and end only the
// subshell's own list.
#[test]
fn audit_b4_f07_compat44_subshell_loop_level() {
    let (rc, out, err) =
        run_file("shopt -s compat44\nfor i in 1 2; do ( break; echo INNER ); echo outer$i; done\n");
    assert_eq!(rc, 0);
    assert_eq!(out, "outer1\nouter2\n");
    assert_eq!(err, "");

    // Default level: the reset applies — break complains and the rest
    // of the subshell still runs.
    let (_, out, err) = run_file("for i in 1 2; do ( break; echo INNER ); echo outer$i; done\n");
    assert_eq!(out, "INNER\nouter1\nINNER\nouter2\n");
    assert!(err.contains("only meaningful"), "err: {err}");

    // break 2 at compat44 unwinds the inner loop AND the subshell list;
    // the parent loop is untouched.
    let (_, out, _) = run_file(
        "shopt -s compat44\nfor i in 1 2; do ( for j in a; do break 2; echo J-AFTER; done; echo SUB-AFTER ); echo outer$i; done\n",
    );
    assert_eq!(out, "outer1\nouter2\n");
}

// ---------------------------------------------------------------- F08
// GNU subst.c:10285-10288: at compat level <= 43 a bad substitution is
// &expand_wdesc_error even in POSIX mode (the command fails, the
// script continues); above 43 POSIX mode takes the fatal arm.
#[test]
fn audit_b4_f08_posix_bad_substitution_compat_gate() {
    let (rc, out, _) = run_file("set -o posix\nshopt -s compat43\necho ${}\necho ALIVE rc=$?\n");
    assert_eq!(out, "ALIVE rc=1\n");
    assert_eq!(rc, 0);

    let (rc, out, _) = run_file("set -o posix\nshopt -s compat31\necho ${}\necho ALIVE rc=$?\n");
    assert_eq!(out, "ALIVE rc=1\n");
    assert_eq!(rc, 0);

    // compat44 and the default level stay fatal in POSIX mode.
    let (rc, out, _) = run_file("set -o posix\nshopt -s compat44\necho ${}\necho ALIVE\n");
    assert_eq!(out, "");
    assert_eq!(rc, 1);
    let (rc, out, _) = run_file("set -o posix\necho ${}\necho ALIVE\n");
    assert_eq!(out, "");
    assert_eq!(rc, 1);

    // -c form: the error class abandons the string with status 1 at
    // compat <= 43; the fatal class maps to 127 (shell.c:1471).
    let (rc, out, _) = run_c("set -o posix; shopt -s compat43; echo ${}; echo ALIVE");
    assert_eq!(out, "");
    assert_eq!(rc, 1);
    let (rc, _, _) = run_c("set -o posix; echo ${}; echo ALIVE");
    assert_eq!(rc, 127);
}

// ---------------------------------------------------------------- F09
// GNU variables.c:5760 sv_shcompat at import: the inherited BASH_COMPAT
// value is kept verbatim, its flags are derived, and an out-of-range
// value is diagnosed at startup. The "43" rebind fires only when a
// genuinely imported BASHOPTS carries a compatNN name
// (shopt.def parse_bashopts -> set_compatibility_level).
#[test]
fn audit_b4_f09_bash_compat_import() {
    let (_, out, err) = run_c_env(
        "echo BC=[$BASH_COMPAT]; shopt compat43",
        &[("BASH_COMPAT", "4.3")],
    );
    assert_eq!(out, "BC=[4.3]\ncompat43            \ton\n");
    assert_eq!(err, "");

    let (_, out, err) = run_c_env(
        "echo BC=[$BASH_COMPAT]; shopt compat43 compat44",
        &[("BASH_COMPAT", "banana")],
    );
    assert_eq!(
        out,
        "BC=[banana]\ncompat43            \toff\ncompat44            \toff\n"
    );
    assert!(
        err.contains("BASH_COMPAT: banana: compatibility value out of range"),
        "err: {err}"
    );

    let (_, _, err) = run_c_env("true", &[("BASH_COMPAT", "4.30")]);
    assert!(
        err.contains("BASH_COMPAT: 4.30: compatibility value out of range"),
        "err: {err}"
    );

    // Imported BASHOPTS with a compat name DOES rebind (GNU parity).
    let (_, out, _) = run_c_env(
        "echo BC=[$BASH_COMPAT]",
        &[("BASH_COMPAT", "4.3"), ("BASHOPTS", "compat43")],
    );
    assert_eq!(out, "BC=[43]\n");
    // Imported BASHOPTS without compat names leaves the value alone.
    let (_, out, _) = run_c_env(
        "echo BC=[$BASH_COMPAT]",
        &[("BASH_COMPAT", "4.3"), ("BASHOPTS", "extglob")],
    );
    assert_eq!(out, "BC=[4.3]\n");
}

// ---------------------------------------------------------------- F10
// GNU variables.c push_posix_temp_var / merge_temporary_env: in POSIX
// mode the assignments preceding a special builtin persist; the
// setattr.def absorption rule is the non-POSIX mechanism only.
#[test]
fn audit_b4_f10_posix_special_builtin_assignment_persistence() {
    // The bare readonly/export invocations also print their listings;
    // assert on the echo lines only.
    let (_, out, _) = run_file(
        "set -o posix\nv=xx\nv=yy\nv=zz readonly\necho ro=[$v]\nv=ww export\necho exp=[$v]\n",
    );
    let echoes: Vec<&str> = out
        .lines()
        .filter(|line| line.starts_with("ro=") || line.starts_with("exp="))
        .collect();
    assert_eq!(echoes, ["ro=[zz]", "exp=[ww]"]);

    let (_, out, _) = run_file("set -o posix\nv=zz :\necho colon=[$v]\n");
    assert_eq!(out, "colon=[zz]\n");

    // Non-POSIX keeps the absorption rule: only operand-named variables
    // persist across export/readonly.
    let (_, out, _) = run_file("v=xx\nv=zz readonly\necho ro=[$v]\n");
    let echoes: Vec<&str> = out.lines().filter(|line| line.starts_with("ro=")).collect();
    assert_eq!(echoes, ["ro=[xx]"]);
    let (_, out, _) = run_file(
        "FOO=val export BAR\necho foo=[${FOO-unset}]\nFOO=val export FOO\necho foo2=[$FOO]\n",
    );
    assert_eq!(out, "foo=[unset]\nfoo2=[val]\n");
}

// ---------------------------------------------------------------- F11
// GNU variables.c:1126 print_var_value: the ansic ($'...') branch is
// gated on posixly_correct == 0; POSIX mode single-quotes the raw
// value (control characters stay literal bytes).
#[test]
fn audit_b4_f11_posix_set_quoting() {
    let (_, out, _) = run_c("set -o posix; v=$(printf 'a\\tb'); set | grep ^v=");
    assert_eq!(out, "v='a\tb'\n");
    // Default mode keeps the $'...' form.
    let (_, out, _) = run_c("v=$(printf 'a\\tb'); set | grep ^v=");
    assert_eq!(out, "v=$'a\\tb'\n");
    // POSIX IFS renders with literal tab/newline bytes inside single
    // quotes (a three-line block); the default form is the one-line
    // $'...' rendering.
    let (_, posix_out, _) = run_c("set -o posix; set");
    assert!(
        posix_out.contains("IFS=' \t\n'"),
        "posix IFS block missing: {posix_out}"
    );
    let (_, plain_out, _) = run_c("set");
    assert!(
        plain_out.contains("IFS=$' \\t\\n'"),
        "default IFS line missing: {plain_out}"
    );
}

// ---------------------------------------------------------------- F12
// GNU variables.c print_var_list (POSIX path): a variable with no
// value prints in utility form (export name / export -a name /
// export -A name / readonly name ...), never declare form; a
// declared-but-never-assigned array/assoc is value-less in both
// modes, and the POSIX valued form carries the variable's own type
// flag.
#[test]
fn audit_b4_f12_posix_valueless_listings() {
    let (_, out, _) = run_c(
        "set -o posix; export ev; declare -a arr; export arr; declare -A asc; export asc; readonly rv; export -p | grep -E '^export (ev|-a arr|-A asc)$'; readonly -p | grep -E '^readonly rv$'",
    );
    assert_eq!(
        out,
        "export -a arr\nexport -A asc\nexport ev\nreadonly rv\n"
    );

    let (_, out, _) = run_c(
        "declare -a a1; declare -a a2=(); declare -A A1; declare -A A2=(); export a1 a2 A1 A2; export -p | grep -E '(a1|a2|A1|A2)(=|$)'; set -o posix; export -p | grep -E '(a1|a2|A1|A2)(=|$)'",
    );
    assert_eq!(
        out,
        "declare -Ax A1\ndeclare -Ax A2=()\ndeclare -ax a1\ndeclare -ax a2=()\n\
         export -A A1\nexport -A A2=()\nexport -a a1\nexport -a a2=()\n"
    );

    // Valued assocs render through the assoc formatter with the -A mark
    // in every listing flavor; the indexed formatter misread the assoc
    // mirror as element text (declare -ax A1=([0]="[k]=v")).
    let (_, out, _) = run_c(
        "declare -A A1=([k]=v); declare -a a1=([0]=x); export A1 a1; export -p | grep -E '(A1|a1)='",
    );
    assert_eq!(
        out,
        "declare -Ax A1=([k]=\"v\" )\ndeclare -ax a1=([0]=\"x\")\n"
    );
    let (_, out, _) = run_c(
        "set -o posix; declare -A A1=([k]=v); readonly A1; readonly -p | grep A1; export A1; export -p | grep A1",
    );
    assert_eq!(
        out,
        "readonly -A A1=([k]=\"v\" )\nexport -A A1=([k]=\"v\" )\n"
    );
    let (_, out, _) = run_c("declare -A A1=([k]=v); readonly A1; readonly -p | grep A1");
    assert_eq!(out, "declare -Ar A1=([k]=\"v\" )\n");
    // A declared-never-assigned readonly assoc lists valueless with -Ar.
    let (_, out, _) = run_c("declare -A A1; readonly A1; readonly -p | grep A1");
    assert_eq!(out, "declare -Ar A1\n");
}
