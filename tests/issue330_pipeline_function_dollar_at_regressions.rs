//! Issue rubash#330: a function called in a pipeline received the caller's
//! `"$@"` as ONE space-joined argument (N=1 vs GNU N=3), silently corrupting
//! FFmpeg configure (config.h lost 783 defines while configure still exited
//! 0 via `map 'eval echo "$v \${$v:-no}"' "$@" | awk` in print_config).
//!
//! GNU semantics: a pipeline element runs the SAME word expansion as any
//! simple command — execute_cmd.c:624 execute_command_internal ->
//! execute_simple_command -> subst.c:13219 expand_word_list_internal, where
//! `"$@"` becomes a LIST of individually quoted words (subst.c:10691
//! `case '@'`: "we have to turn quoting off after we split into the
//! individually quoted arguments"), unquoted `$@`/`$*` field-split, and
//! unquoted patterns pathname-expand. None of that depends on pipeline
//! membership.
//!
//! Root cause: `execute_function_pipeline_stage` expanded each argument
//! word with the scalar `expand_word` (space-join, no field splitting, no
//! glob), unlike the builtin/external stages which use the field-splitting
//! `expand_stage_tail_words` argv builder.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

/// The reporter's exact reproducer: `m "$@" | cat` must see one argument per
/// positional parameter, exactly like the non-piped control call.
#[test]
fn quoted_dollar_at_keeps_words_in_pipeline_function_call() {
    let (stdout, stderr, _) = rubash(
        "m(){ echo \"N=$#\"; for v; do echo \"[$v]\"; done; }\n\
         set -- a b c\n\
         m \"$@\" | cat\n\
         m \"$@\"",
    );
    assert_clean(&stderr);
    let expected = "N=3\n[a]\n[b]\n[c]\nN=3\n[a]\n[b]\n[c]\n";
    assert_eq!(stdout, expected);
}

/// Unquoted `$*` field-splits into one word per parameter in a pipeline.
#[test]
fn unquoted_dollar_star_field_splits_in_pipeline_function_call() {
    let (stdout, stderr, _) = rubash(
        "m(){ echo \"N=$#\"; for v; do echo \"[$v]\"; done; }\n\
         set -- a b c\n\
         m $* | cat",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "N=3\n[a]\n[b]\n[c]\n");
}

/// `"$*"` stays ONE IFS-joined word (both shells agree).
#[test]
fn quoted_dollar_star_stays_one_word_in_pipeline_function_call() {
    let (stdout, stderr, _) = rubash(
        "m(){ echo \"N=$#\"; for v; do echo \"[$v]\"; done; }\n\
         set -- a b c\n\
         m \"$*\" | cat",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "N=1\n[a b c]\n");
}

/// `"$@"` mixed with surrounding literal words keeps its position in the
/// argument list (FFmpeg's `map 'eval ...' "$@"` shape).
#[test]
fn mixed_literal_and_dollar_at_in_pipeline_function_call() {
    let (stdout, stderr, _) = rubash(
        "m(){ echo \"N=$#\"; for v; do echo \"[$v]\"; done; }\n\
         set -- 'x y' z\n\
         m pre \"$@\" post | cat",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "N=4\n[pre]\n[x y]\n[z]\n[post]\n");
}

/// Quoted `"${arr[@]}"` expands to one word per array element.
#[test]
fn quoted_array_at_keeps_elements_in_pipeline_function_call() {
    let (stdout, stderr, _) = rubash(
        "m(){ echo \"N=$#\"; for v; do echo \"[$v]\"; done; }\n\
         arr=(p \"q r\" s)\n\
         m \"${arr[@]}\" | cat",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "N=3\n[p]\n[q r]\n[s]\n");
}

/// An empty `"$@"` produces ZERO arguments (not one empty argument).
#[test]
fn empty_dollar_at_is_zero_words_in_pipeline_function_call() {
    let (stdout, stderr, _) = rubash(
        "m(){ echo \"N=$#\"; for v; do echo \"[$v]\"; done; }\n\
         set --\n\
         m \"$@\" u v | cat",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "N=2\n[u]\n[v]\n");
}

/// Unquoted glob argument words pathname-expand for a pipeline function
/// call like they do for external commands (execute_cmd.c expand_words).
#[test]
fn glob_args_expand_in_pipeline_function_call() {
    let (stdout, stderr, code) = rubash(
        "m(){ echo \"N=$#\"; for v; do echo \"[$v]\"; done; }\n\
         d=$(mktemp -d)\n\
         : > \"$d/zz.330t\"; : > \"$d/aa.330t\"\n\
         cd \"$d\"\n\
         m *.330t | cat\n\
         rm -rf \"$d\"",
    );
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    // Pathname expansion sorts, so the glob yields aa then zz.
    assert_eq!(stdout, "N=2\n[aa.330t]\n[zz.330t]\n");
}

/// A quoted glob pattern stays literal for a pipeline function call.
#[test]
fn quoted_glob_arg_stays_literal_in_pipeline_function_call() {
    let (stdout, stderr, _) = rubash(
        "m(){ echo \"N=$#\"; for v; do echo \"[$v]\"; done; }\n\
         m \"*.g\" | cat",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "N=1\n[*.g]\n");
}
