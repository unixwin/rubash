//! rubash#2131 — a simple command's OWN redirections never reached the
//! fd-fact sync (`sync_fd_alias_facts`): the sync classified only
//! fd-table entries, and a builtin's own redirects are staged as
//! payloads instead of being bound, so `[ -f /dev/stdin ] < file`
//! answered from the AMBIENT stdin (false under a /dev/null harness)
//! while the compound form `{ [ -f /dev/stdin ]; } < file` — which
//! does bind an entry — answered correctly.
//!
//! GNU model: execute_cmd.c applies the command's redirections
//! (redir.c:767-955 do_redirection_internal) BEFORE the builtin runs,
//! and test.c:544 unary_test → lib/sh/eaccess.c:116 sh_stataccess
//! stats the descriptor as bound for THIS command. The answer follows
//! the command's own binding, never the binding it superseded.
//!
//! The fix states each redirected descriptor's fact from its winning
//! redirect inside the sync (the own-redirect overlay), states a
//! staged FUNCTION_STDIN mirror's source fact beside the mirror
//! (FD0_MIRROR_FACT — the compound text-channel rebind), and hands
//! `[[ ]]`'s redirection list to the sync (only its fds 1/2 are ever
//! bound). Expectations hand-measured against GNU bash 5.3.20 via
//! script files; harness stdin is /dev/null unless a test says so.

use std::path::PathBuf;
use std::process::Command;

fn run_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(std::process::Stdio::null())
        .current_dir(fixture_dir("plain"))
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// One fixture dir PER TEST: cargo runs tests in parallel threads,
/// and a shared fixture path lets one test's cleanup delete another
/// test's fixture mid-script.
fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2131-pins-{tag}"));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

/// Runs the script with `$F` bound to a written fixture file and `$O`
/// to a scratch output path inside the test's own fixture dir. The
/// shell's cwd is that dir (mirrors run_c's contract via a local
/// variant so parallel tests never share a cwd either).
fn run_with_fixture(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = fixture_dir(tag);
    let path = dir.join("issue2131-fixture.txt");
    std::fs::write(&path, b"hello world\n").expect("write fixture");
    let shell_path = path.to_string_lossy().replace('\\', "/");
    let out = dir.join("issue2131-out.txt");
    let shell_out = out.to_string_lossy().replace('\\', "/");
    let script = script.replace("$F", &shell_path).replace("$O", &shell_out);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", &script])
        .stdin(std::process::Stdio::null())
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let run = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&out).ok();
    run
}

fn verdicts(stdout: &str) -> Vec<(String, bool)> {
    stdout
        .lines()
        .map(|line| {
            let (name, verdict) = line.rsplit_once('=').expect("verdict line NAME=T/F");
            (name.to_string(), verdict == "T")
        })
        .collect()
}

fn assert_verdicts(stdout: &str, expected: &[(&str, bool)]) {
    let got = verdicts(stdout);
    assert_eq!(got.len(), expected.len(), "verdict count, got: {stdout}");
    for ((name, value), (want_name, want)) in got.iter().zip(expected) {
        assert_eq!(name, want_name, "verdict order, got: {stdout}");
        assert_eq!(value, want, "verdict {name}, got: {stdout}");
    }
}

#[test]
fn issue2131_simple_command_own_input_redirect_states_its_fact() {
    let (stdout, _stderr, code) = run_with_fixture(
        "input",
        "[ -f /dev/stdin ] < $F && echo f=T || echo f=F\n\
         [ -p /dev/stdin ] < $F && echo p=T || echo p=F\n\
         [ -c /dev/stdin ] < $F && echo c=T || echo c=F\n\
         [ -s /dev/stdin ] < $F && echo s=T || echo s=F\n\
         test -f /dev/stdin < $F && echo tf=T || echo tf=F\n",
    );
    assert_eq!(code, Some(0));
    // GNU: the command's fd 0 IS the fixture for its duration.
    assert_verdicts(
        &stdout,
        &[
            ("f", true),
            ("p", false),
            ("c", false),
            ("s", true),
            ("tf", true),
        ],
    );
}

#[test]
fn issue2131_simple_command_own_null_redirect_states_its_fact() {
    let (stdout, _stderr, code) = run_c(
        "[ -c /dev/stdin ] < /dev/null && echo c=T || echo c=F\n\
         [ -p /dev/stdin ] < /dev/null && echo p=T || echo p=F\n\
         [ -f /dev/stdin ] < /dev/null && echo f=T || echo f=F\n\
         [ -w /dev/stdin ] < /dev/null && echo w=T || echo w=F\n",
    );
    assert_eq!(code, Some(0));
    // GNU: /dev/null is a character device, writable, not a pipe/file.
    assert_verdicts(
        &stdout,
        &[("c", true), ("p", false), ("f", false), ("w", true)],
    );
}

