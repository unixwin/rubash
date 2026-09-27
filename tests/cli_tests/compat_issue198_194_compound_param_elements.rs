//! ISSUES #198/#194 regressions: whole-word `$param` / `${param...}`
//! elements inside a compound array assignment.
//!
//! GNU arrayfunc.c:557 expand_compound_array_assignment expands each element
//! word exactly once (expand_words_no_vars, subst.c:12590) and field-splits
//! only unquoted words; field-split products are plain values because
//! W_ASSIGNMENT is set only by the parser on raw `[sub]=value` tokens
//! (parse.y:5786-5796), so assign_compound_array_list (arrayfunc.c:745-747)
//! never re-reads `[sub]=` text that came out of an expansion.

use std::process::Command;

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .output()
        .expect("run compound param element probe");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue198_post_expansion_subscript_words_are_plain_values() {
    // GNU: declare -a var=([0]="[\$(echo" [1]="total" [2]="0)]=1" [3]="[2]=2]")
    // — the comsub text inside $value is data and never executes, and the
    // [sub]=value form is not recognized after expansion.
    let (stdout, stderr, code) = run(
        "value='[$(echo total 0)]=1 [2]=2]'\ndeclare -a var\nvar=($value)\n\
         printf '<%s>' \"${var[@]}\"\nprintf '\\n'",
    );
    assert_eq!(stdout, "<[$(echo><total><0)]=1><[2]=2]>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue198_quoted_whole_word_stays_one_element() {
    let (stdout, stderr, code) = run("value='[5]=x [6]=y'\ndeclare -a var\nvar=(\"$value\")\n\
         echo \"n=${#var[@]} [0]=${var[0]} [6]=${var[6]-UNSET}\"");
    assert_eq!(stdout, "n=1 [0]=[5]=x [6]=y [6]=UNSET\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue198_mixed_body_elements_keep_plain_order() {
    let (stdout, stderr, code) =
        run("v='[9]=q [r]=w'\ndeclare -a e\ne=(a $v b)\nprintf '<%s>' \"${e[@]}\"\nprintf '\\n'");
    assert_eq!(stdout, "<a><[9]=q><[r]=w><b>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue198_raw_subscript_word_keeps_whole_nosplit_value() {
    // [2]=$v is a raw W_ASSIGNMENT word: W_NOSPLIT (parse.y:5790) keeps the
    // whole expanded value at index 2; `[r]=w' inside the value is data.
    let (stdout, stderr, code) = run("v='[9]=q [r]=w'\ndeclare -a m\nm=([2]=$v)\n\
         echo \"[2]=${m[2]} [0]=${m[0]-UNSET}\"");
    assert_eq!(stdout, "[2]=[9]=q [r]=w [0]=UNSET\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue194_quoted_array_star_joins_into_one_element() {
    let (stdout, stderr, code) = run("arrayA=(\"A\" \"B\" \"C\")\narrayB=( \"${arrayA[*]}\" )\n\
         echo \"n=${#arrayB[*]} [0]=${arrayB[0]} [1]=${arrayB[1]-EMPTY}\"");
    assert_eq!(stdout, "n=1 [0]=A B C [1]=EMPTY\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue194_unquoted_array_star_still_field_splits() {
    let (stdout, stderr, code) =
        run("arrayA=(\"A\" \"B\" \"C\")\narrayD=( ${arrayA[*]} )\necho \"d=${#arrayD[*]}\"");
    assert_eq!(stdout, "d=3\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue194_quoted_array_star_with_custom_ifs_join() {
    let (stdout, stderr, code) = run(
        "arrayA=(\"A\" \"B\" \"C\")\nIFS=:\narrayC=( \"${arrayA[*]}\" )\nunset IFS\n\
         echo \"c=<${arrayC[0]}> n=${#arrayC[*]}\"",
    );
    assert_eq!(stdout, "c=<A:B:C> n=1\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
