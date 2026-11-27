//! rubash#2828 — bounded content-keyed parse cache for runtime text
//! re-parsed by `eval` / `trap` actions / `source`.
//!
//! The cache key carries the parsed bytes plus every parser-affecting
//! input — line offset, posix mode, the extglob gate, `expand_aliases`,
//! and the alias-table generation — so an `alias`/`unalias` between two
//! identical runtime strings can never serve a stale alias-expanded AST,
//! while repeated firings reuse the parse.
//!
//! Expected outputs probed against WSL GNU Bash 5.3.x
//! (`wsl /usr/local/bin/bash`, LF-terminated script files):
//!   - `eval 'alias g="echo G"; g x'` first call -> `g: command not
//!     found` (the whole string is parsed before the alias lands, so
//!     `g` does not expand), second identical call -> `G x` (the table
//!     now holds `g`). A stale cache hit would wrongly repeat the
//!     not-found error.
//!   - `trap 'echo ERR-FIRED' ERR; false; false` fires twice;
//!     `trap 'echo USR' USR1` + two `kill -USR1 $$` fires twice.
//!   - `. file` twice runs both times.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_script_file(path: &std::path::Path) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// (a) An alias defined between two `eval` calls with identical text
/// changes what the second call's parse expands — the cache must miss.
#[test]
fn alias_defined_between_evals_reparses() {
    let (stdout, stderr, _) = rubash(
        "shopt -s expand_aliases\n\
         eval 'alias g=\"echo G\"; g x' 2>&1; echo \"rc1=$?\"\n\
         eval 'alias g=\"echo G\"; g x' 2>&1; echo \"rc2=$?\"\n",
    );
    assert!(
        stdout.contains("g: command not found") || stderr.contains("g: command not found"),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(stdout.contains("rc1=127"), "stdout: {stdout}");
    assert!(stdout.contains("G x"), "stdout: {stdout}");
    assert!(stdout.contains("rc2=0"), "stdout: {stdout}");
}

/// (a) Defined -> redefined -> removed: every `eval` of the same text
/// must reflect the alias table live at its own parse, never a cached
/// expansion.
#[test]
fn alias_redefined_and_removed_between_evals_reparses() {
    let (stdout, _stderr, _) = rubash(
        "shopt -s expand_aliases\n\
         alias e='echo A'\n\
         eval 'e one'\n\
         alias e='echo B'\n\
         eval 'e two'\n\
         unalias e\n\
         eval 'e three' 2>&1; echo \"rc=$?\"\n",
    );
    assert!(stdout.starts_with("A one\nB two\n"), "stdout: {stdout}");
    assert!(stdout.contains("e: command not found"), "stdout: {stdout}");
    assert!(stdout.ends_with("rc=127\n"), "stdout: {stdout}");
}

/// Same, with `unalias -a` wiping the table between firings.
#[test]
fn unalias_all_between_evals_reparses() {
    let (stdout, _, _) = rubash(
        "shopt -s expand_aliases\n\
         alias e='echo A'\n\
         eval 'e one'\n\
         unalias -a\n\
         eval 'e one' 2>&1; echo \"rc=$?\"\n",
    );
    assert!(stdout.starts_with("A one\n"), "stdout: {stdout}");
    assert!(stdout.contains("e: command not found"), "stdout: {stdout}");
    assert!(stdout.ends_with("rc=127\n"), "stdout: {stdout}");
}

/// (b) Repeated `eval` of the same text and repeated `trap` firings
/// execute identically — the cached AST is semantically identical to a
/// fresh parse.
#[test]
fn repeated_eval_and_trap_firings_execute_identically() {
    let (stdout, _, code) = rubash(
        "for i in 1 2 3; do eval 'echo \"E$i\"'; done\n\
         x=world\n\
         for i in 1 2; do eval 'echo \"hello $x\"'; done\n\
         eval 'a=$((20+22))'\n\
         echo \"a=$a\"\n",
    );
    assert_eq!(stdout, "E1\nE2\nE3\nhello world\nhello world\na=42\n");
    assert_eq!(code, Some(0));
}

#[test]
fn repeated_signal_trap_firings_execute_identically() {
    let (stdout, _, _) = rubash(
        "trap 'echo USR' USR1\n\
         kill -USR1 $$\n\
         kill -USR1 $$\n\
         echo done\n",
    );
    assert_eq!(stdout, "USR\nUSR\ndone\n");
}

#[test]
fn repeated_err_trap_firings_execute_identically() {
    let (stdout, _, _) = rubash(
        "trap 'echo ERR-FIRED' ERR\n\
         false\n\
         false\n\
         trap - ERR\n\
         true; false\n\
         trap 'echo E2' ERR\n\
         false\n\
         echo end\n",
    );
    assert_eq!(stdout, "ERR-FIRED\nERR-FIRED\nE2\nend\n");
}

/// `source` of the same file twice: each run re-executes identical
/// semantics (and changing the file between sources re-parses).
#[test]
fn source_twice_and_changed_source_reparse() {
    let dir = std::env::temp_dir();
    let file = dir.join(format!("issue2828_source_{}.sh", std::process::id()));
    std::fs::write(&file, "echo \"S1 $v\"\nv=inside\n").expect("write source fixture");
    let script = format!(
        "v=outer\n. \"{}\"\nv=outer2\n. \"{}\"\necho \"v=$v\"\n",
        file.display(),
        file.display()
    );
    let (stdout, _, _) = rubash(&script);
    assert_eq!(stdout, "S1 outer\nS1 outer2\nv=inside\n");
    std::fs::remove_file(&file).ok();
}

/// (c) A flood of distinct runtime strings: far more eval texts than
/// the cache cap (128), each still parsed and executed correctly —
/// eviction under pressure never corrupts semantics.
#[test]
fn distinct_eval_flood_stays_correct() {
    let (stdout, _, code) = rubash(
        "for i in $(seq 1 400); do eval \"echo flood-$i\"; done\n\
         echo flood-done\n",
    );
    let mut lines = stdout.lines();
    for i in 1..=400 {
        assert_eq!(lines.next(), Some(format!("flood-{i}").as_str()));
    }
    assert_eq!(lines.next(), Some("flood-done"));
    assert_eq!(code, Some(0));
}

/// (d) Cache-observability pin (graft #4078): with
/// `RUBASH_PARSE_CACHE_STATS=1` the shell reports the runtime parse
/// cache counters at exit. Repeated identical `eval`s on one script
/// line must hit (the key carries the caller line, so the loop body is
/// a single line); an alias-table mutation between identical `eval`s
/// must miss and re-parse. On a tree without the cache there is no
/// stats line at all, so this pin is red on pristine — a true
/// discriminating pin, not a guard.
#[test]
fn runtime_parse_cache_stats_reports_hits_and_alias_invalidation() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("RUBASH_PARSE_CACHE_STATS", "1")
        .arg("-c")
        .arg(
            "shopt -s expand_aliases\n\
             alias e='echo A'\n\
             for i in 1 2; do eval 'e one'; done\n\
             alias e='echo B'\n\
             for i in 1 2; do eval 'e one'; done\n",
        )
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stdout, "A one\nA one\nB one\nB one\n", "stdout: {stdout}");
    let stats = stderr
        .lines()
        .find(|line| line.starts_with("rubash: runtime-parse-cache "))
        .expect("stats line present; stderr: {stderr}");
    // First loop: miss + hit; alias redefinition bumps the generation,
    // so the second loop misses again, then hits.
    assert!(stats.contains("hits=2"), "stats: {stats}");
    assert!(stats.contains("misses=2"), "stats: {stats}");
}

/// The same flood re-run through a script FILE (script-driver path +
/// eval), checking a revisited early string still executes correctly
/// after eviction churn.
#[test]
fn distinct_eval_flood_then_revisit() {
    let dir = std::env::temp_dir();
    let file = dir.join(format!("issue2828_flood_{}.sh", std::process::id()));
    std::fs::write(
        &file,
        "for i in $(seq 1 300); do eval \"echo f-$i\"; done\n\
         eval 'echo revisit'\n",
    )
    .expect("write flood fixture");
    let (stdout, _, code) = rubash_script_file(&file);
    assert_eq!(stdout.lines().count(), 301, "stdout: {stdout}");
    assert!(stdout.ends_with("revisit\n"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
    std::fs::remove_file(&file).ok();
}
