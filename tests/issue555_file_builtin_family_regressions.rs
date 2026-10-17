//! issue#555 regression: five sibling wrappers in
//! `src/executor/external_file_builtins.rs` — `external_mkdir`,
//! `external_touch`, `external_cp`, `external_rm`, `external_rmdir` — each
//! re-ran `expand_word` over argv that the simple-command executor had
//! ALREADY expanded once (execute_cmd.c execute_disk_command ->
//! expand_words). A single-quoted/escaped operand like `'$FIX/d'` arrives
//! with its quotes removed and a live-looking `$`, so the second pass
//! re-scanned the literal text and silently created/touched/copied/
//! removed the WRONG path: `mkdir '$FIX/d'` built `zz/d`, `rm '$FIX/r'`
//! deleted an existing `zz/r` with exit 0. Same P0 silent-corruption class
//! as #511 (fixed by #552 for cat) and #177 (sed, same file).
//!
//! The fix uses the argv as-is in every operand loop and in cp's
//! -t/--target-directory value, the same rule external_sed (rubash#177)
//! and external_cat (rubash#511) already follow.
//!
//! GNU 5.3.0 oracle probes (WSL /usr/local/bin/bash, 2026-10-12, script
//! file): `mkdir '$FIX/d'` -> rc 1, `mkdir: cannot create directory
//! '$FIX/d': No such file or directory`, no `zz/d`; same literal-name +
//! rc 1 shape for touch/cp/rm; `rmdir '$FIX/rd'` -> rc 1
//! `rmdir: failed to remove '$FIX/rd': ...`, `zz/rd` survives; unquoted
//! `$FIX/t` still expands and creates `zz/t`. rubash's own wrapper
//! leniencies (mkdir's create_dir_all implies -p, rmdir swallows errors)
//! are pre-existing wrapper contracts, NOT this issue, and the assertions
//! below pin the re-expansion invariant within those contracts.
//!
//! Windows-only rows (literal $FIX\d paths) carry #[cfg(windows)]; the rest pin portable no-re-expansion on every host.
//!
//! The family helper: every case below is a row of one table run through
//! `assert_literal_operand_contract` — the next wrapper added to
//! external_file_builtins.rs gets its row here and cannot quietly
//! reintroduce the second expansion.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

struct Output {
    status: i32,
    stdout: String,
    stderr: String,
}

/// Run `script` via -c inside a fresh scratch dir pre-seeded with the
/// family fixture — `zz/c` and `zz/r` files, `zz/rd` directory — and
/// return the three channels plus the dir (caller cleans up; the helper
/// below does it before asserting so failed runs never leak).
fn run_in_fixture_dir(script: &str, tag: &str) -> (Output, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue555-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("zz").join("rd")).expect("scratch dir");
    std::fs::write(dir.join("zz/c"), b"hi").expect("fixture zz/c");
    std::fs::write(dir.join("zz/r"), b"gone").expect("fixture zz/r");

    let mut child = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let _ = child.stdin.take();
    let output = child.wait_with_output().expect("wait for rubash");
    (
        Output {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
        },
        dir,
    )
}

/// One row of the family table: the literal-operand contract EVERY
/// wrapper in external_file_builtins.rs must satisfy. `guard` is the
/// fixture-relative path the BUGGY second expansion would have acted on;
/// `guard_survives` says whether it must still exist afterwards
/// (rm/rmdir fixtures) or must never come into existence
/// (mkdir/touch/cp). `literal_created`, when set, is a fixture-relative
/// path that PROVES the operand was applied to the literal text.
/// `stderr_contains` pins the literal operand name in the diagnostic
/// (None for wrappers that are silent on this path, i.e. rmdir, and for
/// succeeding mkdir runs).
struct Case {
    builtin: &'static str,
    tag: &'static str,
    script: &'static str,
    guard: &'static str,
    guard_survives: bool,
    expect_status: i32,
    stderr_contains: Option<&'static str>,
    literal_created: Option<&'static str>,
}

