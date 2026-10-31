//! rubash#2322 — a pending `{fd}` close carries a list position:
//! a dup positioned AFTER a same-list close of its source must fail
//! exactly as if the close had already happened, because GNU applies
//! a command's redirect list as ONE ordered walk over the live
//! descriptor table (do_redirections, redir.c:246). The dup arm
//! assigns the `{var}` variable only after the dup itself succeeds
//! (redir.c:1010/1113 varassign + the REDIR_VARASSIGN dup arm), so a
//! rejected command leaves the failing entry's variable UNSET while
//! earlier successful binds in the same list persist. A close arm
//! removes the descriptor at its position (redir.c:1219-1248), so a
//! dup BEFORE the close still copies the descriptor as bound at its
//! own position, and a fresh bind of the same slot positioned after
//! the close revives it for later entries.
//!
//! Pre-fix divergences (probes in the cluster workdir, GNU bash
//! 5.3.20 oracle, script files): on the in-process faces (builtin /
//! function) the applier serviced the doomed dup against the raw fd
//! table — pending closes were only recorded for the gate — and
//! BOUND the variable before the gate rejected the command, leaking
//! `w=[10]` where GNU leaves the variable unset. On the same-exe
//! script face, numbered OUTPUT binds ran in a separate later pass,
//! so a `{v}>&-` close of a list-bound output never reached the
//! child: the child's dup kept the ambient descriptor and the
//! "closed" output fd stayed open.
//!
//! FD-ENVIRONMENT SENSITIVITY: these pins allocate and close real
//! descriptors; `{var}` numbers depend on which fds the test
//! process inherits. The drivers are always launched as script
//! files by the harness below with stdin null and only fds 0/1/2
//! open, and the expectations were verified under that scrubbed
//! layout. On in-process faces GNU's own undo-save copies occupy
//! fds >= 10 and shift `{var}` allocations (GNU binds 11 where
//! rubash binds 10), so the in-process pins assert SET/UNSET state
//! and file contents rather than exact variable numbers; exact
//! numbers are asserted only where both shells agree (spawn faces
//! leave `{var}` unset in the parent by definition).
//!
//! Expectations byte-verified against GNU bash 5.3.20 (the oracle
//! binary, script-file probes).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2322-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, body).expect("write fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if name.ends_with(".sh") {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fixture");
        }
    }
}

/// Run `driver.sh` (with `@RUBASH@` replaced by this build's binary
/// path) as the outer shell's main script, stdin null.
fn run_driver(dir: &Path, driver: &str) -> (String, String, Option<i32>) {
    write(dir, "driver.sh", &driver.replace("@RUBASH@", RUBASH));
    let output = Command::new(RUBASH)
        .arg("driver.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash driver");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn fixtures(dir: &Path) {
    write(dir, "seed.txt", "SEED\n");
    write(dir, "f9.txt", "F9\n");
    write(dir, "other.txt", "OTHER\n");
    write(
        dir,
        "reader10.sh",
        "echo \"CHILD-RAN\"\nline=\"\"; IFS= read -r -u 10 line || line=\"READ10-FAIL\"\necho \"fd10=[$line]\"\nline=\"\"; IFS= read -r -u 9 line || line=\"READ9-FAIL\"\necho \"fd9=[$line]\"\n",
    );
    write(
        dir,
        "reader_out.sh",
        "echo \"CHILD-RAN\"\nif echo W10 >&10; then echo \"w10=OK\"; else echo \"w10=FAIL\"; fi\nif echo W9 >&9; then echo \"w9=OK\"; else echo \"w9=FAIL\"; fi\n",
    );
}

/// Headline, external spawn face: the dup's source (fd 9) is closed
/// earlier in the same list, so GNU rejects the command and the
/// child never runs; the parent's own fd 9 is untouched.
#[test]
fn headline_external_input_dup_after_close_rejected() {
    let dir = scratch("headline-ext");
    fixtures(&dir);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9<seed.txt\nv=9\n\"@RUBASH@\" reader10.sh 9<f9.txt {v}<&- {w}<&9\necho \"rc=$? w=[${w-UNSET}]\"\nline=\"\"; IFS= read -r -u 9 line || line=\"PARENT9-FAIL\"\necho \"parent9=[$line]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 w=[UNSET]\nparent9=[SEED]\n");
    assert!(
        stderr.contains("cannot duplicate fd"),
        "expected the GNU dup diagnostic, got stderr: {stderr}"
    );
}

