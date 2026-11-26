//! rubash#1317: a readonly-assignment diagnostic raised inside a
//! compound command is routed through THAT command's redirections,
//! exactly like stderr produced by the command itself.
//!
//! GNU anchor: `execute_cmd.c`: builtin error output follows the
//! redirections in force for the executing command, so
//! `(UID=5) 2>file` writes `UID: readonly variable` into the file
//! and `{ UID=5; } 2>/dev/null` prints nothing. rubash wrote the
//! diagnostic to the OUTER stderr in every form, leaving the file
//! empty and defeating `2>/dev/null`. Message text, line numbers
//! and the rc (1, execution continues) already agreed.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::{Command, Stdio};

/// All four reported forms in one script; the per-form diagnostics
/// must land in the redirect targets, outer stderr must be empty,
/// every status is 1, and the script runs to completion.
#[test]
fn readonly_diagnostics_follow_compound_redirections() {
    let dir = std::env::temp_dir().join(format!("rubash-issue1317-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1317 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(
        &script_path,
        "echo start\n\
         (UID=5) 2>err1.txt\n\
         echo \"rc1=$?\"\n\
         { UID=5; } 2>/dev/null\n\
         echo \"rc2=$?\"\n\
         h() { UID=5; }\n\
         h 2>/dev/null\n\
         echo \"rc3=$?\"\n\
         (readonly q1; q1=2) 2>err4.txt\n\
         echo \"rc4=$?\"\n\
         echo done\n",
    )
    .expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let err1 = std::fs::read_to_string(dir.join("err1.txt")).unwrap_or_default();
    let err4 = std::fs::read_to_string(dir.join("err4.txt")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "start\nrc1=1\nrc2=1\nrc3=1\nrc4=1\ndone\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert!(err1.contains("UID: readonly variable"), "err1.txt: {err1}");
    assert!(err4.contains("q1: readonly variable"), "err4.txt: {err4}");
}
