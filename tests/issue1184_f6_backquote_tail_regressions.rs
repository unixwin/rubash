//! Issue unixwin/rubash#1184 f6 follow-up: a backquote body whose text
//! ends with an escaped backslash (`\\` in the raw body, one `\` after
//! GNU's extraction decode) must keep that backslash in the body's own
//! parse. GNU hands the extracted body string to parse_and_execute
//! as-is (subst.c:7143 command_substitute, subst.c:7413), so the
//! trailing `\` meets EOF there; read_token_word's backslash branch
//! keeps a `\` as literal word data unless the peeked CHARACTER is
//! '\n' — only then is the pair a line continuation (parse.y:5368-5396,
//! the '\n' arm at 5379-5384).
//!
//! The f6 routes subject these bodies to the #515 newline-terminated
//! body reader (the appended `\n` stands in for EOF in the body
//! reader's line accounting). Appending it after an active trailing
//! backslash manufactures a continuation GNU never parses and silently
//! eats the backslash: `` echo "[`echo a\\`]" `` printed `[a]` instead
//! of `[a\]`, and GNU's own tests/comsub2.sub line 2
//! (`${qpath//"`printf '%s' \\`"/}`) stopped stripping. The funnels now
//! withhold the terminator when the body ends with an odd-length
//! backslash run (see backtick_body_ends_with_active_backslash in
//! src/executor/command_substitution.rs).
//!
//! All expectations measured against GNU bash 5.3.20 script files
//! (local oracle build), 2026-10-31. The acceptance pins were red on
//! the composed f6 branch head 40239ab6 (the shape the Windows goldens
//! CI caught in upstream_suite_slice_snapshots / comsub) and are green
//! under the follow-up fix; controls were already GNU-true there.

use std::process::{Command, Stdio};

fn run_rubash_file(script: &str, tag: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!(
        "issue1184_f6_tail_{}_{}.sh",
        tag,
        std::process::id()
    ));
    std::fs::write(&path, script).expect("write probe script");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn assert_gnu(script: &str, tag: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = run_rubash_file(script, tag);
    assert_eq!(code, Some(0), "{tag}: exit code (stderr={stderr:?})");
    assert_eq!(stderr, "", "{tag}: stderr must stay empty");
    assert_eq!(stdout, expected_stdout, "{tag}: stdout");
}

/// The isolated atom (collector probe p3): inside a double-quoted word
/// with literal text around it, a backquote body ending in `\\` yields
/// the single decoded backslash.
#[test]
fn backquote_tail_escaped_backslash_inside_dquotes() {
    assert_gnu(r#"echo "[`printf '%s' \\`]""#, "p3", "[\\]\n");
}

/// Same atom with a word in the body (collector probe v4): the result
/// is `a\`, not `a` — exactly the final decoded backslash was eaten.
#[test]
fn backquote_tail_escaped_backslash_keeps_word_backslash() {
    assert_gnu(r#"echo "[`echo a\\`]""#, "v4", "[a\\]\n");
}

/// The comsub2.sub line-2 shape (collector probe p1): the pattern of
/// ${var//pat/} is produced by a `$(...)` whose text is a backquote
/// substitution ending in `\\`; the pattern is one backslash, so every
/// `\` in the value is stripped.
#[test]
fn backquote_tail_pattern_from_dollar_paren_form() {
    assert_gnu(
        "qpath='\\/tmp\\/foo\\/bar'\nprintf \"%s\\n\" ${qpath//\"`printf '%s' \\\\`\"/}\n",
        "p1",
        "/tmp/foo/bar\n",
    );
}

/// The mutations funnel (collector probe w2): the replacement side of
/// ${var/pat/repl} is a backquote body ending in `\\`; its output — a
/// single backslash — must survive into the result.
#[test]
fn backquote_tail_replacement_side_body() {
    assert_gnu(
        "s='aXb'\necho \"${s/X/`printf '%s' \\\\`}\"\n",
        "w2",
        "a\\b\n",
    );
}

/// GNU tests/comsub2.sub verbatim: the echo line plus all three
/// pattern-removal forms (plain, $() form, pure-backquote form).
#[test]
fn comsub2_sub_all_four_lines() {
    let script = "qpath='\\/tmp\\/foo\\/bar'\n\necho \"$qpath\"\n\nprintf \"%s\\n\" ${qpath//\\\\/}\nprintf \"%s\\n\" ${qpath//\"`printf '%s' \\\\`\"/}\nprintf \"%s\\n\" ${qpath//`printf '%s' \"\\\\\\\\\\\\\\\\\"`/}\n";
    assert_gnu(
        script,
        "comsub2",
        "\\/tmp\\/foo\\/bar\n/tmp/foo/bar\n/tmp/foo/bar\n/tmp/foo/bar\n",
    );
}

/// Control: an even-length trailing backslash run still takes the
/// terminated path — the decoded body `echo a\\` (two backslashes)
/// parses to the word `a\`, terminator or not.
#[test]
fn backquote_tail_even_backslash_run_control() {
    assert_gnu(r#"echo "[`echo a\\\\`]""#, "even", "[a\\]\n");
}

/// Control: the whole-word assignment route (never routed through the
/// embedded walkers) keeps the trailing backslash as before.
#[test]
fn backquote_tail_assignment_route_control() {
    assert_gnu(
        "x=`printf '%s' \\\\`\necho \"[$x]\"\n",
        "assign",
        "[\\]\n",
    );
}

/// Control: a mid-body escaped backslash is untouched either way.
#[test]
fn backquote_mid_body_escaped_backslash_control() {
    assert_gnu(r#"echo "[`echo \\x`]""#, "mid", "[x]\n");
}
