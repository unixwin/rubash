//! Regressions for `type -p` / `type -P` / `type -a` option handling.
//!
//! Baseline: WSL GNU Bash 5.3.0 (`/usr/local/bin/bash`), probed with a full
//! name-kind (builtin/keyword/function/alias/file/not-found) x option
//! (-p/-P/-a/-t/-f and combos, hashed command, PATH duplicates) matrix that
//! is byte-identical after the fixes below.
//!
//! GNU anchors (third_party/bash/builtins/type.def, describe_command):
//! - `type -p NAME` where NAME resolves only to an alias / keyword /
//!   function / builtin sets found=1 and prints NOTHING (the CDESC_PATH_ONLY
//!   arms are silent) -> exit status 0. Previously rubash's `-a` path
//!   suppressed those blocks for PathOnly mode, so `type -ap cd` wrongly
//!   exited 1.
//! - The hash table is consulted only when `all == 0 || CDESC_FORCE_PATH`
//!   (phash_search guard), and the `is hashed (...)` wording lives only in
//!   the SHORTDESC branch, which `type -a` never reaches. Previously
//!   `type -a mycmd` prepended the hashed entry (a duplicate PATH hit
//!   labelled "is hashed (...)") where GNU prints the plain PATH matches.

use std::fs;
use std::path::Path;
use std::process::Command;

