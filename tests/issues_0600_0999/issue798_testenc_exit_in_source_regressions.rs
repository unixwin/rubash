//! Issue rubash#798 (eco-harvest verdict, run 39078222286): the LibreSSL
//! roundtrip tester (asset: WeilerWebServices/Powershell
//! `LibreSSL/tests/testenc.sh`) sourced in the harness sandbox kept printing
//! cipher lines for the whole 12s source wall (`hang:source:niu: ...`), read
//! as "the `exit 1` inside `source` did not terminate execution".
//!
//! Differential triage against WSL GNU bash 5.3.0 says the control-flow
//! surface is at PARITY and the ticket's premise does not hold for the
//! sandbox shape:
//!
//! - `cat $srcdir/openssl.cnf >$test` truncates `$test` to the empty file
//!   BEFORE failing (the redirect happens first), and every later
//!   `$cmd ... > $test.$i.cipher` spawn failure still creates its (empty)
//!   redirect target — so `cmp` compares two empty files, returns 0, and
//!   the `exit 1` branch is legitimately NEVER reached. The full 47-cipher
//!   loop is the CORRECT behavior: GNU runs it to completion too (verified:
//!   identical stdout, rc 0 — rubash 4.5s, msys bash 5.2 5.3s, WSL GNU 5.3
//!   2.3s for the identical 100/201-line transcript). The CI HANG verdict
//!   was that spawn-fail loop exceeding the 12s SOURCE_TIMEOUT on a 2-core
//!   runner — volume, not control flow (the issue's "GNU 只会跑 1 个
//!   cipher 就退出" assumes a non-empty `$test`, which the sandbox never
//!   produces).
//! - The exit semantics themselves are pinned at parity below: when `cmp`
//!   DOES fail (a real roundtrip mismatch), the sourced `exit 1` inside the
//!   `for`/`if` shape terminates the WHOLE shell on the first failure, mid
//!   list, rc 1 — evalstring.c:396-403 catches EXITPROG at
//!   parse_and_execute's own setjmp and :618-619 re-raises
//!   jump_to_top_level out of `_evalfile` (builtins/evalfile.c:302) into
//!   the top level; interactive sessions end the same way (#426's pins,
//!   now also under the loop+`if` shape).
//! - `$?` after a missing-command spawn is 127 in both engines, and the
//!   failed spawn still creates its empty redirect target, so
//!   `[ $? != 0 ]` sees 127 exactly when cmp fails. No #705 interaction:
//!   that ticket changed `exit` USAGE-error values (bash 5.3 exit 2), not
//!   the exit-in-source jump.
//!
//! The fixture keeps the asset's load-bearing shape — `$cmd ... < in > out`
//! with `cmd` absent, `cmp $test $test.clear` + `if [ $? != 0 ] then exit 1
//! else rm ... fi` per iteration — with `cmp`/`rm` as shell functions so
//! the pins are hermetic (no external diff/cmp on PATH), per the
//! minimized-form rule.
#![cfg(windows)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn fixture(tag: &str, cmp_rc: u8) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-it798-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let body = format!(
        r#"test=p
cmd=./missing-openssl
cmp() {{ return {cmp_rc}; }}
rm() {{ :; }}
: > $test
echo leg0
$cmd enc < $test > $test.cipher
$cmd enc < $test.cipher > $test.clear
cmp $test $test.clear
if [ $? != 0 ]
then
	exit 1
else
	rm $test.cipher $test.clear
fi
for i in one two three
do
	echo $i
	$cmd $i -e -k test < $test > $test.$i.cipher
	$cmd $i -d -k test < $test.$i.cipher >$test.$i.clear
	cmp $test $test.clear
	if [ $? != 0 ]
	then
		exit 1
	else
		rm $test.$i.cipher $test.$i.clear
	fi
done
echo LOOP-DONE
"#,
        cmp_rc = cmp_rc
    );
    std::fs::write(dir.join("loop.sh"), body).expect("write fixture");
    dir
}

fn rubash_in(dir: &PathBuf) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.current_dir(dir).env_remove("PS1").env_remove("PS2");
    cmd
}

/// The sandbox shape (cmp=0 — every roundtrip "succeeds" on empty files):
/// the loop legitimately runs through, the shell survives, and the command
/// after the source executes. GNU 5.3.0 `bash -c 'source ./loop.sh; echo
/// AFTER'`: identical stdout, rc 0. The verdict's "loop ran through" is
/// this behavior, not a lost exit.
#[test]
fn sandbox_shape_loop_runs_through_and_shell_survives() {
    let dir = fixture("sandbox", 0);
    let output = rubash_in(&dir)
        .args(["-c", "source ./loop.sh; echo AFTER"])
        .output()
        .expect("run rubash -c");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect::<Vec<_>>(),
        vec!["leg0", "one", "two", "three", "LOOP-DONE", "AFTER"],
        "the whole list must run and the sourcer must continue: {stdout:?}"
    );
}

/// The failing shape (cmp=1 — a real roundtrip mismatch): the FIRST
/// failure's `exit 1` terminates the whole shell mid list — rc 1, later
/// iterations and the post-source command never run. GNU 5.3.0: identical
/// (stdout `leg0` only, rc 1).
#[test]
fn first_failing_cmp_exits_whole_shell_mid_list() {
    let dir = fixture("fail", 1);
    let output = rubash_in(&dir)
        .args(["-c", "source ./loop.sh; echo AFTER"])
        .output()
        .expect("run rubash -c");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect::<Vec<_>>(),
        vec!["leg0"],
        "the first failed cmp must end the shell: {stdout:?}"
    );
}

/// Interactive session, failing shape: the sourced exit ends the SESSION
/// (reader loop unwinds with the jump's status) — the stdin lines after the
/// source are never read.
#[test]
fn sourced_exit_in_loop_ends_interactive_session() {
    let dir = fixture("interactive", 1);
    let mut child = rubash_in(&dir)
        .arg("-i")
        .arg("--norc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"source ./loop.sh\ncommand echo AFTER\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("leg0") && !stdout.contains("AFTER"),
        "the session must die at the sourced exit: {stdout:?}"
    );
}

/// The spawn-fail plumbing the loop's test stands on: `$?` is 127 after a
/// missing-command spawn, and the failed spawn still created its (empty)
/// redirect target — which is exactly why the sandbox shape's `cmp` sees
/// two empty files. GNU 5.3.0: RC=127, EMPTY.
#[test]
fn spawn_fail_is_127_and_redirect_target_is_created_empty() {
    let dir = std::env::temp_dir().join(format!("rubash-it798-rd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("rd.sh"),
        "./definitely-missing-798 > q\necho RC=$?\n[ -f q ] && [ ! -s q ] && echo EMPTY\n",
    )
    .unwrap();
    let output = rubash_in(&dir).arg("./rd.sh").output().expect("run rubash");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect::<Vec<_>>(),
        vec!["RC=127", "EMPTY"],
        "stdout: {stdout:?}"
    );
}
