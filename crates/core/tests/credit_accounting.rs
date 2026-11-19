use energy_nexus_core::{
    CreditAccounting, CreditTiming, Configuring, Memory, Validating, version4,
};
use energy_nexus_wire::{Admission, Earlier, Persisted};
use meta_signal_energy_nexus::{ProviderTarget, Set};
use signal_energy_nexus::{
    At, Baseline, Cents, Credit, Expires, Instant, Level, Minutes, Name, Override, Percent,
    Provider, Reading, Renewal, Reminder, Schedule, Spent, Starts, Target, Threshold, TimeZone,
    Window,
};

const START: i64 = 1_795_046_400;
const EXPIRES: i64 = 1_797_638_400;
const LEAD: i64 = 10_080;

fn configured(spent: Option<i64>) -> Memory {
    let mut memory = Memory::default();
    memory
        .configure(
            Set::Credit(Credit {
                provider: Provider::ClaudeApi,
                granted: Cents(20_000),
                starts: Starts(Instant(START)),
                expires: Expires(Instant(EXPIRES)),
                renewal: Renewal::Once,
                lead_time_option: Some(Minutes(LEAD)),
            }),
            Instant(START),
        )
        .unwrap();
    if let Some(cents) = spent {
        memory
            .configure(
                Set::Spent(Spent {
                    provider: Provider::ClaudeApi,
                    cents: Cents(cents),
                    at: At(Instant(START + 600)),
                }),
                Instant(START + 600),
            )
            .unwrap();
    }
    memory
}

#[test]
fn a_missing_observation_is_unknown_not_zero() {
    let memory = configured(None);
    assert_eq!(
        memory.balance_for(&Provider::ClaudeApi, Instant(START + 1)),
        Err(signal_energy_nexus::BalanceRefused::SpentUnavailable(
            Provider::ClaudeApi
        ))
    );
    assert!(memory.reading_for(&Provider::ClaudeApi, Instant(START + 1)).is_err());
}

#[test]
fn a_grant_is_out_before_its_start_and_its_default_lead_is_seven_days() {
    let mut memory = configured(Some(0));
    let mut grant = memory
        .credit_for(&Provider::ClaudeApi)
        .unwrap()
        .clone();
    grant.lead_time_option = None;
    memory
        .configure(Set::Credit(grant), Instant(START))
        .unwrap();
    let configured = memory.credit_for(&Provider::ClaudeApi).unwrap();
    assert_eq!(configured.lead_time_option, Some(Minutes(7 * 24 * 60)));
    let before = memory
        .reading_for(&Provider::ClaudeApi, Instant(START - 1))
        .unwrap();
    assert!(!before.allowed.0);
    assert_eq!(before.level(memory.configuration()), Level::Out);
}

#[test]
fn invalid_grants_and_spend_observations_are_refused_before_they_enter_memory() {
    let mut memory = Memory::default();
    let invalid_credit = Credit {
        provider: Provider::ClaudeApi,
        granted: Cents(20_000),
        starts: Starts(Instant(START)),
        expires: Expires(Instant(EXPIRES)),
        renewal: Renewal::Once,
        lead_time_option: Some(Minutes(-1)),
    };
    assert!(memory
        .configure(Set::Credit(invalid_credit), Instant(START))
        .is_err());
    assert!(memory.credit_for(&Provider::ClaudeApi).is_none());

    memory
        .configure(
            Set::Credit(Credit {
                provider: Provider::ClaudeApi,
                granted: Cents(20_000),
                starts: Starts(Instant(START)),
                expires: Expires(Instant(EXPIRES)),
                renewal: Renewal::Once,
                lead_time_option: None,
            }),
            Instant(START),
        )
        .unwrap();
    for invalid in [
        Spent {
            provider: Provider::ClaudeApi,
            cents: Cents(-1),
            at: At(Instant(START + 1)),
        },
        Spent {
            provider: Provider::ClaudeApi,
            cents: Cents(1),
            at: At(Instant(START + 1)),
        },
    ] {
        let now = if invalid.cents.0 < 0 { START + 1 } else { START };
        assert!(memory.configure(Set::Spent(invalid), Instant(now)).is_err());
        assert!(memory
            .configuration()
            .spent_vector
            .is_empty());
    }
}

