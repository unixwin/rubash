use std::process::{Command, Stdio};

/// Golden leak assertions (governance 3.1/3.2): transport marker codepoints
/// must never appear in stdout, `declare -p`, or xtrace output. C0 carrier
/// bytes are indistinguishable from legitimate user data at the byte level,
/// so the checks target the unambiguous classes: PUA codepoints (the
/// registry zone, U+E000..=U+E4FF — markers.rs currently tops out at
/// U+E401, UTF-8 lead byte 0xEE) and the named protocol strings
/// (`__RUBASH_*`).
///
/// rubash#2885 E6: this file carries one cell per output channel of the
/// #2879 sweep's §2.1 matrix. The per-marker dimension is NOT hand-written
/// here — `tests/marker_registry_roundtrips.rs`'s registry-driven test iterates the
/// MARKERS table through the core shapes, so a new marker gets cells
/// automatically; this file's job is the per-CHANNEL dimension. The
/// interactive prompt / `read -p` / history cells live in the PTY sibling
/// (`tests/marker_leak_golden_pty.rs`). The one §2.1 channel with no cell
/// anywhere is the debugger/JSON face (sweep Q2: it runs zero decoders and
/// no user-level repro was ever attempted; driving it needs a DAP session,
/// not a shell invocation) — recorded here so the gap is explicit, not
/// silent.

fn run(args: &[&str]) -> Vec<u8> {
    run_full(args, &[]).0
}

