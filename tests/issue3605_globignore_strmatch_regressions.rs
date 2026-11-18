//! Issue rubash#3605 (class A): `enable -f` loadables calling back into the
//! shell needed an exported `strmatch`, and GLOBIGNORE matching must use the
//! same GNU strmatch engine for every pattern.
//!
//! GNU ownership:
//! - lib/glob/strmatch.c:32-39 `strmatch()` is exported from the bash image
//!   so `-undefined dynamic_lookup` loadables (glob-bracket.tests' strmatch.so)
//!   can call it; the FNM flag values live in lib/glob/strmatch.h:35-55.
//! - pathexp.c:543-546 `glob_name_is_acceptable()` runs EVERY ignore spec
//!   through `strmatch(pattern, name, FNM_PATHNAME | FNMATCH_EXTFLAG |
//!   FNMATCH_NOCASEGLOB)` — one matcher, no separate extglob engine.
//! - sm_loop.c:284-290: under FNM_PATHNAME the `*`+rest general case only
//!   recurses for `n < endp` where endp is the next `/` — an extglob sitting
//!   exactly AT the slash position is never tried, which is why GNU says
//!   `*@(/)cd/efg` does NOT match `ab/cd/efg` (glob-bracket.right #20).
//!
//! Root cause fixed: GLOBIGNORE specs containing extglob syntax were routed
//! to the conditional case-pattern engine, which lacks the PATHNAME `endp`
//! bound; `*@(/)cd/efg` wrongly suppressed `ab/cd/efg`. ignore_pattern_matches
//! now calls the unified `strmatch` port with GNU's exact flag set.
#![cfg(unix)]

use std::process::Command;

fn run(script: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3605-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(dir.join("ab/cd")).expect("mkdir fixture");
    std::fs::write(dir.join("ab/cd/efg"), "").expect("touch fixture");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .env_remove("LC_ALL")
        .env_remove("LC_COLLATE")
        .env_remove("LANG");
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// glob-bracket.tests pcheck semantics: GLOBIGNORE=$1 then `*/*/efg*`;
/// output the matched names. GNU 5.3 answers recorded per row.
#[test]
fn issue3605_globignore_extglob_pathname_bound() {
    let script = r#"
shopt -s extglob
GLOBIGNORE='ab@(/)cd/efg'
echo "p1: $(echo */*/efg*)"
GLOBIGNORE='*@(/)cd/efg'
echo "p2: $(echo */*/efg*)"
GLOBIGNORE='*/cd/efg'
echo "p3: $(echo */*/efg*)"
"#;
    // GNU: `ab@(/)cd/efg` matches (literal prefix + @(/)) -> suppressed;
    // `*@(/)cd/efg` does NOT match (`*` cannot reach the `/` where `@(`
    // sits) -> globbed; `*/cd/efg` matches (`*` stops at the first `/`)
    // -> suppressed.
    assert_eq!(run(script), "p1: */*/efg*\np2: ab/cd/efg\np3: */*/efg*\n");
}

/// Non-extglob ignore specs keep matching the full pathname under
/// FNM_PATHNAME: `*` inside a spec never crosses `/`.
#[test]
fn issue3605_globignore_plain_pathname() {
    let script = r#"
GLOBIGNORE='*/efg'
echo "p1: $(echo */*/efg*)"
GLOBIGNORE='*cd*'
echo "p2: $(echo */*/efg*)"
unset GLOBIGNORE
echo "p3: $(echo */*/efg*)"
"#;
    // `*/efg` would need `*` to span `ab/cd` -> GNU: no match -> globbed.
    // `*cd*` can match with `*` covering `ab/` and `/efg`? No: `*cd*`
    // under FNM_PATHNAME — `*` cannot cross `/`, but the literal `cd`
    // sits in the middle component; strmatch tries `*` up to the first
    // `/` only, so `*`=`ab`, `cd` then must match `/cd/...` — GNU: no
    // match -> globbed.
    assert_eq!(run(script), "p1: ab/cd/efg\np2: ab/cd/efg\np3: ab/cd/efg\n");
}

// ---------------------------------------------------------------------------
// The exported C ABI itself: glob-bracket.tests' strmatch.so calls the host
// image's `strmatch()` from inside a loadable builtin. GNU's exported symbol
// wraps `xstrmatch` (strmatch.c:33-39), so the loadable observes the same
// locale dispatch as the shell: MB_CUR_MAX > 1 + multibyte data -> wide
// compile. The fixture (tests/fixtures/dynamic_builtin/strmatch_probe.c)
// prints GNU's 0/1 return value; expectations were recorded from GNU bash
// 5.3.20 on WSL Linux under LC_ALL=C.UTF-8 and LC_ALL=C.
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn strmatch_probe_object() -> &'static Path {
    static OBJECT: OnceLock<PathBuf> = OnceLock::new();
    OBJECT.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/strmatch-probe-fixture");
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        let source = dir.join("strmatch_probe.c");
        std::fs::write(
            &source,
            include_str!("fixtures/dynamic_builtin/strmatch_probe.c"),
        )
        .expect("write fixture source");
        let object = dir.join("strmatch_probe.so");
        let compiler = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let status = Command::new(compiler)
            .args(["-shared", "-fPIC", "-O0", "-o"])
            .arg(&object)
            .arg(&source)
            .status()
            .expect("run C compiler for the strmatch loadable fixture");
        assert!(status.success(), "C fixture compile failed");
        object
    })
}

fn run_loadable(locale: &str) -> (String, String, Option<i32>) {
    let object = strmatch_probe_object();
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3605-loadable-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir fixture");
    let script_path = dir.join("probe.sh");
    std::fs::write(
        &script_path,
        format!(
            "enable -f {object} strmatch_probe\n\
             echo u1=$(strmatch_probe 'u/?.txt' 'u/Ä.txt')\n\
             echo u2=$(strmatch_probe 'u/??.txt' 'u/Ä.txt')\n\
             echo e1=$(strmatch_probe 'ab@(/)cd/efg' 'ab/cd/efg' 33)\n\
             echo e2=$(strmatch_probe '*@(/)cd/efg' 'ab/cd/efg' 33)\n\
             echo b1=$(strmatch_probe 'u/[Ä].txt' 'u/Ä.txt')\n",
            object = object.display()
        ),
    )
    .expect("write probe script");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(&script_path)
        .current_dir(&dir)
        .env("LC_ALL", locale)
        .env_remove("LC_CTYPE")
        .env_remove("LC_MESSAGES")
        .env_remove("LANG")
        .env_remove("BASH_ENV");
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// A loadable builtin resolves `strmatch` against the shell image and the
/// call follows xstrmatch: under UTF-8 `?` and `[Ä]` consume one character
/// (0 = match); under C both stay byte-wise (1 = no match for a 2-byte Ä).
#[test]
fn exported_strmatch_dispatches_on_the_locale() {
    let (stdout, stderr, code) = run_loadable("C.UTF-8");
    assert_eq!(
        stdout, "u1=strmatch=0\nu2=strmatch=1\ne1=strmatch=0\ne2=strmatch=1\nb1=strmatch=0\n",
        "UTF-8 stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));

    let (stdout, stderr, code) = run_loadable("C");
    assert_eq!(
        stdout, "u1=strmatch=1\nu2=strmatch=0\ne1=strmatch=0\ne2=strmatch=1\nb1=strmatch=1\n",
        "C stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
