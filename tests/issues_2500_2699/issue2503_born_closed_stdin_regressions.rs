//! Issue unixwin/rubash#2503: a rubash process BORN with fd 0
//! already closed (or a session that closed it with `exec 0<&-`)
//! read the descriptor as a silent EOF: `read x <&0` failed with
//! rc 1 but NO diagnostic, and the in-process cat exited 0
//! silently. GNU bash 5.3 + coreutils report the descriptor:
//!
//!   read x <&0  -> `read: 0: read error: Bad file descriptor`, rc 1
//!   read y      -> same, rc 1
//!   cat <&0     -> `cat: -: Bad file descriptor` +
//!                  `cat: closing standard input: Bad file descriptor`, rc 1
//!   cat         -> same two lines, rc 1
//!   cat < /dev/null -> rc 0, silent (control)
//!
//! GNU semantics: a dup whose source fd equals its destination is
//! a no-op — redir.c:1151 gates the whole dup application
//! (including the dup2 at redir.c:1193) on redir_fd != redirector —
//! so `<&0` on a closed fd 0 is NOT a redirection failure: the
//! command runs against the closed descriptor and reports it at
//! read time (read.def:911 for read; cat.c's read + close errors
//! for cat). A dup from a DIFFERENT closed fd does fail the
//! redirection (dup2 EBADF) and the command never runs.
//!
//! Root cause of the rubash gap: #2476 built the at-exec
//! descriptor capture (src/fd/unix.rs std_fd_closed_at_exec) but
//! seeded only fd 1 in FdTable::new, so a born-closed fd 0 was
//! seeded as the runtime's sanitized /dev/null stand-in and every
//! consumer saw EOF. The seed now covers fd 0 as well; the read
//! dispatch skips its redirection-failure gate for the self-dup
//! winner so the read-time diagnostic fires; the external stdin
//! wiring hands a self-dup child the closed state instead of a
//! redirection error; and the emulated cat classifies a dup
//! winner whose source is closed as closed stdin.
//!
//! Expectations are the GNU Bash 5.3.20 outputs of the same
//! shapes (oracle binary run side by side in the cluster
//! workdir). The born-closed pins spawn the script directly
//! (kernel shebang exec) with fd 0 closed in pre_exec — a foreign
//! parent, no rubash spawn funnel involved.
#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2503-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

/// Write an executable script whose interpreter is the rubash
/// binary under test; returns its absolute path.
fn write_child(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let script = format!("#!{}\n{body}", env!("CARGO_BIN_EXE_rubash"));
    std::fs::write(&path, script).expect("write child script");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod child script");
    path
}

/// Spawn the script with fd 0 CLOSED at exec (pre_exec runs after
/// stdio setup, so the close is the state the kernel hands the
/// new image). Returns (stdout, stderr, exit code).
fn run_born_closed_stdin(script: &Path) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(script);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(|| {
            if libc::close(0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = cmd.output().expect("run born-closed child");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Run a plain script file under the binary under test with stdin
/// from /dev/null. Returns (stdout, stderr, exit code).
fn run_script_file(script: &Path) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run script");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const FIVE_SHAPES: &str = "read x <&0; echo \"1 read-dup rc=$?\"\n\
                           read y; echo \"2 read rc=$?\"\n\
                           cat <&0; echo \"3 cat-dup rc=$?\"\n\
                           cat; echo \"4 cat rc=$?\"\n\
                           cat < /dev/null; echo \"5 cat-devnull rc=$?\"\n";

const FIVE_SHAPES_STDOUT: &str = "1 read-dup rc=1\n\
                                  2 read rc=1\n\
                                  3 cat-dup rc=1\n\
                                  4 cat rc=1\n\
                                  5 cat-devnull rc=0\n";

fn five_shapes_stderr(path: &Path, first_line: usize) -> String {
    format!(
        "{p}: line {a}: read: 0: read error: Bad file descriptor\n\
         {p}: line {b}: read: 0: read error: Bad file descriptor\n\
         cat: -: Bad file descriptor\n\
         cat: closing standard input: Bad file descriptor\n\
         cat: -: Bad file descriptor\n\
         cat: closing standard input: Bad file descriptor\n",
        p = path.display(),
        a = first_line,
        b = first_line + 1,
    )
}

/// Born with fd 0 closed: every consumer reports the descriptor
/// exactly as GNU does (the read shapes are script lines 2-3,
/// after the shebang line).
#[test]
fn issue2503_born_closed_stdin_five_shapes_match_gnu() {
    let dir = test_dir("born");
    let script = write_child(&dir, "child.sh", FIVE_SHAPES);
    let (out, err, code) = run_born_closed_stdin(&script);
    assert_eq!(out, FIVE_SHAPES_STDOUT);
    assert_eq!(err, five_shapes_stderr(&script, 2));
    assert_eq!(code, Some(0));
}

/// The same five shapes after an in-session `exec 0<&-` produce
/// the same faces (the closed entry, not the birth, governs);
/// the read shapes are script lines 3-4 (shebang, then the exec
/// line, then the shapes).
#[test]
fn issue2503_in_session_closed_stdin_five_shapes_match_gnu() {
    let dir = test_dir("session");
    let script = write_child(&dir, "session.sh", &format!("exec 0<&-\n{FIVE_SHAPES}"));
    let (out, err, code) = run_script_file(&script);
    assert_eq!(out, FIVE_SHAPES_STDOUT);
    assert_eq!(err, five_shapes_stderr(&script, 3));
    assert_eq!(code, Some(0));
}

/// The self-dup carve-out does not widen: a dup from a DIFFERENT
/// closed fd is still a redirection failure (the command never
/// runs), while a self-dup of a closed non-stdio fd is still a
/// silent no-op success (redir.c:1151).
#[test]
fn issue2503_dup_of_closed_source_still_fails_unless_self_dup() {
    let dir = test_dir("dups");
    let script = write_child(
        &dir,
        "dups.sh",
        "exec 3<&-\n\
         cat <&3; echo \"A rc=$?\"\n\
         cat 3<&3; echo \"C rc=$?\"\n",
    );
    let (out, err, code) = run_script_file(&script);
    assert_eq!(out, "A rc=1\nC rc=0\n");
    assert_eq!(
        err,
        format!("{}: line 3: 3: Bad file descriptor\n", script.display())
    );
    assert_eq!(code, Some(0));
}

/// Control: a child born with fd 0 closed whose script never
/// touches stdin is unaffected — no diagnostics, rc 0.
#[test]
fn issue2503_born_closed_stdin_untouched_is_silent() {
    let dir = test_dir("untouched");
    let script = write_child(&dir, "child.sh", "echo hello\n");
    let (out, err, code) = run_born_closed_stdin(&script);
    assert_eq!(out, "hello\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}
