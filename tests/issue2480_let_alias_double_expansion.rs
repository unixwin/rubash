//! rubash#2480 — modernish `seq`: `alias let='let --'` double expansion
//! in respawned pipeline children.
//!
//! GNU contract, two pieces:
//!
//! 1. `builtins/let.def:74-75 let_builtin`: after CHECK_HELPOPT, `let`
//!    skips exactly ONE leading `--` argument
//!    (`if (list && list->word && ISOPTION (list->word->word, '-'))
//!    list = list->next;` — ISOPTION, builtins/common.h:26, is the exact
//!    two-character word). A `--` anywhere else is an arithmetic operand
//!    and fails to parse.
//!
//! 2. `parse.y:3249 alias_expand_token` expands alias words ONLY in the
//!    reader; a function body is alias-expanded at DEFINITION parse time
//!    and then frozen — `execute_cmd.c execute_function` runs the stored
//!    command tree, and no execution-time pass ever expands alias words
//!    again.
//!
//! modernish ships `alias let='let --'` (bin/modernish core aliases) and
//! its sys/base/seq.mm calls `let "..."` inside function bodies that run
//! as PIPELINE stages (`put ... | _Msh_seq_bc`). The body's `let` word
//! was already expanded to `let --` at definition; rubash's respawned
//! pipeline child imported the body without the ambient
//! __RUBASH_ALIAS_STREAMED marker, so the executor-level word fallback
//! expanded `let` a SECOND time (`let --` → `let -- --`), producing
//! `let: --: arithmetic syntax error: operand expected`. The fix marks
//! the streamed guard around every definition-frozen body execution
//! (`execute_function_dispatch`, function_calls.rs).
//!
//! Expectations byte-verified against GNU bash 5.3.20 (WSL, script
//! files).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2480let-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_run(tag: &str, files: &[(&str, &str)], main: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    for (name, contents) in files {
        std::fs::write(dir.join(name), contents).expect("write file");
    }
    std::fs::write(dir.join("probe.sh"), main).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The face-2 signature: a function whose body's `let` word was
/// alias-expanded at definition, CALLED AS A PIPELINE STAGE (a respawned
/// child in rubash, a fork in GNU) — the body must not be re-expanded.
#[test]
fn let_alias_body_in_pipeline_stage_is_not_reexpanded() {
    let (stdout, stderr, code) = rubash_run(
        "pipeline",
        &[],
        "shopt -s expand_aliases\n\
         alias let='let --'\n\
         f() {\n\
         \tE=0\n\
         \tif let \"(E>0 && E!=141) || 1>0\"; then echo COND_TRUE; else echo COND_FALSE; fi\n\
         }\n\
         echo | f\n\
         echo \"rc=$?\"\n",
    );
    // GNU 5.3.20: COND_TRUE, rc 0, empty stderr.
    assert_eq!(stdout, "COND_TRUE\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Same body called in the foreground must behave identically (the
/// foreground path was already guarded by the ambient batch marker; pin
/// it so the guard cannot regress there either).
#[test]
fn let_alias_body_in_foreground_is_not_reexpanded() {
    let (stdout, stderr, code) = rubash_run(
        "foreground",
        &[],
        "shopt -s expand_aliases\n\
         alias let='let --'\n\
         f() {\n\
         \tif let \"1 > 0\"; then echo COND_TRUE; else echo COND_FALSE; fi\n\
         }\n\
         f\n\
         echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "COND_TRUE\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// let_builtin (let.def:74-75): exactly one LEADING `--` is skipped; a
/// later `--` is an arithmetic operand and errors like GNU, and a lone
/// skipped `--` leaves "expression expected".
#[test]
fn let_skips_exactly_one_leading_ddash() {
    let (stdout, stderr, code) = rubash_run(
        "ddash",
        &[],
        "let -- 'x=41'\n\
         echo \"x=$x rc1=$?\"\n\
         let -- --; echo \"rc2=$?\"\n\
         let --; echo \"rc3=$?\"\n",
    );
    // GNU 5.3.20: stdout identical; stderr carries one arithmetic
    // syntax error (line 3) and one "expression expected" (line 4).
    assert_eq!(stdout, "x=41 rc1=0\nrc2=1\nrc3=1\n");
    assert_eq!(
        stderr.matches("let: --: arithmetic syntax error").count(),
        1,
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("let: expression expected"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

/// rubash#1867 regression guard (the UNSTREAMED_FUNCTION_BODIES branch
/// the fix must preserve): a function body defined from a SOURCED file
/// keeps RAW alias words (`alias not='! '`), and the executor-level
/// expansion at call time must still fire for it.
#[test]
fn sourced_body_alias_words_still_expand_at_call() {
    let lib = "g() {\n\
                \tnot false && echo NOT_TRUE\n\
                \tnot true || echo NOT_FALSE\n\
                }\n";
    let (stdout, stderr, code) = rubash_run(
        "unstreamed",
        &[("lib.sh", lib)],
        "shopt -s expand_aliases\n\
         alias not='! '\n\
         . ./lib.sh\n\
         g\n\
         echo \"rc=$?\"\n",
    );
    // GNU 5.3.20: the `not` alias inverts both conditions.
    assert_eq!(stdout, "NOT_TRUE\nNOT_FALSE\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
