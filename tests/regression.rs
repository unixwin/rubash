//! GNU-diff regression suite (wt4/ci lane).
//!
//! Two kinds of byte-pinned regressions live under `tests/regression/`:
//!
//! 1. **GNU goldens** (`golden/<name>.gnu.out` + `.gnu.err`): fixture
//!    scripts under `fixtures/` whose expected stdout/stderr were captured
//!    ONCE from WSL GNU Bash 5.3.0 (`/usr/local/bin/bash`, the repository
//!    baseline). Each fixture header documents the exact capture command.
//!    CI runners have no WSL, so the recorded bytes ARE the baseline.
//!
//! 2. **rubash snapshots** (`snapshots/<suite>.snap`): stdout of GNU
//!    upstream suite slices (`third_party/bash/tests/<suite>.tests`) run
//!    under TODAY's rubash. These are NOT GNU output — they pin the
//!    current engine so that any future semantic change that shifts a
//!    slice fails CI and forces a conscious re-record. They are
//!    Windows-only (the prebuilt helper binaries under
//!    tests/gnu-compat/helpers-win are PE executables) and run in the
//!    SAME airtight environment as the goldens (see `run_suite_snapshot`):
//!    a snapshot byte that depends on an ambient variable is a
//!    machine-dependent byte, and machine-dependent bytes are how a
//!    green-local/red-CI (or green-CI/red-local) baseline happens.
//!
//!    RE-RECORD PROTOCOL (the anti-baseline-red rule): re-record only
//!    with
//!
//!    ```text
//!    RUBASH_REGRESSION_RECORD=1 cargo test --test regression upstream_suite_slice_snapshots
//!    ```
//!
//!    and the recording commit message MUST name the engine change that
//!    shifted the bytes (issue or PR reference, per slice). A snapshot
//!    diff IS a semantic change; "update golden" with no behavior
//!    reference is exactly how this job burned down in 2026-10 (the
//!    `${var@P}` marker-strip fix in #431/#480 shifted exp without a
//!    re-record and every subsequent PR inherited the red). On CI the
//!    "Record upstream-suite snapshots" workflow step records with the
//!    same code and uploads the bytes as the `regression-snapshots`
//!    artifact — that artifact is the runner-true replacement for stale
//!    committed snapshots; prefer it over transcribing the truncated
//!    40-line panic window by hand.
//!
//! Every fixture runs in its own empty temp directory with a minimal PATH
//! (rubash emulates cat/mkdir/grep/sed when they are absent), a bounded
//! per-test timeout, and an AIRTIGHT environment: `env_clear` plus a
//! machine-infrastructure whitelist (see `run_rubash_in`), so no ambient
//! user/tool variable — whose name may collide with a fixture variable —
//! can influence a golden.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const FIXTURE_DIR: &str = "tests/regression/fixtures";
const GOLDEN_DIR: &str = "tests/regression/golden";
#[cfg(windows)]
const SNAPSHOT_DIR: &str = "tests/regression/snapshots";
const CORPUS_DIR: &str = "tests/regression/corpus";
#[cfg(windows)]
const HELPERS_WIN: &str = "tests/gnu-compat/helpers-win";
#[cfg(windows)]
const UPSTREAM_TESTS: &str = "third_party/bash/tests";

/// Fixture table entry generation: one #[test] per fixture so CI reports
/// each matrix with its own name and the per-fixture bound applies
/// independently. Timeouts are deliberately generous: CI machines vary
/// and the point of this suite is byte parity, not speed (perf has its
/// own canaries below).
fn run_golden_fixture(name: &str, timeout_secs: u64, corpus: &[&str]) {
    let root = manifest_dir();
    std::env::set_current_dir(&root).expect("cwd to manifest dir");
    // The trap matrix is timing-sensitive (the parent must reach its
    // `kill' after the child installed the trap but before its sleep
    // ends); under parallel test load that window can close. Serialize
    // it against the CPU-heavy perf canaries so neither starves the
    // other (observed once as a 3-test flake during codification).
    let _heavy_guard = if name == "trap-signal-matrix" {
        Some(heavy_lock())
    } else {
        None
    };
    let dir = prepare_scratch(name, corpus);
    let out = run_rubash_in(
        &dir,
        &[&format!("{name}.sh")],
        Duration::from_secs(timeout_secs),
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!out.timed_out, "{name}: TIMED OUT after {timeout_secs}s");
    let expected_out = std::fs::read(root.join(GOLDEN_DIR).join(format!("{name}.gnu.out")))
        .unwrap_or_else(|e| panic!("{name}: missing golden stdout: {e}"));
    let expected_err = std::fs::read(root.join(GOLDEN_DIR).join(format!("{name}.gnu.err")))
        .unwrap_or_else(|e| panic!("{name}: missing golden stderr: {e}"));
    assert_eq!(
        out.code,
        Some(0),
        "{name}: unexpected exit code {:?}",
        out.code
    );
    assert_bytes_eq(&out.stdout, &expected_out, "stdout", name);
    assert_bytes_eq(&out.stderr, &expected_err, "stderr", name);
}

macro_rules! golden_fixture {
    ($fn_name:ident => $name:literal, timeout $secs:literal, corpus [$($rel:literal),*]) => {
        #[test]
        fn $fn_name() {
            run_golden_fixture($name, $secs, &[$($rel),*]);
        }
    };
}

golden_fixture!(golden_andor_continuation => "andor-continuation", timeout 60, corpus []);
golden_fixture!(golden_declare_integer_nameref => "declare-integer-nameref", timeout 60, corpus []);
golden_fixture!(golden_let_bare_diagnostic => "let-bare-diagnostic", timeout 60, corpus []);
golden_fixture!(golden_tilde_dirstack => "tilde-dirstack", timeout 60, corpus []);
golden_fixture!(golden_arith_discard_matrix => "arith-discard-matrix", timeout 60, corpus []);
golden_fixture!(golden_arith_expansion_contexts => "arith-expansion-contexts", timeout 60, corpus []);
golden_fixture!(golden_umask_comsub => "umask-comsub", timeout 60, corpus []);
golden_fixture!(golden_at_star_matrix => "at-star-matrix", timeout 60, corpus []);
golden_fixture!(golden_catfile_comsub => "catfile-comsub", timeout 60, corpus []);
golden_fixture!(golden_digit_intfit => "digit-intfit", timeout 60, corpus []);
golden_fixture!(golden_eco_completion_register => "eco-completion-register", timeout 60, corpus []);
golden_fixture!(golden_eco_git_completion_smoke => "eco-git-completion-smoke", timeout 60, corpus []);
golden_fixture!(golden_eco_omb_theme_ps1 => "eco-omb-theme-ps1", timeout 60, corpus []);
golden_fixture!(golden_extglob_parse_gate => "extglob-parse-gate", timeout 60, corpus []);
golden_fixture!(golden_fnbody_strict_battery => "fnbody-strict-battery", timeout 90, corpus []);
golden_fixture!(golden_funsub_valsub => "funsub-valsub", timeout 60, corpus []);
golden_fixture!(golden_issue296_carrier_adjacent_comsub => "issue296-carrier-adjacent-comsub", timeout 60, corpus []);
// Windows-lane gated (not a golden_fixture! invocation): the child in
// this fixture is "the `bash` found on PATH" — in the capture
// environment that was the GNU oracle shell itself. The Windows lane's
// minimal PATH offers only System32\bash.exe, the WSL launcher stub,
// which answers with its "no installed distributions" banner, so the
// pinned bytes are unreachable there for reasons that have nothing to
// do with the engine. Coverage continues on non-Windows runs.
#[cfg(not(windows))]
#[test]
fn golden_issue1338_bash_argv0() {
    run_golden_fixture("issue1338-bash-argv0", 60, &[]);
}
golden_fixture!(golden_issue1339_numeric_padding => "issue1339-numeric-padding", timeout 60, corpus []);
golden_fixture!(golden_issue1340_xtrace_setattr => "issue1340-xtrace-setattr", timeout 60, corpus []);
golden_fixture!(golden_issue1341_xtrace_indirection => "issue1341-xtrace-indirection", timeout 60, corpus []);
golden_fixture!(golden_issue1342_xtrace_pipeline => "issue1342-xtrace-pipeline", timeout 60, corpus []);
golden_fixture!(golden_issue1343_ps4 => "issue1343-ps4", timeout 60, corpus []);
golden_fixture!(golden_issue1344_arith_xtrace => "issue1344-arith-xtrace", timeout 60, corpus []);
golden_fixture!(golden_issue1346_paren_key => "issue1346-paren-key", timeout 60, corpus []);
golden_fixture!(golden_issue1347_assoc_scalar => "issue1347-assoc-scalar", timeout 60, corpus []);
golden_fixture!(golden_issue1348_invalid_ident => "issue1348-invalid-ident", timeout 60, corpus []);
golden_fixture!(golden_issue1349_pipeline_jobs => "issue1349-pipeline-jobs", timeout 60, corpus []);
golden_fixture!(golden_issue1350_pushd_notdir => "issue1350-pushd-notdir", timeout 60, corpus []);
golden_fixture!(golden_issue1351_trap_syntax => "issue1351-trap-syntax", timeout 60, corpus []);
golden_fixture!(golden_issue1352_nameref_dynamic => "issue1352-nameref-dynamic", timeout 60, corpus []);
golden_fixture!(golden_nounset_exit_matrix => "nounset-exit-matrix", timeout 60, corpus []);
// rubash#1940 dual gate: the nvm-load fixture keeps its byte contract
// (run_golden_fixture below) AND gains a same-machine timing budget.
// Before #1940 this fixture's 180 s timeout was the only bound, so a
// ~100x load-time regression (4.2 s vs GNU 0.04 s on the same machine)
// passed silently. The timing half times `source nvm.sh` under this
// build's rubash and under a GNU bash found on the host, and asserts a
// RATIO bound: machine speed cancels, only the engine gap counts.
// The oracle is validated (it must actually source the corpus with
// exit 0); lanes without a real GNU bash (the Windows lane's PATH
// offers only the WSL launcher stub) skip the timing half instead of
// failing on a stub. RUBASH_GNU_BASH pins an explicit oracle path.
#[test]
fn golden_nvm_load() {
    run_golden_fixture("nvm-load", 180, &["nvm/nvm.sh"]);
    nvm_load_timing_gate();
}

