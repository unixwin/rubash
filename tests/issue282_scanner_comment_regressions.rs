//! Issue rubash#282 regressions (wt12/modinit lane): modernish init's
//! fatal.sh battery abort, next layer after #256/#257.
//!
//! Root cause (two stacked defects, both fixed here):
//!
//! 1. The text-layer completeness scanners in `script_driver.rs`
//!    (`first_unquoted_char`, `unquoted_delimiter_depth` — the engines
//!    behind `stdin_source_has_unclosed_function_body` — and
//!    `line_paren_delta`) toggled single/double quote state on every
//!    apostrophe or quote they walked past, including text inside
//!    COMMENTS. GNU never lets comment text enter quote state: an
//!    unquoted `#` at word start discards through the newline (parse.y:3630
//!    shell_getc comment branch — `#` when `!interactive ||
//!    interactive_comments` sets PST_COMMENT and discards until EOL), and
//!    read_token_word opens LEX_INCOMMENT only when the `#` begins a word
//!    (parse.y:3937-3940: `retind == 0` or the previous word char is a
//!    newline or a shellblank); while LEX_INCOMMENT is set, "don't bother
//!    counting parens or doing anything else" (parse.y:3922-3931). With a
//!    `# it's fine` comment inside a function body, the stale quote state
//!    made the scanners skip the body's real closing `}` while counting
//!    its opener, so `stdin_source_has_unclosed_function_body` judged the
//!    COMPLETE rest-of-file group unclosed.
//!
//! 2. The misjudged group is still alias-expanded text (the output of
//!    `expand_group_aliases`), but the incomplete-group branch of
//!    `run_source_groups` executed it WITHOUT setting
//!    `__RUBASH_ALIAS_STREAMED` — unlike the complete branch right below
//!    it. With the marker unset, `comsub_body_alias_splice_extracted`
//!    re-spliced a `$( ... )` body that the group-level pass had already
//!    expanded, so `alias let='let --'` applied twice inside a sourced
//!    module: `while let "(i+=1)<4"` became `let -- -- "..."` and the let
//!    builtin died on the leftover `--` operand. GNU expands each alias
//!    exactly once while reading (parse.y:3249 alias_expand_token /
//!    push_string; AL_BEINGEXPANDED at parse.y:3259 prevents re-entry).
//!
//! Observable chain (modernish @63bdae02, LF checkout, goodsh patched to
//! accept PPID=1 candidates): `bin/modernish --use=_IN/sig` died with
//! `sig.mm: line 125: let: --: arithmetic syntax error: operand expected
//! (error token is "-")` while WSL GNU Bash 5.3.0 inits the same module
//! silently; the same fix takes `bin/modernish --test` past the module
//! battery (`--test` still aborts later at sys/cmd/harden.mm line 404 —
//! a separate nested-eval parse divergence, not this issue).
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28; the minimal
//! reproducer matrix lives at target/i282/r3*.out under the lane
//! worktree).

use std::io::Write;
use std::process::{Command, Stdio};

/// Run a driver script FILE plus a module it sources through a function
/// (the modernish `use()` shape: `use() { _Msh_doUse() { . mod >&2; } }`)
/// in its own scratch directory; returns (stdout, stderr, code).
fn run_pair(driver: &str, module: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i282-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("mod.sh"), module).expect("write mod.sh");
    std::fs::write(dir.join("driver.sh"), driver).expect("write driver.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("driver.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash driver");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

// ---------------------------------------------------------------------------
// The minimal faithful reproducer (r3): GNU runs the loop 3 times; the
// pre-fix build printed `let: --: arithmetic syntax error` and n=0.
// ---------------------------------------------------------------------------

const R3_DRIVER: &str = "shopt -s expand_aliases\n\
                         alias let='let --'\n\
                         use() {\n\
                         \t. \"$1\" >&2\n\
                         }\n\
                         use mod.sh\n\
                         echo DONE\n";

const R3_MODULE: &str = "fn() {\n\
                         \t# it's fine\n\
                         \t:\n\
                         }\n\
                         # don't panic\n\
                         n=0\n\
                         for x in $(\n\
                         \ti=0\n\
                         \twhile let \"(i+=1)<4\"; do echo w; done 2>/dev/null)\n\
                         do n=$((n+1)); done\n\
                         echo \"n=$n\"\n";

/// End-to-end: the loop must run 3 times (GNU parity) — one apostrophe
/// inside the function-body comment plus one after the body leave the
/// brace scan balanced overall but desynced at the closing `}`.
#[test]
fn sourced_comment_apostrophe_alias_let_comsub_runs_loop() {
    let (stdout, stderr, _) = run_pair(R3_DRIVER, R3_MODULE);
    // `use mod.sh >&2` routes the module's stdout to stderr, so `n=3`
    // arrives on stderr like in the shell probes.
    assert_eq!(stdout, "DONE\n");
    assert!(
        !stderr.contains("let: --"),
        "alias `let` expanded twice: {stderr}"
    );
    assert!(
        stderr.contains("n=3\n"),
        "loop did not run 3 times: {stderr}"
    );
}

/// A comment-only desync without any alias still must not misjudge the
/// group: the module sources cleanly and the function is callable.
#[test]
fn comment_apostrophes_do_not_break_group_completeness() {
    let (stdout, stderr, _) = run_pair(
        "use() {\n\t. \"$1\" >&2\n}\nuse mod.sh\necho DONE\n",
        "fn() {\n\t# it's fine\n\t:\n}\n\
         # don't panic\n\
         fn\necho called\n",
    );
    assert_eq!(stdout, "DONE\n");
    assert!(stderr.contains("called\n"), "fn not executed: {stderr}");
}

/// line_paren_delta: a `(` mentioned inside a comment must not open a
/// paren group (the heredoc/process-substitution gather stays balanced).
/// The visible effect chosen: a heredoc after a commented line still
/// closes on its delimiter.
#[test]
fn comment_paren_does_not_hold_group_open() {
    let (stdout, stderr, _) = rubash_stdin("# don't (worry\ncat <<'EOF'\nbody\nEOF\necho OK\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout, "body\nOK\n");
}

/// Guard the comment model itself: `#` NOT at word start is literal data
/// (parse.y:3937 requires word start), so `x#y` keeps the `#` and a
/// brace in such a word still counts for the scanner.
#[test]
fn hash_inside_word_is_not_a_comment() {
    let (stdout, stderr, _) = run_pair(
        "use() {\n\t. \"$1\" >&2\n}\nuse mod.sh\necho DONE\n",
        "v='a#b'\ncase $v in\n\
         ( a#b ) echo matched ;;\n\
         ( * ) echo no ;;\n\
         esac\n",
    );
    assert_eq!(stdout, "DONE\n");
    assert!(stderr.contains("matched\n"), "case arm not taken: {stderr}");
}
