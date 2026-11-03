//! rubash#2612 (subsumes #2607) — the in-process `{var}` applier was
//! not GNU's walk: it iterated only the redirect list's `{var}`
//! entries and resolved every dup source against the AMBIENT
//! (persistent) fd table, so a fixed entry earlier in the SAME list
//! that rebound the source descriptor was invisible to the copy
//! (`f 9>o1.txt {v}>&9` delivered the caller's old fd-9 binding at
//! the var's slot — #2607's reproducer). GNU's model is ONE ordered
//! walk over ONE evolving descriptor space (redir.c:237
//! do_redirections; each entry applies at its position,
//! do_redirection_internal redir.c:790; the REDIR_VARASSIGN dup arm
//! copies the descriptor as bound at its position).
//!
//! The fix gives the applier that walk: the funnel (and the compound
//! scope loop) record every fixed entry into an evolving descriptor
//! space as the walk passes it, and a `{var}` dup resolves its
//! source against that space — replaying an earlier entry's file
//! binding, copying the table slot an earlier entry's binding lives
//! in, or failing closed when an earlier entry closed the source.
//! The spawn delivery ladder needed no second walk: with the
//! applier's records corrected, the external-command row resolves
//! through the existing walk_opened/stamp steps.
//!
//! Cases are the fd-mechanism audit's battery rows S01/S05/S16/S17
//! (in-process faces), its B1 spawn row, and the A2/A3 arms of
//! #2607's reproducer family, with generalized names/values.
//! Expectations were produced by running the identical script files
//! under GNU Bash 5.3.20. The allocated descriptor NUMBER is
//! deliberately not pinned for the in-process faces: GNU's walk
//! transiently occupies the first free slot with its undo save, so
//! GNU reports 11 where an implementation without that save
//! reports 10 — the number is the audit's cluster-E divergence,
//! not this issue's. What is pinned: file outcomes, payloads, exit
//! codes, whether the variable is bound afterwards (bound for
//! in-process faces, unbound after a subshell — fork semantics),
//! and post-command descriptor state.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2612-varfd-walk-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).expect("write fixture");
}

/// Run `driver.sh` as the outer shell's main script, stdin null.
fn run_driver(dir: &Path, driver: &str) -> (String, String, Option<i32>) {
    write(dir, "driver.sh", driver);
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

const DATA: &str = "D1\nD2\nD3\n";
const DATA2: &str = "E1\nE2\n";

/// Output-side walk case: an ambient `exec 9>seed.txt`, a writer
/// that emits PAY through `$v`, and one command line carrying the
/// redirect list under test. The post-command variable state GNU
/// reports (bound in-process, unbound after a subshell) is asserted
/// by expect_output's `v_state`.
fn output_case(tag: &str, command_line: &str) -> (String, String, Option<i32>) {
    let dir = scratch(tag);
    write(&dir, "data.txt", DATA);
    write(&dir, "data2.txt", DATA2);
    let driver = format!(
        "exec 9>seed.txt\nwv() {{ echo PAY >&$v; }}\n{command_line}\necho \"RC=$?\"\nif [ -n \"${{v-}}\" ]; then echo \"V=BOUND\"; else echo \"V=UNBOUND\"; fi\necho \"out1=[$(cat out1.txt 2>/dev/null)]\"\necho \"seed=[$(cat seed.txt 2>/dev/null)]\"\n"
    );
    run_driver(&dir, &driver)
}

fn expect_output(got: (String, String, Option<i32>), v_state: &str, out1: &str, seed: &str) {
    let (stdout, stderr, code) = got;
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        format!("RC=0\nV={v_state}\nout1=[{out1}]\nseed=[{seed}]\n")
    );
    assert_eq!(stderr, "");
}

/// Input-side walk case: an ambient fd-9 binding (write for S16,
/// read for S17), a reader that cats through `$v`, and one command
/// line carrying the redirect list under test.
fn input_case(tag: &str, ambient: &str, command_line: &str) -> (String, String, Option<i32>) {
    let dir = scratch(tag);
    write(&dir, "data.txt", DATA);
    write(&dir, "data2.txt", DATA2);
    let driver = format!(
        "{ambient}\nrv() {{ cat <&$v; }}\n{command_line}\necho \"RC=$?\"\nif [ -n \"${{v-}}\" ]; then echo \"V=BOUND\"; else echo \"V=UNBOUND\"; fi\n"
    );
    run_driver(&dir, &driver)
}

