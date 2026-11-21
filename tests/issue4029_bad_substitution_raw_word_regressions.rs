//! rubash#4029 — `bad substitution` must preserve raw quote delimiters and
//! backslashes in the cited word, matching GNU bash 5.3.x.
//!
//! GNU anchors:
//! - `parse.y:5305 read_token_word()`: keeps `'...'`, `"..."`, and `\x` escapes
//!   (dropping only `\<newline>`), converts `$'...'` via `ansiexpand` +
//!   `sh_single_quote` (`parse.y:5548-5576`), and converts `$"..."` via
//!   `locale_expand` + `sh_mkdoublequoted` (`parse.y:5579-5601`).
//! - `subst.c:11229 expand_word_internal()`: at top level passes `word->word`
//!   to `param_expand` -> `parameter_brace_expand`; when entering `"..."`
//!   (`subst.c:11687`), calls `string_extract_double_quoted` (`subst.c:852`)
//!   and expands that extracted segment as its own word.
//! - `subst.c:10278 parameter_brace_expand()`: `report_error(_("%s: bad substitution"), string)`
//!   prints `string` from the active `expand_word_internal` frame.
//!
//! Expectations verified against WSL GNU Bash 5.3.20 via script files.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash4029-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(script: &str) -> (String, String, i32) {
    let dir = tempdir("case");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let rc = output.status.code().unwrap_or(-1);
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, rc)
}

fn diagnostic_word(stderr: &str) -> Option<String> {
    let after = stderr.split_once(": line 1: ")?.1;
    after
        .strip_suffix(": bad substitution\n")
        .map(str::to_string)
}

fn check_bad_subst(script: &str, expected_word: &str, expected_rc: i32) {
    let (_stdout, stderr, rc) = run_script(script);
    assert_eq!(
        rc, expected_rc,
        "script {script:?}: rc {rc}, stderr: {stderr}"
    );
    assert_eq!(
        diagnostic_word(&stderr).as_deref(),
        Some(expected_word),
        "script {script:?}: stderr: {stderr}"
    );
}

#[test]
fn issue4029_single_quoted_affixes() {
    check_bad_subst(
        "echo pre'$(echo sq >&2)'${-x}\n",
        "pre'$(echo sq >&2)'${-x}",
        1,
    );
    check_bad_subst("echo pre'sq'${-x}\n", "pre'sq'${-x}", 1);
    check_bad_subst("echo 'pre'a${-x}\n", "'pre'a${-x}", 1);
    check_bad_subst("echo ${-x}'sq'\n", "${-x}'sq'", 1);
}

#[test]
fn issue4029_double_quoted_affixes() {
    check_bad_subst("echo pre\"dq\"${-x}\n", "pre\"dq\"${-x}", 1);
    check_bad_subst("echo ${-x}\"dq\"\n", "${-x}\"dq\"", 1);
    check_bad_subst("echo \"pre\"a${-x}\n", "\"pre\"a${-x}", 1);
}

#[test]
fn issue4029_inside_double_quotes() {
    check_bad_subst("echo \"pre${-x}post\"\n", "pre${-x}post", 1);
    check_bad_subst("echo pre\"a${-x}b\"post\n", "a${-x}b", 1);
    check_bad_subst("echo \"pre\\\"${-x}post\"\n", "pre\"${-x}post", 1);
    check_bad_subst("echo \"pre\\$${-x}\"\n", "pre\\$${-x}", 1);
    check_bad_subst("echo \"pre\\\\${-x}\"\n", "pre\\\\${-x}", 1);
    check_bad_subst("echo \"pre\\a${-x}\"\n", "pre\\a${-x}", 1);
    check_bad_subst("echo \"pre'${-x}'post\"\n", "pre'${-x}'post", 1);
}

#[test]
fn issue4029_backslash_escapes() {
    check_bad_subst("echo pre\\a${-x}\n", "pre\\a${-x}", 1);
    check_bad_subst("echo a\\ b${-x}\n", "a\\ b${-x}", 1);
    check_bad_subst("echo \\$${-x}\n", "\\$${-x}", 1);
    check_bad_subst("echo \\\\${-x}\n", "\\\\${-x}", 1);
    check_bad_subst("echo ${-x}\\a\n", "${-x}\\a", 1);
}

