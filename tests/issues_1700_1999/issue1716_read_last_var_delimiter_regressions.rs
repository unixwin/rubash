//! rubash#1716 regressions: a single (last) `read` variable kept a
//! lone trailing IFS delimiter verbatim. GNU read.def:1113-1126 binds
//! the last variable through get_word_from_string (subst.c:3268): if
//! extracting the first field exhausts the line, the variable gets
//! the FIELD (trailing delimiter consumed); only when input remains
//! does it get the whole remainder with trailing IFS whitespace
//! stripped.
//!
//! Expectations verified against GNU bash 5.3.20 (self-compiled
//! oracle), including the audit issue's full boundary matrix.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1716-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1716_lone_trailing_delimiter_dropped() {
    // The audit's divergent set: exactly one non-whitespace IFS
    // delimiter in the line, sitting at the end.
    let (stdout, _, code) = rubash_file(
        "for inp in a ba Xa ca cba; do printf '%s\\n' \"$inp\" > in; IFS='a' read a < in; echo \"in=[$inp] a=[$a]\"; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "in=[a] a=[]\nin=[ba] a=[b]\nin=[Xa] a=[X]\nin=[ca] a=[c]\nin=[cba] a=[cb]\n"
    );
}

#[test]
fn issue1716_agreeing_controls_stay_verbatim() {
    // The audit's agreeing controls: delimiter runs, an earlier
    // occurrence, or the single occurrence not at the end — the whole
    // line is assigned verbatim on both shells.
    let (stdout, _, code) = rubash_file(
        "for inp in aa aaa baa acaa aba abca aX bab ab; do printf '%s\\n' \"$inp\" > in; IFS='a' read a < in; echo \"in=[$inp] a=[$a]\"; done\nprintf '\\n' > in; IFS='a' read a < in; echo \"in=[] a=[$a]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "in=[aa] a=[aa]\nin=[aaa] a=[aaa]\nin=[baa] a=[baa]\nin=[acaa] a=[acaa]\nin=[aba] a=[aba]\nin=[abca] a=[abca]\nin=[aX] a=[aX]\nin=[bab] a=[bab]\nin=[ab] a=[ab]\nin=[] a=[]\n"
    );
}

#[test]
fn issue1716_trailing_delimiter_other_ifs_shapes() {
    let (stdout, _, code) = rubash_file(
        "printf 'ba:\\n' > in; IFS=':' read x < in; echo \"colon x=[$x]\"\nprintf 'ba: \\n' > in; IFS=': ' read x < in; echo \"colonspace x=[$x]\"\nprintf 'a:\\n' > in; IFS=':' read x < in; echo \"single x=[$x]\"\nprintf 'ba\\n' > in; IFS='a' read -r x < in; echo \"raw x=[$x]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "colon x=[ba]\ncolonspace x=[ba]\nsingle x=[a]\nraw x=[b]\n"
    );
}

#[test]
fn issue1716_escaped_trailing_space_is_field_data() {
    // Extraction exhausts the line without meeting a delimiter: the
    // extracted field is bound as-is, so the escaped trailing space
    // survives (the escape-blind remainder strip must not run).
    let (stdout, _, code) = rubash_file(
        "printf 'a\\\\ \\n' > in; IFS=' ' read x < in; echo \"x=[$x]\"\nprintf 'a\\\\ b\\n' > in; IFS=' ' read x < in; echo \"y=[$x]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=[a ]\ny=[a b]\n");
}

#[test]
fn issue1716_remainder_and_two_var_controls() {
    let (stdout, _, code) = rubash_file(
        "printf 'a::b\\n' > in; IFS=':' read x < in; echo \"resid x=[$x]\"\nprintf ':a\\n' > in; IFS=':' read x < in; echo \"lead x=[$x]\"\nprintf 'a b\\n' > in; IFS=' ' read x < in; echo \"words x=[$x]\"\nprintf 'a\\n' > in; IFS='a' read x y < in; echo \"two x=[$x] y=[$y]\"\nprintf 'paqa\\n' > in; IFS='a' read x y < in; echo \"two2 x=[$x] y=[$y]\"\nprintf 'paqaa\\n' > in; IFS='a' read x y < in; echo \"two3 x=[$x] y=[$y]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "resid x=[a::b]\nlead x=[:a]\nwords x=[a b]\ntwo x=[] y=[]\ntwo2 x=[p] y=[q]\ntwo3 x=[p] y=[qaa]\n"
    );
}
