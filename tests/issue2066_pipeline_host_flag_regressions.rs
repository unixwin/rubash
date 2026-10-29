//! Issue rubash#2066 (downstream report: SpookyWaste/dsh-bash-native
//! patch 0003, mechanism layer): the pipeline inline arms for the
//! external-file-builtins layer never read the host flag
//! `external_file_builtins_enabled`. The simple-command face has
//! honored the flag all along (`handle_external_file_builtins`
//! returns not-handled when it is off), so a host that turned the
//! in-process layer off still got the emulated cat/sed inside
//! pipelines — cat unconditionally, sed whenever no sed was
//! resolvable on PATH (its inverted recheck half-masked the gap).
//!
//! Fix: the cat, sed and (Windows) tput stage arms return to the
//! external stage when the flag is off, exactly like the
//! simple-command face. Flag on (the default) is byte-identical to
//! the pre-fix behavior; the guard pins below assert that too.
//!
//! The flag is a host API with no CLI/env face, so the pins drive
//! it through the Session Tier-C seam (`Session::executor_mut`),
//! the way an embedding host would. The "ran the real program"
//! evidence is a PATH probe: an executable named `cat`/`sed` that
//! appends its name to a count file and prefixes every stdin line
//! with a marker — if the stage was answered in-process, the probe
//! never runs and the count file stays empty.
//!
//! Probe pins are `#[cfg(unix)]`: the probes are `/bin/sh` scripts.
//! The tput arm is Windows-only and has no pin here; its gate is
//! the same three lines, compile-covered by the Windows CI.

use std::path::PathBuf;
use std::sync::Mutex;

use rubash::api::{RunOptions, Session};

/// Command resolution keeps a process-global cache with a single
/// fingerprint slot (executor/path.rs, rubash#159), and its
/// `hashed` mirror is shared by every executor state in the
/// process. Parallel sessions with different PATH/PWD fingerprints
/// thrash it and can serve each other's results, so these pins
/// serialize (the same pattern as issue681's SERIAL) and every
/// session starts with `hash -r` to purge leaked hash entries.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|p| p.into_inner())
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2066-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn opts() -> RunOptions {
    RunOptions::default()
}

#[cfg(unix)]
mod probe {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use rubash::api::Session;

    /// A directory holding probe `cat`/`sed` executables plus the
    /// count file they append to.
    pub struct ProbeBin {
        pub dir: PathBuf,
        pub bin: PathBuf,
        pub count: PathBuf,
    }

    fn write_probe(bin: &Path, count: &Path, name: &str) {
        let script = format!(
            "#!/bin/sh\necho {name} >> '{count}'\nwhile IFS= read -r line || [ -n \"$line\" ]; do\n  printf 'PROBE-{upper}:%s\\n' \"$line\"\ndone\n",
            name = name,
            count = count.display(),
            upper = name.to_uppercase(),
        );
        let path = bin.join(name);
        std::fs::write(&path, script).expect("write probe");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod probe");
    }

    pub fn probe_bin(dir: &Path) -> ProbeBin {
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).expect("probe bin dir");
        let count = dir.join("count.txt");
        write_probe(&bin, &count, "cat");
        write_probe(&bin, &count, "sed");
        ProbeBin {
            dir: dir.to_path_buf(),
            bin,
            count,
        }
    }

    impl ProbeBin {
        pub fn count(&self) -> String {
            std::fs::read_to_string(&self.count).unwrap_or_default()
        }

        pub fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
        }

        pub fn path_with_probes(&self) -> String {
            format!(
                "{}:{}",
                self.bin.display(),
                std::env::var("PATH").unwrap_or_default()
            )
        }
    }

    /// Session whose PATH is `path`, with the host flag set as
    /// requested (true = the default layer-on state).
    pub fn session(dir: &Path, path: String, layer_on: bool) -> Session {
        let mut session = Session::builder()
            .env("PATH", path)
            .working_directory(dir.to_path_buf())
            .build()
            .expect("session builds");
        // Purge any hash-table entries leaked through the
        // process-global lookup cache by earlier tests in this
        // binary (see the SERIAL note at the top of the file).
        let _ = session.run("hash -r", &rubash::api::RunOptions::default());
        if !layer_on {
            session
                .executor_mut()
                .set_external_file_builtins_enabled(false);
        }
        session
    }
}

