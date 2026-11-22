//! rubash#3965 — External command spawn must preserve raw bytes (such as 0xFF)
//! in environment variable values without widening to UTF-8 `U+00FF` (`0xC3 0xBF`)
//! or leaking `U+E000` raw-byte marker pairs.
//!
//! GNU C source:
//! - `variables.c:4850 make_env_array_from_var_list` -> `mk_env_string`: copies
//!   the variable's value cell bytes verbatim into `envp` with no re-encoding.
//!
//! Platform boundary:
//! - On Unix (`#[cfg(unix)]`), `OsString::from_vec` / `CString::new` passes raw
//!   bytes (`0x80..=0xFF`) verbatim through `execve`.
//! - On Windows (`#[cfg(not(unix))]`), `CreateProcessW` requires UTF-16
//!   `OsString`, so raw bytes map to `U+0080..=U+00FF` (`decode_raw_byte_markers_to_byte_chars`).

#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
fn rubash_bytes(script: &str) -> (Vec<u8>, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C")
        .output()
        .expect("run rubash");
    (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[cfg(unix)]
#[test]
fn issue3965_spawn_preserves_0xff_in_exported_and_prefix_env() {
    let (stdout, stderr, code) = rubash_bytes(
        "export x=$'\\xff'\n\
         /usr/bin/printenv x\n\
         y=$'\\xff' /usr/bin/printenv y\n\
         export z=$(printf '\\377')\n\
         /usr/bin/printenv z\n\
         w=$(printf '\\377') /usr/bin/printenv w",
    );
    assert_eq!(
        stdout,
        vec![0xff, b'\n', 0xff, b'\n', 0xff, b'\n', 0xff, b'\n']
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn issue3965_spawn_preserves_0xff_in_env_builtin_and_pipeline() {
    let (stdout, stderr, code) = rubash_bytes(
        "export x=$'\\xff'\n\
         /usr/bin/env | /bin/grep '^x='\n\
         y=$'\\xff' /usr/bin/printenv y | /bin/cat\n\
         export a=$'\\xff'\n\
         /bin/bash -c 'printf \"%s\" \"$a\"'",
    );
    assert_eq!(stdout, vec![b'x', b'=', 0xff, b'\n', 0xff, b'\n', 0xff]);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

// ---- Captain graft of #4010 (rubash#3965): respawn state channel + exec face ----
//
// The spawn-side half of the graft: respawn children (coproc / procsub /
// async) are rubash itself, so their environment is a STATE channel — the
// parent's stored spellings ride verbatim and a validated respawn imports
// them verbatim (init.rs `entry_encode_imported_env_respawn`). Decoding
// marker pairs to raw bytes at a respawn spawn site hands the child a
// non-UTF-8 env entry, which its `process_env_pairs` import drops
// wholesale (rubash#2064) — the variable would arrive UNSET. The exec
// builtin's target is a foreign child, so its env face decodes like every
// other external boundary.

#[cfg(unix)]
fn rubash_script(script: &str, tag: &str) -> (Vec<u8>, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash3965-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    // Child-side reporter: prints S:<hex of "$x"> when x arrived set in the
    // child, UNSET when it never arrived.
    std::fs::write(
        dir.join("rep.sh"),
        "if [ \"${x+set}\" = set ]; then h=$(printf '%s' \"$x\" | od -An -tx1 -v | tr -d ' \\n'); printf 'S:%s\\n' \"$h\"; else echo 'UNSET'; fi\n",
    )
    .expect("write rep.sh");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .env("LC_ALL", "C")
        .env("G4010_DIR", &dir)
        .output()
        .expect("run rubash");
    let result = (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[cfg(unix)]
#[test]
fn issue3965_spawn_preserves_0x80_0xcd_and_mixed_bytes() {
    let (stdout, stderr, code) = rubash_bytes(
        "export a=$'\\x80'\n\
         export b=$'\\xcd'\n\
         export m=$(printf 'p\\377\\303\\277q')\n\
         /usr/bin/printenv a\n\
         /usr/bin/printenv b\n\
         /usr/bin/printenv m",
    );
    assert_eq!(
        stdout,
        vec![0x80, b'\n', 0xcd, b'\n', 0x70, 0xff, 0xc3, 0xbf, 0x71, b'\n']
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn issue3965_respawn_children_inherit_raw_byte() {
    let (stdout, stderr, code) = rubash_script(
        r#"D=$G4010_DIR
export x=$'\xff'
coproc C { . "$D/rep.sh" > "$D/coproc.out"; }
coproc E { /usr/bin/printenv x > "$D/re.out"; }
for i in $(seq 1 50); do [ -s "$D/coproc.out" ] && [ -s "$D/re.out" ] && break; sleep 0.1; done
echo "coproc=[$(cat "$D/coproc.out")]"
echo "reexport=[$(od -An -tx1 -v < "$D/re.out" | tr -d ' \n')]"
echo "psin=[$(cat <( . "$D/rep.sh" ) | tr -d ' \n')]"
printf 'kick\n' | tee >( . "$D/rep.sh" > "$D/psout.out" ) >/dev/null
for i in $(seq 1 50); do [ -s "$D/psout.out" ] && break; sleep 0.1; done
echo "psout=[$(cat "$D/psout.out")]"
{ . "$D/rep.sh" > "$D/async.out"; } &
wait
echo "async=[$(cat "$D/async.out")]"
( . "$D/rep.sh" > "$D/async2.out" ) &
wait
echo "asyncsub=[$(cat "$D/async2.out")]"
"#,
        "respawn",
    );
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        "coproc=[S:ff]\nreexport=[ff0a]\npsin=[S:ff]\npsout=[S:ff]\nasync=[S:ff]\nasyncsub=[S:ff]\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn issue3965_coproc_children_inherit_byte_variants() {
    let (stdout, stderr, code) = rubash_script(
        r#"D=$G4010_DIR
export a=$'\x80'
export b=$'\xcd'
export m=$(printf 'p\377\303\277q')
coproc C { printf '%s' "$a" | od -An -tx1 -v > "$D/b80.out"; }
coproc E { printf '%s' "$b" | od -An -tx1 -v > "$D/bcd.out"; }
coproc F { printf '%s' "$m" | od -An -tx1 -v > "$D/bmix.out"; }
for i in $(seq 1 50); do [ -s "$D/b80.out" ] && [ -s "$D/bcd.out" ] && [ -s "$D/bmix.out" ] && break; sleep 0.1; done
echo "c80=[$(tr -d ' \n' < "$D/b80.out")]"
echo "ccd=[$(tr -d ' \n' < "$D/bcd.out")]"
echo "cmix=[$(tr -d ' \n' < "$D/bmix.out")]"
"#,
        "coprocbytes",
    );
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        "c80=[80]\nccd=[cd]\ncmix=[70ffc3bf71]\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn issue3965_exec_builtin_preserves_raw_byte() {
    let (stdout, stderr, code) = rubash_bytes(
        "export x=$'\\xff'\n\
         exec /usr/bin/printenv x",
    );
    assert_eq!(stdout, vec![0xff, b'\n']);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn issue3965_async_direct_spawn_preserves_raw_byte() {
    let (stdout, stderr, code) = rubash_script(
        r#"D=$G4010_DIR
x=$'\xff' /usr/bin/printenv x > "$D/a.out" &
wait
od -An -tx1 -v < "$D/a.out" | tr -d ' \n'
echo
"#,
        "asyncdirect",
    );
    assert_eq!(String::from_utf8_lossy(&stdout), "ff0a\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn issue3965_respawn_plain_utf8_and_carrier_values_unchanged() {
    // Guard: values with no raw high bytes must cross a respawn unchanged —
    // a true UTF-8 char (U+00E9) and a carrier byte (0x1C, marker-tagged in
    // storage) both round-trip byte-exactly, and the foreign face agrees.
    let (stdout, stderr, code) = rubash_script(
        r#"D=$G4010_DIR
export z=$'a\303\251b'
export w=$'a\x1cb'
{ printf '%s' "$z" | od -An -tx1 -v | tr -d ' \n'; echo; } > "$D/z.out" &
wait
cat "$D/z.out"
{ printf '%s' "$w" | od -An -tx1 -v | tr -d ' \n'; echo; } > "$D/w.out" &
wait
cat "$D/w.out"
/usr/bin/printenv w | od -An -tx1 -v | tr -d ' \n'
echo
"#,
        "guard",
    );
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        "61c3a962\n611c62\n611c620a\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
