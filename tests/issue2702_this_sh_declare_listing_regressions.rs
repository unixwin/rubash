//! rubash#2702 (owner decision D7, 2026-11-04) — hide the internal
//! THIS_SH variable from `declare` LISTING output.
//!
//! THIS_SH is not a GNU variable at all: the engine injects it at
//! startup (executor init) so shell scripts can self-invoke via
//! `${THIS_SH}` (the GNU test-suite convention), and it is not
//! exported. GNU bash 5.3.20 has no such name, so an internal
//! implementation detail must not appear in the user namespace
//! listing. The hiding rule is the one compgen's candidate walk
//! already applies (builtins/complete.rs, rubash#1799): only the
//! INJECTED spelling — the running executable's own path — is
//! hidden; a caller-supplied value that survived init (the
//! sh/bash-named wrapper exception) is a genuine environment
//! variable and lists. Named `declare -p THIS_SH` lookups and
//! `${THIS_SH}` expansion are deliberately unaffected: the
//! decision covers the listing (清单) only.
//!
//! Expectations measured against GNU bash 5.3.20 (no THIS_SH
//! anywhere) and the pre-existing rubash#1799 compgen rule.

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_c(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_no_this_sh_line(run: &Run, what: &str) {
    assert!(
        !run.stdout.lines().any(|line| line.contains("THIS_SH")),
        "{what} must not list THIS_SH, got:\n{}",
        run.stdout
    );
}

/// Plain `declare` (the full listing) must not contain THIS_SH.
#[test]
fn issue2702_plain_declare_hides_injected_this_sh() {
    let run = run_c("declare");
    assert_eq!(run.code, Some(0));
    assert_no_this_sh_line(&run, "declare");
}

/// `declare -p` with no names is the same listing in declare form.
#[test]
fn issue2702_declare_p_listing_hides_injected_this_sh() {
    let run = run_c("declare -p");
    assert_eq!(run.code, Some(0));
    assert_no_this_sh_line(&run, "declare -p");
}

/// `typeset` is the same builtin and the same listing.
#[test]
fn issue2702_typeset_listing_hides_injected_this_sh() {
    let run = run_c("typeset");
    assert_eq!(run.code, Some(0));
    assert_no_this_sh_line(&run, "typeset");
}

/// The named lookup is NOT part of the decision: an explicit
/// `declare -p THIS_SH` still renders the binding (and documents
/// the listing-only scope of the hiding).
#[test]
fn issue2702_named_declare_p_this_sh_still_renders() {
    let run = run_c("declare -p THIS_SH");
    assert_eq!(run.code, Some(0));
    assert_eq!(
        run.stdout,
        format!(
            "declare -- THIS_SH=\"{}\"\n",
            env!("CARGO_BIN_EXE_rubash").replace('\\', "/")
        )
    );
}

/// The expansion the variable exists for is untouched:
/// `${THIS_SH}` still resolves to the running executable's path.
#[test]
fn issue2702_this_sh_expansion_still_works() {
    let run = run_c("echo \"$THIS_SH\"");
    assert_eq!(run.code, Some(0));
    assert_eq!(
        run.stdout,
        format!("{}\n", env!("CARGO_BIN_EXE_rubash").replace('\\', "/"))
    );
}

/// `set` prints the same variable table through the same walk
/// (plain declare delegates to it), so it hides the injected
/// spelling too.
#[test]
fn issue2702_set_listing_hides_injected_this_sh() {
    let run = run_c("set");
    assert_eq!(run.code, Some(0));
    assert_no_this_sh_line(&run, "set");
}

/// A caller-supplied THIS_SH kept by init is a genuine variable
/// and MUST list — the hiding is value-shaped, not name-shaped
/// (rubash#1799 rule). Init keeps an inherited THIS_SH only when
/// its value names an existing sh/bash-named file (the wrapper
/// exception: a harness-installed `bash` copy the suites invoke),
/// so the fixture points THIS_SH at a `bash`-named copy of the
/// test binary while running the original binary.
#[test]
fn issue2702_caller_supplied_this_sh_still_lists() {
    let dir = std::env::temp_dir().join(format!("rubash2702-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let wrapper = dir.join("bash");
    std::fs::copy(env!("CARGO_BIN_EXE_rubash"), &wrapper).expect("copy binary as bash");
    let wrapper_path = wrapper.to_string_lossy().replace('\\', "/");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "declare -p"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("THIS_SH", &wrapper_path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout
            .lines()
            .any(|line| line == format!("declare -x THIS_SH=\"{wrapper_path}\"")),
        "caller-supplied THIS_SH must list, got:\n{stdout}"
    );
}
