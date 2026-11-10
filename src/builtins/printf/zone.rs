pub(super) struct TimeZoneRule {
    standard_name: String,
    daylight_name: Option<String>,
    standard_offset: i32,
    daylight_offset: i32,
    start_rule: Option<MonthWeekdayRule>,
    end_rule: Option<MonthWeekdayRule>,
    system_local: bool,
    /// A resolved TZif database entry (an Olson name, an absolute
    /// path, or a POSIX-looking name that exists as a file — glibc
    /// tries the file first in every case). When present it owns
    /// `local_time`; the POSIX fields above are all inert.
    tzif: Option<TzifZone>,
}

/// One ttinfo record from a TZif file: a fixed UTC offset, its DST
/// flag, and the abbreviation %Z renders.
#[derive(Debug, Clone)]
struct TzifType {
    offset: i32,
    is_dst: bool,
    abbrev: String,
}

/// A parsed TZif file (RFC 8536): the transition table plus the POSIX
/// footer rule that extrapolates past the last transition.
#[derive(Debug, Clone)]
struct TzifZone {
    transitions: Vec<i64>,
    transition_types: Vec<usize>,
    types: Vec<TzifType>,
    /// Type for epochs before the first transition: the first
    /// standard-time type in file order (glibc's choice), else type 0.
    first_type: usize,
    footer: Option<Box<TimeZoneRule>>,
}

impl TzifZone {
    fn local_time(&self, epoch: i64) -> LocalTimeParts {
        // Past the explicit table the file's POSIX footer rule owns
        // the rendering (glibc extrapolates the same way); at the
        // last transition itself both agree by construction.
        if let (Some(last), Some(footer)) = (self.transitions.last(), &self.footer) {
            if epoch >= *last {
                return footer.local_time(epoch);
            }
        }
        let ty = &self.types[self.type_index_at(epoch)];
        let mut parts = epoch_to_parts(epoch + i64::from(ty.offset));
        parts.zone_name = ty.abbrev.clone();
        parts.offset = ty.offset;
        parts.epoch = epoch;
        parts
    }

    fn type_index_at(&self, epoch: i64) -> usize {
        if self.transitions.is_empty() || epoch < self.transitions[0] {
            return self.first_type;
        }
        let passed = self.transitions.partition_point(|at| *at <= epoch);
        self.transition_types[passed - 1]
    }
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
    pub(super) fn from_env(tz: Option<&str>, tzdir: Option<&str>) -> Self {
        // glibc localtime (GNU printf.def:633 calls localtime after
        // sv_tz) resolves a set TZ in tzset order: a leading ':' is
        // stripped, the name is looked up as a TZif file first (under
        // TZDIR when set, else the system zoneinfo directory — an
        // absolute TZ is the path itself), and only a name with no
        // file is parsed as POSIX rules. A string matching neither
        // renders at offset zero under its leading name (oracle:
        // TZ=Not/AZone -> %Z "Not", +0000). TZ="" is UTC; an UNSET TZ
        // falls back to the system default timezone — on Windows that
        // is the registry timezone (GetTimeZoneInformation), on other
        // platforms UTC (rubash#326: %(fmt)T formatted UTC on a CST
        // host because the unset-TZ case defaulted to UTC).
        // rubash#2705: the TZif step is new — Olson names previously
        // fell through the POSIX parse straight into the UTC fallback.
        match tz {
            // glibc maps an empty TZ to UTC under the name "Universal".
            Some(value) if value.is_empty() => Self {
                standard_name: "Universal".to_string(),
                daylight_name: None,
                standard_offset: 0,
                daylight_offset: 0,
                start_rule: None,
                end_rule: None,
                system_local: false,
                tzif: None,
            },
            Some(value) => {
                let name = value.strip_prefix(':').unwrap_or(value);
                if let Some(zone) = load_tzif(name, tzdir) {
                    return Self {
                        standard_name: String::new(),
                        daylight_name: None,
                        standard_offset: 0,
                        daylight_offset: 0,
                        start_rule: None,
                        end_rule: None,
                        system_local: false,
                        tzif: Some(zone),
                    };
                }
                if let Some(rule) = parse_posix_timezone(name) {
                    return rule;
                }
                let mut index = 0;
                Self {
                    standard_name: read_tz_name(name, &mut index).unwrap_or_default(),
                    daylight_name: None,
                    standard_offset: 0,
                    daylight_offset: 0,
                    start_rule: None,
                    end_rule: None,
                    system_local: false,
                    tzif: None,
                }
            }
            None => system_timezone().unwrap_or_else(Self::utc),
        }
    }

