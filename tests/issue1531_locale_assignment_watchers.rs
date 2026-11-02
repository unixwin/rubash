//! Issue rubash#1531: a PLAIN (unexported) assignment to a locale
//! variable had no effect — the locale sync only ran on the export
//! path, while the readers (glob collation, `[[ < ]]`, ctype, numeric
//! parsing) resolve the locale from the process environment.
//!
//! GNU ownership: variables.c's special_vars table (variables.c:5800)
//! binds LANG, LC_ALL, LC_COLLATE, LC_CTYPE, LC_MESSAGES, LC_NUMERIC
//! and LC_TIME to sv_locale (variables.c:6272), which fires on
//! ASSIGNMENT; locale.c set_locale_var (locale.c:185) / set_lang
//! (locale.c:313) then recompute each category as LC_ALL > LC_<cat> >
//! LANG (an empty value counts as unset). Rubash's readers consult the
//! process environment (src/locale.rs collate_locale_name /
//! locale_name / decimal_point), so the fix mirrors the whole watcher
//! family through the #916 assignment gate
//! (shell_assignment_needs_process_env): every assignment funnel —
//! plain, declare, local, temporary prefix, arithmetic — now syncs
//! the process copy, and the save/restore machinery resyncs it on
//! scope exit, so scoped assignments do not leak.
//!
//! Every expectation below is the literal GNU bash 5.3.20 output for
//! the same script file (env: LANG=C LC_COLLATE=C, de_DE.UTF-8 built
//! on the fly with localedef into a private LOCPATH, the issue1512
//! approach, so the test truly runs wherever localedef can build the
//! locale instead of silently skipping).
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

const NAMES: [&str; 9] = ["a", "A", "ä", "b", "O", "ö", "q", "z", "Z"];

const SCRIPT: &str = r#"
g() { echo "glob: $(echo *)"; }
c() { if [[ a < B ]]; then echo "cmp: T"; else echo "cmp: F"; fi; }
g; c
LC_COLLATE=de_DE.UTF-8; g; c
LC_COLLATE=C; g; c
LANG=de_DE.UTF-8
LC_COLLATE=C; g
LC_COLLATE=; g
unset LC_COLLATE; g
LC_ALL=de_DE.UTF-8; g; c
LC_ALL=; g; c
LC_COLLATE=C; g
f() { local LC_COLLATE=de_DE.UTF-8; g; }
f; g
LC_COLLATE=de_DE.UTF-8 g
g
x=$(LC_COLLATE=de_DE.UTF-8; echo ok); g
declare LC_COLLATE=de_DE.UTF-8; g
declare LC_COLLATE=C; g
v='héllo'
LC_CTYPE=C; echo "len-C: ${#v}"; echo "rm-C: ${v#h?}" | od -An -tx1 | tr -d ' \n'; echo
LC_CTYPE=de_DE.UTF-8; echo "len-de: ${#v}"; echo "rm-de: ${v#h?}"
LC_NUMERIC=de_DE.UTF-8; printf "num-de: %.2f\n" 1.5
LC_NUMERIC=C; printf "num-C: %.2f\n" 1.5
env | grep -c '^LC_COLLATE=' || true
export LC_COLLATE
env | grep -c '^LC_COLLATE=' || true
"#;

const EXPECTED: &str = "\
glob: A O Z a b q z ä ö
cmp: F
glob: a A ä b O ö q z Z
cmp: T
glob: A O Z a b q z ä ö
cmp: F
glob: A O Z a b q z ä ö
glob: a A ä b O ö q z Z
glob: a A ä b O ö q z Z
glob: a A ä b O ö q z Z
cmp: T
glob: a A ä b O ö q z Z
cmp: T
glob: A O Z a b q z ä ö
glob: a A ä b O ö q z Z
glob: A O Z a b q z ä ö
glob: a A ä b O ö q z Z
glob: A O Z a b q z ä ö
glob: A O Z a b q z ä ö
glob: a A ä b O ö q z Z
glob: A O Z a b q z ä ö
len-C: 6
726d2d433a20a96c6c6f0a
len-de: 5
rm-de: llo
num-de: 1,00
num-C: 1.50
0
1
";

/// Compile de_DE.UTF-8 once into a private LOCPATH directory (the
/// issue1512 approach). None only when the host lacks localedef or the
/// i18n sources entirely.
fn locale_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue1531-locales-{}", std::process::id()));
        let target = dir.join("de_DE.UTF-8");
        if !target.is_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let output = Command::new("localedef")
                .args(["-i", "de_DE", "-f", "UTF-8"])
                .arg(&target)
                .output()
                .ok()?;
            if !output.status.success() || !target.is_dir() {
                eprintln!(
                    "localedef de_DE.UTF-8 failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                return None;
            }
        }
        Some(dir)
    })
    .as_ref()
}

#[test]
fn issue1531_plain_locale_assignments_take_effect_immediately() {
    let Some(dir) = locale_dir() else {
        eprintln!("skip: de_DE.UTF-8 cannot be compiled on this host");
        return;
    };
    let fixture =
        std::env::temp_dir().join(format!("rubash-issue1531-fixture-{}", std::process::id()));
    std::fs::create_dir_all(&fixture).expect("create fixture dir");
    for name in NAMES {
        std::fs::write(fixture.join(name), []).expect("create fixture file");
    }
    let script_path =
        std::env::temp_dir().join(format!("rubash-issue1531-probe-{}.sh", std::process::id()));
    std::fs::write(&script_path, SCRIPT).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&fixture)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C")
        .env("LC_COLLATE", "C")
        .env("LOCPATH", dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&fixture);
    let _ = std::fs::remove_file(&script_path);
    assert!(
        output.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), EXPECTED);
}
