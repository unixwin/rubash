//! rubash#2982 regressions: the background wrapper child's `-c` source
//! re-defines the parent's functions from a textual prelude, but under
//! functrace/errtrace (`set -eET`, as bats-exec-test runs) the child
//! inherited the DEBUG and ERR traps LIVE — so they fired before the
//! prelude's function definitions had been evaluated, while the
//! handler functions did not exist yet (GNU's forked async child never
//! has this window: execute_cmd.c:1576 execute_in_subshell copies the
//! table whole, and trap.c:1467 reset_or_restore_signal_handlers /
//! trap.c:1526 reset_signal_handlers keep DEBUG only under functrace,
//! ERR only under errtrace). The failure was nondeterministic — the
//! prelude order is HashMap-random — surfacing as `bats_debug_trap:
//! command not found`, `bats_error_trap: command not found`,
//! `bats_check_status_from_trap: command not found`, or
//! `bats_normalize_windows_dir_path: command not found`, each time
//! killing the background child (rc=127) and, in the bats shape,
//! leaving the test's `kill -SIGINT -- -$PID` with no such process
//! (rubash#2983). The same prelude rendering also dropped a quoted
//! empty word: `[[ $1 == "" ]]` (bats' bats_sort) rendered as
//! `[[ $1 ==  ]]`, a parse error that killed the child outright
//! (rc=2) — GNU print_cmd.c command_print_word_list prints each word's
//! text as lexed, quotes intact.
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
