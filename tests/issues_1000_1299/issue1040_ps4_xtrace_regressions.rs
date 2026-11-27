//! Issues rubash#1040 + rubash#1042 — PS4 expansion runs untraced.
//!
//! GNU `print_cmd.c:462-466 indirection_level_string` expands PS4 with the
//! `x` flag off (`change_flag ('x', FLAG_OFF)` around decode_prompt_string,
//! restored after). Two consequences:
//!
//! * #1040: command substitutions inside PS4 execute untraced. Rubash
//!   traced them — and each inner trace re-expanded PS4, recursing to the
//!   expansion depth limit (~50 `echo P` lines per outer command).
//! * #1042: `$-` expanded in PS4 context omits `x` (GNU `[hBc]`), while a
//!   plain `echo $-` under `set -x` keeps it (`hxBc`). Rubash leaked `x`
//!   into the PS4 read (`[hxBc]`).
//!
//! Expected outputs are byte-verified against GNU Bash 5.3.20
//! (script-file and `-c` probes, 2026-10-22).

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

/// #1040: a command substitution inside PS4 runs untraced — one outer
/// command produces exactly one trace line, not ~50 recursive ones.
#[test]
fn ps4_command_substitution_is_not_traced() {
    let (stdout, stderr, code) = rubash("PS4='[$(echo P)$-] '; set -x; echo hi");
    assert_eq!(stdout, "hi\n");
    assert_eq!(stderr, "[PhBc] echo hi\n");
    assert_eq!(code, Some(0));
}

/// #1042: `$-` inside PS4 has no `x`; the same `$-` as a command argument
/// keeps it.
#[test]
fn ps4_dollar_dash_omits_xtrace_flag() {
    let (stdout, stderr, code) = rubash("set -x; PS4='[$-] '; echo hi; echo $-");
    assert_eq!(stdout, "hi\nhxBc\n");
    assert_eq!(stderr, "+ PS4='[$-] '\n[hBc] echo hi\n[hBc] echo hxBc\n");
    assert_eq!(code, Some(0));
}
