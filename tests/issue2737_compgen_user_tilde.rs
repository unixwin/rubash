//! rubash#2737 — `compgen -A user -- '~ro'` matched nothing:
//! the tilde-prefixed word was compared literally against the
//! username database. GNU's rl_username_completion_function
//! (lib/readline/complete.c:2377) treats the first character as a
//! prefix sigil: it strips it for the database match and
//! re-applies it to every generated candidate, so '~ro' yields
//! '~root' (probed on GNU bash 5.3.20; the plain 'ro' control
//! yields 'root' on both shells).

use std::process::Command;

fn run(script: &str) -> (Vec<String>, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
        output.status.code(),
    )
}

#[cfg(unix)]
#[test]
fn tilde_word_matches_usernames_and_reapplies_tilde() {
    let (lines, code) = run("compgen -A user -- '~ro'");
    assert!(
        lines.iter().any(|line| line == "~root"),
        "'~ro' must offer '~root': {lines:?}"
    );
    assert!(
        lines.iter().all(|line| line.starts_with('~')),
        "every candidate carries the re-applied tilde: {lines:?}"
    );
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn full_tilde_name_and_plain_control() {
    let (lines, code) = run("compgen -A user -- '~root'");
    assert_eq!(lines, vec!["~root".to_string()]);
    assert_eq!(code, Some(0));

    let (lines, code) = run("compgen -A user -- 'ro'");
    assert!(
        lines.iter().any(|line| line == "root"),
        "control: the plain word matches without a tilde: {lines:?}"
    );
    assert!(
        lines.iter().all(|line| !line.starts_with('~')),
        "control: no tilde is invented for a plain word: {lines:?}"
    );
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn tilde_word_with_no_match_fails() {
    let (lines, code) = run("compgen -A user -- '~zzzqq2611'");
    assert!(lines.is_empty(), "{lines:?}");
    assert_eq!(code, Some(1));
}
