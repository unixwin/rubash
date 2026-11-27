//! rubash#2378 — a closed stdout on a FOREIGN (external) spawn
//! reported `write error: Bad file descriptor` and status 1 where GNU
//! exits silently with the child's own status.
//!
//! GNU contract: redir.c r_close_this leaves the fd closed in the
//! child; the child's write fails EBADF inside the child and the shell
//! reports nothing. builtins/common.c:320-334 sh_chkwrite folds a
//! failed flush into a status only for BUILTINS, whose writes the
//! shell itself performs. rubash's spawn planner substitutes a
//! capture pipe for a closed fd (external_output_fds_closed) and the
//! parent replays the drained bytes through the ordered redirect
//! walk — the replay manufactured the builtin verdict for the foreign
//! command (and dropped the child's captured stderr on the way).
//! The fix routes the post-external drain in foreign-drain mode
//! (OutputFdState::foreign_drain): a Closed target discards the
//! drained bytes silently, the sibling stream still routes, and the
//! child's exit status stands. The builtin route keeps the verdict
//! (c5 below pins it).
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle) with
//! the exact fixture below (cluster probes/pins/fx2378). The observer
//! is a real foreign child (#!/bin/sh) that attempts its writes,
//! reports its own printf failures on stderr, and exits 0 — never an
//! ENOEXEC script (those run in-process; the c_external_... pin in
//! cli_tests/fd_redirects.rs owns that different shape and its
//! script-internal builtin diagnostic). On master: c1/c4 report the
//! manufactured diagnostic with rc=1 and lose the child's stderr.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn run_fixture() -> (String, String, Option<i32>) {
    let root = std::env::temp_dir().join(format!("issue2378_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create fixture dir");
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture dir");
    let emit_path: PathBuf = root.join("emit");
    std::fs::write(
        &emit_path,
        "#!/bin/sh\nprintf 'hello-out\\n'\nprintf 'hello-err\\n' >&2\nexit 0\n",
    )
    .expect("write emit helper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&emit_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod emit helper");
    }
    let script_path: PathBuf = root.join("pin2378.sh");
    std::fs::write(&script_path, SCRIPT).expect("write probe script");
    let child = Command::new(RUBASH)
        .arg("pin2378.sh")
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_dir_all(&root);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

const SCRIPT: &str = "\
PATH=\".:$PATH\"
emit >&-; echo \"c1 rc=$?\"
emit 2>&-; echo \"c2 rc=$?\"
emit; echo \"c3 rc=$?\"
x=$(emit >&-); echo \"c4 rc=$? x=[$x]\"
echo builtin >&-; echo \"c5 rc=$?\"
exec 1>&-
emit; echo \"c6 rc=$?\" >&2
";

#[test]
fn issue2378_foreign_closed_stdout_is_silent() {
    let (stdout, stderr, code) = run_fixture();
    assert_eq!(
        stdout,
        "c1 rc=0\n\
         hello-out\n\
         c2 rc=0\n\
         hello-out\n\
         c3 rc=0\n\
         c4 rc=0 x=[]\n\
         c5 rc=1\n"
    );
    assert_eq!(
        stderr,
        "./emit: 2: printf: printf: I/O error\n\
         hello-err\n\
         hello-err\n\
         ./emit: 2: printf: printf: I/O error\n\
         hello-err\n\
         pin2378.sh: line 6: echo: write error: Bad file descriptor\n\
         ./emit: 2: printf: printf: I/O error\n\
         hello-err\n\
         c6 rc=0\n"
    );
    assert_eq!(code, Some(0));
}
