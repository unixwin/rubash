//! Issue rubash#2727 regression (deep-audit B5-13): `compgen`
//! ignored the `-o default` / `-o bashdefault` fallbacks —
//! `execute_compgen` ran the dirnames/plusdirs step and stopped, so an
//! empty generated result stayed empty even when the options demanded
//! the fallback sets.
//!
//! GNU ownership: builtins/complete.def `compgen_builtin` — after
//! gen_compspec_completions returns, a still-empty list with
//! COPT_BASHDEFAULT runs `bash_default_completion`, and a still-empty
//! list with COPT_DEFAULT runs readline's filename completion.
//! Bashdefault first: a non-empty bashdefault set suppresses the
//! filename set. Fallback results print bare (no -P/-S wrapping).
//!
//! Oracle: GNU bash 5.3.20 batch probes in a fixture directory
//! (fizz.txt / fizz2.txt):
//! - `compgen -o default -W zzz -- fi` -> fizz.txt fizz2.txt, rc 0
//! - `compgen -o bashdefault -W zzz -- '$HO'` -> $HOME $HOSTNAME
//!   $HOSTTYPE, rc 0
//! - `compgen -o bashdefault -- gi` -> nothing, rc 1 (no command
//!   names in the compgen context)
//! - `compgen -o default -P PRE: -W zzz -- fi` -> bare names (the
//!   prefix is NOT applied to fallback results)

use std::process::Command;

struct Fixture {
    dir: std::path::PathBuf,
}

impl Fixture {
    fn run(&self, script: &str) -> (Vec<String>, Option<i32>) {
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
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("rubash-2727-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture dir");
    std::fs::write(dir.join("fizz2727a.txt"), "a\n").expect("fixture file");
    std::fs::write(dir.join("fizz2727b.txt"), "b\n").expect("fixture file");
    Fixture { dir }
}

#[test]
fn issue_2727_default_falls_back_to_filenames() {
    let fx = fixture("default");
    let (lines, code) = fx.run("compgen -o default -W zzz -- fizz2727");
    assert_eq!(
        lines,
        vec!["fizz2727a.txt".to_string(), "fizz2727b.txt".to_string()]
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue_2727_bashdefault_falls_back_to_variables() {
    let fx = fixture("bashdefault");
    let (lines, code) =
        fx.run("ZEBRA_HOME=1; ZEBRA_HOST=2; compgen -o bashdefault -W zzz -- '$ZEBRA_H'");
    assert_eq!(
        lines,
        vec!["$ZEBRA_HOME".to_string(), "$ZEBRA_HOST".to_string()]
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue_2727_bashdefault_completes_no_command_names() {
    let fx = fixture("cmdnames");
    let (lines, code) = fx.run("compgen -o bashdefault -- gi");
    assert!(lines.is_empty(), "expected no candidates, got {lines:?}");
    assert_eq!(code, Some(1));
}

#[test]
fn issue_2727_bashdefault_empty_falls_through_to_default() {
    // A plain word yields nothing from bashdefault, so with both
    // options set the filename set still fires.
    let fx = fixture("fallthrough");
    let (lines, code) = fx.run("compgen -o default -o bashdefault -W zzz -- fizz2727");
    assert_eq!(
        lines,
        vec!["fizz2727a.txt".to_string(), "fizz2727b.txt".to_string()]
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue_2727_bashdefault_nonempty_suppresses_default() {
    let fx = fixture("suppress");
    let (lines, code) =
        fx.run("ZEBRA_HOME=1; compgen -o default -o bashdefault -W zzz -- '$ZEBRA_H'");
    assert_eq!(lines, vec!["$ZEBRA_HOME".to_string()]);
    assert_eq!(code, Some(0));
}

#[test]
fn issue_2727_fallback_results_print_bare() {
    let fx = fixture("bare");
    let (lines, code) = fx.run("compgen -o default -P 'PRE:' -W zzz -- fizz2727");
    assert_eq!(
        lines,
        vec!["fizz2727a.txt".to_string(), "fizz2727b.txt".to_string()]
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue_2727_no_option_no_fallback() {
    let fx = fixture("control");
    let (lines, code) = fx.run("compgen -W zzz -- fizz2727");
    assert!(lines.is_empty(), "expected no candidates, got {lines:?}");
    assert_eq!(code, Some(1));
}
