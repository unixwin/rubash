//! Issue rubash#2071: the `test` builtin answered TRUE for path unary
//! operators on an EMPTY path operand (`[ -e "" ]` exited 0).
//!
//! GNU semantics: the operand string goes to the syscall verbatim —
//! test.c:549-550 `sh_stat (arg, &stat_buf)` for -a/-e, test.c:552-559
//! `sh_eaccess (arg, R_OK|W_OK|X_OK)` for -r/-w/-x, `lstat (arg, ...)`
//! for -h/-L — so stat("") fails ENOENT and every path unary is false.
//! Verified against GNU Bash 5.3.20 (self-compiled oracle, script-file
//! census 2026-10-29): all 20 path unary operators return 1 on "" on
//! both the `test`/`[ ]` face and the `[[ ]]` face.
//!
//! Root cause: `test_path()` anchored a relative operand at $PWD, and
//! joining "" onto the cwd yields the cwd itself, so the operators
//! stat'ed the current directory (which exists, is a directory, is
//! readable/writable/executable, is owned by the effective user, and in
//! a setgid directory even carries the -g bit) and answered true. The
//! `[[ ]]` face routes through the same builtin evaluator
//! (`conditional_file_unary` -> `builtins::test::execute`), so one fix
//! in `test_path` covers both faces, plus the -nt/-ot/-ef binary face
//! that shares it via `modified`/`same_file`.
//!
//! Real-world victim: nvm's `nvm_rc_version` guards with
//! `[ ! -e "$NVMRC_PATH" ]`; with the guard penetrated, `nvm use` in a
//! directory without .nvmrc read "" as a directory and reported a bogus
//! `invalid .nvmrc` (fix-2023-nvm-pty REPORT, chain anomaly A).

use std::process::Command;

fn rubash(script: &str) -> Option<i32> {
    rubash_in(script, None)
}

fn rubash_in(script: &str, dir: Option<&std::path::Path>) -> Option<i32> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.arg("-c").arg(script);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let output = command.output().expect("run rubash");
    output.status.code()
}

/// Every path-class unary operator GNU defines for `test`.
const PATH_UNARY_OPS: &[&str] = &[
    "-e", "-a", "-f", "-d", "-r", "-w", "-x", "-s", "-L", "-h", "-p", "-S", "-b", "-c", "-g", "-u",
    "-k", "-O", "-G", "-N",
];

#[test]
fn issue2071_empty_path_all_unary_ops_false_in_brackets() {
    for op in PATH_UNARY_OPS {
        let script = format!("[ {op} \"\" ]");
        assert_eq!(rubash(&script), Some(1), "[ {op} \"\" ] must be false");
    }
}

#[test]
fn issue2071_empty_path_all_unary_ops_false_in_test_builtin() {
    for op in PATH_UNARY_OPS {
        let script = format!("test {op} \"\"");
        assert_eq!(rubash(&script), Some(1), "test {op} \"\" must be false");
    }
}

#[test]
fn issue2071_empty_path_all_unary_ops_false_in_conditional() {
    for op in PATH_UNARY_OPS {
        let script = format!("[[ {op} \"\" ]]");
        assert_eq!(rubash(&script), Some(1), "[[ {op} \"\" ]] must be false");
    }
}

#[test]
fn issue2071_empty_path_via_variable_and_negated_guard() {
    // The nvm shape: a possibly-empty variable inside a negated guard.
    assert_eq!(rubash("p=\"\"; [ ! -e \"$p\" ]"), Some(0));
    assert_eq!(rubash("p=\"\"; [ -e \"$p\" ]"), Some(1));
    assert_eq!(rubash("p=\"\"; [[ ! -d $p ]]"), Some(0));
    assert_eq!(rubash("unset p; [ -e \"$p\" ]"), Some(1));
}

/// Control cells: normal paths must keep their GNU answers (the fix
/// must not dent the non-empty path). Expectations verified against
/// GNU Bash 5.3.20 with the same fixture tree.
#[test]
fn issue2071_normal_path_controls_unchanged() {
    let dir = std::env::temp_dir().join(format!("rubash-2071-ctrl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create control dir");
    std::fs::write(dir.join("nonempty"), b"x").expect("write nonempty");
    std::fs::write(dir.join("emptyfile"), b"").expect("write emptyfile");
    std::fs::create_dir(dir.join("somedir")).expect("mkdir somedir");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("nonempty", dir.join("link")).expect("symlink");
        let status = Command::new("mkfifo")
            .arg(dir.join("somefifo"))
            .status()
            .expect("mkfifo");
        assert!(status.success());
    }

    // (target, op, expected rc) — GNU Bash 5.3.20 oracle values.
    let mut cells: Vec<(&str, &str, i32)> = vec![
        ("nonempty", "-e", 0),
        ("nonempty", "-f", 0),
        ("nonempty", "-s", 0),
        ("nonempty", "-r", 0),
        ("nonempty", "-d", 1),
        ("nonempty", "-L", 1),
        ("nonempty", "-p", 1),
        ("emptyfile", "-e", 0),
        ("emptyfile", "-f", 0),
        ("emptyfile", "-s", 1),
        ("somedir", "-d", 0),
        ("somedir", "-x", 0),
        ("somedir", "-f", 1),
        (".", "-d", 0),
        (".", "-e", 0),
        ("/dev/null", "-c", 0),
        ("/dev/null", "-e", 0),
        ("/dev/null", "-f", 1),
    ];
    #[cfg(unix)]
    cells.extend_from_slice(&[
        ("link", "-L", 0),
        ("link", "-h", 0),
        ("link", "-f", 0),
        ("link", "-e", 0),
        ("somefifo", "-p", 0),
        ("somefifo", "-f", 1),
    ]);

    for (target, op, expected) in cells {
        for script in [format!("[ {op} {target} ]"), format!("[[ {op} {target} ]]")] {
            assert_eq!(
                rubash_in(&script, Some(&dir)),
                Some(expected),
                "{script} (cwd=control dir) must exit {expected}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The binary file comparisons share `test_path` (via `modified` /
/// `same_file`); GNU treats a failed stat on either side explicitly:
/// `file -nt ""` is true (the other side does not exist), `"" -nt file`
/// and `"" -ef ""` are false (test.c file_newer / general.c same_file).
#[test]
fn issue2071_binary_comparisons_with_empty_side() {
    let dir = std::env::temp_dir().join(format!("rubash-2071-bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create control dir");
    std::fs::write(dir.join("nonempty"), b"x").expect("write nonempty");

    assert_eq!(rubash_in("[ nonempty -nt \"\" ]", Some(&dir)), Some(0));
    assert_eq!(rubash_in("[ \"\" -nt nonempty ]", Some(&dir)), Some(1));
    assert_eq!(rubash_in("[ nonempty -ot \"\" ]", Some(&dir)), Some(1));
    assert_eq!(rubash_in("[ \"\" -ef \"\" ]", Some(&dir)), Some(1));
    assert_eq!(rubash_in("[ nonempty -ef nonempty ]", Some(&dir)), Some(0));
    assert_eq!(rubash_in("[ nonempty -ef \"\" ]", Some(&dir)), Some(1));

    let _ = std::fs::remove_dir_all(&dir);
}