#[test]
fn each_credit_lead_crosses_low_even_when_the_percent_rule_says_plenty() {
    let memory = configured(Some(5_000));
    let at = EXPIRES - LEAD * 60;
    let reading = memory.reading_for(&Provider::ClaudeApi, Instant(at)).unwrap();
    assert_eq!(reading.level(memory.configuration()), Level::Low);
    let outside = memory
        .reading_for(&Provider::ClaudeApi, Instant(at - 1))
        .unwrap();
    assert_eq!(outside.level(memory.configuration()), Level::Plenty);

    let empty = configured(Some(20_000));
    let no_money = empty.reading_for(&Provider::ClaudeApi, Instant(at)).unwrap();
    assert_eq!(no_money.level(empty.configuration()), Level::Out);
}

#[test]
fn target_sets_a_daily_cents_budget_and_expiry_is_the_reset() {
    let mut memory = configured(Some(13));
    memory
        .configure(
            Set::Target(ProviderTarget {
                provider: Provider::ClaudeApi,
                target: Target::SpendBy(signal_energy_nexus::SpendBy_Data {
                    hours: signal_energy_nexus::Hours(24),
                    percent: Percent(100),
                }),
            }),
            Instant(START + 700),
        )
        .unwrap();
    let now = START + 700;
    let balance = memory.balance_for(&Provider::ClaudeApi, Instant(now)).unwrap();
    assert_eq!(balance.granted, Cents(20_000));
    assert_eq!(balance.spent, Cents(13));
    assert_eq!(balance.left, Cents(19_987));
    assert_eq!(balance.expires, Expires(Instant(EXPIRES)));
    assert_eq!(balance.per_day, Cents(689));
}

#[test]
fn a_target_outside_a_short_credit_window_paces_evenly_for_its_balance() {
    let mut memory = Memory::default();
    memory
        .configure(
            Set::Credit(Credit {
                provider: Provider::ClaudeApi,
                granted: Cents(3_000),
                starts: Starts(Instant(START)),
                expires: Expires(Instant(START + 2 * 86_400)),
                renewal: Renewal::Once,
                lead_time_option: Some(Minutes(LEAD)),
            }),
            Instant(START),
        )
        .unwrap();
    memory
        .configure(
            Set::Spent(Spent {
                provider: Provider::ClaudeApi,
                cents: Cents(300),
                at: At(Instant(START + 10)),
            }),
            Instant(START + 10),
        )
        .unwrap();
    memory
        .configure(
            Set::Target(ProviderTarget {
                provider: Provider::ClaudeApi,
                target: Target::SpendBy(signal_energy_nexus::SpendBy_Data {
                    hours: signal_energy_nexus::Hours(72),
                    percent: Percent(100),
                }),
            }),
            Instant(START + 10),
        )
        .unwrap();
    let even_budget = memory
        .balance_for(&Provider::ClaudeApi, Instant(START + 10))
        .unwrap();
    assert_eq!(even_budget.per_day, Cents(1_350));
}

#[test]
fn monthly_renewal_clamps_in_the_owner_zone_and_records_known_zero_at_start() {
    let mut memory = Memory::default();
    memory
        .configure(
            Set::TimeZone(TimeZone("America/Mexico_City".into())),
            Instant(START),
        )
        .unwrap();
    memory
        .configure(
            Set::Credit(Credit {
                provider: Provider::OpenAiApi,
                granted: Cents(3_000),
                starts: Starts(Instant(1_704_132_000)), // 2024-01-01 12:00, Mexico City
                expires: Expires(Instant(1_706_724_000)), // 2024-01-31 12:00, Mexico City
                renewal: Renewal::Monthly,
                lead_time_option: Some(Minutes(LEAD)),
            }),
            Instant(1_704_132_000),
        )
        .unwrap();
    let changed = memory.advance_expired(Instant(1_706_724_001)).unwrap();
    assert_eq!(changed, vec![Provider::OpenAiApi]);
    let credit = memory.credit_for(&Provider::OpenAiApi).unwrap();
    assert_eq!(credit.starts, Starts(Instant(1_706_724_000)));
    assert_eq!(credit.expires, Expires(Instant(1_709_229_600))); // 2024-02-29 12:00, Mexico City
    let balance = memory
        .balance_for(&Provider::OpenAiApi, Instant(1_706_724_001))
        .unwrap();
    assert_eq!(balance.spent, Cents(0));
    assert_eq!(balance.left, Cents(3_000));
}

