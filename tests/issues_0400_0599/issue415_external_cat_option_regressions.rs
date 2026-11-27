//! rubash#415: the in-engine `external_cat` emulation silently ignored
//! every formatting option (`-n' numbering, `-A'/`-E'/`-T' … were dropped,
//! rc 0 output identical to the no-option run) and accepted unknown
//! options (`-Z' ran as plain cat instead of GNU's rc-1 diagnostic).
//! Owner: src/executor/external_file_builtins.rs external_cat — the
//! bare-cat / /bin/cat dispatch at handle_external_file_builtins.
//!
//! Behavioral reference: WSL GNU coreutils 9.4 `/usr/bin/cat' script-file
//! probes (2026-10-02; artifacts under target/i415/ — gnu-matrix.out,
//! corners2.sh, perm.sh, order2.sh, matrix/run.sh, 174/174 byte-identical
//! over the option surface x FILE/TWOFILES/STDIN/REDIRECT/DASH/PERMUTE).
//! Observable cat.c contract pinned by those probes:
//! * numbering is `%6d\t' right-aligned; `-b' beats `-n' in EVERY argv
//!   order; an unterminated final line is numbered but NOT newline-
//!   terminated, and `-E'/`-A' give it neither `$' nor a newline.
//! * `-s' collapses runs of adjacent EMPTY lines to one across operand
//!   boundaries (one concatenated filter stream).
//! * `-u' is accepted and ignored; GNU getopt permutes argv (`cat f -n'
//!   numbers, `cat f -Z' still errors); `--' ends options; `-` is the
//!   stdin operand; long options accept unambiguous abbreviations and
//!   report ambiguity listing the possibilities in table order.
//! * usage errors print `{argv0}: invalid option -- 'Z'' /
//!   `unrecognized option' / `is ambiguous; possibilities: …' plus
//!   `Try `{argv0} --help' for more information.' and exit 1.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(20);

fn run_rubash_stdin(args: &[&str], stdin_bytes: Option<&[u8]>) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .stdin(if stdin_bytes.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    if let Some(bytes) = stdin_bytes {
        let mut stdin = child.stdin.take().expect("stdin pipe");
        let bytes = bytes.to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                use std::io::Read;
                child
                    .stdout
                    .take()
                    .expect("stdout pipe")
                    .read_to_end(&mut stdout)
                    .expect("read stdout");
                child
                    .stderr
                    .take()
                    .expect("stderr pipe")
                    .read_to_end(&mut stderr)
                    .expect("read stderr");
                return (stdout, stderr, status.code());
            }
            Ok(None) => {
                if start.elapsed() > DEADLINE {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash {args:?} did not exit within {DEADLINE:?}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("wait rubash: {error}"),
        }
    }
}

/// `cat <args> f.txt g.txt' in a scratch directory holding GNU-probe
/// fixtures: f.txt = `alpha\n\nbravo\tcharlie\ndelta' (unterminated tail),
/// g.txt = `\n\n\nsecond\n'.
fn cat_files(args: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i415-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("f.txt"), b"alpha\n\nbravo\tcharlie\ndelta").expect("f.txt");
    std::fs::write(dir.join("g.txt"), b"\n\n\nsecond\n").expect("g.txt");
    let script = format!("cat {args} f.txt g.txt");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.arg("-c").arg(&script).current_dir(&dir);
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (output.stdout, output.stderr, output.status.code())
}

#[test]
fn number_all_lines_matches_gnu() {
    // WSL GNU 9.4: numbering continues across operands; delta (terminated
    // by g.txt's first byte) is line 4; blanks are numbered.
    let (stdout, stderr, code) = cat_files("-n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        concat!(
            "     1\talpha\n",
            "     2\t\n",
            "     3\tbravo\tcharlie\n",
            "     4\tdelta\n",
            "     5\t\n",
            "     6\t\n",
            "     7\tsecond\n",
        )
    );
}

#[test]
fn number_nonblank_matches_gnu_and_beats_n_in_any_order() {
    let expected = concat!(
        "     1\talpha\n",
        "\n",
        "     2\tbravo\tcharlie\n",
        "     3\tdelta\n",
        "\n",
        "\n",
        "     4\tsecond\n",
    );
    for args in ["-b", "-nb", "-bn", "-n -b", "-b -n", "--number-nonblank"] {
        let (stdout, stderr, code) = cat_files(args);
        assert_eq!(code, Some(0), "{args}");
        assert_eq!(stderr, b"", "{args}");
        assert_eq!(String::from_utf8_lossy(&stdout), expected, "{args}");
    }
}

