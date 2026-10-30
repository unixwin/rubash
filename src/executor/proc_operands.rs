//! Materialize synthetic `/proc` operands into concrete paths for
//! external children (rubash#1905, proc-vfs plan P2).
//!
//! GNU passes operand words to execve literally (execute_cmd.c
//! shell_execve); the child resolves `/proc/stat` because a real
//! procfs serves it. Windows has no procfs: the engine's synthetic
//! /proc (`crate::proc_vfs`) lives only inside the shell, so a child
//! handed the literal spelling gets an OS error. The equivalent of
//! "the file exists" at this boundary is a concrete path: for the
//! registry's top-level content files — the entries whose bytes
//! depend on no reader and no process — the shell writes the
//! generator's spawn-time snapshot to a temp file and hands the child
//! that path.
//!
//! The scoping is deliberate and narrow (see the #1905 design record
//! in the fix workspace):
//!
//! - ONLY the top-level content files materialize
//!   (`special_paths::synthetic_proc_top_level_name`). `/proc/self/*`
//!   names the READER, so a shell-written snapshot would answer with
//!   the shell's own identity — a construction error, not staleness.
//!   Per-pid files bind their existence to a process that may exit
//!   (materializing would turn "vanished" into "stale but readable",
//!   a silent-wrong-answer direction); link entries have no bytes;
//!   directories are listing semantics. All of those keep the literal
//!   word and fail loudly in a child without its own layer, exactly
//!   as before this pass existed.
//! - Children that resolve the spelling through a layer of their own
//!   keep the literal word (`path::child_resolves_synthetic_proc`):
//!   namespace bridges, MSYS2/Cygwin-runtime-hosted tools, and
//!   scripts a POSIX-layer shell will interpret.
//! - The word-shape discipline is the dev-fd pass's
//!   (`split_keyed_operand`): whole words and option-keyed `key=`
//!   forms rewrite; environment-assignment-shaped words stay literal.
//! - The pass is inert off Windows: real procfs serves children
//!   there, and the synthetic class is empty (the proc_vfs stub
//!   discipline — a synthesis layer must never shadow a real one).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use super::dev_fd_operands::{
    split_keyed_operand, DevOperandMaterialization, DevOperandStdin, DevOperandStdout,
};
use super::path::{child_resolves_synthetic_proc, synthetic_proc_skip_positions};
use super::Executor;
use crate::parser::CommandNode;

/// Deferred cleanup from /proc operand materialization: the snapshot
/// temp files are removed when the record drops. Every spawn call
/// site holds the record until its child has exited (the same scope
/// discipline as `DevOperandMaterialization`), so a child never sees
/// its operand vanish mid-run and no snapshot outlives the spawn
/// cycle that created it.
#[derive(Default)]
pub(in crate::executor) struct ProcMaterialization {
    temps: Vec<PathBuf>,
}

impl Drop for ProcMaterialization {
    fn drop(&mut self) {
        for temp in &self.temps {
            // The snapshots are handed out read-only (the 0444
            // shape); on Windows the readonly attribute also blocks
            // deletion, so clear it before removing.
            if let Ok(metadata) = std::fs::metadata(temp) {
                let mut permissions = metadata.permissions();
                if permissions.readonly() {
                    permissions.set_readonly(false);
                    let _ = std::fs::set_permissions(temp, permissions);
                }
            }
            let _ = std::fs::remove_file(temp);
        }
    }
}

/// The registry name a word names when it is exactly a materializable
/// synthetic /proc spelling, else None.
fn proc_operand_name(value: &str) -> Option<&'static str> {
    crate::special_paths::synthetic_proc_top_level_name(value)
}

fn proc_operand_temp(name: &str, bytes: &[u8], m: &mut ProcMaterialization) -> Option<String> {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "rubash-proc-{}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed),
        name
    ));
    std::fs::write(&path, bytes).ok()?;
    // The real entries are mode 0444: hand the child a read-only file
    // so a write-open is refused (the Windows readonly attribute
    // yields ACCESS_DENIED) with the same shape as procfs's EACCES,
    // instead of letting the child silently scribble on a snapshot
    // that vanishes at cleanup.
    if let Ok(metadata) = std::fs::metadata(&path) {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        let _ = std::fs::set_permissions(&path, permissions);
    }
    m.temps.push(path.clone());
    Some(path.to_string_lossy().into_owned())
}

