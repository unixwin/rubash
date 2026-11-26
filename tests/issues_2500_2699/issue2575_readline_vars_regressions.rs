//! rubash#2575 — readline completion variables set via `bind` were
//! stored in the bind registry and reported by `bind -v`, but the
//! engine completion pipeline could not see the registry: the
//! candidate filter in complete_line_candidates_with_dynamic was
//! unconditionally case-sensitive and path completion listed dotfiles
//! unconditionally.
//!
//! GNU ownership (lib/readline/complete.c): complete_fncmp
//! (:2428-2457) folds case under _rl_completion_case_fold
//! (completion-ignore-case) and treats '-'/'_' as equivalent under
//! _rl_completion_case_map (completion-map-case); :2630-2640 skips
//! hidden files when _rl_match_hidden_files == 0 (match-hidden-files
//! off) and the typed filename part is empty; disable-completion
//! removes the completion attempt entirely.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
}

#[test]
fn issue_2575_completion_ignore_case_filters_case_insensitively() {
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -W 'Alpha ALPINE beta' foo");
    // Default (off): the filter is case-sensitive, `al` matches nothing.
    assert!(executor.complete_line("foo al", 6).is_empty());
    run_script(&mut executor, "bind 'set completion-ignore-case on'");
    assert_eq!(
        executor.complete_line("foo al", 6),
        vec!["ALPINE".to_string(), "Alpha".to_string()]
    );
}

#[test]
fn issue_2575_completion_map_case_treats_dash_underscore_equivalent() {
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -W 'foo-bar other' cmd");
    assert!(executor.complete_line("cmd foo_bar", 11).is_empty());
    run_script(&mut executor, "bind 'set completion-map-case on'");
    assert_eq!(
        executor.complete_line("cmd foo_bar", 11),
        vec!["foo-bar".to_string()]
    );
}

#[test]
fn issue_2575_disable_completion_empties_candidates() {
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -W 'alpha alpine' foo");
    assert_eq!(executor.complete_line("foo al", 6).len(), 2);
    run_script(&mut executor, "bind 'set disable-completion on'");
    assert!(executor.complete_line("foo al", 6).is_empty());
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch_with_files() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2575-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::write(dir.join(".hidden_file"), "x").expect("write hidden");
    std::fs::write(dir.join("visible_file"), "x").expect("write visible");
    dir
}

#[test]
fn issue_2575_match_hidden_files_off_skips_dotfiles_on_empty_word() {
    let dir = scratch_with_files();
    let prefix = format!("{}/", dir.to_str().expect("utf8 path"));
    let mut executor = Executor::new();
    // Default (on): an empty filename part lists dotfiles too. The
    // word carries an absolute directory prefix so the test does not
    // depend on the process cwd.
    let line = format!("ls {prefix}");
    let listed = executor.complete_line(&line, line.len());
    assert!(
        listed.iter().any(|c| c.ends_with(".hidden_file")),
        "default must list dotfiles: {listed:?}"
    );
    run_script(&mut executor, "bind 'set match-hidden-files off'");
    let listed = executor.complete_line(&line, line.len());
    assert!(
        listed.iter().any(|c| c.ends_with("visible_file")),
        "visible files still listed: {listed:?}"
    );
    assert!(
        !listed.iter().any(|c| c.ends_with(".hidden_file")),
        "match-hidden-files off must skip dotfiles on an empty word: {listed:?}"
    );
    // A typed dot still reaches them (complete.c:2635 gates only the
    // empty-filename case).
    let line = format!("ls {prefix}.h");
    let listed = executor.complete_line(&line, line.len());
    assert!(
        listed.iter().any(|c| c.ends_with(".hidden_file")),
        "typed prefix still reaches dotfiles: {listed:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