fn run_full(args: &[&str], envs: &[(&str, &str)]) -> (Vec<u8>, Vec<u8>) {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .envs(envs.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    (out.stdout, out.stderr)
}

/// Spawn with a sanitized environment (no inherited vars) plus a minimal
/// PATH/HOME/LC_ALL. The compgen cell asserts on the shell variable
/// namespace, and `cargo test` injects build-stamp vars
/// (`RUBASH_GIT_STAMP`, `RUBASH_GIT_DIRTY_SUFFIX`, from build.rs
/// `cargo:rustc-env`) into the test process env — rubash correctly
/// imports them as shell variables (GNU does the same for any imported
/// env var), so an inherited env would make the cell count the harness,
/// not the product.
fn run_clean_env(args: &[&str]) -> Vec<u8> {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/tmp")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    out.stdout
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
    assert_eq!(
        out,
        format!("a{e002}b").as_bytes(),
        "concatenation a${{v}}b"
    );
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
    assert_eq!(
        out,
        format!("<{e002}>\n").as_bytes(),
        "unquoted U+E002 field"
    );
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
    // NOTE: `case "$v" in *"$v"*)` is not a cell here — the case
    // pattern's quoted segment was a separate blind consumer of the
    // pair, filed as rubash#3014; its cells live in
    // `user_e002_quoted_null_char_matches_case_patterns` below.
}

/// rubash#3014: the case pattern's QUOTED segment is a pattern-side
/// consumer of the #2882 class. GNU expands each case pattern with
/// expand_word_leave_quoted and runs it through
/// quote_string_for_globbing (execute_cmd.c execute_case_command,
/// pattern loop): quote_string (pathexp.c) CTLESC-protects each
/// character of a quoted span — and the E400+E002 transport pair is
/// ONE character (the typed U+E002), so it takes ONE guard as a unit,
/// exactly like the raw-byte marker pair. rubash's
/// escape_case_pattern_literal guarded the pair's halves separately
/// (CTLESC E400 CTLESC E002), so the matcher's flattening could no
/// longer read the pair as one data char and every quoted-segment
/// shape NOMATCHed. Subject side, unquoted pattern form and the
/// `[[ == ]]` forms were already correct (controls below). All cells
/// probe-verified against GNU bash 5.3.20 under LC_ALL=C.UTF-8.
#[test]
fn user_e002_quoted_null_char_matches_case_patterns() {
    let run_utf8 = |script: &str| -> String {
        let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .args(["-c", script])
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rubash");
        let out = child.wait_with_output().expect("wait rubash").stdout;
        String::from_utf8_lossy(&out).into_owned()
    };
    let script = concat!(
        "v=$'\\uE002'\n",
        "case \"$v\" in \"$v\") echo \"exact-quoted: MATCH\";; *) echo \"exact-quoted: NOMATCH\";; esac\n",
        "case \"$v\" in *\"$v\"*) echo \"glob-both: MATCH\";; *) echo \"glob-both: NOMATCH\";; esac\n",
        "case $'\\uE002' in *\"$v\"*) echo \"lit-subj-glob-pat: MATCH\";; *) echo \"lit-subj-glob-pat: NOMATCH\";; esac\n",
        "w=\"a${v}b\"\n",
        "case \"$w\" in *\"$v\"*) echo \"concat-subj: MATCH\";; *) echo \"concat-subj: NOMATCH\";; esac\n",
        "case \"$v\" in *$'\\uE002'*) echo \"ansi-pat: MATCH\";; *) echo \"ansi-pat: NOMATCH\";; esac\n",
        "case \"$v\" in *\"$(printf %s \"$v\")\"*) echo \"comsub-pat: MATCH\";; *) echo \"comsub-pat: NOMATCH\";; esac\n",
        "p=\"$v\"\n",
        "case \"$v\" in *\"$p\"*) echo \"var-pat: MATCH\";; *) echo \"var-pat: NOMATCH\";; esac\n",
        "case \"$v\" in \"$p\") echo \"var-pat-exact: MATCH\";; *) echo \"var-pat-exact: NOMATCH\";; esac\n",
        // Controls: unquoted pattern form and the sanity NOMATCH were
        // already correct on the broken tree, as were the `[[ == ]]`
        // forms — pin them so this fix cannot regress them.
        "case \"$v\" in *$v*) echo \"unquoted-pat: MATCH\";; *) echo \"unquoted-pat: NOMATCH\";; esac\n",
        "case \"ZZZ\" in *\"$v\"*) echo \"sanity: MATCH\";; *) echo \"sanity: NOMATCH\";; esac\n",
        "if [[ \"$v\" == \"$v\" ]]; then echo \"cond-exact: MATCH\"; else echo \"cond-exact: NOMATCH\"; fi\n",
        "if [[ \"$v\" == *\"$v\"* ]]; then echo \"cond-glob: MATCH\"; else echo \"cond-glob: NOMATCH\"; fi\n",
    );
    let expected = concat!(
        "exact-quoted: MATCH\n",
        "glob-both: MATCH\n",
        "lit-subj-glob-pat: MATCH\n",
        "concat-subj: MATCH\n",
        "ansi-pat: MATCH\n",
        "comsub-pat: MATCH\n",
        "var-pat: MATCH\n",
        "var-pat-exact: MATCH\n",
        "unquoted-pat: MATCH\n",
        "sanity: NOMATCH\n",
        "cond-exact: MATCH\n",
        "cond-glob: MATCH\n",
    );
    assert_eq!(run_utf8(script), expected, "case pattern U+E002 cells");
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

// ======================================================================
// rubash#2885 E6 — one cell per §2.1 channel. Expectations were collected
// byte-for-byte from GNU bash 5.3.20 with script-file probes (the sweep
// battery shapes); where a channel's output legitimately carries user
// data codepoints, the cell asserts the literal round-trip instead of
// the leak scan, exactly like the xtrace PUA-word cell above.
// ======================================================================

/// `set` listing — the setattr `quote_listing_value` rendering of the
/// same declare formatting. The array shape is byte-identical to GNU;
/// the raw-C0 scalar shape decodes to the raw bytes (asserted) — GNU
/// additionally wraps it in $'...' quoting, a quoting-style divergence
/// recorded in the #2885 lane REPORT (not a marker leak: no pair, no
/// PUA glyph reaches the listing).
#[test]
fn set_listing_carries_no_markers() {
    let out = run(&["-c", "arr=($'a\\x11b' 'c d'); set | grep '^arr='"]);
    assert_eq!(
        out,
        "arr=([0]=$'a\\021b' [1]=\"c d\")\n".as_bytes(),
        "set listing must render storage bytes in declare form: {out:?}"
    );
    let out = run(&["-c", "v=$'a\\x11b\\x1fc'; set | grep '^v='"]);
    assert_clean(&out, "set listing (C0 scalar)");
    assert_eq!(
        out,
        "v=$'a\\021b\\037c'\n".as_bytes(),
        "set listing must render C0 scalars ansic-quoted like GNU (rubash#3017/#3044), not raw bytes or transport pairs: {out:?}"
    );
}

/// PS4 template — sweep battery cell c21. Expansion-introduced raw bytes
/// (comsub result, stored variable) must decode AFTER prompt expansion
/// (PR #2878): the trace prefix carries the real ESC byte, never the
/// E000+E01C pair.
#[test]
fn ps4_template_decodes_expansion_introduced_markers() {
    let (_, err) = run_full(
        &[
            "-c",
            concat!(
                "PS4='X$(printf \"P\\033Q\") '; set -x; true; set +x; ",
                "x=$'\\x1b'; PS4='P${x}Q '; set -x; true; set +x"
            ),
        ],
        &[],
    );
    assert_clean(&err, "PS4 trace");
    assert!(
        err.windows(4).any(|w| w == b"XP\x1bQ"),
        "comsub ESC must reach the PS4 prefix as a raw byte: {err:?}"
    );
    assert!(
        err.windows(4).any(|w| w == b"P\x1bQ "),
        "stored ESC must reach the PS4 prefix as a raw byte: {err:?}"
    );
}

/// Prompts via the public API (`Session::expand_prompt`) — the host
/// rendering path. A stored ESC, a comsub ESC, a TAB carried by a value
/// (the sweep's open Q1: the ANSI_C_IFS_GUARD must not survive into the
/// rendered prompt), and a typed PUA value all render as their real
/// bytes/chars. (The typed-PUA leg is a known-broken inverted pin,
/// rubash#3038; the other legs assert the correct rendering.)
#[test]
fn prompts_via_public_api_carry_no_markers() {
    use rubash::api::{RunOptions, Session};
    let mut session = Session::builder().build().expect("session builds");
    session
        .run(
            "esc=$(printf 'A\\033B'); tab=$'A\\tB'; pua=$'\\uE314'",
            &RunOptions::default(),
        )
        .expect("seed values run");
    assert_eq!(session.expand_prompt("<$esc>"), "<A\u{1b}B>");
    assert_eq!(session.expand_prompt("$(printf 'P\\033Q')"), "P\u{1b}Q");
    let rendered = session.expand_prompt("<$tab>");
    assert_eq!(rendered, "<A\tB>", "Q1: TAB value must render as TAB");
    assert!(
        !rendered.contains('\u{E401}'),
        "Q1: ANSI_C_IFS_GUARD must not survive into a rendered prompt: {rendered:?}"
    );
    // KNOWN BROKEN (rubash#3038): expand_prompt does not decode the
    // E400 pair for PARAM_WORD_BACKSLASH_GUARD — it renders the pair
    // where GNU renders the raw char. Pinned inverted (registry-test
    // precedent): a fix flips this red.
    assert_eq!(
        session.expand_prompt("<$pua>"),
        "<\u{E400}\u{E314}>",
        "rubash#3038: pua leg must render the raw char once fixed"
    );
}

/// Function bodies (`declare -f`, ast print) — the
/// `fold_literal_char_escapes` channel.
#[test]
fn declare_f_carries_no_markers() {
    let out = run(&["-c", "f() { echo 'a$b' \"$1\"; x=$'\\x11'; }; declare -f f"]);
    assert_clean(&out, "declare -f");
    assert!(
        String::from_utf8_lossy(&out).contains("echo 'a$b'"),
        "function body must print in source form: {out:?}"
    );
}

/// Diagnostics (stderr) — bad-substitution display decodes through the
/// sanctioned decoder; the command-not-found diagnostic and the
/// redirect-target filename fold the E400 literal-char escape pair
/// back to the raw char at the name boundary (rubash#3040; GNU
/// execute_cmd.c:5909 printable_filename over the raw word,
/// redir.c:298 redirection_expand's verbatim filename). All three
/// legs are byte-verified against GNU 5.3.20 on this tree.
#[test]
fn diagnostics_carry_no_markers() {
    // Bad substitution (battery c22 shape).
    let (out, err) = run_full(&["-c", "v=$'\\uE314'; echo ${v@Z}; echo after"], &[]);
    assert_clean(&err, "bad-substitution diagnostic");
    // GNU aborts the -c script on a bad substitution (rc 127, no
    // "after"): byte-verified against GNU bash 5.3.20 on this tree
    // (probes/g4_diag_{gnu,rub}.out/.err). The diagnostic must name
    // the error; the leak scan above already pins it marker-free.
    assert!(
        !String::from_utf8_lossy(&out).contains("after"),
        "bad substitution aborts like GNU, so 'after' must not print: {out:?}"
    );
    assert!(
        String::from_utf8_lossy(&err).contains("bad substitution"),
        "diagnostic must name the error: {err:?}"
    );
    // Command not found (battery c33 shape).
    let (_, err) = run_full(&["-c", "v=$'\\uE314'; \"$v\""], &[]);
    let text = String::from_utf8_lossy(&err);
    // rubash#3040: the diagnostic names the raw char, not the E400
    // literal-char escape pair (GNU: raw char bytes EE 8C 94, -c mode).
    assert!(
        text.contains('\u{E314}') && !text.contains('\u{E400}'),
        "rubash#3040: diagnostic must name the raw char: {text:?}"
    );
    // Redirect target filename.
    // rubash#3040: the redirect creates the file under the raw-char
    // name (GNU: bytes EE 8C 94), not the E400 pair. The redirect
    // itself succeeds silently on both shells, so the pin reads the
    // created filename, not a diagnostic.
    let dir = std::env::temp_dir().join(format!("rubash-golden-redir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let script = format!(
        "cd {} && v=$'\\uE314' && {{ echo x > \"$v\"; }} 2>&1",
        dir.display()
    );
    let _ = run_full(&["-c", &script], &[]);
    let names: Vec<String> = std::fs::read_dir(&dir)
        .expect("read redir dir")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        names,
        vec!["\u{E314}".to_string()],
        "rubash#3040: redirect target must be the raw-char name: {names:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// External boundaries against real child binaries (sweep F1 shapes,
/// fixed by #2881): an exported value, an argv value, and an `env`-
/// command value carrying a typed registry-zone char reach
/// /usr/bin/env and /usr/bin/printf as the RAW char — the E400 escape
/// introducer must not cross the process boundary.
#[test]
#[cfg(unix)]
fn external_boundaries_carry_raw_user_bytes() {
    let out = run(&["-c", "v=$'\\uE314'; /usr/bin/printf '%s' \"$v\""]);
    assert_eq!(
        out,
        "\u{E314}".as_bytes(),
        "argv must carry the raw char to a real child binary: {out:?}"
    );
    let out = run(&["-c", "v=$'\\uE314'; export v; /usr/bin/env"]);
    let line = out
        .split(|&b| b == b'\n')
        .find(|l| l.starts_with(b"v="))
        .expect("exported v reaches the child env");
    assert_eq!(
        line,
        "v=\u{E314}".as_bytes(),
        "child env must carry the raw char: {line:?}"
    );
    let out = run(&["-c", "v=$'\\uE314'; env W=\"$v\" /usr/bin/env"]);
    let line = out
        .split(|&b| b == b'\n')
        .find(|l| l.starts_with(b"W="))
        .expect("env-command W reaches the child env");
    assert_eq!(
        line,
        "W=\u{E314}".as_bytes(),
        "env-command env must carry the raw char: {line:?}"
    );
}

/// Environment import (sweep F6 shapes, fixed by #2884): a parent-set
/// value enters through the import funnel's entry encoding, so
/// `declare -p` renders it exactly like a typed value — a bare
/// registry-zone char is data (not `v=""`), and a raw ESC value gets
/// GNU's $'A\EB' quoting instead of a raw byte inside plain quotes.
#[test]
fn env_import_decodes_like_a_typed_value() {
    let (out, _) = run_full(&["-c", "declare -p IMPORTED"], &[("IMPORTED", "\u{E314}")]);
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains('\u{E314}') && !text.contains("IMPORTED=\"\""),
        "imported registry-zone char must render as data: {text:?}"
    );
    let (out, _) = run_full(&["-c", "declare -p IMPORTED"], &[("IMPORTED", "A\u{1b}B")]);
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("$'A\\EB'"),
        "imported ESC value must take GNU's ANSI-C quoting in declare -p: {text:?}"
    );
}

/// Completion / compgen — a zero-decoder channel, safe only by
/// construction; the cell enrolls it. Candidates render as data, and
/// compgen must not disclose the internal __RUBASH_* namespace
/// (sweep §2.1, incident #1045 was name disclosure). Runs under a
/// sanitized env (see run_clean_env): build-stamp vars from the test
/// harness must not count as disclosed internal names.
#[test]
fn compgen_carries_no_markers() {
    let out = run_clean_env(&[
        "-c",
        "compgen -W 'a$b c*d e\\f' | sort; compgen -A variable | grep -c RUBASH",
    ]);
    assert_clean(&out, "compgen");
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("c*d") && text.trim_end().ends_with('0'),
        "candidates render as data and no internal names are disclosed: {text:?}"
    );
}

/// Jobs builtins / job text — a zero-decoder channel (sweep battery
/// c31 was PASS-equivalent modulo PID digits). The cell leak-scans the
/// `jobs` renderings of a marker-carrying command; PIDs make byte
/// equality impossible, so the assertion is the leak scan itself.
/// The printf job's own stdout is user data, not a jobs rendering: it
/// prints the raw U+E314 the user asked for, so its output goes to
/// /dev/null and the scan sees only the `jobs` renderings (which print
/// the command text in source spelling, GNU-identical).
#[test]
fn jobs_text_carries_no_markers() {
    let out = run(&[
        "-c",
        "printf '%s' $'\\uE314' >/dev/null & sleep 0.2 & jobs; jobs -l; wait; echo done",
    ]);
    assert_clean(&out, "jobs");
    assert!(
        String::from_utf8_lossy(&out).contains("done"),
        "job table must complete: {out:?}"
    );
}
