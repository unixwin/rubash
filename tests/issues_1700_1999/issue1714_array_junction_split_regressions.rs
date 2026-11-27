//! rubash#1714 regressions: the unquoted `${arr[@]}` / `$@` re-split
//! miscounted empty fields at element junctions. GNU joins the list
//! with the first IFS character and field-splits the joined string
//! (subst.c:2957 string_list_dollar_at; subst.c:12377 word_split), so
//! junction empties follow the single-string rules; under a null IFS
//! an entirely empty element/parameter contributes no field.
//!
//! Expectations verified against GNU bash 5.3.20 (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1714-{}-{}",
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

const SHOW: &str =
    "show() { echo \"n=$#\"; local f; for f in \"$@\"; do printf '[%s]' \"$f\"; done; echo; }\n";

#[test]
fn issue1714_delimiter_only_element_nonws_ifs() {
    let script = format!("{SHOW}IFS=':'\narr=('a:b' ':' 'c')\nset -- ${{arr[@]}}\nshow \"$@\"\n");
    let (stdout, _, code) = rubash_file(&script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=5\n[a][b][][][c]\n");
}

#[test]
fn issue1714_delimiter_only_element_whitespace_ifs() {
    let script =
        format!("{SHOW}IFS=$' \\t:'\narr=('a:b' ':' 'c')\nset -- ${{arr[@]}}\nshow \"$@\"\n");
    let (stdout, _, code) = rubash_file(&script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=3\n[a][b][c]\n");
}

#[test]
fn issue1714_empty_positional_null_ifs_vanishes() {
    let script = format!("{SHOW}IFS=''\nset -- '' x ' y '\nset -- $@\nshow \"$@\"\n");
    let (stdout, _, code) = rubash_file(&script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=2\n[x][ y ]\n");
}

#[test]
fn issue1714_leading_empty_element_kept() {
    let script = format!("{SHOW}IFS=': '\narr=('' x ' y ')\nset -- ${{arr[@]}}\nshow \"$@\"\n");
    let (stdout, _, code) = rubash_file(&script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=3\n[][x][y]\n");
}

#[test]
fn issue1714_for_loop_consumption_form() {
    let (stdout, _, code) = rubash_file(
        "IFS=':'\narr=('a:b' ':' 'c')\nc=0; for x in ${arr[@]}; do c=$((c+1)); printf '<%s>' \"$x\"; done\necho \" n=$c\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<a><b><><><c> n=5\n");
}

#[test]
fn issue1714_junction_matrix() {
    let script = format!(
        "{SHOW}IFS=':'\narr=(':' 'a'); set -- ${{arr[@]}}; show \"$@\"\narr=('a:' ':'); set -- ${{arr[@]}}; show \"$@\"\narr=('' ''); set -- ${{arr[@]}}; show \"$@\"\narr=(''); set -- ${{arr[@]}}; show \"$@\"\narr=('a' '' 'b'); set -- ${{arr[@]}}; show \"$@\"\n"
    );
    let (stdout, _, code) = rubash_file(&script);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "n=3\n[][][a]\nn=3\n[a][][]\nn=1\n[]\nn=0\n\nn=3\n[a][][b]\n"
    );
}

#[test]
fn issue1714_mixed_word_junctions() {
    // The same junction rule inside mixed words and for-lists: an unquoted
    // ${arr[@]}/$@ segment joins with IFS[0] into the word's expansion
    // string before splitting, so affixes attach to the first/last field
    // and junction empties survive (probe matrix vs GNU 5.3.20).
    let script = r#"
f() { printf 'argc=%s' "$#"; for a in "$@"; do printf ' <%s>' "$a"; done; echo; }
IFS=':'
arr=('a:b' ':' 'c')
f x${arr[@]}y
f pre${arr[@]}
f ${arr[@]}suf
set -- 'a:b' ':' 'c'
f x$@y
f $@$@
arr2=('p' '' 'q')
f x${arr2[@]}y
"#;
    let (stdout, _, code) = rubash_file(script);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "argc=5 <xa> <b> <> <> <cy>\n\
         argc=5 <prea> <b> <> <> <c>\n\
         argc=5 <a> <b> <> <> <csuf>\n\
         argc=5 <xa> <b> <> <> <cy>\n\
         argc=9 <a> <b> <> <> <ca> <b> <> <> <c>\n\
         argc=3 <xp> <> <qy>\n"
    );
}

#[test]
fn issue1714_positional_junctions() {
    let script = format!(
        "{SHOW}IFS=':'\nset -- 'a:' ':b'; set -- $@; show \"$@\"\nset -- ''; set -- $@; show \"$@\"\nset -- '' ''; set -- $@; show \"$@\"\n"
    );
    let (stdout, _, code) = rubash_file(&script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=4\n[a][][][b]\nn=0\n\nn=1\n[]\n");
}

#[test]
fn issue1714_null_ifs_array_elements() {
    // Null IFS: no splitting; empty elements vanish, but a
    // whitespace-only element is a real word and survives.
    let script = format!(
        "{SHOW}IFS=''\narr=('' x); set -- ${{arr[@]}}; show \"$@\"\narr=(' ' 'x'); set -- ${{arr[@]}}; show \"$@\"\narr=('a b' 'c'); set -- ${{arr[@]}}; show \"$@\"\n"
    );
    let (stdout, _, code) = rubash_file(&script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=1\n[x]\nn=2\n[ ][x]\nn=2\n[a b][c]\n");
}
