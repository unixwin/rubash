//! Issue #832: `$()` fast-path "detour" believed to consult CWD contents —
//! a 0-byte file named `s` in the CWD flipped `echo [$(echo "'s'")]` from
//! `['s']` to `s`, polluting `cargo test` whenever the package root (the
//! test process's CWD) holds a matching single-letter file.
//!
//! Verdict (GNU C source is the specification, rubash#832 investigation):
//! there is NO fast-path admission defect on the main face. The
//! CWD-dependent judgment is pathname expansion of the unquoted
//! comsub-bearing word, which GNU itself performs and which therefore MUST
//! stay CWD-sensitive:
//! - subst.c:12626 in glob_expand_word_list — "If the word isn't an
//!   assignment and contains an unquoted pattern matching character, then
//!   glob it", decided by unquoted_glob_pattern_p (pathexp.c:66);
//! - subst.c:6643 comsub_shouldquote — for an UNQUOTED enclosing word the
//!   command-substitution output is inserted without CTLESC, so the `'`
//!   characters produced by `echo "'s'"` are data, never quoting
//!   operators: the word `[$(echo "'s'")]` expands to the bracket
//!   expression `['s']` over the class {`'`, `s`};
//! - with a matching file the word is REPLACED by the matched filename
//!   (`s`), without one the literal survives quote removal (`['s']`).
//! WSL GNU Bash 5.3.0 oracle (script file, 2026-10-19): byte-identical to
//! rubash on the whole class in every CWD state — see the pins below.
//! The cargo-test pollution was the TEST: the probe wrapped comsub output
//! in an unquoted `[...]` display word, which is exactly the glob face.
//! tests/cli_tests/compat_issue_regressions.rs now quotes its wrappers;
//! this file pins the CWD-sensitive face itself so no future change can
//! "fix" it into GNU divergence.
//!
//! Family sweep (same admission function, glob.rs pathname_expand_word):
//! the legacy text sniffs `starts_with('"')/starts_with('\'')` and
//! `contains('{')/contains('}')` made rubash skip globbing for words whose
//! leading quote / brace characters came from expansion OUTPUT — GNU globs
//! such words (`pathexp.c:66` lists no quote or brace suppressor; only the
//! CTLESC carrier quotes). Oracle-pinned here with matching files present;
//! before the fix rubash printed the literal patterns (`'*`, `{a*`,
//! `[{]*`) where GNU printed the matches.
//!
//! Cross-checks requested by the ticket:
//! - #805/#822 command -v face: `command -v s` / `type -t s` with a normal
//!   PATH never resolve a CWD file (findcmd.c:623 walks PATH only); the
//!   empty/unset-PATH `.` face (findcmd.c:266 path_value) is the one
//!   GNU-documented CWD consultation and stays.
//! - #806 read face: the read builtin takes no directory input on its
//!   admission path — nothing to change.
//!
//! Rust semantic owner: src/executor/glob.rs pathname_expand_word
//! (admission), glob_vector_expand / shell_directory_entries (the CWD
//! consultation itself), command_words.rs:455/480 and
//! command_substitution_values.rs:64 (the callers that glob the unquoted
//! comsub-bearing word).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Outcome {
    code: Option<i32>,
    stdout: String,
}

/// Run rubash -c with the CHILD's CWD pinned to `dir` (files pre-created by
/// the caller). The shell inherits the fixture CWD directly, so no `cd` is
/// needed in the payload and no test-global CWD mutation happens.
fn run_in_dir(dir: &PathBuf, payload: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(payload)
        .current_dir(dir)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash in fixture dir");
    Outcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    }
}

fn fixture_dir(tag: &str, files: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue832-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create issue832 fixture dir");
    for name in files {
        fs::File::create(dir.join(name)).expect("create fixture file");
    }
    dir
}