fn assert_literal_operand_contract(case: &Case) {
    let (out, dir) = run_in_fixture_dir(case.script, case.tag);
    let guard_held = dir.join(case.guard).exists();
    let literal_made = case.literal_created.map(|p| dir.join(p).exists());
    let _ = std::fs::remove_dir_all(&dir);

    if case.guard_survives {
        assert!(
            guard_held,
            "[{}:{}] the quoted operand was silently re-expanded and the \
             guard {} was destroyed: rc={} stdout={:?} stderr={:?}",
            case.builtin, case.tag, case.guard, out.status, out.stdout, out.stderr
        );
    } else {
        assert!(
            !guard_held,
            "[{}:{}] the quoted operand was re-expanded and acted on the \
             wrong path {}: rc={} stdout={:?} stderr={:?}",
            case.builtin, case.tag, case.guard, out.status, out.stdout, out.stderr
        );
    }
    if let Some(literal) = case.literal_created {
        assert!(
            literal_made == Some(true),
            "[{}:{}] the operand was not applied to the literal path {}: \
             rc={} stdout={:?} stderr={:?}",
            case.builtin,
            case.tag,
            literal,
            out.status,
            out.stdout,
            out.stderr
        );
    }
    if let Some(fragment) = case.stderr_contains {
        assert!(
            out.stderr.contains(fragment),
            "[{}:{}] the diagnostic lost the literal operand {:?}: \
             stderr={:?}",
            case.builtin,
            case.tag,
            fragment,
            out.stderr
        );
    }
    assert_eq!(
        out.status, case.expect_status,
        "[{}:{}] unexpected exit status (stdout={:?} stderr={:?})",
        case.builtin, case.tag, out.stdout, out.stderr
    );
}

/// The family table: one row per wrapper in external_file_builtins.rs.
/// Adding a wrapper to that file without adding its row here is the
/// gap this suite exists to close.
fn quoted_cases() -> Vec<Case> {
    vec![
        Case {
            builtin: "mkdir",
            tag: "mkdir-quoted",
            script: "FIX=zz; mkdir '$FIX/d'",
            guard: r"zz\d",
            guard_survives: false,
            expect_status: 0, // create_dir_all implies -p: the literal $FIX/d is created
            stderr_contains: None,
            literal_created: Some(r"$FIX\d"),
        },
        Case {
            builtin: "touch",
            tag: "touch-quoted",
            script: "FIX=zz; touch '$FIX/t'",
            guard: r"zz\t",
            guard_survives: false,
            expect_status: 1,
            stderr_contains: Some("$FIX/t"),
            literal_created: None,
        },
        Case {
            builtin: "cp",
            tag: "cp-quoted",
            script: "FIX=zz; cp zz/c '$FIX/dst'",
            guard: r"zz\dst",
            guard_survives: false,
            expect_status: 1,
            stderr_contains: Some("$FIX"),
            literal_created: None,
        },
        Case {
            builtin: "rm",
            tag: "rm-quoted",
            script: "FIX=zz; rm '$FIX/r'",
            guard: r"zz\r",
            guard_survives: true,
            expect_status: 1,
            stderr_contains: Some("$FIX/r"),
            literal_created: None,
        },
        Case {
            builtin: "rmdir",
            tag: "rmdir-quoted",
            script: "FIX=zz; rmdir '$FIX/rd'",
            guard: r"zz\rd",
            guard_survives: true,
            expect_status: 0, // the wrapper swallows per-operand errors (pre-existing)
            stderr_contains: None,
            literal_created: None,
        },
    ]
}

/// The shared-helper sweep: all five wrappers against the same
/// single-quoted `$VAR` operand contract.
#[cfg(windows)]
#[test]
fn family_table_covers_every_wrapper_in_external_file_builtins() {
    for case in quoted_cases() {
        assert_literal_operand_contract(&case);
    }
}

/// mkdir: the exact repro from the issue — `mkdir '$FIX/d'` must build the
/// literal path, never `zz/d`.
#[cfg(windows)]
#[test]
fn mkdir_single_quoted_variable_operand_is_not_re_expanded() {
    assert_literal_operand_contract(&Case {
        builtin: "mkdir",
        tag: "mkdir-solo",
        script: "FIX=zz; mkdir '$FIX/d'",
        guard: r"zz\d",
        guard_survives: false,
        expect_status: 0,
        stderr_contains: None,
        literal_created: Some(r"$FIX\d"),
    });
}

/// mkdir: the escaped form `\$FIX/d` decodes to the same literal text
/// after the first pass — it must not re-expand either.
#[cfg(windows)]
#[test]
fn mkdir_escaped_dollar_operand_is_literal() {
    assert_literal_operand_contract(&Case {
        builtin: "mkdir",
        tag: "mkdir-escaped",
        script: r"FIX=zz; mkdir \$FIX/d",
        guard: r"zz\d",
        guard_survives: false,
        expect_status: 0,
        stderr_contains: None,
        literal_created: Some(r"$FIX\d"),
    });
}

