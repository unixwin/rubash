//! rubash#3732: a compound's output-process-substitution consumer
//! runs in-process under the prefix redirects as its spawn plan
//! (`with_procsub_spawn_plan`, rubash#3491) — and the plan bind
//! restores a retired entry, the same hazard as #3706.
//!
//! GNU ground truth: a compound group's redirections are undoable
//! (execute_cmd.c:854 `do_redirections (command->redirects,
//! RX_ACTIVE|RX_UNDOABLE)`), and the undo save IS a kernel duplicate
//! of the displaced descriptor (`fcntl (fd, F_DUPFD, SHELL_FD_BASE)`,
//! redir.c:965/1082/1144, recorded by `add_undo_redirect`), so the
//! pre-group descriptor stays alive for the group's duration and
//! `undo_redirections` dups it back. Rubash's plan-bind save only
//! cloned the userspace entry, so the bind-time retire
//! (retire_read_arm/retire_write_arm, rubash#3213) closed the very
//! descriptor the restore was about to replay — the restore
//! re-inserted a retired entry, and the next read/write (or the next
//! spawn's pin pass) hit a dead descriptor.
//!
//! Every cell runs a compound whose redirect list puts a numbered-fd
//! redirect BEFORE the output substitution, so the plan binds that
//! fd for the consumer: `{ echo hello; } 8<redir > >(cat > got.txt)`.
//! The finish step runs AFTER the group's own redirect scope has
//! restored fd 8 to its ambient file, so the plan bind — and only
//! the plan bind — is what retires the ambient descriptor. Each
//! cell asserts the consumer ran (got.txt) and the ambient fd still
//! serves its file afterwards. Every cell except the control and the
//! `<>`-close sharing guard is fail-first on the unfixed base
//! (read/write EBADF, wrong /proc target).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3732-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let mut file = std::fs::File::create(dir.join("driver.sh")).expect("write driver");
    file.write_all(driver.as_bytes()).expect("write driver");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("driver.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The primitive: `File { read }` arm (`8</dev/null`). After the
/// compound, fd 8 reads the pre-compound file from its saved offset.
#[test]
fn file_read_arm_restores_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "read8",
        "printf 'l1\\nl2\\nl3\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { echo hello; } 8</dev/null > >(cat > got.txt)\n\
         cat got.txt\n\
         IFS= read -r l1 <&8; rc1=$?\n\
         IFS= read -r l2 <&8; rc2=$?\n\
         echo \"rc1=$rc1 l1=[$l1] rc2=$rc2 l2=[$l2]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hello\nrc1=0 l1=[l1] rc2=0 l2=[l2]\n");
}

/// Same shape on fd 3: the defect is not slot-specific.
#[test]
fn file_read_arm_restores_fd3() {
    let (stdout, stderr, rc) = run_driver(
        "read3",
        "printf 'x\\ny\\n' > f3.txt\n\
         exec 3<f3.txt\n\
         { echo hello; } 3</dev/null > >(cat > got.txt)\n\
         cat got.txt\n\
         IFS= read -r l <&3; r=$?\n\
         echo \"rc=$r l=[$l]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hello\nrc=0 l=[x]\n");
}

/// The `Closed` arm (`8<&-`): the bind closes fd 8 for the consumer,
/// then restores the ambient file.
#[test]
fn closed_arm_restores_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "close8",
        "printf 'l1\\nl2\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { echo hello; } 8<&- > >(cat > got.txt)\n\
         cat got.txt\n\
         IFS= read -r l <&8; r=$?\n\
         echo \"rc=$r l=[$l]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hello\nrc=0 l=[l1]\n");
}

/// The `TruncatedFile` arm (`8>trunc.txt`): the bind retires the
/// ambient write descriptor; after the compound, writes to fd 8
/// must land in the ambient file at the shared offset.
#[test]
fn truncated_write_arm_restores_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "trunc8",
        "exec 8>w8.txt\n\
         printf 'before\\n' >&8\n\
         { echo hello; } 8>trunc.txt > >(cat > got.txt)\n\
         cat got.txt\n\
         printf 'after\\n' >&8\n\
         cat w8.txt\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hello\nbefore\nafter\n");
}

