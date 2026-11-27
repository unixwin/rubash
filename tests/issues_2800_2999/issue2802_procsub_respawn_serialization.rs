//! rubash#2802: a process substitution in an async or coproc-compound
//! command reached the respawned child as a single-quoted LITERAL
//! (`cat '<(echo via)'`: No such file or directory) instead of a live
//! substitution. GNU executes the substitution in every position
//! (subst.c:6362 process_substitute runs at expansion time, in
//! whatever shell runs the command — the async/coproc respawn re-parses
//! the command's source text, so the text must carry the span
//! verbatim, the way print_cmd.c reprints a WORD's text).
//!
//! Root cause (parser): the word-intake arms for a word-initial
//! `<(...)` (token_actions.rs, RedirectIn arm) and `>(...)`
//! (redirect_assign.rs) pushed the word into `cmd.words` WITHOUT the
//! paired `cmd.word_metadata` entry every other intake site maintains.
//! Index-based consumers then misread the metadata vector: the
//! executor's respawn serializer (command_text.rs
//! command_word_source_text) found no metadata whose value matched
//! the procsub word (and none at all for every word after it), and
//! fell back to single-quoting the word as literal data. The same
//! arms also dropped the glued suffix of a word-initial span
//! (`<(echo hi)x` became TWO words), which GNU's one-pass word
//! assembly (parse.y:5305 read_token_word) never splits.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-11-08): every form below prints `via` (or the glued word as
//! ONE word) with status 0. On the pre-fix tree the six respawn cells
//! and the two glued async/foreground cells fail exactly as the issue
//! reports; the control cells pass on both trees.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Bounded script runner: spawn rubash on a script FILE in a scratch
/// dir, poll for exit (kill past the deadline), then drain the pipes.
/// Returns (stdout, stderr, exit code).
fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2802-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&case)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(25);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within 25s for case {tag}");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, status.code())
}

fn run_c(script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(25);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within 25s for -c case");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.code())
}

#[test]
fn issue2802_async_simple_procsub_word_expands() {
    let (stdout, stderr, code) = run_script(
        "async-simple",
        "cat <(echo via) & wait $!; echo \"rc:$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "via\nrc:0\n", "stderr:\n{stderr}");
}

#[test]
fn issue2802_async_braces_procsub_word_expands() {
    let (stdout, stderr, code) = run_script(
        "async-braces",
        "{ cat <(echo via); } & wait $!; echo \"rc:$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "via\nrc:0\n", "stderr:\n{stderr}");
}

#[test]
fn issue2802_async_subshell_procsub_word_expands() {
    let (stdout, stderr, code) = run_script(
        "async-subshell",
        "( cat <(echo via) ) & wait $!; echo \"rc:$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "via\nrc:0\n", "stderr:\n{stderr}");
}

#[test]
fn issue2802_coproc_braces_procsub_word_expands() {
    let (stdout, stderr, code) = run_script(
        "coproc-braces",
        "coproc { cat <(echo via); }\n\
         read -r r <&${COPROC[0]}\n\
         echo \"v:$r\"\n\
         wait $COPROC_PID; echo \"w:$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "v:via\nw:0\n", "stderr:\n{stderr}");
}

#[test]
fn issue2802_coproc_two_procsub_words_expand() {
    let (stdout, stderr, code) = run_script(
        "coproc-two",
        "coproc { cat <(echo one) <(echo two); }\n\
         read -r r1 <&${COPROC[0]}\n\
         read -r r2 <&${COPROC[0]}\n\
         echo \"v:$r1,$r2\"\n\
         wait $COPROC_PID; echo \"w:$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "v:one,two\nw:0\n", "stderr:\n{stderr}");
}

#[test]
fn issue2802_coproc_subshell_procsub_word_expands() {
    let (stdout, stderr, code) = run_script(
        "coproc-subshell",
        "coproc ( cat <(echo via) )\n\
         read -r r <&${COPROC[0]}\n\
         echo \"v:$r\"\n\
         wait $COPROC_PID; echo \"w:$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "v:via\nw:0\n", "stderr:\n{stderr}");
}

#[test]
fn issue2802_control_coproc_simple_procsub_word_expands() {
    let (stdout, stderr, code) = run_script(
        "coproc-simple",
        "coproc cat <(echo via)\n\
         read -r r <&${COPROC[0]}\n\
         echo \"v:$r\"\n\
         wait $COPROC_PID; echo \"w:$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "v:via\nw:0\n", "stderr:\n{stderr}");
}

#[test]
fn issue2802_control_foreground_and_pipeline() {
    let (stdout, stderr, code) = run_script("fg-pipe", "cat <(echo via)\ncat <(echo via) | cat\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "via\nvia\n", "stderr:\n{stderr}");
}

/// The word after a procsub word must survive serialization too: the
/// metadata misalignment quoted EVERY following word, so
/// `echo a <(echo via) b &` reached the child as
/// `echo a '<(echo via)' 'b'`.
#[test]
fn issue2802_words_around_procsub_word_stay_live() {
    let (stdout, stderr, code) = run_script(
        "around",
        "{ printf '[%s]' a <(echo via) b; echo; } & wait $!\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    // printf prints each word itself: the middle field must be the
    // child's live fd path for the substitution — never the literal
    // `<(echo via)` text the broken serializer shipped.
    assert!(stdout.starts_with("[a]["), "stdout: {stdout:?}");
    assert!(stdout.ends_with("][b]\n"), "stdout: {stdout:?}");
    assert!(!stdout.contains("<("), "stdout: {stdout:?}");
    #[cfg(unix)]
    assert!(stdout.contains("/dev/fd/"), "stdout: {stdout:?}");
}

/// A word-initial span glued to a suffix is ONE word in GNU
/// (parse.y:5305 read_token_word assembles it in one pass): the
/// expanded word is the fd path with the suffix attached, in the
/// foreground and through the async respawn alike.
#[test]
fn issue2802_glued_suffix_is_one_word() {
    for (tag, script) in [
        ("glued-fg", "echo <(echo hi)x\n"),
        ("glued-async", "echo <(echo hi)x & wait $!\n"),
        ("glued-out", "echo >(cat)x\n"),
        ("glued-out-async", "echo >(cat)x & wait $!\n"),
    ] {
        let (stdout, stderr, code) = run_script(tag, script);
        assert_eq!(code, Some(0), "{tag} stderr:\n{stderr}");
        let word = stdout.trim_end();
        assert!(
            !word.contains(char::is_whitespace),
            "{tag}: glued word split into fields: {word:?}"
        );
        assert!(word.ends_with('x'), "{tag}: suffix lost: {word:?}");
        assert!(word.len() > 2, "{tag}: word too short: {word:?}");
        #[cfg(unix)]
        assert!(
            word.starts_with("/dev/fd/"),
            "{tag}: not an fd path: {word:?}"
        );
    }
}

/// The same serializer feeds BASH_COMMAND: the DEBUG trap must see
/// the command with the substitution spelled verbatim, never quoted
/// (GNU print_cmd.c prints the word's text).
#[test]
fn issue2802_bash_command_keeps_procsub_verbatim() {
    let (stdout, stderr, _code) = run_c(
        "trap 'printf \"BC:%s\\n\" \"$BASH_COMMAND\"' DEBUG\n\
         cat <(echo via) >/dev/null\n",
    );
    let line = stdout
        .lines()
        .find(|line| line.contains("cat "))
        .unwrap_or_else(|| panic!("no BASH_COMMAND line for the cat command: {stdout:?}"));
    assert_eq!(line, "BC:cat <(echo via) > /dev/null", "stderr:\n{stderr}");
}
