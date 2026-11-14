//! Pace in useful hours, against the worked examples of vision
//! energy-nexus-v1, "Pace: over or under, per day". The owner's local time is
//! fixed at UTC-6 (America/Mexico_City, which keeps no daylight saving since
//! 2022), where 13 November 2026 16:50 is 1794610200.

use energy_nexus_core::{Configuring, Memory, Pacing, TimeZone, UsefulTime};
use meta_signal_energy_nexus::Set;
use signal_energy_nexus::{
    Baseline, Decimal, Decimating, Hour, Hours, Instant, Minutes, Name, Override, Pace, Percent, Provider,
    Reading, Span, Standing, Target, Usefulness, Weekday, Window,
};

fn zone() -> TimeZone {
    TimeZone::fixed(jiff::tz::offset(-6))
}

/// An instant written in the owner's local time.
fn local(text: &str) -> i64 {
    local_in(text, &zone())
}

fn local_in(text: &str, zone: &TimeZone) -> i64 {
    text.parse::<jiff::civil::DateTime>()
        .unwrap()
        .to_zoned(zone.clone())
        .unwrap()
        .timestamp()
        .as_second()
}

const EVERY_DAY: [Weekday; 7] = [
    Weekday::Monday,
    Weekday::Tuesday,
    Weekday::Wednesday,
    Weekday::Thursday,
    Weekday::Friday,
    Weekday::Saturday,
    Weekday::Sunday,
];
const WEEKDAYS: [Weekday; 5] = [
    Weekday::Monday,
    Weekday::Tuesday,
    Weekday::Wednesday,
    Weekday::Thursday,
    Weekday::Friday,
];

fn span(days: &[Weekday], from: i64, to: i64, usefulness: i64) -> Span {
    Span {
        weekday_vector: days.to_vec(),
        from: Hour(from),
        to: Hour(to),
        usefulness: Usefulness(Percent(usefulness)),
    }
}

fn configured(sets: Vec<Set>) -> Memory {
    let mut memory = Memory::default();
    for set in sets {
        memory.configure(set).expect("the setting is admitted");
    }
    memory
}

fn baseline(percent: i64) -> Set {
    Set::Baseline(Baseline(Percent(percent)))
}

/// The usefulness that counts at a local time.
fn usefulness_at(memory: &Memory, text: &str) -> i64 {
    let zone = zone();
    UsefulTime::new(memory.configuration(), &zone).at(local(text)).0
}

fn decimal(value: f64) -> Decimal {
    Decimal::try_from(value).unwrap()
}

/// His Codex week: 35% used at 13 Nov 16:50, the week resetting 18 Nov 20:21.
fn codex_week() -> Reading {
    Reading {
        provider: Provider::Codex,
        allowed: true,
        window_vector: vec![Window {
            name: Name::Week,
            used: Percent(35),
            resets: Instant(1795054860),
            duration: Minutes(10080),
        }],
        reset_vector: Vec::new(),
        at: Instant(1794610200),
    }
}

/// `Configure.Baseline.20`, then
/// `Configure.Span.{ [ Monday … Sunday ] 8 22 100 }`.
fn every_day_eight_to_ten() -> Vec<Set> {
    vec![baseline(20), Set::Span(span(&EVERY_DAY, 8, 22, 100))]
}

#[test]
fn the_codex_week_reads_even_25_35_over_124_83_5_0_78() {
    assert_eq!(local("2026-11-13T16:50"), 1794610200);
    let memory = configured(every_day_eight_to_ten());
    let pace = codex_week().pace(memory.configuration(), &zone());
    assert_eq!(
        pace,
        vec![Pace {
            provider: Provider::Codex,
            name: Name::Week,
            target: Target::Even,
            expected: Percent(25),
            used: Percent(35),
            standing: Standing::Over,
            hours_left: Hours(124),
            useful_left: decimal(83.5),
            per_useful_hour: decimal(0.78),
        }]
    );
}

