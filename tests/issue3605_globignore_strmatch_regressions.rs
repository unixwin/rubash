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
