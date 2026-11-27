//! Equivalence battery for rubash#2036 S4: the pipeline and
//! command-substitution faces now take their claim sets from the
//! registry slot tables (PIPELINE_STAGE / PIPELINE_PRECHECK /
//! PIPELINE_STANDIN / TIMED_PRODUCER / TIMED_CONSUMER /
//! TIMED_FOLLOWUP / COMSUB_WORD / COMSUB_TIMED / COMSUB_HEREDOC /
//! COMSUB_FRONT) instead of hand-written name lists. The
//! projection moved only the claim keys; every arm body is
//! verbatim. These pins assert the CURRENT behavior of each
//! projected face, measured on the post-projection tree (master
//! 03ae580d + S4) — including the per-face rules the S4 field sink
//! registered (the sequential-route cat arm replacing, the mktemp
//! word arm answering for an unresolvable name).
//!
//! Unix-only: the fake-body shapes are PATH shell scripts. The
//! Windows-only faces (the tput stage arm and the comsub front
//! tput arm) have no pin here; they are compile-covered by the
//! Windows CI and behavior-covered by the pkg10 win-diff cell.
//!
//! Execution note (same disposition as the S1/S2/S3 batteries):
//! integration test targets are not in CI's executed target list,
//! so these pins' execution evidence is local; CI covers their
//! compilation plus the lib consistency pins.

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
    let dir = std::env::temp_dir().join(format!("rubash-2036-s4-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// fakebin (a script `cat` printing FAKE-cat), emptybin, and a
/// two-line f1.txt in the root.
struct Rig {
    root: PathBuf,
    fake_path: String,
    real_path: String,
    empty_path: String,
}

fn rig(tag: &str) -> Rig {
    let root = scratch(tag);
    let fakebin = root.join("fakebin");
    let emptybin = root.join("emptybin");
    fs::create_dir_all(&fakebin).expect("fakebin");
    fs::create_dir_all(&emptybin).expect("emptybin");
    fs::write(root.join("f1.txt"), "alpha\nbeta\n").expect("f1");
    let fake = fakebin.join("cat");
    fs::write(&fake, "#!/bin/sh\necho FAKE-cat\n").expect("fake cat");
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod fake");
    Rig {
        fake_path: format!("{}:/usr/bin:/bin", fakebin.display()),
        real_path: "/usr/bin:/bin".to_string(),
        empty_path: format!("{}", emptybin.display()),
        root,
    }
}

// ------------------------------------------------------- pipeline stages

#[test]
fn s4_stage_arms_answer_their_shapes() {
    let rig = rig("stage-arms");
    let out = run(&rig.root, &rig.real_path, "echo hi | grep hi");
    assert_eq!(out.stdout, "hi\n");
    let out = run(&rig.root, &rig.real_path, "echo hi | grep bye; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n");
    let out = run(&rig.root, &rig.real_path, "printf 'a\\nb\\nc\\n' | head -1");
    assert_eq!(out.stdout, "a\n");
    let out = run(&rig.root, &rig.real_path, "printf 'a\\nb\\n' | wc -l");
    assert_eq!(out.stdout, "2\n");
    let out = run(&rig.root, &rig.real_path, "echo abc | tr a-z A-Z");
    assert_eq!(out.stdout, "ABC\n");
    let out = run(&rig.root, &rig.real_path, "echo hello | sed s/l/L/");
    assert_eq!(out.stdout, "heLlo\n");
    // The trap stage arm answers a `-p` query inline. (A bare
    // `trap | ...` print is not pinned here: its table includes
    // signal dispositions inherited from the parent process, and
    // this binary's in-process Session tests change the host
    // process's dispositions — an engine host-visible side effect
    // that predates S4 and is orthogonal to the projection.)
    let out = run(
        &rig.root,
        &rig.real_path,
        "trap 'echo X' EXIT; trap -p EXIT | head -1",
    );
    assert_eq!(out.stdout, "trap -- 'echo X' EXIT\nX\n");
}

#[test]
fn s4_stage_true_false_status() {
    let rig = rig("stage-status");
    let out = run(
        &rig.root,
        &rig.real_path,
        "false | true; echo rc=$?; true | false; echo rc=$?",
    );
    assert_eq!(out.stdout, "rc=0\nrc=1\n");
}

#[test]
fn s4_stage_precheck_declines_unresolvable_to_127() {
    // PIPELINE_PRECHECK: with head unresolvable the inline arm
    // stands down and the external stage reports the failure.
    let rig = rig("stage-precheck");
    let out = run(&rig.root, &rig.empty_path, "echo hi | head -1; echo rc=$?");
    assert_eq!(out.stdout, "rc=127\n");
    assert!(
        out.stderr.contains("head: command not found"),
        "{:?}",
        out.stderr
    );
}

#[test]
fn s4_stage_cat_arm_replaces_on_the_sequential_route() {
    // The per-face rule registered in S4 (cat row, PipelineStage
    // face): the concurrent planner yields to a resolvable
    // candidate, but on the sequential route — forced here by the
    // fd-2 redirect, which the planner declines — the inline arm
    // itself answers in-process, so the PATH script fake does NOT
    // run and wc counts the file's own two lines.
    let rig = rig("stage-cat-seq");
    let out = run(&rig.root, &rig.fake_path, "cat f1.txt 2>/dev/null | wc -l");
    assert_eq!(out.stdout, "2\n");
}

#[test]
fn s4_stage_standin_yes() {
    let rig = rig("stage-standin");
    let out = run(&rig.root, &rig.real_path, "yes | head -2");
    assert_eq!(out.stdout, "y\ny\n");
}

// ------------------------------------------------------- timed models

#[test]
fn s4_timed_consumer_times_out_142() {
    let rig = rig("timed-timeout");
    let out = run(
        &rig.root,
        &rig.real_path,
        "sleep 1 | read -t 0.05 a; echo rc=$?",
    );
    assert_eq!(out.stdout, "rc=142\n");
}

#[test]
fn s4_timed_producer_consumer_and_followup() {
    let rig = rig("timed-followup");
    // Brace-group consumer: the modeled producer's chunk arrives
    // before the deadline and the followup echo (a TimedFollowup
    // claim) keeps the stream.
    let out = run(
        &rig.root,
        &rig.real_path,
        "(sleep 0.05; echo hi) | { read -t 2 x; echo \"got $x\"; }",
    );
    assert_eq!(out.stdout, "got hi\n");
    // Bare read as the last stage: the model engages (status 0,
    // not the 142 timeout) — the value itself lands in the stage's
    // own shell, as in GNU without lastpipe.
    let out = run(
        &rig.root,
        &rig.real_path,
        "(sleep 0.1; echo hi) | read -t 2 x; echo rc=$?",
    );
    assert_eq!(out.stdout, "rc=0\n");
}

// ------------------------------------------------------- comsub word arms

#[test]
fn s4_comsub_word_echo_printf() {
    let rig = rig("word-basic");
    let out = run(&rig.root, &rig.real_path, "x=$(echo hi); echo \"[$x]\"");
    assert_eq!(out.stdout, "[hi]\n");
    let out = run(
        &rig.root,
        &rig.real_path,
        "x=$(printf 'a%s' b); echo \"[$x]\"",
    );
    assert_eq!(out.stdout, "[ab]\n");
}

#[test]
fn s4_comsub_word_cat_and_its_stand_down() {
    let rig = rig("word-cat");
    let out = run(&rig.root, &rig.real_path, "x=$(cat f1.txt); echo \"[$x]\"");
    assert_eq!(out.stdout, "[alpha\nbeta]\n");
    // The word arm checks the stand-down predicate: unresolvable
    // cat declines through the chain to full execution's 127.
    let out = run(
        &rig.root,
        &rig.empty_path,
        "x=$(cat f1.txt); echo \"rc=$? [$x]\"",
    );
    assert_eq!(out.stdout, "rc=127 []\n");
    assert!(
        out.stderr.contains("cat: command not found"),
        "{:?}",
        out.stderr
    );
}

#[test]
fn s4_comsub_word_builtin_arms() {
    let rig = rig("word-builtins");
    let out = run(
        &rig.root,
        &rig.real_path,
        "x=$(basename /a/b.txt); echo \"[$x]\"",
    );
    assert_eq!(out.stdout, "[b.txt]\n");
    let out = run(
        &rig.root,
        &rig.real_path,
        "umask 027; x=$(umask); echo \"[$x]\"",
    );
    assert_eq!(out.stdout, "[0027]\n");
    let out = run(&rig.root, &rig.real_path, "x=$(pwd); echo \"[$x]\"");
    assert_eq!(out.stdout, format!("[{}]\n", rig.root.display()));
    let out = run(
        &rig.root,
        &rig.real_path,
        "x=$(type -t echo); echo \"[$x]\"",
    );
    assert_eq!(out.stdout, "[builtin]\n");
    let out = run(&rig.root, &rig.real_path, "x=$(kill -l 9); echo \"[$x]\"");
    assert_eq!(out.stdout, "[KILL]\n");
    let out = run(
        &rig.root,
        &rig.real_path,
        "a=$(ulimit -n); b=$(ulimit -n); [ \"$a\" = \"$b\" ] && echo same",
    );
    assert_eq!(out.stdout, "same\n");
}

#[test]
fn s4_comsub_word_mktemp_answers_even_when_unresolvable() {
    // The per-face rule registered in S4 (mktemp row, ComsubWord
    // face): admission is by call shape alone — the arm answers
    // with no mktemp on PATH at all (Unchecked), and of course
    // with one (Replace).
    let rig = rig("word-mktemp");
    let out = run(
        &rig.root,
        &rig.real_path,
        "x=$(mktemp); [ -f \"$x\" ] && echo made; rm -f \"$x\"",
    );
    assert_eq!(out.stdout, "made\n");
    let out = run(
        &rig.root,
        &rig.real_path,
        "p=$PATH; PATH=/nonexistent-empty; x=$(mktemp); PATH=$p; [ -f \"$x\" ] && echo made; rm -f \"$x\"",
    );
    assert_eq!(out.stdout, "made\n");
}

#[test]
fn s4_comsub_exact_body_slots() {
    // The pre-gate exact-body shortcuts key on the same COMSUB_WORD
    // projection (False / True / Colon slots).
    let rig = rig("word-exact");
    let out = run(
        &rig.root,
        &rig.real_path,
        "$(true); echo rc=$?; $(false); echo rc=$?; $(:); echo rc=$?",
    );
    assert_eq!(out.stdout, "rc=0\nrc=1\nrc=0\n");
}

// ------------------------------------------- comsub timed / heredoc / front

#[test]
fn s4_comsub_timed_arms() {
    let rig = rig("timed-arms");
    let out = run(
        &rig.root,
        &rig.real_path,
        "x=$(time echo hi); echo \"[$x]\"",
    );
    assert_eq!(out.stdout, "[hi]\n");
    assert!(out.stderr.contains("real"), "time report: {:?}", out.stderr);
    let out = run(
        &rig.root,
        &rig.real_path,
        "x=$(time cat f1.txt); echo \"[$x]\"",
    );
    assert_eq!(out.stdout, "[alpha\nbeta]\n");
    // The timed Tty arm: not a tty, status 1, silent forms aside.
    let out = run(
        &rig.root,
        &rig.real_path,
        "x=$(time tty); echo \"rc=$? [$x]\"",
    );
    assert_eq!(out.stdout, "rc=1 [not a tty]\n");
}

#[test]
fn s4_comsub_front_tty_bare_and_slash_forms() {
    // COMSUB_FRONT: the front face's tty stdio special case covers
    // the bare name and both registered slash forms identically.
    let rig = rig("front-tty");
    let out = run(
        &rig.root,
        &rig.real_path,
        "x=$(tty); echo \"rc=$? [$x]\"; y=$(/bin/tty); echo \"rc=$? [$y]\"",
    );
    assert_eq!(out.stdout, "rc=1 [not a tty]\nrc=1 [not a tty]\n");
}

#[test]
fn s4_comsub_heredoc_cat_and_sort_u() {
    let rig = rig("heredoc");
    let out = run(
        &rig.root,
        &rig.real_path,
        "x=$(cat <<EOF\nhello\nEOF\n); echo \"[$x]\"",
    );
    assert_eq!(out.stdout, "[hello]\n");
    let out = run(
        &rig.root,
        &rig.real_path,
        "x=$(cat <<EOF | sort -u\nb\na\nb\nEOF\n); echo \"[$x]\"",
    );
    assert_eq!(out.stdout, "[a\nb]\n");
}

// ------------------------------------------- host-flag gates (#2066 arms)
//
// The cat/sed stage arms' host-flag gates ride the projected
// slots unchanged; these pins drive the flag through the Session
// Tier-C seam the way an embedding host would (the #2066 pin
// technique): a PATH probe executable proves which side answered.

use std::sync::Mutex;

use rubash::api::{RunOptions, Session};

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|p| p.into_inner())
}

