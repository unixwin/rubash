use std::process::{Command, Stdio};

/// Golden leak assertions (governance 3.1/3.2): transport marker codepoints
/// must never appear in stdout, `declare -p`, or xtrace output. C0 carrier
/// bytes are indistinguishable from legitimate user data at the byte level,
/// so the checks target the unambiguous classes: PUA codepoints (the
/// registry zone, U+E000..=U+E4FF — markers.rs currently tops out at
/// U+E401, UTF-8 lead byte 0xEE) and the named protocol strings
/// (`__RUBASH_*`).

fn run(args: &[&str]) -> Vec<u8> {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child.wait_with_output().expect("wait rubash").stdout
}

fn assert_no_pua_marker(out: &[u8], ctx: &str) {
    // U+E000..=U+EFFF encodes as EE 80 80 .. EE BF BF. Every registered
    // marker lives in that plane (lead byte 0xEE — the same predicate
    // markers.rs `contains_word_marker_bytes` uses), so scan the whole
    // plane: any private-use char from it reaching stdout is a leak signal.
    // rubash#727: this used to test 0xEF (U+F000..=U+FFFF), which saw no
    // registered marker at all and only ever fired on user data.
    for w in out.windows(3) {
        if w[0] == 0xEE && (0x80..=0xBF).contains(&w[1]) {
            let cp = 0xE000 + (((w[1] & 0x3F) as u32) << 6) + ((w[2] & 0x3F) as u32);
            panic!("{ctx}: PUA codepoint U+{cp:04X} leaked into output: {out:?}");
        }
    }
}

fn assert_no_named_marker(out: &[u8], ctx: &str) {
    let text = String::from_utf8_lossy(out);
    assert!(
        !text.contains("__RUBASH_"),
        "{ctx}: named protocol string leaked into output: {text:?}"
    );
}

fn assert_clean(out: &[u8], ctx: &str) {
    assert_no_pua_marker(out, ctx);
    assert_no_named_marker(out, ctx);
}

/// Marker-heavy word shapes through echo: escaped glob chars, quoted
/// words, ANSI-C decoded carrier bytes, storage words, subscripts.
#[test]
fn echo_stdout_carries_no_markers() {
    let out = run(&[
        "-c",
        concat!(
            "v='a\\$b'\"'\"'c'\"'\"'`d`$e'; echo \"$v\"; ",
            "echo a\\*b \"c d\" 'e$f' $'g\\x11h\\x14i\\x1fj'; ",
            "arr=(\"$v\" $'\\x03\\x05\\x10'); echo \"${arr[@]}\"; ",
            "echo ${v/a\\$b/R} ${#v} ${v:1:3}; declare -A A=([k]=$v); echo \"${A[k]}\""
        ),
    ]);
    assert_clean(&out, "echo");
}

/// `declare -p` must render storage bytes, never transport sentinels.
#[test]
fn declare_p_carries_no_markers() {
    let out = run(&[
        "-c",
        concat!(
            "v='x\\y'\"'\"'z'\"'\"'; declare -p v; ",
            "arr=($'a\\x11b' $'c\\x1fd' 'e$f'); declare -p arr; ",
            "declare -A A=([$'k\\x14']=$'v\\x18'); declare -p A; ",
            "compound=(a=1 b='two words'); declare -p compound"
        ),
    ]);
    assert_clean(&out, "declare -p");
}

