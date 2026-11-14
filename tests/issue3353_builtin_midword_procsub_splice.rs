//! rubash#3353: a mid-word (embedded) process substitution in a BUILTIN
//! command's argument word is not spliced to its `/dev/fd/N` path — the
//! body is spawned and runs (the output-substitution body writes its
//! file), but the builtin receives the raw span text. The external-command
//! argument route splices normally.
//!
//! GNU bash 5.3.20 (issue repro, script-file form):
//!   echo pre>(printf 'hello\n' > ./cap.txt)post   -> pre/dev/fd/63post
//!   printf '%s\n' pre>(printf 'hello\n' > ./cap.txt)post -> pre/dev/fd/63post
//!   echo pre<(printf 'hello\n')post               -> pre/dev/fd/63post
//! and cap.txt holds `hello` in every case (the body ran).
//!
//! GNU anchor: subst.c:11364-11378 (`expand_word_internal`, the `<`/`>`
//! arms) — `extract_process_subst` + `process_substitute` splice the
//! `/dev/fd/N` result into the word being expanded, with no distinction
//! between builtin and external consumers.
//!
//! Faces:
//!   01 — `echo pre>(...)post` (output procsub, single-quoted body)
//!   02 — `printf '%s\n' pre>(...)post` (second builtin)
//!   03 — `echo pre<(...)post` (input procsub)
//!   04 — control: `/usr/bin/echo pre>(...)post` (external route, green
//!        on base and head)
//!   05 — control: word-initial whole-word procsub `echo >(…)` (green on
//!        base and head)
//!   06 — control: assignment RHS mid-word span `x=pre>(…)post` (green on
//!        base and head)
//!   07 — `echo pre>(...)post` with a double-quoted body (same splice
//!        face, different body quoting)

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn rubash_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT");
    command
}

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3353-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_single_process(dir: &Path, script: &str) -> std::process::Output {
    let path = dir.join("single.sh");
    fs::write(&path, script).expect("write single");
    let mut command = rubash_command();
    command.current_dir(dir).arg(shell_test_path(&path));
    command.output().expect("run single")
}

/// Normalize the fd number out of a `/dev/fd/N` path so the pin does not
/// depend on which descriptor the shell handed the substitution (GNU
/// printed 63, rubash 62 in the issue's probes — both are the same face).
fn normalize_dev_fd(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("/dev/fd/") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + "/dev/fd/".len()..];
        let digits: usize = after.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            out.push_str("/dev/fd/");
            rest = after;
        } else {
            out.push_str("/dev/fd/N");
            rest = &after[digits..];
        }
    }
    out.push_str(rest);
    out
}

/// The output-substitution body runs asynchronously; poll briefly for its
/// side-effect file instead of racing the shell's settle.
fn read_cap_or_empty(dir: &Path) -> String {
    for _ in 0..40 {
        if let Ok(content) = fs::read_to_string(dir.join("cap.txt")) {
            if !content.is_empty() {
                return content;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    fs::read_to_string(dir.join("cap.txt")).unwrap_or_default()
}

// 01 — the issue shape, echo + output procsub. GNU: `pre/dev/fd/63post`
// and cap.txt holds hello. Base prints the raw span text.
#[test]
fn issue3353_01_echo_midword_output_procsub_splices_dev_fd() {
    let dir = test_dir("01");
    let output = run_single_process(
        &dir,
        "echo pre>(printf 'hello\\n' > ./cap.txt)post\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        normalize_dev_fd(&String::from_utf8_lossy(&output.stdout)),
        "pre/dev/fd/Npost\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(read_cap_or_empty(&dir), "hello\n", "cap.txt");
    let _ = fs::remove_dir_all(&dir);
}

// 02 — the same face through the printf builtin.
#[test]
fn issue3353_02_printf_midword_output_procsub_splices_dev_fd() {
    let dir = test_dir("02");
    let output = run_single_process(
        &dir,
        "printf '%s\\n' pre>(printf 'hello\\n' > ./cap.txt)post\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        normalize_dev_fd(&String::from_utf8_lossy(&output.stdout)),
        "pre/dev/fd/Npost\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(read_cap_or_empty(&dir), "hello\n", "cap.txt");
    let _ = fs::remove_dir_all(&dir);
}

// 03 — input procsub mid-word: `echo pre<(printf 'hello\n')post`.
#[test]
fn issue3353_03_echo_midword_input_procsub_splices_dev_fd() {
    let dir = test_dir("03");
    let output = run_single_process(&dir, "echo pre<(printf 'hello\\n')post\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        normalize_dev_fd(&String::from_utf8_lossy(&output.stdout)),
        "pre/dev/fd/Npost\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let _ = fs::remove_dir_all(&dir);
}

// 04 — control: the external route splices the same word (green on base).
#[test]
fn issue3353_04_external_midword_output_procsub_splices_dev_fd() {
    let dir = test_dir("04");
    let output = run_single_process(
        &dir,
        "/usr/bin/echo pre>(printf 'hello\\n' > ./cap.txt)post\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        normalize_dev_fd(&String::from_utf8_lossy(&output.stdout)),
        "pre/dev/fd/Npost\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(read_cap_or_empty(&dir), "hello\n", "cap.txt");
    let _ = fs::remove_dir_all(&dir);
}

// 05 — control: a word-initial whole-word procsub splices (green on base).
#[test]
fn issue3353_05_echo_word_initial_output_procsub_splices_dev_fd() {
    let dir = test_dir("05");
    let output = run_single_process(&dir, "echo >(printf 'hello\\n' > ./cap.txt)\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        normalize_dev_fd(&String::from_utf8_lossy(&output.stdout)),
        "/dev/fd/N\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(read_cap_or_empty(&dir), "hello\n", "cap.txt");
    let _ = fs::remove_dir_all(&dir);
}

// 06 — control: assignment RHS mid-word span splices (green on base).
#[test]
fn issue3353_06_assignment_rhs_midword_output_procsub_splices_dev_fd() {
    let dir = test_dir("06");
    let output = run_single_process(
        &dir,
        "x=pre>(printf 'hello\\n' > ./cap.txt)post\necho \"$x\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        normalize_dev_fd(&String::from_utf8_lossy(&output.stdout)),
        "pre/dev/fd/Npost\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(read_cap_or_empty(&dir), "hello\n", "cap.txt");
    let _ = fs::remove_dir_all(&dir);
}

// 07 — double-quoted body: same splice face, different body quoting.
#[test]
fn issue3353_07_echo_midword_output_procsub_double_quoted_body() {
    let dir = test_dir("07");
    let output = run_single_process(
        &dir,
        "echo pre>(printf \"hello\\n\" > ./cap.txt)post\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        normalize_dev_fd(&String::from_utf8_lossy(&output.stdout)),
        "pre/dev/fd/Npost\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(read_cap_or_empty(&dir), "hello\n", "cap.txt");
    let _ = fs::remove_dir_all(&dir);
}
