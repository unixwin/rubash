//! Regression pins for rubash#2345 (audit root cause RC2): a script
//! launched from the CLI shell — `"$TESTSH" child.sh`, or a
//! shebang-less `./child.sh` re-entered after ENOEXEC — must run as
//! a REAL child process, exactly like GNU: distinct real pids per
//! layer (variables.c:546 — `$$` is the process's own pid), a
//! signal domain of its own (a self-addressed SIGKILL kills only
//! the child; the parent observes 137), and a trap domain of its
//! own (the parent's traps stay installed and deferred across the
//! child's lifetime — GNU execute_cmd.c:6237-6260 runs the ENOEXEC
//! re-entry, and 6139 shell_execve the script exec, inside the
//! FORKED child, never in the parent's image).
//!
//! The library/Session embedding path keeps running scripts
//! in-process by design (scope gate); these pins drive the built
//! CLI binary, whose executor is rooted and opted in.
//!
//! Probe shapes descend from the #2345 audit seeds
//! (audit-150/ch1-trap-exit-signal a9/a1 and the m6 morphs); the
//! expected bytes were recorded against the GNU bash 5.3.20 oracle.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2345-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str, executable: bool) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).expect("write script");
    if executable {
        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod +x");
    }
    path
}

/// Run `script` under the built CLI with TESTSH pointing at the
/// same binary (the audit-seed addressing form), in `dir`.
fn run_script(dir: &Path, script: &Path) -> std::process::Output {
    Command::new(bin())
        .arg(script)
        .env("TESTSH", bin())
        .current_dir(dir)
        .output()
        .expect("run script")
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Face (a): three nesting layers report three distinct pids, each
/// layer's $BASHPID equals its $$, and the innermost layer's $$ is
/// its REAL pid (witnessed by a foreign child's $PPID).
#[test]
fn nested_shell_children_have_distinct_real_pids() {
    let dir = temp_dir("pids");
    write(
        &dir,
        "inner.sh",
        "echo \"inner pid=$$ bashpid=$BASHPID\"\necho \"inner realpid=$(/bin/sh -c 'echo $PPID')\"\n",
        false,
    );
    write(
        &dir,
        "mid.sh",
        "echo \"mid pid=$$ bashpid=$BASHPID\"\n\"$TESTSH\" inner.sh\n",
        false,
    );
    let outer = write(
        &dir,
        "outer.sh",
        "echo \"outer pid=$$ bashpid=$BASHPID\"\n\"$TESTSH\" mid.sh\n",
        false,
    );
    let output = run_script(&dir, &outer);
    assert!(
        output.status.success(),
        "runner failed: {}",
        stderr_of(&output)
    );
    let text = stdout_of(&output);
    let field = |line_no: usize, key: &str| -> i64 {
        let line = text
            .lines()
            .nth(line_no)
            .unwrap_or_else(|| panic!("missing line {line_no} in {text:?}"));
        line.split_whitespace()
            .find_map(|tok| tok.strip_prefix(key))
            .unwrap_or_else(|| panic!("missing {key} in {line:?}"))
            .parse()
            .expect("pid parses")
    };
    let outer_pid = field(0, "pid=");
    let outer_bashpid = field(0, "bashpid=");
    let mid_pid = field(1, "pid=");
    let mid_bashpid = field(1, "bashpid=");
    let inner_pid = field(2, "pid=");
    let inner_bashpid = field(2, "bashpid=");
    let inner_realpid = field(3, "realpid=");
    assert_eq!(outer_bashpid, outer_pid, "$$/BASHPID at outer: {text}");
    assert_eq!(mid_bashpid, mid_pid, "$$/BASHPID at mid: {text}");
    assert_eq!(inner_bashpid, inner_pid, "$$/BASHPID at inner: {text}");
    assert_ne!(outer_pid, mid_pid, "layers share a pid: {text}");
    assert_ne!(mid_pid, inner_pid, "layers share a pid: {text}");
    assert_ne!(outer_pid, inner_pid, "layers share a pid: {text}");
    assert_eq!(
        inner_realpid, inner_pid,
        "innermost $$ is not its real pid: {text}"
    );
}

/// Face (b): `kill -KILL $$` in a nested script kills only the
/// child. The wrapper observes 137 and continues; the outer shell
/// receives the child's death notice on its stderr and exits 0.
/// GNU-recorded masked bytes (a9 seed): the notice is
/// `wrapper.sh: line N: N Killed` + 21 spaces + the command text
/// (jobs.c pads the termination description field).
#[test]
fn nested_self_kill_only_kills_the_child() {
    let dir = temp_dir("kill");
    write(&dir, "die.sh", "kill -KILL $$\n", false);
    write(
        &dir,
        "wrapper.sh",
        "\"$TESTSH\" die.sh\necho \"rc=$?\"\n",
        false,
    );
    let outer = write(
        &dir,
        "outer.sh",
        "\"$TESTSH\" wrapper.sh 2>notice.txt\necho \"outer-rc=$?\"\necho notice-masked:\nsed 's/[0-9][0-9]*/N/g' notice.txt\n",
        false,
    );
    let output = run_script(&dir, &outer);
    assert!(
        output.status.success(),
        "outer shell died with the child: status {:?}, stderr {}",
        output.status,
        stderr_of(&output)
    );
    let expected = "rc=137\nouter-rc=0\nnotice-masked:\nwrapper.sh: line N: N Killed                     \"$TESTSH\" die.sh\n";
    assert_eq!(stdout_of(&output), expected);
}

/// Face (c): USR1 sent to the outer shell's pid while a nested
/// child runs must NOT kill the process with the default
/// disposition: GNU defers the parent's trap until the foreground
/// child exits, then runs the handler with the child's $? visible
/// (a1 seed: the nested child exits 7).
#[test]
fn parent_usr1_trap_deferred_until_nested_child_exits() {
    let dir = temp_dir("trap");
    write(&dir, "child.sh", "sleep 0.6\nexit 7\n", false);
    let outer = write(
        &dir,
        "outer.sh",
        "trap 'echo \"H st=$?\"' USR1\n( sleep 0.25; kill -USR1 $$ ) &\n\"$TESTSH\" child.sh\necho \"after rc=$?\"\n",
        false,
    );
    let output = run_script(&dir, &outer);
    assert!(
        output.status.success(),
        "process died by USR1 (trap blackout): status {:?}, stderr {}",
        output.status,
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&output), "H st=7\nafter rc=7\n");
}

