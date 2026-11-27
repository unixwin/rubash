//! rubash#3268 (e) — a shell BORN in a deleted directory must live
//! in that inode, not be transplanted to `/`.
//!
//! GNU keeps no cwd string at startup: the process inode IS the
//! state, and variables.c:907 set_pwd only seeds $PWD — trusting an
//! imported PWD when it is absolute and same_file(PWD, ".") holds,
//! and otherwise calling get_working_directory("shell-init"), which
//! on failure prints the shell-init line and leaves PWD UNBOUND (an
//! imported value keeps its stale contents; an absent one stays
//! absent). The session then works from the dead inode: children
//! inherit it (execute_cmd.c), in-process walks start from it, and
//! `pwd` fails (builtins/cd.def pwd_builtin ->
//! builtins/common.c:592 get_working_directory, errstr
//! general.c:70) until the first successful cd.
//!
//! The old birth code called current_dir() twice (env seed and
//! ShellState literal) and fell back to PathBuf::from("/") on
//! failure: a rubash started by a parent sitting in a deleted
//! directory was born believing its cwd was the root — `pwd`
//! printed `/` rc 0, $PWD was seeded `/`, children were stamped
//! into `/`, and `ls ..` listed the root directory.
//!
//! The fix captures the birth cwd once. On success nothing changes
//! (field = path, PWD seeded, anchor lazy). On failure the field
//! takes the empty path — the honest "unknown", which never
//! resolves and never equals a real directory — the birth anchor is
//! opened on "." (which succeeds on a deleted directory) and names
//! the field, PWD is not seeded, and the shell-init line is printed
//! once (respawned children, GNU's forked children, stay silent; the
//! per-command job-working-directory repeats are deliberately not
//! followed). A validated respawn whose birth getcwd fails adopts
//! the transported PWD as its field — the fork child's inherited
//! the_current_working_directory — because its spawner stamped it
//! into the very inode that string names.
//!
//! Construction (DESIGN S5.4): the test process never chdirs. A
//! wrapper /bin/sh cds into $D/a, a subshell removes a, and the
//! wrapper exec's rubash with its process cwd inside the deleted
//! inode.
//!
//! Oracle: GNU bash 5.3.20 (probe-e-startup.sh / probe-e-respawn.sh,
//! #3268 design volume): cat ../f -> hello rc 0 with the shell-init
//! line on stderr; ls .. lists the parent; pwd -> no stdout, rc 1,
//! the pwd: line on stderr; x=$(cat ../f; true) -> hello; cd out and
//! back recovers; cat "$D/a/../f" -> rc 1; a respawned child's pwd
//! prints the parent's dead-cwd string rc 0.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const INIT_LINE: &str = "shell-init: error retrieving current directory: \
                         getcwd: cannot access parent directories: \
                         No such file or directory";
const PWD_LINE: &str = "pwd: error retrieving current directory: \
                        getcwd: cannot access parent directories: \
                        No such file or directory";

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3268e-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).expect("create scratch/a");
    std::fs::write(dir.join("f"), "hello\n").expect("write scratch/f");
    ScratchDir(dir)
}

fn wait_child(mut child: std::process::Child) -> (String, String, Option<i32>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("poll child") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("child did not exit within 30s");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    use std::io::Read;
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.code())
}

