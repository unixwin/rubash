pub(super) struct TimeZoneRule {
    standard_name: String,
    daylight_name: Option<String>,
    standard_offset: i32,
    daylight_offset: i32,
    start_rule: Option<MonthWeekdayRule>,
    end_rule: Option<MonthWeekdayRule>,
}

#[derive(Debug, Clone, Copy)]
struct MonthWeekdayRule {
    month: u8,
    week: u8,
    weekday: u8,
    seconds: i32,
}

#[derive(Debug, Clone)]
pub(super) struct LocalTimeParts {
    pub(super) year: i32,
    pub(super) month: u8,
    pub(super) day: u8,
    pub(super) hour: u8,
    pub(super) minute: u8,
    pub(super) second: u8,
    pub(super) weekday: u8,
    pub(super) zone_name: String,
    pub(super) offset: i32,
    pub(super) epoch: i64,
}

impl TimeZoneRule {
    pub(super) fn from_env(tz: Option<&str>) -> Self {
        tz.and_then(parse_posix_timezone).unwrap_or_else(|| Self {
            standard_name: "UTC".to_string(),
            daylight_name: None,
            standard_offset: 0,
            daylight_offset: 0,
            start_rule: None,
            end_rule: None,
        })
    }

    pub(super) fn local_time(&self, epoch: i64) -> LocalTimeParts {
        let daylight = self.is_daylight_time(epoch);
        let offset = if daylight {
            self.daylight_offset
        } else {
            self.standard_offset
        };
        let mut parts = epoch_to_parts(epoch + i64::from(offset));
        parts.zone_name = if daylight {
            self.daylight_name
                .clone()
                .unwrap_or_else(|| self.standard_name.clone())
        } else {
            self.standard_name.clone()
        };
        parts.offset = offset;
        parts.epoch = epoch;
        parts
    }

    fn is_daylight_time(&self, epoch: i64) -> bool {
        let (Some(start), Some(end), Some(_)) =
            (self.start_rule, self.end_rule, self.daylight_name.as_ref())
        else {
            return false;
        };

        let standard_parts = epoch_to_parts(epoch + i64::from(self.standard_offset));
        let year = standard_parts.year;
        let start_epoch = transition_epoch(year, start, self.standard_offset);
        let end_epoch = transition_epoch(year, end, self.daylight_offset);
        if start_epoch <= end_epoch {
            epoch >= start_epoch && epoch < end_epoch
        } else {
            epoch >= start_epoch || epoch < end_epoch
        }
    }
}

fn parse_posix_timezone(value: &str) -> Option<TimeZoneRule> {
    if matches!(value, "UTC" | "GMT") {
        return Some(TimeZoneRule {
            standard_name: value.to_string(),
            daylight_name: None,
            standard_offset: 0,
            daylight_offset: 0,
            start_rule: None,
            end_rule: None,
        });
    }

    let bytes = value.as_bytes();
    let mut index = 0;
    let standard_name = read_tz_name(value, &mut index)?;
    let standard_offset = -parse_tz_offset(value, &mut index)?;
    let daylight_name = read_tz_name(value, &mut index);
    let daylight_offset = if daylight_name.is_some() {
        if index < bytes.len() && bytes[index] != b',' {
            -parse_tz_offset(value, &mut index)?
        } else {
            standard_offset + 3600
        }
    } else {
        standard_offset
    };

    let mut start_rule = None;
    let mut end_rule = None;
    if index < bytes.len() && bytes[index] == b',' {
        index += 1;
        start_rule = parse_month_weekday_rule(value, &mut index);
        if index < bytes.len() && bytes[index] == b',' {
            index += 1;
            end_rule = parse_month_weekday_rule(value, &mut index);
        }
    }

    Some(TimeZoneRule {
        standard_name,
        daylight_name,
        standard_offset,
        daylight_offset,
        start_rule,
        end_rule,
    })
}

fn read_tz_name(value: &str, index: &mut usize) -> Option<String> {
    let start = *index;
    while let Some(ch) = value[*index..].chars().next() {
        if !ch.is_ascii_alphabetic() {
            break;
        }
        *index += ch.len_utf8();
    }
    (*index > start).then(|| value[start..*index].to_string())
}