/// Headline, same-exe script face (ENOEXEC child run in-process):
/// same rejection, child never runs.
#[test]
fn headline_script_input_dup_after_close_rejected() {
    let dir = scratch("headline-script");
    fixtures(&dir);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9<seed.txt\nv=9\n./reader10.sh 9<f9.txt {v}<&- {w}<&9\necho \"rc=$? w=[${w-UNSET}]\"\nline=\"\"; IFS= read -r -u 9 line || line=\"PARENT9-FAIL\"\necho \"parent9=[$line]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 w=[UNSET]\nparent9=[SEED]\n");
    assert!(
        stderr.contains("cannot duplicate fd"),
        "expected the GNU dup diagnostic, got stderr: {stderr}"
    );
}

/// Headline, builtin face: the rejected command must not leak the
/// failing dup's variable binding into the shell (the D2 defect:
/// rubash bound w=10 before its gate rejected the command).
#[test]
fn headline_builtin_dup_after_close_rejected_no_bind_leak() {
    let dir = scratch("headline-builtin");
    fixtures(&dir);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9<seed.txt\nv=9\nline=PRE\nread -u 10 line 9<f9.txt {v}<&- {w}<&9\necho \"rc=$? line=[$line] w=[${w-UNSET}]\"\nline=\"\"; IFS= read -r -u 9 line || line=\"PARENT9-FAIL\"\necho \"parent9=[$line]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 line=[PRE] w=[UNSET]\nparent9=[SEED]\n");
    assert!(
        stderr.contains("cannot duplicate fd"),
        "expected the GNU dup diagnostic, got stderr: {stderr}"
    );
}

/// Headline, output spelling on the builtin face.
#[test]
fn headline_builtin_output_dup_after_close_rejected_no_bind_leak() {
    let dir = scratch("headline-builtin-out");
    fixtures(&dir);
    let (stdout, _stderr, code) = run_driver(
        &dir,
        "exec 9>seedout.txt\nv=9\nline=PRE\nread -u 10 line 9>o1.txt {v}>&- {w}>&9\necho \"rc=$? w=[${w-UNSET}]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 w=[UNSET]\n");
}

/// Headline, output spelling on the function face: the function
/// body must not run at all when the walk rejects the list.
#[test]
fn headline_function_output_dup_after_close_rejected() {
    let dir = scratch("headline-func-out");
    fixtures(&dir);
    let (stdout, _stderr, code) = run_driver(
        &dir,
        "reader_out() { echo \"FUNC-RAN\"; }\nexec 9>seedout.txt\nv=9\nreader_out 9>o1.txt {v}>&- {w}>&9\necho \"rc=$? w=[${w-UNSET}]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 w=[UNSET]\n");
}

/// An EARLIER successful `{var}` bind in the same list survives the
/// rejection (GNU binds it before the walk reaches the failing
/// entry); only the failing entry's variable stays unset. The
/// surviving variable's NUMBER is allocation-dependent (GNU's
/// undo-save copies shift it), so this pins state, not the number.
#[test]
fn earlier_successful_bind_survives_rejection() {
    let dir = scratch("earlier-bind");
    fixtures(&dir);
    let (stdout, _stderr, code) = run_driver(
        &dir,
        "exec 9<seed.txt\nv=9\nline=PRE\nread -u 10 line 9<f9.txt {a}<&9 {v}<&- {w}<&9\nrc=$?\nif [ -n \"${a-}\" ]; then astate=SET; else astate=UNSET; fi\necho \"rc=$rc line=[$line] a=[$astate] w=[${w-UNSET}]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 line=[PRE] a=[SET] w=[UNSET]\n");
}

/// Mirror control, external input face: dup BEFORE the close copies
/// the descriptor as bound at its position; the close still lands
/// for the child (fd 9 closed), the parent's fd 9 is untouched.
#[test]
fn mirror_external_input_dup_before_close() {
    let dir = scratch("mirror-ext");
    fixtures(&dir);
    let (stdout, _stderr, code) = run_driver(
        &dir,
        "exec 9<seed.txt\nv=9\n\"@RUBASH@\" reader10.sh 9<f9.txt {w}<&9 {v}<&-\necho \"rc=$? w=[${w-UNSET}]\"\nline=\"\"; IFS= read -r -u 9 line || line=\"PARENT9-FAIL\"\necho \"parent9=[$line]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "CHILD-RAN\nfd10=[F9]\nfd9=[READ9-FAIL]\nrc=0 w=[UNSET]\nparent9=[SEED]\n"
    );
}