#[test]
fn issue4029_ansic_and_locale_quotes() {
    check_bad_subst("echo $'a\\'b'${-x}\n", "'a'\\''b'${-x}", 1);
    check_bad_subst("echo $'a\\nb'${-x}\n", "'a\nb'${-x}", 1);
    check_bad_subst("echo $\"a\\$b\"${-x}\n", "\"a\\$b\"${-x}", 1);
    check_bad_subst("echo $\"a${-x}b\"\n", "a${-x}b", 1);
    check_bad_subst("echo $\"a\\\"${-x}b\"\n", "a\"${-x}b", 1);
    check_bad_subst("echo ${$'x'}\n", "${'x'}", 1);
}

#[test]
fn issue4029_operator_rhs() {
    check_bad_subst("echo ${v:-'a'${-x}}\n", "'a'${-x}", 1);
    check_bad_subst("echo ${v:-\"a${-x}\"}\n", "a${-x}", 1);
    check_bad_subst("echo ${v:-$'a\\'b'${-x}}\n", "'a'\\''b'${-x}", 1);
    check_bad_subst("echo ${v:-pre\"a${-x}b\"post}\n", "a${-x}b", 1);
    check_bad_subst("echo \"${v:-'${-x}'}\"\n", "'${-x}'", 1);
    check_bad_subst("echo \"${v:-\"a${-x}\"}\"\n", "a${-x}", 1);
    check_bad_subst("echo \"${v:-\"a\\\"${-x}\"}\"\n", "a\\\"${-x}", 1);
}

#[test]
fn issue4029_assignments_and_export() {
    check_bad_subst("x=pre\"dq\"${-x}\n", "pre\"dq\"${-x}", 1);
    check_bad_subst("x=\"dq${-x}\"\n", "dq${-x}", 1);
    check_bad_subst("x=esc\\$${-x}\n", "esc\\$${-x}", 1);
    check_bad_subst("x=$'a\\'b'${-x}\n", "'a'\\''b'${-x}", 1);
    check_bad_subst("export x=pre\"dq\"${-x}\n", "x=pre\"dq\"${-x}", 1);
    check_bad_subst("export x=\"dq${-x}\"\n", "dq${-x}", 1);
}

#[test]
fn issue4029_posix_mode_deferral() {
    check_bad_subst("set -o posix; a=pre'sq'${-x}\n", "pre'sq'${-x}", 1);
    check_bad_subst("set -o posix; echo pre\"a${-x}b\"post\n", "a${-x}b", 1);
}

#[test]
fn issue4029_compound_assignment_elements() {
    check_bad_subst("a=([0]=\"pre\\\"${-x}\\\"\")\n", "pre\"${-x}\"", 1);
    check_bad_subst("a=([0]=pre\"a${-x}b\")\n", "a${-x}b", 1);
    check_bad_subst("a=(\"pre${-x}post\")\n", "pre${-x}post", 1);
    check_bad_subst("a=(pre'sq'${-x})\n", "pre'sq'${-x}", 1);
}

#[test]
fn issue4029_compound_commands_redirects_and_braces() {
    check_bad_subst("for k in pre'sq'${-x}; do :; done\n", "pre'sq'${-x}", 1);
    check_bad_subst("for k in pre\"a${-x}b\"post; do :; done\n", "a${-x}b", 1);
    check_bad_subst("case pre'sq'${-x} in *) ;; esac\n", "pre'sq'${-x}", 1);
    check_bad_subst("case pre\"a${-x}b\"post in *) ;; esac\n", "a${-x}b", 1);
    check_bad_subst("[[ pre'sq'${-x} == x ]]\n", "pre'sq'${-x}", 1);
    check_bad_subst("[[ pre\"a${-x}b\"post == x ]]\n", "a${-x}b", 1);
    check_bad_subst("echo hi > pre'sq'${-x}\n", "pre'sq'${-x}", 1);
    check_bad_subst("echo hi > \"pre${-x}post\"\n", "pre${-x}post", 1);
    check_bad_subst("cat <<< pre'sq'${-x}\n", "pre'sq'${-x}", 1);
    check_bad_subst("cat <<< \"pre${-x}post\"\n", "pre${-x}post", 1);
    check_bad_subst("echo {a,b}'q'${-x}\n", "a'q'${-x}", 1);
    check_bad_subst("echo {a,b}\"dq\"${-x}\n", "a\"dq\"${-x}", 1);
}
