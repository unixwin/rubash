//! rubash#3573: an `unset` dynamic associative variable stays unbound
//! (GNU variables.c:3839 unbind_variable; variables.c:1197-1198 — it
//! "loses its special meaning, even if you subsequently set it"), so a
//! later whole-compound assignment binds an ORDINARY variable: an indexed
//! array with arithmetically evaluated subscripts, never an assoc
//! inferred from string subscripts (subst.c:3441 do_compound_assignment;
//! arrayfunc.c:507 assign_array_from_string; arrayfunc.c:700
//! assign_compound_array_list).
//!
//! Root cause (not the generic compound path — that path is already
//! correct for fresh names, see the FRESHVAR lock below): `unset
//! BASH_ALIASES` / `unset BASH_CMDS` never recorded UNSET_DYNAMIC_VARS,
//! because dynamic_parameter_is_set serves only the scalar dynamic
//! registry and omits the two owner-table-backed assoc variables. The
//! #3220 door therefore never stood down for an unset name, declare -p
//! kept serving the owner view, and alias/hash mutations resurrected
//! the variable. The fix records the unbind at every recording site
//! and honors it in the publisher, the element-bind arms and the
//! element-unset arms.
//!
//! Expectations are the GNU bash 5.3.20 outputs measured for the census
//! in this lane (oracle ~/workspace/rubash-audit/gnu-bash-5.3.20/bin/bash).
//! Fail-first was banked on a pre-fix binary (tip d374e789, no #3220
//! gate, no #3573 fix): cells 1, 2, 5, 6, 7, 8, 9, 10, 11, 12, 14 RED
//! (`declare -A ...=()` / owner-view renders / deleted owner entries);
//! locks 3, 4, 13 GREEN on that binary (13 also guards the #3220 head,
//! where the gate routes the whole assignment into the alias table).

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

fn assert_clean(stdout: &str, stderr: &str, code: Option<i32>, expected: &str) {
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
    assert_eq!(stdout, expected, "stderr={stderr:?}");
}

#[test]
fn unset_dynamic_then_whole_assignment_is_plain() {
    // Seed cell moved from PR #3220 (its pin file
    // tests/issue3211_dynamic_whole_assignment.rs): the #3220 gate
    // stands down for unset names, so this walks the generic path.
    let (stdout, stderr, code, _) = run_script(
        "aliases-unset",
        "unset BASH_ALIASES\n\
         BASH_ALIASES=([u]='echo u')\n\
         declare -p BASH_ALIASES\n",
    );
    assert_clean(
        &stdout,
        &stderr,
        code,
        "declare -a BASH_ALIASES=([0]=\"echo u\")\n",
    );
}

#[test]
fn unset_dynamic_cmds_whole_assignment_is_plain() {
    let (stdout, stderr, code, _) = run_script(
        "cmds-unset",
        "unset BASH_CMDS\n\
         BASH_CMDS=([u]='/bin/echo')\n\
         declare -p BASH_CMDS\n",
    );
    assert_clean(
        &stdout,
        &stderr,
        code,
        "declare -a BASH_CMDS=([0]=\"/bin/echo\")\n",
    );
}

#[test]
fn fresh_name_compound_assignment_is_indexed() {
    // Lock: the generic compound path already builds an indexed array
    // for a name with no live declaration — the fix must not regress it.
    let (stdout, stderr, code, _) = run_script(
        "freshvar",
        "FRESHVAR=([u]='echo u')\n\
         declare -p FRESHVAR\n",
    );
    assert_clean(
        &stdout,
        &stderr,
        code,
        "declare -a FRESHVAR=([0]=\"echo u\")\n",
    );
}

#[test]
fn declared_assoc_unset_then_compound_is_indexed() {
    // Lock: `unset` erases an assoc declaration entirely, so the next
    // compound assignment builds an indexed array (GNU variables.c:3839).
    let (stdout, stderr, code, _) = run_script(
        "assoc-unset",
        "declare -A A\n\
         unset A\n\
         A=([u]='echo u')\n\
         declare -p A\n",
    );
    assert_clean(&stdout, &stderr, code, "declare -a A=([0]=\"echo u\")\n");
}

#[test]
fn bare_unset_dynamic_is_not_found() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-bare-unset",
        "unset BASH_ALIASES\n\
         declare -p BASH_ALIASES\n",
    );
    assert_eq!(code, Some(1), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "", "stderr={stderr:?}");
    assert!(
        stderr.contains("declare: BASH_ALIASES: not found"),
        "stderr={stderr:?}"
    );
}

