//! Regression pins for the simulation-layer option-discipline batch
//! (rubash#1941, #1942, #1943, #1944, #1945, #1946, #1947, #1948):
//! the emulated coreutils applets in
//! `src/executor/external_file_builtins.rs` resolve long options the
//! gnulib getopt_long way (exact match or unambiguous prefix,
//! argmatch for option values) and print the coreutils 9.4
//! diagnostics byte-for-byte. Every expectation below was measured
//! against the local GNU coreutils 9.4 oracle.
//!
//! Semantic rows run with the real system PATH: the emulation
//! answers in-process, so the observed diagnostics and file effects
//! are the emulation's own. The mkfifo rows are Windows-gated: off
//! Windows mkfifo declines to the real binary (rubash#1725), so only
//! the Windows arm can pin rubash#1945.

#[cfg(any(unix, windows))]
use std::path::{Path, PathBuf};
#[cfg(any(unix, windows))]
use std::process::{Command, Stdio};

#[cfg(any(unix, windows))]
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

#[cfg(any(unix, windows))]
struct Output {
    status: i32,
    stdout: String,
    stderr: String,
}

#[cfg(any(unix, windows))]
fn run_in_dir(dir: &Path, script: &str) -> Output {
    let path_env = std::env::var("PATH").unwrap_or_default();
    let output = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(dir)
        .env("PATH", path_env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Output {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
    }
}

#[cfg(any(unix, windows))]
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-opts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

// ---------------------------------------------------------------- #1941

#[cfg(unix)]
#[test]
fn issue1941_cp_preserve_invalid_attribute_diagnoses_and_creates_nothing() {
    let dir = scratch("1941-bogus");
    let out = run_in_dir(
        &dir,
        "echo hi > a; cp --preserve=bogus a b; echo rc=$?; test -e b && echo B-EXISTS || echo B-ABSENT",
    );
    assert_eq!(out.stdout, "rc=1\nB-ABSENT\n");
    assert_eq!(
        out.stderr,
        "cp: invalid argument 'bogus' for '--preserve'\n\
         Valid arguments are:\n\
         \x20 - 'mode'\n\
         \x20 - 'timestamps'\n\
         \x20 - 'ownership'\n\
         \x20 - 'links'\n\
         \x20 - 'context'\n\
         \x20 - 'xattr'\n\
         \x20 - 'all'\n\
         Try 'cp --help' for more information.\n"
    );
}

#[cfg(unix)]
#[test]
fn issue1941_cp_preserve_empty_element_is_ambiguous() {
    let dir = scratch("1941-empty");
    let out = run_in_dir(&dir, "echo hi > a; cp --preserve= a b; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert!(out
        .stderr
        .starts_with("cp: ambiguous argument '' for '--preserve'\nValid arguments are:\n"));
}

#[cfg(unix)]
#[test]
fn issue1941_cp_no_preserve_invalid_attribute_names_the_option() {
    let dir = scratch("1941-nopreserve");
    let out = run_in_dir(&dir, "echo hi > a; cp --no-preserve=bogus a b; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert!(out
        .stderr
        .starts_with("cp: invalid argument 'bogus' for '--no-preserve'\n"));
}

#[cfg(unix)]
#[test]
fn issue1941_cp_preserve_links_keeps_hard_links_linked() {
    let dir = scratch("1941-links");
    let out = run_in_dir(
        &dir,
        "mkdir -p src/sub; echo data > src/f1; ln src/f1 src/f2; ln src/f1 src/sub/f3; \
         echo solo > src/sub/solo; cp -r --preserve=links src dst; \
         a=$(stat -c %i dst/f1); b=$(stat -c %i dst/f2); c=$(stat -c %i dst/sub/f3); \
         s=$(stat -c %i dst/sub/solo); \
         if [ \"$a\" = \"$b\" ] && [ \"$a\" = \"$c\" ] && [ \"$a\" != \"$s\" ]; then echo LINKS-OK; else echo LINKS-BAD; fi",
    );
    assert_eq!(out.stdout, "LINKS-OK\n");
    assert_eq!(out.stderr, "");
}

#[cfg(unix)]
#[test]
fn issue1941_cp_preserve_links_across_plain_operands() {
    let dir = scratch("1941-links-plain");
    let out = run_in_dir(
        &dir,
        "mkdir s2 dd; echo x > s2/g1; ln s2/g1 s2/g2; cp --preserve=links s2/g1 s2/g2 dd/; \
         a=$(stat -c %i dd/g1); b=$(stat -c %i dd/g2); \
         if [ \"$a\" = \"$b\" ]; then echo LINKS-OK; else echo LINKS-BAD; fi",
    );
    assert_eq!(out.stdout, "LINKS-OK\n");
}

#[cfg(unix)]
#[test]
fn issue1941_cp_preserve_abbreviated_values_and_mode_apply() {
    let dir = scratch("1941-abbrev");
    let out = run_in_dir(
        &dir,
        "echo hi > a; chmod 640 a; cp --preserve=mod,timestamps a b; echo rc=$?; stat -c %a b",
    );
    assert_eq!(out.stdout, "rc=0\n640\n");
}

// ---------------------------------------------------------------- #1942

#[cfg(unix)]
#[test]
fn issue1942_cp_unknown_long_option_reports_whole_word() {
    let dir = scratch("1942-unknown");
    let out = run_in_dir(&dir, "echo hi > a; cp --bogus a b; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "cp: unrecognized option '--bogus'\nTry 'cp --help' for more information.\n"
    );
    let out = run_in_dir(&dir, "cp --bogus=1 a b; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "cp: unrecognized option '--bogus=1'\nTry 'cp --help' for more information.\n"
    );
}

#[cfg(unix)]
#[test]
fn issue1942_cp_unambiguous_abbreviations_are_accepted() {
    let dir = scratch("1942-abbrev");
    let out = run_in_dir(
        &dir,
        "echo hi > a; mkdir d1; cp --rec a d1; echo rc=$?; test -f d1/a && echo COPIED",
    );
    assert_eq!(out.stdout, "rc=0\nCOPIED\n");
    assert_eq!(out.stderr, "");
}

#[cfg(unix)]
#[test]
fn issue1942_cp_ambiguous_prefix_lists_candidates_in_table_order() {
    let dir = scratch("1942-ambiguous");
    let out = run_in_dir(&dir, "echo hi > a; cp --re a b; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "cp: option '--re' is ambiguous; possibilities: '--recursive' '--remove-destination' '--reflink'\n\
         Try 'cp --help' for more information.\n"
    );
}

#[cfg(unix)]
#[test]
fn issue1942_cp_unknown_short_option_carries_try_line() {
    let dir = scratch("1942-short");
    let out = run_in_dir(&dir, "echo hi > a; cp -Q a b; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "cp: invalid option -- 'Q'\nTry 'cp --help' for more information.\n"
    );
}

// ---------------------------------------------------------------- #1944

#[cfg(unix)]
#[test]
fn issue1944_touch_f_is_a_silent_noop_flag() {
    let dir = scratch("1944-f");
    let out = run_in_dir(
        &dir,
        "touch -f nf; echo rc=$?; test -e nf && echo CREATED; \
         touch -cf nf2; echo rc=$?; test -e nf2 && echo BAD-CREATED || echo NOT-CREATED",
    );
    assert_eq!(out.stdout, "rc=0\nCREATED\nrc=0\nNOT-CREATED\n");
    assert_eq!(out.stderr, "");
}

// ---------------------------------------------------------------- #1947

#[cfg(unix)]
#[test]
fn issue1947_touch_missing_option_values_diagnose() {
    let dir = scratch("1947-missing");
    let out = run_in_dir(&dir, "touch f -r; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "touch: option requires an argument -- 'r'\nTry 'touch --help' for more information.\n"
    );

    let out = run_in_dir(&dir, "touch f --date; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "touch: option '--date' requires an argument\nTry 'touch --help' for more information.\n"
    );

    let out = run_in_dir(&dir, "touch f -t; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "touch: option requires an argument -- 't'\nTry 'touch --help' for more information.\n"
    );

    let out = run_in_dir(&dir, "touch f --time; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "touch: option '--time' requires an argument\nTry 'touch --help' for more information.\n"
    );
}

// ---------------------------------------------------------------- #1948

#[cfg(unix)]
#[test]
fn issue1948_touch_last_reference_wins_and_superseded_never_stats() {
    let dir = scratch("1948-ref");
    let out = run_in_dir(
        &dir,
        "touch -d '2001-01-01' good; touch -d '2002-02-02' good2; \
         touch -r missing -r good out1; echo rc=$?; \
         test \"$(stat -c %Y out1)\" = \"$(stat -c %Y good)\" && echo OUT1-GOOD; \
         touch -r good -r good2 out9; \
         test \"$(stat -c %Y out9)\" = \"$(stat -c %Y good2)\" && echo OUT9-GOOD2",
    );
    assert_eq!(out.stdout, "rc=0\nOUT1-GOOD\nOUT9-GOOD2\n");
    assert_eq!(out.stderr, "");
}

#[cfg(unix)]
#[test]
fn issue1948_touch_winning_bad_reference_still_fails() {
    let dir = scratch("1948-ref-bad");
    let out = run_in_dir(
        &dir,
        "touch -d '2001-01-01' good; touch -r good -r missing out2; echo rc=$?; \
         test -e out2 && echo BAD-CREATED || echo NOT-CREATED",
    );
    assert_eq!(out.stdout, "rc=1\nNOT-CREATED\n");
    assert!(out
        .stderr
        .contains("touch: failed to get attributes of 'missing'"));
}

#[cfg(unix)]
#[test]
fn issue1948_touch_superseded_bad_date_is_never_parsed() {
    let dir = scratch("1948-date");
    let out = run_in_dir(
        &dir,
        "touch -d 'garbage!!' -d '2001-01-01' out3; echo rc=$?; \
         test \"$(stat -c %Y out3)\" = \"$(stat -c %Y out3)\" && echo OUT3-OK; \
         touch -d '2001-01-01' -d 'garbage!!' out4; echo rc=$?",
    );
    // out4's surviving -d is outside the emulation's grammar, so the
    // invocation delegates to the real GNU touch on PATH, which
    // reports the same invalid-date line the emulation would.
    assert_eq!(out.stdout, "rc=0\nOUT3-OK\nrc=1\n");
    assert!(out.stderr.contains("invalid date format 'garbage!!'"));
}

#[cfg(unix)]
#[test]
fn issue1948_touch_date_beats_reference_and_t_conflicts() {
    let dir = scratch("1948-mix");
    let out = run_in_dir(
        &dir,
        "touch -d '2001-01-01' good; touch -d '2002-02-02' y2002; \
         touch -r good -d '2002-02-02' out1; \
         test \"$(stat -c %Y out1)\" = \"$(stat -c %Y y2002)\" && echo DATE-WINS; \
         touch -r missing -t 200101010000 out8; echo rc=$?; \
         touch -t badstamp -t 200101010000 outA; echo rc=$?; \
         test -e outA && echo BAD-CREATED || echo A-ABSENT",
    );
    assert_eq!(out.stdout, "DATE-WINS\nrc=1\nrc=1\nA-ABSENT\n");
    assert!(out
        .stderr
        .contains("touch: cannot specify times from more than one source"));
    assert!(out.stderr.contains("touch: invalid date format 'badstamp'"));
}
// ---------------------------------------------------------------- #1946

#[cfg(unix)]
#[test]
fn issue1946_no_argument_long_options_reject_attached_values() {
    let dir = scratch("1946-eq");
    let out = run_in_dir(
        &dir,
        "echo hi > a; cp --recursive=x a b; echo rc=$?; test -e b && echo B-BAD || echo B-ABSENT",
    );
    assert_eq!(out.stdout, "rc=1\nB-ABSENT\n");
    assert_eq!(
        out.stderr,
        "cp: option '--recursive' doesn't allow an argument\nTry 'cp --help' for more information.\n"
    );

    let out = run_in_dir(
        &dir,
        "touch --no-create=x f; echo rc=$?; test -e f && echo F-BAD || echo F-ABSENT",
    );
    assert_eq!(out.stdout, "rc=1\nF-ABSENT\n");
    assert_eq!(
        out.stderr,
        "touch: option '--no-create' doesn't allow an argument\nTry 'touch --help' for more information.\n"
    );

    let out = run_in_dir(
        &dir,
        "echo hi > a; rm --force=x a; echo rc=$?; test -e a && echo A-KEPT",
    );
    assert_eq!(out.stdout, "rc=1\nA-KEPT\n");
    assert_eq!(
        out.stderr,
        "rm: option '--force' doesn't allow an argument\nTry 'rm --help' for more information.\n"
    );

    // An abbreviated spelling still reports the canonical name.
    let out = run_in_dir(&dir, "touch --no-c=x f; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "touch: option '--no-create' doesn't allow an argument\nTry 'touch --help' for more information.\n"
    );
}

// ---------------------------------------------------------------- #1945

#[cfg(windows)]
#[test]
fn issue1945_mkfifo_missing_operand_and_value_diagnose() {
    let dir = scratch("1945-mkfifo");
    let out = run_in_dir(&dir, "mkfifo; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "mkfifo: missing operand\nTry 'mkfifo --help' for more information.\n"
    );
    let out = run_in_dir(&dir, "mkfifo -m; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "mkfifo: option requires an argument -- 'm'\nTry 'mkfifo --help' for more information.\n"
    );
}

// ---------------------------------------------------------------- #1943

#[cfg(unix)]
#[test]
fn issue1943_chmod_capital_x_honors_directories() {
    let dir = scratch("1943-x");
    let out = run_in_dir(
        &dir,
        "mkdir d; chmod 666 d; chmod a+X d; stat -c %a d; \
         touch f; chmod 644 f; chmod a+X f; stat -c %a f; \
         mkdir e; chmod 666 e; chmod u+X e; stat -c %a e",
    );
    assert_eq!(out.stdout, "777\n644\n766\n");
    assert_eq!(out.stderr, "");
}

#[cfg(unix)]
#[test]
fn issue1943_chmod_capital_x_reads_the_original_mode() {
    // GNU mode_adjust judges X against the mode the file had before
    // this invocation: `u+x` must not arm a later `g+X` clause.
    let dir = scratch("1943-x-orig");
    let out = run_in_dir(&dir, "touch f; chmod 644 f; chmod u+x,g+X f; stat -c %a f");
    assert_eq!(out.stdout, "744\n");
}