/// Face (b) on the ENOEXEC arm: a shebang-less `./die.sh` whose
/// `kill -KILL $$` targets only its own process — the outer shell
/// prints the death notice and survives with $? = 137.
#[test]
fn enoexec_self_kill_only_kills_the_child() {
    let dir = temp_dir("enoexec-kill");
    let outer = write(
        &dir,
        "outer.sh",
        "cat > die.sh <<'DIE'\nkill -KILL $$\nDIE\nchmod +x die.sh\n./die.sh\necho \"outer-survived rc=$?\"\n",
        false,
    );
    let output = run_script(&dir, &outer);
    assert!(
        output.status.success(),
        "outer shell died with the ENOEXEC child: status {:?}",
        output.status
    );
    assert_eq!(stdout_of(&output), "outer-survived rc=137\n");
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("Killed") && stderr.contains("./die.sh"),
        "missing child death notice on outer stderr: {stderr:?}"
    );
}

/// Face (c) on the ENOEXEC arm: the parent's USR1 trap is deferred
/// across a shebang-less child's lifetime and delivered afterwards
/// with the child's status — not swallowed by the re-entry.
/// GNU-recorded bytes for this shape: `H st=7` then `after rc=7`.
#[test]
fn enoexec_usr1_trap_deferred_until_child_exits() {
    let dir = temp_dir("enoexec-trap");
    let outer = write(
        &dir,
        "outer.sh",
        "cat > child.sh <<'CHILD'\nsleep 0.6\nexit 7\nCHILD\nchmod +x child.sh\ntrap 'echo \"H st=$?\"' USR1\n( sleep 0.25; kill -USR1 $$ ) &\n./child.sh\necho \"after rc=$?\"\n",
        false,
    );
    let output = run_script(&dir, &outer);
    assert!(
        output.status.success(),
        "status {:?}, stderr {}",
        output.status,
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&output), "H st=7\nafter rc=7\n");
}

/// The retained exception: when the parent shell itself is
/// executing from its stdin stream, a same-shell child continues
/// that stream (GNU's input-line contract) — the real spawn is
/// declined and the child reads the next line of the parent's
/// input. Pins that the #2345 retarget did not sever the stream.
#[test]
fn stdin_stream_child_still_reads_the_parents_stream() {
    let dir = temp_dir("stdin-stream");
    write(
        &dir,
        "reader.sh",
        "read x\necho \"reader-got: $x\"\n",
        false,
    );
    let payload = write(
        &dir,
        "payload.txt",
        "\"$TESTSH\" reader.sh\npayload-for-child\necho outer-after\n",
        false,
    );
    let stdin_file = fs::File::open(&payload).expect("open payload");
    let output = Command::new(bin())
        .env("TESTSH", bin())
        .current_dir(&dir)
        .stdin(Stdio::from(stdin_file))
        .output()
        .expect("run stdin driver");
    assert!(
        output.status.success(),
        "status {:?}, stderr {}",
        output.status,
        stderr_of(&output)
    );
    assert_eq!(
        stdout_of(&output),
        "reader-got: payload-for-child\nouter-after\n"
    );
}
