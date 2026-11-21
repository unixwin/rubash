//! Issue rubash#2827: `[perf] [[ =~ ]]` regex compile cache regressions.
//!
//! Verifies that `CONDITIONAL_REGEX_CACHE` in `src/executor/conditional.rs`:
//! 1. Never cross-contaminates across `shopt -s nocasematch` / `shopt -u nocasematch`
//!    or `LC_ALL` locale mode transitions (byte mode vs UTF-8 text mode) for the
//!    same pattern.
//! 2. Preserves loop `[[ $s =~ ... ]]` semantics and `BASH_REMATCH` captures
//!    byte-for-byte.
//! 3. Stays bounded and functionally accurate under a flood of distinct patterns.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn nocasematch_toggle_same_pattern_never_cross_contaminates() {
    let script = r#"
pat='^(foo)_([a-z]+)$'
s='FOO_bar'

shopt -u nocasematch
[[ $s =~ $pat ]] && r1="y:${BASH_REMATCH[1]}" || r1="n:${#BASH_REMATCH[@]}"

shopt -s nocasematch
[[ $s =~ $pat ]] && r2="y:${BASH_REMATCH[1]}:${BASH_REMATCH[2]}" || r2="n:${#BASH_REMATCH[@]}"

shopt -u nocasematch
[[ $s =~ $pat ]] && r3="y:${BASH_REMATCH[1]}" || r3="n:${#BASH_REMATCH[@]}"

echo "$r1|$r2|$r3"
"#;
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.trim(), "n:0|y:FOO:bar|n:0");
}

#[test]
fn locale_mode_toggle_same_pattern_never_cross_contaminates() {
    let script = r#"
s='é'
pat='^.$'
class_pat='^[[:alpha:]]$'

LC_ALL=C
[[ $s =~ $pat ]] && c1=y || c1=n
[[ $s =~ $class_pat ]] && c2=y || c2=n

LC_ALL=C.UTF-8
[[ $s =~ $pat ]] && u1=y || u1=n
[[ $s =~ $class_pat ]] && u2=y || u2=n

LC_ALL=C
[[ $s =~ $pat ]] && c3=y || c3=n
[[ $s =~ $class_pat ]] && c4=y || c4=n

echo "$c1,$c2|$u1,$u2|$c3,$c4"
"#;
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.trim(), "n,n|y,y|n,n");
}

#[test]
fn loop_regex_match_and_bash_rematch_match_uncached_semantics() {
    let script = r#"
pat='^([a-z]+)-([0-9]+)(-[a-z]+)?$'
out=""
for s in "alpha-12" "BETA-99" "gamma-34-ok" "bad" "delta-0"; do
    if [[ $s =~ $pat ]]; then
        out+="[$?:${#BASH_REMATCH[@]}:${BASH_REMATCH[0]}:${BASH_REMATCH[1]}:${BASH_REMATCH[2]}:${BASH_REMATCH[3]-UNSET}:${BASH_REMATCH[3]+SET}]"
    else
        out+="[fail:$?:${#BASH_REMATCH[@]}]"
    fi
done
echo "$out"
"#;
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        "[0:4:alpha-12:alpha:12::SET][fail:1:0][0:4:gamma-34-ok:gamma:34:-ok:SET][fail:1:0][0:4:delta-0:delta:0::SET]"
    );
}

#[test]
fn invalid_regex_cache_preserves_status_2_and_diagnostic() {
    let script = r#"
pat='^(unclosed'
[[ abc =~ $pat ]]; rc1=$?
[[ abc =~ $pat ]]; rc2=$?
echo "rc1=$rc1 rc2=$rc2"
"#;
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "rc1=2 rc2=2");
    assert!(
        stderr.matches("invalid regular expression").count() == 2,
        "expected two invalid-regex diagnostics, got: {stderr:?}"
    );
}

#[test]
fn distinct_pattern_flood_stays_accurate() {
    let script = r#"
ok=0
for ((i=0; i<200; i++)); do
    s="k${i}=v${i}"
    pat="^k${i}=(v[0-9]+)$"
    if [[ $s =~ $pat ]] && [[ ${BASH_REMATCH[1]} == "v${i}" ]]; then
        ok=$((ok + 1))
    fi
done
# Re-run a previously evicted pattern to ensure re-compilation works cleanly.
[[ "k0=v0" =~ ^k0=(v[0-9]+)$ ]] && last="${BASH_REMATCH[1]}" || last="miss"
echo "ok=$ok last=$last"
"#;
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.trim(), "ok=200 last=v0");
}
