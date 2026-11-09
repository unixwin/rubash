//! rubash#2907: the real git contrib chain, end to end.
//!
//! git's own contrib scripts (git-completion.bash, git-prompt.sh) are
//! GPL and are NOT vendored in this repo (see
//! tests/regression/corpus/PROVENANCE.txt); the self-authored smoke in
//! tests/regression/fixtures/eco-git-completion-smoke.sh pins only the
//! helper protocol the real script is built on. This pin runs the REAL
//! scripts' live entry points against real repositories:
//!
//! - Chain A sources the real git-completion.bash and drives the
//!   registered live entry `__git_wrap__git_main` (what readline calls
//!   on TAB; git 2.43 has no `_git` symbol) with bound
//!   COMP_WORDS/COMP_CWORD/COMP_LINE/COMP_POINT, plus one direct
//!   `__git_main` call. Each case's repository state feeds the next
//!   assertion: the COMPREPLY bytes are the assertion.
//! - Chain B sources the real git-prompt.sh and renders `__git_ps1`
//!   across eight repository states (clean / dirty / staged /
//!   untracked / all+stash / detached / ahead / no-repo), including
//!   the no-arg print form and the 2-arg pcmode PS1-mutating form.
//!
//! Both chains run under this build's rubash AND under a GNU bash
//! oracle on the host, over the SAME deterministically built
//! repositories (tests/fixtures/ecosuite/git-contrib/make-repos.sh),
//! and the captured stdout must match byte for byte; stderr must match
//! after replacing the two sourced scripts' host paths with their
//! basenames (an error inside a sourced function names its definition
//! file; the spelling of that name is a host-path fact, not a semantic
//! one — the filename and line it resolves to are asserted exactly).
//!
//! Skip discipline mirrors the nvm timing gate in tests/regression.rs:
//! the scripts, the oracle, and git itself are discovered on the host
//! (RUBASH_GIT_CONTRIB_DIR / RUBASH_GNU_BASH pin explicit locations);
//! a host without them skips instead of failing on a stub. On a host
//! that has them, a divergence is a hard failure — that is the point
//! of the pin: eco-2907's two findings (the assignment-carrier leak
//! behind the detached-HEAD prompt, #3034; the sourced-file error
//! filename, #3035) both surface here as single differing lines.
//!
//! A second pin re-checks the 16 mvdan/sh parse fixtures
//! (tests/fixtures/ecosuite/mvdan-n/, see MANIFEST.md): `-n` exit
//! codes must agree with the oracle per file. Wiring the full
//! 559-snippet execution differential into CI is #3033's decision,
//! not this pin's.

mod common;

use common::{wait_bounded, BoundedOutcome};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const CHAIN_TIMEOUT: Duration = Duration::from_secs(300);

