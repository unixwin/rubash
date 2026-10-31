//! rubash#2390 — process-substitution words in a `for` list were
//! re-parsed on the for-list word production path: two adjacent
//! procsub words glued into one iteration value
//! (`/dev/fd/63/dev/fd/62`), backslashes inside a procsub body were
//! eaten before the body ran, and a quoted `"<(...)"` word still
//! materialised as a process substitution instead of staying
//! literal. Direct procsub positions (command arguments, array
//! assignment) were unaffected — the defect is for-list-only.
//!
//! GNU ground: the parser hands the expander a WORD_LIST whose
//! boundaries are structural — one WORD_DESC per word
//! (parse.y word_list production) — and expand_words
//! (subst.c:12582) expands each word on its own. Inside
//! expand_word_internal the `<`/`>` case (subst.c:11369-11391)
//! materialises a process substitution only when the `<(` is not
//! inside double quotes (`quoted & (Q_HERE_DOCUMENT|Q_DOUBLE_QUOTES)`
//! blocks it) and extracts the body verbatim with
//! extract_process_subst, so the body's bytes — quotes and
//! backslashes included — reach process_substitute (subst.c:6362)
//! untouched. A for list therefore iterates once per parser word,
//! each procsub pathname separate, each body byte-exact, and a
//! quoted procsub spelling stays a literal word.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle)
//! with script-file probes; /dev/fd numbers are normalised before
//! comparison. All for-list cells are red on master 5727567f.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Outcome {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

/// Run `script` (written to a temp file) under the shell at `bin`
/// with a bounded wait; /dev/fd digits are normalised to `N`.
fn run_script(bin: &str, tag: &str, script: &str) -> Outcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue2390-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write probe script");
    let mut child = Command::new(bin)
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn shell");
    let mut out_pipe = child.stdout.take().expect("stdout pipe");
    let mut err_pipe = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status.code(),
            None => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    panic!("shell did not exit within 30s for probe {tag}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    let stdout = out_thread.join().expect("join stdout drain");
    let stderr = err_thread.join().expect("join stderr drain");
    let _ = std::fs::remove_dir_all(&dir);
    Outcome {
        stdout: normalize_fd_numbers(&String::from_utf8_lossy(&stdout)),
        stderr: normalize_fd_numbers(&String::from_utf8_lossy(&stderr)),
        code,
    }
}

fn normalize_fd_numbers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("/dev/fd/") {
        out.push_str(&rest[..pos]);
        out.push_str("/dev/fd/");
        let after = &rest[pos + "/dev/fd/".len()..];
        let digits: usize = after.chars().take_while(|c| c.is_ascii_digit()).count();
        out.push('N');
        rest = &after[digits..];
    }
    out.push_str(rest);
    out
}

fn assert_case(tag: &str, script: &str, want_stdout: &str, want_code: i32) {
    let got = run_script(RUBASH, tag, script);
    assert_eq!(got.stdout, want_stdout, "stdout for {tag}");
    assert_eq!(got.stderr, "", "stderr for {tag}");
    assert_eq!(got.code, Some(want_code), "exit code for {tag}");
}

/// Glue family: two adjacent procsub words iterate TWICE, each
/// pathname its own word (GNU: "one\ntwo\n", rc 0).
#[cfg(unix)]
#[test]
fn issue2390_two_adjacent_procsubs_iterate_separately() {
    assert_case(
        "glue2",
        "for f in <(echo one) <(echo two); do cat \"$f\"; done\n",
        "one\ntwo\n",
        0,
    );
}

/// Glue family: three adjacent procsub words.
#[cfg(unix)]
#[test]
fn issue2390_three_adjacent_procsubs_iterate_separately() {
    assert_case(
        "glue3",
        "for f in <(echo a) <(echo b) <(echo c); do cat \"$f\"; done\n",
        "a\nb\nc\n",
        0,
    );
}

/// Glue family: a plain word between two procsubs keeps its own
/// iteration slot and order.
#[cfg(unix)]
#[test]
fn issue2390_procsub_plain_procsub_order_and_count() {
    assert_case(
        "gluemix",
        "for f in <(echo one) mid <(echo two); do \
         if [ \"$f\" = mid ]; then echo MID; else cat \"$f\"; fi; done\n",
        "one\nMID\ntwo\n",
        0,
    );
}

