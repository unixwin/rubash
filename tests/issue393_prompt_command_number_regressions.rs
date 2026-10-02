//! Issue rubash#393 regression: `\#` in `${var@P}` expanded to 0 instead
//! of GNU's current_command_number, and the wt33 `set -o`/`shopt -o`
//! width expectation was wrong for GNU.
//!
//! GNU spec:
//! - shell.c:183 `int current_command_number = 1;`
//! - eval.c:178 (reader_loop): `current_command_number++` once per command
//!   LIST read, BEFORE executing it — script lines, interactive lines,
//!   stdin lines. A semicolon-joined `a; b` on one line is ONE list
//!   (read_command gathers the whole line).
//! - parse.y:6568-6574 (decode_prompt_string `case '#'`): n =
//!   current_command_number, minus one ONLY when decoding_prompt is a real
//!   ps0/ps1/ps2 prompt. `${var@P}` calls decode_prompt_string(s, 0)
//!   (subst.c:8769); `is_prompt` is 0 so decoding_prompt keeps its prior
//!   value (parse.y:6290) — in a script that is NULL, and ps0_prompt is
//!   NULL too, so `NULL != NULL` is false and NO compensation applies:
//!   the raw counter prints.
//! - Sourced/eval'd text runs inside an executing list (evalstring.c has
//!   no increment): the counter freezes at the source line's value; a
//!   subshell forks the live value; `bash -c` never enters reader_loop
//!   (shell.c run_one_command) so it stays 1.
//! - `set -o` readable width is set.def:291 MINUS_O_FORMAT "%-15s\t%s\n";
//!   `shopt -o` is shopt.def:73 OPTFMT "%-20s\t%s\n".
//!
//! Verified byte-for-byte against WSL GNU Bash 5.3.0 script-file probes
//! (target/gapfix2/{h9a,h9b,h9c,h09,outer,multi,semi,oneline,cc,so2,sho}.sh
//! — both stdout and rc identical).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue393-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue393 scratch dir");
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
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn run_rubash_c(argument: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue393c-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue393c scratch dir");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(argument)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash -c probe");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The issue's H09 shape: an echo on script line k prints k+1 (init 1 plus
/// one increment per line-list, GNU eval.c:178 — `\#` shows the counter
/// DURING execution, after this list's own increment).
#[test]
fn at_p_command_number_counts_reader_lists() {
    let outcome = run_rubash_script(
        "p=\"history=\\! command=\\#\"\n\
         echo \"${p@P}\"\n\
         printf \"[%s]\\n\" \"${p@P}\"\n\
         true\n\
         echo \"${p@P}\"\n",
    );
    assert_eq!(outcome.code, Some(0));
    // GNU (target/gapfix2/h09.sh): 3, 4, 6.
    assert_eq!(
        outcome.stdout,
        "history=1 command=3\n[history=1 command=4]\nhistory=1 command=6\n"
    );
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}

/// A semicolon-joined sequence on ONE line is ONE GNU list: the counter
/// steps once per line, not per command.
#[test]
fn at_p_command_number_steps_per_line_not_per_command() {
    let outcome = run_rubash_script(
        "x=\"\\#\"\n\
         echo a; echo b\n\
         echo \"c=${x@P}\"\n",
    );
    assert_eq!(outcome.code, Some(0));
    // GNU (target/gapfix2/semi.sh): line 3 -> 4.
    assert_eq!(outcome.stdout, "a\nb\nc=4\n");
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}

/// Sourced lines run inside the `. file` list: the counter freezes at that
/// list's value (GNU target/gapfix2/outer.sh: 3, 4, 4, 5).
#[test]
fn at_p_command_number_freezes_inside_source() {
    let outcome = run_rubash_script(
        "printf 'echo \"inner1=${x@P}\"\necho \"inner2=${x@P}\"\n' > inner.sh\n\
         x=\"\\#\"\n\
         echo \"before=${x@P}\"\n\
         . ./inner.sh\n\
         echo \"after=${x@P}\"\n",
    );
    assert_eq!(outcome.code, Some(0));
    // GNU (target/gapfix2/outer5.sh, 5-line script: printf/x/before/./after):
    // before=4, the `. ./inner.sh` list runs at 5 and freezes, after=6.
    assert_eq!(outcome.stdout, "before=4\ninner1=5\ninner2=5\nafter=6\n");
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}

/// A subshell forks the live counter (GNU target/gapfix2/multi.sh).
#[test]
fn at_p_command_number_inherited_by_subshell() {
    let outcome = run_rubash_script(
        "x=\"\\#\"\n\
         echo one\n\
         echo \"c=${x@P}\"\n\
         (echo \"sub=${x@P}\")\n\
         echo \"d=${x@P}\"\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "one\nc=4\nsub=5\nd=6\n");
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}

/// `bash -c` never enters reader_loop (shell.c run_one_command): the
/// counter stays 1 (GNU target/gapfix2/cc.sh: inner bash -c prints 1).
#[test]
fn at_p_command_number_is_one_in_command_string() {
    let outcome = run_rubash_c("x=\"\\#\"; echo \"c=${x@P}\"");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "c=1\n");
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}

/// `set -o` pads names to 15 then TAB (set.def:291 MINUS_O_FORMAT);
/// `shopt -o` pads to 20 then TAB (shopt.def:73 OPTFMT). Byte-verified
/// against GNU (target/gapfix2/{so2,sho}.sh od -c).
#[test]
fn set_o_and_shopt_o_readable_widths() {
    let outcome = run_rubash_script("set -o | head -1\nshopt -o pipefail\n");
    // GNU: `shopt -o pipefail` exits 1 while pipefail is off (both shells,
    // probe target/gapfix2/sp.sh).
    assert_eq!(outcome.code, Some(1));
    // "allexport" (9) + 6 spaces + TAB + "off"; "pipefail" (8) + 12
    // spaces + TAB + "off".
    assert_eq!(
        outcome.stdout,
        "allexport      \toff\npipefail            \toff\n"
    );
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}
