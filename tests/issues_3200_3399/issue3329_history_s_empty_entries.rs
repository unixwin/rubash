//! rubash#3329 — `history -s ''` (and whitespace-only arguments) silently
//! dropped the entry. GNU records it as a real (empty) history entry:
//! it occupies a history number, is listed by `history` / `fc -l`,
//! participates in HISTSIZE throttling, is written by `history -w`, and
//! is the target of `!!` / `!-1` expansion.
//!
//! GNU model (third_party/bash, Bash-5.3 patch 20):
//! - `builtins/history.def:421-451` push_history: `string_list(list)`
//!   (subst.c:2854, joins all words with " ", empty words included)
//!   goes through `check_add_history(s, 1)` unconditionally.
//! - `bashhist.c:789-807` check_add_history: the only filters are
//!   `check_history_control` and `history_should_ignore` — there is no
//!   emptiness check. `check_history_control` (bashhist.c:694-708) drops
//!   only a line whose first character is exactly `' '` under
//!   ignorespace/ignoreboth; a tab-leading line is kept.
//! - Emptiness filtering lives at the line-recording call sites, not in
//!   the shared gate: `bashhist.c:643` pre_process_line records only
//!   `*return_value` (non-empty), `builtins/fc.def:770-780` fc_replhist
//!   returns early on an empty command.
//! - Bare `history -s` is a no-op success: `history.def:175-178`
//!   `if (list) push_history (list); return (EXECUTION_SUCCESS);`
//!
//! Expectations below were verified against GNU bash 5.3.20 (script file,
//! /tmp/gnu-bash-3349/bash) with the probe matrix in the issue.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3329-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_rubash(script: &str, dir: &PathBuf) -> (String, String, i32) {
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn history_s_empty_entry_is_listed_and_expands() {
    let dir = tempdir("list");
    let (stdout, stderr, _) = run_rubash(
        "history -s ''\nhistory\nhistory -p '!!' >/dev/null 2>&1; echo \"expand rc=$?\"\n",
        &dir,
    );
    assert_eq!(stdout, "    1  \nexpand rc=0\n", "stderr: {stderr}");
}

#[test]
fn history_s_empty_entry_participates_in_numbering() {
    let dir = tempdir("numbering");
    let (stdout, stderr, _) = run_rubash(
        "history -s 'echo a'; history -s ''; history -s 'echo b'; history\n",
        &dir,
    );
    assert_eq!(
        stdout, "    1  echo a\n    2  \n    3  echo b\n",
        "stderr: {stderr}"
    );
}

#[test]
fn history_s_empty_entry_participates_in_histsize_throttling() {
    let dir = tempdir("histsize");
    let (stdout, stderr, _) = run_rubash(
        "HISTSIZE=1; history -s 'echo a'; history -s ''; history\n",
        &dir,
    );
    // GNU keeps only the empty entry, still numbered 2.
    assert_eq!(stdout, "    2  \n", "stderr: {stderr}");
}

#[test]
fn history_s_empty_entry_is_written_by_w() {
    let dir = tempdir("write");
    let histfile = dir.join("hist.txt");
    let (stdout, stderr, _) = run_rubash(
        &format!("history -s ''\nhistory -w {}\n", histfile.display()),
        &dir,
    );
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(
        std::fs::read(&histfile).expect("read histfile"),
        b"\n",
        "history -w must write one empty line for the empty entry"
    );
}

#[test]
fn history_s_empty_entry_visible_to_fc_and_deletable() {
    let dir = tempdir("fc");
    let (stdout, stderr, _) = run_rubash(
        "history -s ''\nfc -l\nhistory -d 1; echo \"d rc=$?\"\nhistory | wc -l\n",
        &dir,
    );
    assert_eq!(stdout, "1\t \nd rc=0\n0\n", "stderr: {stderr}");
}

#[test]
fn history_s_empty_entry_is_minus_one_expansion_target() {
    let dir = tempdir("minusone");
    let (stdout, stderr, _) = run_rubash(
        "history -s 'echo a'; history -s ''; history -p '!-1'; echo \"expand rc=$?\"\n",
        &dir,
    );
    assert_eq!(stdout, "\nexpand rc=0\n", "stderr: {stderr}");
}

#[test]
fn history_s_whitespace_only_args_are_recorded() {
    let dir = tempdir("ws");
    let (stdout, stderr, _) =
        run_rubash("history -s ' '; history -s '\t'; history | wc -l\n", &dir);
    assert_eq!(stdout, "2\n", "stderr: {stderr}");
}

#[test]
fn history_s_empty_words_join_with_spaces() {
    let dir = tempdir("join");
    let (stdout, stderr, _) = run_rubash(
        "history -s a '' b; history\nhistory -c; history -s '' ''; history\n",
        &dir,
    );
    assert_eq!(stdout, "    1  a  b\n    1   \n", "stderr: {stderr}");
}

#[test]
fn history_s_bare_is_a_noop_success() {
    let dir = tempdir("bare");
    let (stdout, stderr, _) =
        run_rubash("history -s; echo \"bare rc=$?\"\nhistory | wc -l\n", &dir);
    assert_eq!(stdout, "bare rc=0\n0\n", "stderr: {stderr}");
}

#[test]
fn history_s_multi_arg_concat_control() {
    let dir = tempdir("concat");
    let (stdout, stderr, _) = run_rubash("history -s a b c; history\n", &dir);
    assert_eq!(stdout, "    1  a b c\n", "stderr: {stderr}");
}

#[test]
fn history_s_ignorespace_keeps_tab_drops_space() {
    let dir = tempdir("ignorespace");
    let (stdout, stderr, _) = run_rubash(
        "HISTCONTROL=ignorespace; history -s '\t'; history | wc -l; history -s ' x'; history | wc -l; history -s ' '; history | wc -l\n",
        &dir,
    );
    // GNU check_history_control drops only *line == ' '; a tab-leading
    // line is kept even under ignorespace.
    assert_eq!(stdout, "1\n1\n1\n", "stderr: {stderr}");
}

#[test]
fn history_s_ignoredups_dedups_empty() {
    let dir = tempdir("ignoredups");
    let (stdout, stderr, _) = run_rubash(
        "HISTCONTROL=ignoredups; history -s ''; history -s ''; history | wc -l\n",
        &dir,
    );
    assert_eq!(stdout, "1\n", "stderr: {stderr}");
}

#[test]
fn history_s_erasedups_removes_prior_empty() {
    let dir = tempdir("erasedups");
    let (stdout, stderr, _) = run_rubash(
        "HISTCONTROL=erasedups; history -s ''; history -s 'echo a'; history -s ''; history\n",
        &dir,
    );
    assert_eq!(stdout, "    1  echo a\n    2  \n", "stderr: {stderr}");
}