fn expect_input(got: (String, String, Option<i32>), payload: &str, v_state: &str) {
    let (stdout, stderr, code) = got;
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{payload}RC=0\nV={v_state}\n"));
    assert_eq!(stderr, "");
}

// ----------------------------------------------------------------
// S01: `9>out1.txt {v}>&9` — the dup must copy the SAME LIST's
// rebind of fd 9, not the ambient seed binding (#2607's shape).
// ----------------------------------------------------------------

#[test]
fn s01_fn_dup_copies_same_list_rebind() {
    expect_output(
        output_case("s01-fn", "wv 9>out1.txt {v}>&9"),
        "BOUND",
        "PAY",
        "",
    );
}

#[test]
fn s01_builtin_dup_copies_same_list_rebind() {
    expect_output(
        output_case("s01-builtin", "eval 'echo PAY >&$v' 9>out1.txt {v}>&9"),
        "BOUND",
        "PAY",
        "",
    );
}

#[test]
fn s01_group_dup_copies_same_list_rebind() {
    expect_output(
        output_case("s01-group", "{ echo PAY >&$v; } 9>out1.txt {v}>&9"),
        "BOUND",
        "PAY",
        "",
    );
}

#[test]
fn s01_subshell_dup_copies_same_list_rebind() {
    expect_output(
        output_case("s01-sub", "( echo PAY >&$v ) 9>out1.txt {v}>&9"),
        "UNBOUND",
        "PAY",
        "",
    );
}

// ----------------------------------------------------------------
// S05: `9>&- 9>out1.txt {v}>&9` — close, rebind, then dup: the copy
// resolves against the space the close+rebind evolved (A3's
// close/reopen arm).
// ----------------------------------------------------------------

#[test]
fn s05_fn_dup_after_close_and_rebind() {
    expect_output(
        output_case("s05-fn", "wv 9>&- 9>out1.txt {v}>&9"),
        "BOUND",
        "PAY",
        "",
    );
}

#[test]
fn s05_builtin_dup_after_close_and_rebind() {
    expect_output(
        output_case("s05-builtin", "eval 'echo PAY >&$v' 9>&- 9>out1.txt {v}>&9"),
        "BOUND",
        "PAY",
        "",
    );
}

#[test]
fn s05_group_dup_after_close_and_rebind() {
    expect_output(
        output_case("s05-group", "{ echo PAY >&$v; } 9>&- 9>out1.txt {v}>&9"),
        "BOUND",
        "PAY",
        "",
    );
}

#[test]
fn s05_subshell_dup_after_close_and_rebind() {
    expect_output(
        output_case("s05-sub", "( echo PAY >&$v ) 9>&- 9>out1.txt {v}>&9"),
        "UNBOUND",
        "PAY",
        "",
    );
}

// ----------------------------------------------------------------
// S02 control: `{v}>&9 9>out1.txt` — the dup comes FIRST, so it
// keeps the ambient binding even though the list later rebinds 9.
// Guards against an over-eager "list wins" implementation: the
// walk is positional, not list-global.
// ----------------------------------------------------------------

#[test]
fn s02_control_dup_before_rebind_keeps_ambient() {
    expect_output(
        output_case("s02-fn", "wv {v}>&9 9>out1.txt"),
        "BOUND",
        "",
        "PAY",
    );
}

// ----------------------------------------------------------------
// S16: `9<data.txt {v}<&9` with an ambient WRITE binding on 9 —
// the list's read bind replaces it before the input dup resolves
// (A2's input arm; the ambient direction must not leak).
// ----------------------------------------------------------------

#[test]
fn s16_fn_input_dup_copies_same_list_read_bind() {
    expect_input(
        input_case("s16-fn", "exec 9>seed.txt", "rv 9<data.txt {v}<&9"),
        DATA,
        "BOUND",
    );
}

#[test]
fn s16_builtin_input_dup_copies_same_list_read_bind() {
    expect_input(
        input_case(
            "s16-builtin",
            "exec 9>seed.txt",
            "eval 'cat <&$v' 9<data.txt {v}<&9",
        ),
        DATA,
        "BOUND",
    );
}

#[test]
fn s16_group_input_dup_copies_same_list_read_bind() {
    expect_input(
        input_case(
            "s16-group",
            "exec 9>seed.txt",
            "{ cat <&$v; } 9<data.txt {v}<&9",
        ),
        DATA,
        "BOUND",
    );
}

#[test]
fn s16_subshell_input_dup_copies_same_list_read_bind() {
    expect_input(
        input_case(
            "s16-sub",
            "exec 9>seed.txt",
            "( cat <&$v ) 9<data.txt {v}<&9",
        ),
        DATA,
        "UNBOUND",
    );
}

