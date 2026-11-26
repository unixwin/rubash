//! rubash#1717 — `[[ =~ ]]` accepted the non-POSIX class names
//! `[[:word:]]` and `[[:ascii:]]`; GNU rejects every class name outside
//! the twelve POSIX ones with rc=2 and the regcomp REG_ECTYPE text
//! "Invalid character class name".
//!
//! GNU owner: lib/sh/shmatch.c:86 sh_regmatch() hands the pattern to
//! libc regcomp, whose `[:name:]` validation is the wctype property
//! table — exactly {alnum alpha blank cntrl digit graph lower print
//! punct space upper xdigit}. "word"/"ascii" are regex-crate (Perl)
//! class names, not wctype properties, so glibc and bionic both
//! reject them (dual-oracle verified, LC_ALL=C and C.UTF-8 alike).
//!
//! The glob matcher is a different GNU subsystem with its own table:
//! lib/glob/smatch.c:196-200 cclass_name[] deliberately carries 14
//! names including "ascii" and "word" (CC_ASCII / CC_WORD), and an
//! unknown glob class is skipped (no match), never an error. The
//! control group at the bottom pins that the =~ tightening does not
//! leak into the glob side.
//!
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1717-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(tag: &str, body: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .env("LC_ALL", lc_all)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const INVALID_PROBE: &str = "\
[[ a =~ [[:word:]] ]]; echo \"word=$?\"
[[ _ =~ [[:word:]] ]]; echo \"wordus=$?\"
[[ 5 =~ [[:ascii:]] ]]; echo \"ascii=$?\"
[[ a =~ [[:foo:]] ]]; echo \"foo=$?\"
[[ a =~ [^[:word:]] ]]; echo \"neg=$?\"
[[ a =~ [[:Word:]] ]]; echo \"cased=$?\"
p='[[:word:]]'; [[ a =~ $p ]]; echo \"var=$?\"
";

fn assert_invalid_classes(tag: &str, lc_all: &str) {
    let (stdout, stderr, code) = run_script(tag, INVALID_PROBE, lc_all);
    assert_eq!(
        stdout, "word=2\nwordus=2\nascii=2\nfoo=2\nneg=2\ncased=2\nvar=2\n",
        "lc_all={lc_all}"
    );
    assert_eq!(code, Some(0), "lc_all={lc_all}");
    for pat in [
        "[[:word:]]",
        "[[:ascii:]]",
        "[[:foo:]]",
        "[^[:word:]]",
        "[[:Word:]]",
    ] {
        assert!(
            stderr.contains(&format!(
                "invalid regular expression `{pat}': Invalid character class name"
            )),
            "lc_all={lc_all} pat={pat} stderr: {stderr}"
        );
    }
}

#[test]
fn invalid_class_names_rejected_byte_locale() {
    assert_invalid_classes("c", "C");
}

#[test]
fn invalid_class_names_rejected_utf8_locale() {
    assert_invalid_classes("utf8", "C.UTF-8");
}

const LEGAL_PROBE: &str = "\
[[ a =~ [[:alpha:]] ]]; echo \"alpha-m=$?\"
[[ 5 =~ [[:alpha:]] ]]; echo \"alpha-n=$?\"
[[ 5 =~ [[:digit:]] ]]; echo \"digit-m=$?\"
[[ a =~ [[:digit:]] ]]; echo \"digit-n=$?\"
[[ a =~ [[:alnum:]] ]]; echo \"alnum-m=$?\"
[[ ' ' =~ [[:blank:]] ]]; echo \"blank-m=$?\"
[[ a =~ [[:blank:]] ]]; echo \"blank-n=$?\"
[[ a =~ [[:cntrl:]] ]]; echo \"cntrl-n=$?\"
[[ a =~ [[:graph:]] ]]; echo \"graph-m=$?\"
[[ ' ' =~ [[:graph:]] ]]; echo \"graph-n=$?\"
[[ a =~ [[:lower:]] ]]; echo \"lower-m=$?\"
[[ A =~ [[:lower:]] ]]; echo \"lower-n=$?\"
[[ a =~ [[:print:]] ]]; echo \"print-m=$?\"
[[ A =~ [[:upper:]] ]]; echo \"upper-m=$?\"
[[ a =~ [[:upper:]] ]]; echo \"upper-n=$?\"
[[ '.' =~ [[:punct:]] ]]; echo \"punct-m=$?\"
[[ a =~ [[:punct:]] ]]; echo \"punct-n=$?\"
[[ ' ' =~ [[:space:]] ]]; echo \"space-m=$?\"
[[ a =~ [[:space:]] ]]; echo \"space-n=$?\"
[[ f =~ [[:xdigit:]] ]]; echo \"xdigit-m=$?\"
[[ g =~ [[:xdigit:]] ]]; echo \"xdigit-n=$?\"
[[ 5 =~ [[:alpha:][:digit:]] ]]; echo \"union=$?\"
";

const LEGAL_EXPECTED: &str = "alpha-m=0\nalpha-n=1\ndigit-m=0\ndigit-n=1\nalnum-m=0\n\
blank-m=0\nblank-n=1\ncntrl-n=1\ngraph-m=0\ngraph-n=1\nlower-m=0\nlower-n=1\n\
print-m=0\nupper-m=0\nupper-n=1\npunct-m=0\npunct-n=1\nspace-m=0\nspace-n=1\n\
xdigit-m=0\nxdigit-n=1\nunion=0\n";

#[test]
fn legal_twelve_classes_unchanged_both_locales() {
    for lc_all in ["C", "C.UTF-8"] {
        let (stdout, stderr, code) = run_script("legal", LEGAL_PROBE, lc_all);
        assert_eq!(stdout, LEGAL_EXPECTED, "lc_all={lc_all}");
        assert_eq!(stderr, "", "lc_all={lc_all}");
        assert_eq!(code, Some(0), "lc_all={lc_all}");
    }
}

#[test]
fn non_class_bracket_shapes_stay_valid() {
    // `[:word:]` without the outer bracket pair is a plain character
    // list (members : w o r d), not a class span — GNU accepts it.
    let body = "\
[[ w =~ [:word:] ]]; echo \"list-w=$?\"
[[ : =~ [:word:] ]]; echo \"list-colon=$?\"
[[ a =~ [a-z] ]]; echo \"range=$?\"
";
    for lc_all in ["C", "C.UTF-8"] {
        let (stdout, stderr, code) = run_script("shapes", body, lc_all);
        assert_eq!(
            stdout, "list-w=0\nlist-colon=0\nrange=0\n",
            "lc_all={lc_all}"
        );
        assert_eq!(stderr, "", "lc_all={lc_all}");
        assert_eq!(code, Some(0), "lc_all={lc_all}");
    }
}

#[test]
fn glob_side_class_names_not_polluted() {
    // Control group: the glob matcher (lib/glob/smatch.c cclass_name[])
    // has its own 14-name table — word/ascii match there by design,
    // and an unknown glob class is a silent no-match, never rc=2.
    let body = "\
[[ a == [[:word:]] ]]; echo \"g-word-a=$?\"
[[ _ == [[:word:]] ]]; echo \"g-word-us=$?\"
[[ 5 == [[:ascii:]] ]]; echo \"g-ascii-5=$?\"
[[ a == [[:foo:]] ]]; echo \"g-foo=$?\"
[[ a == [[:alpha:]] ]]; echo \"g-alpha=$?\"
case a in ([[:word:]]) echo \"case-word=W\";; (*) echo \"case-word=N\";; esac
case 5 in ([[:ascii:]]) echo \"case-ascii=W\";; (*) echo \"case-ascii=N\";; esac
";
    let expected = "g-word-a=0\ng-word-us=0\ng-ascii-5=0\ng-foo=1\ng-alpha=0\n\
case-word=W\ncase-ascii=W\n";
    for lc_all in ["C", "C.UTF-8"] {
        let (stdout, stderr, code) = run_script("glob", body, lc_all);
        assert_eq!(stdout, expected, "lc_all={lc_all}");
        assert_eq!(stderr, "", "lc_all={lc_all}");
        assert_eq!(code, Some(0), "lc_all={lc_all}");
    }
}
