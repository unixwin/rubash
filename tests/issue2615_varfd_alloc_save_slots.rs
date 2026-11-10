//! rubash#2615 — `{var}` fd allocation ran in the fd_table's model
//! space, ignoring the transient undo-save slots GNU's do_redirections
//! walk occupies in the SAME descriptor space: when an earlier entry of
//! the list rebinds a descriptor that is open at its walk position,
//! add_undo_redirect (redir.c:1296) dups it to the lowest free slot
//! >= SHELL_FD_BASE (redir.c:86) — starting above the dup source when
//! the source is >= SHELL_FD_BASE (the fdbase rule, redir.c:1303) —
//! and that save slot stays occupied for the rest of the walk, so a
//! later REDIR_VARASSIGN F_DUPFD allocation (redir.c:965/1082/1144)
//! must count it. A close entry whose redirector is open saves it the
//! same way before closing (redir.c:1253). An entry whose redirector
//! is NOT open registers only a close-undo and occupies nothing.
//!
//! Expectations verified against GNU bash 5.3.20 with identical
//! script files (fresh process, stdin null).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2615-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(dir: &PathBuf, text: &str) -> (String, String, Option<i32>) {
    let script = dir.join("probe.sh");
    std::fs::write(&script, text).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2615_exec_rebind_save_slot_counts() {
    // SP18/E1: the walk's undo-save of rebound fd 9 holds slot 10, so
    // the {v} dup allocates 11 (GNU), not 10.
    let dir = tempdir("e1");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 9>seed.txt\nexec 9>o1.txt {v}>&9\necho \"V=$v\"\n",
    );
    assert_eq!(stdout, "V=11\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_no_save_when_redirector_closed() {
    // Control: fd 9 was never open, so the entry registers only a
    // close-undo — no save slot — and {v} allocates 10 in both shells.
    let dir = tempdir("ctl");
    let (stdout, stderr, code) = run_script(&dir, "exec 9>o1.txt {v}>&9\necho \"V=$v\"\n");
    assert_eq!(stdout, "V=10\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_builtin_rebind_save_slot_counts() {
    // Same walk inside an in-process builtin command.
    let dir = tempdir("builtin");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 9>seed.txt\necho hi 9>o1.txt {v}>&9\necho \"V=$v\"\n",
    );
    assert_eq!(stdout, "hi\nV=11\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_two_rebinds_hold_two_save_slots() {
    // Two rebound open descriptors hold two save slots (10 and 11).
    let dir = tempdir("two");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 8>s8.txt\nexec 9>s9.txt\nexec 8>a.txt 9>b.txt {v}>&8\necho \"V=$v\"\n",
    );
    assert_eq!(stdout, "V=12\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_dup_save_starts_above_high_source() {
    // fdbase rule (redir.c:1303): the undo-save for `9>&12` starts its
    // search at 13, not at 10 — so slot 11 stays free for the {var}
    // allocation even though 10 and 12 are open. A save modelled from
    // SHELL_FD_BASE would take 11 and push {v} to 13.
    let dir = tempdir("fdbase");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 10>fa.txt\nexec 12>fc.txt\nexec 9>seed.txt\necho hi 9>&12 {v}>fv.txt\necho \"V=$v\"\n",
    );
    assert_eq!(stdout, "hi\nV=11\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_close_of_open_fd_holds_save_slot() {
    // A close entry saves its open redirector first (redir.c:1253):
    // the save of 11 takes slot 10, 11 is closed, and the {var}
    // allocation lands on 11 — not on the freed-then-first 10.
    let dir = tempdir("close");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 11>f11.txt\necho hi 11>&- {v}>fv.txt\necho \"V=$v\"\n",
    );
    assert_eq!(stdout, "hi\nV=11\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_stdio_redirector_save_counts() {
    // The save discipline covers the stdio redirectors too: fd 1 is
    // open, so `>f.txt` saves it at slot 10 before binding, and the
    // {var} allocation lands on 11 (GNU), not 10.
    let dir = tempdir("stdio");
    let (stdout, stderr, code) = run_script(&dir, "echo hi >f.txt {v}>g.txt\necho \"V=$v\"\n");
    assert_eq!(stdout, "V=11\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_move_entry_holds_two_saves() {
    // A move entry earns TWO saves when its redirector is open: the
    // dup save of the redirector (from above the source) and the
    // close save of the source itself (redir.c:1175) — slots 10
    // and 11 here, so {v} allocates 12.
    let dir = tempdir("move");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 9>seed.txt\nexec 8>e8.txt\necho hi 8>&9- {v}>g.txt\necho \"V=$v\"\n",
    );
    assert_eq!(stdout, "hi\nV=12\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_move_source_close_frees_its_slot() {
    // The move's source close frees the source's slot for the same
    // walk: saves land at 12 and 13 (10 and 11 open), the move
    // closes 10, and the {var} allocation takes the freed 10.
    let dir = tempdir("movefree");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 9>seed.txt\nexec 10>e10.txt\nexec 11>e11.txt\necho hi 9>&10- {v}>g.txt\necho \"V=$v\"\n",
    );
    assert_eq!(stdout, "hi\nV=10\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_walk_close_frees_ambient_slot() {
    // A close entry frees its ambient descriptor for the walk's
    // later allocations (its own save sits at 11 and does not
    // block): `cmd 10>&- {v}>g` allocates v=10 in GNU.
    let dir = tempdir("closefree");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 10>f.txt\necho hi 10>&- {v}>g.txt\necho \"V=$v\"\n",
    );
    assert_eq!(stdout, "hi\nV=10\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_save_lives_for_the_commands_duration() {
    // The save is a real dup that stays open while the owning
    // command runs: a `{var}` allocation inside a group or function
    // body whose command list rebound an open descriptor counts
    // the save slot too. Group shape: ambient {9}, the group's
    // save lands at 10, so v=11. The builtin leaf's `{v}` fd is
    // NOT closed when the group ends — GNU keeps it open (oracle-
    // verified: /proc/$$/fd still lists 11 after the group and
    // the descriptor stays writable) — so at the function call
    // the ambient space is {9, 11}, the call's save lands at 10,
    // and w allocates past both: F=12. (An earlier draft of this
    // cell expected F=11 from a sequence where an unrelated
    // persistent fd 12 changed the ambient space; the oracle on
    // THIS script prints 12.)
    let dir = tempdir("body");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 9>seed.txt\n{ {v}>inner.txt echo hi; echo \"G=$v\"; } 9>o1.txt\nfn() { {w}>inner2.txt echo hi; echo \"F=$w\"; }\nfn 9>o2.txt\n",
    );
    assert_eq!(stdout, "hi\nG=11\nhi\nF=12\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_fork_child_walks_count_no_saves() {
    // The saves counted by the cells above exist only in an
    // UNDOABLE walk (redir.c:243 gates the whole undo machinery on
    // RX_UNDOABLE). A forked child's walks run RX_ACTIVE only — an
    // external command's list (execute_cmd.c:5522) and a subshell's
    // own list (execute_cmd.c:1761) — so there the SAME list shapes
    // allocate without the save: the external child's dup lands on
    // fd 10 (its reader reads fd 10), and a `{var}` allocation in a
    // subshell body whose list rebound an open fd allocates 10,
    // not 11. GNU-verified with this script (THIS_SH pointing at
    // the shell under test on both sides).
    let dir = tempdir("fork");
    std::fs::write(dir.join("seed.txt"), "SEED\n").expect("write seed");
    std::fs::write(dir.join("f9.txt"), "F9\n").expect("write f9");
    std::fs::write(
        dir.join("reader.sh"),
        "IFS= read -r -u 10 line || line=\"READ10-FAIL\"\necho \"CHILD10=[$line]\"\n",
    )
    .expect("write reader");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 9<seed.txt\n\"${THIS_SH}\" reader.sh 9<f9.txt {w}<&9\necho \"rc=$? w=[${w-UNSET}]\"\n( {v}>isub.txt echo hi; echo \"SUBV=$v\" ) 9>osub.txt\n",
    );
    assert_eq!(
        stdout, "CHILD10=[F9]\nrc=0 w=[UNSET]\nhi\nSUBV=10\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2615_ambient_occupancy_control() {
    // S08 control: persistent occupancy already agreed — ambient 10
    // and 11 open, a bare {var} bind allocates 12 in both shells.
    let dir = tempdir("s08");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 10>amb10.txt\nexec 11>amb11.txt\n{v}>o1.txt echo hi\necho \"V=$v\"\n",
    );
    assert_eq!(stdout, "hi\nV=12\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
