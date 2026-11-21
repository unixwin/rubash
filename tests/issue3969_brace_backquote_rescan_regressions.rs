//! rubash#3969: GNU hands every brace product to word expansion as a
//! fresh word (braces.c:12750 brace_expand_word_list ->
//! subst.c:13062 shell_expand_word_list). A `{a..A}` sequence produces
//! the ASCII backquote (96) between `A`(65) and `a`(97), and when that
//! product is glued to further text the expansion scan's `` ` `` case
//! (subst.c:11568-11593) string_extracts to a closing backquote that
//! does not exist — `bad substitution: no closing "`" in …`, an
//! expand_word_error that abandons the whole word list
//! (subst.c:13093-13110). Only an isolated product ending in the
//! backquote stays literal (the bare-backquote compatibility exception,
//! subst.c:11574-11576, the #3968 behavior).
//!
//! Oracle: GNU bash 5.3.20 on WSL /usr/local/bin/bash, LF script files;
//! every expectation below was byte-compared there.

use std::process::Command;

fn run_c(script: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c probe")
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The command/word list the erroring product belongs to is discarded
/// (GNU exp_jump_to_top_level(DISCARD)), so `;`-joined text never runs;
/// the diagnostic carries the product's tail from the backquote.
fn assert_bad_substitution(script: &str, tail: &str) {
    let output = run_c(script);
    assert_eq!(stdout_of(&output), "", "stdout for {script:?}");
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains(&format!("bad substitution: no closing \"`\" in {tail}\n")),
        "stderr for {script:?}: {stderr:?}"
    );
    assert_eq!(
        stderr.matches("bad substitution").count(),
        1,
        "expected exactly one diagnostic for {script:?}: {stderr:?}"
    );
}

#[test]
fn isolated_backquote_product_stays_literal() {
    // #3968 contract: `{a..A}` alone is 33 words; index 2 (descending
    // from 'a') is the literal backquote, and the trailing bare `` ` ``
    // product is not an unclosed substitution.
    let output = run_c("set -- {a..A}; echo \"n=$# second=<$2>\"");
    assert_eq!(stdout_of(&output), "n=33 second=<`>\n");
    assert_eq!(stderr_of(&output), "");
    let output = run_c("a=({a..A}); echo \"n=${#a[@]} e1=<${a[1]}>\"");
    assert_eq!(stdout_of(&output), "n=33 e1=<`>\n");
    assert_eq!(stderr_of(&output), "");
}

#[test]
fn backquote_product_glued_to_parameter_fails() {
    assert_bad_substitution("set -- {a..A}$x", "`$x");
    assert_bad_substitution("set -- {a..A}${x}", "`${x}");
}

#[test]
fn backquote_product_glued_to_literal_fails() {
    assert_bad_substitution("set -- {a..A}x", "`x");
    assert_bad_substitution("set -- {a..A}xy", "`xy");
    // The single-quoted tail is word text the scan must read raw: the
    // `'` inside the extraction is ordinary data (string_extract,
    // subst.c:791).
    assert_bad_substitution("set -- {a..A}'x'", "`'x'");
    // An escaped backquote in the postamble cannot close the generated
    // one either: `\x` is one escaped character for the scan.
    assert_bad_substitution("set -- {a..A}\\`", "`\\`");
}

#[test]
fn backquote_product_glued_to_second_brace_fails() {
    assert_bad_substitution("set -- {a..A}{1..2}", "`1");
    assert_bad_substitution("set -- {a..A}{a..A}", "`a");
    // The tail starts at the backquote (string+t_index), not the product.
    assert_bad_substitution("set -- {1..2}{a..A}x", "`x");
}

#[test]
fn backquote_product_glued_to_comsub_or_dquote_fails() {
    assert_bad_substitution("set -- {a..A}$(echo hi)", "`$(echo hi)");
    assert_bad_substitution("set -- {a..A}\"x\"", "`\"x\"");
}

#[test]
fn products_before_the_erroring_one_are_not_emitted() {
    // GNU expands products in order and DISCARDs the whole list: the
    // `a`/`b` words before the backquote element never reach `set --`.
    assert_bad_substitution("set -- {A..z}x; echo n=$#", "`x");
    let output = run_c("set -- {a..A}x; echo n=$#\necho next");
    // The same `;`-joined echo is discarded with the word list; the next
    // command line still runs.
    assert_eq!(stdout_of(&output), "next\n");
}

