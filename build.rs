//! Build script (rubash#488): capture the git revision the binary was built
//! from so `--version` can append a `build: <hash> (<profile>)` traceability
//! line. Builds without git (source tarballs, sandboxed checkouts) report
//! "unknown"; release pipelines can pin the value explicitly through the
//! `RUBASH_BUILD_HASH` environment variable.

use std::env;
use std::process::Command;

fn main() {
    // No `cargo:rerun-if-changed` directives on purpose: cargo's default
    // invalidation (rerun the build script when any package file changes)
    // is exactly the freshness contract a commit hash wants — every real
    // commit touches a tracked source file, so the next build recaptures
    // HEAD. Only the explicit override needs its own invalidation hook.
    println!("cargo:rerun-if-env-changed=RUBASH_BUILD_HASH");
    println!("cargo:rustc-env=RUBASH_BUILD_HASH={}", build_hash());
}

/// The packaged hash: an explicit `RUBASH_BUILD_HASH` override wins (release
/// pipelines pin the commit they package), otherwise `git rev-parse --short
/// HEAD` is captured at build time, falling back to "unknown" when git is
/// unavailable, the tree is not a repository, or it has no commits.
fn build_hash() -> String {
    if let Ok(hash) = env::var("RUBASH_BUILD_HASH") {
        let hash = hash.trim();
        if !hash.is_empty() {
            return hash.to_string();
        }
    }
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|hash| !hash.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}
