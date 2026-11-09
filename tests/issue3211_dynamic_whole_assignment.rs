//! rubash#3211: whole-variable assignments to the dynamic table
//! variables must route through the owner tables, the way GNU's
//! assign hooks do — `assign_aliasvar` (variables.c:1785) lands each
//! element in the alias table via `add_alias`, `assign_hashcmd`
//! (variables.c:1715) via `phash_insert`, and a scalar bind is an
//! element bind with key "0" (variables.c:3140). Before the fix a
//! compound assignment wrote only the mirror/typed cell, and the
//! command-boundary reconciliation against the (untouched) owner
//! table discarded it.
//!
//! Expectations are the GNU bash 5.3.20 outputs measured for the
//! census in `~/workspace/fix-3211/census-probe.sh` /
//! `census-probe2.sh` (see the lane REPORT).

use std::path::PathBuf;
use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3211-{name}-{}", std::process::id()));
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
fn aliases_whole_assignment_lands_in_owner_table() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-basic",
        "alias old='echo old'\n\
         BASH_ALIASES=([x]='echo x')\n\
         alias x\n\
         alias old\n\
         printf '%s\\n' \"${BASH_ALIASES[x]}\"\n",
    );
    assert_clean(
        &stdout,
        &stderr,
        code,
        "alias x='echo x'\nalias old='echo old'\necho x\n",
    );
}

#[test]
fn cmds_whole_assignment_lands_in_owner_table() {
    let (stdout, stderr, code, _) = run_script(
        "cmds-basic",
        "hash -p /bin/ls ls\n\
         BASH_CMDS=([y]='/bin/echo')\n\
         printf '%s\\n' \"${BASH_CMDS[y]}\"\n\
         printf '%s\\n' \"${BASH_CMDS[ls]}\"\n",
    );
    assert_clean(&stdout, &stderr, code, "/bin/echo\n/bin/ls\n");
}

#[test]
fn aliases_append_form_adds_entries() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-append",
        "BASH_ALIASES=([a]=1)\n\
         BASH_ALIASES+=([b]=2)\n\
         alias a\n\
         alias b\n",
    );
    assert_clean(&stdout, &stderr, code, "alias a='1'\nalias b='2'\n");
}

#[test]
fn aliases_scalar_bind_and_scalar_append_use_element_zero() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-scalar",
        "BASH_ALIASES=foo\n\
         alias 0\n\
         BASH_ALIASES=([0]=ab)\n\
         BASH_ALIASES+=cd\n\
         alias 0\n",
    );
    assert_clean(&stdout, &stderr, code, "alias 0='foo'\nalias 0='abcd'\n");
}

#[test]
fn aliases_empty_compound_keeps_owner_entries() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-empty",
        "alias q='echo q'\n\
         BASH_ALIASES=()\n\
         alias q\n\
         printf '%s\\n' \"${#BASH_ALIASES[@]}\"\n",
    );
    assert_clean(&stdout, &stderr, code, "alias q='echo q'\n1\n");
}

#[test]
fn aliases_invalid_name_in_list_diagnoses_and_continues() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-invalid",
        "BASH_ALIASES=(['a b']=1 [good]=2)\n\
         echo \"rc=$?\"\n\
         alias good\n",
    );
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "rc=0\nalias good='2'\n", "stderr={stderr:?}");
    assert!(
        stderr.contains("`a b': invalid alias name"),
        "stderr={stderr:?}"
    );
}

#[test]
fn aliases_intra_list_append_element() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-intra-append",
        "BASH_ALIASES=([k]=a [k]+=b)\n\
         alias k\n",
    );
    assert_clean(&stdout, &stderr, code, "alias k='ab'\n");
}

#[test]
fn aliases_kvpair_form_lands() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-kvpair",
        "BASH_ALIASES=(kk 'echo kk')\n\
         alias kk\n",
    );
    assert_clean(&stdout, &stderr, code, "alias kk='echo kk'\n");
}

#[test]
fn dirstack_whole_assignment_still_routes() {
    let (stdout, stderr, code, _) = run_script(
        "dirstack-lock",
        "cd /\n\
         pushd /tmp >/dev/null\n\
         pushd /var >/dev/null\n\
         DIRSTACK=([1]=/usr [2]=/etc)\n\
         dirs -p\n",
    );
    assert_clean(&stdout, &stderr, code, "/var\n/usr\n/etc\n");
}

#[test]
fn element_assignment_regression_lock() {
    let (stdout, stderr, code, _) = run_script(
        "element-lock",
        "BASH_ALIASES[k]=v\n\
         alias k\n\
         BASH_CMDS[c]='/bin/c'\n\
         printf '%s\\n' \"${BASH_CMDS[c]}\"\n",
    );
    assert_clean(&stdout, &stderr, code, "alias k='v'\n/bin/c\n");
}

#[test]
fn unset_dynamic_then_whole_assignment_is_plain() {
    let (stdout, stderr, code, _) = run_script(
        "aliases-unset",
        "unset BASH_ALIASES\n\
         BASH_ALIASES=([u]='echo u')\n\
         declare -p BASH_ALIASES\n",
    );
    assert_eq!(code, Some(0), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stderr, "", "stdout={stdout:?}");
    assert_eq!(
        stdout, "declare -a BASH_ALIASES=([0]=\"echo u\")\n",
        "stderr={stderr:?}"
    );
}
