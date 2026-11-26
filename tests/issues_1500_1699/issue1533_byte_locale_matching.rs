//! Issue rubash#1533: in a single-byte locale (LC_ALL=C) glob `?`
//! must match ONE BYTE, but the matcher staged Unicode chars, so a
//! multibyte character counted once instead of once per byte:
//! `${v#h?llo}` on v='héllo' removed the whole value (GNU keeps it)
//! and `${v#h?}` removed the whole é (GNU leaves its trailing byte).
//!
//! GNU ownership: the matchers walk byte strings when the locale is
//! not multibyte — subst.c pat_substr / match_upattern and lib/glob
//! sm_loop.c advance one `unsigned char` per `?`; there is no
//! character decoding step. The fix stages pattern and word as byte
//! units (one unit per UTF-8 byte of any non-ASCII text char) whenever
//! LC_CTYPE is not UTF-8, across the conditional/case matchers, the
//! extglob entries, prefix/suffix removal (byte-position candidates)
//! and pathname glob staging, while UTF-8 locales keep character
//! semantics. Case folding in a byte locale is ASCII-only.
//!
//! Every expectation below is the literal GNU bash 5.3.20 output for
//! the same script file. The C and C.UTF-8 tiers need no compiled
//! locale and always run; de_DE.UTF-8 is built with localedef into a
//! private LOCPATH (the issue1512 approach) and must equal C.UTF-8 on
//! this battery (no collation-sensitive forms).
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

const SCRIPT: &str = r#"
v='héllo'
V='HÉLLO'
echo "rem1 [${v#h?llo}]"
printf 'rem2: %s\n' "${v#h?}" | od -An -tx1 | tr -d ' \n'; echo
echo "rem3 [${v##h*}]"
echo "rem4 [${v%o?}]"
echo "rem5 [${v%%?*}]"
echo "rem6 [${v#?}]"
echo "rem7 [${v#??????}]"
echo "sub1 [${v/h?llo/X}]"
echo "sub2 [${v//?/x}]"
echo "sub3 [${v/é/X}]"
echo "sub4 [${v/hél/Y}]"
[[ $v == h?llo ]] && echo "eq1 T" || echo "eq1 F"
[[ $v == h?????o ]] && echo "eq2 T" || echo "eq2 F"
[[ $v == ????? ]] && echo "eq3 T" || echo "eq3 F"
[[ $v == ?????? ]] && echo "eq4 T" || echo "eq4 F"
[[ $v == h[?]llo ]] && echo "eq5 T" || echo "eq5 F"
[[ $v == [é]llo ]] && echo "eq6 T" || echo "eq6 F"
[[ $v == h[é]llo ]] && echo "eq7 T" || echo "eq7 F"
[[ $v == *[!a-z] ]] && echo "eq8 T" || echo "eq8 F"
case $v in
  h?llo) echo "case1 T" ;;
  *) echo "case1 F" ;;
esac
case $v in
  h?????o) echo "case2 T" ;;
  *) echo "case2 F" ;;
esac
shopt -s nocasematch
[[ $V == h?llo ]] && echo "nc1 T" || echo "nc1 F"
[[ $V == HÉLLO ]] && echo "nc2 T" || echo "nc2 F"
shopt -u nocasematch
echo "len ${#v}"
echo "substr [${v:1:1}]"
echo "substr2 [${v:1:2}]"
d=$(mktemp -d); cd "$d" || exit 1
touch 'héllo' plain
echo "glob1: $(echo h?llo)"
echo "glob2: $(echo h?????o)"
echo "glob3: $(echo h????o)"
echo "glob4: $(echo *)"
shopt -s nocaseglob
echo "glob5: $(echo H?LLO)"
shopt -u nocaseglob
"#;

const EXPECTED_C: &str = "\
rem1 [héllo]
72656d323a20a96c6c6f0a
rem3 []
rem4 [héllo]
rem5 []
rem6 [éllo]
rem7 []
sub1 [héllo]
sub2 [xxxxxx]
sub3 [hXllo]
sub4 [Ylo]
eq1 F
eq2 F
eq3 F
eq4 T
eq5 F
eq6 F
eq7 F
eq8 F
case1 F
case2 F
nc1 F
nc2 T
len 6
substr [�]
substr2 [é]
glob1: h?llo
glob2: h?????o
glob3: héllo
glob4: héllo plain
glob5: H?LLO
";

const EXPECTED_UTF8: &str = "\
rem1 []
72656d323a206c6c6f0a
rem3 []
rem4 [héllo]
rem5 []
rem6 [éllo]
rem7 [héllo]
sub1 [X]
sub2 [xxxxx]
sub3 [hXllo]
sub4 [Ylo]
eq1 T
eq2 F
eq3 T
eq4 F
eq5 F
eq6 F
eq7 T
eq8 F
case1 T
case2 F
nc1 T
nc2 T
len 5
substr [é]
substr2 [él]
glob1: héllo
glob2: h?????o
glob3: h????o
glob4: héllo plain
glob5: héllo
";

/// Compile de_DE.UTF-8 once into a private LOCPATH directory (the
/// issue1512 approach).
fn locale_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue1533-locales-{}", std::process::id()));
        let target = dir.join("de_DE.UTF-8");
        if !target.is_dir() {
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

fn run(lc_all: &str, locpath: Option<&PathBuf>) -> String {
    let script_path = std::env::temp_dir().join(format!(
        "rubash-issue1533-probe-{}-{}.sh",
        std::process::id(),
        lc_all.replace(['.', '/'], "_")
    ));
    std::fs::write(&script_path, SCRIPT).expect("write probe script");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", lc_all);
    if let Some(path) = locpath {
        command.env("LOCPATH", path);
    }
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_file(&script_path);
    assert!(
        output.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn issue1533_byte_semantics_in_c_locale() {
    assert_eq!(run("C", None), EXPECTED_C);
}

#[test]
fn issue1533_char_semantics_kept_in_c_utf8_locale() {
    assert_eq!(run("C.UTF-8", None), EXPECTED_UTF8);
}

#[test]
fn issue1533_char_semantics_kept_in_de_de_utf8_locale() {
    let Some(dir) = locale_dir() else {
        eprintln!("skip: de_DE.UTF-8 cannot be compiled on this host");
        return;
    };
    assert_eq!(run("de_DE.UTF-8", Some(dir)), EXPECTED_UTF8);
}