#[test]
fn the_same_energy_reads_lower_with_more_nights_ahead() {
    // Two windows at 50% used, each 36 hours from its reset: one with a day
    // and two nights ahead, one with two days and a night.
    let memory = configured(every_day_eight_to_ten());
    let reading = |at: &str| {
        let at = local(at);
        Reading {
            provider: Provider::Codex,
            allowed: true,
            window_vector: vec![Window {
                name: Name::Week,
                used: Percent(50),
                resets: Instant(at + 36 * 3600),
                duration: Minutes(10080),
            }],
            reset_vector: Vec::new(),
            at: Instant(at),
        }
    };
    // 20:00 to 08:00 two days on: two nights and one day ahead.
    let nights = reading("2026-11-13T20:00").pace(memory.configuration(), &zone()).remove(0);
    // 08:00 to 20:00 the next day: two days and one night ahead.
    let days = reading("2026-11-13T08:00").pace(memory.configuration(), &zone()).remove(0);
    assert_eq!(nights.hours_left, days.hours_left);
    assert!(nights.useful_left.float() < days.useful_left.float());
    assert!(nights.per_useful_hour.float() > days.per_useful_hour.float());
}

#[test]
fn one_who_keeps_the_weekend() {
    let memory = configured(vec![baseline(20), Set::Span(span(&WEEKDAYS, 9, 18, 100))]);
    assert_eq!(usefulness_at(&memory, "2026-11-16T10:00"), 100, "a Monday morning");
    assert_eq!(usefulness_at(&memory, "2026-11-16T08:59"), 20, "before nine");
    assert_eq!(usefulness_at(&memory, "2026-11-16T18:00"), 20, "To is the hour the span ends");
    assert_eq!(usefulness_at(&memory, "2026-11-14T10:00"), 20, "a Saturday, at the baseline");
}

#[test]
fn one_who_has_a_job() {
    let memory = configured(vec![
        baseline(20),
        Set::Span(span(&[Weekday::Tuesday, Weekday::Thursday], 19, 23, 100)),
        Set::Span(span(&[Weekday::Saturday, Weekday::Sunday], 9, 13, 50)),
    ]);
    assert_eq!(usefulness_at(&memory, "2026-11-17T20:00"), 100, "a Tuesday evening");
    assert_eq!(usefulness_at(&memory, "2026-11-18T20:00"), 20, "a Wednesday evening");
    assert_eq!(usefulness_at(&memory, "2026-11-19T22:59"), 100, "a Thursday evening");
    assert_eq!(usefulness_at(&memory, "2026-11-14T10:00"), 50, "a Saturday morning");
    assert_eq!(usefulness_at(&memory, "2026-11-15T13:00"), 20, "a Sunday afternoon");
}

#[test]
fn a_gradient_through_the_day() {
    let memory = configured(vec![
        baseline(20),
        Set::Span(span(&EVERY_DAY, 6, 10, 60)),
        Set::Span(span(&EVERY_DAY, 10, 14, 100)),
        Set::Span(span(&EVERY_DAY, 14, 22, 70)),
    ]);
    for (at, usefulness) in [
        ("2026-11-16T05:00", 20),
        ("2026-11-16T07:00", 60),
        ("2026-11-16T12:00", 100),
        ("2026-11-16T15:00", 70),
        ("2026-11-16T22:00", 20),
    ] {
        assert_eq!(usefulness_at(&memory, at), usefulness, "{at}");
    }
    // One day: 4 h at 60, 4 at 100, 8 at 70 and 8 at the baseline, 20.
    let zone = zone();
    let useful = UsefulTime::new(memory.configuration(), &zone);
    let day = useful.weight(local("2026-11-16T00:00"), local("2026-11-17T00:00"));
    assert_eq!(UsefulTime::hours(day), decimal(13.6));
}

#[test]
fn the_more_specific_span_counts() {
    // Fewer days counts over more days.
    let memory = configured(vec![
        Set::Span(span(&EVERY_DAY, 8, 22, 100)),
        Set::Span(span(&WEEKDAYS, 9, 18, 50)),
    ]);
    assert_eq!(usefulness_at(&memory, "2026-11-16T10:00"), 50, "a Monday: five days over seven");
    assert_eq!(usefulness_at(&memory, "2026-11-14T10:00"), 100, "a Saturday: only the seven");
    // As many days: fewer hours counts.
    let memory = configured(vec![
        Set::Span(span(&[Weekday::Monday], 10, 12, 90)),
        Set::Span(span(&[Weekday::Monday], 8, 22, 30)),
    ]);
    assert_eq!(usefulness_at(&memory, "2026-11-16T11:00"), 90);
    assert_eq!(usefulness_at(&memory, "2026-11-16T09:00"), 30);
    // The order they were configured in does not matter.
    let memory = configured(vec![
        Set::Span(span(&WEEKDAYS, 9, 18, 50)),
        Set::Span(span(&EVERY_DAY, 8, 22, 100)),
    ]);
    assert_eq!(usefulness_at(&memory, "2026-11-16T10:00"), 50);
}

