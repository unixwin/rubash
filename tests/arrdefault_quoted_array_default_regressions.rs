//! Quoted array-default expansion regressions (wt66/arrdefault lane).
//!
//! The golden-journey blocker (wt61, bash-it unique themes): bash_it.sh's
//! finalize loop `for f in "${BASH_IT_LIB[@]:-}"` ran ONE iteration with a
//! single joined word under rubash while GNU bash runs one iteration per
//! element.
//!
//! Root cause: the for-list expander
//! (`expand_for_word_values_result`, executor/command_words.rs) lacked the
//! fully-double-quoted (`STORAGE_WORD_PREFIX`) braced-alternate fan-outs the
//! command-argv path already runs (executor/command_prepare.rs), so a quoted
//! `"${arr[@]:-}"` fell to the scalar operator expander, which joins `[@]`
//! with spaces (`parameter_operator_value`). GNU expands a for-list word
//! exactly like a command word (`subst.c:13219 expand_word_list_internal`),
//! and `parameter_brace_expand` calls `chk_atstar` on every valid array
//! reference BEFORE the operator switch (`subst.c:10119-10128`), so the
//! double-quoted `[@]` operand sets `quoted_dollar_at` (`subst.c:7635-7639`)
//! even under `:-`; with the array set and non-null the `-`/`:-` arm just
//! uses TEMP (`subst.c:10350-10390`), the `string_list_dollar_at` value
//! (`arrayfunc.c:1552`) whose element separators survive double-quote
//! joining — one word per element.
//!
//! Second cell of the same class: `:+`/`+` with an unset or EMPTY array
//! yields ZERO words (like `"$@"` with no positional parameters), not one
//! empty word — `array_value_internal` returns NULL for an unset variable
//! and for an empty list (`arrayfunc.c:1517-1533`), the `+` not-used branch
//! leaves the word NULL (`subst.c:10386-10420`), and an empty expansion in
//! a `quoted_dollar_at` word is discarded whole (`subst.c:12036-12048`).
//! A scalar cell keeps the alternate (`arrayfunc.c:1535-1539` reads
//! `value_cell` as a one-element list), and all-empty elements
//! (`a=("" "")`) keep the alternate too (array22.sub).
//!
//! Every expectation below is byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file matrix 2026-10-02; artifacts under
//! target/issue-suites/results/arrdefault/, 114 cases x script + -c modes,
//! 0 diff lines).

use std::process::Command;

/// Run a script FILE in its own scratch directory and return
/// (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-arrdef-{}-{}",
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

fn assert_script(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, expected_stdout, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}

/// The golden-journey blocker itself: bash_it.sh's library finalize loop.
#[test]
fn bash_it_finalize_loop_iterates_per_element() {
    assert_script(
        concat!(
            "BASH_IT_LIB=(lib/utilities.bash lib/log.bash)\n",
            "count=0\n",
            "for f in \"${BASH_IT_LIB[@]:-}\"; do\n",
            "  echo \"source:$f\"\n",
            "  count=$((count+1))\n",
            "done\n",
            "echo \"iterations=$count\"\n",
        ),
        "source:lib/utilities.bash\nsource:lib/log.bash\niterations=2\n",
    );
}

/// bash-it powerline's `__powerline_prompt_command` segment loop
/// (`${POWERLINE_PROMPT[@]-"user_info" "scm" ...}`): the quoted multi-word
/// default yields one word per quoted element; a live array overrides it.
#[test]
fn powerline_segment_loop_default_and_live() {
    assert_script(
        concat!(
            "unset POWERLINE_PROMPT\n",
            "for segment in ${POWERLINE_PROMPT[@]-\"user_info\" \"scm\" \"python_venv\"}; do\n",
            "  echo \"segment:$segment\"\n",
            "done\n",
            "POWERLINE_PROMPT=(\"cwd\" \"scm\")\n",
            "for segment in ${POWERLINE_PROMPT[@]-\"user_info\" \"scm\"}; do\n",
            "  echo \"live:$segment\"\n",
            "done\n",
        ),
        "segment:user_info\nsegment:scm\nsegment:python_venv\nlive:cwd\nlive:scm\n",
    );
}

