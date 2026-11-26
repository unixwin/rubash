//! Issue rubash#314 regressions (wt13/sweepfix lane).
//!
//! A multi-line function definition nested DIRECTLY inside a loop body,
//! with the loop itself inside a function, failed to PARSE
//! (`syntax error: unexpected end of file from `{` command on line N`
//! for the for-in flavor, `syntax error: `;' unexpected` for the
//! arithmetic-for flavor) — killing liquidprompt's shunit2 test_disk and
//! test_ram at rc=2 before any test ran. One-line inner definitions and
//! the same nesting WITHOUT the outer function wrapper parsed fine, so
//! the failure needed all three ingredients.
//!
//! GNU contract (vendored third_party/bash, parse.y):
//! - parse.y:1054-1061 function_def productions — a function definition
//!   is an ordinary compound command element in every command-list
//!   position, including a loop body inside a function body; nothing
//!   restricts nesting.
//! - parse.y:879-880 for_command/select_command brace-form body
//!   `... '{' list '}'` vs the do-form `... do list done`: once `do` is
//!   read, `done` is the ONLY loop terminator; a `}` inside the body
//!   closes the innermost open brace group (yacc pairs it with the `{`
//!   that opened it), never the loop and never an enclosing group.
//!
//! Root cause fixed (parser/support.rs):
//! `update_compound_boundary_stack`'s LOOP frame accepted a boundary `}`
//! as a terminator even when the `}` belonged to a brace group opened
//! INSIDE the loop body (the nested `g() {`), and that same `}` then
//! double-served as the enclosing function group's closer in
//! `matching_brace_group_end` — the function's body ended at the nested
//! `g`'s brace, truncating the stream ("unexpected end of file").
//! The fix:
//! - a brace group opened inside any open keyword frame pushes its own
//!   `}` frame, so its closer pops only that frame;
//! - the `do` of a do-form loop transforms the frame to done-only
//!   (LOOP_DO_TERMINATOR), so the `}` terminator shortcut keeps applying
//!   only to the brace-form production (and a brace-form body's `}`
//!   closes the loop as in the grammar);
//! - `matching_brace_group_end` skips the depth accounting for any
//!   token that pushed or popped a frame in the same pass, so one token
//!   can never serve two grammar roles.
//!
//! Every expectation is byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-29; the 11-shape
//! variant matrix under target/sweepfix/).

use std::process::Command;

fn rubash_n(script: &str) -> Option<i32> {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i314-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-n")
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash -n");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        String::from_utf8_lossy(&output.stderr).is_empty(),
        "stderr not empty"
    );
    output.status.code()
}

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i314x-{}-{}",
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

/// The original 6-line reproducer and the arithmetic-for flavor.
#[test]
fn nested_funcdef_in_loop_in_function_parses() {
    assert_eq!(
        rubash_n("f() {\nfor x in a b; do\n    g() {\n        echo\n    }\ndone\n}\n"),
        Some(0)
    );
    assert_eq!(
        rubash_n(concat!(
            "f() {\n",
            "for (( i=0; i < ${#outputs[@]}; i++ )); do\n",
            "    df() {\n",
            "        printf mock\n",
            "    }\n",
            "done\n",
            "}\n"
        )),
        Some(0)
    );
}

/// The loop flavors from the issue matrix: while-read, keyword-form
/// wrapper, one-line inner, non-function body, deep nesting — and the
/// brace-form loop bodies that share the terminator machinery.
#[test]
fn nested_funcdef_loop_flavor_matrix_parses() {
    for script in [
        // while-read flavor
        "f() {\nwhile read -r l; do\n    g() {\n        echo\n    }\ndone\n}\n",
        // `function NAME {` wrapper
        "function test_disk {\nfor x in a b; do\n    g() {\n        echo\n    }\ndone\n}\n",
        // one-line inner definition
        "f() {\nfor x in a b; do\n    g() { echo; }\ndone\n}\n",
        // no wrapper function
        "for x in a b; do\n    g() {\n        echo\n    }\ndone\n",
        // arithmetic in the nested body
        "f() {\nfor x in a b; do\n    g() {\n        echo $(( 1 + 2 ))\n    }\ndone\n}\n",
        // two loop levels
        concat!(
            "f() {\n",
            "for i in 0 1; do\n",
            "    for j in a b; do\n",
            "        h() {\n",
            "            echo nested\n",
            "        }\n",
            "    done\n",
            "done\n",
            "}\n"
        ),
        // brace-form loop bodies still terminate on their own `}`
        "f() { for i; { echo braceform; } }\n",
        // brace-form loop with a nested definition inside
        "f() {\nfor i; {\n    echo braceform2\n    g() { echo inner; }\n}\n}\n",
        // if-body wrapper inside the loop (was already green, stays green)
        "f() {\nfor x in a b; do\n    if [ 1 ]; then\n        echo\n    fi\ndone\n}\n",
    ] {
        assert_eq!(rubash_n(script), Some(0), "script: {script:?}");
    }
}

/// The definition is executable: the loop-defined function exists and
/// runs after the outer function executes (liquidprompt mock shape).
#[test]
fn nested_funcdef_in_loop_executes() {
    let (stdout, stderr, code) = rubash_file(concat!(
        "f() {\n",
        "for x in a b; do\n",
        "    g() {\n",
        "        echo \"g called with $1\"\n",
        "    }\n",
        "done\n",
        "}\n",
        "f\n",
        "g hello\n",
        "echo \"rc=$?\"\n",
    ));
    assert_eq!(stdout, "g called with hello\nrc=0\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}