// ----------------------------------------------------------------
// S17: `9<data2.txt {v}<&9` with an ambient READ binding on 9 —
// the dup must deliver data2's content, not the ambient file's.
// ----------------------------------------------------------------

#[test]
fn s17_fn_input_dup_prefers_same_list_bind() {
    expect_input(
        input_case("s17-fn", "exec 9<data.txt", "rv 9<data2.txt {v}<&9"),
        DATA2,
        "BOUND",
    );
}

#[test]
fn s17_builtin_input_dup_prefers_same_list_bind() {
    expect_input(
        input_case(
            "s17-builtin",
            "exec 9<data.txt",
            "eval 'cat <&$v' 9<data2.txt {v}<&9",
        ),
        DATA2,
        "BOUND",
    );
}

#[test]
fn s17_group_input_dup_prefers_same_list_bind() {
    expect_input(
        input_case(
            "s17-group",
            "exec 9<data.txt",
            "{ cat <&$v; } 9<data2.txt {v}<&9",
        ),
        DATA2,
        "BOUND",
    );
}

#[test]
fn s17_subshell_input_dup_prefers_same_list_bind() {
    expect_input(
        input_case(
            "s17-sub",
            "exec 9<data.txt",
            "( cat <&$v ) 9<data2.txt {v}<&9",
        ),
        DATA2,
        "UNBOUND",
    );
}

// ----------------------------------------------------------------
// Dup chain: `9>out1.txt 8>&9 {v}>&8` — the {var} source is itself
// bound by an earlier dup entry of the same list; resolution is
// transitive through the walk space.
// ----------------------------------------------------------------

#[test]
fn dup_chain_through_fixed_dup_entry() {
    expect_output(
        output_case("x1", "wv 9>out1.txt 8>&9 {v}>&8"),
        "BOUND",
        "PAY",
        "",
    );
}

// ----------------------------------------------------------------
// Move chain: `8>&9- {v}>&8` — the fixed move binds 8 from the
// ambient 9 and closes 9 at its position; the {var} dup then
// copies 8's binding. GNU leaves the caller's fd 9 closed after
// the call (the move has no undo record for its source), which
// the trailing probe pins.
// ----------------------------------------------------------------

#[test]
fn move_chain_var_dup_copies_moved_binding() {
    let dir = scratch("x2");
    write(&dir, "data.txt", DATA);
    write(&dir, "data2.txt", DATA2);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9>seed.txt\nwv() { echo PAY >&$v; }\nwv 8>&9- {v}>&8\necho \"RC=$?\"\nif [ -n \"${v-}\" ]; then echo \"V=BOUND\"; else echo \"V=UNBOUND\"; fi\necho \"seed=[$(cat seed.txt 2>/dev/null)]\"\nif echo POST >&9 2>/dev/null; then echo \"nine=open\"; else echo \"nine=closed\"; fi\necho \"seed2=[$(cat seed.txt 2>/dev/null)]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "RC=0\nV=BOUND\nseed=[PAY]\nnine=closed\nseed2=[PAY]\n"
    );
    // GNU also leaks the trailing probe's diagnostic: redirections
    // process left to right, so `>&9` fails before `2>/dev/null` is
    // applied (oracle-verified byte-identical, bash 5.3.20).
    assert_eq!(stderr, "driver.sh: line 7: 9: Bad file descriptor\n");
}

// ----------------------------------------------------------------
// Move-source undo rule (the #2612 repair): GNU's move arm adds
// the source's undo record — add_undo_redirect (redir_fd,
// r_close_this, -1), redir.c:1175 — ONLY when the redirector is
// open at the entry's walk position (the fcntl (redirector,
// F_GETFD) test, redir.c:1173). The move chain above pins the
// inactive half; the three cases below pin the active half and
// the sibling scope/arm faces of the same rule.
// ----------------------------------------------------------------

