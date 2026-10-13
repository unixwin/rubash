//! Issue #585: the help surface forks on the invocation name.
//!
//! Under a bash/sh argv[0] the engine's `--help` prints the GNU usage
//! contract (version line + option tables, rc 0, on stdout) instead of the
//! product help; under any other name (rubash, niu) the product help stays
//! byte-identical. Oracle: WSL GNU bash 5.3.0(1)-release, `bash --help` and
//! `exec -a sh bash --help` (the usage lines carry the invoked name, and a
//! `-` login marker is kept in them). The shim layer needs no change: the
//! fork lives here, keyed off the engine's own argv[0].

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Copy the engine binary under an invocation name; argv[0]'s basename is
/// the surface the help fork keys off. fs::copy keeps the unix exec bit.
fn engine_copy_as(name: &str) -> PathBuf {
    let dir = Path::new("target").join("issue585");
    fs::create_dir_all(&dir).expect("create issue585 target dir");
    let copy = dir.join(name);
    fs::copy(env!("CARGO_BIN_EXE_rubash"), &copy).expect("copy engine binary");
    copy
}

fn run_help(binary: &Path) -> (i32, String, String) {
    let output = Command::new(binary)
        .arg("--help")
        .output()
        .expect("run --help");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The GNU `--help` contract, extra=1 (shell.c:2057-2103 show_shell_usage):
/// version line first, the usage pair naming the invoked shell, the engine's
/// accepted long-option table, the shell-options block, then the extra
/// footer. The option table lists exactly what the engine's parser accepts
/// (the LONG_OPTIONS contract set); the version string is the engine's own
/// BASH_VERSION and machtype in GNU's `version %s-(%s)` shape.
fn expected_gnu_help(shell_name: &str) -> String {
    let mut expected = String::new();
    expected.push_str(&format!(
        "GNU bash, version 5.3.0(1)-release-({})\n",
        rubash::executor::machtype_value()
    ));
    expected.push_str(&format!(
        "Usage:\t{shell_name} [GNU long option] [option] ...\n"
    ));
    expected.push_str(&format!(
        "\t{shell_name} [GNU long option] [option] script-file ...\n"
    ));
    expected.push_str("GNU long options:\n");
    for name in [
        "debug",
        "debugger",
        "dump-po-strings",
        "dump-strings",
        "help",
        "init-file",
        "login",
        "noediting",
        "noprofile",
        "norc",
        "posix",
        "pretty-print",
        "rcfile",
        "restricted",
        "verbose",
        "version",
    ] {
        expected.push_str(&format!("\t--{name}\n"));
    }
    expected.push_str("Shell options:\n");
    expected.push_str("\t-ilrsD or -c command or -O shopt_option\t\t(invocation only)\n");
    expected.push_str("\t-abefhkmnptuvxBCEHPT or -o option\n");
    expected.push_str(&format!(
        "Type `{shell_name} -c \"help set\"' for more information about shell options.\n"
    ));
    expected.push_str(&format!(
        "Type `{shell_name} -c help' for more information about shell builtin commands.\n"
    ));
    expected.push_str("Use the `bashbug' command to report bugs.\n");
    expected.push('\n');
    expected.push_str("bash home page: <http://www.gnu.org/software/bash>\n");
    expected.push_str("General help using GNU software: <http://www.gnu.org/gethelp/>\n");
    expected
}

#[test]
fn bash_invocation_help_is_the_gnu_usage_contract_on_stdout() {
    let name = if cfg!(windows) { "bash.exe" } else { "bash" };
    let (code, stdout, stderr) = run_help(&engine_copy_as(name));

    assert_eq!(code, 0, "--help exits EXECUTION_SUCCESS");
    assert_eq!(stderr, "", "GNU help goes to stdout, not stderr");
    assert_eq!(stdout, expected_gnu_help("bash"));
    // The product surface must be gone under this invocation name.
    assert!(!stdout.contains("Usage: rubash"));
    assert!(!stdout.contains("Identity:"));
}

#[test]
fn sh_invocation_help_names_sh_in_the_usage_lines() {
    let name = if cfg!(windows) { "sh.exe" } else { "sh" };
    let (code, stdout, stderr) = run_help(&engine_copy_as(name));

    assert_eq!(code, 0);
    assert_eq!(stderr, "");
    assert_eq!(stdout, expected_gnu_help("sh"));
    // The sh usage line is the sh form — GNU prints the invoked name, never
    // the canonical bash spelling.
    assert!(stdout.contains("Usage:\tsh [GNU long option] [option] ..."));
    assert!(!stdout.contains("Usage:\tbash"));
    assert!(stdout.contains("Type `sh -c help'"));
    assert!(!stdout.contains("Type `bash -c help'"));
}

#[test]
fn login_marker_bash_keeps_the_dash_in_the_usage_lines() {
    // shell.c set_shell_name keeps argv[0]'s basename verbatim: the oracle
    // (`exec -a -bash bash --help`) prints `Usage:\t-bash ...` while the
    // bash-vs-other decision strips the login marker.
    let name = if cfg!(windows) { "-bash.exe" } else { "-bash" };
    let (code, stdout, stderr) = run_help(&engine_copy_as(name));

    assert_eq!(code, 0);
    assert_eq!(stderr, "");
    assert_eq!(stdout, expected_gnu_help("-bash"));
}

#[test]
fn product_help_is_unchanged_under_non_shell_names() {
    // `niu --help` (and the engine's own name) keep the product help byte
    // for byte: the GNU fork appears ONLY under bash/sh invocation names.
    // Pin the default persona so an ambient RUBASH_IDENTITY cannot noise
    // the comparison.
    let expected_tail = if cfg!(windows) {
        "\n\nIdentity:\n  persona: msys (default; MSYS2-compatible)\n  switch:  export RUBASH_IDENTITY=native for the honest-native persona\n  query:   rubash --identity prints the persona and effective values\n"
    } else {
        "\n\nIdentity:\n  persona: native (honest-native; non-Windows builds are always native)\n  switch:  n/a (RUBASH_IDENTITY only selects personas on Windows)\n  query:   rubash --identity prints the persona and effective values\n"
    };
    let expected = format!("Usage: rubash [-c command] [script]{expected_tail}");

    let name = if cfg!(windows) { "niu.exe" } else { "niu" };
    let mut runs = Vec::new();
    runs.push(run_help_pinned(env!("CARGO_BIN_EXE_rubash").as_ref()));
    runs.push(run_help_pinned(&engine_copy_as(name)));
    for (code, stdout, stderr) in runs {
        assert_eq!(code, 0);
        assert_eq!(stderr, "");
        assert_eq!(stdout, expected, "product help must stay byte-identical");
        assert!(!stdout.contains("GNU long options:"));
        assert!(!stdout.contains("Usage:\t"));
    }
}

fn run_help_pinned(binary: &Path) -> (i32, String, String) {
    let output = Command::new(binary)
        .arg("--help")
        .env_remove("RUBASH_IDENTITY")
        .output()
        .expect("run --help");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}