/// Glue family: a plain word AFTER a procsub is its own iteration.
#[cfg(unix)]
#[test]
fn issue2390_procsub_then_plain_word() {
    assert_case(
        "gluetail",
        "for f in <(echo one) tail; do \
         if [ \"$f\" = tail ]; then echo TAIL; else cat \"$f\"; fi; done\n",
        "one\nTAIL\n",
        0,
    );
}

/// Glue family control: a single procsub word (green before and
/// after the fix).
#[cfg(unix)]
#[test]
fn issue2390_single_procsub_control() {
    assert_case(
        "single",
        "for f in <(echo one); do cat \"$f\"; done\n",
        "one\n",
        0,
    );
}

/// Backslash family: `printf 'x\ny\n'` body keeps its backslashes,
/// so the produced stream has real newlines.
#[cfg(unix)]
#[test]
fn issue2390_procsub_body_backslashes_survive() {
    assert_case(
        "bs1",
        "for f in <(printf 'x\\ny\\n'); do cat \"$f\"; done\n",
        "x\ny\n",
        0,
    );
}

/// Backslash family: second morphology from the issue.
#[cfg(unix)]
#[test]
fn issue2390_procsub_body_backslashes_survive_ab_cd() {
    assert_case(
        "bs2",
        "for f in <(printf 'ab\\ncd\\n'); do cat \"$f\"; done\n",
        "ab\ncd\n",
        0,
    );
}

/// Backslash family: `%s\n` format body.
#[cfg(unix)]
#[test]
fn issue2390_procsub_body_format_backslash_survives() {
    assert_case(
        "bs3",
        "for f in <(printf '%s\\n' hello world); do cat \"$f\"; done\n",
        "hello\nworld\n",
        0,
    );
}

/// Backslash family control: a body without backslashes.
#[cfg(unix)]
#[test]
fn issue2390_procsub_body_without_backslashes_control() {
    assert_case(
        "bsctl",
        "for f in <(seq 1 3); do cat \"$f\"; done\n",
        "1\n2\n3\n",
        0,
    );
}

/// Quote family: a double-quoted procsub spelling is a LITERAL
/// word (subst.c:11369: Q_DOUBLE_QUOTES blocks materialisation);
/// the unquoted neighbour still materialises.
#[cfg(unix)]
#[test]
fn issue2390_double_quoted_procsub_stays_literal() {
    assert_case(
        "dq",
        "for f in \"<(echo one)\" <(echo two); do echo \"f=[$f]\"; done\n",
        "f=[<(echo one)]\nf=[/dev/fd/N]\n",
        0,
    );
}

/// Quote family: a single-quoted procsub spelling is literal too.
#[cfg(unix)]
#[test]
fn issue2390_single_quoted_procsub_stays_literal() {
    assert_case(
        "sq",
        "for f in '<(echo one)'; do echo \"f=[$f]\"; done\n",
        "f=[<(echo one)]\n",
        0,
    );
}

/// Adjacency control (rubash#1933 shape): text glued DIRECTLY to
/// a procsub span with no intervening whitespace is part of the
/// same word — the pathname takes the suffix.
#[cfg(unix)]
#[test]
fn issue2390_glued_suffix_stays_one_word() {
    assert_case(
        "suffix",
        "for f in <(echo hi)x; do echo \"[$f]\"; done\n",
        "[/dev/fd/Nx]\n",
        0,
    );
}

/// Adjacency control: a prefix glued directly to the span is part
/// of the same word too.
#[cfg(unix)]
#[test]
fn issue2390_glued_prefix_stays_one_word() {
    assert_case(
        "prefix",
        "for f in x<(echo hi); do echo \"[$f]\"; done\n",
        "[x/dev/fd/N]\n",
        0,
    );
}

/// Expansion results are data: a variable whose VALUE spells a
/// procsub is field-split as literal text, never materialised
/// (GNU: two iterations over the literal pieces).
#[cfg(unix)]
#[test]
fn issue2390_variable_value_procsub_text_stays_data() {
    assert_case(
        "varvalue",
        "x='<(echo hi)'\nfor f in $x; do echo \"[$f]\"; done\n",
        "[<(echo]\n[hi)]\n",
        0,
    );
}

/// Output-side procsub in a for list iterates per word as well.
#[cfg(unix)]
#[test]
fn issue2390_output_procsub_words_iterate_separately() {
    assert_case(
        "outglue",
        "for f in >(cat > out1.txt) >(cat > out2.txt); do echo \"f=[$f]\"; done\n",
        "f=[/dev/fd/N]\nf=[/dev/fd/N]\n",
        0,
    );
}
