//! rubash#2943 — `type -a` must report the cwd match once per PATH
//! element GNU actually visits, not once per empty element in a naive
//! split.
//!
//! GNU anchors (bash 5.3):
//! - `findcmd.c:266-281` `path_value`: an empty PATH value normalizes
//!   to `"."` before any iteration.
//! - `findcmd.c:437/481` `user_command_matches`: the match list is
//!   built by `while (path_list && path_list[path_index])` with
//!   `get_next_path_element` (findcmd.c:322, empty unit -> `".")`.
//! - `general.c:1058` `extract_colon_unit`: each call leaves the index
//!   resting on a colon; an empty unit consumes that colon too, so a
//!   run of adjacent empty elements yields fewer visits than the
//!   element count — the second empty in a run folds into the first's
//!   terminator.
//!
//! Expected counts below (number of `e is ./e` lines with an executable
//! `e` in the cwd and nowhere else on PATH) were taken from GNU bash
//! 5.3.20 directly; they are also the issue's probe table.
//!
//! The enumeration had a second, independent defect in
//! `command_paths`: an empty PATH additionally pushed
//! `command_path_in_dot`'s "./name" on top of the walk's own "."
//! element, and an unset PATH ran the walk on "" — both reported one
//! phantom cwd match (the unset case must report nothing at all).

use std::path::PathBuf;
use std::process::Command;

fn run_type_a(path_value: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir: PathBuf =
        std::env::temp_dir().join(format!("issue2943_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("e"), "#!/bin/sh\necho hi\n").expect("write e");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("e"), std::fs::Permissions::from_mode(0o755))
            .expect("chmod e");
    }
    let script = format!("PATH={path_value}\ntype -a e\n");
    std::fs::write(dir.join("s.sh"), script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("s.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn cwd_matches(path_value: &str) -> usize {
    run_type_a(path_value)
        .lines()
        .filter(|line| *line == "e is ./e")
        .count()
}

#[test]
fn type_a_unset_path_reports_nothing() {
    // findcmd.c:477: path_value returns NULL for an unset PATH, so
    // user_command_matches has no list to walk — GNU reports
    // `type: e: not found`. (The single-match faces differ: plain
    // `type e` with unset PATH reports the cwd-absolute path via
    // find_user_command + sh_makepath; that face is not this pin.)
    let dir: PathBuf = std::env::temp_dir().join(format!("issue2943_unset_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("e"), "#!/bin/sh\necho hi\n").expect("write e");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("e"), std::fs::Permissions::from_mode(0o755))
            .expect("chmod e");
    }
    std::fs::write(dir.join("s.sh"), "unset PATH\ntype -a e\n").expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("s.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.lines().any(|line| line.starts_with("e is ")),
        "unset PATH must report no match, got stdout: {stdout}"
    );
}

#[test]
fn type_a_empty_path_element_visit_counts() {
    // (PATH, expected `e is ./e` lines) — GNU bash 5.3.20 oracle.
    let cases: &[(&str, usize)] = &[
        ("", 1),                 // path_value normalizes "" to "."
        (":", 1),                // one empty element
        ("::", 2),               // two visits, then the string ends
        (":::", 2),              // the run folds: third colon is a terminator
        (":/usr/bin:", 2),       // separated empties do not fold
        ("/usr/bin::", 1),       // trailing run folds to one visit
        ("::/usr/bin", 1),       // leading run folds to one visit
        (":/usr/bin:::/tmp", 2), // one + a folded run of three
        (":::/usr/bin::", 3),    // folded two + one + folded trailing
    ];
    for (path, want) in cases {
        assert_eq!(cwd_matches(path), *want, "PATH={path:?}");
    }
}
