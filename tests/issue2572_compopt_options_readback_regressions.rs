//! rubash#2572 — `compopt` inside a completion function mutated a
//! throwaway copy: `completion_dynamic_for_line` cloned the governing
//! spec into `pcomp_curcs` (where the bare compopt form correctly
//! mutated it), then restored the saved spec and dropped the clone,
//! so consumers read the REGISTERED options and the mutation had no
//! effect — `compopt +o default` did not suppress the filename
//! fallback, `compopt -o dirnames` did not enable it.
//!
//! GNU ownership: pcomplete.c gen_progcomp_completions copies the
//! compspec (`compspec_copy`), points pcomp_curcs at the copy during
//! generation (pcomp_set_compspec_options, pcomplete.c:1481-1490,
//! mutates the copy's options only), and folds the result back into
//! the caller (`*foundp |= cs->options`) — bashline.c then calls
//! pcomp_set_readline_variables with the USED spec's options including
//! the mutations. The port reads the mutated copy's options into
//! `CompgenDynamic::options_override` before the restore, and the
//! fallback block consumes the effective options.
//!
//! Candidate spelling: directory candidates carry the trailing
//! slash, the engine's insertion-ready convention across the whole
//! completion pin family (see issue2570_compspec_fallback FILENAMES/
//! DIRS). Two expectations here originally omitted the slash and
//! failed on unmodified master; captain-adjudicated wrong-pin
//! 2026-11-05 (GNU appends the slash at line-insert time,
//! lib/readline/complete.c insert_match; rubash candidates are
//! insertion-ready strings).

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

/// Fixture directory (file zzfile.txt, dir zzdirone/) as the session
/// cwd, with the three compopt-shaping completion functions defined.
fn fixture(tag: &str) -> Fixture {
    let lock = FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = std::env::temp_dir().join(format!("rubash-2572-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("zzdirone")).expect("fixture dir");
    std::fs::write(dir.join("zzfile.txt"), "a\n").expect("fixture file");
    let old_cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&dir).expect("enter fixture");
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        &format!(
            "cd {}\n_off() {{ compopt +o default; COMPREPLY=(); }}\n_dir() {{ compopt -o dirnames; COMPREPLY=(); }}\n_nodir() {{ compopt +o dirnames; COMPREPLY=(); }}\n_plain() {{ COMPREPLY=(); }}",
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

#[test]
fn compopt_plus_o_default_suppresses_the_filename_fallback() {
    let mut fx = fixture("off");
    // Control: without compopt, -o default fills the empty result
    // with the fixture's filename candidates.
    let got = fx.complete("complete -o default -F _plain mycmd", "mycmd zz");
    assert_eq!(
        got,
        vec!["zzdirone/".to_string(), "zzfile.txt".to_string()],
        "control: the registered -o default fallback fires"
    );
    // With compopt +o default inside the function, the fallback is off.
    let got = fx.complete("complete -o default -F _off mycmd", "mycmd zz");
    assert_eq!(
        got,
        Vec::<String>::new(),
        "compopt +o default must suppress the filename fallback"
    );
}

#[test]
fn compopt_minus_o_dirnames_enables_the_dirnames_fallback() {
    let mut fx = fixture("dir");
    let got = fx.complete("complete -F _dir mycmd", "mycmd zz");
    assert_eq!(
        got,
        vec!["zzdirone/".to_string()],
        "compopt -o dirnames must switch the dirnames fallback on for this attempt"
    );
}

#[test]
fn compopt_plus_o_dirnames_removes_a_registered_dirnames() {
    let mut fx = fixture("nodir");
    let got = fx.complete("complete -o dirnames -F _nodir mycmd", "mycmd zz");
    assert_eq!(
        got,
        Vec::<String>::new(),
        "compopt +o dirnames must switch a registered dirnames fallback off"
    );
}
