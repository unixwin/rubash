//! Issue rubash#394 regression: a combined `&>` / `&>>` (and `2>`)
//! redirect whose target is a process substitution, on a compound command
//! body (subshell/while/case/if/for/select/time), failed with
//! `>(...): Invalid argument` and dropped the whole body's output.
//!
//! GNU spec: redir.c:832-838 — `&>word`/`&>>word` (r_err_and_out /
//! r_append_err_and_out) and `2>word` resolve word through
//! redirection_expand (redir.c:298), which forks the process substitution
//! (subst.c:6362 process_substitute) and binds the descriptors to the
//! substitution's /dev/fd pipe via dup2 — the literal `>(` text is NEVER
//! open()ed as a filename. Both stdout and stderr of the compound body
//! (including the select prompt) flow into the substitution; the `time`
//! report stays on the shell's real stderr.
//!
//! Root cause in rubash: the parser pushes the `&>` entry into the
//! ordered `redirects` list with the verbatim `>(` text before mirroring
//! the fd-1/fd-2 slots. materialize_compound_output_process_substitutions
//! rewrote only the mirror slots of its clone; the compound fd-table walk
//! (open_compound_output_redirects) runs on the ORIGINAL node, so
//! create_redirect_output reached Windows open() with the `>(` text —
//! ERROR_INVALID_NAME, printed as `>(...): Invalid argument`, aborting
//! the compound.
//!
//! Fix: a scoped `>(`-text -> carrier-path memo
//! (materialize_.../finish_.../expand_redirect_target) so every consumer
//! of the original node's redirect entries resolves the carrier, and the
//! walk binds fd 1/2 to it exactly like GNU's dup2.
//!
//! Verified byte-for-byte against WSL GNU Bash 5.3.0 script-file probes
//! (target/gapfix2/p394/{shapes2,plain2err,p2e2,extra,k065,amp}.sh).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue394-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue394 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    // LF line endings: probes run identically under WSL GNU Bash for
    // baseline comparison.
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash probe");
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// All seven compound bodies capture through `&>`/`&>>` (GNU shapes2
/// matrix: subshell+read-error, while, case, if, for, select incl. its
/// prompt, time body). The read error line is checked modulo the $0
/// prefix form (invocation spelling differs across hosts).
#[test]
fn combined_procsub_captures_every_compound_body() {
    let outcome = run_rubash_script(
        "( echo out; read -u x value ) &> >(cat > f1.txt)\n\
         sleep 0.3\n\
         while true; do echo wout; break; done &>> >(cat > f2.txt)\n\
         sleep 0.3\n\
         case one in one) echo cout ;; esac &>> >(cat > f3.txt)\n\
         sleep 0.3\n\
         if true; then echo iout; fi &> >(cat > f4.txt)\n\
         sleep 0.3\n\
         for item in one; do echo fout; done &> >(cat > f5.txt)\n\
         sleep 0.3\n\
         select item in one; do echo sout; break; done &> >(cat > f6.txt) <<< 1\n\
         sleep 0.3\n\
         time { echo tout; } &> >(cat > f7.txt)\n\
         sleep 0.3\n\
         printf 'f1=[%s] f2=[%s] f3=[%s] f4=[%s] f5=[%s] f6=[%s] f7=[%s]\\n' \\\n\
           \"$(tr '\\n' ' ' < f1.txt)\" \"$(tr '\\n' ' ' < f2.txt)\" \\\n\
           \"$(tr '\\n' ' ' < f3.txt)\" \"$(tr '\\n' ' ' < f4.txt)\" \\\n\
           \"$(tr '\\n' ' ' < f5.txt)\" \"$(tr '\\n' ' ' < f6.txt)\" \\\n\
           \"$(tr '\\n' ' ' < f7.txt)\"\n",
    );
    assert_eq!(outcome.code, Some(0));
    // `&>` also captures the subshell's STDERR (the read diagnostic) —
    // match GNU's captured bytes modulo the $0 path prefix.
    let stdout = outcome.stdout;
    let assert_contains = |needle: &str, what: &str| {
        assert!(stdout.contains(needle), "missing {what}: {stdout}");
    };
    assert_contains("out ", "f1 stdout");
    assert_contains(
        "read: x: invalid file descriptor specification",
        "f1 stderr",
    );
    assert_contains("f2=[wout ]", "while body");
    assert_contains("f3=[cout ]", "case body");
    assert_contains("f4=[iout ]", "if body");
    assert_contains("f5=[fout ]", "for body");
    // GNU captures the select PROMPT (`1) one` menu + PS3) into &> too.
    assert_contains("1) one", "select menu");
    assert_contains("sout", "select body");
    assert_contains("f7=[tout ]", "time body");
    // No diagnostic may leak: the Invalid argument family is gone and the
    // time report stays on real stderr, not in the captured files.
    assert!(!stdout.contains("Invalid argument"), "stdout: {stdout}");
    assert!(
        !stdout.contains("real"),
        "time report leaked into stdout: {stdout}"
    );
}

/// `2> >(...)` on a compound body routes fd 2 of the body into the
/// substitution (probe p2e2: both shells print `[ew]`).
#[test]
fn stderr_procsub_on_compound_captures_body_stderr() {
    let outcome = run_rubash_script(
        "while true; do echo ew 1>&2; break; done 2> >(cat > ef.txt)\n\
         sleep 0.3\n\
         echo \"ef=[$(cat ef.txt)]\"\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "ef=[ew]\n");
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}

/// The plain `> >(...)` form keeps working (issue's K065 shape).
#[test]
fn plain_procsub_on_compound_still_captures() {
    let outcome = run_rubash_script(
        "{ echo out; } > >(cat > gout.txt)\n\
         sleep 0.3\n\
         echo \"g=[$(cat gout.txt)]\"\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "g=[out]\n");
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}

/// Body output is captured ONCE (no fd-binding + splice double write).
#[test]
fn combined_procsub_does_not_double_body_output() {
    let outcome = run_rubash_script(
        "while true; do echo once; break; done &> >(cat > once.txt)\n\
         sleep 0.3\n\
         echo \"count=$(grep -c once once.txt)\"\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "count=1\n");
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}
