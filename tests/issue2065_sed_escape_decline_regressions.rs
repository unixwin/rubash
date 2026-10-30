//! rubash#2065 — the in-process sed bridge literalized the GNU BRE
//! escape family (`\b \B \w \W \s \S \< \>`) as ordinary letters,
//! silently answering wrong in both directions (over- and
//! under-matching) on the simple-command face, where the bridge
//! replaces a resolvable real sed instead of yielding to it. The
//! escape dispatch is now a whitelist: escapes the engine does not
//! replicate byte-for-byte decline the whole fast path so the real
//! sed answers. The same flip covers the replacement side (GNU case
//! folding `\U \L \u \l \E`, `\a \f \r \v`, `\0`, a `\N` reference
//! past the pattern's group count), backslash escapes inside
//! bracket expressions, partially-understood multi-line scripts,
//! and the `0d` address GNU rejects.
//!
//! Expectations verified against GNU sed (script files, dual-run).
//! Clue reported downstream by SpookyWaste (dsh-bash-native patch
//! 0003); the fix is rubash's own.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Fixture {
    dir: PathBuf,
    empty_bin: PathBuf,
    fake_bin: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("issue2065-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create fixture dir");
        let empty_bin = dir.join("emptybin");
        fs::create_dir_all(&empty_bin).expect("create empty bin dir");
        // A PATH body for `sed` that only prints a marker: on the
        // simple-command face the bridge replaces a resolvable body
        // for shapes it answers itself, so the marker proves a shape
        // fell through (declined) versus being answered internally.
        let fake_bin = dir.join("fakebin");
        fs::create_dir_all(&fake_bin).expect("create fake bin dir");
        let fake_sed = fake_bin.join("sed");
        fs::write(&fake_sed, "#!/bin/sh\necho FAKE-SED-RAN\nexit 0\n").expect("write fake sed");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&fake_sed, fs::Permissions::from_mode(0o755))
                .expect("chmod fake sed");
        }
        Fixture {
            dir,
            empty_bin,
            fake_bin,
        }
    }

    /// Ambient PATH: a real sed answers whatever the bridge declines.
    fn run(&self, input: &str, command: &str) -> (String, String, Option<i32>) {
        self.run_with_path(input, command, None)
    }

    /// Empty PATH: nothing resolves, so the bridge never engages
    /// and a declined (or unanswerable) shape is an honest 127.
    fn run_no_sed(&self, input: &str, command: &str) -> (String, String, Option<i32>) {
        self.run_with_path(input, command, Some(&self.empty_bin))
    }

    /// Marker-sed PATH: proves whether the bridge answered a shape
    /// itself (real output) or fell through (the marker runs).
    fn run_fake(&self, input: &str, command: &str) -> (String, String, Option<i32>) {
        self.run_with_path(input, command, Some(&self.fake_bin))
    }

    fn run_with_path(
        &self,
        input: &str,
        command: &str,
        path: Option<&PathBuf>,
    ) -> (String, String, Option<i32>) {
        fs::write(self.dir.join("in.txt"), input).expect("write input");
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
        cmd.arg("-c").arg(command).current_dir(&self.dir);
        if let Some(path) = path {
            cmd.env("PATH", path);
        }
        let output = cmd.output().expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn sed_cmd(script: &str) -> String {
    format!("sed '{script}' < in.txt")
}

/// With a real GNU sed resolvable, every declined shape must come
/// back with GNU's own answer (the bridge used to shadow it).
#[test]
fn gnu_escape_family_falls_through_to_real_sed() {
    let fx = Fixture::new("family");
    let cases: &[(&str, &str, &str)] = &[
        // (script, input, GNU stdout) — \b in both directions first:
        // the pre-fix bridge matched the literal letters "ab" here.
        ("s/a\\b/X/", "ab\n", "ab\n"),
        ("s/a\\b/X/", "a b\n", "X b\n"),
        ("s/a\\B/X/", "ab\n", "Xb\n"),
        ("s/a\\B/X/", "a b\n", "a b\n"),
        ("s/\\w/X/", "a b\n", "X b\n"),
        ("s/\\W/X/", "a b\n", "aXb\n"),
        ("s/\\s/X/", "asb\n", "asb\n"),
        ("s/\\s/X/", "a b\n", "aXb\n"),
        ("s/\\S/X/", "a b\n", "X b\n"),
        ("s/\\<a/X/", "ba\n", "ba\n"),
        ("s/\\<a/X/", " ab\n", " Xb\n"),
        ("s/b\\>/X/", "ab\n", "aX\n"),
        // Buffer anchors and character escapes decline the same way.
        ("s/\\`a/X/", "ab\n", "Xb\n"),
        ("s/\\a/X/", "a\u{7}b\n", "aXb\n"),
        // Undefined letters decline too (whitelist, not a denylist).
        ("s/\\d/X/", "adb\n", "aXb\n"),
        // Inside bracket expressions: `\t` is replicated (tab class
        // in the supported set below), the irregular family declines.
        ("s/[\\b]/X/", "abb\n", "aXb\n"),
        ("s/[\\a]/X/", "a\u{7}b\n", "aXb\n"),
    ];
    for (script, input, expected) in cases {
        let (stdout, stderr, code) = fx.run(input, &sed_cmd(script));
        assert_eq!(&stdout, expected, "script {script} input {input:?}");
        assert_eq!(stderr, "", "script {script}");
        assert_eq!(code, Some(0), "script {script}");
    }
    // `\'` (buffer-end anchor) needs double-quote wrapping: a lone
    // quote character cannot appear inside the single-quoted script.
    let (stdout, _, code) = fx.run("ab\n", "sed \"s/b\\\\'/X/\" < in.txt");
    assert_eq!(stdout, "aX\n");
    assert_eq!(code, Some(0));
}

/// With no sed on PATH at all, a declined shape must surface as an
/// honest 127 — never as a fabricated internal answer.
#[test]
fn declined_shapes_are_honest_127_without_real_sed() {
    let fx = Fixture::new("decline");
    let scripts = [
        "s/a\\b/X/",
        "s/\\w/X/",
        "s/\\s/X/",
        "s/\\<a/X/",
        "s/\\d/X/",
        "s/\\a/X/",
        "s/[\\b]/X/",
        "s/\\(a\\)/\\U\\1/",
        "s/[z-a]/X/",
    ];
    for script in scripts {
        let (stdout, _, code) = fx.run_no_sed("ab\n", &sed_cmd(script));
        assert_eq!(
            code,
            Some(127),
            "script {script} must decline, got {stdout:?}"
        );
        assert_eq!(stdout, "", "script {script}");
    }
}

/// Shapes inside the whitelist must still be answered by the engine
/// itself — proven against a marker fake sed: if the bridge fell
/// through, the marker would run instead of the real answer.
#[test]
fn supported_shapes_still_answered_internally() {
    let fx = Fixture::new("supported");
    let cases: &[(&str, &str, &str)] = &[
        ("s/a\\+/X/", "aaa\n", "X\n"),
        ("s/a\\?b/X/", "b\n", "X\n"),
        ("s/a\\?b/X/", "ab\n", "X\n"),
        ("s/\\*/X/", "a*b\n", "aXb\n"),
        ("s/\\./X/", "a.b\n", "aXb\n"),
        // `\\.` is an escaped backslash plus any-char: it must match
        // the backslash in "a\z" and must NOT match the dot in "a.b"
        // (an internal pattern rewrite used to flip both).
        ("s/\\\\./X/", "a\\z\n", "aX\n"),
        ("s/\\\\./X/", "a.b\n", "a.b\n"),
        ("s/[[:space:]]/_/", "a b\n", "a_b\n"),
        ("s/\\t/X/", "a\tb\n", "aXb\n"),
        ("s/[\\t]/X/", "a\tb\n", "aXb\n"),
        ("s/\\(ab\\)\\1/X/", "abab\n", "X\n"),
        // A group whose body matches empty still completes its
        // piece: `\(a*\)b` matches "b" with an empty capture.
        ("s/\\(a*\\)b/[\\1]/", "b\n", "[]\n"),
        ("s/\\(a*\\)b/[\\1]/", "aab\n", "[aa]\n"),
        ("s/\\(a\\)/[\\1:&]/", "a\n", "[a:a]\n"),
        ("s/a/b\\/c/", "a\n", "b/c\n"),
        ("s/^a/X/", "abc\n", "Xbc\n"),
        ("s/c$/X/", "abc\n", "abX\n"),
    ];
    for (script, input, expected) in cases {
        let (stdout, stderr, code) = fx.run_fake(input, &sed_cmd(script));
        assert_eq!(&stdout, expected, "script {script} input {input:?}");
        assert_eq!(stderr, "", "script {script}");
        assert_eq!(code, Some(0), "script {script}");
    }
    // The `Nd` delete face is part of the same processor.
    let (stdout, _, code) = fx.run_fake("l1\nl2\nl3\n", "sed -e '2d' < in.txt");
    assert_eq!(stdout, "l1\nl3\n");
    assert_eq!(code, Some(0));
}

/// And the declined side of the same discriminator: with only the
/// marker sed on PATH, a declined shape runs the marker — visible
/// proof the bridge stood down instead of answering.
#[test]
fn declined_shapes_run_the_path_body() {
    let fx = Fixture::new("fallthrough");
    for script in ["s/a\\b/X/", "s/\\w/X/", "s/a\\{2,3\\}/X/"] {
        let (stdout, _, code) = fx.run_fake("ab\n", &sed_cmd(script));
        assert_eq!(stdout, "FAKE-SED-RAN\n", "script {script}");
        assert_eq!(code, Some(0), "script {script}");
    }
}

/// `\{m,n\}` and `\|` already declined before this fix; they must
/// not regress now that the dispatch is a whitelist.
#[test]
fn interval_and_alternation_still_decline() {
    let fx = Fixture::new("known");
    let (stdout, _, code) = fx.run("aaaa\n", &sed_cmd("s/a\\{2,3\\}/X/"));
    assert_eq!(stdout, "Xa\n");
    assert_eq!(code, Some(0));
    let (stdout, _, code) = fx.run("y\n", &sed_cmd("s/x\\|y/X/"));
    assert_eq!(stdout, "X\n");
    assert_eq!(code, Some(0));
}

/// Replacement-side escapes the engine does not replicate decline
/// to the real sed, whose answers (including its errors) are GNU's.
#[test]
fn replacement_side_declines() {
    let fx = Fixture::new("replacement");
    let cases: &[(&str, &str, &str)] = &[
        ("s/\\(a\\)\\(b\\)/\\U\\1\\2/", "abc\n", "ABc\n"),
        ("s/\\(A\\)\\(B\\)/\\L\\1\\2/", "ABc\n", "abc\n"),
        ("s/b/\\a/", "ab\n", "a\u{7}\n"),
        // GNU expands `\0` in the replacement to the whole match.
        ("s/b/\\0/", "ab\n", "ab\n"),
    ];
    for (script, input, expected) in cases {
        let (stdout, stderr, code) = fx.run(input, &sed_cmd(script));
        assert_eq!(&stdout, expected, "script {script} input {input:?}");
        assert_eq!(stderr, "", "script {script}");
        assert_eq!(code, Some(0), "script {script}");
    }
    // A `\N` reference past the pattern's group count is a GNU
    // error (exit 1), not an empty expansion.
    for script in ["s/b/\\9/", "s/\\(a\\)/\\2/"] {
        let (stdout, _, code) = fx.run("ab\n", &sed_cmd(script));
        assert_eq!(code, Some(1), "script {script}");
        assert_eq!(stdout, "", "script {script}");
    }
}

/// Script-form faces from the same census: GNU rejects `0d`, and a
/// multi-line script is understood whole or not at all — a line the
/// engine cannot parse must not be silently dropped.
#[test]
fn script_form_faces() {
    let fx = Fixture::new("forms");
    let (stdout, _, code) = fx.run("l1\nl2\n", "sed -e '0d' < in.txt");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");

    let (stdout, _, code) = fx.run("abc\n", "sed -e 's/a/X/\ny/b/Y/' < in.txt");
    assert_eq!(stdout, "XYc\n");
    assert_eq!(code, Some(0));

    let (stdout, _, code) = fx.run("l1\nl2\n", "sed -e 's/l/L/\n1d' < in.txt");
    assert_eq!(stdout, "L2\n");
    assert_eq!(code, Some(0));

    // Addressed commands were never answered internally; the real
    // sed's results stand as the no-regression baseline.
    let (stdout, _, code) = fx.run("xa\nxb\n", "sed -e '2s/x/y/' < in.txt");
    assert_eq!(stdout, "xa\nyb\n");
    assert_eq!(code, Some(0));
    let (stdout, _, code) = fx.run("l1\nl2\n", "sed -e '$d' < in.txt");
    assert_eq!(stdout, "l1\n");
    assert_eq!(code, Some(0));
}
