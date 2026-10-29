//! Multi-session concurrency isolation harness (1.5.0 multi-session line).
//!
//! Question under test: can N independent `Executor`s — the exact driving
//! surface niubash uses today (`Executor::new` + `script_driver::run_source`,
//! one Executor per host thread) — run concurrently without leaking state
//! into each other?
//!
//! Method:
//! - Every session owns a private directory; its scripts report results ONLY
//!   through files inside that directory (never through captured stdout —
//!   the stdout-capture surface is a separate object under test elsewhere).
//! - The same step list runs (a) serially, one fresh Executor per session
//!   on the main thread, producing the baseline file trees, and (b)
//!   concurrently, one scoped thread per session, each thread creating its
//!   own Executor in place (Executor is !Send — Rc/RefCell internals — so
//!   it is born, driven and dropped on its worker thread) and stepping
//!   through the same scripts with a `Barrier` after every step. The
//!   barrier forces worst-case interleaving *between* `run_source` calls,
//!   which is exactly how a REPL-style host drives the engine.
//! - A second concurrent mode drives each session's steps as ONE whole
//!   `run_source` call (script-file style), without barriers.
//! - Serial vs concurrent trees are compared byte-for-byte after two
//!   documented normalizations: each session directory is replaced by
//!   `<SESSi>` (so a path pointing at the WRONG session still mismatches),
//!   and digit runs in `jobslist.txt` are replaced by `#` (job PIDs are
//!   legitimately different in every run).
//!
//! Everything lives in a single `#[test]` on purpose: cargo runs test
//! functions of one binary on parallel threads of one process, and this
//! harness deliberately mutates process-global state (cwd) through the
//! engine — two harness tests running side by side would measure each
//! other, not the engine.
//!
//! Run with: cargo test --test multi_session_isolation --locked
//! A human-readable deviation summary is written to
//! target/multi_session_summary.txt (relative to the crate root).

use rubash::executor::Executor;
use rubash::script_driver::run_source;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Barrier;
use std::time::Instant;

/// Restore the test process cwd on drop. Historical note: pre-G-02 the
/// engine's `cd` chdir'd the whole process, so session scripts leaked
/// directories into cargo itself; post-G-02 `cd` moves only the session
/// cwd field in ShellState and this guard is just belt-and-braces for
/// the harness's own relative-path uses.
struct CwdGuard {
    saved: PathBuf,
}

impl CwdGuard {
    fn take() -> Self {
        Self {
            saved: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.saved);
    }
}

fn sess_dir(root: &Path, i: usize) -> PathBuf {
    root.join(format!("sess{i}"))
}

fn shell_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// The per-session step list. Every step is one `run_source` call in
/// stepwise mode; whole-script mode concatenates them. `{D}` is the
/// session's absolute directory. Steps are identical in shape for every
/// session, parameterized only by the session index.
fn steps_for(i: usize, dir: &Path) -> Vec<String> {
    let d = shell_path(dir);
    vec![
        // 0: cwd — move this session to its own directory.
        format!("cd {d}"),
        // 1: vars + funcs — session-private variable, function, arithmetic.
        format!(
            "MYVAL=val{i}\n\
             myfn() {{ echo \"fn:$MYVAL:$1\" > {d}/func.txt; }}\n\
             acc=0\n\
             for n in 1 2 3 4 5; do acc=$((acc + n * {})); done\n\
             echo \"$MYVAL $acc\" > {d}/vars.txt\n\
             myfn arg{i}\n\
             declare -p MYVAL > {d}/declare.txt",
            i + 1
        ),
        // 2: cwd (a) — builtin pwd (reads the executor's $PWD).
        format!("pwd > {d}/cwd_builtin.txt"),
        // 3: cwd (b) — external /bin/pwd. Post-G-02 the child is spawned
        // with current_dir(session cwd), so this reports the SESSION's
        // directory; pre-G-02 it reported the shared process cwd and
        // stepwise rounds cross-talked deterministically.
        format!("/bin/pwd > {d}/cwd_external.txt"),
        // 4: cwd (c) — relative-path redirect: wherever this file lands IS
        // the verdict on which directory the session really writes in.
        format!("echo relcontent-{i} > rel.txt"),
        // 5: cwd (d) — external command reading a relative path.
        format!(
            "/bin/cat rel.txt > {d}/cwd_cat.txt 2> {d}/cwd_cat.err\n\
             echo \"catrc:$?\" >> {d}/cwd_cat.txt"
        ),
        // 6: jobs — background job, jobs listing, wait, all in-session.
        format!(
            "/bin/sleep 0.3 & p=$!\n\
             jobs > {d}/jobslist.txt\n\
             wait $p\n\
             echo \"waitrc:$?\" > {d}/jobs.txt"
        ),
        // 7: traps — DEBUG trap counter accumulated across two commands.
        format!(
            "dbg=0\n\
             trap 'dbg=$((dbg+1))' DEBUG\n\
             echo one\n\
             echo two\n\
             trap - DEBUG\n\
             echo \"dbg:$dbg\" > {d}/debugtrap.txt"
        ),
        // 8: comsub + subshell — substitution result and the guarantee
        // that a `( )` assignment does not leak back into the parent.
        format!(
            "x=$(/bin/echo inner-{i})\n\
             y=outer-{i}\n\
             ( y=changed-{i}; echo \"sub:$y\" > {d}/sub.txt )\n\
             echo \"$x|$y\" > {d}/comsub.txt"
        ),
        // 9: external commands + pipeline + redirect.
        format!("/bin/echo pipe-{i} | /usr/bin/tr a-z A-Z > {d}/pipe.txt"),
        // 10: traps — EXIT trap marker; last step for this session.
        format!("trap 'echo exited-{i} > {d}/exit_trap.txt' EXIT\n exit 0"),
    ]
}

