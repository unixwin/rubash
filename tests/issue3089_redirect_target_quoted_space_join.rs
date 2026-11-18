//! rubash#3089 — a redirect target built from a quoted space-bearing
//! segment joined with a substitution segment (`'a b c'$(printf '')`,
//! `'a b c'${V}`, `'a\$b c'$(...)`) is reported as
//! `ambiguous redirect`, while GNU creates the single file.
//!
//! GNU semantics: redir.c:298-345 `redirection_expand` runs the
//! target word through the ORDINARY word-expansion pipeline
//! (expand_words_no_vars → subst.c expand_word_list_internal). The
//! quoted segment's spaces carry CTLESC protection through field
//! splitting, so they can never split a field; the operand is legal
//! iff the expansion yields EXACTLY ONE field (zero or more than one
//! → NULL → AMBIGUOUS_REDIRECT at redir.c:823/865/939/1246, printed
//! by redirection_error at redir.c:201). The criterion is a FIELD
//! COUNT, not the presence of whitespace: `'a b'${V}` with `V=' d'`
//! IS ambiguous in GNU too, because that space comes from an
//! unquoted expansion and really splits (oracle probe q13).
//!
//! rubash root cause: the redirect operand is flattened to a String
//! early (src/executor/expand_word.rs:162 expand_redirect_target →
//! expand_word → expand_redirect_operand_fields, which performs
//! brace/glob handling but no real field split), and the ambiguity
//! verdict is reconstructed by a TEXTUAL gate —
//! redirect_target_is_ambiguous (src/executor/execution_misc.rs:399,
//! reached via redirect_operand_ambiguous, expand_word.rs:126) —
//! which scans the RAW word for an unquoted expansion introducer
//! and checks whether the expanded string contains any whitespace.
//! For `'a b c'$(printf '')` the raw word has an unquoted `$` and the
//! expansion contains the QUOTED space, so the gate misfires: it
//! cannot see which field each space belongs to — the quoted/IFS
//! marking is lost exactly at the join (the #2999 lane's REPORT §6
//! diagnosis, from which this issue was split).
//!
//! Oracle evidence: GNU bash 5.3.20, probes under
//! `~/workspace/fix-W2b/probes/gnu/` (q1-q14). Expected values are
//! the GNU results; the three genuinely-ambiguous shapes are pinned
//! as negative controls so a fix cannot pass by simply never
//! reporting ambiguity.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash` in a per-test
//! temp working directory; created filenames are asserted on the
//! filesystem, contents read back through the shell (chain form:
//! one step's state feeds the next, with the intermediate filename
//! asserted in between). `LC_ALL=C.UTF-8` is set on the child
//! explicitly so the pin is hermetic under any ambient locale.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Out {
    rc: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run(script: &str, cwd: &PathBuf) -> Out {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash")
        .wait_with_output()
        .expect("wait rubash");
    Out {
        rc: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3089-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expect_ok(out: &Out, ctx: &str) {
    assert_eq!(
        out.rc,
        0,
        "{ctx}: rc {} != 0; stderr: {}",
        out.rc,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr not empty: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn expect_ambiguous(out: &Out, dir: &PathBuf, ctx: &str) {
    assert_eq!(out.rc, 1, "{ctx}: rc {} != 1 (GNU exits 1)", out.rc);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("ambiguous redirect"),
        "{ctx}: stderr must report ambiguous redirect, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let created: Vec<String> = fs::read_dir(dir)
        .expect("read temp dir")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        created.is_empty(),
        "{ctx}: no file may be created, found {created:?}"
    );
}

/// Issue shape 1: quoted space segment + empty command substitution.
#[test]
fn target_quoted_space_plus_comsub() {
    let dir = temp_dir("comsub");
    let out = run(": > 'a b c'$(printf '')", &dir);
    expect_ok(&out, "'a b c'$(printf '') as redirect target");
    assert!(dir.join("a b c").is_file(), "file 'a b c' must be created");
    let _ = fs::remove_dir_all(&dir);
}

/// Issue shape 2: quoted space segment + empty parameter expansion.
#[test]
fn target_quoted_space_plus_empty_param() {
    let dir = temp_dir("param");
    let out = run("V=; : > 'a b c'${V}", &dir);
    expect_ok(&out, "'a b c'${V} (V empty) as redirect target");
    assert!(dir.join("a b c").is_file(), "file 'a b c' must be created");
    let _ = fs::remove_dir_all(&dir);
}

/// Issue shape 3: quoted backslash segment + comsub — the created
/// name keeps the literal backslash (`a\$b c`), exactly as GNU.
#[test]
fn target_quoted_backslash_plus_comsub() {
    let dir = temp_dir("backslash");
    let out = run(r#": > 'a\$b c'$(printf '')"#, &dir);
    expect_ok(&out, "'a\\$b c'$(printf '') as redirect target");
    assert!(
        dir.join("a\\$b c").is_file(),
        "file 'a\\$b c' must be created"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Join order reversed: substitution segment first, quoted space after.
#[test]
fn target_comsub_then_quoted_space() {
    let dir = temp_dir("reversed");
    let out = run(": > $(printf '')'a b c'", &dir);
    expect_ok(&out, "$(printf '')'a b c' as redirect target");
    assert!(dir.join("a b c").is_file(), "file 'a b c' must be created");
    let _ = fs::remove_dir_all(&dir);
}

/// Quoted space segment + a parameter whose value has NO space:
/// the join is one field (`a bd`), GNU creates it.
#[test]
fn target_quoted_space_plus_nonempty_param() {
    let dir = temp_dir("paramd");
    let out = run("V=d; : > 'a b'${V}", &dir);
    expect_ok(&out, "'a b'${V} (V=d) as redirect target");
    assert!(dir.join("a bd").is_file(), "file 'a bd' must be created");
    let _ = fs::remove_dir_all(&dir);
}

/// Chain form: write through the joined target, assert the
/// intermediate filename, then read the content back.
#[test]
fn target_write_then_read_chain() {
    let dir = temp_dir("chain");
    let out = run("printf 'DATA' > 'a b c'$(printf ''); cat 'a b c'", &dir);
    expect_ok(&out, "write via joined target then cat");
    assert_eq!(
        out.stdout, b"DATA",
        "content must round-trip through the joined name"
    );
    assert!(dir.join("a b c").is_file(), "file 'a b c' must exist");
    let _ = fs::remove_dir_all(&dir);
}

/// Chain form with append: two joined-target writes accumulate.
#[test]
fn target_append_chain() {
    let dir = temp_dir("append");
    let out = run(
        "printf 'A' > 'a b c'$(printf ''); printf 'B' >> 'a b c'$(printf ''); cat 'a b c'",
        &dir,
    );
    expect_ok(&out, "append via joined target");
    assert_eq!(
        out.stdout, b"AB",
        "both writes must land in the one joined-name file"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Input direction: `<` with a joined target reads the file whose
/// name carries the quoted space.
#[test]
fn target_input_redirect_join() {
    let dir = temp_dir("input");
    fs::write(dir.join("a b c"), b"SP").expect("pre-create 'a b c'");
    let out = run("cat < 'a b c'$(printf '')", &dir);
    expect_ok(&out, "cat < 'a b c'$(printf '')");
    assert_eq!(
        out.stdout, b"SP",
        "input redirect must open the joined name"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Numbered-fd form: `exec 3>` with a joined target, write through
/// the fd, read back by name.
#[test]
fn target_fd_redirect_join_chain() {
    let dir = temp_dir("fd3");
    let out = run(
        "exec 3>'a b c'$(printf ''); echo ok >&3; exec 3>&-; cat 'a b c'",
        &dir,
    );
    expect_ok(&out, "exec 3>'a b c'$(printf '') chain");
    assert_eq!(
        out.stdout, b"ok\n",
        "fd write must land in the joined-name file"
    );
    assert!(dir.join("a b c").is_file(), "file 'a b c' must exist");
    let _ = fs::remove_dir_all(&dir);
}

/// Control: a purely quoted target (no substitution segment) has
/// always worked.
#[test]
fn target_pure_quoted_control() {
    let dir = temp_dir("pure");
    let out = run(": > 'a b c'", &dir);
    expect_ok(&out, "pure quoted target");
    assert!(dir.join("a b c").is_file(), "file 'a b c' must be created");
    let _ = fs::remove_dir_all(&dir);
}

/// Negative control: an UNQUOTED variable carrying a space really
/// splits — GNU reports ambiguous redirect and creates nothing.
#[test]
fn negative_unquoted_var_splits() {
    let dir = temp_dir("negvar");
    let out = run("V='x y'; : > $V", &dir);
    expect_ambiguous(&out, &dir, "$V (V='x y') as redirect target");
    let _ = fs::remove_dir_all(&dir);
}

/// Negative control: a command substitution yielding two words is
/// ambiguous in GNU as well.
#[test]
fn negative_comsub_two_words() {
    let dir = temp_dir("negcomsub");
    let out = run(": > $(printf 'x y')", &dir);
    expect_ambiguous(&out, &dir, "$(printf 'x y') as redirect target");
    let _ = fs::remove_dir_all(&dir);
}

/// Negative control: quoted segment joined with a parameter whose
/// value STARTS with a space — that space is unquoted expansion
/// output and splits, so GNU reports ambiguous redirect. A fix
/// that simply stops reporting ambiguity fails here.
#[test]
fn negative_quoted_plus_splitting_param() {
    let dir = temp_dir("negsplit");
    let out = run("V=' d'; : > 'a b'${V}", &dir);
    expect_ambiguous(&out, &dir, "'a b'${V} (V=' d') as redirect target");
    let _ = fs::remove_dir_all(&dir);
}
