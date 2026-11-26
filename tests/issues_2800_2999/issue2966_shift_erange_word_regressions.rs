//! rubash#2966 — `shift`'s out-of-range diagnostic renders
//! GNU's `sh_erange` argument (builtins/common.c:252-259): the
//! raw count word when one was given, and no word segment at
//! all when the count is implicit (builtins/shift.def:74,86
//! pass `list ? list->word->word : NULL`). The executor's apply
//! step used to format the parsed amount instead, so `shift 03`
//! reported `3` and a bare out-of-range `shift` reported `1`.
//! Expectations below are byte-exact against GNU bash 5.3.20
//! run on the same script files (script-file mode; the stderr
//! prefix is the script name rubash was invoked with).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2966_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

fn check(body: &str, status: i32, stdout: &str, stderr: &str) {
    let run = run_script(body);
    assert_eq!(run.status, status, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, stdout, "stderr: {}", run.stderr);
    assert_eq!(run.stderr, stderr);
    let _ = std::fs::remove_dir_all(&run.dir);
}

#[test]
fn erange_renders_the_raw_word_not_the_parsed_count() {
    // shopt line 1, set line 2, shift line 3.
    check(
        "shopt -s shift_verbose\nset -- a b\nshift 03\necho \"rc=$?\"\n",
        0,
        "rc=1\n",
        "case.sh: line 3: shift: 03: shift count out of range\n",
    );
    check(
        "shopt -s shift_verbose\nset -- a b\nshift +3\necho \"rc=$?\"\n",
        0,
        "rc=1\n",
        "case.sh: line 3: shift: +3: shift count out of range\n",
    );
    check(
        "shopt -s shift_verbose\nset -- a b\nshift ' 3'\necho \"rc=$?\"\n",
        0,
        "rc=1\n",
        "case.sh: line 3: shift:  3: shift count out of range\n",
    );
    check(
        "shopt -s shift_verbose\nset -- a b\nshift '3 '\necho \"rc=$?\"\n",
        0,
        "rc=1\n",
        "case.sh: line 3: shift: 3 : shift count out of range\n",
    );
}

#[test]
fn implicit_count_erange_has_no_word_segment() {
    // The E3 matrix cell (shift--bare--posix): posix mode turns
    // print_shift_error on; the implicit count passes NULL.
    check(
        "set -o posix\nshift\necho \"rc=$?\"\n",
        0,
        "rc=1\n",
        "case.sh: line 2: shift: shift count out of range\n",
    );
}

#[test]
fn out_of_range_stays_silent_without_verbose_or_posix() {
    check("set -- a b\nshift 3\necho \"rc=$?\"\n", 0, "rc=1\n", "");
}

#[test]
fn negative_count_erange_keeps_the_raw_word() {
    // The negative site is unconditional in shift.def and
    // already rendered the word; guard it against the refactor.
    check(
        "set -- a b\nshift -2\necho \"rc=$?\"\n",
        0,
        "rc=1\n",
        "case.sh: line 2: shift: -2: shift count out of range\n",
    );
}
