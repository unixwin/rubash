/// rubash#3605 class B (run-nquote): GNU pushes alias-expansion text onto
/// the input (parse.y:2055 push_string) and consumes it through
/// shell_getc, so the yy_readline_get line_number bump (parse.y:2504)
/// fires only on REAL input fetches — a `\n` inside an alias value is a
/// command separator but not a line advance. Commands born from one
/// alias expansion all stamp the alias word's line; the next real input
/// line advances from there. Rubash splices the expansion into flat
/// text, so the batch lexer must skip `line_number += 1` on
/// expansion-introduced newline terminators (expansion marks carried
/// through expand_group_aliases_marked → run_source_*_marked →
/// tokenize_*_marked).
use std::io::Write;

fn run_script(lines: &[&str]) -> (String, i32) {
    let dir = std::env::temp_dir().join(format!("rubash-3605-alias-lines-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(file, "{}", lines.join("\n")).unwrap();
    drop(file);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// Extract the `line N: <cmd>: command not found` diagnostics GNU prints
/// for missing commands (`program name` varies between shells — keep
/// only the line stamp and the missing word).
fn missing_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter(|line| line.contains("command not found"))
        .map(|line| {
            // `<prog>: line N: cmd: command not found` -> `line N: cmd`
            let body = line.split_once(": ").map(|(_, t)| t).unwrap_or(line);
            body.strip_suffix(": command not found")
                .unwrap_or(body)
                .to_string()
        })
        .collect()
}

#[test]
fn alias_embedded_newlines_stamp_alias_word_line() {
    // nquote4.sub shape: `foo` at line 4 expands to three commands; GNU
    // reports `quux` at line 4 (the alias word's line), not line 6.
    let (stderr, _code) = run_script(&[
        "shopt -s expand_aliases",
        "",
        "alias foo=$'echo foo\\n\\nquux\\n'",
        "foo",
        "echo after",
    ]);
    let missing = missing_lines(&stderr);
    assert_eq!(
        missing,
        vec!["line 4: quux".to_string()],
        "stderr: {stderr}"
    );
}

#[test]
fn real_lines_after_expansion_keep_counting() {
    // The expansion's trailing real `\n` still advances the counter: the
    // first real line after `foo` reports one past the alias word's line.
    let (stderr, _code) = run_script(&[
        "shopt -s expand_aliases",
        "alias foo=$'echo a\\necho b\\necho c'",
        "foo",
        "m1",
        "m2",
    ]);
    let missing = missing_lines(&stderr);
    assert_eq!(
        missing,
        vec!["line 4: m1".to_string(), "line 5: m2".to_string()],
        "stderr: {stderr}"
    );
}

#[test]
fn mid_line_alias_expansion_same_stamp() {
    // `x=1; foo` — the alias word is not at logical-line start; pushed
    // commands still stamp the word's line.
    let (stderr, _code) = run_script(&[
        "shopt -s expand_aliases",
        "alias foo=$'echo foo\\nquux\\n'",
        "x=1; foo",
        "badcmd_end",
    ]);
    let missing = missing_lines(&stderr);
    assert_eq!(
        missing,
        vec!["line 3: quux".to_string(), "line 4: badcmd_end".to_string()],
        "stderr: {stderr}"
    );
}