fn run_rubash(args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// wt33 (#373): run rubash with the process cwd set to `dir`. The empty-/
/// unset-PATH tests search the CURRENT directory (GNU: an empty PATH entry
/// means `.`), so they must execute inside the fixture directory —
/// otherwise `type -p e` scans the crate root and finds nothing. GNU probe
/// wt33-373/run/B14 (empty PATH -> `./e`) and B15 (unset PATH -> the
/// physical cwd path of `e`), both byte-identical.
fn run_rubash_in(dir: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Build an isolated fixture under target/: bin1/mycmd and bin2/mycmd
/// executables. Returns (dir, prologue assigning PATH to the two bins).
fn setup_fixture(tag: &str) -> (std::path::PathBuf, String) {
    let dir = Path::new("target").join(format!("type-p-{}", tag));
    let bin1 = dir.join("bin1");
    let bin2 = dir.join("bin2");
    fs::create_dir_all(&bin1).expect("mkdir bin1");
    fs::create_dir_all(&bin2).expect("mkdir bin2");
    for bin in [&bin1, &bin2] {
        let script = bin.join("mycmd");
        fs::write(&script, "#!/bin/sh\necho hi\n").expect("write mycmd");
        make_executable(&script);
    }
    (
        dir,
        format!(
            "PATH=\"{b1}:{b2}\"\n",
            b1 = bin1.to_string_lossy().replace('\\', "/"),
            b2 = bin2.to_string_lossy().replace('\\', "/"),
        ),
    )
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {
    // Windows: file presence is enough for rubash's lookup.
}

// ---------------------------------------------------------------------------
// `type -p` — silent success for non-file command kinds (GNU type.def:
// found=1, CDESC_PATH_ONLY arm prints nothing)
// ---------------------------------------------------------------------------

#[test]
fn type_p_builtin_is_silent_success() {
    let (stdout, stderr, code) = run_rubash(&["-c", "type -p cd; echo rc=$?"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

#[test]
fn type_p_function_is_silent_success() {
    let (stdout, stderr, code) = run_rubash(&["-c", "f() { :; }\ntype -p f\necho rc=$?\n"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

#[test]
fn type_p_keyword_is_silent_success() {
    let (stdout, stderr, code) = run_rubash(&["-c", "type -p if; echo rc=$?"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

#[test]
fn type_p_not_found_still_fails() {
    let (stdout, _stderr, code) = run_rubash(&["-c", "type -p nosuchname; echo rc=$?"]);
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(code, Some(0));
}

// ---------------------------------------------------------------------------
// `type -P` — forced PATH search past builtin/keyword/function
// ---------------------------------------------------------------------------

#[test]
fn type_p_prints_disk_file_path() {
    let (dir, prologue) = setup_fixture("print-path");
    let script = format!("{prologue}type -p mycmd\ntype -P mycmd\n");
    let script_path = dir.join("print.sh");
    fs::write(&script_path, &script).expect("write script");

    let (stdout, stderr, code) = run_rubash(&[script_path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");
    assert!(lines[0].ends_with("bin1/mycmd"), "stdout: {stdout}");
    assert!(lines[1].ends_with("bin1/mycmd"), "stdout: {stdout}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn type_p_forced_search_fails_when_no_disk_file() {
    // -P skips builtin/keyword/function lookup entirely: nothing printed,
    // exit status 1 (GNU describe_command returns found=0).
    let (stdout, _stderr, code) = run_rubash(&["-c", "type -P cd; echo rc=$?"]);
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(code, Some(0));
}

// ---------------------------------------------------------------------------
// `type -a` — no phantom hashed entry, plain labels
// ---------------------------------------------------------------------------

#[test]
fn type_a_lists_all_path_matches_without_hashed_label() {
    let (dir, prologue) = setup_fixture("all");
    // Execute once to populate the hash table, then enumerate.
    let script = format!("{prologue}mycmd >/dev/null\ntype -a mycmd\n");
    let script_path = dir.join("all.sh");
    fs::write(&script_path, &script).expect("write script");

    let (stdout, stderr, code) = run_rubash(&[script_path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");
    assert!(
        lines
            .iter()
            .all(|l| l.starts_with("mycmd is ") && !l.contains("hashed")),
        "stdout: {stdout}"
    );
    assert!(lines[0].ends_with("bin1/mycmd"), "stdout: {stdout}");
    assert!(lines[1].ends_with("bin2/mycmd"), "stdout: {stdout}");
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// `type -ap` — PathOnly must not flip found for alias/keyword/function/
// builtin kinds (previously `type -ap cd` exited 1)
// ---------------------------------------------------------------------------

#[test]
fn type_ap_builtin_and_function_exit_zero_silently() {
    let (dir, prologue) = setup_fixture("ap");
    let script = format!("{prologue}type -ap cd\ntype -ap myfunc\ntype -ap nosuch\n");
    let script_path = dir.join("ap.sh");
    fs::write(&script_path, &script).expect("write script");

    let (stdout, stderr, code) = run_rubash(&[script_path.to_str().unwrap()]);
    // GNU: cd and myfunc are "found" (silent under -ap); nosuch is not found
    // but CDESC_PATH_ONLY suppresses the sh_notfound message, so nothing is
    // printed anywhere and only the exit status records the failure.
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    assert_eq!(code, Some(1), "overall exit must reflect nosuch");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn type_ap_single_builtin_only_name_exits_zero() {
    let (stdout, stderr, code) = run_rubash(&["-c", "type -ap cd; echo rc=$?"]);
    assert_eq!(stdout, "rc=0\n");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

// ---------------------------------------------------------------------------
// Empty/unset PATH — GNU findcmd.c:266 path_value + builtins/type.def:390
// describe_command STREQ->file_status->sh_makepath(MP_DOCWD) semantics:
// PATH="" searches "." (prints "./name"); unset PATH returns NAME, then
// reports the physical cwd as an absolute path. `type -a` iterates
// user_command_matches: empty PATH contributes "./name", unset PATH nothing.
// ---------------------------------------------------------------------------

fn setup_dot_fixture(tag: &str) -> std::path::PathBuf {
    let dir = Path::new("target").join(format!("type-dot-{}", tag));
    fs::create_dir_all(&dir).expect("mkdir");
    let e = dir.join("e");
    fs::write(&e, "#!/bin/sh\nexit 0\n").expect("write e");
    make_executable(&e);
    dir
}

#[test]
fn type_p_empty_path_reports_dot_name() {
    let dir = setup_dot_fixture("empty")
        .canonicalize()
        .expect("canonicalize fixture dir");
    let script = "PATH=\ntype -p e\n";
    let script_path = dir.join("s.sh");
    fs::write(&script_path, script).expect("write script");

    let (stdout, stderr, code) = run_rubash_in(&dir, &[script_path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "./e\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn type_p_unset_path_reports_physical_cwd() {
    let dir = setup_dot_fixture("unset")
        .canonicalize()
        .expect("canonicalize fixture dir");
    let script =
        "unset PATH\nz=$(type -p e)\ncase $z in */e) echo ok;; *) echo \"bad:[$z]\";; esac\n";
    let script_path = dir.join("s.sh");
    fs::write(&script_path, script).expect("write script");

    let (stdout, stderr, code) = run_rubash_in(&dir, &[script_path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ok\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn type_a_unset_path_finds_nothing() {
    let dir = setup_dot_fixture("a-unset");
    let script = "unset PATH\ntype -a e\necho rc=$?\n";
    let script_path = dir.join("s.sh");
    fs::write(&script_path, script).expect("write script");

    let (stdout, _stderr, code) = run_rubash(&[script_path.to_str().unwrap()]);
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(code, Some(0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn type_a_empty_path_reports_dot_name() {
    let dir = setup_dot_fixture("a-empty")
        .canonicalize()
        .expect("canonicalize fixture dir");
    let script = "PATH=\ntype -a e\n";
    let script_path = dir.join("s.sh");
    fs::write(&script_path, script).expect("write script");

    let (stdout, stderr, code) = run_rubash_in(&dir, &[script_path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "e is ./e\n");
    let _ = fs::remove_dir_all(&dir);
}
