//! rubash#3054 regressions: the respawn serialization drops
//! `first_word_end_line` (#3028's stamp: a simple command reports
//! the line where its FIRST element finished reading — GNU
//! parse.y:5849-5851 / parse.y:811, execute_cmd.c:936).
//! `CommandLineSpan` (parser/nodes.rs) carries only
//! line/end_line/logical_line, so a body replayed through the
//! respawn triple re-derives the stamp from the re-parsed rendered
//! text — where the split word has already collapsed onto one
//! line — and falls back to the pre-#3052 text-count approximation:
//! the command reports its first word's START line.
//!
//! GNU's fork-shaped children execute the parent's parsed tree, so
//! no stamp is ever lost there; the respawned `rubash -c` child
//! must be handed the stamp through the span transport
//! (__RUBASH_FORK_FN_SPANS for function bodies,
//! __RUBASH_FORK_ROOT_SPANS for root text bodies — both share the
//! triple encoding in executor/compound_exec.rs).
//!
//! Expectations measured against GNU bash 5.3.20 with script files
//! (2026-11-09). Harness shape follows the #2202/#2332 pins: two
//! prelude lines bind $OUT, transported bodies append readings to
//! $OUT, a trailing `cat "$OUT"` surfaces them.
//!
//! Unix-only for the same reasons as the #2058/#2202 pins.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Case {
    dir: PathBuf,
}

fn make_case(tag: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("rubash3054-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    Case { dir }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

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
        .arg("A")
        .arg("B")
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

/// Async child: a transported function body whose command's first
/// word is split by a continuation — the command's own `$LINENO`
/// is the line the word finishes on (its end line), not its start.
#[test]
fn issue3054_async_fn_split_first_word_lineno() {
    let case = make_case("async-lineno");
    let body = "\
f() {
ec\\
ho \"x=$LINENO\" >> \"$OUT\"
}
f &
wait
";
    let (stdout, stderr) = run_script(&case, body);
    let x = line_of(body, "ho \"x=$LINENO\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("x={x}\n"));
}

/// Async child, diagnostic face: the split name's command-not-found
/// error in the transported body reports the word-end line, with
/// the script's name as prefix (as GNU's forked child reports it).
#[test]
fn issue3054_async_fn_split_first_word_error_line() {
    let case = make_case("async-error");
    let body = "\
f() {
foo\\
bar
}
f &
wait
echo \"rc=$?\"
";
    let (stdout, stderr) = run_script(&case, body);
    let line = line_of(body, "bar");
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(
        stderr,
        format!("case.sh: line {line}: foobar: command not found\n")
    );
}

/// Process substitution (input form): the producer child calling a
/// transported function whose body command's first word is split.
#[test]
fn issue3054_procsub_in_fn_split_first_word_lineno() {
    let case = make_case("procsub-in");
    let body = "\
f() {
ec\\
ho \"x=$LINENO\" >> \"$OUT\"
}
cat < <(f) > /dev/null
";
    let (stdout, stderr) = run_script(&case, body);
    let x = line_of(body, "ho \"x=$LINENO\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("x={x}\n"));
}

/// Coproc child: same transported-body face.
#[test]
fn issue3054_coproc_fn_split_first_word_lineno() {
    let case = make_case("coproc");
    let body = "\
f() {
ec\\
ho \"x=$LINENO\" >> \"$OUT\"
}
coproc C { f; }
sleep 0.5
";
    let (stdout, stderr) = run_script(&case, body);
    let x = line_of(body, "ho \"x=$LINENO\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("x={x}\n"));
}

/// Process substitution (output form): the consumer child receives
/// the prelude text, so this rides the definition-time re-seed.
#[test]
fn issue3054_procsub_out_fn_split_first_word_lineno() {
    let case = make_case("procsub-out");
    let body = "\
f() {
ec\\
ho \"x=$LINENO\" >> \"$OUT\"
}
true > >(f)
sleep 0.5
";
    let (stdout, stderr) = run_script(&case, body);
    let x = line_of(body, "ho \"x=$LINENO\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("x={x}\n"));
}

/// Root text body carrier (__RUBASH_FORK_ROOT_SPANS): an async
/// subshell body whose first word is split reports the word-end
/// line in the respawned child.
#[test]
fn issue3054_async_subshell_split_first_word_lineno() {
    let case = make_case("async-subshell");
    let body = "\
( ec\\
ho \"y=$LINENO\" >> \"$OUT\" ) &
wait
";
    let (stdout, stderr) = run_script(&case, body);
    let y = line_of(body, "ho \"y=$LINENO\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("y={y}\n"));
}

/// Root text body, diagnostic face: an async subshell body whose
/// first element is a redirection with a split plain target — the
/// following word's error reports the target's end line in the
/// respawned child.
#[test]
fn issue3054_async_subshell_redirect_first_split_target_error_line() {
    let case = make_case("async-subshell-redir");
    let body = "\
( >x\\
y foooo ) &
wait
";
    let (stdout, stderr) = run_script(&case, body);
    let line = line_of(body, "y foooo");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!("case.sh: line {line}: foooo: command not found\n")
    );
}

