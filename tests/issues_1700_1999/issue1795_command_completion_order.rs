//! Issue rubash#1795 regression: `compgen -A command` emitted its
//! candidate groups in the wrong order (builtins and keywords ahead of
//! aliases and functions).
//!
//! GNU bashline.c:2005 command_word_completion_function maps over the
//! sources in a fixed order: case 0 aliases, case 1 shell reserved
//! words (word_token_alist order), case 2 function names, case 3 shell
//! builtins (shell_builtins table order), then the PATH walk. Within a
//! group the table order is kept, so for the word `e` the keyword
//! group is `else elif esac` (else precedes elif in the token table)
//! and the builtin group starts `echo enable eval ...` (echo precedes
//! enable in the builtins table).
//!
//! Oracle: GNU bash 5.3.20 — `compgen -A command e` prints
//! else/elif/esac, then echo/enable/eval/exec/exit/export, then the
//! PATH hits; with `alias ezz` and function `efn` defined, ezz leads
//! and efn sits between the keywords and the builtins.

use std::process::Command;

fn rubash_empty_path(script: &str) -> (String, Option<i32>) {
    // An empty PATH directory isolates the non-PATH groups: no PATH
    // candidate can interleave with the table-order assertions.
    let dir = std::env::temp_dir().join(format!("rubash-1795-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("PATH", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

#[test]
fn command_candidates_follow_gnu_source_group_order() {
    let (stdout, rc) = rubash_empty_path("compgen -A command e");
    let got: Vec<String> = stdout.lines().map(str::to_string).collect();
    assert_eq!(rc, Some(0));
    assert_eq!(
        got,
        [
            "else", "elif", "esac", // keywords, word_token_alist order
            "echo", "enable", "eval", "exec", "exit",
            "export", // builtins, shell_builtins table order
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>(),
        "-A command groups must be keywords then builtins in table order"
    );
}

#[test]
fn command_candidates_place_aliases_first_and_functions_before_builtins() {
    let (stdout, rc) = rubash_empty_path("alias ezz='echo hi'; efn() { :; }; compgen -A command e");
    let got: Vec<String> = stdout.lines().map(str::to_string).collect();
    assert_eq!(rc, Some(0));
    assert_eq!(
        got,
        [
            "ezz", // alias group first
            "else", "elif", "esac", // keywords
            "efn",  // functions after keywords, before builtins
            "echo", "enable", "eval", "exec", "exit", "export",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>(),
        "alias must lead, function must sit between keywords and builtins"
    );
}

#[test]
fn command_candidates_keep_path_duplicates_after_the_tables() {
    // rubash#1089's no-dedup rule is untouched by the reorder: a name
    // that is both a builtin and a PATH file appears once per source,
    // with the PATH hits after the table groups.
    let dir = std::env::temp_dir().join(format!("rubash-1795-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let fake = dir.join("echo");
    std::fs::write(&fake, "#!/bin/sh\nexit 0\n").expect("fake echo");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&fake).expect("stat").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).expect("chmod");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("compgen -A command echo")
        .env("PATH", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let got: Vec<String> = stdout.lines().map(str::to_string).collect();
    assert_eq!(
        got,
        vec!["echo".to_string(), "echo".to_string()],
        "builtin echo first, PATH echo second, neither deduplicated"
    );
}