/// mkdir: an unset variable stays literal — the operand names a directory
/// literally called `$NOSUCHVAR555/d` in the cwd, not `/d` at the fs root.
#[cfg(windows)]
#[test]
fn mkdir_unset_variable_operand_stays_literal() {
    assert_literal_operand_contract(&Case {
        builtin: "mkdir",
        tag: "mkdir-unset",
        script: "mkdir '$NOSUCHVAR555/d'",
        guard: r"zz\d",
        guard_survives: false,
        expect_status: 0,
        stderr_contains: None,
        literal_created: Some(r"$NOSUCHVAR555\d"),
    });
}

/// touch: `touch '$FIX/t'` must fail on the literal name (GNU: `touch:
/// cannot touch '$FIX/t': No such file or directory`, exit 1) — it must
/// never create `zz/t`.
#[test]
fn touch_single_quoted_variable_operand_is_not_re_expanded() {
    assert_literal_operand_contract(&Case {
        builtin: "touch",
        tag: "touch-solo",
        script: "FIX=zz; touch '$FIX/t'",
        guard: r"zz\t",
        guard_survives: false,
        expect_status: 1,
        stderr_contains: Some("$FIX/t"),
        literal_created: None,
    });
}

/// touch: the escaped form is the same literal text.
#[test]
fn touch_escaped_dollar_operand_is_literal() {
    assert_literal_operand_contract(&Case {
        builtin: "touch",
        tag: "touch-escaped",
        script: r"FIX=zz; touch \$FIX/t",
        guard: r"zz\t",
        guard_survives: false,
        expect_status: 1,
        stderr_contains: Some("$FIX/t"),
        literal_created: None,
    });
}

/// touch: an unset variable keeps its literal name in the diagnostic.
#[test]
fn touch_unset_variable_operand_error_keeps_literal_name() {
    assert_literal_operand_contract(&Case {
        builtin: "touch",
        tag: "touch-unset",
        script: "touch '$NOSUCHVAR555/t'",
        guard: r"zz\t",
        guard_survives: false,
        expect_status: 1,
        stderr_contains: Some("$NOSUCHVAR555/t"),
        literal_created: None,
    });
}

/// touch.c: a failed operand prints a diagnostic and the loop continues —
/// the later literal operand is still stamped, status 1 overall.
#[test]
fn touch_failing_quoted_operand_does_not_stop_later_operands() {
    let (out, dir) = run_in_fixture_dir("FIX=zz; touch '$FIX/t' zz/c", "touch-mixed");
    let zz_c = dir.join("zz").join("c").exists();
    let zz_t = dir.join("zz").join("t").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(zz_c, "touch stopped at the failed operand; GNU continues");
    assert!(!zz_t, "touch re-expanded the quoted operand and made zz/t");
    assert_eq!(out.status, 1, "any operand failure leaves status 1");
    assert!(out.stderr.contains("$FIX/t"), "stderr={:?}", out.stderr);
}

/// cp: a single-quoted DESTINATION must fail on the literal name (GNU:
/// `cp: cannot create regular file '$FIX/dst': ...`, exit 1), never land
/// the copy in `zz/dst`.
#[test]
fn cp_single_quoted_destination_is_not_re_expanded() {
    assert_literal_operand_contract(&Case {
        builtin: "cp",
        tag: "cp-solo",
        script: "FIX=zz; cp zz/c '$FIX/dst'",
        guard: r"zz\dst",
        guard_survives: false,
        expect_status: 1,
        stderr_contains: Some("$FIX"),
        literal_created: None,
    });
}

/// cp: a single-quoted SOURCE must fail naming the literal operand (GNU:
/// `cp: cannot stat '$FIX/src': No such file or directory`), never copy
/// the re-expanded neighbor `zz/src` under a wrong name.
#[test]
fn cp_single_quoted_source_is_not_re_expanded() {
    assert_literal_operand_contract(&Case {
        builtin: "cp",
        tag: "cp-src",
        script: "FIX=zz; cp '$FIX/src' zz/d2",
        guard: r"zz\d2",
        guard_survives: false,
        expect_status: 1,
        stderr_contains: Some("$FIX/src"),
        literal_created: None,
    });
}

