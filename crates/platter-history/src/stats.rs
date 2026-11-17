//! What the History view shows, worked out from stored samples and cleanups. Pure: the caller gives
//! the clock (`now`) and the zone (`offset`), so a test can say what day it is.

use crate::day::{day_number, day_text, parse_day};
use crate::store::{Sample, StoredCleanup};

/// Fewer samples than this, or samples spanning fewer days, are too thin to say how fast the disk grows.
pub const MIN_DAYS: usize = 7;

/// The most days a window may cover.
pub const MAX_WINDOW_DAYS: u32 = 366;

/// The numbers of one window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// Today, `YYYY-MM-DD`.
    pub today: String,
    /// The samples inside the window, oldest first.
    pub samples: Vec<Sample>,
    /// What the used space would have been without the cleanups, one value for each sample.
    pub without: Vec<u64>,
    /// The cleanups inside the window, newest first.
    pub cleanups: Vec<StoredCleanup>,
    /// The sum of what they freed (see `docs/architecture.md`, "History").
    pub freed: u64,
    pub thin: bool,
    /// Bytes per day, rounded; `None` when thin or when the disk does not grow.
    pub growth_per_day: Option<u64>,
    /// Whole days until the disk is full at that rate; `None` when `growth_per_day` is.
    pub days_until_full: Option<u64>,
}

/// The first local day of a window of `days` days ending on the day of `now`.
pub fn first_day(now: i64, offset: i64, days: u32) -> i64 {
    day_number(now, offset) - i64::from(days.clamp(1, MAX_WINDOW_DAYS)) + 1
}

/// The earliest Unix time a cleanup of the window can have, loosened by a day on each side of the
/// zone's changes: [`summarize`] makes the exact cut.
pub fn window_start(now: i64, offset: i64, days: u32) -> i64 {
    (first_day(now, offset, days) - 1) * 86_400 - offset
}

/// Works out the window of the last `days` days. `samples` and `cleanups` may reach further back
/// (`samples` oldest first, `cleanups` newest first): what lies outside the window is left out.
pub fn summarize(
    now: i64,
    offset: i64,
    days: u32,
    samples: &[Sample],
    cleanups: &[StoredCleanup],
) -> Summary {
    let today = day_number(now, offset);
    let first = first_day(now, offset, days);
    let inside = |day: i64| (first..=today).contains(&day);

    let samples: Vec<Sample> = samples
        .iter()
        .filter(|sample| parse_day(&sample.day).is_some_and(inside))
        .cloned()
        .collect();
    let cleanups: Vec<StoredCleanup> = cleanups
        .iter()
        .filter(|cleanup| inside(day_number(cleanup.at, offset)))
        .cloned()
        .collect();

    let without = without_cleanups(&samples, &cleanups);
    let freed = cleanups
        .iter()
        .map(|cleanup| cleanup.freed)
        .fold(0, u64::saturating_add);
    let thin = is_thin(&samples);
    let (growth_per_day, days_until_full) = if thin {
        (None, None)
    } else {
        growth(&samples, &without)
    };

    Summary {
        today: day_text(today),
        samples,
        without,
        cleanups,
        freed,
        thin,
        growth_per_day,
        days_until_full,
    }
}

/// The used space of each sample plus what the cleanups after the first sample freed up to it.
fn without_cleanups(samples: &[Sample], cleanups: &[StoredCleanup]) -> Vec<u64> {
    let Some(first) = samples.first() else {
        return Vec::new();
    };

    samples
        .iter()
        .map(|sample| {
            let freed = cleanups
                .iter()
                .filter(|cleanup| cleanup.at > first.at && cleanup.at <= sample.at)
                .map(|cleanup| cleanup.freed)
                .fold(0, u64::saturating_add);

            sample.used.saturating_add(freed)
        })
        .collect()
}

/// Fewer than [`MIN_DAYS`] samples, or samples spanning fewer than [`MIN_DAYS`] days.
fn is_thin(samples: &[Sample]) -> bool {
    if samples.len() < MIN_DAYS {
        return true;
    }

    // The days from the first to the last one, both counted.
    span_days(samples).is_none_or(|span| span + 1 < MIN_DAYS as i64)
}

/// Days from the first sample's day to the last one's.
fn span_days(samples: &[Sample]) -> Option<i64> {
    let first = parse_day(&samples.first()?.day)?;
    let last = parse_day(&samples.last()?.day)?;

    Some(last - first)
}

