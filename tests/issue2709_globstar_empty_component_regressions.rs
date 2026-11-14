//! rubash#2709 — empty pathname components next to `**` are dropped
//! by the globstar walk, in both of its decisions:
//!
//! 1. Arity: `**//*.txt` collapses to `**/*.txt`, so top-level files
//!    match. GNU keeps the empty component as a real level: the `**`
//!    collapse in glob.c (glob.c:1188-1210) folds only a sequence of
//!    `**` components into one `**`; a `**` followed by an empty
//!    component and a filename is not folded, so at least one real
//!    directory level is required.
//! 2. Result names: `sub//**//e.txt` must print `sub//deep/e.txt` —
//!    the literal directory prefix is preserved verbatim when result
//!    names are joined (glob.c glob_dir_to_array, glob.c:1058/1087),
//!    and `sub//**` prints `sub//`, `sub//c.txt`, ... Empty
//!    components away from `**` (`sub//c.txt`, `x//y//z.txt`) are
//!    already preserved by both shells — those cells are locks.
//!
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2709-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Fixture tree (script runs with the fixture as cwd):
/// a.txt, b.txt, e.txt, sub/c.txt, sub/deep/e.txt, x/y/z.txt
fn run_in_tree(tag: &str, body: &str) -> (String, bool) {
    let dir = tempdir(tag);
    let fix = dir.join("fix");
    std::fs::create_dir_all(fix.join("sub/deep")).expect("fixture dirs");
    std::fs::create_dir_all(fix.join("x/y")).expect("fixture dirs");
    for f in ["a.txt", "b.txt", "e.txt"] {
        std::fs::write(fix.join(f), "").expect("fixture file");
    }
    std::fs::write(fix.join("sub/c.txt"), "").expect("fixture file");
    std::fs::write(fix.join("sub/deep/e.txt"), "").expect("fixture file");
    std::fs::write(fix.join("x/y/z.txt"), "").expect("fixture file");
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("../probe.sh")
        .current_dir(&fix)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let ok = output.status.success();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, ok)
}

#[test]
fn globstar_double_slash_requires_a_directory_level() {
    let (stdout, ok) = run_in_tree(
        "arity",
        "shopt -s globstar\n\
         echo '== A =='; printf '%s\\n' **//*.txt\n\
         echo '== I =='; printf '%s\\n' **//e.txt\n\
         echo '== J =='; printf '%s\\n' **/e.txt\n",
    );
    assert!(ok);
    assert_eq!(
        stdout,
        "== A ==\n\
         sub/c.txt\n\
         sub/deep/e.txt\n\
         x/y/z.txt\n\
         == I ==\n\
         sub/deep/e.txt\n\
         == J ==\n\
         e.txt\n\
         sub/deep/e.txt\n"
    );
}

#[test]
fn globstar_empty_components_survive_in_result_names() {
    let (stdout, ok) = run_in_tree(
        "names",
        "shopt -s globstar\n\
         echo '== B =='; printf '%s\\n' sub//**//e.txt\n\
         echo '== G =='; printf '%s\\n' sub//**/*.txt\n\
         echo '== H =='; printf '%s\\n' sub//**\n",
    );
    assert!(ok);
    assert_eq!(
        stdout,
        "== B ==\n\
         sub//deep/e.txt\n\
         == G ==\n\
         sub//c.txt\n\
         sub//deep/e.txt\n\
         == H ==\n\
         sub//\n\
         sub//c.txt\n\
         sub//deep\n\
         sub//deep/e.txt\n"
    );
}

#[test]
fn empty_components_away_from_globstar_are_preserved() {
    let (stdout, ok) = run_in_tree(
        "locks",
        "shopt -s globstar\n\
         echo '== C =='; printf '%s\\n' sub//c.txt\n\
         echo '== D =='; printf '%s\\n' ./*.txt\n\
         echo '== E =='; printf '%s\\n' **/*.txt\n\
         echo '== K =='; printf '%s\\n' x//y//z.txt\n\
         echo '== L =='; printf '%s\\n' **//\n",
    );
    assert!(ok);
    assert_eq!(
        stdout,
        "== C ==\n\
         sub//c.txt\n\
         == D ==\n\
         ./a.txt\n\
         ./b.txt\n\
         ./e.txt\n\
         == E ==\n\
         a.txt\n\
         b.txt\n\
         e.txt\n\
         sub/c.txt\n\
         sub/deep/e.txt\n\
         x/y/z.txt\n\
         == K ==\n\
         x//y//z.txt\n\
         == L ==\n\
         sub/\n\
         sub/deep/\n\
         x/\n\
         x/y/\n"
    );
}
