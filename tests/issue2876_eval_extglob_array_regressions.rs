//! rubash#2876 — An eval'd compound array assignment whose element
//! carries an extglob pattern is a syntax error even with extglob on.
//!
//! GNU keeps ONE live extglob value: `shopt` updates `extglob_flag`
//! (builtins/shopt.def:640), `reset_parser` propagates it to the
//! parser's `extended_glob` (parse.y:3502), and `eval`'s
//! parse_and_execute (builtins/evalstring.c) parses AND runs the
//! string against that same live value. rubash splits compound
//! assignment elements at execution time
//! (`split_compound_assignment_words` reads the parse gate when the
//! assignment runs), so the gate the eval path pins for its reparse
//! must still be in force while the eval'd text executes — restoring
//! it right after the parse leaves the execution-side split reading
//! the ambient stream gate and the `(` after a pattern operator is
//! not consumed as a group.
//!
//! This is the bash-completion `_comp_expand_glob` shape
//! (`eval -- "$1=($2)"`, bash_completion line 358) that broke shell
//! startup with the distro bash-completion 2.14 loaded.
//!
//! Expectations verified against GNU bash 5.3.20, script-file and
//! `-c` invocations: the five-operator outputs, the expand_glob
//! result list, and the failure statuses (an eval'd array-assignment
//! syntax error yields rc=1; the case-pattern shape yields rc=2, see
//! issue1114) are byte-exact; diagnostic wording is matched on its
//! stable core because the line prefix carries the binary's own path.

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

#[test]
fn eval_compound_assignment_accepts_all_five_extglob_operators() {
    let (stdout, stderr, code) = rubash(
        "shopt -s extglob\n\
         for pat in '!(a)' '+(a)' '*(a)' '?(a)' '@(a|b)'; do\n\
         eval \"arr=(x/$pat)\"; echo \"eval $pat -> rc=$? arr=${arr[*]}\";\n\
         done",
    );
    assert_eq!(
        stdout,
        "eval !(a) -> rc=0 arr=x/!(a)\n\
         eval +(a) -> rc=0 arr=x/+(a)\n\
         eval *(a) -> rc=0 arr=x/*(a)\n\
         eval ?(a) -> rc=0 arr=x/?(a)\n\
         eval @(a|b) -> rc=0 arr=x/@(a|b)\n"
    );
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_compound_assignment_bash_completion_expand_glob_shape() {
    // bash-completion 2.14 `_comp_expand_glob` (line 335) runs
    // `eval -- "$1=($2)"` (line 358); `_comp_sysv_services` passes
    // '"$svcdir"/!($_comp_backup_glob)'. Rebuild that call shape
    // against real files: the .bak entry must be excluded by the
    // negated pattern, the other two must land in the result array.
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2876-expand-glob-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    for name in ["alpha", "beta", "alpha.bak"] {
        std::fs::write(dir.join(name), b"").expect("create fixture file");
    }
    let dir = dir.to_string_lossy().into_owned();
    let script = format!(
        "shopt -s extglob\n\
         _comp_backup_glob='*.bak'\n\
         _comp_expand_glob() {{ local resultvar=$1 glob=$2; eval -- \"$resultvar=($glob)\"; }}\n\
         _comp_expand_glob svcs \"\\\"{dir}\\\"/!($_comp_backup_glob)\"\n\
         echo \"rc=$? svcs=${{svcs[*]}}\""
    );
    let (stdout, stderr, code) = rubash(&script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        stdout,
        format!("rc=0 svcs={dir}/alpha {dir}/beta\n"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_extglob_controls_do_not_regress() {
    // Shapes that already agree with GNU must keep agreeing: a direct
    // compound assignment, an eval'd case pattern (#1114), an eval'd
    // bare glob word, a nested eval, and a trap action string carrying
    // the same assignment shape.
    let dir = std::env::temp_dir().join(format!("rubash-issue2876-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create empty dir");
    let script = format!(
        "shopt -s extglob\n\
         arr=(x/!(a)); echo \"direct rc=$? arr=${{arr[*]}}\"\n\
         v=abc123\n\
         eval 'case $v in +([a-z])+([0-9])) echo case-m;; *) echo case-no;; esac'\n\
         echo \"case rc=$?\"\n\
         cd \"{}\" || exit 99\n\
         eval \"echo bare/!(x)\"\n\
         echo \"bare rc=$?\"\n\
         eval 'eval \"narr=(z/!(c))\"'\n\
         echo \"nested rc=$? narr=${{narr[*]}}\"\n\
         trap 'tarr=(y/!(b)); echo \"in-trap tarr=${{tarr[*]}}\"' EXIT",
        dir.to_string_lossy()
    );
    let (stdout, stderr, code) = rubash(&script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        stdout,
        "direct rc=0 arr=x/!(a)\n\
         case-m\n\
         case rc=0\n\
         bare/!(x)\n\
         bare rc=0\n\
         nested rc=0 narr=z/!(c)\n\
         in-trap tarr=y/!(b)\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn eval_compound_assignment_extglob_off_stays_rejected() {
    // Parity controls (GNU rejects both, rc=1): with extglob never
    // enabled the eval'd assignment is a syntax error, and enabling
    // extglob inside the same eval string does not rescue it — the
    // string is parsed with the state at eval entry, so the embedded
    // `shopt -s` never runs either.
    let (stdout, _, code) = rubash(
        "eval 'arr=(x/!(a))' 2>&1; echo \"off-eval rc=$?\"\n\
         eval 'shopt -s extglob; arr=(x/!(a))' 2>&1; echo \"self-enable rc=$?\"\n\
         shopt -q extglob; echo \"extglob-after rc=$?\"",
    );
    assert!(
        stdout.contains("syntax error near unexpected token `('"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("off-eval rc=1\n"), "stdout: {stdout}");
    assert!(stdout.contains("self-enable rc=1\n"), "stdout: {stdout}");
    assert!(stdout.contains("extglob-after rc=1\n"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_shopt_toggle_inside_eval_persists() {
    // A shopt change executed BY the eval'd text is a real shell-state
    // change (builtins/shopt.def:640): it must still be in force for
    // the next eval's parse AND execution, and disabling it inside an
    // eval must close the gate again. Re-pinning the gate to the live
    // option state when the eval unit returns (instead of restoring
    // the value saved at entry) is what keeps this true.
    let (stdout, _, code) = rubash(
        "eval 'shopt -s extglob'; echo \"enable-via-eval rc=$?\"\n\
         eval 'arr=(x/!(a))'; echo \"second-eval rc=$? arr=${arr[*]}\"\n\
         eval 'shopt -u extglob'\n\
         eval 'arr=(x/!(a))' 2>&1; echo \"after-disable-eval rc=$?\"",
    );
    assert!(
        stdout.contains("enable-via-eval rc=0\n"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("second-eval rc=0 arr=x/!(a)\n"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("syntax error near unexpected token `('"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("after-disable-eval rc=1\n"),
        "stdout: {stdout}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn eval_compound_assignment_in_function_and_comsub() {
    let (stdout, stderr, code) = rubash(
        "shopt -s extglob\n\
         f() { eval \"arr=(x/!(a))\"; echo \"func rc=$? arr=${arr[*]}\"; }\n\
         f\n\
         out=$(eval 'carr=(x/!(a)); printf \"%s\" \"${carr[0]}\"')\n\
         echo \"comsub rc=$? out=$out\"",
    );
    assert_eq!(
        stdout,
        "func rc=0 arr=x/!(a)\n\
         comsub rc=0 out=x/!(a)\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