/// `"${a[@]}"` with an empty declared array keeps the quoted-null default
/// word per GNU (the `:-` alternate is used and quoted-empty is retained).
#[test]
fn powerline_segment_loop_empty_declared_array() {
    assert_script(
        concat!(
            "POWERLINE_PROMPT=()\n",
            "for segment in ${POWERLINE_PROMPT[@]-\"user_info\" \"scm\"}; do\n",
            "  echo \"empty:$segment\"\n",
            "done\n",
        ),
        "empty:user_info\nempty:scm\n",
    );
}

/// The core class: `"${arr[@]:-}"` per state, for-in word yield + argv +
/// assignment RHS (assignment joins — PF_ASSIGNRHS semantics).
#[test]
fn quoted_at_default_class_set_array() {
    assert_script(
        concat!(
            "arr=(a b)\n",
            "for f in \"${arr[@]:-}\"; do echo \"[$f]\"; done\n",
            "printf '<%s>' \"${arr[@]:-}\"; echo\n",
            "v=\"${arr[@]:-}\"; echo \"[$v]\"\n",
            "for f in \"${arr[@]-\"${arr[@]}\"}\"; do echo \"[$f]\"; done\n",
        ),
        "[a]\n[b]\n<a><b>\n[a b]\n[a]\n[b]\n",
    );
}

#[test]
fn quoted_at_default_class_empty_and_unset_arrays() {
    // Empty declared and unset arrays take the (empty) alternate: one
    // retained empty word for `:-`/`-`, and `[*]` joins to one empty word.
    assert_script(
        concat!(
            "arr=()\n",
            "for f in \"${arr[@]:-}\"; do echo \"[$f]\"; done\n",
            "for f in \"${arr[@]-}\"; do echo \"[$f]\"; done\n",
            "for f in \"${arr[*]:-}\"; do echo \"[$f]\"; done\n",
            "for f in \"${arr[@]:-x}\"; do echo \"[$f]\"; done\n",
            "unset arr\n",
            "for f in \"${arr[@]:-}\"; do echo \"[$f]\"; done\n",
            "for f in \"${arr[@]-D}\"; do echo \"[$f]\"; done\n",
            "for f in \"${arr[*]:-}\"; do echo \"[$f]\"; done\n",
            "for f in \"${arr[@]:-D}\"; do echo \"[$f]\"; done\n",
        ),
        "[]\n[]\n[]\n[x]\n[]\n[D]\n[]\n[D]\n",
    );
}

/// `:+`/`+` on an unset or empty array yields ZERO words ("$@"-like
/// empty-list discard), while a scalar cell keeps the alternate and
/// all-empty elements keep it too.
#[test]
fn quoted_at_plus_class_zero_words() {
    assert_script(
        concat!(
            "unset arr\n",
            "arr2=()\n",
            "s=abc\n",
            "ee=(\"\" \"\")\n",
            "echo \"unset-plus:$(for f in \"${arr[@]+Z}\"; do printf '<%s>' \"$f\"; done)\"\n",
            "echo \"unset-colonplus:$(for f in \"${arr[@]:+Z}\"; do printf '<%s>' \"$f\"; done)\"\n",
            "echo \"empty-plus:$(for f in \"${arr2[@]+Z}\"; do printf '<%s>' \"$f\"; done)\"\n",
            "echo \"empty-colonplus:$(for f in \"${arr2[@]:+Z}\"; do printf '<%s>' \"$f\"; done)\"\n",
            "echo \"scalar-set:$(for f in \"${s[@]:+y}\"; do printf '<%s>' \"$f\"; done)\"\n",
            "echo \"all-empty-colonplus:$(for f in \"${ee[@]:+y}\"; do printf '<%s>' \"$f\"; done)\"\n",
            "set --\n",
            "echo \"at-empty:$(for f in \"${@:+y}\"; do printf '<%s>' \"$f\"; done)\"\n",
        ),
        concat!(
            "unset-plus:\n",
            "unset-colonplus:\n",
            "empty-plus:\n",
            "empty-colonplus:\n",
            "scalar-set:<y>\n",
            "all-empty-colonplus:<y>\n",
            "at-empty:\n",
        ),
    );
}