    fn utc() -> Self {
        Self {
            standard_name: "UTC".to_string(),
            daylight_name: None,
            standard_offset: 0,
            daylight_offset: 0,
            start_rule: None,
            end_rule: None,
            system_local: false,
            tzif: None,
        }
    }

    pub(super) fn local_time(&self, epoch: i64) -> LocalTimeParts {
        if let Some(zone) = &self.tzif {
            return zone.local_time(epoch);
        }
        // GNU printf.def:633 formats `localtime(&secs)`; with TZ unset libc
        // resolves the system zone (/etc/localtime), so delegate per-epoch
        // to localtime_r — a fixed snapshot offset would misrender DST
        // history. strftime %z then prints tm_gmtoff exactly like GNU.
        #[cfg(unix)]
        if self.system_local {
            return system_local_time(epoch);
        }
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
            system_local: false,
            tzif: None,
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
    } else if daylight_name.is_some() {
        // POSIX 8.1.3: a DST name without explicit rules gets
        // implementation-defined defaults — glibc uses the current USA
        // rules (second Sunday in March to the first Sunday in November,
        // 02:00 local), so `TZ=EST5EDT` observes EDT in summer exactly
        // like GNU localtime (verified WSL GNU 5.3.0).
        start_rule = Some(MonthWeekdayRule {
            month: 3,
            week: 2,
            weekday: 0,
            seconds: 2 * 3600,
        });
        end_rule = Some(MonthWeekdayRule {
            month: 11,
            week: 1,
            weekday: 0,
            seconds: 2 * 3600,
        });
    }

    Some(TimeZoneRule {
        standard_name,
        daylight_name,
        standard_offset,
        daylight_offset,
        start_rule,
        end_rule,
        system_local: false,
        tzif: None,
    })
}

/// Zoneinfo directories glibc searches when TZDIR is not set (its
/// compiled-in TZDIR is /usr/share/zoneinfo; the lib path and the
/// Android location cover the other hosts this engine ships on).
const DEFAULT_ZONEINFO_DIRS: &[&str] = &[
    "/usr/share/zoneinfo",
    "/usr/share/lib/zoneinfo",
    "/system/usr/share/zoneinfo",
];

/// Resolve a TZ name to a TZif zone, glibc tzfile order: an absolute
/// name is the path itself; a relative name is looked up under TZDIR
/// when TZDIR is set (that directory alone), else under the default
/// zoneinfo directories. A `..` path element never resolves (glibc
/// refuses parent traversal in TZ names). Any failure — missing file,
/// malformed contents — returns None and the caller falls through to
/// the POSIX parse, exactly like tzset.
fn load_tzif(name: &str, tzdir: Option<&str>) -> Option<TzifZone> {
    if name.is_empty() || name.split('/').any(|part| part == "..") {
        return None;
    }
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if name.starts_with('/') {
        candidates.push(std::path::PathBuf::from(name));
    } else if let Some(dir) = tzdir.filter(|dir| !dir.is_empty()) {
        candidates.push(std::path::PathBuf::from(dir).join(name));
    } else {
        candidates.extend(
            DEFAULT_ZONEINFO_DIRS
                .iter()
                .map(|dir| std::path::PathBuf::from(dir).join(name)),
        );
    }
    for path in candidates {
        if let Ok(bytes) = std::fs::read(&path) {
            if let Some(zone) = parse_tzif(&bytes) {
                return Some(zone);
            }
        }
    }
    None
}

