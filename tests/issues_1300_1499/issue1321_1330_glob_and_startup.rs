//! Issues rubash#1321/#1322 (GLOBIGNORE matching), rubash#1323
//! (globstar `prefix/**/` and `./` prefix), rubash#1325 (clean-env
//! startup namespace), rubash#1326 (UID/EUID/PPID integer attribute),
//! rubash#1327 (`declare -pf` reports a missing function), rubash#1330
//! (`export -n name=value` still performs the assignment).
//!
//! Expected outputs verified against GNU bash 5.3.20.

use std::io::Write;
use std::process::Command;

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn fixture_tree() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-k7glob-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub1")).expect("mkdir sub1");
    std::fs::create_dir_all(dir.join("sub2")).expect("mkdir sub2");
    std::fs::create_dir_all(dir.join("d/e")).expect("mkdir d/e");
    std::fs::File::create(dir.join("a.txt"))
        .unwrap()
        .write_all(b"x")
        .unwrap();
    std::fs::File::create(dir.join("sub1/x.txt"))
        .unwrap()
        .write_all(b"x")
        .unwrap();
    dir
}

#[test]
fn globignore_slash_pattern_matches_full_path() {
    let dir = fixture_tree();
    let (stdout, _, _) = rubash_c(&format!(
        "cd {}; GLOBIGNORE='sub1/*'; echo sub1/*",
        dir.display()
    ));
    assert_eq!(stdout, "sub1/*\n");
}

#[test]
fn globignore_matches_dir_results_by_basename() {
    let dir = fixture_tree();
    let (stdout, _, _) = rubash_c(&format!(
        "cd {}; GLOBIGNORE='sub*'; echo */; echo *",
        dir.display()
    ));
    assert_eq!(stdout, "d/\na.txt d\n");
}

#[test]
fn globignore_uses_strmatch_pathname_semantics() {
    // pathexp.c glob_name_is_acceptable(): strmatch(pattern, name,
    // FNM_PATHNAME) on the FULL collected name — a non-trailing `*`
    // never crosses `/`, a trailing `*` matches any remainder, and
    // there is no basename fallback.
    let dir = fixture_tree();
    std::fs::create_dir_all(dir.join("d/x")).expect("mkdir d/x");
    std::fs::File::create(dir.join("d/x/inner"))
        .unwrap()
        .write_all(b"x")
        .unwrap();
    std::fs::create_dir_all(dir.join("d2")).expect("mkdir d2");
    std::fs::File::create(dir.join("d2/a.txt"))
        .unwrap()
        .write_all(b"x")
        .unwrap();
    let (stdout, _, _) = rubash_c(&format!(
        "cd {}; GLOBIGNORE='*/inner'; echo d/*/inner; GLOBIGNORE='d/*'; echo d/*/inner; GLOBIGNORE='a.txt'; echo d2/*",
        dir.display()
    ));
    assert_eq!(stdout, "d/x/inner\nd/*/inner\nd2/a.txt\n");
}

#[test]
fn globstar_trailing_slash_includes_base_and_keeps_dot_prefix() {
    let dir = fixture_tree();
    let (stdout, _, _) = rubash_c(&format!(
        "cd {}; shopt -s globstar; echo d/**/; echo ./**/",
        dir.display()
    ));
    assert_eq!(stdout, "d/ d/e/\n./ ./d/ ./d/e/ ./sub1/ ./sub2/\n");
}

#[test]
fn uid_euid_ppid_are_integer_readonly() {
    let (stdout, _, _) = rubash_c("declare -p UID EUID PPID");
    assert!(stdout.contains("declare -ir UID="), "{stdout}");
    assert!(stdout.contains("declare -ir EUID="), "{stdout}");
    assert!(stdout.contains("declare -ir PPID="), "{stdout}");
}

#[test]
fn declare_pf_reports_missing_function() {
    let (_, stderr, code) = rubash_c("declare -pf NOPE_FUNC_XYZ");
    assert!(
        stderr.contains("declare: NOPE_FUNC_XYZ: not found"),
        "{stderr}"
    );
    assert_eq!(code, Some(1));
    // Plain -f stays silent for a missing function.
    let (_, stderr, code) = rubash_c("declare -f NOPE_FUNC_XYZ");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(1));
}

#[test]
fn export_n_still_performs_assignment() {
    let (stdout, _, _) = rubash_c(
        "unset q; export -n q=5; echo \"q=[$q]\"; export -p | grep -c 'declare -x q' || true",
    );
    assert!(stdout.contains("q=[5]"), "{stdout}");
}

#[test]
fn clean_env_startup_namespace() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_clear()
        .arg("-c")
        .arg("declare -p TERM; export -p | grep -E ' (PATH|SHELL)=' | wc -l; set | grep -c COMP_WORDBREAKS")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("declare -- TERM=\"dumb\""), "{stdout}");
    assert!(stdout.contains("\n0\n"), "{stdout}");
}
