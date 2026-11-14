//! rubash#2776 — compgen applied the FIGNORE filter to its own
//! enumeration: with FIGNORE='~', `compgen -f` returned nothing
//! for a sole `*~` file. GNU's compgen never applies FIGNORE —
//! the filter is readline's filename_completion_ignore
//! (bashline.c:3065-3091), installed as
//! rl_ignore_some_completions_function for interactive filename
//! completion attempts only. gen_compspec_completions (compgen's
//! engine) does not consult it. Probed on GNU bash 5.3.20: with
//! FIGNORE='~', `compgen -f -- ''` and `compgen -A file -- ''`
//! list the `~` file in both force_fignore states.
//!
//! The guard cell pins the other side: the interactive candidate
//! path DOES keep the FIGNORE filter (rubash#2576), so the fix
//! must remove the filter from compgen's enumeration without
//! touching the readline-attempt filter.

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

struct Fixture {
    dir: std::path::PathBuf,
    old_cwd: std::path::PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let lock = FIXTURE_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("rubash-2776-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fixture dir");
        std::fs::write(dir.join("only2776~"), "x\n").expect("fixture file");
        std::fs::write(dir.join("plain2776.txt"), "x\n").expect("fixture file");
        let old_cwd = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&dir).expect("chdir fixture");
        Fixture {
            dir,
            old_cwd,
            _lock: lock,
        }
    }

    fn compgen(&self, script: &str) -> (Vec<String>, Option<i32>) {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(script)
            .current_dir(&self.dir)
            .output()
            .expect("run rubash");
        let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect();
        lines.sort();
        (lines, output.status.code())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.old_cwd);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const BOTH: &[&str] = &["only2776~", "plain2776.txt"];

#[test]
fn compgen_file_ignores_fignore() {
    let fx = Fixture::new("compgen");
    for script in [
        "FIGNORE='~'; compgen -f -- ''",
        "FIGNORE='~'; compgen -A file -- ''",
        "FIGNORE='~'; shopt -s force_fignore; compgen -f -- ''",
        "FIGNORE='~'; shopt -u force_fignore; compgen -f -- ''",
    ] {
        let (lines, code) = fx.compgen(script);
        assert_eq!(
            lines,
            BOTH.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "compgen must list the FIGNORE-suffixed file: {script}"
        );
        assert_eq!(code, Some(0), "{script}");
    }
}

#[test]
fn interactive_path_keeps_fignore() {
    let fx = Fixture::new("live");
    let mut executor = Executor::new();
    let script = "FIGNORE='~'";
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "setup failed: {result:?}");
    let line = "cat plain2776";
    let mut candidates = executor.complete_line(line, line.len());
    candidates.sort();
    assert_eq!(
        candidates,
        vec!["plain2776.txt".to_string()],
        "guard: the readline-attempt filter still drops FIGNORE suffixes"
    );
    let line = "cat o";
    let candidates = executor.complete_line(line, line.len());
    assert!(
        !candidates.iter().any(|c| c == "only2776~"),
        "guard: the ~ file stays filtered interactively: {candidates:?}"
    );
    drop(fx);
}
