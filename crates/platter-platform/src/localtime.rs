//! The local time zone's offset from UTC, read from the TZif files every Linux has (`/etc/localtime`, or
//! `$TZ`), without a dependency. Used where a file format asks for local time (the trash's
//! `DeletionDate`).
//!
//! The transitions in the file decide; after the last one (compact files stop early) the POSIX rule in
//! the file's footer decides, for the usual `Mm.w.d` form (`CET-1CEST,M3.5.0,M10.5.0/3`). Anything that
//! cannot be read gives UTC.
//!
//! On Windows there are no such files: the offset comes from the system's time zone, read through `jiff`
//! (the zone Windows is set to, with its rules). The TZif reader below is then not used, and still builds
//! and is tested there.

#![cfg_attr(windows, allow(dead_code))]

use std::path::{Component, Path, PathBuf};

const ZONEINFO: &str = "/usr/share/zoneinfo";
/// Larger is not a time zone file.
const MAX_TZIF_BYTES: u64 = 1 << 20;

/// Seconds to add to UTC to get local time at `at` (unix seconds). 0 when unknown.
#[cfg(windows)]
pub fn local_offset(at: i64) -> i64 {
    jiff::Timestamp::from_second(at)
        .map(|timestamp| i64::from(jiff::tz::TimeZone::system().to_offset(timestamp).seconds()))
        .unwrap_or(0)
}

/// Seconds to add to UTC to get local time at `at` (unix seconds). 0 when unknown.
#[cfg(not(windows))]
pub fn local_offset(at: i64) -> i64 {
    let from_file = zone_file()
        .and_then(|path| read_capped(&path))
        .and_then(|bytes| offset_from_tzif(&bytes, at));

    // `TZ` may also be a rule itself (`CET-1CEST,M3.5.0,M10.5.0/3`) rather than a zone name.
    from_file
        .or_else(|| {
            let tz = std::env::var("TZ").ok()?;
            posix_offset(tz.strip_prefix(':').unwrap_or(&tz), at)
        })
        .unwrap_or(0)
}

