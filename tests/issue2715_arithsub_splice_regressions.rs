//! rubash#2715 — a command substitution inside `$(( ))` whose
//! output (or the body text itself) contains `;` or a newline.
//!
//! GNU's classification is decided before any expansion: subst.c
//! param_expand's LPAREN case (subst.c:10842-10862) extracts the
//! `$(` body, requires the text after the second `(` to end in `)`
//! and pass chk_arithsub (subst.c:9727), and from then on the
//! construct is arithmetic for good — expand_arith_string splices
//! the substitution output into the expression as TEXT
//! (subst.c:7143 command_substitute returns the text), evalexp
//! parses it, and a parse failure is an arithmetic diagnostic
//! (expand_wdesc_error): the spliced text is never executed as
//! commands.
//!
//! rubash classified these shapes as arithmetic too, but its
//! `$((` arm fell back to EXECUTING the expression text as a
//! command substitution whenever evaluation failed and the text
//! contained both `;` and `(` — producing `3: command not found`
//! and even assigning the literal spliced text.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-08),
//! file mode, byte for byte (the diagnostic prefix is the script
//! name `case.sh`, matching how GNU prefixes it).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2715-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_exact(tag: &str, run: &Run, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(run.stdout, stdout, "{tag}: stdout");
    assert_eq!(run.stderr, stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(code), "{tag}: rc");
}

#[test]
fn issue2715_assignment_semicolon_body_is_arithmetic_error() {
    // The issue's own shape: the spliced text ` ; echo 1` is an
    // arithmetic syntax error; the assignment stores nothing and
    // the script continues. GNU does NOT print the literal text.
    let run = run_file(
        "assign",
        "y=$(( $(false); echo $?))\necho \"y=[$y]\"\necho AFTER\n",
    );
    assert_exact(
        "assign",
        &run,
        "y=[]\nAFTER\n",
        "case.sh: line 1: ; echo 1: arithmetic syntax error: operand expected (error token is \"; echo 1\")\n",
        0,
    );
}

#[test]
fn issue2715_assignment_semicolon_body_space_variant() {
    // A blank before `))` is part of the spliced text, so the
    // diagnostic echoes it (error token carries the blank too).
    let run = run_file(
        "assign-sp",
        "y=$(( $(false); echo $? ))\necho \"y=[$y]\"\necho AFTER\n",
    );
    assert_exact(
        "assign-sp",
        &run,
        "y=[]\nAFTER\n",
        "case.sh: line 1: ; echo 1 : arithmetic syntax error: operand expected (error token is \"; echo 1 \")\n",
        0,
    );
}

#[test]
fn issue2715_multiline_output_is_arithmetic_error() {
    // The substitution output `1\n2` splices as text; evalexp
    // fails on the `2`. Nothing is executed as a command, and the
    // enclosing script continues after the failed command.
    let run = run_file("multiline", "echo $(( $(echo 1; echo 2)))\necho AFTER\n");
    assert_exact(
        "multiline",
        &run,
        "AFTER\n",
        "case.sh: line 1: 1\n2: arithmetic syntax error in expression (error token is \"2\")\n",
        0,
    );
}

#[test]
fn issue2715_multiline_output_space_variant() {
    let run = run_file("multiline-sp", "echo $(( $(echo 1; echo 2) ))\n");
    assert_exact(
        "multiline-sp",
        &run,
        "",
        "case.sh: line 1: 1\n2 : arithmetic syntax error in expression (error token is \"2 \")\n",
        1,
    );
}

#[test]
fn issue2715_multiline_output_inside_expression() {
    let run = run_file("expr", "echo $(( $(echo 3; echo 4) + 1 ))\n");
    assert_exact(
        "expr",
        &run,
        "",
        "case.sh: line 1: 3\n4 + 1 : arithmetic syntax error in expression (error token is \"4 + 1 \")\n",
        1,
    );
}

#[test]
fn issue2715_semicolon_after_substitution_is_operand_error() {
    let run = run_file("semi", "echo $(( $(false); 5))\n");
    assert_exact(
        "semi",
        &run,
        "",
        "case.sh: line 1: ; 5: arithmetic syntax error: operand expected (error token is \"; 5\")\n",
        1,
    );
}

#[test]
fn issue2715_semicolon_inside_substitution_output() {
    // The output text `a;b` splices whole: `;b` is an invalid
    // arithmetic operator, not a command separator.
    let run = run_file("semi-out", "echo $(( $(echo 'a;b')))\n");
    assert_exact(
        "semi-out",
        &run,
        "",
        "case.sh: line 1: a;b: arithmetic syntax error: invalid arithmetic operator (error token is \";b\")\n",
        1,
    );
}

#[test]
fn issue2715_valid_substitutions_still_evaluate() {
    // Controls: single-value substitution output keeps evaluating,
    // nested arithmetic substitution works, and shapes GNU
    // classifies as command substitution (paren-balance rule)
    // still execute as substitutions.
    let run = run_file(
        "controls",
        "echo $(( $(echo 5) + 1 ))\n\
         echo $(( $(echo 2) * 3 ))\n\
         echo $(( $(echo $((1+1))) + 1 ))\n\
         echo $(( echo ab cde ) )\n\
         echo $((echo hi) || echo lo)\n",
    );
    assert_exact("controls", &run, "6\n6\n3\nab cde\nhi\n", "", 0);
}
