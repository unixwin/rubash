//! rubash#3581 — a QUOTED list subject (`@`, `*`, `name[@]`, `name[*]`)
//! must read the `#`/`%` removal gate's non-empty branch even when its
//! joined word is `""`: GNU marks a quoted-null list element QUOTED_NULL
//! (subst.c:10133-10140), so the `*value == '\0'` test at subst.c:10320-
//! 10327 sees a non-empty word, expands the pattern, and diagnoses a bad
//! substitution inside it — where the same joined `""` UNQUOTED is silent.
//!
//! Scalar, element and bare-array-name subjects carry no QUOTED_NULL, so
//! `""` stays silent quoted or not; an absent list stays silent either way.
//! Heredoc bodies count as a quoted context (`Q_HERE_DOCUMENT` behaves
//! quoted for list forms — GNU diagnoses there too), and the verdict
//! abandons the whole command (execute_cmd.c:5522 -> redir.c
//! do_redirections runs in the forked child; the child's
//! `parameter_bad_substitution` latch now crosses the fork-emulation
//! restore in with_forked_heredoc_expansion).
//!
//! The `${}` fragment's quote context reaches the gate through
//! EXTQUOTE_NAME_QUOTED_CONTEXT (`in_double || double_quoted || heredoc`
//! recorded at each embedded-walker dispatch, plus the lexer's
//! QUOTED_EXPANSION_SITE marker on `$` expansions it saw inside quotes —
//! the fragment's `quoted` bit per GNU subst.c).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3581-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` under rubash and return (stdout, stderr).
fn run_script(script: &str) -> (String, String) {
    let dir = tempdir("case");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr)
}

#[test]
fn quoted_star_list_with_empty_element_diagnoses() {
    // GNU abandons the rest of the failing command's `;`-list, so the
    // status probe runs on the next line (subst.c:10288 DISCARD).
    let (out, err) = run_script(
        "a=(\"\"); echo X\"${a[*]#*a=${-y}}\"Y\necho rc1=$?\n\
         a=(\"\" \"\"); echo P\"${a[*]#*a=${-y}}\"Q\necho rc2=$?\n",
    );
    assert_eq!(out, "rc1=1\nrc2=1\n", "out={out}");
    assert!(
        err.matches("*a=${-y}: bad substitution").count() == 2,
        "err={err}"
    );
    assert!(!out.contains("XY"), "out={out}");
}

#[test]
fn quoted_scalar_at_list_with_empty_element_diagnoses() {
    // A scalar read through `[@]`/`[*]` is a one-element list, so its
    // quoted "" carries QUOTED_NULL and diagnoses too.
    let (out, err) = run_script(
        "x=\"\"; echo A\"${x[@]#*a=${-y}}\"B\necho rc1=$?\n\
         x=\"\"; echo C\"${x[*]#*a=${-y}}\"D\necho rc2=$?\n",
    );
    assert_eq!(out, "rc1=1\nrc2=1\n", "out={out}");
    assert!(
        err.matches("*a=${-y}: bad substitution").count() == 2,
        "err={err}"
    );
}

#[test]
fn quoted_positional_star_with_empty_element_diagnoses() {
    let (out, err) = run_script("set -- \"\"; echo C\"${*#*a=${-y}}\"D\necho rc1=$?\n");
    assert_eq!(out, "rc1=1\n", "out={out}");
    assert!(err.contains("*a=${-y}: bad substitution"), "err={err}");
}

#[test]
fn unquoted_list_with_empty_element_stays_silent() {
    // The joined "" list keeps the removal silent UNQUOTED (subst.c:10320).
    let (out, err) = run_script(
        "a=(\"\"); echo X${a[*]#*a=${-y}}Y; echo rc1=$?\n\
         set -- \"\"; echo C${*#*a=${-y}}D; echo rc2=$?\n",
    );
    assert_eq!(out, "XY\nrc1=0\nCD\nrc2=0\n", "out={out}");
    assert_eq!(err, "");
}

#[test]
fn scalar_and_element_subjects_stay_silent_quoted() {
    // No QUOTED_NULL on scalars/elements/bare names: "" is silent quoted.
    let (out, err) = run_script(
        "x=\"\"; echo A\"${x#*a=${-y}}\"B; echo rc1=$?\n\
         a=(\"\"); echo C\"${a[0]#*a=${-y}}\"D; echo rc2=$?\n\
         a=(\"\"); echo E\"${a#*a=${-y}}\"F; echo rc3=$?\n",
    );
    assert_eq!(out, "AB\nrc1=0\nCD\nrc2=0\nEF\nrc3=0\n", "out={out}");
    assert_eq!(err, "");
}

#[test]
fn absent_list_stays_silent_quoted_or_not() {
    let (out, err) = run_script(
        "a=(); echo A\"${a[*]#*a=${-y}}\"B; echo rc1=$?\n\
         a=(); echo C\"${a[@]#*a=${-y}}\"D; echo rc2=$?\n\
         set --; echo E\"${*#*a=${-y}}\"F; echo rc3=$?\n",
    );
    assert_eq!(out, "AB\nrc1=0\nCD\nrc2=0\nEF\nrc3=0\n", "out={out}");
    assert_eq!(err, "");
}

#[test]
fn heredoc_body_list_pattern_diagnoses_and_aborts_command() {
    // GNU: a bad substitution inside a heredoc body is an expand_word_error
    // in the forked child's do_redirections — the command never runs.
    let (out, err) = run_script(
        "a=(\"\"); cat <<EOT\nX\"${a[*]#*a=${-y}}\"Y\nEOT\n\
         echo rc=$?\n\
         a=(\"\"); cat <<EOT\nX\"${a[@]#*a=${-y}}\"Y\nEOT\n\
         echo rc2=$?\n",
    );
    assert_eq!(out, "rc=1\nrc2=1\n", "out={out}");
    assert!(
        err.matches("*a=${-y}: bad substitution").count() == 2,
        "err={err}"
    );
}

#[test]
fn valid_pattern_still_removes_for_quoted_list() {
    // A quoted non-empty list still gets the pattern applied.
    let (out, err) = run_script("a=(xfoo yfoo); echo \"${a[*]#x}\"; echo \"${a[*]%foo}\"\n");
    // GNU joins [*] with IFS then removes once: "xfoo yfoo"#x -> "foo yfoo",
    // "...%foo" -> "x y".
    assert_eq!(out, "foo yfoo\nx y\n", "out={out}");
    assert_eq!(err, "");
}