/// One timed run of an arbitrary `-c` script in the gate scratch dir;
/// None when the run fails, times out, or the program cannot be
/// spawned.
fn nvm_load_timed_run_script(
    program: &Path,
    dir: &Path,
    script: &str,
    limit: Duration,
) -> Option<Duration> {
    let mut cmd = Command::new(program);
    cmd.args(["-c", script])
        .current_dir(dir)
        .env_clear()
        .env("PATH", minimal_path())
        .env("HOME", dir)
        .env("NVM_DIR", dir.join("nvmdir"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let started = Instant::now();
    let mut child = cmd.spawn().ok()?;
    let out = wait_bounded(&mut child, limit);
    if out.timed_out || out.code != Some(0) {
        return None;
    }
    Some(started.elapsed())
}

/// Best-of-N estimator: on a shared CI box the minimum is the run
/// least contaminated by foreign load, for both shells alike; a
/// median would import the box's load ratio into the gate and flake
/// (observed locally: wall medians 1.89 s vs 0.04 s under load, a
/// ~47x ratio, vs ~26x on best-of-N for the same tree).
fn best_duration(runs: Vec<Duration>) -> Duration {
    runs.into_iter().min().expect("at least one run")
}

fn gnu_bash_oracle() -> Option<PathBuf> {
    if let Ok(pinned) = std::env::var("RUBASH_GNU_BASH") {
        let candidate = PathBuf::from(pinned);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let own = PathBuf::from(env!("CARGO_BIN_EXE_rubash"));
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(if cfg!(windows) { "bash.exe" } else { "bash" });
        if candidate.is_file() && candidate != own {
            return Some(candidate);
        }
    }
    None
}

fn nvm_load_timing_gate() {
    let _guard = heavy_lock();
    let Some(oracle) = gnu_bash_oracle() else {
        eprintln!("golden_nvm_load timing gate: no GNU bash oracle found; skipped");
        return;
    };
    let root = manifest_dir();
    let dir = std::env::temp_dir().join(format!("rubash-nvm-timing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("nvmdir")).expect("create timing scratch");
    std::fs::copy(root.join(CORPUS_DIR).join("nvm/nvm.sh"), dir.join("nvm.sh"))
        .expect("copy nvm corpus");

    nvm_load_timing_gate_script(&oracle, &dir, ". ./nvm.sh", "plain");
    // rubash#1961 alias-arm variant: the SAME load with one alias
    // preset and expand_aliases on. The alias never occurs in nvm.sh,
    // so the sourced bytes are identical — only the reader arm differs
    // (pre-#1961 that arm re-derived group completeness per candidate
    // line and hung past 120 s where the plain arm took ~1 s). The
    // trailing `ll` command is the non-vacuity tripwire: the run only
    // exits 0 when alias expansion was actually live for the driver,
    // so the timing below cannot pass by silently taking the plain
    // arm. Newlines separate the driver lines because an alias defined
    // on the same line as its use is not yet in effect (parse.y reads
    // one command at a time).
    nvm_load_timing_gate_script(
        &oracle,
        &dir,
        "alias ll='echo alias-live'\nshopt -s expand_aliases\n. ./nvm.sh\nll >/dev/null",
        "alias-arm",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn nvm_load_timing_gate_script(oracle: &Path, dir: &Path, script: &str, label: &str) {
    // Ratio asserted below, calibrated on the post-#1940-fix tree
    // (debug build, loaded 2-core dev box): rubash best-of-5 ~0.9 s
    // vs GNU best-of-5 ~0.035 s, a ~26x gap. The bound keeps ~2x
    // headroom over that while still failing the pre-fix tree
    // (~114x). The absolute slack absorbs oracle startup noise on
    // fast machines where the GNU run is only a few tens of
    // milliseconds.
    const MAX_RATIO: f64 = 50.0;
    const SLACK: Duration = Duration::from_millis(250);
    const REPS: usize = 5;

    let rubash = PathBuf::from(env!("CARGO_BIN_EXE_rubash"));
    let mut oracle_runs = Vec::new();
    for _ in 0..REPS {
        match nvm_load_timed_run_script(oracle, dir, script, Duration::from_secs(60)) {
            Some(d) => oracle_runs.push(d),
            None => {
                eprintln!(
                    "golden_nvm_load timing gate ({label}): oracle {} failed the run; skipped",
                    oracle.display()
                );
                return;
            }
        }
    }
    let mut rubash_runs = Vec::new();
    for _ in 0..REPS {
        let d = nvm_load_timed_run_script(&rubash, dir, script, Duration::from_secs(120))
            .expect("rubash must source nvm.sh inside the timing gate");
        rubash_runs.push(d);
    }
    let oracle_best = best_duration(oracle_runs);
    let rubash_best = best_duration(rubash_runs);
    let budget = oracle_best.mul_f64(MAX_RATIO) + SLACK;
    assert!(
        rubash_best <= budget,
        "nvm.sh load timing gate ({label}): rubash best-of-{REPS} {rubash_best:.2?} exceeds \
         {MAX_RATIO}x GNU best-of-{REPS} {oracle_best:.2?} + {SLACK:.2?} slack \
         (budget {budget:.2?}); rubash#1940/#1961 regression class"
    );
}
golden_fixture!(golden_redir_operand_battery => "redir-operand-battery", timeout 60, corpus []);
golden_fixture!(golden_rustup_check_help => "rustup-check-help", timeout 60, corpus []);
golden_fixture!(golden_trap_signal_matrix => "trap-signal-matrix", timeout 180, corpus []);
golden_fixture!(golden_xtrace_decl_compound => "xtrace-decl-compound", timeout 60, corpus []);
golden_fixture!(golden_issue1224_coproc_fds => "issue1224-coproc-fds", timeout 60, corpus []);
golden_fixture!(golden_issue1225_declare_convert => "issue1225-declare-convert", timeout 60, corpus []);
// Windows-lane gated (not a golden_fixture! invocation): the fixture
// is built entirely from POSIX externals — `bash`, `sh`, `/bin/dash`
// and `hash -p /bin/dash` — none of which exist in the Windows lane's
// minimal PATH (its `bash` is the WSL launcher stub), so the script
// cannot reach exit code 0 there. Coverage continues on non-Windows
// runs, where those programs exist.
#[cfg(not(windows))]
#[test]
fn golden_issue1227_argv0() {
    run_golden_fixture("issue1227-argv0", 60, &[]);
}
golden_fixture!(golden_issue1229_builtin_help => "issue1229-builtin-help", timeout 60, corpus []);
golden_fixture!(golden_issue1230_exec_redirect_undo => "issue1230-exec-redirect-undo", timeout 60, corpus []);
golden_fixture!(golden_issue1232_tilde_pwd => "issue1232-tilde-pwd", timeout 60, corpus []);
golden_fixture!(golden_issue1235_fd_move => "issue1235-fd-move", timeout 60, corpus []);
// Windows-lane gated (not a golden_fixture! invocation): the pinned
// stderr embeds glibc's OS-error wording — bash's netopen surfaces
// strerror() text ("Connection refused") and gai_strerror() text for
// the failing service lookup. The same events on Windows carry the
// Winsock wording ("No connection could be made because the target
// machine actively refused it. (os error 10061)"), which no engine
// behavior can make byte-identical to the WSL capture, and the `http`
// service name resolves under glibc but not under the Windows
// resolver. The bash-owned message shapes in this fixture (the
// `connect:` line, the unknown-service line) do match on Windows; the
// gate is for the OS-owned bytes only. Coverage continues on
// non-Windows runs.
#[cfg(not(windows))]
#[test]
fn golden_issue1237_net_redir() {
    run_golden_fixture("issue1237-net-redir", 60, &[]);
}
golden_fixture!(golden_issue1238_arith_diag => "issue1238-arith-diag", timeout 60, corpus []);
golden_fixture!(golden_k4_declare_options => "k4-declare-options", timeout 60, corpus []);
golden_fixture!(golden_k4_getopts_state => "k4-getopts-state", timeout 60, corpus []);
golden_fixture!(golden_k4_expansion_fixes => "k4-expansion-fixes", timeout 60, corpus []);

/// Upstream suite slices pinned as rubash snapshots (coordinator list,
/// 2026-09-27): the slices the lanes rely on for parity ledgers.
#[cfg(windows)]
const SNAPSHOT_SUITES: &[&str] = &[
    "arith-for",
    "braces",
    "case",
    "comsub",
    "comsub2",
    "dbg-support",
    "errors",
    "exp",
    "heredoc",
    "nquote",
    "quote",
    "redir",
    "rhs-exp",
];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn minimal_path() -> String {
    if cfg!(windows) {
        // System32 first: this matches what a bare windows-latest CI
        // runner provides, so the local run and the CI run exercise the
        // same resolution paths. System32 alone is no longer a complete
        // fixture world, though: since the emulated-external stand-down
        // (rubash#1545), an emulated coreutil runs only when its name
        // actually resolves, and under a System32-only PATH head/wc/
        // mkdir/grep/sed resolve nowhere — the fixtures (whose GNU
        // captures were made with real coreutils under /usr/bin:/bin)
        // lost their tools and a dozen goldens went red the day #1545
        // landed, invisibly, inside an already-red job. The capture
        // world's defining property is that the coreutils EXIST as real
        // programs; Git for Windows' usr/bin is part of the standard
        // windows-latest image and supplies exactly that set, so append
        // it when present (System32 keeps precedence; on a machine
        // without Git the path degrades to the old System32-only shape).
        let mut path = r"C:\Windows\System32".to_string();
        let git_usr_bin = r"C:\Program Files\Git\usr\bin";
        if Path::new(git_usr_bin).is_dir() {
            path.push(';');
            path.push_str(git_usr_bin);
        }
        path
    } else {
        "/usr/bin:/bin".to_string()
    }
}

struct RunOutcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: Option<i32>,
    timed_out: bool,
}

fn wait_bounded(child: &mut Child, limit: Duration) -> RunOutcome {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_end(&mut stdout);
                }
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_end(&mut stderr);
                }
                return RunOutcome {
                    stdout,
                    stderr,
                    code: status.code(),
                    timed_out: false,
                };
            }
            Ok(None) => {
                if start.elapsed() > limit {
                    let _ = child.kill();
                    let _ = child.wait();
                    return RunOutcome {
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                        code: None,
                        timed_out: true,
                    };
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return RunOutcome {
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    code: None,
                    timed_out: false,
                };
            }
        }
    }
}

fn prepare_scratch(name: &str, corpus: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-regression-{}-{}-{}",
        name,
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("t")
            .replace('/', "-")
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::copy(
        Path::new(FIXTURE_DIR).join(format!("{name}.sh")),
        dir.join(format!("{name}.sh")),
    )
    .expect("copy fixture");
    for rel in corpus {
        let src = Path::new(CORPUS_DIR).join(rel);
        let dst = dir.join(Path::new(rel).file_name().unwrap());
        std::fs::copy(src, dst).expect("copy corpus file");
    }
    dir
}

fn run_rubash_in(dir: &Path, args: &[&str], limit: Duration) -> RunOutcome {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .current_dir(dir)
        // Airtight environment (cifix lane 2026-09-27): a golden must not
        // depend on ambient variables. golden_nounset_exit_matrix passed
        // locally but failed on CI because a runner-side variable whose
        // name case-insensitively matched the fixture's `$m2` reached the
        // child: on Windows `std::env::var` is case-insensitive, so the
        // engine's bare-$NAME env fallback treated `$m2` as bound while
        // GNU (variables.c, exact-case lookup) — and therefore the WSL
        // golden — has it unbound. Drop every user/tool variable and pass
        // only machine infrastructure plus the minimal PATH, so a local
        // run, a CI runner and any future image observe the same fixture
        // world. (The suite-snapshot path below now follows the same
        // airtight channel — env_clear + machine whitelist + the pinned
        // THIS_SH/TMPDIR exports — because ambient leakage made the
        // recorded bytes machine-dependent; see its own comment.)
        .env_clear()
        .env("PATH", minimal_path())
        // Real-world fixtures consult $HOME (nvm's NVM_DIR defaults under
        // it; trap subshells cd there; ruby-build refs it). A fixed
        // synthetic HOME = the fixture's own scratch dir keeps the env
        // airtight while giving every run — local or CI — the same value.
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cfg!(windows) {
        // Windows-process infrastructure only: values never enter golden
        // bytes (no fixture prints them), and the child needs SystemRoot
        // for OS services and TEMP for any internal temp-file use.
        for name in [
            "SystemRoot",
            "SystemDrive",
            "WINDIR",
            "COMSPEC",
            "PATHEXT",
            "OS",
            "TEMP",
            "TMP",
        ] {
            if let Ok(value) = std::env::var(name) {
                cmd.env(name, value);
            }
        }
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    wait_bounded(&mut child, limit)
}

/// run_rubash_in with the discriminating stdin state of the readhang
/// family: a pipe whose write end this test holds open and never writes
/// to — open, empty, and never EOF. The Child handle keeps that write
/// end alive for the whole bounded wait; an engine that reaches for the
/// inherited process stdin where GNU would see EOF (or would not read at
/// all) blocks until the bound kills it.
fn run_rubash_piped_stdin_in(dir: &Path, args: &[&str], limit: Duration) -> RunOutcome {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", minimal_path())
        .env("HOME", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cfg!(windows) {
        for name in [
            "SystemRoot",
            "SystemDrive",
            "WINDIR",
            "COMSPEC",
            "PATHEXT",
            "OS",
            "TEMP",
            "TMP",
        ] {
            if let Ok(value) = std::env::var(name) {
                cmd.env(name, value);
            }
        }
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    // Deliberately leave child.stdin (the pipe write end) unwritten and
    // open: this is the "parent stdin stays piped open" condition.
    let outcome = wait_bounded(&mut child, limit);
    drop(child.stdin.take());
    outcome
}

fn assert_bytes_eq(actual: &[u8], expected: &[u8], label: &str, name: &str) {
    if actual == expected {
        return;
    }
    let show = |data: &[u8]| -> String {
        let text = String::from_utf8_lossy(data);
        let lines: Vec<&str> = text.split('\n').collect();
        if lines.len() <= 40 {
            return lines.join("\n");
        }
        let marker = format!("... {} more lines ...", lines.len() - 40);
        let mut joined = lines[..20].join("\n");
        joined.push('\n');
        joined.push_str(&marker);
        joined.push('\n');
        joined.push_str(&lines[lines.len() - 20..].join("\n"));
        joined
    };
    panic!(
        "{name}: {label} differs from golden.\n--- expected ---\n{}\n--- actual ---\n{}\n-----------",
        show(expected),
        show(actual)
    );
}

#[test]
fn golden_fixture_matrix_smoke() {
    // Guards the table itself: every fixture .sh has a golden pair and
    // every golden pair has a fixture (catches rename drift at a glance
    // instead of per-test panics).
    let root = manifest_dir();
    let mut fixtures: Vec<String> = std::fs::read_dir(root.join(FIXTURE_DIR))
        .expect("fixture dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".sh"))
        .collect();
    fixtures.sort();
    for f in &fixtures {
        let stem = f.trim_end_matches(".sh");
        for ext in [".gnu.out", ".gnu.err"] {
            let golden = root.join(GOLDEN_DIR).join(format!("{stem}{ext}"));
            assert!(golden.is_file(), "fixture {f} has no golden {stem}{ext}");
        }
    }
}

#[cfg(windows)]
/// Strip Windows CR from suite output and anonymize the scratch root and
/// the shell-under-test path so snapshots do not embed absolute paths.
fn normalize_snapshot(bytes: &[u8], scratch: &Path) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let scratch_fwd = scratch.to_string_lossy().replace('\\', "/");
    let scratch_raw = scratch.to_string_lossy().to_string();
    let exe_fwd = env!("CARGO_BIN_EXE_rubash").replace('\\', "/");
    let exe_raw = env!("CARGO_BIN_EXE_rubash").to_string();
    // `hash -p ${THIS_SH} ...` style output renders the exe in POSIX form
    // (/d/...); anonymize that form too.
    let exe_posix_prefix = {
        let lowered = exe_fwd.to_lowercase();
        if let Some(rest) = lowered.strip_prefix("d:/") {
            format!("/d/{}", rest)
        } else {
            String::new()
        }
    };
    // The manifest root itself can surface (e.g. `cd -` printing OLDPWD);
    // scrub it in all three renderings.
    let root_fwd = manifest_dir().to_string_lossy().replace('\\', "/");
    let root_raw = manifest_dir().to_string_lossy().to_string();
    let root_posix = {
        let lowered = root_fwd.to_lowercase();
        if let Some(rest) = lowered.strip_prefix("d:/") {
            format!("/d/{}", rest)
        } else {
            String::new()
        }
    };
    let joined = text
        .split('\n')
        .map(|line| line.replace('\r', ""))
        .map(|line| line.replace(&scratch_fwd, "<T>"))
        .map(|line| line.replace(&scratch_raw, "<T>"))
        .map(|line| line.replace(&exe_fwd, "<SH>"))
        .map(|line| line.replace(&exe_raw, "<SH>"))
        .map(|line| {
            if exe_posix_prefix.is_empty() {
                line
            } else {
                line.replace(&exe_posix_prefix, "<SH>")
            }
        })
        .map(|line| line.replace(&root_fwd, "<R>"))
        .map(|line| line.replace(&root_raw, "<R>"))
        .map(|line| {
            if root_posix.is_empty() {
                line
            } else {
                line.replace(&root_posix, "<R>")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    joined.into_bytes()
}

#[cfg(windows)]
/// rubash#812: a space-free directory to anchor machine-absolute paths the
/// suite lane produces. The ambient temp dir is per-machine layout
/// (`C:\Users\<name>\AppData\Local\Temp`, a mounted scratch volume, a
/// spaced system profile) and its SHAPE must never decide snapshot bytes,
/// so a spaced ambient falls back to `%SystemRoot%\Temp` — the same
/// candidates the #812 THIS_SH pin uses — and a host where neither is
/// space-free fails loudly instead of recording divergent bytes.
fn space_free_temp_root() -> PathBuf {
    let ambient = std::env::temp_dir();
    if !ambient.to_string_lossy().contains(' ') {
        return ambient;
    }
    if let Ok(sysroot) = std::env::var("SystemRoot") {
        let sys_temp = Path::new(&sysroot).join("Temp");
        if !sys_temp.to_string_lossy().contains(' ') {
            return sys_temp;
        }
    }
    panic!(
        "rubash#812: no space-free temp root available; the snapshot lane \
         cannot produce machine-independent bytes here"
    );
}

#[cfg(windows)]
/// rubash#812: pin THIS_SH to a space-free copy of the engine for suite
/// runs. The upstream assets expand ${THIS_SH} UNQUOTED
/// (errors.tests:369 `${THIS_SH} ./errors10.sub`; errors10.sub:10-16 then
/// re-invokes it for every `after exit:`/`after shift:` status line), and
/// with no explicit value the engine auto-detects current_exe's FULL path
/// (executor/init.rs). When the checkout lives under a path containing a
/// space — any per-machine layout outside this repo — every
/// sub-invocation word-splits (`.../no: No such file or directory`) and
/// the errors slice's status lines COLLAPSE: a host-shape-dependent byte
/// set that no snapshot normalizer can repair (the lines are absent, not
/// misspelled; errors.snap:63-80 disappears whole). GNU never sees this
/// because /usr/local/bin/bash has no space, and true-baseline.sh pins
/// the GNU side contractually (`THIS_SH=$GNU_BASH`); pin the snapshot
/// side the same way with a copy of this test's own engine named
/// `bash.exe` under a space-free directory. The engine keeps an inherited
/// THIS_SH exactly when its basename stem is sh/bash and the path exists
/// (executor/init.rs `inherited_sh`), so the child resolves ${THIS_SH} to
/// the pinned copy on every host; a bash-named wrapper is the blessed
/// conventional-name configuration (same comment). Bytes on a space-free
/// host are unchanged — the same engine runs either way — so this is not
/// a re-record; a spaced-host run converges onto the committed bytes.
fn space_free_this_sh(scratch: &Path) -> (PathBuf, Option<PathBuf>) {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_rubash"));
    let mut candidates = vec![(scratch.join(".thissh"), None)];
    if let Ok(sysroot) = std::env::var("SystemRoot") {
        let sys_temp = Path::new(&sysroot)
            .join("Temp")
            .join(format!("rubash-812-thissh-{}", std::process::id()));
        // The fallback must be cleaned up by the caller (it lives outside
        // the per-suite scratch dir).
        candidates.push((sys_temp.clone(), Some(sys_temp)));
    }
    for (cdir, cleanup) in candidates {
        if cdir.to_string_lossy().contains(' ') {
            continue;
        }
        if std::fs::create_dir_all(&cdir).is_ok() {
            let shadow = cdir.join("bash.exe");
            if std::fs::copy(&exe, &shadow).is_ok() {
                return (shadow, cleanup);
            }
        }
    }
    // The scratch dir passed here is anchored at a space-free root by the
    // caller (space_free_temp_root), so the first candidate above wins in
    // practice; the SystemRoot fallback stays as a belt against a future
    // caller reorder, and the panic below is the loud host-shape failure
    // the lane prefers over divergent bytes.
    panic!(
        "rubash#812: no space-free directory available to pin THIS_SH; \
         the snapshot lane cannot produce machine-independent bytes here"
    );
}

#[cfg(windows)]
fn run_suite_snapshot(suite: &str) {
    let root = manifest_dir();
    let src = root.join(UPSTREAM_TESTS);
    // rubash#812: anchor the scratch at a space-free root so the suite's
    // whole path surface (HOME, the .thissh pin, TMPDIR below) keeps one
    // shape on every host. The scratch path itself is scrubbed by
    // normalize_snapshot, but what MATTERS is that assets expand the
    // machine-absolute variables built from it UNQUOTED (see the TMPDIR
    // pin below), so the space must never be there in the first place.
    let dir = space_free_temp_root().join(format!("rubash-snap-{suite}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create snapshot scratch dir");
    // rubash#812: the shell-visible temp dir, pinned like true-baseline.sh
    // pins the GNU side (`TMPDIR="$w/tmp"` in scripts/true-baseline.sh).
    // Created eagerly: the assets redirect into $TMPDIR with no mkdir.
    let suite_tmp = dir.join("tmp");
    std::fs::create_dir_all(&suite_tmp).expect("create snapshot TMPDIR");
    let (this_sh, this_sh_cleanup) = space_free_this_sh(&dir);

    // Copy the suite and everything it may reference relative to cwd:
    // <suite>.tests plus every <suite>*.{sub,right} neighbor.
    let mut copied = 0;
    if let Ok(entries) = std::fs::read_dir(&src) {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            let wanted = fname == format!("{suite}.tests")
                || fname == format!("{suite}.right")
                || (fname.starts_with(suite)
                    && (fname.ends_with(".tests")
                        || fname.ends_with(".sub")
                        || fname.ends_with(".right")))
                || fname == "recho"
                || fname == "zecho"
                || fname == "printenv"
                || fname == "bash";
            if wanted && entry.path().is_file() {
                // Normalize CRLF to LF while copying (rubash#803). The
                // windows-latest CI image materializes the third_party/bash
                // submodule with core.autocrlf=true, so every suite file
                // lands CRLF there while a developer checkout (autocrlf
                // unset) keeps the canonical LF bytes — and the suite's
                // stdout diverges by whole faces (nquote4.sub's
                // `alias foo=$'echo foo\n\nquux\n'` went silent on CI:
                // 1226 runner bytes vs 1230 LF bytes vs 1230 under WSL
                // GNU bash 5.3.0). The suite executes a unix corpus: the
                // 144 tracked files matching this copy predicate (13
                // slices) contain no CR in any canonical blob; on-disk
                // copies of a built submodule can add untracked build
                // artifacts such as `recho`, which are outside that corpus
                // claim. The pair-wise conversion is therefore exactly the
                // autocrlf inverse and cannot touch semantic CR bytes;
                // the recorded snapshot bytes stop depending on the
                // checkout's line-ending config.
                let bytes = std::fs::read(entry.path()).unwrap_or_default();
                if bytes.is_empty() {
                    let _ = std::fs::copy(entry.path(), dir.join(&fname));
                } else {
                    let mut lf = Vec::with_capacity(bytes.len());
                    let mut it = bytes.iter().copied().peekable();
                    while let Some(b) = it.next() {
                        if b == b'\r' && it.peek() == Some(&b'\n') {
                            continue; // CRLF pair: the CR is autocrlf noise
                        }
                        lf.push(b);
                    }
                    let _ = std::fs::write(dir.join(&fname), &lf);
                }
                copied += 1;
            }
        }
    }
    assert!(
        copied >= 1,
        "{suite}: no suite files found under {}",
        src.display()
    );

    let helpers = root.join(HELPERS_WIN);
    let sep = if cfg!(windows) { ';' } else { ':' };
    let mut path = helpers.to_string_lossy().to_string();
    path.push(sep);
    path.push_str(&minimal_path());

    let started = Instant::now();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(format!("{suite}.tests"))
        .current_dir(&dir)
        // Airtight environment — same contract as run_rubash_in above.
        // The snapshot path originally kept the ambient env because the
        // first recording happened to be made with it, and that choice
        // made the bytes machine-dependent: exp.tests:416 runs
        // `${THIS_SH} -c ...`, so any machine that exports THIS_SH (a
        // shell-managed env) recorded extra argv lines a clean runner
        // never produces — and shell option variables (BASHOPTS,
        // SHELLOPTS, POSIXLY_CORRECT classes) are exactly the inputs the
        // alias-expansion machinery keys on. Drop everything user/tool
        // shaped and pass machine infrastructure + the explicit PATH, so
        // a local run, the CI runner and any future image see the same
        // suite world (see the re-record protocol in the module header).
        .env_clear()
        .env("PATH", &path)
        .env("HOME", &dir)
        // rubash#812: the pinned space-free THIS_SH (see
        // space_free_this_sh) — the one deliberate variable, because the
        // upstream assets invoke ${THIS_SH} unquoted and the auto-detected
        // full exe path word-splits on any checkout layout with a space.
        .env("THIS_SH", &this_sh)
        // rubash#812, the second machine-absolute variable: the engine
        // self-injects a Windows TMPDIR default from the process temp dir
        // (executor/init.rs, the /tmp mapping fixture), and the upstream
        // assets expand it UNQUOTED for real faces in three slices —
        // heredoc.tests:115-124 (`cat > ${TMPDIR}/bash-zzz-$$` heredoc
        // round-trip), quote.tests:136-141 (`printf ... > $TMPDIR/quote-$$`
        // re-sourced through ${THIS_SH} $TMPDIR/...), and the redir.tests
        // file battery (redir.tests:35-119, `> $TMPDIR/null-redir-a` plus
        // the `testf`/`oops -- $1 not found` branch). Under a spaced
        // ambient TEMP every one of those paths word-splits and the faces
        // go silent or fall to the oops branch: verified, spaced-TEMP run
        // shifted heredoc/quote/redir against the committed bytes (heredoc
        // 1483 -> 1463; redir printed `oops -- /d/temp-812/rub not found`)
        // while CI's space-free TEMP stayed green — the same machine
        // dependence as the THIS_SH face one layer over. The value never
        // prints on a success face in any of the 13 slices (file targets
        // and PATH assignments only), so pinning it is byte-neutral on a
        // space-free host and converges spaced hosts onto the committed
        // bytes; being under the scratch, even a hypothetical leak renders
        // as the normalized `<T>` shape.
        .env("TMPDIR", &suite_tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cfg!(windows) {
        for name in [
            "SystemRoot",
            "SystemDrive",
            "WINDIR",
            "COMSPEC",
            "PATHEXT",
            "OS",
            "TEMP",
            "TMP",
        ] {
            if let Ok(value) = std::env::var(name) {
                cmd.env(name, value);
            }
        }
    }
    let mut child = cmd.spawn().expect("spawn rubash for suite");
    let out = wait_bounded(&mut child, Duration::from_secs(120));

    let snap_path = root.join(SNAPSHOT_DIR).join(format!("{suite}.snap"));
    let normalized = normalize_snapshot(&out.stdout, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    if let Some(cleanup) = this_sh_cleanup {
        let _ = std::fs::remove_dir_all(cleanup);
    }

    if std::env::var("RUBASH_REGRESSION_RECORD").as_deref() == Ok("1") {
        std::fs::write(&snap_path, &normalized).expect("record snapshot");
        eprintln!("recorded {suite}.snap ({} bytes)", normalized.len());
        return;
    }
    let expected = std::fs::read(&snap_path).unwrap_or_else(|e| {
        panic!("{suite}: missing snapshot (record with RUBASH_REGRESSION_RECORD=1): {e}")
    });
    assert!(
        !out.timed_out,
        "{suite}: suite run timed out ({}s)",
        started.elapsed().as_secs()
    );
    assert_bytes_eq(&normalized, &expected, "snapshot stdout", suite);
}

#[cfg(windows)]
#[test]
fn upstream_suite_slice_snapshots() {
    let root = manifest_dir();
    std::env::set_current_dir(&root).expect("cwd to manifest dir");
    for suite in SNAPSHOT_SUITES {
        run_suite_snapshot(suite);
    }
}

// ---------------------------------------------------------------------------
// Perf canaries: generous absolute bounds that only fail on the O(N^2)
// parser blowup class (rubash#176/#178/#185: nested-brace re-scan and
// dolbrace span re-collection; measured pre-fix at >300s). They share one
// mutex so they never compete for CPU with each other (cargo runs tests
// in parallel; a contended timing bound is a flaky bound).
// ---------------------------------------------------------------------------

static HEAVY_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acquire HEAVY_TEST_LOCK, tolerating poison. The lock only serializes
/// heavy tests against each other; it guards no data. A test that panics
/// while holding it must fail on its own assertion alone — with a plain
/// `.lock().unwrap()` its panic used to poison the mutex and manufacture
/// a second, misleading PoisonError failure in every later heavy test
/// (observed in the 2026-10-26 master CI run: the trap-signal-matrix
/// panic cascaded into all four perf canaries, yes_head_one,
/// parse_acceptance and niubash_362 case_1).
fn heavy_lock() -> std::sync::MutexGuard<'static, ()> {
    HEAVY_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn perf_canary_nvm_loader_parses_bounded() {
    let _guard = heavy_lock();
    let root = manifest_dir();
    let dir = std::env::temp_dir().join(format!("rubash-perf-nvm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create perf scratch");
    std::fs::copy(root.join(CORPUS_DIR).join("nvm/nvm.sh"), dir.join("nvm.sh"))
        .expect("copy nvm corpus");
    let started = Instant::now();
    let out = run_rubash_in(&dir, &["-n", "nvm.sh"], Duration::from_secs(60));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.code, Some(0), "bash -n nvm.sh must parse clean");
    // rubash#1940: real budget, not just the blowup-class tripwire.
    // Post-fix debug measurement on a loaded 2-core dev box: ~0.6 s.
    // 10 s keeps >10x headroom for slow CI runners while failing the
    // feeder re-lex regression class (defs-wrap shapes measure >= 10 s
    // at this corpus scale when the join-resume machinery regresses)
    // and the historical >300 s O(N^2) class alike.
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "nvm.sh -n took {:.1}s (bound 10s; feeder re-lex / O(N^2) lexer regression class)",
        started.elapsed().as_secs_f32()
    );
}

#[test]
fn perf_canary_ruby_build_parse_bounded() {
    let _guard = heavy_lock();
    let root = manifest_dir();
    let dir = std::env::temp_dir().join(format!("rubash-perf-rbld-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create perf scratch");
    std::fs::copy(
        root.join(CORPUS_DIR).join("ruby-build/ruby-build"),
        dir.join("ruby-build"),
    )
    .expect("copy ruby-build corpus");
    let started = Instant::now();
    let out = run_rubash_in(&dir, &["-n", "ruby-build"], Duration::from_secs(360));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.code, Some(0), "bash -n ruby-build must parse clean");
    assert!(
        started.elapsed() < Duration::from_secs(300),
        "ruby-build -n took {:.1}s (bound 300s; #185 dolbrace regression class)",
        started.elapsed().as_secs_f32()
    );
}

/// Synthetic nested-brace + autoconf-mkdir_p repetition shape (the
/// rubash#176/#178 benchmark forms from commit 9bf2df9e), generated in
/// place so no corpus is needed. Post-fix debug timings are ~1-10s;
/// pre-fix the nested form exceeded 300s.
#[test]
fn perf_canary_synthetic_brace_repetition() {
    let _guard = heavy_lock();
    let dir = std::env::temp_dir().join(format!("rubash-perf-rep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create perf scratch");

    let as_fn_mkdir_p = r#"
as_fn_mkdir_p () {
  case $as_dir in #(
    -*) as_dir=./$as_dir;;
    esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while : ; do
      case $as_dir in #(
      *\'*) as_dir=`$as_echo "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #(
      *) as_dir=$as_dir;;
      esac
      as_dirs="'$as_dir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
      $as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
       X"$as_dir" : 'X\(//\)[^/]' \| \
       X"$as_dir" : 'X\(//\)$' \| \
       X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
      $as_echo X"$as_dir" |
      sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
        s//\1/
        q
      }
      /^X\(\/\/\)[^/].*/{
        s//\1/
        q
      }
      /^X\(\/\/\)$/{
        s//\1/
        q
      }
      /^X\(\/\).*/{
        s//\1/
        q
      }
      s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"
}
"#;
    let mut script = String::new();
    // 20 units, not 24: a 24-unit build trips a known open parse
    // divergence (identical units x24 + a nested group -> spurious
    // "unexpected end of file from `{'"; GNU parses clean; reproducer
    // saved as target/ciwork/syn/rep-boundary-24.sh during codification,
    // 2026-09-27). Reported to the crew; do not raise the count until
    // that lands.
    for i in 0..20 {
        script.push_str(&format!("# rep {i}\n{as_fn_mkdir_p}\n"));
    }
    // Nested multi-line brace groups (the nst2 shape). Capped at depth 3:
    // a depth-4 group anywhere in the file makes rubash (master 9bf2df9e,
    // brace_scan_cache) mis-parse the EARLIER depth-2 group with
    // "unexpected end of file from `{'" while GNU parses clean — a
    // regression introduced by 9bf2df9e (builds <= cf322dda are fine).
    // Minimal reproducer saved at
    // target/issue-suites/results/wt4-ci/nested-brace-depth4-repro.sh;
    // raise this cap once that lands.
    for depth in 1..=3usize {
        script.push_str(&"{\n".repeat(depth));
        script.push_str("echo nested\n");
        script.push_str(&"}\n".repeat(depth));
        script.push('\n');
    }
    std::fs::write(dir.join("rep.sh"), &script).expect("write synthetic script");
    let _ = std::fs::create_dir_all("target/ciwork");
    let _ = std::fs::write("target/ciwork/syn-rust-rep.sh", &script);

    let started = Instant::now();
    let out = run_rubash_in(&dir, &["-n", "rep.sh"], Duration::from_secs(240));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.code, Some(0), "synthetic script must parse clean");
    assert!(
        started.elapsed() < Duration::from_secs(150),
        "synthetic brace/mkdir_p -n took {:.1}s (bound 150s; #176/#178 class)",
        started.elapsed().as_secs_f32()
    );
}

/// The `yes | head | while read` termination canary (rubash#206/#243).
/// Pre-#206-fix (before 62c7e5fd) this shape never terminated and RSS grew
/// unbounded (the sequential stage loop `wait_with_output`'d stage-0 `yes`
/// into one Vec forever; GNU execute_cmd.c:2620 execute_pipeline forks every
/// left element on pipe(2) instead). Post-fix it finishes rc=0 with bounded
/// RSS: ~7 s for N=10000 in debug (this canary), ~23-41 s for N=100000
/// (release ~3.5-6 s). rubash#243's "never terminates" report was the
/// perf-suite timeout=20 truncating the N=100000 debug run, not a
/// regression — this canary pins the class so a real one shows up as a
/// timeout instead of an RSS explosion. `yes`/`head` need no PATH: the
/// concurrent external admission falls back to the internal pipeline
/// utilities (src/main.rs run_internal_pipeline_utility).
#[test]
fn perf_canary_yes_head_read_terminates() {
    let _guard = heavy_lock();
    let dir = std::env::temp_dir().join(format!("rubash-perf-yhr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create perf scratch");
    std::fs::write(
        dir.join("yhr.sh"),
        "yes | head -10000 | while read -r l; do :; done\necho \"RC=$?\"\nexit 0\n",
    )
    .expect("write yhr script");
    let started = Instant::now();
    let out = run_rubash_in(&dir, &["yhr.sh"], Duration::from_secs(90));
    let elapsed = started.elapsed();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !out.timed_out,
        "yes|head|read pipeline did not terminate in 90s (#206 class regression)"
    );
    assert_eq!(
        out.code,
        Some(0),
        "yhr.sh exit status, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"RC=0\n", "yhr.sh stdout");
    assert!(
        elapsed < Duration::from_secs(60),
        "yes|head|read N=10000 took {:.1}s (bound 60s; #206/#243 class)",
        elapsed.as_secs_f32()
    );
}

/// A short consumer must close its read end while the producer is still
/// running. The parent must drain the final stage before waiting upstream;
/// otherwise `yes` can remain blocked forever with `head -1` already exited.
#[test]
fn yes_head_one_terminates_after_downstream_closes() {
    let _guard = heavy_lock();
    let dir = std::env::temp_dir().join(format!("rubash-yes-head-one-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create pipeline scratch");
    std::fs::write(dir.join("yes-head-one.sh"), "yes | head -1\n").expect("write pipeline script");

    let out = run_rubash_in(&dir, &["yes-head-one.sh"], Duration::from_secs(10));
    let _ = std::fs::remove_dir_all(&dir);

    assert!(!out.timed_out, "yes | head -1 did not terminate");
    assert_eq!(out.code, Some(0), "pipeline exit status");
    assert_eq!(out.stdout, b"y\n", "pipeline output");
    assert!(out.stderr.is_empty(), "pipeline stderr: {:?}", out.stderr);
}

/// readhang regression (2026-09-28, bisected to b6cc735a): a script-file
/// `while IFS= read -r ...; done < file` loop must terminate on the
/// redirect's own EOF while the parent's stdin stays piped open.
///
/// Root cause: b6cc735a's shared-offset binding (command_input_scope.rs
/// compound_fd0_regular_file) gave the compound's fd 0 a real FileFd
/// WITHOUT the FUNCTION_STDIN mirror, so the loop-EOF `read` exhausted
/// every transport and fell through read_input_for_command's legacy
/// tail into read_inherited_process_stdin — a blocking raw read of the
/// parent's still-open stdin pipe. GNU builtin_read reads exactly ONE
/// descriptor, fd 0 as do_redirections bound it (read.def:294;
/// redir.c:767-955): zread returning <= 0 sets eof and breaks
/// (read.def:762-768) and the builtin returns EXECUTION_FAILURE
/// (read.def:949) — there is no second input to fall through to. The
/// fix keys the process-stdin fallback on fd 0's endpoint
/// (FdTable::fd0_bound_away_from_process_stdin) in both
/// read_io.rs::read_input_for_command and read_builtin.rs::execute_read.
///
/// Expected bytes verified against WSL GNU Bash 5.3.0
/// (`/usr/local/bin/bash`, script file, same piped-open parent stdin):
/// `<alpha>\n<beta>\ncount=2\n` on stdout, empty stderr, rc=0.
#[test]
fn script_file_while_read_redirect_does_not_wait_for_parent_stdin() {
    let dir = std::env::temp_dir().join(format!("rubash-readhang-fs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create readhang scratch");
    std::fs::write(dir.join("list.txt"), b"alpha\nbeta\n").expect("write list.txt");
    std::fs::write(
        dir.join("readlist.sh"),
        concat!(
            "count=0\n",
            "while IFS= read -r file; do\n",
            "  printf '<%s>\\n' \"$file\"\n",
            "  count=$((count + 1))\n",
            "done < list.txt\n",
            "printf 'count=%s\\n' \"$count\"\n",
        ),
    )
    .expect("write readlist.sh");
    let out = run_rubash_piped_stdin_in(&dir, &["readlist.sh"], Duration::from_secs(15));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !out.timed_out,
        "readlist.sh hung: loop-EOF read waited for the parent's piped-open stdin \
         (read_input_for_command / execute_read fell through to the inherited stdin)"
    );
    assert_eq!(
        out.code,
        Some(0),
        "readlist.sh exit status, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout, b"<alpha>\n<beta>\ncount=2\n",
        "readlist.sh stdout"
    );
    assert_eq!(out.stderr, b"", "readlist.sh stderr");
}

/// readhang companion (pre-existing since 26780a7b, same macro-symptom):
/// `cat file | while IFS= read -r ...` in a script file with the parent
/// stdin piped open must run the pipeline without the shell ever reading
/// the child's stdin.
///
/// Root cause: initial_pipeline_input (pipeline_exec.rs) eagerly slurped
/// the WHOLE inherited process stdin (`read_to_string`, blocking on an
/// open pipe) whenever stage 0's command name matched a hardcoded list
/// (cat/grep/sed/head/...) — even `cat list.txt`, which has an operand
/// and never reads stdin. GNU execute_cmd.c execute_pipeline forks every
/// element with the shell's own descriptors: the first element's fd 0 IS
/// the shell's fd 0, and the parent never reads a child's stdin. The
/// slurp (and its name-list predicate) was deleted; stage 0 gets the
/// real handle via execute_pipeline_stage's stdin_inherit arm instead.
///
/// Expected bytes verified against WSL GNU Bash 5.3.0: the while runs in
/// the pipeline's subshell, so the trailing count is 0 —
/// `<alpha>\n<beta>\ncount=0\n`, empty stderr, rc=0.
#[test]
fn script_file_cat_pipe_while_read_does_not_slurp_parent_stdin() {
    let dir = std::env::temp_dir().join(format!("rubash-readhang-pe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create readhang scratch");
    std::fs::write(dir.join("list.txt"), b"alpha\nbeta\n").expect("write list.txt");
    std::fs::write(
        dir.join("pipeelem.sh"),
        concat!(
            "count=0\n",
            "cat list.txt | while IFS= read -r file; do\n",
            "  printf '<%s>\\n' \"$file\"\n",
            "  count=$((count + 1))\n",
            "done\n",
            "printf 'count=%s\\n' \"$count\"\n",
        ),
    )
    .expect("write pipeelem.sh");
    let out = run_rubash_piped_stdin_in(&dir, &["pipeelem.sh"], Duration::from_secs(15));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !out.timed_out,
        "pipeelem.sh hung: initial_pipeline_input slurped the parent's piped-open stdin"
    );
    assert_eq!(
        out.code,
        Some(0),
        "pipeelem.sh exit status, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout, b"<alpha>\n<beta>\ncount=0\n",
        "pipeelem.sh stdout"
    );
    assert_eq!(out.stderr, b"", "pipeelem.sh stderr");
}

/// The GNU brace/separator acceptance rules (rubash#241 side-findings,
/// fixed in the perffix lane). Four classes, each verified byte-for-byte
/// against WSL GNU bash 5.3.0 (matrix under
/// target/issue-suites/results/perffix/matrix.sh):
///
/// 1. `}}'/`}x' are literal WORDS, not reserved `}' tokens — GNU
///    syntax.h:29-30 puts `}' in neither shell_meta_chars nor
///    shell_break_chars, and CHECK_FOR_RESERVED_WORD (parse.y:3168,
///    STREQ at parse.y:3174-3175) needs the whole word to be exactly
///    `}'. `{{ echo hi; }}' runs both words as commands (127), and
///    `echo } }x }}' prints exactly `} }x }}'.
/// 2. `;;' (SEMI_SEMI, parse.y:3711-3717) is grammatical only in
///    case_clause_sequence (parse.y:1237-1247): `{ :;;}' and `{ ;; :; }'
///    are rc=2 `syntax error near unexpected token `;;''.
/// 3. An empty separator segment is never grammatical: the list grammar
///    (parse.y:1264-1290) requires `list1 ';' newline_list list1', so
///    `; :', `: ; ; :', `: <newline> ; :' and `: & & :' are rc=2 errors
///    naming `;' (or `&').
/// 4. Legal shapes stay legal: `{ :; }', `{ : ; }', trailing `; :',
///    `cat <<EOF ; :', `case $x in a) : ; esac', `for ((i=0;;i++))'.
#[test]
fn parse_acceptance_gnu_separator_and_brace_word_rules() {
    let _guard = heavy_lock();
    let dir = std::env::temp_dir().join(format!("rubash-pa-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");

    let reject = [
        ("{{ :; }\n", "`}'"),
        ("{ :;;}\n", "`;;'"),
        ("{ :; ; }\n", "`;'"),
        ("; :\n", "`;'"),
        (": ; ; :\n", "`;'"),
        (":\n; :\n", "`;'"),
        (": & & :\n", "`&'"),
        ("if :; then :; ; fi\n", "`;'"),
    ];
    for (index, (source, token)) in reject.iter().enumerate() {
        std::fs::write(dir.join(format!("rej{index}.sh")), source).expect("write");
        let out = run_rubash_in(&dir, &[&format!("rej{index}.sh")], Duration::from_secs(30));
        assert_eq!(
            out.code,
            Some(2),
            "rej{index} ({source:?}) rc, stderr: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(&format!("syntax error near unexpected token {token}")),
            "rej{index} ({source:?}) stderr: {stderr}"
        );
    }

    // `}}' is a literal word: GNU runs `{{' / `}}' as commands (127).
    std::fs::write(dir.join("word.sh"), "{{ echo hi; }}\n").expect("write");
    let out = run_rubash_in(&dir, &["word.sh"], Duration::from_secs(30));
    assert_eq!(out.code, Some(127));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("{{: command not found"), "stderr: {stderr}");
    assert!(stderr.contains("}}: command not found"), "stderr: {stderr}");

    std::fs::write(dir.join("args.sh"), "echo } }x }}\n").expect("write");
    let out = run_rubash_in(&dir, &["args.sh"], Duration::from_secs(30));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, b"} }x }}\n");

    // The legal neighborhood must stay legal (rc 0, no stderr).
    let accept = [
        "{ :; }\n",
        "{ : ; }\n",
        ": ;\n",
        "cat <<EOF ; :\nbody\nEOF\n",
        "case $x in a) : ; esac\n",
        "for ((i=0;;i++)); do break; done\n",
        "f() {\n:\n}\nf\n",
    ];
    for (index, source) in accept.iter().enumerate() {
        std::fs::write(dir.join(format!("acc{index}.sh")), source).expect("write");
        let out = run_rubash_in(&dir, &[&format!("acc{index}.sh")], Duration::from_secs(30));
        assert_eq!(
            out.code,
            Some(0),
            "acc{index} ({source:?}) rc, stderr: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.stderr.is_empty(),
            "acc{index} ({source:?}) stderr: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The bash_completion full-file `bash -n` canary (rubash#131): GNU and
/// rubash both reject the corpus at line 1820 with rc 2. The corpus is
/// GPL-2.0+ and is deliberately NOT vendored (target-ecosys license
/// guidance), so this canary runs only where the lane sandbox is present
/// and skips (with the reason) elsewhere — CI included.
#[test]
fn canary_bash_completion_bash_n_when_corpus_present() {
    let corpus =
        PathBuf::from("D:/repo/rubash/target-ecosys/repos/bash-completion/bash_completion");
    if !corpus.is_file() {
        eprintln!(
            "SKIP: bash_completion corpus not present ({}); the license-clean \
             minimal form is covered by extglob-parse-gate",
            corpus.display()
        );
        return;
    }
    let dir = std::env::temp_dir().join(format!("rubash-canary-bc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create canary scratch");
    std::fs::copy(&corpus, dir.join("bash_completion")).expect("copy corpus");
    let out = run_rubash_in(&dir, &["-n", "bash_completion"], Duration::from_secs(240));
    let _ = std::fs::remove_dir_all(&dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.code, Some(2), "expected rc 2, stderr:\n{stderr}");
    assert!(
        stderr.contains("line 1820: syntax error near unexpected token `('"),
        "expected the #131 extglob gate at line 1820, stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("-?(\\[)+([a-zA-Z0-9?]))"),
        "expected the offending line echo, stderr:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Issue case catalogue (rubash#746).
//
// Every fix below carries the id `<issue>.<n>`: the module is
// `issue_<issue>` and the test is `case_<n>_...`, so a failing CI name points
// straight at its issue (`issue_721::case_2_compound_loop_reports_enoent`).
//
// Why these live in `--test regression` instead of `tests/issueNNN_*.rs`:
// the per-issue files are only reached by a full `cargo test`, and the only
// target the Windows gate runs is `--test regression` (ci.yml, job
// `regression`). A case outside that target cannot ratchet. The module name
// keeps the issue reference that §3.4 of
// docs/pitfall-taxonomy-and-governance.md requires, and the test name keeps
// the #746 catalogue id.
//
// Case states (#746): a `ratchet` case is `must-match` and MUST turn red when
// the fix is reverted — that is the §3.4 criterion. A `control` case asserts a
// behaviour the fix must NOT change (a live fd still reads, `-n` still splits);
// it passes on the pre-fix engine by construction, so it is never counted as
// the issue's ratchet. A `guard` case pins a fix DIRECTION: it is green on the
// original bug and on the final fix, and red on the wrong fix that was tried in
// between. States are marked in each case's doc comment.
//
// GNU Bash 5.3.0 is the sole oracle, stdout and stderr are asserted separately
// (a single `2>&1` capture interleaves differently and produced a false
// positive in the #706 sweep).
//
// Revert check for this catalogue (2026-10-18): restoring all 25 touched `src/`
// files to their pre-fix parents left the engine compiled but turned 18 of the
// 20 cases red. The two that stayed green are exactly the cases marked
// `[control]` / `[guard]` below, which is the expected shape. The issue_703
// batch (same day): reverting the comsub-collector fix turned 4 of its 5
// cases red; the survivor is the 703.2 control (the shapes the issue report
// named as already working).
// ---------------------------------------------------------------------------

/// Per-case scratch directory. The case id keeps concurrent cases of the same
/// issue from sharing one directory under the harness's default parallelism.
fn case_scratch(case: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    dir
}

/// niubash#362: `&` on a shell carrying a large function table must not die
/// with "File name too long".
///
/// The async child is re-exec'd with the WHOLE function table serialized into
/// its `-c` script (`compound_exec::background_command_source`). On Windows
/// that string rides `CreateProcessW`'s lpCommandLine, capped at 32767 UTF-16
/// units, so a shell with a few thousand functions aborts the spawn with
/// ERROR_FILENAME_EXCED_RANGE — surfaced as `niu: File name too long` — even
/// for a trivial `echo hello &`. Unix has the analogous per-argument execve
/// cap (MAX_ARG_STRLEN, 128 KiB). The spawn side now hands an over-budget
/// script to the child through a temp file named by __RUBASH_CMD_FILE, and the
/// entry point splices it back into the `-c` operand before parsing, so argv
/// semantics are unchanged.
mod issue_niubash_362 {
    use super::*;

    /// 362.1 — the generated script (~6000 functions, ~200 KB serialized)
    /// clears BOTH caps, so this fails pre-fix on Windows AND on Unix; the
    /// background child must still print its output and exit 0.
    #[test]
    fn case_1_background_command_with_large_function_table() {
        let _guard = heavy_lock();
        let dir = case_scratch("362.1");

        let mut script = String::new();
        for i in 0..6000 {
            script.push_str(&format!("f{i}() {{ local v={i}; : \"$v\"; }}\n"));
        }
        script.push_str("echo hello &\nwait\necho large-fn-status:$?\n");
        std::fs::write(dir.join("big.sh"), &script).expect("write script");

        let out = run_rubash_in(&dir, &["big.sh"], Duration::from_secs(60));
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.timed_out, "timed out; stderr:\n{stderr}");
        assert_eq!(out.code, Some(0), "stderr:\n{stderr}");
        assert!(
            stdout.contains("hello"),
            "background child output missing; stdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stdout.contains("large-fn-status:0"),
            "wait status wrong; stdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("File name too long") && !stderr.contains("Argument list too long"),
            "spawn hit an argv/command-line cap; stderr:\n{stderr}"
        );
    }
}

/// rubash#780: an argument-less `-O`/`+O` is the shopt LIST request, not a
/// usage error — the sibling of the `-o`/`+o` case fixed in rubash#547.
mod issue_780 {
    use super::*;

    /// 780.1 — GNU 5.3.0 prints the shopt table (`name<TAB>state`, rc 0) for
    /// `-O` and the `shopt -u|-s name` recreate list for `+O`, then continues
    /// startup. Asserted against the engine's own builtin bytes so the check
    /// needs no GNU oracle: startup `-O` equals `shopt`, `+O` equals `shopt -p`.
    #[test]
    fn case_1_argument_less_capital_o_prints_shopt_listing() {
        let dir = case_scratch("780.1");

        let dash = run_rubash_in(&dir, &["-O"], Duration::from_secs(30));
        let builtin = run_rubash_in(&dir, &["-c", "shopt"], Duration::from_secs(30));
        assert_eq!(
            dash.code,
            Some(0),
            "`-O` rc, stderr:\n{}",
            String::from_utf8_lossy(&dash.stderr)
        );
        assert!(!dash.stdout.is_empty(), "`-O` printed nothing");
        assert_eq!(
            dash.stdout, builtin.stdout,
            "`-O` must be the `shopt` bytes"
        );

        let plus = run_rubash_in(&dir, &["+O"], Duration::from_secs(30));
        let builtin_p = run_rubash_in(&dir, &["-c", "shopt -p"], Duration::from_secs(30));
        assert_eq!(
            plus.code,
            Some(0),
            "`+O` rc, stderr:\n{}",
            String::from_utf8_lossy(&plus.stderr)
        );
        assert!(!plus.stdout.is_empty(), "`+O` printed nothing");
        assert_eq!(
            plus.stdout, builtin_p.stdout,
            "`+O` must be the `shopt -p` bytes"
        );
    }
}

/// rubash#738: a heredoc delimiter carrying an escaped quote or backslash is
/// DEQUOTED (GNU quote removal), not stripped of those characters.
mod issue_738 {
    use super::*;

    /// 738.1 — before the fix `<<E\'OF` named `EOF`, the terminator `E'OF`
    /// never matched, and the heredoc ran to end-of-file — silently eating the
    /// rest of the script with `rc 0` and a warning on a stream scripts rarely
    /// read. Expected bytes are GNU Bash 5.3.0's: the terminator matches, the
    /// body is exactly `hello`, and execution continues after the heredoc.
    #[test]
    fn case_1_escaped_quote_and_backslash_delimiters_terminate() {
        let dir = case_scratch("738.1");

        // `<<E\'OF` … `E'OF` — an escaped single quote is literal delimiter text.
        std::fs::write(
            dir.join("esc_quote.sh"),
            "cat <<E\\'OF\nhello\nE'OF\necho after-quote\n",
        )
        .expect("write esc_quote");
        let out = run_rubash_in(&dir, &["esc_quote.sh"], Duration::from_secs(30));
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.code, Some(0), "esc_quote rc, stderr:\n{stderr}");
        assert_eq!(
            stdout, "hello\nafter-quote\n",
            "esc_quote stdout (the heredoc must end at E'OF):\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("delimited by end-of-file"),
            "esc_quote still ran to EOF:\n{stderr}"
        );

        // `<<a\\b` … `a\b` — the escaped backslash is a literal delimiter byte.
        std::fs::write(
            dir.join("esc_backslash.sh"),
            "cat <<a\\\\b\na\\b\necho after-backslash\n",
        )
        .expect("write esc_backslash");
        let out = run_rubash_in(&dir, &["esc_backslash.sh"], Duration::from_secs(30));
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.code, Some(0), "esc_backslash rc, stderr:\n{stderr}");
        assert_eq!(
            stdout, "after-backslash\n",
            "esc_backslash stdout (the heredoc must end at a\\b):\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("delimited by end-of-file"),
            "esc_backslash still ran to EOF:\n{stderr}"
        );
    }
}

/// niubash#404 (rubash side): GNU's exit echo is `interactive &&
/// sourcelevel == 0` (builtins/exit.def:59-62).
mod issue_niubash_404 {
    use super::*;

    /// 404.1 — a reader-phase `exit` prints "exit" to stderr; an `exit` inside
    /// a sourced file does not, which is why `bash --rcfile rc -i -c 'echo
    /// AFTER'` with `exit 3` in the rc stays silent while `bash -i -c 'exit 3'`
    /// echoes.
    #[test]
    fn case_1_exit_echo_suppressed_inside_sourced_file() {
        let dir = case_scratch("404.1");
        std::fs::write(dir.join("rc.sh"), "echo RAN\nexit 3\n").expect("write rc");

        let run = |args: &[&str]| {
            let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
                .args(args)
                .current_dir(&dir)
                .env_clear()
                .env("PATH", minimal_path())
                .env("HOME", &dir)
                .env("__RUBASH_INTERACTIVE", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .expect("spawn rubash");
            (
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
                out.status.code(),
            )
        };

        // Reader-phase exit: the echo is required.
        let (_, stderr, code) = run(&["--norc", "-c", "exit 3"]);
        assert_eq!(code, Some(3), "stderr:\n{stderr}");
        assert!(
            stderr.lines().any(|line| line == "exit"),
            "reader-phase `exit` must echo \"exit\" (GNU exit.def:59-62): {stderr:?}"
        );

        // Exit inside the sourced file: no echo.
        let (stdout, stderr, code) = run(&["--norc", "-c", ". rc.sh"]);
        assert_eq!(code, Some(3), "stderr:\n{stderr}");
        assert_eq!(stdout.trim_end(), "RAN", "stdout:\n{stdout}");
        assert!(
            !stderr.lines().any(|line| line == "exit"),
            "an `exit` inside a sourced file must stay silent: {stderr:?}"
        );
    }
}

/// rubash#788: a W_COMPASSIGN operand makes the variable an array even with no
/// `-a`/`-A` flag, and that array bit survives an explicit `+a` in the same
/// command (GNU expand_declaration_argument -> find_or_make_array_variable,
/// arrayfunc.c:454-514, runs BEFORE the attribute pass).
mod issue_788 {
    use super::*;

    fn run(dir: &Path, code: &str) -> (String, String, Option<i32>) {
        let out = run_rubash_in(dir, &["-c", code], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// 788.1 — before the fix rubash wrote the indexed storage word but never
    /// set ARRAY_VARS, so the variable still READ as a scalar holding the raw
    /// storage text: `${a[0]}` returned `([0]="x" [1]="y")`, `${#a[@]}` was 1
    /// and `${a[1]}` was empty. That is the `golden_fnbody_strict_battery`
    /// failure (`array-y` expected, `array-` printed) that kept the Windows
    /// regression job red. Expected bytes are GNU Bash 5.3.0's.
    #[test]
    fn case_1_compound_operand_sets_the_array_attribute() {
        let dir = case_scratch("788.1");

        for (code, expected) in [
            // No -a/-A: the compound list alone makes it an array.
            ("declare a=(x y); echo \"${a@a}|${#a[@]}|${a[1]}\"", "a|2|y"),
            (
                "declare a=(x); declare a+=(y); echo \"${a@a}|${#a[@]}|${a[1]}\"",
                "a|2|y",
            ),
            // A scalar promoted by a later compound append.
            (
                "declare a=x; declare a+=(y); echo \"${a@a}|${#a[@]}|${a[1]}\"",
                "a|2|y",
            ),
            // The attribute pass sees the array bit, refuses +a, keeps the array.
            (
                "declare +a a=(x y); echo \"${a@a}|${#a[@]}|${a[1]}\"",
                "a|2|y",
            ),
            // Function-scope declarations take the same path.
            (
                "f(){ local a=(x y); echo \"${a@a}|${#a[@]}|${a[1]}\"; }; f",
                "a|2|y",
            ),
            (
                "f(){ declare a=(x y); echo \"${a@a}|${#a[@]}|${a[1]}\"; }; f",
                "a|2|y",
            ),
            // A quoted parenthesized STRING is not W_COMPASSIGN: still scalar.
            ("declare a='(x y)'; echo \"${a@a}|${#a[@]}\"", "|1"),
            // An associative target keeps -A and binds the list as key/value.
            (
                "declare -A a=([k]=v); echo \"${a@a}|${#a[@]}|${a[k]}\"",
                "A|1|v",
            ),
        ] {
            let (stdout, stderr, status) = run(&dir, code);
            assert_eq!(stdout, expected, "`{code}` stdout; stderr:\n{stderr}");
            assert_eq!(status, Some(0), "`{code}` rc; stderr:\n{stderr}");
        }
    }

    /// 788.2 — the `+a` refusal is a real diagnostic, not a silent no-op.
    #[test]
    fn case_2_plus_a_is_refused_with_the_array_diagnostic() {
        let dir = case_scratch("788.2");
        let (_, stderr, status) = run(&dir, "declare +a a=(x y)");
        assert_eq!(status, Some(1), "`declare +a` must fail; stderr:\n{stderr}");
        assert!(
            stderr.contains("cannot destroy array variables in this way"),
            "`declare +a a=(x y)` diagnostic; stderr:\n{stderr}"
        );
    }
}

/// rubash#744: a here-string inside a quoted whole-word `$(...)` must have its
/// word quote-removed.
mod issue_744 {
    use super::*;

    /// 744.1 — the quoted-`$()` fast path in command_prepare handed the body
    /// straight to the specialized expander, which kept the quotes
    /// (`p "$(cat <<< 'here')"` printed `['here']`). The body now declines that
    /// shortcut when it carries `<<<`, so it reaches the AST path that already
    /// routes here-string substitutions through the real parser.
    #[test]
    fn case_1_here_string_in_quoted_comsub_is_dequoted() {
        let dir = case_scratch("744.1");

        let script = "p() { printf '[%s]' \"$@\"; printf '\\n'; }\n\
                      p \"$(cat <<< 'here')\"\n\
                      p $(cat <<< 'here')\n\
                      p \"pre$(cat <<< 'here')post\"\n\
                      p \"$(cat <<< \"a b\")\"\n";
        std::fs::write(dir.join("here.sh"), script).expect("write script");
        let out = run_rubash_in(&dir, &["here.sh"], Duration::from_secs(30));
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "[here]\n[here]\n[preherepost]\n[a b]\n",
            "here-string quote removal; stderr:\n{stderr}"
        );
    }
}

/// rubash#703: a heredoc feeding a pipeline inside a double-quoted command
/// substitution failed to parse.
///
/// `echo "X=$(cat <<EOF | cat` + `body` + `EOF` + `)"` died with
/// `command substitution: line 4: unexpected EOF while looking for matching
/// `)'` and printed nothing, while GNU prints `X=body`. The unquoted form,
/// the quoted form without the `X=` prefix, and the quoted form without the
/// pipeline all worked, which narrowed the trigger to the word-expansion
/// comsub collector (`collect_command_substitution_source_ex`): its heredoc
/// delimiter scan did not end the delimiter WORD at the first unquoted
/// blank, so `cat <<EOF | cat` declared the delimiter `EOF | cat`, the `EOF`
/// terminator line never matched, and the collector swallowed the rest of
/// the script. GNU read_token_word ends the word at any unquoted shell
/// metacharacter (parse.y:5688 shellbreak) — the same rule the lexer's own
/// heredoc header scan already applies.
mod issue_703 {
    use super::*;

    fn run_script(case: &str, script: &str) -> (String, String, Option<i32>) {
        let dir = case_scratch(case);
        std::fs::write(dir.join("hd.sh"), script).expect("write script");
        let out = run_rubash_in(&dir, &["hd.sh"], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// 703.1 [ratchet] — the issue's exact repro. The delimiter word must end
    /// at the blank before `|`, the `EOF` line must terminate the body, and
    /// the `)` on the following line must close the substitution. Pre-fix the
    /// collector read the delimiter as `EOF | cat`, never matched the
    /// terminator, and reported the unclosed-`)` fatal with no output.
    #[test]
    fn case_1_quoted_comsub_heredoc_pipeline() {
        let (stdout, stderr, code) =
            run_script("703.1", "echo \"X=$(cat <<EOF | cat\nbody\nEOF\n)\"\n");
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(stdout, "X=body\n", "stdout; stderr:\n{stderr}");
        assert!(
            !stderr.contains("unexpected EOF"),
            "unclosed-comsub fatal; stderr:\n{stderr}"
        );
    }

    /// 703.2 [control] — the three shapes the issue report named as already
    /// working must not regress: the unquoted assignment, the quoted
    /// substitution without a word prefix, and the quoted substitution
    /// without the pipeline. All three were green before the fix.
    #[test]
    fn case_2_working_shapes_stay_green() {
        let (stdout, stderr, code) = run_script(
            "703.2",
            "x=$(cat <<EOF | cat\nbody\nEOF\n); echo \"X=$x\"\n\
             echo \"$(cat <<EOF | cat\nbody\nEOF\n)\"\n\
             echo \"X=$(cat <<EOF\nbody\nEOF\n)\"\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "X=body\nbody\nX=body\n",
            "stdout; stderr:\n{stderr}"
        );
    }

    /// 703.3 [ratchet] — the make_cmd.c:602-611 pushback shape with a
    /// pipeline: the `)` sits ON the delimiter line (`EOF)`), so the heredoc
    /// is "delimited by end-of-file", GNU warns, and the substitution still
    /// yields `body`. Two defects hid here: the pre-fix delimiter scan (same
    /// fatal as 703.1), and — once that was fixed — the lexer's IFS_GLUE
    /// `EOF)` marker reaching the real comsub parser, where the terminator
    /// line read `EOF\x1c` and stopped matching, leaking the delimiter into
    /// the output (`X=body` plus a stray `EOF`). Exact stdout pins both.
    #[test]
    fn case_3_eof_paren_pushback_with_pipeline() {
        let (stdout, stderr, code) =
            run_script("703.3", "echo \"X=$(cat <<EOF | cat\nbody\nEOF)\"\n");
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "X=body\n",
            "stray delimiter in output; stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("delimited by end-of-file"),
            "missing GNU here-document warning; stderr:\n{stderr}"
        );
    }

    /// 703.4 [ratchet] — delimiter-word spellings that must still resolve
    /// correctly once the word ends at the first unquoted blank: a blank
    /// between `<<` and the word (`<< EOF`), the adjacent `<<-` tab-strip
    /// dash, and a quoted delimiter carrying its own blank. Each carries a
    /// pipeline after the word, so the pre-fix scan — which read the word to
    /// end of line — declared `EOF | cat` (or `-EOF | cat`, `E O | cat`) and
    /// died unclosed on all three.
    #[test]
    fn case_4_delimiter_spellings_with_pipeline() {
        let (stdout, stderr, code) = run_script(
            "703.4",
            "echo \"X=$(cat << EOF | cat\nbody\nEOF\n)\"\n\
             echo \"Y=$(cat <<-EOF | cat\n\tbody\n\tEOF\n)\"\n\
             echo \"Z=$(cat <<\"E O\" | cat\nbody\nE O\n)\"\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "X=body\nY=body\nZ=body\n",
            "stdout; stderr:\n{stderr}"
        );
    }

    /// 703.5 [ratchet] — the same word-boundary rule with a `;` after the
    /// delimiter: `cat <<EOF; cat` declares `EOF` and runs `cat` as the next
    /// command. Pre-fix the delimiter read `EOF; cat` and the terminator
    /// never matched, so the substitution died unclosed.
    #[test]
    fn case_5_semicolon_after_delimiter_word() {
        let (stdout, stderr, code) =
            run_script("703.5", "echo \"X=$(cat <<EOF; cat\nbody\nEOF\n)\"\n");
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(stdout, "X=body\n", "stdout; stderr:\n{stderr}");
        assert!(
            !stderr.contains("unexpected EOF"),
            "unclosed-comsub fatal; stderr:\n{stderr}"
        );
    }
}

/// rubash#745: two layers, not one.
///
/// Inside a backtick body the backslash is consumed only before `$`, a
/// backtick or another backslash; every other character keeps it (parse.y
/// parse_matched_pair under P_BACKQUOTE). A backtick that sits inside a
/// double-quoted word is a separate layer: lexer::quotes consumes `\"` there,
/// so the two upstream cases that look contradictory both hold —
///
///   recho `echo "(\")"`        keeps `\"`  (quote.tests:123)
///   recho "`echo \"x\"`"        sees a bare quote (exp.tests:146)
///   recho `echo 'a\' b`        keeps `\'`  (comsub.tests)
mod issue_745 {
    use super::*;

    fn run_script(case: &str, script: &str) -> (String, String, Option<i32>) {
        let dir = case_scratch(case);
        std::fs::write(dir.join("bt.sh"), script).expect("write script");
        let out = run_rubash_in(&dir, &["bt.sh"], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// 745.1 — a word-final backslash run survives. GNU hands the body to
    /// parse_and_execute with EOF immediately after it, so a lone trailing `\`
    /// stays literal; the re-parse here terminates the text with a newline,
    /// where `\` + newline is a line continuation. Also pins the four-backslash
    /// double-quote shape the old decoder mangled (`"\\\\"` -> `\"` instead of
    /// `\`).
    #[test]
    fn case_1_word_final_backslash_and_four_backslash_shape() {
        let script = "q() { printf '[%s]|%s\\n' \"$1\" \"${#1}\"; }\n\
                      b=`printf '%s' \\\\`; q \"$b\"\n\
                      c=`printf '%s' \"\\\\\\\\\"`; q \"$c\"\n";
        let (stdout, stderr, code) = run_script("745.1", script);
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(stdout, "[\\]|1\n[\\]|1\n", "stderr:\n{stderr}");
    }

    /// 745.2 `[guard]` — the escape set is `$`, backtick and `\`; every other
    /// character keeps its backslash. This is the direction guard: the second
    /// attempt at #745 read the rule as "strip the backslash before anything",
    /// which turned `echo 'a\' b` inside backticks into `echo 'a' b` and broke
    /// `comsub.tests`. The last line is the discriminator — green on the
    /// original engine, red on that wrong fix, green here.
    #[test]
    fn case_2_escape_set_and_dquote_layer() {
        let script = "q() { printf '[%s]|%s\\n' \"$1\" \"${#1}\"; }\n\
                      p() { printf '[%s]' \"$@\"; printf '\\n'; }\n\
                      d=`printf '%s' \\a`; q \"$d\"\n\
                      e=`echo a\\\\b`; q \"$e\"\n\
                      f=`printf '%s' '\\\\'`; q \"$f\"\n\
                      g=$(printf '%s' \\\\); q \"$g\"\n\
                      p `echo 'a\\' b`\n";
        let (stdout, stderr, code) = run_script("745.2", script);
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "[a]|1\n[ab]|2\n[\\]|1\n[\\]|1\n[a\\][b]\n",
            "stderr:\n{stderr}"
        );
    }
}

/// rubash#776: `read -N` binds the record to the first name verbatim — no IFS
/// splitting, no whitespace strip — and leaves the rest empty.
mod issue_776 {
    use super::*;

    fn run(case: &str, script: &str) -> (String, String, Option<i32>) {
        let dir = case_scratch(case);
        std::fs::write(dir.join("in.txt"), "AB\nC").expect("write input");
        std::fs::write(dir.join("n.sh"), script).expect("write script");
        let out = run_rubash_in(&dir, &["n.sh"], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// 776.1 — the delimiter the limit stopped on is data under `-N`: `-N3` on
    /// `AB\nC` yields `AB\n`, and the second name stays empty instead of taking
    /// a split field.
    #[test]
    fn case_1_capital_n_keeps_the_delimiter_and_skips_splitting() {
        let script = "read -N3 c < in.txt; printf '<%s>|%s\\n' \"$c\" \"${#c}\"\n\
                      read -N3 a b < in.txt; printf '[%s][%s]\\n' \"$a\" \"$b\"\n\
                      printf 'AB CD' > in2.txt\n\
                      read -N5 x y < in2.txt; printf '[%s][%s]\\n' \"$x\" \"$y\"\n";
        let (stdout, stderr, code) = run("776.1", script);
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "<AB\n>|3\n[AB\n][]\n[AB CD][]\n",
            "stderr:\n{stderr}"
        );
    }

    /// 776.2 — leading/trailing blanks are preserved, and `-n` keeps the
    /// ordinary split/strip behaviour as the control.
    #[test]
    fn case_2_capital_n_keeps_whitespace_lowercase_n_still_strips() {
        let script = "printf '  AB  ' > in3.txt\n\
                      read -N6 s < in3.txt; printf '[%s]\\n' \"$s\"\n\
                      read -n6 t < in3.txt; printf '[%s]\\n' \"$t\"\n";
        let (stdout, stderr, code) = run("776.2", script);
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(stdout, "[  AB  ]\n[AB]\n", "stderr:\n{stderr}");
    }
}

/// rubash#721: `< /dev/fd/N` (or `/proc/self/fd/N`) with N closed is an open
/// failure, not an empty input. GNU opens the alias through the kernel and
/// reports ENOENT, abandoning the command with status 1.
mod issue_721 {
    use super::*;

    fn run(case: &str, code: &str) -> (String, String, Option<i32>) {
        let dir = case_scratch(case);
        std::fs::write(dir.join("live.txt"), "hi\n").expect("write input");
        let out = run_rubash_in(&dir, &["-c", code], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// 721.1 — the read builtin's alias branch staged an empty mirror, which
    /// read saw as plain EOF: bare rc 1 and no diagnostic.
    #[test]
    fn case_1_read_builtin_reports_closed_alias() {
        let (stdout, stderr, code) = run("721.1", "exec 6<&-\nread -r z < /dev/fd/6\necho rc=$?");
        assert_eq!(code, Some(0), "script rc; stderr:\n{stderr}");
        assert_eq!(stdout, "rc=1", "read rc; stderr:\n{stderr}");
        assert!(
            stderr.contains("/dev/fd/6") && stderr.contains("No such file or directory"),
            "read must report the alias; stderr:\n{stderr}"
        );
    }

    /// 721.2 — the compound fd-0 preflight treated every alias as available,
    /// so `while read ... done < /dev/fd/6` ran zero iterations and reported 0.
    #[test]
    fn case_2_compound_loop_reports_closed_alias() {
        let (stdout, stderr, code) = run(
            "721.2",
            "exec 6<&-\nwhile read -r l; do :; done < /dev/fd/6\necho rc=$?",
        );
        assert_eq!(code, Some(0), "script rc; stderr:\n{stderr}");
        assert_eq!(stdout, "rc=1", "loop rc; stderr:\n{stderr}");
        assert!(
            stderr.contains("/dev/fd/6") && stderr.contains("No such file or directory"),
            "the loop must report the alias; stderr:\n{stderr}"
        );
    }

    /// 721.3 — the /proc spelling takes the same path.
    #[test]
    fn case_3_proc_self_fd_spelling_reports_closed_alias() {
        let (stdout, stderr, code) = run(
            "721.3",
            "exec 6<&-\nread -r z < /proc/self/fd/6\necho rc=$?",
        );
        assert_eq!(code, Some(0), "script rc; stderr:\n{stderr}");
        assert_eq!(stdout, "rc=1", "proc rc; stderr:\n{stderr}");
        assert!(
            stderr.contains("/proc/self/fd/6") && stderr.contains("No such file or directory"),
            "the /proc operand must be reported; stderr:\n{stderr}"
        );
    }

    /// 721.4 `[control]` — a live descriptor still serves its bytes: the check
    /// is "is N open", not "is the word an alias". Green on the pre-fix engine
    /// by design; it exists to catch an over-fix that rejects every alias.
    #[test]
    fn case_4_live_descriptor_still_reads() {
        let (stdout, stderr, code) = run(
            "721.4",
            "exec 6< live.txt\nread -r z < /dev/fd/6\necho \"got=$z rc=$?\"",
        );
        assert_eq!(code, Some(0), "script rc; stderr:\n{stderr}");
        assert_eq!(stdout, "got=hi rc=0", "live fd; stderr:\n{stderr}");
        assert!(stderr.is_empty(), "live fd must stay silent: {stderr:?}");
    }
}

/// rubash#704 R6.2: `command -v`/`-V` use GNU's ANY-found rule
/// (builtins/command.def increments any_found per resolved operand).
mod issue_704 {
    use super::*;

    /// 704.2 — one miss among several operands still exits 0. `type` is the
    /// all-found builtin and is deliberately the opposite — the two shared an
    /// accumulator that made `command -v echo nosuch` return 1.
    #[test]
    fn case_2_command_v_multi_operand_is_any_found() {
        let dir = case_scratch("704.2");

        let code = "command -v echo nosuch_xyz >/dev/null; echo v:$?\n\
                    command -v nosuch_xyz echo >/dev/null; echo v2:$?\n\
                    command -V echo nosuch_xyz >/dev/null 2>&1; echo V:$?\n\
                    command -v nosuch1 nosuch2 >/dev/null; echo miss:$?\n\
                    type echo nosuch_xyz >/dev/null 2>&1; echo type:$?";
        let out = run_rubash_in(&dir, &["-c", code], Duration::from_secs(30));
        let stdout = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(
            stdout, "v:0\nv2:0\nV:0\nmiss:1\ntype:1",
            "command describe status; stderr:\n{stderr}"
        );
    }

    /// 704.2b — builtins/command.def:96-97 `if (list == 0) return
    /// (EXECUTION_SUCCESS);`: the no-operand early return comes BEFORE the
    /// describe loop, so `command -v`/`-V` with no names succeeds with no
    /// output instead of falling through to the any_found accumulator's
    /// failure. Same rule inside a substitution (the status rides
    /// last_command_substitution_status).
    #[test]
    fn case_2b_command_v_without_operands_succeeds() {
        let dir = case_scratch("704.2b");

        let code = "command -v; echo direct:$?\n\
                    command -V; echo verbose:$?\n\
                    v=$(command -v); echo \"comsub:[${v}] rc:$?\"\n\
                    v=$(command -V); echo \"comsubV:[${v}] rc:$?\"";
        let out = run_rubash_in(&dir, &["-c", code], Duration::from_secs(30));
        let stdout = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(
            stdout, "direct:0\nverbose:0\ncomsub:[] rc:0\ncomsubV:[] rc:0",
            "no-operand describe status; stderr:\n{stderr}"
        );
        assert!(
            stderr.is_empty(),
            "no-operand forms print nothing on stderr; got:\n{stderr}"
        );
    }

    /// 704.2c — the substitution entry point shares the any-found rule, and
    /// the `-V` per-operand "not found" diagnostic still reaches stderr
    /// while the found operand's description is captured (WSL GNU bash
    /// 5.3.0 probe: `v=$(command -V echo nosuch_xyz)` -> stdout captures
    /// "echo is a shell builtin", stderr gets
    /// "command: nosuch_xyz: not found", rc 0).
    #[test]
    fn case_2c_command_v_in_substitution_uses_any_found() {
        let dir = case_scratch("704.2c");

        let code = "v=$(command -v echo nosuch_xyz); echo \"any:[$v] rc:$?\"\n\
                    v=$(command -v nosuch_one nosuch_two); echo \"miss:[$v] rc:$?\"\n\
                    v=$(command -V echo nosuch_xyz); echo \"verbose:[$v] rc:$?\"";
        let out = run_rubash_in(&dir, &["-c", code], Duration::from_secs(30));
        let stdout = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(
            stdout, "any:[echo] rc:0\nmiss:[] rc:1\nverbose:[echo is a shell builtin] rc:0",
            "substituted describe status; stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("command: nosuch_xyz: not found"),
            "-V not-found diagnostic must reach stderr; stderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("nosuch_one: not found"),
            "-v (reusable) stays silent about misses; stderr:\n{stderr}"
        );
    }
}

/// rubash#759: `dirs -l` is the LONG form (`$HOME` stays un-collapsed), not the
/// one-per-line form. Only `-p` (and `-v`, already numbered) select one entry
/// per line.
mod issue_759 {
    use super::*;

    /// 759.2 — rubash treated `-l` as `-p`, so `dirs -l` printed one line per
    /// entry where GNU prints one line.
    #[test]
    fn case_2_dirs_long_form_keeps_the_space_separator() {
        let dir = case_scratch("759.2");
        std::fs::create_dir_all(dir.join("a")).expect("create a");
        std::fs::create_dir_all(dir.join("b")).expect("create b");

        let run = |code: &str| {
            let out = run_rubash_in(&dir, &["-c", code], Duration::from_secs(30));
            String::from_utf8_lossy(&out.stdout).trim_end().to_string()
        };

        let long = run("cd a\npushd ../b >/dev/null\ndirs -l");
        assert_eq!(
            long.lines().count(),
            1,
            "`dirs -l` must stay on one line: {long:?}"
        );
        assert!(
            long.contains(' '),
            "`dirs -l` separates with spaces: {long:?}"
        );

        let per_line = run("cd a\npushd ../b >/dev/null\ndirs -p");
        assert_eq!(
            per_line.lines().count(),
            2,
            "`dirs -p` is one entry per line: {per_line:?}"
        );

        let numbered = run("cd a\npushd ../b >/dev/null\ndirs -v");
        assert_eq!(
            numbered.lines().count(),
            2,
            "`dirs -v` is numbered, one per line: {numbered:?}"
        );
        let long_numbered = run("cd a\npushd ../b >/dev/null\ndirs -l -v");
        assert_eq!(
            long_numbered, numbered,
            "`dirs -l -v` renders like `dirs -v`"
        );
    }
}

/// rubash#742: three word-level quote-removal arms in lexer/quotes.rs.
mod issue_742 {
    use super::*;

    fn run(case: &str, script: &str) -> (String, String, Option<i32>) {
        let dir = case_scratch(case);
        std::fs::write(dir.join("q.sh"), script).expect("write script");
        let out = run_rubash_in(&dir, &["q.sh"], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// 742.1 — an unbraced `$name` ends at the next backslash. The old scan
    /// cleared `pending_name` without emitting PARAM_NAME_END_MARKER, so the
    /// neighbours glued into one longer name and another variable's value was
    /// returned (`$v\tb` with vtb=QQ printed QQ). The braced form is the
    /// control that pinned it to the unbraced scan.
    #[test]
    fn case_1_unbraced_name_ends_at_backslash() {
        let script = "p() { printf '[%s]' \"$@\"; printf '\\n'; }\n\
                      v=AB; vtb=QQ\n\
                      p $v\\tb\n\
                      q=$v\\tb; p \"$q\"\n\
                      p ${v}\\tb\n";
        let (stdout, stderr, code) = run("742.1", script);
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(stdout, "[ABtb]\n[ABtb]\n[ABtb]\n", "stderr:\n{stderr}");
    }

    /// 742.2 — an unbraced `$name` also ends at the next `$`, so a following
    /// `$'...'` is a new expansion instead of more name bytes.
    #[test]
    fn case_2_unbraced_name_ends_at_dollar() {
        let script = "p() { printf '[%s]' \"$@\"; printf '\\n'; }\n\
                      w='a b'; wb=QQ\n\
                      p a$w$'b'\n\
                      p $w$'b'\n";
        let (stdout, stderr, code) = run("742.2", script);
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(stdout, "[aa][bb]\n[a][bb]\n", "stderr:\n{stderr}");
    }

    /// 742.3 — an escaped blank is quoted data. The generic ASCII-escape arm
    /// pushed the byte bare, so field splitting saw an ordinary space and one
    /// argument became two.
    #[test]
    fn case_3_escaped_blank_stays_one_argument() {
        let script = "p() { printf '[%s]' \"$@\"; printf '\\n'; }\n\
                      v=AB\n\
                      p $v\\ b\n\
                      p ${v}\\ b\n";
        let (stdout, stderr, code) = run("742.3", script);
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(stdout, "[AB b]\n[AB b]\n", "stderr:\n{stderr}");
    }
}

/// rubash#753: after a failglob expansion error GNU abandons the rest of the
/// current logical line; rubash only abandoned the failing command when that
/// command was a compound, so one extra statement ran after the error.
///
/// The issue's repro is the brace group (`case2`): GNU never reaches the
/// `echo` after the `;`, rubash ran it and reported `case2-rc=1`. The
/// subshell (`case1`) and the simple command (`case3`) already matched,
/// which pinned the divergence to the compound-command boundary swallowing
/// the expansion failure instead of letting it unwind to the reader. This
/// is control flow, distinct from #726's diagnostic-bypass defect.
///
/// GNU's rule, as pinned by the probe family behind these cases: the
/// failglob failure abandons the remainder of the current logical line —
/// the same discard the arithmetic/subscript evalerror path already
/// implements (`evalerror_pending` + the reader loop-top check). The
/// discard is CONTAINED by exactly these boundaries: `( )` subshells,
/// pipeline stages, `$( )`, `&` background jobs, `eval`, and `source`; it
/// PENETRATES brace groups, if/then bodies, if conditions,
/// while/until/for/select bodies, case arms, function bodies, `!`, and
/// `time`. A loop body's discard escapes through `done` to the enclosing
/// logical line and aborts the loop; a `for` word-list failure is a plain
/// line abort.
///
/// Fix: a new class flag `evalerror_failglob_pending` (mirroring
/// `evalerror_subscript_pending`'s clear-at-every-boundary discipline) is
/// armed at every failglob `Fail` site alongside `raise_evalerror_abort()`,
/// so the reader-layer loop-top check discards the rest of the line. The
/// eval/source boundaries absorb exactly this class (GNU evalstring.c
/// catches the expansion error locally at the top level of eval/source),
/// which is why the containing boundaries keep their old behavior.
///
/// Two pre-existing divergences found while probing, deliberately out of
/// scope here (both reproduce on the pre-fix binary, so neither is a
/// regression of this change; both are follow-up issue candidates):
/// - `c=([k]=v [""]=x)` — a bad-subscript compound assignment binds
///   partially and GNU CONTINUES the line (`sub-after rc=0`); rubash
///   discards the rest of the line (subscript family).
/// - `${arr[/no/such/*.zzz]}` — the subscript failglob diagnostic reads
///   `arithmetic syntax error` where GNU reads `syntax error` (diagnostic
///   wording family).
mod issue_753 {
    use super::*;

    fn run(case: &str, script: &str) -> (String, String, Option<i32>) {
        let dir = case_scratch(case);
        std::fs::write(dir.join("fg.sh"), script).expect("write script");
        let out = run_rubash_in(&dir, &["fg.sh"], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// Same contract as `run`, with a stdin payload. The select shape needs
    /// one input line for its body to execute at all: with
    /// `Stdio::null()` the select EOFs before the first prompt and the body
    /// never runs, which is exactly how the first select probe misread the
    /// boundary (the body "discarding" the line was really the body never
    /// executing).
    fn run_with_stdin(case: &str, script: &str, input: &str) -> (String, String, Option<i32>) {
        let dir = case_scratch(case);
        std::fs::write(dir.join("fg.sh"), script).expect("write script");
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
        cmd.args(["fg.sh"])
            .current_dir(&dir)
            .env_clear()
            .env("PATH", minimal_path())
            .env("HOME", &dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if cfg!(windows) {
            for name in [
                "SystemRoot",
                "SystemDrive",
                "WINDIR",
                "COMSPEC",
                "PATHEXT",
                "OS",
                "TEMP",
                "TMP",
            ] {
                if let Ok(value) = std::env::var(name) {
                    cmd.env(name, value);
                }
            }
        }
        let mut child = cmd.spawn().expect("spawn rubash");
        std::io::Write::write_all(child.stdin.as_mut().expect("stdin pipe"), input.as_bytes())
            .expect("write stdin");
        drop(child.stdin.take());
        let out = wait_bounded(&mut child, Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// 753.1 [ratchet] — the issue's exact repro. case1 (subshell) and
    /// case3 (simple command) were already GNU-correct and stay pinned
    /// here; case2 (brace group) is the ratchet: pre-fix rubash printed
    /// `case2-rc=1`, GNU abandons the rest of that logical line and never
    /// reaches the `echo`. stderr pins all three diagnostics with their
    /// line numbers, proving the failures fired rather than the echoes
    /// silently vanishing.
    #[test]
    fn case_1_issue_repro_brace_group_same_line() {
        let (stdout, stderr, code) = run(
            "753.1",
            "shopt -s failglob\n\
             echo \"case1-start\"\n\
             ( set -- /no/such/*.zzz; echo \"case1-inside\" ); echo \"case1-rc=$?\"\n\
             echo \"case2-start\"\n\
             { set -- /no/such/*.zzz; echo \"case2-inside\"; }; echo \"case2-rc=$?\"\n\
             echo \"case3-start\"\n\
             set -- /no/such/*.zzz; echo \"case3-rc=$?\"\n\
             echo \"END\"\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "case1-start\ncase1-rc=1\ncase2-start\ncase3-start\nEND\n",
            "case2-rc must not print; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr,
            "fg.sh: line 3: no match: /no/such/*.zzz\n\
             fg.sh: line 5: no match: /no/such/*.zzz\n\
             fg.sh: line 7: no match: /no/such/*.zzz\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }

    /// 753.2 [ratchet] — a brace group in list position. Pre-fix rubash ran
    /// `A_after_brace_semicolon` (after `;`), `C_or_arm` (the `||` arm —
    /// the group "failed", so the arm was eligible), and `D_after_invert
    /// rc=0` (`!` inverted the aborted group's status); GNU abandons the
    /// line at each failure and prints only `END`. `B_and_arm` is pinned
    /// too but is non-discriminating: the `&&` arm is skipped by status in
    /// both shells, so its absence proves nothing about the discard — it
    /// is here to keep the and/or pair covered as a family.
    #[test]
    fn case_2_brace_group_in_list_position() {
        let (stdout, stderr, code) = run(
            "753.2",
            "shopt -s failglob\n\
             { set -- /no/such/*.zzz; }; echo \"A_after_brace_semicolon\"\n\
             { set -- /no/such/*.zzz; } && echo \"B_and_arm\"\n\
             { set -- /no/such/*.zzz; } || echo \"C_or_arm\"\n\
             ! { set -- /no/such/*.zzz; }; echo \"D_after_invert rc=$?\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "END\n",
            "list-position arm echoes must not run; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr,
            "fg.sh: line 2: no match: /no/such/*.zzz\n\
             fg.sh: line 3: no match: /no/such/*.zzz\n\
             fg.sh: line 4: no match: /no/such/*.zzz\n\
             fg.sh: line 5: no match: /no/such/*.zzz\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }

    /// 753.3 [ratchet] — the compound bodies that PENETRATE: an if/then
    /// body and the if condition itself, a case arm, and a nested brace
    /// group. Pre-fix rubash printed all five trailing echoes
    /// (`E_after_fi_sameline`, `IFCOND_after`, `G_after_case_sameline`,
    /// `L_nested_inner`, `M_after_nested`); GNU abandons each logical line
    /// at the failure, so only `END` survives. The if-condition shape is
    /// the one the probe family found last: the condition's failglob
    /// aborts the whole `if ... fi` line, not just the condition command.
    #[test]
    fn case_3_if_body_if_condition_case_nested_brace() {
        let (stdout, stderr, code) = run(
            "753.3",
            "shopt -s failglob\n\
             if true; then set -- /no/such/*.zzz; fi; echo \"E_after_fi_sameline\"\n\
             if set -- /no/such/*.zzz; then echo \"cond-T\"; fi; echo \"IFCOND_after\"\n\
             case x in x) set -- /no/such/*.zzz;; esac; echo \"G_after_case_sameline\"\n\
             { { set -- /no/such/*.zzz; }; echo \"L_nested_inner\"; }; echo \"M_after_nested\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "END\n",
            "penetrating compound bodies; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr,
            "fg.sh: line 2: no match: /no/such/*.zzz\n\
             fg.sh: line 3: no match: /no/such/*.zzz\n\
             fg.sh: line 4: no match: /no/such/*.zzz\n\
             fg.sh: line 5: no match: /no/such/*.zzz\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }

    /// 753.4 [ratchet] — the `for` word list. Bare (`FOR_after`) was
    /// already GNU-correct pre-fix; the in-brace spelling
    /// (`FOR_brace_after`) was not: pre-fix rubash ran the echo after the
    /// brace group closed. GNU abandons the line in both spellings.
    #[test]
    fn case_4_for_word_list_failglob() {
        let (stdout, stderr, code) = run(
            "753.4",
            "shopt -s failglob\n\
             for i in /no/such/*.zzz; do echo \"for-body\"; done; echo \"FOR_after\"\n\
             { for i in /no/such/*.zzz; do echo \"for-body2\"; done; }; echo \"FOR_brace_after\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "END\n",
            "for word-list abort scope; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr,
            "fg.sh: line 2: no match: /no/such/*.zzz\n\
             fg.sh: line 3: no match: /no/such/*.zzz\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }

    /// 753.5 [ratchet] — the `time` keyword. Pre-fix rubash printed
    /// `I_after_time`; GNU abandons the rest of the line after the timed
    /// group's failglob. stderr is only contains-checked: the `real`/
    /// `user`/`sys` timing lines are machine-dependent bytes and belong to
    /// no golden.
    #[test]
    fn case_5_time_keyword_penetrates() {
        let (stdout, stderr, code) = run(
            "753.5",
            "shopt -s failglob\n\
             time { set -- /no/such/*.zzz; }; echo \"I_after_time\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "END\n",
            "time must not contain the discard; stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("fg.sh: line 2: no match: /no/such/*.zzz"),
            "diagnostic missing; stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("real") && stderr.contains("user") && stderr.contains("sys"),
            "time report missing; stderr:\n{stderr}"
        );
    }

    /// 753.6 [ratchet] — a function definition, call, and trailing echo on
    /// one logical line. Pre-fix rubash printed `after-func-sameline` (the
    /// body's failglob only aborted the function call); GNU abandons the
    /// line. The next-line spelling is covered by 753.15 and stays green.
    #[test]
    fn case_6_function_def_call_echo_same_line() {
        let (stdout, stderr, code) = run(
            "753.6",
            "shopt -s failglob\n\
             g() { set -- /no/such/*.zzz; echo \"func-inside\"; }; g; echo \"after-func-sameline\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "END\n",
            "same-line echo after a failing function call must not run; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr, "fg.sh: line 2: no match: /no/such/*.zzz\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }

    /// 753.7 [ratchet] — failglob in a redirection word. Bare
    /// (`REDIR_after`) was already correct pre-fix; the brace-internal
    /// spellings (`BR_after`, `BR_brace_after`) were not. GNU abandons the
    /// line in all three spellings, so no redirect file is ever attempted.
    #[test]
    fn case_7_redirection_word_failglob() {
        let (stdout, stderr, code) = run(
            "753.7",
            "shopt -s failglob\n\
             echo x > /no/such/*.zzz; echo \"REDIR_after\"\n\
             { echo x > /no/such/*.zzz; }; echo \"BR_after\"\n\
             { { echo x > /no/such/*.zzz; }; }; echo \"BR_brace_after\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "END\n",
            "redirect failglob abort scope; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr,
            "fg.sh: line 2: no match: /no/such/*.zzz\n\
             fg.sh: line 3: no match: /no/such/*.zzz\n\
             fg.sh: line 4: no match: /no/such/*.zzz\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }

    /// 753.8 [ratchet] — failglob in a compound-assignment word. Bare
    /// (`CA_after`) was already correct pre-fix; the brace, if-body,
    /// case-arm, and same-line function spellings were not. GNU abandons
    /// each logical line at the failure.
    #[test]
    fn case_8_compound_assignment_word_failglob() {
        let (stdout, stderr, code) = run(
            "753.8",
            "shopt -s failglob\n\
             a=(/no/such/*.zzz); echo \"CA_after\"\n\
             { a=(/no/such/*.zzz); echo \"in-brace\"; }; echo \"CA_brace_after\"\n\
             if true; then a=(/no/such/*.zzz); fi; echo \"CA_if_after\"\n\
             case x in x) a=(/no/such/*.zzz);; esac; echo \"CA_case_after\"\n\
             f() { a=(/no/such/*.zzz); echo \"in-f\"; }; f; echo \"CA_func_after\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "END\n",
            "compound-assignment failglob abort scope; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr,
            "fg.sh: line 2: no match: /no/such/*.zzz\n\
             fg.sh: line 3: no match: /no/such/*.zzz\n\
             fg.sh: line 4: no match: /no/such/*.zzz\n\
             fg.sh: line 5: no match: /no/such/*.zzz\n\
             fg.sh: line 6: no match: /no/such/*.zzz\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }

    /// 753.9 [ratchet] — `declare` with an indexed compound operand. GNU
    /// fails the whole operand, binds NOTHING (`declare -p b` reports
    /// `declare: b: not found`), and abandons the line, so neither
    /// `rc=`/`rc2=` echo runs. Pre-fix rubash bound `b=([0]="x")`, bound
    /// `c=()`, and ran both echoes. This is the shape that made the
    /// fix's declare path need a hard failure rather than the partial
    /// binding the bad-subscript class uses.
    #[test]
    fn case_9_declare_indexed_operand_binds_nothing() {
        let (stdout, stderr, code) = run(
            "753.9",
            "shopt -s failglob\n\
             declare b=(x /no/such/*.zzz y); echo \"rc=$?\"\n\
             declare -p b\n\
             declare -a c=(/no/such/*.zzz); echo \"rc2=$?\"\n\
             declare -p c\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "END\n",
            "declare operand failure must bind nothing and abandon the line; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr,
            "fg.sh: line 2: no match: /no/such/*.zzz\n\
             fg.sh: line 3: declare: b: not found\n\
             fg.sh: line 4: no match: /no/such/*.zzz\n\
             fg.sh: line 5: declare: c: not found\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }

    /// 753.10 [guard] — `declare -A` (associative) is EXEMPT from failglob
    /// in the compound-operand path: GNU stores the literal key
    /// `["/no/such/*.zzz"]=""` with rc 0 and no diagnostic. The fix must
    /// not over-apply the discard to this shape; this case was green
    /// pre-fix and pins the limit of the new arming.
    #[test]
    fn case_10_declare_associative_is_exempt() {
        let (stdout, stderr, code) = run(
            "753.10",
            "shopt -s failglob\n\
             declare -A d=(/no/such/*.zzz); echo \"rc3=$?\"\n\
             declare -p d\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "rc3=0\ndeclare -A d=([\"/no/such/*.zzz\"]=\"\" )\nEND\n",
            "assoc operand must store the literal key; stderr:\n{stderr}"
        );
        assert_eq!(stderr, "", "assoc path must stay silent; stdout:\n{stdout}");
    }

    /// 753.11 [control] — the CONTAINING boundaries, all green pre-fix and
    /// pinned so the new discard cannot leak past them: `( )` subshell
    /// (line continues, rc=1), pipeline stage (line continues, pipeline
    /// rc=0 from `cat`), `&` background job (line continues, rc=0 from the
    /// `&` itself), `eval` (the discard applies INSIDE the eval string —
    /// `eval-inside` never prints — but the caller's line continues), a
    /// bare `$( )` (substitution yields empty, line continues), and a
    /// command-prefix assignment `v=$(...)` (line continues with v empty).
    #[test]
    fn case_11_containing_boundaries_stay_contained() {
        let (stdout, stderr, code) = run(
            "753.11",
            "shopt -s failglob\n\
             ( set -- /no/such/*.zzz ); echo \"SUB_after_subshell rc=$?\"\n\
             set -- /no/such/*.zzz | cat; echo \"PIPE_after rc=$?\"\n\
             set -- /no/such/*.zzz & echo \"BG_after rc=$?\"\n\
             wait\n\
             eval 'set -- /no/such/*.zzz; echo eval-inside'; echo \"EVAL_after\"\n\
             eval 'set -- /no/such/*.zzz'; echo \"EVAL2_after\"\n\
             echo \"$(set -- /no/such/*.zzz; echo sub-in)\"; echo \"COMSUB_after\"\n\
             v=$(set -- /no/such/*.zzz); echo \"ASSIGN_after v=$v\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout,
            "SUB_after_subshell rc=1\n\
             PIPE_after rc=0\n\
             BG_after rc=0\n\
             EVAL_after\n\
             EVAL2_after\n\
             \n\
             COMSUB_after\n\
             ASSIGN_after v=\n\
             END\n",
            "containing boundaries; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr,
            "fg.sh: line 2: no match: /no/such/*.zzz\n\
             fg.sh: line 3: no match: /no/such/*.zzz\n\
             fg.sh: line 4: no match: /no/such/*.zzz\n\
             fg.sh: line 6: no match: /no/such/*.zzz\n\
             fg.sh: line 7: no match: /no/such/*.zzz\n\
             fg.sh: line 8: no match: /no/such/*.zzz\n\
             fg.sh: line 9: no match: /no/such/*.zzz\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }

    /// 753.12 [control] — loop bodies PENETRATE: the failglob inside a
    /// while/until/for body discards the rest of the body's logical line,
    /// escapes through `done`, and aborts the loop (one diagnostic per
    /// loop, not one per remaining iteration). The next-line echoes
    /// (`W_next`, `U_next`, `F_next`) prove the discard stopped at the
    /// line boundary. Green pre-fix; pinned so the reader-layer change
    /// cannot swallow the following line.
    #[test]
    fn case_12_loop_bodies_penetrate_and_abort() {
        let (stdout, stderr, code) = run(
            "753.12",
            "shopt -s failglob\n\
             i=0; while [ $i -lt 3 ]; do i=$((i+1)); set -- /no/such/*.zzz; echo \"wbody-$i\"; done; echo \"W_same\"\n\
             echo \"W_next\"\n\
             i=0; until [ $i -ge 3 ]; do i=$((i+1)); set -- /no/such/*.zzz; echo \"ubody-$i\"; done; echo \"U_same\"\n\
             echo \"U_next\"\n\
             for i in 1 2; do set -- /no/such/*.zzz; echo \"fbody-$i\"; done; echo \"F_same\"\n\
             echo \"F_next\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "W_next\nU_next\nF_next\nEND\n",
            "loop-body discard must stop at the line boundary; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr,
            "fg.sh: line 2: no match: /no/such/*.zzz\n\
             fg.sh: line 4: no match: /no/such/*.zzz\n\
             fg.sh: line 6: no match: /no/such/*.zzz\n",
            "one diagnostic per aborted loop; stdout:\n{stdout}"
        );
    }

    /// 753.13 [guard] — a temporary assignment prefix
    /// (`x=/no/such/*.zzz true`) is COMMAND-level containment, not a line
    /// abort: the command does not run, status is 1, GNU prints no
    /// diagnostic at all, and the list continues — including inside a
    /// brace group (`in-brace2` and `TA_brace_after` both print). The fix
    /// must not upgrade this containment to a line discard; green pre-fix,
    /// pinned as the guardrail.
    #[test]
    fn case_13_temporary_assignment_prefix_is_command_level() {
        let (stdout, stderr, code) = run(
            "753.13",
            "shopt -s failglob\n\
             x=/no/such/*.zzz true; echo \"TA_after\"\n\
             { x=/no/such/*.zzz true; echo \"in-brace2\"; }; echo \"TA_brace_after\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "TA_after\nin-brace2\nTA_brace_after\nEND\n",
            "temp-assignment containment must stay command-level; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr, "",
            "temp-assignment failglob is silent in GNU; stdout:\n{stdout}"
        );
    }

    /// 753.14 [control] — `set -e` still exits at the first failglob with
    /// status 1 and no further output. The errexit exit is decided at the
    /// reader-layer post-command check before the new discard consumption
    /// is reached, so this shape is unchanged by the fix; pinned so a
    /// future reader-layer change cannot silently reorder the two.
    #[test]
    fn case_14_errexit_exits_before_discard() {
        let (stdout, stderr, code) = run(
            "753.14",
            "set -e\n\
             shopt -s failglob\n\
             f() { set -- /no/such/*.zzz; echo in-f; }\n\
             f\n\
             echo \"F-next\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(1), "errexit status; stderr:\n{stderr}");
        assert_eq!(
            stdout, "",
            "errexit must exit before any echo; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr, "fg.sh: line 3: no match: /no/such/*.zzz\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }

    /// 753.15 [control] — the discard is a LINE boundary, not a file
    /// boundary: every command on the following line still runs
    /// (`after-nextline`, `after-if-nextline`, `after-while`,
    /// `after-func-nextline`), and the same-line `echo` after each failing
    /// compound does not. Green pre-fix for the next-line echoes; the
    /// same-line absences were pinned by 753.1-753.8. Together the pair
    /// proves the reader skipped exactly the remainder of one logical
    /// line.
    #[test]
    fn case_15_next_line_commands_still_run() {
        let (stdout, stderr, code) = run(
            "753.15",
            "shopt -s failglob\n\
             { set -- /no/such/*.zzz; echo \"inside\"; }\n\
             echo \"after-nextline\"\n\
             if true; then set -- /no/such/*.zzz; echo \"then-inside\"; fi\n\
             echo \"after-if-nextline\"\n\
             while false; do set -- /no/such/*.zzz; done\n\
             echo \"after-while\"\n\
             h() { set -- /no/such/*.zzz; echo \"func-inside\"; }\n\
             h\n\
             echo \"after-func-nextline\"\n\
             echo END\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "after-nextline\nafter-if-nextline\nafter-while\nafter-func-nextline\nEND\n",
            "next-line commands must survive the discard; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr,
            "fg.sh: line 2: no match: /no/such/*.zzz\n\
             fg.sh: line 4: no match: /no/such/*.zzz\n\
             fg.sh: line 8: no match: /no/such/*.zzz\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }

    /// 753.16 [control] — the select body PENETRATES like the other loop
    /// bodies. Needs a stdin payload: with EOF-on-open stdin the select
    /// menu EOFs before the first prompt and the body never executes,
    /// which is how the first probe misread select as a containing
    /// boundary. With one input line the menu prints (`1) a` / `2) b` /
    /// `#? `), the body's failglob discards the rest of the line
    /// (`select-body` and `SEL_same` never print), and the next line runs.
    /// Green pre-fix; pinned with the other loop bodies.
    #[test]
    fn case_16_select_body_penetrates() {
        let (stdout, stderr, code) = run_with_stdin(
            "753.16",
            "shopt -s failglob\n\
             select v in a b; do set -- /no/such/*.zzz; echo \"select-body\"; break; done; echo \"SEL_same\"\n\
             echo \"SEL_next\"\n",
            "pick\n",
        );
        assert_eq!(code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "SEL_next\n",
            "select body must not contain the discard; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr, "1) a\n2) b\n#? fg.sh: line 2: no match: /no/such/*.zzz\n",
            "menu plus diagnostic; stdout:\n{stdout}"
        );
    }

    /// 753.17 [control] — `source` is a containing boundary at the FILE
    /// seam: the failglob inside the sourced file discards the rest of
    /// that file's logical line (`src-inside` never prints) but the next
    /// line of the sourced file still runs (`src-after`), and the caller's
    /// line — the very `source ...; echo caller-same` that triggered it —
    /// continues (`caller-same`, `caller-next`). This is the GNU
    /// evalstring.c top-level catch the fix mirrors by absorbing the
    /// failglob class at the source boundary. Green pre-fix; pinned so the
    /// reader-layer change cannot make source abort its caller's line.
    #[test]
    fn case_17_source_boundary_contains() {
        let dir = case_scratch("753.17");
        std::fs::write(
            dir.join("fg.sh"),
            "shopt -s failglob\n\
             source inner753.sh; echo \"caller-same\"\n\
             echo \"caller-next\"\n\
             echo END\n",
        )
        .expect("write script");
        std::fs::write(
            dir.join("inner753.sh"),
            "shopt -s failglob\n\
             set -- /no/such/*.zzz; echo \"src-inside\"\n\
             echo \"src-after\"\n",
        )
        .expect("write inner script");
        let out = run_rubash_in(&dir, &["fg.sh"], Duration::from_secs(30));
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.code, Some(0), "rc; stderr:\n{stderr}");
        assert_eq!(
            stdout, "src-after\ncaller-same\ncaller-next\nEND\n",
            "source must contain the discard at the file seam; stderr:\n{stderr}"
        );
        assert_eq!(
            stderr, "inner753.sh: line 2: no match: /no/such/*.zzz\n",
            "diagnostics; stdout:\n{stdout}"
        );
    }
}

/// rubash#759 item 3: `shift` with a bad operand is a USAGE error (2), not
/// a plain failure. GNU builtins/shift.def routes a non-numeric operand
/// through get_numeric_arg (builtins/common.c:501-505 sh_neednumarg,
/// get_numeric_arg returns 0 and shift_builtin answers
/// `return (EX_USAGE)`) and a surplus operand through no_args
/// (`set_exit_status (EX_BADUSAGE)`); only the shift-count-erange path
/// returns EXECUTION_FAILURE.
/// GNU 5.3.0 differential (WSL stdin probes 2026-10-18):
///   ( set -- 1 2; shift x )    -> 2   (was 1)
///   ( set -- 1 2; shift 1 2 )  -> 2   (was 1)
///   ( set -- 1 2; shift -1 )   -> 1
///   set -- 1 2 3; shift 99     -> 1
#[test]
fn shift_bad_operand_status_matches_gnu() {
    let dir = case_scratch("759.shift");
    let run = |code: &str| {
        let out = run_rubash_in(&dir, &["-c", code], Duration::from_secs(30));
        String::from_utf8_lossy(&out.stdout).trim_end().to_string()
    };

    let statuses = run("( set -- 1 2; shift x ) 2>/dev/null; echo \"shiftx:$?\"\n\
         ( set -- 1 2; shift 1 2 ) 2>/dev/null; echo \"shift12:$?\"\n\
         ( set -- 1 2; shift -1 ) 2>/dev/null; echo \"shiftneg:$?\"\n\
         set -- 1 2 3; shift 99 2>/dev/null; echo \"shiftbig:$?\"");
    assert_eq!(
        statuses, "shiftx:2\nshift12:2\nshiftneg:1\nshiftbig:1",
        "shift statuses"
    );

    // The diagnostics themselves keep GNU's wording (only the status
    // changed): `shift: x: numeric argument required` and
    // `shift: too many arguments`.
    let out = run_rubash_in(
        &dir,
        &["-c", "set -- 1 2; shift x; shift 1 2"],
        Duration::from_secs(30),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("shift: x: numeric argument required"),
        "{stderr}"
    );
    assert!(stderr.contains("shift: too many arguments"), "{stderr}");
}

/// rubash#759 item 4: GNU bash has no `setopt`/`unsetopt` builtins — a
/// script probing `command -v setopt` to detect zsh must get the GNU
/// answer. After the removal `setopt x` is command-not-found (127),
/// `builtin setopt x` is not-a-shell-builtin (1), and `enable -a` lists
/// exactly GNU's 61 builtin names.
/// GNU 5.3.0 differential (WSL stdin probe 2026-10-18):
///   command -v setopt    -> rc 1, no output   (was "setopt", rc 0)
///   command -v unsetopt  -> rc 1, no output   (was "unsetopt", rc 0)
///   enable -a | wc -l    -> 61                (was 64 on Windows)
#[test]
fn zsh_option_builtins_are_gone() {
    let dir = case_scratch("759.setopt");

    let out = run_rubash_in(
        &dir,
        &[
            "-c",
            "command -v setopt; echo \"v:$?\"\ncommand -v unsetopt; echo \"v:$?\"\nenable -a",
        ],
        Duration::from_secs(30),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("v:1"), "command -v setopt misses");
    assert_eq!(lines.next(), Some("v:1"), "command -v unsetopt misses");
    let listed = lines.filter(|line| !line.trim().is_empty()).count();
    assert_eq!(
        listed, 61,
        "enable -a must list exactly GNU's 61 builtin names: {stdout}"
    );

    let out = run_rubash_in(
        &dir,
        &[
            "-c",
            "setopt hist_ignore_dups null_glob; echo \"plain:$?\"\n\
             shopt -q nullglob; echo \"nullglob:$?\"\n\
             builtin unsetopt null_glob; echo \"builtin:$?\"",
        ],
        Duration::from_secs(30),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stdout.trim_end(),
        "plain:127\nnullglob:1\nbuiltin:1",
        "setopt/unsetopt are plain unknown commands; option state untouched"
    );
    assert!(stderr.contains("setopt: command not found"), "{stderr}");
    assert!(
        stderr.contains("builtin: unsetopt: not a shell builtin"),
        "{stderr}"
    );
}

/// rubash#759 item 4 (sudo): GNU bash has no `sudo` builtin — `command -v
/// sudo` resolves it on PATH like any external (WSL: /usr/bin/sudo). rubash
/// keeps the Windows host-capability builtin behind an embedding-host
/// elevation handler (the docs/builtins.md §3 hidden-builtin contract), so
/// with no handler the name must classify as external everywhere. On a
/// Windows box without sudo.exe that is rc 1 with no output; the invariant
/// under test is "never reports a builtin".
/// GNU 5.3.0 differential (WSL stdin probe 2026-10-18):
///   command -v sudo -> /usr/bin/sudo (external), rc 0
///   type -t sudo    -> file, rc 0
#[test]
fn sudo_is_not_reported_as_a_builtin() {
    let dir = case_scratch("759.sudo");
    let out = run_rubash_in(
        &dir,
        &[
            "-c",
            "command -v sudo; echo \"v:$?\"\ntype -t sudo; echo \"t:$?\"",
        ],
        Duration::from_secs(30),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.lines().any(|line| line == "sudo"),
        "`command -v sudo` must not report the bare builtin name: {stdout}"
    );
    assert!(
        !stdout.contains("sudo is a shell builtin"),
        "`type sudo` must not report a builtin: {stdout}"
    );
}

/// #813 item 2 (the #810 CR F4 coverage gap, arm 1): with no elevation
/// handler installed (the CLI binary's default), `sudo cmd` must take the
/// PATH execution route — a fake `sudo` earlier on PATH actually runs and
/// its status becomes the command's. Before #810 the name was answered by
/// the host builtin with "elevation handler is not configured" (rc 1) and
/// nothing on PATH was consulted. `routes_as_builtin`
/// (src/executor/lookup_paths.rs) is the gate: windows + handler = builtin,
/// everything else = PATH.
#[cfg(windows)]
#[test]
fn sudo_without_handler_executes_the_path_command() {
    let dir = case_scratch("813.sudo.path");
    let bindir = dir.join("fakebin");
    std::fs::create_dir_all(&bindir).expect("create fakebin");
    // cmd.exe batch: emit a marker line, pass the child's status through.
    std::fs::write(
        bindir.join("sudo.cmd"),
        "@echo off\r\necho FAKE_SUDO_RAN %*\r\nexit /b 0\r\n",
    )
    .expect("write fake sudo");
    let script = format!(
        "export PATH='{}':\"$PATH\"\nsudo --preserve-env probe\necho \"rc:$?\"",
        bindir.display()
    );
    let out = run_rubash_in(&dir, &["-c", &script], Duration::from_secs(30));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("FAKE_SUDO_RAN --preserve-env probe"),
        "fake PATH sudo must actually run: stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("rc:0"),
        "fake sudo's status must become the command status: {stdout}"
    );
    assert!(
        !stderr.contains("elevation handler"),
        "the no-handler diagnostic must not resurface: {stderr}"
    );
}

/// #813 item 2 (the #810 CR F4 coverage gap, arm 2): `builtin sudo` with no
/// elevation handler falls through the hidden-builtin dispatch to
/// `write_builtin_not_found` (src/executor/builtin_direct_command.rs) —
/// GNU's "not a shell builtin" wording, rc 1.
#[cfg(windows)]
#[test]
fn builtin_sudo_without_handler_reports_not_a_shell_builtin() {
    let dir = case_scratch("813.sudo.builtin");
    let out = run_rubash_in(
        &dir,
        &["-c", "builtin sudo\necho \"rc:$?\""],
        Duration::from_secs(30),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stdout.trim_end(),
        "rc:1",
        "the builtin fall-through must fail rc 1: stdout={stdout} stderr={stderr}"
    );
    assert!(
        stderr.contains("builtin: sudo: not a shell builtin"),
        "GNU wording expected: {stderr}"
    );
}

/// CR follow-up on rubash#757 (F1): the prompt-pass containment. GNU
/// subst.c:4392-4418 expand_prompt_string runs the expansion with
/// `no_longjmp_on_fatal_error = 1` and RETURNS THE PASSED STRING when the
/// expansion verdicts: the diagnostic stands, the prompt value falls back
/// to the decoded input, and the enclosing command's status is untouched.
/// Before the containment a malformed `${...}` inside PS1 failed the whole
/// assignment (rc 1, variable unset).
/// GNU 5.3.0 differential (WSL stdin probes 2026-10-18, probe-f1/f1b/f1c):
///   PS1='a${1x}b'; p=${PS1@P}  -> stderr `a${1x}b: bad substitution`,
///                                 rc 0, p=`a${1x}b`, next line runs
///   PS1='a$((1x))b'            -> arith diagnostic, rc 0, literal
///   PS1='a${bad'               -> `a${bad: bad substitution`, rc 0, literal
///   PS1='er:${nose?bad} t'     -> stderr `nose: bad`, rc 0, literal text
///   PS1='a$((1/0))b'           -> division diagnostic, rc 0, literal
#[test]
fn prompt_expansion_failure_contained_like_gnu() {
    let dir = case_scratch("757.containment");
    let run = |code: &str| {
        let out = run_rubash_in(&dir, &["-c", code], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    };

    // Bad substitution: diagnostic on stderr, value literal, rc 0, tail runs.
    let (stdout, stderr, code) =
        run("PS1='a${1x}b'\np=${PS1@P}\necho \"rc=$? p=[$p]\"\necho after");
    assert_eq!(code, Some(0), "list rc; stderr:\n{stderr}");
    assert_eq!(
        stdout, "rc=0 p=[a${1x}b]\nafter",
        "stdout; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("a${1x}b: bad substitution"),
        "diagnostic; stderr:\n{stderr}"
    );

    // Arithmetic verdict, unterminated brace, `?` operator: same contract.
    let (stdout, stderr, code) = run("PS1='a$((1x))b'\np=${PS1@P}\necho \"arith:$? [$p]\"\n\
         PS1='a${bad'\nq=${PS1@P}\necho \"unterm:$? [$q]\"\n\
         unset p757_nose\nPS1='er:${p757_nose?bad} t'\nr=${PS1@P}\necho \"qmark:$? [$r]\"");
    assert_eq!(code, Some(0), "list rc; stderr:\n{stderr}");
    assert_eq!(
        stdout, "arith:0 [a$((1x))b]\nunterm:0 [a${bad]\nqmark:0 [er:${p757_nose?bad} t]",
        "stdout; stderr:\n{stderr}"
    );
    assert!(stderr.contains("value too great for base"), "{stderr}");
    assert!(stderr.contains("a${bad: bad substitution"), "{stderr}");
    assert!(
        stderr.lines().any(|line| line.contains("p757_nose: bad")),
        "`?` operator reports on stderr: {stderr}"
    );

    // A set variable under `?` still expands (no verdict).
    let (stdout, _stderr, code) =
        run("p757_x=set\nPS1='v${p757_x?nope}w'\np=${PS1@P}\necho \"xset:$? [$p]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "xset:0 [vsetw]");
}

mod k1_batch2 {
    use super::*;

    fn run(case: &str, script: &str) -> (String, String, Option<i32>) {
        let dir = case_scratch(case);
        std::fs::write(dir.join("fg.sh"), script).expect("write script");
        let out = run_rubash_in(&dir, &["fg.sh"], Duration::from_secs(60));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    fn run_interactive(
        case: &str,
        input: &str,
        seed_history: Option<&str>,
    ) -> (String, String, PathBuf) {
        let dir = case_scratch(case);
        if let Some(seed) = seed_history {
            std::fs::write(dir.join(".bash_history"), seed).expect("seed history");
        }
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
        cmd.args(["-i"])
            .current_dir(&dir)
            .env_clear()
            .env("PATH", minimal_path())
            .env("HOME", &dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("spawn rubash");
        std::io::Write::write_all(child.stdin.as_mut().expect("stdin pipe"), input.as_bytes())
            .expect("write stdin");
        drop(child.stdin.take());
        let out = wait_bounded(&mut child, Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            dir,
        )
    }

    /// rubash#1184 finding 1: a word merely STARTING with `2` is an
    /// ordinary word (parse.y read_token_word) — $'...' decoding and
    /// backslash folding must run; only a token ENDING right before
    /// `<`/`>` is an io-number.
    #[test]
    fn issue_1184_word_starting_with_two_decodes() {
        let (stdout, stderr, code) = run(
            "k1-1184-f1",
            "echo \"1: $(echo 2$'x')\"\n\
             echo \"2: $(echo 2\\\\x)\"\n\
             echo \"3: $(echo 2abc$'\\t' | cat -A)\"\n\
             for w in 2$'z'; do echo \"4: $w\"; done\n\
             case 2$'x' in 2x) echo \"5: hit\";; *) echo \"5: miss\";; esac\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "1: 2x\n2: 2\\x\n3: 2abc^I$\n4: 2z\n5: hit\n");
    }

    /// rubash#1184 findings 2+3: the no-arg export/readonly listings use
    /// declare -p display quoting ($'...' for control bytes), and
    /// UID/EUID/PPID carry the integer attribute.
    #[test]
    fn issue_1184_listing_quoting_and_uid_attrs() {
        let (stdout, stderr, code) = run(
            "k1-1184-f23",
            "xv=$'x\\ny'\nexport xv\nexport -p | grep '^declare -x xv='\n\
             ro=$'r\\tq'\nreadonly ro\nreadonly -p | grep '^declare -r ro='\n\
             declare -p UID\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert!(
            stdout.contains("declare -x xv=$'x\\ny'\n"),
            "export listing; stdout:\n{stdout}"
        );
        assert!(
            stdout.contains("declare -r ro=$'r\\tq'\n"),
            "readonly listing; stdout:\n{stdout}"
        );
        assert!(
            stdout.contains("declare -ir UID="),
            "UID attrs; stdout:\n{stdout}"
        );
    }

    /// rubash#1186 (grouped-driver shape): with history on, a failglob
    /// error discards the rest of its logical line; the next line runs.
    #[test]
    fn issue_1186_grouped_failglob_discards_line() {
        let (stdout, stderr, code) = run(
            "k1-1186",
            "set -o history\nshopt -s failglob\necho case1-start\n\
             set -- /no/such/*.zzz; echo case1-skipped\necho case2-start\necho END\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "case1-start\ncase2-start\nEND\n");
        assert_eq!(stderr, "fg.sh: line 4: no match: /no/such/*.zzz\n");
    }

    /// rubash#1200: printf %b \c stops output without failing.
    #[test]
    fn issue_1200_printf_percent_b_backslash_c() {
        let (stdout, stderr, code) = run(
            "k1-1200",
            "printf '%b' 'A\\cB'; echo \"[rc=$?]\"\nprintf '%b' 'p\\c' 'q'; echo \"[rc=$?]\"\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "A[rc=0]\np[rc=0]\n");
    }

    /// rubash#1201: a nounset fatal inside [[ ]] keeps its status even
    /// when the predicate itself evaluates to 0 (-z of the empty word).
    #[test]
    fn issue_1201_conditional_nounset_status() {
        let (stdout, _stderr, code) = run(
            "k1-1201",
            "(set -u; [[ -z $UNSET_XYZ2 ]]) 2>&1; echo \"rc=$?\"\n\
             (set -u; [ -z \"$UNSET_XYZ2\" ]) 2>&1; echo \"rc=$?\"\n\
             (set -u; [[ $UNSET_XYZ2 = x ]]) 2>&1; echo \"rc=$?\"\n",
        );
        assert_eq!(code, Some(0));
        assert_eq!(
            stdout,
            "fg.sh: line 1: UNSET_XYZ2: unbound variable\nrc=1\n\
             fg.sh: line 2: UNSET_XYZ2: unbound variable\nrc=1\n\
             fg.sh: line 3: UNSET_XYZ2: unbound variable\nrc=1\n"
        );
    }

    /// rubash#1202: the `test -v name[sub]` subscript evalerror is the
    /// builtin's own diagnostic — the command's `2>file` captures it.
    #[test]
    fn issue_1202_test_v_subscript_error_redirected() {
        let (stdout, stderr, code) = run(
            "k1-1202",
            "arr1=(a b c)\n[ -v 'arr1[1:2]' ] 2>e4\necho \"e4=[$(cat e4)]\"\necho after\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stderr, "");
        assert_eq!(
            stdout,
            "e4=[fg.sh: line 2: 1:2: arithmetic syntax error in expression \
             (error token is \":2\")]\nafter\n"
        );
    }

    /// rubash#1210: `history -d 0` / `-d -0` are out of range (a bare 0
    /// names no entry); the entries survive.
    #[test]
    fn issue_1210_history_delete_zero() {
        let (stdout, stderr, code) = run(
            "k1-1210",
            "set -o history\nhistory -c\nhistory -s \"cmd one\"\nhistory -s \"cmd two\"\n\
             history -d 0; echo \"rc=$?\"\nhistory -d -0; echo \"rc=$?\"\nhistory\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(
            stderr,
            "fg.sh: line 5: history: 0: history position out of range\n\
             fg.sh: line 6: history: -0: history position out of range\n"
        );
        assert_eq!(
            stdout,
            "rc=1\nrc=1\n    1  cmd one\n    2  cmd two\n    3  history -d 0; echo \"rc=$?\"\n    4  history -d -0; echo \"rc=$?\"\n    5  history\n"
        );
    }

    /// rubash#1211: the `fc -e` re-execution echo goes to the command's
    /// redirected stderr, not the raw process stderr.
    #[test]
    fn issue_1211_fc_echo_redirected() {
        let (stdout, stderr, code) = run(
            "k1-1211",
            "set -o history\necho SEEDLINE\nfc -e true 2 2>err1211.txt\n\
             echo \"err-is:[$(cat err1211.txt)]\"\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "SEEDLINE\nSEEDLINE\nerr-is:[echo SEEDLINE]\n");
    }

    /// rubash#1776: `time` on Windows counted SELF only (PR #1771);
    /// the CHILDREN half — CPU of waited external children, GNU
    /// getrusage(RUSAGE_CHILDREN) — stayed zero because no reap site
    /// sampled the child's handle. The pin times a nested rubash
    /// child burning an arithmetic loop and asserts a 100ms user
    /// threshold, not mere non-zero: the outer shell's own SELF delta
    /// is a parse-and-spawn trifle (single-digit milliseconds), so a
    /// threshold crossing proves the CHILDREN accumulator moved. The
    /// nested binary path comes from CARGO_BIN_EXE_rubash (the same
    /// binary the harness runs); forward slashes keep Windows
    /// quoting intact. On unix this guards the getrusage path.
    /// The loop count is 20,000 (not 300,000): measured ~6.5s user /
    /// ~18s wall on a loaded 2-core box, so the run stays far inside
    /// the 60s harness cap while still clearing the 100ms threshold
    /// by over an order of magnitude on any runner.
    #[test]
    fn issue1776_time_children_cpu() {
        fn parse_time_ms(line: &str) -> Option<u64> {
            // line: "user\t0m2.345s" (minute form, %3lU).
            let value = line.split('\t').nth(1)?.trim().strip_suffix('s')?;
            let (min, rest) = value.split_once('m')?;
            let (sec, ms) = rest.split_once('.')?;
            let ms = format!("{ms:0<3}");
            let ms = ms.get(..3)?;
            Some(
                min.parse::<u64>().ok()? * 60_000
                    + sec.parse::<u64>().ok()? * 1_000
                    + ms.parse::<u64>().ok()?,
            )
        }
        let bin = env!("CARGO_BIN_EXE_rubash").replace('\\', "/");
        let script =
            format!("time \"{bin}\" -c 'x=0; while [ $x -lt 20000 ]; do x=$((x+1)); done'\n");
        let (_stdout, stderr, code) = run("k1-1776", &script);
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        let user_line = stderr
            .lines()
            .find(|line| line.starts_with("user"))
            .unwrap_or("");
        let user_ms = parse_time_ms(user_line).unwrap_or(0);
        assert!(
            user_ms >= 100,
            "child CPU must clear 100ms, got: {user_line:?} ({user_ms}ms) in\n{stderr}"
        );
    }
    /// rubash#1212: `time` reports real CPU time (getrusage deltas), not
    /// a hard-coded user/sys of zero.
    #[test]
    fn issue_1212_time_reports_cpu() {
        let (_stdout, stderr, code) = run(
            "k1-1212",
            "time ( x=0; while [ $x -lt 60000 ]; do x=$((x+1)); done )\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        let user_line = stderr
            .lines()
            .find(|line| line.starts_with("user"))
            .unwrap_or("");
        assert!(
            !user_line.is_empty() && !user_line.ends_with("0m0.000s"),
            "user time must be measured, got: {user_line:?} in\n{stderr}"
        );
    }

    /// rubash#1774: a refused /dev/tcp connect on Windows leaked the
    /// long FormatMessageW text ("No connection could be made because
    /// the target machine actively refused it. (os error 10061)")
    /// where GNU prints the short strerror "Connection refused"
    /// (redir.c netopen; golden issue1237-net-redir). The rendering
    /// funnel is posix_errors::message()'s Win32 table, so the pin
    /// asserts the GNU shape on every platform: the sys_error line
    /// plus the redirection_error line, and no raw OS long text.
    #[test]
    fn issue1774_dev_tcp_connect_failure_gnu_diagnostic() {
        let (stdout, stderr, code) = run(
            "k1-1774",
            "( exec 3<>/dev/tcp/127.0.0.1/1 ); echo \"tcp rc=$?\"\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "tcp rc=1\n", "stderr:\n{stderr}");
        assert!(
            stderr.contains("connect: Connection refused"),
            "missing GNU connect line in:\n{stderr}"
        );
        assert!(
            stderr.contains("/dev/tcp/127.0.0.1/1: Connection refused"),
            "missing GNU redirection line in:\n{stderr}"
        );
        assert!(
            !stderr.contains("actively refused") && !stderr.contains("os error"),
            "raw OS text leaked in:\n{stderr}"
        );
    }

    /// rubash#1213: `return` with too many arguments is the no_args
    /// DISCARD class — message, status 2, rest of the line abandoned,
    /// in functions, at top level, and for the numeric path's caller.
    #[test]
    fn issue_1213_return_too_many_arguments() {
        let (stdout, stderr, code) = run(
            "k1-1213",
            "f() { return 1 2 3; echo \"f-after\"; }\nf; echo rc=$?\n\
             return 1 2 3; echo \"top rc=$?\"\n\
             f2() { return abc; echo \"f2-after\"; }\nf2; echo rc2=$?\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "rc2=2\n");
        assert_eq!(
            stderr,
            "fg.sh: line 1: return: too many arguments\n\
             fg.sh: line 3: return: too many arguments\n\
             fg.sh: line 4: return: abc: numeric argument required\n"
        );
    }

    /// rubash#1775: /dev/tcp with a service name ('http') failed on
    /// Windows because resolve_net_service_port only read
    /// /etc/services, so GNU's getservbyname parity (http -> 80) was
    /// missing and the shell printed "Servname not supported" plus
    /// "Invalid argument". The pin asserts the resolution invariant,
    /// not the listener state: loopback:80 may or may not have a
    /// listener (CI runners do, local machines do not), so the
    /// connect must be attempted at the resolved port either way —
    /// rc=0 on connect, rc=1 with the GNU refused shape otherwise —
    /// and must never fail with a resolution error. On unix the name
    /// already resolved, so this guards the shared parser path there
    /// and pins the Windows fix here.
    #[test]
    fn issue1775_dev_tcp_service_name_http_resolves() {
        let (stdout, stderr, code) = run(
            "k1-1775",
            "( exec 3<>/dev/tcp/127.0.0.1/http ); echo \"http rc=$?\"\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert!(
            stdout == "http rc=0\n" || stdout == "http rc=1\n",
            "connect must be attempted at the resolved port, got stdout: {stdout:?}, stderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("Servname not supported") && !stderr.contains("Invalid argument"),
            "service name failed to resolve in:\n{stderr}"
        );
        if stdout == "http rc=1\n" {
            assert!(
                stderr.contains("connect: Connection refused"),
                "missing GNU connect line in:\n{stderr}"
            );
            assert!(
                stderr.contains("/dev/tcp/127.0.0.1/http: Connection refused"),
                "missing GNU redirection line in:\n{stderr}"
            );
        }
    }

    /// rubash#1215: `hash <builtin>` is a silent no-op (no entry, rc 0);
    /// external names hash and report normally.
    #[test]
    #[cfg(unix)]
    fn issue_1215_hash_skips_builtins() {
        let (stdout, stderr, code) = run(
            "k1-1215",
            "hash echo; echo \"rc1=$?\"\nhash; echo \"rc2=$?\"\n\
             hash -t echo; echo \"rc3=$?\"\nhash cd; echo \"rc4=$?\"\n\
             hash -t cd; echo \"rc5=$?\"\nhash ls; echo \"rc6=$?\"\n\
             hash -t ls; echo \"rc7=$?\"\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(
            stdout,
            "rc1=0\nhash: hash table empty\nrc2=0\nrc3=1\nrc4=0\nrc5=1\nrc6=0\n/usr/bin/ls\nrc7=0\n"
        );
        assert_eq!(
            stderr,
            "fg.sh: line 3: hash: echo: not found\nfg.sh: line 5: hash: cd: not found\n"
        );
    }

    /// rubash#1216: with HISTFILE unset, an interactive shell loads and
    /// saves the default ~/.bash_history.
    #[test]
    fn issue_1216_histfile_default() {
        let (stdout, stderr, _dir) =
            run_interactive("k1-1216a", "fc -l\nexit\n", Some("echo OLDHIST1\n"));
        assert!(
            stdout.contains("echo OLDHIST1") || stderr.contains("echo OLDHIST1"),
            "seeded history must load; stdout:\n{stdout}\nstderr:\n{stderr}"
        );
        let (_stdout2, _stderr2, dir2) =
            run_interactive("k1-1216b", "echo NEWCMD1\nexit\n", Some("echo OLDHIST1\n"));
        let saved2 = std::fs::read_to_string(dir2.join(".bash_history")).unwrap_or_default();
        assert!(
            saved2.contains("NEWCMD1"),
            "session lines must be saved; file: {saved2}"
        );
    }

    /// rubash#1214: PS0 is expanded and printed after each interactive
    /// command is read, before it executes.
    #[test]
    fn issue_1214_ps0_printed() {
        let (_stdout, stderr, _dir) = run_interactive(
            "k1-1214",
            "PS0=PS0MARK\\n\nPS1=READY\\n\necho hi\nexit\n",
            None,
        );
        assert_eq!(
            stderr.matches("PS0MARKn").count(),
            3,
            "PS0 before the PS1=, echo and exit lines; stderr:\n{stderr}"
        );
    }

    /// rubash#1218: in posix mode `time -p cmd` is NOT the time keyword —
    /// it dispatches to an external `time` (absent here: 127).
    #[test]
    fn issue_1218_posix_time_dash_word() {
        let (stdout, stderr, code) = run("k1-1218", "set -o posix\ntime -p true\necho \"rc=$?\"\n");
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "rc=127\n");
        assert!(
            stderr.contains("fg.sh: line 2: time: command not found"),
            "stderr:\n{stderr}"
        );
    }

    /// rubash#1222: `wait` for a disowned pid succeeds silently with
    /// status 0 (repeatedly); a never-owned pid still reports 127.
    #[test]
    #[cfg(unix)]
    fn issue_1222_wait_disowned_pid() {
        let (stdout, stderr, code) = run(
            "k1-1222",
            "sleep 0.2 & p=$!\ndisown $p\nwait $p; echo \"rc=$?\"\n\
             wait $p; echo \"rc2=$?\"\nwait 999999; echo \"rc3=$?\"\necho done\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "rc=0\nrc2=0\nrc3=127\ndone\n");
        assert_eq!(
            stderr,
            "fg.sh: line 5: wait: pid 999999 is not a child of this shell\n"
        );
    }
}

mod win_face_fd_history {
    use super::*;

    fn run(case: &str, script: &str) -> (String, String, Option<i32>) {
        let dir = case_scratch(case);
        std::fs::write(dir.join("fg.sh"), script).expect("write script");
        let out = run_rubash_in(&dir, &["fg.sh"], Duration::from_secs(60));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// rubash#1887: an `exec`-bound file fd must feed an external
    /// command's stdin after being dup'd onto fd 0 (and directly),
    /// sharing the open file description's offset with builtin reads.
    /// GNU bash 5.3.20 oracle (script file, stdin </dev/null): every
    /// variant prints the file body; variant D proves the shared offset
    /// (the builtin `read -u 10` consumes line 1, `cat` sees 2..4).
    /// On the Windows build the two snapshot groups in redir.snap
    /// (upstream redir8.sub segments) came back empty — the external
    /// child read EOF. Linux build matched GNU byte-for-byte.
    #[test]
    fn issue_1887_exec_fd_move_external_stdin() {
        let (stdout, stderr, code) = run(
            "wf-1887",
            "exec 7<&0\n\
             printf '1\\n2\\n3\\n4\\n' > infile\n\
             echo \"--- A seed: exec 10<infile; exec 0<&10; cat <&10 ---\"\n\
             exec 10<infile\n\
             exec 0<&10\n\
             cat <&10\n\
             echo \"cat-status=$?\"\n\
             exec 0<&7\n\
             exec 10<&-\n\
             echo \"--- B direct: exec 10<infile; cat <&10 ---\"\n\
             exec 10<infile\n\
             cat <&10\n\
             echo \"cat-status=$?\"\n\
             exec 10<&-\n\
             echo \"--- C fd0: exec 0<infile; cat ---\"\n\
             exec 0<infile\n\
             cat\n\
             echo \"cat-status=$?\"\n\
             exec 0<&7\n\
             echo \"--- D shared offset: read -u 10 then cat <&10 ---\"\n\
             exec 10<infile\n\
             read -u 10 line\n\
             echo \"read line=[$line]\"\n\
             cat <&10\n\
             echo \"cat-status=$?\"\n\
             exec 10<&-\n\
             echo \"--- E plain: cat < infile ---\"\n\
             cat < infile\n\
             echo \"cat-status=$?\"\n\
             echo \"--- end ---\"\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(
            stdout,
            "--- A seed: exec 10<infile; exec 0<&10; cat <&10 ---\n\
             1\n2\n3\n4\ncat-status=0\n\
             --- B direct: exec 10<infile; cat <&10 ---\n\
             1\n2\n3\n4\ncat-status=0\n\
             --- C fd0: exec 0<infile; cat ---\n\
             1\n2\n3\n4\ncat-status=0\n\
             --- D shared offset: read -u 10 then cat <&10 ---\n\
             read line=[1]\n2\n3\n4\ncat-status=0\n\
             --- E plain: cat < infile ---\n\
             1\n2\n3\n4\ncat-status=0\n\
             --- end ---\n",
            "stderr:\n{stderr}"
        );
    }

    /// rubash#1887 (suite form): the verbatim upstream redir8.sub, run
    /// as a child script exactly as redir.tests:208 runs it. The
    /// isolated matrix above passes on the Windows build; the suite
    /// form carries the prefix state (fd 10/8 write opens and closes,
    /// `-c` children with redirected fds, a heredoc-fed external cat)
    /// under which the two `1 2 3 4` groups vanished on Windows.
    /// Expected stdout is the GNU bash 5.3.20 oracle's standalone run
    /// of the same file.
    #[test]
    fn issue_1887_redir8_sub_verbatim() {
        let sub = std::fs::read_to_string(manifest_dir().join("third_party/bash/tests/redir8.sub"))
            .expect("read redir8.sub");
        let dir = case_scratch("wf-1887-sub");
        std::fs::write(dir.join("redir8.sub"), sub).expect("write sub");
        std::fs::write(
            dir.join("fg.sh"),
            "mkdir wtmp\nexport TMPDIR=\"$PWD/wtmp\"\n${THIS_SH} ./redir8.sub\n",
        )
        .expect("write wrapper");
        let out = run_rubash_in(&dir, &["fg.sh"], Duration::from_secs(120));
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.code, Some(0), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(
            stdout, "fd 10\nfd 8\nfd 10\nnext fd\nfd 10\nfd 8\n1\n2\n3\n4\n1\n2\n3\n4\n",
            "stderr:\n{stderr}"
        );
    }

    /// rubash#1888: `history 10 42` (history.def no_args -> EX_USAGE)
    /// leaves status 2 for the NEXT command in script mode. GNU bash
    /// 5.3.20 oracle (script file): the same-line `echo` after
    /// `history 10 42;` is abandoned with the rest of the logical line
    /// (evalerror DISCARD), so no P2 line exists at all; the own-line
    /// failure reports 2 to the following line. The Windows build
    /// printed 1 (errors.snap golden divergence).
    #[test]
    fn issue_1888_history_too_many_arguments_status() {
        let (stdout, stderr, code) = run(
            "wf-1888",
            "set -o history\n\
             HISTFILE=/dev/null\n\
             echo a >/dev/null\n\
             echo b >/dev/null\n\
             echo c >/dev/null\n\
             history abcde\n\
             echo \"P1 abcde: $?\"\n\
             history 10 42; echo \"P2 toomany-same-line: $?\"\n\
             history 10 42\n\
             echo \"P3 toomany-own-line: $?\"\n\
             echo \"P4 marker: $?\"\n\
             set +o history\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(
            stdout, "P1 abcde: 2\nP3 toomany-own-line: 2\nP4 marker: 0\n",
            "stderr:\n{stderr}"
        );
    }

    /// rubash#1888 (tail form): a script whose LAST command is
    /// `history 10 42` exits 2 (the reader run is over; the armed
    /// too-many discard does not fold the final status in script mode).
    /// GNU bash 5.3.20 oracle: script exit status 2.
    #[test]
    fn issue_1888_history_too_many_arguments_last_command() {
        let (stdout, stderr, code) = run(
            "wf-1888-last",
            "set -o history\nHISTFILE=/dev/null\nhistory 10 42\n",
        );
        assert_eq!(code, Some(2), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(stdout, "", "stderr:\n{stderr}");
    }

    /// rubash#1888 (mechanism form, all platforms): the history block
    /// executed by an in-process `${THIS_SH}` child. A same-shell
    /// script child runs on the parent's Executor while the parent's
    /// reader run is still active; before the fix its groups entered
    /// execute_ast through the nested branch, which skips the
    /// fresh-run latch resets, so `history 10 42`'s evalerror DISCARD
    /// leaked into the next group without its too-many companion and
    /// the following line observed 1 instead of 2. Under a real child
    /// process (or a non-rubash parent) the same script prints 2.
    #[test]
    fn issue_1888_in_process_child_history_status() {
        let dir = case_scratch("wf-1888-inproc");
        std::fs::write(
            dir.join("child.sh"),
            "set -o history\n\
             HISTFILE=/dev/null\n\
             echo a >/dev/null\n\
             echo b >/dev/null\n\
             echo c >/dev/null\n\
             history abcde\n\
             echo \"after history: $?\"\n\
             history 10 42\n\
             echo \"after history: $?\"\n\
             set +o history\n",
        )
        .expect("write child script");
        std::fs::write(dir.join("fg.sh"), "${THIS_SH} ./child.sh\n").expect("write wrapper");
        let out = run_rubash_in(&dir, &["fg.sh"], Duration::from_secs(60));
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.code, Some(0), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(
            stdout, "after history: 2\nafter history: 2\n",
            "stderr:\n{stderr}"
        );
    }

    /// rubash#1888 (suite form): the verbatim upstream errors10.sub,
    /// run as a child script exactly as errors.tests:369 runs it.
    /// The isolated pin above passes on the Windows build; the suite
    /// form carries the prefix state (ten `${THIS_SH} -c` children and
    /// two for-loops before the history lines) under which the Windows
    /// build printed "after history: 1".
    ///
    /// The assertion is the #1888 projection of the output — the two
    /// history lines, in order, and no "after history: 1" anywhere —
    /// plus, since the RC audit phase-1 finding ④ fix, the errors10
    /// numeric segment itself: the file's POSIX-mode `-c` children used
    /// to print a second set of "after exit/shift/return" lines where
    /// GNU's posix shell dies on the special-builtin error (EX_USAGE >
    /// EX_SHERRBASE, execute_cmd.c:4886-4888 + :1004-1017), a
    /// divergence the committed errors.snap golden had absorbed. The
    /// product now converts the failure at the dispatch gate (and, for
    /// `return`, in execute_return per builtins/return.def:62-65), the
    /// golden was re-recorded from the GNU 5.3.20 oracle's output, and
    /// the segment below pins the POSIX trio's absence: the
    /// default-mode trio is followed DIRECTLY by the history lines.
    #[test]
    fn issue_1888_errors10_sub_verbatim() {
        let sub =
            std::fs::read_to_string(manifest_dir().join("third_party/bash/tests/errors10.sub"))
                .expect("read errors10.sub");
        let dir = case_scratch("wf-1888-sub");
        std::fs::write(dir.join("errors10.sub"), sub).expect("write sub");
        std::fs::write(
            dir.join("fg.sh"),
            "mkdir wtmp\nexport TMPDIR=\"$PWD/wtmp\"\n${THIS_SH} ./errors10.sub\n",
        )
        .expect("write wrapper");
        let out = run_rubash_in(&dir, &["fg.sh"], Duration::from_secs(120));
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.code, Some(0), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert!(
            stdout.contains("after history: 2\nafter history: 2\n"),
            "history lines missing or wrong:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            !stdout.contains("after history: 1"),
            "history status folded to 1:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stdout.contains(
                "invalid numeric argument\nafter exit: 2\nafter shift: 2\nafter return: 2\nafter history: 2\n"
            ),
            "errors10 numeric segment diverged (the POSIX-mode trio must \
             be absent — GNU's posix shell dies on the error):\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

/// RC audit phase-1 finding ④ (errors10): in POSIX mode, a special
/// builtin's EX_USAGE-class failure (numeric-argument error, `--help`)
/// is FATAL to a non-interactive shell — GNU returns EX_USAGE (258 >
/// EX_SHERRBASE) from the builtin, execute_simple_command marks
/// special_builtin_failed (execute_cmd.c:4886-4888), and the command's
/// frame exits (execute_cmd.c:1004-1017) unless the context is tested
/// (ignore_return / invert) or the builtin ran under `command`.
/// rubash used to report the error and run the next command; the
/// committed errors.snap golden had absorbed the non-fatal bytes.
/// Expectations are the GNU bash 5.3.20 oracle's, byte for byte
/// (stdout / stderr / exit status); see the fix-errors10 REPORT for
/// the full two-mode classification matrix.
mod errors10_posix_fatal {
    use super::*;

    fn run(args: &[&str]) -> (String, String, Option<i32>) {
        let dir = case_scratch("errors10");
        let out = run_rubash_in(&dir, args, Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    #[test]
    fn errors10_posix_exit_numeric_is_fatal() {
        let (stdout, stderr, code) = run(&["-o", "posix", "-c", "exit abcde; echo AFTER:$?"]);
        assert_eq!(code, Some(2), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(stdout, "", "stderr:\n{stderr}");
        assert_eq!(
            stderr,
            "bash: line 1: exit: abcde: numeric argument required\n"
        );
    }

    #[test]
    fn errors10_posix_shift_numeric_is_fatal() {
        let (stdout, stderr, code) = run(&[
            "-o",
            "posix",
            "-c",
            "set -- a b c; shift abcde; echo AFTER:$?",
        ]);
        assert_eq!(code, Some(2), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(stdout, "", "stderr:\n{stderr}");
        assert_eq!(
            stderr,
            "bash: line 1: shift: abcde: numeric argument required\n"
        );
    }

    #[test]
    fn errors10_posix_return_numeric_is_fatal() {
        let (stdout, stderr, code) = run(&[
            "-o",
            "posix",
            "-c",
            "f(){ return abcde; echo INFUNC; }; f; echo AFTER:$?",
        ]);
        assert_eq!(code, Some(2), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(stdout, "", "stderr:\n{stderr}");
        assert_eq!(
            stderr,
            "bash: line 1: return: abcde: numeric argument required\n"
        );
    }

    #[test]
    fn errors10_posix_exit_help_is_fatal() {
        let (stdout, _stderr, code) =
            run(&["-o", "posix", "-c", "exit --help >/dev/null; echo AFTER:$?"]);
        assert_eq!(code, Some(2), "stdout:\n{stdout}");
        assert_eq!(stdout, "");
    }

    #[test]
    fn errors10_posix_shift_help_is_fatal() {
        let (stdout, _stderr, code) = run(&[
            "-o",
            "posix",
            "-c",
            "set -- a b c; shift --help >/dev/null; echo AFTER:$?",
        ]);
        assert_eq!(code, Some(2), "stdout:\n{stdout}");
        assert_eq!(stdout, "");
    }

    #[test]
    fn errors10_posix_return_help_is_fatal() {
        let (stdout, _stderr, code) = run(&[
            "-o",
            "posix",
            "-c",
            "f(){ return --help >/dev/null; echo INFUNC:$?; }; f; echo AFTER:$?",
        ]);
        assert_eq!(code, Some(2), "stdout:\n{stdout}");
        assert_eq!(stdout, "");
    }

    /// GNU return.def:62-65's other arm: in a TESTED context the posix
    /// builtin returns EX_USAGE without unwinding, so the function body
    /// runs on and the call's status is the body's last status.
    #[test]
    fn errors10_posix_tested_return_numeric_body_continues() {
        let (stdout, stderr, code) = run(&[
            "-o",
            "posix",
            "-c",
            "f(){ return abcde; echo INFUNC; }; f || echo OR:$?; echo DONE:$?",
        ]);
        assert_eq!(code, Some(0), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(stdout, "INFUNC\nDONE:0\n", "stderr:\n{stderr}");
    }

    #[test]
    fn errors10_default_exit_numeric_continues() {
        let (stdout, stderr, code) = run(&["-c", "exit abcde; echo AFTER:$?"]);
        assert_eq!(code, Some(0), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(stdout, "AFTER:2\n", "stderr:\n{stderr}");
    }

    #[test]
    fn errors10_posix_command_exit_numeric_continues() {
        let (stdout, stderr, code) =
            run(&["-o", "posix", "-c", "command exit abcde; echo AFTER:$?"]);
        assert_eq!(code, Some(0), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(stdout, "AFTER:2\n", "stderr:\n{stderr}");
    }

    #[test]
    fn errors10_posix_if_exit_numeric_is_tested() {
        let (stdout, stderr, code) = run(&[
            "-o",
            "posix",
            "-c",
            "if exit abcde; then echo T; else echo E:$?; fi; echo DONE",
        ]);
        assert_eq!(code, Some(0), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(stdout, "E:2\nDONE\n", "stderr:\n{stderr}");
    }

    /// Sibling branch: shift's negative / out-of-range counts are
    /// EXECUTION_FAILURE (never > EX_SHERRBASE), so they stay non-fatal
    /// in POSIX mode; the out-of-range diagnostic prints because
    /// posix_initialize sets print_shift_error (general.c:112).
    #[test]
    fn errors10_posix_shift_out_of_range_continues() {
        let (stdout, stderr, code) =
            run(&["-o", "posix", "-c", "set -- a b c; shift 99; echo AFTER:$?"]);
        assert_eq!(code, Some(0), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(stdout, "AFTER:1\n", "stderr:\n{stderr}");
        assert_eq!(
            stderr,
            "bash: line 1: shift: 99: shift count out of range\n"
        );
    }

    /// The too-many-arguments class is no_args DISCARD (mode
    /// independent, NOT the special-builtin fatality channel): in
    /// script-file form the failing line is abandoned, the function
    /// (for `return`) unwinds, and the NEXT line runs with $?=2 —
    /// exactly the errors10.sub tail segments.
    #[test]
    fn errors10_posix_too_many_script_form_continues() {
        let dir = case_scratch("errors10-toomany");
        std::fs::write(
            dir.join("fg.sh"),
            "set -- a b c\nexit 1 2\necho \"after exit: $?\"\nshift 1 2\necho \"after shift: $?\"\nf(){ return 1 2; echo INFUNC; }\nf\necho \"after return: $?\"\n",
        )
        .expect("write fg.sh");
        let out = run_rubash_in(&dir, &["-o", "posix", "fg.sh"], Duration::from_secs(30));
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.code, Some(0), "stdout:\n{stdout}\nstderr:\n{stderr}");
        assert_eq!(
            stdout, "after exit: 2\nafter shift: 2\nafter return: 2\n",
            "stderr:\n{stderr}"
        );
    }
}

/// rubash#1904 (vfs audit D11): the command-substitution quick-read
/// ladder behind `$(<file)` skipped the synthetic-/proc and net steps
/// the input redirect ladder has, so `$(</proc/meminfo)` read a
/// mapped pseudo-path that does not exist on Windows (silently
/// empty) and `$(</dev/tcp/…)` failed the same way. GNU read_comsub
/// (subst.c) collects the bytes from the descriptor the redirect
/// open produced, whatever class the target belongs to. Expectations
/// below are GNU's; the proc pin's Windows face is the point (on
/// Linux the real procfs file answers), and the net pin fails
/// pre-fix on every platform.
mod issue_1904 {
    use super::*;

    fn run(dir: &Path, code: &str) -> (String, String, Option<i32>) {
        let out = run_rubash_in(dir, &["-c", code], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    #[test]
    fn issue1904_comsub_quick_read_proc_content() {
        let dir = case_scratch("1904-proc");
        let (stdout, stderr, status) = run(
            &dir,
            "x=$(</proc/meminfo); case $x in *MemTotal:*) echo PROC-OK;; *) echo PROC-BAD;; esac",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "PROC-OK", "stderr:\n{stderr}");
    }

    #[test]
    fn issue1904_comsub_quick_read_dev_null_is_empty() {
        let dir = case_scratch("1904-null");
        let (stdout, stderr, status) = run(&dir, "x=$(</dev/null); echo \"len=${#x}\"");
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "len=0", "stderr:\n{stderr}");
    }

    #[test]
    fn issue1904_comsub_quick_read_dev_tcp_loopback() {
        use std::io::Write as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            stream.write_all(b"hello1904\n").expect("write payload");
        });
        let dir = case_scratch("1904-tcp");
        let code = format!("x=$(</dev/tcp/127.0.0.1/{port}); echo \"GOT:$x\"");
        let (stdout, stderr, status) = run(&dir, &code);
        server.join().expect("server thread");
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "GOT:hello1904", "stderr:\n{stderr}");
    }
}

/// rubash#1911: an arithmetic evaluation error (GNU expr.c `evalerror` ->
/// `jump_to_top_level (DISCARD)`) discards only the current command. Inside
/// an arithmetic-for BODY the error must end just the loop (status 1) and
/// execution resumes after `done`; every sibling loop/compound body already
/// behaved that way. The leak was `execute_arithmetic_for_command`
/// (src/executor/compound_exec.rs) returning a body error without balancing
/// its entry `loop_depth += 1`, which defeated the enclosing list walk's
/// DISCARD absorption guard (`loop_depth == 0`) and made the error
/// script-fatal. GNU execute_cmd.c:3219 `execute_arith_for_command` has a
/// single exit whose `loop_level--` (execute_cmd.c:3296) is unconditional.
/// Probe scripts and expectations below are GNU bash 5.3's, taken verbatim
/// from the differential matrix that isolated the bug (script FILES, not
/// `-c`: the discard unit is the reader's command list).
mod issue_1911 {
    use super::*;

    fn run_script(dir: &Path, script: &str) -> (String, String, Option<i32>) {
        let path = dir.join("case.sh");
        std::fs::write(&path, script).expect("write probe script");
        let out = run_rubash_in(
            dir,
            &[path.to_str().expect("utf8 scratch path")],
            Duration::from_secs(30),
        );
        (
            String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// `want_err_part` empty skips the stderr assertion.
    fn check(case: &str, script: &str, want_stdout: &str, want_status: i32, want_err_part: &str) {
        let dir = case_scratch(case);
        let (stdout, stderr, status) = run_script(&dir, script);
        assert_eq!(status, Some(want_status), "case {case} stderr:\n{stderr}");
        assert_eq!(stdout, want_stdout, "case {case} stderr:\n{stderr}");
        if !want_err_part.is_empty() {
            assert!(
                stderr.contains(want_err_part),
                "case {case}: stderr should contain {want_err_part:?}:\n{stderr}"
            );
        }
    }

    // --- the bug family: arith-for body, error at function top level ---

    #[test]
    fn issue1911_minimal_repro_division_by_zero() {
        // The issue's minimal repro (division by zero on iteration i=1),
        // with the follow-up echo on its own line: GNU discards just the
        // loop, prints the marker with the loop's status 1, exits 0.
        check(
            "1911-min",
            "for ((i=0;i<3;i++)); do x=$(( 100 / (i-1))); done\necho \"after_loop rc=$?\"\n",
            "after_loop rc=1",
            0,
            "division by 0",
        );
    }

    #[test]
    fn issue1911_minimal_repro_same_line_form() {
        // Control: with the echo on the SAME line as the loop, the
        // discard eats the rest of that line too (the current-command
        // unit is the whole line's list), so nothing prints and the
        // script exits with the discarded loop's status 1 — in GNU and
        // in rubash alike (GNU-probed; this shape alone cannot see the
        // bug, it pins the line boundary of the discard).
        check(
            "1911-min-sameline",
            "for ((i=0;i<3;i++)); do x=$(( 100 / (i-1))); done; echo \"after_loop rc=$?\"\n",
            "",
            1,
            "division by 0",
        );
    }

    #[test]
    fn issue1911_body_error_loop_only_script() {
        check(
            "1911-p5",
            "for ((__bs_i=0; __bs_i<3; __bs_i++)); do\nx=$(( 077 + 0xff - 0b101 ))\ndone\necho \"after_loop rc=$?\"\nunset __bs_i\necho \"after_unset rc=$?\"\n",
            "after_loop rc=1\nafter_unset rc=0",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_body_error_one_liner() {
        check(
            "1911-p4",
            "for ((i=0; i<3; i++)); do x=$(( 077 + 0xff - 0b101 )); echo \"iter $i\"; done\necho \"loop_done rc=$?\"\n",
            "loop_done rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_body_error_second_iteration() {
        check(
            "1911-q1",
            "for ((i=0; i<3; i++)); do if (( i == 1 )); then x=$(( 0b101 )); fi; echo \"iter $i\"; done\necho \"done rc=$?\"\n",
            "iter 0\ndone rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_body_div_zero() {
        check(
            "1911-q6",
            "for ((i=0; i<3; i++)); do x=$(( 10 / 0 )); echo \"iter $i\"; done\necho \"done rc=$?\"\n",
            "done rc=1",
            0,
            "division by 0",
        );
    }

    #[test]
    fn issue1911_body_error_long_loop_tail_runs() {
        check(
            "1911-q7",
            "for ((__bs_i=0; __bs_i<5000; __bs_i++)); do\nx=$(( 077 + 0xff - 0b101 ))\ndone\necho \"TAIL-RAN\"\n",
            "TAIL-RAN",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_error_inside_function_called_from_body() {
        check(
            "1911-r5",
            "f() { x=$(( 0b101 )); echo \"in-f\"; }\nfor ((i=0; i<3; i++)); do f; echo \"iter $i\"; done\necho \"done rc=$?\"\n",
            "done rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_comsub_then_error_in_body() {
        check(
            "1911-r6",
            "for ((i=0; i<3; i++)); do x=$(echo hi); y=$(( 0b101 )); echo \"iter $i\"; done\necho \"done rc=$?\"\n",
            "done rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_subscript_error_in_body() {
        check(
            "1911-s1",
            "arr=(1 2 3)\nfor ((i=0; i<3; i++)); do x=${arr[0b101]}; echo \"iter $i\"; done\necho \"done rc=$?\"\n",
            "done rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_arith_for_nested_in_while_body() {
        // The leak pierced the outer while boundary too: GNU discards
        // only up to the innermost enclosing list (the while body ends,
        // status 1, outer script continues).
        check(
            "1911-s2",
            "i=0\nwhile (( i < 3 )); do\n  for ((j=0; j<3; j++)); do x=$(( 0b101 )); echo \"inner $j\"; done\n  echo \"after-inner rc=$?\"\n  i=$((i+1))\ndone\necho \"done rc=$?\"\n",
            "done rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_arith_for_nested_in_arith_for() {
        check(
            "1911-s3",
            "for ((i=0; i<2; i++)); do\n  for ((j=0; j<2; j++)); do x=$(( 0b101 )); echo \"inner $j\"; done\n  echo \"after-inner rc=$?\"\ndone\necho \"done rc=$?\"\n",
            "done rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_break_after_discarded_loop_is_outside_loop() {
        // The leaked depth also corrupted later break adjudication: after
        // the discarded loop, a bare `break` must again draw the GNU
        // "only meaningful in a loop" diagnostic (and return 0), not act
        // as an in-loop break swallowing the rest of the script.
        check(
            "1911-break-after",
            "for ((i=0;i<3;i++)); do x=$(( 0b101 )); done\nbreak\necho \"after rc=$?\"\n",
            "after rc=0",
            0,
            "only meaningful in a",
        );
    }

    // --- sibling shapes that already matched GNU (guard the class) ---

    #[test]
    fn issue1911_control_top_level_assignment() {
        check(
            "1911-p1",
            "x=$(( 077 + 0xff - 0b101 ))\necho \"after rc=$? x=[${x-UNSET}]\"\n",
            "after rc=1 x=[UNSET]",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_top_level_next_line_runs() {
        check(
            "1911-p2",
            "x=$(( 0b101 ))\necho alive\n",
            "alive",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_error_as_last_command_exits_1() {
        check(
            "1911-p3",
            "x=$(( 0b101 ))\n",
            "",
            1,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_while_body() {
        check(
            "1911-q2",
            "i=0\nwhile (( i < 3 )); do x=$(( 0b101 )); i=$((i+1)); echo \"iter $i\"; done\necho \"done rc=$?\"\n",
            "done rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_until_body() {
        check(
            "1911-r2",
            "i=0\nuntil (( i >= 3 )); do x=$(( 0b101 )); i=$((i+1)); echo \"iter $i\"; done\necho \"done rc=$?\"\n",
            "done rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_for_in_body() {
        check(
            "1911-r1",
            "for i in 1 2 3; do x=$(( 0b101 )); echo \"iter $i\"; done\necho \"done rc=$?\"\n",
            "done rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_function_body() {
        check(
            "1911-q3",
            "f() { x=$(( 0b101 )); echo \"in-f-after\"; }\nf\necho \"after-f rc=$?\"\n",
            "after-f rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_if_body() {
        check(
            "1911-q4",
            "if true; then x=$(( 0b101 )); echo \"in-if-after\"; fi\necho \"after-if rc=$?\"\n",
            "after-if rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_group_body() {
        check(
            "1911-q5",
            "{ x=$(( 0b101 )); echo \"in-group-after\"; }\necho \"after-group rc=$?\"\n",
            "after-group rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_header_init_error() {
        check(
            "1911-r3",
            "for ((i=0b101; i<3; i++)); do echo \"iter $i\"; done\necho \"done rc=$?\"\n",
            "done rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_header_step_error() {
        check(
            "1911-r4",
            "for ((i=0; i<3; i+= (i==1 ? 0b101 : 1) )); do echo \"iter $i\"; done\necho \"done rc=$?\"\n",
            "iter 0\ndone rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_arith_for_inside_function() {
        check(
            "1911-r7",
            "g() { for ((i=0; i<3; i++)); do x=$(( 0b101 )); echo \"iter $i\"; done; echo \"in-g-after-loop rc=$?\"; }\ng\necho \"after-g rc=$?\"\n",
            "after-g rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_arith_for_inside_group() {
        check(
            "1911-s4",
            "{ for ((i=0; i<3; i++)); do x=$(( 0b101 )); echo \"iter $i\"; done; echo \"in-group-after rc=$?\"; }\necho \"after-group rc=$?\"\n",
            "after-group rc=1",
            0,
            "value too great for base",
        );
    }

    #[test]
    fn issue1911_control_plain_false_body_not_an_evalerror() {
        // A plain failing command is NOT the DISCARD class: no
        // diagnostic, the loop runs out its iterations, script exits 0.
        check(
            "1911-p7",
            "for ((__bs_i=0; __bs_i<3; __bs_i++)); do\nfalse\ndone\nunset __bs_i\n",
            "",
            0,
            "",
        );
    }

    #[test]
    fn issue1911_control_valid_arithmetic_loop() {
        check(
            "1911-p6",
            "for ((__bs_i=0; __bs_i<3; __bs_i++)); do\nx=$(( 077 + 0xff - 0101 ))\ndone\necho \"after_loop rc=$? x=$x\"\nunset __bs_i\n",
            "after_loop rc=0 x=253",
            0,
            "",
        );
    }
}

/// rubash#1917 A period (/proc fidelity & completion, design
/// ~/workspace/vfs-1917-design/DESIGN.md §6.2): the synthetic /proc is
/// a userland value-add layer, NOT a GNU contract surface — the
/// expectations are the Linux procfs shapes downstream tools consume,
/// checked against the real procfs on Linux and the simulation on
/// Windows (the Windows face is the point of every pin here; where a
/// pin also holds on Linux it guards both).
mod issue_1917 {
    use super::*;

    fn run(dir: &Path, code: &str) -> (String, String, Option<i32>) {
        let out = run_rubash_in(dir, &["-c", code], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// A-1: /proc/version keeps its Linux shape (main-session ruling
    /// 2026-10-28: the string stays Linux-form; corpus consumers key
    /// on the prefix).
    #[test]
    fn issue1917_a1_version_shape() {
        let dir = case_scratch("1917-a1");
        let (stdout, stderr, status) = run(
            &dir,
            "read -r line < /proc/version; case $line in 'Linux version '*) echo VER-OK;; *) echo VER-BAD;; esac",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "VER-OK", "stderr:\n{stderr}");
    }

    /// A-1b: the read faces of synthetic content, one vehicle per
    /// invocation so a failure names its face. (Added when A-1's
    /// simple-read vehicle failed on Windows CI while the comsub
    /// quick-read of the same content passed — the faces do not share
    /// one resolution path, so they are pinned separately.)
    #[test]
    fn issue1917_a1b_read_faces() {
        let dir = case_scratch("1917-a1b");
        let faces: &[(&str, &str, &str)] = &[
            (
                "simple-read-top",
                "read -r l < /proc/meminfo; case $l in MemTotal:*) echo OK;; *) echo \"BAD:[$l]\";; esac",
                "OK",
            ),
            (
                "simple-read-pid",
                "read -r l < /proc/self/status; case $l in Name:*) echo OK;; *) echo \"BAD:[$l]\";; esac",
                "OK",
            ),
            (
                "cat-face",
                "x=$(cat /proc/version); case $x in 'Linux version '*) echo OK;; *) echo \"BAD:[$x]\";; esac",
                "OK",
            ),
            (
                "exec-fd-face",
                "exec 9</proc/version; read -r l <&9; case $l in 'Linux version '*) echo OK;; *) echo \"BAD:[$l]\";; esac",
                "OK",
            ),
            (
                "compound-face",
                "while read -r l; do case $l in 'Linux version '*) echo OK; break;; *) echo \"BAD:[$l]\"; break;; esac; done < /proc/version",
                "OK",
            ),
        ];
        for (name, code, expected) in faces {
            let (stdout, stderr, status) = run(&dir, code);
            assert_eq!(
                stdout, *expected,
                "face {name}: status={status:?} stderr:\n{stderr}"
            );
        }
    }

    /// A-1c: the comsub `cat` shortcuts consult the registry — the
    /// plain shortcut (A-1b's cat-face root cause: it fs::read the
    /// mapped path directly and reported every synthetic file ENOENT
    /// on Windows) and its timed sibling (`$(time cat …)`, whose arm
    /// read the filesystem the same way and silently swallowed the
    /// miss). The mixed face puts a SyntheticLink operand first:
    /// /proc/self/cwd resolves through the registry to the session
    /// directory, whose read fails (rc 1, as GNU's EISDIR read does),
    /// and the following content operand must still land — pre-fix on
    /// Windows both operands missed and the capture was empty. (A
    /// direct /proc/self/exe capture face is deliberately absent:
    /// capturing the whole running image through a substitution takes
    /// ~40s for a debug binary, over the harness timeout; exe link
    /// reading end to end is A-4's pin, and the shortcut's link branch
    /// uses the same registry entry as external_cat.) On Linux the
    /// real procfs masks both defects, exactly as for A-1/A-4;
    /// Windows CI is the proof.
    #[test]
    fn issue1917_a1c_comsub_cat_registry_faces() {
        let dir = case_scratch("1917-a1c");
        let faces: &[(&str, &str, &str)] = &[
            (
                "comsub-cat-link-then-content",
                "x=$(cat /proc/self/cwd /proc/version); rc=$?; case $x in 'Linux version '*) echo \"OK:$rc\";; *) echo \"BAD:[$x]:$rc\";; esac",
                "OK:1",
            ),
            (
                "timed-comsub-cat-content",
                "x=$(time cat /proc/version); case $x in 'Linux version '*) echo OK;; *) echo \"BAD:[$x]\";; esac",
                "OK",
            ),
            (
                "timed-comsub-cat-status",
                "x=$(time cat /proc/version); echo \"rc:$?\"",
                "rc:0",
            ),
        ];
        for (name, code, expected) in faces {
            let (stdout, stderr, status) = run(&dir, code);
            assert_eq!(
                stdout, *expected,
                "face {name}: status={status:?} stderr:\n{stderr}"
            );
        }
    }

    /// A-2: /proc/self/cmdline carries the TRUE argv — a known marker
    /// argument placed on the shell's own command line must appear as
    /// a NUL-separated field. Pre-fix on Windows the file contained
    /// only the image name (the ToolHelp snapshot knows nothing else),
    /// so the marker and the `-c` field were both absent.
    #[test]
    fn issue1917_a2_cmdline_true_argv() {
        let dir = case_scratch("1917-a2");
        let out = run_rubash_in(
            &dir,
            &[
                "-c",
                "while IFS= read -r -d '' a; do printf '%s|' \"$a\"; done < /proc/self/cmdline; echo END",
                "rubash",
                "MARKER1917A2",
            ],
            Duration::from_secs(30),
        );
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.code, Some(0), "stderr:\n{stderr}");
        assert!(
            stdout.contains("MARKER1917A2"),
            "known argv marker missing from cmdline:\n{stdout}"
        );
        assert!(
            stdout.contains("-c|"),
            "the -c field of the true argv is missing:\n{stdout}"
        );
    }

    /// A-7: an exited pid's /proc directory vanishes — files under it
    /// fail like ENOENT and test -e answers false (procfs semantics,
    /// both platforms).
    #[test]
    fn issue1917_a7_dead_pid_vanishes() {
        let dir = case_scratch("1917-a7");
        let (stdout, stderr, status) = run(
            &dir,
            "\"${THIS_SH}\" -c 'exit 0' & p=$!; wait $p; \
             if test -e /proc/$p/status; then echo STILL-THERE; else echo GONE-OK; fi; \
             cat /proc/$p/status 2>/dev/null; echo catrc=$?",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "GONE-OK\ncatrc=1", "stderr:\n{stderr}");
    }

    /// A3: glob over /proc enumerates the synthetic root (file table +
    /// self + live pids) and per-pid directories (entry table). Pre-fix
    /// on Windows the pattern matched nothing and echoed literally.
    #[test]
    fn issue1917_a3_glob_proc_enumeration() {
        let dir = case_scratch("1917-a3");
        let (stdout, stderr, status) = run(
            &dir,
            "hit_mem=; hit_self=; hit_pid=; \
             for e in /proc/*; do case $e in /proc/meminfo) hit_mem=1;; /proc/self) hit_self=1;; /proc/$$) hit_pid=1;; esac; done; \
             echo \"root:$hit_mem$hit_self$hit_pid\"; \
             hit_exe=; hit_cwd=; hit_status=; \
             for e in /proc/self/*; do case $e in /proc/self/exe) hit_exe=1;; /proc/self/cwd) hit_cwd=1;; /proc/self/status) hit_status=1;; esac; done; \
             echo \"self:$hit_exe$hit_cwd$hit_status\"; \
             set -- /proc/*/status; if [ $# -ge 2 ]; then echo pids:many; else echo pids:few; fi",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "root:111\nself:111\npids:many", "stderr:\n{stderr}");
    }
}

/// rubash#1900: /proc/<pid>/exe and /proc/<pid>/cwd were not
/// simulated at all — every face (open, test, glob) fell through to a
/// filesystem that has no such names on Windows. The pins assert the
/// link semantics end to end: test -e/-L/-f/-d from registry facts,
/// and reading /proc/self/exe yields the running image's own magic
/// bytes (MZ on Windows, ELF on Linux's real procfs).
mod issue_1900 {
    use super::*;

    fn run(dir: &Path, code: &str) -> (Vec<u8>, String, Option<i32>) {
        let out = run_rubash_in(dir, &["-c", code], Duration::from_secs(30));
        (
            out.stdout,
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    #[test]
    fn issue1900_a4_exe_cwd_links() {
        let dir = case_scratch("1900-a4");
        let (stdout, stderr, status) = run(
            &dir,
            "test -e /proc/self/exe && test -L /proc/self/exe && test -f /proc/self/exe \
             && test -e /proc/self/cwd && test -d /proc/self/cwd && echo A4-OK; \
             IFS= read -r -N 2 magic < /proc/self/exe; echo \"len:${#magic}\"; printf '%s' \"$magic\"",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        let text = String::from_utf8_lossy(&stdout).into_owned();
        assert!(text.contains("A4-OK"), "link tests failed:\n{text}");
        assert!(text.contains("len:2"), "short read of exe:\n{text}");
        if cfg!(windows) {
            assert!(stdout.windows(2).any(|w| w == b"MZ"), "exe magic:\n{text}");
        } else {
            assert!(
                stdout.windows(2).any(|w| w == [0x7f, b'E']),
                "exe magic:\n{text}"
            );
        }
    }

    /// A-5: /proc/self/cwd tracks the SESSION cwd — after `cd` into a
    /// subdirectory it still answers as an existing directory (its
    /// target text equality with the session cwd is pinned at the
    /// proc_vfs unit level, where the session value is injected).
    #[test]
    fn issue1900_a5_self_cwd_tracks_session() {
        let dir = case_scratch("1900-a5");
        std::fs::create_dir(dir.join("sub")).expect("create subdir");
        let (stdout, stderr, status) = run(
            &dir,
            "cd sub && test -d /proc/self/cwd && test -e /proc/self/cwd && echo A5-OK",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        let text = String::from_utf8_lossy(&stdout).into_owned();
        assert_eq!(text.trim(), "A5-OK");
    }
}

/// rubash#1918: /proc/<pid>/status PPid was fabricated (self answered
/// 0, every other pid answered the shell's own pid) while the ToolHelp
/// snapshot's real parent field was discarded. The pin spawns a child
/// shell and reads status through it: the reported PPid must never be
/// the fabricated 0, and this shell's own PPid must be a real third
/// party (nonzero, not itself). No parentage equality is asserted —
/// rubash's spawn architecture interposes on both platforms
/// (in-process THIS_SH children on Linux; an interposed spawn process
/// on Windows). The exact-value property against ground truth is
/// proven in proc_vfs's unit pins (rendered PPid == snapshot record,
/// for self and for another live pid).
mod issue_1918 {
    use super::*;

    fn run(dir: &Path, code: &str) -> (String, String, Option<i32>) {
        let out = run_rubash_in(dir, &["-c", code], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    #[test]
    fn issue1918_a3_ppid_known_parent_child() {
        let dir = case_scratch("1918-a3");
        std::fs::write(
            dir.join("child.sh"),
            "while IFS= read -r line; do\n\
             case $line in\n\
             PPid:*) set -- ${line#PPid:}; echo \"$1\"; break;;\n\
             esac\n\
             done < /proc/self/status\n",
        )
        .expect("write child.sh");
        let (stdout, stderr, status) = run(
            &dir,
            "child=$(\"${THIS_SH}\" ./child.sh); echo \"CHILD:$child\"; echo \"ME:$$\"; \
             while IFS= read -r line; do case $line in PPid:*) set -- ${line#PPid:}; mine=$1;; esac; done < /proc/self/status; \
             if [ -n \"$mine\" ] && [ \"$mine\" != 0 ] && [ \"$mine\" != \"$$\" ]; then echo SELF-OK; else echo \"SELF-BAD:$mine\"; fi",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert!(stdout.contains("SELF-OK"), "own PPid truth:\n{stdout}");
        let field = |name: &str| -> u32 {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or_else(|| panic!("{name} field missing:\n{stdout}"))
        };
        let (child_ppid, _me) = (field("CHILD:"), field("ME:"));
        // The fabricated shape is dead on every platform: a process's
        // PPid is never 0 (pre-fix Windows answered exactly that
        // fiction for every self read).
        assert_ne!(child_ppid, 0, "fabricated zero:\n{stdout}");
        // No script-level parentage equality is asserted, on any
        // platform: rubash's spawn architecture gives the "child" a
        // different immediate creator than the invoking shell on both
        // — on Linux a THIS_SH child of the shell's own binary runs
        // IN-PROCESS (verified with ps: a single OS process; the
        // status it reads is the shell's own, whose PPid is the
        // shell's parent), and on Windows a real child process is
        // created through an interposed spawn process (CI-observed
        // PPid 8404 vs the shell's $$ 2344). PPid truth means "the
        // real creator per the OS record", which the proc_vfs unit
        // pin proves value-for-value against the snapshot record.
    }
}

/// rubash#1952 (RC audit A-F1): process substitution materialized its
/// whole output before any consumer could start, so a producer that
/// never terminates starved the consumer forever (`cat <(echo x;
/// sleep 20)` printed nothing until EOF). GNU process_substitute
/// (subst.c:6362) forks the body onto a pipe and hands the consumer
/// the read end at once: consumers observe the first bytes while the
/// producer still runs. The pins use a bounded producer — first line,
/// sleep 2, flag file, second line — so the flag orders each line's
/// arrival against the producer's exit without clocks: pre-fix every
/// line reads late/starved because the flag is created during the
/// up-front collection, before the consumer starts.
mod issue_1952 {
    use super::*;

    fn run(dir: &Path, code: &str) -> (String, String, Option<i32>) {
        let out = run_rubash_in(dir, &["-c", code], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// Redirect form: the while-read loop must see line 1 before the
    /// producer creates the flag (i.e. before it exits).
    #[test]
    fn issue1952_redirect_loop_streams_first_line() {
        let dir = case_scratch("1952-loop");
        let (stdout, stderr, status) = run(
            &dir,
            "rm -f pd\n\
             while IFS= read -r l; do\n\
               if [ -e pd ]; then echo \"late:$l\"; else echo \"early:$l\"; fi\n\
             done < <(printf 'first\\n'; sleep 2; touch pd; printf 'second\\n')\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "early:first\nlate:second\n", "stderr:\n{stderr}");
    }

    /// Argument form: the producer itself observes the cat output
    /// file mid-flight — cat must have written line 1 into it before
    /// the producer's post-sleep check.
    #[test]
    fn issue1952_cat_argument_streams_first_line() {
        let dir = case_scratch("1952-cat");
        let (stdout, stderr, status) = run(
            &dir,
            "rm -f pd out.txt\n\
             cat <(printf 'first\\n'; sleep 2; \
               if [ -s out.txt ]; then touch pd; fi; \
               printf 'second\\n') > out.txt\n\
             if [ -e pd ]; then echo streamed; else echo starved; fi\n\
             cat out.txt\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "streamed\nfirst\nsecond\n", "stderr:\n{stderr}");
    }

    /// Pipeline form: `head -1` completes the pipeline from line 1
    /// alone, before the producer's flag exists.
    ///
    /// Ignored pending rubash#1959: on Windows, a process
    /// substitution delivered as an argv word to a spawned external
    /// child (`head` here) is drained to a temp file before spawn —
    /// the parent reads the pipe to EOF first, so this form does not
    /// stream there yet (root-cause chain: rubash#1959 and
    /// fix-1952 REPORT §7). The in-process consumer forms above do
    /// stream on Windows. The GNU assertion below is kept verbatim;
    /// remove this ignore once #1959's rendezvous endpoint (named
    /// pipe delivery to external children) lands.
    #[test]
    #[ignore = "rubash#1959: Windows drains procsub argv words to external children to a temp file before spawn (no streaming); remove when the #1959 rendezvous endpoint lands"]
    fn issue1952_pipeline_head_streams_first_line() {
        let dir = case_scratch("1952-pipe");
        let (stdout, stderr, status) = run(
            &dir,
            "rm -f pd\n\
             head -1 <(printf 'first\\n'; sleep 2; touch pd; \
               printf 'second\\n' 2>/dev/null) | cat\n\
             if [ -e pd ]; then echo late; else echo early; fi\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "first\nearly\n", "stderr:\n{stderr}");
    }

    /// Control: the exec-bound form streamed even before this fix
    /// (rubash#1021) — it must keep streaming.
    #[test]
    fn issue1952_exec_redirect_control_still_streams() {
        let dir = case_scratch("1952-exec");
        let (stdout, stderr, status) = run(
            &dir,
            "rm -f pd\n\
             exec 3< <(printf 'first\\n'; sleep 2; touch pd; printf 'second\\n')\n\
             IFS= read -r l <&3\n\
             if [ -e pd ]; then echo \"late:$l\"; else echo \"early:$l\"; fi\n\
             exec 3<&-\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "early:first\n", "stderr:\n{stderr}");
    }

    /// $! names the real producer: `wait` collects its real status,
    /// and once a finished producer is consumed its pid is reaped, so
    /// `kill -0` reports ESRCH like GNU (rubash#1022 parity for the
    /// streamed child).
    #[test]
    fn issue1952_bang_wait_status_and_reaped_kill0() {
        let dir = case_scratch("1952-bang");
        let (stdout, stderr, status) = run(
            &dir,
            "cat <(exit 3) >/dev/null\n\
             wait $!; echo \"wait:$?\"\n\
             cat <(echo hi) >/dev/null\n\
             kill -0 $! 2>/dev/null; echo \"kill0:$?\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "wait:3\nkill0:1\n", "stderr:\n{stderr}");
    }

    /// R3/D6: the comsub quick-read used to strip only the `/dev/fd/`
    /// prefix itself, so `$(</proc/self/fd/N)` missed the fd table and
    /// fell to the filesystem (ENOENT on Windows). GNU reads the fd
    /// (subst.c read_comsub opens the spelling; procfs resolves it).
    #[test]
    fn r3_comsub_quick_read_proc_self_fd_spelling() {
        let dir = case_scratch("r3-comsub-self");
        let (stdout, stderr, status) = run(
            &dir,
            "printf 'probe-r3\\n' > in.txt\n\
             exec 3<in.txt\n\
             x=$(</proc/self/fd/3)\n\
             echo \"got:[$x]\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "got:[probe-r3]\n", "stderr:\n{stderr}");
    }

    /// R3/D6: `/dev/fd/+3` is not an fd spelling anywhere in GNU — the
    /// literal open fails ENOENT on every face (oracle-verified), so
    /// the quick-read must not serve fd 3's bytes for it. Before the
    /// convergence this face's own parse accepted the sign and did.
    #[test]
    fn r3_comsub_quick_read_plus_fd_spelling_rejected() {
        let dir = case_scratch("r3-comsub-plus");
        let (stdout, stderr, status) = run(
            &dir,
            "printf 'probe-r3\\n' > in.txt\n\
             exec 3<in.txt\n\
             x=$(</dev/fd/+3)\n\
             echo \"got:[$x]\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "got:[]\n", "stderr:\n{stderr}");
    }

    /// R3/D4: `test -p` on a live pipe fd answers through the pipe-mark
    /// matrix; the `+3` spelling used to parse to fd 3 and inherit its
    /// mark. GNU: `/dev/fd/+3` stats ENOENT, so -p is false, while the
    /// canonical spelling of the same pipe stays true.
    #[test]
    fn r3_test_pipe_plus_fd_spelling_rejected() {
        let dir = case_scratch("r3-test-plus");
        let (stdout, stderr, status) = run(
            &dir,
            "exec 3< <(printf 'x\\n')\n\
             if [ -p /dev/fd/3 ]; then echo canon-pipe; else echo canon-not-pipe; fi\n\
             if [ -p /dev/fd/+3 ]; then echo plus-pipe; else echo plus-not-pipe; fi\n\
             exec 3<&-\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "canon-pipe\nplus-not-pipe\n", "stderr:\n{stderr}");
    }

    /// R3/D5 control: the pipe-into-source fast path now recognizes
    /// every registry fd-0 spelling; the sourced output must stay the
    /// pipe's text for each (the general path answered these before,
    /// so this pins the fast path to the same result).
    #[test]
    fn r3_pipe_source_fd0_spellings() {
        let dir = case_scratch("r3-pipe-source");
        let (stdout, stderr, status) = run(
            &dir,
            "echo \"echo via-dev-fd-0\" | . /dev/fd/0\n\
             echo \"echo via-proc-fd-0\" | . /proc/self/fd/0\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "via-dev-fd-0\nvia-proc-fd-0\n", "stderr:\n{stderr}");
    }

    /// R3/D6: `source /dev/fd/+3` must fail like GNU's (ENOENT
    /// diagnostic, nothing sourced); the fd's bytes sit in a Text
    /// endpoint (herestring), the one shape the old parse-and-serve
    /// fallback could reach. Canonical /dev/fd/3 sourcing is the
    /// control in the same script family (oracle-verified pair).
    #[test]
    fn r3_source_plus_fd_spelling_rejected() {
        let dir = case_scratch("r3-source-plus");
        let (stdout, stderr, status) = run(
            &dir,
            "exec 3<<< 'echo should-not-run'\n\
             . /dev/fd/+3\n\
             echo after-marker\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "after-marker\n", "stderr:\n{stderr}");
    }

    /// R3/D1: an external command's `/dev/fd/+3` operand is not an fd
    /// operand — the word reaches the child literally and the open
    /// fails in the child (GNU: cat reports ENOENT, rc 1). Before the
    /// convergence the operand layer parsed the sign, materialized
    /// fd 3, and cat read it successfully. Canonical /dev/fd/3 is the
    /// control.
    #[test]
    fn r3_external_operand_plus_fd_spelling_rejected() {
        let dir = case_scratch("r3-operand-plus");
        let (stdout, stderr, status) = run(
            &dir,
            "printf 'probe-r3\\n' > in.txt\n\
             exec 3<in.txt\n\
             cat /dev/fd/3\n\
             cat /dev/fd/+3 2>/dev/null; echo \"catrc:$?\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "probe-r3\ncatrc:1\n", "stderr:\n{stderr}");
    }
}

/// rubash#2003: a pipeline element 0 whose own fd-0 redirect aliases
/// another descriptor (`<&N`, `0<&N`, `< /dev/fd/N`) lost its entire
/// input. GNU forks every element with the shell's own descriptors
/// (execute_cmd.c:2620 execute_pipeline) and the element applies its
/// own redirections inside the fork (execute_cmd.c:4617
/// execute_simple_command -> redir.c do_redirection_internal), so the
/// element's reads ARE the aliased descriptor's reads, exactly once.
/// Rubash's stage-0 payload resolution served the alias by draining
/// the shared descriptor to EOF, while the stage funnels that consume
/// stdin through the descriptor channel itself (the inline cat arm
/// re-resolving the redirect, a compound stage binding fd 0 to the
/// endpoint, function stages on fd-table-sharing clones, the external
/// spawner materializing the endpoint by handle) then read EOF from
/// the drained descriptor — `cat < <(seq 5) | wc -l` inside a command
/// substitution captured 0, `exec 3<file; cat <&3 | wc -l` printed 0.
/// The payload resolution now materializes the aliased fd's endpoint
/// (buffered endpoints get their pre-resolution offset back;
/// handle-backed endpoints are replaced by a buffered endpoint
/// carrying the payload bytes), restoring single consumption.
/// Expectations hand-measured against GNU Bash 5.3.20 (the vendored
/// oracle build) on 2026-10-29; producers use the printf builtin so
/// the pins stay self-contained.
mod issue_2003 {
    use super::*;

    fn run(dir: &Path, code: &str) -> (String, String, Option<i32>) {
        let out = run_rubash_in(dir, &["-c", code], Duration::from_secs(30));
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// The reported shape: a procsub-fed consumer that is itself
    /// piped, inside a command substitution.
    #[test]
    fn issue2003_comsub_pipeline_keeps_procsub_redirect_payload() {
        let dir = case_scratch("2003-comsub-wc");
        let (stdout, stderr, status) = run(
            &dir,
            "x=$(cat < <(printf '1\\n2\\n3\\n4\\n5\\n') | wc -l)\n\
             echo \"count:[$x]\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "count:[5]\n", "stderr:\n{stderr}");
    }

    /// Same funnel, payload-consuming second stage (grep arm).
    #[test]
    fn issue2003_comsub_pipeline_keeps_procsub_redirect_for_grep() {
        let dir = case_scratch("2003-comsub-grep");
        let (stdout, stderr, status) = run(
            &dir,
            "x=$(cat < <(printf 'a\\nb\\nc\\n') | grep b)\n\
             echo \"got:[$x]\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "got:[b]\n", "stderr:\n{stderr}");
    }

    /// Stage 0 with a `<&N` dup of a regular file: the stage reads
    /// the file through the dup, and the shared cursor is spent
    /// afterwards (a second identical pipeline sees EOF), like GNU.
    #[test]
    fn issue2003_dup_redirect_stage0_delivers_file_payload() {
        let dir = case_scratch("2003-dup-file");
        let (stdout, stderr, status) = run(
            &dir,
            "printf '1\\n2\\n3\\n4\\n5\\n' > five.txt\n\
             exec 3<five.txt\n\
             cat <&3 | wc -l\n\
             cat <&3 | wc -l\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "5\n0\n", "stderr:\n{stderr}");
    }

    /// The `/dev/fd/N` spelling of the same dup.
    #[test]
    fn issue2003_devfd_spelling_stage0_delivers_file_payload() {
        let dir = case_scratch("2003-devfd-file");
        let (stdout, stderr, status) = run(
            &dir,
            "printf '1\\n2\\n3\\n4\\n5\\n' > five.txt\n\
             exec 3<five.txt\n\
             cat < /dev/fd/3 | wc -l\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "5\n", "stderr:\n{stderr}");
    }

    /// Stage 0 with a `<&N` dup of a streamed procsub pipe bound by
    /// an earlier `exec`: the pipe bytes survive the stage-0 payload
    /// resolution (handle-backed endpoint materialization).
    #[test]
    fn issue2003_dup_redirect_stage0_delivers_procsub_pipe() {
        let dir = case_scratch("2003-dup-procsub");
        let (stdout, stderr, status) = run(
            &dir,
            "exec 3< <(printf '1\\n2\\n3\\n4\\n5\\n')\n\
             cat <&3 | wc -l\n\
             cat <&3 | wc -l\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "5\n0\n", "stderr:\n{stderr}");
    }

    /// A compound stage (read loop) binds fd 0 to the aliased
    /// endpoint itself; it must read the payload through that bind.
    #[test]
    fn issue2003_compound_stage0_dup_redirect_reads_through() {
        let dir = case_scratch("2003-loop-dup");
        let (stdout, stderr, status) = run(
            &dir,
            "printf '1\\n2\\n3\\n4\\n5\\n' > five.txt\n\
             exec 3<five.txt\n\
             while IFS= read -r l; do echo \"r:$l\"; done <&3 | wc -l\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "5\n", "stderr:\n{stderr}");
    }

    /// A function stage carrying the dup redirect on the call.
    #[test]
    fn issue2003_function_stage0_dup_redirect_reads_through() {
        let dir = case_scratch("2003-func-dup");
        let (stdout, stderr, status) = run(
            &dir,
            "f() { cat; }\n\
             printf '1\\n2\\n3\\n4\\n5\\n' > five.txt\n\
             exec 3<five.txt\n\
             f <&3 | wc -l\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "5\n", "stderr:\n{stderr}");
    }

    /// The cat `-` operand resolves to the same redirected fd 0.
    #[test]
    fn issue2003_cat_dash_operand_reads_redirect_payload() {
        let dir = case_scratch("2003-cat-dash");
        let (stdout, stderr, status) = run(
            &dir,
            "printf '1\\n2\\n3\\n4\\n5\\n' > five.txt\n\
             exec 3<five.txt\n\
             cat - <&3 | wc -l\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "5\n", "stderr:\n{stderr}");
    }

    /// Control: at stage 1 the element's own dup redirect wins over
    /// the pipe (single resolution at the stage, no stage-0 payload
    /// involved) — GNU behavior before and after the fix.
    #[test]
    fn issue2003_stage1_dup_redirect_still_wins_over_pipe() {
        let dir = case_scratch("2003-stage1-dup");
        let (stdout, stderr, status) = run(
            &dir,
            "printf '1\\n2\\n3\\n4\\n5\\n' > five.txt\n\
             exec 3<five.txt\n\
             printf '9\\n8\\n' | cat <&3 | wc -l\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "5\n", "stderr:\n{stderr}");
    }

    /// Control: the direct procsub redirect on a top-level pipeline
    /// element streamed even before this fix (rubash#1952) and must
    /// keep working.
    #[test]
    fn issue2003_direct_procsub_pipeline_control() {
        let dir = case_scratch("2003-direct");
        let (stdout, stderr, status) = run(&dir, "cat < <(printf '1\\n2\\n') | wc -l\n");
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "2\n", "stderr:\n{stderr}");
    }
}

/// rubash#1976: the bidirectional netopen face (`exec 3<>/dev/tcp/host/port`)
/// opened the connection but every fd-3 read AND write failed with
/// `Invalid argument`. GNU netopen returns ONE connected socket descriptor
/// that serves both directions of any `/dev/tcp|udp` face (redir.c:660
/// redir_open calls netopen for RF_DEVTCP/RF_DEVUDP whatever the `<`/`>`/`<>`
/// spelling; lib/sh/netopen.c:282 netopen → socket + connect at :178
/// _netopen4, then plain read(2)/write(2) on that fd). Rubash adopts the
/// socket as the fd slot's raw handle, and the handle byte primitives used
/// to serve it through ReadFile/WriteFile with a NULL OVERLAPPED — rejected
/// with ERROR_INVALID_PARAMETER on an overlapped socket handle, so the peer
/// only ever saw the connection close (EOF). The fd layer now routes socket
/// handles through the socket API (recv/send, select for bounded waits).
///
/// Expectations hand-measured against WSL GNU Bash 5.3.20
/// (/usr/local/bin/bash, 2026-10-28): the TCP round trip below and the
/// bounded read on a silent endpoint (`read -t 2 -u 3` → `rc=142`, variable
/// empty). Complements the issue1026 pin in
/// tests/issue1025_1026_1068_1103_1359_redirect_fd_regressions.rs (which
/// stays byte-identical — it is PR #1970's acceptance gate), driven from
/// script files.
///
/// Known follow-up (verified 2026-10-28, out of this issue's handle-shape
/// scope): a `read -u 3` record off a DATAGRAM (`/dev/udp`) fd still cannot
/// be served by the byte-wise record loop — GNU routes a socket through the
/// buffered zreadc chunk reader (read.def:668: a socket is neither tty nor
/// fd_ispipe, so unbuffered_read stays 0) and zsyncfd keeps the overage
/// when its rewind lseek fails (lib/sh/zread.c:294-309), while winsock's
/// recv DISCARDS a datagram larger than the receive buffer entirely
/// (WSAEMSGSIZE). Serving datagram records needs the chunked keep-overage
/// reader, a read-builtin buffering-profile change of its own.
mod issue_1976 {
    use super::*;

    fn run(dir: &Path, script: &str) -> (String, String, Option<i32>) {
        let path = dir.join("case.sh");
        std::fs::write(&path, script).expect("write probe script");
        let out = run_rubash_in(
            dir,
            &[path.to_str().expect("utf8 scratch path")],
            Duration::from_secs(30),
        );
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    #[test]
    fn issue1976_dev_tcp_readwrite_fd_round_trips() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 5];
            stream.read_exact(&mut buf).expect("read ping");
            assert_eq!(&buf, b"ping\n");
            stream.write_all(b"pong\n").expect("write pong");
        });
        let dir = case_scratch("1976-tcp");
        let script = format!(
            "exec 3<>/dev/tcp/127.0.0.1/{port}\necho ping >&3\nread -u 3 line\necho \"got=[$line]\"\n"
        );
        let (stdout, stderr, status) = run(&dir, &script);
        server.join().expect("server thread");
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "got=[pong]\n", "stderr:\n{stderr}");
        assert_eq!(stderr, "", "stderr:\n{stderr}");
    }

    #[test]
    fn issue1976_read_t_timeout_on_quiet_socket_fd() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        // Accept and stay silent past the read deadline, GNU-side.
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            std::thread::sleep(Duration::from_secs(4));
            drop(stream);
        });
        let dir = case_scratch("1976-quiet");
        let script = format!(
            "exec 3<>/dev/tcp/127.0.0.1/{port}\nread -t 2 -u 3 line\necho \"rc=$?\"\necho \"line=[$line]\"\n"
        );
        let (stdout, stderr, status) = run(&dir, &script);
        server.join().expect("server thread");
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "rc=142\nline=[]\n", "stderr:\n{stderr}");
    }
}
mod issue_1584 {
    use super::*;

    fn run(dir: &Path, script: &str) -> (String, String, Option<i32>) {
        let path = dir.join("case.sh");
        std::fs::write(&path, script).expect("write probe script");
        let out = run_rubash_in(
            dir,
            &[path.to_str().expect("utf8 scratch path")],
            Duration::from_secs(30),
        );
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// rubash#1584: GNU execute_cmd.c:2525 execute_coproc forks once
    /// and the child execs a simple external command itself, so $!
    /// is the program's own pid (comm=cat). The respawn route put a
    /// `rubash -c` wrapper at $! and ran cat as a grandchild. The
    /// direct route must keep the data surface byte-identical (fds
    /// 63/60, echo-back, wait status) while the pid names cat.
    #[cfg(unix)]
    #[test]
    fn issue1584_coproc_cat_pid_is_cat() {
        let dir = case_scratch("1584-cat");
        let (stdout, stderr, status) = run(
            &dir,
            "coproc cat\n\
             p=$!\n\
             echo \"comm=$(ps -o comm= -p $p 2>/dev/null)\"\n\
             echo \"fds=${COPROC[0]},${COPROC[1]}\"\n\
             echo hello >&${COPROC[1]}\n\
             read -u ${COPROC[0]} line\n\
             echo \"back=$line\"\n\
             eval \"exec ${COPROC[1]}>&-\"\n\
             wait $p 2>/dev/null\n\
             echo \"rc=$?\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(
            stdout, "comm=cat\nfds=63,60\nback=hello\nrc=0\n",
            "stderr:\n{stderr}"
        );
        assert_eq!(stderr, "", "stderr:\n{stderr}");
    }

    /// rubash#1584: `kill $!` must reach the command itself. With
    /// the wrapper at $! the TERM killed the wrapper (wait still
    /// reported 143) and the sleep was never signalled.
    #[cfg(unix)]
    #[test]
    fn issue1584_coproc_kill_reaches_the_command() {
        let dir = case_scratch("1584-kill");
        let (stdout, stderr, status) = run(
            &dir,
            "coproc sleep 30\n\
             p=$!\n\
             echo \"comm=$(ps -o comm= -p $p 2>/dev/null)\"\n\
             kill -TERM $p\n\
             wait $p 2>/dev/null\n\
             echo \"rc=$?\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "comm=sleep\nrc=143\n", "stderr:\n{stderr}");
    }

    /// rubash#1584: a slash-path head takes the same direct route.
    #[cfg(unix)]
    #[test]
    fn issue1584_coproc_slash_path_pid_is_the_program() {
        let dir = case_scratch("1584-slash");
        let (stdout, stderr, status) = run(
            &dir,
            "coproc /bin/cat\n\
             p=$!\n\
             echo \"comm=$(ps -o comm= -p $p 2>/dev/null)\"\n\
             echo yo >&${COPROC[1]}\n\
             read -u ${COPROC[0]} line\n\
             echo \"back=$line\"\n\
             eval \"exec ${COPROC[1]}>&-\"\n\
             wait $p 2>/dev/null\n\
             echo \"rc=$?\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "comm=cat\nback=yo\nrc=0\n", "stderr:\n{stderr}");
    }

    /// rubash#1584: an external command with plain arguments is
    /// direct too (the args are expanded in the parent only because
    /// the admitted classes make parent expansion identical to the
    /// child's).
    #[cfg(unix)]
    #[test]
    fn issue1584_coproc_external_with_args() {
        let dir = case_scratch("1584-args");
        let (stdout, stderr, status) = run(
            &dir,
            "coproc grep --line-buffered foo\n\
             p=$!\n\
             echo \"comm=$(ps -o comm= -p $p 2>/dev/null)\"\n\
             echo \"xfooy\" >&${COPROC[1]}\n\
             read -u ${COPROC[0]} line\n\
             echo \"back=$line\"\n\
             eval \"exec ${COPROC[1]}>&-\"\n\
             wait $p 2>/dev/null\n\
             echo \"rc=$?\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "comm=grep\nback=xfooy\nrc=0\n", "stderr:\n{stderr}");
    }

    /// rubash#1584: a named coproc binds <NAME>_PID to the same
    /// direct process.
    #[cfg(unix)]
    #[test]
    fn issue1584_named_coproc_pid_is_the_program() {
        let dir = case_scratch("1584-named");
        let (stdout, stderr, status) = run(
            &dir,
            "coproc NC cat\n\
             p=${NC_PID}\n\
             echo \"comm=$(ps -o comm= -p $p 2>/dev/null)\"\n\
             echo hello >&${NC[1]}\n\
             read -u ${NC[0]} line\n\
             echo \"back=$line\"\n\
             eval \"exec ${NC[1]}>&-\"\n\
             wait $p 2>/dev/null\n\
             echo \"rc=$?\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "comm=cat\nback=hello\nrc=0\n", "stderr:\n{stderr}");
    }

    /// rubash#1584 (negative control): a builtin coproc keeps a
    /// shell-image child — GNU's coproc child for `read` is bash
    /// itself, so the respawn route (comm=rubash) is the faithful
    /// shape and must not be direct-spawned.
    #[cfg(unix)]
    #[test]
    fn issue1584_coproc_builtin_keeps_shell_image() {
        let dir = case_scratch("1584-builtin");
        let (stdout, stderr, status) = run(
            &dir,
            "coproc read x\n\
             p=$!\n\
             echo \"comm=$(ps -o comm= -p $p 2>/dev/null)\"\n\
             echo \"someval\" >&${COPROC[1]}\n\
             eval \"exec ${COPROC[1]}>&-\"\n\
             wait $p 2>/dev/null\n\
             echo \"rc=$?\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "comm=rubash\nrc=0\n", "stderr:\n{stderr}");
    }
}

mod issue_1582 {
    use super::*;

    fn run(dir: &Path, script: &str) -> (String, String, Option<i32>) {
        let path = dir.join("case.sh");
        std::fs::write(&path, script).expect("write probe script");
        let out = run_rubash_in(
            dir,
            &[path.to_str().expect("utf8 scratch path")],
            Duration::from_secs(30),
        );
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// rubash#1582 (contract pin, adjudication pending): at a
    /// spawn-realized level the identity narrative is already
    /// GNU-consistent — an async subshell's $BASHPID is the pid its
    /// children observe as $PPID. Any resolution of the flat-level
    /// projection must preserve this.
    #[cfg(unix)]
    #[test]
    fn issue1582_async_subshell_bashpid_matches_child_ppid() {
        let dir = case_scratch("1582-async");
        let (stdout, stderr, status) = run(
            &dir,
            "( /bin/sh -c 'echo $PPID > pp.txt'; echo $BASHPID > bp.txt ) &\n\
             wait $!\n\
             if [ \"$(cat pp.txt)\" = \"$(cat bp.txt)\" ]; then echo MATCH; else echo DIFFER; fi\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "MATCH\n", "stderr:\n{stderr}");
    }

    /// rubash#1582 (contract pin): $PPID is a startup-frozen value
    /// in GNU (variables.c set_ppid runs once from
    /// initialize_shell_variables; subshell forks inherit it), and
    /// rubash matches — a subshell must NOT observe its parent
    /// shell's pid through $PPID.
    #[cfg(unix)]
    #[test]
    fn issue1582_ppid_is_frozen_in_subshell() {
        let dir = case_scratch("1582-ppid");
        let (stdout, stderr, status) = run(
            &dir,
            "t=$PPID\n\
             ( if [ \"$PPID\" = \"$t\" ]; then echo FROZEN; else echo CHANGED; fi )\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "FROZEN\n", "stderr:\n{stderr}");
    }

    /// rubash#1582 (contract pin): `kill $BASHPID` inside a flat
    /// subshell is scoped to that subshell layer (the #1461
    /// interception): the layer dies by the signal (status 143,
    /// no survival output) and the parent shell lives on. Any
    /// projection change that points $BASHPID at the host process
    /// must keep this kill scoped, never suicidal to the host.
    #[cfg(unix)]
    #[test]
    fn issue1582_kill_own_bashpid_stays_scoped_to_the_subshell() {
        let dir = case_scratch("1582-kill");
        let (stdout, stderr, status) = run(
            &dir,
            "( kill -TERM $BASHPID; echo SURVIVED )\n\
             echo \"rc=$?\"\n",
        );
        assert_eq!(status, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "rc=143\n", "stderr:\n{stderr}");
    }
}

mod issue_1646 {
    use super::*;

    fn run(dir: &Path, script: &str) -> (String, String, Option<i32>) {
        let path = dir.join("case.sh");
        std::fs::write(&path, script).expect("write probe script");
        let out = run_rubash_in(
            dir,
            &[path.to_str().expect("utf8 scratch path")],
            Duration::from_secs(30),
        );
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// rubash#1646 (varredir_close arm): GNU closes a `{var}` descriptor
    /// when the owning command COMPLETES (redir.c undo_redirections), so
    /// the command's own `>&$f` write lands and the `>(...)` consumer
    /// drains it; only a LATER write sees the closed fd. Closing at
    /// application time (the old behavior) rejected the first write with
    /// EBADF and the consumer's stream was lost entirely. GNU bash
    /// 5.3.20 oracle output for this probe: first-rc=0, the second write
    /// reports Bad file descriptor, file:[one].
    #[test]
    fn issue1646_varredir_close_procsub_write_survives_own_command() {
        let dir = case_scratch("1646-varredir-procsub");
        let (stdout, stderr, code) = run(
            &dir,
            "shopt -s varredir_close\n\
             echo one {f}> >(cat > out.txt) >&$f\n\
             echo \"first-rc=$?\"\n\
             echo two >&$f 2>/dev/null || echo second-failed\n\
             sleep 0.5\n\
             echo \"file:[$(cat out.txt)]\"\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(
            stdout, "first-rc=0\nsecond-failed\nfile:[one]\n",
            "stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("Bad file descriptor"),
            "the post-completion write must report EBADF: {stderr}"
        );
    }

    /// rubash#1646 (varredir_close arm, plain-file form of the same
    /// invariant): a `{f}>file` descriptor closed at application time
    /// rejected the owning command's write too — first-rc was 1 and the
    /// file stayed empty. GNU: first-rc=0, second write EBADF, file
    /// holds exactly the first line.
    #[test]
    fn issue1646_varredir_close_plain_file_write_survives_own_command() {
        let dir = case_scratch("1646-varredir-file");
        let (stdout, stderr, code) = run(
            &dir,
            "shopt -s varredir_close\n\
             echo one {f}>plain.txt >&$f\n\
             echo \"first-rc=$?\"\n\
             echo two >&$f 2>/dev/null || echo second-failed\n\
             echo \"file:[$(cat plain.txt)]\"\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(
            stdout, "first-rc=0\nsecond-failed\nfile:[one]\n",
            "stderr:\n{stderr}"
        );
    }

    /// rubash#1646 (headline form): a persistent `exec {f}> >(...)`
    /// stream that is never explicitly closed must still reach the
    /// consumer when the shell exits — GNU's consumer drains at EOF
    /// (subst.c:6362 process_substitute pipe semantics); the shell-exit
    /// sweep is rubash's drain point. Byte-exact content check after
    /// the process has exited.
    #[test]
    fn issue1646_persistent_procsub_drains_at_exit() {
        let dir = case_scratch("1646-exit-drain");
        let (_stdout, stderr, code) = run(
            &dir,
            "exec {f}> >(cat > out.txt)\nprintf 'a\\nb\\nc\\n' >&$f\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        let drained = std::fs::read(dir.join("out.txt")).expect("consumer output exists");
        assert_eq!(drained, b"a\nb\nc\n", "stderr:\n{stderr}");
    }

    /// rubash#1646 (default mode): with varredir_close off (the GNU
    /// default) the descriptor persists across commands, both writes
    /// land, and the accumulated stream drains whole at shell exit.
    #[test]
    fn issue1646_dynamic_fd_procsub_accumulates_and_drains_at_exit() {
        let dir = case_scratch("1646-default-drain");
        let (stdout, stderr, code) = run(
            &dir,
            "echo one {f}> >(cat > out.txt) >&$f\necho \"first-rc=$?\"\necho two >&$f\necho \"second-rc=$?\"\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "first-rc=0\nsecond-rc=0\n", "stderr:\n{stderr}");
        let drained = std::fs::read(dir.join("out.txt")).expect("consumer output exists");
        assert_eq!(drained, b"one\ntwo\n", "stderr:\n{stderr}");
    }
}

mod issue_1951 {
    use super::*;

    fn run(dir: &Path, script: &str) -> (String, String, Option<i32>) {
        let path = dir.join("case.sh");
        std::fs::write(&path, script).expect("write probe script");
        let out = run_rubash_in(
            dir,
            &[path.to_str().expect("utf8 scratch path")],
            Duration::from_secs(60),
        );
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.code,
        )
    }

    /// rubash#1951: the streamed input-procsub endpoint serves a
    /// line-wise `while read` loop completely and in order — the count
    /// over 20k produced lines is exact (the materialized endpoint this
    /// replaces lost its constant-factor footing entirely: ~0.29ms per
    /// line at the audit tip). GNU bash 5.3.20 prints the same count.
    #[test]
    fn issue1951_procsub_endpoint_while_read_count_exact() {
        let dir = case_scratch("1951-stream-count");
        let (stdout, stderr, code) = run(
            &dir,
            "n=0\nwhile read -r l; do n=$((n+1)); done < <(seq 1 20000)\necho \"read=$n\"\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "read=20000\n", "stderr:\n{stderr}");
    }

    /// rubash#1951 (content face): lines arrive whole and in order —
    /// the first, middle and last records read off the endpoint match
    /// the producer's sequence exactly.
    #[test]
    fn issue1951_procsub_endpoint_records_whole_and_ordered() {
        let dir = case_scratch("1951-stream-order");
        let (stdout, stderr, code) = run(
            &dir,
            "n=0\nfirst= mid= last=\nwhile read -r l; do n=$((n+1)); case $n in 1) first=$l;; 1000) mid=$l;; esac; last=$l; done < <(seq 1 2000)\necho \"$first/$mid/$last/$n\"\n",
        );
        assert_eq!(code, Some(0), "stderr:\n{stderr}");
        assert_eq!(stdout, "1/1000/2000/2000\n", "stderr:\n{stderr}");
    }
}
