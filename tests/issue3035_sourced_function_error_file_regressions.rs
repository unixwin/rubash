//! rubash#3035: an error raised inside a function defined in a
//! sourced file reported the TOP-LEVEL script's filename (the line
//! was already the definition file's line). GNU reports the file
//! that defines the function.
//!
//! GNU ground truth (bash 5.3.20, verified by script-file probes):
//! the diagnostic prolog name is `get_name_for_error` (error.c:90-120)
//! = BASH_SOURCE[0]; a call frame's BASH_SOURCE entry is the
//! function's DEFINITION file (execute_cmd.c:5310-5313
//! execute_function pushes `shell_fn->source_file`, recorded at
//! definition time by make_cmd.c:746-771 make_function_def from
//! BASH_SOURCE[0]). So the name follows the definition file no
//! matter who calls the function, while a function defined in the
//! top-level script itself keeps the script's name.
//!
//! Rubash's frame stack already carried the right values
//! (`${BASH_SOURCE[0]}` inside the function read `./errlib.sh`);
//! the divergence was the builtin-layer private `diagnostic_prefix`
//! copies (test/ulimit/trap/cd/...), which paired the per-frame
//! `__RUBASH_CURRENT_LINE` with the top-level-only
//! `__RUBASH_SCRIPT_NAME`.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, files: &[(&str, &str)], driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3035-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    for (name, body) in files {
        let mut file = std::fs::File::create(dir.join(name)).expect("write fixture");
        file.write_all(body.as_bytes()).expect("write fixture");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(driver)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const ERRLIB: &str = "myfunc() {\n  local c=1\n  while [ $c -lt $cword ]; do\n    c=2\n  done\n  echo \"myfunc body ran\"\n}\n";

const ERRLIB2: &str =
    "utest() { ulimit -Z; }\nctest() { cd /nonexistent-rubash-3035-dir; }\nttest() { trap -Z; }\n";

/// The issue seed: source-then-call a function whose body errors;
/// the stderr filename must be the sourced file.
#[test]
fn sourced_function_test_error_reports_definition_file() {
    let (stdout, stderr, rc) = run_driver(
        "err",
        &[
            ("errlib.sh", ERRLIB),
            ("isolate-err.sh", "source ./errlib.sh\ncword=\"\"\nmyfunc\n"),
        ],
        "isolate-err.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "myfunc body ran\n");
    assert_eq!(
        stderr, "./errlib.sh: line 3: [: 1: unary operator expected\n",
        "GNU names the definition file, not the top-level script"
    );
}

/// The frame name is the definition file even when the caller is a
/// function defined in the top-level script (the caller chain does
/// not matter — execute_cmd.c pushes the DEFINITION file).
#[test]
fn sourced_function_error_via_caller_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_driver(
        "wrap",
        &[
            ("errlib.sh", ERRLIB),
            (
                "isolate-wrap.sh",
                "source ./errlib.sh\ncword=\"\"\nwrapper() { myfunc; }\nwrapper\n",
            ),
        ],
        "isolate-wrap.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "myfunc body ran\n");
    assert_eq!(
        stderr, "./errlib.sh: line 3: [: 1: unary operator expected\n",
        "GNU names the definition file even through a caller function"
    );
}

/// Control: a function defined in the top-level script itself
/// reports the script's own name (no over-correction).
#[test]
fn top_level_function_error_reports_own_file() {
    let (_stdout, stderr, rc) = run_driver(
        "self",
        &[(
            "isolate-self.sh",
            "selffunc() {\n  local c=1\n  while [ $c -lt $cword ]; do\n    c=2\n  done\n}\ncword=\"\"\nselffunc\n",
        )],
        "isolate-self.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(
        stderr, "isolate-self.sh: line 3: [: 1: unary operator expected\n",
        "a top-level-defined function keeps the script name"
    );
}

/// The same name source feeds the whole builtin_error family:
/// ulimit inside a sourced-file function names the sourced file.
#[test]
fn sourced_function_ulimit_error_reports_definition_file() {
    let (_stdout, stderr, _rc) = run_driver(
        "ulimit",
        &[
            ("errlib2.sh", ERRLIB2),
            ("d-ulimit.sh", "source ./errlib2.sh\nutest\n"),
        ],
        "d-ulimit.sh",
    );
    assert_eq!(
        stderr,
        "./errlib2.sh: line 1: ulimit: -Z: invalid option\nulimit: usage: ulimit [-SHabcdefiklmnpqrstuvxPRT] [limit]\n"
    );
}

/// cd inside a sourced-file function names the sourced file.
#[test]
fn sourced_function_cd_error_reports_definition_file() {
    let (_stdout, stderr, _rc) = run_driver(
        "cd",
        &[
            ("errlib2.sh", ERRLIB2),
            ("d-cd.sh", "source ./errlib2.sh\nctest\n"),
        ],
        "d-cd.sh",
    );
    assert_eq!(
        stderr,
        "./errlib2.sh: line 2: cd: /nonexistent-rubash-3035-dir: No such file or directory\n"
    );
}

/// trap inside a sourced-file function names the sourced file.
#[test]
fn sourced_function_trap_error_reports_definition_file() {
    let (_stdout, stderr, _rc) = run_driver(
        "trap",
        &[
            ("errlib2.sh", ERRLIB2),
            ("d-trap.sh", "source ./errlib2.sh\nttest\n"),
        ],
        "d-trap.sh",
    );
    assert_eq!(
        stderr,
        "./errlib2.sh: line 3: trap: -Z: invalid option\ntrap: usage: trap [-Plp] [[action] signal_spec ...]\n"
    );
}
