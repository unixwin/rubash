//! rubash#3089 — a redirect target joining a quoted space segment with a
//! substitution/parameter suffix reported `ambiguous redirect`.
//!
//! GNU ground truth: redir.c:298 `redirection_expand` runs the word through
//! expand_words_no_vars (subst.c:12590) and returns NULL only when the
//! expanded word list has zero or more than one field (redir.c:325-333).
//! Quoted text is CTLESC-protected as it is added (subst.c:4773
//! `quote_string`, via subst.c:11862 `add_quoted_string`), so a space inside
//! a quoted segment never splits a field (subst.c:12397 `word_list_split`):
//! `'a b c'$(printf '')` is ONE field and GNU creates the file `a b c`.
//! An UNQUOTED expansion result that itself carries IFS whitespace still
//! splits and stays ambiguous (`'a b c'$(printf 'x y')`, `'a b c'$V` with
//! `V='x y'`), and the split is judged on the live IFS, not on whitespace
//! alone (`IFS=:` splits on `:`; a space is then ordinary data).
//!
//! Expectations verified against GNU bash 5.3.20 with script files
//! (probe matrix in the lane REPORT).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3089-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(dir: &Path, script: &str) -> Run {
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn expect_created(dir: &Path, run: &Run, file: &str, content: &str) {
    assert_eq!(run.stdout, "rc=0\n", "stderr: {}", run.stderr);
    assert!(
        !run.stderr.contains("ambiguous"),
        "unexpected ambiguity diagnostic: {}",
        run.stderr
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(file)).expect("read created target"),
        content,
        "created file {file:?} content"
    );
}

fn expect_ambiguous(dir: &Path, run: &Run) {
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("ambiguous redirect"),
        "expected ambiguity diagnostic, got: {}",
        run.stderr
    );
    // Nothing was created: the directory holds only the probe script.
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["probe.sh".to_string()],
        "no file may be created"
    );
}

#[test]
fn quoted_space_plus_empty_comsub_creates_file() {
    let dir = tempdir("comsub-empty");
    let run = run(
        &dir,
        "printf 'hello\\n' > 'a b c'$(printf ''); echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a b c", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quoted_space_plus_empty_param_creates_file() {
    let dir = tempdir("param-empty");
    let run = run(
        &dir,
        "V=\nprintf 'hello\\n' > 'a b c'${V}; echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a b c", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quoted_backslash_space_plus_comsub_creates_file() {
    let dir = tempdir("backslash");
    let run = run(
        &dir,
        "printf 'hello\\n' > 'a\\$b c'$(printf ''); echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a\\$b c", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pure_quoted_space_control_creates_file() {
    let dir = tempdir("pure-quoted");
    let run = run(&dir, "printf 'hello\\n' > 'a b c'; echo \"rc=$?\"\n");
    expect_created(&dir, &run, "a b c", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn double_quoted_space_plus_empty_comsub_creates_file() {
    let dir = tempdir("dq-comsub");
    let run = run(
        &dir,
        "printf 'hello\\n' > \"a b c\"$(printf ''); echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a b c", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quoted_space_plus_nonempty_comsub_creates_joined_file() {
    let dir = tempdir("comsub-z");
    let run = run(
        &dir,
        "printf 'hello\\n' > 'a b c'$(printf 'Z'); echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a b cZ", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quoted_space_plus_nospace_param_creates_joined_file() {
    let dir = tempdir("param-xy");
    let run = run(
        &dir,
        "V=xy\nprintf 'hello\\n' > 'a b c'${V}; echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a b cxy", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn escaped_space_plus_empty_comsub_creates_file() {
    let dir = tempdir("escaped");
    let run = run(
        &dir,
        "printf 'hello\\n' > a\\ b$(printf ''); echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a b", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quoted_space_plus_empty_backquote_creates_file() {
    let dir = tempdir("backquote");
    let run = run(
        &dir,
        "printf 'hello\\n' > 'a b c'`printf ''`; echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a b c", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn empty_comsub_prefix_plus_quoted_space_creates_file() {
    let dir = tempdir("prefix");
    let run = run(
        &dir,
        "printf 'hello\\n' > $(printf '')'a b c'; echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a b c", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ansi_c_quoted_space_plus_empty_comsub_creates_file() {
    let dir = tempdir("ansi");
    let run = run(
        &dir,
        "printf 'hello\\n' > $'a b'$(printf ''); echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a b", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quoted_space_plus_trailing_space_param_creates_file() {
    // GNU: a trailing IFS blank in the glued expansion result ends the
    // single field; it does not create a second one.
    let dir = tempdir("param-trail");
    let run = run(
        &dir,
        "V='x '\nprintf 'hello\\n' > 'a b'${V}; echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a bx", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn input_redirect_quoted_space_plus_comsub_reads_file() {
    let dir = tempdir("input");
    std::fs::write(dir.join("a b c"), "in\n").expect("seed");
    let run = run(&dir, "cat < 'a b c'$(printf ''); echo \"rc=$?\"\n");
    assert_eq!(run.stdout, "in\nrc=0\n", "stderr: {}", run.stderr);
    assert!(!run.stderr.contains("ambiguous"), "stderr: {}", run.stderr);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn append_redirect_quoted_space_plus_empty_param_appends() {
    let dir = tempdir("append");
    let run = run(
        &dir,
        "printf 'x\\n' > 'a b c'$(printf '')\nV=\nprintf 'y\\n' >> 'a b c'${V}; echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "a b c", "x\ny\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quoted_space_plus_space_comsub_stays_ambiguous() {
    let dir = tempdir("comsub-space");
    let run = run(
        &dir,
        "printf 'hello\\n' > 'a b c'$(printf 'x y'); echo \"rc=$?\"\n",
    );
    expect_ambiguous(&dir, &run);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quoted_space_plus_space_param_stays_ambiguous() {
    let dir = tempdir("param-space");
    let run = run(
        &dir,
        "V='x y'\nprintf 'hello\\n' > 'a b c'${V}; echo \"rc=$?\"\n",
    );
    expect_ambiguous(&dir, &run);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quoted_space_plus_leading_space_param_stays_ambiguous() {
    let dir = tempdir("param-lead");
    let run = run(
        &dir,
        "V=' x'\nprintf 'hello\\n' > 'a b'${V}; echo \"rc=$?\"\n",
    );
    expect_ambiguous(&dir, &run);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unquoted_var_split_stays_ambiguous() {
    let dir = tempdir("varsplit");
    let run = run(&dir, "V='a b'\nprintf 'hello\\n' > $V; echo \"rc=$?\"\n");
    expect_ambiguous(&dir, &run);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn custom_ifs_colon_param_split_stays_ambiguous() {
    // With IFS=:, an unquoted expansion result carrying `:` splits even
    // though the joined target's only whitespace is the quoted one.
    let dir = tempdir("ifs-colon-split");
    let run = run(
        &dir,
        "IFS=:; V='a:b'\nprintf 'hello\\n' > 'x y'${V}; echo \"rc=$?\"\n",
    );
    expect_ambiguous(&dir, &run);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn custom_ifs_space_is_data_when_not_in_ifs() {
    // With IFS=:, a space inside an unquoted expansion result is ordinary
    // data, not a field separator.
    let dir = tempdir("ifs-colon-space");
    let run = run(
        &dir,
        "IFS=:; V='a b'\nprintf 'hello\\n' > 'x y'${V}; echo \"rc=$?\"\n",
    );
    expect_created(&dir, &run, "x ya b", "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
}
