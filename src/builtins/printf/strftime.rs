use super::zone::LocalTimeParts;

pub(super) fn strftime_subset(format: &str, time: &LocalTimeParts) -> String {
    let mut output = String::new();
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            output.push(ch);
            continue;
        }
        let Some(specifier) = chars.next() else {
            output.push('%');
            break;
        };
        match specifier {
            '%' => output.push('%'),
            'a' => output.push_str(WEEKDAYS_ABBR[time.weekday as usize]),
            'A' => output.push_str(WEEKDAYS_FULL[time.weekday as usize]),
            'b' | 'h' => output.push_str(MONTHS_ABBR[time.month as usize - 1]),
            'B' => output.push_str(MONTHS_FULL[time.month as usize - 1]),
            'd' => output.push_str(&format!("{:02}", time.day)),
            'e' => output.push_str(&format!("{:2}", time.day)),
            'H' => output.push_str(&format!("{:02}", time.hour)),
            'I' => output.push_str(&format!("{:02}", twelve_hour(time.hour))),
            'M' => output.push_str(&format!("{:02}", time.minute)),
            'S' => output.push_str(&format!("{:02}", time.second)),
            'Y' => output.push_str(&format!("{:04}", time.year)),
            'y' => output.push_str(&format!("{:02}", time.year.rem_euclid(100))),
            'F' => output.push_str(&format!(
                "{:04}-{:02}-{:02}",
                time.year, time.month, time.day
            )),
            'T' => output.push_str(&format!(
                "{:02}:{:02}:{:02}",
                time.hour, time.minute, time.second
            )),
            'r' => output.push_str(&format!(
                "{:02}:{:02}:{:02} {}",
                twelve_hour(time.hour),
                time.minute,
                time.second,
                if time.hour < 12 { "AM" } else { "PM" }
            )),
            'p' => output.push_str(if time.hour < 12 { "AM" } else { "PM" }),
            'z' => output.push_str(&format_offset(time.offset)),
            'Z' => output.push_str(&time.zone_name),
            's' => output.push_str(&time.epoch.to_string()),
            'x' => output.push_str(&format!(
                "{:02}/{:02}/{:02}",
                time.month,
                time.day,
                time.year.rem_euclid(100)
            )),
            'X' => output.push_str(&format!(
                "{:02}:{:02}:{:02}",
                time.hour, time.minute, time.second
            )),
            other => {
                output.push('%');
                output.push(other);
            }
        }
    }
    output
}

fn twelve_hour(hour: u8) -> u8 {
    match hour % 12 {
        0 => 12,
        other => other,
    }
}

fn format_offset(offset: i32) -> String {
    let sign = if offset < 0 { '-' } else { '+' };
    let abs = offset.abs();
    format!("{sign}{:02}{:02}", abs / 3600, (abs % 3600) / 60)
}

const WEEKDAYS_ABBR: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAYS_FULL: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS_ABBR: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_FULL: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