fn parse_tz_offset(value: &str, index: &mut usize) -> Option<i32> {
    let mut sign = 1;
    if let Some(ch) = value[*index..].chars().next() {
        if ch == '-' {
            sign = -1;
            *index += 1;
        } else if ch == '+' {
            *index += 1;
        }
    }
    let hours = parse_number(value, index)? as i32;
    let mut minutes = 0;
    let mut seconds = 0;
    if value.as_bytes().get(*index) == Some(&b':') {
        *index += 1;
        minutes = parse_number(value, index)? as i32;
        if value.as_bytes().get(*index) == Some(&b':') {
            *index += 1;
            seconds = parse_number(value, index)? as i32;
        }
    }
    Some(sign * (hours * 3600 + minutes * 60 + seconds))
}

fn parse_month_weekday_rule(value: &str, index: &mut usize) -> Option<MonthWeekdayRule> {
    if value.as_bytes().get(*index) != Some(&b'M') {
        return None;
    }
    *index += 1;
    let month = parse_number(value, index)? as u8;
    if value.as_bytes().get(*index) != Some(&b'.') {
        return None;
    }
    *index += 1;
    let week = parse_number(value, index)? as u8;
    if value.as_bytes().get(*index) != Some(&b'.') {
        return None;
    }
    *index += 1;
    let weekday = parse_number(value, index)? as u8;
    let mut seconds = 2 * 3600;
    if value.as_bytes().get(*index) == Some(&b'/') {
        *index += 1;
        seconds = parse_tz_offset(value, index)?;
    }
    Some(MonthWeekdayRule {
        month,
        week,
        weekday,
        seconds,
    })
}

fn parse_number(value: &str, index: &mut usize) -> Option<u32> {
    let start = *index;
    while value.as_bytes().get(*index).is_some_and(u8::is_ascii_digit) {
        *index += 1;
    }
    (*index > start)
        .then(|| value[start..*index].parse().ok())
        .flatten()
}

fn transition_epoch(year: i32, rule: MonthWeekdayRule, offset_before: i32) -> i64 {
    let day = nth_weekday_of_month(year, rule.month, rule.week, rule.weekday);
    let days = days_from_civil(year, u32::from(rule.month), u32::from(day));
    days * 86_400 + i64::from(rule.seconds) - i64::from(offset_before)
}

fn nth_weekday_of_month(year: i32, month: u8, week: u8, weekday: u8) -> u8 {
    let first_weekday = weekday_from_date(year, month, 1);
    let mut day = 1 + ((7 + weekday as i32 - first_weekday as i32) % 7) as u8;
    if week < 5 {
        day += 7 * (week - 1);
    } else {
        while day + 7 <= days_in_month(year, month) {
            day += 7;
        }
    }
    day
}

fn weekday_from_date(year: i32, month: u8, day: u8) -> u8 {
    let days = days_from_civil(year, u32::from(month), u32::from(day));
    (days + 4).rem_euclid(7) as u8
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 30,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn epoch_to_parts(epoch: i64) -> LocalTimeParts {
    let days = epoch.div_euclid(86_400);
    let seconds_of_day = epoch.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    LocalTimeParts {
        year,
        month: month as u8,
        day: day as u8,
        hour: (seconds_of_day / 3600) as u8,
        minute: ((seconds_of_day % 3600) / 60) as u8,
        second: (seconds_of_day % 60) as u8,
        weekday: (days + 4).rem_euclid(7) as u8,
        zone_name: "UTC".to_string(),
        offset: 0,
        epoch,
    }
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - (month <= 2) as i32;
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let year_of_era = (year - era * 400) as u32;
    let month_prime = month as i32 + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime as u32 + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    i64::from(era) * 146_097 + i64::from(day_of_era) - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = (if days >= 0 { days } else { days - 146_096 }) / 146_097;
    let day_of_era = (days - era * 146_097) as u32;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era as i32 + era as i32 * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    year += (month <= 2) as i32;
    (year, month, day)
}
