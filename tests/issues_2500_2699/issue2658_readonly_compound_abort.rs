// rubash#2658 ([upstream]): a readonly-variable diagnostic inside a
// function was printed twice and did not abort the function body.
//
// GNU mechanism (subst.c:12949-13036 expand_declaration_argument →
// subst.c:13033 exp_jump_to_top_level): a W_COMPASSIGN operand of an
// assignment builtin binds during word expansion. A readonly refusal on
// that nested bind reports err_readonly (error.c:455 — bare
// `{name}: readonly variable`) and longjmps DISCARD: at top level the
// rest of the line dies; inside a function the function body dies and
// the special builtin never runs. declare/typeset/local instead surface
// the nested bind through builtin_error (this_command_name = the
// enclosing function name), skip the bind, and let the outer builtin
// report the same operand a second time with its own name — no abort.

use std::io::Write;
use std::process::Command;

fn run_script(body: &str) -> (String, String, i32) {
    let path = std::env::temp_dir().join(format!(
        "rubash_issue2658_{}_{}.sh",
        std::process::id(),
        body.len()
    ));
    {
        let mut file = std::fs::File::create(&path).expect("create probe script");
        file.write_all(body.as_bytes()).unwrap();
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.env("LC_ALL", "C.UTF-8").arg(&path);
    let out = cmd.output().expect("spawn rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn readonly_compound_in_function_reports_once_and_aborts() {
    let (stdout, stderr, _) = run_script(
        "r=(1); readonly r\nf() { readonly r=(4); declare -p r; echo f-end; }\nf\ndeclare -p r\n",
    );
    assert_eq!(
        stderr.matches("r: readonly variable").count(),
        1,
        "{stderr}"
    );
    assert!(stderr.contains("line 2: r: readonly variable"), "{stderr}");
    assert!(!stdout.contains("f-end"), "{stdout}");
    // Only the outer declare printed — the one inside f never ran.
    assert_eq!(stdout.matches("declare -ar r").count(), 1, "{stdout}");
    assert!(stdout.contains("declare -ar r=([0]=\"1\")"), "{stdout}");
}

#[test]
fn export_compound_in_function_reports_once_and_aborts() {
    let (stdout, stderr, _) =
        run_script("t=(1); readonly t\nf() { export t=(4); echo f-end; }\nf\necho outer\n");
    assert_eq!(
        stderr.matches("t: readonly variable").count(),
        1,
        "{stderr}"
    );
    assert!(!stdout.contains("f-end"), "{stdout}");
    assert!(stdout.contains("outer"), "{stdout}");
}

#[test]
fn declare_compound_in_function_reports_twice_and_continues() {
    let (stdout, stderr, _) =
        run_script("r=(1); readonly r\nf() { declare r=(4); echo f-end; }\nf\necho done\n");
    assert!(
        stderr.contains("line 2: f: r: readonly variable"),
        "{stderr}"
    );
    assert!(
        stderr.contains("line 2: declare: r: readonly variable"),
        "{stderr}"
    );
    assert!(stdout.contains("f-end"), "{stdout}");
    assert!(stdout.contains("done"), "{stdout}");
}

#[test]
fn local_compound_in_function_reports_twice_and_continues() {
    let (stdout, stderr, _) =
        run_script("r=(1); readonly r\nf() { local r=(4); echo f-end; }\nf\necho done\n");
    assert!(
        stderr.contains("line 2: f: r: readonly variable"),
        "{stderr}"
    );
    assert!(
        stderr.contains("line 2: local: r: readonly variable"),
        "{stderr}"
    );
    assert!(stdout.contains("f-end"), "{stdout}");
}

#[test]
fn scalar_readonly_in_function_reports_once_and_continues() {
    let (stdout, stderr, _) =
        run_script("s=1; readonly s\nf() { readonly s=2; echo f-end; }\nf\necho done\n");
    assert_eq!(
        stderr.matches("s: readonly variable").count(),
        1,
        "{stderr}"
    );
    assert!(stdout.contains("f-end"), "{stdout}");
    assert!(stdout.contains("done"), "{stdout}");
}

#[test]
fn top_level_readonly_compound_discards_rest_of_line() {
    let (stdout, stderr, _) =
        run_script("r=(1); readonly r; readonly r=(4); echo same-line\necho next-line\n");
    assert_eq!(
        stderr.matches("r: readonly variable").count(),
        1,
        "{stderr}"
    );
    assert_eq!(stdout, "next-line\n", "{stdout}");
}

// rubash#4160 (R1): `declare -g` (and `typeset -g` / `local -g`) with a
// compound assignment against a readonly global inside a function has no
// invalid_readonly_attrs letters replayed into `make_internal_declare`
// (`declare -g r` with `offset == 0` succeeds), so `do_word_assignment` ->
// `do_compound_assignment` reports one bare-name `err_readonly` line and
// calls `bash_variable_assignment_error(0)` (`DISCARD` — function body
// aborts with rc=1).
#[test]
fn issue4160_declare_g_readonly_compound_reports_once_and_aborts() {
    let (stdout, stderr, code) = run_script(
        "readonly r=1\n\
         f() { declare -g r=(2); echo AFTER_F; }\n\
         g() { typeset -g r=(2); echo AFTER_G; }\n\
         h() { local -g r=(2); echo AFTER_H; }\n\
         f\n\
         echo \"rc_f=$?\"\n\
         g\n\
         echo \"rc_g=$?\"\n\
         h\n\
         echo \"rc_h=$?\"\n\
         declare -p r\n",
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout, "rc_f=1\nrc_g=1\nrc_h=1\ndeclare -r r=\"1\"\n",
        "{stdout}"
    );
    assert_eq!(
        stderr.matches("r: readonly variable").count(),
        3,
        "{stderr}"
    );
    assert!(!stderr.contains("f: r:"), "{stderr}");
    assert!(!stderr.contains("g: r:"), "{stderr}");
    assert!(!stderr.contains("h: r:"), "{stderr}");
    assert!(!stderr.contains("declare: r:"), "{stderr}");
    assert!(!stderr.contains("typeset: r:"), "{stderr}");
    assert!(!stderr.contains("local: r:"), "{stderr}");
}

// rubash#4160 (R1 array target + `-gr` / `-g +r`): `declare -g` against an
// existing readonly array or with `-r` / `+r` also passes the valueless
// `make_internal_declare` (`+r` is not replayed in `omap`) and aborts at
// `do_word_assignment` with one bare-name diagnostic. Contrast: `-ga` / `-gA`
// replay `-a` / `-A` (`invalid_readonly_attrs`) into `make_internal_declare`,
// which fails under `W_FORCELOCAL` (`skip = 1`) and prints two layers without
// aborting.
#[test]
fn issue4160_declare_g_array_and_flag_variants() {
    let (stdout, stderr, code) = run_script(
        "readonly -a r=(1)\n\
         f1() { declare -g r=(2); echo AFTER1; }\n\
         f2() { declare -gr r=(2); echo AFTER2; }\n\
         f3() { declare -g +r r=(2); echo AFTER3; }\n\
         f4() { declare -ga r=(2); echo AFTER4; }\n\
         f1\n\
         echo \"rc1=$?\"\n\
         f2\n\
         echo \"rc2=$?\"\n\
         f3\n\
         echo \"rc3=$?\"\n\
         f4\n\
         echo \"rc4=$?\"\n",
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "rc1=1\nrc2=1\nrc3=1\nAFTER4\nrc4=0\n", "{stdout}");
    assert!(
        stderr.contains("line 2: r: readonly variable\n"),
        "{stderr}"
    );
    assert!(
        stderr.contains("line 3: r: readonly variable\n"),
        "{stderr}"
    );
    assert!(
        stderr.contains("line 4: r: readonly variable\n"),
        "{stderr}"
    );
    assert!(
        stderr.contains("line 5: f4: r: readonly variable\n"),
        "{stderr}"
    );
    assert!(
        stderr.contains("line 5: declare: r: readonly variable\n"),
        "{stderr}"
    );
    assert_eq!(
        stderr.matches("r: readonly variable").count(),
        5,
        "{stderr}"
    );
}

// rubash#4160 (R2): `declare` / `local` / `typeset` with a compound
// assignment against a same-scope readonly local inside a function.
// `make_local_variable` returns the existing same-scope variable (`old_var->
// context == variable_context`) without error, the valueless
// `make_internal_declare` succeeds, and `do_word_assignment` ->
// `do_compound_assignment` emits one bare-name `err_readonly` and DISCARDs.
// With `-a` / `-A` / `-i`, `make_internal_declare` fails at
// `invalid_readonly_attrs` (`skip = 1`) and emits two layers (bare first line
// because the prior command in the function cleared `this_command_name`, then
// builtin-named second line) without aborting.
#[test]
fn issue4160_same_scope_readonly_local_compound_reports_once_and_aborts() {
    let (stdout, stderr, code) = run_script(
        "f1() { local -r r=1; declare r=(2); echo AFTER1; }\n\
         f2() { local -ra r=(1); local r=(2); echo AFTER2; }\n\
         f3() { local -ra r=(1); typeset r=(2); echo AFTER3; }\n\
         f4() { local -r r=1; local -a r=(2); echo AFTER4; }\n\
         f1\n\
         echo \"rc1=$?\"\n\
         f2\n\
         echo \"rc2=$?\"\n\
         f3\n\
         echo \"rc3=$?\"\n\
         f4\n\
         echo \"rc4=$?\"\n",
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "rc1=1\nrc2=1\nrc3=1\nAFTER4\nrc4=0\n", "{stdout}");
    assert!(
        stderr.contains("line 1: r: readonly variable\n"),
        "{stderr}"
    );
    assert!(
        stderr.contains("line 2: r: readonly variable\n"),
        "{stderr}"
    );
    assert!(
        stderr.contains("line 3: r: readonly variable\n"),
        "{stderr}"
    );
    assert!(
        stderr.contains("line 4: r: readonly variable\n"),
        "{stderr}"
    );
    assert!(
        stderr.contains("line 4: local: r: readonly variable\n"),
        "{stderr}"
    );
    assert_eq!(
        stderr.matches("r: readonly variable").count(),
        5,
        "{stderr}"
    );
}