/// Rewrite synthetic-/proc operand words in `args` to snapshot temp
/// paths for a child spawned for `program`. `command_name` is the
/// command word as typed (the data-primary protection for the
/// WinuxCmd class keys off it, via `synthetic_proc_skip_positions`).
///
/// Verbatim outcomes, all deliberate: no resolved program (nothing
/// sound to classify), a non-Windows build, or a child class that
/// resolves the spellings itself. A temp-creation failure likewise
/// keeps the literal word — the child then fails loudly, as it did
/// before this pass existed; materialization must never block a
/// spawn.
pub(in crate::executor) fn materialize_proc_operands(
    args: &[String],
    command_name: Option<&str>,
    program: Option<&Path>,
    env_vars: &HashMap<String, String>,
) -> (Vec<String>, ProcMaterialization) {
    let m = ProcMaterialization::default();
    let Some(program) = program else {
        return (args.to_vec(), m);
    };
    if !cfg!(windows) || child_resolves_synthetic_proc(program) {
        return (args.to_vec(), m);
    }
    let skip = synthetic_proc_skip_positions(program, env_vars, command_name, args);
    let mut m = m;
    let rewritten = args
        .iter()
        .enumerate()
        .map(|(index, arg)| {
            if skip.contains(&index) {
                return arg.clone();
            }
            let Some((prefix, value)) =
                split_keyed_operand(arg, |word| proc_operand_name(word).is_some())
            else {
                return arg.clone();
            };
            let name = proc_operand_name(value).expect("splitter checked the value");
            // A fresh snapshot at the spawn boundary — the generator
            // answers a new snapshot per call, matching the procfs
            // contract that content is generated when read; the read
            // this child is about to do is anchored to its spawn. If
            // the generator declines (a name-table/generator drift),
            // keep the literal word: a loud failure in the child,
            // never a wrong snapshot.
            let Some(bytes) = crate::special_paths::synthetic_proc_content(value) else {
                return arg.clone();
            };
            match proc_operand_temp(name, &bytes, &mut m) {
                Some(path) => format!("{prefix}{path}"),
                None => arg.clone(),
            }
        })
        .collect();
    (rewritten, m)
}

/// Whether any word in `args` names a materializable synthetic /proc
/// operand — the admission probe for spawn fast paths that bypass the
/// operand passes (the background direct plan in compound_exec): such
/// shapes decline to the wrapper route, whose child serves them
/// through the full foreground machinery, the same decline family as
/// process substitutions there. Inert off Windows (real procfs).
pub(in crate::executor) fn args_name_synthetic_proc(args: &[String]) -> bool {
    if !cfg!(windows) {
        return false;
    }
    args.iter()
        .any(|arg| split_keyed_operand(arg, |word| proc_operand_name(word).is_some()).is_some())
}

