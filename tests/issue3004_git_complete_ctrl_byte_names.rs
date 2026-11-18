//! rubash#3004 — git completion (t9902 #165) loses filenames carrying
//! literal \x1c-\x1f bytes.
//!
//! Two faces of one carrier-protocol violation (markers.rs governance:
//! a user byte in the carrier range is DATA and must ride the raw-byte
//! marker pair through transport; consumers decode the pair, they never
//! interpret the payload as protocol):
//!
//! 1. Field splitting. A stored value pair-encodes its carrier bytes
//!    (substitution_metadata::bytes_to_shell_text), but the byte-domain
//!    splitter `field_split_with_raw_byte_markers` (arrays.rs) decoded
//!    the pairs FIRST and then treated every decoded 0x1C byte as the
//!    IFS_GLUE protocol marker, dropping it. git-completion's
//!    `__gitcomp_file_direct` runs `COMPREPLY=($1)` with IFS=$'\n' over
//!    exactly such a captured value, so a candidate's \x1c vanished
//!    (`separators\x1cin\x1ddir` came back `separatorsin\x1ddir`) and
//!    path completion mismatched/emptied. GNU: subst.c:12377
//!    word_split/list_string splits the expansion result on IFS chars
//!    only; a data \x1c is an ordinary char end to end.
//!
//! 2. The in-process emulated file builtins (mkdir/touch/chmod/cp/rm
//!    in external_file_builtins.rs) consumed the cooked words — which
//!    carry the pair encoding — as filesystem paths verbatim, so the
//!    pair's UTF-8 bytes landed on disk as the name (the t9902 fixture
//!    creation face: `mkdir`/`touch` under the shell under test create
//!    U+E000-pair names instead of the raw-byte names). The real spawn
//!    funnel decodes the pair before execve (path.rs decode_bytes,
//!    rubash#141); the emulations stand in for that binary and must see
//!    the same decoded argv. GNU: execute_cmd.c:6139 shell_execve
//!    hands the child the raw word bytes.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions; filesystem cells create fixtures with std::fs under
//! raw-byte names (never through the shell under test) and inspect the
//! directory with std::fs afterwards. `LC_ALL=C.UTF-8` is pinned on
//! the child so the pin is hermetic under any ambient locale.
//!
//! Scope note: multi-element array STORAGE of pair-carrying elements
//! (the declare/local mirror serializer) and unquoted splitting in
//! command-word position (for-list / `set --`) lose carrier bytes
//! through two further, pre-existing faces of the same protocol
//! family; both reproduce on the base with \x1f-only values (which
//! this fix does not alter) and are filed separately — the cells here
//! pin the #3004 chain itself (capture -> function arg -> IFS split ->
//! single candidate, and the emulated-builtin operand boundary).

use std::process::{Command, Stdio};

fn run(script: &str) -> Vec<u8> {
    run_in(None, script)
}

fn run_in(dir: Option<&std::path::Path>, script: &str) -> Vec<u8> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let output = cmd
        .spawn()
        .expect("spawn rubash")
        .wait_with_output()
        .expect("wait rubash");
    assert!(
        output.status.success(),
        "rubash -c {script:?} exited with {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn expect(script: &str, expected: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(
        out,
        expected,
        "{ctx}: got {:?}, want {:?}",
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(expected)
    );
}

// ---------------------------------------------------------------------
// Face 1: field splitting must keep data carrier bytes.
// ---------------------------------------------------------------------

#[test]
fn completion_chain_split_keeps_all_ctrl_bytes() {
    // The t9902 #165 shape end to end (minus git itself): comsub
    // capture -> function positional -> IFS=$'\n' array assignment,
    // which is __gitcomp_file_direct's `COMPREPLY=($1)`.
    expect(
        "raw=$(printf 'separators\\034in\\035dir/sep\\036in\\037file'); \
         f() { local IFS=$'\\n'; local -a a=($1); printf '%s' \"${a[0]}\"; }; \
         f \"$raw\"",
        b"separators\x1cin\x1ddir/sep\x1ein\x1ffile",
        "candidate text must survive the IFS-newline split byte-for-byte",
    );
}

#[test]
fn split_keeps_each_ctrl_byte_in_array_assignment() {
    // One cell per registry byte \x1c-\x1f: the element length is the
    // observable (3 = byte kept, 2 = byte dropped by the splitter).
    for (octal, byte) in [("034", 0x1cu8), ("035", 0x1d), ("036", 0x1e), ("037", 0x1f)] {
        let script = format!(
            "v=$(printf 'a\\{octal}b'); \
             g() {{ local IFS=$'\\n'; local -a a=($1); printf '%s' \"${{#a[0]}}\"; }}; \
             g \"$v\""
        );
        let out = run(&script);
        assert_eq!(
            out,
            b"3",
            "byte 0x{byte:02x} must survive the split (got element length {:?})",
            String::from_utf8_lossy(&out)
        );
    }
}

