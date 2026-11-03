//! Deep-audit round 2, lane B2 (parallel expansion walks) regressions.
//!
//! B2-F03 — prompt `\nnn` octal escapes. GNU decode_prompt_string
//! (parse.y:6326-6329) copies a three-byte window starting at the first
//! octal digit (NUL-padded at end of string) and passes it to read_octal
//! (builtins/common.c:562), which fails unless the whole window is octal
//! digits. So `\101` decodes, `\7` at end of string decodes, but `\07 `,
//! `\18` and `\70x` stay literal, and `\000` contributes no byte
//! (parse.y:6332-6337 CTLESC/CTLNUL pair). rubash's reader consumed up
//! to three octal digits greedily and decoded whenever any were present.
//!
//! B2-F07 — scalar array append `arr+=word` (no parens) is not a
//! compound list: GNU binds the one expanded word to element 0 verbatim
//! (expand_string_assignment, subst.c:4345 — no pathname expansion; the
//! `[subscript]=` element grammar exists only inside
//! assign_compound_array_list, arrayfunc.c:700, entered solely for
//! parenthesized values). rubash ran scalar appends through the
//! compound-list token machinery in both append_array_value copies
//! (executor + declare storage), so `a+=[2]=z` wrote element 2, `a+=[0]=z`
//! replaced element 0, and `a+=*.txt` glob-expanded into new elements.

use std::process::Command;

fn shell_bin() -> std::path::PathBuf {
    // NOTE: the `bash` bin is a thin forwarder to the INSTALLED Niubash
    // (src/bin/bash.rs) — testing through it would exercise the old engine.
    // The real engine binary is `rubash`.
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return std::path::PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

fn run(script: &str) -> (i32, String, String) {
    let output = Command::new(shell_bin())
        .args(["-c", script])
        .output()
        .expect("run shell");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Decode `PS1` (set to the given literal escape text) through `${PS1@P}`
/// and return the output bytes as lowercase hex.
fn prompt_hex(escape_text: &str) -> String {
    let script = format!("PS1='{escape_text}'; printf '%s' \"${{PS1@P}}\" | od -An -tx1");
    let (rc, stdout, stderr) = run(&script);
    assert_eq!(rc, 0, "prompt decode failed: {stderr}");
    stdout.split_whitespace().collect()
}

#[test]
fn b2_f03_prompt_octal_full_window_decodes() {
    // \101 -> 'A'
    assert_eq!(prompt_hex(r"A\101B"), "414142");
    // \077 -> '?'
    assert_eq!(prompt_hex(r"A\077B"), "413f42");
    // \010 -> BS
    assert_eq!(prompt_hex(r"A\010B"), "410842");
    // \033 -> ESC (single byte)
    assert_eq!(prompt_hex(r"A\033B"), "411b42");
    // \1011 -> 'A' then a literal '1' (exactly three digits consumed)
    assert_eq!(prompt_hex(r"A\1011B"), "41413142");
}

#[test]
fn b2_f03_prompt_octal_partial_window_stays_literal() {
    // Window "07B"/"07 "/"70B"/"7B"/"18B" is not all octal digits, so GNU
    // leaves the backslash and every digit literal (read_octal -> -1).
    assert_eq!(prompt_hex(r"A\07B"), "415c303742");
    assert_eq!(prompt_hex(r"A\07 B"), "415c30372042");
    assert_eq!(prompt_hex(r"A\70B"), "415c373042");
    assert_eq!(prompt_hex(r"A\7B"), "415c3742");
    assert_eq!(prompt_hex(r"A\18B"), "415c313842");
    assert_eq!(prompt_hex(r"A\1B"), "415c3142");
}

#[test]
fn b2_f03_prompt_octal_end_of_string_window_decodes() {
    // The window is NUL-padded at end of string, so a short digit run
    // there still decodes.
    assert_eq!(prompt_hex(r"A\7"), "4107");
    assert_eq!(prompt_hex(r"A\07"), "4107");
    assert_eq!(prompt_hex(r"A\70"), "4138");
}

#[test]
fn b2_f03_prompt_octal_nul_contributes_no_byte() {
    // \000 decodes to the CTLESC/CTLNUL pair internally; the resulting
    // string carries no byte for it and following text survives.
    assert_eq!(prompt_hex(r"A\000B"), "4142");
}

#[test]
fn b2_f07_scalar_append_bracket_text_is_literal() {
    let (rc, stdout, _) = run("a=(x); a+=[2]=z; declare -p a");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -a a=([0]=\"x[2]=z\")\n");

    let (rc, stdout, _) = run("a=(x y); a+=[0]=z; declare -p a");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -a a=([0]=\"x[0]=z\" [1]=\"y\")\n");

    let (rc, stdout, _) = run("a=(); a+=[3]=z; declare -p a");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -a a=([0]=\"[3]=z\")\n");
}

#[test]
fn b2_f07_scalar_append_is_not_glob_expanded() {
    let (rc, stdout, _) = run("a=(x); a+='*.txt'; declare -p a");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -a a=([0]=\"x*.txt\")\n");

    let (rc, stdout, _) = run("a=(x); a+=*.txt; declare -p a");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -a a=([0]=\"x*.txt\")\n");
}

#[test]
fn b2_f07_declare_scalar_append_bracket_text_is_literal() {
    let (rc, stdout, _) = run("declare -a a=(x); declare 'a+=[2]=z'; declare -p a");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -a a=([0]=\"x[2]=z\")\n");

    let (rc, stdout, _) = run("declare -a b=(x); declare 'b+=*.txt'; declare -p b");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -a b=([0]=\"x*.txt\")\n");
}

#[test]
fn b2_f07_compound_and_element_append_forms_still_work() {
    // Control: the compound `(...)` element grammar is untouched —
    // `[sub]=` inside parens still addresses elements, and the true
    // `a[sub]+=` element-append form still appends per element.
    let (rc, stdout, _) = run("a=(x); a+=([2]=z); declare -p a");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -a a=([0]=\"x\" [2]=\"z\")\n");

    let (rc, stdout, _) = run("a=(x y); a[1]+=z; declare -p a");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -a a=([0]=\"x\" [1]=\"yz\")\n");
}