/// xtrace echoes expanded words — every marker must be decoded first.
#[test]
fn xtrace_carries_no_markers() {
    let out = run(&[
        "-c",
        "set -x; v='a\\$b'; echo \"$v\" 'q q' $'\\x11\\x1f'; arr=(1 2); echo \"${arr[@]}\"; set +x",
    ]);
    let child_err = {
        let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .args([
                "-c",
                "set -x; v='a\\$b'; echo \"$v\" 'q q' $'\\x11\\x1f'; arr=(1 2); echo \"${arr[@]}\"; set +x",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rubash");
        child.wait_with_output().expect("wait rubash").stderr
    };
    assert_clean(&out, "xtrace stdout");
    assert_clean(&child_err, "xtrace stderr");

    // PUA user data must trace as the literal char, not its E400
    // literal-char escape (the xtrace verbatim-word arm leaked it once).
    // This output legitimately CONTAINS U+E314 — it is the user's own data —
    // so the PUA-plane leak assertion cannot apply here (rubash#727 made it
    // visible; before the fix it was blind). Assert the literal round-trip
    // instead: exactly one char per traced word, no E400 escape, no named
    // protocol string.
    let child_err = {
        let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .args(["-c", "set -x; v=$'\\uE314Z'; echo \"$v\"; set +x"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rubash");
        child.wait_with_output().expect("wait rubash").stderr
    };
    let traced = String::from_utf8_lossy(&child_err);
    assert!(
        !traced.contains('\u{E400}'),
        "PUA word traced through the E400 literal-char escape: {traced:?}"
    );
    assert_eq!(
        traced.matches('\u{E314}').count(),
        2,
        "PUA data must trace literally once per traced word: {traced:?}"
    );
    assert_no_named_marker(&child_err, "xtrace stderr (PUA word)");
}

/// Reparse boundary: eval / alias / command-substitution bodies that
/// carry marker bytes must not leak them when reparsed.
#[test]
fn reparse_boundary_carries_no_markers() {
    let out = run(&[
        "-c",
        concat!(
            "shopt -s expand_aliases; ",
            "v='a\\$b'; eval 'echo \"$v\"'; ",
            "alias p='echo \"$v\"'; p; ",
            "t=$(echo \"$v\"); echo \"t=$t\"; ",
            "eval \"echo $'\\x11X\\x1fY'\""
        ),
    ]);
    assert_clean(&out, "reparse");
}

/// User-supplied registry-zone chars ($'\uE314') are data: they must
/// round-trip as literal chars, never trigger marker semantics.
#[test]
fn user_pua_chars_roundtrip_as_data() {
    let out = run(&[
        "-c",
        "v=$'\\uE314X\\uE301Y'; printf '%s' \"$v\"; printf '\\n'",
    ]);
    // U+E314 + 'X' + U+E301 + 'Y' — the guard/ DATA_* codepoints must be
    // emitted literally, not decoded as markers (which would eat X/Y).
    assert_eq!(
        out,
        "\u{E314}X\u{E301}Y\n".as_bytes(),
        "user PUA chars must round-trip literally"
    );
}

/// rubash#2706: QUOTED_EXPANSION_SITE (U+E31B) is inserted into words
/// by the pre-expansion marking pass and consumed by the expansion
/// walkers; it must never reach output even on the mixed-quote shapes
/// that produce it internally — and a user-typed U+E31B (E400-escaped
/// at entry) is data and must round-trip literally.
#[test]
fn quoted_expansion_site_marker_never_leaks() {
    let out = run(&[
        "-c",
        "a='1 2'; b='3 4'; printf '<%s>\\n' $a\"$b\" \"$a\"$b $a\"$(echo 'p q')\"; \
         for x in $a\"$b\"; do echo \"[$x]\"; done",
    ]);
    assert_clean(&out, "quoted expansion sites");
    let out = run(&["-c", "echo $'\\uE31B'"]);
    assert_eq!(
        out,
        "\u{E31B}\n".as_bytes(),
        "user U+E31B must round-trip literally"
    );
}

/// printf %b / %q output paths.
#[test]
fn printf_paths_carry_no_markers() {
    let out = run(&[
        "-c",
        "v='a\\$b'; printf '%q\\n' \"$v\"; printf '%s\\n' \"${v//\\\\/X}\"; printf '%b\\n' 'a\\x41b'",
    ]);
    assert_clean(&out, "printf");
}

/// rubash#2882 (PUA sweep F2): U+E002 is QUOTED_NULL_MARKER. A typed
/// `$'\uE002'` arrives E400-escaped (push_literal_char) and is user
/// DATA, but blind consumers that `cow_replace`/`contains` the bare
/// marker ate it: `printf %s "$v"` emitted a stray U+E400 alone,
/// `echo "$v"` emitted nothing, `"a${v}b"` dropped the char. GNU
/// protects a user CTLNUL with CTLESC at read time (parse.y
/// got_character) so dequote passes (subst.c remove_quoted_nulls)
/// strip only the BARE marker — the char survives every consumer.
/// Storage was always fine (`declare -p`, `${#v}` correct on the
/// broken tree too — they are the control cells). The correct output
/// legitimately CONTAINS U+E002, so these cells assert exact bytes
/// instead of the no-PUA predicate. Run under LC_ALL=C.UTF-8: in the
/// POSIX locale GNU itself keeps the literal `\uE002` spelling.
#[test]
fn user_e002_quoted_null_char_roundtrips_as_data() {
    let e002 = "\u{E002}";
    let run_utf8 = |script: &str| -> Vec<u8> {
        let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .args(["-c", script])
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rubash");
        child.wait_with_output().expect("wait rubash").stdout
    };
    // printf %s of the quoted value: the char, nothing else (the
    // broken tree emitted a stray U+E400 here).
    let out = run_utf8("v=$'\\uE002'; printf '%s' \"$v\"");
    assert_eq!(out, e002.as_bytes(), "printf %s of quoted U+E002 value");
    // Braced form, same expectation.
    let out = run_utf8("v=$'\\uE002'; printf '%s' \"${v}\"");
    assert_eq!(out, e002.as_bytes(), "printf %s of braced U+E002 value");
    // echo: char + newline (the broken tree emitted just the newline).
    let out = run_utf8("v=$'\\uE002'; echo \"$v\"");
    assert_eq!(out, format!("{e002}\n").as_bytes(), "echo of U+E002 value");
    // Concatenation inside a double-quoted word keeps the char.
    let out = run_utf8("v=$'\\uE002'; w=\"a${v}b\"; printf '%s' \"$w\"");
    assert_eq!(out, format!("a{e002}b").as_bytes(), "concatenation a${{v}}b");
    let out = run_utf8("v=$'\\uE002'; echo \"x${v}y\"");
    assert_eq!(out, format!("x{e002}y\n").as_bytes(), "echo concatenation");
    // Assignment round-trip: the stored value is the char.
    let out = run_utf8("v=$'\\uE002'; u=$v; printf '%s' \"$u\"");
    assert_eq!(out, e002.as_bytes(), "assignment round-trip of U+E002");
    // Two typed chars stay two chars.
    let out = run_utf8("x=$'\\uE002'$'\\uE002'; printf '%s' \"$x\"");
    assert_eq!(out, format!("{e002}{e002}").as_bytes(), "double U+E002");
    // Unquoted expansion: one field carrying the char.
    let out = run_utf8("v=$'\\uE002'; printf '<%s>\\n' $v");
    assert_eq!(out, format!("<{e002}>\n").as_bytes(), "unquoted U+E002 field");
    // Alternate words, quoted and unquoted (field-split gate shapes).
    let out = run_utf8("v=$'\\uE002'; e=; printf '%s' \"${e:-$v}\"");
    assert_eq!(out, e002.as_bytes(), "quoted alternate with U+E002");
    let out = run_utf8("v=$'\\uE002'; e=; printf '<%s>\\n' ${e:-a${v}b}");
    assert_eq!(
        out,
        format!("<a{e002}b>\n").as_bytes(),
        "unquoted alternate with U+E002"
    );
    // External argv boundary: the child sees the raw char.
    let out = run_utf8("v=$'\\uE002'; /usr/bin/printf '%s' \"$v\"");
    assert_eq!(out, e002.as_bytes(), "external printf argv U+E002");
    // Controls: storage views were already correct — pin them so the
    // fix cannot regress them. `declare -p` renders the raw char and
    // must not show the E400 escape introducer; length is 1.
    let out = run_utf8("v=$'\\uE002'; declare -p v");
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains(e002) && !text.contains('\u{E400}'),
        "declare -p must render the raw U+E002 char: {text:?}"
    );
    let out = run_utf8("v=$'\\uE002'; echo \"len=${#v}\"");
    assert_eq!(out, b"len=1\n".as_slice(), "length of U+E002 value");
    // Conditional match against itself (the conditional.rs consumer is
    // already E400-aware; pin the agreement).
    let out = run_utf8("v=$'\\uE002'; case \"$v\" in *\"$v\"*) echo MATCH;; *) echo NOMATCH;; esac");
    assert_eq!(out, b"MATCH\n".as_slice(), "case match of U+E002 value");
}

/// rubash#727: the assertion must actually see the registry's codepoints
/// (U+E000-based, UTF-8 lead byte 0xEE). Before the fix it tested 0xEF and
/// therefore could not fire on any registered marker.
#[test]
#[should_panic(expected = "U+E301")]
fn pua_assertion_sees_a_registered_marker() {
    assert_no_pua_marker("\u{E301}".as_bytes(), "selftest");
}

/// rubash#727: and it must NOT fire on U+F000..=U+FFFF (lead byte 0xEF) —
/// a different PUA block the registry never uses. The old predicate fired
/// only on this false-positive path.
#[test]
fn pua_assertion_ignores_the_f000_block() {
    assert_no_pua_marker("\u{F000}".as_bytes(), "selftest");
}
