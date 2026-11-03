//! rubash#2723 — environment import de-exported every lowercase-first
//! variable name for executors constructed inside a shared process
//! (and dropped single-character names on every path): a parent-set
//! `http_proxy` / `my_var` / `x=1` was visible to the shell itself but
//! invisible to its children.
//!
//! GNU ground: variables.c:368 initialize_shell_variables binds every
//! imported environment entry with att_exported|att_imported
//! (variables.c:513-516) — the environment's exported-ness is imported
//! as-is, whatever the name's case or length; invalid names ride the
//! export environment via bind_invalid_envvar (variables.c:3307).
//! The old uppercase-first heuristic guarded against sibling executors'
//! shell-local mirror writes accumulating in a shared process
//! environment, but the mirror (set_process_env) writes only a fixed
//! uppercase/internal whitelist, so no lowercase name in the
//! environment can be a mirror write.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2723-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_with_env(dir: &Path, script: &str) -> String {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env("http_proxy", "http://proxy:8080")
        .env("my_var", "hello")
        .env("_myvar", "under")
        .env("foo", "bar")
        .env("x", "1")
        .env("RUBASH_BIN", RUBASH)
        .output()
        .expect("run rubash");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "",
        "stderr for {script:?}"
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn issue2723_imported_names_keep_their_exported_attribute() {
    let dir = scratch("marks");
    let out = run_with_env(&dir, "declare -p http_proxy my_var _myvar foo x\n");
    assert_eq!(
        out,
        "declare -x http_proxy=\"http://proxy:8080\"\n\
         declare -x my_var=\"hello\"\n\
         declare -x _myvar=\"under\"\n\
         declare -x foo=\"bar\"\n\
         declare -x x=\"1\"\n"
    );
}

#[test]
fn issue2723_imported_names_reach_child_processes() {
    let dir = scratch("child");
    // The child is a fresh rubash process whose only view of the names
    // is the environment the parent built from its export marks.
    let out = run_with_env(
        &dir,
        "\"$RUBASH_BIN\" -c 'echo \"child:$http_proxy|$my_var|$_myvar|$foo|$x\"'\n",
    );
    assert_eq!(out, "child:http://proxy:8080|hello|under|bar|1\n");
}
