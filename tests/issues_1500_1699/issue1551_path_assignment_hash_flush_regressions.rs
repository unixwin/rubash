//! rubash#1551 — PATH assignment must flush the command hash table.
//!
//! GNU registers an assignment function for PATH in the special_vars
//! table (variables.c:5819 `{ "PATH", sv_path }`), and sv_path
//! (variables.c:5919) is exactly `phash_flush()` (hashcmd.c:57): ANY
//! operation that binds a value to PATH — plain or `+=` assignment,
//! temp-prefix bind (and its restore), declare/typeset/local with or
//! without a value, export/readonly with a value, read, printf -v, an
//! arithmetic bind — or unbinds it (`unset PATH`) empties the whole
//! remembered-location table, even for a same-value bind.
//! Attribute-only operations (`export PATH`, `declare -p PATH`) never
//! bind and keep the table. Expectations below are the GNU bash
//! 5.3.20 oracle's, probed from script files under `env -i`.
//!
//! The fix fires `builtins::hash::flush_command_hash_table` at every
//! bind funnel, mirroring the sv_locale funnel discipline of #1531.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Two PATH directories, `pa` and `pb`, each holding a `foo` script
/// that prints a distinct marker.
struct Fixture {
    dir: PathBuf,
    pa: String,
    pb: String,
}

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-1551-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pa = dir.join("pa");
        let pb = dir.join("pb");
        fs::create_dir_all(&pa).expect("create pa");
        fs::create_dir_all(&pb).expect("create pb");
        for (base, marker) in [(&pa, "from-a"), (&pb, "from-b")] {
            let foo = base.join("foo");
            fs::write(&foo, format!("#!/bin/sh\necho {marker}\n")).expect("write foo");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&foo, fs::Permissions::from_mode(0o755)).expect("chmod foo");
            }
        }
        Fixture {
            pa: pa.to_string_lossy().into_owned(),
            pb: pb.to_string_lossy().into_owned(),
            dir,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue1551_plain_assignment_flushes_and_re_resolves() {
    let fx = Fixture::new();
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH={pa}; foo; PATH={pb}; foo; hash",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("from-a"), "stdout: {stdout}");
    assert!(stdout.contains("from-b"), "stdout: {stdout}");
    // The table holds ONLY the new resolution (GNU: `1 /tmp/pb/foo`).
    assert!(stdout.contains(&fx.pb), "stdout: {stdout}");
    assert!(!stdout.contains(&fx.pa), "stdout: {stdout}");
}

#[test]
fn issue1551_explicit_hash_entry_dies_with_assignment() {
    let fx = Fixture::new();
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; PATH={pb}; foo",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("from-b"), "stdout: {stdout}");
    assert!(!stdout.contains("from-a"), "stdout: {stdout}");
}

#[test]
fn issue1551_hashed_name_unresolvable_after_path_moves() {
    let (stdout, stderr, code) =
        rubash_c("hash -p /usr/bin/cat cat; PATH=/nonexistent; cat /etc/hostname; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1551_same_value_assignment_still_flushes() {
    let fx = Fixture::new();
    let (stdout, stderr, code) = rubash_c(&format!("PATH={pa}; foo; PATH=$PATH; hash", pa = fx.pa));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("hash table empty"), "stdout: {stdout}");
}

#[test]
fn issue1551_unset_path_flushes() {
    let fx = Fixture::new();
    let (stdout, stderr, code) = rubash_c(&format!("PATH={pa}; foo; unset PATH; hash", pa = fx.pa));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("hash table empty"), "stdout: {stdout}");
}

#[test]
fn issue1551_temp_prefix_binds_and_restores_flush() {
    let fx = Fixture::new();
    // GNU: a temp-prefix bind flushes on the way in AND on restore, so
    // after `PATH=<pb> foo` runs (from-b), the caller's next `foo`
    // re-resolves under the restored PATH and the table holds only
    // that fresh entry.
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH={pa}; foo; PATH={pb} foo; foo; hash",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let markers: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with("from-"))
        .collect();
    assert_eq!(markers, ["from-a", "from-b", "from-a"], "stdout: {stdout}");
    assert!(stdout.contains(&fx.pa), "stdout: {stdout}");
    assert!(!stdout.contains(&fx.pb), "stdout: {stdout}");
}

#[test]
fn issue1551_temp_prefix_on_external_flushes() {
    let fx = Fixture::new();
    // GNU: after `PATH=<pb> /usr/bin/true` the table is EMPTY — entries
    // recorded before the temp bind died at the bind, and the restore
    // bind flushed again before anything re-resolved.
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH={pa}; foo; PATH={pb} /usr/bin/true; hash",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("hash table empty"), "stdout: {stdout}");
}