/// A byte cursor over a TZif image; every read is bounds-checked so a
/// malformed file degrades to None instead of panicking.
struct TzifCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> TzifCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let slice = self.data.get(self.pos..self.pos.checked_add(count)?)?;
        self.pos += count;
        Some(slice)
    }

    fn skip(&mut self, count: usize) -> Option<()> {
        self.take(count).map(|_| ())
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|slice| slice[0])
    }

    fn be_u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|slice| u32::from_be_bytes(slice.try_into().expect("4-byte slice")))
    }

    fn be_i32(&mut self) -> Option<i32> {
        self.be_u32().map(|value| value as i32)
    }

    fn be_i64(&mut self) -> Option<i64> {
        self.take(8)
            .map(|slice| i64::from_be_bytes(slice.try_into().expect("8-byte slice")))
    }
}

/// The six header counters of one TZif block (RFC 8536 section 3.1).
/// The cursor must sit just past the 4-byte magic; the version byte
/// and the 15 reserved bytes (16 total) precede the counters.
struct TzifCounts {
    isutcnt: usize,
    isstdcnt: usize,
    leapcnt: usize,
    timecnt: usize,
    typecnt: usize,
    charcnt: usize,
}

fn read_tzif_counts(cursor: &mut TzifCursor) -> Option<TzifCounts> {
    cursor.skip(16)?;
    Some(TzifCounts {
        isutcnt: cursor.be_u32()? as usize,
        isstdcnt: cursor.be_u32()? as usize,
        leapcnt: cursor.be_u32()? as usize,
        timecnt: cursor.be_u32()? as usize,
        typecnt: cursor.be_u32()? as usize,
        charcnt: cursor.be_u32()? as usize,
    })
}

fn parse_tzif(data: &[u8]) -> Option<TzifZone> {
    if data.len() < 44 || &data[0..4] != b"TZif" {
        return None;
    }
    let version = data[4];
    let mut cursor = TzifCursor::new(data);
    cursor.skip(4)?;
    let counts = read_tzif_counts(&mut cursor)?;
    if version == 0 {
        // Version 1: the single block uses 32-bit transition times and
        // 8-byte leap-second records, and carries no footer.
        return parse_tzif_block(&mut cursor, &counts, false, None);
    }
    // Versions 2-4: skip the v1 block, parse the 64-bit block, then
    // read the footer TZ string enclosed in newlines.
    skip_tzif_block(&mut cursor, &counts, false)?;
    cursor.skip(4)?;
    let counts = read_tzif_counts(&mut cursor)?;
    let mut zone = parse_tzif_block(&mut cursor, &counts, true, None)?;
    let rest = &data[cursor.pos.min(data.len())..];
    if rest.len() >= 2 && rest[0] == b'\n' {
        if let Some(end) = rest[1..].iter().position(|byte| *byte == b'\n') {
            let footer_text = String::from_utf8_lossy(&rest[1..1 + end]).into_owned();
            if !footer_text.is_empty() {
                zone.footer = parse_posix_timezone(&footer_text).map(Box::new);
            }
        }
    }
    Some(zone)
}

fn skip_tzif_block(cursor: &mut TzifCursor, counts: &TzifCounts, wide: bool) -> Option<()> {
    let time_size = if wide { 8 } else { 4 };
    let leap_size = if wide { 12 } else { 8 };
    cursor.skip(counts.timecnt.checked_mul(time_size)?)?;
    cursor.skip(counts.timecnt)?;
    cursor.skip(counts.typecnt.checked_mul(6)?)?;
    cursor.skip(counts.charcnt)?;
    cursor.skip(counts.leapcnt.checked_mul(leap_size)?)?;
    cursor.skip(counts.isstdcnt)?;
    cursor.skip(counts.isutcnt)?;
    Some(())
}

