//! rubash#2388 — a heredoc body is LOST inside an async subshell.
//!
//! GNU's fork-shaped children execute the parsed COMMAND tree, so a
//! here-document stays attached to its redirected command across the
//! subshell boundary (execute_cmd.c / redir.c here_document_to_fd,
//! redir.c:423). Rubash respawns a `rubash -c` child from re-rendered
//! TEXT (bash_command_sequence_text / bash_command_source_text), and
//! that renderer never emitted heredocs at all: the headers live only
//! in cmd.redirects as RedirectKind::HereDoc (no mirror field carries
//! them) and neither render path printed them, so the child re-parsed
//! a command with no heredoc and the body vanished (the #2332 pin
//! documented the loss and deferred the body expectation here).
//!
//! The renderer now follows print_cmd.c: the header is printed inline
//! in the redirect list (print_redirection_list) and the body is
//! deferred past the whole command line — past `|`, `&&`/`||`
//! connectors and the `&` terminator (print_heredoc_bodies /
//! print_deferred_heredocs) — with newline separators after a
//! delimiter line, which admits nothing else on its line.
//!
//! Expectations measured against GNU bash 5.3.20 with script files
//! (2026-11-01). Unix-only, like the #2332 pins it extends.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Case {
    dir: PathBuf,
}

fn make_case(tag: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("rubash2388-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    Case { dir }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Same harness shape as the #2332 pins: two prelude lines bind $OUT;
/// transported bodies append to $OUT and the script cats it at the
/// end (synchronized by `wait`).
fn run_script(case: &Case, script: &str) -> (String, String) {
    let path = case.dir.join("case.sh");
    let full = format!(
        "OUT=\"{}\"\n: > \"$OUT\"\n{}\ncat \"$OUT\"\n",
        case.dir.join("out.txt").display(),
        script
    );
    std::fs::write(&path, full).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&case.dir)
        .arg("case.sh")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    assert_eq!(
        output.status.code(),
        Some(0),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// 1-based line number of the first line containing `needle`, in the
/// full script layout run_script produces (two prelude lines + body).
fn line_of(body: &str, needle: &str) -> usize {
    format!("OUT=\n: >\n{body}")
        .lines()
        .position(|line| line.contains(needle))
        .map(|index| index + 1)
        .unwrap_or_else(|| panic!("needle {needle:?} not in script"))
}

/// The #2332 seed shape with the body expectation #2388 owns: pre,
/// the literal heredoc body, and post — in order, with the $LINENO
/// readings still exact.
#[test]
fn issue2388_asyncsub_heredoc_body_in_function() {
    let case = make_case("seed");
    let script = r#"f() {
  (
    echo "pre=$LINENO" >> "$OUT"
    cat <<'XEOF' >> "$OUT"
literal
XEOF
    echo "post=$LINENO" >> "$OUT"
  ) &
  wait
}
f"#;
    let (stdout, _) = run_script(&case, script);
    let pre = line_of(script, "pre=");
    let post = line_of(script, "post=");
    assert_eq!(stdout, format!("pre={pre}\nliteral\npost={post}\n"));
}

/// Top-level async subshell (the whole subshell node is the
/// transported body): the heredoc body arrives between its
/// neighbours.
#[test]
fn issue2388_toplevel_asyncsub_heredoc_body() {
    let case = make_case("toplevel");
    let script = r#"(
  echo "before" >> "$OUT"
  cat <<'EOF' >> "$OUT"
top-body
EOF
  echo "after" >> "$OUT"
) &
wait"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "before\ntop-body\nafter\n");
}

/// An UNQUOTED delimiter: the body expands in the child, against the
/// transported environment, exactly as GNU's fork child expands the
/// body it carries.
#[test]
fn issue2388_asyncsub_heredoc_unquoted_body_expands() {
    let case = make_case("expand");
    let script = r#"v=transported
(
  cat <<EOF >> "$OUT"
val=$v
EOF
) &
wait"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "val=transported\n");
}

/// `<<-` strips leading tabs from the transported body.
#[test]
fn issue2388_asyncsub_heredoc_strip_tabs() {
    let case = make_case("striptabs");
    let script = "(\n  cat <<-'EOF' >> \"$OUT\"\n\t\tTabbed\n\tEOF\n) &\nwait";
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "Tabbed\n");
}

/// Two heredoc commands in sequence inside the transported body:
/// each body lands after its own command line and the separator
/// after a delimiter line is a newline.
#[test]
fn issue2388_asyncsub_two_heredocs_in_body() {
    let case = make_case("two");
    let script = r#"(
  cat <<'A' >> "$OUT"
first
A
  cat <<'B' >> "$OUT"
second
B
) &
wait"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "first\nsecond\n");
}

/// A heredoc feeding a pipeline stage inside the async subshell: the
/// body defers past the whole pipeline line and still feeds stage 1.
#[test]
fn issue2388_asyncsub_heredoc_in_pipeline() {
    let case = make_case("pipeline");
    let script = r#"(
  cat <<'EOF' | tr 'a-z' 'A-Z' >> "$OUT"
piped-body
EOF
) &
wait"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "PIPED-BODY\n");
}

/// The heredoc rides on the subshell node itself (`( ... ) <<EOF &`,
/// the compound-redirect render path).
#[test]
fn issue2388_asyncsub_own_redirect_heredoc() {
    let case = make_case("ownredir");
    let script = r#"( cat >> "$OUT" ) <<'EOF' &
own-body
EOF
wait"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "own-body\n");
}

/// A heredoc directly backgrounded inside a transported function
/// body: the `&` terminator stays on the header line and the body
/// follows it.
#[test]
fn issue2388_backgrounded_heredoc_inside_function_body() {
    let case = make_case("bginside");
    let script = r#"f() {
  cat <<'EOF' >> "$OUT" &
bg-body
EOF
  wait
}
f"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "bg-body\n");
}

/// Control: the synchronous (in-process) subshell in a function
/// already delivered its heredoc body and must keep doing so.
#[test]
fn issue2388_sync_subshell_heredoc_control() {
    let case = make_case("sync");
    let script = r#"f() {
  (
    cat <<'EOF' >> "$OUT"
sync-body
EOF
  )
}
f"#;
    let (stdout, _) = run_script(&case, script);
    assert_eq!(stdout, "sync-body\n");
}