/// Run rubash BORN in the deleted $D/a (wrapper construction above).
/// With `unset_pwd`, the exec goes through `env -u PWD` so no PWD is
/// imported; otherwise the wrapper shell's own cd exports
/// PWD=$D/a, the stale-import shape.
fn run_born_dead(dir: &Path, body: &str, unset_pwd: bool) -> (String, String, Option<i32>) {
    let rubash = env!("CARGO_BIN_EXE_rubash");
    let wrapper = if unset_pwd {
        "cd \"$D/a\" && (cd ..; rmdir a) && exec env -u PWD \"$0\" -c \"$1\""
    } else {
        "cd \"$D/a\" && (cd ..; rmdir a) && exec \"$0\" -c \"$1\""
    };
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg(wrapper)
        .arg(rubash)
        .arg(body)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .env("D", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wrapper sh");
    wait_child(child)
}

/// Run rubash as a normal live-born process with its process cwd at
/// `dir`, executing the script file `script` (already written).
fn run_live(dir: &Path, script: &Path) -> (String, String, Option<i32>) {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .env("D", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    wait_child(child)
}

#[test]
fn issue3268e_birth_cat_parent_operand_reads_via_birth_anchor() {
    let dir = scratch("cat");
    let (stdout, stderr, _) = run_born_dead(&dir.0, "cat ../f; echo \"cat_rc=$?\"", true);
    assert_eq!(stdout, "hello\ncat_rc=0\n");
    // The shell-init line appears exactly once (captain ruling: the
    // birth line only, no per-command repeats).
    assert_eq!(stderr.matches(INIT_LINE).count(), 1, "stderr: {stderr}");
}

#[test]
fn issue3268e_birth_ls_parent_lists_the_real_parent() {
    let dir = scratch("ls");
    let (stdout, _, _) = run_born_dead(&dir.0, "ls ..; echo \"ls_rc=$?\"", true);
    assert!(stdout.lines().any(|line| line == "f"), "stdout: {stdout}");
    assert!(stdout.contains("ls_rc=0"), "stdout: {stdout}");
    assert!(
        !stdout.lines().any(|line| line == "usr"),
        "session was transplanted to /: {stdout}"
    );
}

#[test]
fn issue3268e_birth_pwd_fails_and_pwd_stays_unbound_without_import() {
    let dir = scratch("pwd-unbound");
    let (stdout, stderr, _) = run_born_dead(
        &dir.0,
        "pwd; echo \"pwd_rc=$?\"; echo \"PWD=[${PWD-UNSET}]\"",
        true,
    );
    assert_eq!(stdout, "pwd_rc=1\nPWD=[UNSET]\n");
    assert!(stderr.contains(INIT_LINE), "stderr: {stderr}");
    assert!(stderr.contains(PWD_LINE), "stderr: {stderr}");
}

#[test]
fn issue3268e_birth_pwd_import_keeps_its_stale_value() {
    // GNU set_pwd does not rebind PWD when get_working_directory
    // fails: an imported PWD survives verbatim (variables.c:907),
    // while `pwd` still fails — the variable is not the working
    // directory cache.
    let dir = scratch("pwd-stale");
    let dead = dir.0.join("a");
    let (stdout, stderr, _) = run_born_dead(
        &dir.0,
        "echo \"PWD=[$PWD]\"; pwd; echo \"pwd_rc=$?\"",
        false,
    );
    assert!(
        stdout.contains(&format!("PWD=[{}]", dead.display())),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("pwd_rc=1"), "stdout: {stdout}");
    assert!(stderr.contains(PWD_LINE), "stderr: {stderr}");
}

#[test]
fn issue3268e_birth_comsub_full_path_reads_via_anchor() {
    let dir = scratch("comsub-full");
    let (stdout, _, _) = run_born_dead(&dir.0, "x=$(cat ../f; true); echo \"x=[$x]\"", true);
    assert_eq!(stdout, "x=[hello]\n");
}

#[test]
fn issue3268e_birth_first_cd_recovers_the_session() {
    let dir = scratch("recover");
    let (stdout, _, rc) =
        run_born_dead(&dir.0, "cd /; cd \"$D\"; pwd; cat f; echo \"rc=$?\"", true);
    assert!(
        stdout.contains(&format!("{}\n", dir.0.display())),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("hello"), "stdout: {stdout}");
    assert!(stdout.contains("rc=0"), "stdout: {stdout}");
    assert_eq!(rc, Some(0));
}

#[test]
fn issue3268e_birth_absolute_path_through_dead_dir_still_fails() {
    // GNU parity lock (already true before the fix): an absolute
    // spelling that traverses the dead directory by name is ENOENT
    // — the anchor serves RELATIVE walks from the inode, never
    // name-based resurrection.
    let dir = scratch("abs-dead");
    let (stdout, _, _) = run_born_dead(&dir.0, "cat \"$D/a/../f\"; echo \"rc=$?\"", true);
    assert_eq!(stdout, "rc=1\n");
}

#[test]
fn issue3268e_birth_respawned_child_shares_the_dead_inode() {
    // A child respawned by a born-dead session is stamped into the
    // birth anchor's inode: its own relative reads work, and its
    // pwd fails exactly like its parent's (no cwd string exists to
    // inherit — the parent's PWD is unbound).
    let dir = scratch("birth-respawn");
    let (stdout, _, _) = run_born_dead(
        &dir.0,
        "( pwd; echo \"cpwd_rc=$?\"; cat ../f ) & wait",
        true,
    );
    assert!(stdout.contains("cpwd_rc=1"), "stdout: {stdout}");
    assert!(stdout.contains("hello"), "stdout: {stdout}");
}

#[test]
fn issue3268e_respawn_async_child_of_runtime_dead_session_inherits_cwd() {
    // E6, whitelisted-spawn arm: the parent is born live, cds into
    // a, and a-deletion happens at runtime. GNU's forked async child
    // inherits the cached string and the dead inode.
    let dir = scratch("respawn-async");
    let script = dir.0.join("body.sh");
    std::fs::write(
        &script,
        "cd \"$D/a\"\n\
         ( cd ..; rmdir a )\n\
         ( echo \"ASYNC-BEGIN\"; pwd; echo \"async_pwd_rc=$?\"; \
         echo \"async PWD=[$PWD]\"; cat ../f; echo \"async_cat_rc=$?\"; \
         echo \"ASYNC-END\" ) &\n\
         wait $!\n",
    )
    .expect("write body");
    let (stdout, _, _) = run_live(&dir.0, &script);
    let dead = dir.0.join("a");
    assert!(
        stdout.contains(&format!("\n{}\n", dead.display())),
        "child pwd must print the dead cwd string: {stdout}"
    );
    assert!(stdout.contains("async_pwd_rc=0"), "stdout: {stdout}");
    assert!(
        stdout.contains(&format!("async PWD=[{}]", dead.display())),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("hello"), "stdout: {stdout}");
    assert!(stdout.contains("async_cat_rc=0"), "stdout: {stdout}");
}

#[test]
fn issue3268e_respawn_coproc_child_of_runtime_dead_session_inherits_cwd() {
    // E6, Command-spawn arm (stamp_spawn_cwd): same GNU fork
    // inheritance through a coproc body.
    let dir = scratch("respawn-coproc");
    let script = dir.0.join("body.sh");
    std::fs::write(
        &script,
        "cd \"$D/a\"\n\
         ( cd ..; rmdir a )\n\
         coproc PROBE {\n\
         echo \"CHILD-BEGIN\"\n\
         pwd; echo \"child_pwd_rc=$?\"\n\
         echo \"child PWD=[$PWD]\"\n\
         cat ../f; echo \"child_cat_rc=$?\"\n\
         echo \"CHILD-END\"\n\
         }\n\
         cat <&${PROBE[0]}\n\
         exec {PROBE[0]}<&-\n\
         wait $PROBE_PID\n",
    )
    .expect("write body");
    let (stdout, _, _) = run_live(&dir.0, &script);
    let dead = dir.0.join("a");
    assert!(
        stdout.contains(&format!("\n{}\n", dead.display())),
        "child pwd must print the dead cwd string: {stdout}"
    );
    assert!(stdout.contains("child_pwd_rc=0"), "stdout: {stdout}");
    assert!(
        stdout.contains(&format!("child PWD=[{}]", dead.display())),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("hello"), "stdout: {stdout}");
    assert!(stdout.contains("child_cat_rc=0"), "stdout: {stdout}");
}

#[test]
fn issue3268e_normal_birth_is_unchanged() {
    // Control (green before and after): a live birth prints its
    // directory, reads its files, and prints no shell-init line.
    let dir = scratch("normal");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("pwd; cat f")
        .current_dir(&dir.0)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let (stdout, stderr, rc) = wait_child(child);
    assert_eq!(stdout, format!("{}\nhello\n", dir.0.display()));
    assert!(!stderr.contains("shell-init"), "stderr: {stderr}");
    assert_eq!(rc, Some(0));
}

#[test]
fn issue3268e_birth_comsub_shortcut_reads_via_anchor() {
    let dir = scratch("comsub-shortcut");
    let (stdout, _, _) = run_born_dead(&dir.0, "x=$(cat ../f); echo \"x=[$x]\"", true);
    assert_eq!(stdout, "x=[hello]\n");
}
