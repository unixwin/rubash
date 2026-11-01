//! rubash#2429 — a heredoc as the LAST command of a `case` clause is
//! destroyed by the respawn text renderer: the clause terminator was
//! emitted onto the heredoc delimiter line (`HD ;;`), leaving the
//! delimiter unterminated (here-doc EOF warning) and the whole `case`
//! unparseable (syntax error) in the respawned child, so the body was
//! silently lost. Residual of #2388 (whose renderer taught the
//! sequence/pipeline/function arms the delimiter-line rule but missed
//! the case-clause arm).
//!
//! GNU contract (print_cmd.c, bash 5.3): `print_case_clauses`
//! (print_cmd.c:760) prints the clause action, flushes the deferred
//! heredoc bodies (`PRINT_DEFERRED_HEREDOCS ("")`, print_cmd.c:787 —
//! the body + delimiter line, print_heredoc_bodies print_cmd.c:1032),
//! and only then prints the terminator via `newline(";;")` /
//! `newline(";&")` / `newline(";;&")` (print_cmd.c:788-793): the
//! terminator always starts on its own line, because a heredoc
//! delimiter line admits nothing after it. The renderer now takes a
//! newline before the terminator whenever the rendered clause body
//! ends with a heredoc body (the same render flag the sequence and
//! brace-group closers already consult), for all three terminators.
//!
//! Expectations measured against GNU bash 5.3.20 with script files
//! (2026-11-01). Unix-only, like the #2388 pins.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Case {
    dir: PathBuf,
}

fn make_case(tag: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("rubash2429-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    Case { dir }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Run the fixture as a script file and return (stdout, stderr). The
/// script must exit 0 and stderr must carry neither the here-doc EOF
/// warning nor the respawned child's `case` syntax error.
fn run_script(case: &Case, script: &str) -> (String, String) {
    let path = case.dir.join("case.sh");
    std::fs::write(&path, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&case.dir)
        .arg("case.sh")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "script failed: {stderr}");
    assert!(
        !stderr.contains("here-document") && !stderr.contains("syntax error"),
        "respawn diagnostics leaked: {stderr}"
    );
    (String::from_utf8_lossy(&output.stdout).into_owned(), stderr)
}

/// The #2429 seed: heredoc as the last command of a clause, `;;`
/// terminator, whole function respawned via `f a &`.
#[test]
fn issue2429_case_clause_heredoc_last_command() {
    let case = make_case("seed");
    let script = r#"f() {
  case $1 in
    a) cat <<HD
hello-body
HD
    ;;
  esac
}
f a &
wait
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "hello-body\ndone rc=0\n");
}

/// Control: a heredoc MID-clause, followed by another command — the
/// terminator follows a normal command line here and already worked;
/// it must keep working.
#[test]
fn issue2429_case_clause_heredoc_mid_clause_control() {
    let case = make_case("mid");
    let script = r#"f() {
  case $1 in
    a) cat <<HD
mid-body
HD
       echo after-cmd
    ;;
  esac
}
f a &
wait
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "mid-body\nafter-cmd\ndone rc=0\n");
}

/// Two clauses, each ending with its own heredoc: both bodies and
/// both terminators survive (the second clause is not collateral).
#[test]
fn issue2429_case_two_clauses_each_ending_heredoc() {
    let case = make_case("multi");
    let script = r#"f() {
  case $1 in
    a) cat <<HA
body-a
HA
    ;;
    b) cat <<HB
body-b
HB
    ;;
  esac
}
f b &
wait
f a &
wait
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "body-b\nbody-a\ndone rc=0\n");
}

/// `;&` (fallthrough) after a clause-final heredoc: the terminator
/// takes the newline too, and the fallthrough still executes.
#[test]
fn issue2429_case_clause_heredoc_fallthrough() {
    let case = make_case("fall");
    let script = r#"f() {
  case $1 in
    a) cat <<HA
fall-body
HA
    ;&
    b) echo fall-into-b
    ;;
  esac
}
f a &
wait
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "fall-body\nfall-into-b\ndone rc=0\n");
}

