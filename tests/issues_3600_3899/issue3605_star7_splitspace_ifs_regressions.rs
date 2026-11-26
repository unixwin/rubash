//! rubash#3605 — mid-word `${IFS=}`/`${IFS:=-}` binds and the W_SPLITSPACE
//! model for unquoted `$*`/`${*}` (dollar-star7.sub).
//!
//! GNU evaluates a word's fragments strictly left-to-right
//! (subst.c:11229 expand_word_internal): a `${IFS=}` bind takes effect at
//! the fragment's position — earlier fragments render against the pre-bind
//! IFS, later ones against the bound value. Pieces produced by an unquoted
//! `$*`/`${*}` while IFS is set-but-null carry W_SPLITSPACE
//! (subst.c:10665-10668), which marks the assembled word to split on " "
//! regardless of a later mid-word IFS bind (subst.c:11535 →
//! subst.c:12103-12107), and a quoted-site expansion span keeps its
//! whitespace CTLESC-protected. The assignment pre-scan hoists binds ahead
//! of the render pass, so IFS-dependent earlier fragments are frozen at
//! their pre-bind value (the rubash#1169 mechanism, extended to
//! `${*}`/`${@}`/`[*]`/`[@]` list expansions).
//!
//! All expectations measured against GNU bash 5.3.20.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "rubash-i3605s7-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> String {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out_path = self.dir.join("out.txt");
        let out_file = fs::File::create(&out_path).unwrap();
        let start = Instant::now();
        let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if start.elapsed() > Duration::from_secs(20) {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut s = String::new();
        fs::File::open(&out_path)
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        s
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The dollar-star7 puzzle word: the first `${*}` renders under the unset
/// IFS (plain join, split-eligible pieces), the second under the set-null
/// IFS (W_SPLITSPACE pieces, element whitespace protected), the quoted
/// `"${*}"` under the freshly bound `-`. The final split runs on " ".
#[test]
fn issue3605_midword_ifs_bind_orders_positionals() {
    let s = Scratch::new("ordering");
    let out = s.run(
        "set -f -- a 'b c' d\n\
         unset -v IFS\n\
         printf '<%s> ' ${*}${IFS=}${*}${IFS:=-}\"${*}\"\n\
         echo\n",
    );
    assert_eq!(out, "<a> <b> <c> <da> <b c> <d-a-b c-d> \n");
}

/// Same word over single-char positionals: piece 1 splits, piece 2's
/// first element glues to piece 1's last, the `-` bound mid-word does
/// NOT split (the whole word carries W_SPLITSPACE).
#[test]
fn issue3605_splitspace_beats_later_ifs_bind() {
    let s = Scratch::new("splitspace");
    let out = s.run(
        "set -f -- a b c\n\
         unset -v IFS\n\
         printf '<%s> ' ${*}${IFS=}${*}${IFS:=-}\"${*}\"\n\
         echo\n",
    );
    assert_eq!(out, "<a> <b> <ca> <b> <c-a-b-c> \n");
}

/// A `${IFS=}` bind mid-word leaves IFS null for the word_split that
/// runs after expansion, so the joined word stays one field.
#[test]
fn issue3605_ifs_bind_suppresses_later_split() {
    let s = Scratch::new("nosplit");
    let out = s.run(
        "set -f -- a 'b c' d\n\
         unset -v IFS\n\
         printf '<%s> ' ${*}${IFS=}\n\
         echo\n\
         unset -v IFS\n\
         printf '<%s> ' ${*}X${IFS=}\n\
         echo\n",
    );
    assert_eq!(out, "<a b c d> \n<a b c dX> \n");
}

/// The `${x#$*}`/`${x%$*}` pattern contexts take the dollar_star concat
/// (expand_no_split_dollar_star, subst.c:237/3048/4518) — no
/// W_SPLITSPACE join leaks into the pattern (dollar-star4/star5).
#[test]
fn issue3605_pattern_removal_uses_concat_join() {
    let s = Scratch::new("pattern");
    let out = s.run(
        "set -- a b\n\
         IFS=\n\
         a=abcd\n\
         echo \"${a#$*}\"\n\
         echo \"${a%$*}\"\n",
    );
    assert_eq!(out, "cd\nabcd\n");
}
