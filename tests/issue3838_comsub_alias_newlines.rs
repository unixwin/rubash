//! rubash#3838: alias-introduced newlines inside `$(...)`, `` `...` ``,
//! `${ ...; }`, and `${| ...; }` command substitutions must not advance
//! line numbers in either the enclosing script or the comsub body's own
//! runtime re-lex (GNU bash 5.3 parse.y:2504 yy_readline_get only bumps
//! line_number on real input fetches, not push_string alias text).

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script(lines: &[&str]) -> (String, i32) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "rubash-3838-comsub-lines-{}-{}",
        std::process::id(),
        id
    ));
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

fn missing_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter(|line| line.contains("command not found"))
        .map(|line| {
            let body = line.split_once(": ").map(|(_, t)| t).unwrap_or(line);
            body.strip_suffix(": command not found")
                .unwrap_or(body)
                .to_string()
        })
        .collect()
}

#[test]
fn dollar_paren_comsub_alias_newlines_keep_outer_and_inner_lines() {
    let (stderr, _code) = run_script(&[
        "shopt -s expand_aliases",
        "alias foo=$'echo 1\\n\\nquux\\n'",
        "x=$(foo)",
        "bad_after",
    ]);
    assert_eq!(
        missing_lines(&stderr),
        vec!["line 3: quux".to_string(), "line 4: bad_after".to_string()],
        "stderr: {stderr}"
    );
}

#[test]
fn dollar_paren_comsub_mixed_real_and_alias_newlines() {
    let (stderr, _code) = run_script(&[
        "shopt -s expand_aliases",
        "alias foo=$'echo 1\\nquux'",
        "echo $(echo 0",
        "foo",
        "mid_real",
        ")",
        "bad_after",
    ]);
    assert_eq!(
        missing_lines(&stderr),
        vec![
            "line 4: quux".to_string(),
            "line 5: mid_real".to_string(),
            "line 7: bad_after".to_string(),
        ],
        "stderr: {stderr}"
    );
}

#[test]
fn backtick_comsub_alias_newlines_keep_inner_and_outer_lines() {
    let (stderr, _code) = run_script(&[
        "shopt -s expand_aliases",
        "alias foo=$'echo 1\\n\\nquux\\n'",
        "x=`foo`",
        "bad_after",
    ]);
    assert_eq!(
        missing_lines(&stderr),
        vec!["line 3: quux".to_string(), "line 4: bad_after".to_string()],
        "stderr: {stderr}"
    );
}

#[test]
fn funsub_and_valsub_alias_newlines_keep_inner_and_outer_lines() {
    let (stderr, _code) = run_script(&[
        "shopt -s expand_aliases",
        "alias foo=$'echo 1\\n\\nquux'",
        "x=${ foo; }",
        "y=${| foo; }",
        "bad_after",
    ]);
    assert_eq!(
        missing_lines(&stderr),
        vec![
            "line 3: quux".to_string(),
            "line 4: quux".to_string(),
            "line 5: bad_after".to_string(),
        ],
        "stderr: {stderr}"
    );
}

#[test]
fn nested_comsub_alias_newlines_inside_dollar_paren_and_backtick() {
    let (stderr, _code) = run_script(&[
        "shopt -s expand_aliases",
        "alias foo=$'echo 1\\n\\nquux\\n'",
        "x=$(echo $(foo); outer_bad)",
        "y=`echo $(foo); bt_bad`",
        "bad_after",
    ]);
    assert_eq!(
        missing_lines(&stderr),
        vec![
            "line 3: quux".to_string(),
            "line 3: outer_bad".to_string(),
            "line 4: quux".to_string(),
            "line 4: bt_bad".to_string(),
            "line 5: bad_after".to_string(),
        ],
        "stderr: {stderr}"
    );
}

#[test]
fn function_with_comsub_alias_newlines_does_not_inflate_lines() {
    let (stderr, _code) = run_script(&[
        "shopt -s expand_aliases",
        "alias foo=$'echo 1\\n\\nquux\\n'",
        "f() { x=$(foo); }",
        "f",
        "bad_after",
    ]);
    assert_eq!(
        missing_lines(&stderr),
        vec!["line 4: quux".to_string(), "line 5: bad_after".to_string()],
        "stderr: {stderr}"
    );
}
