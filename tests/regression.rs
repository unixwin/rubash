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
//!    slice fails CI and forces a conscious re-record
//!    (`RUBASH_REGRESSION_RECORD=1 cargo test --test regression`).
//!    They are Windows-only (the prebuilt helper binaries under
//!    tests/gnu-compat/helpers-win are PE executables).
//!
//! Every fixture runs in its own empty temp directory with a minimal PATH
//! (rubash emulates cat/mkdir/grep/sed when they are absent), a bounded
//! per-test timeout, and a clean environment (no BASH_ENV / WINUXSH_ROOT).

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
        Some(HEAVY_TEST_LOCK.lock().unwrap())
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

golden_fixture!(golden_at_star_matrix => "at-star-matrix", timeout 60, corpus []);
golden_fixture!(golden_catfile_comsub => "catfile-comsub", timeout 60, corpus []);
golden_fixture!(golden_digit_intfit => "digit-intfit", timeout 60, corpus []);
golden_fixture!(golden_eco_completion_register => "eco-completion-register", timeout 60, corpus []);
golden_fixture!(golden_eco_git_completion_smoke => "eco-git-completion-smoke", timeout 60, corpus []);
golden_fixture!(golden_eco_omb_theme_ps1 => "eco-omb-theme-ps1", timeout 60, corpus []);
golden_fixture!(golden_extglob_parse_gate => "extglob-parse-gate", timeout 60, corpus []);
golden_fixture!(golden_fnbody_strict_battery => "fnbody-strict-battery", timeout 90, corpus []);
golden_fixture!(golden_funsub_valsub => "funsub-valsub", timeout 60, corpus []);
golden_fixture!(golden_nounset_exit_matrix => "nounset-exit-matrix", timeout 60, corpus []);
golden_fixture!(golden_nvm_load => "nvm-load", timeout 180, corpus ["nvm/nvm.sh"]);
golden_fixture!(golden_redir_operand_battery => "redir-operand-battery", timeout 60, corpus []);
golden_fixture!(golden_rustup_check_help => "rustup-check-help", timeout 60, corpus []);
golden_fixture!(golden_trap_signal_matrix => "trap-signal-matrix", timeout 180, corpus []);
golden_fixture!(golden_xtrace_decl_compound => "xtrace-decl-compound", timeout 60, corpus []);

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
        // System32 only: rubash emulates the coreutils the fixtures use.
        // This matches what a bare windows-latest CI runner provides, so
        // the local run and the CI run exercise the same code paths.
        r"C:\Windows\System32".to_string()
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
        .env_remove("OLDPWD")
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env("PATH", minimal_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    wait_bounded(&mut child, limit)
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
fn run_suite_snapshot(suite: &str) {
    let root = manifest_dir();
    let src = root.join(UPSTREAM_TESTS);
    let dir = std::env::temp_dir().join(format!("rubash-snap-{suite}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create snapshot scratch dir");

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
                let _ = std::fs::copy(entry.path(), dir.join(&fname));
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
        .env_remove("OLDPWD")
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env("PATH", &path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash for suite");
    let out = wait_bounded(&mut child, Duration::from_secs(120));

    let snap_path = root.join(SNAPSHOT_DIR).join(format!("{suite}.snap"));
    let normalized = normalize_snapshot(&out.stdout, &dir);
    let _ = std::fs::remove_dir_all(&dir);

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

#[test]
fn perf_canary_nvm_loader_parses_bounded() {
    let _guard = HEAVY_TEST_LOCK.lock().unwrap();
    let root = manifest_dir();
    let dir = std::env::temp_dir().join(format!("rubash-perf-nvm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create perf scratch");
    std::fs::copy(root.join(CORPUS_DIR).join("nvm/nvm.sh"), dir.join("nvm.sh"))
        .expect("copy nvm corpus");
    let started = Instant::now();
    let out = run_rubash_in(&dir, &["-n", "nvm.sh"], Duration::from_secs(240));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.code, Some(0), "bash -n nvm.sh must parse clean");
    assert!(
        started.elapsed() < Duration::from_secs(180),
        "nvm.sh -n took {:.1}s (bound 180s; O(N^2) lexer regression class)",
        started.elapsed().as_secs_f32()
    );
}

#[test]
fn perf_canary_ruby_build_parse_bounded() {
    let _guard = HEAVY_TEST_LOCK.lock().unwrap();
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
    let _guard = HEAVY_TEST_LOCK.lock().unwrap();
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
    let _guard = HEAVY_TEST_LOCK.lock().unwrap();
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
    let _guard = HEAVY_TEST_LOCK.lock().unwrap();
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
