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
    assert!(
        out.starts_with(b"v=a\x11b\x1fc"),
        "set listing must carry the decoded raw bytes, not transport pairs: {out:?}"
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
/// sanctioned decoder; the command-not-found and redirection-error
/// paths do NOT fold them yet: the command-not-found diagnostic and
/// the redirect-target filename both carry the E400 pair instead of
/// the raw char. Those two legs are known-broken inverted pins
/// (rubash#3040); only the bad-substitution leg is byte-verified
/// against GNU 5.3.20 on this tree.
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
    // KNOWN BROKEN (rubash#3040): the diagnostic names the E400 pair
    // instead of the raw char (GNU: raw char, -c mode). Pinned
    // inverted (registry-test precedent): a fix flips this red.
    assert!(
        text.contains('\u{E400}'),
        "rubash#3040: diagnostic must name the raw char once fixed: {text:?}"
    );
    // Redirect target filename.
    // KNOWN BROKEN (rubash#3040): the redirect creates the file with
    // the E400 pair in its name (GNU: raw char). The redirect itself
    // succeeds silently on both shells, so the pin reads the created
    // filename, not a diagnostic. Pinned inverted: a fix flips red.
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
        vec!["\u{E400}\u{E314}".to_string()],
        "rubash#3040: redirect target must be the raw-char name once fixed: {names:?}"
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
