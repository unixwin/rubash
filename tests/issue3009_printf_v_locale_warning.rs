//! Regression pins for rubash#3009 — `printf -v` wrote locale-family
//! variables through its own bind path and skipped GNU's sv_locale
//! consequence, so a bad locale name bound via `printf -v LC_ALL ...`
//! stayed silent where every other channel warns.
//!
//! GNU ownership: printf.def's PRETURN runs
//! `builtin_bind_variable (vname, ...)` and then
//! `stupidly_hack_special_variables (vname)` — keyed on the literal -v
//! operand name, NOT the nameref-resolved target. sv_locale
//! (variables.c:6272) -> locale.c set_locale_var performs the real
//! setlocale attempt and warns `setlocale: <VAR>: cannot change locale
//! (<value>)` (locale.c:225 / locale.c:302) on failure, with the
//! `: <strerror(errno)>` suffix only while errno survives — glibc sets
//! it only on the FIRST failed lookup of a name in the process, so a
//! repeat bind of the same bad name warns without the suffix (the
//! rubash#1692 shape). LANG never warns (set_lang is silent), but a
//! later category bind reads LANG through get_locale_var precedence
//! and warns about the category with LANG's value. Because the hack is
//! keyed on the operand name, a bind THROUGH a nameref
//! (`declare -n r=LC_ALL; printf -v r <bad>`) stores into LC_ALL
//! silently, an array-element bind (`printf -v 'LC_ALL[0]' <bad>`) is
//! silent, and the %n count bind is silent too.
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for the
//! same script files on a system where the bogus names do not resolve.
//! The bogus names are chosen so no real locale can ever match them.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3009-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Base command with the whole locale family removed from the
/// inherited environment so each case starts from a known state.
fn base() -> Command {
    let mut cmd = Command::new(BIN);
    for var in [
        "LANG",
        "LC_ALL",
        "LC_COLLATE",
        "LC_CTYPE",
        "LC_MESSAGES",
        "LC_NUMERIC",
        "LC_TIME",
    ] {
        cmd.env_remove(var);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd
}

fn run_script(tag: &str, body: &str) -> (String, String, String, Option<i32>) {
    let dir = scratch(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = base()
        .arg(&script)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let result = (
        script.to_string_lossy().into_owned(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn issue3009_printf_v_lc_all_warns_like_gnu() {
    // The bug face: the value lands, stdout and status are unaffected,
    // and the warning fires with the first-failure strerror suffix.
    let (path, stdout, stderr, code) = run_script(
        "printf-v-lc-all",
        "printf -v LC_ALL bogus_LOCALE\necho \"after:[$LC_ALL]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after:[bogus_LOCALE]\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (bogus_LOCALE): No such file or directory\n"
        )
    );
}

#[test]
fn issue3009_printf_v_category_warns_with_its_own_name() {
    let (path, _stdout, stderr, code) = run_script(
        "printf-v-category",
        "printf -v LC_CTYPE bogus_CTYPE_LOCALE\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_CTYPE: cannot change locale (bogus_CTYPE_LOCALE): No such file or directory\n"
        )
    );
}

#[test]
fn issue3009_printf_v_repeat_bind_warns_without_suffix_second_time() {
    // Same errno-consumption shape as rubash#1692: the second failing
    // attempt of the same name in one process warns without the suffix.
    let (path, _stdout, stderr, code) = run_script(
        "printf-v-twice",
        "printf -v LC_ALL bogus_LOCALE\nprintf -v LC_ALL bogus_LOCALE\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (bogus_LOCALE): No such file or directory\n\
             {path}: line 2: warning: setlocale: LC_ALL: cannot change locale (bogus_LOCALE)\n"
        )
    );
}

#[test]
fn issue3009_printf_v_lang_is_silent_but_feeds_category_precedence() {
    // LANG itself never warns (set_lang is silent). Binding LC_CTYPE to
    // the empty string afterwards runs the precedence read: LC_CTYPE
    // falls back to LANG's bogus value and the warning names LC_CTYPE
    // with that value (locale.c:301 via get_locale_var).
    let (path, stdout, stderr, code) = run_script(
        "printf-v-lang",
        "printf -v LANG bogus_LANG_LOCALE\nprintf -v LC_CTYPE ''\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: warning: setlocale: LC_CTYPE: cannot change locale (bogus_LANG_LOCALE): No such file or directory\n"
        )
    );
}

#[test]
fn issue3009_printf_v_silent_faces_stay_silent() {
    // A resolvable name, a nameref bind INTO LC_ALL (the hack is keyed
    // on the operand name `r`, not the resolved target), and the %n
    // count bind all stay silent in GNU.
    let (_path, stdout, stderr, code) = run_script(
        "printf-v-silent",
        "printf -v LC_ALL C\n\
         declare -n r=LC_ALL\n\
         printf -v r bogus_LOCALE\n\
         printf 'abc%n' LC_TIME >/dev/null\n\
         echo \"after:[$LC_ALL]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after:[bogus_LOCALE]\n");
    assert_eq!(stderr, "", "no setlocale warning in the silent faces");
}

#[test]
fn issue3009_printf_v_array_element_bind_is_silent() {
    // An array-element operand is not a locale-family name, so GNU's
    // operand-keyed hack never fires for it. (Only the warning surface
    // is asserted here; the stored value's readback shape is a
    // separate storage-conversion matter, see the lane REPORT.)
    let (_path, stdout, stderr, code) = run_script(
        "printf-v-element",
        "printf -v 'LC_ALL[0]' bogus_LOCALE\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, "", "array-element bind must not warn");
}

#[test]
fn issue3009_printf_v_after_plain_bind_still_warns() {
    // The plain channel warns first (with suffix); the printf -v bind
    // of the same bad name warns again, without the suffix — the two
    // channels share the one funnel and its errno state.
    let (path, _stdout, stderr, code) = run_script(
        "mixed-channels",
        "LC_ALL=bogus_LOCALE\nprintf -v LC_ALL bogus_LOCALE\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (bogus_LOCALE): No such file or directory\n\
             {path}: line 2: warning: setlocale: LC_ALL: cannot change locale (bogus_LOCALE)\n"
        )
    );
}

#[test]
fn issue3009_control_channels_warn_in_the_same_shape() {
    // Control group (already correct before this fix, pinned so the
    // funnel convergence cannot drift them): plain assignment and a
    // declare bind of a bad category name.
    let (path, _stdout, stderr, code) = run_script(
        "controls",
        "LC_ALL=bogus_LOCALE\ndeclare LC_NUMERIC=bogus_NUMERIC_LOCALE\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (bogus_LOCALE): No such file or directory\n"
        ),
        "declare LC_NUMERIC under a non-empty LC_ALL is not attempted in GNU either"
    );
}
