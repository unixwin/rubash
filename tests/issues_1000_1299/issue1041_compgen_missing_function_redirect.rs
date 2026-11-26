//! issue#1041 regression: `compgen -F <missing-func>` "function not found"
//! must travel with the builtin's redirected stderr, in order after the
//! `-F` warning — never straight to the session stderr.
//!
//! GNU ownership: builtins/complete.def:669 compgen_builtin drives
//! pcomplete.c:1251 gen_compspec_completions; a missing -F function is
//! pcomplete.c:1058-1066 internal_error (error_prolog family) printed to
//! the builtin's stderr, so `compgen -F f -- "" 2>e.txt >o.txt` leaves
//! o.txt empty and appends BOTH lines to e.txt in warning/not-found order
//! (rc 1). Rubash used to eprintln! the not-found line, bypassing every
//! redirect (e.txt got the warning only) and emitting it before the
//! buffered warning.
//!
//! Oracle: GNU bash 5.3.20 — r1.sh `compgen -F _nosuchfunc_xyz -- ""
//! 2>e.txt >o.txt` gives empty o.txt, e.txt with both
//! `r1.sh: line 1: compgen: warning: ...` and
//! `r1.sh: line 1: completion: function ... not found`, rc 1, empty
//! session stderr.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-1041-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run_script(path: &Path) -> (i32, String, String) {
    // Redirect targets inside the script (`2>e.txt`) resolve against the
    // child's working directory, so run from the script's own directory.
    let output = Command::new(shell_bin())
        .arg(path)
        .current_dir(path.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).replace('\r', ""),
        String::from_utf8_lossy(&output.stderr).replace('\r', ""),
    )
}

/// The issue's exact repro through a script file.
#[test]
fn compgen_missing_function_stays_on_redirected_stderr_in_order() {
    let dir = scratch_dir("order");
    let script = dir.join("r1.sh");
    std::fs::write(
        &script,
        "compgen -F _nosuchfunc_xyz -- \"\" 2>e.txt >o.txt; echo \"rc=$?\"\n",
    )
    .expect("write script");
    let (status, stdout, session_stderr) = run_script(&script);
    assert_eq!(status, 0, "script itself exits 0 (rc is echoed)");
    assert_eq!(stdout, "rc=1\n", "compgen fails 1 with no matches");

    let captured = std::fs::read_to_string(dir.join("e.txt")).expect("e.txt");
    let printed = std::fs::read_to_string(dir.join("o.txt")).expect("o.txt");
    assert!(
        printed.is_empty(),
        "stdout redirect stays empty, got {printed:?}"
    );
    let script_name = rubash::executor::shell_pwd_display_path(&script.to_string_lossy());
    let expected = format!(
        "{script_name}: line 1: compgen: warning: -F option may not work as you expect\n\
         {script_name}: line 1: completion: function `_nosuchfunc_xyz' not found\n"
    );
    assert_eq!(captured, expected, "warning then not-found, both captured");
    assert!(
        session_stderr.is_empty(),
        "nothing may leak to the session stderr, got {session_stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
