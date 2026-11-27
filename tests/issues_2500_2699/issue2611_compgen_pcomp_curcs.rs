//! rubash#2611 — `compopt` executed inside a function that the
//! compgen builtin itself ran via -F must find a completion
//! context: GNU binds the governing compspec copy as pcomp_curcs
//! for the whole generation window (pcomplete.c:1520-1531
//! gen_progcomp_completions), and compgen generates through the
//! same gen_compspec_completions machinery, so a compopt inside a
//! compgen-run function is legal and its option mutations fold
//! back into the attempt (the `*foundp |= cs->options` read-back).
//! rubash's compgen resolution ran the -F function with no
//! pcomp_curcs bound, so the compopt errored "not currently
//! executing completion function" and the mutation was lost.
//!
//! The guard at the bottom pins the opposite edge: at top level
//! (no completion attempt in flight) GNU errors for the same
//! compopt (probed on GNU bash 5.3.20), so the binding must be
//! scoped to the generation window, not global.

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;
use std::sync::{Mutex, MutexGuard};

static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
}

struct Fixture {
    executor: Executor,
    dir: std::path::PathBuf,
    old_cwd: std::path::PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let lock = FIXTURE_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("rubash-2611-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fixture dir");
        std::fs::create_dir_all(dir.join("xdir2611")).expect("fixture subdir");
        let old_cwd = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&dir).expect("chdir fixture");
        Fixture {
            executor: Executor::new(),
            dir,
            old_cwd,
            _lock: lock,
        }
    }

    fn complete(&mut self, line: &str) -> Vec<String> {
        self.executor.complete_line(line, line.len())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.old_cwd);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn compopt_inside_compgen_function_is_legal() {
    let mut fx = Fixture::new("legal");
    run_script(
        &mut fx.executor,
        "_inner2611() { if compopt -o nospace 2>/dev/null; then COMPREPLY=(INNER2611-OK); \
         else COMPREPLY=(INNER2611-ERR); fi; }; \
         _outer2611() { COMPREPLY=( $(compgen -F _inner2611 -- \"$2\") ); }; \
         complete -F _outer2611 cmd2611",
    );
    assert_eq!(
        fx.complete("cmd2611 x"),
        vec!["INNER2611-OK".to_string()],
        "compopt inside the compgen-run function must not error"
    );
}

#[test]
fn compopt_inside_compgen_function_mutation_folds_back() {
    let mut fx = Fixture::new("foldback");
    run_script(
        &mut fx.executor,
        "_inner2611() { compopt -o dirnames 2>/dev/null; COMPREPLY=(); }; \
         _outer2611() { COMPREPLY=( $(compgen -F _inner2611 -- \"$2\") ); }; \
         complete -F _outer2611 cmd2611",
    );
    assert_eq!(
        fx.complete("cmd2611 x"),
        vec!["xdir2611/".to_string()],
        "compopt -o dirnames inside the compgen-run function enables the \
         dirnames fallback for this attempt (directory candidates carry \
         the trailing slash, the engine's insertion-ready convention)"
    );
}

#[test]
fn compopt_outside_any_completion_still_errors() {
    // Guard (subprocess): GNU bash 5.3.20 errors for a bare compopt
    // at top level and for compopt inside a top-level compgen -F
    // function — the binding is a generation-window state, not a
    // permanent one. The in-function rc is observed through a global
    // variable, NOT an echo: compgen captures the function's stdout
    // as its generation channel, so an `echo "in_rc=$?"` inside the
    // function never reaches the script's stdout — verified against
    // GNU bash 5.3.20 itself, whose stdout for the echo form is also
    // just "bare_rc=1" (the rc reads 1 identically on both shells
    // through the variable channel and through stderr).
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "_f2611() { compopt -o nospace 2>/dev/null; IN_RC2611=$?; COMPREPLY=(x); }; \
             compgen -F _f2611 -- x >/dev/null; echo \"in_rc=$IN_RC2611\"; \
             compopt 2>/dev/null; echo \"bare_rc=$?\"",
        )
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("in_rc=1"),
        "top-level compgen -F: compopt must still error, got: {stdout}"
    );
    assert!(
        stdout.contains("bare_rc=1"),
        "bare compopt at top level must still error, got: {stdout}"
    );
}