#[test]
fn scalar_bind_after_unset_is_plain() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-scalar",
        "unset BASH_ALIASES\n\
         BASH_ALIASES=foo\n\
         declare -p BASH_ALIASES\n",
    );
    assert_clean(&stdout, &stderr, code, "declare -- BASH_ALIASES=\"foo\"\n");
}

#[test]
fn append_compound_after_unset_is_indexed() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-append",
        "unset BASH_ALIASES\n\
         BASH_ALIASES+=([v]='echo v')\n\
         declare -p BASH_ALIASES\n",
    );
    assert_clean(
        &stdout,
        &stderr,
        code,
        "declare -a BASH_ALIASES=([0]=\"echo v\")\n",
    );
}

#[test]
fn element_bind_after_unset_is_indexed() {
    // GNU evaluates the subscript arithmetically ([k] -> [0]) and the
    // alias table is untouched.
    let (stdout, stderr, code, _) = run_script(
        "aliases-element",
        "unset BASH_ALIASES\n\
         BASH_ALIASES[k]=v\n\
         declare -p BASH_ALIASES\n",
    );
    assert_clean(
        &stdout,
        &stderr,
        code,
        "declare -a BASH_ALIASES=([0]=\"v\")\n",
    );
}

#[test]
fn alias_after_unset_does_not_resurrect() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-alias-after-unset",
        "unset BASH_ALIASES\n\
         alias q='echo q'\n\
         declare -p BASH_ALIASES\n\
         alias q\n",
    );
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "alias q='echo q'\n", "stderr={stderr:?}");
    assert!(
        stderr.contains("declare: BASH_ALIASES: not found"),
        "stderr={stderr:?}"
    );
}

#[test]
fn element_unset_after_whole_unset_keeps_alias() {
    // The owner table is a separate object: unsetting an element of an
    // unbound dynamic variable is a silent no-op (variables.c:3839).
    let (stdout, stderr, code, _) = run_script(
        "aliases-elem-unset",
        "alias k='echo k'\n\
         unset BASH_ALIASES\n\
         unset BASH_ALIASES[k]\n\
         alias k\n",
    );
    assert_clean(&stdout, &stderr, code, "alias k='echo k'\n");
}

#[test]
fn declare_assoc_after_unset_is_plain_assoc() {
    // `declare -A` after unset creates a plain assoc variable; the
    // compound assignment then fills it, and no alias is created.
    let (stdout, stderr, code, _) = run_script(
        "aliases-redeclare",
        "unset BASH_ALIASES\n\
         declare -A BASH_ALIASES\n\
         BASH_ALIASES=([u]='echo u')\n\
         declare -p BASH_ALIASES\n",
    );
    assert_clean(
        &stdout,
        &stderr,
        code,
        "declare -A BASH_ALIASES=([u]=\"echo u\" )\n",
    );
}

#[test]
fn hash_after_unset_does_not_resurrect() {
    let (stdout, stderr, code, _) = run_script(
        "cmds-hash-after-unset",
        "unset BASH_CMDS\n\
         hash -p /bin/ls ls\n\
         declare -p BASH_CMDS\n",
    );
    assert_eq!(code, Some(1), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "", "stderr={stderr:?}");
    assert!(
        stderr.contains("declare: BASH_CMDS: not found"),
        "stderr={stderr:?}"
    );
}

#[test]
fn whole_assignment_after_unset_creates_no_alias() {
    // Lock: the generic path never touches the alias table (also guards
    // the #3220 head, where the gate routes this bind into the table).
    let (stdout, stderr, code, _) = run_script(
        "aliases-no-alias",
        "unset BASH_ALIASES\n\
         BASH_ALIASES=([u]='echo u')\n\
         alias u\n\
         echo \"alias-rc=$?\"\n",
    );
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "alias-rc=1\n", "stderr={stderr:?}");
    assert!(stderr.contains("alias: u: not found"), "stderr={stderr:?}");
}

#[test]
fn cmds_element_unset_after_whole_unset_keeps_hash() {
    let (stdout, stderr, code, _) = run_script(
        "cmds-elem-unset",
        "hash -p /bin/ls ls\n\
         unset BASH_CMDS\n\
         unset BASH_CMDS[ls]\n\
         hash -t ls\n",
    );
    assert_clean(&stdout, &stderr, code, "/bin/ls\n");
}
