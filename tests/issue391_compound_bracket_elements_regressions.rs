//! rubash#391: compound-assignment SPACED bracket elements —
//! `arr=([ "a]=b" ] = value [ empty ] =)` kept every bracket word literal
//! and split it on the spaces; GNU 5.3.0 stores `<[ a]=b ]><=><value><e>`
//! in a cwd containing a file `e`.
//!
//! GNU spec: parse.y:5635-5651 read_token_word under PST_COMPASSIGN — an
//! element-LEADING `[` (token_index == 0, parse.y:5637) consumes the
//! matched `[...]` span via parse_matched_pair (P_ARRAYSUB) and copies it
//! into the word with strcpy (parse.y:5645-5647): the span's interior —
//! quoted `]`s and spaces included — is word DATA, and the word continues
//! past the closer (`[a b]*` ends at the next shellbreak). Without a
//! following `=` the element is a plain bracket-glob word whose pathname
//! expansion runs at storage (expand_words_no_vars, arrayfunc.c:610 +
//! subst.c:12590: quote removal + glob, no field splitting — a no-match
//! pattern stays literal).
//!
//! Byte-verified against WSL GNU Bash 5.3.0 SCRIPT-FILE probes with a file
//! `e` in the cwd (artifacts:
//! target/issue-suites/results/wt39-caseerr/issue391-matrix/, 2026-10-02).
//! The `-c` string-eval pipeline keeps a separate element-splitting layer
//! (residual, tracked in the lane report); the script-file form is the
//! canonical AGENTS.md baseline shape.

use std::process::Command;

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join("issue391-probe");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::remove_file(dir.join("e"));
    std::fs::write(dir.join("e"), b"").expect("create file e");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
    )
}

#[test]
fn issue_reproducer_spaced_elements_glob_and_stay_whole() {
    let (stdout, stderr, code) = rubash_script(
        "arr=([ \"a]=b\" ] = value [ empty ] =)\nprintf '<%s>' \"${arr[@]}\"; echo\n",
    );
    assert_eq!(stderr, "");
    assert_eq!(stdout, "<[ a]=b ]><=><value><e><=>\n");
    assert_eq!(code, Some(0));
}

#[test]
fn bare_bracket_word_globs_to_one_char_files() {
    let (stdout, stderr, code) =
        rubash_script("arr=([ empty ] =)\nprintf '<%s>' \"${arr[@]}\"; echo\n");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "<e><=>\n");
    assert_eq!(code, Some(0));

    // A matching bracket pattern stores the MATCH (`e`), not the pattern.
    let (stdout, _, _) = rubash_script("arr=([ empty ])\ndeclare -p arr\n");
    assert!(stdout.contains("declare -a arr=([0]=\"e\")"), "{stdout}");
}

#[test]
fn no_match_bracket_pattern_stays_one_literal_element() {
    // `[a b]*` as a pattern matches nothing in the probe cwd (no one-char
    // `a`/`b`/space file): GNU keeps the whole word literal, ONE element.
    let (stdout, stderr, code) = rubash_script("arr=([a b]*)\nprintf '<%s>' \"${arr[@]}\"; echo\n");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "<[a b]*>\n");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_bracket_subscript_elements_still_work() {
    // The `[k]=v` class must not regress: the `=` directly after the
    // matched `]` still forms a subscripted element (parse.y:5637), and a
    // space inside the VALUE is element data.
    let (stdout, stderr, code) = rubash_script(
        "arr=([0]=one [1]=two [2]=\"spaced value\")\nprintf '<%s>' \"${arr[@]}\"; echo\n",
    );
    assert_eq!(stderr, "");
    assert_eq!(stdout, "<one><two><spaced value>\n");
    assert_eq!(code, Some(0));

    // An INVALID arithmetic subscript (`a b`) is the evaluator's error on
    // both shells (GNU: `arithmetic syntax error in expression (error
    // token is "b")`, empty array).
    let (stdout, stderr, _) = rubash_script("arr=([a b]=3)\ndeclare -p arr\n");
    assert!(
        stderr.contains("arithmetic syntax error in expression"),
        "{stderr}"
    );
    assert!(stdout.contains("declare -a arr=()"), "{stdout}");
}

#[test]
fn simple_glob_elements_still_expand() {
    // Pre-existing class neighbors: `e` stays, quoted `*` stays literal,
    // extglob groups keep their spaces (rubash#389).
    let (stdout, _, _) = rubash_script("arr=(e)\nprintf '<%s>' \"${arr[@]}\"; echo\n");
    assert_eq!(stdout, "<e>\n");
    let (stdout, _, _) = rubash_script("arr=('*')\nprintf '<%s>' \"${arr[@]}\"; echo\n");
    assert_eq!(stdout, "<*>\n");
    let (stdout, _, code) = rubash_script(
        "shopt -s extglob\narr=([ empty ] = @(e))\nprintf '<%s>' \"${arr[@]}\"; echo\nshopt -u extglob\n",
    );
    assert_eq!(stdout, "<e><=><e>\n");
    assert_eq!(code, Some(0));
}