/// cp: the escaped destination is the same literal text.
#[test]
fn cp_escaped_dollar_destination_is_literal() {
    assert_literal_operand_contract(&Case {
        builtin: "cp",
        tag: "cp-escaped",
        script: r"FIX=zz; cp zz/c \$FIX/dst",
        guard: r"zz\dst",
        guard_survives: false,
        expect_status: 1,
        stderr_contains: Some("$FIX"),
        literal_created: None,
    });
}

/// cp: the -t/--target-directory VALUE is argv too — `cp -t '$FIX' zz/r`
/// must target the literal path, not the re-expanded `zz` directory.
#[test]
fn cp_target_directory_option_value_is_argv_too() {
    let (out, dir) = run_in_fixture_dir("FIX=zz; cp -t '$FIX' zz/r", "cp-topt");
    let literal = dir.join("$FIX");
    let literal_made = literal.exists();
    let zz_r = dir.join("zz").join("r").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        literal_made,
        "cp -t re-expanded the quoted target-directory value: stdout={:?} stderr={:?}",
        out.stdout, out.stderr
    );
    assert!(zz_r, "the source fixture vanished: stdout={:?}", out.stdout);
    assert_eq!(
        out.status, 0,
        "status={:?} stderr={:?}",
        out.stdout, out.stderr
    );
}

/// rm: `rm '$FIX/r'` must fail naming the literal operand (GNU: `rm:
/// cannot remove '$FIX/r': No such file or directory`, exit 1) and the
/// existing `zz/r` must survive — silent deletion is the worst member of
/// this family.
#[cfg(windows)]
#[test]
fn rm_single_quoted_variable_operand_is_not_re_expanded() {
    assert_literal_operand_contract(&Case {
        builtin: "rm",
        tag: "rm-solo",
        script: "FIX=zz; rm '$FIX/r'",
        guard: r"zz\r",
        guard_survives: true,
        expect_status: 1,
        stderr_contains: Some("$FIX/r"),
        literal_created: None,
    });
}

/// rm: the escaped form is the same literal text.
#[cfg(windows)]
#[test]
fn rm_escaped_dollar_operand_is_literal() {
    assert_literal_operand_contract(&Case {
        builtin: "rm",
        tag: "rm-escaped",
        script: r"FIX=zz; rm \$FIX/r",
        guard: r"zz\r",
        guard_survives: true,
        expect_status: 1,
        stderr_contains: Some("$FIX/r"),
        literal_created: None,
    });
}

/// rm: an unset variable keeps its literal name in the diagnostic.
#[cfg(windows)]
#[test]
fn rm_unset_variable_operand_error_keeps_literal_name() {
    assert_literal_operand_contract(&Case {
        builtin: "rm",
        tag: "rm-unset",
        script: "rm '$NOSUCHVAR555/r'",
        guard: r"zz\r",
        guard_survives: true,
        expect_status: 1,
        stderr_contains: Some("$NOSUCHVAR555/r"),
        literal_created: None,
    });
}

/// rm.c: a failed operand prints a diagnostic and the loop continues —
/// the later literal operand is still removed, status 1 overall.
#[test]
fn rm_failing_quoted_operand_does_not_stop_later_operands() {
    let (out, dir) = run_in_fixture_dir("FIX=zz; rm '$FIX/r' zz/c", "rm-mixed");
    let zz_r = dir.join("zz").join("r").exists();
    let zz_c = dir.join("zz").join("c").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        zz_r,
        "rm destroyed the guard zz/r for a quoted operand: stderr={:?}",
        out.stderr
    );
    assert!(
        !zz_c,
        "rm stopped at the failed operand; GNU continues: stderr={:?}",
        out.stderr
    );
    assert_eq!(out.status, 1, "any operand failure leaves status 1");
    assert!(out.stderr.contains("$FIX/r"), "stderr={:?}", out.stderr);
}

/// rmdir: `rmdir '$FIX/rd'` must never remove the re-expanded neighbor
/// `zz/rd` (the wrapper swallows per-operand errors and exits 0 — its
/// pre-existing contract; the survival invariant is this test).
#[cfg(windows)]
#[test]
fn rmdir_single_quoted_variable_operand_is_not_re_expanded() {
    assert_literal_operand_contract(&Case {
        builtin: "rmdir",
        tag: "rmdir-solo",
        script: "FIX=zz; rmdir '$FIX/rd'",
        guard: r"zz\rd",
        guard_survives: true,
        expect_status: 0,
        stderr_contains: None,
        literal_created: None,
    });
}

