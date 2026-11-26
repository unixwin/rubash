//! rubash#384: a `$'...'` ANSI-C unit whose body contains an escaped quote
//! (`\'`) made a `$(...)` command substitution fail with
//! `command substitution: line 1: unexpected EOF while looking for
//! matching `)'` — GNU executes it and prints the decoded quote.
//!
//! GNU spec:
//! - parse.y:5541-5549 read_token_word (shellexp branch): `$'` is ONE
//!   self-contained unit consumed via `parse_matched_pair(..., P_ALLOWESC)`
//!   — a backslash escapes ANY following character, including the closing
//!   `'` (parse.y:3826 defines P_ALLOWESC; parse.y:3992-3996 sets
//!   LEX_PASSNEXT for a `\\` inside a `'`-delimited unit).
//! - parse.y:4451 parse_comsub decides the comsub body extent with that
//!   same reader (yyparse with `shell_eof_token = close`, parse.y:4519),
//!   so in `$(echo foo$'\''bar)` the unit ends at its THIRD quote and the
//!   following `)` is the substitution closer.
//!
//! Two Rust owners shared the missing invariant:
//! - `collect_command_substitution_source_ex` (executor/embedded_
//!   mutations.rs) had no `$'` arm: the plain `'` arm flipped single-quote
//!   state and `\'` was not an escape while `single`, so the closer `)`
//!   looked quoted and the collector ran to EOF.
//! - `quoted_case_pattern_end` (executor/compound_exec.rs) honored `\\`
//!   escapes only for `"`: for a `$'` unit it ended at the escaped quote,
//!   left a residual `'`, and the case word/pattern walkers (and the
//!   here-string segment walker) spun forever on it — a CPU hang exposed
//!   at top level (`case $'a\\'b' in ...`) and inside comsubs once the
//!   collector stopped mis-erroring.
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash) script-file probes
//! (target/issue-suites/results/wt37-gapfix1/, 2026-10-02).

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The failure family includes an infinite loop (the case walkers), so
/// every run is deadline-pinned: a regression hangs instead of printing.
const RUN_LIMIT: Duration = Duration::from_secs(20);

struct RunOutcome {
    timed_out: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue384-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue384 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("create probe script");
    // LF line endings only: .gitattributes pins *.sh eol=lf and CR bytes
    // corrupt ANSI-C quoting probes.
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");

    let stdout_pipe = child.stdout.take().expect("piped stdout");
    let stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stdout_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stderr_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break Some(status),
            None => {
                if started.elapsed() >= RUN_LIMIT {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let stdout = stdout_reader.join().expect("join stdout reader");
    let stderr = stderr_reader.join().expect("join stderr reader");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        timed_out,
        code: status.and_then(|status| status.code()),
        stdout,
        stderr,
    }
}

fn assert_script(script: &str, expected_stdout: &[u8]) {
    let outcome = run_rubash_script(script);
    assert!(
        !outcome.timed_out,
        "rubash hung on {script:?} (the #384 case-walker spin)"
    );
    assert_eq!(outcome.code, Some(0), "rc for {script:?}");
    assert_eq!(outcome.stdout, expected_stdout, "stdout for {script:?}");
    assert!(
        outcome.stderr.is_empty(),
        "stderr for {script:?}: {:?}",
        String::from_utf8_lossy(&outcome.stderr)
    );
}

#[test]
fn comsub_body_with_ansic_escaped_quote_executes() {
    // The #384 reproducer (GNU: `foo'bar`, rc 0, empty stderr).
    assert_script("echo \"$(echo foo$'\\''bar)\"\n", b"foo'bar\n");
}

#[test]
fn comsub_ansic_escaped_quote_in_assignment_and_bare_words() {
    assert_script("x=$(echo q$'\\''w); echo \"C:$x\"\n", b"C:q'w\n");
    assert_script("echo \"$(echo a$'\\'')b\"\n", b"a'b\n");
    assert_script(
        "echo \"$( echo $'x\\'y' ; echo second )\"\n",
        b"x'y\nsecond\n",
    );
    assert_script("echo \"$(printf '%s\\n' $'p\\'q')\"\n", b"p'q\n");
}

#[test]
fn nested_comsub_with_ansic_escaped_quote() {
    assert_script(
        "echo \"$(echo \"$(echo nested$'z\\'w')\")\"\n",
        b"nestedz'w\n",
    );
}

#[test]
fn ansic_escaped_quote_is_not_a_comment_or_a_closer() {
    // `)` and `#` inside the unit are data, not the substitution closer
    // or a comment introducer.
    assert_script("echo \"[$(echo $')x'y)]\"\n", b"[)xy]\n");
    assert_script("echo \"$(echo $'#nc')\"\n", b"#nc\n");
}

#[test]
fn case_word_and_pattern_with_ansic_escaped_quote_terminate() {
    // The hang family: the case walkers must find the unit's real close.
    assert_script(
        "case $'a\\'b' in $'a\\'b') echo m;; *) echo n;; esac\n",
        b"m\n",
    );
    assert_script("case $'s\\'t' in no) echo m;; *) echo n;; esac\n", b"n\n");
}

#[test]
fn comsub_case_with_ansic_escaped_quote_pattern() {
    // m11 matrix shape: case inside $(...) with `$'\''*` pattern.
    assert_script(
        "echo \"11:[$(case $'x' in $'\\''*) echo m;; *) echo n;; esac)]\"\n",
        b"11:[n]\n",
    );
}

#[test]
fn for_list_and_herestring_with_ansic_escaped_quote() {
    assert_script(
        "for w in $'a\\'b' c; do echo \"[$w]\"; done\n",
        b"[a'b]\n[c]\n",
    );
    assert_script("f() { echo \"[$1]\"; }; f $'arg\\'x'\n", b"[arg'x]\n");
    assert_script("cat <<< $'hs\\'q'\n", b"hs'q\n");
}

#[test]
fn ansic_escaped_quote_in_double_quotes_stays_literal() {
    // Inside double quotes `$'` is literal text (GNU: `$'x` — no unit).
    assert_script("echo \"$(echo \"$'x\")\"\n", b"$'x\n");
}