#[test]
fn compound_array_assignment_fails_whole() {
    // The pre-fix bug paired the generated backquote ACROSS products and
    // executed the middle text (`1: command not found`); GNU abandons
    // the whole assignment instead.
    assert_bad_substitution("a=({a..A}{1..2}); echo n=${#a[@]}", "`1");
    assert_bad_substitution("a=({a..A}x); echo n=${#a[@]}", "`x");
    // declare-family compound args: the parens are token delimiters, so
    // the diagnostic tail ends at the body product, `)` excluded.
    assert_bad_substitution("declare -a h=({a..A}x); echo n=${#h[@]}", "`x");
    assert_bad_substitution("declare -a i=(p{a..A}q); echo n=${#i[@]}", "`q");
}

#[test]
fn for_select_compgen_and_redirect_lists_fail() {
    assert_bad_substitution("for w in {a..A}x; do echo $w; done; echo s=$?", "`x");
    assert_bad_substitution("compgen -W '{a..A}x' >/dev/null; echo s=$?", "`x");
    assert_bad_substitution("echo hi > {a..A}y; echo s=$?", "`y");
}

#[test]
fn balanced_backquote_pairs_in_products_still_execute() {
    // A generated backquote that CLOSES is a real command substitution;
    // the verdict only covers the unclosed case.
    let output = run_c("set -- {a..c}`echo hi`; echo \"n=$#\"; printf '%s\\n' \"$@\"");
    assert_eq!(stdout_of(&output), "n=3\nahi\nbhi\nchi\n");
    assert_eq!(stderr_of(&output), "");
    // Backquotes inside a double-quoted postamble stay live and close.
    let output = run_c("set -- {a..c}\"x`echo Q`y\"; echo \"n=$#\"; printf '%s\\n' \"$@\"");
    assert_eq!(stdout_of(&output), "n=3\naxQy\nbxQy\ncxQy\n");
    assert_eq!(stderr_of(&output), "");
}

#[test]
fn words_without_backquote_products_are_unaffected() {
    let output = run_c("set -- {a..c}x; echo \"n=$# $*\"");
    assert_eq!(stdout_of(&output), "n=3 ax bx cx\n");
    assert_eq!(stderr_of(&output), "");
    let output = run_c("x=XX; set -- {a..c}$x; echo \"n=$# $*\"");
    assert_eq!(stdout_of(&output), "n=3 aXX bXX cXX\n");
    assert_eq!(stderr_of(&output), "");
    // Prefix-glued products keep the bare-trailing-backquote exception:
    // `x{a..A}` ends each product in the backquote — literal data.
    let output = run_c("set -- x{a..A}; echo \"n=$# second=<$2>\"");
    assert_eq!(stdout_of(&output), "n=33 second=<x`>\n");
    assert_eq!(stderr_of(&output), "");
}

// ---------------------------------------------------------------------------
// rubash#3969 graft (captain stage B follow-up): the argv[0]-`sh` class
// boundary. GNU bash invoked as `sh` binds POSIXLY_CORRECT, and that is
// exactly where the two bad-substitution classes part ways (oracle:
// GNU bash 5.3.20, script files):
//
//   * the `${...}` class is &expand_wdesc_fatal under posixly_correct
//     (subst.c:10277-10288): the script ABORTS, exit 1 (`-c`: 127);
//   * this PR's brace-product `` ` `` class returns the plain
//     &expand_wdesc_error in every mode (subst.c:11568-11593): DISCARD,
//     the script CONTINUES, exit 0 (`-c`: the list is abandoned, exit 1).
//
// The un-grafted PR latched its verdict on the shared
// parameter_bad_substitution flag, whose generic consumers apply the
// `${...}` class's FORCE_EOF arm — under the `sh` name the title shapes
// aborted the whole script (exit 1, `-c` exit 127). The graft latches
// the discard-only class marker with the verdict. The two title pins
// below are fail-first against the un-grafted head; the `${x!y}` pin is
// the same-class guard (green on both heads by design — it stops an
// over-broad "fix" that would downgrade the `${...}` class too).