#[test]
fn show_ends_marks_newlines_and_never_terminates_unterminated_tail() {
    // GNU: `$' only where the input HAS a newline. f.txt alone ends with
    // the unterminated `delta' — no `$', no added newline.
    let dir = std::env::temp_dir().join(format!(
        "rubash-i415e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("f.txt"), b"alpha\n\nbravo\tcharlie\ndelta").expect("f.txt");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "cat -E f.txt"])
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "alpha$\n$\nbravo\tcharlie$\ndelta"
    );
}

#[test]
fn show_tabs_and_show_all_match_gnu() {
    let (stdout, _, code) = cat_files("-A");
    assert_eq!(code, Some(0));
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        concat!(
            "alpha$\n",
            "$\n",
            "bravo^Icharlie$\n",
            "delta$\n",
            "$\n",
            "$\n",
            "second$\n",
        )
    );
    let (stdout, _, _) = cat_files("-T");
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        concat!(
            "alpha\n",
            "\n",
            "bravo^Icharlie\n",
            "delta\n",
            "\n",
            "\n",
            "second\n",
        )
    );
}

#[test]
fn squeeze_blank_collapses_runs_across_operands() {
    // g.txt's three blank lines collapse to one; numbering counts the
    // SURVIVING blank (GNU -ns probe).
    let (stdout, _, code) = cat_files("-ns");
    assert_eq!(code, Some(0));
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        concat!(
            "     1\talpha\n",
            "     2\t\n",
            "     3\tbravo\tcharlie\n",
            "     4\tdelta\n",
            "     5\t\n",
            "     6\tsecond\n",
        )
    );
    // -s alone: no numbering at all.
    let (stdout, _, _) = cat_files("-s");
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        concat!(
            "alpha\n",
            "\n",
            "bravo\tcharlie\n",
            "delta\n",
            "\n",
            "second\n",
        )
    );
}

#[test]
fn u_is_accepted_and_ignored() {
    let (stdout, stderr, code) = cat_files("-u");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        concat!(
            "alpha\n",
            "\n",
            "bravo\tcharlie\n",
            "delta\n",
            "\n",
            "\n",
            "second\n",
        )
    );
}

#[test]
fn long_forms_and_unambiguous_abbreviation_match_short_forms() {
    let (short, _, _) = cat_files("-n");
    // `number' and `number-nonblank' share the prefix `number', so only
    // the exact form and abbreviations reaching past it are unambiguous
    // (`--numb'/`--numbe' are GNU ambiguity errors, see below).
    for args in ["--number", "--number-", "--number-n", "--number-no"] {
        let (stdout, stderr, code) = cat_files(args);
        assert_eq!(code, Some(0), "{args}");
        assert_eq!(stderr, b"", "{args}");
        assert!(stdout.starts_with(b"     1\talpha\n"), "{args}");
    }
    // `--number' itself numbers all lines exactly like -n.
    let (stdout, _, _) = cat_files("--number");
    assert_eq!(stdout, short);
    let (all_short, _, _) = cat_files("-A");
    let (stdout, _, _) = cat_files("--show-all");
    assert_eq!(stdout, all_short);
}

