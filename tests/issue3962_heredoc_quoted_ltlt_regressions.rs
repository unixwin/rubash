//! Issue unixwin/rubash#3962: `exit` inside a sourced file running in a
//! subshell stopped the rest of the parent script when the sourced file
//! held a multi-line string containing `<<`:
//!
//! ```sh
//! # g.sh:  r="\n<<x\n"\nexit 0
//! ( . ./g.sh )
//! echo AFTER          # GNU prints AFTER; rubash printed nothing.
//! ```
//!
//! GNU contract: read_token lexes `<<` at token level only — the
//! `case '<'` arm (parse.y:3692-3701) returning LESS_LESS /
//! LESS_LESS_MINUS / LESS_LESS_LESS, each later registered by
//! push_heredoc (parse.y:3106). Inside a word, quote or substitution
//! body — the state read_token_word (parse.y:5305) carries across a
//! physical line — `<<` is literal text and declares nothing.
//!
//! Root cause: the driver-level heredoc-declaration scan
//! (`scan_heredoc_operators_full`) reset its quoting state on every
//! physical line, so a `<<` on a line that began inside an open `"`/`'`
//! declared a phantom heredoc. The body never arrived, the pending-group
//! gatherer (read_next_source_group) glued the rest of the file into one
//! incomplete unit, and the subshell's `exit` propagated through the
//! parent as if it had run top-level.
//!
//! The fix carries a context stack across physical lines —
//! `'…'`/`$'…'`/`"…"`/`` ` `` bodies, `$(`/`<(`/`>(`/bare `(` spans
//! (operator-live: a `<<` inside gathers from this stream, parse.y:3120
//! gather_here_documents), `${` bodies and `((`/`$((` arithmetic depth
//! (parse.y:3727-3728 parse_dparen; replaces the old lone `arith_depth`
//! i64) — so each line is judged by the context it actually begins in.
//!
//! Expectations byte-verified against GNU bash 5.3.x (`/usr/local/bin/bash`,
//! script files): every case below produced identical stdout and rc.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3962-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script_in(dir: &PathBuf, name: &str, script: &str) -> (String, String, Option<i32>) {
    let file = dir.join(name);
    std::fs::write(&file, script).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&file)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn issue3962_subshell_source_multiline_dquote_ltlt_then_parent_continues() {
    // The exact report: a multi-line double-quoted string containing `<<`
    // in a sourced file, `exit` inside a parenthesized subshell — the
    // parent must survive and print AFTER.
    let dir = tempdir("dquote");
    std::fs::write(dir.join("g.sh"), "r=\"\n<<x\n\"\nexit 0\n").unwrap();
    let (stdout, stderr, code) = run_script_in(&dir, "main.sh", "( . ./g.sh )\necho AFTER\n");
    assert_eq!(stdout, "AFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3962_subshell_source_multiline_squote_ltlt_then_parent_continues() {
    // Single-quoted multi-line string carrying `<<` — same phantom class.
    let dir = tempdir("squote");
    std::fs::write(dir.join("g.sh"), "r='\n<<x\n'\nexit 0\n").unwrap();
    let (stdout, stderr, code) = run_script_in(&dir, "main.sh", "( . ./g.sh )\necho AFTER\n");
    assert_eq!(stdout, "AFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3962_comsub_source_multiline_quote_ltlt_then_parent_continues() {
    // The command-substitution form from the report.
    let dir = tempdir("comsub");
    std::fs::write(dir.join("g.sh"), "r=\"\n<<x\n\"\nexit 0\n").unwrap();
    let (stdout, stderr, code) =
        run_script_in(&dir, "main.sh", "x=$( { . ./g.sh; } )\necho AFTER\n");
    assert_eq!(stdout, "AFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3962_subshell_source_multiline_quote_ltlt_without_exit() {
    // The quoted `<<` alone (no `exit`) also glued the group: the sourced
    // assignment completes, the subshell ends normally, parent continues.
    let dir = tempdir("noexit");
    std::fs::write(dir.join("g.sh"), "r=\"\n<<x\n\"\n").unwrap();
    let (stdout, stderr, code) = run_script_in(&dir, "main.sh", "( . ./g.sh )\necho AFTER\n");
    assert_eq!(stdout, "AFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3962_brace_expansion_multiline_ltlt_declares_nothing() {
    // A `<<` inside a multi-line `${}` word is expansion text, never an
    // operator (parse.y:3877 parse_matched_pair) — and the group must not
    // be glued open.
    let dir = tempdir("brace");
    let (stdout, stderr, code) = run_script_in(
        &dir,
        "main.sh",
        "x=${a-\n<<z}\necho \"got:${x}\"\necho AFTER\n",
    );
    assert_eq!(stdout, "got:\n<<z\nAFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3962_comsub_heredoc_across_lines_still_declares() {
    // `$(` is operator-live: a `<<` inside still declares a heredoc whose
    // body gathers from this input (parse.y:3120 gather_here_documents).
    let dir = tempdir("live-comsub");
    let (stdout, stderr, code) = run_script_in(
        &dir,
        "main.sh",
        "v=$(cat <<CV\nbody\nCV\n)\necho \"v=$v\"\necho AFTER\n",
    );
    assert_eq!(stdout, "v=body\nAFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3962_arith_shift_operators_still_suppressed() {
    // The carried arith depth's job still holds: `<<` inside `((`/`$((`
    // spanning lines is the shift operator (parse.y:3727-3728
    // parse_dparen), never a heredoc.
    let dir = tempdir("arith");
    let (stdout, stderr, code) = run_script_in(
        &dir,
        "main.sh",
        "((x = 1 <<\n3))\necho \"x=$x\"\necho AFTER\n",
    );
    assert_eq!(stdout, "x=8\nAFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3962_real_heredoc_plus_exit_in_sourced_subshell() {
    // Control: a REAL heredoc in the sourced file still gathers its body
    // and `exit` still terminates only the subshell.
    let dir = tempdir("real-heredoc");
    std::fs::write(dir.join("g.sh"), "cat <<EOT\nbody\nEOT\nexit 0\n").unwrap();
    let (stdout, stderr, code) = run_script_in(&dir, "main.sh", "( . ./g.sh )\necho AFTER\n");
    assert_eq!(stdout, "body\nAFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3962_oneline_quoted_ltlt_unaffected() {
    // One-line quoted `<<` was never broken — keep it that way.
    let dir = tempdir("oneline");
    std::fs::write(dir.join("g.sh"), "r=\"<<x\"\nexit 0\n").unwrap();
    let (stdout, stderr, code) = run_script_in(&dir, "main.sh", "( . ./g.sh )\necho AFTER\n");
    assert_eq!(stdout, "AFTER\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
