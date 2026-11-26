//! rubash#1018 + rubash#1019: `select` must bind REPLY and print the menu once.
//!
//! GNU spec (vendored tree `bash.git @9c465866`, Bash-5.3 patch 20):
//! - execute_cmd.c:3426-3493 select_query: the menu prints iff print_menu,
//!   the prompt prints every pass, and the line is read through the read
//!   builtin with no NAMEs — builtins/read.def:987-1011 binds it to REPLY
//!   verbatim (no IFS strip) with backslash unescaping. An empty REPLY
//!   re-arms print_menu and reprompts WITHOUT running the body; an invalid
//!   number returns "" (the body still runs with an empty selection).
//! - execute_cmd.c:3495-3631 execute_select_command with config-top.h:89
//!   KSH_COMPATIBLE_SELECT: show_menu starts 1 and, after each body run,
//!   drops to 0 unless the body emptied REPLY.
//! - general.c:248 valid_number (strtoimax, base 10): leading/trailing
//!   C-locale whitespace skipped, optional sign, overflow fails — so
//!   ` 2 ` selects 2 while `   ` (no digits) runs the body empty.
//!
//! Expectations byte-verified against GNU Bash 5.3.20 script-file probes
//! (`printf .. | { select ..; }`, stdout vs stderr captured separately).

use std::process::Command;

fn rubash(script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (output.stdout, output.stderr, output.status.code())
}

#[test]
fn select_binds_reply_to_selection_number() {
    // rubash#1018: GNU binds REPLY to the entered number ("2"), rubash
    // left it untouched.
    let (stdout, stderr, code) = rubash(
        "printf '2\\n' | { select ch in foo bar baz; do echo \"ch=[$ch] REPLY=[$REPLY]\"; break; done; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"ch=[bar] REPLY=[2]\n");
    assert_eq!(stderr, b"1) foo\n2) bar\n3) baz\n#? ");
}

#[test]
fn select_reply_binding_overwrites_preset_sentinel() {
    // rubash#1018, second probe: a preset REPLY must not survive the read.
    let (stdout, _, code) = rubash(
        "printf '1\\n' | { REPLY=sentinel; select ch in aa bb; do echo \"ch=[$ch] REPLY=[$REPLY]\"; break; done; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"ch=[aa] REPLY=[1]\n");
}

#[test]
fn select_prints_menu_once_for_invalid_then_valid_input() {
    // rubash#1019: `9` is out of range (body runs empty), `1` selects;
    // GNU prints the menu once, then one `#? ` prompt per pass.
    let (stdout, stderr, code) = rubash(
        "printf '9\\n1\\n' | { select ch in foo bar; do echo \"ch=[$ch]\"; [[ -n $ch ]] && break; done; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"ch=[]\nch=[foo]\n");
    assert_eq!(stderr, b"1) foo\n2) bar\n#? #? ");
}

#[test]
fn select_empty_line_reprints_menu_without_running_body() {
    // GNU select_query: empty REPLY re-arms the menu and reprompts; the
    // body runs only for the later `1`.
    let (stdout, stderr, code) = rubash(
        "printf '\\n1\\n' | { select ch in foo bar; do echo \"ch=[$ch] REPLY=[$REPLY]\"; [[ -n $ch ]] && break; done; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"ch=[foo] REPLY=[1]\n");
    assert_eq!(stderr, b"1) foo\n2) bar\n#? 1) foo\n2) bar\n#? ");
}

#[test]
fn select_whitespace_only_line_runs_body_with_verbatim_reply() {
    // `   ` is not empty (valid_number finds no digits): the body runs
    // with an empty selection and REPLY keeps its spaces.
    let (stdout, stderr, code) = rubash(
        "printf '   \\n1\\n' | { select ch in foo bar; do echo \"ch=[$ch] REPLY=[$REPLY]\"; [[ -n $ch ]] && break; done; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"ch=[] REPLY=[   ]\nch=[foo] REPLY=[1]\n");
    assert_eq!(stderr, b"1) foo\n2) bar\n#? #? ");
}

#[test]
fn select_body_clearing_reply_reprints_menu() {
    // KSH_COMPATIBLE_SELECT tail: a body that empties REPLY re-arms the
    // menu for the next pass.
    let (stdout, stderr, code) = rubash(
        "printf '1\\n1\\n' | { select ch in aa bb; do echo \"ch=[$ch] REPLY=[$REPLY]\"; if [[ -n $GOT ]]; then break; fi; GOT=1; REPLY=; done; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"ch=[aa] REPLY=[1]\nch=[aa] REPLY=[1]\n");
    assert_eq!(stderr, b"1) aa\n2) bb\n#? 1) aa\n2) bb\n#? ");
}

#[test]
fn select_unterminated_tail_binds_reply_and_fails_without_body() {
    // GNU zread/read.def: the partial line still binds REPLY but fails
    // the read — stdout newline, status 1, body never runs.
    let (stdout, stderr, code) = rubash(
        "printf '2' | { select ch in foo bar; do echo \"body ch=[$ch]\"; break; done; echo \"after rc=$? REPLY=[$REPLY]\"; }",
    );
    assert_eq!(stdout, b"\nafter rc=1 REPLY=[2]\n");
    assert_eq!(stderr, b"1) foo\n2) bar\n#? ");
    assert_eq!(code, Some(0));
}

#[test]
fn select_backslash_newline_continues_without_reprinting_menu() {
    // GNU builtins/read.def:802-830: `\<newline>` vanishes mid-stream and
    // reading continues, so `\` + `1` is one logical line `1` — one menu,
    // one prompt, ch=aa REPLY=1.
    let (stdout, stderr, code) = rubash(
        "printf '%s\\n' '\\' '1' | { select ch in aa bb; do echo \"ch=[$ch] REPLY=[$REPLY]\"; [[ -n $ch ]] && break; done; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"ch=[aa] REPLY=[1]\n");
    assert_eq!(stderr, b"1) aa\n2) bb\n#? ");
}

#[test]
fn select_escaped_backslash_does_not_continue() {
    // Even trailing run (`a\\`): no continuation — the line unescapes to
    // `a\`, valid_number fails, the body runs empty, and the next line
    // reads normally with no menu reprint (REPLY non-empty both times).
    let (stdout, stderr, code) = rubash(
        "printf '%s\\n' 'a\\\\' 'b' | { select ch in aa bb; do echo \"ch=[$ch] REPLY=[$REPLY]\"; [[ $REPLY == b ]] && break; done; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"ch=[] REPLY=[a\\]\nch=[] REPLY=[b]\n");
    assert_eq!(stderr, b"1) aa\n2) bb\n#? #? ");
}

#[test]
fn select_clean_eof_clears_reply_and_fails_without_body() {
    // GNU: immediate EOF binds REPLY="" and fails — stdout newline,
    // status 1, body never runs.
    let (stdout, stderr, code) = rubash(
        "printf '' | { REPLY=sentinel; select ch in aa bb; do echo body; break; done; echo \"rc=$? REPLY=[$REPLY]\"; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"\nrc=1 REPLY=[]\n");
    assert_eq!(stderr, b"1) aa\n2) bb\n#? ");
}
