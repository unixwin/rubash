//! Issue rubash#2570 regression: the compspec `-o` fallback options
//! (default / bashdefault / dirnames / plusdirs) were parsed by
//! `complete`/`compgen` but never applied in the interactive
//! line-completion path, so a compspec whose function yields an empty
//! COMPREPLY produced zero candidates (niubash#828: fzf's
//! `complete -D` registration blanks TAB completion for every command).
//!
//! GNU ownership:
//! - `pcomplete.c:1435-1457 gen_compspec_completions`: on an empty
//!   generated result, COPT_DIRNAMES nominates directory completion
//!   instead; COPT_PLUSDIRS appends directory candidates whenever the
//!   dirnames branch did not fire (non-empty results included).
//! - `bashline.c:1801-1822 attempt_shell_completion`: with the result
//!   still empty, COPT_DEFAULT leaves readline's filename completion
//!   open, and COPT_BASHDEFAULT falls into `bash_default_completion`
//!   (bashline.c:1830) — $-variable, ~-username, @-hostname and
//!   command-position sets only; a plain argument word completes
//!   NOTHING under -o bashdefault alone.
//!
//! Oracle: GNU bash 5.3.20 driven on a real pty (fresh `bash -i` per
//! case, fixture: files alpha.txt/alpine.log, dirs appledir/applydir/):
//! -o default alone -> all four filename candidates; -o bashdefault
//! alone -> [] for a plain word, $ZEBRA_VAR for `$ZE`, ~root for `~ro`;
//! both (the fzf shape, per-command and via -D) -> filenames;
//! -o dirnames on word `a` -> the two dirs only, and it beats
//! -o default when both are set; -o plusdirs appends the dirs to a
//! non-empty function result and fills an empty one; no -o -> [].

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
}

