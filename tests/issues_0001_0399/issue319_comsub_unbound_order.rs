//! Issue rubash#319 regression: under `set -u`, a word (or word list) with a
//! command substitution to the LEFT of an unbound parameter must show the
//! child's side effects before the parent's fatal `unbound variable`.
//!
//! GNU expands left-to-right: subst.c:11229 expand_word_internal forks each
//! command substitution as the walk reaches it (subst.c:7143
//! command_substitute), so the child's stderr passes through before the walk
//! hits the later `${nope}` and raises the nounset fatal (probes q5a/q5c/q5e
//! of the fix16 lane). An unbound parameter to the LEFT of the comsub never
//! lets the comsub run (probe q5d).

use std::io::Write;
use std::process::Command;

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i319-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).expect("create probe");
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

/// Strip the invocation-dependent `<path>: ` prolog from each stderr line.
fn normalized(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(|line| match line.find(": line ") {
            Some(idx) => line[idx + 2..].to_string(),
            None => line.to_string(),
        })
        .collect()
}

/// `w=$(echo hi >&2)${nope}` — the child's `hi` precedes the fatal
/// diagnostic, both on stderr, rc 1.
#[test]
fn comsub_left_of_unbound_prints_child_output_first() {
    let (stdout, stderr, rc) = run_script("set -u\nw=$(echo hi >&2)${nope}\necho done\n");
    assert_eq!(stdout, "", "stdout");
    assert_eq!(
        normalized(&stderr),
        vec![
            "hi".to_string(),
            "line 2: nope: unbound variable".to_string(),
        ],
        "stderr"
    );
    assert_eq!(rc, Some(1));
}

/// Multiple child writes stay in order before the diagnostic.
#[test]
fn comsub_two_child_writes_precede_unbound() {
    let (stdout, stderr, rc) = run_script("set -u\nw=$(echo hi >&2; echo err >&2)${nope}\n");
    assert_eq!(stdout, "", "stdout");
    assert_eq!(
        normalized(&stderr),
        vec![
            "hi".to_string(),
            "err".to_string(),
            "line 2: nope: unbound variable".to_string(),
        ],
        "stderr"
    );
    assert_eq!(rc, Some(1));
}

/// Argument position across words: `echo $(...) ${nope}` — the earlier
/// word's comsub still runs before the later word's unbound fatal.
#[test]
fn comsub_in_earlier_word_runs_before_later_unbound() {
    let (stdout, stderr, rc) = run_script("set -u\necho $(echo hi >&2) ${nope}\n");
    assert_eq!(stdout, "", "stdout");
    assert_eq!(
        normalized(&stderr),
        vec![
            "hi".to_string(),
            "line 2: nope: unbound variable".to_string(),
        ],
        "stderr"
    );
    assert_eq!(rc, Some(1));
}

/// Unbound to the LEFT of the comsub: the comsub never runs (GNU probe q5d).
#[test]
fn unbound_left_of_comsub_never_runs_comsub() {
    let (stdout, stderr, rc) = run_script("set -u\nw=a${nope}$(echo hi >&2)\n");
    assert_eq!(stdout, "", "stdout");
    assert_eq!(
        normalized(&stderr),
        vec!["line 2: nope: unbound variable".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(1));
}

/// Without nounset nothing changes: the word expands normally.
#[test]
fn without_nounset_word_expands_normally() {
    let (stdout, stderr, rc) = run_script("w=$(echo -n hi)${nope}\necho \"[$w]\"\n");
    assert_eq!(stdout, "[hi]\n", "stdout");
    assert_eq!(stderr, "", "stderr");
    assert_eq!(rc, Some(0));
}
