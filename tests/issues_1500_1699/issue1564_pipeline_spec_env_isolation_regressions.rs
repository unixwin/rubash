//! rubash#1564 — speculative pipeline-spec expansion must not leak
//! variable bindings into the parent shell.
//!
//! GNU runs every pipeline element through execute_simple_command in
//! its own forked child (execute_cmd.c:2620 execute_pipeline): the
//! element's word expansion — including side-effecting forms like
//! `${var:=word}` and `$((var=expr))` — happens in the child and dies
//! with it. rubash's concurrent pipeline attempt expands each
//! element's words speculatively on the PARENT executor to build the
//! spawn spec (and expands tempenv assignment values again at spawn
//! time via apply_external_environment): a successful bind used to
//! stay in the parent table after the pipeline, visible to the rest
//! of the script and to sibling elements' own spec expansions. The
//! spec/spawn expansions are now bracketed by a variable-table
//! snapshot/restore; when a speculative bind touches an EXPORTED
//! name, the attempt declines to the sequential stage executor,
//! whose per-element child is the only path that can hand the bound
//! value to that element's own child process the way GNU does.
//!
//! Expectations measured against GNU bash 5.3.20 with script files
//! in this environment (byte-level stdout, statuses; each form run
//! 3x with identical results).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-1564-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("script");
    let path = dir.join("case.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("./case.sh")
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn issue_1564_bare_word_bind_stays_in_the_element() {
    // First, middle and last element positions.
    for script in [
        "${w:=5} | cat | cat\necho \"w=[${w:-unset}]\"\n",
        "true | ${w:=5} | cat\necho \"w=[${w:-unset}]\"\n",
        "true | ${w:=5}\necho \"w=[${w:-unset}]\"\n",
        "${w:=5} | cat | cat | cat\necho \"w=[${w:-unset}]\"\n",
    ] {
        let (stdout, stderr, code) = run_script(script);
        assert_eq!(stdout, "w=[unset]\n", "script: {script:?} stderr: {stderr}");
        assert_eq!(code, Some(0));
        assert_eq!(
            stderr.matches("5: command not found").count(),
            1,
            "script: {script:?} stderr: {stderr}"
        );
    }
}

#[test]
fn issue_1564_arithmetic_bind_stays_in_the_element() {
    let (stdout, stderr, code) = run_script("$((w=5)) | cat\necho \"w=[${w:-unset}]\"\n");
    assert_eq!(stdout, "w=[unset]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("5: command not found").count(), 1);
}

#[test]
fn issue_1564_two_elements_do_not_share_binds() {
    let (stdout, stderr, code) =
        run_script("${a:=1} | ${b:=2}\necho \"a=[${a:-unset}] b=[${b:-unset}]\"\n");
    assert_eq!(stdout, "a=[unset] b=[unset]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("command not found").count(), 2);
    // A sibling's spec expansion must not SEE the earlier element's
    // bind either: `cat $w` gets no file operand and reads the pipe.
    let (stdout, stderr, code) = run_script("${w:=5} | cat $w\necho \"w=[${w:-unset}]\"\n");
    assert_eq!(stdout, "w=[unset]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("command not found").count(), 1);
    assert!(!stderr.contains("cat: 5"), "stderr: {stderr}");
}

#[test]
fn issue_1564_bind_in_admitted_element_argument_is_isolated() {
    // The element is admitted (cat resolves), its own argument bind
    // produces the file operand for that element only.
    let (stdout, stderr, code) = run_script("cat ${w:=5} | wc -c\necho \"w=[${w:-unset}]\"\n");
    assert_eq!(stdout, "0\nw=[unset]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("cat: 5").count(), 1, "stderr: {stderr}");
}

#[test]
fn issue_1564_bind_resolving_to_a_real_command_still_runs_it() {
    // The spec is admitted and env really executes — with the bind
    // visible inside the element's own expansion but not after.
    let (stdout, stderr, code) =
        run_script("${w:=env} | grep -c '^PATH='\necho \"w=[${w:-unset}]\"\n");
    assert_eq!(stdout, "1\nw=[unset]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1564_exported_bind_is_visible_to_the_element_child_only() {
    // `w` is exported and unset: the element's own child process sees
    // the value its expansion bound (GNU prints w=env), while the
    // parent shell variable stays unset.
    let (stdout, stderr, code) =
        run_script("export w\n${w:=env} | grep '^w='\necho \"rc=$? w=[${w:-unset}]\"\n");
    assert_eq!(stdout, "w=env\nrc=0 w=[unset]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1564_tempenv_value_bind_is_isolated() {
    // The bind hides in a tempenv assignment VALUE, expanded at spawn
    // time on the concurrent path.
    let (stdout, stderr, code) =
        run_script("x=${w:=5} cat | wc -c\necho \"w=[${w:-unset}] x=[${x:-unset}]\"\n");
    assert_eq!(stdout, "0\nw=[unset] x=[unset]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1564_array_element_bind_is_isolated() {
    let (stdout, stderr, code) = run_script("${a[1]:=5} | cat\necho \"a1=[${a[1]:-unset}]\"\n");
    assert_eq!(stdout, "a1=[unset]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("5: command not found").count(), 1);
}

#[test]
fn issue_1564_heredoc_body_bind_stays_in_the_element() {
    // The element's heredoc body is expanded to materialize its stdin;
    // in GNU that expansion happens inside the forked element, so a
    // bind in the body never reaches the parent (oracle: stdout "2"
    // for the expanded body, w unset afterwards).
    let (stdout, stderr, code) =
        run_script("cat <<EOF | wc -c\n${w:=5}\nEOF\necho \"w=[${w:-unset}]\"\n");
    assert_eq!(stdout, "2\nw=[unset]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1564_plain_command_bind_still_applies() {
    // Control: outside a pipeline the same expansion binds normally.
    let (stdout, stderr, code) = run_script("${w:=5}\necho \"w=[${w:-unset}]\"\n");
    assert_eq!(stdout, "w=[5]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("5: command not found").count(), 1);
    // And a builtin-element pipeline already isolated its bind.
    let (stdout, stderr, code) = run_script(": ${w:=5} | cat\necho \"w=[${w:-unset}]\"\n");
    assert_eq!(stdout, "w=[unset]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}
