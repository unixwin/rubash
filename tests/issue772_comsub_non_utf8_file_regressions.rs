//! Issue rubash#772 (item 1) — `$(cat FILE)` on a file whose content is not
//! valid UTF-8 yields an **empty** substitution plus a misleading diagnostic,
//! while the same `cat` outside a substitution prints the bytes correctly.
//!
//! GNU bash passes a child's stdout through as opaque bytes: for a one-byte
//! `0xFF` file, `x=$(cat f)` leaves `x` one byte long.  rubash returns an empty
//! `x` and reports
//!
//! ```text
//! cat: 'f': No such file or directory
//! ```
//!
//! for a file that plainly exists.  Two facts pin the trigger:
//!
//! * `cat f` run directly (no substitution) is correct, and so is
//!   `x=$(cat f 2>/dev/null)`;
//! * passing the file by redirection (`x=$(cat < f)`) or through a pipe
//!   (`printf '\xff' | cat -`) is also correct.
//!
//! That the failure disappears once stderr is captured inside the substitution
//! is the signature of a fast path declining when stderr is redirected
//! (`src/executor/pipeline_exec.rs:2018` has the same `stderr_capture.is_some()`
//! shape in `execute_external_prefix_concurrently`).
//!
//! The `#[ignore]`d tests are the reproductions and must be un-ignored with the
//! fix; the `control_*` tests already pass and exist so that the non-UTF-8
//! fixture, the scratch directory and the invocation form are themselves under
//! test — otherwise a green control is indistinguishable from a broken fixture.
//! The redirection form is one of those controls: it is **correct today**, and
//! that is precisely what localises the defect to the operand form.

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One scratch directory per test, holding a one-byte `0xFF` file `f` and an
/// ASCII control file `g`.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-772-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    fs::write(dir.join("f"), [0xffu8]).expect("write non-UTF-8 file");
    fs::write(dir.join("g"), b"abc").expect("write ASCII file");
    dir
}

fn run_in(dir: &Path, script: &str) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash -c");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// ---------------------------------------------------------------------------
// Controls — these pass today and protect the fixture and the invocation form
// ---------------------------------------------------------------------------

#[test]
fn control_ascii_file_substitutes_correctly() {
    let dir = scratch("ctrl-ascii");
    let (code, out, err) = run_in(&dir, r#"x=$(cat g); echo "${#x}""#);
    assert_eq!(code, Some(0), "stderr={err:?}");
    assert_eq!(out.trim(), "3", "ASCII content must survive: {out:?}");
}

#[test]
fn control_cat_outside_substitution_prints_the_byte() {
    let dir = scratch("ctrl-direct");
    // Deliberately routed through a redirection only: a `wc -c < out` here would
    // be a *second* command substitution, i.e. the very case under test, and the
    // control would silently become a reproduction.
    let (code, out, err) = run_in(&dir, r#"cat f > out; wc -c < out"#);
    assert_eq!(code, Some(0), "stderr={err:?}");
    assert_eq!(out.trim(), "1", "direct cat must keep the byte: {out:?}");
}

#[test]
fn control_redirection_form_keeps_the_byte() {
    // Passing the file by redirection is correct today, which is why it is a
    // control and not a reproduction: the defect is specific to the *operand*
    // form inside a substitution.
    let dir = scratch("ctrl-redir");
    let (code, out, err) = run_in(&dir, r#"x=$(cat < f); echo "${#x}""#);
    assert_eq!(code, Some(0), "stderr={err:?}");
    assert_eq!(out.trim(), "1", "stderr={err:?}");
}

// ---------------------------------------------------------------------------
// Reproductions
// ---------------------------------------------------------------------------

#[test]
#[ignore = "rubash#772: diverges on master (empty substitution + bogus 'No such file'); un-ignore with the fix"]
fn command_substitution_of_non_utf8_file_is_not_empty() {
    let dir = scratch("comsub");
    let (code, out, err) = run_in(&dir, r#"x=$(cat f); echo "${#x}""#);
    assert_eq!(code, Some(0), "stderr={err:?}");
    assert_eq!(
        out.trim(),
        "1",
        "GNU keeps the raw byte; rubash returns empty (stderr={err:?})"
    );
}

#[test]
#[ignore = "rubash#772: diverges on master — the invalid byte is dropped, so a two-operand cat silently loses content (3 of 4 bytes)"]
fn command_substitution_with_two_operands_keeps_every_byte() {
    let dir = scratch("two-operands");
    let (code, out, err) = run_in(&dir, r#"x=$(cat g f); echo "${#x}""#);
    assert_eq!(code, Some(0), "stderr={err:?}");
    assert_eq!(
        out.trim(),
        "4",
        "abc + 0xFF is four bytes; a short count means content was dropped (stderr={err:?})"
    );
}
