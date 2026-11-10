//! Regression pins for rubash#3066 — the declare channel fires the
//! shared sv_locale funnel (`locale_assignment_warning`) only ONCE per
//! name, but GNU's declare_builtin runs
//! stupidly_hack_special_variables(name) after EVERY variable it
//! processes (builtins/declare.def:1105), so a bind that itself fires
//! the funnel plus the declare-side hack fire TWICE:
//!   LC_ALL=bogus_XX; declare -a LC_ALL; :
//! warns at the plain bind and AGAIN at the attribute conversion, and
//!   declare LC_ALL=(bogus_XX); :
//! warns for the compound bind and again for declare's own hack call.
//! rubash warns only once in both shapes; the conversion/compound
//! second fire is missing. A plain scalar declare bind
//! (`declare LC_ALL=bogus_XX`) fires exactly once in GNU too — the
//! hack call IS that fire — and stays a single-fire control here.
//!
//! GNU ownership: sv_locale (variables.c:6272) reads the binding via
//! get_string_value (variables.c:2495) and set_locale_var
//! (locale.c:185) warns on a failed setlocale, appending
//! `: <strerror(errno)>` only when errno is set — glibc leaves errno
//! set only on the FIRST failed lookup of a given locale name per
//! process, so the second fire of the same name warns without the
//! suffix. `local` shares declare_builtin, so its compound bind
//! double-fires the same way.
//!
//! Scope split with #3097: this file pins the fire COUNT (and the
//! line attribution of the conversion fire). What the declare
//! channel's warnings NAME — the scalar projection vs the raw
//! compound storage render — is the value face tracked by #3097
//! (the funnel's `stored_scalar_value` judges array-ness via
//! `attr_of` while declare maintains it in the mark sets), so the
//! compound cells below deliberately assert counts and warning
//! shape only, never the named value of a compound bind.
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for the
//! same script files in a locale-clean environment (only C/POSIX
//! locales installed; the bogus names can never resolve).
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3066-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Base command with the whole locale family removed from the
/// inherited environment so each case starts from a known state.
fn base() -> Command {
    let mut cmd = Command::new(BIN);
    for var in [
        "LANG",
        "LC_ALL",
        "LC_COLLATE",
        "LC_CTYPE",
        "LC_MESSAGES",
        "LC_NUMERIC",
        "LC_TIME",
    ] {
        cmd.env_remove(var);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd
}

fn run_script(tag: &str, body: &str) -> (String, String, String, Option<i32>) {
    let dir = scratch(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = base()
        .arg(&script)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let result = (
        script.to_string_lossy().into_owned(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// One warning line for `var` naming `value`, with the first-lookup
/// errno suffix, at script line `line`.
fn warn(path: &str, line: u32, var: &str, value: &str) -> String {
    format!(
        "{path}: line {line}: warning: setlocale: {var}: cannot change locale ({value}): No such file or directory\n"
    )
}

/// The setlocale warning lines in `stderr`, in order.
fn warning_lines(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|line| line.contains("warning: setlocale:"))
        .collect()
}

#[test]
fn issue3066_declare_a_conversion_refires_funnel() {
    // The plain bind warns (first failure: errno suffix); converting
    // the existing scalar to an array re-fires the funnel with the
    // converted value — same name, so no suffix the second time.
    let (path, _stdout, stderr, code) =
        run_script("convert", "LC_ALL=bogus_XX; declare -a LC_ALL; :\n");
    assert_eq!(code, Some(0));
    let lines = warning_lines(&stderr);
    assert_eq!(lines.len(), 2, "expected two warnings, got: {stderr}");
    assert_eq!(
        lines[0].to_string() + "\n",
        warn(&path, 1, "LC_ALL", "bogus_XX")
    );
    assert!(
        lines[1].starts_with(&format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale"
        )),
        "conversion fire missing or misattributed: {stderr}"
    );

    // Split across lines, the conversion fire is attributed to the
    // declare statement's own line.
    let (path, _stdout, stderr, code) =
        run_script("convert-lines", "LC_ALL=bogus_XX\ndeclare -a LC_ALL\n:\n");
    assert_eq!(code, Some(0));
    let lines = warning_lines(&stderr);
    assert_eq!(lines.len(), 2, "expected two warnings, got: {stderr}");
    assert_eq!(
        lines[0].to_string() + "\n",
        warn(&path, 1, "LC_ALL", "bogus_XX")
    );
    assert!(
        lines[1].starts_with(&format!(
            "{path}: line 2: warning: setlocale: LC_ALL: cannot change locale"
        )),
        "conversion fire missing or misattributed: {stderr}"
    );
}

#[test]
fn issue3066_declare_a_conversion_controls() {
    // Fresh conversion target (no prior scalar): the conversion of
    // an unset name binds nothing, both shells stay silent.
    let (_path, _stdout, stderr, code) = run_script("fresh", "declare -a LC_ALL; :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");

    // Converting a VALID locale value fires the funnel, but the
    // setlocale attempt succeeds — silent.
    let (_path, _stdout, stderr, code) = run_script("valid", "LC_ALL=C; declare -a LC_ALL; :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
}

#[test]
fn issue3066_declare_compound_fires_twice() {
    // Count only (the named value is #3097's face): GNU fires for
    // the compound bind and again for declare's per-variable hack.
    let (_path, _stdout, stderr, code) = run_script("compound", "declare LC_ALL=(bogus_XX); :\n");
    assert_eq!(code, Some(0));
    let lines = warning_lines(&stderr);
    assert_eq!(lines.len(), 2, "expected two warnings, got: {stderr}");
    assert!(
        lines
            .iter()
            .all(|line| line.contains("setlocale: LC_ALL: cannot change locale")),
        "both fires must be LC_ALL setlocale warnings: {stderr}"
    );
}

#[test]
fn issue3066_scalar_declare_fires_once() {
    // Control: a plain scalar declare bind is a single fire in GNU
    // (declare.def:1105's hack call is that fire; there is no second
    // bind underneath). Must stay exactly one warning.
    let (path, _stdout, stderr, code) = run_script("scalar", "declare LC_ALL=bogus_XX; :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));
}

#[test]
fn issue3066_declare_a_conversion_chain_readback() {
    // The converted binding is a real array afterwards: declare -p
    // and the scalar read (element 0) see the old scalar value, and
    // the warning trail is exactly the bind fire + conversion fire.
    let (path, stdout, stderr, code) = run_script(
        "chain",
        "LC_ALL=bogus_XX; declare -a LC_ALL; declare -p LC_ALL; echo \"elem0=[$LC_ALL]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -a LC_ALL=([0]=\"bogus_XX\")\nelem0=[bogus_XX]\n"
    );
    let lines = warning_lines(&stderr);
    assert_eq!(lines.len(), 2, "expected two warnings, got: {stderr}");
    assert_eq!(
        lines[0].to_string() + "\n",
        warn(&path, 1, "LC_ALL", "bogus_XX")
    );
}

#[test]
fn issue3066_declare_a_conversion_then_valid_bind() {
    // Chain: after the conversion, a scalar bind of a valid value
    // lands in element 0 and fires the funnel silently — the trail
    // stays exactly the two conversion warnings.
    let (path, _stdout, stderr, code) = run_script(
        "then-valid",
        "LC_ALL=bogus_XX; declare -a LC_ALL; LC_ALL=C; :\n",
    );
    assert_eq!(code, Some(0));
    let lines = warning_lines(&stderr);
    assert_eq!(lines.len(), 2, "expected two warnings, got: {stderr}");
    assert_eq!(
        lines[0].to_string() + "\n",
        warn(&path, 1, "LC_ALL", "bogus_XX")
    );
}

#[test]
fn issue3066_declare_assoc_conversion_refires_funnel() {
    // The -A conversion of an existing scalar is the same declare
    // face: plain bind fire + conversion fire.
    let (path, _stdout, stderr, code) =
        run_script("assoc-convert", "LC_ALL=bogus_XX; declare -A LC_ALL; :\n");
    assert_eq!(code, Some(0));
    let lines = warning_lines(&stderr);
    assert_eq!(lines.len(), 2, "expected two warnings, got: {stderr}");
    assert_eq!(
        lines[0].to_string() + "\n",
        warn(&path, 1, "LC_ALL", "bogus_XX")
    );
    assert!(
        lines[1].starts_with(&format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale"
        )),
        "conversion fire missing: {stderr}"
    );
}

#[test]
fn issue3066_local_compound_fires_twice() {
    // `local` is declare_builtin in a function frame: its compound
    // bind carries the same double fire (count only — the named
    // value is #3097's face).
    let (_path, _stdout, stderr, code) =
        run_script("local", "f() { local LC_ALL=(bogus_XX); :; }; f\n");
    assert_eq!(code, Some(0));
    let lines = warning_lines(&stderr);
    assert_eq!(lines.len(), 2, "expected two warnings, got: {stderr}");
    assert!(
        lines
            .iter()
            .all(|line| line.contains("setlocale: LC_ALL: cannot change locale")),
        "both fires must be LC_ALL setlocale warnings: {stderr}"
    );
}
