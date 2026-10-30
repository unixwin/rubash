//! Issue unixwin/rubash#2249: a `<>` redirect on a FUNCTION CALL bound
//! only the READ endpoint of fd 0 — the write endpoint was never
//! modelled, so a body write like `echo x >&0` failed with
//! `Bad file descriptor` even though GNU opens the target read-write.
//!
//! GNU: `<>` is r_input_output — make_cmd.c:682-683 gives the redirect
//! flags O_RDWR|O_CREAT and redir.c:725 redir_open performs a single
//! open, so fd 0 is ONE descriptor serving both directions over one
//! open file description (shared offset). execute_function applies the
//! call's redirections with do_redirections (execute_cmd.c:5181) and
//! undoes them on return, so the caller's fd 0 comes back untouched.
//! Every other `<>` face in rubash routes through
//! set_fd_readwrite_file / FileFd::open_readwrite_pair; the
//! function-call stdin bind (function_call_stdin) was the one face
//! that installed FileFd::open_read alone. The fix binds the
//! read-write pair there (net targets bind the connected socket in
//! both directions) and extends the binding's undo contract to the
//! fd-0 env keys the bind overwrites.
//!
//! Item (2) of the issue: execute_direct_shell_script's fd-0 install
//! in external_finish.rs opened the bare mapped operand — the same
//! unanchored ladder #2240 removed from function_call_stdin. With a
//! same-named decoy in the startup directory, the child script read
//! the decoy through a live endpoint; the install now takes the
//! #2240 ladder (windows_operand_file_path + session_path anchor).
//!
//! Expectations measured against GNU bash 5.3.20 (owner-compiled
//! oracle) with script files, 2026-10-31, master 1077ff82 (probes
//! probeA–probeJ in the cluster workdir). The numbered-fd pin is an
//! adjacent control (passes on master too); the fd-0 pins are red on
//! master (`echo: write error: Bad file descriptor`, empty file).

use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Fixture {
    root: PathBuf,
    work: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("issue2249_{}_{}", tag, std::process::id()));
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
fn issue2249_function_readwrite_write_lands_in_file() {
    let fx = fixture("write");
    let (stdout, stderr, code) = fx.run(
        "\
k () { echo written >&0; }
k <>out.txt; echo \"rc=$?\"
echo \"out.txt: [$(cat out.txt)]\"
",
    );
    assert_eq!(stdout, "rc=0\nout.txt: [written]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2249_function_readwrite_shared_offset() {
    // One open file description: the body's read advances the same
    // offset the body's later write continues from (GNU: the write
    // lands at offset 2, overwriting `cd` — not at 0).
    let fx = fixture("offset");
    let (stdout, stderr, _code) = fx.run(
        "\
printf 'abcdef\\n' > shared.txt
f () { head -c 2 <&0; echo; printf PQ >&0; }
f <>shared.txt; echo \"rc=$?\"
echo \"file: [$(cat shared.txt)]\"
",
    );
    assert_eq!(stdout, "ab\nrc=0\nfile: [abPQef]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2249_function_readwrite_binding_restored_on_return() {
    // The caller's fd 0 (a pipe read end) must come back on return:
    // the post-call read consumes the pipe, and a post-call `>&0`
    // write fails again exactly like GNU.
    let fx = fixture("restore");
    let (stdout, stderr, _code) = fx.run(
        "\
printf 'AB\\n' > bind.txt
f () { echo w >&0; cat <&0; }
printf 'l1\\nl2\\n' |
{
  f <>bind.txt
  echo \"f-rc=$?\"
  read y; echo \"y=$y\"
  echo z >&0; echo \"post-write-rc=$?\"
}
echo \"bind.txt: [$(cat bind.txt)]\"
",
    );
    assert_eq!(stdout, "\nf-rc=0\ny=l1\npost-write-rc=1\nbind.txt: [w]\n");
    assert!(
        stderr.contains("Bad file descriptor"),
        "stderr should carry the restored-fd write error, got: {stderr}"
    );
}

#[test]
fn issue2249_function_readwrite_relative_target_after_cd() {
    // The pair bind rides the #2240 anchored ladder: after `cd sub`
    // the write lands in sub/rel.txt, the launch namesake is untouched.
    let fx = fixture("cdanchor");
    let (stdout, stderr, _code) = fx.run(
        "\
mkdir -p sub
printf 'launch\\n' > rel.txt
printf 'stale\\n' > sub/rel.txt
f () { echo deep >&0; }
cd sub && f <>rel.txt; echo \"rc=$?\"
echo \"sub rel.txt: [$(cat rel.txt)]\"
echo \"launch rel.txt: [$(cat ../rel.txt)]\"
",
    );
    assert_eq!(
        stdout,
        "rc=0\nsub rel.txt: [deep]\nlaunch rel.txt: [launch]\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue2249_function_readwrite_creates_missing_file() {
    // GNU r_input_output carries O_CREAT: a missing target is created
    // by the redirect itself, not only by the validation probe.
    let fx = fixture("create");
    let (stdout, stderr, _code) = fx.run(
        "\
rm -f created.txt
f () { echo new >&0; }
f <>created.txt; echo \"rc=$?\"
echo \"created.txt: [$(cat created.txt)]\"
",
    );
    assert_eq!(stdout, "rc=0\ncreated.txt: [new]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2249_function_readwrite_definition_redirect() {
    // A `<>` stored on the definition applies per call through the
    // same function_call_stdin path.
    let fx = fixture("defredir");
    let (stdout, stderr, _code) = fx.run(
        "\
f () { echo def >&0; } <>def.txt
f; echo \"rc=$?\"
echo \"def.txt: [$(cat def.txt)]\"
",
    );
    assert_eq!(stdout, "rc=0\ndef.txt: [def]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2249_function_readwrite_dev_null() {
    let fx = fixture("devnull");
    let (stdout, stderr, _code) = fx.run(
        "\
f () { echo n >&0; echo \"write-rc=$?\"; read x <&0; echo \"read-rc=$?\"; }
f <>/dev/null; echo \"rc=$?\"
echo done
",
    );
    assert_eq!(stdout, "write-rc=0\nread-rc=1\nrc=0\ndone\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2249_function_numbered_readwrite_control() {
    // Adjacent control: a numbered `3<>` on a call already routes
    // through the scoped redirect face and works; it must not move.
    let fx = fixture("numbered");
    let (stdout, stderr, _code) = fx.run(
        "\
f () { echo via3 >&3; }
f 3<>num.txt; echo \"rc=$?\"
echo \"num.txt: [$(cat num.txt)]\"
",
    );
    assert_eq!(stdout, "rc=0\nnum.txt: [via3]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2249_child_script_stdin_anchored_after_cd_with_decoy() {
    // Issue item (2): the in-process child script's `< file` install
    // must anchor at the session cwd. A same-named decoy in the
    // startup directory used to win the live endpoint; the text
    // fallback only masked the shape when no decoy existed.
    let fx = fixture("childanchor");
    std::fs::create_dir_all(fx.work.join("sub")).expect("create sub");
    std::fs::write(fx.work.join("f.txt"), "launch-data\n").expect("write decoy");
    std::fs::write(fx.work.join("sub/f.txt"), "sub-data\n").expect("write sub f.txt");
    let child = fx.work.join("sub/child.sh");
    std::fs::write(&child, "cat\n").expect("write child script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o755))
            .expect("chmod child script");
    }
    let (stdout, stderr, _code) = fx.run(
        "\
cd sub && ./child.sh <f.txt; echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "sub-data\nrc=0\n");
    assert_eq!(stderr, "");
}
