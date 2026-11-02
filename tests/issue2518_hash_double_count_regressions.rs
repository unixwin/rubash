//! rubash#2518 — a declined emulation consult double-recorded the hash
//! hit: `handle_external_file_builtins` recorded the resolution up
//! front (the consult) and, when the emulation arm declined, the
//! fall-through external path recorded the same execution again, so
//! `sed --version; hash` showed 2 hits where GNU shows 1.
//!
//! GNU ownership: findcmd.c:344 `search_for_command` — called exactly
//! once per command execution (execute_cmd.c:5808
//! execute_disk_command) — is the single hash-table interaction an
//! execution produces: a phash_search hit bump (findcmd.c:365 ->
//! hashlib.c:254 times_found++) or a phash_insert with found=1
//! (findcmd.c:416/423/426 -> hashcmd.c:117). The consult is not an
//! execution, so the fix records only after the emulation layer has
//! actually answered (`record_emulated_command_resolution`); a decline
//! leaves the count to the real-path record in external_inner.
//!
//! Expectations verified against GNU bash 5.3.20 (WSL script-file
//! oracle, 2026-11-02): the probe below is byte-identical on both
//! shells. The fixture `bin/sed` keeps the emulated name resolving on
//! PATH on any host; the `while read`/`case` filter normalizes the
//! host-specific path spelling so the assertion pins NAME and HIT
//! COUNT only (the path form itself is pinned by the unix-side
//! issue1648 tests).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2518-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// The full probe, one section per invariant:
/// A — declined consult (`sed --version` is not in the emulation's
///     grammar) counts ONE hit, not two;
/// B — an admitted emulation shape (`sed s/a/b/` on a herestring)
///     still counts one;
/// C — hit counts keep incrementing across repeated runs (1 + 2 = 3);
/// D — `hash -r` clears the table;
/// E — the plain non-emulated execution path is unchanged (one hit).
const PROBE: &str = "\
mkdir -p bin
printf '#!/bin/sh\\nexit 0\\n' > bin/sed
printf '#!/bin/sh\\nexit 0\\n' > bin/myecho
chmod +x bin/sed bin/myecho
export PATH=\"$PWD/bin:$PATH\"
hash -r
sed --version >/dev/null 2>&1
hash | while read -r hits path; do case \"$path\" in */sed|*/sed.exe) echo \"A sed $hits\";; esac; done
hash -r
sed s/a/b/ <<< hi >/dev/null
hash | while read -r hits path; do case \"$path\" in */sed|*/sed.exe) echo \"B sed $hits\";; esac; done
sed --version >/dev/null 2>&1
sed --version >/dev/null 2>&1
hash | while read -r hits path; do case \"$path\" in */sed|*/sed.exe) echo \"C sed $hits\";; esac; done
hash -r
hash; echo \"D rc=$?\"
myecho >/dev/null 2>&1
hash | while read -r hits path; do case \"$path\" in */myecho|*/myecho.exe) echo \"E myecho $hits\";; esac; done
";

#[test]
fn declined_emulation_consult_counts_one_hash_hit() {
    let dir = tempdir("declined");
    let script = dir.join("probe.sh");
    std::fs::write(&script, PROBE).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let expected = "A sed 1\nB sed 1\nC sed 3\nhash: hash table empty\nD rc=0\nE myecho 1\n";
    assert_eq!(stdout, expected, "stderr: {stderr}");
    assert!(output.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The declined-consult shape in isolation, with a second emulated
/// name (`mkdir --version`, a different decline gate) so the fix is
/// pinned at the dispatch class, not the sed symptom.
#[test]
fn declined_consult_on_another_emulated_name_counts_one() {
    let dir = tempdir("mkdir");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "\
mkdir -p bin
printf '#!/bin/sh\\nexit 0\\n' > bin/mkdir
chmod +x bin/mkdir
export PATH=\"$PWD/bin:$PATH\"
hash -r
mkdir --version >/dev/null 2>&1
hash | while read -r hits path; do case \"$path\" in */mkdir|*/mkdir.exe) echo \"mkdir $hits\";; esac; done
mkdir --version >/dev/null 2>&1
mkdir --version >/dev/null 2>&1
hash | while read -r hits path; do case \"$path\" in */mkdir|*/mkdir.exe) echo \"mkdir $hits\";; esac; done
",
    )
    .expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "mkdir 1\nmkdir 3\n", "stderr: {stderr}");
    assert!(output.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}
