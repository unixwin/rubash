//! rubash#2987 regressions: a background job is respawned as a child
//! process whose `-c` source serializes the whole function table plus
//! the job body through `bash_command_source_text`. That renderer's
//! conditional arm joined the COOKED `[[ ]]` args (quote markers
//! stripped at cond-arg collection) with spaces, so any function whose
//! body carried a quoted conditional operand corrupted the child's
//! ENTIRE parse:
//!
//!   `[[ "$1" != '|' ]]`  ->  `[[ $1 != | ]]`
//!       ("unexpected argument `|' to conditional binary operator")
//!   `[[ "$1" == "" ]]`   ->  `[[ $1 == ]]`
//!       ("unexpected argument `]]' to conditional binary operator")
//!   `[[ $x == "p q" ]]`  ->  `[[ $x == p q ]]` (syntax error)
//!   `[[ "$1" == "a*" ]]` ->  `[[ $1 == a* ]]` (parses, but the quoted
//!       literal silently becomes a pattern — wrong semantics)
//!
//! The child died before executing one command. In bats-core this is
//! exactly how the per-test timeout watchdog dies: bats-exec-test's
//! own function table carries such conditionals (e.g.
//! test_functions.bash's `[[ "$current_command_or_arg" != '|' ]]`),
//! so the countdown subshell `( ... ) &` never ran, no TERM was ever
//! sent, and `BATS_TEST_TIMEOUT=1` tests slept their full duration
//! and passed (timeout.bats 1 ok / 3 not-ok under rubash, 4/0 GNU).
//!
//! The for/select word lists in the same renderer had the identical
//! cooked-join defect (`for i in "a b" c` respawned as
//! `for i in a b c`). GNU prints every one of these word lists from
//! the stored word text, quotes intact: print_cmd.c:881
//! print_cond_node (`cond->op->word`), print_cmd.c:586
//! command_print_word_list, :602 print_for_command_head, :656
//! print_select_command_head — the same raw contract the case arm of
//! this renderer already keeps (rubash#1704). GNU's async path never
//! re-serializes at all: execute_cmd.c:701 runs the job through a
//! single make_child fork that inherits the parsed table whole.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2987-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2987_respawn_quoted_pipe_operand_survives() {
    // The bats test_functions.bash shape: a quoted '|' as the RHS of
    // a binary conditional operator, in a function the respawned
    // child only CALLS (the prelude carries every function).
    let (stdout, stderr, code) = rubash_file(
        "f() { if [[ \"$1\" != '|' ]]; then echo \"not-pipe\"; else echo \"pipe\"; fi }\n( f '|' ) &\nwait\n( f x ) &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "pipe\nnot-pipe\n");
}

#[test]
fn issue2987_respawn_quoted_empty_operand_survives() {
    let (stdout, stderr, code) = rubash_file(
        "g() { if [[ \"$1\" == \"\" ]]; then echo \"empty\"; else echo \"nonempty\"; fi }\n( g \"\" ) &\nwait\n( g y ) &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "empty\nnonempty\n");
}

#[test]
fn issue2987_respawn_quoted_space_and_pattern_operands_survive() {
    // A quoted space must stay ONE operand; a quoted glob must stay a
    // LITERAL (unquoted re-parse would silently make it a pattern).
    let (stdout, stderr, code) = rubash_file(
        "f() { [[ \"$1\" == \"p q\" ]] && echo \"space-eq\" || echo \"space-ne\"; [[ \"$2\" == \"a*\" ]] && echo \"literal-star\" || echo \"pattern-star\"; }\n( f \"p q\" 'a*' ) &\nwait\n( f \"p q\" abc ) &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "space-eq\nliteral-star\nspace-eq\npattern-star\n");
}

#[test]
fn issue2987_respawn_regex_alternation_and_escaped_space_survive() {
    let (stdout, stderr, code) = rubash_file(
        "f() { [[ \"$1\" =~ ^(a|b)$ ]] && echo \"alt-yes\" || echo \"alt-no\"; [[ \"$1\" =~ ^a\\ b$ ]] && echo \"esc-yes\" || echo \"esc-no\"; }\n( f 'a b' ) &\nwait\n( f c ) &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "alt-no\nesc-yes\nalt-no\nesc-no\n");
}

#[test]
fn issue2987_respawn_unary_and_bang_operands_survive() {
    let (stdout, stderr, code) = rubash_file(
        "f() { [[ -n \"$1\" && \"$2\" == '!' ]] && echo \"both\" || echo \"not-both\"; [[ ! -e /nonexistent-rubash-2987 ]] && echo \"neg-ok\"; }\n( f x '!' ) &\nwait\n( f '' '!' ) &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "both\nneg-ok\nnot-both\nneg-ok\n");
}

#[test]
fn issue2987_respawn_inline_subshell_conditional_survives() {
    // No function call: the job BODY itself carries the conditional
    // (the absorbed-paren respawn path renders the body directly).
    let (stdout, stderr, code) = rubash_file(
        "x=\"\"\n( [[ $x == \"\" ]] && echo \"inline-empty\" ) &\nwait\n( [[ $x != '|' ]] && echo \"inline-pipe\" ) &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "inline-empty\ninline-pipe\n");
}

#[test]
fn issue2987_respawn_for_word_list_quoting_survives() {
    let (stdout, stderr, code) = rubash_file(
        "f() { for i in \"a b\" c; do echo \"for:[$i]\"; done }\n( f ) &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "for:[a b]\nfor:[c]\n");
}

#[test]
fn issue2987_respawn_select_word_list_quoting_survives() {
    let (stdout, _, code) = rubash_file(
        "f() { select s in \"a b\" c; do echo \"sel:[$s]\"; break; done <<< \"1\"; }\n( f ) &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "sel:[a b]\n");
}

#[test]
fn issue2987_watchdog_countdown_subshell_fires() {
    // The bats_start_timeout_countdown shape in miniature: a
    // background countdown subshell containing its own background
    // sleeper, a TERM trap and a `wait`, which after the wait calls
    // a FUNCTION from the parent's table. A sibling function with a
    // quoted conditional operand rides the same prelude (never
    // called) — on the broken build its serialization alone killed
    // the countdown child's parse, so FIRED never printed.
    let (stdout, stderr, code) = rubash_file(
        "fired() { echo \"COUNTDOWN-FIRED\"; }\npoison() { [[ \"$1\" != '|' ]] && echo poison; }\n(\n  (eval exec {0..255}\">&-\"; sleep 1) &\n  trap \"kill $!; exit 0\" TERM\n  wait\n  trap '' TERM\n  fired\n) &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "COUNTDOWN-FIRED\n");
}

#[test]
fn issue2987_watchdog_term_reaches_main_shell() {
    // The full bats chain: the countdown fires after ~1s and TERMs
    // the main shell (whose $$ the respawned child still reports),
    // the main shell's TERM trap runs and exits 42 instead of
    // sleeping the full 5 seconds.
    let (stdout, stderr, code) = rubash_file(
        "watchdog() {\n  (\n    (eval exec {0..255}\">&-\"; sleep 1) &\n    trap \"kill $!; exit 0\" TERM\n    wait\n    trap '' TERM\n    kill -TERM $$\n  ) &\n}\npoison() { [[ \"$1\" != '|' ]] && echo poison; }\ntrap 'echo MAIN-TERMED; exit 42' TERM\nwatchdog\nsleep 5\necho MAIN-SLEPT-FULL\n",
    );
    assert_eq!(code, Some(42));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "MAIN-TERMED\n");
}
