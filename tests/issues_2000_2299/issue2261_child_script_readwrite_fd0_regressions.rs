//! Issue unixwin/rubash#2261: a `<>` redirect on a child SCRIPT
//! command (`./child.sh <>file`) installed only the READ endpoint of
//! fd 0 in execute_direct_shell_script (external_finish.rs) — the
//! write endpoint was never modelled, so a child write like
//! `echo x >&0` failed with `Bad file descriptor` even though GNU
//! opens the target read-write. Sibling face of rubash#2249 (the
//! function-call face, fixed by binding the read-write pair in
//! function_call_stdin).
//!
//! GNU: `<>` is r_input_output — make_cmd.c:682-683 gives the redirect
//! flags O_RDWR|O_CREAT and redir.c:725 redir_open performs a single
//! open, so the child's fd 0 is ONE descriptor serving both directions
//! over one open file description (shared offset), applied before the
//! child runs and gone when it exits. Every other `<>` face in rubash
//! routes through set_fd_readwrite_file / FileFd::open_readwrite_pair;
//! the child-script install was the remaining face that installed
//! FileFd::open_read alone. The fix binds the read-write pair there
//! (plus the fd-0 output env bookkeeping, mirroring
//! save_and_apply_fd0_readwrite_env); the undo is the install site's
//! existing whole-table / whole-env save-restore around the child
//! run. Net targets keep the function_call_stdin fall-through, whose
//! net arm binds the connected socket — the same routing plain `<`
//! takes for them.
//!
//! Baseline shape note (by mechanism): with a PRE-EXISTING target the
//! read-only install won on master and the pins below are red there
//! (EBADF / untouched file / child runs against a directory). With a
//! MISSING target, master's open_read failed and function_call_stdin
//! bound the pair as a side effect, so the create pin may already
//! pass on master — it locks the GNU behavior either way.
//!
//! Expectations measured against GNU bash 5.3.20 (owner-compiled
//! oracle) with script files, 2026-10-31, master 5b0ae861 (probes
//! probeP1–probeP10 in the cluster workdir; probeP8 `<> /dev/null`
//! observation — write-rc=0 / read-rc=1 — is reported in the cluster
//! REPORT but deliberately not pinned here: the null device stays on
//! the pre-existing fall-through this fix does not touch).
//! Adjacent pins that must not move: issue2249 (function face,
//! incl. its child-script `<` anchor pin) and issue2240 (operand
//! ladder / write-endpoint faces).

