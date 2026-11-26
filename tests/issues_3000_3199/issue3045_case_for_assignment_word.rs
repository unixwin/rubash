//! rubash#3045 — `${var=word}` assignment is lost in case-subject /
//! for-word-list expansion.
//!
//! Root cause: the `${var=word}` bind lives in the `&mut` apply pass
//! (`Executor::apply_parameter_assignment_expansions_in_word`,
//! `src/executor/parameter_words.rs`), which the plain-argument path
//! (`expand_command_word`, `command_prepare.rs`) and the case-pattern
//! path (`expand_word_mut`) both run before rendering — but the case
//! subject (`expand_case_word`, `shell_options.rs`) and the for/select
//! word list (`expand_for_word_values_impl`, `command_words.rs`) render
//! through the `&self` `expand_word` only, so the value comes out right
//! while the bind never happens.
//!
//! GNU anchors: the case subject goes through
//! `expand_word_leave_quoted` (execute_cmd.c:3686) and the for list
//! through `expand_words_no_vars` (execute_cmd.c:3014) →
//! `expand_word_list_internal` (subst.c:13219) → `shell_expand_word_list`
//! (subst.c:13062) — the normal expansion pipeline in both cases, which
//! runs `parameter_brace_expand` (subst.c:9777) whose `case '='`
//! (subst.c:10346) calls `parameter_brace_expand_rhs` (subst.c:7966)
//! and binds via `bind_variable`. There is no GNU path on which the
//! assignment is a render-only side effect.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions. `LC_ALL=C.UTF-8` is set on the child explicitly so the
//! pin is hermetic under any ambient locale.

use std::process::{Command, Stdio};

