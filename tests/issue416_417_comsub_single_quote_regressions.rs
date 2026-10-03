//! Issue rubash#416 + #417 (themesweep P1 wave, lane wt54/quotecd):
//! single-quoted text inside a `$( )` command substitution — and in any
//! pipeline stage after the first — reached the consumer with backslashes
//! removed (`\n` -> `n`) and `${...}` shapes mangled.
//!
//! Three root causes, all fixed:
//!
//! 1. `expand_braces_with_optional_raw` + `word_contains_brace_group`
//!    (command_prepare.rs) re-read the COOKED pipeline-stage word as shell
//!    syntax. The cooked word encodes a quoted `$` as the DATA_DOLLAR
//!    carrier (U+001F), so the `${`-body skip failed, `word_contains_
//!    brace_group` reported a brace group inside `${...}`-shaped data, and
//!    the follow-up `remove_shell_quotes` consumed data backslashes as
//!    escapes. GNU spec: braces.c:679-680 brace_gobbler "treat ${...} like
//!    \{...}" — a `${` never contributes brace syntax; brace expansion runs
//!    on the word text with quoting intact (expand_word_internal ->
//!    braces.c brace_expand), never on quote-removed data.
//! 2. `push_external_args` (executor/path.rs) delivered argv to
//!    MSYS2-hosted children (Git-for-Windows `usr/bin` tools) unquoted, and
//!    the child's POSIX runtime re-expands unquoted command-line tokens
//!    (`${S}` -> `$S`, `{S}` -> `S`, `\n` -> newline; probe 2026-10-02).
//!    GNU spec: execute_cmd.c:6139+ shell_execve hands execve the raw word
//!    bytes — nothing ever expands them again. The MSYS expansion-grammar
//!    bytes (`\ $ { } ~ ` backtick quote) now ride double-quoted for
//!    POSIX-runtime-hosted children only, so native children keep the
//!    pinned unquoted dialect (niubash#124(b)).
//! 3. `brace_expanded_pipeline_stage` (pipeline_exec.rs) rewrote a stage's
//!    `words` while leaving `word_metadata` on the pre-expansion word, so
//!    the per-word expansion re-ran braces from the stale raw and
//!    duplicated the tail fields (`printf 'x\n' | printf '%s\n' {a,b}`
//!    printed `a b b`). GNU runs brace expansion exactly once per word
//!    (subst.c:11229 expand_word_internal -> braces.c brace_expand); the
//!    pre-pass is gone and the per-word machinery owns the step.
//!
//! Every expectation below was byte-verified against WSL GNU Bash 5.3.0
//! (script-file probes, target/probe/matrix.sh + i6b.sh, 2026-10-02).

use std::process::Command;

fn run_script(script: &str) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

/// rubash#416 exact reproducer: the single-quoted sed program
/// `'${s/^\(.*\)exit$/\1/p;}'` must reach sed verbatim inside `$( )`.
#[test]
fn comsub_single_quoted_sed_pattern_substitution_stays_verbatim() {
    let (stdout, stderr, ok) = run_script(
        "out=$(printf '%s\\n' \"PROMPTSEG> exit\" | sed -n '${s/^\\(.*\\)exit$/\\1/p;}')\n\
         printf 'PIPE_LITERAL=[%s]\\n' \"$out\"",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "PIPE_LITERAL=[PROMPTSEG> ]\n");
}

/// rubash#417 exact reproducer: hawaii50's join — a backslash pattern and a
/// literal `${SEP}` replacement inside `$( )` on Windows' MSYS sed.
#[test]
fn comsub_single_quoted_sed_backslash_and_brace_dollar_survive_exec() {
    let (stdout, stderr, ok) = run_script(
        "out=$(printf 'You do not need one\\nsecond line\\n' | sed -e :a -e '$!N;s/\\n/${SEP}/;ta')\n\
         printf 'JOINED=[%s]\\n' \"$out\"",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "JOINED=[You do not need one${SEP}second line]\n");
}

/// A single-quoted word in pipeline stage >= 2 keeps its backslashes,
/// `${...}` text and `$` bytes when the consumer is a builtin.
#[test]
fn pipeline_stage_single_quoted_word_reaches_builtin_verbatim() {
    let (stdout, stderr, ok) = run_script(
        "VAR=live\n\
         printf 'x\\n' | printf 'W2=[%s]\\n' 'a\\nb${VAR}c'\n\
         printf 'x\\n' | cat | printf 'W3=[%s]\\n' 'q\\r$!t${U}v'",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "W2=[a\\nb${VAR}c]\nW3=[q\\r$!t${U}v]\n");
}

/// The same integrity for a script-file consumer: argv crossing a real
/// child boundary (the script re-renders "$@" byte-for-byte).
#[test]
fn pipeline_stage_single_quoted_word_reaches_script_child_verbatim() {
    let dir = std::env::temp_dir().join("rubash-issue417-argv");
    std::fs::create_dir_all(&dir).unwrap();
    let dumper = dir.join("dump-args.sh");
    std::fs::write(
        &dumper,
        "for a in \"$@\"; do printf 'ARG=[%s]\\n' \"$a\"; done\n",
    )
    .unwrap();
    let dumper = dumper.to_string_lossy().replace('\\', "/");
    let (stdout, stderr, ok) = run_script(&format!(
        "printf 'x\\n' | {dumper} 'a\\nb${{VAR}}c' '$s/^\\(.*\\)$/'"
    ));
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "ARG=[a\\nb${VAR}c]\nARG=[$s/^\\(.*\\)$/]\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// GNU braces.c runs brace expansion ONCE per word: a brace word in a
/// pipeline stage must not duplicate its tail fields (the stale-metadata
/// double expansion), and quoted braces stay literal.
#[test]
fn pipeline_stage_brace_expansion_runs_once_and_respects_quotes() {
    let (stdout, stderr, ok) = run_script(
        "printf 'x\\n' | printf 'G1=[%s]\\n' {a,b}\n\
         printf 'x\\n' | printf 'G2=[%s]\\n' pre{a,b}'$V'post\n\
         printf 'x\\n' | printf 'G3=[%s]\\n' '{a,b}'{c,d}",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "G1=[a]\nG1=[b]\nG2=[prea$Vpost]\nG2=[preb$Vpost]\nG3=[{a,b}c]\nG3=[{a,b}d]\n"
    );
}

/// Top-level (non-pipeline) single-quoted words were always correct; pin
/// them so the pipeline fix cannot trade one surface for the other.
#[test]
fn top_level_single_quoted_word_stays_verbatim() {
    let (stdout, stderr, ok) = run_script("printf 'TOP=[%s]\\n' 'a\\nb${VAR}c' '$s/^\\(.*\\)$/'");
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "TOP=[a\\nb${VAR}c]\nTOP=[$s/^\\(.*\\)$/]\n");
}