/// The `File { write }` append arm (`8>>app.txt`): same write-side
/// shape without truncation.
#[test]
fn append_write_arm_restores_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "append8",
        "printf 'seed\\n' > app.txt\n\
         exec 8>w8.txt\n\
         printf 'before\\n' >&8\n\
         { echo hello; } 8>>app.txt > >(cat > got.txt)\n\
         cat got.txt\n\
         printf 'after\\n' >&8\n\
         cat w8.txt\n\
         cat app.txt\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hello\nbefore\nafter\nseed\n");
}

/// The `File { read, write }` arm (`8<>other.txt`) on a `<>`
/// ambient pair: the read arm still serves the ambient file, and a
/// write after it lands at the shared offset.
#[test]
fn readwrite_arm_restores_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "rw8",
        "printf 'x\\n' > rw.txt\n\
         printf 'o\\n' > other.txt\n\
         exec 8<>rw.txt\n\
         { echo hello; } 8<>other.txt > >(cat > got.txt)\n\
         cat got.txt\n\
         IFS= read -r l <&8; r=$?\n\
         echo \"rc=$r l=[$l]\"\n\
         printf 'after\\n' >&8\n\
         cat rw.txt\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hello\nrc=0 l=[x]\nx\nafter\n");
}

/// The `CarrierRead` arm (`8< <(...)`): the bind retires the ambient
/// read descriptor to open the input carrier. Unix-only: the
/// `<(...)` input procsub's pipe write end is not delivered on
/// Windows (pre-existing platform gap, red on base and head alike).
#[cfg(unix)]
#[test]
fn carrier_read_arm_restores_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "carrierread8",
        "printf 'l1\\nl2\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { echo hello; } 8< <(printf 'p1\\np2\\n') > >(cat > got.txt)\n\
         cat got.txt\n\
         IFS= read -r l <&8; r=$?\n\
         echo \"rc=$r l=[$l]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hello\nrc=0 l=[l1]\n");
}

/// The `CarrierWrite` arm (`8> >(...)`): the bind retires the ambient
/// write descriptor to append-open the earlier output's carrier.
#[test]
fn carrier_write_arm_restores_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "carrierwrite8",
        "exec 8>w8.txt\n\
         printf 'before\\n' >&8\n\
         { echo hello; } 8> >(cat >> cap8.txt) > >(cat > got.txt)\n\
         cat got.txt\n\
         test -e cap8.txt && echo CAP-EXISTS\n\
         printf 'after\\n' >&8\n\
         cat w8.txt\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hello\nCAP-EXISTS\nbefore\nafter\n");
}

/// Control: a compound with no fd-8 prefix redirect leaves fd 8
/// alone. Passes on base and head alike; it guards against an
/// over-correction that would disturb unbound plan fds.
#[test]
fn control_no_displacing_prefix_leaves_fd8_alone() {
    let (stdout, stderr, rc) = run_driver(
        "plain8",
        "printf 'l1\\nl2\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { echo hello; } > >(cat > got.txt)\n\
         cat got.txt\n\
         IFS= read -r l <&8; r=$?\n\
         echo \"rc=$r l=[$l]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hello\nrc=0 l=[l1]\n");
}

/// The /proc face (Linux-only): an external child spawned after the
/// compound must see fd 8 pointing at the pre-compound file — the
/// spawn pin pass must not dup2 a recycled descriptor onto it.
#[test]
#[cfg(target_os = "linux")]
fn external_child_sees_restored_fd8() {
    let (stdout, stderr, rc) = run_driver(
        "proc8",
        "printf 'F8\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { echo hello; } 8</dev/null > >(cat > got.txt)\n\
         ls -l /proc/self/fd/8\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert!(
        stdout.contains("f8.txt"),
        "fd 8 must point at f8.txt, got: {stdout}"
    );
}

/// Sharing guard (not fail-first for #3732 — it passes on the
/// unfixed base too, where the write fails because the restore is
/// dead): a `<>` pair shares ONE descriptor between its arms, so
/// `exec 8<&-` after the compound must close the whole descriptor —
/// the write must fail, exactly like GNU's close(2) on fd 8. This
/// locks the undo save's sharing preservation: duplicating the two
/// arms independently would leave the write arm live here.
#[test]
fn readwrite_pair_close_input_closes_both() {
    let (stdout, _stderr, rc) = run_driver(
        "rwclose8",
        "printf 'x\\n' > rw.txt\n\
         exec 8<>rw.txt\n\
         { echo hello; } 8<&- > >(cat > got.txt)\n\
         exec 8<&-\n\
         echo \"after\" >&8; r=$?\n\
         echo \"rc=$r\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
}
