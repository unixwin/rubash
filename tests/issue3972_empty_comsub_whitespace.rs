use std::{env, process::Command};

// rubash#3972: same-line whitespace-only funsub (`${ }`, `${  }`, `${\t}`)
// failed with `bad substitution` / `unexpected EOF while looking for matching '}'`
// because `continuation.rs` initialized `term_ready: false` for funsub pushes,
// whereas GNU `parse.y:1115` (`funsub: DOLBRACE newline_list '}'`) and
// `parse.y:4545` (`current_token = '\n'`) start the body at command-terminator
// position.

fn run_c(script: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c probe")
}

fn assert_clean(script: &str, expected_stdout: &str) {
    let output = run_c(script);
    assert!(
        output.status.success(),
        "script {script:?} failed (status={:?}): {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected_stdout);
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn empty_same_line_funsub_and_comsub_variants() {
    assert_clean("echo blank --${ }--", "blank ----\n");
    assert_clean("echo blank --${  }--", "blank ----\n");
    assert_clean("echo blank --${\t}--", "blank ----\n");
    assert_clean("echo blank --${ \n}--", "blank ----\n");
    assert_clean("echo blank --$()--", "blank ----\n");
    assert_clean("echo blank --$(\n)--", "blank ----\n");
    assert_clean("echo pipe --${| }--", "pipe ----\n");
    assert_clean("echo \"quoted --${  }--\"", "quoted ----\n");
}

#[test]
fn nonempty_funsub_without_terminator_still_fails() {
    let output = run_c("echo ${ echo hi }");
    assert!(!output.status.success(), "expected syntax/EOF error");
}