#[test]
fn unknown_short_option_is_rejected_with_gnu_wording() {
    // The original report: `cat -Z f.txt' ran as plain cat with rc 0.
    let (stdout, stderr, code) = cat_files("-Z");
    assert_eq!(stdout, b"");
    assert_eq!(code, Some(1));
    assert_eq!(
        String::from_utf8_lossy(&stderr),
        concat!(
            "cat: invalid option -- 'Z'\n",
            "Try 'cat --help' for more information.\n",
        )
    );
    // GNU getopt permutes argv: the option is still rejected AFTER the
    // operand position.
    let dir = std::env::temp_dir().join(format!(
        "rubash-i415z-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("f.txt"), b"x\n").expect("f.txt");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "cat f.txt -Z"])
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        concat!(
            "cat: invalid option -- 'Z'\n",
            "Try 'cat --help' for more information.\n",
        )
    );
}

#[test]
fn unrecognized_and_ambiguous_long_options_match_gnu() {
    let (stdout, stderr, code) = cat_files("--badlong");
    assert_eq!(stdout, b"");
    assert_eq!(code, Some(1));
    assert_eq!(
        String::from_utf8_lossy(&stderr),
        concat!(
            "cat: unrecognized option '--badlong'\n",
            "Try 'cat --help' for more information.\n",
        )
    );
    // GNU lists the possibilities in longopts table order.
    let (stdout, stderr, code) = cat_files("--numb");
    assert_eq!(stdout, b"");
    assert_eq!(code, Some(1));
    assert_eq!(
        String::from_utf8_lossy(&stderr),
        concat!(
            "cat: option '--numb' is ambiguous; possibilities: ",
            "'--number-nonblank' '--number'\n",
            "Try 'cat --help' for more information.\n",
        )
    );
}

#[test]
fn stdin_legs_apply_options() {
    // The direct (non-pipeline) path with inherited stdin.
    let (stdout, stderr, code) = run_rubash_stdin(&["-c", "cat -n"], Some(b"one\ntwo\n"));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        "     1\tone\n     2\ttwo\n"
    );

    let (stdout, _, code) = run_rubash_stdin(&["-c", "cat -A"], Some(b"a\tb\n"));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"a^Ib$\n");
}

#[test]
fn dash_operand_reads_stdin_then_files_in_order() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i415d-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("g.txt"), b"file\n").expect("g.txt");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "cat - g.txt"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(b"stdin\n")
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "stdin\nfile\n");
}

#[test]
fn double_dash_ends_option_parsing() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i415dd-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("-n"), b"x\n").expect("-n");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "cat -- -n"])
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "x\n");
}

#[test]
fn plain_concatenation_is_byte_identical_passthrough() {
    // The no-option fast path must not change bytes (unterminated tail
    // included) — the pre-fix behavior for plain cat stays.
    let (stdout, stderr, code) = cat_files("");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        concat!(
            "alpha\n",
            "\n",
            "bravo\tcharlie\n",
            "delta\n",
            "\n",
            "\n",
            "second\n",
        )
    );
}

#[test]
fn pipeline_stage_cat_still_numbers_through_the_real_binary() {
    // The pipeline stage keeps its real-subprocess route: `cat -n | cat'
    // was already correct before the fix and must stay correct.
    let (stdout, stderr, code) = run_rubash_stdin(&["-c", "cat -n | cat"], Some(b"one\ntwo\n"));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        "     1\tone\n     2\ttwo\n"
    );
}

// rubash#415 CI regression (run-coproc exit 124): GNU cat streams stdin
// as chunks arrive (cat.c byte-copy); the buffered drain deadlocked
// `coproc { cat - ; }` against an interactive writer. Pinned per the
// standing hang-test rule — the second read must see the SECOND write
// before any writer close.
#[test]
fn cat_dash_in_coproc_streams_interactively() {
    let script = r####"coproc REFLECT { cat - ; }
echo flop >&"${REFLECT[1]}"
exec {REFLECT[1]}>&-

"####;
    let _ = script;
    let probe =
        std::process::Command::new(std::env::current_exe().ok().map(|_| "").unwrap_or_default());
    let _ = probe;
    // The shape needs a live coproc + timed reads; run the engine binary
    // against the probe script with a hard timeout so a deadlock fails
    // the test instead of hanging it.
    let dir = std::env::temp_dir().join("rubash-issue415-coproc-cat");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("probe.sh");
    std::fs::write(
        &path,
        "coproc REFLECT { cat - ; }
echo flop >&\"${REFLECT[1]}\"
read -t 5 L <&\"${REFLECT[0]}\"
echo \"R=[$L]\"
kill $REFLECT_PID 2>/dev/null
wait 2>/dev/null
echo fin
",
    )
    .unwrap();
    let binary = crate_dir_binary();
    let out = std::process::Command::new(&binary)
        .arg(&path)
        .output()
        .expect("engine binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("R=[flop]"),
        "streamed echo missing: {stdout}"
    );
    assert!(stdout.contains("fin"), "script did not finish: {stdout}");
}

fn crate_dir_binary() -> std::path::PathBuf {
    // target/debug/rubash(.exe) relative to the crate manifest at build time.
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("debug");
    path.push(if cfg!(windows) {
        "rubash.exe"
    } else {
        "rubash"
    });
    path
}