/// Read a session directory into a sorted rel-path -> bytes map.
fn read_tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut tree = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return tree;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Ok(bytes) = std::fs::read(&path) {
                tree.insert(name, bytes);
            }
        }
    }
    tree
}

/// Normalize one file's bytes for cross-run comparison: session directory
/// strings become `<SESSj>` tokens (index-preserving, so a file that names
/// the wrong session still differs), and `jobslist.txt` digit runs become
/// `#` (PIDs differ legitimately per run).
fn normalize(name: &str, bytes: &[u8], dirs: &[PathBuf]) -> Vec<u8> {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    for (j, d) in dirs.iter().enumerate() {
        text = text.replace(&shell_path(d), &format!("<SESS{j}>"));
    }
    if name == "jobslist.txt" {
        let mut out = String::with_capacity(text.len());
        let mut in_digits = false;
        for ch in text.chars() {
            if ch.is_ascii_digit() {
                if !in_digits {
                    out.push('#');
                }
                in_digits = true;
            } else {
                out.push(ch);
                in_digits = false;
            }
        }
        text = out;
    }
    text.into_bytes()
}

type Trees = Vec<BTreeMap<String, Vec<u8>>>;

fn compare_trees(
    label: &str,
    serial: &Trees,
    other: &Trees,
    serial_dirs: &[PathBuf],
    other_dirs: &[PathBuf],
    deviations: &mut Vec<String>,
) -> usize {
    let mut mismatches = 0;
    for i in 0..serial.len() {
        let mut names: Vec<&String> = serial[i].keys().chain(other[i].keys()).collect();
        names.sort();
        names.dedup();
        for name in names {
            let s = serial[i].get(name).map(|b| normalize(name, b, serial_dirs));
            let o = other[i].get(name).map(|b| normalize(name, b, other_dirs));
            if s != o {
                mismatches += 1;
                deviations.push(format!(
                    "{label} sess{i} {name}: serial={} concurrent={}",
                    s.as_deref().map(show).unwrap_or_else(|| "<missing>".into()),
                    o.as_deref().map(show).unwrap_or_else(|| "<missing>".into()),
                ));
            }
        }
    }
    mismatches
}

fn show(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace('\n', "\\n")
}

/// Drive one session through its steps on the CURRENT thread with a fresh
/// Executor; used for the serial baseline. Returns per-step exit statuses.
fn run_session_serial(i: usize, dir: &Path) -> Vec<i32> {
    let mut executor = Executor::new();
    steps_for(i, dir)
        .iter()
        .map(|step| run_source(&mut executor, step, false))
        .collect()
}