impl Executor {
    /// The exec builtin's operand plumbing (the spawn arms of
    /// `execute_exec_spawn` in trap_exec): the dev-fd pass and the
    /// /proc pass in one call, so the eight arms share one shape. The
    /// child-class policies resolve the target program from the
    /// word after `exec`; an unresolvable name keeps the words
    /// verbatim — the exec is about to report that failure against
    /// pristine arguments.
    pub(in crate::executor) fn materialize_exec_operands(
        &self,
        cmd: &CommandNode,
        args: &[String],
        stdin: DevOperandStdin<'_>,
        stdout: DevOperandStdout,
    ) -> (Vec<String>, DevOperandMaterialization, ProcMaterialization) {
        let program = cmd
            .words
            .get(1)
            .and_then(|name| self.find_scoped_user_command(name));
        let (dev_args, dev_ops) =
            self.materialize_dev_fd_operands(args, stdin, stdout, program.as_deref());
        let (proc_args, proc_ops) = materialize_proc_operands(
            &dev_args,
            cmd.words.get(1).map(String::as_str),
            program.as_deref(),
            &self.shell_state.env_vars,
        );
        (proc_args, dev_ops, proc_ops)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| word.to_string()).collect()
    }

    fn temp_dir_program() -> PathBuf {
        // A plain native program path: no runtime DLL beside it, no
        // bridge stem, not a script — the materialize class.
        std::env::temp_dir().join("rubash-proc-test-native.exe")
    }

    /// The word-shape discipline matches the dev-fd pass's: whole
    /// words and option-keyed forms name operands; assignment-shaped
    /// words and longer paths do not.
    #[test]
    fn issue1905_word_shapes_split_like_dev_fd() {
        for (arg, expected) in [
            ("/proc/stat", Some(("", "/proc/stat"))),
            ("--file=/proc/stat", Some(("--file=", "/proc/stat"))),
            ("if=/proc/meminfo", Some(("if=", "/proc/meminfo"))),
            ("FOO=/proc/stat", None),
            ("/proc/stat/extra", None),
            ("/proc/self/status", None),
            ("/proc/1/status", None),
            ("/proc/stat/", None),
            ("x/proc/stat", None),
        ] {
            assert_eq!(
                split_keyed_operand(arg, |word| proc_operand_name(word).is_some()),
                expected,
                "split({arg:?})"
            );
        }
    }

    /// The rewrite: a native child gets an existing, read-only temp
    /// file holding the generator's bytes; the record's Drop removes
    /// it; non-operand words pass through untouched. Byte equality is
    /// asserted on /proc/version (a fixed string per build); the
    /// dynamic entries are checked by their stable head lines, since
    /// the generator answers a fresh snapshot per call by design.
    #[test]
    fn issue1905_native_child_gets_snapshot_temp() {
        let program = temp_dir_program();
        let (rewritten, record) = materialize_proc_operands(
            &words(&["/proc/version", "plain", "--file=/proc/stat"]),
            Some("tool"),
            Some(&program),
            &HashMap::new(),
        );
        assert_eq!(rewritten[1], "plain");
        let version_path = PathBuf::from(&rewritten[0]);
        assert!(
            version_path.is_file(),
            "snapshot temp exists: {version_path:?}"
        );
        assert_eq!(
            std::fs::read(&version_path).expect("read snapshot"),
            crate::special_paths::synthetic_proc_content("/proc/version")
                .expect("generator answers on Windows"),
            "snapshot bytes are the generator's"
        );
        assert!(
            std::fs::metadata(&version_path)
                .expect("metadata")
                .permissions()
                .readonly(),
            "snapshot is read-only like the 0444 entry"
        );
        let stat_path = PathBuf::from(
            rewritten[2]
                .strip_prefix("--file=")
                .expect("keyed form keeps its prefix"),
        );
        let stat_bytes = std::fs::read(&stat_path).expect("read stat snapshot");
        assert!(
            stat_bytes.starts_with(b"cpu "),
            "stat snapshot carries the generator's head line"
        );
        drop(record);
        assert!(!version_path.exists(), "record drop removes the snapshot");
        assert!(!stat_path.exists());
    }

    /// Children with a layer of their own keep the literal word: a
    /// bridge stem, and a program whose directory ships the MSYS
    /// runtime DLL.
    #[test]
    fn issue1905_layered_children_keep_literal_words() {
        let bridge = PathBuf::from(r"C:\Windows\System32\ssh.exe");
        let (rewritten, _record) = materialize_proc_operands(
            &words(&["/proc/stat"]),
            Some("ssh"),
            Some(&bridge),
            &HashMap::new(),
        );
        assert_eq!(rewritten, words(&["/proc/stat"]));

        let dir = std::env::temp_dir().join(format!("rubash-proc-hosted-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("hosted dir");
        std::fs::write(dir.join("msys-2.0.dll"), b"").expect("fake runtime dll");
        let hosted = dir.join("cat.exe");
        let (rewritten, _record) = materialize_proc_operands(
            &words(&["/proc/stat"]),
            Some("cat"),
            Some(&hosted),
            &HashMap::new(),
        );
        assert_eq!(rewritten, words(&["/proc/stat"]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The background-plan admission probe sees exactly the
    /// materializable shapes.
    #[test]
    fn issue1905_args_probe_matches_materializable_shapes() {
        assert!(args_name_synthetic_proc(&words(&["/proc/stat"])));
        assert!(args_name_synthetic_proc(&words(&["--file=/proc/uptime"])));
        assert!(!args_name_synthetic_proc(&words(&["/proc/self/status"])));
        assert!(!args_name_synthetic_proc(&words(&["FOO=/proc/stat"])));
        assert!(!args_name_synthetic_proc(&words(&["/etc/hostname"])));
    }
}
