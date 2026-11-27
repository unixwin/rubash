//! Issue rubash#1512: LC_COLLATE was never consulted — glob sorting and
//! `[[ < ]]` / `[[ > ]]` used byte order under every locale.
//!
//! GNU ownership:
//! - Glob order: pathexp.c globsort_namecmp() -> lib/sh/stringvec.c
//!   strvec_posixcmp() -> strcoll(3) with a first-byte/strcmp tie-break.
//! - `[[ a < b ]]`: execute_cmd.c passes TEST_LOCALE for `[[ ]]`, and
//!   test.c binary_test() compares with strcoll(3) when posixly_correct
//!   or the compatibility level is above 40 (strcmp otherwise). The
//!   `test`/`[` builtin reaches binary_test WITHOUT TEST_LOCALE and keeps
//!   byte order — it is a separate implementation and is NOT covered here.
//!
//! Root causes fixed:
//! - src/locale.rs host_collate_name() stripped the codeset on POSIX
//!   ("de_DE.UTF-8" -> "de_DE"); glibc setlocale() then fails whenever the
//!   locale exists only under its full name (the usual localedef result),
//!   so platform_collate() always fell back to strcmp.
//! - src/executor/conditional.rs compared `[[ < ]]` operands with Rust's
//!   bytewise str ordering and never called strcoll.
//! - LC_COLLATE was missing from the locale sync lists, so an in-script
//!   `export LC_COLLATE=...` never reached the collation readers (GNU
//!   variables.c sv_lc_collate calls setlocale on assignment).
//!
//! Every expectation below is the literal GNU bash 5.3.20 output for the
//! same script file. The de_DE.UTF-8 locale is compiled on the fly with
//! localedef into a private LOCPATH; when the host lacks the i18n sources
//! the locale-dependent tests skip (the C-locale tests still run).
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

const NAMES: [&str; 9] = ["a", "A", "ä", "b", "O", "ö", "q", "z", "Z"];
const BYTE_ORDER: &str = "A O Z a b q z ä ö";
const DE_ORDER: &str = "a A ä b O ö q z Z";

const MATRIX_SCRIPT: &str = r#"
echo "glob: $(echo *)"
for pair in "a B" "ä b" "B a" "a A" "Z a" "ö p" ".a a"; do
  set -- $pair
  if [[ $1 < $2 ]]; then r1=T; else r1=F; fi
  if [[ $1 > $2 ]]; then r2=T; else r2=F; fi
  echo "cmp: $1<$2=$r1 $1>$2=$r2"
done
"#;

const BYTE_CMPS: &str = "\
cmp: a<B=F a>B=T
cmp: ä<b=F ä>b=T
cmp: B<a=T B>a=F
cmp: a<A=F a>A=T
cmp: Z<a=T Z>a=F
cmp: ö<p=F ö>p=T
cmp: .a<a=T .a>a=F
";

const DE_CMPS: &str = "\
cmp: a<B=T a>B=F
cmp: ä<b=T ä>b=F
cmp: B<a=F B>a=T
cmp: a<A=T a>A=F
cmp: Z<a=F Z>a=T
cmp: ö<p=T ö>p=F
cmp: .a<a=T .a>a=F
";

/// Compile de_DE.UTF-8 once into a private LOCPATH directory.
/// None when the host cannot build it (no localedef / i18n sources).
fn locale_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue1512-locales-{}", std::process::id()));
        let target = dir.join("de_DE.UTF-8");
        if !target.is_dir() {
            // localedef creates the final component only, not parents.
            let _ = std::fs::create_dir_all(&dir);
            let output = Command::new("localedef")
                .args(["-i", "de_DE", "-f", "UTF-8"])
                .arg(&target)
                .output()
                .ok()?;
            if !output.status.success() || !target.is_dir() {
                eprintln!(
                    "localedef de_DE.UTF-8 failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                return None;
            }
        }
        Some(dir)
    })
    .as_ref()
}

