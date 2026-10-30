//! rubash#2287 — the job table's recorded pgrp for a background
//! pipeline must name a REAL process group containing every stage.
//!
//! The issue alleged a fiction: pipeline_exec.rs stage spawns carry
//! no setpgid, so (in GNU's in-shell model) the recorded pgrp would
//! name a group nobody belongs to, and hangup_all_jobs' killpg —
//! plus a user's `kill -- -<pgrp>` — would ESRCH while GNU delivers.
//!
//! rubash's background pipeline is not run in-shell: the job is the
//! grouped background wrapper (#2251 wired its spawn with
//! new_process_group under monitor mode), and every pipeline stage
//! is a child of that wrapper that inherits its group. The recorded
//! pgrp (the wrapper's pid, surfaced by `jobs -p`) is therefore a
//! real, populated group. These pins lock that invariant at the
//! /proc level so a future refactor that detaches the stages from
//! the job's group (per-stage regrouping, a detached stage spawner)
//! fails here instead of silently reviving the #2287 shape:
//!
//!   * monitor on: `jobs -p` names the wrapper's pid, the wrapper
//!     leads a real group with that pgid, BOTH stage processes sit
//!     in it, and killpg(recorded, SIGHUP) kills every stage — the
//!     exact delivery hangup_all_jobs (trap_exec.rs) relies on.
//!   * monitor off (control): nothing is grouped — the wrapper and
//!     both stages keep the shell's own ambient pgid, matching the
//!     pre-#2251 / no-job-control shape where the recorded pgrp is
//!     0 and hangup_all_jobs skips the job.
//!
//! GNU bash 5.3.20 reference for the same cells (login route probed
//! separately, see the cluster probes): stages are direct children
//! of the shell sharing pgid = first stage pid; huponexit kills
//! them on exit, and without huponexit they survive — the same
//! observable outcomes rubash reaches through the wrapper group.
//!
//! Linux-only: the assertions read /proc and use killpg.

#![cfg(target_os = "linux")]

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2287-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[derive(Clone, Debug)]
struct ProcRow {
    pid: u32,
    ppid: u32,
    pgid: u32,
    comm: String,
}

fn proc_row(pid: u32) -> Option<ProcRow> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm may contain spaces/parens; fields after the last ") " are
    // state, ppid, pgrp, ...
    let after = stat.rsplit(") ").next()?;
    let mut fields = after.split_whitespace();
    let _state = fields.next()?;
    let ppid: u32 = fields.next()?.parse().ok()?;
    let pgid: u32 = fields.next()?.parse().ok()?;
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()?
        .trim()
        .to_string();
    Some(ProcRow {
        pid,
        ppid,
        pgid,
        comm,
    })
}