fn write_probe(bin: &Path, count: &Path, name: &str) {
    let script = format!(
        "#!/bin/sh\necho {name} >> '{count}'\nwhile IFS= read -r line || [ -n \"$line\" ]; do\n  printf 'PROBE-{upper}:%s\\n' \"$line\"\ndone\n",
        name = name,
        count = count.display(),
        upper = name.to_uppercase(),
    );
    let path = bin.join(name);
    fs::write(&path, script).expect("write probe");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod probe");
}

fn gated_session(dir: &Path, path: String, layer_on: bool) -> Session {
    let mut session = Session::builder()
        .env("PATH", path)
        .working_directory(dir.to_path_buf())
        .build()
        .expect("session builds");
    // Purge hash entries leaked through the process-global lookup
    // cache by earlier tests in this binary (see the #2066 file's
    // SERIAL note; the cache itself is rubash#2078's contract-batch
    // item, not S4's).
    let _ = session.run("hash -r", &RunOptions::default());
    if !layer_on {
        session
            .executor_mut()
            .set_external_file_builtins_enabled(false);
    }
    session
}

#[test]
fn s4_pipeline_cat_gate_follows_the_host_flag() {
    let _serial = serial();
    let root = scratch("gate-cat");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("probe bin");
    let count = root.join("count.txt");
    write_probe(&bin, &count, "cat");
    let path = format!("{}:/usr/bin:/bin", bin.display());

    // Flag OFF: the stage arm stands down and the PATH program runs.
    let mut session = gated_session(&root, path.clone(), false);
    let outcome = session
        .run(
            "printf 'hello\\n' | cat > out-off.txt",
            &RunOptions::default(),
        )
        .expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(
        fs::read_to_string(&count).unwrap_or_default(),
        "cat\n",
        "the PATH cat must have run with the flag off"
    );
    assert_eq!(
        fs::read_to_string(root.join("out-off.txt")).expect("out-off"),
        "PROBE-CAT:hello\n"
    );

    // Flag ON (default): the in-process arm answers; probe silent.
    let mut session = gated_session(&root, path, true);
    let outcome = session
        .run(
            "printf 'hello\\n' | cat > out-on.txt",
            &RunOptions::default(),
        )
        .expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(
        fs::read_to_string(&count).unwrap_or_default(),
        "cat\n",
        "no second probe run with the flag on"
    );
    assert_eq!(
        fs::read_to_string(root.join("out-on.txt")).expect("out-on"),
        "hello\n"
    );
}

