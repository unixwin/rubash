//! rubash#3731: a persistent output process substitution's deferred
//! body must leave the shell's fd 0 reading the pre-body target from
//! its saved offset — after the substitution closes, fd 0 is not
//! restored to the outer file.
//!
//! GNU ground truth: the substitution body is the child GNU forked at
//! open time; its stdio is the shell's as of the fork, and the shell's
//! own fd 0 is untouched by the body — a read after the substitution
//! closes serves the outer file's next line (o2, then o3). Rubash's
//! `execute_persistent_output_process_substitution`
//! (src/executor/trap_exec.rs:7357) saved only a bare `Rc` clone of
//! the fd-0 entry, so the bind-time retire (retire_read_arm /
//! retire_write_arm, rubash#3213) closed the very descriptor the
//! restore was about to replay — the restore re-inserted a retired
//! entry, and the next read off fd 0 failed EBADF. Same hazard as
//! #3706 (group redirects) and #3727 (function-call stdin), on the
//! procsub-body-parse face; same fix (kernel-dup snapshot via
//! `FdTable::saved_entry`, GNU `fcntl (fd, F_DUPFD, SHELL_FD_BASE)`,
//! redir.c:965/1082/1144).
//!
//! Trigger discipline: a `>(...)` on a command's fd 1 is persistent
//! — the deferred body does NOT run at the command's end (it runs at
//! the fd's close), so a bare `echo hello > >(:)` leaves the reads
//! ahead of the body and the cells pass vacuously. Every regression
//! cell below therefore binds the substitution to fd 3 and closes it
//! explicitly (`exec 3>&-`), which runs the deferred body
//! synchronously (close_persistent_output_fd) before the post-close
//! reads. Nine cells are fail-first on the unfixed base (measured
//! on Linux, base f56716b5: 2 passed / 9 failed): eight regression
//! cells (primitive, body-reads, cat, compound, twice, readwrite,
//! loop, nested — post-close read rc=1, empty line) plus devnull,
//! which fails on the EBADF diagnostic itself (`read: 0: read error:
//! Bad file descriptor` on stderr) even though its rc and stdout
//! match. The no-procsub control and the implicit-close documentation
//! cell pass on base too.
//! Unix-only hazard: the bind-time retire (retire_read_arm /
//! retire_write_arm) is `#[cfg(unix)]` — a no-op on Windows — so
//! the fail-first leg MUST run on Linux; on Windows the pin passes
//! on base by construction and only the no-regression leg is
//! meaningful there.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3731-{}-{tag}", std::process::id()));
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

/// The primitive: bind `>(:)` to fd 3, write, close explicitly — the
/// deferred body runs at the close, binds fd 0 to the carrier text
/// for the body's duration, and must restore the outer file
/// afterwards — o2, then o3, both rc=0.
#[test]
fn persistent_output_procsub_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "primitive",
        "printf 'o1\\no2\\no3\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         exec 3> >(:)\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo hello >&3\n\
         exec 3>&-\n\
         IFS= read -r l2 <&0; r2=$?\n\
         IFS= read -r l3 <&0; r3=$?\n\
         echo \"r1=$r1 l1=[$l1] r2=$r2 l2=[$l2] r3=$r3 l3=[$l3]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "r1=0 l1=[o1] r2=0 l2=[o2] r3=0 l3=[o3]\n");
}

/// The body reads the carrier text off fd 0 ("hello") at the close,
/// then the outer file resumes — the bind itself works and the
/// restore after it works too.
#[test]
fn procsub_body_reads_carrier_text_then_restore() {
    let (stdout, stderr, rc) = run_driver(
        "bodyreads",
        "printf 'o1\\no2\\no3\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         exec 3> >(IFS= read -r x; echo \"in-body:[$x]\")\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo hello >&3\n\
         exec 3>&-\n\
         IFS= read -r l2 <&0; r2=$?\n\
         IFS= read -r l3 <&0; r3=$?\n\
         echo \"r1=$r1 l1=[$l1] r2=$r2 l2=[$l2] r3=$r3 l3=[$l3]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "in-body:[hello]\nr1=0 l1=[o1] r2=0 l2=[o2] r3=0 l3=[o3]\n"
    );
}

/// `>(cat)`: the body prints the carrier text at the close, then the
/// outer file resumes.
#[test]
fn procsub_cat_body_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "catbody",
        "printf 'o1\\no2\\no3\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         exec 3> >(cat)\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo hello >&3\n\
         exec 3>&-\n\
         IFS= read -r l2 <&0; r2=$?\n\
         IFS= read -r l3 <&0; r3=$?\n\
         echo \"r1=$r1 l1=[$l1] r2=$r2 l2=[$l2] r3=$r3 l3=[$l3]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hello\nr1=0 l1=[o1] r2=0 l2=[o2] r3=0 l3=[o3]\n");
}

