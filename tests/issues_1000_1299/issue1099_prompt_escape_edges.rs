//! Issue rubash#1099 regression: prompt-escape edge handling in `@P`.
//!
//! `\l` decodes to the ttyname(0) basename else literal `tty`
//! (parse.y:6598-6607) — `tty` here because script stdin is not a tty.
//! `\D{fmt}` delegates fmt to the system strftime (parse.y:6390-6425):
//! an empty fmt becomes %X, an unclosed brace still formats, and glibc
//! strftime emits an unknown `%x` conversion verbatim — percent kept
//! (verified on bash 5.3.20 + glibc: `\D{%N}` -> `%N`).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn backslash_l_falls_back_to_tty_without_a_terminal() {
    let (stdout, _, _) = rubash("PS1='\\l'; echo \"${PS1@P}\"");
    assert_eq!(stdout, "tty\n");
}

#[test]
fn d_brace_unknown_conversion_keeps_the_percent() {
    let (stdout, _, _) = rubash("PS1='\\D{%N}|\\D{%Q|%E}'; echo \"${PS1@P}\"");
    assert_eq!(stdout, "%N|%Q|%E\n");
}

#[test]
fn d_brace_empty_defaults_to_locale_time() {
    let (stdout, _, _) = rubash("PS1='\\D{}'; echo \"${PS1@P}\"");
    let line = stdout.trim_end();
    let digits: Vec<&str> = line.split(':').collect();
    assert_eq!(digits.len(), 3, "want HH:MM:SS, got {line:?}");
    assert!(digits
        .iter()
        .all(|f| f.len() == 2 && f.chars().all(|c| c.is_ascii_digit())));
}

#[test]
fn d_brace_unclosed_still_formats() {
    // parse.y:6398-6400: the run ends at `}` or NUL alike; `c` stays 0 so
    // add_string does not re-emit the `\D` prefix.
    let (stdout, _, _) = rubash("PS1='\\D{abc'; echo \"${PS1@P}\"");
    assert_eq!(stdout, "abc\n");
}

#[test]
fn bare_d_without_brace_stays_literal() {
    // parse.y:6391: `if (string[1] != '{') goto not_escape`.
    let (stdout, _, _) = rubash("PS1='x\\D'; echo \"${PS1@P}\"");
    assert_eq!(stdout, "x\\D\n");
}

#[test]
fn printf_paren_t_shares_the_same_strftime_contract() {
    // printf.def `%()T` reaches the same system-strftime delegation.
    let (stdout, _, _) = rubash("printf '%(%N)T\\n' 1700000000");
    assert_eq!(stdout, "%N\n");
}
