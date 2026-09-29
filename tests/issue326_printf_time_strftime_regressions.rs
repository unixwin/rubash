//! Issue rubash#326: `printf '%(fmt)T'` — the strftime specs `%l` (12-hour
//! space-padded), `%k` (24-hour space-padded), `%u` (ISO weekday), `%V`
//! (ISO week number) and `%G` (ISO week-based year) printed literally,
//! and the time was formatted in UTC even on a local-timezone host
//! (pbb section 076: `Tue 29 Sep - %l:21 AM` and the wrong half of the
//! day).
//!
//! GNU contract (vendored third_party/bash): builtins/printf.def:633
//! `tm = localtime (&secs)` — the format then goes to the SYSTEM strftime
//! (glibc), which owns %l/%k/%u/%V/%G. localtime honors TZ (sv_tz is
//! called first at printf.def:628-630); an unset TZ uses the system
//! default timezone; a POSIX TZ string with a DST name and no explicit
//! rules gets the USA defaults (glibc: 2nd Sunday March .. 1st Sunday
//! November, 02:00 local — `TZ=EST5EDT` observes EDT in summer).
//!
//! Fix: strftime_subset gains %l/%k/%u plus the ISO-week %V/%G pair
//! (week 01 = the week containing the year's first Thursday); zone.rs
//! falls back to the Windows registry timezone (GetTimeZoneInformation)
//! when TZ is unset, maps TZ="" to "Universal" like glibc, and applies
//! the US default DST rules for rule-less DST names.
//!
//! Platform notes: %Z renders the Windows registry StandardName (here
//! the localized "中国标准时间") where GNU shows the POSIX abbreviation
//! (CST) — text-only, offsets and every numeric spec are exact; Olson
//! TZ names (America/New_York) need tzdata files and stay UTC.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

/// The issue's Reproducer A shape (fixed epoch for determinism):
/// `%l|%e|%k|%u|%V|%G`. 1760000000 = 2025-10-09 (Thursday) 08:53:20 UTC.
#[test]
fn padded_hours_iso_weekday_and_week_year() {
    let (stdout, stderr, code) = rubash("TZ=UTC printf '%(%l|%e|%k|%u|%V|%G)T\\n' 1760000000");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, " 8| 9| 8|4|41|2025\n");
}

/// The pbb date-section compound format (fixed epoch):
/// 1760000000 UTC = Thu Oct 9 2025 08:53:20.
#[test]
fn pbb_date_format_with_padded_12_hour() {
    let (stdout, stderr, code) = rubash("TZ=UTC printf '%(%a %d %b - %l:%M %p)T\\n' 1760000000");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "Thu 09 Oct -  8:53 AM\n");
}

/// ISO week edges: the year's first Thursday defines week 01.
/// 2025-12-29 (Monday) is week 01 of 2026; 2024-12-30 (Monday) week 01 of
/// 2025; 2026-01-01 (Thursday) week 01 of 2026.
#[test]
fn iso_week_boundaries() {
    let (stdout, stderr, code) = rubash(
        "TZ=UTC printf '%(%V|%G|%u)T\\n' 1767139200\n\
         TZ=UTC printf '%(%V|%G|%u)T\\n' 1735689600\n\
         TZ=UTC printf '%(%V|%G|%u)T\\n' 1767225600",
    );
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "01|2026|3\n01|2025|3\n01|2026|4\n");
}

/// ISO weekday numbering: Sunday is 7, not 0 (2023-12-31 is a Sunday).
#[test]
fn iso_weekday_sunday_is_seven() {
    let (stdout, stderr, code) = rubash("TZ=UTC printf '%(%u)T\\n' 1704067199");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "7\n");
}

/// POSIX TZ strings format localtime: CST-8 (no DST), fixed epoch.
#[test]
fn posix_tz_cst_formats_localtime() {
    let (stdout, stderr, code) = rubash("TZ=CST-8 printf '%(%Y%m%d%H%M%S|%Z|%z)T\\n' 0");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "19700101080000|CST|+0800\n");
}

/// A DST name without explicit rules gets the US defaults
/// (`TZ=EST5EDT`): summer epoch renders EDT, winter epoch EST.
/// 1747000000 = 2025-05-11 21:46:40 UTC (EDT); 1738000000 =
/// 2025-01-27 21:46:40 UTC (EST).
#[test]
fn ruleless_dst_name_uses_us_defaults() {
    let (stdout, stderr, code) = rubash(
        "TZ=EST5EDT printf '%(%H|%Z)T\\n' 1747000000\n\
         TZ=EST5EDT printf '%(%H|%Z)T\\n' 1738000000",
    );
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "17|EDT\n12|EST\n");
}

/// TZ="" maps to UTC under the name "Universal" (glibc behavior).
#[test]
fn empty_tz_is_universal() {
    let (stdout, stderr, code) = rubash("TZ= printf '%(%H|%Z)T\\n' 1760000000");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "08|Universal\n");
}

/// TZ=UTC stays UTC.
#[test]
fn utc_tz_unchanged() {
    let (stdout, stderr, code) = rubash("TZ=UTC printf '%(%H|%Z|%z)T\\n' 1760000000");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "08|UTC|+0000\n");
}

/// With TZ unset, the output is the SYSTEM local time (Windows registry
/// timezone), not hardcoded UTC. Deterministic on any host: compare the
/// unset-TZ hour against PowerShell's own local-time rendering of the
/// same instant... too heavy for CI — assert instead that unset-TZ and a
/// matching explicit zone produce the same value by round-tripping via
/// date's %z: the unset-TZ offset must match `date +%z` (the OS's own
/// local offset for the same second).
#[test]
fn unset_tz_uses_system_localtime_offset() {
    let (stdout, stderr, code) = rubash("unset TZ\nprintf '%(%z)T\\n' -1\ndate +%z");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    let mut lines = stdout.lines();
    let printf_zone = lines.next().unwrap_or_default();
    let date_zone = lines.next().unwrap_or_default();
    assert_eq!(
        printf_zone, date_zone,
        "printf %(fmt)T offset must match the OS local timezone"
    );
}
