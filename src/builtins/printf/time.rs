#[path = "strftime.rs"]
mod strftime;
#[path = "zone.rs"]
mod zone;

use super::number::{invalid_number_error, parse_i64};
use super::value::{apply_width, truncate_precision};
use super::{FormatSpec, ParsedNumber};
use std::collections::HashMap;
use strftime::strftime_subset;
use zone::TimeZoneRule;

pub(super) fn format_time_value(
    value: &str,
    spec: &FormatSpec,
    env_vars: &HashMap<String, String>,
) -> (String, Option<String>) {
    let ParsedNumber {
        value: seconds,
        invalid,
    } = parse_i64(value);
    // GNU getintmax distinguishes a MISSING argument (the caller passes
    // the -1 "now" sentinel, never an empty string) from a PRESENT empty
    // one: strtoimax on "" leaves ep == s, so chk_converror reports
    // `: invalid number' and the value is 0 (rubash#1116). parse_i64's
    // empty arm models the missing case, so re-arm the diagnostic here,
    // where empty can only mean present.
    let invalid = if value.is_empty() {
        Some(String::new())
    } else {
        invalid
    };
    let seconds = match seconds {
        -1 => current_epoch_seconds(),
        -2 => env_vars
            .get("__RUBASH_SHELL_START_EPOCH")
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(current_epoch_seconds),
        other => other,
    };
    // GNU printf.def getintmax clamps an out-of-range operand to
    // INTMAX_MAX/MIN and chk_converror raises ERANGE; the subsequent
    // localtime() on the clamped value fails, and bash renders the
    // conversion against the zeroed tm — the epoch (rubash#1286:
    // `printf '%(%F)T' 99999999999999999999` prints 1970-01-01 in GNU
    // 5.3.20, both signs; rubash wrapped the value through its civil
    // conversion and printed year 219250468). The ERANGE diagnostic
    // itself is unchanged (invalid_number_error).
    let seconds = if invalid
        .as_deref()
        .is_some_and(|marker| marker.starts_with("__rubash_printf_overflow__:"))
    {
        0
    } else {
        seconds
    };
    let timezone = TimeZoneRule::from_env(
        env_vars.get("TZ").map(String::as_str),
        env_vars.get("TZDIR").map(String::as_str),
    );
    let local = timezone.local_time(seconds);
    let format = spec.time_format.as_deref().unwrap_or_default();
    let format = if format.is_empty() { "%X" } else { format };
    let rendered = strftime_subset(format, &local);

    let mut width_spec = spec.clone();
    width_spec.zero_pad = false;
    (
        apply_width(truncate_precision(rendered, spec.precision), &width_spec),
        invalid.map(|value| invalid_number_error(&value)),
    )
}

pub(crate) fn format_current_time(format: &str, env_vars: &HashMap<String, String>) -> String {
    let timezone = TimeZoneRule::from_env(
        env_vars.get("TZ").map(String::as_str),
        env_vars.get("TZDIR").map(String::as_str),
    );
    let local = timezone.local_time(current_epoch_seconds());
    strftime_subset(format, &local)
}

/// Format an arbitrary epoch second with the same strftime subset and
/// TZ rules as `printf %()T` (history listings render HISTTIMEFORMAT
/// against each entry's recorded timestamp this way — builtins/
/// history.def display_history via strftime, rubash#1070).
pub(crate) fn format_epoch_time(
    format: &str,
    epoch: i64,
    env_vars: &HashMap<String, String>,
) -> String {
    let timezone = TimeZoneRule::from_env(
        env_vars.get("TZ").map(String::as_str),
        env_vars.get("TZDIR").map(String::as_str),
    );
    let local = timezone.local_time(epoch);
    strftime_subset(format, &local)
}

/// `format_epoch_time` restricted to the epochs the civil
/// conversion can represent: GNU renders a history timestamp only
/// when localtime succeeds (builtins/history.def histtime), and
/// localtime fails when the local year minus 1900 (tm_year) does
/// not fit a C int. Returns None outside the representable domain
/// so the caller can print GNU's "invalid timestamp" form
/// (rubash#1669). The domain check runs on an i64 year recomputed
/// from the day count, before `local_time` is consulted — the
/// conversion behind it wraps its i32 year arithmetic for epochs
/// this large.
pub(crate) fn format_epoch_time_if_valid(
    format: &str,
    epoch: i64,
    env_vars: &HashMap<String, String>,
) -> Option<String> {
    // Screen on the UTC year first, in i64: the civil conversion
    // behind `local_time` does its year arithmetic in i32 and wraps
    // (or, in debug builds, panics) for epochs this large, so an
    // out-of-domain epoch must never reach it. Zone offsets are
    // bounded by a day, so a two-year margin absorbs any local/UTC
    // year difference.
    let utc_year = civil_year_from_days(epoch.div_euclid(86_400));
    if utc_year < i64::from(i32::MIN) + 2 || utc_year > i64::from(i32::MAX) - 2 {
        return None;
    }
    let timezone = TimeZoneRule::from_env(
        env_vars.get("TZ").map(String::as_str),
        env_vars.get("TZDIR").map(String::as_str),
    );
    let local = timezone.local_time(epoch);
    let local_days = (epoch + i64::from(local.offset)).div_euclid(86_400);
    if civil_year_from_days(local_days) != i64::from(local.year) {
        return None;
    }
    Some(strftime_subset(format, &local))
}

/// The proleptic Gregorian year containing a day count (days since
/// 1970-01-01), computed in i64 so years far outside the C int range
/// stay exact (the Hinnant civil-from-days algorithm).
fn civil_year_from_days(days: i64) -> i64 {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    if month <= 2 {
        year + 1
    } else {
        year
    }
}

fn current_epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
