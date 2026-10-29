//! Probe battery for rubash#2036 S1 (registry data first, zero behavior
//! change): pins the CURRENT precedence and introspection behavior of the
//! emulated / fast-path command faces exactly as measured by the #2036
//! audit (audit §4 precedence matrix, probes2.sh on master 5ec322cb;
//! audit §3.1 introspection run, probes.sh). Expectations below are the
//! engine's measured behavior, INCLUDING its warts — each wart is marked
//! WART with the slice that will adjudicate it (S2 introspection, S3/S4
//! stand-down two-axis policy). Nothing here asserts GNU behavior where
//! the engine currently diverges; the corresponding GNU expectations are
//! the audit's §3.3 ledger and land with the S5 corpus.
//!
//! Unix-only: the fake-body shapes are PATH shell scripts and the stty
//! expectations are the Linux ioctl texts the audit measured. The
//! Windows-side three-way differential battery is audit win-diff/ (pkg10).
//!
//! S2 update: the introspection section no longer pins the
//! zero-disclosure contract. Under the S2 ruling the describe faces
//! disclose a live in-process claim (`type -t` -> `emulation`,
//! `type X` -> "X is a shell emulation of <path>"), and the KD-1
//! drift pins (compgen -b / enable lacking `env`) turned positive:
//! every introspection face projects from the single registry.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
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
        .join("rubash")
}

struct Output {
    stdout: String,
    stderr: String,
}