/// `;;&` (test-next) after a clause-final heredoc.
#[test]
fn issue2429_case_clause_heredoc_test_next() {
    let case = make_case("testnext");
    let script = r#"f() {
  case $1 in
    a) cat <<HA
tn-body
HA
    ;;&
    *a*) echo pattern-hit
    ;;
  esac
}
f a &
wait
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "tn-body\npattern-hit\ndone rc=0\n");
}

/// Two heredoc commands inside ONE clause, the second one
/// clause-final: both bodies land after their own lines and the
/// terminator still starts fresh.
#[test]
fn issue2429_case_clause_two_heredocs() {
    let case = make_case("twohd");
    let script = r#"f() {
  case $1 in
    a) cat <<H1
one
H1
       cat <<H2
two
H2
    ;;
  esac
}
f a &
wait
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "one\ntwo\ndone rc=0\n");
}

/// The case rides inside a backgrounded subshell (the subshell node is
/// the transported body) rather than a function.
#[test]
fn issue2429_case_heredoc_in_background_subshell() {
    let case = make_case("subcase");
    let script = r#"( case a in
  a) cat <<HD
sub-case-body
HD
  ;;
  esac ) &
wait
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "sub-case-body\ndone rc=0\n");
}

/// A pipeline whose first stage is fed by the heredoc, as the
/// clause-final command: the body defers past the pipeline line, then
/// the terminator starts fresh.
#[test]
fn issue2429_case_clause_heredoc_pipeline_last() {
    let case = make_case("pipe");
    let script = r#"f() {
  case $1 in
    a) cat <<HD | tr 'a-z' 'A-Z'
pipe-body
HD
    ;;
  esac
}
f a &
wait
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "PIPE-BODY\ndone rc=0\n");
}

/// `<<-` tab-stripping heredoc as the clause-final command.
#[test]
fn issue2429_case_clause_heredoc_strip_tabs() {
    let case = make_case("striptab");
    let script = "f() {\n\tcase $1 in\n\ta) cat <<-HD\n\t\ttabbed-body\n\tHD\n\t;;\n\tesac\n}\nf a &\nwait\necho \"done rc=$?\"\n";
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "tabbed-body\ndone rc=0\n");
}

/// A nested case as the outer clause's body, the INNER clause ending
/// with a heredoc: the inner terminator takes the newline; the outer
/// terminator follows `esac` on the same render and stays spaced.
#[test]
fn issue2429_nested_case_inner_clause_heredoc() {
    let case = make_case("nested");
    let script = r#"f() {
  case $1 in
    a) case $2 in
         x) cat <<HD
nested-body
HD
         ;;
       esac
    ;;
  esac
}
f a x &
wait
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "nested-body\ndone rc=0\n");
}

/// A command AFTER the case in the transported function body: the
/// separator after `esac` must stay a normal `; ` (the case text does
/// not end with a heredoc even though a clause inside it did).
#[test]
fn issue2429_case_heredoc_then_command_after_case() {
    let case = make_case("after");
    let script = r#"f() {
  case $1 in
    a) cat <<HD
after-body
HD
    ;;
  esac
  echo post-case
}
f a &
wait
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "after-body\npost-case\ndone rc=0\n");
}

/// Control: the same case inside a `$(...)` respawn (the comsub
/// printer's case arm) already placed the terminator on its own line
/// and must keep doing so.
#[test]
fn issue2429_case_clause_heredoc_comsub_control() {
    let case = make_case("comsub");
    let script = r#"f() {
  x=$(case $1 in
    a) cat <<HD
comsub-body
HD
    ;;
  esac)
  echo "got=$x"
}
f a &
wait
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "got=comsub-body\ndone rc=0\n");
}

/// Control: the foreground (in-process, no respawn) face of the seed.
#[test]
fn issue2429_case_clause_heredoc_foreground_control() {
    let case = make_case("fg");
    let script = r#"f() {
  case $1 in
    a) cat <<HD
fg-body
HD
    ;;
  esac
}
f a
echo "done rc=$?"
"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "fg-body\ndone rc=0\n");
}
