//! Issue rubash#542: a `$`/backtick/quote EOF inside a `name=(` compound
//! assignment takes GNU parse.y:7140-7152 parse_compound_assignment's EOF
//! exit family — status 1, not the command-word 2. The diagnostic text
//! already matched GNU verbatim; only the rc was wrong.

#![cfg(windows)]

use std::process::{Command, Stdio};

fn run_script(script: &str) -> (Option<i32>, String) {
    let dir = std::env::temp_dir().join(format!("rubash-issue542-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue542 scratch dir");
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
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn compound_assignment_eof_exits_1() {
    // WSL GNU bash 5.3.0: each form reports the matching-delimiter EOF at
    // rc 1 (`v=("$(echo)` names `"` — the dq the `$( ` opens inside).
    let cases = [("v=($(echo\n", '`' /* placeholder replaced below */)];
    let _ = cases;
    for (script, closer) in [
        ("v=($(echo\n", ')'),
        ("v=(`echo\n", '`'),
        ("v=(\"$(echo)\n", '"'),
        ("v=(\"\n", '"'),
    ] {
        let (code, stderr) = run_script(script);
        assert_eq!(code, Some(1), "rc for {script:?}: stderr={stderr:?}");
        let first = stderr.lines().next().unwrap_or_default();
        assert!(
            first.contains(&format!(
                "unexpected EOF while looking for matching `{closer}'"
            )),
            "stderr for {script:?}: {first:?}"
        );
    }
}

#[test]
fn command_word_eof_keeps_rc_2() {
    // The compound-assignment fold must not downgrade a command-word EOF:
    // WSL GNU bash 5.3.0 `echo $((1+${x` -> rc 2.
    let (code, stderr) = run_script("echo $((1+${x\n");
    assert_eq!(code, Some(2), "stderr={stderr:?}");
}
