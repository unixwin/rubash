//! rubash#2573 — the `-o nospace` / `-o noquote` / `-o filenames` /
//! `-o fullquote` / `-o nosort` compspec options were parsed and
//! printed (`complete -p` round-tripped them) but never applied:
//! `Session::complete` returned bare candidate texts, so no host
//! could suppress the appended space, skip filename quoting, or
//! preserve generation order.
//!
//! GNU ownership: bashline.c calls pcomp_set_readline_variables
//! (pcomplete.c:1458-1477) with the USED compspec's options after
//! generation — COPT_FILENAMES → rl_filename_completion_desired,
//! COPT_FULLQUOTE → rl_full_quoting_desired, COPT_NOSPACE →
//! rl_completion_suppress_append, COPT_NOQUOTE →
//! rl_filename_quoting_desired inverted, COPT_NOSORT →
//! rl_sort_completion_matches inverted. The port adds the policy
//! channel: `Session::complete_detailed` returns a
//! `CompletionOutcome { candidates, policy }` (Session::complete
//! keeps its signature and delegates), the engine honors nosort
//! itself (generation order instead of the sorted stream), and the
//! policy derives from the EFFECTIVE options — compopt mutations
//! included (rubash#2572's options_override).

use rubash::api::{CompletionPolicy, RunOptions, Session};

fn session_with(script: &str) -> Session {
    let mut session = Session::builder().build().expect("session builds");
    session
        .run(script, &RunOptions::default())
        .expect("registration script runs");
    session
}

#[test]
fn nospace_reaches_the_policy() {
    let mut session = session_with("complete -o nospace -W 'zzone zztwo' mycmd");
    let outcome = session.complete_detailed("mycmd zz", 8);
    assert_eq!(
        outcome
            .candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["zzone", "zztwo"]
    );
    assert!(
        outcome.policy.suppress_append,
        "-o nospace -> suppress_append"
    );
    assert!(outcome.policy.filename_quoting_desired);
    assert!(outcome.policy.sort_matches);
    assert!(!outcome.policy.filename_completion_desired);
}

#[test]
fn nosort_preserves_generation_order() {
    let mut session = session_with("complete -o nosort -W 'zztwo zzone' mycmd");
    let outcome = session.complete_detailed("mycmd zz", 8);
    assert_eq!(
        outcome
            .candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["zztwo", "zzone"],
        "nosort: the wordlist's generation order is the result, unsorted"
    );
    assert!(
        !outcome.policy.sort_matches,
        "-o nosort -> sort_matches false"
    );

    // Control: the same wordlist without nosort comes back sorted.
    let mut session = session_with("complete -W 'zztwo zzone' mycmd");
    let outcome = session.complete_detailed("mycmd zz", 8);
    assert_eq!(
        outcome
            .candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["zzone", "zztwo"]
    );
    assert!(outcome.policy.sort_matches);
}

#[test]
fn noquote_filenames_fullquote_faces() {
    let mut session =
        session_with("complete -o noquote -o filenames -o fullquote -W 'zzone' mycmd");
    let outcome = session.complete_detailed("mycmd zz", 8);
    assert!(
        !outcome.policy.filename_quoting_desired,
        "-o noquote clears filename_quoting_desired"
    );
    assert!(outcome.policy.filename_completion_desired);
    assert!(outcome.policy.full_quoting_desired);
    assert!(!outcome.policy.suppress_append);
}

#[test]
fn compopt_mutation_reaches_the_policy() {
    let mut session =
        session_with("_f() { compopt -o nospace; COMPREPLY=(zzone); }; complete -F _f mycmd");
    let outcome = session.complete_detailed("mycmd zz", 8);
    assert!(
        outcome.policy.suppress_append,
        "a compopt -o nospace inside the completion function must reach the policy (the registered spec has no nospace)"
    );
}

#[test]
fn no_spec_attempt_reports_the_default_policy() {
    let mut session = session_with(":");
    let outcome = session.complete_detailed("zz", 2);
    assert_eq!(outcome.policy, CompletionPolicy::default());
}

#[test]
fn complete_keeps_its_signature_and_candidates() {
    let mut session = session_with("complete -o nospace -W 'zzone zztwo' mycmd");
    let plain = session.complete("mycmd zz", 8);
    let detailed = session.complete_detailed("mycmd zz", 8);
    assert_eq!(plain, detailed.candidates);
}
