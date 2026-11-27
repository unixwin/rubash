//! rubash#2543 regressions: a respawned async child re-defined every
//! function in the parent's table through `background_function_prelude`
//! while the `__RUBASH_READONLY_FUNCTIONS` attribute transport had
//! already landed in the child's env at startup (init.rs), so every
//! readonly-marked function tripped the `readonly function' guard at its
//! OWN inheritance definition — a `name: readonly function' noise wall
//! (modernish: 40 lines per async respawn) and, worse, the define was
//! refused, dropping the function from the child entirely.
//!
//! GNU contract (Bash 5.3.20 oracle, script-file probes):
//! - `execute_intern_function` (execute_cmd.c:6325-6331) raises
//!   `readonly function' only when `find_function' returns an EXISTING
//!   function that is readonly — the guard protects a REdefinition,
//!   never a first definition.
//! - A forked async child (`execute_in_subshell` execute_cmd.c:1761-1763
//!   -> make_child) inherits the whole function table by memory copy and
//!   defines NOTHING: `{ h; } &` with h readonly prints h's output,
//!   silently.
//! - A redefinition of an inherited readonly function inside the child
//!   body still errors once and keeps the original body.
//! - Env-imported functions (variables.c:397-446
//!   `initialize_shell_variables`, the parse_and_execute SEVAL_FUNCDEF
//!   call at :418) are defined exactly once at startup; GNU never
//!   re-renders them into a child.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled WSL oracle).

use std::process::Command;

fn rubash_file_env(script: &str, env: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2543-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .envs(env.iter().copied())
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    rubash_file_env(script, &[])
}

#[test]
fn issue2543_minimal_exported_readonly_function_async_respawn_is_silent() {
    // The minimized modernish shape: an exported readonly function used
    // by an async compound child. GNU 5.3.20: stdout "done", empty
    // stderr, rc 0. Pre-fix rubash printed `f: readonly function' AND
    // `f: command not found' (the inheritance define was refused).
    let (stdout, stderr, code) =
        rubash_file("f(){ :; }\nreadonly -f f\nexport -f f\n{ f; } &\nwait\necho done\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "done\n");
}

#[test]
fn issue2543_readonly_function_inherited_by_async_child_works() {
    // GNU fork: the child keeps the function (readonly attribute
    // included) and it RUNS. `{ h; } &` prints H, silently.
    let (stdout, stderr, code) =
        rubash_file("h(){ echo H; }\nreadonly -f h\n{ h; } &\nwait\necho done-inherit\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "H\ndone-inherit\n");
}

#[test]
fn issue2543_child_body_redefinition_of_readonly_function_still_errors() {
    // The guard must keep protecting an EXISTING readonly function: a
    // redefinition inside the child body errors exactly once (GNU
    // execute_cmd.c:6325) and the original body survives — the child
    // then calls k and gets ORIG, and the parent is unaffected.
    // GNU 5.3.20: stdout "ORIG\nparent:\nORIG\nend\n", stderr one
    // "line 3: k: readonly function" line.
    let (stdout, stderr, code) = rubash_file(
        "k(){ echo ORIG; }\nreadonly -f k\n{ k(){ echo NEW; }; k; } &\nwait\necho parent:\nk\necho end\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ORIG\nparent:\nORIG\nend\n");
    assert_eq!(stderr.lines().count(), 1);
    assert!(
        stderr.contains("k: readonly function"),
        "expected the one readonly-function diagnostic, got: {stderr}"
    );
}

#[test]
fn issue2543_parent_redefinition_of_readonly_function_still_errors() {
    // The parent-shell behavior is unchanged: redefining an existing
    // readonly function at top level errors (GNU execute_cmd.c:6325).
    let (stdout, stderr, code) = rubash_file("f(){ :; }\nreadonly -f f\nf(){ :; }\necho rc=$?\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr.lines().count(), 1);
    assert!(stderr.contains("f: readonly function"));
    assert_eq!(code, Some(0));
}

#[test]
fn issue2543_nonreadonly_functions_unaffected_by_async_respawn() {
    // Non-readonly functions (exported and plain) keep working through
    // the prelude exactly as before the fix.
    let (stdout, stderr, code) =
        rubash_file("a(){ echo A$1; }\nb(){ echo B$1; }\nexport -f b\n{ a 1; b 2; } &\nwait\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "A1\nB2\n");
}

#[test]
fn issue2543_env_carried_function_not_redefined_by_prelude() {
    // The exported-function env transport: rubash launched by a shell
    // that exported g (BASH_FUNC_g%% rides the process env into every
    // respawn child). The child's startup import defines g ONCE
    // (variables.c:418); the prelude must not re-render it — with g
    // additionally readonly-marked, a prelude re-definition would be a
    // REdefinition of an existing readonly function and error.
    // GNU 5.3.20 (`BASH_FUNC_g%%=() {  echo ENVG; } bash probe.sh`):
    // stdout "ENVG\nend\n", empty stderr, rc 0.
    let (stdout, stderr, code) = rubash_file_env(
        "readonly -f g\n{ g; } &\nwait\necho end\n",
        &[("BASH_FUNC_g%%", "() {  echo ENVG; }")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "ENVG\nend\n");
}

#[test]
fn issue2543_env_carried_stale_carrier_loses_to_fresh_prelude_rendering() {
    // If the parent REDEFINED the function after importing it, the env
    // carrier is stale: the prelude's fresh rendering must win in the
    // child (GNU's fork carries the current body). The child prints
    // NEWBODY (from the prelude), not ENVOLD (from the stale carrier),
    // and stays silent: the prelude define of a not-yet-existing
    // function is the inheritance, and the child's env import ran first
    // so the prelude define here is an ordinary redefinition of a
    // non-readonly function, which GNU also admits silently.
    let (stdout, stderr, code) = rubash_file_env(
        "g(){ echo NEWBODY; }\n{ g; } &\nwait\necho end\n",
        &[("BASH_FUNC_g%%", "() {  echo ENVOLD; }")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "NEWBODY\nend\n");
}
