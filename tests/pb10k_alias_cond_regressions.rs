//! Alias expansion must not leak into `[[ ]]` conditional words, extglob
//! pattern words, or case patterns (wt56/alias-cond; niubash + oh-my-bash
//! with `alias ..='cd ..'` live).
//!
//! Repro chain: sourcing bash_completion under an interactive shell with
//! oh-my-bash's `alias ..='cd ..'` died at bash_completion line 1376 —
//!
//! ```text
//! bash_completion: line 1376: syntax error in conditional expression: unexpected token `../'
//!     if [[ $cur != ?(*/)_omb_directories_cd ../ ]]; then
//! ```
//!
//! i.e. rubash spliced `cd ..` into the `?(*/)..` extglob pattern word.
//!
//! GNU spec:
//! - The `[[ ]]` interior is read by parse_cond_command's recursive
//!   descent (read_token's cond branch parse.y:3586-3604), whose direct
//!   read_token calls bypass yylex — the ONLY `last_read_token'
//!   bookkeeper (parse.y:3076-3078). last_read_token stays frozen at
//!   COND_START, which reserved_word_acceptable never accepts, so
//!   alias_expand_token (parse.y:3249, gate :3254) never fires inside
//!   `[[ ]]` — not after `(`, `)`, `&&`, `||`, or an extglob `X(`.
//! - parse.y:5464-5477: with extglob on, `X(` (PATTERN_CHAR syntax.h:90)
//!   consumes its matched `(...)` into the SAME word (parse_matched_pair),
//!   so extglob fragments outside `[[ ]]` (case patterns, argument words,
//!   assignment RHS) are one token and never alias candidates either.
//! - parse.y:3481-3484: `]]' returns COND_END on PST_CONDEXPR alone,
//!   before alias expansion — so the token after `]]` IS a command
//!   position again and must still expand.
//!
//! Verified byte-for-byte against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-10-02, artifacts under
//! target/issue-suites/results/wt56-aliascond/). The `-i` piped-stdin
//! shape is the tests/issue297_300 + pb10k pattern; the probes turn
//! `expand_aliases` on themselves so the cases do not depend on the
//! interactive default.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(stdin: &[u8]) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The owner's production line (bash_completion:1376 shape): the real
/// alias `..='cd ..'` must not splice into the `?(*/)..` extglob pattern
/// on the rhs of `!=' inside `[[ ]]` — GNU prints HIT, a leaked splice
/// is a `syntax error in conditional expression` instead.
#[test]
fn cond_extglob_rhs_production_shape_survives_a_live_dots_alias() {
    let (stdout, stderr) = run_stdin(
        b"shopt -s expand_aliases\n\
          alias ..='cd ..'\n\
          cur=xx\n\
          if [[ $cur != ?(*/).. ]]; then echo HIT; else echo MISS; fi\n",
    );
    assert_eq!(stdout, "HIT\n");
    assert!(
        !stderr.contains("syntax error"),
        "stderr leaked the alias splice: {stderr}"
    );
    assert!(
        !stderr.contains("../"),
        "stderr leaked the expanded alias text: {stderr}"
    );
}

/// Interior words of `[[ ]]` are never alias candidates — not the lhs,
/// not a plain rhs, not after `(`/`)` grouping, not after `&&`/`||`, not
/// a regex operand. GNU prints the pattern-true side for each; any splice
/// would be a conditional syntax error.
#[test]
fn cond_interior_alias_words_stay_literal() {
    let (stdout, stderr) = run_stdin(
        b"shopt -s expand_aliases\n\
          alias zz='echo ZZ'\n\
          [[ zz == zz ]] && echo L1-TRUE || echo L1-FALSE\n\
          [[ ( zz == zz ) ]] && echo L2-TRUE || echo L2-FALSE\n\
          [[ a == b || zz == zz ]] && echo L3-TRUE || echo L3-FALSE\n\
          [[ a == b && zz == zz ]] && echo L4-TRUE || echo L4-FALSE\n\
          [[ ! zz == zz ]] && echo L5-TRUE || echo L5-FALSE\n\
          [[ abc =~ zz ]] && echo L6-TRUE || echo L6-FALSE\n",
    );
    let expected = "L1-TRUE\nL2-TRUE\nL3-TRUE\nL4-FALSE\nL5-FALSE\nL6-FALSE\n";
    assert_eq!(stdout, expected);
    assert!(
        !stderr.contains("syntax error"),
        "stderr leaked a cond splice: {stderr}"
    );
}

/// Over-reach guard (parse.y:3484 + parse.y:3157 via COND_END in
/// reserved_word_acceptable): the command AFTER `]]` is a real command
/// position and must still expand — GNU runs the alias there.
#[test]
fn command_after_cond_end_still_expands() {
    let (stdout, stderr) = run_stdin(
        b"shopt -s expand_aliases\n\
          alias zz='echo ZZ'\n\
          [[ a == b ]]; zz\n\
          [[ a == a ]] && zz\n",
    );
    assert_eq!(stdout, "ZZ\nZZ\n");
    assert!(
        !stderr.contains("command not found"),
        "alias failed to expand after ]]: {stderr}"
    );
}

/// Extglob pattern words outside `[[ ]]` are one token (parse.y:5464:
/// PATTERN_CHAR + parse_matched_pair), so their interior and tail are
/// never alias candidates: case patterns keep matching, `@(alias)/tail`
/// echoes literally, and an assignment RHS keeps the pattern text.
/// `shopt -s extglob` precedes the case statement because GNU parses it
/// with the flag live at read time.
#[test]
fn extglob_pattern_words_stay_literal_with_alias_names_inside() {
    let (stdout, stderr) = run_stdin(
        b"shopt -s expand_aliases\n\
          shopt -s extglob\n\
          alias zz='echo ZZ'\n\
          case zz in ?(a)|zz) echo PAT ;; *) echo STAR ;; esac\n\
          echo @(zz)/tail\n\
          x=?(zz)file\n\
          printf 'x: [%s]\\n' \"$x\"\n",
    );
    let expected = "PAT\n@(zz)/tail\nx: [?(zz)file]\n";
    assert_eq!(stdout, expected);
    assert!(
        !stderr.contains("syntax error") && !stderr.contains("command not found"),
        "stderr leaked: {stderr}"
    );
}