/// Compound body: `>(echo a; echo b)` prints both lines at the
/// close, then the outer file resumes. (A `{ ...; }` group as the
/// procsub body — `>({ echo a; echo b; })` — is valid GNU but rubash
/// rejects it as "ambiguous redirect": a separate parser gap, not
/// this issue; recorded in REPORT §4.)
#[test]
fn compound_procsub_body_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "compound",
        "printf 'o1\\no2\\no3\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         exec 3> >(echo a; echo b)\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo hello >&3\n\
         exec 3>&-\n\
         IFS= read -r l2 <&0; r2=$?\n\
         IFS= read -r l3 <&0; r3=$?\n\
         echo \"r1=$r1 l1=[$l1] r2=$r2 l2=[$l2] r3=$r3 l3=[$l3]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a\nb\nr1=0 l1=[o1] r2=0 l2=[o2] r3=0 l3=[o3]\n");
}

/// Two sequential substitutions on fd 3: each close saves and
/// restores fd 0.
#[test]
fn two_sequential_procsubs_restore_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "twice",
        "printf 'o1\\no2\\no3\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         IFS= read -r l1 <&0; r1=$?\n\
         exec 3> >(:); echo one >&3; exec 3>&-\n\
         exec 3> >(:); echo two >&3; exec 3>&-\n\
         IFS= read -r l2 <&0; r2=$?\n\
         IFS= read -r l3 <&0; r3=$?\n\
         echo \"r1=$r1 l1=[$l1] r2=$r2 l2=[$l2] r3=$r3 l3=[$l3]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "r1=0 l1=[o1] r2=0 l2=[o2] r3=0 l3=[o3]\n");
}

/// `exec 0<>outer.txt` (read-write pair, one open file description)
/// restores fd 0 the same way — the kernel-dup snapshot preserves
/// the pair's sharing.
#[test]
fn readwrite_outer_stdin_restores_after_procsub() {
    let (stdout, stderr, rc) = run_driver(
        "readwrite",
        "printf 'o1\\no2\\no3\\n' > outer.txt\n\
         exec 0<>outer.txt\n\
         exec 3> >(:)\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo hello >&3\n\
         exec 3>&-\n\
         IFS= read -r l2 <&0; r2=$?\n\
         IFS= read -r l3 <&0; r3=$?\n\
         echo \"r1=$r1 l1=[$l1] r2=$r2 l2=[$l2] r3=$r3 l3=[$l3]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "r1=0 l1=[o1] r2=0 l2=[o2] r3=0 l3=[o3]\n");
}

/// End-to-end: a `while read` loop whose body closes a substitution
/// every iteration must keep reading the outer file — o1, o2, o3.
#[test]
fn loop_procsub_restores_outer_stdin_each_iteration() {
    let (stdout, stderr, rc) = run_driver(
        "loop",
        "printf 'o1\\no2\\no3\\n' > outer.txt\n\
         while IFS= read -r l; do exec 3> >(:); echo x >&3; exec 3>&-; echo \"after:[$l]\"; done < outer.txt\n\
         echo \"loop-done\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "after:[o1]\nafter:[o2]\nafter:[o3]\nloop-done\n");
}

/// Nesting: `g` binds and closes the substitution; the outer file
/// survives the function frame's save/restore.
#[test]
fn nested_procsub_call_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "nested",
        "printf 'o1\\no2\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         g() { exec 3> >(:); echo hello >&3; exec 3>&-; echo \"in-g\"; }\n\
         g\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-g\nr1=0 l1=[o1]\n");
}

/// `exec 0</dev/null` (non-regular target) restores fd 0 — the
/// post-close read is a clean EOF, not an EBADF error.
#[test]
fn dev_null_outer_stdin_restores_after_procsub() {
    let (stdout, stderr, rc) = run_driver(
        "devnull",
        "exec 0</dev/null\n\
         exec 3> >(:)\n\
         echo hello >&3\n\
         exec 3>&-\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "r1=1 l1=[]\n");
}

/// Control: no substitution — the reads serve o1 then o2. Guards the
/// fix against changing the inherit path.
#[test]
fn control_no_procsub_reads_continue() {
    let (stdout, stderr, rc) = run_driver(
        "control",
        "printf 'o1\\no2\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         IFS= read -r l1 <&0; r1=$?\n\
         IFS= read -r l2 <&0; r2=$?\n\
         echo \"r1=$r1 l1=[$l1] r2=$r2 l2=[$l2]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "r1=0 l1=[o1] r2=0 l2=[o2]\n");
}

/// Documentation: without an explicit close the body runs at shell
/// exit, after the reads — the reads still serve o1/o2/o3 and the
/// body output follows. Guards the fix against changing the
/// implicit-close timing.
#[test]
fn implicit_close_body_runs_at_exit_reads_still_fine() {
    let (stdout, stderr, rc) = run_driver(
        "implicit",
        "printf 'o1\\no2\\no3\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo hello > >(IFS= read -r x; echo \"in-body:[$x]\")\n\
         IFS= read -r l2 <&0; r2=$?\n\
         IFS= read -r l3 <&0; r3=$?\n\
         echo \"r1=$r1 l1=[$l1] r2=$r2 l2=[$l2] r3=$r3 l3=[$l3]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "r1=0 l1=[o1] r2=0 l2=[o2] r3=0 l3=[o3]\nin-body:[hello]\n"
    );
}
