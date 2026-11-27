//! rubash#1126 — With PATH unset, internal fast-path commands still
//! execute (cat, sleep, piped head) — inconsistently with direct
//! invocation.
//!
//! GNU execute_cmd.c find_shell_builtin / shell_exec: with PATH unset
//! no external command resolves, in ANY syntactic position — a name
//! that only exists as an internal shortcut of the engine must not
//! bypass the PATH lookup that a direct invocation goes through.
//! Every form below is 127 + "No such file or directory".
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` string): the
//! rc markers are byte-exact; diagnostics matched on their stable core
//! (line prefix carries the binary's own path).

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
fn unset_path_blocks_fastpath_commands_everywhere() {
    let (stdout, stderr, code) = rubash(
        "unset PATH; printf 'x\\n' | cat; echo cat_rc=$?\n\
         unset PATH; sleep 0; echo sleep_rc=$?\n\
         unset PATH; head -1 <<<direct; echo head_rc=$?\n\
         unset PATH; ls /tmp >/dev/null; echo ls_rc=$?",
    );
    assert_eq!(stdout, "cat_rc=127\nsleep_rc=127\nhead_rc=127\nls_rc=127\n");
    for name in ["cat", "sleep", "head", "ls"] {
        assert!(
            stderr.contains(&format!("{name}: No such file or directory")),
            "missing diagnostic for {name}; stderr: {stderr}"
        );
    }
    assert_eq!(code, Some(0));
}
