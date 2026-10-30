//! rubash#2090 — the `exec` persistent-redirect arms (the `{fd}` var
//! arm and the plain stdio arm in execute_stdio_only_exec_redirect /
//! execute_dynamic_fd_var_redirect) must not materialize a QUOTED
//! process-substitution target by VALUE.
//!
//! GNU: a `<(`/`>(` span is a process substitution only when the
//! parser saw it UNQUOTED — expand_word_internal (subst.c:11365-11372)
//! refuses the span when the `<` was quoted (Q_DOUBLE_QUOTES). A quoted
//! `"<(touch MARKER)"` redirect target is a literal filename: redir.c
//! redir_open opens that name (fails ENOENT for `<`, creates it for
//! `>`/`>>`/`<>`), and the substitution body never runs.
//!
//! rubash matched the exec arms' strip_prefix("<(")/strip_prefix(">(")
//! on the quote-stripped target value, so `exec {fd}< "<(touch MARKER)"`
//! spawned the body (MARKER created) and reported nothing — the
//! materialized "success" swallowed the diagnostic whole. The fix gates
//! every procsub admission in the exec persistent path on the same
//! recorded-span check the other materialization arms use since
//! rubash#1931 (external_setup redirect_target_span_is_recorded).
//!
//! All expectations measured against GNU bash 5.3.20 with script files
//! (~/workspace/rubash-audit GNU oracle, 2026-10-29).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    /// Files left in the case dir (excluding case.sh), name -> contents.
    files: BTreeMap<String, String>,
}

fn run(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2090-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg(&case)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(&dir).expect("list case dir") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != "case.sh" {
            let contents = std::fs::read_to_string(entry.path()).unwrap_or_default();
            files.insert(name, contents);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
        files,
    }
}

#[test]
fn issue2090_exec_fdvar_quoted_input_target_is_literal() {
    // The seed: GNU reports the literal open's ENOENT, the body never
    // runs, and the shell survives (the {fd} open failure is not fatal).
    let run = run("seed", "exec {fd}< \"<(touch MARKER)\"\necho REACHED\n");
    assert!(
        !run.files.contains_key("MARKER"),
        "quoted procsub body must not execute: files: {:?}",
        run.files.keys().collect::<Vec<_>>()
    );
    assert!(
        run.stderr
            .contains("<(touch MARKER): No such file or directory"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.stdout, "REACHED\n", "stdout: {}", run.stdout);
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue2090_exec_fdvar_quoted_readwrite_target_creates_literal() {
    // `<>` opens O_RDWR|O_CREAT (make_cmd.c:682): the literal name is
    // created, the body never runs, no diagnostic.
    let run = run("rw", "exec {fd}<> \"<(touch MARKER)\"\necho REACHED\n");
    assert!(!run.files.contains_key("MARKER"), "files: {:?}", run.files);
    assert!(
        run.files.contains_key("<(touch MARKER)"),
        "the literal file must be created: files: {:?}",
        run.files.keys().collect::<Vec<_>>()
    );
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "REACHED\n");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue2090_exec_fdvar_quoted_output_targets_create_literal() {
    for (tag, op) in [("out", ">"), ("app", ">>")] {
        let run = run(
            tag,
            &format!("exec {{fd}}{op} \">(touch MARKER)\"\necho REACHED\n"),
        );
        assert!(
            !run.files.contains_key("MARKER"),
            "{op}: files: {:?}",
            run.files
        );
        assert!(
            run.files.contains_key(">(touch MARKER)"),
            "{op}: the literal file must be created: files: {:?}",
            run.files.keys().collect::<Vec<_>>()
        );
        assert_eq!(run.stdout, "REACHED\n", "{op} stdout: {}", run.stdout);
        assert_eq!(run.code, Some(0), "{op}");
    }
}

#[test]
fn issue2090_exec_plain_quoted_output_target_is_literal() {
    // The unnamed exec arm shares the admission: `exec > ">(...)"`
    // writes following output into the literal file.
    let run = run("plainout", "exec > \">(touch MARKER)\"\necho REACHED\n");
    assert!(!run.files.contains_key("MARKER"), "files: {:?}", run.files);
    assert_eq!(
        run.files.get(">(touch MARKER)").map(String::as_str),
        Some("REACHED\n"),
        "files: {:?}",
        run.files
    );
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue2090_exec_quoted_input_reads_existing_literal() {
    // When the literal file exists, the exec input arm must read IT,
    // not spawn the span's body.
    let run = run(
        "lit",
        "printf 'LITERAL\\n' > \"<(echo hi)\"\nexec < \"<(echo hi)\"\nread x\necho \"x=$x\"\n",
    );
    assert_eq!(run.stdout, "x=LITERAL\n", "stdout: {}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue2090_exec_unquoted_procsub_still_materializes() {
    let run_in = run("control-in", "exec {fd}< <(echo hi)\ncat <&$fd\n");
    assert_eq!(run_in.stdout, "hi\n", "stdout: {}", run_in.stdout);
    assert_eq!(run_in.stderr, "", "stderr: {}", run_in.stderr);
    assert_eq!(run_in.code, Some(0));

    let run_out = run(
        "control-out",
        "exec {fd}> >(cat > CAPTURE)\necho hello >&$fd\nexec {fd}>&-\nsleep 1\ncat CAPTURE\n",
    );
    assert_eq!(run_out.stdout, "hello\n", "stdout: {}", run_out.stdout);
    assert_eq!(run_out.code, Some(0));
}
