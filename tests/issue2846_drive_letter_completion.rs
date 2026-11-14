//! rubash#2846: a Windows drive-letter word must survive completion
//! word extraction whole.
//!
//! Default `COMP_WORDBREAKS` contains `:` (bashline.c:313), so the
//! engine's completion split cut `C:/tmp/.../pl` at the colon and
//! completed only the post-colon fragment — the drive prefix was
//! discarded and a drive path could never complete. GNU has no
//! drive letters, so there is no GNU oracle for the carve-out
//! itself; the defect is the engine's completion word extraction
//! (niubash #918 probe): a leading `X:` drive prefix at the start
//! of a word is not a word break. Colons anywhere else — after a
//! multi-letter head, mid-word — keep breaking exactly as GNU's
//! split does.

use std::path::PathBuf;

use rubash::api::{RunOptions, Session};

fn opts() -> RunOptions {
    RunOptions::default()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2846-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn session() -> Session {
    Session::builder().build().expect("session builds")
}

fn run(session: &mut Session, line: &str) {
    session
        .run(line, &opts())
        .unwrap_or_else(|err| panic!("run {line:?} failed: {err:?}"));
}

fn read(dir: &PathBuf, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).expect("probe output written by the hook")
}

/// The hook's view of the split: `$2` (the word being completed),
/// `COMP_CWORD` and `COMP_WORDS` for `mycmd C:/tmp/fixt/pl` must be
/// the whole drive word, not the post-colon fragment.
#[test]
fn hook_sees_the_whole_drive_word() {
    let dir = temp_dir("hook");
    let mut session = session();
    run(
        &mut session,
        &format!(
            "probe_fn() {{ \
               printf '%s' \"$2\" > '{d}/cur.txt'; \
               printf '%s' \"$COMP_CWORD\" > '{d}/cword.txt'; \
               printf '%s' \"${{COMP_WORDS[*]}}\" > '{d}/words.txt'; \
               COMPREPLY=(); \
             }}",
            d = dir.display()
        ),
    );
    run(&mut session, "complete -F probe_fn mycmd");
    let line = "mycmd C:/tmp/fixt/pl";
    let _ = session.complete_detailed(line, line.len());
    assert_eq!(read(&dir, "cur.txt"), "C:/tmp/fixt/pl");
    assert_eq!(read(&dir, "cword.txt"), "1");
    assert_eq!(read(&dir, "words.txt"), "mycmd C:/tmp/fixt/pl");
}

/// Wordlist filtering keys off the same current word: with the
/// drive prefix intact, the `C:/...` entries are the ones that
/// match the typed prefix.
#[test]
fn wordlist_candidates_keep_the_drive_prefix() {
    let mut session = session();
    run(
        &mut session,
        "complete -W 'C:/tmp/plain.txt C:/tmp/other.txt /tmp/plain.txt' mycmd",
    );
    let line = "mycmd C:/tmp/pl";
    let outcome = session.complete_detailed(line, line.len());
    let texts: Vec<&str> = outcome
        .candidates
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect();
    assert_eq!(texts, ["C:/tmp/plain.txt"]);
}

/// End to end through filename completion: a directory literally
/// named `C:` under the session cwd stands in for the drive root
/// (the engine resolves the word's directory part against the
/// session cwd on this platform), so the completed candidate must
/// carry the drive prefix.
#[test]
fn filename_candidates_keep_the_drive_prefix() {
    let dir = temp_dir("fs");
    let fixt = dir.join("C:").join("tmp").join("fixt");
    std::fs::create_dir_all(&fixt).expect("fixture tree");
    std::fs::write(fixt.join("plain.txt"), "x").expect("fixture file");
    let mut session = session();
    run(&mut session, &format!("cd {}", dir.display()));
    let line = "mycmd C:/tmp/fixt/pl";
    let outcome = session.complete_detailed(line, line.len());
    let texts: Vec<&str> = outcome
        .candidates
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect();
    assert_eq!(texts, ["C:/tmp/fixt/plain.txt"]);
}

/// The carve-out is only the drive prefix: a colon after a
/// multi-letter head still breaks, so the word being completed is
/// the post-colon fragment, exactly as GNU's split has it.
#[test]
fn colon_after_a_longer_head_still_breaks() {
    let mut session = session();
    run(
        &mut session,
        "complete -W '/tmp/plain.txt C:/tmp/plain.txt' mycmd",
    );
    let line = "mycmd ab:/tmp/pl";
    let outcome = session.complete_detailed(line, line.len());
    let texts: Vec<&str> = outcome
        .candidates
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect();
    assert_eq!(texts, ["/tmp/plain.txt"]);
}

/// At command position the drive word is the initial word: an `-I`
/// hook governs it and sees the whole word at index 0.
#[test]
fn drive_word_at_command_position_reaches_the_initial_hook() {
    let dir = temp_dir("cmdpos");
    let mut session = session();
    run(
        &mut session,
        &format!(
            "probe_fn() {{ \
               printf '%s' \"$2\" > '{d}/cur.txt'; \
               printf '%s' \"$COMP_CWORD\" > '{d}/cword.txt'; \
               printf '%s' \"${{COMP_WORDS[*]}}\" > '{d}/words.txt'; \
               COMPREPLY=(); \
             }}",
            d = dir.display()
        ),
    );
    run(&mut session, "complete -I -F probe_fn");
    let line = "C:/tmp/fixt/pl";
    let _ = session.complete_detailed(line, line.len());
    assert_eq!(read(&dir, "cur.txt"), "C:/tmp/fixt/pl");
    assert_eq!(read(&dir, "cword.txt"), "0");
    assert_eq!(read(&dir, "words.txt"), "C:/tmp/fixt/pl");
}