fn drop_dir(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn acceptance_bare_cmdsub_is_cwd_independent() {
    // The ticket's acceptance criterion: `$(echo 's')` behaves identically
    // with and without a 0-byte `s` in the CWD. The bare comsub word carries
    // no glob characters, so pathname expansion never consults the
    // directory — GNU oracle: `s` in both states.
    let empty = fixture_dir("bare-empty", &[]);
    let with_s = fixture_dir("bare-s", &["s"]);

    let payload = "x=$(echo 's'); printf '<%s>\n' \"$x\"";
    let without = run_in_dir(&empty, payload);
    let with = run_in_dir(&with_s, payload);

    assert_eq!(without.code, Some(0));
    assert_eq!(with.code, Some(0));
    assert_eq!(without.stdout, "<s>\n");
    assert_eq!(with.stdout, "<s>\n");

    drop_dir(&empty);
    drop_dir(&with_s);
}

#[test]
fn quoted_wrapper_faces_are_cwd_independent() {
    // The hermetic cli-test face: double-quoted display wrappers suppress
    // pathname expansion, so the nested quote literals survive byte-exact
    // regardless of what the CWD holds (GNU oracle: identical output with
    // and without `s`/`'`/`q` files).
    let empty = fixture_dir("quoted-empty", &[]);
    let with_s = fixture_dir("quoted-s", &["s"]);

    let payload = concat!(
        "echo \"[$(echo \"'s'\")]\";",
        "v=$(echo \"'s'\"); echo \"[$v]\";",
        "echo \"[$(echo s)]\"",
    );
    let without = run_in_dir(&empty, payload);
    let with = run_in_dir(&with_s, payload);

    assert_eq!(without.code, Some(0));
    assert_eq!(with.code, Some(0));
    assert_eq!(without.stdout, "['s']\n['s']\n[s]\n");
    assert_eq!(with.stdout, without.stdout);

    drop_dir(&empty);
    drop_dir(&with_s);
}

#[test]
fn unquoted_bracket_class_matches_like_gnu() {
    // The engine verdict pins (GNU 5.3.0 oracle, script file): the unquoted
    // word `[$(echo "'s'")]` is the bracket expression ['s'] over the class
    // {`'`, `s`} — the matched FILENAME replaces the word, no match keeps
    // the literal. Empty CWD -> `['s']`, 0-byte `s` -> `s`, apostrophe file
    // -> `'` (the class contains the quote character too).
    let empty = fixture_dir("class-empty", &[]);
    let with_s = fixture_dir("class-s", &["s"]);
    let with_quote = fixture_dir("class-quote", &["'"]);

    let payload = "echo [$(echo \"'s'\")]";
    assert_eq!(run_in_dir(&empty, payload).stdout, "['s']\n");
    assert_eq!(run_in_dir(&with_s, payload).stdout, "s\n");
    assert_eq!(run_in_dir(&with_quote, payload).stdout, "'\n");

    drop_dir(&empty);
    drop_dir(&with_s);
    drop_dir(&with_quote);
}

#[test]
fn expansion_output_quote_and_brace_chars_do_not_block_globbing() {
    // The family fix (glob.rs admission sniffs removed): quote/brace
    // characters arriving from expansion output are DATA (subst.c:6643
    // comsub_shouldquote CTLESC-quotes only IFS-special bytes; pathexp.c:66
    // lists no quote/brace suppressor), so the word still globs.
    // GNU oracle with the same fixture files: `y` / `{ab` / `{ab {zz`;
    // the empty-CWD controls keep the unmatched literals.
    let empty = fixture_dir("fam-empty", &[]);
    let with_files = fixture_dir("fam-files", &["'y", "{ab", "{zz"]);

    let apostrophe_star = "echo $(echo \"'\")*";
    assert_eq!(run_in_dir(&empty, apostrophe_star).stdout, "'*\n");
    assert_eq!(run_in_dir(&with_files, apostrophe_star).stdout, "'y\n");

    let literal_brace = "echo {a*";
    assert_eq!(run_in_dir(&empty, literal_brace).stdout, "{a*\n");
    assert_eq!(run_in_dir(&with_files, literal_brace).stdout, "{ab\n");

    let comsub_brace = "echo [$(echo '{')]*";
    assert_eq!(run_in_dir(&empty, comsub_brace).stdout, "[{]*\n");
    assert_eq!(run_in_dir(&with_files, comsub_brace).stdout, "{ab {zz\n");

    drop_dir(&empty);
    drop_dir(&with_files);
}

#[test]
fn command_v_and_type_do_not_resolve_cwd_files_off_path() {
    // #805/#822 cross-check: command resolution walks PATH only
    // (findcmd.c:623 find_user_command_in_path) — a 0-byte `s` in the CWD
    // must stay invisible to `command -v s` / `type -t s` while a normal
    // PATH is set. The only CWD consultation GNU performs is the
    // empty/unset-PATH `.` normalization (findcmd.c:266 path_value), which
    // is not exercised here.
    let with_s = fixture_dir("cmdv-s", &["s"]);
    let payload = "command -v s; echo rc=$?; type -t s; echo rc=$?";
    let out = run_in_dir(&with_s, payload);
    assert_eq!(out.code, Some(0));
    // `command -v s` prints nothing (rc 1) and `type -t s` prints nothing
    // (rc 1) — the only output is the two rc echoes.
    assert_eq!(out.stdout, "rc=1\nrc=1\n");
    drop_dir(&with_s);
}
