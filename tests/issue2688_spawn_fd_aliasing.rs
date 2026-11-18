//! Issue rubash#2688 (A2-01): spawn fd aliasing — the spawn transport
//! materializes the same open file description at TWO descriptor
//! numbers in an external child. GNU: a forked child inherits exactly
//! the shell's user descriptors at their numbers (the child branch of
//! execute_disk_command closes only the shell's own bitmap fds,
//! execute_cmd.c close_fd_bitmap, and the shell keeps its bookkeeping
//! descriptors above the user range via move_to_high_fd,
//! general.c:702). Measured against GNU bash 5.3.20 with the same
//! scripts: opening `exec 3>f` adds exactly the single entry
//! `3 -> f` to a child's /proc/self/fd listing.
//!
//! Rubash's spawn pins each ambient fd-table descriptor onto its slot
//! in the child (pin_ambient_fds_in_child, external_inner.rs), fed by
//! inheritable dups of the table handles (src/fd/unix.rs
//! duplicate_handle_inheritable). On the base the child additionally
//! sees the dup's own number as an alias of the same file.
//!
//! Assertions are differential within one script run: a baseline
//! phase (no user fd open) and a later phase are compared, so
//! descriptors the test harness or the enumerator itself holds
//! (identical in both phases) cancel out. For every test file we
//! assert the exact set of descriptor numbers naming it, and that the
//! total listing grows by exactly the number of user fds opened.
//!
//! Scope note: no command-substitution cell. On the base, a comsub's
//! foreign child receives ambient descriptors only on some execution
//! paths (measured: repeated plain-path runs see no ambient fd at
//! all; one observed run saw the pin land WITH the alias) — a comsub
//! path-selection divergence of its own, recorded in the F3 lane
//! REPORT's residual observations. This pin stays scoped to the
//! aliasing mechanism itself, whose spawn paths are deterministic.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2688-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    // The enumerator: a foreign GNU bash listing its own descriptors
    // with symlink targets reduced to basenames, so an alias (same
    // file at a second number) is directly visible.
    std::fs::write(
        dir.join("enum.sh"),
        "#!/usr/bin/bash\nfor d in /proc/$$/fd/*; do n=${d##*/}; t=$(readlink \"$d\"); echo \"$n ${t##*/}\"; done\n",
    )
    .expect("write enum.sh");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
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

/// One phase of a child's fd listing: descriptor number -> target
/// basename.
type Listing = BTreeMap<u32, String>;

fn parse_phase(stdout: &str, phase: &str) -> Listing {
    let mut map = BTreeMap::new();
    let mut in_phase = false;
    for line in stdout.lines() {
        if line == phase {
            in_phase = true;
            continue;
        }
        if line.starts_with("PHASE") {
            in_phase = false;
        }
        if in_phase {
            let mut parts = line.splitn(2, ' ');
            if let (Some(n), Some(t)) = (parts.next(), parts.next()) {
                if let Ok(n) = n.parse::<u32>() {
                    map.insert(n, t.to_string());
                }
            }
        }
    }
    map
}

fn parse_listing(stdout: &str) -> Listing {
    let mut map = BTreeMap::new();
    for line in stdout.lines() {
        let mut parts = line.splitn(2, ' ');
        if let (Some(n), Some(t)) = (parts.next(), parts.next()) {
            if let Ok(n) = n.parse::<u32>() {
                map.insert(n, t.to_string());
            }
        }
    }
    map
}

/// Descriptor numbers whose target basename is `name`.
fn numbers_for(listing: &Listing, name: &str) -> Vec<u32> {
    listing
        .iter()
        .filter(|(_, t)| t.as_str() == name)
        .map(|(n, _)| *n)
        .collect()
}

fn enum_cmd(dir: &Path) -> String {
    format!("/usr/bin/bash {}", dir.join("enum.sh").display())
}

