//! rubash#1704 regressions: in full nvm.sh context the alias paths
//! silently degraded — `nvm_list_aliases` printed nothing and the
//! respawned children logged `syntax error near unexpected token
//! '('`. Root cause family: nvm_list_aliases calls helper functions
//! in the BACKGROUND inside piped subshells; rubash respawns a
//! background job as a child process whose `-c` source serializes
//! every function body through `bash_command_text` +
//! `canonicalize_comsub_spans`, and that text pipeline was lossy in
//! two places:
//!
//! 1. `ComsubPrinter`'s pipeline arm joined stages with a bare
//!    space, flattening `$(echo abc | awk '...')` into the single
//!    simple command `echo abc awk '...'` on re-parse. GNU
//!    print_cmd.c:249-272 (cm_connection, connector '|') prints the
//!    stages joined by " | ".
//! 2. `bash_command_text` rendered assignment words from their
//!    cooked values instead of the raw token text, dropping the
//!    quoting: `x="$(...)"` exposed as `x=$(...)` and a multi-line
//!    quoted value (`e='line1\nline2'`, nvm_nvmrc_invalid_msg's
//!    shape) split into separate commands on re-parse. GNU
//!    print_cmd.c:578-586 (print_simple_command /
//!    command_print_word_list) prints the word's raw text, quotes
//!    intact; only `$()` spans are replaced by the canonical
//!    print_comsub form (parse.y:4451/4632).
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1704-{}-{}",
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

#[test]
fn issue1704_background_function_comsub_pipeline_survives_respawn() {
    let (stdout, stderr, code) =
        rubash_file("g() { x=\"$(echo abc | awk '{print $1}')\"; echo \"x=$x\"; }\ng &\nwait\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "x=abc\n");
}

#[test]
fn issue1704_background_function_nested_comsub_pipeline_survives_respawn() {
    // The nvm_get_colors shape: a pipeline inside a comsub inside a
    // double-quoted comsub argument.
    let (stdout, stderr, code) = rubash_file(
        "pcc() { echo \"P:$1\"; }\ng() { local C; C=bygre; echo \"$(pcc \"$(echo \"$C\" | awk '{ print substr($0, 1, 1); }')\")\"; }\ng &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "P:b\n");
}

#[test]
fn issue1704_background_function_multiline_quoted_value_survives_respawn() {
    // The nvm_nvmrc_invalid_msg shape: a multi-line single-quoted
    // assignment value must re-parse as ONE assignment in the child.
    let (stdout, stderr, code) =
        rubash_file("g() { local e; e='line1\nline2'; echo \"e=[$e]\"; }\ng &\nwait\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "e=[line1\nline2]\n");
}

#[test]
fn issue1704_background_calls_in_piped_subshell_list_alias_shape() {
    // The nvm_list_aliases skeleton: helper functions backgrounded
    // inside a subshell whose output is piped to sort.
    let (stdout, stderr, code) = rubash_file(
        "print_one() { local v; v=\"$(echo \"$1\" | awk '{print $1}')\"; echo \"one:$v\"; }\n( print_one alpha & print_one beta & wait ) | sort\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "one:alpha\none:beta\n");
}

#[test]
fn issue1704_bash_command_keeps_assignment_quoting_and_comsub_pipe() {
    let (stdout, _, code) = rubash_file(
        "trap 'echo \"BC:[$BASH_COMMAND]\"' DEBUG\nx=\"$(echo abc | awk '{print $1}')\"\ne='line1\nline2'\necho done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "BC:[x=\"$(echo abc | awk '{print $1}')\"]\nBC:[e='line1\nline2']\nBC:[echo done]\ndone\n"
    );
}

#[test]
fn issue1704_background_function_empty_string_case_pattern_survives_respawn() {
    // The nvm_download_artifact shape: a `''` case pattern rendered
    // from its cooked (empty) form produced an unparseable clause
    // head in the child's prelude — the whole respawn died with a
    // syntax error near `esac`.
    let (stdout, stderr, code) =
        rubash_file("g() { case \"$1\" in '') echo empty ;; *) echo other ;; esac; }\ng &\nwait\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "empty\n");
}

#[test]
fn issue1704_background_function_quoted_subject_and_alternation_survive_respawn() {
    let (stdout, stderr, code) = rubash_file(
        "g() { case \"${1-}\" in a|b) echo ab ;; '') echo empty ;; *) echo other ;; esac; }\ng b &\nwait\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "ab\n");
}

#[test]
fn issue1704_background_function_plain_pipeline_body_control() {
    // Control: a pipeline at body level (not inside a comsub) already
    // respawned correctly — pin it against regressions of the fix.
    let (stdout, stderr, code) =
        rubash_file("g() { echo abc | awk '{ print substr($0, 1, 1); }'; }\n( g & wait ) | sort\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a\n");
}

#[test]
fn issue1704_foreground_comsub_pipeline_control() {
    // Control: foreground execution never used the serialized text —
    // it must stay byte-identical.
    let (stdout, _, code) = rubash_file("x=\"$(echo abc | awk '{print $1}')\"\necho \"x=$x\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=abc\n");
}