fn parse_tzif_block(
    cursor: &mut TzifCursor,
    counts: &TzifCounts,
    wide: bool,
    footer: Option<Box<TimeZoneRule>>,
) -> Option<TzifZone> {
    if counts.typecnt == 0 {
        return None;
    }
    let mut transitions = Vec::with_capacity(counts.timecnt);
    for _ in 0..counts.timecnt {
        transitions.push(if wide {
            cursor.be_i64()?
        } else {
            i64::from(cursor.be_i32()?)
        });
    }
    let mut transition_types = Vec::with_capacity(counts.timecnt);
    for _ in 0..counts.timecnt {
        let index = cursor.u8()? as usize;
        if index >= counts.typecnt {
            return None;
        }
        transition_types.push(index);
    }
    let mut raw_types = Vec::with_capacity(counts.typecnt);
    for _ in 0..counts.typecnt {
        let offset = cursor.be_i32()?;
        let is_dst = cursor.u8()? != 0;
        let abbrev_index = cursor.u8()? as usize;
        raw_types.push((offset, is_dst, abbrev_index));
    }
    let abbrev_bytes = cursor.take(counts.charcnt)?;
    let mut types = Vec::with_capacity(counts.typecnt);
    for (offset, is_dst, abbrev_index) in raw_types {
        let tail = abbrev_bytes.get(abbrev_index..)?;
        let end = tail
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(tail.len());
        types.push(TzifType {
            offset,
            is_dst,
            abbrev: String::from_utf8_lossy(&tail[..end]).into_owned(),
        });
    }
    let leap_size = if wide { 12 } else { 8 };
    cursor.skip(counts.leapcnt.checked_mul(leap_size)?)?;
    cursor.skip(counts.isstdcnt)?;
    cursor.skip(counts.isutcnt)?;
    let first_type = types.iter().position(|ty| !ty.is_dst).unwrap_or(0);
    Some(TzifZone {
        transitions,
        transition_types,
        types,
        first_type,
        footer,
    })
}

