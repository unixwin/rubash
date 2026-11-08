// Dead-code warnings for the `#[path]`-included registry file are
// silenced crate-wide here: the registry's helper fns/consts serve the
// library, and only a subset is used by this test crate. The library's
// own dead-code detection is unaffected (separate crate).
#![allow(dead_code)]

//! Registry-driven typed-data cells (rubash#2885 E6) — integration test.
//!
//! Every codepoint in the `MARKERS` registry is typed as user data
//! (`v=$'…'`) and must round-trip byte-for-byte through the three core
//! shapes. The registry table is compiled into this crate directly
//! (`#[path]` below), so a newly registered marker gets cells
//! automatically — the same table the library uses, not a copy and not
//! a new public export: `MARKERS` stays `pub(crate)` and the ledger is
//! untouched. Precedent for including a source file into a test crate:
//! `tests/build_rs_dirty.rs` (`#[path = "../build.rs"]`).
//!
//! Why this lives in `tests/` and not in `src/executor/markers.rs`:
//! the cells drive the *built* binary, and CI runs `cargo test --lib`,
//! which does not build the bin target — the old lib test's
//! `CARGO_BIN_EXE_rubash` / `current_exe` fallback then hit its
//! `.expect` (CI head 4eeb49dd, three legs red on the same panic).
//! Integration tests get `CARGO_BIN_EXE_rubash` from cargo, like every
//! other binary-driving test in this repo; `cargo test --lib` never
//! builds this file at all.
//!
//! Side effect of the `#[path]` include: the registry file's own
//! `#[cfg(test)]` module compiles here too, so its self-contained
//! tests (the env-namespace census, the boundary-claims test, the
//! carrier-const coverage tests, ...) also run in this binary
//! alongside the registry test below. Same assertions, same pass —
//! they are counted once in the lib suite and once here. The two
//! decoder-coupled tests (`user_reachable_rows_have_an_entry_reencode_story`,
//! `boundary_claims_name_a_decoder`) live in
//! `src/executor/markers_decoder_tests.rs` instead: they call the real
//! decoders through crate paths, which do not exist in this crate.
//!
//! The one-word visibility change this include needed —
//! `contains_word_marker_bytes`: `pub(in crate::executor)` to
//! `pub(crate)` — is crate-internal only; the `crate::executor` path
//! does not exist in this crate.

#[path = "../src/executor/markers.rs"]
mod markers;

use markers::MARKERS;

/// rubash#2885 E6 — the registry-driven typed-data cells. Every
/// codepoint in the table is typed as user data (`v=$'…'`) and must
/// round-trip byte-for-byte through the three core shapes — the
/// sweep battery collected the same cells from GNU bash 5.3.20
/// (52/54 codepoints GNU-identical on the #2885 base). Because the
/// cells iterate `MARKERS`, a newly registered marker gets coverage
/// automatically; the per-CHANNEL dimension lives in
/// `tests/marker_leak_golden.rs` (+ its PTY sibling).
///
/// Known-broken cells are pinned in `TYPED_DATA_EXCEPTIONS` with
/// their tracking issue and asserted to still differ — when the fix
/// lands, this test goes red and the exception must be removed.
#[test]
fn registry_typed_data_roundtrips_core_shapes() {
    use std::collections::BTreeSet;
    use std::process::{Command, Stdio};

    /// (codepoint, shape) cells known to diverge from GNU, with the
    /// issue that owns the fix. Shape ids: 1 = `printf '%s' "$v"`,
    /// 2 = `echo "$v"`, 3 = `w="a${v}b"` (assignment-concat).
    const TYPED_DATA_EXCEPTIONS: &[(u32, u8, &str)] = &[
        // (empty: the #2882 E002 cells and the #3016 E316 shape-3 cell
        // round-trip since those fixes merged; exceptions are added
        // here only with a live tracking issue.)
    ];

    // Cargo sets CARGO_BIN_EXE_<name> for integration tests when the
    // package has a binary target; `cargo test --lib` never builds
    // this file at all, so this cannot fail for a missing binary the
    // way the old lib-test fallback did (see the file header).
    let bin = env!("CARGO_BIN_EXE_rubash");

    let mut codes: BTreeSet<u32> = BTreeSet::new();
    for info in MARKERS {
        let cp = info.code as u32;
        if cp == 0 {
            // NUL sentinels (CASE_PATTERN_*): a NUL cannot be
            // typed into a shell string in GNU either — there is
            // no typed-data shape to gate.
            continue;
        }
        if info.name == "QUOTED_HEREDOC_MARKER" {
            // String marker: its `code` is the '_' of the named
            // string "__RUBASH_HD1__"; the golden's named-string
            // leak scan covers it.
            continue;
        }
        codes.insert(cp);
    }
    assert!(
        codes.len() >= 40,
        "registry scan must see the marker population, saw {} codepoints",
        codes.len()
    );
    for &(cp, _, _) in TYPED_DATA_EXCEPTIONS {
        assert!(
            codes.contains(&cp),
            "exception table names U+{cp:04X}, which is not a registry codepoint"
        );
    }

    for cp in codes {
        let ch = char::from_u32(cp).expect("registry code is a char");
        let escape = if cp < 0x100 {
            format!("\\x{cp:02x}")
        } else {
            format!("\\u{cp:04X}")
        };
        let script = format!(
            "v=$'{escape}'; printf '%s' \"$v\"; printf '\\n@@2@@\\n'; \
             echo \"$v\"; printf '@@3@@\\n'; w=\"a${{v}}b\"; \
             printf '%s' \"$w\"; printf '\\n@@END@@\\n'"
        );
        let out = Command::new(bin)
            .args(["-c", &script])
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .output()
            .expect("run registry cell");
        assert!(
            out.status.success(),
            "registry cell U+{cp:04X} ran clean: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = out.stdout;
        let mut segs = stdout.split(|b| *b == b'\n');
        // Segment layout: [S1] @@2@@ [S2] @@3@@ [S3] @@END@@ []
        let s1 = segs.next().unwrap_or_default().to_vec();
        assert_eq!(
            segs.next(),
            Some("@@2@@".as_bytes()),
            "cell frame for U+{cp:04X}: {stdout:?}"
        );
        let s2 = segs.next().unwrap_or_default().to_vec();
        assert_eq!(
            segs.next(),
            Some("@@3@@".as_bytes()),
            "cell frame for U+{cp:04X}: {stdout:?}"
        );
        let s3 = segs.next().unwrap_or_default().to_vec();

        let mut expected_char = [0u8; 4];
        let char_bytes = ch.encode_utf8(&mut expected_char).as_bytes().to_vec();
        let shapes: [(u8, Vec<u8>, Vec<u8>); 3] = [
            (1, s1, char_bytes.clone()),
            (2, s2, char_bytes.clone()),
            (
                3,
                s3,
                [b"a".as_slice(), &char_bytes, b"b".as_slice()].concat(),
            ),
        ];
        for (shape, got, want) in shapes {
            let exception = TYPED_DATA_EXCEPTIONS
                .iter()
                .find(|&&(ecp, eshape, _)| ecp == cp && eshape == shape);
            match exception {
                Some((_, _, tracker)) => assert_ne!(
                    got, want,
                    "U+{cp:04X} shape {shape} now round-trips — remove its \
                     exception (tracked by {tracker})"
                ),
                None => assert_eq!(
                    got, want,
                    "typed U+{cp:04X} must round-trip through shape {shape} \
                     (GNU-collected cell, rubash#2885 E6)"
                ),
            }
        }
    }
}