#[test]
fn an_override_counts_over_every_span() {
    // The dentist: Monday 16 November, 14:00 to 16:00, at nothing.
    let dentist = Override {
        from: Instant(1794859200),
        to: Instant(1794866400),
        usefulness: Usefulness(Percent(0)),
    };
    assert_eq!(local("2026-11-16T14:00"), 1794859200);
    let mut sets = every_day_eight_to_ten();
    sets.push(Set::Span(span(&[Weekday::Monday], 14, 16, 100)));
    sets.push(Set::Override(dentist.clone()));
    let memory = configured(sets);
    assert_eq!(usefulness_at(&memory, "2026-11-16T13:59"), 100);
    assert_eq!(usefulness_at(&memory, "2026-11-16T14:00"), 0);
    assert_eq!(usefulness_at(&memory, "2026-11-16T15:30"), 0);
    assert_eq!(usefulness_at(&memory, "2026-11-16T16:00"), 100);
    // His week with the afternoon away: two useful hours fewer ahead.
    let pace = codex_week().pace(memory.configuration(), &zone()).remove(0);
    assert_eq!(pace.useful_left, decimal(81.5));
    assert_eq!(pace.per_useful_hour, decimal(0.8));
    assert_eq!(pace.expected, Percent(26));
    // Removed, the week reads as before.
    let mut memory = memory;
    memory.configure(Set::RemoveOverride(dentist)).unwrap();
    let pace = codex_week().pace(memory.configuration(), &zone()).remove(0);
    assert_eq!(pace.useful_left, decimal(83.5));
}

#[test]
fn an_override_that_does_not_fall_on_the_hour() {
    let memory = configured(vec![
        baseline(0),
        Set::Override(Override {
            from: Instant(local("2026-11-16T10:15")),
            to: Instant(local("2026-11-16T10:45")),
            usefulness: Usefulness(Percent(100)),
        }),
    ]);
    let zone = zone();
    let useful = UsefulTime::new(memory.configuration(), &zone);
    let weight = useful.weight(local("2026-11-16T00:00"), local("2026-11-17T00:00"));
    assert_eq!(UsefulTime::hours(weight), decimal(0.5));
}

#[test]
fn a_span_whose_to_is_before_its_from_runs_past_midnight() {
    let memory = configured(vec![
        baseline(20),
        Set::Span(span(&[Weekday::Friday], 22, 2, 80)),
    ]);
    assert_eq!(usefulness_at(&memory, "2026-11-13T23:00"), 80, "Friday night");
    assert_eq!(usefulness_at(&memory, "2026-11-14T01:30"), 80, "into Saturday");
    assert_eq!(usefulness_at(&memory, "2026-11-14T02:00"), 20);
    assert_eq!(usefulness_at(&memory, "2026-11-13T01:00"), 20, "Friday's own small hours");
}

#[test]
fn the_hours_are_the_owners_local_hours_through_daylight_saving() {
    // A zone that keeps daylight saving: 08:00 in July is 13:00 UTC, in
    // November 14:00 UTC.
    let central = TimeZone::posix("CST6CDT,M3.2.0,M11.1.0").unwrap();
    let memory = configured(vec![baseline(20), Set::Span(span(&EVERY_DAY, 8, 22, 100))]);
    let useful = UsefulTime::new(memory.configuration(), &central);
    let july = local_in("2026-07-01T08:00", &central);
    assert_eq!(july % 86_400, 13 * 3_600);
    assert_eq!(useful.at(july).0, 100);
    assert_eq!(useful.at(july - 1).0, 20);
    let november = local_in("2026-11-16T08:00", &central);
    assert_eq!(november % 86_400, 14 * 3_600);
    assert_eq!(useful.at(november).0, 100);
    assert_eq!(useful.at(november - 1).0, 20);
    // The day daylight saving ends has 25 hours, 14 of them useful.
    let day = useful.weight(
        local_in("2026-11-01T00:00", &central),
        local_in("2026-11-02T00:00", &central),
    );
    assert_eq!(UsefulTime::hours(day), decimal(16.2), "14 at 100 and 11 at 20");
}

