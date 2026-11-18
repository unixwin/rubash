//! Build script (rubash#488): capture the git revision the binary was built
//! from so `--version` can append a `build: <hash> (<profile>)` traceability
//! line. Builds without git (source tarballs, sandboxed checkouts) report
//! "unknown".
//!
//! rubash#827, recommendation 3: a bare HEAD hash does not guarantee content
//! — the v1.4.0 incident shipped a banner naming a commit whose tree the
//! binary did not match (the build compiled HEAD plus uncommitted edits).
//! A tree with uncommitted changes therefore stamps `<rev>-dirty-<8 hex>`
//! where the digest names WHICH uncommitted state was compiled: same edits,
//! same digest; anything else changes the banner.
//!
//! The explicit `RUBASH_BUILD_HASH` pin cannot launder a dirty tree either.
//! The pin is read crate-side via `option_env!` (see build_info.rs) — it
//! wins only the revision half; this script emits the worktree's dirty
//! half under its own `RUBASH_GIT_DIRTY_SUFFIX` name, and a pinned hash
//! still gains `-dirty[-<digest>]` whenever the tree is dirty.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Emit NO `cargo:rerun-if-*` directive at all. Emitting any of them —
    // `rerun-if-changed` OR `rerun-if-env-changed` — opts this script out
    // of cargo's default whole-package rescan, and the stamp is only fresh
    // because an uncommitted edit re-runs the script and re-computes both
    // halves. The PR #939 review showed `rerun-if-env-changed` alone was
    // enough to serve a stale bare HEAD into a dirty rebuild — exactly the
    // #827 incident shape. The `RUBASH_BUILD_HASH` pin needs no hook here:
    // rustc dep-info tracks the env vars `option_env!` reads, so a changed
    // pin recompiles the crate on its own.
    //
    // The build script's cwd is the package root; CARGO_MANIFEST_DIR names
    // the same directory explicitly and keeps working if that ever changes.
    let dir = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let (stamp, dirty) = match revision_parts(&dir) {
        Some((revision, dirty)) => (format!("{revision}{dirty}"), dirty),
        None => ("unknown".to_string(), String::new()),
    };
    // The git-derived stamp goes under its OWN name, distinct from the
    // `RUBASH_BUILD_HASH` pin build_info.rs reads via `option_env!` — a
    // git stamp must never be mistaken for (or overwrite) a pipeline pin.
    println!("cargo:rustc-env=RUBASH_GIT_STAMP={stamp}");
    if !dirty.is_empty() {
        println!("cargo:rustc-env=RUBASH_GIT_DIRTY_SUFFIX={dirty}");
    }
    // GNU bash exports its own symbols to loadable builtins: configure:23147
    // (`linux*) LOCAL_LDFLAGS=-rdynamic # allow dynamic loading`). Without
    // `-rdynamic` a `#[no_mangle] extern "C"` export such as
    // `lib/glob/strmatch.c:32-39 strmatch()` lands only in the static
    // symbol table, and a loadable builtin that calls back into the shell
    // image (glob-bracket.tests' strmatch.so) dies at its first call.
    // Binaries only: test harness executables need no loadable ABI.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-rdynamic");
    }
}

/// The git-derived stamp parts for one checkout: `(short rev, suffix)`
/// where `suffix` is `-dirty[-<digest>]` or empty. `None` when git is
/// unavailable, the tree is not a repository, or it has no commits.
fn revision_parts(dir: &Path) -> Option<(String, String)> {
    let head = git(dir, &["rev-parse", "--short", "HEAD"])?;
    if !head.status.success() {
        return None;
    }
    let revision = String::from_utf8_lossy(&head.stdout).trim().to_string();
    if revision.is_empty() {
        return None;
    }
    Some((revision, dirty_suffix_for_checkout(dir)))
}

/// The stamp for one explicit checkout directory: `<short rev>` when the
/// tracked tree matches HEAD, `<short rev>-dirty-<digest>` when it does not.
/// `pub` only so the `#[path]`-included tests reach it; a build script has
/// no API surface.
pub fn revision_label_for_checkout(dir: &Path) -> Option<String> {
    let (revision, dirty) = revision_parts(dir)?;
    Some(format!("{revision}{dirty}"))
}

/// `-dirty` (plus `-<digest>` when it can be computed) for a tree whose
/// tracked worktree differs from HEAD, or an empty string for a clean
/// tree. `git describe --dirty` semantics (builtin/describe.c
/// describe_default, `--dirty` runs `diff-index --quiet HEAD`): only
/// tracked content that differs from HEAD makes a tree dirty; untracked
/// files never do. `git status --porcelain` alone also counts `??`
/// entries, so a tree whose only changes are untracked scratch files must
/// not bake a bogus `-dirty` into the version banner.
///
/// `pub` for the same reason as `revision_label_for_checkout`; emitted
/// under `RUBASH_GIT_DIRTY_SUFFIX` so a pinned `RUBASH_BUILD_HASH` still
/// gains the dirty half crate-side.
pub fn dirty_suffix_for_checkout(dir: &Path) -> String {
    if let Some(status) = git(dir, &["status", "--porcelain", "--untracked-files=no"]) {
        if status.status.success() && !String::from_utf8_lossy(&status.stdout).trim().is_empty() {
            let mut suffix = String::from("-dirty");
            if let Some(digest) = worktree_digest(dir) {
                suffix.push('-');
                suffix.push_str(&digest);
            }
            return suffix;
        }
    }
    String::new()
}

/// A short digest over the uncommitted tracked changes (`git diff HEAD`,
/// binary contents included), so a `-dirty` stamp names WHICH dirty state
/// was compiled. `git diff HEAD` covers staged and unstaged tracked edits
/// in one deterministic patch; untracked files are not part of it (matching
/// the dirty rule above). Git may be absent or fail here: the stamp then
/// stays plain `-dirty`, which is still true, just less specific.
fn worktree_digest(dir: &Path) -> Option<String> {
    let diff = git(dir, &["diff", "HEAD", "--binary"])?;
    if !diff.status.success() {
        return None;
    }
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(&diff.stdout);
    // 8 hex chars: collision-proof enough for a display marker, short
    // enough to stay readable in the banner.
    Some(
        digest
            .iter()
            .take(4)
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// `git` may be absent on a build host, and a source tarball has no `.git`
/// at all. Both are fine: the revision just stays "unknown".
fn git(dir: &Path, args: &[&str]) -> Option<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
}