/// Unix seconds as local `YYYY-MM-DDThh:mm:ss`.
pub fn local_timestamp(at: i64) -> String {
    let local = at.saturating_add(local_offset(at));
    let days = local.div_euclid(86_400);
    let rest = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

fn zone_file() -> Option<PathBuf> {
    let Some(tz) = std::env::var_os("TZ") else {
        return Some(PathBuf::from("/etc/localtime"));
    };
    let tz = tz.to_str()?;
    let name = tz.strip_prefix(':').unwrap_or(tz);
    if name.is_empty() {
        return None;
    }

    let path = Path::new(name);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    // A zone name such as `Europe/Berlin`; nothing that could leave the zone folder.
    if path.components().all(|part| matches!(part, Component::Normal(_))) {
        return Some(Path::new(ZONEINFO).join(path));
    }
    None
}

fn read_capped(path: &Path) -> Option<Vec<u8>> {
    crate::read_regular_file(path, MAX_TZIF_BYTES).ok()
}

/// The UT offset at `at` from a TZif file (RFC 8536), or `None` when it cannot be read.
fn offset_from_tzif(bytes: &[u8], at: i64) -> Option<i64> {
    let header = Header::read(bytes, 0)?;
    let (body, header, wide) = if header.version >= b'2' {
        let second = 44 + header.v1_len();
        (second + 44, Header::read(bytes, second)?, true)
    } else {
        (44, header, false)
    };

    let time_size = if wide { 8 } else { 4 };
    let times = body;
    let indexes = times + header.timecnt * time_size;
    let types = indexes + header.timecnt;
    let data_end = types
        + header.typecnt * 6
        + header.charcnt
        + header.leapcnt * (time_size + 4)
        + header.isstdcnt
        + header.isutcnt;
    if data_end > bytes.len() || header.typecnt == 0 {
        return None;
    }

    let utoff = |index: usize| -> Option<i64> {
        let at = types + index * 6;
        Some(i64::from(i32::from_be_bytes(
            bytes.get(at..at + 4)?.try_into().ok()?,
        )))
    };
    let time = |index: usize| -> Option<i64> {
        let at = times + index * time_size;
        Some(if wide {
            i64::from_be_bytes(bytes.get(at..at + 8)?.try_into().ok()?)
        } else {
            i64::from(i32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
        })
    };

    let count = header.timecnt;
    let last_time = if count > 0 { time(count - 1)? } else { i64::MIN };
    // After the last transition the footer's rule decides; without a usable one, the table still does.
    if wide
        && at >= last_time
        && let Some(offset) = footer(bytes, data_end).and_then(|text| posix_offset(text, at))
    {
        return Some(offset);
    }

    // The last transition at or before `at`; before the first one, the first type.
    let mut low = 0;
    let mut high = count;
    while low < high {
        let middle = (low + high) / 2;
        if time(middle)? <= at {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    let kind = if low == 0 {
        0
    } else {
        usize::from(*bytes.get(indexes + low - 1)?)
    };
    if kind >= header.typecnt {
        return None;
    }
    utoff(kind)
}

/// The POSIX TZ rule after the data of a version 2+ file, between two newlines.
fn footer(bytes: &[u8], data_end: usize) -> Option<&str> {
    let rest = bytes.get(data_end..)?.strip_prefix(b"\n")?;
    let line = rest.split(|&byte| byte == b'\n').next()?;
    std::str::from_utf8(line).ok().filter(|text| !text.is_empty())
}

struct Header {
    version: u8,
    isutcnt: usize,
    isstdcnt: usize,
    leapcnt: usize,
    timecnt: usize,
    typecnt: usize,
    charcnt: usize,
}

impl Header {
    fn read(bytes: &[u8], at: usize) -> Option<Self> {
        let header = bytes.get(at..at + 44)?;
        if &header[..4] != b"TZif" {
            return None;
        }
        let count = |index: usize| -> Option<usize> {
            let start = 20 + index * 4;
            usize::try_from(u32::from_be_bytes(header[start..start + 4].try_into().ok()?)).ok()
        };

        Some(Self {
            version: header[4],
            isutcnt: count(0)?,
            isstdcnt: count(1)?,
            leapcnt: count(2)?,
            timecnt: count(3)?,
            typecnt: count(4)?,
            charcnt: count(5)?,
        })
    }

    /// Length of the version 1 data block (32-bit times).
    fn v1_len(&self) -> usize {
        self.timecnt * 5 + self.typecnt * 6 + self.charcnt + self.leapcnt * 8 + self.isstdcnt + self.isutcnt
    }
}

/// The UT offset at `at` from a POSIX TZ string such as `CET-1CEST,M3.5.0,M10.5.0/3`. Only the `Mm.w.d`
/// rule form is understood; a zone without daylight saving needs no rule.
fn posix_offset(text: &str, at: i64) -> Option<i64> {
    let mut rest = text;
    skip_name(&mut rest)?;
    let std = -parse_offset(&mut rest)?;
    if rest.is_empty() {
        return Some(std);
    }

    skip_name(&mut rest)?;
    let dst = if rest.starts_with(',') {
        std + 3600
    } else {
        -parse_offset(&mut rest)?
    };
    let rest = rest.strip_prefix(',')?;
    let (start, end) = rest.split_once(',')?;

    let (year, _, _) = civil_from_days((at + std).div_euclid(86_400));
    let starts = rule_time(start, year)? - std;
    let ends = rule_time(end, year)? - dst;
    let in_dst = if starts < ends {
        at >= starts && at < ends
    } else {
        at >= starts || at < ends
    };

    Some(if in_dst { dst } else { std })
}

fn skip_name(rest: &mut &str) -> Option<()> {
    if let Some(quoted) = rest.strip_prefix('<') {
        let end = quoted.find('>')?;
        *rest = &quoted[end + 1..];
        return Some(());
    }
    let length = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    if length < 3 {
        return None;
    }
    *rest = &rest[length..];
    Some(())
}

/// `[+-]hh[:mm[:ss]]` in seconds, as written (positive west of Greenwich).
fn parse_offset(rest: &mut &str) -> Option<i64> {
    let (sign, body) = match rest.as_bytes().first()? {
        b'-' => (-1, &rest[1..]),
        b'+' => (1, &rest[1..]),
        _ => (1, *rest),
    };
    let length = body
        .find(|c: char| !(c.is_ascii_digit() || c == ':'))
        .unwrap_or(body.len());
    let mut seconds: i64 = 0;
    for (index, part) in body[..length].split(':').enumerate() {
        // POSIX allows up to 167 hours; anything longer is not a rule.
        if part.is_empty() || part.len() > 3 {
            return None;
        }
        let value: i64 = part.parse().ok()?;
        seconds = seconds.checked_add(value.checked_mul(*[3600, 60, 1].get(index)?)?)?;
    }
    *rest = &body[length..];
    Some(sign * seconds)
}

/// Local seconds since the epoch at which the rule `Mm.w.d[/time]` happens in `year`.
fn rule_time(rule: &str, year: i64) -> Option<i64> {
    let (date, time) = rule.split_once('/').unwrap_or((rule, "2"));
    let mut fields = date.strip_prefix('M')?.split('.');
    let month: i64 = fields.next()?.parse().ok()?;
    let week: i64 = fields.next()?.parse().ok()?;
    let weekday: i64 = fields.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=5).contains(&week) || !(0..=6).contains(&weekday) {
        return None;
    }

    let first = days_from_civil(year, month, 1);
    let first_weekday = (first + 4).rem_euclid(7);
    let mut day = 1 + (weekday - first_weekday).rem_euclid(7) + (week - 1) * 7;
    while day > days_in_month(year, month) {
        day -= 7;
    }

    let mut time = time;
    let seconds = parse_offset(&mut time)?;
    Some((first + day - 1) * 86_400 + seconds)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    let next = if month == 12 {
        days_from_civil(year + 1, 1, 1)
    } else {
        days_from_civil(year, month + 1, 1)
    };
    next - days_from_civil(year, month, 1)
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day number (inverse of [`days_from_civil`]).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-07-01 12:00 UTC and 2026-01-15 12:00 UTC.
    const SUMMER: i64 = 1_782_907_200;
    const WINTER: i64 = 1_768_478_400;

    #[test]
    fn dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(days_from_civil(2000, 2, 29)), (2000, 2, 29));
        assert_eq!(civil_from_days(days_from_civil(2026, 11, 16)), (2026, 11, 16));
    }

    #[test]
    fn the_posix_rule_of_central_europe() {
        let rule = "CET-1CEST,M3.5.0,M10.5.0/3";

        assert_eq!(posix_offset(rule, SUMMER), Some(7200));
        assert_eq!(posix_offset(rule, WINTER), Some(3600));
        // 2026-03-29 01:00 UTC is 03:00 CEST, the first second of summer time; a second before is winter.
        let switch = days_from_civil(2026, 3, 29) * 86_400 + 3600;
        assert_eq!(posix_offset(rule, switch), Some(7200));
        assert_eq!(posix_offset(rule, switch - 1), Some(3600));
    }

    #[test]
    fn southern_and_plain_zones() {
        let sydney = "AEST-10AEDT,M10.1.0,M4.1.0/3";
        assert_eq!(
            posix_offset(sydney, WINTER),
            Some(11 * 3600),
            "January is summer there"
        );
        assert_eq!(posix_offset(sydney, SUMMER), Some(10 * 3600));
        assert_eq!(posix_offset("UTC0", SUMMER), Some(0));
        assert_eq!(posix_offset("<+0530>-5:30", SUMMER), Some(5 * 3600 + 1800));
        assert_eq!(posix_offset("EST5EDT,M3.2.0,M11.1.0", WINTER), Some(-5 * 3600));
    }

    #[test]
    fn a_real_zone_file_agrees_with_its_rule_when_present() {
        let Some(bytes) = read_capped(Path::new("/usr/share/zoneinfo/Europe/Berlin")) else {
            return;
        };

        assert_eq!(offset_from_tzif(&bytes, SUMMER), Some(7200));
        assert_eq!(offset_from_tzif(&bytes, WINTER), Some(3600));
        assert_eq!(
            offset_from_tzif(&bytes, 4_102_444_800 + 180 * 86_400),
            Some(7200),
            "2100, from the footer"
        );
    }

    #[test]
    fn a_file_without_a_footer_still_answers_from_its_table() {
        let Some(bytes) = read_capped(Path::new("/usr/share/zoneinfo/Europe/Berlin")) else {
            return;
        };
        let footer_at = bytes
            .iter()
            .rposition(|&b| b == b'\n')
            .and_then(|end| bytes[..end].iter().rposition(|&b| b == b'\n'));
        let Some(start) = footer_at else { return };
        let mut cut = bytes[..start].to_vec();
        cut.extend_from_slice(b"\n\n");

        assert_eq!(
            offset_from_tzif(&cut, SUMMER),
            Some(7200),
            "the transitions reach 2037"
        );
        assert!(
            offset_from_tzif(&cut, 4_102_444_800).is_some(),
            "past them, the last type"
        );
    }

    #[test]
    fn a_hostile_rule_never_overflows() {
        assert_eq!(posix_offset("AAA99999999999999999", SUMMER), None);
        assert_eq!(posix_offset("AAA-1BBB,M3.5.0/99999999999,M10.5.0", SUMMER), None);
        assert_eq!(
            posix_offset("AAA-1BBB,M3.5.0/167,M10.5.0", SUMMER),
            Some(7200),
            "167 h is allowed"
        );
    }

    #[test]
    fn garbage_is_not_a_zone() {
        assert_eq!(offset_from_tzif(b"not a zone file", SUMMER), None);
        assert_eq!(offset_from_tzif(b"TZif2", SUMMER), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_offset_is_the_systems_zone_and_the_day_it_makes_is_the_local_one() {
        for at in [SUMMER, WINTER] {
            let offset = local_offset(at);

            // Every zone in use is within -12 h and +14 h, in whole quarter hours.
            assert!((-12 * 3600..=14 * 3600).contains(&offset), "{offset}");
            assert_eq!(offset % 900, 0, "{offset}");

            // The local day and time are what the system's zone says for that moment.
            let local = jiff::tz::TimeZone::system().to_datetime(jiff::Timestamp::from_second(at).unwrap());
            assert_eq!(
                local_timestamp(at),
                local.strftime("%Y-%m-%dT%H:%M:%S").to_string()
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_time_the_calendar_cannot_hold_falls_back_to_utc() {
        assert_eq!(local_offset(i64::MAX), 0);
        assert_eq!(local_offset(i64::MIN), 0);
    }
}
