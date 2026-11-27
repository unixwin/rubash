//! rubash#3938 — the kernel-landing integrity of persistent exec
//! redirects: three residuals of the #3930 placement.
//!
//! #3930 taught `exec N>file` (N below SHELL_FD_BASE) to dup2 the file
//! onto its kernel number (place_persistent_file, redir.c:1017/:1120)
//! with a placement undo log (redir.c add_undo_redirect /
//! exec_redirection_undo_list). Three shapes stayed outside it:
//!
//! - **Face A, dup redirects**: `exec 3>&1` is a table-only Rc alias —
//!   kernel descriptor 3 never exists, while GNU's duplicating
//!   redirector dup2's the source onto the redirector unconditionally
//!   (redir.c:1190, case r_duplicating_input/r_duplicating_output:
//!   "This is correct. 2>&1 means dup2 (1, 2);"). The dup face now
//!   places through the same guard suite (place_persistent_dup_arms):
//!   kernel dup2 + placement undo, the caller's clone arms kept so the
//!   per-endpoint-kind fact marks (rubash#3432) keep answering.
//! - **Face B, fd >= SHELL_FD_BASE occupied**: `exec 12>b` when 12 was
//!   already occupied stayed on the #3213 free-only materialization
//!   (refuse when the number is taken) — virtual fallback. GNU's dup2
//!   replaces the occupant unconditionally (redir.c:1017). The
//!   SHELL_FD_BASE ceiling inside place_persistent_file is lifted; the
//!   held-scan guard still refuses numbers a live {var} allocation
//!   owns, and the dir/socket refusal still protects the cwd anchor
//!   and the signal socketpair.
//! - **Face C, snapshot restore leaves a dup-relationship
//!   descriptor**: a scope save (saved_entry/saved_table,
//!   rubash#3706/#3814) duplicates each kernel-backed endpoint
//!   (F_DUPFD_CLOEXEC at/above SHELL_FD_BASE) so the bind-time retire
//!   cannot kill the description; replaying the save back into a slot
//!   whose kernel number the scope's bind vacated used to keep the
//!   entry naming the PARKED duplicate — the kernel number stayed
//!   absent and the private duplicate leaked until process end
//!   (`exec 3>f; { :; } 3</dev/null` left an extra descriptor naming f
//!   beside a kernel fd 3 that no longer existed). The compound
//!   restores now land the entry back onto its slot: dup2 the save
//!   onto the number, rebuild the arms on the landed descriptor, close
//!   the parked duplicate once nothing else names it — GNU's undo
//!   replay, a r_move record (redir.c:1399 add_undo_fd_redirect; dup2
//!   at :1190, source close at :1230).
//!
//! Every pin is the /proc observable the GNU oracle (5.3.20, WSL,
//! script-file probes) produces byte-for-byte. Unix-only: the whole
//! check IS the kernel fd table. The spawn-transient descriptors a
//! `$(ls /proc/$$/fd | wc -l)` count absorbs are deliberately not
//! pinned (see issue3640's header note) — the pins read individual
//! descriptor links and assert absence of EXTRA links naming the
//! placed file.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3938-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