/// Same move chain, but fd 8 is already open when the call runs:
/// the redirector is active at the move's position, so GNU DOES
/// record the source's undo and fd 9 comes back after the call —
/// and fd 8 returns to its own prior binding.
#[test]
fn move_chain_target_already_open_restores_source() {
    let dir = scratch("x2b");
    write(&dir, "data.txt", DATA);
    write(&dir, "data2.txt", DATA2);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9>seed.txt\nexec 8>other.txt\nwv() { echo PAY >&$v; }\nwv 8>&9- {v}>&8\necho \"RC=$?\"\nif [ -n \"${v-}\" ]; then echo \"V=BOUND\"; else echo \"V=UNBOUND\"; fi\nif echo POST9 >&9 2>/dev/null; then echo \"nine=open\"; else echo \"nine=closed\"; fi\nif echo POST8 >&8 2>/dev/null; then echo \"eight=open\"; else echo \"eight=closed\"; fi\necho \"seed=[$(cat seed.txt 2>/dev/null)]\"\necho \"other=[$(cat other.txt 2>/dev/null)]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "RC=0\nV=BOUND\nnine=open\neight=open\nseed=[PAY\nPOST9]\nother=[POST8]\n"
    );
    assert_eq!(stderr, "");
}

/// The group face of the inactive half: a brace group's own
/// `8>&9-` (redirector never bound) closes the caller's fd 9 for
/// good, exactly like the function face. (The probe writes its
/// failing redirect with fd 2 pre-bound to /dev/null so the case
/// pins descriptor state, not the diagnostic channel.)
#[test]
fn group_move_with_inactive_redirector_closes_source() {
    let dir = scratch("x3");
    write(&dir, "data.txt", DATA);
    write(&dir, "data2.txt", DATA2);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9>seed.txt\n{ echo IN >&8; } 8>&9-\necho \"RC=$?\"\nif echo POST 2>/dev/null >&9; then echo \"nine=open\"; else echo \"nine=closed\"; fi\necho \"seed=[$(cat seed.txt 2>/dev/null)]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "RC=0\nnine=closed\nseed=[IN]\n");
    assert_eq!(stderr, "");
}

/// The input arm of the rule, active half: `8<&9-` on a function
/// call whose redirector fd 8 is already open — GNU records the
/// source's undo, so fd 9 is readable again after the call (at
/// the shared description's offset, hence at EOF after the body
/// drained it), and fd 8 keeps its own binding.
#[test]
fn input_move_target_already_open_restores_source() {
    let dir = scratch("x4");
    write(&dir, "data.txt", DATA);
    write(&dir, "data2.txt", DATA2);
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9<data.txt\nexec 8<data2.txt\nrv() { cat <&$v; }\nrv 8<&9- {v}<&8\necho \"RC=$?\"\nif [ -n \"${v-}\" ]; then echo \"V=BOUND\"; else echo \"V=UNBOUND\"; fi\nif cat <&9 2>/dev/null; then echo \"nine=open\"; else echo \"nine=closed\"; fi\nif cat <&8 2>/dev/null; then echo \"eight=open\"; else echo \"eight=closed\"; fi\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "D1\nD2\nD3\nRC=0\nV=BOUND\nnine=open\nE1\nE2\neight=open\n"
    );
    assert_eq!(stderr, "");
}

// ----------------------------------------------------------------
// B1 (spawn row): an external command whose list rebinds 9, dups
// it into {v}, and points fd 1 at the var's slot (`1>&10` in the
// audit fixture; here the writer's own $v slot via a second dup
// target is avoided — the child writes through fd 1, redirected
// onto the descriptor the walk bound for the var). GNU: the
// child's PAY lands in out1.txt and the ambient seed stays empty.
// ----------------------------------------------------------------

#[cfg(unix)]
#[test]
fn b1_external_spawn_dup_copies_same_list_rebind() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("b1");
    write(&dir, "data.txt", DATA);
    write(&dir, "data2.txt", DATA2);
    write(&dir, "w.sh", "echo PAY\n");
    let mut perms = fs::metadata(dir.join("w.sh"))
        .expect("stat w.sh")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(dir.join("w.sh"), perms).expect("chmod w.sh");
    // The var's descriptor number is the shell's own allocation;
    // the driver learns it from a prior identical in-process bind
    // is NOT stable across implementations, so the child instead
    // receives fd 1 dup'd from the var slot by number the DRIVER
    // prints after a probe bind. Simpler and number-free: the
    // child script writes through the var-named slot by having the
    // driver pass the list as `9>out1.txt {v}>&9 1>&$v` — no: $v
    // expands in the parent before the list applies. The audit's
    // B1 spells the var's slot literally (10 on both engines for
    // this sparse list); keep that spelling.
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9>seed.txt\n./w.sh 9>out1.txt {v}>&9 1>&10\necho \"RC=$?\"\necho \"out1=[$(cat out1.txt 2>/dev/null)]\"\necho \"seed=[$(cat seed.txt 2>/dev/null)]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "RC=0\nout1=[PAY]\nseed=[]\n");
    assert_eq!(stderr, "");
}
