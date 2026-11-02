//! rubash#2586 — `__RUBASH_TOP_LEVEL_NAME` and `__RUBASH_GETOPTS_OPTIND`
//! were missing from the init strip list and the respawn transport,
//! while their sibling keys (`__RUBASH_SCRIPT_NAME`,
//! `__RUBASH_GETOPTS_OFFSET`) were present on both surfaces:
//!
//! - A fresh executor inheriting an ambient `__RUBASH_TOP_LEVEL_NAME`
//!   never installed its own (the source builtin's writer is
//!   install-if-absent), so BASH_SOURCE consumers reported the PARENT's
//!   script identity. GNU derives $0/BASH_SOURCE from the shell's own
//!   argv at entry (shell.c), never from the environment.
//! - A fresh executor inheriting an ambient `__RUBASH_GETOPTS_OPTIND`
//!   (the readonly-OPTIND shadow counter, getopts_enable.rs) resumed a
//!   foreign scan position and replayed/skipped getopts arguments. GNU
//!   keeps the scan position in the shell-internal `sh_optind`
//!   (builtins/getopts.def); a new shell starts at OPTIND=1.
//!
//! The fix strips both keys at fresh construction (env map + process
//! mirror for TOP_LEVEL_NAME) and carries both across respawns with
//! their siblings (transport key list + the explicit script-identity
//! inserts).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script_with_env(
    script: &str,
    extra_files: &[(&str, &str)],
    envs: &[(&str, &str)],
) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2586-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path: PathBuf = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    for (name, body) in extra_files {
        std::fs::write(dir.join(name), body).expect("write extra file");
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(path.to_str().expect("utf8 path"))
        .current_dir(&dir);
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn issue_2586_ambient_getopts_shadow_does_not_skip_arguments() {
    // Planted shadow counter 3 would make a readonly-OPTIND getopts loop
    // resume past both options. A fresh shell must start the scan at 1.
    let script =
        "readonly OPTIND=1\nset -- -a -b\nwhile getopts \"ab\" o; do echo \"got $o\"; done\n";
    let (stdout, stderr, _code) =
        run_script_with_env(script, &[], &[("__RUBASH_GETOPTS_OPTIND", "3")]);
    assert_eq!(stdout, "got a\ngot b\n", "stderr: {stderr}");
}

#[test]
fn issue_2586_ambient_top_level_name_does_not_hijack_source_identity() {
    // With a planted bashdb-generated top-level name, push_bash_source
    // canonicalizes sourced paths (the debugger path shape). A fresh
    // shell must strip the plant, so BASH_SOURCE[0] inside the sourced
    // file stays exactly the relative spelling the script used.
    let script = "source lib.sh\n";
    let lib = "echo \"src0=${BASH_SOURCE[0]}\"\n";
    let (stdout, stderr, _code) = run_script_with_env(
        script,
        &[("lib.sh", lib)],
        &[("__RUBASH_TOP_LEVEL_NAME", "/planted/bashdb-generated")],
    );
    assert_eq!(stdout, "src0=lib.sh\n", "stderr: {stderr}");
}

#[test]
fn issue_2586_unplanted_source_identity_still_works() {
    // Control: ordinary sourcing reports the sourced file as written.
    let script = "source lib.sh\n";
    let lib = "echo \"src0=${BASH_SOURCE[0]}\"\n";
    let (stdout, stderr, _code) = run_script_with_env(script, &[("lib.sh", lib)], &[]);
    assert_eq!(stdout, "src0=lib.sh\n", "stderr: {stderr}");
}
