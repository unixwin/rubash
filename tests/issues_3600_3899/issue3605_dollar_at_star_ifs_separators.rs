//! Issue rubash#3605 regression: inside `${param OP word}` the `$@`/`$*`
//! elements keep their internal whitespace protected and the ELEMENT
//! boundaries are joined by the separator GNU's string_list_dollar_*
//! pick — not by a bare ' '.
//!
//! GNU anchors:
//! - subst.c:12095-12101 comment: "string_list_dollar_at has separated the
//!   positional parameters with the first character of $IFS" — under
//!   IFS=':' the element boundary byte is ':' so the ambient splitter
//!   recovers the fields.
//! - subst.c:2975-3000 string_list_dollar_at: under PF_ASSIGNRHS the
//!   separator is always ' ' (`IFS=':' v=${v- "$@"}` stores `a b`), else
//!   $IFS[0] when IFS is set non-empty, else ' ' — a set-but-empty IFS
//!   still yields ' ' for `$@`.
//! - subst.c:2902-2930 string_list_dollar_star uses ifs_firstchar
//!   (subst.c:2864): a set-but-empty IFS returns a zero-length separator
//!   and the elements concatenate (`IFS='' x=$*` stores `ab`).
//!
//! Every expectation below is byte-verified against GNU Bash 5.3.20
//! (/opt/homebrew/bin/bash, script-file probes /tmp/glue14-23.sh
//! 2026-11-16).

use std::process::Command;

/// Run a script FILE and return (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i3605atstar-{}-{}",
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

/// `$@` element boundaries follow IFS[0], while the whitespace INSIDE an
/// element stays protected (dollar-at-star9 `Q${q- "$@" }W` family).
#[test]
fn dollar_at_boundaries_follow_ifs_first_char() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "set -- p 'q r' r\n",
        "unset q IFS\n",
        "recho A${q- \"$@\" }B\n",
        "recho Q${q- \"$@\" }W\n",
        "IFS=':'\n",
        "recho A${q- \"$@\" }B\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        stdout,
        concat!(
            "argv[1] = <Ap>\nargv[2] = <q r>\nargv[3] = <rB>\n",
            "argv[1] = <Qp>\nargv[2] = <q r>\nargv[3] = <rW>\n",
            "argv[1] = <A p>\nargv[2] = <q r>\nargv[3] = <r B>\n",
        )
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// `$*` under a set-but-empty IFS concatenates elements with no
/// separator at all (subst.c:2902 ifs_firstchar): `set -- '' ''` yields
/// rhs `  `, `set -- a b` yields `ab`.
#[test]
fn dollar_star_empty_ifs_concatenates() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "IFS=''\n",
        "set -- '' ''\n",
        "recho ${v= \"$*\" }\n",
        "recho \"$v\"\n",
        "set -- a b\n",
        "x=$*\n",
        "printf 'x=<%s>\\n' \"$x\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "argv[1] = <  >\nargv[1] = <  >\nx=<ab>\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// Under PF_ASSIGNRHS `$@` elements are separated with ' ' even when IFS
