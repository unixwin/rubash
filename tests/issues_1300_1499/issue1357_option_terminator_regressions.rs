//! Option-terminator / declaration-builtin family — regression pins
//! for the z3-family issues rubash#1357, rubash#1377, rubash#1379,
//! rubash#1380 and rubash#1402, closed on workspace-probe evidence
//! (rubash-fix3 z3 probes t1357/t1377/t1379/t1380/t1402) that never
//! landed in the repo.
//!
//! * #1357: `shopt -s`/`-u` combined with `-q` (either order, combined
//!   or separate flags) must still change the option — GNU treats it
//!   as set/unset + query, not as a pure query.
//! * #1377: `local -- -p` must not swallow `-p` as an option end-marker
//!   operand: GNU rejects it as an invalid identifier (status 1).
//! * #1379: `return -- N` must return N (the `--` is consumed as
//!   end-of-options, not parsed as the status operand).
//! * #1380: flag-like operands after `--` in `dirs -- -l` / `-- -c`
//!   must be ignored as operands, not applied as flags.
//! * #1402: `local 1x=1` (digit-start name) must be rejected with
//!   "not a valid identifier" instead of silently succeeding.
//!
//! All expectations are GNU bash 5.3.20 behaviour for the verbatim
//! probe scripts, run from a file in a fresh directory.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-bfa-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    fn rubash_file(&self, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.0)
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn issue1357_shopt_set_unset_with_query_flag_still_applies() {
    let scratch = Scratch::new("1357");
    // Verbatim probe t1357.sh. The `shopt nullglob` query lines pin the
    // option state after each -q combination; statuses pin the query
    // result. GNU prints option names padded to column 20 + TAB.
    let script = "shopt -u nullglob\nshopt -sq nullglob; echo \"rc-sq-unset=$?\"\nshopt nullglob\nshopt -u nullglob\nshopt -s -q nullglob; echo \"rc-s-q-unset=$?\"\nshopt nullglob\nshopt -s nullglob\nshopt -uq nullglob; echo \"rc-uq-set=$?\"\nshopt nullglob\nshopt -s nullglob\nshopt -u -q nullglob; echo \"rc-u-q-set=$?\"\nshopt nullglob\nshopt -qs nullglob; echo \"rc-qs-unset=$?\"\nshopt nullglob\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(
        stdout,
        "rc-sq-unset=0\nnullglob            \ton\nrc-s-q-unset=0\nnullglob            \ton\nrc-uq-set=0\nnullglob            \toff\nrc-u-q-set=0\nnullglob            \toff\nrc-qs-unset=0\nnullglob            \ton\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1377_local_dashdash_does_not_swallow_operand() {
    let scratch = Scratch::new("1377");
    // Verbatim probe t1377.sh.
    let script = "f() { local -- -p; echo \"rc=$?\"; }; f\nf2() { local -- l=1; echo \"rc2=$? l=$l\"; }; f2\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "rc=1\nrc2=0 l=1\n");
    assert!(
        stderr.contains("`-p': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue1379_return_dashdash_consumes_terminator() {
    let scratch = Scratch::new("1379");
    // Verbatim probe t1379.sh.
    let script = "f() { return -- 1; }; f; echo \"rc=$?\"\ng() { return --; }; g; echo \"rc=$?\"\nh() { return -- 2; echo \"notreached\"; }; h; echo \"rc=$?\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "rc=1\nrc=0\nrc=2\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1380_dirs_dashdash_ignores_flag_like_operands() {
    let scratch = Scratch::new("1380");
    // Verbatim probe t1380.sh. (`dirs -- -c` must NOT clear the stack;
    // `dirs -- -l` must NOT switch to long format.)
    let script = "cd /; dirs -- -l; echo \"rc=$?\"\npushd /tmp >/dev/null; dirs -- -c; echo \"rc=$?\"; dirs\ndirs -l\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "/\nrc=0\n/tmp /\nrc=0\n/tmp /\n/tmp /\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1380_dirs_dashdash_from_home() {
    let scratch = Scratch::new("1380b");
    // Verbatim probe t1380b.sh, with HOME pinned to the scratch dir:
    // stack entries under HOME render as `~` in short form and as the
    // literal path with -l.
    let home = fs::canonicalize(&scratch.0)
        .expect("canonicalize scratch")
        .to_string_lossy()
        .into_owned();
    let script = "cd \"$HOME\"; dirs -- -l; echo \"rc=$?\"\ndirs -l; echo \"rc=$?\"\npushd /tmp >/dev/null; dirs -- -c; echo \"rc=$?\"; dirs; popd >/dev/null\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(
        stdout,
        format!("~\nrc=0\n{home}\nrc=0\n/tmp ~\nrc=0\n/tmp ~\n")
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1402_local_rejects_digit_start_names() {
    let scratch = Scratch::new("1402");
    // Verbatim probe t1402.sh.
    let script = "f() { local 1x=1; echo \"rc=$?\"; }; f\ng() { local 9var=x; echo \"rc=$?\"; }; g\nh() { local 1x; echo \"rc=$?\"; }; h\ni() { local ok=1; echo \"rc=$? ok=$ok\"; }; i\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "rc=1\nrc=1\nrc=1\nrc=0 ok=1\n");
    for part in [
        "`1x=1': not a valid identifier",
        "`9var=x': not a valid identifier",
        "`1x': not a valid identifier",
    ] {
        assert!(stderr.contains(part), "stderr {stderr:?} missing {part:?}");
    }
    assert_eq!(code, Some(0));
}
