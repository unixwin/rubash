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
    let seconds = match seconds {
        -1 => current_epoch_seconds(),
        -2 => env_vars
            .get("__RUBASH_SHELL_START_EPOCH")
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(current_epoch_seconds),
        other => other,
    };
    let timezone = TimeZoneRule::from_env(env_vars.get("TZ").map(String::as_str));
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
    let timezone = TimeZoneRule::from_env(env_vars.get("TZ").map(String::as_str));
    let local = timezone.local_time(current_epoch_seconds());
    strftime_subset(format, &local)
}

fn current_epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