use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Fixture {
    root: PathBuf,
    work: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("issue2261_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let work = root.join("work");
    std::fs::create_dir_all(&work).expect("create fixture work dir");
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture root");
    let work = root.join("work");
    Fixture { root, work }
}

impl Fixture {
    fn run(&self, script: &str) -> (String, String, Option<i32>) {
        let script_path = self.root.join("probe.sh");
        std::fs::write(&script_path, script).expect("write probe script");
        let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&script_path)
            .current_dir(&self.work)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rubash");
        let out = child.wait_with_output().expect("wait rubash");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn issue2261_child_script_readwrite_write_lands_in_file() {
    // The issue shape verbatim (target pre-exists): the child's
    // `echo written >&0` overwrites at the shared offset 0.
    let fx = fixture("write");
    let (stdout, stderr, code) = fx.run(
        "\
printf 'echo written >&0\\n' > child.sh
chmod +x child.sh
printf 'zzzzzz\\n' > out.txt
./child.sh <>out.txt; echo \"rc=$?\"
echo \"out.txt: [$(cat out.txt)]\"
",
    );
    assert_eq!(stdout, "rc=0\nout.txt: [written]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2261_child_script_readwrite_shared_offset() {
    // One open file description: the child's read advances the same
    // offset the child's later write continues from (GNU: the write
    // lands at offset 2, overwriting `cd` — not at 0).
    let fx = fixture("offset");
    let (stdout, stderr, _code) = fx.run(
        "\
printf 'head -c 2 <&0; echo; printf PQ >&0\\n' > child.sh
chmod +x child.sh
printf 'abcdef\\n' > shared.txt
./child.sh <>shared.txt; echo \"rc=$?\"
echo \"file: [$(cat shared.txt)]\"
",
    );
    assert_eq!(stdout, "ab\nrc=0\nfile: [abPQef]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2261_child_script_readwrite_creates_missing_file() {
    // GNU r_input_output carries O_CREAT: a missing target is created
    // by the redirect itself and the child's write lands in it.
    let fx = fixture("create");
    let (stdout, stderr, _code) = fx.run(
        "\
printf 'echo new >&0\\n' > child.sh
chmod +x child.sh
rm -f created.txt
./child.sh <>created.txt; echo \"rc=$?\"
echo \"created.txt: [$(cat created.txt)]\"
",
    );
    assert_eq!(stdout, "rc=0\ncreated.txt: [new]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2261_child_script_readwrite_parent_fd0_restored() {
    // The parent's fd 0 (a pipe read end) must be exactly what it was
    // after the child exits: the parent still reads the pipe, and a
    // parent-side `>&0` write fails again exactly like GNU.
    let fx = fixture("restore");
    let (stdout, stderr, _code) = fx.run(
        "\
printf 'echo w >&0\\n' > child.sh
chmod +x child.sh
printf 'XX\\n' > bind.txt
printf 'l1\\nl2\\n' |
{
  ./child.sh <>bind.txt
  echo \"child-rc=$?\"
  read y; echo \"y=$y\"
  echo z >&0; echo \"post-write-rc=$?\"
}
echo \"bind.txt: [$(cat bind.txt)]\"
",
    );
    assert_eq!(stdout, "child-rc=0\ny=l1\npost-write-rc=1\nbind.txt: [w]\n");
    assert!(
        stderr.contains("Bad file descriptor"),
        "stderr should carry the restored-fd write error, got: {stderr}"
    );
}

#[test]
fn issue2261_child_script_readwrite_cd_anchor_write_side() {
    // The pair bind rides the #2240 anchored ladder (#2260 install):
    // after `cd sub` the child's write lands in sub/f.txt at the
    // shared offset; the launch-directory namesake is untouched.
    let fx = fixture("cdanchor");
    let (stdout, stderr, _code) = fx.run(
        "\
printf 'head -c 2 <&0; echo; printf PQ >&0\\n' > child.sh
chmod +x child.sh
mkdir -p sub
cp child.sh sub/child.sh
printf 'launch\\n' > f.txt
printf 'abcdef\\n' > sub/f.txt
cd sub && ./child.sh <>f.txt; echo \"rc=$?\"
echo \"sub f.txt: [$(cat f.txt)]\"
echo \"launch f.txt: [$(cat ../f.txt)]\"
",
    );
    assert_eq!(
        stdout,
        "ab\nrc=0\nsub f.txt: [abPQef]\nlaunch f.txt: [launch]\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue2261_child_script_readwrite_directory_fails() {
    // GNU's O_RDWR open of a directory fails EISDIR: the child does
    // not run, the command reports the target, status 1.
    let fx = fixture("dirfail");
    let (stdout, stderr, _code) = fx.run(
        "\
printf 'echo SHOULD_NOT_RUN\\n' > child.sh
chmod +x child.sh
mkdir -p somedir
./child.sh <>somedir; echo \"rc=$?\"
echo end
",
    );
    assert_eq!(stdout, "rc=1\nend\n");
    assert!(
        stderr.contains("Is a directory"),
        "stderr should name the EISDIR failure, got: {stderr}"
    );
}

#[test]
fn issue2261_child_script_numbered_readwrite_control() {
    // Adjacent control: a numbered `3<>` on an external command
    // already routes through the scoped redirect face and works; it
    // must not move.
    //
    // OBSERVER NOTE (rubash#2286): the observing child is the nested
    // rubash binary re-invoked as a real spawned process, NOT a
    // `./child.sh` script: rubash runs ENOEXEC scripts in-process,
    // where spawn-delivered fd bindings do not appear the same way,
    // so the old script-child green could not by itself prove the
    // spawn delivery face this control guards (the same harness
    // defect PR #2283 repaired for the #2270 pins). The observation
    // is unchanged: the child writes `via3` to its fd 3 and exits 0;
    // the redirect shape and both expectations are unchanged.
    let fx = fixture("numbered");
    let script = format!(
        "\
rm -f num.txt
\"{}\" -c 'echo via3 >&3' 3<>num.txt; echo \"rc=$?\"
echo \"num.txt: [$(cat num.txt)]\"
",
        env!("CARGO_BIN_EXE_rubash")
    );
    let (stdout, stderr, _code) = fx.run(&script);
    assert_eq!(stdout, "rc=0\nnum.txt: [via3]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2261_child_script_plain_input_control() {
    // Adjacent control: plain `<` on a child script keeps serving
    // the file through the install's read endpoint.
    let fx = fixture("plainin");
    let (stdout, stderr, _code) = fx.run(
        "\
printf 'cat\\n' > child.sh
chmod +x child.sh
printf 'read-data\\n' > in.txt
./child.sh <in.txt; echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "read-data\nrc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2261_child_script_plain_input_missing_control() {
    // Adjacent control: plain `<` on a missing file still reports
    // the target and does not run the child (issue #250 shape).
    let fx = fixture("plainmissing");
    let (stdout, stderr, _code) = fx.run(
        "\
printf 'echo SHOULD_NOT_RUN\\n' > child.sh
chmod +x child.sh
rm -f missing.txt
./child.sh <missing.txt; echo \"rc=$?\"
echo end
",
    );
    assert_eq!(stdout, "rc=1\nend\n");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr should name the missing target, got: {stderr}"
    );
}
