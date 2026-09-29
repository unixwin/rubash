//! Issue rubash#316 residue regression: a word with quoted segments around
//! an UNQUOTED glob character must stay pathname-expansion eligible even
//! when it starts and ends with a quote and contains `${` — the
//! fully-quoted-word sentinel (STORAGE_WORD_PREFIX) used to claim such
//! words because it tested only the first/last characters.
//!
//! GNU subst.c expand_word_internal globs by per-character quote flags:
//! `"$DIR"/*"${empty}"` (bash-it's reloader shape with an empty type
//! variable) has a bare `*` between the quoted segments, so the for-list
//! iterates the matched files — both on the first loop and on every later
//! loop with different values of the same variable.

use std::io::Write;
use std::process::Command;

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i316-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("enabled")).expect("create temp dir");
    for name in ["a.aliases.bash", "b.aliases.bash", "c.completion.bash"] {
        std::fs::File::create(dir.join("enabled").join(name)).expect("fixture file");
    }
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).expect("create probe");
    let script = script.replace(
        "@DIR@",
        &dir.join("enabled")
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/"),
    );
    file.write_all(script.as_bytes()).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Quoted-empty at word END (`"$DIR"/*"${empty}"`) must glob — the sentinel
/// used to make it the literal `*`.
#[test]
fn quoted_empty_at_word_end_still_globs() {
    let (stdout, stderr, rc) = run_script(
        "DIR=@DIR@\nempty=\nfor f in \"$DIR\"/*\"${empty}\"; do echo \"H:${f##*/}\"; done\n",
    );
    assert_eq!(
        stdout, "H:a.aliases.bash\nH:b.aliases.bash\nH:c.completion.bash\n",
        "stderr: {stderr}"
    );
    assert_eq!(rc, Some(0));
}

/// The reloader sequence: same-shape loop with non-empty values before the
/// empty one (aliases, completion, then empty) — every loop globs.
#[test]
fn reloader_sequence_all_loops_glob() {
    let (stdout, stderr, rc) = run_script(
        "DIR=@DIR@\n_t=aliases\nfor f in \"$DIR\"/*\"${_t}.bash\"; do echo \"A:${f##*/}\"; done\n_t=completion\nfor f in \"$DIR\"/*\"${_t}.bash\"; do echo \"B:${f##*/}\"; done\n_t=\nfor f in \"$DIR\"/*\"${_t}.bash\"; do echo \"C:${f##*/}\"; done\n",
    );
    assert_eq!(
        stdout,
        "A:a.aliases.bash\nA:b.aliases.bash\nB:c.completion.bash\nC:a.aliases.bash\nC:b.aliases.bash\nC:c.completion.bash\n",
        "stderr: {stderr}"
    );
    assert_eq!(rc, Some(0));
}

/// Glob-free fully-quoted `${...}` words keep the sentinel behavior
/// (`"${v:-~}"` tilde2.tests shape must not regress).
#[test]
fn glob_free_quoted_brace_word_keeps_literal_tilde() {
    let (stdout, stderr, rc) = run_script("v=\nfor f in \"${v:-~}\"; do echo \"T:$f\"; done\n");
    // `~` from a QUOTED default stays a literal tilde (tilde2.tests).
    assert_eq!(stdout, "T:~\n", "stderr: {stderr}");
    assert_eq!(rc, Some(0));
}

/// Unquoted glob with a NON-empty quoted tail still globs (matrix shape A
/// with matching files).
#[test]
fn nonempty_quoted_tail_still_globs() {
    let (stdout, stderr, rc) = run_script(
        "DIR=@DIR@\nnx=aliases\nfor f in \"$DIR\"/*\"${nx}\".bash; do echo \"N:${f##*/}\"; done\n",
    );
    assert_eq!(
        stdout, "N:a.aliases.bash\nN:b.aliases.bash\n",
        "stderr: {stderr}"
    );
    assert_eq!(rc, Some(0));
}
