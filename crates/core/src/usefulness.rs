//! Useful time. Each hour of the week counts at the usefulness its owner gave
//! it: an override over every span, then the most specific span naming the
//! hour, then the baseline (vision energy-nexus-v1, "Pace").
//!
//! Time is weighed in percent-seconds, exactly: a second at usefulness 100 is
//! 100, so an hour at full use is 360 000.

use crate::Configuration;
use jiff::tz::TimeZone;
use jiff::Timestamp;
use signal_energy_nexus::{Baseline, Decimal, Override, Span, Weekday};
use std::cmp::Reverse;

/// One hour at full use, in percent-seconds.
const FULL_HOUR: i128 = 100 * 3_600;
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

/// The rules a setting of the schedule keeps, on `Configure` and on every
/// memory read or written.
pub trait ScheduleChecking {
    fn check(&self) -> Result<(), String>;
}
fn percent(value: i64, what: &str) -> Result<(), String> {
    if (0..=100).contains(&value) {
        Ok(())
    } else {
        Err(format!("{what} must be between 0 and 100"))
    }
}
impl ScheduleChecking for Span {
    /// At least one day; hours 0 to 23, From and To apart. A span whose To
    /// is before its From runs past midnight into the next day.
    fn check(&self) -> Result<(), String> {
        if self.weekday_vector.is_empty() {
            return Err("a span names at least one day".into());
        }
        if !(0..=23).contains(&self.from.0) || !(0..=23).contains(&self.to.0) {
            return Err("a span's hours are between 0 and 23".into());
        }
        if self.from.0 == self.to.0 {
            return Err("a span's From and To are different hours".into());
        }
        percent(self.usefulness.0 .0, "usefulness")
    }
}
impl ScheduleChecking for Override {
    fn check(&self) -> Result<(), String> {
        if self.from.0 >= self.to.0 {
            return Err("an override's From is before its To".into());
        }
        percent(self.usefulness.0 .0, "usefulness")
    }
}
impl ScheduleChecking for Baseline {
    fn check(&self) -> Result<(), String> {
        percent(self.0 .0, "the baseline")
    }
}

/// What a span says of the hours of the week.
pub trait SpanReading {
    /// Its days in week order, each once: two spans naming the same days and
    /// hours at the same usefulness are one span.
    fn normalized(&self) -> Span;
    /// Whether it names this hour of this day, Monday being day 0.
    fn names(&self, weekday: i64, hour: i64) -> bool;
    /// How many days it names.
    fn day_count(&self) -> usize;
    /// How many hours of a day it names.
    fn hour_count(&self) -> i64;
}
fn day_index(weekday: &Weekday) -> i64 {
    match weekday {
        Weekday::Monday => 0,
        Weekday::Tuesday => 1,
        Weekday::Wednesday => 2,
        Weekday::Thursday => 3,
        Weekday::Friday => 4,
        Weekday::Saturday => 5,
        Weekday::Sunday => 6,
    }
}
impl SpanReading for Span {
    fn normalized(&self) -> Span {
        let mut weekday_vector = self.weekday_vector.clone();
        weekday_vector.sort_by_key(day_index);
        weekday_vector.dedup();
        Span {
            weekday_vector,
            ..self.clone()
        }
    }
    fn names(&self, weekday: i64, hour: i64) -> bool {
        let on = |day: i64| self.weekday_vector.iter().any(|named| day_index(named) == day);
        let (from, to) = (self.from.0, self.to.0);
        if from < to {
            from <= hour && hour < to && on(weekday)
        } else {
            (hour >= from && on(weekday)) || (hour < to && on((weekday + 6) % 7))
        }
    }
    fn day_count(&self) -> usize {
        self.normalized().weekday_vector.len()
    }
    fn hour_count(&self) -> i64 {
        (self.to.0 - self.from.0).rem_euclid(24)
    }
}

/// The schedule of one configuration, read in the owner's time zone.
pub struct UsefulTime<'a> {
    schedule: &'a [Span],
    overrides: &'a [Override],
    baseline: i64,
    zone: &'a TimeZone,
}
impl<'a> UsefulTime<'a> {
    pub fn new(configuration: &'a Configuration, zone: &'a TimeZone) -> Self {
        Self {
            schedule: &configuration.schedule,
            overrides: &configuration.overrides,
            baseline: configuration.baseline.0 .0,
            zone,
        }
    }
    /// The usefulness that counts at an instant (Unix seconds), and the
    /// next instant at which it may change: the next local hour, or the
    /// next edge of an override.
    pub fn at(&self, instant: i64) -> (i64, i64) {
        let offset = Timestamp::from_second(instant)
            .map(|timestamp| i64::from(self.zone.to_offset(timestamp).seconds()))
            .unwrap_or(0);
        let local = instant + offset;
        let next = self
            .overrides
            .iter()
            .flat_map(|stretch| [stretch.from.0, stretch.to.0])
            .filter(|edge| *edge > instant)
            .fold(instant + HOUR - local.rem_euclid(HOUR), i64::min);
        let usefulness = self
            .overrides
            .iter()
            .enumerate()
            .filter(|(_, stretch)| stretch.from.0 <= instant && instant < stretch.to.0)
            .min_by_key(|(index, stretch)| (stretch.to.0 - stretch.from.0, Reverse(*index)))
            .map(|(_, stretch)| stretch.usefulness.0 .0)
            .unwrap_or_else(|| {
                // 1970-01-01, day 0, was a Thursday: day 3 of a week that begins on Monday.
                let weekday = (local.div_euclid(DAY) + 3).rem_euclid(7);
                let hour = local.rem_euclid(DAY) / HOUR;
                self.schedule
                    .iter()
                    .enumerate()
                    .filter(|(_, span)| span.names(weekday, hour))
                    .min_by_key(|(index, span)| {
                        (span.day_count(), span.hour_count(), Reverse(*index))
                    })
                    .map(|(_, span)| span.usefulness.0 .0)
                    .unwrap_or(self.baseline)
            });
        (usefulness, next)
    }
    /// The time from one instant to another, each second weighed at its
    /// usefulness, in percent-seconds; nothing when `to` is not after `from`.
    pub fn weight(&self, from: i64, to: i64) -> i128 {
        let mut weight = 0;
        let mut at = from;
        while at < to {
            let (usefulness, next) = self.at(at);
            let end = next.min(to);
            weight += i128::from(usefulness) * i128::from(end - at);
            at = end;
        }
        weight
    }
    /// A weight in useful hours, to a tenth.
    pub fn hours(weight: i128) -> Decimal {
        let tenths = (weight * 10 + FULL_HOUR / 2) / FULL_HOUR;
        Decimal::try_from(tenths as f64 / 10.0).unwrap_or_default()
    }
    /// The percent left per useful hour left, to a hundredth. With no useful
    /// hour left, the whole percent left.
    pub fn per_useful_hour(percent_left: i64, weight: i128) -> Decimal {
        let left = i128::from(percent_left.max(0));
        let hundredths = match weight {
            0 => left * 100,
            weight => (left * 100 * FULL_HOUR * 2 + weight) / (2 * weight),
        };
        Decimal::try_from(hundredths as f64 / 100.0).unwrap_or_default()
    }
}
