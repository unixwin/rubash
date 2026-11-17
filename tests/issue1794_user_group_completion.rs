//! Issue rubash#1794 regression: `compgen -A user` / `compgen -A group`
//! returned only environment-derived names (USER/LOGNAME resp.
//! GROUP/GROUPNAME), and nothing at all when those were unset.
//!
//! GNU lib/readline/complete.c:2377 rl_username_completion_function
//! walks getpwent() entries in database order over /etc/passwd, and
//! bashline.c:2723 bash_groupname_completion_function walks getgrent()
//! over /etc/group; the name is the first colon field of each record.
//! The environment is never consulted.
//!
//! Oracle: GNU bash 5.3.20 — `compgen -A user` prints every passwd
//! name (root, daemon, bin, ...) in file order; `compgen -A user ro`
//! prints root; a name present only in $USER never appears.

#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
fn rubash(script: &str) -> (String, String, Option<i32>) {
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

/// Re-derive the expected candidates from the on-disk database under
/// the getpwent()/getgrent() rule so the assertion survives any DB
/// contents: first colon field of each non-comment, non-blank record,
/// in file order, prefix-filtered by the word.
#[cfg(unix)]
fn expected_candidates(db: &str, word: &str) -> Option<Vec<String>> {
    let contents = std::fs::read_to_string(PathBuf::from(db)).ok()?;
    let mut out = Vec::new();
    for line in contents.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let name = line.split(':').next().unwrap_or("");
        if !name.is_empty() && name.starts_with(word) {
            out.push(name.to_string());
        }
    }
    Some(out)
}

#[cfg(unix)]
#[test]
fn user_candidates_come_from_the_passwd_database() {
    let Some(expected) = expected_candidates("/etc/passwd", "") else {
        eprintln!("no passwd database on this host; skipping");
        return;
    };
    assert!(!expected.is_empty(), "passwd database must have records");
    let (stdout, _, rc) = rubash("compgen -A user");
    let got: Vec<String> = stdout.lines().map(str::to_string).collect();
    assert_eq!(rc, Some(0));
    assert_eq!(got, expected, "candidate list must follow database order");
}

#[cfg(unix)]
#[test]
fn group_candidates_come_from_the_group_database() {
    let Some(expected) = expected_candidates("/etc/group", "") else {
        eprintln!("no group database on this host; skipping");
        return;
    };
    assert!(!expected.is_empty(), "group database must have records");
    let (stdout, _, rc) = rubash("compgen -A group");
    let got: Vec<String> = stdout.lines().map(str::to_string).collect();
    assert_eq!(rc, Some(0));
    assert_eq!(got, expected, "candidate list must follow database order");
}

#[cfg(unix)]
#[test]
fn user_and_group_prefix_filtering_uses_the_database() {
    for (action, db) in [("user", "/etc/passwd"), ("group", "/etc/group")] {
        let Some(all) = expected_candidates(db, "") else {
            eprintln!("no {db} database on this host; skipping");
            return;
        };
        // A two-character prefix of the first record exercises the
        // generator-side prefix filter against the database contents.
        let Some(first) = all.first() else {
            continue;
        };
        let prefix: String = first.chars().take(2).collect();
        let Some(expected) = expected_candidates(db, &prefix) else {
            return;
        };
        let (stdout, _, rc) = rubash(&format!("compgen -A {action} {prefix}"));
        let got: Vec<String> = stdout.lines().map(str::to_string).collect();
        assert_eq!(rc, Some(0));
        assert_eq!(got, expected, "-A {action} {prefix} must filter the DB");
    }
}

#[cfg(unix)]
#[test]
fn environment_only_names_are_not_candidates() {
    // A name that exists only in the environment is not in the system
    // database and must not be offered (the pre-fix implementation
    // returned exactly these names).
    let (stdout, _, _) = rubash(
        "USER=zz_env_only_user_1794 LOGNAME=zz_env_only_user_1794 compgen -A user; \
         GROUP=zz_env_only_group_1794 GROUPNAME=zz_env_only_group_1794 compgen -A group",
    );
    assert!(
        !stdout.lines().any(|line| line == "zz_env_only_user_1794"),
        "env-only user name must not be a -A user candidate: {stdout}"
    );
    assert!(
        !stdout.lines().any(|line| line == "zz_env_only_group_1794"),
        "env-only group name must not be a -A group candidate: {stdout}"
    );
}
