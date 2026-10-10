//! Build provenance for `--version` (rubash#488).
//!
//! The GNU-compat banner (`GNU bash, version 5.3.0(1)-release (...)`) is the
//! rubash#154 persona contract and must stay byte-stable for the ecosystem;
//! the build fingerprint is appended AFTER that block so any rubash binary
//! traces to the exact commit and profile it was built from. Dev builds can
//! no longer masquerade as the installed release — the fake-baseline class
//! of incident behind rubash#488.

/// Short git hash captured by `build.rs` (`git rev-parse --short HEAD`).
/// "unknown" means git was unavailable at build time (tarball, sandboxed
/// checkout); release pipelines pin the value via `RUBASH_BUILD_HASH`.
pub fn build_hash() -> &'static str {
    match option_env!("RUBASH_BUILD_HASH") {
        Some(hash) => hash,
        None => "unknown",
    }
}

/// Build profile: `debug` while debug assertions are compiled in,
/// `release` otherwise (`cfg!(debug_assertions)` mirrors cargo's profiles).
pub fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// The lines `--version` appends after the GNU license block.
///
/// Line 1 is the build fingerprint (`build: <git-hash> (<profile>)`). On
/// Windows a second line disambiguates the rubash#154 persona: the engine is
/// a native Windows build with no MSYS runtime underneath, so agents and
/// scripts reading the banner do not mistake the `*-pc-msys` MACHTYPE for an
/// MSYS2/Cygwin bash port. Non-Windows builds already report an honest
/// native triple and stay two-line-clean (`build:` only).
pub fn version_appendix() -> String {
    let mut appendix = format!("build: {} ({})", build_hash(), build_profile());
    if cfg!(windows) {
        appendix.push_str(
            "\nnote: native Windows build without an MSYS runtime; MSYS identifiers \
             are a compatibility persona only (see docs/platform-support.md)",
        );
    }
    appendix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_hash_is_short_hex_or_unknown() {
        let hash = build_hash();
        let valid = hash == "unknown"
            || (!hash.is_empty()
                && hash.len() >= 7
                && hash.len() <= 40
                && hash
                    .chars()
                    .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()));
        assert!(
            valid,
            "build hash {hash:?} is not a short git hash or unknown"
        );
    }

    #[test]
    fn build_profile_is_debug_or_release() {
        assert!(
            build_profile() == "debug" || build_profile() == "release",
            "unexpected profile {:?}",
            build_profile()
        );
        // The profile must agree with the assertion state it is derived
        // from: unit tests run under `cargo test` (debug profile).
        if cfg!(debug_assertions) {
            assert_eq!(build_profile(), "debug");
        }
    }

    #[test]
    fn version_appendix_starts_with_the_build_fingerprint() {
        let appendix = version_appendix();
        let mut lines = appendix.lines();
        assert_eq!(
            lines.next(),
            Some(&format!("build: {} ({})", build_hash(), build_profile())[..])
        );
        assert!(lines.all(|line| !line.is_empty()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_appendix_carries_the_persona_note() {
        let appendix = version_appendix();
        let note = appendix
            .lines()
            .nth(1)
            .expect("persona note line after the build line");
        assert!(note.starts_with("note: "), "note line shape: {note:?}");
        assert!(note.contains("native Windows build"), "{note:?}");
        assert!(note.contains("without an MSYS runtime"), "{note:?}");
        assert!(note.contains("compatibility persona"), "{note:?}");
        assert!(note.contains("docs/platform-support.md"), "{note:?}");
        assert_eq!(
            appendix.lines().count(),
            2,
            "exactly build + note on Windows"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_appendix_is_the_build_line_only() {
        assert_eq!(version_appendix().lines().count(), 1);
    }
}
