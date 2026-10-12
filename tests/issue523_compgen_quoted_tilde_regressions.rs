//! Issue #523: `compgen -d -- "~/"` — a quoted tilde-slash completion word
//! keeps its literal `~/` display prefix while the directory listing
//! resolves against $HOME.
//!
//! GNU spec (pcomplete.c tilde-expands the completion word for the lookup;
//! the returned names keep the user's spelling): WSL GNU Bash 5.3.0 lists
//! `~/.atuin`-style entries with rc 0 for `compgen -d -- "~/"`, the same
//! set the unquoted `compgen -d -- ~/` completes (through absolute paths,
//! since the shell expands that form before the builtin runs). rubash
//! returned empty: the completion channel treated the dequoted `~/` as a
//! literal directory name.
//!
//! Isolation (from the issue): same build, `compgen -d -- ~/` works,
//! `compgen -d -- "~/"` was empty -> the quoted tilde-slash prefix is the
//! exact divergence. huck cross-reference: huck#295 (tilde recognition
//! belongs to the expansion phase, not the lexer) — same root-cause shape.
//!
//! Rust semantic owner: builtins/complete.rs path_completion_candidates
//! (the `~/` word resolves its listing against $HOME and rewrites the
//! physical prefix back to `~`).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
}

fn run_with_home(home: &std::path::Path, command: &str) -> RunOutcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", command])
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env("HOME", home)
        .output()
        .expect("run rubash -c");
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    }
}

fn scratch_home(tag: &str) -> std::path::PathBuf {
    let home = std::env::temp_dir().join(format!("rubash-issue523-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".cache")).expect("create .cache");
    std::fs::create_dir_all(home.join(".config")).expect("create .config");
    std::fs::create_dir_all(home.join("sub")).expect("create sub");
    std::fs::write(home.join(".file"), "").expect("create .file");
    home
}

/// Corpus F136: the quoted tilde-slash lists $HOME directories under the
/// literal `~/` prefix, rc 0.
#[test]
fn issue523_quoted_tilde_slash_lists_home_directories() {
    let home = scratch_home("d");
    let out = run_with_home(&home, "compgen -d -- \"~/\"");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "~/.cache\n~/.config\n~/sub\n");
    let _ = std::fs::remove_dir_all(&home);
}

/// A tilde-slash word with a subdirectory segment resolves under $HOME and
/// keeps the `~/sub/` display prefix.
#[test]
fn issue523_quoted_tilde_subdirectory_prefix() {
    let home = scratch_home("s");
    std::fs::create_dir_all(home.join("sub/nested")).expect("create nested");
    let out = run_with_home(&home, "compgen -d -- \"~/sub/\"");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "~/sub/nested\n");
    let _ = std::fs::remove_dir_all(&home);
}

/// The file action shares the resolution: `-f "~/.f"` completes the hidden
/// file under the `~/` display prefix.
#[test]
fn issue523_quoted_tilde_file_action() {
    let home = scratch_home("f");
    let out = run_with_home(&home, "compgen -f -- \"~/.f\"");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "~/.file\n");
    let _ = std::fs::remove_dir_all(&home);
}

/// The unquoted spelling keeps its behavior: the shell expands `~/` before
/// the builtin runs, so the completions are physical paths.
#[test]
fn issue523_unquoted_tilde_stays_expanded() {
    let home = scratch_home("u");
    let out = run_with_home(&home, "compgen -d -- ~/ | sort");
    assert_eq!(out.code, Some(0));
    let home_str = home.to_string_lossy().replace('\\', "/");
    assert_eq!(
        out.stdout,
        format!("{home_str}/.cache\n{home_str}/.config\n{home_str}/sub\n")
    );
    let _ = std::fs::remove_dir_all(&home);
}