#[test]
fn issue2688_only_fd3_child_sees_exactly_fd3() {
    let dir = tempdir("only3");
    let script = format!(
        "echo PHASE0; {}; exec 3>f3.txt; echo PHASE1; {}",
        enum_cmd(&dir),
        enum_cmd(&dir)
    );
    let run = rubash_c(&script, &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    let p0 = parse_phase(&run.stdout, "PHASE0");
    let p1 = parse_phase(&run.stdout, "PHASE1");
    assert_eq!(
        numbers_for(&p1, "f3.txt"),
        vec![3],
        "fd 3's file must appear at exactly one number; p1={p1:?}"
    );
    assert_eq!(
        p1.len(),
        p0.len() + 1,
        "opening one user fd adds exactly one child descriptor; p0={p0:?} p1={p1:?}"
    );
}

#[test]
fn issue2688_only_fd5_child_sees_exactly_fd5() {
    let dir = tempdir("only5");
    let script = format!(
        "echo PHASE0; {}; exec 5>f5.txt; echo PHASE1; {}",
        enum_cmd(&dir),
        enum_cmd(&dir)
    );
    let run = rubash_c(&script, &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    let p0 = parse_phase(&run.stdout, "PHASE0");
    let p1 = parse_phase(&run.stdout, "PHASE1");
    assert_eq!(
        numbers_for(&p1, "f5.txt"),
        vec![5],
        "fd 5's file must appear at exactly one number; p1={p1:?}"
    );
    assert_eq!(
        p1.len(),
        p0.len() + 1,
        "opening one user fd adds exactly one child descriptor; p0={p0:?} p1={p1:?}"
    );
}

#[test]
fn issue2688_fds3_and_5_then_close_3() {
    let dir = tempdir("close3");
    let script = format!(
        "echo PHASE0; {}; exec 3>f3.txt 5>f5.txt; echo PHASE1; {}; exec 3>&-; echo PHASE2; {}",
        enum_cmd(&dir),
        enum_cmd(&dir),
        enum_cmd(&dir)
    );
    let run = rubash_c(&script, &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    let p0 = parse_phase(&run.stdout, "PHASE0");
    let p1 = parse_phase(&run.stdout, "PHASE1");
    let p2 = parse_phase(&run.stdout, "PHASE2");
    assert_eq!(numbers_for(&p1, "f3.txt"), vec![3], "p1={p1:?}");
    assert_eq!(numbers_for(&p1, "f5.txt"), vec![5], "p1={p1:?}");
    assert_eq!(p1.len(), p0.len() + 2, "p0={p0:?} p1={p1:?}");
    // After closing fd 3, its file must be gone entirely — in
    // particular it must not linger as an alias of fd 4's neighbour
    // or of fd 5's file.
    assert_eq!(
        numbers_for(&p2, "f3.txt"),
        Vec::<u32>::new(),
        "closed fd 3's file leaked into the child; p2={p2:?}"
    );
    assert_eq!(numbers_for(&p2, "f5.txt"), vec![5], "p2={p2:?}");
    assert_eq!(p2.len(), p0.len() + 1, "p0={p0:?} p2={p2:?}");
}

#[test]
fn issue2688_read_fd3_child_sees_exactly_fd3() {
    let dir = tempdir("read3");
    std::fs::write(dir.join("f3r.txt"), "seed\n").expect("seed");
    let script = format!(
        "echo PHASE0; {}; exec 3<f3r.txt; echo PHASE1; {}",
        enum_cmd(&dir),
        enum_cmd(&dir)
    );
    let run = rubash_c(&script, &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    let p0 = parse_phase(&run.stdout, "PHASE0");
    let p1 = parse_phase(&run.stdout, "PHASE1");
    assert_eq!(numbers_for(&p1, "f3r.txt"), vec![3], "p1={p1:?}");
    assert_eq!(p1.len(), p0.len() + 1, "p0={p0:?} p1={p1:?}");
}

#[test]
fn issue2688_varfd_boundary_no_alias() {
    // The issue lists {varname}-allocated fds as a boundary (no alias
    // observed there); in a foreign child's listing on this base the
    // same spawn mechanism aliases them too (measured: fv.txt at the
    // allocated number AND at the pin source's number). GNU contract
    // is a single number either way, so the cell asserts exactly that.
    let dir = tempdir("varfd");
    let script = format!(
        "echo PHASE0; {}; exec {{v}}>fv.txt; echo \"V=$v\"; echo PHASE1; {}",
        enum_cmd(&dir),
        enum_cmd(&dir)
    );
    let run = rubash_c(&script, &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    let v: u32 = run
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("V="))
        .and_then(|s| s.parse().ok())
        .expect("varfd number reported");
    let p0 = parse_phase(&run.stdout, "PHASE0");
    let p1 = parse_phase(&run.stdout, "PHASE1");
    assert_eq!(numbers_for(&p1, "fv.txt"), vec![v], "p1={p1:?}");
    assert_eq!(p1.len(), p0.len() + 1, "p0={p0:?} p1={p1:?}");
}

#[test]
fn issue2688_pipeline_stage_child_sees_exactly_fd3() {
    let dir = tempdir("pipe");
    let script = format!("exec 3>f3.txt; {} | cat", enum_cmd(&dir));
    let run = rubash_c(&script, &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    let listing = parse_listing(&run.stdout);
    assert_eq!(
        numbers_for(&listing, "f3.txt"),
        vec![3],
        "pipeline stage child must see fd 3's file at exactly one number; listing={listing:?}"
    );
}

#[test]
fn issue2688_chain_write_then_reopen_moves_the_single_alias_free_fd() {
    // Chain: a child writes through fd 3 (state lands in f3.txt), the
    // shell then closes 3 and reopens it onto a different file, and a
    // second child enumerates — the listing must track the shell's
    // current table exactly, with no residue of the first binding.
    let dir = tempdir("chain");
    let script = format!(
        "exec 3>f3.txt; /usr/bin/bash -c 'echo via3 >&3'; exec 3>&-; exec 3>f3b.txt; {}",
        enum_cmd(&dir)
    );
    let run = rubash_c(&script, &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(
        std::fs::read_to_string(dir.join("f3.txt")).unwrap_or_default(),
        "via3\n",
        "child write through fd 3 must land in f3.txt"
    );
    let listing = parse_listing(&run.stdout);
    assert_eq!(
        numbers_for(&listing, "f3.txt"),
        Vec::<u32>::new(),
        "stale binding leaked after reopen; listing={listing:?}"
    );
    assert_eq!(
        numbers_for(&listing, "f3b.txt"),
        vec![3],
        "listing={listing:?}"
    );
}