#[test]
fn s4_pipeline_sed_gate_follows_the_host_flag() {
    let _serial = serial();
    let root = scratch("gate-sed");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("probe bin");
    let count = root.join("count.txt");
    write_probe(&bin, &count, "sed");
    let path = format!("{}:/usr/bin:/bin", bin.display());

    let mut session = gated_session(&root, path.clone(), false);
    let outcome = session
        .run(
            "printf 'a b\\n' | sed 's/b/B/' > out-off.txt",
            &RunOptions::default(),
        )
        .expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(
        fs::read_to_string(&count).unwrap_or_default(),
        "sed\n",
        "the PATH sed must have run with the flag off"
    );
    assert_eq!(
        fs::read_to_string(root.join("out-off.txt")).expect("out-off"),
        "PROBE-SED:a b\n"
    );

    // Flag ON with a PATH-resolvable probe: the probe STILL runs.
    // The sed arm's stand-down is InvertedSed (its registry row):
    // the in-process bridge answers only hash-resolved shapes the
    // PATH walk misses, and the arm's inverted recheck routes a
    // walk hit to the external stage. The gate evidence is the
    // flag-off case above (with the flag off the arm declines
    // before any recheck) plus the cat flag-on case.
    let mut session = gated_session(&root, path, true);
    let outcome = session
        .run(
            "printf 'a b\\n' | sed 's/b/B/' > out-on.txt",
            &RunOptions::default(),
        )
        .expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(
        fs::read_to_string(&count).unwrap_or_default(),
        "sed\nsed\n",
        "the inverted recheck routes the resolvable probe to the external stage"
    );
    assert_eq!(
        fs::read_to_string(root.join("out-on.txt")).expect("out-on"),
        "PROBE-SED:a b\n"
    );
}