/// All-empty elements expand to one (empty) word per element; `[*]` joins
/// with IFS[0] into a single word whose emptiness `:-` then tests.
#[test]
fn quoted_at_default_class_empty_elements() {
    assert_script(
        concat!(
            "arr=(\"\" \"\")\n",
            "for f in \"${arr[@]:-}\"; do echo \"[$f]\"; done\n",
            "printf '<%s>' \"${arr[@]:-}\"; echo\n",
            "v=\"${arr[@]:-}\"; echo \"[$v]\"\n",
            "for f in \"${arr[@]:+y}\"; do echo \"[$f]\"; done\n",
        ),
        "[]\n[]\n<><>\n[ ]\n[y]\n",
    );
}

/// Nested guard `"${a[@]+\"${a[@]}\"}"` (the #147 family): per-element words
/// when set; zero words when unset or empty.
#[test]
fn nested_quoted_guard_class() {
    assert_script(
        concat!(
            "a=(p q)\n",
            "for f in \"${a[@]+\\\"${a[@]}\\\"}\"; do echo \"[$f]\"; done\n",
            "printf '<%s>' \"${a[@]+\\\"${a[@]}\\\"}\"; echo\n",
            "v=\"${a[@]+\\\"${a[@]}\\\"}\"; echo \"[$v]\"\n",
            "a=()\n",
            "echo \"empty:$(for f in \"${a[@]+\\\"${a[@]}\\\"}\"; do printf '<%s>' \"$f\"; done)\"\n",
            "unset a\n",
            "echo \"unset:$(for f in \"${a[@]+\\\"${a[@]}\\\"}\"; do printf '<%s>' \"$f\"; done)\"\n",
        ),
        "[\"p]\n[q\"]\n<\"p><q\">\n[\"p q\"]\nempty:\nunset:\n",
    );
}

/// #147 regression guards: the `${arr[@]+"${arr[@]}"}` self-referential
/// idiom family must stay green.
#[test]
fn guard_147_idioms_stay_green() {
    assert_script(
        concat!(
            "arr=(x y)\n",
            "for f in ${arr[@]+\"${arr[@]}\"}; do echo \"[$f]\"; done\n",
            "printf '<%s>' ${arr[@]+\"${arr[@]}\"}; echo\n",
            "for f in ${arr+\"${arr[@]}\"}; do echo \"[$f]\"; done\n",
            "arr=()\n",
            "echo \"count=$(for f in ${arr[@]+\"${arr[@]}\"}; do echo x; done | wc -l | tr -d ' ')\"\n",
            "b=(\"\" x)\n",
            "for f in ${b[@]+\"${b[@]:0:2}\"}; do echo \"[$f]\"; done\n",
        ),
        "[x]\n[y]\n<x><y>\n[x]\n[y]\ncount=0\n[]\n[x]\n",
    );
}

/// The plain quoted expansion siblings (no operator) must stay green.
#[test]
fn plain_quoted_at_siblings_stay_green() {
    assert_script(
        concat!(
            "arr=(a b)\n",
            "for f in \"${arr[@]}\"; do echo \"[$f]\"; done\n",
            "echo \"${arr[@]}\"\n",
            "printf '<%s>' \"${arr[@]}\"; echo\n",
            "v=\"${arr[@]}\"; echo \"[$v]\"\n",
            "empty=()\n",
            "printf '<%s>' \"${empty[@]}\"; echo\n",
            "v=\"${empty[@]}\"; echo \"[$v]\"\n",
        ),
        "[a]\n[b]\na b\n<a><b>\n[a b]\n<>\n[]\n",
    );
}