#[test]
fn issue1551_local_assignment_and_frame_restore_flush() {
    let fx = Fixture::new();
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH={pa}; foo; f() {{ local PATH={pb}; foo; }}; f; foo; hash",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let markers: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with("from-"))
        .collect();
    assert_eq!(markers, ["from-a", "from-b", "from-a"], "stdout: {stdout}");
    assert!(stdout.contains(&fx.pa), "stdout: {stdout}");
    assert!(!stdout.contains(&fx.pb), "stdout: {stdout}");
}

#[test]
fn issue1551_declare_with_and_without_value_flushes() {
    let fx = Fixture::new();
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; declare PATH={pb}; foo",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("from-b"), "stdout: {stdout}");

    // Value-less declare names the variable without binding a value,
    // but GNU still runs the bind path: the table empties and the
    // /nonexistent PATH leaves foo unresolvable.
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; declare PATH; foo; echo rc=$?",
        pa = fx.pa
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("rc=127"), "stdout: {stdout}");
    assert!(
        stderr.contains("foo: command not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1551_export_with_value_flushes_without_value_keeps() {
    let fx = Fixture::new();
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; export PATH={pb}; foo",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("from-b"), "stdout: {stdout}");

    // Attribute-only: GNU does NOT flush (no bind happens).
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; export PATH; foo",
        pa = fx.pa
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("from-a"), "stdout: {stdout}");

    // Same for a name-only readonly (its apply path materializes the
    // current value for the process-env mirror, which is not a bind).
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; readonly PATH; foo",
        pa = fx.pa
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("from-a"), "stdout: {stdout}");
}

#[test]
fn issue1551_read_and_printf_v_binds_flush() {
    let fx = Fixture::new();
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; read PATH <<< {pb}; foo",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("from-b"), "stdout: {stdout}");

    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; printf -v PATH %s {pb}; foo",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("from-b"), "stdout: {stdout}");
}

#[test]
fn issue1551_temp_prefix_on_function_after_temp_on_builtin() {
    let fx = Fixture::new();
    // Sequence pin: a temp bind on a builtin first (its restore runs),
    // then a temp bind on a FUNCTION — GNU flushes at both binds and
    // both restores, so the seeded -p entry is dead after each call.
    // (The function's tempvar install/restore takes a different path
    // than a builtin's; both must fire sv_path.)
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; PATH={pb} true; foo; echo rc1=$?; \
         PATH=/nonexistent; hash -p {pa}/foo foo; g() {{ :; }}; PATH={pb} g; foo; echo rc2=$?",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("rc1=127"), "stdout: {stdout}");
    assert!(stdout.contains("rc2=127"), "stdout: {stdout}");
    assert!(!stdout.contains("from-a"), "stdout: {stdout}");
}

#[test]
fn issue1551_fork_boundaries_isolate_the_table() {
    let fx = Fixture::new();
    // GNU forks pipeline elements, command substitutions and ( )
    // subshells: a child's PATH bind (flush) or `hash -p` write never
    // reaches the parent's table.
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; echo {pb} | read PATH; foo; echo rc=$?",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("from-a"), "stdout: {stdout}");
    assert!(stdout.contains("rc=0"), "stdout: {stdout}");

    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; x=$(PATH={pb}; true); foo; \
         y=$(hash -p {pb}/foo foo; true); foo",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let markers: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with("from-"))
        .collect();
    assert_eq!(markers, ["from-a", "from-a"], "stdout: {stdout}");

    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; ( PATH={pb} ); foo",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("from-a"), "stdout: {stdout}");

    // Input process substitution bodies are forked children too
    // (subst.c:6362 process_substitute).
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH=/nonexistent; hash -p {pa}/foo foo; /usr/bin/cat <(PATH={pb}; true) >/dev/null; foo",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("from-a"), "stdout: {stdout}");
}

#[test]
fn issue1551_bash_cmds_view_follows_the_flush() {
    let fx = Fixture::new();
    let (stdout, stderr, code) = rubash_c(&format!(
        "PATH={pa}; foo; PATH={pb}; foo; declare -p BASH_CMDS",
        pa = fx.pa,
        pb = fx.pb
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains(&fx.pb), "stdout: {stdout}");
    assert!(!stdout.contains(&fx.pa), "stdout: {stdout}");
}