fn run(script: &str) -> Vec<u8> {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let output = child.wait_with_output().expect("wait rubash");
    assert!(
        output.status.success(),
        "rubash -c {script:?} exited with {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn expect(script: &str, expected: &str, ctx: &str) {
    let out = run(script);
    assert_eq!(
        out,
        expected.as_bytes(),
        "{ctx}: got {:?}, want {expected:?}",
        String::from_utf8_lossy(&out)
    );
}

#[test]
fn case_subject_assign_persists() {
    expect(
        "unset -v v; case ${v=bar} in (bar) echo matched;; esac; echo \"v=[$v]\"",
        "matched\nv=[bar]\n",
        "case subject ${v=bar} must bind v",
    );
}

#[test]
fn case_subject_assign_colon_persists() {
    expect(
        "unset -v v; case ${v:=bar} in (bar) echo matched;; esac; echo \"v=[$v]\"",
        "matched\nv=[bar]\n",
        "case subject ${v:=bar} must bind v",
    );
}

#[test]
fn case_subject_assign_persists_under_nounset() {
    expect(
        "set -u; unset -v v; case ${v=bar} in (bar) :;; esac; echo \"v=[$v]\"",
        "v=[bar]\n",
        "case subject bind must survive a set -u re-read",
    );
}

#[test]
fn for_word_list_assign_persists() {
    expect(
        "unset -v v; for x in ${v=forw}; do :; done; echo \"v=[$v]\"",
        "v=[forw]\n",
        "for word list ${v=forw} must bind v",
    );
}

#[test]
fn for_word_list_assign_colon_persists() {
    expect(
        "unset -v v; for x in ${v:=forw}; do :; done; echo \"v=[$v]\"",
        "v=[forw]\n",
        "for word list ${v:=forw} must bind v",
    );
}

#[test]
fn harden_shape_case_word_condition_reread_in_else() {
    // modernish harden.mm:301/337: the assignment sits in a case-word
    // condition and the variable is re-read in the else branch.
    expect(
        "unset -v v; P=set; \
         if case ${v=qux} in (qux) ;; (*) ! : ;; esac && [ -z \"${P+set}\" ]; then echo then; \
         else case ${v} in (qux) echo \"else v=[$v]\";; (*) echo \"else other\";; esac; fi",
        "else v=[qux]\n",
        "harden shape: case-word assignment must be visible in the else branch",
    );
}

#[test]
fn case_subject_left_to_right_freeze() {
    // GNU evaluates a word's fragments strictly left-to-right: the
    // earlier `${a:-d}` resolves against the pre-assignment value.
    expect(
        "unset -v a; case \"${a:-d}/${a:=x}\" in (d/x) echo \"ok a=[$a]\";; (*) echo BAD;; esac",
        "ok a=[x]\n",
        "earlier fragment in the case subject must see the pre-assignment value",
    );
}

#[test]
fn for_word_list_left_to_right_freeze() {
    expect(
        "unset -v a; for x in \"${a:-d}\" \"${a:=x}\"; do echo \"x=[$x]\"; done; echo \"a=[$a]\"",
        "x=[d]\nx=[x]\na=[x]\n",
        "earlier for-list word must see the pre-assignment value",
    );
}

#[test]
fn case_subject_comsub_alternate_runs_once() {
    // The apply pass expands the alternate and the render must reuse
    // the memoized value, not re-execute the command substitution.
    expect(
        "unset -v v; f=cnt-$$.txt; rm -f \"$f\"; \
         case ${v:=$(echo x >>\"$f\"; echo got)} in (got) echo \"v=[$v] n=$(wc -l <\"$f\")\";; esac; rm -f \"$f\"",
        "v=[got] n=1\n",
        "case subject comsub alternate must run exactly once",
    );
}

#[test]
fn for_word_list_comsub_alternate_runs_once() {
    expect(
        "unset -v v; f=cnt-$$.txt; rm -f \"$f\"; \
         for x in ${v:=$(echo x >>\"$f\"; echo got)}; do echo \"x=[$x] v=[$v] n=$(wc -l <\"$f\")\"; done; rm -f \"$f\"",
        "x=[got] v=[got] n=1\n",
        "for-list comsub alternate must run exactly once",
    );
}

#[test]
fn procsub_body_assign_does_not_leak_argv() {
    // GNU subst.c:6362 process_substitute forks (subst.c:6412
    // make_child FORK_PROCSUB); the body runs in the child, so its
    // assignment must not reach the parent. Failed on base
    // (argv apply pass bound the body text); the scan fix in
    // scan_word_prefix_quote_state covers `<(...)` like `$(...)`.
    expect(
        "unset -v v; : <(echo \"${v=leak}\" >/dev/null); echo \"v=[${v-unset}]\"",
        "v=[unset]\n",
        "argv <(...) body assignment must not leak into the parent",
    );
}

#[test]
fn procsub_body_assign_does_not_leak_case() {
    // Guard for the case-subject apply pass added here: it scans the
    // raw subject word, so without the `<(...)` scan fix it would bind
    // the body text in the parent (base had no leak only because case
    // subjects never ran the apply pass at all).
    expect(
        "unset -v v; case <(echo \"${v=leak}\" >/dev/null) in (*) echo \"v=[${v-unset}]\";; esac",
        "v=[unset]\n",
        "case subject <(...) body assignment must not leak into the parent",
    );
}

#[test]
fn procsub_body_assign_does_not_leak_case_output() {
    expect(
        "unset -v v; case >(echo \"${v=leak}\" >/dev/null) in (*) echo \"v=[${v-unset}]\";; esac",
        "v=[unset]\n",
        "case subject >(...) body assignment must not leak into the parent",
    );
}

#[test]
fn procsub_body_assign_does_not_leak_for() {
    // The for path applies post-materialization (the word is already a
    // /dev/fd pathname), so the body text is never scanned; guard.
    expect(
        "unset -v v; for x in <(echo \"${v=leak}\" >/dev/null); do :; done; echo \"v=[${v-unset}]\"",
        "v=[unset]\n",
        "for-list <(...) body assignment must not leak into the parent",
    );
}

#[test]
fn for_word_list_assign_mixed_positional_at() {
    // The `$@`/array-at early arms in expand_for_word_values_impl
    // return before the brace path, so a `${v=...}` affix on a word
    // they claim lost its bind until the apply pass moved to the top
    // of the impl (rubash#3045). GNU binds the affix (the word-list
    // expansion is one left-to-right pass, subst.c:13219).
    expect(
        "unset -v v; set -- A B; for x in ${v=mix}\"$@\"; do :; done; echo \"v=[$v]\"",
        "v=[mix]\n",
        "for-list ${v=mix}\"$@\" must bind v",
    );
}

#[test]
fn for_word_list_assign_mixed_array_at() {
    expect(
        "unset -v v; a=(P Q); for x in ${v=mix}\"${a[@]}\"; do :; done; echo \"v=[$v]\"",
        "v=[mix]\n",
        "for-list ${v=mix}\"${a[@]}\" must bind v",
    );
}

#[test]
fn for_word_list_assign_after_positional_at() {
    expect(
        "unset -v v; set -- A B; for x in \"$@\"${v=mix}; do :; done; echo \"v=[$v]\"",
        "v=[mix]\n",
        "for-list \"$@\"${v=mix} must bind v",
    );
}

#[test]
fn for_word_list_assign_mixed_unquoted_array_at() {
    expect(
        "unset -v v; a=(P Q); for x in ${v=mix}${a[@]}; do :; done; echo \"v=[$v]\"",
        "v=[mix]\n",
        "for-list ${v=mix}${a[@]} must bind v",
    );
}

#[test]
fn for_word_list_assign_after_array_at() {
    expect(
        "unset -v v; a=(P Q); for x in \"${a[@]}\"${v=mix}; do :; done; echo \"v=[$v]\"",
        "v=[mix]\n",
        "for-list \"${a[@]}\"${v=mix} must bind v",
    );
}

#[test]
fn for_word_list_assign_mixed_unset_array_at() {
    expect(
        "unset -v v; unset -v a; for x in ${v=mix}${a[@]}; do :; done; echo \"v=[$v]\"",
        "v=[mix]\n",
        "for-list ${v=mix}${a[@]} with unset a must bind v",
    );
}

#[test]
fn assign_after_comsub_with_quoted_span_case() {
    // rubash#3045: the quote-state scan's in-double `"` arm did not
    // consume the closing quote, so a fragment after a comsub body
    // containing a `"..."` span read as inside_cs and the apply pass
    // skipped it. The comsub's own `${v=x}` binds in the child (parent
    // v stays unset); the glued `${w=y}` must bind in the parent.
    expect(
        "unset -v v w; case $(echo \"${v=x}\")${w=y} in (x*) echo \"v=[${v-unset}] w=[$w]\";; esac",
        "v=[unset] w=[y]\n",
        "case subject fragment after a comsub with a quoted span must still bind",
    );
}

#[test]
fn assign_after_comsub_with_quoted_span_for() {
    expect(
        "unset -v v w; for x in $(echo \"${v=x}\")${w=y}; do :; done; echo \"v=[${v-unset}] w=[$w]\"",
        "v=[unset] w=[y]\n",
        "for-list fragment after a comsub with a quoted span must still bind",
    );
}

#[test]
fn case_pattern_assign_still_persists() {
    // Already worked before the fix (case patterns go through the
    // mutable expander); guard against regression.
    expect(
        "unset -v v; case foo in ${v=foo}) echo \"v=[$v]\";; esac",
        "v=[foo]\n",
        "case pattern ${v=foo} must keep binding v",
    );
}

#[test]
fn plain_argument_assign_still_persists() {
    // Already worked before the fix (argv path runs the apply pass);
    // guard against regression.
    expect(
        "unset -v v; echo ${v=plain} >/dev/null; echo \"v=[$v]\"",
        "v=[plain]\n",
        "plain argument ${v=plain} must keep binding v",
    );
}
