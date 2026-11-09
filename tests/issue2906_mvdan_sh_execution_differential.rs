//! rubash #2906 — mvdan/sh 559-snippet execution differential, pinned in-repo.
//!
//! The eco-2906 lane ran the full differential out-of-repo (extractor +
//! chain driver in the ecosuite workspace) against GNU bash 5.3.20: 560
//! snippets (559 numbered + the tSKIP appendix) at execution depth, two
//! full runs with an identical verdict set. This test pins that result so
//! the differential cannot silently rot:
//!
//! - `tests/fixtures/ecosuite/mvdan-sh/` holds the whole corpus
//!   (t0000..t0558 + tSKIP.sh, extracted from mvdan/sh@aebdf2b8 by
//!   `scripts/mvdan-sh-extract.py`), a `MANIFEST.tsv`, the pinned GNU
//!   oracle ledger (`gnu-ledger.tsv`) and the tracked divergence ledger
//!   (`known-divergences.tsv`).
//! - Every snippet is executed under the rubash binary under test with
//!   the chain's isolation: fresh empty cwd+HOME, env = {PATH, HOME}
//!   only, stdin /dev/null, 5 s timeout. Parse level (`-n` rc) and exec
//!   level (stdout bytes + rc) are compared. stderr is NOT compared
//!   (advisory axis; wording divergences are filed separately).
//! - Outputs are path-normalized before comparison (shell binary path,
//!   snippet path, scratch/HOME path) — the eco chain's "path artifact"
//!   exclusion — so the ledgers are checkout-location independent. The
//!   shell's own PPID in `typeset`/`declare` dumps is likewise masked to
//!   `<PPID>`: it names the process tree the test happens to run under,
//!   not shell behaviour, so those ledger rows are parent independent.
//! - A snippet listed in `known-divergences.tsv` must reproduce its
//!   recorded divergent output exactly (and stay divergent from GNU).
//!   When a fix for the tracked issue lands, that cell turns red here:
//!   move the snippet out of `known-divergences.tsv` in the same change.
//!   A snippet NOT listed must match the GNU ledger exactly; any new
//!   divergence turns the cell red and needs a filed issue + ledger row.
//!
//! CI wiring of this differential (a dedicated job vs a periodic eco
//! chain) is #3033's decision, deliberately not made here; this pin runs
//! wherever the full integration-test set runs:
//!   cargo test --test issue2906_mvdan_sh_execution_differential
//!
//! Ledger regeneration: re-run the GNU half and the rubash half of the
//! eco chain (ecosuite `scripts/run-mvdan-sh-chain.py` isolation) and
//! re-derive the two TSVs; see the fixture README.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(5);
const SHARDS: usize = 8;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ecosuite/mvdan-sh")
}

fn legacy_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ecosuite/mvdan-n")
}

fn rubash_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rubash")
}

/// One ledger row: parse rc, exec expectation, expected stdout (hex).
#[derive(Debug, Clone, PartialEq)]
enum ExecExpect {
    Rc(i32),
    Timeout,
}

#[derive(Debug, Clone)]
struct Expect {
    parse_rc: i32,
    exec: ExecExpect,
    stdout_hex: String,
}

fn parse_exec_rc(s: &str) -> ExecExpect {
    if s == "TIMEOUT" {
        ExecExpect::Timeout
    } else {
        ExecExpect::Rc(s.parse().expect("ledger rc"))
    }
}

fn read_ledger(path: &Path, rc_cols: (usize, usize), hex_col: usize) -> BTreeMap<String, Expect> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut map = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        map.insert(
            f[0].to_string(),
            Expect {
                parse_rc: f[rc_cols.0].parse().expect("ledger parse rc"),
                exec: parse_exec_rc(f[rc_cols.1]),
                stdout_hex: f.get(hex_col).copied().unwrap_or("").to_string(),
            },
        );
    }
    map
}

fn gnu_ledger() -> BTreeMap<String, Expect> {
    read_ledger(&corpus_dir().join("gnu-ledger.tsv"), (1, 2), 3)
}

fn known_divergences() -> BTreeMap<String, (String, Expect)> {
    let path = corpus_dir().join("known-divergences.tsv");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut map = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        map.insert(
            f[0].to_string(),
            (
                f[1].to_string(),
                Expect {
                    parse_rc: f[2].parse().expect("ledger parse rc"),
                    exec: parse_exec_rc(f[3]),
                    stdout_hex: f.get(4).copied().unwrap_or("").to_string(),
                },
            ),
        );
    }
    map
}

