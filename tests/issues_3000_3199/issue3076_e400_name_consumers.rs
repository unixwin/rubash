//! Regression pins for rubash#3076 — the stored E400 literal-char
//! escape pair (U+E400 + char) rode through NAME-consumption
//! boundaries that never folded it back to the raw char, so any file
//! or command whose name contains a registry-zone char (here
//! U+E314, stored as the pair U+E400 U+E314) was unfindable /
//! misclassified / misreported:
//!
//!   * PATH lookup (`find_user_command`) consulted the pair name and
//!     reported command-not-found for an existing executable.
//!   * slash-name spawn classified a non-executable file 127
//!     ("No such file or directory") instead of GNU's 126
//!     ("Permission denied") because the metadata probe ran on the
//!     pair name; a slash-named executable did not run at all.
//!   * `cd` probed and reported the pair (operand, CDPATH joins,
//!     cdable_vars substituted values, composite operands).
//!   * `exec` failure diagnostics printed the pair and its slash
//!     classification was likewise 127-instead-of-126.
//!   * the same unfused pair leaked at every other name boundary the
//!     census reached: source (cwd + PATH search + diagnostics),
//!     pushd, glob pattern matching, history -r file, bind -f,
//!     enable -f diagnostics, hash -p pathname + listing, test file
//!     operands (unary and binary, `[ ]` and `[[ ]]`),
//!     command -v / type (via the shared lookup), pipeline-stage
//!     slash classification, and the `$(cd … && pwd)` fast path.
//!
//! GNU ownership: GNU has no pair — the expanded word IS the name.
//! findcmd.c:348 search_for_command / findcmd.c:623
//! find_user_command_in_path walk PATH with the name bytes verbatim;
//! execute_cmd.c:5909+ shell_execve classifies by the errno of the
//! real file (EACCES on an existing non-executable => 126
//! "Permission denied", ENOENT => 127); builtins/cd.def:427 reports
//! the operand bytes; builtins/exec.def:158-172 composes
//! full_pathname and the "cannot execute" diagnostics from the name
//! bytes; builtins/pushd.def delegates the Dir operand to the cd
//! machinery; source.def:179 searches $PATH with the name bytes;
//! test.c:549-559 stats the operand bytes.
//!
//! #3040 folded the pair at two boundaries (command-not-found
//! diagnostic + redirect operand). These pins hold every boundary
//! from the #3076 census to the same gate + decoder: a name is
//! looked up, probed, executed and reported in its raw-char form.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts (~/workspace/fix-3076/probes) under LC_ALL=C.UTF-8.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const BIN: &str = env!("CARGO_BIN_EXE_rubash");
/// The raw registry-zone char the fixture names carry.
const CH: char = '\u{E314}';
/// The E400 escape-pair introducer: must never appear in any
/// lookup result, classification, or diagnostic.
const GATE: char = '\u{E400}';

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3076-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn write(path: &Path, content: &str, mode: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture parent");
    }
    std::fs::write(path, content).expect("fixture file");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("fixture mode");
}

/// Build the char-named fixture tree (mirrors the probe fixture).
fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let ch = CH.to_string();
    write(
        &root.join("bin").join(&ch),
        "#!/bin/sh\necho RAN-PATH\n",
        0o755,
    );
    write(&root.join("noexec").join(&ch), "not a program\n", 0o644);
    write(
        &root.join("execfile").join(&ch),
        "#!/bin/sh\necho RAN-SLASH\n",
        0o755,
    );
    std::fs::create_dir_all(root.join("dir").join(&ch).join("sub")).expect("fixture dir tree");
    write(&root.join("srcdir").join(&ch), "echo SOURCED-CWD\n", 0o644);
    write(
        &root.join("srcpath/bin").join(&ch),
        "echo SOURCED-PATH\n",
        0o644,
    );
    write(&root.join("glob").join(format!("zz{ch}x")), "x\n", 0o644);
    write(&root.join("hist").join(&ch), "echo HISTLINE\n", 0o644);
    write(
        &root.join("bind").join(&ch),
        "\"\\C-g\": \"echo BINDOK\\n\"\n",
        0o644,
    );
    let newer = root.join("cmp").join(format!("a{ch}"));
    let older = root.join("cmp").join("b");
    write(&newer, "new\n", 0o644);
    write(&older, "old\n", 0o644);
    let f = std::fs::File::open(&older).expect("older file");
    f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000))
        .expect("older mtime");
    let f = std::fs::File::open(&newer).expect("newer file");
    f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000))
        .expect("newer mtime");
    root
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