fn growth(samples: &[Sample], without: &[u64]) -> (Option<u64>, Option<u64>) {
    let (Some(span), Some(last), Some(first_without), Some(last_without)) = (
        span_days(samples),
        samples.last(),
        without.first(),
        without.last(),
    ) else {
        return (None, None);
    };
    if span <= 0 {
        return (None, None);
    }

    let grown = i128::from(*last_without) - i128::from(*first_without);
    if grown <= 0 {
        return (None, None);
    }
    let per_day = grown as f64 / span as f64;
    // Too small to be a whole byte a day is no growth worth a forecast.
    let rounded = per_day.round();
    if rounded < 1.0 {
        return (None, None);
    }

    let room = last.total.saturating_sub(last.used) as f64;
    let days_until_full = (room / per_day).floor();

    (Some(rounded as u64), Some(days_until_full as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Method, MethodKind};

    const DAY: i64 = 86_400;
    /// 2026-11-17 12:00 UTC.
    const NOW: i64 = 1_794_916_800;

    /// A sample taken at noon, `back` days before `NOW`.
    fn sample(back: i64, used: u64) -> Sample {
        let at = NOW - back * DAY;
        Sample {
            day: day_text(day_number(at, 0)),
            at,
            total: 1000,
            used,
        }
    }

    fn cleanup(back: i64, hours: i64, freed: u64) -> StoredCleanup {
        StoredCleanup {
            id: 1,
            at: NOW - back * DAY + hours * 3600,
            freed,
            trashed: 0,
            what: vec!["Docker".to_owned()],
            methods: vec![Method {
                kind: MethodKind::Deleted,
                command: None,
            }],
            item_count: 0,
            unrestored: 0,
            gone: 0,
            items: Vec::new(),
        }
    }

    #[test]
    fn only_the_days_of_the_window_count() {
        let samples = [sample(40, 100), sample(5, 200), sample(0, 250)];
        let cleanups = [cleanup(1, 1, 30), cleanup(60, 0, 999)];

        let summary = summarize(NOW, 0, 10, &samples, &cleanups);

        assert_eq!(summary.today, "2026-11-17");
        assert_eq!(summary.samples.len(), 2);
        assert_eq!(summary.cleanups.len(), 1);
        assert_eq!(summary.freed, 30);
    }

    #[test]
    fn without_cleanups_adds_what_was_freed_since_the_first_sample() {
        let samples = [sample(4, 500), sample(3, 520), sample(2, 440), sample(0, 460)];
        // 60 freed on day -3 after its sample, 25 freed on day 0 before its sample, 70 freed before the
        // first sample (already part of it).
        let cleanups = [cleanup(0, -3, 25), cleanup(3, 2, 60), cleanup(4, -4, 70)];

        let summary = summarize(NOW, 0, 30, &samples, &cleanups);

        assert_eq!(summary.without, [500, 520, 500, 545]);
        assert_eq!(summary.without[0], samples[0].used, "the lines start together");
    }

    #[test]
    fn without_a_sample_nothing_is_invented() {
        let summary = summarize(NOW, 0, 30, &[], &[cleanup(2, 0, 10)]);

        assert!(summary.samples.is_empty() && summary.without.is_empty());
        assert!(summary.thin);
        assert_eq!((summary.growth_per_day, summary.days_until_full), (None, None));
        assert_eq!(summary.freed, 10, "the cleanup is still a cleanup");
    }

    #[test]
    fn a_few_days_are_thin_and_give_no_rate() {
        let few: Vec<Sample> = (0..6)
            .map(|back| sample(back, 100 + 10 * (6 - back as u64)))
            .collect();
        let summary = summarize(NOW, 0, 30, &few, &[]);

        assert!(summary.thin);
        assert_eq!((summary.growth_per_day, summary.days_until_full), (None, None));

        // Seven samples that all fall within three days are thin too.
        let same_days: Vec<Sample> = (0..7)
            .map(|index| Sample {
                at: NOW - 2 * DAY + index,
                day: day_text(day_number(NOW - 2 * DAY + index * 20_000, 0)),
                total: 1000,
                used: 100 + index as u64,
            })
            .collect();
        let narrow = summarize(NOW, 0, 30, &same_days, &[]);
        assert!(narrow.thin);
    }

    #[test]
    fn the_rate_and_the_time_to_full_come_from_the_series_without_cleanups() {
        // Seven daily samples growing by 10 a day, one 40 freed in between: the disk would have grown
        // by 10 a day plus what was freed.
        let samples: Vec<Sample> = (0..7)
            .map(|index| sample(6 - index, 100 + 10 * index as u64))
            .collect();
        let summary = summarize(NOW, 0, 30, &samples, &[]);

        assert!(!summary.thin);
        assert_eq!(summary.growth_per_day, Some(10));
        // 1000 total, 160 used on the last day: 840 left at 10 a day.
        assert_eq!(summary.days_until_full, Some(84));

        let freed = [cleanup(3, 6, 60)];
        let with_cleanup = summarize(NOW, 0, 30, &samples, &freed);
        assert_eq!(
            with_cleanup.growth_per_day,
            Some(20),
            "60 freed over 6 days adds 10 a day"
        );
    }

    #[test]
    fn a_disk_that_does_not_grow_has_no_forecast() {
        let samples: Vec<Sample> = (0..8)
            .map(|index| sample(7 - index, 500 - 5 * index as u64))
            .collect();

        let summary = summarize(NOW, 0, 30, &samples, &[]);

        assert!(!summary.thin);
        assert_eq!((summary.growth_per_day, summary.days_until_full), (None, None));
    }

    #[test]
    fn the_local_day_decides_what_is_today() {
        let late = NOW + 11 * 3600 + 1800; // 23:30 UTC on the 17th
        let samples = [sample(0, 1)];

        let utc = summarize(late, 0, 1, &samples, &[]);
        let warsaw = summarize(late, 3600, 1, &samples, &[]);

        assert_eq!(utc.today, "2026-11-17");
        assert_eq!(warsaw.today, "2026-11-18");
        assert_eq!(utc.samples.len(), 1);
        assert!(warsaw.samples.is_empty(), "the sample is from the day before");
    }
}