fn snippet_ids() -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(corpus_dir())
        .expect("read corpus dir")
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            if name.starts_with('t') && name.ends_with(".sh") {
                Some(name[1..name.len() - 3].to_string())
            } else {
                None
            }
        })
        .collect();
    ids.sort();
    ids
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn wait_timeout(child: &mut Child) -> Option<i32> {
    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => return status.code(),
            None => {
                if start.elapsed() >= TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// Run one snippet once (parse-only or exec) under the chain's isolation.
/// Returns (rc, timed_out, stdout bytes). Scratch is recreated per run,
/// exactly like the chain driver.
fn run_once(bin: &str, script: &Path, work: &Path, parse_only: bool) -> (i32, bool, Vec<u8>) {
    let scratch = work.join("scratch");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).expect("create scratch");
    let stdout_path = work.join("stdout.bin");
    let stdout_file = fs::File::create(&stdout_path).expect("create stdout capture");
    let mut cmd = Command::new(bin);
    if parse_only {
        cmd.arg("-n");
    }
    cmd.arg(script)
        .current_dir(&scratch)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &scratch)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(if parse_only {
            Stdio::null()
        } else {
            Stdio::from(stdout_file)
        });
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    match wait_timeout(&mut child) {
        Some(rc) => {
            let out = if parse_only {
                Vec::new()
            } else {
                fs::read(&stdout_path).unwrap_or_default()
            };
            (rc, false, out)
        }
        None => (124, true, fs::read(&stdout_path).unwrap_or_default()),
    }
}

fn normalize(data: &[u8], bin: &str, script: &Path, work: &Path) -> Vec<u8> {
    let scratch = work.join("scratch");
    let mut out = data.to_vec();
    for (pat, rep) in [
        (bin.as_bytes(), b"<SHELL>".as_slice()),
        (script.to_string_lossy().as_bytes(), b"<SCRIPT>".as_slice()),
        (scratch.to_string_lossy().as_bytes(), b"<HOME>".as_slice()),
    ] {
        if pat.is_empty() {
            continue;
        }
        let mut res = Vec::with_capacity(out.len());
        let mut i = 0;
        while i < out.len() {
            if out[i..].starts_with(pat) {
                res.extend_from_slice(rep);
                i += pat.len();
            } else {
                res.push(out[i]);
                i += 1;
            }
        }
        out = res;
    }
    mask_ppid(&out)
}