/// is non-whitespace (subst.c:2975-3000 comment "make sure positional
/// parameters are separated with a space even when word splitting will
/// not occur").
#[test]
fn dollar_at_assignrhs_joins_on_space() {
    let script = concat!(
        "set -- a b\n",
        "IFS=':'\n",
        "unset v\n",
        "v=${v- \"$@\"}\n",
        "printf 'v=<%s>\\n' \"$v\"\n",
        "unset v\n",
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "recho ${v- \"$@\" }\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "v=< a b>\nargv[1] = < a>\nargv[2] = <b >\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// The `=` operator stores the has_dollar_at rhs verbatim — the
/// W_ASSIGNRHS list_string keeps the edge whitespace
/// (`x=${q- "$@" }` stores ` p q r r `).
#[test]
fn assign_operator_stores_verbatim_rhs() {
    let script = concat!(
        "set -- p 'q r' r\n",
        "unset x\n",
        "x=${q- \"$@\" }\n",
        "printf 'x=<%s>\\n' \"$x\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "x=< p q r r >\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// An empty IFS disables field splitting, but the protection pairs a
/// running splitter would consume must still be stripped — GNU's final
/// dequote (subst.c:4807 dequote_string) runs regardless of IFS.
/// `IFS=''; X='X Y'; recho ${0+ "$X" }` is ` X Y `, never ` X\x1c Y `
/// (the quoted expansion whitespace inside `"$X"` rides an IFS_GLUE
/// pair that leaked verbatim when the splitter was skipped).
#[test]
fn empty_ifs_still_strips_protection_glue() {
    let script = concat!(
        "IFS=''\n",
        "set -- X Y\n",
        "X='X Y'\n",
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "recho ${0+ \"$X\" }\n",
        "recho ${v- \"$@\" }\n",
        "v=$'a b'\n",
        "recho ${0+ \"$v\" }\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    // All expectations byte-verified against GNU Bash 5.3.20.
    assert_eq!(
        stdout,
        "argv[1] = < X Y >\nargv[1] = < X>\nargv[2] = <Y >\nargv[1] = < a b >\n"
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// A declaration-builtin operand whose raw token is assignment-shaped
/// (`export`/`readonly`/`declare`/`typeset`/`local` + `name=...`,
/// general.c:480) expands with PF_ASSIGNRHS: bare `$@`/`${@}` elements
/// separate with ' ' — never $IFS[0] — because no word splitting runs
/// (string_list_dollar_at subst.c:2975-3000). `typeset t="x$@y"` under
/// IFS=, binds `xa,x b yy`, not `xa,x,b yy` (dollar-at-star8 a1/a2).
/// `$*` is unaffected (string_list_dollar_star joins IFS[0] regardless).
/// All expectations byte-verified against GNU Bash 5.3.20.
#[test]
fn declare_assignment_operand_dollar_at_joins_on_space() {
    let script = concat!(
        "IFS=,\n",
        "set -- a b c\n",
        "typeset t=\"x$@y\" u=$@ w=\"${*}\" x=$*\n",
        "printf 't=<%s> u=<%s> w=<%s> x=<%s>\\n' \"$t\" \"$u\" \"$w\" \"$x\"\n",
        "export e=\"$@\"\n",
        "readonly r=\"p$@q\"\n",
        "declare -p e r\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        stdout,
        concat!(
            "t=<xa b cy> u=<a b c> w=<a,b,c> x=<a,b,c>\n",
            "declare -x e=\"a b c\"\ndeclare -r r=\"pa b cq\"\n",
        )
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// The `=`/`:=` bind path dequotes the rhs the way GNU's
/// parameter_brace_expand_rhs does (`dequote_string` before
/// `bind_variable`, subst.c:8096/8151): a quoted-empty rhs span leaves
/// a CTLNUL that NOSPLIT2 keeps out of ASSIGNRHS results (subst.c:11853),
/// so the variable cell never holds the sentinel. After `recho ${v= ''}`,
/// `recho $v` must produce zero words — not echo the raw marker byte
/// (dollar-star10 family, `set -- ''`).
/// All expectations byte-verified against GNU Bash 5.3.20.
#[test]
fn assign_operator_stores_dequoted_empty_rhs() {
    let script = concat!(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n",
        "set -- ''\n",
        "unset v\n",
        "recho ${v= ''}\n",
        "recho $v\n",
        "printf 'v=<%s>\\n' \"$v\"\n",
        "unset v\n",
        "recho ${v= $*}\n",
        "recho $v\n",
        "printf 'v=<%s>\\n' \"$v\"\n",
        "unset v IFS\n",
        "recho ${v= ''}\n",
        "recho $v\n",
        "printf 'v=<%s>\\n' \"$v\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "v=< >\nv=< >\nv=< >\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn star_modified_forms_join_on_ifs_first_char() {
    // GNU string_list_pos_params (subst.c:3030-3057): every modified `*`
    // form — transform `${*@Q}`/`${*@a}`, pattern removal `${*/1/X}` and
    // case mod `${*,,}` — joins elements with ifs_firstchar, which is the
    // empty string when IFS is set empty. The Posix-888 dollar_at ' '
    // fallback applies only to the UNQUOTED expansion. Byte-verified
    // against GNU Bash 5.3.20 (probe /tmp/s9_q2.sh 2026-11-16).
    let (stdout, stderr, code) = rubash_file(
        "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n\
         set -- 1 2\n\
         IFS=\n\
         recho \"${*@Q}\"\n\
         recho \"${var-${*@Q}}\"\n\
         unset var; recho \"${var-x${*@Q}y}\"\n\
         recho \"${*/1/X}\"\n\
         recho \"${*,,}\"\n",
    );
    assert_eq!(
        stdout,
        "argv[1] = <'1''2'>\n\
         argv[1] = <'1''2'>\n\
         argv[1] = <x'1''2'y>\n\
         argv[1] = <X2>\n\
         argv[1] = <12>\n"
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}