/// A temp fixture directory holding the nine probe files.
fn fixture() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1512-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    for name in NAMES {
        std::fs::write(dir.join(name), []).expect("create fixture file");
    }
    dir
}

/// Run `script` (written to a file, per the repo's probe discipline) with
/// the rubash binary in the fixture dir under a controlled environment.
fn run(script: &str, lc_all: Option<&str>, locpath: Option<&PathBuf>) -> String {
    let dir = fixture();
    // The script file must live OUTSIDE the fixture dir: `echo *` would
    // otherwise glob the script itself into the expectations.
    let script_path = std::env::temp_dir().join(format!(
        "rubash-issue1512-probe-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&script_path, script).expect("write probe script");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .env_remove("LC_ALL")
        .env_remove("LC_COLLATE")
        .env_remove("LANG")
        .env_remove("LOCPATH");
    if let Some(value) = lc_all {
        command.env("LC_ALL", value);
    }
    if let Some(path) = locpath {
        command.env("LOCPATH", path);
    }
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&script_path);
    assert!(
        output.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn issue1512_glob_and_conditional_byte_order_in_c_locale() {
    let stdout = run(MATRIX_SCRIPT, Some("C"), None);
    assert_eq!(stdout, format!("glob: {BYTE_ORDER}\n{BYTE_CMPS}"));
}

#[test]
fn issue1512_glob_and_conditional_byte_order_in_c_utf8_locale() {
    let stdout = run(MATRIX_SCRIPT, Some("C.UTF-8"), None);
    assert_eq!(stdout, format!("glob: {BYTE_ORDER}\n{BYTE_CMPS}"));
}

#[test]
fn issue1512_glob_order_follows_lc_collate_de_de() {
    let Some(dir) = locale_dir() else {
        eprintln!("skip: de_DE.UTF-8 cannot be compiled on this host");
        return;
    };
    let stdout = run("echo \"glob: $(echo *)\"\n", Some("de_DE.UTF-8"), Some(dir));
    assert_eq!(stdout, format!("glob: {DE_ORDER}\n"));
}

#[test]
fn issue1512_conditional_compare_follows_lc_collate_de_de() {
    let Some(dir) = locale_dir() else {
        eprintln!("skip: de_DE.UTF-8 cannot be compiled on this host");
        return;
    };
    let stdout = run(MATRIX_SCRIPT, Some("de_DE.UTF-8"), Some(dir));
    assert_eq!(stdout, format!("glob: {DE_ORDER}\n{DE_CMPS}"));
}

#[test]
fn issue1512_export_lc_collate_switches_collation_in_script() {
    let Some(dir) = locale_dir() else {
        eprintln!("skip: de_DE.UTF-8 cannot be compiled on this host");
        return;
    };
    // LANG=C baseline; the exports must re-collate mid-script and back
    // (GNU variables.c sv_lc_collate watchers).
    let script = r#"
echo "g1: $(echo *)"
export LC_COLLATE=de_DE.UTF-8
echo "g2: $(echo *)"
if [[ a < B ]]; then echo "cmp: T"; else echo "cmp: F"; fi
export LC_COLLATE=C
echo "g3: $(echo *)"
if [[ a < B ]]; then echo "cmp3: T"; else echo "cmp3: F"; fi
"#;
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    let fix = fixture();
    let script_path = std::env::temp_dir().join(format!(
        "rubash-issue1512-probe-{}-export.sh",
        std::process::id()
    ));
    std::fs::write(&script_path, script).expect("write probe script");
    command
        .arg(&script_path)
        .current_dir(&fix)
        .env_remove("BASH_ENV")
        .env_remove("LC_ALL")
        .env_remove("LC_COLLATE")
        .env("LANG", "C")
        .env("LOCPATH", dir);
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&fix);
    let _ = std::fs::remove_file(&script_path);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        format!("g1: {BYTE_ORDER}\ng2: {DE_ORDER}\ncmp: T\ng3: {BYTE_ORDER}\ncmp3: F\n")
    );
}
