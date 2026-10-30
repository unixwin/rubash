//! rubash#1867: a numbered-fd here-string/heredoc bound to a compound
//! command or a function was staged wrong or never staged at all:
//!
//! - The compound staging helper fed the parser's STORAGE form
//!   (STORAGE_WORD_PREFIX + token value) to the heredoc expansion, so
//!   `read -u 3` received the raw prefix byte (0x1d) glued to the first
//!   line — and because a here-string's trailing newline was never
//!   appended, the final line failed `read`'s delimiter test and a
//!   `while read` loop silently dropped it. A literal `3<<<'a\b'`
//!   delivered nothing at all.
//! - The canonical ladder (the one the fd-0 and /dev/fd consumers
//!   already used) is: typed carrier expansion, else storage-word
//!   ANSI-C decode / word expansion, and a here-string always gains
//!   its trailing newline (GNU redir.c here_document_to_fd). It now
//!   lives in one place (expand_heredoc_redirect_text /
//!   expand_stored_here_string_word) and every staging site consumes
//!   it — including the preexpand carrier, which wraps the storage
//!   form verbatim into a NeedsExpansion carrier
//!   (expand_here_string_mut_from_carrier strips it there).
//! - if/case/for/select executors and the function-call wrap carried
//!   NO numbered-heredoc staging (and the call wrap never opened
//!   numbered input redirects): `read: 3: invalid file descriptor`.
//!   The function-definition redirect store also dropped the
//!   heredoc_redirects bodies, so definition-side binds had nothing
//!   to stage.
//! - A stored word's lexer-decoded `$'...'` blanks carry the
//!   ANSI_C_IFS_GUARD field-split transport marker; the delivered
//!   byte stream must strip it (E400-aware) like every other value
//!   boundary.
//!
//! Oracle: GNU bash 5.3.20, script files, byte-compared on every
//! shape below.

#![cfg(unix)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// Run `case.sh` (with `data3.txt` beside it) under rubash in its own
/// process group with a watchdog deadline. Returns
/// (stdout, stderr, success).
fn run_script_watchdog(script: &str, timeout: Duration) -> (String, String, bool) {
    use std::os::unix::process::CommandExt;
    let dir = scratch_tempdir();
    std::fs::write(dir.0.join("data3.txt"), "one\ntwo\nthree\n").expect("write data");
    let script_path = dir.0.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let mut child = Command::new(rubash_bin())
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir.0)
        .current_dir(&dir.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .expect("spawn rubash");
    let pgid = child.id();
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status),
            None => {
                if Instant::now() >= deadline {
                    let _ = Command::new("/bin/kill")
                        .arg("-KILL")
                        .arg(format!("-{pgid}"))
                        .status();
                    let _ = child.kill();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut stderr);
    }
    let _ = child.wait();
    (
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
        status.is_some_and(|s| s.success()),
    )
}

fn assert_script(script: &str, expected_stdout: &str) {
    let (stdout, stderr, success) = run_script_watchdog(script, Duration::from_secs(20));
    assert!(success, "run must complete, stderr: {stderr}");
    assert_eq!(stdout, expected_stdout, "script: {script}");
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
}

#[test]
fn issue1867_while_loop_herestring_reads_all_lines() {
    // Issue seed: no 0x1d prefix on the first line, and the last line
    // survives the `while read` condition (trailing newline present).
    assert_script(
        "while read -r -u 3 v; do printf 'while: [%s]\\n' \"$v\"; done 3<<<\"$(<data3.txt)\"\n",
        "while: [one]\nwhile: [two]\nwhile: [three]\n",
    );
}

#[test]
fn issue1867_group_and_subshell_herestring() {
    assert_script(
        "{ read -r -u 3 a; read -r -u 3 b; printf 'group: [%s][%s]\\n' \"$a\" \"$b\"; } 3<<<\"$(<data3.txt)\"\n( read -r -u 3 c; printf 'sub: [%s]\\n' \"$c\" ) 3<<<\"$(<data3.txt)\"\n",
        "group: [one][two]\nsub: [one]\n",
    );
}

#[test]
fn issue1867_if_for_case_herestring() {
    // These executors staged nothing at all: Bad file descriptor.
    assert_script(
        "if true; then read -r -u 3 a; printf 'if: [%s]\\n' \"$a\"; fi 3<<<\"$(<data3.txt)\"\nfor i in 1; do read -r -u 3 b; printf 'for: [%s]\\n' \"$b\"; done 3<<<\"$(<data3.txt)\"\ncase x in x) read -r -u 3 c; printf 'case: [%s]\\n' \"$c\";; esac 3<<<\"$(<data3.txt)\"\n",
        "if: [one]\nfor: [one]\ncase: [one]\n",
    );
}

#[test]
fn issue1867_function_call_herestring_and_file() {
    assert_script(
        "f() { read -r -u 3 a; read -r -u 3 b; printf 'func: [%s][%s]\\n' \"$a\" \"$b\"; }\nf 3<<<\"$(<data3.txt)\"\ng() { read -r -u 3 a; printf 'funcfile: [%s]\\n' \"$a\"; }\ng 3<data3.txt\n",
        "func: [one][two]\nfuncfile: [one]\n",
    );
}

#[test]
fn issue1867_function_definition_herestring() {
    assert_script(
        "f() { read -r -u 3 a; read -r -u 3 b; printf 'def: [%s][%s]\\n' \"$a\" \"$b\"; } 3<<<\"$(<data3.txt)\"\nf\n",
        "def: [one][two]\n",
    );
}

#[test]
fn issue1867_dynamic_fd_herestring() {
    assert_script(
        "exec {v}<<<\"$(<data3.txt)\"; read -r -u $v a; read -r -u $v b; printf 'dynfd: [%s][%s]\\n' \"$a\" \"$b\"\n",
        "dynfd: [one][two]\n",
    );
}

#[test]
fn issue1867_literal_forms_in_compounds() {
    // Literal single-quoted word (was: empty) and the ANSI-C form
    // whose decoded blank must arrive guard-free.
    assert_script(
        "{ read -r -u 3 a; read -r -u 3 b; printf 'glit: [%s][%s]\\n' \"$a\" \"$b\"; } 3<<<'a\\b'\nif true; then read -r -u 3 c; printf 'iflit: [%s]\\n' \"$c\"; fi 3<<<$'x\\ty'\n",
        "glit: [a\\b][]\niflit: [x\ty]\n",
    );
}

#[test]
fn issue1867_numbered_heredoc_compounds() {
    // Control family: plain heredocs on numbered fds of compounds
    // and function definitions keep working through the same ladder.
    assert_script(
        "while read -r -u 4 v; do printf 'hd: [%s]\\n' \"$v\"; done 4<<EOF\nl1\nl2\nEOF\nf() { read -r -u 4 a; printf 'defhd: [%s]\\n' \"$a\"; } 4<<HEOF\nfromdef\nHEOF\nf\n",
        "hd: [l1]\nhd: [l2]\ndefhd: [fromdef]\n",
    );
}

#[test]
fn issue1867_external_consumer_reads_staged_fd() {
    // The issue's control shape: an external draining the staged fd
    // sees the same clean bytes.
    assert_script("{ cat <&3; } 3<<<\"$(<data3.txt)\"\n", "one\ntwo\nthree\n");
}
