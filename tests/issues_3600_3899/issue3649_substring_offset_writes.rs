//! rubash#3649 regressions: an arithmetic write in a list-subject
//! substring offset (`${a[@]:$((i+=1))}`, `${@:…}`, `${a[*]:…}`) was
//! silently lost -- the expansion value was right, but `i` stayed 0.
//! GNU applies the write (subst.c verify_substring_values -> evalexp
//! against the live environment).
//!
//! Root cause: the whole-word list fan-outs (`array_at_word_values`,
//! `quoted_positional_at_word_values`) expand these forms through the
//! `&self` substring parse, whose offset arithmetic ran on a cloned env
//! and discarded the writes; the fan-out then returns the sliced values
//! directly, so the `&mut` substring parse (which applies writes live)
//! never runs for them. The fix makes the offset/length writes ride
//! with the parse result (`parse_parameter_substring_with_writes`) and
//! commits them exactly once, only at the consumer that uses the result
//! -- probes discard them, so a bare scalar `${v:i+=1}` still evaluates
//! exactly once.
//!
//! Expectations verified byte-for-byte against GNU bash 5.2.21.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3649-{}-{}",
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
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn check(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "script: {script}\nstderr: {stderr}");
    assert_eq!(stderr, "", "script: {script}");
    assert_eq!(stdout, expected_stdout, "script: {script}");
}

#[test]
fn seed_array_at_quoted() {
    check(
        "i=0; a=(p q); echo \"${a[@]:$((i+=1))}\"; echo \"i=$i\"\n",
        "q\ni=1\n",
    );
}

#[test]
fn positional_at_quoted() {
    check(
        "i=0; set -- p q; echo \"${@: $((i+=1))}\"; echo \"i=$i\"\n",
        "p q\ni=1\n",
    );
}

#[test]
fn positional_at_unquoted() {
    check(
        "i=0; set -- p q; echo ${@: $((i+=1))}; echo \"i=$i\"\n",
        "p q\ni=1\n",
    );
}

#[test]
fn array_at_unquoted() {
    check(
        "i=0; a=(p q); echo ${a[@]:$((i+=1))}; echo \"i=$i\"\n",
        "q\ni=1\n",
    );
}

#[test]
fn scalar_control_already_green() {
    check(
        "i=0; v=abcdef; echo \"${v:$((i+=1))}\"; echo \"i=$i\"\n",
        "bcdef\ni=1\n",
    );
}

#[test]
fn scalar_bare_offset_applies_once() {
    // v1 global-queue design double-applied this (`i=2`): an
    // operator-arm drain fired between the fan-out probes and the real
    // expansion. The probe parses must not commit their writes.
    check(
        "i=0; v=abcdef; echo \"${v:i+=1}\"; echo \"i=$i\"\n",
        "bcdef\ni=1\n",
    );
}

#[test]
fn scalar_bare_offset_with_length() {
    check(
        "i=0; v=abcdef; echo \"${v:i+=1:2}\"; echo \"i=$i\"\n",
        "bc\ni=1\n",
    );
}

#[test]
fn positional_star_quoted() {
    // Positional substring offsets count $0 as 0, so offset 1 is $1
    // onwards, joined with the first IFS char.
    check(
        "i=0; set -- p q; echo \"${*:$((i+=1))}\"; echo \"i=$i\"\n",
        "p q\ni=1\n",
    );
}

#[test]
fn array_star_quoted() {
    check(
        "i=0; a=(p q); echo \"${a[*]:$((i+=1))}\"; echo \"i=$i\"\n",
        "q\ni=1\n",
    );
}

#[test]
fn offset_and_length_writes() {
    check(
        "i=0; j=0; a=(p q r); echo \"${a[@]:$((i+=1)):$((j+=1))}\"; echo \"i=$i j=$j\"\n",
        "q\ni=1 j=1\n",
    );
}

#[test]
fn length_write() {
    check(
        "i=0; a=(p q r); echo \"${a[@]:1:$((i+=1))}\"; echo \"i=$i\"\n",
        "q\ni=1\n",
    );
}

#[test]
fn two_words_observe_write_left_to_right() {
    // A slice runs to the end of the array: the first word yields
    // ("q", "r"), the second (offset 2 now) yields ("r").
    check(
        "i=0; a=(p q r); echo \"${a[@]:$((i+=1))}\" \"${a[@]:$((i+=1))}\"; echo \"i=$i\"\n",
        "q r r\ni=2\n",
    );
}

#[test]
fn for_list_array_at() {
    check(
        "i=0; a=(p q); for x in \"${a[@]:$((i+=1))}\"; do echo \"$x\"; done; echo \"i=$i\"\n",
        "q\ni=1\n",
    );
}

#[test]
fn comsub_does_not_leak_write() {
    check(
        "i=0; a=(p q); echo \"$(echo \"${a[@]:$((i+=1))}\")\"; echo \"i=$i\"\n",
        "q\ni=0\n",
    );
}

#[test]
fn unset_array_offset_not_evaluated() {
    check(
        "i=0; echo \"${u[@]:$((i+=1))}\"; echo \"i=$i\"\n",
        "\ni=0\n",
    );
}

#[test]
fn out_of_range_offset_length_not_evaluated() {
    // GNU verify_substring_values returns -1 before the length text is
    // expanded or evaluated (rubash#2752).
    check(
        "i=0; j=0; a=(p q); echo \"${a[@]:$((i+=5)):$((j+=1))}\"; echo \"i=$i j=$j\"\n",
        "\ni=5 j=0\n",
    );
}

#[test]
fn bare_arith_offset_array_at() {
    check(
        "i=0; a=(p q); echo \"${a[@]:i+=1}\"; echo \"i=$i\"\n",
        "q\ni=1\n",
    );
}

#[test]
fn indirect_ref_slice() {
    check(
        "i=0; a=(p q); ref=\"a[@]\"; echo \"${!ref:$((i+=1))}\"; echo \"i=$i\"\n",
        "q\ni=1\n",
    );
}

#[test]
fn compound_assignment_slice() {
    check(
        "i=0; a=(p q r); b=(\"${a[@]:$((i+=1))}\"); echo \"${b[@]}\"; echo \"i=$i\"\n",
        "q r\ni=1\n",
    );
}

#[test]
fn assoc_slice() {
    check(
        "i=0; declare -A m=([k]=v1); echo \"${m[@]:$((i+=1))}\"; echo \"i=$i\"\n",
        "v1\ni=1\n",
    );
}
