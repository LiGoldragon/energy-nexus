//! A `SpendBy` target is stated against the window's reset: so many hours
//! before it, a percent of the window spent. It recurs with every reset. A
//! target governs both week windows, Week and FableWeek, and its hours are
//! bounded by the Week; the Session takes none, since the session is a ceiling
//! on the rate of work and a Session spent out is a fault in the pace.

use energy_nexus_core::{Configuring, Memory, Pacing, Remembering, Validating};
use jiff::tz::TimeZone;
use meta_signal_energy_nexus::{ConfigureRefused, ProviderTarget, Set};
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
fn a_target_applies_to_both_week_windows_and_not_the_session() {
    // 92 percent by 24 hours before the reset: the Week and Claude's
    // FableWeek have the target; the five-hour Session, whatever the hours,
    // has none.
    let memory = codex_target(spend_by(24, 92));
    let reset = 10 * WEEK;
    let paces = reading(
        &[
            (Name::Session, reset - WEEK + 3600, 300),
            (Name::Week, reset, 10080),
            (Name::FableWeek, reset, 10080),
        ],
        30,
        reset - WEEK + 72 * 3600,
    )
    .pace(memory.configuration(), &TimeZone::UTC);
    assert_eq!(paces[0].name, Name::Session);
    assert_eq!(paces[0].target, Target::Even);
    assert_eq!(paces[1].name, Name::Week);
    assert_eq!(paces[1].target, spend_by(24, 92));
    assert_eq!(paces[2].name, Name::FableWeek);
    assert_eq!(paces[2].target, spend_by(24, 92));
    // The two week windows, at the same usage and instant, are paced alike.
    assert_eq!(paces[1].expected, paces[2].expected);
    assert_eq!(paces[1].hours_left, paces[2].hours_left);
    assert_eq!(paces[1].standing, paces[2].standing);
}

#[test]
fn a_fable_week_is_paced_by_the_target_on_its_own_usage() {
    // Week used 30, FableWeek used 60, three days in, 92 percent by 24 hours
    // before the reset: the FableWeek stands Over its target on its own use,
    // the Week Under, and both are paced by the same target and deadline.
    let memory = codex_target(spend_by(24, 92));
    let reset = 10 * WEEK;
    let at = reset - WEEK + 72 * 3600;
    let mut value = reading(&[(Name::Week, reset, 10080), (Name::FableWeek, reset, 10080)], 30, at);
    value.window_vector[1].used = Used(Percent(60));
    let paces = value.pace(memory.configuration(), &TimeZone::UTC);
    assert_eq!(paces[0].standing, Standing::Under);
    assert_eq!(paces[1].standing, Standing::Over);
    assert_eq!(paces[1].target, spend_by(24, 92));
    assert_eq!(paces[1].expected.0, Percent(46), "half of 92 percent");
    assert_eq!(paces[1].hours_left.0, Hours(72));
    // The Session alone is paced Even, whatever the hours.
    let session = reading(&[(Name::Session, reset, 300)], 10, reset - 3600)
        .pace(memory.configuration(), &TimeZone::UTC)
        .remove(0);
    assert_eq!(session.target, Target::Even);
}

#[test]
fn a_fable_week_shorter_than_the_hours_is_paced_even() {
    // The refusal is bound by the Week window; a FableWeek of fewer hours than
    // the target (as a reading may hold) is paced Even, as any short window.
    let memory = codex_target(spend_by(24, 92));
    let reset = 10 * WEEK;
    let pace = reading(&[(Name::FableWeek, reset, 600)], 10, reset - 3600)
        .pace(memory.configuration(), &TimeZone::UTC)
        .remove(0);
    assert_eq!(pace.target, Target::Even);
}

#[test]
fn a_session_takes_no_target_even_one_it_could_hold() {
    // Five hours fit the five-hour Session, but a target is the Week's.
    let memory = codex_target(spend_by(5, 100));
    let reset = 10 * WEEK;
    let pace = reading(&[(Name::Session, reset, 300)], 0, reset - 5 * 3600)
        .pace(memory.configuration(), &TimeZone::UTC)
        .remove(0);
    assert_eq!(pace.target, Target::Even);
}

#[test]
fn a_week_exactly_as_long_as_the_hours_is_paced_by_them() {
    let memory = codex_target(spend_by(168, 100));
    let reset = 10 * WEEK;
    let pace = reading(&[(Name::Week, reset, 10080)], 0, reset - 168 * 3600)
        .pace(memory.configuration(), &TimeZone::UTC)
        .remove(0);
    // The target falls at the week's start: the week is paced by it, not Even.
    assert_eq!(pace.target, spend_by(168, 100));
    assert_eq!(pace.hours_left.0, Hours(0));
}

#[test]
fn a_session_spent_out_is_a_fault_in_the_pace() {
    let memory = Memory::default();
    let reset = 10 * WEEK;
    let standing = |name: Name, used: i64| {
        reading(&[(name, reset, 300)], used, reset - 3600)
            .pace(memory.configuration(), &TimeZone::UTC)
            .remove(0)
            .standing
    };
    assert_eq!(standing(Name::Session, 100), Standing::Spent);
    assert_ne!(standing(Name::Session, 99), Standing::Spent);
    assert_eq!(standing(Name::Session, 0), Standing::Under);
    // A Week or FableWeek used to 100 is a window run past its target, not a
    // fault in the pace.
    assert_eq!(standing(Name::Week, 100), Standing::Over);
    assert_eq!(standing(Name::FableWeek, 100), Standing::Over);
}

#[test]
fn a_target_of_more_hours_than_the_week_lasts_is_refused_by_the_held_reading() {
    let mut memory = Memory::default();
    let target = |hours| {
        Set::Target(ProviderTarget {
            provider: Provider::Codex,
            target: spend_by(hours, 100),
        })
    };
    // No reading is held for Codex, so no Week is known: the target is accepted.
    memory.configure(target(1000), NOW).expect("no reading held, accepted");
    memory.remember(reading(
        &[(Name::Session, 10 * WEEK, 300), (Name::Week, 10 * WEEK, 10080)],
        10,
        10 * WEEK - 3600,
    ));
    let before = memory.configuration().clone();
    let refused = ProviderTarget {
        provider: Provider::Codex,
        target: spend_by(169, 100),
    };
    assert_eq!(
        memory.configure(target(169), NOW),
        Err(ConfigureRefused::TargetExceedsWeek(refused)),
        "169 hours exceed the 168-hour week"
    );
    assert_eq!(memory.configuration(), &before, "a refused target changes nothing");
    memory.configure(target(168), NOW).expect("the whole week is the most");
    // Claude's week is not Codex's reading: with none held for it, accepted.
    memory
        .configure(
            Set::Target(ProviderTarget {
                provider: Provider::Claude,
                target: spend_by(1000, 100),
            }),
            NOW,
        )
        .expect("no Claude reading is held");
    // A rule broken is the other refusal, in words.
    assert!(matches!(
        memory.configure(target(-1), NOW),
        Err(ConfigureRefused::RuleRefused(_))
    ));
}

#[test]
fn hours_before_the_reset_are_a_whole_number_from_none_up() {
    let mut memory = Memory::default();
    for refused in [
        Set::Target(ProviderTarget {
            provider: Provider::Codex,
            target: spend_by(-1, 50),
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