fn sh_probe_dir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rubash3969-sh-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create sh probe dir");
    let sh_path = dir.join(if cfg!(windows) { "sh.exe" } else { "sh" });
    std::fs::copy(env!("CARGO_BIN_EXE_rubash"), &sh_path).expect("copy binary as sh");
    dir
}

/// Run a script FILE under the argv[0] name `sh`.
fn run_sh_script(script: &str) -> std::process::Output {
    let dir = sh_probe_dir();
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let sh_path = dir.join(if cfg!(windows) { "sh.exe" } else { "sh" });
    let output = Command::new(&sh_path)
        .arg(&script_path)
        .output()
        .expect("run sh script probe");
    let _ = std::fs::remove_dir_all(&dir);
    output
}

/// Run a `-c` string under the argv[0] name `sh`.
fn run_sh_c(script: &str) -> std::process::Output {
    let dir = sh_probe_dir();
    let sh_path = dir.join(if cfg!(windows) { "sh.exe" } else { "sh" });
    let output = Command::new(&sh_path)
        .arg("-c")
        .arg(script)
        .output()
        .expect("run sh -c probe");
    let _ = std::fs::remove_dir_all(&dir);
    output
}

/// Under the `sh` name a brace-product failure is DISCARDed exactly as
/// under the `bash` name: the diagnostic prints once, the failing
/// command produces nothing, the NEXT script line runs, exit 0.
fn assert_sh_discard_continues(script: &str, tail: &str) {
    let output = run_sh_script(script);
    assert_eq!(
        stdout_of(&output),
        "CONTINUED\n",
        "stdout for {script:?}"
    );
    assert_eq!(output.status.code(), Some(0), "status for {script:?}");
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("bad substitution: no closing ")
            && stderr.contains(&format!(" in {tail}\n")),
        "stderr for {script:?}: {stderr:?}"
    );
    assert_eq!(
        stderr.matches("bad substitution").count(),
        1,
        "expected exactly one diagnostic for {script:?}: {stderr:?}"
    );
}

#[test]
fn sh_name_brace_product_failure_discards_and_continues() {
    assert_sh_discard_continues("echo {a..A}x\necho CONTINUED\n", "`x");
    assert_sh_discard_continues("echo {a..A}$x\necho CONTINUED\n", "`$x");
}

#[test]
fn sh_name_compound_assignment_failure_discards_and_continues() {
    // The assignment-only path consumes the verdict through its own
    // generic consumer (command_prepare) — same class gate.
    assert_sh_discard_continues("a=({a..A}x)\necho CONTINUED\n", "`x");
    assert_sh_discard_continues("declare -a i=(p{a..A}q)\necho CONTINUED\n", "`q");
}

#[test]
fn sh_name_c_form_brace_product_failure_exits_1_not_127() {
    // In `-c` form GNU's DISCARD abandons the whole string with the
    // failed command's status 1; 127 is the FORCE_EOF mapping and
    // belongs to the `${...}` class alone.
    let output = run_sh_c("echo {a..A}x; echo CONTINUED");
    assert_eq!(stdout_of(&output), "", "stdout: {:?}", stdout_of(&output));
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("bad substitution: no closing ") && stderr.contains(" in `x\n"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn sh_name_parameter_bad_substitution_stays_fatal() {
    // Same-class guard: the `${...}` class keeps its POSIX FORCE_EOF
    // semantics under the `sh` name — the script aborts with status 1
    // and the continuation line never runs (GNU-as-sh, oracle-verified).
    // Green on the un-grafted head too; it fences the graft's scope.
    let output = run_sh_script("echo ${x!y}\necho CONTINUED\n");
    assert_eq!(stdout_of(&output), "", "stdout: {:?}", stdout_of(&output));
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("${x!y}: bad substitution\n"),
        "stderr: {stderr:?}"
    );
    // And the `-c` form keeps the FORCE_EOF status mapping (127).
    let output = run_sh_c("echo ${x!y}; echo CONTINUED");
    assert_eq!(stdout_of(&output), "", "stdout: {:?}", stdout_of(&output));
    assert_eq!(output.status.code(), Some(127));
}