#[test]
fn each_credit_exposes_the_next_start_low_and_expiry_transition() {
    let credit = Credit {
        provider: Provider::ClaudeApi,
        granted: Cents(20_000),
        starts: Starts(Instant(START)),
        expires: Expires(Instant(EXPIRES)),
        renewal: Renewal::Once,
        lead_time_option: Some(Minutes(LEAD)),
    };
    assert_eq!(credit.next_transition(Instant(START - 1)), Some(Instant(START)));
    assert_eq!(
        credit.next_transition(Instant(EXPIRES - LEAD * 60 - 1)),
        Some(Instant(EXPIRES - LEAD * 60))
    );
    assert_eq!(credit.next_transition(Instant(EXPIRES - 1)), Some(Instant(EXPIRES)));
    assert_eq!(credit.next_transition(Instant(EXPIRES)), None);
}

#[test]
fn the_version_four_archive_keeps_every_old_setting_and_gains_empty_credit_state() {
    let provider = Provider::Codex;
    let reading = Reading {
        provider: provider.clone(),
        allowed: signal_energy_nexus::Allowed(true),
        window_vector: vec![Window {
            name: Name::Week,
            used: signal_energy_nexus::Used(Percent(20)),
            resets: signal_energy_nexus::Resets(Instant(2_000)),
            duration: signal_energy_nexus::Duration(Minutes(10_080)),
        }],
        reset_vector: Vec::new(),
        at: At(Instant(1_000)),
    };
    let old = version4::Memory {
        latest_vector: vec![version4::Latest {
            provider: provider.clone(),
            reading: reading.clone(),
            level: Level::Fair,
        }],
        history: version4::History(vec![reading.clone()]),
        configuration: version4::Configuration {
            threshold_vector: vec![Threshold {
                level: Level::Fair,
                left: signal_energy_nexus::Left(Percent(40)),
            }],
            reminder_vector: Vec::<Reminder>::new(),
            redeem_expiring: version4::RedeemExpiring(Minutes(720)),
            redeem_used_at_least: version4::RedeemUsedAtLeast(Percent(5)),
            targets: version4::Targets(vec![ProviderTarget {
                provider: provider.clone(),
                target: Target::Even,
            }]),
            schedule: version4::Schedule(Vec::new()),
            override_vector: Vec::<Override>::new(),
            baseline: Baseline(Percent(80)),
            time_zone_option: Some(TimeZone("America/Mexico_City".into())),
        },
    };
    let archive = rkyv::to_bytes::<rkyv::rancor::Error>(&old).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"ENXMEMRY");
    bytes.extend_from_slice(&4_u32.to_le_bytes());
    bytes.extend_from_slice(&(archive.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&archive);
    let path = std::env::temp_dir().join(format!("energy-credit-v4-{}.rkyv", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let admission = Memory::admit(&path).unwrap();
    let _ = std::fs::remove_file(path);
    let Admission::Upgraded(Earlier::Version(4), upgrade) = admission else {
        panic!("version 4 is upgraded");
    };
    assert_eq!(upgrade.memory.latest(&provider).unwrap().reading, reading);
    assert_eq!(upgrade.memory.history(), &[reading]);
    assert_eq!(upgrade.memory.configuration().redeem_expiring.0, Minutes(720));
    assert_eq!(upgrade.memory.configuration().redeem_used_at_least.0, Percent(5));
    assert_eq!(upgrade.memory.configuration().targets.0.len(), 1);
    assert_eq!(upgrade.memory.configuration().baseline, Baseline(Percent(80)));
    assert_eq!(
        upgrade.memory.configuration().time_zone_option,
        Some(TimeZone("America/Mexico_City".into()))
    );
    assert!(upgrade.memory.configuration().credit_vector.is_empty());
    assert!(upgrade.memory.configuration().spent_vector.is_empty());
    upgrade.memory.configuration().validate().unwrap();
}