#[cfg(unix)]
#[test]
fn issue2066_flag_off_pipeline_cat_runs_the_path_program() {
    let _serial = serial();
    let probes = probe::probe_bin(&temp_dir("off-cat"));
    let mut session = probe::session(&probes.dir, probes.path_with_probes(), false);
    let outcome = session
        .run("printf 'hello\\n' | cat > out.txt", &opts())
        .expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(probes.count(), "cat\n", "the PATH cat must have run");
    assert_eq!(probes.read("out.txt"), "PROBE-CAT:hello\n");
}

#[cfg(unix)]
#[test]
fn issue2066_flag_off_pipeline_sed_runs_the_path_program() {
    let _serial = serial();
    let probes = probe::probe_bin(&temp_dir("off-sed"));
    let mut session = probe::session(&probes.dir, probes.path_with_probes(), false);
    let outcome = session
        .run("printf 'a b\\n' | sed 's/b/B/' > out.txt", &opts())
        .expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(probes.count(), "sed\n", "the PATH sed must have run");
    // The probe passes stdin through with its marker; the in-process
    // bridge would have applied the substitution instead.
    assert_eq!(probes.read("out.txt"), "PROBE-SED:a b\n");
}

#[cfg(unix)]
#[test]
fn issue2066_flag_off_comsub_pipeline_cat_runs_the_path_program() {
    let _serial = serial();
    let probes = probe::probe_bin(&temp_dir("off-comsub-cat"));
    let mut session = probe::session(&probes.dir, probes.path_with_probes(), false);
    let outcome = session
        .run(
            "x=$(printf 'hi\\n' | cat); printf '%s' \"$x\" > out.txt",
            &opts(),
        )
        .expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(probes.count(), "cat\n", "the PATH cat must have run");
    assert_eq!(probes.read("out.txt"), "PROBE-CAT:hi");
}

#[cfg(unix)]
#[test]
fn issue2066_flag_off_pipeline_cat_without_a_path_program_is_127() {
    let _serial = serial();
    // PATH holds an empty directory only: with the layer off there
    // is no bridge to answer, so the stage must report the GNU
    // lookup failure instead of silently emulating (the sed arm's
    // pre-fix shape, and cat's).
    let dir = temp_dir("off-nopath");
    let empty = dir.join("empty-bin");
    std::fs::create_dir_all(&empty).expect("empty bin dir");
    let mut session = probe::session(&dir, empty.display().to_string(), false);
    let outcome = session
        .run("printf 'x\\n' | cat > out.txt", &opts())
        .expect("run");
    assert_eq!(outcome.status, 127);
}

#[cfg(unix)]
#[test]
fn issue2066_flag_off_simple_command_cat_runs_the_path_program() {
    let _serial = serial();
    // Cross-face consistency guard: the simple-command face was
    // already flag-gated before #2066; both faces must agree.
    let probes = probe::probe_bin(&temp_dir("off-simple-cat"));
    let mut session = probe::session(&probes.dir, probes.path_with_probes(), false);
    std::fs::write(probes.dir.join("input.txt"), "data\n").expect("input");
    let outcome = session
        .run("cat < input.txt > out.txt", &opts())
        .expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(probes.count(), "cat\n", "the PATH cat must have run");
    assert_eq!(probes.read("out.txt"), "PROBE-CAT:data\n");
}

#[cfg(unix)]
#[test]
fn issue2066_flag_on_pipeline_cat_answers_in_process() {
    let _serial = serial();
    // Guard (zero behavior change with the layer on): a resolvable
    // PATH cat does NOT run — the in-process arm replaces it
    // (registry stand_down: Replace), with full GNU option surface.
    let probes = probe::probe_bin(&temp_dir("on-cat"));
    let mut session = probe::session(&probes.dir, probes.path_with_probes(), true);
    let outcome = session
        .run("printf 'hello\\n' | cat -n > out.txt", &opts())
        .expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(probes.count(), "", "the PATH cat must NOT have run");
    assert_eq!(probes.read("out.txt"), "     1\thello\n");
}

