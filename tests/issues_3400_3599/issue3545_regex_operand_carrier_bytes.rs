//! rubash#3545 — `[[ STR =~ REGEX ]]` with carrier bytes
//! (0x0c, 0x14..=0x1f, incl. ESC) in the regex operand never matches.
//!
//! A carrier byte written as a `$'...'` literal, or arriving via `${c}`
//! after a `|` in the regex operand, fails to match in rubash while GNU
//! bash 5.3.20 matches; the same byte via `a${c}b` at the start of the
//! operand matches on both sides. Non-carrier control bytes (0x01, 0x07,
//! 0x7f) agree on both sides.
//!
//! GNU: execute_cmd.c:4068 `arg2 = cond_expand_word (cond->right->op,
//! mode)` (mode 2 for `=~` above compat 31) hands the expanded operand
//! to sh_regmatch (execute_cmd.c:4096) as raw bytes — subst.c:4114
//! QGLOB_REGEXP / pathexp.c:212 quote_string_for_globbing only
//! literalizes quoted regex metacharacters, the carrier byte itself is
//! data and reaches regcomp unchanged. Expectations below are the GNU
//! 5.3.20 oracle's (script file, `LC_ALL` unset, i.e. the C-locale
//! byte-mode path).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_file(name: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3545-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    let script_path: PathBuf = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash script file");
    let _ = fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's repro loop verbatim (master 8b48c164).
#[test]
fn issue_repro_loop_verbatim() {
    let script = r#"for c in $'\x01' $'\x07' $'\x0c' $'\x14' $'\x1b' $'\x1f' $'\x7f'; do
  w="a${c}b"
  [[ $w =~ a${c}b ]] && r1=y || r1=n
  [[ $w =~ $'\x01'|${c} ]] && r2=y || r2=n
  printf '%02x var=%s alt=%s\n' "'$c" $r1 $r2
done
w=$'a\033[0mb'
[[ $w =~ $'\033' ]] && echo lit-y || echo lit-n
"#;
    let (stdout, stderr, status) = rubash_file("repro-loop", script);
    assert_eq!(
        stdout,
        "01 var=y alt=y\n07 var=y alt=y\n0c var=y alt=y\n14 var=y alt=y\n1b var=y alt=y\n1f var=y alt=y\n7f var=y alt=y\nlit-y\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// The full assignment-carrier set (`is_assignment_carrier_byte`:
/// 0x0c, 0x11, 0x13, 0x14..=0x1f) plus non-carrier controls, each in
/// the var form, the after-`|` form and the `$'...'` literal form.
#[test]
fn full_carrier_set_var_alt_and_lit() {
    const BYTES: [&str; 18] = [
        "0c", "11", "13", "14", "15", "16", "17", "18", "19", "1a", "1b", "1c", "1d", "1e", "1f",
        "01", "07", "7f",
    ];
    let mut script = String::new();
    for hex in BYTES {
        script.push_str(&format!(
            "c=$'\\x{hex}'\nw=\"a${{c}}b\"\n[[ $w =~ a${{c}}b ]] && r1=y || r1=n\n[[ $w =~ $'\\x01'|${{c}} ]] && r2=y || r2=n\n[[ $w =~ $'\\x{hex}' ]] && r3=y || r3=n\nprintf '%s %s %s %s\\n' \"{hex}\" \"$r1\" \"$r2\" \"$r3\"\n"
        ));
    }
    let (stdout, stderr, status) = rubash_file("full-set", &script);
    let expected: String = BYTES.iter().map(|hex| format!("{hex} y y y\n")).collect();
    assert_eq!(stdout, expected, "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Position contrast for the after-`|` shape plus the captured text:
/// the carrier byte matches bare, on either side of `|`, at the
/// start of the operand, and `BASH_REMATCH[0]` is the byte itself.
#[test]
fn pipe_position_contrast_and_rematch() {
    let script = r#"c=$'\x1b'
w="a${c}b"
[[ $w =~ ${c} ]] && echo bare-y || echo bare-n
[[ $w =~ x|${c} ]] && echo alt2-y || echo alt2-n
[[ $w =~ ${c}|x ]] && echo alt3-y || echo alt3-n
[[ $w =~ a${c}b ]] && echo var-y || echo var-n
[[ $w =~ $'\x01'|${c} ]] && echo alt-y || echo alt-n
[[ $w =~ $'\033' ]] && echo lit-y || echo lit-n
w2=$'a\033[0mb'
[[ $w2 =~ $'\033' ]] && printf 'rematch=%02x len=%s\n' "'${BASH_REMATCH[0]}" "${#BASH_REMATCH[0]}" || echo rematch-n
"#;
    let (stdout, stderr, status) = rubash_file("positions", script);
    assert_eq!(
        stdout, "bare-y\nalt2-y\nalt3-y\nvar-y\nalt-y\nlit-y\nrematch=1b len=1\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}
