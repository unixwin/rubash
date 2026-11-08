//! Regression pins for rubash#3104 — the raw-byte marker pair
//! (U+E000 + payload char, the transport for a stored byte such as
//! 0x1C) rode through NAME-consumption boundaries that only folded
//! the E400 literal-char family, so any file or command whose name
//! contains a raw byte was unfindable / misprobed / created under
//! its pair spelling:
//!
//!   * a redirect target landed on disk under the pair spelling
//!     (bytes EE 80 80 EE 80 9D for one 0x1C byte) instead of the
//!     raw byte name (variable, comsub, and `$'...'` word forms);
//!   * `test` / `[` / `[[ ]]` file operands probed the pair name and
//!     reported false for existing files (unary and binary forms);
//!   * `source`, `cd`, `pushd`, and the `$(cd … && pwd)` fast path
//!     resolved the pair name and failed;
//!   * PATH lookup (pair-named directory entry and pair-named
//!     command), slash-name spawn, and `exec` missed the real file;
//!   * `type` reported the pair spelling; glob literal segments
//!     never matched the real directory entry; `bind -f` and
//!     `history -w` consumed / created the pair spelling;
//!   * `$(< file)` and `read … < file` followed the redirect chain
//!     into the pair name.
//!
//! This is the E000-domain counterpart of rubash#3076 (which folded
//! the E400 family at the same boundaries through the shared gate +
//! boundary decoder). GNU ownership: GNU has no pair — the expanded
//! word IS the name. redir.c:298 redirection_expand returns the
//! string do_redirection_internal opens verbatim; findcmd.c:348 /
//! findcmd.c:623 walk PATH with the name bytes verbatim;
//! execute_cmd.c:6139 shell_execve hands execve the raw word bytes;
//! builtins/cd.def:427, builtins/source.def:179, test.c:549-559 and
//! builtins/exec.def:158-172 all consume the operand bytes as-is.
//!
//! A genuine U+E000 char in a name (typed `$'\uE000'`, transported
//! as an E400 pair) is user data, not a raw-byte pair: the last two
//! pins hold that it still resolves to the char-named file.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts (~/workspace/fix-3104/probes) under LC_ALL=C.UTF-8.
#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const BIN: &str = env!("CARGO_BIN_EXE_rubash");
/// The raw byte the fixture names carry (0x1C, file separator).
const B: u8 = 0x1C;
/// UTF-8 of the raw-byte pair introducer U+E000: must never appear
/// in a lookup result, a diagnostic, or an on-disk name produced by
/// these probes (except the genuine-U+E000 control pins).
const PAIR_HEAD: &[u8] = "\u{E000}".as_bytes();

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3104-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn write_bytes(path: &Path, content: &[u8], mode: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture parent");
    }
    std::fs::write(path, content).expect("fixture file");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("fixture mode");
}

/// Join a fixture path whose final component carries the raw byte:
/// `name` is the ASCII stem, the byte and `tail` follow it.
fn byte_named(dir: &Path, stem: &str, tail: &str) -> PathBuf {
    let mut name = stem.as_bytes().to_vec();
    name.push(B);
    name.extend_from_slice(tail.as_bytes());
    dir.join(OsStr::from_bytes(&name))
}

/// Build the byte-named fixture tree (mirrors the probe fixture).
fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let sdir = byte_named(&root, "s", "d");
    write_bytes(&sdir.join("f.sh"), b"echo SOURCED-OK\n", 0o644);
    write_bytes(
        &root.join("plaindir").join("f.sh"),
        b"echo SOURCED-PLAIN\n",
        0o644,
    );
    let ab = byte_named(&root, "a", "b");
    write_bytes(&ab, b"AB\n", 0o644);
    let bindir = byte_named(&root, "bin", "d");
    write_bytes(&bindir.join("mycmd"), b"#!/bin/sh\necho MYCMD-RAN\n", 0o755);
    let paircmd = byte_named(&bindir, "c", "m");
    write_bytes(&paircmd, b"#!/bin/sh\necho PAIRCMD-RAN\n", 0o755);
    let bindf = byte_named(&root, "b", "f");
    write_bytes(&bindf, b"\"\\C-g\": abort\n", 0o644);
    // Genuine U+E000 char in a name (user data, E400-transported).
    write_bytes(&root.join("u\u{E000}v"), b"U\n", 0o644);
    let f = std::fs::File::open(sdir.join("f.sh")).expect("older file");
    f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000))
        .expect("older mtime");
    let f = std::fs::File::open(&ab).expect("newer file");
    f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_100))
        .expect("newer mtime");
    root
}

/// All entry names under `root`, as raw bytes, relative, sorted.
fn entry_names(root: &Path) -> Vec<Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let rel = path.strip_prefix(base).expect("rel");
            out.push(rel.as_os_str().as_bytes().to_vec());
            if entry.file_type().expect("ft").is_dir() {
                walk(&path, base, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
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
        stdout: out.stdout,
        stderr: out.stderr,
        rc: out.status.code().unwrap_or(-1),
    }
}

