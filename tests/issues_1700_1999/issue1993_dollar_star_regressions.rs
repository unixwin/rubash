//! rubash#1993 — `$*`/`${*}` with empty positional parameters and in
//! separator forms. With a set-empty IFS, Posix interp 888 dispatches
//! unquoted `$*` to the dollar_at word list (subst.c
//! string_list_pos_params:3047-3072): one word per parameter, and an
//! entirely-empty parameter contributes NO word. The command-word and
//! operator-word arms returned the positional window verbatim (empty
//! parameter became an empty word), the for-list path had no arm at
//! all and joined the parameters with the empty separator into ONE
//! word, and the double-quoted `${param+word}` alternate routed only
//! `$@` through the quoted re-lex, field-splitting `$*` and losing the
//! join. In a quoted context `$*` joins with IFS[0] into the single
//! field the affixes glue to (subst.c string_list_dollar_star).
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
        let dir =
            std::env::temp_dir().join(format!("rubash-i1993-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out_path = self.dir.join("out.txt");
        let err_path = self.dir.join("err.txt");
        let out_file = fs::File::create(&out_path).unwrap();
        let err_file = fs::File::create(&err_path).unwrap();
        let start = Instant::now();
        let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .unwrap();
        let rc = loop {
            match child.try_wait().unwrap() {
                Some(status) => break status.code().unwrap_or(-1),
                None => {
                    if start.elapsed() > Duration::from_secs(20) {
                        let _ = child.kill();
                        let _ = child.wait();
                        break -1;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        let read = |p: &PathBuf| {
            let mut s = String::new();
            fs::File::open(p).unwrap().read_to_string(&mut s).unwrap();
            s
        };
        (
            read(&out_path),
            read(&err_path).replace(&case.to_string_lossy().to_string(), "SH"),
            rc,
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue1993_unquoted_star_null_ifs_drops_empty_parameters() {
    let s = Scratch::new("cmdwords");
    // GNU: [a b][c] — the empty parameter contributes no word, the
    // "a b" parameter stays one word.
    let (out, err, rc) =
        s.run("set -- \"a b\" \"\" c\nIFS=\nprintf '[%s]' $*\necho\nprintf '[%s]' ${*}\necho\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[a b][c]\n[a b][c]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1993_for_list_star_null_ifs_iterates_per_parameter() {
    let s = Scratch::new("forlist");
    // GNU: the for list expands like a command word list
    // (expand_word_list_internal) — per-parameter iteration, empties
    // dropped; the old for path iterated once over the concatenation.
    let (out, err, rc) = s.run(
        "set -- \"a b\" \"\" c\nIFS=\nfor x in $*; do printf '[%s]' \"$x\"; done\necho\nfor x in ${*}; do printf '[%s]' \"$x\"; done\necho\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[a b][c]\n[a b][c]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1993_quoted_plus_word_star_joins_with_ifs_first_char() {
    let s = Scratch::new("plusword");
    // GNU: "${1+  $*  }" is ONE field — affixes plus the IFS[0] join
    // (a double space where the empty parameter joins); "${1+  $@  }"
    // is one field per parameter with the affixes glued to the ends.
    let (out, err, rc) = s.run(
        "set -- \"a b\" \"\" c\nunset IFS\nprintf '[%s]\\n' \"${1+  $*  }\"\nprintf '[%s]\\n' \"${1+  $@  }\"\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[  a b  c  ]\n[  a b]\n[]\n[c  ]\n");
    assert_eq!(err, "");
    // IFS=: — the join character is IFS[0].
    let (out, err, rc) = s.run("set -- \"a b\" \"\" c\nIFS=:\nprintf '[%s]\\n' \"${1+  $*  }\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[  a b::c  ]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1993_operator_word_star_null_ifs_drops_empty_parameters() {
    let s = Scratch::new("opword");
    // GNU: ${var-$*} under IFS= goes through W_SPLITSPACE over the
    // escaped elements — the split consumes the join spaces, so an
    // empty positional contributes no word (same for a leading one).
    let (out, err, rc) = s.run(
        "unset var\nset -- \"a b\" \"\" c\nIFS=\nprintf '[%s]' ${var-$*}\necho\nset -- \"\" x\nprintf '[%s]' ${var-$*}\necho\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[a b][c]\n[x]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1993_nonempty_ifs_forms_unchanged() {
    let s = Scratch::new("ifscolon");
    // GNU agreement guards: IFS=: unquoted $* iterates the
    // per-parameter words (an empty parameter survives as an empty
    // word — only the NULL-IFS dispatch drops it); quoted "$*" is
    // the colon join.
    let (out, err, rc) = s.run(
        "set -- \"a b\" \"\" c\nIFS=:\nfor x in $*; do printf '[%s]' \"$x\"; done\necho\nprintf '[%s]\\n' \"$*\"\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[a b][][c]\n[a b::c]\n");
    assert_eq!(err, "");
}