#[test]
fn issue2131_pipeline_stage_own_redirect_beats_the_stage_pipe() {
    let (stdout, _stderr, code) = run_with_fixture(
        "stage",
        "printf x | [ -f /dev/stdin ] < $F && echo f=T || echo f=F\n\
         printf x | [ -p /dev/stdin ] && echo pstage=T || echo pstage=F\n",
    );
    assert_eq!(code, Some(0));
    // GNU execute_cmd.c:2620: do_piping first, then the element's own
    // do_redirections — the stage's own `< file` replaces the pipe,
    // while the unredirected stage keeps it.
    assert_verdicts(&stdout, &[("f", true), ("pstage", true)]);
}

#[test]
fn issue2131_compound_mirror_rebind_states_its_fact() {
    let (stdout, _stderr, code) = run_c(
        "{ [ -c /dev/stdin ]; } < /dev/null && echo c=T || echo c=F\n\
         { [ -p /dev/stdin ]; } < /dev/null && echo p=T || echo p=F\n\
         ( [ -c /dev/stdin ] ) < /dev/null && echo subc=T || echo subc=F\n",
    );
    assert_eq!(code, Some(0));
    // The filed #2132 reproducer shape: a compound's text-channel
    // rebind to /dev/null installs no fd-table entry; its fact is the
    // mirror's own statement.
    assert_verdicts(&stdout, &[("c", true), ("p", false), ("subc", true)]);
}

#[test]
fn issue2131_dup_and_close_own_redirects_state_their_facts() {
    let (stdout, _stderr, code) = run_with_fixture(
        "dup",
        "exec 3<$F\n\
         [ -f /dev/stdin ] <&3 && echo dupf=T || echo dupf=F\n\
         [ -p /dev/stdin ] <&3 && echo dupp=T || echo dupp=F\n\
         [ -e /dev/stdin ] <&- && echo close_e=T || echo close_e=F\n\
         [ -r /dev/stdin ] <&- && echo close_r=T || echo close_r=F\n",
    );
    assert_eq!(code, Some(0));
    // GNU: <&3 duplicates fd 3's binding (the fixture); <&- closes
    // fd 0 for the command, so every predicate fails.
    assert_verdicts(
        &stdout,
        &[
            ("dupf", true),
            ("dupp", false),
            ("close_e", false),
            ("close_r", false),
        ],
    );
}

#[test]
fn issue2131_numbered_own_redirects_state_their_facts() {
    let (stdout, _stderr, code) = run_with_fixture(
        "numbered",
        "[ -f /dev/fd/3 ] 3<$F && echo f3=T || echo f3=F\n\
         [ -c /dev/fd/4 ] 4</dev/null && echo c4=T || echo c4=F\n\
         [ -p /dev/fd/4 ] 4</dev/null && echo p4=T || echo p4=F\n",
    );
    assert_eq!(code, Some(0));
    assert_verdicts(&stdout, &[("f3", true), ("c4", true), ("p4", false)]);
}

#[test]
fn issue2131_simple_command_own_output_redirect_states_its_fact() {
    let (stdout, _stderr, code) = run_with_fixture(
        "output",
        "[ -f /dev/stdout ] > $O && echo f=T || echo f=F\n\
         [ -f /dev/stderr ] 2> $O && echo e=T || echo e=F\n\
         [ -f /dev/fd/4 ] 4> $O && echo f4=T || echo f4=F\n\
         [ -f /dev/stdout ] >> $O && echo app=T || echo app=F\n\
         [ -c /dev/stdout ] > /dev/null && echo nc=T || echo nc=F\n\
         [ -p /dev/stdout ] > /dev/null && echo np=T || echo np=F\n",
    );
    assert_eq!(code, Some(0));
    // GNU: the command's own output descriptors are the opened files
    // (regular, created empty) or the null device.
    assert_verdicts(
        &stdout,
        &[
            ("f", true),
            ("e", true),
            ("f4", true),
            ("app", true),
            ("nc", true),
            ("np", false),
        ],
    );
}

#[test]
fn issue2131_conditional_own_redirects_state_their_facts() {
    let (stdout, _stderr, code) = run_with_fixture(
        "cond",
        "[[ -f /dev/stdin ]] < $F && echo f=T || echo f=F\n\
         [[ -c /dev/stdin ]] < /dev/null && echo c=T || echo c=F\n\
         [[ -p /dev/stdin ]] < /dev/null && echo p=T || echo p=F\n\
         [[ -f /dev/stdout ]] > $O && echo o=T || echo o=F\n",
    );
    assert_eq!(code, Some(0));
    assert_verdicts(
        &stdout,
        &[("f", true), ("c", true), ("p", false), ("o", true)],
    );
}
