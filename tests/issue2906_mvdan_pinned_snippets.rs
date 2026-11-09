//! rubash#2906 follow-through: the 16 mvdan/sh snippets pinned under
//! `tests/fixtures/ecosuite/mvdan-n/` were harvested for the parse-level
//! divergences of rubash#220/#221/#222, but no test referenced them
//! (verified 2026-11-08: the only `mvdan` hits in the repo were docs and
//! the fixture MANIFEST). The eco-2906 chain re-ran the full 559-snippet
//! execution differential (two full runs, identical exec verdicts) and
//! confirmed all 16 pins parse- AND exec-match GNU bash 5.3.20 again.
//! This stem wires the 16 pins into the test surface so the family
//! cannot silently regress: for each snippet, rubash must reproduce
//! the GNU parse rc (`-n`), the GNU exec rc, and the GNU stdout bytes.
//!
//! stderr is deliberately NOT pinned: the exec judgment of the eco-2906
//! chain is stdout bytes + rc, with stderr advisory (it carries wording
//! diffs such as t0382's, plus ordering/timing artifacts — see #3033).
//!
//! Boundary with rubash#3033: this stem pins only the 16 harvested
//! snippets. Whether the full 559-snippet differential chain is wired
//! into CI as a job or kept as a periodic eco-lane run is the decision
//! recorded in #3033; nothing here pre-empts it.
//!
//! Expected values below were re-captured from the GNU bash 5.3.20
//! oracle (~/workspace/rubash-audit build) on 2026-11-08, script-file
//! form, `env -i PATH=/usr/bin:/bin`, fresh scratch cwd per snippet;
//! the parse-rc column also matches tests/fixtures/ecosuite/MANIFEST.md.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// (fixture file, GNU `-n` rc, GNU exec rc). GNU stdout was empty for
/// all 16 at capture time; the exec assertion compares bytes, so a
/// future non-empty GNU shape would need a table extension, not a
/// silent pass.
const PINS: &[(&str, i32, i32)] = &[
    ("t0026.sh", 0, 1),
    ("t0068.sh", 2, 2),
    ("t0142.sh", 2, 2),
    ("t0143.sh", 2, 2),
    ("t0144.sh", 2, 2),
    ("t0164.sh", 0, 127),
    ("t0169.sh", 2, 2),
    ("t0173.sh", 2, 2),
    ("t0286.sh", 0, 0),
    ("t0382.sh", 2, 2),
    ("t0383.sh", 0, 0),
    ("t0411.sh", 0, 1),
    ("t0415.sh", 2, 2),
    ("t0459.sh", 1, 1),
    ("t0525.sh", 2, 2),
    ("t0526.sh", 2, 2),
];

fn minimal_path() -> String {
    if cfg!(windows) {
        let mut path = r"C:\Windows\System32".to_string();
        let git_usr_bin = r"C:\Program Files\Git\usr\bin";
        if PathBuf::from(git_usr_bin).is_dir() {
            path.push(';');
            path.push_str(git_usr_bin);
        }
        path
    } else {
        "/usr/bin:/bin".to_string()
    }
}

struct Outcome {
    stdout: Vec<u8>,
    code: Option<i32>,
    timed_out: bool,
}

fn run_rubash(dir: &std::path::Path, args: &[&str], limit: Duration) -> Outcome {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", minimal_path())
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let mut out_pipe = child.stdout.take().expect("stdout pipe");
    let drain = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    // stderr is advisory only; drain it so a chatty snippet cannot
    // block on a full pipe.
    let mut err_pipe = child.stderr.take().expect("stderr pipe");
    let drain_err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if start.elapsed() > limit {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Outcome {
                        stdout: drain.join().unwrap_or_default(),
                        code: None,
                        timed_out: true,
                    };
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break None,
        }
    };
    let _ = drain_err.join();
    Outcome {
        stdout: drain.join().unwrap_or_default(),
        code,
        timed_out: false,
    }
}

#[test]
fn issue2906_mvdan_pinned_snippets_parse_and_exec_match_gnu() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = root.join("tests/fixtures/ecosuite/mvdan-n");
    for (file, gnu_parse_rc, gnu_exec_rc) in PINS {
        let scratch = std::env::temp_dir().join(format!(
            "rubash-issue2906-{}-{}",
            file.trim_end_matches(".sh"),
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("create scratch");
        std::fs::copy(fixture_dir.join(file), scratch.join(file)).expect("copy snippet");

        let parse = run_rubash(&scratch, &["-n", file], Duration::from_secs(30));
        assert!(!parse.timed_out, "{file}: parse run timed out");
        assert_eq!(
            parse.code,
            Some(*gnu_parse_rc),
            "{file}: parse rc differs from GNU {gnu_parse_rc}"
        );

        let exec = run_rubash(&scratch, &[file], Duration::from_secs(30));
        assert!(!exec.timed_out, "{file}: exec run timed out");
        assert_eq!(
            exec.code,
            Some(*gnu_exec_rc),
            "{file}: exec rc differs from GNU {gnu_exec_rc}"
        );
        assert!(
            exec.stdout.is_empty(),
            "{file}: exec stdout differs from GNU (empty at capture): {:?}",
            String::from_utf8_lossy(&exec.stdout)
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
