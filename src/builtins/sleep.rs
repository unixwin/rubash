//! The Bash `sleep` builtin-compatible command used by job and coprocess tests.

use std::time::Duration;

pub fn execute(args: &[String]) -> (i32, Option<String>) {
    let mut total_seconds = 0.0_f64;
    let mut index = 0;
    if args.first().is_some_and(|arg| arg == "--") {
        index = 1;
    }
    if index == args.len() {
        return (1, Some("sleep: missing operand\n".to_string()));
    }

    for value in &args[index..] {
        let Some(seconds) = parse_duration(value) else {
            return (1, Some(format!("sleep: invalid time interval '{value}'\n")));
        };
        total_seconds += seconds;
    }

    if total_seconds.is_sign_negative() || !total_seconds.is_finite() {
        return (1, Some("sleep: invalid time interval\n".to_string()));
    }
    std::thread::sleep(Duration::from_secs_f64(total_seconds));
    (0, None)
}

/// Sum of the sleep durations in `args`, or None when an operand is
/// missing/unparseable. Shared by `execute` and the executor's
/// interruptible fast path (`command_dispatch_primary.rs`), which needs the
/// total without blocking.
pub fn total_duration(args: &[String]) -> Option<f64> {
    let mut index = 0;
    if args.first().is_some_and(|arg| arg == "--") {
        index = 1;
    }
    if index == args.len() {
        return None;
    }
    let mut total_seconds = 0.0_f64;
    for value in &args[index..] {
        total_seconds += parse_duration(value)?;
    }
    Some(total_seconds)
}

pub fn can_execute_fast_path(args: &[String]) -> bool {
    total_duration(args).is_some()
}

fn parse_duration(value: &str) -> Option<f64> {
    let (number, multiplier) = match value.chars().last()? {
        's' => (&value[..value.len() - 1], 1.0),
        'm' => (&value[..value.len() - 1], 60.0),
        'h' => (&value[..value.len() - 1], 3600.0),
        'd' => (&value[..value.len() - 1], 86400.0),
        _ => (value, 1.0),
    };
    let seconds = number.parse::<f64>().ok()?;
    (seconds >= 0.0 && seconds.is_finite()).then_some(seconds * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_units() {
        assert_eq!(parse_duration("2"), Some(2.0));
        assert_eq!(parse_duration("2m"), Some(120.0));
        assert_eq!(parse_duration("0.01s"), Some(0.01));
    }

    #[test]
    fn rejects_invalid_values() {
        assert_eq!(parse_duration("nope"), None);
        assert_eq!(parse_duration("-1"), None);
    }
}