/// Spawn rubash on `script` inside `dir` (the issue3640 shape: a fresh
/// top-level `-c` shell per check, real inherited descriptors absent —
/// the placement faces here do not need the hand-off harness).
fn rubash_c(dir: &Path, script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn stdout_trimmed(run: &Run) -> &str {
    run.stdout.trim()
}

// ------------------------------------------------------------------
// Face A — dup redirects materialize onto their kernel number.
// ------------------------------------------------------------------

/// `exec 3>&1`: kernel fd 3 exists and names the SAME open file
/// description as fd 1 (GNU redir.c:1190 dup2). Before the fix the dup
/// was a table-only alias: `/proc/$$/fd/3` did not exist.
#[cfg(unix)]
#[test]
fn issue3938_dup_output_3_to_1_lands_on_kernel_fd3() {
    let dir = scratch("a1-dup-out");
    let run = rubash_c(
        &dir.0,
        "exec 3>&1; [ \"$(readlink /proc/$$/fd/3)\" = \"$(readlink /proc/$$/fd/1)\" ] && echo dup-ok || echo dup-missing",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(stdout_trimmed(&run), "dup-ok", "stderr: {}", run.stderr);
}

/// `exec 3<&0`: the input-direction dup lands the same way (the source
/// is the symbolic process-stdin arm; the kernel dup2 still runs).
#[cfg(unix)]
#[test]
fn issue3938_dup_input_3_from_0_lands_on_kernel_fd3() {
    let dir = scratch("a2-dup-in");
    let run = rubash_c(
        &dir.0,
        "exec 3<&0; [ \"$(readlink /proc/$$/fd/3)\" = \"$(readlink /proc/$$/fd/0)\" ] && echo dupin-ok || echo dupin-missing",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(stdout_trimmed(&run), "dupin-ok", "stderr: {}", run.stderr);
}

/// `exec 3>&1; exec 3>&-`: the placed descriptor is owned by the slot —
/// the close drops the kernel number (GNU r_close_this,
/// redir.c:1239). Before the fix fd 3 did not exist to begin with; the
/// regression this guards is a placement whose descriptor survives the
/// close.
#[cfg(unix)]
#[test]
fn issue3938_dup_placed_descriptor_closes_with_the_slot() {
    let dir = scratch("a3-dup-close");
    let run = rubash_c(
        &dir.0,
        "exec 3>&1; exec 3>&-; [ -e /proc/$$/fd/3 ] && echo still-open || echo closed-ok",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(stdout_trimmed(&run), "closed-ok", "stderr: {}", run.stderr);
}

// ------------------------------------------------------------------
// Face B — fd >= SHELL_FD_BASE occupied: dup2 replaces the occupant.
// ------------------------------------------------------------------

/// `exec 12>a; exec 12>b`: the second placement displaces the first
/// (GNU redir.c:1017 dup2 replaces whatever held the number; the
/// displaced occupant's save dies with the exec's commit). Before the
/// fix the second exec fell back to virtual and fd 12 kept naming
/// a.txt — or nothing.
#[cfg(unix)]
#[test]
fn issue3938_exec_12_over_occupied_number_places_b() {
    let dir = scratch("b1-exec12");
    let run = rubash_c(
        &dir.0,
        "exec 12>a.txt; exec 12>b.txt; [ \"$(readlink /proc/$$/fd/12)\" = \"$PWD/b.txt\" ] && echo placed || echo fell-back",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(stdout_trimmed(&run), "placed", "stderr: {}", run.stderr);
}

/// The displaced occupant is genuinely displaced: writes through the
/// placed number land in b, never in a (the a descriptor is closed by
/// the commit — GNU disposes the exec undo list, redir.c:245-251).
#[cfg(unix)]
#[test]
fn issue3938_exec_12_write_reaches_the_placed_file_only() {
    let dir = scratch("b2-exec12-write");
    let run = rubash_c(
        &dir.0,
        "exec 12>a.txt; exec 12>b.txt; echo hi >&12; exec 12>&-; printf 'a=['; cat a.txt; printf ']b=['; cat b.txt; echo ']'",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        stdout_trimmed(&run),
        "a=[]b=[hi\n]",
        "stderr: {}",
        run.stderr
    );
}

// ------------------------------------------------------------------
// Face C — the scope restore lands the save back on the slot.
// ------------------------------------------------------------------

/// `exec 3>f; { :; } 3</dev/null`: after the compound's undo, kernel
/// fd 3 names f again (GNU restores the pre-scope descriptor at its
/// number) and NO second descriptor names f (the scope's private save
/// duplicate is closed — redir.c:1399 add_undo_fd_redirect replays a
/// r_move record: dup2 back, close the save). Before the fix fd 3
/// vanished (the table named the parked duplicate at a high number)
/// and the duplicate leaked.
#[cfg(unix)]
#[test]
fn issue3938_compound_restore_lands_the_save_on_the_slot() {
    let dir = scratch("c1-compound-restore");
    let run = rubash_c(
        &dir.0,
        "exec 3>f.txt; { :; } 3</dev/null; \
         [ \"$(readlink /proc/$$/fd/3)\" = \"$PWD/f.txt\" ] && echo landed || echo fd3-lost; \
         for f in 0 1 2 3 4 5 6 7 8 9; do readlink /proc/$$/fd/$f >>links.txt 2>/dev/null; done; \
         echo \"links=$(grep -c \"$PWD/f.txt\" links.txt)\"",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        stdout_trimmed(&run),
        "landed\nlinks=1",
        "stderr: {}",
        run.stderr
    );
}

/// The same landing across a function-call scope (`f` body runs under
/// its own compound bracket) — the restore shape the #3706/#3814 dup
/// saves serve.
#[cfg(unix)]
#[test]
fn issue3938_function_scope_restore_leaves_no_duplicate() {
    let dir = scratch("c2-func-restore");
    let run = rubash_c(
        &dir.0,
        "f() { :; }; exec 3>f.txt; f; \
         [ \"$(readlink /proc/$$/fd/3)\" = \"$PWD/f.txt\" ] && echo landed || echo fd3-lost; \
         for f in 0 1 2 3 4 5 6 7 8 9; do readlink /proc/$$/fd/$f >>links.txt 2>/dev/null; done; \
         echo \"links=$(grep -c \"$PWD/f.txt\" links.txt)\"",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        stdout_trimmed(&run),
        "landed\nlinks=1",
        "stderr: {}",
        run.stderr
    );
}