/// All descendants of `root` (including root), from a full /proc
/// ppid snapshot — the per-task children file is racy against
/// short-lived spawns.
fn proc_tree(root: u32) -> Vec<ProcRow> {
    let mut table: Vec<ProcRow> = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() {
                if let Some(row) = proc_row(pid) {
                    table.push(row);
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut frontier = vec![root];
    while let Some(pid) = frontier.pop() {
        if let Some(row) = table.iter().find(|r| r.pid == pid) {
            out.push(row.clone());
            frontier.extend(table.iter().filter(|r| r.ppid == pid).map(|r| r.pid));
        }
    }
    out
}

fn pid_dead(pid: u32) -> bool {
    // A zombie counts as dead: its /proc cmdline is empty.
    match std::fs::read(format!("/proc/{pid}/cmdline")) {
        Ok(bytes) => bytes.is_empty(),
        Err(_) => true,
    }
}

fn wait_until<F: FnMut() -> bool>(mut cond: F, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn read_file(path: &PathBuf) -> String {
    let mut s = String::new();
    let _ = std::fs::File::open(path).map(|mut f| f.read_to_string(&mut s));
    s
}

/// Supervised script shell in its own session; Drop SIGKILLs the
/// whole session so no pipeline stage can leak on a failure path.
struct SupervisedShell {
    child: std::process::Child,
    pid: u32,
}

impl Drop for SupervisedShell {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGKILL);
            libc::kill(self.pid as i32, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

struct PipelineScene {
    shell: SupervisedShell,
    dir: PathBuf,
    wrapper: ProcRow,
    stages: Vec<ProcRow>,
    recorded_pgrp: Option<u32>,
}

/// Spawn `rubash <script>` running a background pipeline under the
/// requested monitor mode, wait for the wrapper + both stages to be
/// visible in /proc, and hand the scene to the caller. The script
/// parks on a gate file so the tree stays put during inspection;
/// `release` lets it clean the job up and exit.
fn launch_pipeline(tag: &str, monitor: bool, sleep_secs: u32) -> PipelineScene {
    let dir = tempdir(tag);
    let script = dir.join("scene.sh");
    let gate = dir.join("go");
    let jobs_p_file = dir.join("jobs_p.txt");
    let mut body = String::new();
    if monitor {
        body.push_str("set -m\n");
    }
    body.push_str(&format!("sleep {sleep_secs} | cat &\n"));
    body.push_str(&format!("jobs -p > '{}'\n", jobs_p_file.to_string_lossy()));
    body.push_str(&format!(
        "n=0\nwhile [ ! -e '{}' ] && [ $n -lt 200 ]; do sleep 0.1; n=$((n+1)); done\n",
        gate.to_string_lossy()
    ));
    if monitor {
        // Kill the job's whole group, not just the wrapper: the
        // stages sit in the wrapper's group, not the shell's, so a
        // wrapper-only kill would orphan them for their full sleep.
        body.push_str("jp=$(jobs -p)\nkill -9 -- -$jp 2>/dev/null\nwait 2>/dev/null\nexit 0\n");
    } else {
        // No group was minted; the stages share the shell's session
        // group, which the test's Drop guard SIGKILLs wholesale.
        body.push_str("kill -9 %1 2>/dev/null\nwait 2>/dev/null\nexit 0\n");
    }
    std::fs::write(&script, body).expect("write scene script");

    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(&script)
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().expect("spawn rubash scene shell");
    let shell_pid = child.id();
    let shell = SupervisedShell {
        child,
        pid: shell_pid,
    };

    // Wait for the tree: shell -> wrapper (rubash) -> sleep + cat.
    let mut wrapper: Option<ProcRow> = None;
    let mut stages: Vec<ProcRow> = Vec::new();
    let appeared = wait_until(
        || {
            let tree = proc_tree(shell_pid);
            let wrappers: Vec<&ProcRow> = tree
                .iter()
                .filter(|r| r.ppid == shell_pid && r.comm == "rubash")
                .collect();
            if let Some(w) = wrappers.first() {
                let kids: Vec<ProcRow> = tree
                    .iter()
                    .filter(|r| r.ppid == w.pid && (r.comm == "sleep" || r.comm == "cat"))
                    .cloned()
                    .collect();
                if kids.len() >= 2 {
                    wrapper = Some((*w).clone());
                    stages = kids;
                    return true;
                }
            }
            false
        },
        Duration::from_secs(10),
    );
    assert!(
        appeared,
        "background pipeline tree never appeared under shell {shell_pid} (monitor={monitor})"
    );

    let recorded_pgrp = if monitor {
        let text = read_file(&jobs_p_file);
        text.trim()
            .lines()
            .next()
            .and_then(|line| line.trim().parse::<u32>().ok())
    } else {
        None
    };

    PipelineScene {
        shell,
        dir,
        wrapper: wrapper.expect("wrapper captured"),
        stages,
        recorded_pgrp,
    }
}

impl PipelineScene {
    fn release(mut self) {
        let _ = std::fs::write(self.dir.join("go"), b"");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.shell.child.try_wait() {
                Ok(Some(_)) => break,
                _ if Instant::now() >= deadline => break,
                _ => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue2287_background_pipeline_stages_share_recorded_pgrp_group() {
    let scene = launch_pipeline("membership", true, 61);
    let recorded = scene
        .recorded_pgrp
        .expect("jobs -p must report the pipeline job's recorded pgrp");
    assert_eq!(
        recorded, scene.wrapper.pid,
        "recorded pgrp must be the background wrapper job's pid"
    );
    assert_eq!(
        scene.wrapper.pgid, recorded,
        "the wrapper must LEAD the group the job table recorded \
         (pgid {} != recorded {})",
        scene.wrapper.pgid, recorded
    );
    assert_eq!(scene.stages.len(), 2, "expected sleep + cat stages");
    for stage in &scene.stages {
        assert_eq!(
            stage.pgid, recorded,
            "stage {} ({}) sits in pgid {}, not the recorded group {} — \
             the recorded pgrp would be a fiction for this stage",
            stage.pid, stage.comm, stage.pgid, recorded
        );
    }
    scene.release();
}

#[test]
fn issue2287_background_pipeline_killpg_by_recorded_pgrp_reaches_stages() {
    let scene = launch_pipeline("delivery", true, 62);
    let recorded = scene
        .recorded_pgrp
        .expect("jobs -p must report the pipeline job's recorded pgrp");
    let stage_pids: Vec<u32> = scene.stages.iter().map(|s| s.pid).collect();
    // Exactly the signal hangup_all_jobs sends at huponexit, and the
    // delivery a user's `kill -- -<pgrp>` performs.
    let rc = unsafe { libc::killpg(recorded as i32, libc::SIGHUP) };
    assert_eq!(
        rc, 0,
        "killpg({recorded}, SIGHUP) failed — the recorded pgrp names \
         no live group (ESRCH is the #2287 claim)"
    );
    for pid in &stage_pids {
        assert!(
            wait_until(|| pid_dead(*pid), Duration::from_secs(5)),
            "stage {pid} survived killpg({recorded}, SIGHUP)"
        );
    }
    assert!(
        wait_until(|| pid_dead(scene.wrapper.pid), Duration::from_secs(5)),
        "wrapper {} survived killpg({recorded}, SIGHUP)",
        scene.wrapper.pid
    );
    scene.release();
}

#[test]
fn issue2287_background_pipeline_without_monitor_keeps_ambient_group() {
    let scene = launch_pipeline("control", false, 63);
    let shell_row = proc_row(scene.shell.pid).expect("shell row");
    assert_eq!(
        scene.wrapper.pgid, shell_row.pgid,
        "without job control the wrapper must NOT mint a process \
         group (wrapper pgid {} != shell pgid {})",
        scene.wrapper.pgid, shell_row.pgid
    );
    for stage in &scene.stages {
        assert_eq!(
            stage.pgid, shell_row.pgid,
            "without job control stage {} ({}) must keep the ambient \
             group {}, got {}",
            stage.pid, stage.comm, shell_row.pgid, stage.pgid
        );
    }
    scene.release();
}
