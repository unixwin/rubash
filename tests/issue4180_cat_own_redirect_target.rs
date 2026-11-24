//! Issue rubash#4180 — an operand naming the command's OWN redirect
//! target (`cat out >> out`, `cat out > out`) never reached the
//! emulated cat's operand serve at all: `parse_cat_argv` re-derived
//! redirection from word text and dropped every word equal to a
//! redirect target (and every word shaped like a redirect operator,
//! plus the word after it). With the operand list emptied, the serve
//! fell through to the no-operand stdin drain — `cat out >> out`
//! blocked forever on a live fd 0 (and silently appended whatever
//! stdin delivered when stdin had data), the `>` form truncating the
//! file first. `cmd.words` is pure argv — the parser already separated
//! every redirection into the node's redirect fields — so the parser
//! now keeps all words, and the operand reaches the plain serve,
//! where the rubash#4175 same-file judgment (`cat_operand_is_output_
//! file`, the one predicate both faces share) refuses the append
//! form exactly like GNU.
//!
//! GNU envelope (cloud GNU Bash 5.3.20 oracle and a local GNU
//! coreutils 9.4 cat, byte-identical on every shape below, script
//! files throughout): append refuses with `cat: out: input file is
//! output file`, status 1, the file untouched and stdin unread; the
//! truncate form's target is emptied by the redirection before cat
//! runs, so cat serves an empty file, status 0, silent — stdin bytes
//! are never written into it; a quoted `>` is an operand like any
//! other (`cat: '>': No such file or directory`, status 1, later
//! operands still served); the pipeline last stage refuses the same
//! way (its diagnostic carries the stage's usual script prefix).
//!
//! Every run goes through a bounded wait: the pristine failure mode
//! is a hang, so each pin carries its own timeout guard and a hang
//! records as a failure, never as a stuck suite.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

