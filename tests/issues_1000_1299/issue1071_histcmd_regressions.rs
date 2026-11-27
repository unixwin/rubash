//! rubash#1071: `HISTCMD` is bound in every shell (interactive or
//! not) and tracks the history position.
//!
//! GNU anchor: `variables.c` / history support: HISTCMD starts at
//! the current history length, `history -s` appends move it only
//! once history is enabled with `set -o history`. rubash never
//! bound the variable at all (`${HISTCMD-UNSET}` printed UNSET in
//! every context).
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::{Command, Stdio};

/// The reported sequence, run as a script file: 0 at start, still
/// 0 after `history -s` with history off, 3 once history is on and
/// two entries were appended.
#[test]
fn histcmd_tracks_history_position() {
    let dir = std::env::temp_dir().join(format!("rubash-issue1071-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1071 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(
        &script_path,
        "echo \"h1=${HISTCMD-UNSET}\"\n\
         history -s foo\n\
         echo \"h2=${HISTCMD-UNSET}\"\n\
         set -o history\n\
         history -s bar\n\
         echo \"h3=${HISTCMD-UNSET}\"\n",
    )
    .expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "h1=0\nh2=0\nh3=3\n"
    );
}

/// Even a bare `-c` run has HISTCMD bound (to 0), not UNSET.
#[test]
fn histcmd_is_bound_in_dash_c_runs() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo \"h=${HISTCMD-UNSET}\"")
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "h=0\n");
}
