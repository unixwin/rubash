//! Issue rubash#2705: `printf '%(...)T'` ignored Olson TZ names
//! (America/New_York, Asia/Tokyo, ...) and silently rendered UTC.
//!
//! GNU contract (bash-5.3 builtins/printf.def, the `%(` case): the
//! format is rendered from `localtime(&secs)` after `sv_tz("TZ")`, so
//! the C library's tzset rules own the TZ string: a leading `:` is
//! stripped, the name is looked up as a TZif file under TZDIR (or the
//! system zoneinfo directory) FIRST — even for POSIX-looking names —
//! and only when no file matches is the string parsed as a POSIX TZ
//! rule. A string that matches neither renders at offset zero under
//! its leading alphabetic name (`TZ=Not/AZone` -> %Z "Not", +0000).
//! Expected outputs below are byte-verified against the GNU bash
//! 5.3.20 oracle on this host's tzdata.
//!
//! Bounded skip: the TZif grids need the system tzdata files named in
//! each test; when a file is absent from every zoneinfo directory the
//! test returns early (the environment lacks the database — that is
//! not a product failure). The fallback and POSIX grids need no files.

use std::path::PathBuf;
use std::process::Command;

fn rubash_tz(tz: &str, epoch: &str, extra_env: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg("printf '%(%Y-%m-%d %H:%M:%S %Z %z)T\\n' ".to_string() + epoch)
        .env("TZ", tz);
    for (name, value) in extra_env {
        cmd.env(name, value);
    }
    let output = cmd.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_tz(tz: &str, epoch: &str, expected: &str) {
    assert_tz_env(tz, epoch, expected, &[]);
}

fn assert_tz_env(tz: &str, epoch: &str, expected: &str, extra_env: &[(&str, &str)]) {
    let (stdout, stderr, code) = rubash_tz(tz, epoch, extra_env);
    assert_eq!(code, Some(0), "TZ={tz} stderr: {stderr}");
    assert!(stderr.is_empty(), "TZ={tz} stderr: {stderr}");
    assert_eq!(stdout, expected, "TZ={tz} epoch {epoch}");
}

/// True when `name` exists as a file in a zoneinfo directory the
/// product searches (TZDIR is test-controlled, so only the compiled
/// defaults count here).
fn zoneinfo_has(name: &str) -> bool {
    [
        "/usr/share/zoneinfo",
        "/usr/share/lib/zoneinfo",
        "/system/usr/share/zoneinfo",
    ]
    .iter()
    .any(|dir| PathBuf::from(dir).join(name).is_file())
}

#[test]
fn olson_new_york_winter_and_summer() {
    if !zoneinfo_has("America/New_York") {
        return;
    }
    assert_tz("America/New_York", "0", "1969-12-31 19:00:00 EST -0500\n");
    assert_tz(
        "America/New_York",
        "1625097600",
        "2021-06-30 20:00:00 EDT -0400\n",
    );
    // Before the 1970 epoch the file's transition table still owns the
    // rendering (1899: EST, the 1883 LMT->EST transition is long past).
    assert_tz(
        "America/New_York",
        "-2208988800",
        "1899-12-31 19:00:00 EST -0500\n",
    );
}

#[test]
fn olson_colon_form_and_absolute_path() {
    if !zoneinfo_has("America/New_York") {
        return;
    }
    assert_tz(":America/New_York", "0", "1969-12-31 19:00:00 EST -0500\n");
    assert_tz(
        "/usr/share/zoneinfo/America/New_York",
        "0",
        "1969-12-31 19:00:00 EST -0500\n",
    );
}

#[test]
fn olson_tokyo_london_sydney() {
    if zoneinfo_has("Asia/Tokyo") {
        assert_tz(
            "Asia/Tokyo",
            "1000000000",
            "2001-09-09 10:46:40 JST +0900\n",
        );
        assert_tz("Asia/Tokyo", "0", "1970-01-01 09:00:00 JST +0900\n");
    }
    if zoneinfo_has("Europe/London") {
        assert_tz(
            "Europe/London",
            "1625097600",
            "2021-07-01 01:00:00 BST +0100\n",
        );
        assert_tz(
            "Europe/London",
            "1609459200",
            "2021-01-01 00:00:00 GMT +0000\n",
        );
    }
    if zoneinfo_has("Australia/Sydney") {
        // Southern hemisphere: January is daylight time.
        assert_tz(
            "Australia/Sydney",
            "1609459200",
            "2021-01-01 11:00:00 AEDT +1100\n",
        );
        assert_tz(
            "Australia/Sydney",
            "1625097600",
            "2021-07-01 10:00:00 AEST +1000\n",
        );
    }
}

#[test]
fn olson_future_epoch_uses_tzif_footer_rules() {
    if !zoneinfo_has("America/New_York") {
        return;
    }
    // 2100 is past the file's explicit transition table; the TZif
    // footer POSIX rule (EST5EDT,M3.2.0,M11.1.0) extrapolates it.
    assert_tz(
        "America/New_York",
        "4118083200",
        "2100-06-30 20:00:00 EDT -0400\n",
    );
}

#[test]
fn olson_lookup_honours_tzdir_and_beats_posix_parse() {
    // A TZif file named XYZ5 in a private TZDIR must win over the
    // POSIX reading of XYZ5 (which would render 19:00 XYZ -0500):
    // glibc tries the file first.
    let source = ["/usr/share/zoneinfo", "/usr/share/lib/zoneinfo"]
        .iter()
        .map(|dir| PathBuf::from(dir).join("Asia/Tokyo"))
        .find(|path| path.is_file());
    let Some(source) = source else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("rubash-2705-tzdir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create TZDIR");
    std::fs::copy(&source, dir.join("XYZ5")).expect("copy Tokyo TZif as XYZ5");
    let tzdir = dir.to_string_lossy().into_owned();
    assert_tz_env(
        "XYZ5",
        "0",
        "1970-01-01 09:00:00 JST +0900\n",
        &[("TZDIR", &tzdir)],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_zone_falls_back_to_leading_name_at_zero_offset() {
    // No such TZif file anywhere: GNU renders UTC time under the
    // string's leading name, not under the literal name "UTC".
    assert_tz("Not/AZone", "0", "1970-01-01 00:00:00 Not +0000\n");
    assert_tz(":Not/AZone", "0", "1970-01-01 00:00:00 Not +0000\n");
}

#[test]
fn posix_forms_still_render_as_before() {
    // Guards: the POSIX path and the named UTC zone are unchanged.
    // EST5EDT may resolve through its TZif file where one exists;
    // either route renders these epochs identically (oracle-verified).
    assert_tz("EST5EDT", "1625097600", "2021-06-30 20:00:00 EDT -0400\n");
    assert_tz("UTC", "0", "1970-01-01 00:00:00 UTC +0000\n");
    assert_tz("CST-8", "0", "1970-01-01 08:00:00 CST +0800\n");
}
