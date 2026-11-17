//! rubash#3808: a `{ ...; }` brace-group body on a process substitution
//! is rejected with `ambiguous redirect`; GNU accepts it.
//!
//! GNU owner: parse.y:3776-3796 (read_token returns `<(`/`>(` as a
//! process-substitution WORD, never as a redirect operator) +
//! parse.y:4451 parse_comsub (the body is a full command list, so a
//! `{ ...; }` group is a legal body) + subst.c:1311
//! extract_process_subst / subst.c:6362 process_substitute. The
//! subshell-list body `>(echo a; echo b)` already works in rubash
//! (the form the #3731 pins use); only the group body is rejected.
//! Found during the #3731 fix; the group form is a separate parser
//! gap, not part of #3801.
//!
//! GNU ground truth (bash 5.3.20, script-file probes): the fd-3
//! driver below prints `a`, `b`, then the three outer reads
//! (`r1=0 l1=[o1] r2=0 l2=[o2] r3=0 l3=[o3]`), rc=0; the
//! while-read input form prints `got:[a]`, `got:[b]`, rc=0; the
//! standalone word form runs the body (prints `a`, `b`) and then
//! tries to execute the expanded `/dev/fd/N` word (rc=126,
//! `Permission denied` on Linux — that exec failure is NOT pinned
//! here: the standalone cell pins only that the body runs and no
//! `ambiguous redirect` is reported, since the `/dev/fd` exec face
//! is platform-dependent).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3808-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let mut file = std::fs::File::create(dir.join("driver.sh")).expect("write driver");
    file.write_all(driver.as_bytes()).expect("write driver");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("driver.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The #3731 compound driver shape with a group body: `exec 3>
/// >({ echo a; echo b; })` must bind fd 3 onto the substitution and
/// print both body lines at the close, with the outer fd 0 reads
/// undisturbed.
#[test]
fn group_body_output_procsub_on_fd3() {
    let (stdout, stderr, rc) = run_driver(
        "fd3",
        "printf 'o1\\no2\\no3\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         exec 3> >({ echo a; echo b; })\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo hello >&3\n\
         exec 3>&-\n\
         IFS= read -r l2 <&0; r2=$?\n\
         IFS= read -r l3 <&0; r3=$?\n\
         echo \"r1=$r1 l1=[$l1] r2=$r2 l2=[$l2] r3=$r3 l3=[$l3]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a\nb\nr1=0 l1=[o1] r2=0 l2=[o2] r3=0 l3=[o3]\n");
}

/// Input side: a group body as the target of an input redirect
/// (`done < <({ ...; })`) feeds the loop both lines.
#[test]
fn group_body_input_procsub_feeds_loop() {
    let (stdout, stderr, rc) = run_driver(
        "input",
        "while IFS= read -r l; do echo \"got:[$l]\"; done < <({ echo a; echo b; })\n\
         echo \"rc=$?\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "got:[a]\ngot:[b]\nrc=0\n");
}

/// Control: the same while-read loop with the subshell-list body
/// passes on base — the gap is the group body, not the input
/// procsub shape.
#[test]
fn subshell_body_input_procsub_feeds_loop_control() {
    let (stdout, stderr, rc) = run_driver(
        "input-control",
        "while IFS= read -r l; do echo \"got:[$l]\"; done < <(echo a; echo b)\n\
         echo \"rc=$?\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "got:[a]\ngot:[b]\nrc=0\n");
}

/// The issue's literal seed: a standalone `>({ echo a; echo b; })`
/// word. GNU runs the body (prints `a`, `b`) and only then fails
/// trying to execute the expanded `/dev/fd/N` word — it never
/// reports `ambiguous redirect`. The body output is awaited with a
/// sleep (the body runs asynchronously); the exec-failure
/// diagnostic is deliberately not pinned (fd number and wording
/// are platform-dependent).
#[test]
fn group_body_standalone_word_runs_body() {
    let (stdout, stderr, rc) = run_driver(
        "standalone",
        ">({ echo a; echo b; })\n\
         sleep 0.5\n\
         echo done\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "a\nb\ndone\n", "stderr: {stderr}");
    assert!(!stderr.contains("ambiguous redirect"), "stderr: {stderr}");
}