/// How the script's fd 0 is provided. The pristine drain reads the
/// process stdin, so the stdin shape is part of each pin's verdict:
/// a data pipe exposes the mis-serve as bytes, a held-open pipe
/// exposes it as a hang.
enum Stdin {
    Null,
    /// Bytes are written and the pipe is closed before waiting.
    DataThenEof(&'static str),
    /// The pipe stays open and empty for the whole run.
    HeldOpen,
}

fn tempdir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash4180-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

struct Run {
    stdout: String,
    stderr: String,
    timed_out: bool,
    files: HashMap<String, Option<String>>,
}

/// Run `script` as probe.sh in a fresh tempdir; capture the process
/// streams (via files, so a blocked child can never fill a pipe the
/// harness is not reading) plus the named artifacts. The child is
/// given at most 20 seconds; past that it is killed and the run is
/// marked timed out — on pristine code the self-redirect shapes hang,
/// and the timeout IS the fail-first signature.
fn run_script(tag: &str, script: &str, stdin: Stdin) -> Run {
    let dir = tempdir(tag);
    std::fs::write(dir.join("probe.sh"), script).expect("write script");
    let stdout_file = std::fs::File::create(dir.join("stdout.txt")).expect("stdout file");
    let stderr_file = std::fs::File::create(dir.join("stderr.txt")).expect("stderr file");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg("probe.sh")
        .current_dir(&dir)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));
    match stdin {
        Stdin::Null => {
            command.stdin(Stdio::null());
        }
        Stdin::DataThenEof(_) | Stdin::HeldOpen => {
            command.stdin(Stdio::piped());
        }
    }
    let mut child: Child = command.spawn().expect("run rubash");
    if let Stdin::DataThenEof(data) = stdin {
        let mut pipe = child.stdin.take().expect("stdin pipe");
        pipe.write_all(data.as_bytes()).expect("write stdin");
        drop(pipe);
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut timed_out = false;
    loop {
        match child.try_wait().expect("poll child") {
            Some(_) => break,
            None => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let mut files = HashMap::new();
    for name in ["out", "other", "status.txt"] {
        files.insert(
            name.to_string(),
            std::fs::read_to_string(dir.join(name)).ok(),
        );
    }
    let run = Run {
        stdout: std::fs::read_to_string(dir.join("stdout.txt")).unwrap_or_default(),
        stderr: std::fs::read_to_string(dir.join("stderr.txt")).unwrap_or_default(),
        timed_out,
        files,
    };
    let _ = std::fs::remove_dir_all(&dir);
    run
}

#[test]
fn own_append_target_refused_with_null_stdin() {
    let run = run_script(
        "append-null",
        "printf 'hello\\n' > out\ncat out >> out\necho \"cat_rc=$?\" > status.txt\n",
        Stdin::Null,
    );
    assert!(!run.timed_out, "run hung (pristine stdin-drain signature)");
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=1\n"));
    assert!(
        run.stderr.contains("cat: out: input file is output file"),
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.files["out"].as_deref(), Some("hello\n"));
}

#[test]
fn own_append_target_refused_with_data_stdin() {
    // With data on fd 0 the pristine drain "succeeds" — it appends
    // the stdin bytes to the target and reports 0. GNU refuses and
    // never touches stdin.
    let run = run_script(
        "append-data",
        "printf 'hello\\n' > out\ncat out >> out\necho \"cat_rc=$?\" > status.txt\n",
        Stdin::DataThenEof("PIPEIN\n"),
    );
    assert!(!run.timed_out, "run hung (pristine stdin-drain signature)");
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=1\n"));
    assert!(
        run.stderr.contains("cat: out: input file is output file"),
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.files["out"].as_deref(), Some("hello\n"));
}

#[test]
fn own_append_target_never_blocks_on_held_open_stdin() {
    let run = run_script(
        "append-held",
        "printf 'hello\\n' > out\ncat out >> out\necho \"cat_rc=$?\" > status.txt\n",
        Stdin::HeldOpen,
    );
    assert!(
        !run.timed_out,
        "cat out >> out blocked on fd 0 (the #4180 deadlock)"
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=1\n"));
    assert!(
        run.stderr.contains("cat: out: input file is output file"),
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.files["out"].as_deref(), Some("hello\n"));
}

#[test]
fn own_truncate_target_serves_empty_with_data_stdin() {
    // GNU: the redirection empties the file before cat runs, cat
    // serves the empty operand, status 0 — the stdin bytes are not
    // cat's input and must not land in the file.
    let run = run_script(
        "truncate-data",
        "printf 'hello\\n' > out\ncat out > out\necho \"cat_rc=$?\" > status.txt\n",
        Stdin::DataThenEof("PIPEIN\n"),
    );
    assert!(!run.timed_out, "run hung (pristine stdin-drain signature)");
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=0\n"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.files["out"].as_deref(), Some(""));
}

#[test]
fn own_truncate_target_never_blocks_on_held_open_stdin() {
    let run = run_script(
        "truncate-held",
        "printf 'hello\\n' > out\ncat out > out\necho \"cat_rc=$?\" > status.txt\n",
        Stdin::HeldOpen,
    );
    assert!(
        !run.timed_out,
        "cat out > out blocked on fd 0 (the #4180 deadlock)"
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=0\n"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.files["out"].as_deref(), Some(""));
}

#[test]
fn redirect_first_operand_survives() {
    // `cat > out out`: the operand follows the redirection in the
    // text and spells the target — it is still the operand. GNU
    // truncates, then serves the empty file; stdin is never read.
    let run = run_script(
        "redirect-first",
        "printf 'hello\\n' > out\ncat > out out\necho \"cat_rc=$?\" > status.txt\n",
        Stdin::DataThenEof("PIPEIN\n"),
    );
    assert!(!run.timed_out, "run hung (pristine stdin-drain signature)");
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=0\n"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.files["out"].as_deref(), Some(""));
}

#[test]
fn quoted_operator_word_is_an_operand() {
    // A quoted `>` is data, not redirection: GNU tries to open it,
    // reports it through quotef, and still serves the later operand.
    let run = run_script(
        "quoted-op",
        "printf 'seed\\n' > other\ncat '>' other\necho \"first_rc=$?\" > status.txt\ncat '>'\necho \"second_rc=$?\" >> status.txt\n",
        Stdin::Null,
    );
    assert!(!run.timed_out, "run hung (pristine stdin-drain signature)");
    assert_eq!(
        run.files["status.txt"].as_deref(),
        Some("first_rc=1\nsecond_rc=1\n")
    );
    assert_eq!(run.stdout, "seed\n");
    assert_eq!(
        run.stderr
            .matches("cat: '>': No such file or directory")
            .count(),
        2,
        "stderr: {:?}",
        run.stderr
    );
}

#[test]
fn pipeline_last_stage_own_append_target_refused() {
    // The stage arm shares parse_cat_argv, so it dropped the operand
    // the same way and served the stage's stdin instead. GNU refuses
    // (the stage diagnostic carries the pipeline face's script
    // prefix, like its other diagnostics).
    let run = run_script(
        "pipeline-last",
        "printf 'hello\\n' > out\ntrue | cat out >> out\necho \"cat_rc=$?\" > status.txt\n",
        Stdin::Null,
    );
    assert!(!run.timed_out, "run hung (pristine stdin-drain signature)");
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=1\n"));
    assert!(
        run.stderr.contains("cat: out: input file is output file"),
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.files["out"].as_deref(), Some("hello\n"));
}

#[test]
fn different_target_append_still_serves() {
    // Guard: operands naming some OTHER file were never the bug.
    let run = run_script(
        "different-target",
        "printf 'hello\\n' > out\nprintf 'seed\\n' > other\ncat other >> out\necho \"cat_rc=$?\" > status.txt\n",
        Stdin::Null,
    );
    assert!(!run.timed_out);
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=0\n"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.files["out"].as_deref(), Some("hello\nseed\n"));
}

#[test]
fn operand_with_input_redirect_of_same_name_still_serves() {
    // Guard: `cat out < out` — GNU serves the operand and ignores
    // fd 0 (operands are present). Pristine reached the same bytes
    // by accident through the drain; the serve must stay correct
    // now that the operand goes through the plain path.
    let run = run_script(
        "input-same-name",
        "printf 'hello\\n' > out\ncat out < out\necho \"cat_rc=$?\" > status.txt\n",
        Stdin::Null,
    );
    assert!(!run.timed_out);
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=0\n"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.stdout, "hello\n");
}
