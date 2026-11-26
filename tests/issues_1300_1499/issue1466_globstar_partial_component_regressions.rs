//! rubash#1466 — with globstar on, `**` as PART of a path component
//! was passed through literally or misread as recursion; GNU treats a
//! partial-component `**` like `*` (no recursion). Whole-component
//! `**` forms were already correct.
//!
//! Probe: clusters/x2/repro/1466.sh (fixture tree built in a scratch
//! dir; the script itself lives outside the globbed directory).
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1466-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn globstar_partial_component_behaves_like_star() {
    let dir = tempdir("glob");
    let fix = dir.join("fix");
    std::fs::create_dir_all(fix.join("sub/deep")).expect("fixture dirs");
    for f in ["a.txt", "ab.txt", "b.md", "sp ace.txt"] {
        std::fs::write(fix.join(f), "").expect("fixture file");
    }
    std::fs::write(fix.join("sub/inner.txt"), "").expect("fixture file");
    std::fs::write(fix.join("sub/deep/z.txt"), "").expect("fixture file");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "shopt -s globstar\n\
         echo \"1:\"; echo **.txt\n\
         echo \"2:\"; echo **.md\n\
         echo \"3:\"; echo a**\n\
         echo \"4:\"; echo sub**/*.txt\n\
         echo \"5:\"; echo */**/*.txt\n\
         echo \"6:\"; echo **\n\
         echo \"7:\"; echo sub/**/*.txt\n",
    )
    .expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("../probe.sh")
        .current_dir(&fix)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        stdout,
        "1:\na.txt ab.txt sp ace.txt\n\
         2:\nb.md\n\
         3:\na.txt ab.txt\n\
         4:\nsub/inner.txt\n\
         5:\nsub/deep/z.txt sub/inner.txt\n\
         6:\na.txt ab.txt b.md sp ace.txt sub sub/deep sub/deep/z.txt sub/inner.txt\n\
         7:\nsub/deep/z.txt sub/inner.txt\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
