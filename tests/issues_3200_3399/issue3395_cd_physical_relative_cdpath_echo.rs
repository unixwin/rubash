//! rubash#3395 — `cd -P` through a CDPATH unit announces the
//! sh_makepath join of the unit and the operand verbatim.
//!
//! GNU builtins/cd.def:360 builds `temp = sh_makepath (path, dirname,
//! MP_DOTILDE)` (lib/sh/makepath.c:66: unit + `/` unless the unit
//! already ends in one + operand, as spelled), and cd.def:370 prints
//! `no_symlinks ? temp : the_current_working_directory`. So under -P a
//! RELATIVE unit announces a relative path (`sub/target`), `./` and
//! `..` components stay, and an absolute unit keeps its `./` too;
//! under -L (and with no flag) the announcement is the new logical
//! cwd, absolute. rubash absolutized the -P announcement through the
//! logical destination (the #1329 fix only replaced the physical
//! directory). $PWD was already correct in every shape.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes (scratch cwd normalized to B below).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3395-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["sub/target", "a", "b/target", "nest/x/target", "t"] {
        fs::create_dir_all(dir.join(sub)).expect("create scratch subdir");
    }
    dir
}

/// Run `script` as a script FILE from inside the scratch dir; the
/// script prints its own cwd-relative results.
fn run_script(dir: &Path, script: &str) -> String {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(dir);
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const PRELUDE: &str = r#"base=$PWD
show() { o=$("$@"; echo "rc=$? pwd=${PWD#"$base"}"); echo "[${o//"$base"/B}]"; }
"#;

#[test]
fn issue3395_relative_unit_announces_relative_join() {
    let dir = scratch("rel");
    let out = run_script(
        &dir,
        &format!(
            "{PRELUDE}{}",
            r#"CDPATH=sub show cd -P target
CDPATH=a:b show cd -P target
CDPATH=nest/x show cd -P target
CDPATH=sub/ show cd -P target
CDPATH=:sub show cd -P target
CDPATH=sub show cd -P target/
"#
        ),
    );
    assert_eq!(
        out,
        "[sub/target\nrc=0 pwd=/sub/target]\n\
         [b/target\nrc=0 pwd=/b/target]\n\
         [nest/x/target\nrc=0 pwd=/nest/x/target]\n\
         [sub/target\nrc=0 pwd=/sub/target]\n\
         [sub/target\nrc=0 pwd=/sub/target]\n\
         [sub/target/\nrc=0 pwd=/sub/target]\n"
    );
}

#[test]
fn issue3395_dot_components_stay_verbatim_under_physical() {
    let dir = scratch("dots");
    let out = run_script(
        &dir,
        &format!(
            "{PRELUDE}{}",
            r#"CDPATH=./sub show cd -P target
CDPATH=t/../sub show cd -P target
CDPATH=$base/./sub show cd -P target
"#
        ),
    );
    assert_eq!(
        out,
        "[./sub/target\nrc=0 pwd=/sub/target]\n\
         [t/../sub/target\nrc=0 pwd=/sub/target]\n\
         [B/./sub/target\nrc=0 pwd=/sub/target]\n"
    );
}

#[test]
fn issue3395_logical_controls_announce_absolute_cwd() {
    let dir = scratch("logical");
    let out = run_script(
        &dir,
        &format!(
            "{PRELUDE}{}",
            r#"CDPATH=sub show cd -L target
CDPATH=sub show cd target
CDPATH=./sub show cd target
show cd -P sub/target
"#
        ),
    );
    assert_eq!(
        out,
        "[B/sub/target\nrc=0 pwd=/sub/target]\n\
         [B/sub/target\nrc=0 pwd=/sub/target]\n\
         [B/sub/target\nrc=0 pwd=/sub/target]\n\
         [rc=0 pwd=/sub/target]\n"
    );
}

#[test]
fn issue3395_oldpwd_after_physical_cdpath_hit() {
    let dir = scratch("dash");
    let out = run_script(
        &dir,
        &format!(
            "{PRELUDE}{}",
            r#"o=$(CDPATH=sub; cd -P target; cd - >/dev/null; cd -); echo "[${o//"$base"/B}]"
"#
        ),
    );
    assert_eq!(out, "[sub/target\nB/sub/target]\n");
}

#[cfg(unix)]
#[test]
fn issue3395_symlink_hit_announces_link_spelling() {
    let dir = scratch("link");
    fs::create_dir_all(dir.join("b/real")).expect("create real dir");
    std::os::unix::fs::symlink("real", dir.join("b/link")).expect("symlink");
    let out = run_script(
        &dir,
        &format!(
            "{PRELUDE}{}",
            r#"CDPATH=b show cd -P link
CDPATH=b show cd -P link/
CDPATH=b show cd link
"#
        ),
    );
    assert_eq!(
        out,
        "[b/link\nrc=0 pwd=/b/real]\n\
         [b/link/\nrc=0 pwd=/b/real]\n\
         [B/b/link\nrc=0 pwd=/b/link]\n"
    );
}