/// Mirror, output spelling, spawn face: the child's fd 10 writes
/// land in the list-bound o1.txt; fd 9 (dup source, closed after
/// the dup) delivers the dup — wait, GNU: the dup copies fd 9 as
/// bound at ITS position (the o1 bind), the later close closes
/// fd 9 itself, so the child's W9 fails and W10 lands in o1.txt.
#[test]
fn mirror_output_spawn_dup_before_close() {
    let dir = scratch("mirror-out-spawn");
    fixtures(&dir);
    let (stdout, _stderr, code) = run_driver(
        &dir,
        "exec 9>seedout.txt\nv=9\n\"@RUBASH@\" reader_out.sh 9>o1.txt {w}>&9 {v}>&-\necho \"rc=$? w=[${w-UNSET}]\"\necho \"o1=[$(cat o1.txt)]\"\necho \"seedout=[$(cat seedout.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "CHILD-RAN\nw10=OK\nw9=FAIL\nrc=0 w=[UNSET]\no1=[W10]\nseedout=[]\n"
    );
}

/// Mirror, output spelling, same-exe script face — the D1a defect:
/// pre-fix the numbered output bind ran in a separate later pass,
/// so the child saw fd 10 = the AMBIENT seedout descriptor and
/// fd 9 still open on o1.txt (W10 landed in seedout.txt, W9 in
/// o1.txt). GNU: fd 10 = o1.txt, fd 9 closed.
#[test]
fn mirror_output_script_dup_before_close() {
    let dir = scratch("mirror-out-script");
    fixtures(&dir);
    let (stdout, _stderr, code) = run_driver(
        &dir,
        "exec 9>seedout.txt\nv=9\n\"@RUBASH@\" reader_out.sh 9>o1.txt {w}>&9 {v}>&-\necho \"rc=$? w=[${w-UNSET}]\"\necho \"o1=[$(cat o1.txt)]\"\necho \"seedout=[$(cat seedout.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "CHILD-RAN\nw10=OK\nw9=FAIL\nrc=0 w=[UNSET]\no1=[W10]\nseedout=[]\n"
    );
}

/// A fresh bind of the closed slot positioned AFTER the close
/// revives the descriptor for later entries: the dup then copies
/// the NEW binding (other.txt), and the close has still happened
/// for the child's own fd 9 view of the ORIGINAL binding order —
/// GNU delivers fd 10 = OTHER and fd 9 closed.
#[test]
fn rebind_after_close_revives_for_later_dup() {
    let dir = scratch("rebind");
    fixtures(&dir);
    let (stdout, _stderr, code) = run_driver(
        &dir,
        "exec 9<seed.txt\nv=9\n\"@RUBASH@\" reader10.sh 9<f9.txt {v}<&- 9<other.txt {w}<&9\necho \"rc=$? w=[${w-UNSET}]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "CHILD-RAN\nfd10=[OTHER]\nfd9=[READ9-FAIL]\nrc=0 w=[UNSET]\n"
    );
}

/// A close of an UNRELATED descriptor does not poison a later dup:
/// the command runs and the dup copies its source normally (the
/// child's fd 9 is the list's own 9<f9.txt binding, already drained
/// by the fd 10 read — READ9-FAIL is the GNU-agreeing outcome).
#[test]
fn unrelated_close_does_not_block_dup() {
    let dir = scratch("unrelated");
    fixtures(&dir);
    let (stdout, _stderr, code) = run_driver(
        &dir,
        "exec 9<seed.txt\nexec 8<other.txt\nv=8\n\"@RUBASH@\" reader10.sh 9<f9.txt {v}<&- {w}<&9\necho \"rc=$? w=[${w-UNSET}]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "CHILD-RAN\nfd10=[F9]\nfd9=[READ9-FAIL]\nrc=0 w=[UNSET]\n");
}

/// Close-position also governs a dup whose source is bound ONLY by
/// the ambient table (no same-list bind): closing the ambient fd
/// in the list still rejects a later dup of it.
#[test]
fn ambient_close_only_rejects_later_dup() {
    let dir = scratch("ambient-close");
    fixtures(&dir);
    let (stdout, _stderr, code) = run_driver(
        &dir,
        "exec 9<seed.txt\nv=9\n\"@RUBASH@\" reader10.sh {v}<&- {w}<&9\necho \"rc=$? w=[${w-UNSET}]\"\nline=\"\"; IFS= read -r -u 9 line || line=\"PARENT9-FAIL\"\necho \"parent9=[$line]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 w=[UNSET]\nparent9=[SEED]\n");
}

/// Headline, output spelling, external face: rejected before the
/// child runs; neither output file receives anything.
#[test]
fn headline_external_output_dup_after_close_rejected() {
    let dir = scratch("headline-ext-out");
    fixtures(&dir);
    let (stdout, _stderr, code) = run_driver(
        &dir,
        "exec 9>seedout.txt\nv=9\n\"@RUBASH@\" reader_out.sh 9>o1.txt {v}>&- {w}>&9\necho \"rc=$? w=[${w-UNSET}]\"\necho \"o1=[$(cat o1.txt)]\"\necho \"seedout=[$(cat seedout.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 w=[UNSET]\no1=[]\nseedout=[]\n");
}
