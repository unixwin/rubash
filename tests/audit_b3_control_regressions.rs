//! Deep-audit round 2, lane B3 (control flow / functions / scoping)
//! regression pins. Every expectation is the literal GNU bash 5.3.20
//! behavior for the same source, probed against the vendored oracle
//! binary; the owning GNU C source is cited per finding.

use std::process::{Command, Output};

fn run(source: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .args(["--norc", "-c", source])
        .env_remove("BASH_ENV");
    command.output().expect("run rubash")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// B3-F01: GNU source_file runs the RETURN trap unconditionally after a
// sourced file completes (builtins/evalfile.c:395 run_return_trap,
// trap.c:1328). The null device is an ordinary empty file to GNU —
// it opens, reads EOF, and the trap still fires. rubash's source
// builtin early-returned for null-device spellings, skipping the
// whole source tail (trap fire, source frame, positional save).
#[test]
fn b3_f01_return_trap_fires_after_sourcing_dev_null() {
    let output = run("trap 'echo RT' RETURN; . /dev/null; echo \"rc:$?\"");
    assert_eq!(stdout(&output), "RT\nrc:0\n");
}

#[test]
fn b3_f01_return_trap_fires_after_sourcing_nul_spelling() {
    let output = run("trap 'echo RT' RETURN; source /dev/null; echo done");
    assert_eq!(stdout(&output), "RT\ndone\n");
}

// Control faces that already matched GNU and must keep matching: a
// real empty file fires, functrace inheritance fires inside a
// function, and without inheritance a source inside a function does
// NOT fire (execute_cmd.c:5295 resets the trap for the body, so
// evalfile.c:395's run_return_trap finds nothing trapped).
#[test]
fn b3_f01_return_trap_source_faces_stay_aligned() {
    let dir = std::env::temp_dir().join(format!("rubash-b3-f01-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let empty = dir.join("empty.sh");
    std::fs::write(&empty, "").expect("write empty fixture");
    let script = format!(
        "trap 'echo RT' RETURN; . {}; echo after; f() {{ . {}; echo in-f; }}; f; echo end",
        empty.display(),
        empty.display()
    );
    let output = run(&script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout(&output), "RT\nafter\nin-f\nend\n");
}

// B3-F01 (failure faces): GNU reaches source_file whenever a filename
// was produced, and source_file runs the RETURN trap after
// evalfile_internal returns on ANY outcome (evalfile.c:395) — a failed
// open (not-found under the cwd fallback, a directory, an unreadable
// file) fires exactly like a clean run. Only the filename == 0 search
// outcome (here: a `-p` miss) returns before source_file.
#[test]
fn b3_f01_return_trap_fires_after_failed_source() {
    let output =
        run("trap 'echo RT' RETURN; . ./rubash-b3-no-such-file 2>/dev/null; echo \"rc:$?\"");
    assert_eq!(stdout(&output), "RT\nrc:1\n");

    let output = run("trap 'echo RT' RETURN; . / 2>/dev/null; echo \"rc:$?\"");
    assert_eq!(stdout(&output), "RT\nrc:1\n");
}

#[test]
fn b3_f01_return_trap_skips_path_search_miss() {
    let output = run(
        "trap 'echo RT' RETURN; source -p /rubash-b3-no-such-dir somefile 2>/dev/null; echo \"rc:$?\"",
    );
    assert_eq!(stdout(&output), "rc:1\n");
}

// B3-F01 (completion classes): the trap tail belongs to source_file's
// return, so a sourced file's syntax error (evalstring.c returns
// EX_BADUSAGE ordinarily) and a top-level `return` (ordinary
// completion) both fire, while a sourced `exit` unwinds past
// source_file (jump_to_top_level EXITPROG) and must NOT fire.
#[test]
fn b3_f01_return_trap_completion_classes() {
    let dir = std::env::temp_dir().join(format!("rubash-b3-f01c-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("syn.sh"), "echo before-err\nif true; then\n").expect("write syn");
    std::fs::write(dir.join("ret.sh"), "return 5\n").expect("write ret");
    std::fs::write(dir.join("exit.sh"), "exit 3\n").expect("write exit");
    let script = format!(
        "trap 'echo RT' RETURN; . {0}/syn.sh 2>/dev/null; echo \"syn:$?\"; . {0}/ret.sh; echo \"ret:$?\"; . {0}/exit.sh; echo never",
        dir.display()
    );
    let output = run(&script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout(&output), "before-err\nRT\nsyn:2\nRT\nret:5\n");
    assert_eq!(output.status.code(), Some(3));
}

// B3-F04: GNU's kill option scan judges an attached `-s<alpha>` word
// by the text AFTER the option letter (builtins/kill.def: word[1]
// == 's' && ISALPHA(word[2]) -> sigspec = word+2), so `-sigterm`
// judges "igterm" and is always `invalid signal specification`.
// rubash's executor pre-parser (limit_builtins kill_request) judged
// word+1 ("sigterm", SIG-prefix-decodable), so the tracked-job /
// dead-pid faces consumed the command before the builtin's faithful
// parser could reject it: `kill -sigterm <dead-pid>` printed
// "No such process" and `kill -sigterm %1` silently delivered TERM.
#[test]
fn b3_f04_kill_sig_prefixed_bare_word_rejected_for_dead_pid() {
    let output = run("kill -sigterm 999999; echo \"rc:$?\"");
    assert_eq!(stdout(&output), "rc:1\n");
    assert!(
        stderr(&output).contains("kill: igterm: invalid signal specification"),
        "stderr was: {}",
        stderr(&output)
    );
}

#[test]
fn b3_f04_kill_sig_prefixed_bare_word_rejected_for_job() {
    let output = run("sleep 30 & kill -sigterm %1; echo \"rc:$?\"; kill -KILL %1 2>/dev/null; wait 2>/dev/null; echo end");
    assert_eq!(stdout(&output), "rc:1\nend\n");
    assert!(
        stderr(&output).contains("kill: igterm: invalid signal specification"),
        "stderr was: {}",
        stderr(&output)
    );
}

#[test]
fn b3_f04_kill_sig_prefixed_bare_word_rejected_for_self() {
    let output = run("kill -sigterm $$; echo \"rc:$?\"");
    assert_eq!(stdout(&output), "rc:1\n");
    assert!(
        stderr(&output).contains("kill: igterm: invalid signal specification"),
        "stderr was: {}",
        stderr(&output)
    );
}

// The valid forms must keep working through both dispatch faces:
// bare `-TERM`, `-s TERM`, `-s sigterm` (the SIG prefix IS legal in
// the -s sigspec, kill.def use_sigspec -> decode_signal with
// DSIG_SIGPREFIX), attached `-sTERM`, and the numeric forms.
#[test]
fn b3_f04_kill_valid_spec_forms_unchanged() {
    for form in ["-TERM", "-s TERM", "-s sigterm", "-sTERM", "-15", "-n 15"] {
        let output = run(&format!("kill {form} 999999; echo \"rc:$?\""));
        assert_eq!(stdout(&output), "rc:1\n", "form {form}");
        assert!(
            stderr(&output).contains("kill: (999999) - No such process"),
            "form {form} stderr was: {}",
            stderr(&output)
        );
    }
}