/// Control for the cell above: the same subshell body run
/// in-process (no respawn) — already correct since #3052, must
/// not move.
#[test]
fn issue3054_subshell_redirect_first_split_target_in_process_control() {
    let case = make_case("subshell-redir-control");
    let body = "\
( >x\\
y foooo )
";
    let (stdout, stderr) = run_script(&case, body);
    let line = line_of(body, "y foooo");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!("case.sh: line {line}: foooo: command not found\n")
    );
}

/// Chained: a transported body whose first element is a split
/// ASSIGNMENT word — the `$LINENO` in its own RHS expands under
/// the stamp (the assignment word's end line), and the value
/// carries the reading to the next body command.
#[test]
fn issue3054_async_fn_split_assignment_word_lineno_chain() {
    let case = make_case("async-assign");
    let body = "\
f() {
x=va\\
lue_$LINENO
echo \"x=$x\" >> \"$OUT\"
}
f &
wait
";
    let (stdout, stderr) = run_script(&case, body);
    let end = line_of(body, "lue_$LINENO");
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("x=value_{end}\n"));
}

/// Traversal alignment: the split word is in the SECOND body
/// command; the first (unsplit) command keeps its own line and
/// the second reports its word-end line — the span re-seed must
/// stay aligned per node, not shift by the collapsed line.
#[test]
fn issue3054_async_fn_second_command_split_word_lineno() {
    let case = make_case("async-second");
    let body = "\
f() {
echo \"p=$LINENO\" >> \"$OUT\"
ec\\
ho \"x=$LINENO\" >> \"$OUT\"
}
f &
wait
";
    let (stdout, stderr) = run_script(&case, body);
    let p = line_of(body, "p=$LINENO");
    let x = line_of(body, "ho \"x=$LINENO\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("p={p}\nx={x}\n"));
}

/// Nested respawn: the async child runs the transported f, which
/// itself backgrounds g — g's split first word still reports its
/// word-end line after two transports.
#[test]
fn issue3054_nested_respawn_split_first_word_lineno() {
    let case = make_case("nested");
    let body = "\
g() {
ec\\
ho \"g=$LINENO\" >> \"$OUT\"
}
f() {
g &
wait
}
f &
wait
";
    let (stdout, stderr) = run_script(&case, body);
    let g = line_of(body, "ho \"g=$LINENO\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("g={g}\n"));
}

/// Control: the plain in-process call was fixed by #3052 and must
/// not move — same body, no respawn anywhere.
#[test]
fn issue3054_direct_call_split_first_word_lineno_control() {
    let case = make_case("direct");
    let body = "\
f() {
ec\\
ho \"x=$LINENO\" >> \"$OUT\"
}
f
";
    let (stdout, stderr) = run_script(&case, body);
    let x = line_of(body, "ho \"x=$LINENO\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("x={x}\n"));
}
