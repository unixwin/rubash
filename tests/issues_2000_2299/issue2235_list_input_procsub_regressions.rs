//! Issue unixwin/rubash#2235: list-level INPUT process substitution
//! on simple external commands.
//!
//! GNU walks ONE ordered redirect list in the forked child
//! (redir.c:237 do_redirections; each entry's word is expanded at
//! its own turn — redirection_expand, redir.c:298 — and a `<(...)`
//! word forks its producer then, process_substitute, subst.c:6362).
//! So a numbered entry `3< <(...)` binds fd 3 for the command no
//! matter which command form carries it, a later `<&3` in the same
//! list dups that binding, and the binding is delivered to foreign
//! children exactly like an `exec`-bound fd. rubash's materializer
//! only served the dedicated fd-0 redirect_in mirror and the
//! dynamic-fd ({var}) applier, so on a simple external command the
//! producer was never spawned and the fd stayed unbound; and even
//! for plain files the spawn-time stdin dup resolved `<&N` only
//! against the ambient fd table, never the command's own list.
//!
//! These pins run rubash as a subprocess and observe files/stdout
//! only; they were run by executing the test binaries directly
//! (outside the build slot, whose flock fd 9 would leak into the
//! children). Producers/consumers are async children, so file
//! assertions poll to a deadline.
//!
//! Verified against GNU Bash 5.3.20 (script-file probes in the
//! cluster workdir): see REPORT.md for the probe ledger. On the
//! pre-fix master every numbered pin below is red (EBADF / empty
//! output); the three control pins (exec / group / {var}) are
//! green on master and guard the restructure.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2235-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

/// Poll until `path` exists or the deadline passes.
fn wait_file_exists(path: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    path.exists()
}

#[test]
fn issue2235_external_dup_of_own_procsub_input() {
    // Battery cell c4: the producer's bytes reach a foreign child
    // through the command's own fd-3 binding plus a same-list dup.
    let (stdout, stderr, code) = rubash_stdin("cat 3< <(printf 'pf\\n') <&3\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "pf\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2235_external_sort_reads_own_procsub_input() {
    // Battery cell f4: a second foreign consumer shape.
    let (stdout, stderr, code) = rubash_stdin("sort 3< <(printf 'b\\na\\n') <&3\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a\nb\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2235_sh_child_inherits_procsub_input() {
    // Battery cell f2: the binding crosses to a foreign shell child
    // at its fd number (the #1686 delivery face, procsub source).
    let (stdout, stderr, code) = rubash_stdin("sh -c 'cat <&3' 3< <(printf 'sf\\n')\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "sf\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2235_rubash_child_inherits_procsub_input() {
    // Battery cell e3: the binding crosses to a rubash child through
    // the env channel.
    let script = format!(
        "'{}' -c 'cat <&3' 3< <(printf 'ef\\n')\n",
        env!("CARGO_BIN_EXE_rubash")
    );
    let (stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ef\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2235_stdin_dup_of_own_plain_file_input() {
    // Battery cells c12/f3: the plain-file sibling of the procsub
    // cell — same ordered-application mechanism (an fd-0 dup copies
    // the command's own earlier binding), no substitution involved.
    let dir = test_dir("plain");
    std::fs::write(dir.join("seed.txt"), "seed-line\n").expect("write seed");
    let script = format!("cd {}\ncat 3<seed.txt <&3\n", dir.display());
    let (stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2235_stdin_redirect_from_own_numbered_input() {
    // Battery cell f5: the dup target is fd 0 itself (`0<&3`).
    let (stdout, stderr, code) = rubash_stdin("cat 3< <(printf 'a\\nb\\n') 0<&3\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a\nb\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2235_last_numbered_input_wins_both_producers_run() {
    // Two entries bind fd 3: the later one is what the dup copies
    // (GNU's ordered walk), but the earlier entry's producer still
    // ran — its side effect must be observable.
    let dir = test_dir("lastwins");
    let script = format!(
        "cd {dir}\ncat 3< <(printf 'A\\n'; touch markerA) 3< <(printf 'B\\n') <&3\n",
        dir = dir.display()
    );
    let (stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "B\n", "stderr: {stderr}");
    assert!(
        wait_file_exists(&dir.join("markerA")),
        "the overwritten entry's producer never ran (markerA missing); stderr: {stderr}"
    );
}

#[test]
fn issue2235_numbered_input_producer_runs_without_consumer() {
    // A binding nobody reads still forks its producer at expansion
    // time (subst.c:6362): the side effect lands even though the
    // command itself ignores fd 3.
    let dir = test_dir("sideeffect");
    let script = format!("cd {dir}\ntrue 3< <(touch marker)\n", dir = dir.display());
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        wait_file_exists(&dir.join("marker")),
        "producer side effect missing; stderr: {stderr}"
    );
}

#[test]
fn issue2235_control_exec_input_procsub() {
    // Control (green on master): the exec face was already served
    // by the ambient-table path.
    let (stdout, stderr, code) = rubash_stdin("exec 3< <(printf 'e\\n')\ncat <&3\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "e\n", "stderr: {stderr}");
}

#[test]
fn issue2235_control_group_input_procsub() {
    // Control (green on master): the compound face was already
    // served by the compound numbered-input scope.
    let (stdout, stderr, code) = rubash_stdin("{ cat <&3; } 3< <(printf 'g\\n')\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "g\n", "stderr: {stderr}");
}

#[test]
fn issue2235_control_dynamic_fd_input_procsub() {
    // Control (green on master): the {var} face belongs to the
    // dynamic-fd applier, a different cell from this issue's.
    let (stdout, stderr, code) = rubash_stdin("exec {v}< <(printf 'v\\n')\ncat <&$v\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "v\n", "stderr: {stderr}");
}
