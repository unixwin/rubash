//! rubash#1081: `type` / `command -V` must know EVERY reserved word,
//! including `coproc` and `[[`, which were missing from the table.
//!
//! GNU anchor: `builtins/type.def` resolves names against the full
//! reserved-word list; `coproc is a shell keyword` and
//! `type -t '[['` → `keyword`. rubash answered "not found" (rc 1)
//! for exactly those two words while every other keyword agreed.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::{Command, Stdio};

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!("rubash-issue1081-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1081 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The reported matrix: `coproc` and `[[` report as keywords via
/// `type`, `type -t` and `command -V`; the `if` control already
/// agreed; a genuinely unknown name still fails with rc 1.
#[test]
fn type_knows_coproc_and_double_bracket() {
    let (code, stdout, stderr) = run_script(
        "type coproc; echo \"rc1=$?\"\n\
         type -t coproc; type -t '[['\n\
         command -V coproc; echo \"rc2=$?\"\n\
         type if; echo \"rc3=$?\"\n\
         type nosuchxyz123; echo \"rc4=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "coproc is a shell keyword\n\
         rc1=0\n\
         keyword\n\
         keyword\n\
         coproc is a shell keyword\n\
         rc2=0\n\
         if is a shell keyword\n\
         rc3=0\n\
         rc4=1\n"
    );
    assert!(
        stderr.contains("type: nosuchxyz123: not found"),
        "stderr: {stderr}"
    );
}