/// Run one probe script (written into the fixture root) with the
/// fixture as cwd, exactly like the GNU/base probe matrix.
fn run(root: &Path, body: &str) -> Outcome {
    let script = root.join("case.sh");
    std::fs::write(&script, body).expect("case script");
    let out = Command::new(BIN)
        .arg(&script)
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .env("FIX", root)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

/// No diagnostic or listing may ever carry the E400 gate char.
fn assert_no_gate(out: &Outcome) {
    assert!(
        !out.stdout.contains(GATE) && !out.stderr.contains(GATE),
        "E400 pair leaked: stdout={:?} stderr={:?}",
        out.stdout,
        out.stderr
    );
}

// ---- command lookup (findcmd.c:348 / findcmd.c:623) ----

#[test]
fn path_lookup_runs_char_named_executable() {
    let root = fixture("path");
    let out = run(
        &root,
        "export PATH=\"$FIX/bin:/usr/bin:/bin\"\nv=$'\\uE314'; \"$v\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, "RAN-PATH\nrc=0\n");
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}

#[test]
fn command_v_and_type_report_the_raw_path() {
    let root = fixture("cmdv");
    let out = run(
        &root,
        "export PATH=\"$FIX/bin:/usr/bin:/bin\"\nv=$'\\uE314'; command -v \"$v\"; echo \"rc=$?\"; type \"$v\"; echo \"rc=$?\"\n",
    );
    let path = format!("{}/bin/{CH}", root.display());
    assert_eq!(out.stdout, format!("{path}\nrc=0\n{CH} is {path}\nrc=0\n"));
    assert_no_gate(&out);
}

// ---- slash-name spawn classification (execute_cmd.c shell_execve) ----

#[test]
fn slash_noexec_is_126_permission_denied() {
    let root = fixture("slashperm");
    let out = run(
        &root,
        "cd \"$FIX/noexec\" || exit 99\nv=$'\\uE314'; \"./$v\"; echo \"rc=$?\"\n",
    );
    // GNU: rc 126, "Permission denied" (the base reported 127 /
    // "No such file or directory" — the pair-named probe missed the
    // file). The diagnostic's path SPELLING (absolutized candidate
    // vs GNU's as-typed word) is a pre-existing plain-name form
    // difference, not part of this family; the classification, the
    // errno text, and the raw-char name bytes are the pin.
    assert_eq!(out.stdout, "rc=126\n");
    assert!(
        out.stderr.contains("Permission denied") && out.stderr.contains(CH),
        "stderr={:?}",
        out.stderr
    );
    assert_no_gate(&out);
}

#[test]
fn slash_exec_runs() {
    let root = fixture("slashexec");
    let out = run(
        &root,
        "cd \"$FIX/execfile\" || exit 99\nv=$'\\uE314'; \"./$v\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, "RAN-SLASH\nrc=0\n");
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}

#[test]
fn pipeline_slash_noexec_is_126() {
    let root = fixture("pipeperm");
    let out = run(
        &root,
        "cd \"$FIX/noexec\" || exit 99\nv=$'\\uE314'; \"./$v\" | cat; echo \"rc=${PIPESTATUS[0]}\"\n",
    );
    // The pipeline stage classifies through the same execve errno
    // switch: GNU's stage reports "Permission denied"; the base
    // reported "No such file or directory" for the pair name. (The
    // stage's PIPESTATUS value itself is a pre-existing plain-name
    // divergence — 0 for plain names on the base too — and is not
    // asserted here.)
    assert!(
        out.stderr.contains("Permission denied") && out.stderr.contains(CH),
        "stderr={:?}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("No such file or directory"),
        "stderr={:?}",
        out.stderr
    );
    assert_no_gate(&out);
}

// ---- cd (builtins/cd.def:427) ----

#[test]
fn cd_into_char_dir() {
    let root = fixture("cd");
    let out = run(&root, "v=$'\\uE314'; cd \"dir/$v\"; echo \"rc=$?\"; pwd\n");
    assert_eq!(out.stdout, format!("rc=0\n{}/dir/{CH}\n", root.display()));
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}

#[test]
fn cd_composite_operand() {
    let root = fixture("cdsub");
    let out = run(
        &root,
        "cd \"$FIX/dir\" || exit 99\nv=$'\\uE314'; cd \"$v/sub\"; echo \"rc=$?\"; pwd\n",
    );
    assert_eq!(
        out.stdout,
        format!("rc=0\n{}/dir/{CH}/sub\n", root.display())
    );
    assert_no_gate(&out);
}

#[test]
fn cd_missing_reports_raw_char() {
    let root = fixture("cdmiss");
    let out = run(&root, "v=$'\\uE314'; cd \"./$v\"; echo \"rc=$?\"\n");
    assert_eq!(out.stdout, "rc=1\n");
    assert!(
        out.stderr
            .contains(&format!("cd: ./{CH}: No such file or directory")),
        "stderr={:?}",
        out.stderr
    );
    assert_no_gate(&out);
}

#[test]
fn cd_cdable_vars_uses_the_substituted_value() {
    let root = fixture("cdable");
    let out = run(
        &root,
        "cd \"$FIX/dir\" || exit 99\nshopt -s cdable_vars\nv=$'\\uE314'; dest=\"$v\"; cd dest; echo \"rc=$?\"; pwd\n",
    );
    assert_eq!(
        out.stdout,
        format!("{CH}\nrc=0\n{}/dir/{CH}\n", root.display())
    );
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}

#[test]
fn comsub_cd_fast_path_resolves_char_dir() {
    let root = fixture("comsubcd");
    let out = run(
        &root,
        "v=$'\\uE314'; x=$(cd \"dir/$v\" && pwd); echo \"rc=$?\"; echo \"$x\"\n",
    );
    assert_eq!(out.stdout, format!("rc=0\n{}/dir/{CH}\n", root.display()));
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}

// ---- exec (builtins/exec.def:158-172) ----

#[test]
fn exec_not_found_reports_raw_char() {
    let root = fixture("execmiss");
    let out = run(&root, "v=$'\\uE314'; exec \"$v\"\n");
    assert_eq!(out.rc, 127);
    assert!(
        out.stderr.contains(&format!("exec: {CH}: not found")),
        "stderr={:?}",
        out.stderr
    );
    assert_no_gate(&out);
}

#[test]
fn exec_slash_noexec_is_126_permission_denied() {
    let root = fixture("execperm");
    let out = run(&root, "v=$'\\uE314'; exec \"$FIX/noexec/$v\"\n");
    // GNU: rc 126 "Permission denied"; the base gave 127 with the
    // pair in the message. (The spawn layer's exact message framing
    // — an `exec:` prolog and an `(os error 13)` suffix — is a
    // pre-existing plain-name form difference.)
    assert_eq!(out.rc, 126);
    assert!(
        out.stderr.contains("Permission denied") && out.stderr.contains(CH),
        "stderr={:?}",
        out.stderr
    );
    assert_no_gate(&out);
}

#[test]
fn exec_slash_missing_reports_raw_full_path() {
    let root = fixture("execslashmiss");
    let out = run(&root, "v=$'\\uE314'; exec \"$FIX/noexec/x$v\"\n");
    assert_eq!(out.rc, 127);
    assert!(
        out.stderr.contains(&format!(
            "{}/noexec/x{CH}: No such file or directory",
            root.display()
        )),
        "stderr={:?}",
        out.stderr
    );
    assert_no_gate(&out);
}

// ---- source (source.def:179) ----

#[test]
fn source_cwd_char_file() {
    let root = fixture("srccwd");
    let out = run(
        &root,
        "cd \"$FIX/srcdir\" || exit 99\nv=$'\\uE314'; source \"$v\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, "SOURCED-CWD\nrc=0\n");
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}

#[test]
fn source_path_search_char_file() {
    let root = fixture("srcpath");
    let out = run(
        &root,
        "export PATH=\"$FIX/srcpath/bin:/usr/bin:/bin\"\nv=$'\\uE314'; source \"$v\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, "SOURCED-PATH\nrc=0\n");
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}

#[test]
fn source_missing_reports_raw_char() {
    let root = fixture("srcmiss");
    let out = run(&root, "v=$'\\uE314'; source \"./$v\"; echo \"rc=$?\"\n");
    assert_eq!(out.stdout, "rc=1\n");
    assert!(
        out.stderr
            .contains(&format!("./{CH}: No such file or directory")),
        "stderr={:?}",
        out.stderr
    );
    assert_no_gate(&out);
}

// ---- pushd (builtins/pushd.def, Dir operand via cd machinery) ----

#[test]
fn pushd_popd_char_dir() {
    let root = fixture("pushd");
    let out = run(
        &root,
        "v=$'\\uE314'; pushd \"dir/$v\" >/dev/null; echo \"rc=$?\"; pwd; popd >/dev/null; echo \"rc2=$?\"\n",
    );
    assert_eq!(
        out.stdout,
        format!("rc=0\n{}/dir/{CH}\nrc2=0\n", root.display())
    );
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}

// ---- test operands (test.c:549-559) ----

#[test]
fn test_unary_operands_see_the_raw_file() {
    let root = fixture("testun");
    let out = run(
        &root,
        "cd \"$FIX\" || exit 99\nv=$'\\uE314'\n[ -e \"bin/$v\" ] && echo E-YES\n[ -f \"bin/$v\" ] && echo F-YES\n[ -x \"noexec/$v\" ] || echo X-NO\n[ -d \"dir/$v\" ] && echo D-YES\n[[ -e \"bin/$v\" ]] && echo CE-YES\n[[ -d \"dir/$v\" ]] && echo CD-YES\n",
    );
    assert_eq!(out.stdout, "E-YES\nF-YES\nX-NO\nD-YES\nCE-YES\nCD-YES\n");
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}

#[test]
fn test_binary_operands_see_the_raw_file() {
    let root = fixture("testbin");
    let out = run(
        &root,
        "cd \"$FIX/cmp\" || exit 99\nv=$'\\uE314'\n[ \"a$v\" -nt b ] && echo NT-YES\n[ b -ot \"a$v\" ] && echo OT-YES\n[ \"a$v\" -ef \"a$v\" ] && echo EF-YES\n",
    );
    assert_eq!(out.stdout, "NT-YES\nOT-YES\nEF-YES\n");
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}

// ---- glob matching ----

#[test]
fn glob_pattern_matches_char_name() {
    let root = fixture("glob");
    let out = run(
        &root,
        "cd \"$FIX/glob\" || exit 99\nv=$'\\uE314'; pre=\"zz$v\"; echo $pre*; echo \"$pre\"*\n",
    );
    assert_eq!(out.stdout, format!("zz{CH}x\nzz{CH}x\n"));
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}

// ---- history / bind / enable / hash ----

#[test]
fn history_read_char_file() {
    let root = fixture("hist");
    // Absolute operand: a RELATIVE history filename after `cd` is a
    // separate pre-existing session-cwd anchoring gap (plain names
    // fail identically on the base); this pin holds the E400 fold.
    let out = run(
        &root,
        "set -o history\nv=$'\\uE314'; history -r \"$FIX/hist/$v\"; echo \"rc=$?\"; history\n",
    );
    assert!(out.stdout.contains("rc=0"), "stdout={:?}", out.stdout);
    assert!(
        out.stdout.contains("echo HISTLINE"),
        "stdout={:?}",
        out.stdout
    );
    assert_no_gate(&out);
}

#[test]
fn bind_reads_char_file() {
    let root = fixture("bind");
    // Absolute operand, as in history_read_char_file (the relative
    // form's anchoring gap is pre-existing and name-independent).
    let out = run(
        &root,
        "v=$'\\uE314'; bind -f \"$FIX/bind/$v\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, "rc=0\n");
    assert!(
        !out.stderr.contains("cannot read"),
        "stderr={:?}",
        out.stderr
    );
    assert_no_gate(&out);
}

#[test]
fn enable_f_diagnostic_reports_raw_char() {
    let root = fixture("enable");
    let out = run(
        &root,
        "v=$'\\uE314'; enable -f \"./$v\" true; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, "rc=1\n");
    assert!(
        out.stderr.contains(&format!("./{CH}")),
        "stderr={:?}",
        out.stderr
    );
    assert_no_gate(&out);
}

#[test]
fn hash_p_pathname_runs_and_lists_raw() {
    let root = fixture("hashp");
    let out = run(
        &root,
        "v=$'\\uE314'; hash -p \"$FIX/execfile/$v\" mycmd314; mycmd314; echo \"rc=$?\"; hash -p \"$FIX/bin/$v\" mycmd315; hash\n",
    );
    assert!(
        out.stdout.contains("RAN-SLASH\nrc=0\n"),
        "stdout={:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains(&format!("{}/bin/{CH}", root.display())),
        "stdout={:?}",
        out.stdout
    );
    assert_eq!(out.stderr, "");
    assert_no_gate(&out);
}