fn rubash_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// GNU bash oracle discovery, same shape as tests/regression.rs:
/// RUBASH_GNU_BASH pins an explicit path; otherwise the first `bash`
/// on PATH that is not this build's rubash.
fn gnu_bash_oracle() -> Option<PathBuf> {
    if let Ok(pinned) = std::env::var("RUBASH_GNU_BASH") {
        let candidate = PathBuf::from(pinned);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let own = rubash_bin();
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(if cfg!(windows) { "bash.exe" } else { "bash" });
        if candidate.is_file() && candidate != own {
            return Some(candidate);
        }
    }
    None
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// True when `file` reads as the real git contrib script (contains its
/// entry point), not a stub or an unrelated file of the same name.
fn script_looks_real(file: &Path, entry: &str) -> bool {
    std::fs::read_to_string(file)
        .map(|text| text.contains(entry))
        .unwrap_or(false)
}

/// Locate the real (git-completion.bash, git-prompt.sh) pair.
/// RUBASH_GIT_CONTRIB_DIR names a directory holding both files under
/// their contrib names; otherwise fall back to the pair the host's
/// git installation ships (bash-completion's `git` file IS
/// git-completion.bash; git-sh-prompt in git's exec-path IS
/// git-prompt.sh).
fn git_contrib_scripts(git: &Path) -> Option<(PathBuf, PathBuf)> {
    if let Ok(dir) = std::env::var("RUBASH_GIT_CONTRIB_DIR") {
        let dir = PathBuf::from(dir);
        let comp = dir.join("git-completion.bash");
        let prompt = dir.join("git-prompt.sh");
        if script_looks_real(&comp, "__git_main") && script_looks_real(&prompt, "__git_ps1") {
            return Some((comp, prompt));
        }
        return None;
    }
    let comp = PathBuf::from("/usr/share/bash-completion/completions/git");
    if !script_looks_real(&comp, "__git_main") {
        return None;
    }
    let mut cmd = Command::new(git);
    cmd.arg("--exec-path")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let out = cmd.output().ok()?;
    let exec_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let prompt = PathBuf::from(exec_path).join("git-sh-prompt");
    if script_looks_real(&prompt, "__git_ps1") {
        return Some((comp, prompt));
    }
    None
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("ecosuite")
}

fn run_bounded(mut cmd: Command, limit: Duration) -> BoundedOutcome {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn child for issue2907 chain");
    wait_bounded(&mut child, limit)
}

/// Assert a repository is in the state the chain expects it to feed
/// from; a state that drifted would silently vacate the case built on
/// it, so the states are asserted before any shell runs.
fn assert_repo_states(git: &Path, repos: &Path) {
    let porcelain = |sub: &str| -> String {
        let mut cmd = Command::new(git);
        cmd.args([
            "-C",
            repos.join(sub).to_str().unwrap(),
            "status",
            "--porcelain",
        ]);
        cmd.env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", repos.join("home"));
        let out = run_bounded(cmd, Duration::from_secs(60));
        assert_eq!(out.code, Some(0), "git status failed for {sub}");
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    assert_eq!(porcelain("clean"), "", "clean repo must be clean");
    assert_eq!(porcelain("dirty"), " M README.md\n", "dirty repo state");
    assert_eq!(porcelain("staged"), "A  staged.txt\n", "staged repo state");
    assert_eq!(
        porcelain("untracked"),
        "?? untracked.txt\n",
        "untracked repo state"
    );
    assert_eq!(
        porcelain("all"),
        " M README.md\nA  staged.txt\n?? untracked.txt\n",
        "all repo state"
    );
    // Detached HEAD: symbolic-ref must fail, and HEAD must sit on the
    // doubly-tagged initial commit so `git describe --exact-match`
    // resolves a tag name (the B6 cell's input).
    let mut cmd = Command::new(git);
    cmd.args([
        "-C",
        repos.join("detached").to_str().unwrap(),
        "symbolic-ref",
        "-q",
        "HEAD",
    ]);
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", repos.join("home"));
    let out = run_bounded(cmd, Duration::from_secs(60));
    assert_eq!(out.code, Some(1), "detached repo must have no symref HEAD");
}

fn run_driver(
    shell: &Path,
    scratch: &Path,
    repos: &Path,
    comp: &Path,
    prompt: &Path,
) -> BoundedOutcome {
    let driver = fixtures_dir().join("git-contrib").join("driver.sh");
    let mut cmd = Command::new(shell);
    cmd.args(["--noprofile", "--norc"])
        .arg(&driver)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", repos.join("home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("LC_ALL", "C.UTF-8")
        .env("REPOS", repos)
        .env("GITCOMP", comp)
        .env("GITPROMPT", prompt)
        .current_dir(scratch);
    run_bounded(cmd, CHAIN_TIMEOUT)
}

/// Replace each sourced script's host path with its basename, so the
/// stderr comparison asserts WHICH file an error names without
/// depending on where the host keeps that file.
fn normalize_script_paths(bytes: &[u8], comp: &Path, prompt: &Path) -> Vec<u8> {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    for script in [comp, prompt] {
        let full = script.to_string_lossy().into_owned();
        let base = script
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        text = text.replace(&full, &base);
    }
    text.into_bytes()
}

fn first_diff(a: &[u8], b: &[u8]) -> String {
    let a_lines: Vec<&[u8]> = a.split(|c| *c == b'\n').collect();
    let b_lines: Vec<&[u8]> = b.split(|c| *c == b'\n').collect();
    for i in 0..a_lines.len().max(b_lines.len()) {
        let x = a_lines.get(i).copied().unwrap_or(b"");
        let y = b_lines.get(i).copied().unwrap_or(b"");
        if x != y {
            return format!(
                "first difference at line {}:\n  gnu:   {:?}\n  rubash: {:?}",
                i + 1,
                String::from_utf8_lossy(x),
                String::from_utf8_lossy(y)
            );
        }
    }
    format!(
        "outputs differ only in length (gnu {} bytes, rubash {} bytes)",
        a.len(),
        b.len()
    )
}

#[test]
fn issue2907_git_contrib_chain_matches_gnu() {
    let Some(oracle) = gnu_bash_oracle() else {
        eprintln!("issue2907: no GNU bash oracle found; skipped");
        return;
    };
    let Some(git) = on_path("git") else {
        eprintln!("issue2907: no git on PATH; skipped");
        return;
    };
    let Some((comp, prompt)) = git_contrib_scripts(&git) else {
        eprintln!(
            "issue2907: real git-completion.bash / git-prompt.sh not found \
             (set RUBASH_GIT_CONTRIB_DIR); skipped"
        );
        return;
    };

    let scratch = std::env::temp_dir().join(format!("rubash-issue2907-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("create issue2907 scratch dir");
    let repos = scratch.join("repos");
    std::fs::create_dir_all(&repos).expect("create repos root");

    // Build the deterministic repositories with the oracle shell (the
    // builder is setup, not the surface under test).
    let make_repos = fixtures_dir().join("git-contrib").join("make-repos.sh");
    let mut cmd = Command::new(&oracle);
    cmd.arg(&make_repos)
        .arg(&repos)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C.UTF-8")
        .current_dir(&scratch);
    let built = run_bounded(cmd, CHAIN_TIMEOUT);
    assert_eq!(
        built.code,
        Some(0),
        "make-repos.sh failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert_repo_states(&git, &repos);

    let gnu = run_driver(&oracle, &scratch, &repos, &comp, &prompt);
    let rub = run_driver(&rubash_bin(), &scratch, &repos, &comp, &prompt);
    let _ = std::fs::remove_dir_all(&scratch);

    assert_eq!(
        gnu.code,
        Some(0),
        "GNU driver run failed: {}",
        String::from_utf8_lossy(&gnu.stderr)
    );
    assert_eq!(
        rub.code,
        Some(0),
        "rubash driver run failed: {}",
        String::from_utf8_lossy(&rub.stderr)
    );
    let gnu_out = String::from_utf8_lossy(&gnu.stdout).into_owned();
    // The chain must have actually run end to end on the GNU side
    // before any comparison means anything: every case header and the
    // terminal marker present.
    for marker in [
        "CASE A1-subcmd",
        "CASE A11-diffopt",
        "CASE A15-direct-__git_main",
        "PROMPT B1-clean",
        "PROMPT B6-detached",
        "PROMPT B12",
        "DRIVER DONE",
    ] {
        assert!(
            gnu_out.contains(marker),
            "GNU chain output is missing {marker}; chain did not run end to end"
        );
    }
    assert_eq!(
        rub.stdout,
        gnu.stdout,
        "issue2907 chain stdout diverges from GNU bash:\n{}",
        first_diff(&gnu.stdout, &rub.stdout)
    );
    let gnu_err = normalize_script_paths(&gnu.stderr, &comp, &prompt);
    let rub_err = normalize_script_paths(&rub.stderr, &comp, &prompt);
    assert_eq!(
        rub_err,
        gnu_err,
        "issue2907 chain stderr diverges from GNU bash:\n{}",
        first_diff(&gnu_err, &rub_err)
    );
}

#[test]
fn issue2907_mvdan_parse_pins_match_gnu() {
    let Some(oracle) = gnu_bash_oracle() else {
        eprintln!("issue2907: no GNU bash oracle found; mvdan re-check skipped");
        return;
    };
    let dir = fixtures_dir().join("mvdan-n");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read mvdan-n fixtures dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "sh").unwrap_or(false))
        .collect();
    files.sort();
    assert_eq!(
        files.len(),
        16,
        "MANIFEST.md pins exactly 16 mvdan-n fixtures; found {}",
        files.len()
    );
    let rub = rubash_bin();
    let mut disagreements = Vec::new();
    for file in &files {
        let rc_of = |shell: &Path| -> Option<i32> {
            let mut cmd = Command::new(shell);
            cmd.arg("-n")
                .arg(file)
                .env_clear()
                .env("PATH", "/usr/bin:/bin");
            run_bounded(cmd, Duration::from_secs(60)).code
        };
        let gnu_rc = rc_of(&oracle);
        let rub_rc = rc_of(&rub);
        if gnu_rc != rub_rc {
            disagreements.push(format!(
                "{}: gnu rc={gnu_rc:?} rubash rc={rub_rc:?}",
                file.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "mvdan-n parse pins disagree with GNU bash:\n{}",
        disagreements.join("\n")
    );
}