#[test]
fn split_guard_quoted_blank_in_alternate_still_protected() {
    // The IFS_GLUE protocol's legitimate job: whitespace quoted inside
    // a ${var-word} alternate must NOT become a split point (GNU
    // posixexp2 37 shape). This cell passes before and after the fix;
    // it guards against "fixing" the drop by removing glue handling.
    expect(
        "e=; arr=(${e:-a' 'b}); printf '%s' \"${#arr[@]}\"",
        b"1",
        "quoted blank in alternate must stay one field",
    );
}

#[test]
fn split_guard_plain_value_still_splits() {
    expect(
        "v=$(printf 'a b'); arr=($v); printf '%s' \"${#arr[@]}\"",
        b"2",
        "ordinary unquoted expansion still field-splits",
    );
}

// ---------------------------------------------------------------------
// Face 2: emulated file builtins must consume decoded operands.
// ---------------------------------------------------------------------

#[cfg(unix)]
mod fs_cells {
    use super::run_in;
    use std::os::unix::ffi::OsStrExt;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue3004-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn names(dir: &std::path::Path) -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = std::fs::read_dir(dir)
            .expect("read scratch dir")
            .map(|entry| entry.expect("dir entry").file_name().as_bytes().to_vec())
            .collect();
        out.sort();
        out
    }

    #[test]
    fn touch_from_variable_creates_raw_byte_name() {
        let dir = scratch("touch");
        run_in(Some(&dir), "n=$(printf 'f\\034g'); touch -- \"$n\"");
        assert_eq!(names(&dir), vec![b"f\x1cg".to_vec()]);
    }

    #[test]
    fn touch_from_literal_creates_raw_byte_name() {
        let dir = scratch("touchlit");
        run_in(Some(&dir), "touch -- $'h\\034i'");
        assert_eq!(names(&dir), vec![b"h\x1ci".to_vec()]);
    }

    #[test]
    fn mkdir_from_variable_creates_raw_byte_name() {
        let dir = scratch("mkdir");
        run_in(Some(&dir), "m=$(printf 'd\\035e'); mkdir -- \"$m\"");
        assert_eq!(names(&dir), vec![b"d\x1de".to_vec()]);
    }

    #[test]
    fn cp_dest_from_variable_creates_raw_byte_name() {
        let dir = scratch("cp");
        std::fs::write(dir.join("src"), b"data\n").expect("write src");
        run_in(Some(&dir), "n=$(printf 'f\\034g'); cp src \"$n\"");
        assert_eq!(names(&dir), vec![b"f\x1cg".to_vec(), b"src".to_vec()]);
    }

    #[test]
    fn rm_from_variable_removes_raw_byte_name() {
        let dir = scratch("rm");
        std::fs::write(dir.join(std::ffi::OsStr::from_bytes(b"r\x1em")), b"x").expect("fixture");
        run_in(Some(&dir), "r=$(printf 'r\\036m'); rm -- \"$r\"");
        assert_eq!(names(&dir), Vec::<Vec<u8>>::new());
    }

    #[test]
    fn chmod_from_variable_stamps_raw_byte_name() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("chmod");
        let file = dir.join(std::ffi::OsStr::from_bytes(b"c\x1cd"));
        std::fs::write(&file, b"x").expect("fixture");
        // No `--`: the emulated chmod's option grammar mishandles a
        // post-mode `--` (a separate pre-existing gap); the carrier
        // mechanism pinned here is mode + operand delivery.
        run_in(Some(&dir), "c=$(printf 'c\\034d'); chmod 600 \"$c\"");
        let mode = std::fs::metadata(&file)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "chmod must reach the raw-named file");
    }

    #[test]
    fn cat_from_variable_reads_raw_byte_name() {
        let dir = scratch("cat");
        std::fs::write(
            dir.join(std::ffi::OsStr::from_bytes(b"v\x1cw")),
            b"payload\n",
        )
        .expect("fixture");
        let out = run_in(Some(&dir), "v=$(printf 'v\\034w'); cat -- \"$v\"");
        assert_eq!(out, b"payload\n");
    }

    #[test]
    fn control_real_mv_dest_from_variable() {
        // mv is NOT emulated: the real binary receives funnel-decoded
        // argv today. Control cell — passes before and after; it pins
        // the boundary the emulations must match.
        let dir = scratch("mv");
        std::fs::write(dir.join("src"), b"x\n").expect("write src");
        run_in(Some(&dir), "n=$(printf 'f\\034g'); mv src \"$n\"");
        assert_eq!(names(&dir), vec![b"f\x1cg".to_vec()]);
    }
}
