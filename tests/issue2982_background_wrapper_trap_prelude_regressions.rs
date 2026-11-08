//! rubash#2982 regressions: the background wrapper child's `-c` source
//! re-defines the parent's functions from a textual prelude. The three
//! tests below pin the prelude RENDERING regressions — a quoted empty
//! word dropped from a `[[ ]]` comparison (`[[ $1 == "" ]]` rendered
//! as `[[ $1 ==  ]]`, a parse error that killed the child outright,
//! rc=2) and a STORAGE_WORD_PREFIX carrier leaked into `for` word
//! lists (the respawned child re-lexed `for p in \x1d${arr[@]}` and
//! lost the first element). Both rendering defects were fixed by
//! merged #2990 (commit fb96e741, respawn serializers render words
//! via the raw contract); the tests below are that fix's regression
//! coverage, kept on this branch because this lane found them — they
//! fail on the pre-#2990 base (89e18a92) and pass on any tree that
//! contains #2990. GNU print_cmd.c command_print_word_list prints each
//! word's text as lexed, quotes intact.
//!
//! This PR's own change (C2) is separate: `reset_traps_for_background_wrapper`
//! + `background_trap_rearm_source` reset the functrace/errtrace-kept
//! DEBUG and ERR traps while the wrapper child evaluates its prelude
//! and re-arm them before the body, closing the window the textual
//! respawn creates (GNU's forked async child never has it:
//! execute_cmd.c:1576 execute_in_subshell copies the table whole).
//! Honesty note: no independently fail-first probe shape for C2 alone
//! was found — function definitions skip the DEBUG trap (verified on
//! GNU and rubash: a trap installed before three definitions fires
//! only for the following command), the wrapper child reads no startup
//! file ($BASH_ENV is not sourced by the respawned child, probed), and
//! the prelude is definitions-only, so no trap can fire while the
//! table is incomplete; the observed trap errors were downstream of the
//! C1 parse truncation (see REPORT §C2 honesty note). C2 is therefore
//! defense-in-depth, verified behavior-preserving, not pin-proven.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2982-{}-{}",
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

/// The bats_sort shape: a function whose `[[ ]]` compares against a
/// quoted empty string, backgrounded so it rides the wrapper prelude.
/// Before the fix the prelude rendered `[[ $1 ==  ]]` and the child
/// died at parse (rc=2); after the fix the child parses and the
/// function behaves like GNU.
#[test]
fn issue2982_background_function_with_quoted_empty_conditional() {
    let (stdout, stderr, code) = rubash_file(
        "sortish() { if [[ $1 == \"\" ]]; then echo empty; else echo \"got:$1\"; fi; }\nsortish \"\" &\nwait\nsortish x &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "empty\ngot:x\n");
}

/// The quoted-glob companion: `[[ $x == "a*b" ]]` must stay a literal
/// comparison in the respawned child, not degrade to the glob `a*b`.
#[test]
fn issue2982_background_function_with_quoted_glob_conditional() {
    let (stdout, stderr, code) = rubash_file(
        "g() { if [[ $1 == \"a*b\" ]]; then echo literal; else echo other; fi; }\ng 'a*b' &\nwait\ng 'axb' &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "literal\nother\n");
}

/// The `for`-word companion: the parser tags stored words with the
/// STORAGE_WORD_PREFIX carrier (0x1D), which the prelude renderer
/// emitted verbatim — the respawned child re-lexed `for p in
/// \x1d${arr[@]}` and lost the first element. Before the fix the
/// backgrounded loop printed `got:` then `got:b`; after the fix it
/// matches GNU (`got:a` / `got:b`).
#[test]
fn issue2982_background_function_with_for_over_array() {
    let (stdout, stderr, code) = rubash_file(
        "show() { local arr=(\"a\" \"b\"); for p in \"${arr[@]}\"; do echo \"got:$p\"; done; }\nshow &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "got:a\ngot:b\n");
}
