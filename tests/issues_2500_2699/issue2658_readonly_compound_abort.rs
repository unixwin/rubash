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
