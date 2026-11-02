//! rubash#2605 — a standalone array-element assignment word whose
//! subscript contains a shell operator character was split at the
//! operator: `a[63|4]=1` executed as the two commands `a[63` and
//! `4]=1` ("command not found" x2), losing ble.sh's cancel-key
//! entries at load.
//!
//! GNU ground: read_token_word's `[` arm (parse.y:5636) fires when
//! the word so far is an identifier (token_is_ident, parse.y:5291)
//! and an assignment word is acceptable (assignment_acceptable,
//! parse.y:3163 = command_token_position && !PST_CASEPAT), and
//! parse_matched_pair (parse.y:3906-4210, P_ARRAYSUB) then consumes
//! the span to the balancing `]` — nested `[` counted, quotes,
//! backslash pairs and $-constructs recursed as units — so EVERY
//! shellbreak character inside the span (`| & ; < > ( )`, blanks,
//! newlines) is word data, never an operator. The arm knows nothing
//! of PST_ASSIGNOK: in a declare/typeset/local/export/readonly
//! operand the span still splits (the same boundary rubash#2463
//! established for spaced subscripts), and argument words, scalar
//! assignment values, redirect targets and case patterns split too
//! (in patterns `|` is the alternation separator). A bare command
//! word gets the span as well — `a[64|3]` is one failed command
//! name, not a pipeline — because the arm's condition is only
//! ident-prefix + position, with no `=` tail.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle
//! with script-file probes; the script path is normalized to
//! SCRIPT, mirroring tests/issue2463_*.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2605-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `script` as a script FILE; return (stdout, stderr) with the
/// script path normalized to SCRIPT.
fn run_script(dir: &Path, script: &str) -> (String, String) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let normalize =
        |text: &str| text.replace(&script_path.to_string_lossy().into_owned(), "SCRIPT");
    (
        normalize(&String::from_utf8_lossy(&output.stdout)),
        normalize(&String::from_utf8_lossy(&output.stderr)),
    )
}