#[test]
fn with_no_schedule_every_hour_counts_whole() {
    let memory = Memory::default();
    let pace = codex_week().pace(memory.configuration(), &zone()).remove(0);
    // 44.48 of 168 hours spent: 26%, as time counted plainly.
    assert_eq!(pace.expected, Percent(26));
    assert_eq!(pace.hours_left, Hours(124));
    assert_eq!(pace.useful_left, decimal(123.5));
    assert_eq!(pace.per_useful_hour, decimal(0.53));
}

#[test]
fn a_window_whose_every_hour_counts_nothing_counts_time_plainly() {
    let memory = configured(vec![baseline(0)]);
    let pace = codex_week().pace(memory.configuration(), &zone()).remove(0);
    assert_eq!(pace.expected, Percent(26));
    assert_eq!(pace.useful_left, decimal(0.0));
    assert_eq!(pace.per_useful_hour, decimal(65.0), "the whole percent left");
}

#[test]
fn a_spend_by_target_counts_its_useful_hours_to_the_instant() {
    let memory = configured(every_day_eight_to_ten());
    let mut reading = codex_week();
    reading.window_vector[0].used = Percent(92);
    // Spend the week whole by Sunday 15 November, 20:00.
    let by = local("2026-11-15T20:00");
    let mut memory = memory;
    memory
        .configure(Set::Target(meta_signal_energy_nexus::ProviderTarget {
            provider: Provider::Codex,
            target: Target::SpendBy(Instant(by)),
        }))
        .unwrap();
    let pace = reading.pace(memory.configuration(), &zone()).remove(0);
    // 13 Nov 16:50 to 15 Nov 20:00: 51.17 hours; 5.17 + 2 + 14 + 2 + 12 useful.
    assert_eq!(pace.hours_left, Hours(51));
    assert_eq!(pace.useful_left, decimal(35.2));
    assert_eq!(pace.per_useful_hour, decimal(0.23));
    assert_eq!(pace.standing, Standing::Over);
}

#[test]
fn the_schedule_keeps_its_rules() {
    let mut memory = Memory::default();
    for refused in [
        Set::Span(span(&[], 8, 22, 100)),
        Set::Span(span(&EVERY_DAY, 8, 24, 100)),
        Set::Span(span(&EVERY_DAY, -1, 8, 100)),
        Set::Span(span(&EVERY_DAY, 8, 8, 100)),
        Set::Span(span(&EVERY_DAY, 8, 22, 101)),
        Set::Override(Override {
            from: Instant(10),
            to: Instant(10),
            usefulness: Usefulness(Percent(0)),
        }),
        Set::Override(Override {
            from: Instant(10),
            to: Instant(20),
            usefulness: Usefulness(Percent(-1)),
        }),
        baseline(101),
    ] {
        assert!(memory.configure(refused.clone()).is_err(), "{refused:?}");
    }
    assert_eq!(memory.configuration(), Memory::default().configuration());
}

#[test]
fn a_span_is_one_span_whatever_order_its_days_are_named_in() {
    let mut memory = Memory::default();
    let applied = memory
        .configure(Set::Span(span(
            &[Weekday::Thursday, Weekday::Tuesday, Weekday::Thursday],
            19,
            23,
            100,
        )))
        .unwrap();
    assert_eq!(
        applied,
        Set::Span(span(&[Weekday::Tuesday, Weekday::Thursday], 19, 23, 100)),
        "the span as it is kept"
    );
    memory
        .configure(Set::Span(span(&[Weekday::Tuesday, Weekday::Thursday], 19, 23, 100)))
        .unwrap();
    assert_eq!(memory.configuration().schedule.len(), 1);
    memory
        .configure(Set::RemoveSpan(span(&[Weekday::Thursday, Weekday::Tuesday], 19, 23, 100)))
        .unwrap();
    assert!(memory.configuration().schedule.is_empty());
}