/// Drive N sessions concurrently, one scoped thread each, barrier after
/// every step (stepwise mode) or as one whole script per session when
/// `whole_script` is set. Each Executor is created inside its worker.
fn run_sessions_concurrent(root: &Path, n: usize, whole_script: bool) -> Trees {
    for i in 0..n {
        std::fs::create_dir_all(sess_dir(root, i)).unwrap();
    }
    let barrier = Barrier::new(n);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for i in 0..n {
            let barrier = &barrier;
            let dir = sess_dir(root, i);
            handles.push(scope.spawn(move || {
                let mut executor = Executor::new();
                // All Executors exist before any script runs, so a
                // constructor-time process-env snapshot cannot be poisoned
                // by a sibling session's exports.
                barrier.wait();
                if whole_script {
                    let script = steps_for(i, &dir).join("\n");
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run_source(&mut executor, &script, false)
                    }));
                } else {
                    for step in steps_for(i, &dir) {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            run_source(&mut executor, &step, false)
                        }));
                        barrier.wait();
                    }
                }
            }));
        }
        for h in handles {
            let _ = h.join();
        }
    });
    (0..n).map(|i| read_tree(&sess_dir(root, i))).collect()
}

fn fresh_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rubash-ms-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// Fixed-workload script for the throughput indicator (NOT a benchmark:
/// one repetition per size, on a shared loaded machine).
fn workload(_i: usize, dir: &Path) -> String {
    let d = shell_path(dir);
    format!(
        "s=0; str=\"\"\n\
         for ((k=0;k<6000;k++)); do s=$((s + k % 7)); str=\"${{str}}x\"; \
         if [ $((k % 2000)) -eq 0 ]; then /bin/echo tick >> {d}/tp_out.txt; fi; done\n\
         echo \"$s ${{#str}}\" > {d}/tp.txt"
    )
}

fn throughput(n: usize, concurrent: bool) -> (std::time::Duration, Trees) {
    let root = fresh_root(&format!(
        "tp-{}-{}",
        n,
        if concurrent { "conc" } else { "ser" }
    ));
    for i in 0..n {
        std::fs::create_dir_all(sess_dir(&root, i)).unwrap();
    }
    let start = Instant::now();
    if concurrent {
        std::thread::scope(|scope| {
            for i in 0..n {
                let dir = sess_dir(&root, i);
                scope.spawn(move || {
                    let mut executor = Executor::new();
                    run_source(&mut executor, &workload(i, &dir), false);
                });
            }
        });
    } else {
        for i in 0..n {
            let dir = sess_dir(&root, i);
            let mut executor = Executor::new();
            run_source(&mut executor, &workload(i, &dir), false);
        }
    }
    let elapsed = start.elapsed();
    let trees = (0..n).map(|i| read_tree(&sess_dir(&root, i))).collect();
    (elapsed, trees)
}

