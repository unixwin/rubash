//! rubash#2043 (S5 of the rubash#2036 sync line): A-class pins —
//! cells where an emulated/in-process face answered a WRONG result
//! silently (rc 0 or a swallowed operand where GNU errors). The
//! three-way corpus lives in `tests/difftest/cases-2036/`; these are
//! the in-suite regression nails for the same four shapes, with the
//! exact GNU coreutils 9.4 / diffutils diagnostics where the face
//! itself produces them.
//!
//!   A1 `mkdir` with no operand exited 0 (fake success) — GNU:
//!      "mkdir: missing operand", rc 1.
//!   A2 `$(basename a b c)` silently dropped the extra operand —
//!      GNU: rc 1, empty value, extra-operand diagnostic.
//!   A3 `$(basename --suffix=.txt /a/b.txt)` treated the option word
//!      as the path and answered ".txt" — GNU: "b".
//!   A4 the diff shebang shortcut judged ANY two files differing
//!      only in the shebang line as equal — GNU diff: rc 1 with the
//!      difference block. The shortcut now answers only for the
//!      process-substitution shape it exists for.

use std::process::Command;

fn rubash() -> std::path::PathBuf {
    let mut path = std::env::current_exe().expect("current exe");
    path.pop();
    path.pop();
    path.push("rubash");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    path
}

struct Output {
    status: i32,
    stdout: String,
    stderr: String,
}

fn run(script: &str, dir: &std::path::Path) -> Output {
    let output = Command::new(rubash())
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("spawn rubash");
    Output {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2043-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

// ---- A1 -----------------------------------------------------------------

#[test]
fn issue2043_a1_mkdir_no_operand_is_missing_operand() {
    let dir = scratch("a1");
    for script in ["mkdir", "mkdir -p", "mkdir --"] {
        let out = run(script, &dir);
        assert_eq!(out.status, 1, "{script}: GNU exits 1, got {}", out.status);
        assert_eq!(
            out.stderr, "mkdir: missing operand\nTry 'mkdir --help' for more information.\n",
            "{script}: exact GNU coreutils 9.4 diagnostic"
        );
        assert_eq!(out.stdout, "", "{script}: nothing on stdout");
    }
    // The admitted shapes are untouched: a real operand still works.
    let out = run("mkdir newdir; echo rc=$?", &dir);
    assert_eq!(out.stdout, "rc=0\n");
    assert!(dir.join("newdir").is_dir());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- A2 / A3 --------------------------------------------------------------

#[test]
fn issue2043_a2_comsub_basename_extra_operand() {
    let dir = scratch("a2");
    // The value and status are GNU's: rc 1, empty substitution. The
    // extra-operand diagnostic also reaches the terminal since
    // rubash#2086 closed the comsub child-stderr passthrough gap
    // (a spawned child's stderr inside $() was swallowed wholesale by
    // the substitution plumbing; the S5 corpus case
    // cp-comsub-diagnostic now asserts the GNU expectation too).
    let out = run("x=$(basename a b c); echo \"rc=$? [$x]\"", &dir);
    assert_eq!(out.stdout, "rc=1 []\n");
    assert_eq!(
        out.stderr,
        "basename: extra operand 'c'\nTry 'basename --help' for more information.\n"
    );
    // On the simple face the same decline reaches the real basename
    // with its diagnostic intact — the machinery's target behavior.
    let out = run("basename a b c", &dir);
    assert_eq!(out.status, 1);
    assert_eq!(
        out.stderr,
        "basename: extra operand 'c'\nTry 'basename --help' for more information.\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2043_a3_comsub_basename_option_word_is_an_option() {
    let dir = scratch("a3");
    let out = run(
        "x=$(basename --suffix=.txt /a/b.txt); echo \"rc=$? [$x]\"",
        &dir,
    );
    assert_eq!(
        out.stdout, "rc=0 [b]\n",
        "GNU answers the suffix-stripped name"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2043_comsub_basename_shares_simple_face_surgery() {
    let dir = scratch("surgery");
    // Shapes the divergent second implementation answered differently
    // from the simple face (and GNU): the root path ("" vs "/"), a
    // `--` terminator (option word treated as path), and a suffix
    // equal to the whole name (stripped vs kept).
    for (script, want) in [
        ("x=$(basename /); echo \"[$x]\"", "[/]\n"),
        ("x=$(basename -- -x); echo \"[$x]\"", "[-x]\n"),
        ("x=$(basename txt txt); echo \"[$x]\"", "[txt]\n"),
        ("x=$(basename /a/b.txt .txt); echo \"[$x]\"", "[b]\n"),
        ("x=$(basename); echo \"rc=$? [$x]\"", "rc=1 []\n"),
    ] {
        let out = run(script, &dir);
        assert_eq!(out.stdout, want, "{script}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- A4 -----------------------------------------------------------------

#[test]
fn issue2043_a4_diff_shebang_only_difference_is_a_difference() {
    let dir = scratch("a4");
    std::fs::write(dir.join("d1"), "#!/a\necho x\n").expect("write d1");
    std::fs::write(dir.join("d2"), "#!/b\necho x\n").expect("write d2");
    let out = run("diff d1 d2", &dir);
    assert_eq!(out.status, 1, "GNU diff reports the shebang difference");
    assert!(
        out.stdout.contains("1c1\n< #!/a\n---\n> #!/b\n"),
        "real diff output expected, got {:?}",
        out.stdout
    );
    // Identical files still compare equal through the real diff.
    let out = run("diff d1 d1; echo rc=$?", &dir);
    assert_eq!(out.stdout, "rc=0\n");
    let _ = std::fs::remove_dir_all(&dir);
}
