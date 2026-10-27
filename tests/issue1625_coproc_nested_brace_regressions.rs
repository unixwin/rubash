//! rubash#1625 — a named coproc with a brace-group body was a syntax
//! error (`syntax error near unexpected token '}'`, rc 2, whole
//! script rejected) when it appeared inside a brace group or a
//! function body, although the identical form works at top level.
//!
//! Root cause: matching_brace_group_end's word_command_open_brace
//! walk validates a `coproc NAME {` opener by walking left over the
//! prefix chain, but any KEYWORD predecessor outside the four prefix
//! keywords made it return false — including the enclosing group's
//! own `{`, which is precisely the command-position boundary that
//! proves the coproc starts a command. The falsifying predecessor
//! is a WORD (`echo coproc a {` keeps `{` a plain argument); a
//! keyword predecessor after a consumed prefix chain confirms
//! command position, so the walk now returns what it accumulated.
//!
//! Expectations verified against GNU bash 5.3.20 script-file probes.

use std::process::{Command, Stdio};

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1625-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn coproc_brace_body_inside_brace_group_and_function_parses() {
    // The issue's regression seed verbatim.
    let (stdout, stderr, code) = rubash_script(
        "{ coproc FC2 { cat; }; echo grouped-ok; }\n\
         f() { coproc FC3 { cat; }; echo func-ok; }\n\
         f\n",
    );
    assert_eq!(stdout, "grouped-ok\nfunc-ok\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_spawned_inside_brace_group_is_usable() {
    // Not just parseable: the coproc registered inside the group
    // round-trips data under its name.
    let (stdout, stderr, code) = rubash_script(
        "{ coproc FC4 { cat; }; echo hello >&${FC4[1]}; read -u ${FC4[0]} l; echo \"got=[$l]\"; exec {FC4[0]}>&- {FC4[1]}>&-; }\n",
    );
    assert_eq!(stdout, "got=[hello]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_word_before_brace_stays_a_plain_argument() {
    // The walk's guard, kept: after a plain command word, `coproc`
    // is data and `{` does not open a group (parse-level: the words
    // survive verbatim and no parse error is recorded).
    let ast = rubash::parser::parse(&rubash::lexer::tokenize("echo coproc a {"));
    assert_eq!(ast.commands.len(), 1);
    assert_eq!(ast.commands[0].words, ["echo", "coproc", "a", "{"]);
    assert!(ast.commands[0]
        .get_assignment("__RUBASH_PARSE_ERROR__")
        .is_none());
}
