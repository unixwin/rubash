//! rubash#2856 (B5-11) — the `complete -E` / `-I` host gate was
//! unreachable, and the engine's own blank-line corners diverged:
//! W1 (`"   "` at end-of-line took `-E`, GNU takes `-I`/default),
//! W2 (`"   ls"` at column 0 took `-I`, GNU takes `-E`), W3 (an
//! empty line with only `-I` registered resolved to nothing, GNU
//! takes `-I` via the bashline.c post-chain arm).
//!
//! GNU ownership: bashline.c `attempt_shell_completion` (~1681-1810)
//! — arm 3 (`start == 0 && start < s1`, cursor at column 0 with the
//! command name after whitespace) and arm 4 (`e == 0`, beginning of
//! an empty line) call `programmable_completions (EMPTYCMD, …)`;
//! the post-chain arm (`in_command_position && foundcs == 0 &&
//! iw_compspec`, :1785-1792) runs `-I` when the EMPTYCMD call found
//! nothing. `-D` never governs the command word (arms 2/6 set
//! `foundcs = 0` and only the post-chain `-I` arm follows).
//! Probed on GNU bash 5.3.20 (prep B5-11, b01-b18, 18/18 PASS).
//!
//! Pins run at the `Executor::complete_line` level; the "default"
//! shapes (b04/b09/b16/b17, arm-8b) assert the spec word is absent
//! from the builtin command completion — with a non-empty guard so
//! the absence is not vacuous.

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
}

fn assert_no_spec_word(candidates: &[String], word: &str, what: &str) {
    assert!(
        !candidates.is_empty(),
        "{what}: the default arm must run (empty list would make the absence vacuous)"
    );
    assert!(
        !candidates.iter().any(|c| c == word),
        "{what}: {word:?} must not appear: {candidates:?}"
    );
}

#[test]
fn b511_empty_line_empty_compspec_governs() {
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -E -W 'eone etwo'");
    assert_eq!(
        executor.complete_line("", 0),
        vec!["eone".to_string(), "etwo".to_string()],
        "b01: empty line + -E"
    );
}

#[test]
fn b511_empty_line_default_compspec_governs() {
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -D -W 'dalpha dbeta'");
    assert_eq!(
        executor.complete_line("", 0),
        vec!["dalpha".to_string(), "dbeta".to_string()],
        "b03: empty line + -D"
    );
}

#[test]
fn b511_empty_line_empty_beats_default() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "complete -E -W 'eone etwo'; complete -D -W 'dalpha dbeta'",
    );
    assert_eq!(
        executor.complete_line("", 0),
        vec!["eone".to_string(), "etwo".to_string()],
        "b05: empty line + -E + -D: only the -E words"
    );
}

#[test]
fn b511_empty_line_initial_word_only() {
    // W3: an empty line with only -I registered resolves to -I
    // (GNU post-chain arm), not to nothing.
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -I -W 'ione itwo'");
    assert_eq!(
        executor.complete_line("", 0),
        vec!["ione".to_string(), "itwo".to_string()],
        "b14: empty line + only -I"
    );
}

#[test]
fn b511_command_word_initial_governs() {
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -I -W 'ione itwo'");
    assert_eq!(
        executor.complete_line("i", 1),
        vec!["ione".to_string(), "itwo".to_string()],
        "b02: command word + -I"
    );
}

#[test]
fn b511_command_word_initial_beats_default() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "complete -I -W 'ione itwo'; complete -D -W 'dalpha dbeta'",
    );
    assert_eq!(
        executor.complete_line("i", 1),
        vec!["ione".to_string(), "itwo".to_string()],
        "b06: command word + -I + -D: only the -I words"
    );
}

#[test]
fn b511_command_word_default_never_governs() {
    // Q-D: -D never governs the command word.
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -D -W 'dalpha dbeta'");
    let candidates = executor.complete_line("d", 1);
    assert_no_spec_word(&candidates, "dalpha", "b04: command word + only -D");
}

#[test]
fn b511_argument_word_default_governs() {
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -D -W 'dalpha dbeta'");
    assert_eq!(
        executor.complete_line("zzcmd d", 6),
        vec!["dalpha".to_string(), "dbeta".to_string()],
        "b07: argument word of an unregistered command + -D (both -W words share the 'd' prefix)"
    );
}