fn no_pair(out: &Outcome) {
    assert!(
        !out.stdout.windows(3).any(|w| w == PAIR_HEAD)
            && !out.stderr.windows(3).any(|w| w == PAIR_HEAD),
        "raw-byte pair leaked: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---- redirect targets (redir.c:298) ----

#[test]
fn redirect_var_target_lands_raw_byte_name() {
    let root = fixture("redirvar");
    let out = run(
        &root,
        "n=$(printf 'r\\0341'); printf X > \"$n\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, b"rc=0\n");
    let names = entry_names(&root);
    assert!(
        names.iter().any(|n| n == b"r\x1c1"),
        "raw byte name missing: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.windows(3).any(|w| w == PAIR_HEAD)),
        "pair-spelled name landed: {names:?}"
    );
    assert_eq!(
        std::fs::read(byte_named(&root, "r", "1")).expect("landed content"),
        b"X"
    );
    no_pair(&out);
}

#[test]
fn redirect_comsub_target_lands_raw_byte_name() {
    let root = fixture("redircomsub");
    let out = run(
        &root,
        "printf Y > \"$(printf 'r\\0342')\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, b"rc=0\n");
    let names = entry_names(&root);
    assert!(
        names.iter().any(|n| n == b"r\x1c2"),
        "raw byte name missing: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.windows(3).any(|w| w == PAIR_HEAD)),
        "pair-spelled name landed: {names:?}"
    );
    no_pair(&out);
}

#[test]
fn redirect_ansic_target_lands_raw_byte_name() {
    let root = fixture("rediransic");
    let out = run(&root, "printf Z > $'r\\0343'; echo \"rc=$?\"\n");
    assert_eq!(out.stdout, b"rc=0\n");
    let names = entry_names(&root);
    assert!(
        names.iter().any(|n| n == b"r\x1c3"),
        "raw byte name missing: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.windows(3).any(|w| w == PAIR_HEAD)),
        "pair-spelled name landed: {names:?}"
    );
    no_pair(&out);
}

// ---- test / [ / [[ ]] operands (test.c:549-559) ----

#[test]
fn test_operands_probe_raw_byte_names() {
    let root = fixture("testops");
    let out = run(
        &root,
        "n=$(printf 's\\034d'); test -d \"$n\" && echo TD-YES || echo TD-NO; [ -f \"$n/f.sh\" ] && echo TF-YES || echo TF-NO\n",
    );
    assert_eq!(out.stdout, b"TD-YES\nTF-YES\n");
    no_pair(&out);
}

#[test]
fn conditional_file_predicates_probe_raw_byte_names() {
    let root = fixture("cond");
    let out = run(
        &root,
        "n=$(printf 'a\\034b'); [[ -e $n ]] && echo CE-YES || echo CE-NO; m=$(printf 's\\034d'); [[ -d $m ]] && echo CDD-YES || echo CDD-NO\n",
    );
    assert_eq!(out.stdout, b"CE-YES\nCDD-YES\n");
    no_pair(&out);
}

#[test]
fn test_binary_operand_probes_raw_byte_names() {
    let root = fixture("testbin");
    let out = run(
        &root,
        "n=$(printf 'a\\034b'); m=$(printf 's\\034d/f.sh'); test \"$n\" -nt \"$m\" && echo NT-YES || echo NT-NO\n",
    );
    assert_eq!(out.stdout, b"NT-YES\n");
    no_pair(&out);
}

// ---- source / cd / pushd (source.def:179, cd.def:427) ----

#[test]
fn source_runs_byte_named_file() {
    let root = fixture("source");
    let out = run(
        &root,
        "n=$(printf 's\\034d'); source \"$n/f.sh\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, b"SOURCED-OK\nrc=0\n");
    no_pair(&out);
}

#[test]
fn cd_enters_byte_named_dir() {
    let root = fixture("cd");
    let out = run(
        &root,
        "n=$(printf 's\\034d'); cd \"$n\" && echo CD-IN || echo CD-OUT\n",
    );
    assert_eq!(out.stdout, b"CD-IN\n");
    no_pair(&out);
}

#[test]
fn pushd_enters_byte_named_dir() {
    let root = fixture("pushd");
    let out = run(
        &root,
        "n=$(printf 's\\034d'); pushd \"$n\" >/dev/null 2>&1 && { echo PUSHD-IN; popd >/dev/null; } || echo PUSHD-OUT\n",
    );
    assert_eq!(out.stdout, b"PUSHD-IN\n");
    no_pair(&out);
}

#[test]
fn comsub_cd_fast_path_resolves_byte_named_dir() {
    let root = fixture("comsubcd");
    let out = run(
        &root,
        "n=$(printf 's\\034d'); x=$(cd \"$n\" && pwd); echo \"[$x]\"\n",
    );
    let expect = format!("[{}/s\x1cd]\n", root.display()).into_bytes();
    assert_eq!(out.stdout, expect);
    no_pair(&out);
}

// ---- command lookup and spawn (findcmd.c:348/623, execute_cmd.c:6139) ----

#[test]
fn path_lookup_finds_byte_named_dir_and_command() {
    let root = fixture("pathlookup");
    let out = run(
        &root,
        "d=$(printf 'bin\\034d'); PATH=\"$PWD/$d:$PATH\"; mycmd; c=$(printf 'c\\034m'); \"$c\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, b"MYCMD-RAN\nPAIRCMD-RAN\nrc=0\n");
    no_pair(&out);
}

#[test]
fn slash_name_spawn_runs_byte_named_executable() {
    let root = fixture("slash");
    let out = run(
        &root,
        "f=\"$PWD/$(printf 'bin\\034d')/$(printf 'c\\034m')\"; \"$f\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, b"PAIRCMD-RAN\nrc=0\n");
    no_pair(&out);
}

#[test]
fn exec_builtin_runs_byte_named_executable() {
    let root = fixture("execb");
    let out = run(
        &root,
        "n=\"$PWD/$(printf 'bin\\034d')/$(printf 'c\\034m')\"; ( exec \"$n\" ); echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, b"PAIRCMD-RAN\nrc=0\n");
    no_pair(&out);
}

#[test]
fn type_reports_raw_byte_path() {
    let root = fixture("type");
    let out = run(
        &root,
        "d=$(printf 'bin\\034d'); PATH=\"$PWD/$d:$PATH\"; type mycmd\n",
    );
    let expect = format!("mycmd is {}/bin\x1cd/mycmd\n", root.display()).into_bytes();
    assert_eq!(out.stdout, expect);
    no_pair(&out);
}

#[test]
fn command_not_found_diag_quotes_the_raw_byte() {
    let root = fixture("notfound");
    let out = run(&root, "n=$(printf 'z\\0349'); \"$n\"; echo \"rc=$?\"\n");
    assert_eq!(out.stdout, b"rc=127\n");
    assert!(
        out.stderr
            .windows(b"$'z\\0349': command not found".len())
            .any(|w| w == b"$'z\\0349': command not found"),
        "stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    no_pair(&out);
}

// ---- glob literal segments ----

#[test]
fn glob_literal_segment_matches_byte_named_entry() {
    let root = fixture("glob");
    let out = run(&root, "n=$(printf 'a\\034'); printf '<%s>\\n' \"$n\"*\n");
    assert_eq!(out.stdout, b"<a\x1cb>\n");
    no_pair(&out);
}

// ---- bind -f / history -w file operands ----

#[test]
fn bind_f_reads_byte_named_file() {
    let root = fixture("bindf");
    let out = run(
        &root,
        "n=$(printf 'b\\034f'); bind -f \"$n\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, b"rc=0\n");
    no_pair(&out);
}

#[test]
fn history_write_lands_raw_byte_name() {
    let root = fixture("histw");
    let out = run(
        &root,
        "set -o history; history -s probeentry; n=$(printf 'h\\034f'); history -w \"$n\"; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, b"rc=0\n");
    let names = entry_names(&root);
    assert!(
        names.iter().any(|n| n == b"h\x1cf"),
        "raw byte history file missing: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.windows(3).any(|w| w == PAIR_HEAD)),
        "pair-spelled name landed: {names:?}"
    );
    no_pair(&out);
}

// ---- redirect-chain readers ----

#[test]
fn comsub_readfile_reads_byte_named_file() {
    let root = fixture("readfile");
    let out = run(
        &root,
        "n=$(printf 'a\\034b'); v=$(< \"$n\"); echo \"[$v]\"\n",
    );
    assert_eq!(out.stdout, b"[AB]\n");
    no_pair(&out);
}

#[test]
fn read_builtin_redirect_reads_byte_named_file() {
    let root = fixture("readredir");
    let out = run(
        &root,
        "n=$(printf 'a\\034b'); read -r line < \"$n\"; echo \"[$line]\"\n",
    );
    assert_eq!(out.stdout, b"[AB]\n");
    no_pair(&out);
}

// ---- controls: plain names and the genuine U+E000 char ----

#[test]
fn plain_name_controls_unchanged() {
    let root = fixture("plain");
    let out = run(
        &root,
        "printf X > plain1; test -d plaindir && echo TD-YES; source plaindir/f.sh; cd plaindir && echo CD-IN\n",
    );
    assert_eq!(out.stdout, b"TD-YES\nSOURCED-PLAIN\nCD-IN\n");
    let names = entry_names(&root);
    assert!(names.iter().any(|n| n == b"plain1"), "{names:?}");
}

#[test]
fn literal_ue000_char_name_still_resolves() {
    let root = fixture("ue000");
    let out = run(
        &root,
        "n=$'u\\uE000v'; test -f \"$n\" && echo UE-YES || echo UE-NO; printf Q > $'w\\uE000x'; echo \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, b"UE-YES\nrc=0\n");
    let names = entry_names(&root);
    assert!(
        names.iter().any(|n| n == "w\u{E000}x".as_bytes()),
        "char-named file missing: {names:?}"
    );
}
