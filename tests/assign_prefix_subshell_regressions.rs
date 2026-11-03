//! niubash#452 / rubash#1354: an assignment word followed by `(` is NOT
//! an assignment-prefix + compound shape in GNU's grammar.
//!
//! The eco-audit corpus (scripts/harvest/snapshots/wt92-local-20261005/)
//! recorded GNU `bash -n` ACCEPTING lines like
//! `x=plugin::@(disable|enable|load)` (oh-my-bash cli.bash) and rubash was
//! changed to admit an assignment prefix before a subshell on that basis.
//! Direct measurement on the GNU 5.3.20 oracle (script files) shows the
//! corpus reading was wrong for the default state:
//!
//! - extglob OFF: `x=plugin::@(disable|enable|load)`, `x=a@(b|c)`,
//!   `pat=-*@([aAbcelmnNosu]|t-)` and `x=1 (echo hi)` are ALL syntax
//!   errors near `(` (rc 2) — an assignment word is a prefix of a simple
//!   command only (parse.y simple_command), never of a compound command,
//!   and the same holds for `A=1 if/for/case/while/((/{` (rubash#1354).
//! - extglob ON (set on a prior line): `x=plugin::@(disable|enable|load)`
//!   is ONE assignment word — `@(...)` is lexed into the word, no
//!   subshell is involved, and the value stays literal.

use std::process::Command;

fn shell_bin() -> std::path::PathBuf {
    // NOTE: the `bash` bin is a thin forwarder to the INSTALLED Niubash
    // (src/bin/bash.rs) — testing through it would exercise the old engine.
    // The real engine binary is `rubash`.
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

fn run(script: &str) -> (i32, String, String) {
    let output = Command::new(shell_bin())
        .args(["-c", script])
        .output()
        .expect("run shell");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn parse_rc(script: &str) -> i32 {
    let output = Command::new(shell_bin())
        .args(["-n", "-c", script])
        .output()
        .expect("run shell -n");
    output.status.code().unwrap_or(-1)
}

fn run_file(script: &str) -> (i32, String, String) {
    let path = std::env::temp_dir().join(format!("rubash-assign-prefix-{}.sh", std::process::id()));
    std::fs::write(&path, script).expect("write temp script");
    let output = Command::new(shell_bin())
        .arg(&path)
        .output()
        .expect("run shell file");
    let _ = std::fs::remove_file(&path);
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn assignment_word_followed_by_subshell_rejects() {
    // GNU 5.3.20, extglob off: every one of these is a syntax error.
    assert_eq!(parse_rc("x=a@(b|c)"), 2);
    assert_eq!(parse_rc("pat=-*@([aAbcelmnNosu]|t-)"), 2);
    assert_eq!(parse_rc("x=plugin::@(disable|enable|load)"), 2);
    assert_eq!(parse_rc("x=1 (echo hi)"), 2);
    assert_eq!(parse_rc("x=1 ( (echo hi) )"), 2);
}

#[test]
fn assignment_prefix_before_compound_rejects() {
    // rubash#1354: the whole compound family, not just subshells.
    assert_eq!(parse_rc("A=1 if true; then echo x; fi"), 2);
    assert_eq!(parse_rc("A=1 for i in 1; do echo $i; done"), 2);
    assert_eq!(parse_rc("A=1 case x in x) echo;; esac"), 2);
    assert_eq!(parse_rc("A=1 while false; do echo; done"), 2);
    assert_eq!(parse_rc("A=1 ((1+1))"), 2);
    assert_eq!(parse_rc("A=1 { echo hi; }"), 2);
    let (code, stdout, _) = run("A=1 (echo hi); echo after");
    assert_eq!(code, 2);
    assert_eq!(stdout, "");
}

#[test]
fn extglob_form_is_one_assignment_word() {
    // With extglob set on a prior line the `@(...)` belongs to the
    // assignment word itself; the value is stored literally.
    let (code, stdout, stderr) =
        run_file("shopt -s extglob\nx=plugin::@(disable|enable|load)\necho \"[$x]\"\n");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "[plugin::@(disable|enable|load)]\n");
}

#[test]
fn empty_value_and_genuine_errors_still_reject() {
    // `x= (` stays the compound-assignment ambiguity GNU rejects (#221)
    assert_eq!(parse_rc("x= (echo hi)"), 2);
    // a bare word before `(` is GNU's function-def production, which demands
    // `name() {' — `foo (echo hi)` blames `echo` in GNU too
    assert_eq!(parse_rc("foo (echo hi)"), 2);
    // redirection between prefix and compound has no GNU-accept evidence
    assert_eq!(parse_rc("x=1 < in.txt (echo hi)"), 2);
}