#[test]
fn operator_class_inside_assignment_subscripts_stays_one_word() {
    let dir = scratch("ops");
    let (stdout, stderr) = run_script(
        &dir,
        "a[64|3]=v1; echo \"or=${a[67]}\"\n\
         b[1|2]=v2; echo \"or2=${b[3]}\"\n\
         k=1; v=2; h[k|v]=hv; echo \"vars=${h[3]}\"\n\
         c[3&5]=v3; echo \"and=${c[1]}\"\n\
         d[1<2]=v4; echo \"lt=${d[1]}\"\n\
         e[1>2]=v5; echo \"gt=${e[0]}\"\n\
         f[2>1]=v6; echo \"gt2=${f[1]}\"\n\
         g[(64|3)]=v7; echo \"paren=${g[67]}\"\n\
         i[1||2]=v8; echo \"oror=${i[1]}\"\n\
         j[3&&1]=v9; echo \"andand=${j[1]}\"\n\
         a[64|3]+=x; echo \"plus=${a[67]}\"\n\
         _ble_edit_async_read_is_cancel_key[63|_ble_decode_Ctrl]=1; echo \"ble1=${_ble_edit_async_read_is_cancel_key[63]}\"\n\
         _ble_edit_async_read_is_cancel_key[104|_ble_decode_Ctrl]=1; echo \"ble2=${_ble_edit_async_read_is_cancel_key[104]} count=${#_ble_edit_async_read_is_cancel_key[@]}\"\n",
    );
    assert_eq!(
        stdout,
        "or=v1\nor2=v2\nvars=hv\nand=v3\nlt=v4\ngt=v5\ngt2=v6\nparen=v7\n\
         oror=v8\nandand=v9\nplus=v1x\nble1=1\nble2=1 count=2\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn command_positions_keep_the_span_whole() {
    let dir = scratch("positions");
    let (stdout, stderr) = run_script(
        &dir,
        "echo start; a[64|3]=A; echo \"semi=${a[67]}\"; unset a\n\
         true && a[64|3]=B; echo \"and=${a[67]}\"; unset a\n\
         false || a[64|3]=C; echo \"or=${a[67]}\"; unset a\n\
         ( a[64|3]=E; echo \"sub=${a[67]}\" )\n\
         { a[64|3]=F; echo \"grp=${a[67]}\"; }; unset a\n\
         if a[64|3]=G; then echo \"if=${a[67]}\"; fi; unset a\n\
         ! a[64|3]=H; echo \"bang=${a[67]}\"; unset a\n\
         x=1 a[64|3]=J; echo \"chain=${a[67]}\"; unset a\n\
         a[64|3]=K | cat; echo \"pipe-done\"\n\
         while a[64|3]=W; do echo \"while=${a[67]}\"; unset a; break; done\n\
         until a[64|3]=U; do echo \"until-body\"; break; done; echo \"until=${a[67]}\"; unset a\n",
    );
    assert_eq!(
        stdout,
        "start\nsemi=A\nand=B\nor=C\nsub=E\ngrp=F\nif=G\nbang=H\nchain=J\n\
         pipe-done\nwhile=W\nuntil=U\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn nested_comsub_quoted_close_and_spaced_spans() {
    let dir = scratch("spans");
    let (stdout, stderr) = run_script(
        &dir,
        "b=(0 0 0 7); a[b[1|2]]=N; echo \"nested=${a[7]}\"\n\
         a[$(echo 64|cat)]=C; echo \"comsub=${a[64]}\"\n\
         declare -A m; m['x]y']=V; echo \"assoc=${m['x]y']}\"\n\
         a[ 64 | 3 ]=S; echo \"spaced=${a[67]}\"\n",
    );
    assert_eq!(stdout, "nested=N\ncomsub=C\nassoc=V\nspaced=S\n");
    assert_eq!(stderr, "");
}

#[test]
fn semicolon_subscript_is_an_arithmetic_error_not_a_split() {
    let dir = scratch("semicolon");
    let (stdout, stderr) = run_script(&dir, "a[1;2]=S\necho \"semi-survived\"\n");
    // The span is consumed whole, so the failure surfaces from the
    // arithmetic evaluator (GNU: `1;2: arithmetic syntax error:
    // invalid arithmetic operator`), the rest of the line unwinds,
    // and the script continues at the next line. A lexer split
    // would instead report `a[1` as a missing command.
    assert_eq!(stdout, "semi-survived\n");
    assert!(
        stderr.contains("arithmetic syntax error"),
        "expected an arithmetic diagnostic, got: {stderr}"
    );
    assert!(
        !stderr.contains("command not found"),
        "the word must not split into commands, got: {stderr}"
    );
}

#[test]
fn declaration_operands_and_other_positions_still_split() {
    let dir = scratch("controls");
    let (stdout, stderr) = run_script(
        &dir,
        "declare a[64|3]=A; echo \"declare-idx=${!a[@]}\"\n\
         echo a[64|3]; echo \"arg-rc=$?\"\n\
         x=a[64|3]; echo \"scalar=[$x]\"\n\
         f() { local c[64|3]=C; echo \"local-idx=${!c[@]}\"; }\n\
         f\n\
         export d[64|3]=D; echo \"export-idx=${!d[@]}\"\n\
         readonly e[64|3]=E; echo \"readonly-idx=${!e[@]}\"\n",
    );
    assert_eq!(
        stdout,
        "declare-idx=\narg-rc=127\nscalar=[]\nlocal-idx=\nexport-idx=\nreadonly-idx=\n"
    );
    assert_eq!(
        stderr,
        "SCRIPT: line 1: declare: `a[64': not a valid identifier\n\
         SCRIPT: line 1: 3]=A: command not found\n\
         SCRIPT: line 2: 3]: command not found\n\
         SCRIPT: line 3: 3]: command not found\n\
         SCRIPT: line 4: local: `c[64': not a valid identifier\n\
         SCRIPT: line 4: 3]=C: command not found\n\
         SCRIPT: line 6: export: `d[64': not a valid identifier\n\
         SCRIPT: line 6: 3]=D: command not found\n\
         SCRIPT: line 7: readonly: `e[64': not a valid identifier\n\
         SCRIPT: line 7: 3]=E: command not found\n"
    );
}

#[test]
fn bare_command_word_with_subscript_is_one_word() {
    let dir = scratch("bare");
    let (stdout, stderr) = run_script(
        &dir,
        "a[64|3]; echo \"bare-rc=$?\"\n\
         a[1 2]; echo \"spaced-bare-rc=$?\"\n",
    );
    assert_eq!(stdout, "bare-rc=127\nspaced-bare-rc=127\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: a[64|3]: command not found\n\
         SCRIPT: line 2: a[1 2]: command not found\n"
    );
}

#[test]
fn redirect_target_and_case_pattern_still_split() {
    let dir = scratch("targets");
    let (stdout, stderr) = run_script(
        &dir,
        "d=\"rd$$\"; mkdir \"$d\"; cd \"$d\" || echo CDFAIL\n\
         echo hi > a[64|3]\n\
         echo \"rc=$?\"\n\
         ls\n\
         cat 'a[64'\n\
         cd ..; rm -rf \"$d\"\n\
         case '4]' in a[64|4]) echo case-first;; 4]) echo case-second;; esac\n",
    );
    // The target word splits at the pipe: the file is `a[64`, the
    // tail `3]` is a separate (missing) command, and the pipeline
    // status is 127. In the case pattern the `|` is alternation, so
    // the subject `4]` matches the FIRST clause's second pattern.
    assert_eq!(stdout, "rc=127\na[64\nhi\ncase-first\n");
    assert_eq!(stderr, "SCRIPT: line 2: 3]: command not found\n");
}

#[test]
fn assignment_word_after_completed_redirect_stays_whole() {
    let dir = scratch("redirlist");
    let (stdout, stderr) = run_script(
        &dir,
        "> /dev/null a[64|3]=A\n\
         echo \"whole=${a[64+3]}\"\n\
         > /dev/null b[64|3]\n\
         echo \"rc=$?\"\n\
         > /dev/null a[64|3]=A b[64|3]=B\n\
         echo \"chain=${a[67]}${b[67]}\"\n",
    );
    // PST_REDIRLIST (make_cmd.c:483 set at simple-command creation,
    // parse.y:3157-3161 in command_token_position): a redirection
    // list opened AHEAD of the command word leaves the following word
    // in command position, so its subscript span is whole — with an
    // `=` tail, as a bare command name, and across an assignment
    // chain that follows the completed redirection.
    assert_eq!(stdout, "whole=A\nrc=127\nchain=AB\n");
    assert_eq!(stderr, "SCRIPT: line 3: b[64|3]: command not found\n");
}

#[test]
fn redirect_after_first_word_element_does_not_arm() {
    let dir = scratch("redir-clear");
    let (stdout, stderr) = run_script(
        &dir,
        "a=1 > /dev/null b[64|3]=B\n\
         echo \"rc=$?\"\n\
         > /dev/null a=1 > /dev/null b[64|3]=B\n\
         echo \"rc2=$?\"\n",
    );
    // The first word element joining the simple command clears
    // PST_REDIRLIST (make_cmd.c:489), so a redirection list opened
    // AFTER it never arms a later word: `b[64|3]=B` splits at the
    // pipe into the two missing commands `b[64` and `3]=B`, both
    // when the element precedes the list and when an assignment
    // element intervenes between two lists.
    assert_eq!(stdout, "rc=127\nrc2=127\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: b[64: command not found\n\
         SCRIPT: line 1: 3]=B: command not found\n\
         SCRIPT: line 3: b[64: command not found\n\
         SCRIPT: line 3: 3]=B: command not found\n"
    );
}

#[test]
fn redirect_target_variants_split() {
    let dir = scratch("target-variants");
    let (stdout, stderr) = run_script(
        &dir,
        "d=\"tv$$\"; mkdir \"$d\"; cd \"$d\" || echo CDFAIL\n\
         echo hi >> a[64|3]\n\
         echo \"app-rc=$?\"\n\
         cat <<< a[64|3]\n\
         echo \"hs-rc=$?\"\n\
         > a[1 2]\n\
         echo \"ws-rc=$?\"\n\
         printf x > 'a[64'\n\
         echo hi < a[64|3]\n\
         echo \"in-rc=$?\"\n\
         test -f 'a[64' && echo \"f64=yes\"\n\
         test -f 'a[64|3]' || echo \"fwhole=no\"\n\
         test -f 'a[1' && echo \"f1=yes\"\n\
         cd ..; rm -rf \"$d\"\n",
    );
    // parsing_redirection (parse.y:3147) is uniform across the
    // redirection operator class: the word after `>>`, `<<<`, `>`
    // or `<` is a target and splits at the operator inside the
    // brackets (and at the blank, for the whitespace shape — the
    // whitespace exemption shares the same position gate). The
    // `<` line's `hi` is piped into the missing `3]` command, so
    // it never reaches stdout.
    assert_eq!(
        stdout,
        "app-rc=127\nhs-rc=127\nws-rc=127\nin-rc=127\n\
         f64=yes\nfwhole=no\nf1=yes\n"
    );
    assert_eq!(
        stderr,
        "SCRIPT: line 2: 3]: command not found\n\
         SCRIPT: line 4: 3]: command not found\n\
         SCRIPT: line 6: 2]: command not found\n\
         SCRIPT: line 9: 3]: command not found\n"
    );
}

#[test]
fn quoted_close_inside_expansion_subscripts() {
    let dir = scratch("quoted-close");
    let (stdout, stderr) = run_script(
        &dir,
        "declare -A m\n\
         m['x]y']=Value\n\
         echo \"dq=${m[\"x]y\"]}\"\n\
         echo \"cs=${m[$(echo 'x]y')]}\"\n\
         echo \"sub=${m['x]y']:1:3}\"\n",
    );
    // The expansion-side subscript reader is skipsubscript
    // (subst.c:2189) = skip_matched_pair with quote/expansion unit
    // skipping on (subst.c:2083-2186): a `]` inside single or
    // double quotes, or inside a nested `$(...)`, is data, so all
    // three reads find the element stored under the key `x]y`.
    assert_eq!(stdout, "dq=Value\ncs=Value\nsub=alu\n");
    assert_eq!(stderr, "");
}

#[test]
fn quoted_close_expansion_reads_all_forms() {
    let dir = scratch("quoted-close-all");
    let (stdout, stderr) = run_script(
        &dir,
        "declare -A m\n\
         m['x]y']=Value\n\
         echo \"dq=${m[\"x]y\"]}\"\n\
         echo \"cs=${m[$(echo 'x]y')]}\"\n\
         echo \"sub=${m['x]y']:1:3}\"\n\
         echo \"plain=${m['x]y']}\"\n\
         echo unq=${m['x]y']}\n\
         s=abcdef\n\
         echo \"off=${s:${m['x]y']}}\"\n\
         echo \"def=${m['x]y']:-fallback}\"\n",
    );
    // Round 3 (rubash#2605): EVERY subscript reader in the expansion
    // path is GNU's one skipsubscript (subst.c:2186, flags 0) — the
    // brace-name gate (subst.c:816 string_extract, subst.c:7595
    // valid_array_reference), the leading-ref split, the reference
    // length scan and the substring offset scan
    // (subst.c:11129 expand_array_subscript) — so the quoted `]`
    // closes nothing anywhere: the plain and unquoted reads, the
    // operator form, and a read embedded in a substring OFFSET
    // expression all resolve the element stored under `x]y`. The
    // offset read yields `Value`, an unset variable in arithmetic,
    // i.e. offset 0 — the whole string.
    assert_eq!(
        stdout,
        "dq=Value\ncs=Value\nsub=alu\nplain=Value\nunq=Value\n\
         off=abcdef\ndef=Value\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn assignment_prefix_stays_armed_across_element_words() {
    let dir = scratch("prefix-chain");
    let (stdout, stderr) = run_script(
        &dir,
        "a[64|3]=A b[64|3]=B\n\
         echo \"plain=${a[67]}${b[67]}\"\n\
         a[1]=C b[64|3]=D\n\
         echo \"mixed=${a[1]}${b[67]}\"\n\
         x=1 a[64|3]=E b[64|3]=F\n\
         echo \"scalarfirst=${a[67]}${b[67]}\"\n\
         a[64|3]=G b[64|3]=H | cat\n\
         echo \"stage-done\"\n",
    );
    // Round 3 (rubash#2605): GNU's reader returns ASSIGNMENT_WORD
    // for every assignment word read in command position — the
    // element form included (general.c:480 assignment(), bracket
    // arm; parse.y:5846 with parse.y:5789-5796) — so
    // command_token_position (parse.y:3157-3161) stays armed across
    // the whole prefix and every following element word keeps its
    // span: no split, no pipeline, and no `not a valid identifier`
    // (that diagnostic belongs only to the tempenv path,
    // variables.c:3536 assign_in_env — see the boundary pin below).
    // The pipeline-stage form binds both words in the stage
    // subshell, silently.
    assert_eq!(stdout, "plain=AB\nmixed=CD\nscalarfirst=EF\nstage-done\n");
    assert_eq!(stderr, "");
}

#[test]
fn env_prefix_element_word_keeps_gnu_invalidid_boundary() {
    let dir = scratch("env-prefix-boundary");
    let (stdout, stderr) = run_script(
        &dir,
        "a[64|3]=K sleep 0\n\
         echo \"rc=$?\"\n",
    );
    // The hard boundary (NOT a defect, oracle-verified): an element
    // assignment word in front of a real command name is a tempenv
    // word, and GNU's assign_in_env (variables.c:3536-3566) takes
    // the name before `=` — `a[64|3]`, brackets included, via
    // assignment()'s offset (general.c:480) — and valid_identifier
    // rejects it: sh_invalidid, the word is skipped, the command
    // still runs and the status is the command's. Round 3 must not
    // "fix" this face while repairing the assignment-statement
    // faces above.
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: `a[64|3]': not a valid identifier\n"
    );
}

#[test]
fn redirect_first_commands_do_not_degrade_across_lines() {
    let dir = scratch("redirlist-cross-line");
    let (stdout, stderr) = run_script(
        &dir,
        "> /dev/null b[64|3]\n\
         echo \"rc=$?\"\n\
         > /dev/null a[64|3]=A b[64|3]=B\n\
         echo \"chain=${a[67]}${b[67]}\"\n",
    );
    // Round 4: the redirlist pair (PST_REDIRLIST + the word-element
    // latch) belongs to ONE simple command — GNU drops the bit at
    // command end (make_cmd.c:816) and the next command's
    // make_simple_command starts it fresh (make_cmd.c:483). The
    // state the lexer carries across logical lines must fold the
    // terminating '\n' the same way: the bare word on line 1 behaves
    // in both shells, and the chain on line 3 must still find the
    // list open even though two complete commands (each with word
    // elements of its own) intervened. Before the fold, the latch
    // outlived its command and EVERY word on line 3 split.
    assert_eq!(stdout, "rc=127\nchain=AB\n");
    assert_eq!(stderr, "SCRIPT: line 1: b[64|3]: command not found\n");
}

#[test]
fn redirect_first_commands_in_a_row_stay_armed() {
    let dir = scratch("redirlist-row");
    let (stdout, stderr) = run_script(
        &dir,
        "> /dev/null a[64|3]=A\n\
         echo \"one=${a[67]}\"\n\
         > /dev/null b[64|3]\n\
         echo \"rc=$?\"\n\
         > /dev/null c[64|3]=C d[64|3]=D\n\
         echo \"two=${c[67]}${d[67]}\"\n\
         > /dev/null e[64|3]=E\n\
         echo \"three=${e[67]}\"\n",
    );
    // Four redirect-first commands in a row — single assignment,
    // bare command word, assignment chain, single assignment again —
    // each separated by an ordinary command: arming is a per-command
    // property (make_cmd.c:483/489/816), never a property of how
    // many commands ran before, so every line's span stays whole.
    assert_eq!(stdout, "one=A\nrc=127\ntwo=CD\nthree=E\n");
    assert_eq!(stderr, "SCRIPT: line 3: b[64|3]: command not found\n");
}