#[test]
fn b511_argument_word_own_spec_beats_default() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "complete -W 'gfoo gbar' mycmd; complete -D -W 'dalpha dbeta'",
    );
    assert_eq!(
        executor.complete_line("mycmd g", 7),
        vec!["gbar".to_string(), "gfoo".to_string()],
        "b08: argument word + own spec + -D: only the own words (text-sorted)"
    );
}

#[test]
fn b511_whitespace_line_eol_empty_does_not_govern() {
    // W1: a whitespace-only line with the cursor at end-of-line
    // takes -I/default, never -E.
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -E -W 'eone etwo'");
    let candidates = executor.complete_line("   ", 3);
    assert_no_spec_word(&candidates, "eone", "b09: whitespace line EOL + -E");
}

#[test]
fn b511_whitespace_line_eol_initial_governs() {
    // W1, the -I half: same shape with only -I registered.
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -I -W 'ione itwo'");
    assert_eq!(
        executor.complete_line("   ", 3),
        vec!["ione".to_string(), "itwo".to_string()],
        "b13: whitespace line EOL + -I"
    );
}

#[test]
fn b511_whitespace_line_col0_empty_beats_initial() {
    // W2: column 0 with the command name after whitespace takes -E.
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "complete -E -W 'eone etwo'; complete -I -W 'ione itwo'",
    );
    assert_eq!(
        executor.complete_line("   ", 0),
        vec!["eone".to_string(), "etwo".to_string()],
        "b15: whitespace line col 0 + -E + -I: only the -E words"
    );
}

#[test]
fn b511_name_after_whitespace_col0_empty_governs() {
    // W2: "   ls" at column 0 takes -E, not -I.
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "complete -E -W 'eone etwo'; complete -I -W 'ione itwo'",
    );
    assert_eq!(
        executor.complete_line("   ls", 0),
        vec!["eone".to_string(), "etwo".to_string()],
        "b11: '   ls' col 0 + -E + -I: only the -E words"
    );
}

#[test]
fn b511_name_after_whitespace_col0_initial_without_empty() {
    // W2, the -I half: no -E registered, -I governs.
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -I -W 'ione itwo'");
    assert_eq!(
        executor.complete_line("   ls", 0),
        vec!["ione".to_string(), "itwo".to_string()],
        "b12: '   ls' col 0 + only -I"
    );
}

#[test]
fn b511_name_start_default_never_governs() {
    // Q-D at the name start (arm 2): -D never governs.
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -D -W 'dalpha dbeta'");
    let candidates = executor.complete_line("d", 0);
    assert_no_spec_word(&candidates, "dalpha", "b16: name start + only -D");
}

#[test]
fn b511_name_start_empty_never_governs() {
    // -E does not govern the name start either.
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -E -W 'eone etwo'");
    let candidates = executor.complete_line("d", 0);
    assert_no_spec_word(&candidates, "eone", "b17: name start + only -E");
}

#[test]
fn b511_name_start_initial_governs() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "complete -E -W 'eone etwo'; complete -I -W 'ione itwo'",
    );
    assert_eq!(
        executor.complete_line("d", 0),
        vec!["ione".to_string(), "itwo".to_string()],
        "b18: name start + -E + -I: only the -I words"
    );
}

#[test]
fn b511_separator_position_initial_governs() {
    // Arm 8: an empty command name after a separator takes -I.
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -I -W 'ione itwo'");
    assert_eq!(
        executor.complete_line("echo hi; ", 9),
        vec!["ione".to_string(), "itwo".to_string()],
        "arm 8: 'echo hi; ' + -I"
    );
}

#[test]
fn b511_separator_position_empty_never_governs() {
    // Arm 8 with only -E: -E does not govern a separator-followed
    // command position.
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -E -W 'eone etwo'");
    let candidates = executor.complete_line("echo hi; ", 9);
    assert_no_spec_word(&candidates, "eone", "arm 8: 'echo hi; ' + only -E");
}

// --- The position-aware gate itself: `Executor::has_compspec_at`
// must agree with the generation path on every shape above. ---

fn gate_executor() -> Executor {
    Executor::new()
}