/// `typeset` / `declare` dumps print the shell's own PPID, which is fixed
/// by the process tree the test runs under, not by shell behaviour. Mask
/// the digit run in both dump shapes — bare `PPID=82765` and quoted
/// `declare -ir PPID="82765"` — replacing it with `<PPID>` so the ledger
/// rows for those snippets reproduce under any parent process.
fn mask_ppid(data: &[u8]) -> Vec<u8> {
    const KEY: &[u8] = b"PPID=";
    let mut res: Vec<u8> = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        // The key must start a variable name: do not mask a longer name
        // that merely ends in `PPID` (e.g. a user variable `XPPID=`).
        let boundary = match res.last() {
            None => true,
            Some(&b) => !(b.is_ascii_alphanumeric() || b == b'_'),
        };
        if boundary && data[i..].starts_with(KEY) {
            let mut j = i + KEY.len();
            let quoted = data.get(j) == Some(&b'"');
            if quoted {
                j += 1;
            }
            let digits_start = j;
            while j < data.len() && data[j].is_ascii_digit() {
                j += 1;
            }
            if j > digits_start && (!quoted || data.get(j) == Some(&b'"')) {
                res.extend_from_slice(KEY);
                if quoted {
                    res.push(b'"');
                }
                res.extend_from_slice(b"<PPID>");
                if quoted {
                    res.push(b'"');
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        res.push(data[i]);
        i += 1;
    }
    res
}

/// Check one snippet against its expectation; Err describes the mismatch.
fn check_snippet(id: &str, expect: &Expect, work_root: &Path) -> Result<(), String> {
    let bin = rubash_bin();
    let script = corpus_dir().join(format!("t{id}.sh"));
    let work = work_root.join(format!("case-{id}"));
    fs::create_dir_all(&work).expect("create work dir");
    let (parse_rc, parse_to, _) = run_once(bin, &script, &work, true);
    // A timeout here is never the expectation (parse has no TIMEOUT
    // cells); under heavy machine load a debug-binary spawn can exceed
    // the limit spuriously, so retry a timed-out run once before
    // judging it. A completed run is never retried: rc/stdout
    // mismatches are deterministic and must stand.
    let (parse_rc, parse_to, _) = if parse_to {
        run_once(bin, &script, &work, true)
    } else {
        (parse_rc, parse_to, Vec::new())
    };
    if parse_to || parse_rc != expect.parse_rc {
        return Err(format!(
            "t{id}: parse rc {parse_rc} (timeout={parse_to}), expected {}",
            expect.parse_rc
        ));
    }
    let (exec_rc, exec_to, raw_out) = run_once(bin, &script, &work, false);
    let (exec_rc, exec_to, raw_out) = if exec_to && !matches!(expect.exec, ExecExpect::Timeout) {
        run_once(bin, &script, &work, false)
    } else {
        (exec_rc, exec_to, raw_out)
    };
    match &expect.exec {
        ExecExpect::Timeout => {
            if !exec_to {
                return Err(format!(
                    "t{id}: expected TIMEOUT (still running at {TIMEOUT:?}), exited rc {exec_rc}"
                ));
            }
        }
        ExecExpect::Rc(want) => {
            if exec_to {
                return Err(format!("t{id}: timed out, expected rc {want}"));
            }
            if exec_rc != *want {
                return Err(format!("t{id}: exec rc {exec_rc}, expected {want}"));
            }
            let got = hex_encode(&normalize(&raw_out, bin, &script, &work));
            if got != expect.stdout_hex {
                return Err(format!(
                    "t{id}: stdout mismatch\n  expected(hex): {}\n  actual(hex):   {}",
                    expect.stdout_hex, got
                ));
            }
        }
    }
    Ok(())
}

fn run_shard(shard: usize) {
    let gnu = gnu_ledger();
    let known = known_divergences();
    let ids = snippet_ids();
    let work_root = std::env::temp_dir().join(format!(
        "rubash-issue2906-{}-shard{shard}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&work_root);
    fs::create_dir_all(&work_root).expect("create shard work root");
    let mut failures: Vec<String> = Vec::new();
    for (idx, id) in ids.iter().enumerate() {
        if idx % SHARDS != shard {
            continue;
        }
        if let Some((issue, expect)) = known.get(id) {
            // A tracked divergence must stay exactly as recorded AND stay
            // divergent from the GNU ledger; either change means the
            // tracking issue moved and the ledger must be updated.
            if let Err(e) = check_snippet(id, expect, &work_root) {
                failures.push(format!("{e} [tracked divergence, issue #{issue}]"));
            } else if let Some(g) = gnu.get(id) {
                if g.parse_rc == expect.parse_rc
                    && g.exec == expect.exec
                    && g.stdout_hex == expect.stdout_hex
                {
                    failures.push(format!(
                        "t{id}: known-divergences row is identical to the GNU ledger (stale row, issue #{issue})"
                    ));
                }
            }
        } else {
            let expect = gnu
                .get(id)
                .unwrap_or_else(|| panic!("t{id}: missing GNU ledger row"));
            if let Err(e) = check_snippet(id, expect, &work_root) {
                failures.push(format!("{e} [expected GNU parity]"));
            }
        }
    }
    let _ = fs::remove_dir_all(&work_root);
    assert!(
        failures.is_empty(),
        "shard {shard}: {} snippet mismatch(es):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn corpus_is_complete_and_matches_manifest() {
    let ids = snippet_ids();
    assert_eq!(ids.len(), 560, "corpus must hold 559 numbered + tSKIP");
    let manifest = fs::read_to_string(corpus_dir().join("MANIFEST.tsv")).expect("read MANIFEST");
    let rows: Vec<&str> = manifest.lines().skip(1).filter(|l| !l.is_empty()).collect();
    assert_eq!(
        rows.len(),
        559,
        "MANIFEST must ledger all 559 numbered snippets"
    );
    for (i, row) in rows.iter().enumerate() {
        let num = row.split('\t').next().unwrap();
        assert_eq!(num, format!("{i:04}"), "MANIFEST row {i} out of order");
        assert!(
            ids.binary_search(&num.to_string()).is_ok(),
            "MANIFEST snippet t{num} missing from corpus"
        );
    }
    assert!(ids.iter().any(|i| i == "SKIP"), "tSKIP appendix missing");
    assert_eq!(
        gnu_ledger().len(),
        560,
        "GNU ledger must cover every snippet"
    );
}

#[test]
fn legacy_mvdan_n_fixtures_byte_identical_to_corpus() {
    // The extractor's validation gate: the 16 historically pinned fixtures
    // must match the extracted corpus byte-for-byte.
    let mut names: Vec<String> = fs::read_dir(legacy_dir())
        .expect("read legacy fixtures")
        .filter_map(|e| {
            let n = e.ok()?.file_name().into_string().ok()?;
            n.ends_with(".sh").then_some(n)
        })
        .collect();
    names.sort();
    assert_eq!(names.len(), 16, "16 historically pinned fixtures");
    for name in names {
        let legacy = fs::read(legacy_dir().join(&name)).expect("read legacy fixture");
        let corpus = fs::read(corpus_dir().join(&name)).expect("read corpus snippet");
        assert_eq!(legacy, corpus, "{name}: legacy fixture != extracted corpus");
    }
}

#[test]
fn differential_shard_0() {
    run_shard(0);
}
#[test]
fn differential_shard_1() {
    run_shard(1);
}
#[test]
fn differential_shard_2() {
    run_shard(2);
}
#[test]
fn differential_shard_3() {
    run_shard(3);
}
#[test]
fn differential_shard_4() {
    run_shard(4);
}
#[test]
fn differential_shard_5() {
    run_shard(5);
}
#[test]
fn differential_shard_6() {
    run_shard(6);
}
#[test]
fn differential_shard_7() {
    run_shard(7);
}
