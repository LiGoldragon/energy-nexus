//! A `SpendBy` target is stated against the window's reset: so many hours
//! before it, a percent of the window spent. It recurs with every reset.

use energy_nexus_core::{Configuring, Memory, Pacing, Validating};
use jiff::tz::TimeZone;
use meta_signal_energy_nexus::{ProviderTarget, Set, WindowTarget};
use signal_energy_nexus::{
    Allowed, At, Duration, Hours, Instant, Minutes, Name, Percent, Provider, Reading, Resets,
    SpendBy_Data, Standing, Target, Used, Window,
};

const NOW: Instant = Instant(0);
/// A week, in seconds.
const WEEK: i64 = 7 * 24 * 3600;

fn spend_by(hours: i64, percent: i64) -> Target {
    Target::SpendBy(SpendBy_Data {
        hours: Hours(hours),
        percent: Percent(percent),
    })
}

fn codex_target(target: Target) -> Memory {
    let mut memory = Memory::default();
    memory
        .configure(
            Set::Target(ProviderTarget {
                provider: Provider::Codex,
                target,
            }),
            NOW,
        )
        .expect("the target is admitted");
    memory
}

/// A Codex reading of one window, `elapsed` seconds into it.
fn reading(windows: &[(Name, i64, i64)], used: i64, at: i64) -> Reading {
    Reading {
        provider: Provider::Codex,
        allowed: Allowed(true),
        window_vector: windows
            .iter()
            .map(|(name, resets, minutes)| Window {
                name: name.clone(),
                used: Used(Percent(used)),
                resets: Resets(Instant(*resets)),
                duration: Duration(Minutes(*minutes)),
            })
            .collect(),
        reset_vector: Vec::new(),
        at: At(Instant(at)),
    }
}

#[test]
fn a_target_set_this_week_applies_after_the_reset_to_the_next_window() {
    // 92 percent spent 24 hours before each reset.
    let memory = codex_target(spend_by(24, 92));
    let zone = TimeZone::UTC;
    let first_reset = 10 * WEEK;
    let start = first_reset - WEEK;
    // Three days into the week: the target falls on day six, so half its way.
    let this_week = reading(&[(Name::Week, first_reset, 10080)], 30, start + 72 * 3600)
        .pace(memory.configuration(), &zone)
        .remove(0);
    assert_eq!(this_week.target, spend_by(24, 92));
    assert_eq!(this_week.hours_left.0, Hours(72));
    assert_eq!(this_week.expected.0, Percent(46), "half of 92 percent");
    assert_eq!(this_week.standing, Standing::Under);
    // The window resets, and the same configuration, never set again, holds
    // the same target for the next window: three days in, the same pace.
    let next_week = reading(
        &[(Name::Week, first_reset + WEEK, 10080)],
        30,
        start + WEEK + 72 * 3600,
    )
    .pace(memory.configuration(), &zone)
    .remove(0);
    assert_eq!(next_week, this_week);
    // An hour after the reset, in the new window, the target is 143 hours
    // away, not already past.
    let just_after = reading(
        &[(Name::Week, first_reset + WEEK, 10080)],
        0,
        first_reset + 3600,
    )
    .pace(memory.configuration(), &zone)
    .remove(0);
    assert_eq!(just_after.hours_left.0, Hours(143));
    assert_eq!(just_after.target, spend_by(24, 92));
    assert_eq!(just_after.standing, Standing::Under);
}

#[test]
fn between_the_target_and_the_reset_the_window_is_past_its_target() {
    let memory = codex_target(spend_by(24, 92));
    let reset = 10 * WEEK;
    let pace = reading(&[(Name::Week, reset, 10080)], 80, reset - 6 * 3600)
        .pace(memory.configuration(), &TimeZone::UTC)
        .remove(0);
    assert_eq!(pace.hours_left.0, Hours(0));
    assert_eq!(pace.expected.0, Percent(92), "the whole of the target is due");
    assert_eq!(pace.standing, Standing::Under);
}

#[test]
fn zero_hours_is_the_reset_itself() {
    let reset = 10 * WEEK;
    let at = reset - 3 * 24 * 3600;
    let target = reading(&[(Name::Week, reset, 10080)], 30, at)
        .pace(codex_target(spend_by(0, 100)).configuration(), &TimeZone::UTC)
        .remove(0);
    let even = reading(&[(Name::Week, reset, 10080)], 30, at)
        .pace(Memory::default().configuration(), &TimeZone::UTC)
        .remove(0);
    assert_eq!(target.expected, even.expected);
    assert_eq!(target.hours_left, even.hours_left);
}

#[test]
fn a_window_shorter_than_the_hours_is_paced_evenly() {
    // 24 hours before the reset: the week has them, the five-hour session does not.
    let memory = codex_target(spend_by(24, 92));
    let reset = 10 * WEEK;
    let paces = reading(
        &[(Name::Session, reset - WEEK + 3600, 300), (Name::Week, reset, 10080)],
        30,
        reset - WEEK + 72 * 3600,
    )
    .pace(memory.configuration(), &TimeZone::UTC);
    assert_eq!(paces[0].name, Name::Session);
    assert_eq!(paces[0].target, Target::Even);
    assert_eq!(paces[1].name, Name::Week);
    assert_eq!(paces[1].target, spend_by(24, 92));
}

#[test]
fn a_window_exactly_as_long_as_the_hours_is_paced_by_them() {
    let memory = codex_target(spend_by(5, 100));
    let reset = 10 * WEEK;
    let pace = reading(&[(Name::Session, reset, 300)], 0, reset - 5 * 3600)
        .pace(memory.configuration(), &TimeZone::UTC)
        .remove(0);
    assert_eq!(pace.target, spend_by(5, 100));
    assert_eq!(pace.hours_left.0, Hours(0));
}

#[test]
fn hours_before_the_reset_are_a_whole_number_from_none_up() {
    let mut memory = Memory::default();
    for refused in [
        Set::Target(ProviderTarget {
            provider: Provider::Codex,
            target: spend_by(-1, 50),
        }),
        Set::WindowTarget(WindowTarget {
            provider: Provider::Codex,
            name: Name::Week,
            target: spend_by(-24, 100),
        }),
        Set::Target(ProviderTarget {
            provider: Provider::Codex,
            target: spend_by(24, 101),
        }),
        Set::Target(ProviderTarget {
            provider: Provider::Codex,
            target: spend_by(24, -1),
        }),
    ] {
        assert!(memory.configure(refused.clone(), NOW).is_err(), "{refused:?}");
    }
    assert_eq!(memory.configuration(), Memory::default().configuration());
    for admitted in [spend_by(0, 0), spend_by(24, 92), spend_by(1000, 100)] {
        assert!(
            memory
                .configure(
                    Set::Target(ProviderTarget {
                        provider: Provider::Codex,
                        target: admitted.clone(),
                    }),
                    NOW
                )
                .is_ok(),
            "{admitted:?}"
        );
    }
    // A stored memory keeps the rule too: it is refused at the write.
    let mut stored = Memory::default();
    stored.configuration.targets.0[0].target = spend_by(-3, 100);
    assert!(stored.configuration.validate().is_err());
}
