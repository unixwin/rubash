//! rubash#2298 — a numbered read-write redirect (`3<>file`) on an
//! external command was not delivered to the spawned child in a
//! writable form: the numbered walk in apply_external_fd_delivery
//! classified `<>` with the input kinds and opened the target
//! through the O_RDONLY input ladder, so the child held a read-only
//! description of the slot and a write through it failed EBADF
//! (the read direction happened to work, which is how the class
//! hid). GNU r_input_output opens O_RDWR|O_CREAT (make_cmd.c:682)
//! through redir_open (redir.c:725) and the ONE open file
//! description serves both directions in the forked child
//! (do_redirections, redir.c:237).
//!
//! The fix opens `<>` entries in the walk through the classified
//! readwrite ladder (probe_input_redirect_readwrite — the same open
//! the builtin faces use for `<>`) and stamps both env channels
//! with the single dup, which the child's startup restore pairs
//! into one shared FileFd (init.rs). The pre_exec pin already
//! delivered the descriptor itself; it now delivers an O_RDWR one.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle) with
//! the probe cells in the cluster workdir (probes/p1–p10): cells 1,
//! 2 and 4 are red on master c537d124 and green with the fix; the
//! read cell and the controls are green on both.
//!
//! OBSERVER NOTE: observers are a foreign `sh` and the rubash
//! binary itself re-invoked — never a `./reader.sh` script: rubash
//! runs ENOEXEC scripts in-process, where spawn delivery does not
//! appear (rubash#2270/#2286).

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Fixture {
    root: PathBuf,
    work: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("issue2298_{}_{}", tag, std::process::id()));
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
        let child = Command::new(RUBASH)
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

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.work.join(name)).unwrap_or_default()
    }
}

/// Write direction, foreign child, missing target: `<>` must CREATE
/// the file (O_CREAT) and the child must be able to write through
/// the delivered slot. The issue's reproducer.
#[cfg(unix)]
#[test]
fn issue2298_spawn_readwrite_write_foreign_child() {
    let fx = fixture("write");
    let (stdout, stderr, code) = fx.run("sh -c 'echo via3 >&3' 3<>num.txt\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(fx.read("num.txt"), "via3\n");
}

/// Write direction, nested rubash child: the env-channel restore
/// must rebuild the slot as a read-write binding (both HANDLE and
/// WHANDLE stamped from the one description), not read-only.
#[cfg(unix)]
#[test]
fn issue2298_spawn_readwrite_write_nested_child() {
    let fx = fixture("nested_write");
    let script = format!("'{RUBASH}' -c 'echo via3 >&3' 3<>num.txt\necho rc=$?\n");
    let (stdout, _stderr, code) = fx.run(&script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(fx.read("num.txt"), "via3\n");
}

/// Read direction, foreign child: a pre-seeded file read through
/// the `<>` slot. (Green on master too — the read half of the
/// O_RDONLY delivery worked; this guards it stays green.)
#[cfg(unix)]
#[test]
fn issue2298_spawn_readwrite_read_foreign_child() {
    let fx = fixture("read");
    std::fs::write(fx.work.join("seed.txt"), "seed-line\n").expect("seed");
    let (stdout, _stderr, code) = fx.run("sh -c 'cat <&3' 3<>seed.txt\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "seed-line\nrc=0\n");
}

/// One description, shared offset: the child reads a line through
/// the slot, then writes at the offset the read left — the write
/// lands mid-file over the unread tail, proving the delivered
/// descriptor is a single O_RDWR open file description (a
/// read-only delivery fails the write; two separate opens would
/// not share the offset).
#[cfg(unix)]
#[test]
fn issue2298_spawn_readwrite_one_description_shared_offset() {
    let fx = fixture("shared_offset");
    std::fs::write(fx.work.join("rw.txt"), "line-one\nREST\n").expect("seed");
    let (stdout, _stderr, code) = fx.run(
        "sh -c 'IFS= read -r line <&3; echo got:$line; echo back >&3' 3<>rw.txt\necho rc=$?\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:line-one\nrc=0\n");
    assert_eq!(fx.read("rw.txt"), "line-one\nback\n");
}

/// Same one-description shape through a nested rubash child: its
/// builtin read advances the restored slot's offset and its write
/// continues there.
#[cfg(unix)]
#[test]
fn issue2298_spawn_readwrite_nested_read_then_write() {
    let fx = fixture("nested_rw");
    std::fs::write(fx.work.join("rw.txt"), "line-one\nREST\n").expect("seed");
    let script = format!(
        "'{RUBASH}' -c 'IFS= read -r line <&3; echo got:$line; echo back >&3' 3<>rw.txt\necho rc=$?\n"
    );
    let (stdout, _stderr, code) = fx.run(&script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:line-one\nrc=0\n");
    assert_eq!(fx.read("rw.txt"), "line-one\nback\n");
}

/// Control: an exec-bound (ambient) `<>` slot already delivered as
/// one shared description — a spawned child reads the first line
/// and the parent shell continues at the shared offset. Guards the
/// ambient face the fix must not disturb.
#[cfg(unix)]
#[test]
fn issue2298_ambient_readwrite_shared_offset_control() {
    let fx = fixture("ambient");
    std::fs::write(fx.work.join("amb.txt"), "first\nsecond\n").expect("seed");
    let (stdout, _stderr, code) = fx.run(
        "exec 3<>amb.txt\nsh -c 'IFS= read -r a <&3; echo child:$a'\nIFS= read -r b <&3\necho parent:$b\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "child:first\nparent:second\n");
}

/// Control: the write-only numbered spawn delivery (`3>`) the
/// readwrite class sits next to — must not move.
#[cfg(unix)]
#[test]
fn issue2298_spawn_write_only_control() {
    let fx = fixture("write_only");
    let (stdout, _stderr, code) = fx.run("sh -c 'echo via3 >&3' 3>wo.txt\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(fx.read("wo.txt"), "via3\n");
}

/// Control: the builtin face over an ambient `<>` binding writes
/// through the slot (the in-process face verified working when the
/// issue was filed).
#[cfg(unix)]
#[test]
fn issue2298_builtin_ambient_readwrite_control() {
    let fx = fixture("builtin_ambient");
    let (stdout, _stderr, code) = fx.run("exec 3<>bi.txt\necho via3 >&3\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(fx.read("bi.txt"), "via3\n");
}