fn read_tz_name(value: &str, index: &mut usize) -> Option<String> {
    // POSIX TZ names are either a run of alphabetic characters or a
    // <...> quoted form whose contents (digits, '+' and '-' included,
    // e.g. <+09>) are the name — glibc accepts both, and TZif footer
    // strings use the quoted form for numeric abbreviations.
    if value[*index..].starts_with('<') {
        let rest = &value[*index + 1..];
        let end = rest.find('>')?;
        if end == 0 {
            return None;
        }
        let name = rest[..end].to_string();
        *index += end + 2;
        return Some(name);
    }
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

/// Weekday (0=Sunday) of a civil date — the strftime %V/%G ISO-week
/// computation needs it for arbitrary dates, not just the rendered one.
pub(super) fn weekday_of(year: i32, month: u8, day: u8) -> u8 {
    weekday_from_date(year, month, day)
}

/// The system default timezone, used when TZ is unset (glibc localtime's
/// /etc/localtime role). Windows: GetTimeZoneInformation (the registry
/// timezone, Bias + optional DST rules); other platforms: None (UTC).
#[cfg(windows)]
fn system_timezone() -> Option<TimeZoneRule> {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::Time::{
        GetTimeZoneInformation, TIME_ZONE_ID_INVALID, TIME_ZONE_INFORMATION,
    };

    let mut info = TIME_ZONE_INFORMATION::default();
    // SAFETY: `info` is a valid, fully-owned TIME_ZONE_INFORMATION; the
    // API only writes into it.
    let result = unsafe { GetTimeZoneInformation(&mut info) };
    if result == TIME_ZONE_ID_INVALID {
        return None;
    }
    let name_from_utf16 = |raw: &[u16]| -> String {
        let len = raw.iter().position(|ch| *ch == 0).unwrap_or(raw.len());
        String::from_utf16_lossy(&raw[..len])
    };
    // TIME_ZONE_INFORMATION: UTC = local + Bias. Offsets here are seconds
    // EAST of UTC, so standard = -(bias + standard_bias) * 60.
    let has_dst = info.DaylightDate.wMonth != 0 && info.StandardDate.wMonth != 0;
    let standard_offset = -(info.Bias + info.StandardBias) * 60;
    let daylight_offset = if has_dst {
        -(info.Bias + info.DaylightBias) * 60
    } else {
        standard_offset
    };
    // Windows' rule struct maps directly onto the POSIX `Mm.w.d`
    // MonthWeekdayRule: wDay 1-5 (5 = last), wDayOfWeek 0=Sunday.
    let rule = |raw: &SYSTEMTIME| -> Option<MonthWeekdayRule> {
        if raw.wMonth == 0 {
            return None;
        }
        Some(MonthWeekdayRule {
            month: raw.wMonth as u8,
            week: raw.wDay as u8,
            weekday: raw.wDayOfWeek as u8,
            seconds: i32::from(raw.wHour) * 3600 + i32::from(raw.wMinute) * 60,
        })
    };
    Some(TimeZoneRule {
        standard_name: name_from_utf16(&info.StandardName),
        daylight_name: has_dst.then(|| name_from_utf16(&info.DaylightName)),
        standard_offset,
        daylight_offset,
        start_rule: has_dst.then(|| rule(&info.DaylightDate)).flatten(),
        end_rule: has_dst.then(|| rule(&info.StandardDate)).flatten(),
        system_local: false,
        tzif: None,
    })
}

#[cfg(not(windows))]
fn system_timezone() -> Option<TimeZoneRule> {
    // GNU localtime with TZ unset reads the system zone; libc owns that
    // database (/etc/localtime, Android tzdata), so mark the rule and let
    // local_time() delegate per-epoch to localtime_r (tm_gmtoff/tm_zone).
    #[cfg(unix)]
    {
        return Some(TimeZoneRule {
            standard_name: String::new(),
            daylight_name: None,
            standard_offset: 0,
            daylight_offset: 0,
            start_rule: None,
            end_rule: None,
            system_local: true,
            tzif: None,
        });
    }
    #[cfg(not(unix))]
    {
        return None;
    }
}

#[cfg(unix)]
// `libc::time_t` is marked deprecated for musl/ohos targets (musl 1.2 switched
// it to 64-bit) and this crate compiles with `-D warnings` on the cross-target
// checks, so the deprecation broke niubash's CI for every PR. The value domain
// here is an epoch second count that `localtime_r` consumes unchanged, so the
// switch itself is not a behaviour change for us; allow it locally rather than
// inventing a target-specific alias.
#[allow(deprecated)]
fn system_local_time(epoch: i64) -> LocalTimeParts {
    let stamp = epoch as libc::time_t;
    let mut broken: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `stamp` and `broken` are valid owned values; localtime_r only
    // reads the first and writes the second, returning null on failure.
    let ok = unsafe { libc::localtime_r(&stamp, &mut broken) };
    if ok.is_null() {
        let mut fallback = epoch_to_parts(epoch);
        fallback.epoch = epoch;
        return fallback;
    }
    let zone_name = if broken.tm_zone.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(broken.tm_zone) }
            .to_string_lossy()
            .into_owned()
    };
    LocalTimeParts {
        year: broken.tm_year + 1900,
        month: (broken.tm_mon + 1) as u8,
        day: broken.tm_mday as u8,
        hour: broken.tm_hour as u8,
        minute: broken.tm_min as u8,
        second: broken.tm_sec as u8,
        weekday: (broken.tm_wday as u8) % 7,
        zone_name,
        offset: broken.tm_gmtoff as i32,
        epoch,
    }
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