/// Run one `-c` script in `dir` with an explicit PATH (the audit battery
/// drives every shape through PATH control; the ambient PATH is never
/// inherited so a stray hashed/fake entry cannot leak between shapes).
fn run(dir: &Path, path: &str, script: &str) -> Output {
    let output = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(dir)
        .env("PATH", path)
        .env_remove("BASH_ENV")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Output {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2036-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn write_fake(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, body).expect("write fake");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fake");
}

/// fakebin/{cat,stty,sleep} shell-script fakes (print FAKE-<name>),
/// realbin = symlinks to the system tools, emptybin, slashbin/cat.
struct Rig {
    root: PathBuf,
    fake_path: String,
}

fn rig(tag: &str) -> Rig {
    let root = scratch(tag);
    let fakebin = root.join("fakebin");
    let realbin = root.join("realbin");
    let emptybin = root.join("emptybin");
    let slashbin = root.join("slashbin");
    for d in [&fakebin, &realbin, &emptybin, &slashbin] {
        fs::create_dir_all(d).expect("rig dirs");
    }
    for name in ["cat", "stty", "sleep"] {
        write_fake(&fakebin, name, &format!("#!/bin/sh\necho FAKE-{name}\n"));
        let real = Command::new("which")
            .arg(name)
            .output()
            .expect("which")
            .stdout;
        let real = String::from_utf8_lossy(&real).trim().to_string();
        std::os::unix::fs::symlink(&real, realbin.join(name)).expect("realbin symlink");
    }
    write_fake(&slashbin, "cat", "#!/bin/sh\necho SLASHFAKE-cat\n");
    fs::write(root.join("f1.txt"), "alpha\nbeta\n").expect("f1");
    Rig {
        fake_path: format!("{}:/usr/bin:/bin", fakebin.display()),
        root,
    }
}

const IOCTL: &str = "stty: 'standard input': Inappropriate ioctl for device";

/// Absolute path of the system `head`, resolved before any PATH games —
/// the audit battery pipes through `"$HEAD"` (a slash form) so the
/// downstream stage never enters the engine's pipeline-arm path and the
/// shape measures only the upstream name's precedence.
fn head_abs() -> String {
    let out = Command::new("which")
        .arg("head")
        .output()
        .expect("which head")
        .stdout;
    String::from_utf8_lossy(&out).trim().to_string()
}

// ------------------------------------------------------------ §4 shape 1

/// Shape 1 (audit §4): plain call, script fake first on PATH.
/// stty and sleep are answered by the engine even though a resolvable
/// body exists (existence gate, not yield-to-real). The cat leg of
/// this shape is a PIPELINE form (`cat f1.txt | head -1`): its yield
/// to the fake is the pipeline planner's, not the simple face's —
/// attribution corrected by the S4 four-quadrant re-measurement
/// (03ae580d): the simple face replaces BOTH fake shapes
/// (shape-blind, see shape 1'), the pipeline face yields to BOTH
/// via the concurrent direct-spawn planner. The audit's original
/// "simple face yields to a script fake" reading compared this
/// pipeline leg against a simple-form leg and is superseded.
/// WART (stty/sleep): S3/S4 stand-down adjudication.
#[test]
fn issue2036_shape1_plain_script_fake() {
    let rig = rig("s1");
    let head = head_abs();
    let out = run(
        &rig.root,
        &rig.fake_path,
        &format!("cat f1.txt | {head} -1"),
    );
    assert_eq!(out.stdout, "FAKE-cat\n", "stderr: {}", out.stderr);
    let out = run(&rig.root, &rig.fake_path, "stty size 2>&1");
    assert!(out.stdout.contains(IOCTL), "got: {}", out.stdout);
    let out = run(&rig.root, &rig.fake_path, "sleep 0.01; echo rc=$?");
    assert_eq!(
        out.stdout, "rc=0\n",
        "fake sleep must not run: {}",
        out.stdout
    );
}

/// Shape 1' (audit §4/B5): same shape with an ELF fake (a copy of
/// /bin/true), SIMPLE form. cat is answered by the engine (prints
/// the file; the ELF fake prints nothing). Attribution corrected
/// (S4 four-quadrant re-measurement, 03ae580d): the simple face is
/// SHAPE-BLIND — it replaces script fakes exactly as it replaces
/// ELF fakes; no layer distinguishes script vs ELF candidates. The
/// audit's contrary reading rested on shape 1's cat leg, which is
/// a pipeline form (see shape 1). The pipeline face yields to both
/// shapes (see the B5 pin below). WART: S3/S4 adjudication.
#[test]
fn issue2036_shape1_elf_fake_cat_is_replaced() {
    let rig = rig("s1e");
    let elf = rig.root.join("fakebin/cat");
    fs::copy("/bin/true", &elf).expect("elf fake");
    fs::set_permissions(&elf, fs::Permissions::from_mode(0o755)).expect("chmod elf");
    let out = run(&rig.root, &rig.fake_path, "cat f1.txt");
    assert_eq!(out.stdout, "alpha\nbeta\n", "stderr: {}", out.stderr);
}

// ------------------------------------------------------- §4 shapes 3 / 4

/// Shape 3: PATH set but holds none of the names — every face stands
/// down and the spawn fails 127 (the #1545-aligned failure side).
#[test]
fn issue2036_shape3_empty_path_stands_down() {
    let rig = rig("s3");
    let empty = rig.root.join("emptybin").display().to_string();
    let out = run(
        &rig.root,
        &empty,
        "cat f1.txt 2>&1; echo cat_rc=$?; stty size 2>&1; echo stty_rc=$?; \
         sleep 0.01 2>&1; echo sleep_rc=$?",
    );
    assert!(
        out.stdout.contains("cat: command not found"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("cat_rc=127"), "{}", out.stdout);
    assert!(
        out.stdout.contains("stty: command not found"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("stty_rc=127"), "{}", out.stdout);
    assert!(
        out.stdout.contains("sleep: command not found"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("sleep_rc=127"), "{}", out.stdout);
}

/// Shape 4: PATH unset — same 127 stand-down, with the engine's
/// ENOENT-flavored diagnostic (audit matrix, measured verbatim).
#[test]
fn issue2036_shape4_unset_path_stands_down() {
    let rig = rig("s4");
    let out = run(
        &rig.root,
        &rig.fake_path,
        "unset PATH; cat f1.txt 2>&1; echo cat_rc=$?; stty size 2>&1; \
         echo stty_rc=$?; sleep 0.01 2>&1; echo sleep_rc=$?",
    );
    assert!(
        out.stdout.contains("cat: No such file or directory"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("cat_rc=127"), "{}", out.stdout);
    assert!(out.stdout.contains("stty_rc=127"), "{}", out.stdout);
    assert!(out.stdout.contains("sleep_rc=127"), "{}", out.stdout);
}

// ------------------------------------------------- §4 shapes 5 / 8 / 9

/// Shapes 5/8/9: a defined function shadows every face — plain call,
/// pipeline stage, and command substitution alike (audit §4: all three
/// shapes FUNC for cat and stty; function shadow is uniform here).
#[test]
fn issue2036_shapes589_function_shadows_all_faces() {
    let rig = rig("s589");
    let out = run(
        &rig.root,
        &rig.fake_path,
        "cat() { echo FUNC-cat; }; stty() { echo FUNC-stty; }; \
         sleep() { echo FUNC-sleep; }; \
         echo plain_cat=$(cat f1.txt); echo plain_stty=$(stty size); \
         echo plain_sleep=$(sleep 9); \
         echo pipe_cat=$(printf 'p1\\np2\\n' | cat); \
         echo pipe_stty=$(printf 'x\\n' | stty size); \
         echo comsub_cat=$(cat f1.txt); echo comsub_stty=$(stty size)",
    );
    for want in [
        "plain_cat=FUNC-cat",
        "plain_stty=FUNC-stty",
        "plain_sleep=FUNC-sleep",
        "pipe_cat=FUNC-cat",
        "pipe_stty=FUNC-stty",
        "comsub_cat=FUNC-cat",
        "comsub_stty=FUNC-stty",
    ] {
        assert!(
            out.stdout.contains(want),
            "missing {want} in {}",
            out.stdout
        );
    }
}

// ------------------------------------------------------- §4 shapes 6 / 7

/// Shape 6 (audit §4): `command` prefix. cat and stty are STILL
/// answered by the engine (the fake does not run), while sleep escapes
/// to a real spawn of the fake — the same shape gives three different
/// answers across names. WART (inconsistent interception): S3/S4.
#[test]
fn issue2036_shape6_command_prefix_split_answers() {
    let rig = rig("s6");
    let head = head_abs();
    let out = run(
        &rig.root,
        &rig.fake_path,
        &format!(
            "echo ccat=$(command cat f1.txt | {head} -1); \
             echo cstty=$(command stty size 2>&1); command sleep 0.01; echo csleep_rc=$?"
        ),
    );
    assert!(out.stdout.contains("ccat=alpha"), "{}", out.stdout);
    assert!(out.stdout.contains(IOCTL), "{}", out.stdout);
    assert!(
        out.stdout.contains("FAKE-sleep"),
        "command sleep must spawn the fake: {}",
        out.stdout
    );
    assert!(out.stdout.contains("csleep_rc=0"), "{}", out.stdout);
}

/// Shape 7: `env` prefix spawns for real (functions do not apply).
#[test]
fn issue2036_shape7_env_prefix_spawns() {
    let rig = rig("s7");
    let head = head_abs();
    let out = run(
        &rig.root,
        &rig.fake_path,
        &format!("cat() {{ echo FUNC-cat; }}; echo ecat=$(env cat f1.txt | {head} -1)"),
    );
    assert!(out.stdout.contains("ecat=FAKE-cat"), "{}", out.stdout);
}

// ------------------------------------------------- §4 shapes 10–13

/// Shape 10: running cat (here the script fake, spawned) leaves a
/// `hash -l` record naming the resolved PATH candidate; switching PATH
/// to realbin makes the real body answer without needing `hash -r`
/// first (same outcome as GNU here; mechanism unpinned, audit §4).
#[test]
fn issue2036_shape10_hash_record_and_path_switch() {
    let rig = rig("s10");
    let realbin = rig.root.join("realbin").display().to_string();
    let head = head_abs();
    let script = format!(
        "cat f1.txt >/dev/null; hash -l | grep cat; \
         export PATH={realbin}; echo after_switch=$(cat f1.txt | {head} -1); \
         hash -r; echo after_hash_r=$(cat f1.txt | {head} -1)"
    );
    let out = run(&rig.root, &rig.fake_path, &script);
    assert!(
        out.stdout.contains("builtin hash -p") && out.stdout.contains("/fakebin/cat cat"),
        "hash record missing: {}",
        out.stdout
    );
    assert!(out.stdout.contains("after_switch=alpha"), "{}", out.stdout);
    assert!(out.stdout.contains("after_hash_r=alpha"), "{}", out.stdout);
}

/// Shape 11: slash form pointing at the fake runs the fake.
#[test]
fn issue2036_shape11_slash_form_runs_fake() {
    let rig = rig("s11");
    let slash = rig.root.join("slashbin/cat").display().to_string();
    let out = run(&rig.root, &rig.fake_path, &format!("{slash} f1.txt"));
    assert_eq!(out.stdout, "SLASHFAKE-cat\n");
}

/// Shape 12: escape hatches (xargs / find -exec) spawn the fake.
#[test]
fn issue2036_shape12_escape_hatches_spawn() {
    let rig = rig("s12");
    let head = head_abs();
    let out = run(
        &rig.root,
        &rig.fake_path,
        &format!(
            "echo xargs_cat=$(printf 'f1.txt\\n' | xargs cat 2>&1 | {head} -1); \
             echo find_cat=$(find . -name f1.txt -exec cat {{}} + 2>&1 | {head} -1)"
        ),
    );
    assert!(out.stdout.contains("xargs_cat=FAKE-cat"), "{}", out.stdout);
    assert!(out.stdout.contains("find_cat=FAKE-cat"), "{}", out.stdout);
}

/// Shape 13 + enable faces: `type -t echo` reports builtin;
/// `enable -n echo` releases echo to the PATH body; `enable stty` /
/// `enable cat` refuse with the not-a-builtin diagnostic (the emulated
/// names are not builtins and stay unmanageable by enable — their
/// claims are disclosed by the describe faces, not the builtin table).
#[test]
fn issue2036_shape13_enable_faces() {
    let rig = rig("s13");
    let out = run(
        &rig.root,
        &rig.fake_path,
        "echo before=$(type -t echo); enable -n echo; \
         echo after=$(type -t echo); enable stty 2>&1; echo stty_rc=$?; \
         enable cat 2>&1; echo cat_rc=$?",
    );
    assert!(out.stdout.contains("before=builtin"), "{}", out.stdout);
    assert!(out.stdout.contains("after=file"), "{}", out.stdout);
    assert!(
        out.stdout.contains("enable: stty: not a shell builtin"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("stty_rc=1"), "{}", out.stdout);
    assert!(
        out.stdout.contains("enable: cat: not a shell builtin"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("cat_rc=1"), "{}", out.stdout);
    // KD-1, closed by S2: env is in the registry projection every
    // face reads, so enable manages it like any other builtin — and
    // the disable is real, not a report-only state: after
    // `enable -n env` the describe face stops reporting the builtin
    // and dispatch falls through to the PATH body (the fakebin script
    // here). Before the disable, the bare name runs the in-process
    // env (its output is the environment, never the fake's marker).
    write_fake(
        &rig.root.join("fakebin"),
        "env",
        "#!/bin/sh\necho FAKE-env\n",
    );
    let out = run(
        &rig.root,
        &rig.fake_path,
        "enable env; echo env_rc=$?; env >/dev/null; echo on_rc=$?; \
         enable -n env; echo envn_rc=$?; echo t=$(type -t env); echo off=$(env)",
    );
    assert!(out.stdout.contains("env_rc=0"), "{}", out.stdout);
    assert!(out.stdout.contains("on_rc=0"), "{}", out.stdout);
    assert!(out.stdout.contains("envn_rc=0"), "{}", out.stdout);
    assert!(out.stdout.contains("t=file"), "{}", out.stdout);
    assert!(out.stdout.contains("off=FAKE-env"), "{}", out.stdout);
}

// ------------------------------------------------------- §3.1 introspection

/// Names whose simple-command claim the shell takes in-process
/// (registry classes HiddenFastpath / Fastpath / EmulatedExternal):
/// while the claim is live they must disclose it (S2 ruling).
const SIMPLE_CLAIM_NAMES: &[&str] = &[
    "stty", "uname", "arch", "sleep", "dirname", "basename", "cat", "sed", "mkdir", "touch",
    "chmod", "cp", "rm", "rmdir", "mkfifo", "tty",
];

/// Names claimed only in other shapes (pipeline stage, command
/// substitution, the diff shortcut) — plus `tput`, whose claim is
/// Windows-only and therefore not live on this unix battery. For all
/// of these the simple-command question honestly gets the file answer.
const SHAPE_SCOPED_NAMES: &[&str] = &[
    "head", "grep", "wc", "tr", "yes", "diff", "mktemp", "sort", "tput",
];

/// §3.1 as adjudicated by S2: the zero-disclosure contract is
/// reversed. A live simple-command claim reports `emulation` on
/// type -t, the sentence "X is a shell emulation of <path>" on
/// type / command -V, and the bare name on command -v (invoking the
/// name reproduces the in-process behavior; invoking the path would
/// not). Shape-scoped claims keep the file answer.
#[test]
fn issue2036_introspection_emulated_names_report_file() {
    let dir = scratch("intro");
    let path = std::env::var("PATH").unwrap_or_default();
    let mut script = String::new();
    for n in SIMPLE_CLAIM_NAMES.iter().chain(SHAPE_SCOPED_NAMES) {
        script.push_str(&format!("echo {n}=\"$(type -t {n} 2>&1)\"\n"));
    }
    let out = run(&dir, &path, &script);
    for n in SIMPLE_CLAIM_NAMES {
        assert!(
            out.stdout.contains(&format!("{n}=emulation")),
            "{n} must disclose its live claim, got: {}",
            out.stdout
        );
    }
    for n in SHAPE_SCOPED_NAMES {
        assert!(
            out.stdout.contains(&format!("{n}=file")),
            "{n} must introspect as file, got: {}",
            out.stdout
        );
    }
    // The verbose and reusable forms of the disclosure.
    let out = run(
        &dir,
        &path,
        "type stty; command -V cat; echo v=$(command -v stty); type -p stty; echo p_rc=$?",
    );
    assert!(
        out.stdout.contains("stty is a shell emulation of /"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("cat is a shell emulation of /"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("v=stty"), "{}", out.stdout);
    // type -p on a claim is silent-but-found, like a builtin: the
    // whole run prints exactly the four expected lines.
    assert!(out.stdout.contains("p_rc=0"), "{}", out.stdout);
    assert_eq!(out.stdout.lines().count(), 4, "{}", out.stdout);
    // type -a prints the claim line AND the file line(s).
    let out = run(&dir, &path, "type -a stty");
    assert!(
        out.stdout.contains("stty is a shell emulation of /"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("stty is /"), "{}", out.stdout);
    // Stand-down honesty: when the name does not resolve, no claim is
    // reported — the not-found answer returns (the claim is an
    // existence gate, exactly as dispatch behaves).
    let empty = scratch("introempty");
    let out = run(
        &dir,
        empty.to_str().unwrap(),
        "echo t=\"$(type -t stty)\"; type stty 2>/dev/null; echo rc=$?",
    );
    assert!(out.stdout.contains("t=\n"), "{}", out.stdout);
    assert!(out.stdout.contains("rc=1"), "{}", out.stdout);
}

/// §3.1: `env` is the one honest exception — a real (rubash-extension)
/// builtin, reported as such on every introspection face.
#[test]
fn issue2036_introspection_env_reports_builtin() {
    let dir = scratch("introenv");
    let path = std::env::var("PATH").unwrap_or_default();
    let out = run(
        &dir,
        &path,
        "type env; echo t=$(type -t env); echo v=$(command -v env)",
    );
    assert!(
        out.stdout.contains("env is a shell builtin"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("t=builtin"), "{}", out.stdout);
    assert!(out.stdout.contains("v=env"), "{}", out.stdout);
}

/// §3.1 + audit M23, as adjudicated by S2 (KD-1 closed): `compgen -b`
/// projects from the same registry as every other introspection
/// face — the hand-maintained second list that lacked `env` is gone,
/// so the full 62-name builtin set (GNU 61 + the env extension) is
/// listed. (This pin was the drift pin in S1; it is positive now.)
#[test]
fn issue2036_introspection_compgen_b_known_drift_env() {
    let dir = scratch("introcg");
    let path = std::env::var("PATH").unwrap_or_default();
    let out = run(&dir, &path, "compgen -b | sort | tr '\\n' ' '; echo");
    let expected = ". : [ alias bg bind break builtin caller cd command compgen complete \
                    compopt continue declare dirs disown echo enable env eval exec exit export \
                    false fc fg getopts hash help history jobs kill let local logout mapfile \
                    popd printf pushd pwd read readarray readonly return set shift shopt \
                    source suspend test times trap true type typeset ulimit umask unalias \
                    unset wait ";
    assert_eq!(
        out.stdout.trim_end(),
        expected.trim_end(),
        "compgen -b projection changed"
    );
}

/// §3.1 + audit M23 (extended by S1), as adjudicated by S2 (KD-1
/// closed): `enable -a` projects from the registry like every other
/// face — 62 names including `env`, and `enable env` / `enable -n env`
/// manage it (pinned in the shape-13 test). The claimed-but-not-builtin
/// names stay unlisted: they are not builtins.
#[test]
fn issue2036_introspection_enable_a_lists_builtins_only() {
    let dir = scratch("introena");
    let path = std::env::var("PATH").unwrap_or_default();
    let out = run(&dir, &path, "enable -a");
    let names: Vec<&str> = out
        .stdout
        .lines()
        .filter_map(|l| {
            l.strip_prefix("enable ")
                .or_else(|| l.strip_prefix("enable -n "))
        })
        .collect();
    assert_eq!(names.len(), 62, "enable -a rows: {:?}", names);
    assert!(
        names.contains(&"env"),
        "env must be listed (KD-1 closed): {:?}",
        names
    );
    for n in ["stty", "cat", "sleep", "uname"] {
        assert!(!names.contains(&n), "{n} must not be listed");
    }
}

/// §3.1: `time` introspects as a keyword (it is never the WinuxCmd
/// applet inside the shell).
#[test]
fn issue2036_introspection_time_reports_keyword() {
    let dir = scratch("introtime");
    let path = std::env::var("PATH").unwrap_or_default();
    let out = run(&dir, &path, "type time; echo t=$(type -t time)");
    assert!(
        out.stdout.contains("time is a shell keyword"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("t=keyword"), "{}", out.stdout);
}

/// §1.4 hash-record face: an emulated execution writes a
/// PATH-resolution record into the hash table (the record names the
/// PATH candidate, not the in-process fact — introspection's only
/// half-disclosure). Both names are made resolvable-but-replaced:
/// mkdir's candidate is a script (the external layer answers script
/// candidates in-process) and cat's is an ELF copy of /bin/true
/// (shape 1': the engine replaces ELF candidates, so the record here
/// cannot be ordinary spawn hashing — nothing was spawned).
#[test]
fn issue2036_hash_record_after_emulated_runs() {
    let rig = rig("hash");
    write_fake(
        &rig.root.join("fakebin"),
        "mkdir",
        "#!/bin/sh\necho FAKE-mkdir\n",
    );
    let elf = rig.root.join("fakebin/cat");
    fs::copy("/bin/true", &elf).expect("elf fake");
    fs::set_permissions(&elf, fs::Permissions::from_mode(0o755)).expect("chmod elf");
    let out = run(
        &rig.root,
        &rig.fake_path,
        "mkdir newdir; cat f1.txt >/dev/null; hash -l",
    );
    assert!(
        rig.root.join("newdir").is_dir(),
        "emulated mkdir must create the directory"
    );
    assert!(
        out.stdout.contains("/fakebin/mkdir mkdir"),
        "hash record for mkdir: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("/fakebin/cat cat"),
        "hash record for cat: {}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("FAKE-mkdir"),
        "fake mkdir must not run"
    );
}

// ------------------------------------------------------- stand-down warts

/// Audit B1 (§3.3), S3-adjudicated FailClosed and FIXED in S5
/// (rubash#2046): the comsub basename arm now consults the same
/// stand-down predicate as the cat arm. With PATH=/nonexistent the
/// substitution fails closed — empty value, status 127 — exactly
/// like the simple-command form of the same name and like the comsub
/// dirname shape (no arm). This pin replaces the wart pin that held
/// the pre-fix answer (`comsub_basename=c`) until adjudication.
#[test]
fn issue2036_comsub_basename_stands_down() {
    let dir = scratch("w1");
    let out = run(
        &dir,
        "/nonexistent",
        "comsub_basename=$(basename /a/b/c); \
         echo \"val=[$comsub_basename] rc=$?\"; \
         basename /a/b/c 2>&1; echo simp_rc=$?; \
         echo comsub_dirname=$(dirname /a/b/c 2>&1); echo done",
    );
    assert!(
        out.stdout.contains("val=[] rc=127"),
        "comsub basename must fail closed (empty value, status 127): {}",
        out.stdout
    );
    assert!(out.stdout.contains("simp_rc=127"), "{}", out.stdout);
    assert!(
        !out.stdout.contains("comsub_dirname=/a/b"),
        "dirname comsub arm must not answer: {}",
        out.stdout
    );
}

/// The comsub cat arms DO consult stand-down: with PATH=/nonexistent
/// the substitution comes back empty with status 127.
#[test]
fn issue2036_comsub_cat_stands_down() {
    let rig = rig("w2");
    let out = run(
        &rig.root,
        "/nonexistent",
        "x=$(cat f1.txt); echo rc=$?; echo \"x=[$x]\"",
    );
    assert!(out.stdout.contains("rc=127"), "{}", out.stdout);
    assert!(out.stdout.contains("x=[]"), "{}", out.stdout);
}

/// Audit B5 (§3.3): pipeline stage with an ELF fake — the pipeline
/// yields to the fake (wc counts the fake's empty output), the exact
/// opposite of the simple-command form in shape 1'. Attribution
/// (S4): the yield belongs to the pipeline face's concurrent
/// direct-spawn planner, and it is shape-blind — script fakes yield
/// there too; the simple face replaces both shapes. S3 §3.2-a RULED
/// this cell KEEP (both faces keep their current values), so the
/// pin now guards the ruled shape rather than an open wart.
#[test]
fn issue2036_wart_pipeline_cat_yields_to_elf_fake() {
    let rig = rig("w5");
    let elf = rig.root.join("fakebin/cat");
    fs::copy("/bin/true", &elf).expect("elf fake");
    fs::set_permissions(&elf, fs::Permissions::from_mode(0o755)).expect("chmod elf");
    let out = run(&rig.root, &rig.fake_path, "cat f1.txt | wc -l");
    assert_eq!(out.stdout.trim(), "0", "stderr: {}", out.stderr);
}