#[test]
fn b511_gate_empty_line_shapes() {
    let mut executor = gate_executor();
    // b10: nothing registered — the gate stays shut.
    assert!(!executor.has_compspec_at("", 0));
    run_script(&mut executor, "complete -E -W 'eone etwo'");
    assert!(executor.has_compspec_at("", 0), "b01: empty line + -E");
    let mut executor = gate_executor();
    run_script(&mut executor, "complete -D -W 'dalpha dbeta'");
    assert!(executor.has_compspec_at("", 0), "b03: empty line + -D");
    let mut executor = gate_executor();
    run_script(&mut executor, "complete -I -W 'ione itwo'");
    assert!(
        executor.has_compspec_at("", 0),
        "b14: empty line + only -I (W3)"
    );
}

#[test]
fn b511_gate_command_word_shapes() {
    let mut executor = gate_executor();
    run_script(&mut executor, "complete -D -W 'dalpha dbeta'");
    assert!(
        !executor.has_compspec_at("d", 1),
        "b04/Q-D: -D never governs the command word"
    );
    assert!(
        !executor.has_compspec_at("d", 0),
        "b16/Q-D: -D never governs the name start"
    );
    let mut executor = gate_executor();
    run_script(&mut executor, "complete -I -W 'ione itwo'");
    assert!(executor.has_compspec_at("i", 1), "b02: command word + -I");
    assert!(executor.has_compspec_at("d", 0), "b18: name start + -I");
    let mut executor = gate_executor();
    run_script(&mut executor, "complete -E -W 'eone etwo'");
    assert!(
        !executor.has_compspec_at("d", 0),
        "b17: -E never governs the name start"
    );
}

#[test]
fn b511_gate_whitespace_line_shapes() {
    let mut executor = gate_executor();
    run_script(&mut executor, "complete -E -W 'eone etwo'");
    assert!(
        !executor.has_compspec_at("   ", 3),
        "b09/W1: whitespace line EOL + -E: nothing governs"
    );
    assert!(
        executor.has_compspec_at("   ", 0),
        "b15/W2: whitespace line col 0 + -E"
    );
    assert!(
        executor.has_compspec_at("   ls", 0),
        "b11/W2: name after whitespace at col 0 + -E"
    );
    let mut executor = gate_executor();
    run_script(&mut executor, "complete -I -W 'ione itwo'");
    assert!(
        executor.has_compspec_at("   ", 3),
        "b13/W1: whitespace line EOL + -I"
    );
    assert!(
        executor.has_compspec_at("   ls", 0),
        "b12/W2: name after whitespace at col 0 + -I"
    );
}

#[test]
fn b511_gate_argument_and_separator_shapes() {
    let mut executor = gate_executor();
    run_script(&mut executor, "complete -D -W 'dalpha dbeta'");
    assert!(
        executor.has_compspec_at("zzcmd d", 6),
        "b07: argument word + -D"
    );
    assert!(
        !executor.has_compspec_at("echo hi; ", 9),
        "arm 8: separator position + only -D: -D never governs a fresh command word"
    );
    let mut executor = gate_executor();
    run_script(
        &mut executor,
        "complete -W 'gfoo gbar' mycmd; complete -D -W 'dalpha dbeta'",
    );
    assert!(
        executor.has_compspec_at("mycmd g", 7),
        "b08: argument word + own spec"
    );
    let mut executor = gate_executor();
    run_script(&mut executor, "complete -I -W 'ione itwo'");
    assert!(
        executor.has_compspec_at("echo hi; ", 9),
        "arm 8: separator position + -I"
    );
}

#[test]
fn b511_gate_progcomp_off_shuts_the_gate() {
    // The gate and the generation path agree: with progcomp off
    // no compspec governs (bashline.c:1681).
    let mut executor = gate_executor();
    run_script(&mut executor, "complete -E -W 'eone etwo'");
    assert!(executor.has_compspec_at("", 0));
    run_script(&mut executor, "shopt -u progcomp");
    assert!(
        !executor.has_compspec_at("", 0),
        "progcomp off: the gate stays shut even with -E registered"
    );
    let candidates = executor.complete_line("", 0);
    assert!(
        !candidates.contains(&"eone".to_string()),
        "progcomp off: generation agrees with the gate: {candidates:?}"
    );
}
