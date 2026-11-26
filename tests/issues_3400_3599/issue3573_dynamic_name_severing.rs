//! rubash#3573: BASH_ALIASES and BASH_CMDS are dynamic views of the
//! alias/hash tables only while unbound. GNU variables.c:3839
//! unbind_variable severs that link permanently: `unset` makes the name
//! an ordinary variable for the rest of the shell's lifetime, so a
//! compound `NAME=([k]=v)` on the severed name builds a fresh INDEXED
//! array (do_compound_assignment subst.c:3441), never the assoc the
//! stale attribute would imply. `unset` is the ONLY sever: a scalar
//! `NAME=v` on the LIVE name does NOT sever — bind_variable_internal
//! (variables.c:3128-3144) dispatches it to the variable's assign_func
//! with key "0", so the value lands as owner-table element [0] and the
//! dynamic link survives.
//!
//! Oracle: vendored GNU bash 5.3.20 (third_party/bash/bash) run on the
//! same script files; the `declare -p` assertions below are GNU's own
//! hash-bucket render order, byte-for-byte.

use std::path::PathBuf;
use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3573-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create probe directory");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        dir,
    )
}

#[test]
fn unset_dynamic_name_stays_unbound() {
    // GNU: `declare -p` reports not found and the alias table no longer
    // resurrects the name — no `declare -A` re-materialization.
    let (stdout, stderr, code, _) = run_script(
        "unset-stays-unbound",
        "unset BASH_ALIASES\n\
         declare -p BASH_ALIASES\n\
         alias q='echo q'\n\
         declare -p BASH_ALIASES\n\
         echo \"arr:${BASH_ALIASES[@]-NONE}\"\n\
         echo done\n",
    );
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "arr:NONE\ndone\n", "stdout={stdout:?}");
    assert_eq!(
        stderr.matches("declare: BASH_ALIASES: not found").count(),
        2,
        "stderr={stderr:?}"
    );
}

#[test]
fn unset_then_compound_assign_builds_indexed() {
    // The issue's core case: `NAME=([k]=v)` on an unbound dynamic name
    // is a fresh indexed array whose binding outlives owner-table
    // writes.
    let (stdout, stderr, code, _) = run_script(
        "unset-compound",
        "unset BASH_ALIASES\n\
         BASH_ALIASES=([u]='echo u')\n\
         declare -p BASH_ALIASES\n\
         alias q='echo q'\n\
         declare -p BASH_ALIASES\n\
         unset BASH_CMDS\n\
         BASH_CMDS=(a b c)\n\
         declare -p BASH_CMDS\n",
    );
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
    assert_eq!(
        stdout,
        "declare -a BASH_ALIASES=([0]=\"echo u\")\n\
         declare -a BASH_ALIASES=([0]=\"echo u\")\n\
         declare -a BASH_CMDS=([0]=\"a\" [1]=\"b\" [2]=\"c\")\n"
    );
}

#[test]
fn live_scalar_assign_binds_element_zero() {
    // GNU variables.c:3128-3144: a scalar `NAME=v` bind on the LIVE
    // dynamic name dispatches to assign_aliasvar with key "0" — it
    // binds owner-table element [0] and the dynamic link survives,
    // so the later `alias s` still merges in. The element order below
    // is GNU's own hash-bucket order, byte-for-byte from the 5.3.20
    // oracle (`[r] [0]` then `[s] [r] [0]`).
    // (A compound `NAME=(...)` on the live name merges into the owner
    // table too — a separate GNU mechanism.)
    let (stdout, stderr, code, _) = run_script(
        "live-scalar",
        "alias r='echo r'\n\
         BASH_ALIASES=hello\n\
         declare -p BASH_ALIASES\n\
         alias s='echo s'\n\
         declare -p BASH_ALIASES\n\
         echo \"elem0:${BASH_ALIASES[0]}\"\n",
    );
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
    assert_eq!(
        stdout,
        "declare -A BASH_ALIASES=([r]=\"echo r\" [0]=\"hello\" )\n\
         declare -A BASH_ALIASES=([s]=\"echo s\" [r]=\"echo r\" [0]=\"hello\" )\n\
         elem0:hello\n"
    );
}

#[test]
fn live_scalar_assign_binds_element_zero_hashcmds() {
    // Same dispatch for BASH_CMDS (assign_hashcmd,
    // variables.c:1715): scalar `BASH_CMDS=v` is hash element [0],
    // and `hash -p` afterwards still merges into the live view.
    let (stdout, stderr, code, _) = run_script(
        "live-scalar-cmds",
        "BASH_CMDS=hi\n\
         declare -p BASH_CMDS\n\
         hash -p /bin/ls ls2\n\
         declare -p BASH_CMDS\n\
         echo \"elem0:${BASH_CMDS[0]}\"\n",
    );
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
    assert_eq!(
        stdout,
        "declare -A BASH_CMDS=([0]=\"hi\" )\n\
         declare -A BASH_CMDS=([0]=\"hi\" [ls2]=\"/bin/ls\" )\n\
         elem0:hi\n"
    );
}

#[test]
fn element_assign_on_severed_name_is_plain() {
    // `NAME[k]=v` on a severed name binds an ordinary element — the
    // subscript is arithmetic on a fresh indexed array (k=0), and no
    // alias is created in the owner table.
    let (stdout, stderr, code, _) = run_script(
        "severed-element",
        "unset BASH_ALIASES\n\
         BASH_ALIASES[k]='v'\n\
         declare -p BASH_ALIASES\n",
    );
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
    assert_eq!(stdout, "declare -a BASH_ALIASES=([0]=\"v\")\n");
}

#[test]
fn redeclared_assoc_after_unset_stays_assoc() {
    // A genuine `declare -A` rebind after the sever is a real assoc
    // declaration — the compound assign keeps the keyed elements.
    let (stdout, stderr, code, _) = run_script(
        "redeclare-assoc",
        "unset BASH_ALIASES\n\
         declare -A BASH_ALIASES\n\
         BASH_ALIASES=([u]='echo u')\n\
         declare -p BASH_ALIASES\n",
    );
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
    assert_eq!(stdout, "declare -A BASH_ALIASES=([u]=\"echo u\" )\n");
}

#[test]
fn unsevered_dynamic_view_still_rebuilds() {
    // Regression guard for the unsevered contract (rubash#3107): the
    // materialized assoc still tracks the owner tables.
    let (stdout, stderr, code, _) = run_script(
        "unsevered",
        "alias q='echo q'\n\
         declare -p BASH_ALIASES\n\
         echo \"elem:${BASH_ALIASES[q]}\"\n",
    );
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
    assert_eq!(
        stdout,
        "declare -A BASH_ALIASES=([q]=\"echo q\" )\nelem:echo q\n"
    );
}