#[cfg(unix)]
#[test]
fn issue2066_flag_on_pipeline_sed_keeps_its_inverted_recheck() {
    let _serial = serial();
    // Guard: with the layer on, a resolvable PATH sed still wins
    // over the bridge (the arm's pre-existing inverted recheck,
    // rubash#1462) — the #2066 gate must not disturb it.
    let probes = probe::probe_bin(&temp_dir("on-sed"));
    let mut session = probe::session(&probes.dir, probes.path_with_probes(), true);
    let outcome = session
        .run("printf 'a b\\n' | sed 's/b/B/' > out.txt", &opts())
        .expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(probes.count(), "sed\n", "the PATH sed must have run");
    assert_eq!(probes.read("out.txt"), "PROBE-SED:a b\n");
}

/// The one shape the sed bridge actually serves: the name is
/// HASH-resolved but the arm's lookup misses. The two lookups
/// differ in exactly one place — the dispatch pre-check's
/// `command_path_without_bridge` reads the env hash table entry
/// without stat'ing it (so the fail-closed stand-down does not
/// fire), while the arm's `find_user_command`, with the
/// `checkhash` shopt set, stats the hashed target, drops a
/// vanished one (findcmd.c:367-380 check_hashed_filenames), and
/// then misses on the PATH walk. So the seed is: `checkhash` on,
/// a `hash -p` entry pointing at a path that does not exist, and
/// PATH holding only an empty directory. (With checkhash off the
/// vanished target is returned as-is and the spawn fails — GNU's
/// own behavior — so checkhash is load-bearing here.) The caller
/// must run `shopt -u checkhash` before dropping the session:
/// the shopt state is process-global.
#[cfg(unix)]
fn hash_seeded_sed_session(name: &str, layer_on: bool) -> (Session, probe::ProbeBin) {
    let probes = probe::probe_bin(&temp_dir(name));
    let empty = probes.dir.join("empty-bin");
    std::fs::create_dir_all(&empty).expect("empty bin dir");
    let mut session = probe::session(&probes.dir, empty.display().to_string(), layer_on);
    let vanished_sed = probes.dir.join("gone").join("sed");
    let outcome = session
        .run(
            &format!(
                "shopt -s checkhash; hash -p '{}' sed",
                vanished_sed.display()
            ),
            &opts(),
        )
        .expect("seed hash");
    assert_eq!(outcome.status, 0, "hash -p seeds the resolution");
    (session, probes)
}

#[cfg(unix)]
#[test]
fn issue2066_flag_on_pipeline_sed_hash_resolved_shape_uses_the_bridge() {
    let _serial = serial();
    // Guard: with the layer on, the bridge still serves its one
    // shape — output is the bridge's substitution, and no external
    // program runs (the hash target has vanished).
    let (mut session, probes) = hash_seeded_sed_session("on-sed-hash", true);
    let outcome = session
        .run("printf 'a b\\n' | sed 's/b/B/' > out.txt", &opts())
        .expect("run");
    let count = probes.count();
    let out = probes.read("out.txt");
    let _ = session.run("shopt -u checkhash", &opts());
    assert_eq!(outcome.status, 0);
    assert_eq!(count, "", "the bridge, not the program, answered");
    assert_eq!(out, "a B\n");
}

#[cfg(unix)]
#[test]
fn issue2066_flag_off_pipeline_sed_hash_resolved_shape_falls_to_the_real_program() {
    let _serial = serial();
    // THE sed discriminator for #2066: this is the shape where the
    // pre-fix arm answered with the bridge even when the host had
    // turned the layer off (the inverted recheck masked every
    // PATH-resolvable shape, so only the hash-resolved one exposes
    // it). Post-fix the stage must go external — and since the
    // hashed target has vanished and PATH holds nothing, the
    // external stage reports the lookup failure (127) instead of
    // the bridge silently answering.
    let (mut session, probes) = hash_seeded_sed_session("off-sed-hash", false);
    let outcome = session
        .run("printf 'a b\\n' | sed 's/b/B/' > out.txt", &opts())
        .expect("run");
    let count = probes.count();
    let out = probes.read("out.txt");
    let _ = session.run("shopt -u checkhash", &opts());
    assert_eq!(outcome.status, 127);
    assert_eq!(count, "", "no program ran; the bridge must not answer");
    assert_eq!(out, "");
}