/// The fixture tests move the process cwd: path completion resolves
/// "." against the session cwd, and a live session keeps PWD and the
/// process cwd in agreement, so the harness establishes both (scripted
/// `cd` for PWD, set_current_dir for the process side) and serializes
/// on a mutex so parallel test threads cannot interleave cwd moves.
static FIXTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Fixture {
    executor: Executor,
    dir: std::path::PathBuf,
    old_cwd: std::path::PathBuf,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn complete(&mut self, registration: &str, line: &str) -> Vec<String> {
        run_script(&mut self.executor, registration);
        self.executor.complete_line(line, line.len())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.old_cwd);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Fixture directory (files alpha.txt/alpine.log, dirs
/// appledir/applydir/) plus the two completion functions, entered as
/// the session cwd.
fn fixture(tag: &str) -> Fixture {
    let lock = FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = std::env::temp_dir().join(format!("rubash-2570-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("appledir")).expect("fixture dir");
    std::fs::create_dir_all(dir.join("applydir")).expect("fixture dir");
    std::fs::write(dir.join("alpha.txt"), "a\n").expect("fixture file");
    std::fs::write(dir.join("alpine.log"), "a\n").expect("fixture file");
    let old_cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&dir).expect("enter fixture");
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        &format!(
            "cd {}\n_empty() {{ COMPREPLY=(); }}\n_two() {{ COMPREPLY=(zebra_one zebra_two); }}\nexport ZEBRA_VAR=1",
            dir.display()
        ),
    );
    Fixture {
        executor,
        dir,
        old_cwd,
        _lock: lock,
    }
}

// Candidate strings are bare names: the engine's path generators
// never append a trailing slash (the host renders directory matches
// with one), on this path exactly as on the no-compspec path.
const FILENAMES: [&str; 4] = ["alpha.txt", "alpine.log", "appledir", "applydir"];
const DIRS: [&str; 2] = ["appledir", "applydir"];

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Pin (1) + the captain's #828 repro shape: a -D default compspec
/// whose function yields empty COMPREPLY, here via a per-command spec
/// with -o default: filename candidates come back.
#[test]
fn default_option_fills_empty_function_result_with_filenames() {
    let mut fx = fixture("def");
    let got = fx.complete("complete -o default -F _empty mycmd", "mycmd ");
    assert_eq!(got, strings(&FILENAMES));
}

/// Pin (2): -o bashdefault alone — GNU bash_default_completion yields
/// NOTHING for a plain argument word, the $-variable set for a $
/// word, and the username set for a ~ word.
#[test]
fn bashdefault_alone_plain_word_empty_but_variable_word_completes() {
    let mut fx = fixture("bashdef");
    let got = fx.complete("complete -o bashdefault -F _empty mycmd", "mycmd ");
    assert_eq!(
        got,
        Vec::<String>::new(),
        "bashdefault must not fall back to filenames for a plain word"
    );
    let got = fx.complete("complete -o bashdefault -F _empty mycmd", "mycmd $ZE");
    assert_eq!(got, strings(&["$ZEBRA_VAR"]));
    let got = fx.complete("complete -o bashdefault -F _empty mycmd", "mycmd ~ro");
    assert!(
        got.iter().all(|c| c.starts_with('~')),
        "bashdefault ~ word must stay in the username set, got {got:?}"
    );
}

/// Pin (3): the `cd` shape — -o dirnames with an empty function
/// result nominates directory candidates only.
#[test]
fn dirnames_fills_empty_function_result_with_directories_only() {
    let mut fx = fixture("dirnames");
    let got = fx.complete("complete -o dirnames -F _empty mycmd", "mycmd a");
    assert_eq!(got, strings(&DIRS));
}

/// Pin (4): -o plusdirs is NOT an empty-only fallback — it appends
/// directory candidates to a non-empty function result, and fills an
/// empty one.
#[test]
fn plusdirs_appends_directories_to_function_results() {
    let mut fx = fixture("plusdirs");
    // GNU oracle shape: empty word, non-empty function result — the
    // directories join the zebras.
    let got = fx.complete("complete -o plusdirs -F _two mycmd", "mycmd ");
    assert_eq!(
        got,
        strings(&["appledir", "applydir", "zebra_one", "zebra_two"]),
        "plusdirs must append directories to non-empty results"
    );
    // The appended directories are word-filtered like the CA_DIRECTORY
    // action itself (bashline.c bash_directory_completion_matches via
    // rl_filename_completion_function): for word `z` none qualify.
    let got = fx.complete("complete -o plusdirs -F _two mycmd", "mycmd z");
    assert_eq!(
        got,
        strings(&["zebra_one", "zebra_two"]),
        "plusdirs directory candidates are prefix-filtered by the word"
    );
    let got = fx.complete("complete -o plusdirs -F _empty mycmd", "mycmd ");
    assert_eq!(
        got,
        strings(&DIRS),
        "plusdirs must fill an empty result with directories"
    );
}

/// Pin (5): a spec WITHOUT any -o fallback and an empty function
/// result still yields zero candidates (no over-fallback).
#[test]
fn empty_function_result_without_options_stays_empty() {
    let mut fx = fixture("noopts");
    let got = fx.complete("complete -F _empty mycmd", "mycmd ");
    assert_eq!(got, Vec::<String>::new());
}

/// Pin (6): the fzf-shaped registration end to end — a -D default
/// compspec plus per-command specs, all yielding empty COMPREPLY:
/// filename completion is restored for arbitrary commands (the
/// niubash#828 / Termux repro).
#[test]
fn fzf_shaped_default_compspec_restores_filename_completion() {
    let mut fx = fixture("fzf");
    for line in ["ls ", "zznope ", "somecmd "] {
        let got = fx.complete(
            "complete -o bashdefault -o default -F _empty -D\ncomplete -o bashdefault -o default -F _empty ls",
            line,
        );
        assert_eq!(
            got,
            strings(&FILENAMES),
            "fzf-shaped registration must restore filenames for {line:?}"
        );
    }
}

/// GNU precedence: on an empty result -o dirnames nominates directory
/// completion INSTEAD of -o default's filenames (pcomplete.c: the
/// dirnames branch replaces; plusdirs is the appending one).
#[test]
fn dirnames_takes_precedence_over_default_on_empty_result() {
    let mut fx = fixture("precedence");
    let got = fx.complete("complete -o dirnames -o default -F _empty mycmd", "mycmd ");
    assert_eq!(got, strings(&DIRS));
}

/// GNU: -o dirnames does NOT fire when the function produced results
/// (pcomplete.c: the dirnames branch requires an empty list).
#[test]
fn dirnames_does_not_fire_on_nonempty_function_result() {
    let mut fx = fixture("dirnames-nonempty");
    let got = fx.complete("complete -o dirnames -F _two mycmd", "mycmd z");
    assert_eq!(got, strings(&["zebra_one", "zebra_two"]));
}

/// -o default is likewise empty-only, and its filename candidates are
/// prefix-filtered by the word being completed.
#[test]
fn default_option_respects_results_and_prefix() {
    let mut fx = fixture("def-prefix");
    let got = fx.complete("complete -o default -F _two mycmd", "mycmd z");
    assert_eq!(
        got,
        strings(&["zebra_one", "zebra_two"]),
        "-o default is an empty-result fallback only"
    );
    // GNU oracle for word `ap`: appledir and applydir only —
    // alpha.txt and alpine.log start with "al", not "ap".
    let got = fx.complete("complete -o default -F _empty mycmd", "mycmd ap");
    assert_eq!(
        got,
        strings(&["appledir", "applydir"]),
        "-o default filenames must be prefix-filtered"
    );
}
