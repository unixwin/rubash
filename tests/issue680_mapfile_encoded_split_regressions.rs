//! rubash#680 (mapfile input-side): encoded-split contract.
//!
//! GNU builtins/mapfile.def:177-209 zgetline loop + bind_array_element
//! splits raw bytes with no re-encode. The fast path must be byte-identical
//! to the decode path; any doubt keeps the decode path (rubash#117). NUL
//! behavior is unchanged (pre-existing divergence, still decode path).

use std::process::Command;

fn rubash(source: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", source])
        .output()
        .expect("run rubash");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Plain newline input takes the fast path with identical storage.
#[test]
fn mapfile_plain_newline_split_is_stable() {
    assert_eq!(
        rubash("mapfile -t n <<< $'1\\n20\\n300'; declare -p n"),
        "declare -a n=([0]=\"1\" [1]=\"20\" [2]=\"300\")\n"
    );
}

/// ASCII non-carrier `-d` takes the fast path, with and without `-t`.
#[test]
fn mapfile_custom_delim_split_is_stable() {
    assert_eq!(
        rubash("mapfile -d : -t c <<< $'a:b:c'; declare -p c"),
        "declare -a c=([0]=\"a\" [1]=\"b\" [2]=$'c\\n')\n"
    );
    assert_eq!(
        rubash("mapfile -d : k <<< $'a:b'; declare -p k"),
        "declare -a k=([0]=\"a:\" [1]=$'b\\n')\n"
    );
}

/// Carrier bytes never take the fast path: they force the decode path,
/// whose output is pinned here byte-for-byte against GNU bash 5.3
/// (`declare -a c=([0]=$'a\021b' [1]=$'x\023y')`).
#[test]
fn mapfile_carrier_bytes_stay_on_decode_path() {
    assert_eq!(
        rubash("mapfile -t c <<< $'a\\x11b\\nx\\x13y'; declare -p c"),
        "declare -a c=([0]=$'a\\021b' [1]=$'x\\023y')\n"
    );
}
#[test]
fn mapfile_nul_delim_stays_on_decode_path() {
    let dir = std::env::temp_dir().join(format!("mapfile-nuldelim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("input.txt");
    std::fs::write(&file, b"a:b:c").unwrap();
    let script = format!(
        "mapfile -d '' z < {}; declare -p z",
        file.to_string_lossy().replace('\'', "'\\''")
    );
    assert_eq!(rubash(&script), "declare -a z=([0]=\"a:b:c\")\n");
    std::fs::remove_dir_all(&dir).ok();
}
