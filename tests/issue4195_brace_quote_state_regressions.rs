//! rubash#4195: a backslash inside a single-quoted span has NO
//! escaping power (GNU braces.c brace_gobbler pass_next applies only
//! in unquoted, double-quote and backquote state). Three faces of the
//! one misconception: brace expansion swallowed the closing quote of
//! `'\''` spans (C1-C5, F10; C6 turned that into an eval
//! double-execution), the protecting expansion walker resolved a
//! cooked word's span-data `\` + `$`/backtick as escape syntax and
//! killed the live substitution after it (F5-F8), and the compound
//! array store ran a second unquote pass over dequoted element text
//! and ate the span's backslash (F1-F3).
//!
//! The script below is the triage repro verbatim; its expected stdout
//! is the GNU Bash 5.3.20 tarball oracle's output, byte-for-byte
//! (F9's stderr wording is a separate face and not compared).

use std::process::Command;

const REPRO: &str = "echo \"== C1 ==\"\nprintf '<%s>' '\\'\"x\"'{a,b}'; echo\necho \"== C2 ==\"\nset -- '\\'\"x\"'{1..3}'; echo \"$#\"\necho \"== C3 ==\"\nprintf '<%s>' '\\'{a,b}; echo\necho \"== C4 ==\"\nprintf '<%s>' {'\\',b}; echo\necho \"== C5 ==\"\nprintf '<%s>' {\"\\\"\",b}; echo\necho \"== C6 eval ==\"\neval 'echo '\"'\"'\\'\"'\"'; echo RUN '\"'\"'{a,b}'\"'\"\necho \"== F1 array ==\"\na=('\\'x); echo \"${a[0]}\"\necho \"== F2 array ==\"\na=('q\\'\"y\"); echo \"${a[0]}\"\necho \"== F3 array ==\"\na=(\"\\\\\"x); echo \"${a[0]}\"\necho \"== F4 array-ok ==\"\na[0]='\\'x; echo \"${a[0]}\"\na=(\\\\x); echo \"${a[0]}\"\necho \"== F5 cmdsubst ==\"\necho '\\'$(echo x)\nset -- '\\'$(echo x); echo $#\necho \"== F6 backquote ==\"\necho '\\'`echo x`\necho \"== F7 assign ==\"\nx='\\'$(echo x); echo \"$x\"\necho \"== F8 dollar ==\"\nprintf '<%s>' '\\'$u$(echo x)z; echo\necho \"== F9 dollar-ok ==\"\n'\\y'$(echo x)\necho \"== F10 ansi-c ==\"\nprintf '<%s>' {$'\\'',b}; echo\necho \"== END ==\"\n";
const GNU_STDOUT: &str = "== C1 ==\n<\\x{a,b}>\n== C2 ==\n1\n== C3 ==\n<\\a><\\b>\n== C4 ==\n<\\><b>\n== C5 ==\n<\"><b>\n== C6 eval ==\n\\\nRUN {a,b}\n== F1 array ==\n\\x\n== F2 array ==\nq\\y\n== F3 array ==\n\\x\n== F4 array-ok ==\n\\x\n\\x\n== F5 cmdsubst ==\n\\x\n1\n== F6 backquote ==\n\\x\n== F7 assign ==\n\\x\n== F8 dollar ==\n<\\xz>\n== F9 dollar-ok ==\n== F10 ansi-c ==\n<'><b>\n== END ==\n";

#[test]
fn brace_quote_state_repro_matches_gnu_5320() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(REPRO)
        .output()
        .expect("run rubash");
    assert_eq!(
        output.stdout,
        GNU_STDOUT.as_bytes(),
        "rubash#4195: 16-group repro must match the GNU 5.3.20 baseline byte-for-byte"
    );
}