/// rmdir: the escaped form is the same literal text.
#[cfg(windows)]
#[test]
fn rmdir_escaped_dollar_operand_is_literal() {
    assert_literal_operand_contract(&Case {
        builtin: "rmdir",
        tag: "rmdir-escaped",
        script: r"FIX=zz; rmdir \$FIX/rd",
        guard: r"zz\rd",
        guard_survives: true,
        expect_status: 0,
        stderr_contains: None,
        literal_created: None,
    });
}

/// rmdir: an unset variable stays unexpanded — nothing named by the
/// literal text exists, so the guard directory must survive untouched.
#[cfg(windows)]
#[test]
fn rmdir_unset_variable_operand_stays_unexpanded() {
    assert_literal_operand_contract(&Case {
        builtin: "rmdir",
        tag: "rmdir-unset",
        script: "rmdir '$NOSUCHVAR555/rd'",
        guard: r"zz\rd",
        guard_survives: true,
        expect_status: 0,
        stderr_contains: None,
        literal_created: None,
    });
}

/// Quote fidelity must not swallow live expansions: every wrapper still
/// honors the unquoted `$VAR` form (GNU parity both ways).
#[cfg(windows)]
#[test]
fn unquoted_operands_still_expand_for_the_whole_family() {
    let (out, dir) = run_in_fixture_dir(
        "FIX=zz; mkdir $FIX/d; touch $FIX/t; cp zz/c $FIX/dst; rm $FIX/r; rmdir $FIX/rd",
        "family-live",
    );
    let made_d = dir.join(r"zz\d").exists();
    let made_t = dir.join(r"zz\t").exists();
    let made_dst = dir.join(r"zz\dst").exists();
    let kept_r = dir.join(r"zz\r").exists();
    let kept_rd = dir.join(r"zz\rd").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.status, 0,
        "stdout={:?} stderr={:?}",
        out.stdout, out.stderr
    );
    assert!(made_d, "unquoted mkdir must expand: {:?}", out.stderr);
    assert!(made_t, "unquoted touch must expand: {:?}", out.stderr);
    assert!(made_dst, "unquoted cp must expand: {:?}", out.stderr);
    assert!(
        !kept_r,
        "unquoted rm must expand and delete zz/r: {:?}",
        out.stderr
    );
    assert!(
        !kept_rd,
        "unquoted rmdir must expand and remove zz/rd: {:?}",
        out.stderr
    );
}

/// The command-substitution dispatch shares the same wrappers: a quoted
/// operand inside `$()` stays literal there too (mirrors the #511
/// comsub-cat test).
#[test]
fn command_substitution_mkdir_honors_single_quotes() {
    let (out, dir) =
        run_in_fixture_dir("FIX=zz; d=$(mkdir '$FIX/d'); echo \"[$d]\"", "comsub-mkdir");
    let zz_d = dir.join("zz").join("d").exists();
    let literal = dir.join("$FIX").join("d").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.stdout, "[]\n",
        "stdout={:?} stderr={:?}",
        out.stdout, out.stderr
    );
    assert!(
        !zz_d,
        "$(...) mkdir re-expanded the quoted operand: stderr={:?}",
        out.stderr
    );
    assert!(
        literal,
        "$(...) mkdir did not apply the literal operand: stderr={:?}",
        out.stderr
    );
}

/// Guard the neighboring contract while touching these loops: pathnames
/// still reach the wrappers through the once-at-dispatch expansion, and
/// quoted-prefix concatenation (`'zz'/inner`) stays a single literal path.
#[test]
fn pathname_expansion_still_reaches_the_wrappers_once() {
    let (out, dir) = run_in_fixture_dir("mkdir 'zz'/inner; rm zz/c; ls zz", "pathname-once");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.stdout.contains("inner"),
        "mkdir 'zz'/inner did not build zz/inner: stdout={:?} stderr={:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stdout.contains("rd") && out.stdout.contains('r'),
        "zz listing lost entries: stdout={:?} stderr={:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        !out.stdout.contains('c'),
        "rm zz/c did not run: stdout={:?}",
        out.stdout
    );
    assert_eq!(out.status, 0, "stderr={:?}", out.stderr);
}
