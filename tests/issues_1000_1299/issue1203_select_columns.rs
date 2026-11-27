//! rubash#1203 regressions: `select` menu layout ignored COLUMNS and
//! always printed a single column. The menu now follows GNU
//! execute_cmd.c:3375-3418 print_select_list — column-major rows sized by
//! COLS / max_elem_len, with execute_cmd.c:3356 indent's tab-run
//! separators (tabsize 8) — and general.c:1461 default_columns
//! (COLUMNS > 0, else console width, else 80).
//!
//! Expectations verified byte-for-byte against WSL GNU Bash 5.3.20:
//! `COLUMNS=40` and `COLUMNS=20` wrap to multi-column layouts while
//! `COLUMNS=80` on six short items stays single-column.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1203-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn select_menu_columns_40_is_column_major() {
    let script = "COLUMNS=40\nselect x in aaaa bbbb cccc dddd eeee ffff; do echo \"c=[$x]\"; break; done <<< \"1\"\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "c=[aaaa]\n");
    // GNU: two rows of three columns, tab between col 1 and col 2, spaces
    // between col 2 and col 3 (indent's next-stop rule).
    assert_eq!(
        stderr,
        "1) aaaa\t 3) cccc  5) eeee\n2) bbbb\t 4) dddd  6) ffff\n#? "
    );
}

#[test]
fn select_menu_columns_20_is_single_column() {
    // GNU at COLUMNS=20: max_elem_len = 4+1+2+2 = 9, cols = 2, rows = 3 —
    // a two-column layout.
    let script = "COLUMNS=20\nselect x in aaaa bbbb cccc dddd eeee ffff; do echo \"c=[$x]\"; break; done <<< \"1\"\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "c=[aaaa]\n");
    assert_eq!(
        stderr,
        "1) aaaa\t 4) dddd\n2) bbbb\t 5) eeee\n3) cccc\t 6) ffff\n#? "
    );
}

#[test]
fn select_menu_columns_80_stays_single_column() {
    let script = "COLUMNS=80\nselect x in aaaa bbbb cccc dddd eeee ffff; do echo \"c=[$x]\"; break; done <<< \"1\"\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "c=[aaaa]\n");
    assert_eq!(
        stderr,
        "1) aaaa\n2) bbbb\n3) cccc\n4) dddd\n5) eeee\n6) ffff\n#? "
    );
}