#[test]
fn multi_session_isolation_matrix() {
    let _cwd = CwdGuard::take();
    let mut summary = String::new();
    let mut deviations: Vec<String> = Vec::new();
    let mut total_mismatch = 0usize;
    let mut total_compared = 0usize;

    // ---- Serial baselines for N=2 and N=4 -------------------------------
    let mut baselines: BTreeMap<usize, (Trees, Vec<PathBuf>)> = BTreeMap::new();
    for n in [2usize, 4] {
        let root = fresh_root(&format!("serial{n}"));
        let dirs: Vec<PathBuf> = (0..n).map(|i| sess_dir(&root, i)).collect();
        for d in &dirs {
            std::fs::create_dir_all(d).unwrap();
        }
        for i in 0..n {
            let _statuses = run_session_serial(i, &dirs[i]);
            // Defensive only (post-G-02 cd no longer moves the process).
            let _ = std::env::set_current_dir(&_cwd.saved);
        }
        let trees: Trees = dirs.iter().map(|d| read_tree(d)).collect();
        total_compared += trees.iter().map(|t| t.len()).sum::<usize>();
        summary.push_str(&format!(
            "serial baseline N={n}: files per session: {:?}\n",
            trees.iter().map(|t| t.len()).collect::<Vec<_>>()
        ));
        baselines.insert(n, (trees, dirs));
    }

    // ---- Concurrent stepwise rounds (barrier between run_source calls) --
    for n in [2usize, 4] {
        let (serial, serial_dirs) = baselines.get(&n).unwrap();
        for round in 0..5 {
            let root = fresh_root(&format!("conc{n}-r{round}"));
            let trees = run_sessions_concurrent(&root, n, false);
            let _ = std::env::set_current_dir(&_cwd.saved);
            let dirs: Vec<PathBuf> = (0..n).map(|i| sess_dir(&root, i)).collect();
            total_compared += trees.iter().map(|t| t.len()).sum::<usize>();
            let label = format!("stepwise N={n} round{round}");
            let before = deviations.len();
            total_mismatch +=
                compare_trees(&label, serial, &trees, serial_dirs, &dirs, &mut deviations);
            summary.push_str(&format!(
                "{label}: {} deviation(s)\n",
                deviations.len() - before
            ));
        }
    }

    // ---- Concurrent whole-script rounds (one run_source per session) ----
    {
        let n = 4usize;
        let (serial, serial_dirs) = baselines.get(&n).unwrap();
        for round in 0..3 {
            let root = fresh_root(&format!("whole{n}-r{round}"));
            let trees = run_sessions_concurrent(&root, n, true);
            let _ = std::env::set_current_dir(&_cwd.saved);
            let dirs: Vec<PathBuf> = (0..n).map(|i| sess_dir(&root, i)).collect();
            let label = format!("whole-script N={n} round{round}");
            let before = deviations.len();
            total_mismatch +=
                compare_trees(&label, serial, &trees, serial_dirs, &dirs, &mut deviations);
            summary.push_str(&format!(
                "{label}: {} deviation(s)\n",
                deviations.len() - before
            ));
        }
    }

    // ---- G-03: env-mirror rollback --------------------------------------
    // Session A mirrors a variable into the process env and spawn-verifies
    // it; session B (born BEFORE the write) drops first; A spawn-verifies
    // again — the variable must survive B's Drop. Pre-G-03, Drop restored
    // the whole birth snapshot and silently un-wrote A's variable.
    // `__RUBASH_*` names pass the process-env mirror gate unconditionally.
    {
        const PROBE: &str = "__RUBASH_MS_PROBE";
        std::env::remove_var(PROBE);
        let root = fresh_root("envrollback");
        let dir_a = sess_dir(&root, 0);
        std::fs::create_dir_all(&dir_a).unwrap();
        let da = shell_path(&dir_a);
        let mut a = Executor::new();
        let b = Executor::new();
        run_source(&mut a, &format!("{PROBE}=probe-a"), false);
        let mirrored = std::env::var(PROBE).ok();
        summary.push_str(&format!(
            "G-03 env rollback: after A assigns, process env = {mirrored:?}\n"
        ));
        if mirrored.as_deref() != Some("probe-a") {
            deviations.push(format!(
                "G-03 env rollback: A's assignment never reached the process env mirror ({mirrored:?})"
            ));
        }
        run_source(
            &mut a,
            &format!("/usr/bin/env > {da}/env_before.txt"),
            false,
        );
        drop(b);
        let after_b = std::env::var(PROBE).ok();
        summary.push_str(&format!(
            "G-03 env rollback: after B drops, process env = {after_b:?}\n"
        ));
        if after_b.as_deref() != Some("probe-a") {
            deviations.push(format!(
                "G-03 env rollback: B's Drop rolled back A's live variable (now {after_b:?})"
            ));
        }
        run_source(&mut a, &format!("/usr/bin/env > {da}/env_after.txt"), false);
        let env_after = std::fs::read_to_string(dir_a.join("env_after.txt")).unwrap_or_default();
        if !env_after.contains(&format!("{PROBE}=probe-a")) {
            deviations.push(
                "G-03 env rollback: A's spawned child no longer sees the variable after B's Drop"
                    .to_string(),
            );
        }
        drop(a);
        let after_a = std::env::var(PROBE).ok();
        summary.push_str(&format!(
            "G-03 env rollback: after A drops, process env = {after_a:?} (owner restore)\n"
        ));
        if after_a.is_some() {
            deviations.push(format!(
                "G-03 env rollback: A's own Drop failed to restore its journaled write (still {after_a:?})"
            ));
        }
    }

    // ---- G-01: anti-starvation -------------------------------------------
    // Session A runs a long PURE-COMPUTE script (arithmetic only — no
    // spawns, no mirrored assignments, so post-G-01 it takes the
    // process-state lock zero times). Session B's trivial command must
    // complete, within a bounded wait, while A is still running.
    // Pre-G-01, execute_ast held EXECUTION_LOCK for the whole run and B
    // could not start until A finished entirely.
    {
        let root = fresh_root("antistarve");
        let dir_a = sess_dir(&root, 0);
        let dir_b = sess_dir(&root, 1);
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        let da = shell_path(&dir_a);
        let db = shell_path(&dir_b);
        let a_script = format!(
            "echo started > {da}/started.txt\n\
             s=0\n\
             for ((k=0;k<3000000;k++)); do s=$((s + k % 7)); done\n\
             echo \"done:$s\" > {da}/done.txt"
        );
        let b_script = format!("echo b-trivial > {db}/b.txt");
        let started = dir_a.join("started.txt");
        let done = dir_a.join("done.txt");
        let (a_elapsed, b_elapsed, a_still_running) = std::thread::scope(|scope| {
            let a_handle = scope.spawn(|| {
                let t = Instant::now();
                let mut executor = Executor::new();
                run_source(&mut executor, &a_script, false);
                t.elapsed()
            });
            // Wait until A is demonstrably inside its run.
            let wait_start = Instant::now();
            while !started.exists() && wait_start.elapsed().as_secs() < 60 {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let mut b_executor = Executor::new();
            let tb = Instant::now();
            run_source(&mut b_executor, &b_script, false);
            let b_elapsed = tb.elapsed();
            let a_still_running = started.exists() && !done.exists();
            let a_elapsed = a_handle.join().unwrap_or_default();
            (a_elapsed, b_elapsed, a_still_running)
        });
        let b_content = std::fs::read_to_string(dir_b.join("b.txt")).unwrap_or_default();
        summary.push_str(&format!(
            "G-01 anti-starvation: A pure-compute total={a_elapsed:?}; B trivial elapsed={b_elapsed:?} while A running={a_still_running}; B output={b_content:?}\n"
        ));
        if !a_still_running {
            deviations.push(
                "G-01 anti-starvation: A finished before B ran — the scenario did not overlap"
                    .to_string(),
            );
        }
        if b_elapsed.as_secs() > 60 {
            deviations.push(format!(
                "G-01 anti-starvation: B's trivial command took {b_elapsed:?} behind A's compute run"
            ));
        }
        if b_content.trim() != "b-trivial" {
            deviations.push(format!(
                "G-01 anti-starvation: B's output wrong: {b_content:?}"
            ));
        }
    }

    // ---- Throughput indicator: fixed workload, sessions 1/2/4 -----------
    summary.push_str("throughput indicator (single rep, debug build, loaded host):\n");
    for n in [1usize, 2, 4] {
        let (serial_t, serial_trees) = throughput(n, false);
        let _ = std::env::set_current_dir(&_cwd.saved);
        let (conc_t, conc_trees) = throughput(n, true);
        let _ = std::env::set_current_dir(&_cwd.saved);
        let agree = serial_trees
            .iter()
            .zip(conc_trees.iter())
            .all(|(a, b)| a.get("tp.txt") == b.get("tp.txt"));
        summary.push_str(&format!(
            "  sessions={n} serial_total={serial_t:?} concurrent_wall={conc_t:?} tp_txt_agree={agree}\n"
        ));
        // G-01 acceptance: with the lock narrowed to touch points, the
        // compute-dominated workload must actually parallelize.
        if n >= 2 && conc_t.as_secs_f64() > serial_t.as_secs_f64() * 0.85 {
            deviations.push(format!(
                "G-01 throughput: sessions={n} concurrent_wall {conc_t:?} not significantly below serial_total {serial_t:?}"
            ));
        }
    }

    summary.push_str(&format!(
        "TOTAL: {total_mismatch} mismatching file(s) over {total_compared} compared file instances; {} deviation line(s)\n",
        deviations.len()
    ));
    for d in &deviations {
        summary.push_str(&format!("DEVIATION {d}\n"));
    }
    let summary_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("multi_session_summary.txt");
    if let Some(parent) = summary_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&summary_path, &summary);
    println!("{summary}");

    assert!(
        deviations.is_empty(),
        "multi-session isolation deviations ({}):\n{}",
        deviations.len(),
        deviations.join("\n")
    );
}
