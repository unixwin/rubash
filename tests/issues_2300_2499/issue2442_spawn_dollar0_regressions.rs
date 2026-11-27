//! rubash#2442 — non-foreground spawn routes passed the anchored
//! absolute path as the execve filename, so a script child's `$0`
//! diverged from GNU on every route except the plain foreground
//! spawn fixed for #2378.
//!
//! GNU contract: there is one resolution point per disk command —
//! execute_cmd.c:5810 `search_for_command` inside
//! execute_disk_command — and shell_execve (execute_cmd.c:6133)
//! hands its result to execve unchanged for every spawn shape.
//! findcmd.c:553 `find_in_path_element` builds that result with
//! `sh_makepath (xpath, name, 0)`: the PATH element as written,
//! joined verbatim; a name carrying a slash is never searched
//! (findcmd.c:385-387) and is its own filename. Pipeline members
//! reach the same funnel (execute_cmd.c:2620 routes each element
//! through execute_simple_command), as do command-substitution
//! children (subst.c:7143), background children and coprocs. The
//! kernel hands the execve filename to a `#!` interpreter as the
//! script pathname, which is what the interpreter reports as `$0`.
//!
//! The one deliberate exception is the `exec` builtin: exec.def:151
//! resolves through search_for_command but exec.def:172 rebinds the
//! target with `full_pathname` before shell_execve (exec.def:231),
//! so a script run through `exec` reports the ABSOLUTE path — the
//! last pin below holds that divergence open on purpose.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle) with
//! the exact fixture below. On master, every line except the plain
//! foreground control and the `exec` exception printed the anchored
//! `/…/bin/zero` form.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn run_fixture() -> (String, String, Option<i32>, PathBuf) {
    let root = std::env::temp_dir().join(format!("issue2442_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("bin")).expect("create fixture bin dir");
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture dir");
    let zero_path: PathBuf = root.join("bin").join("zero");
    std::fs::write(&zero_path, "#!/bin/sh\necho \"zero=$0\"\n").expect("write zero helper");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&zero_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod zero helper");
    }
    let script_path: PathBuf = root.join("pin2442.sh");
    std::fs::write(&script_path, SCRIPT).expect("write probe script");
    let child = Command::new(RUBASH)
        .arg("pin2442.sh")
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let code = out.status.code();
    let _ = std::fs::remove_dir_all(&root);
    (stdout, stderr, code, root)
}

const SCRIPT: &str = "\
bin/zero
echo p | bin/zero
export PATH=\"bin:$PATH\"
echo p | zero
bin/zero </dev/null | cat
x=$(bin/zero); echo \"comsub: $x\"
bin/zero & wait
env bin/zero
env zero
coproc bin/zero; read -r line <&${COPROC[0]}; echo \"coproc: $line\"
( exec bin/zero )
";

#[test]
fn issue2442_every_spawn_route_reports_the_as_written_zero() {
    let (stdout, stderr, code, root) = run_fixture();
    let expected = format!(
        "zero=bin/zero\n\
         zero=bin/zero\n\
         zero=bin/zero\n\
         zero=bin/zero\n\
         comsub: zero=bin/zero\n\
         zero=bin/zero\n\
         zero=bin/zero\n\
         zero=bin/zero\n\
         coproc: zero=bin/zero\n\
         zero={}/bin/zero\n",
        root.display()
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, expected, "stderr: {stderr}");
}
