//! rubash#3263 — trust condition (a) in `expand_redirect_target` must not
//! falsely trust field-splitting when `mark_literal_ifs_chars` placed
//! `IFS_GLUE` on another character in the same word while
//! `mark_quoted_ifs_from_raw` bailed out and left quoted IFS whitespace
//! unprotected.
//!
//! GNU semantics: `redir.c:298 redirection_expand` expands the target via
//! `expand_words_no_vars` (`subst.c:11229 expand_word_internal`,
//! `subst.c:12377 word_split`): quoted characters are CTLESC-protected by
//! `quote_string` (`subst.c:11867`) so `word_split` never splits or strips
//! quoted IFS whitespace.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Out {
    rc: i32,
    stderr: Vec<u8>,
}

fn run(script: &str, cwd: &PathBuf) -> Out {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash")
        .wait_with_output()
        .expect("wait rubash");
    Out {
        rc: output.status.code().unwrap_or(-1),
        stderr: output.stderr,
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3263-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn list_files(dir: &PathBuf) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read temp dir")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn quoted_leading_space_with_literal_ifs_glue_never_strips_space_on_bail() {
    let dir = temp_dir("leading-space-bail");
    let out = run(
        r#"IFS=" ,"
v=
echo ok > " a$[1+2]",$v
"#,
        &dir,
    );
    let files = list_files(&dir);
    let _ = fs::remove_dir_all(&dir);

    assert!(
        !files.iter().any(|name| name == "a3,"),
        "rubash#3263: false trust condition (a) stripped quoted leading space into 'a3,'; files={files:?}, rc={}, stderr={}",
        out.rc,
        String::from_utf8_lossy(&out.stderr)
    );
    if out.rc == 0 {
        assert_eq!(files, vec![" a3,".to_string()]);
    } else {
        assert_eq!(out.rc, 1);
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("ambiguous redirect"),
            "expected ambiguous redirect fallback, got: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(files.is_empty());
    }
}

#[test]
fn glued_literal_ifs_and_quoted_whitespace_with_matching_simulation() {
    let dir = temp_dir("glued-matching");

    let out_empty = run(
        r#"IFS=" ,"
v=
echo ok > " a b",$v
"#,
        &dir,
    );
    assert_eq!(
        out_empty.rc,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out_empty.stderr)
    );
    assert_eq!(list_files(&dir), vec![" a b,".to_string()]);

    let out_x = run(
        r#"IFS=" ,"
v=x
echo ok > " a b",$v
"#,
        &dir,
    );
    assert_eq!(
        out_x.rc,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out_x.stderr)
    );
    assert_eq!(
        list_files(&dir),
        vec![" a b,".to_string(), " a b,x".to_string()]
    );

    let out_comsub = run(
        r#"IFS=" ,"
echo ok > " c d",$(printf '')
"#,
        &dir,
    );
    assert_eq!(
        out_comsub.rc,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out_comsub.stderr)
    );
    assert_eq!(
        list_files(&dir),
        vec![
            " a b,".to_string(),
            " a b,x".to_string(),
            " c d,".to_string()
        ]
    );

    let out_ambig_space = run(
        r#"IFS=" ,"
v="x y"
echo ok > " e f",$v
"#,
        &dir,
    );
    assert_eq!(out_ambig_space.rc, 1);
    assert!(String::from_utf8_lossy(&out_ambig_space.stderr).contains("ambiguous redirect"));

    let out_ambig_comma = run(
        r#"IFS=" ,"
v="x,y"
echo ok > " e f",$v
"#,
        &dir,
    );
    assert_eq!(out_ambig_comma.rc, 1);
    assert!(String::from_utf8_lossy(&out_ambig_comma.stderr).contains("ambiguous redirect"));

    let _ = fs::remove_dir_all(&dir);
}
