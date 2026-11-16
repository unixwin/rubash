//! rubash#3325 — inside a timed body, an inner compound command's own
//! redirection failure never reached the ERR verdict
//! (`time { if true; then echo body; fi < /bad; echo cont; }` printed no
//! ERR trap line; GNU prints it once).
//!
//! GNU `execute_cmd.c:1430 time_command` only hands the command to
//! `execute_command_internal`; whether `run_error_trap` / the ERREXIT
//! jump run is decided inside that command's own execution. A nested
//! command of a timed body is an ordinary command there — its compound
//! redirect failure takes `execute_cmd.c:854-883`, its simple-command
//! failure `execute_cmd.c:993-1010` — and a statement compound (brace,
//! if, for, while/until, case, select) never adjudicates at its own level
//! except for its own `do_redirections` failure. Rubash's timed deferral
//! (`timed_dispatch_depth`, rubash#1611) applied to every command inside
//! the timed body, and the time branch re-adjudicated the final status of
//! statement-compound roots — losing inner redirect failures and firing
//! the trap twice for a failing last body command.
//!
//! The non-redirect shapes (double fire, `&&`-exempt last command, `!`)
//! are the #1611/#1277 family and are accounted in their own tests below,
//! as the issue's acceptance asks. Expectations are WSL GNU bash 5.3.20's,
//! probed from script files. `$LINENO` inside the trap is deliberately not
//! pinned: the compound-redirect-failure ERR line diverges independently
//! of `time` (filed separately).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_file(name: &str, script: &str) -> (String, Option<i32>) {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash-issue3325-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    let script_path = dir.join("probe.sh");
    let body = format!("exec 2>/dev/null\ntrap 'echo gotERR' ERR\nTIMEFORMAT=\n{script}");
    fs::write(&script_path, body).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .output()
        .expect("run rubash script file");
    let _ = fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        output.status.code(),
    )
}

fn check(name: &str, script: &str, expected: &str, expected_rc: i32) {
    let (stdout, rc) = rubash_file(name, script);
    assert_eq!(stdout, expected, "case {name}: script {script:?}");
    assert_eq!(rc, Some(expected_rc), "case {name}: exit status");
}

#[test]
fn inner_compound_redirect_failure_in_timed_body_fires_err() {
    // The issue's shape and its compound siblings.
    for (name, inner) in [
        ("if", "if true; then echo body; fi < /bad"),
        ("for", "for i in 1; do echo b; done < /bad"),
        ("while", "while false; do :; done < /bad"),
        ("brace", "{ echo b; } < /bad"),
        ("case", "case x in x) echo b;; esac < /bad"),
    ] {
        check(
            name,
            &format!("time {{ {inner}; echo cont; }}\necho rc=$?\n"),
            "gotERR\ncont\nrc=0\n",
            0,
        );
    }
}

#[test]
fn inner_redirect_failure_as_last_body_command_fires_once() {
    check(
        "last",
        "time { echo a; if true; then echo body; fi < /bad; }\necho rc=$?\n",
        "a\ngotERR\nrc=1\n",
        0,
    );
}

#[test]
fn timed_compound_own_redirect_failure_still_fires() {
    check(
        "own-brace",
        "time { echo a; } < /bad\necho rc=$?\n",
        "gotERR\nrc=1\n",
        0,
    );
    check(
        "own-if",
        "time if true; then echo body; fi < /bad\necho rc=$?\n",
        "gotERR\nrc=1\n",
        0,
    );
}

#[test]
fn inner_redirect_failure_in_timed_body_takes_errexit() {
    check(
        "errexit-if",
        "set -e\ntime { if true; then echo b; fi < /bad; echo cont; }\necho after\n",
        "gotERR\n",
        1,
    );
    check(
        "errexit-while",
        "set -e\ntime { while false; do :; done < /bad; echo x; }\necho after\n",
        "gotERR\n",
        1,
    );
    // An &&-list member is exempt (GNU: no trap, no exit).
    check(
        "errexit-andor",
        "set -e\ntime { echo a; if :; then :; fi < /bad; } && echo no\necho after\n",
        "a\nafter\n",
        0,
    );
}

#[test]
fn untimed_and_function_controls_unchanged() {
    check(
        "untimed",
        "{ if true; then echo body; fi < /bad; echo cont; }\necho rc=$?\n",
        "gotERR\ncont\nrc=0\n",
        0,
    );
    // ERR is not inherited into the function without errtrace.
    check(
        "func",
        "f() { if true; then echo body; fi < /bad; echo fcont; }\ntime f\n",
        "fcont\n",
        0,
    );
    check(
        "pipe",
        "time { if true; then echo body; fi < /bad | cat; echo cont; }\n",
        "cont\n",
        0,
    );
}

// --- #1611/#1277 family: non-redirect failures, accounted separately ---

#[test]
fn timed_statement_compound_does_not_refire_its_body_failure() {
    for (name, cmd) in [
        ("brace-last", "time { echo a; false; }"),
        ("if-then-false", "time { if false; then :; fi; false; }"),
        ("timed-if", "time if true; then false; fi"),
        ("timed-for", "time for i in 1; do false; done"),
        ("nested-brace", "time { { false; }; }"),
        ("nested-subshell", "time { ( false ); }"),
        ("pipeline-last", "time { true | false; }"),
    ] {
        let (stdout, _) = rubash_file(name, &format!("{cmd}\necho end\n"));
        assert_eq!(
            stdout.matches("gotERR").count(),
            1,
            "case {name}: {cmd:?} -> {stdout:?}"
        );
    }
    check(
        "func-in-brace",
        "f() { false; }\ntime { f; }\necho end\n",
        "gotERR\nend\n",
        0,
    );
}

#[test]
fn timed_statement_compound_with_exempt_last_command_is_not_a_failure() {
    check("andand", "time { false && true; }\necho end\n", "end\n", 0);
    check(
        "andand-errexit",
        "set -e\ntime { false && true; }\necho after\n",
        "after\n",
        0,
    );
}

#[test]
fn timed_inverted_compound_suppresses_body_err() {
    check(
        "bang",
        "time ! { false; }\nset -e\ntime ! { false; }\necho end\n",
        "end\n",
        0,
    );
}

#[test]
fn timed_chain_keeps_body_errexit() {
    check(
        "chain",
        "set -e\ntime time { echo a; false; }\necho after\n",
        "a\ngotERR\n",
        1,
    );
    check(
        "simple-still-fires",
        "time false\ntime true | false\necho end\n",
        "gotERR\ngotERR\nend\n",
        0,
    );
}
