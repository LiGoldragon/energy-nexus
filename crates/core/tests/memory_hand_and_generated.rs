//! The Memory records were hand-written until energy-nexus 0b9e6b4 and are
//! generated from `ethos/memory.ethos` since. The hand declarations are
//! frozen here verbatim (fields made public), and every record is archived
//! through each and read through the other: the bytes are the same, so
//! `energy.rkyv` format version 2 is read and written as before.
use energy_nexus_core::{
    Configuration, History, Latest, Memory, RedeemExpiring, RedeemUsedAtLeast, Targets,
    WindowTargets,
};
use signal_energy_nexus::{
    Address, Allowed, At, Duration, Expires, Instant, Left, Level, Minutes, Name, Percent,
    Provider, ProviderTarget, Reading, Recipient, Reminder, Reset, ResetId, ResetStatus, Resets,
    Target, Threshold, Used, Window, WindowTarget,
};

mod hand {
    use signal_energy_nexus::{
        Level, Minutes, Percent, Provider, ProviderTarget, Reading, Reminder, Threshold,
        WindowTarget,
    };
    #[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
    pub struct Latest {
        pub provider: Provider,
        pub reading: Reading,
        pub level: Level,
    }
    #[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
    pub struct Configuration {
        pub thresholds: Vec<Threshold>,
        pub reminders: Vec<Reminder>,
        pub redeem_expiring: Minutes,
        pub redeem_used_at_least: Percent,
        pub targets: Vec<ProviderTarget>,
        pub window_targets: Vec<WindowTarget>,
    }
    #[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
    pub struct Memory {
        pub latest: Vec<Latest>,
        pub history: Vec<Reading>,
        pub configuration: Configuration,
    }
}

fn reading(provider: Provider, used: i64, at: i64) -> Reading {
    Reading {
        provider,
        allowed: Allowed(used < 100),
        window_vector: vec![
            Window {
                name: Name::Session,
                used: Used(Percent(used / 2)),
                resets: Resets(Instant(at + 3_600)),
                duration: Duration(Minutes(300)),
            },
            Window {
                name: Name::Week,
                used: Used(Percent(used)),
                resets: Resets(Instant(at + 86_400)),
                duration: Duration(Minutes(10_080)),
            },
            Window {
                name: Name::FableWeek,
                used: Used(Percent(used / 3)),
                resets: Resets(Instant(at + 90_000)),
                duration: Duration(Minutes(10_080)),
            },
        ],
        reset_vector: vec![
            Reset {
                reset_id: ResetId("one".into()),
                reset_status: ResetStatus::Available,
                expires: Expires(Instant(at + 7_200)),
            },
            Reset {
                reset_id: ResetId("two".into()),
                reset_status: ResetStatus::Redeeming,
                expires: Expires(Instant(at + 9_000)),
            },
            Reset {
                reset_id: ResetId("three".into()),
                reset_status: ResetStatus::Redeemed,
                expires: Expires(Instant(at)),
            },
        ],
        at: At(Instant(at)),
    }
}

fn reminder(level: Level, model: signal_flow::Model) -> Reminder {
    Reminder {
        provider: Provider::Codex,
        level,
        recipient: Recipient(Address {
            aspect: signal_flow::Aspect::Mind,
            topic: signal_flow::Topic("energyNexus".into()),
            model,
        }),
    }
}

/// A Memory with every field and every variant of every record set.
fn generated() -> Memory {
    let readings = vec![
        reading(Provider::Claude, 12, 1_000),
        reading(Provider::Codex, 64, 2_000),
        reading(Provider::Codex, 100, 3_000),
    ];
    let latest = |reading: &Reading, level| Latest {
        provider: reading.provider.clone(),
        reading: reading.clone(),
        level,
    };
    Memory {
        latest_vector: vec![
            latest(&readings[0], Level::Plenty),
            latest(&readings[1], Level::Fair),
            latest(&readings[2], Level::Out),
        ],
        history: History(readings),
        configuration: Configuration {
            threshold_vector: vec![
                Threshold { level: Level::Plenty, left: Left(Percent(50)) },
                Threshold { level: Level::Fair, left: Left(Percent(20)) },
                Threshold { level: Level::Low, left: Left(Percent(0)) },
            ],
            reminder_vector: vec![
                reminder(Level::Low, signal_flow::Model::Sol),
                reminder(Level::Out, signal_flow::Model::Fable),
            ],
            redeem_expiring: RedeemExpiring(Minutes(1_440)),
            redeem_used_at_least: RedeemUsedAtLeast(Percent(3)),
            targets: Targets(vec![
                ProviderTarget { provider: Provider::Claude, target: Target::Even },
                ProviderTarget {
                    provider: Provider::Codex,
                    target: Target::SpendBy(Instant(1_762_450_320)),
                },
            ]),
            window_targets: WindowTargets(vec![WindowTarget {
                provider: Provider::Claude,
                name: Name::FableWeek,
                target: Target::Even,
            }]),
        },
    }
}

fn hand(memory: &Memory) -> hand::Memory {
    let configuration = &memory.configuration;
    hand::Memory {
        latest: memory
            .latest_vector
            .iter()
            .map(|latest| hand::Latest {
                provider: latest.provider.clone(),
                reading: latest.reading.clone(),
                level: latest.level.clone(),
            })
            .collect(),
        history: memory.history.0.clone(),
        configuration: hand::Configuration {
            thresholds: configuration.threshold_vector.clone(),
            reminders: configuration.reminder_vector.clone(),
            redeem_expiring: configuration.redeem_expiring.0,
            redeem_used_at_least: configuration.redeem_used_at_least.0,
            targets: configuration.targets.0.clone(),
            window_targets: configuration.window_targets.0.clone(),
        },
    }
}

#[test]
fn the_generated_memory_archives_as_the_hand_memory_both_ways() {
    for memory in [generated(), Memory::default()] {
        let hand = hand(&memory);
        let through_hand = rkyv::to_bytes::<rkyv::rancor::Error>(&hand).unwrap();
        let through_generated = rkyv::to_bytes::<rkyv::rancor::Error>(&memory).unwrap();
        assert_eq!(through_hand.as_slice(), through_generated.as_slice());
        let read_as_generated =
            rkyv::from_bytes::<Memory, rkyv::rancor::Error>(&through_hand).unwrap();
        assert_eq!(read_as_generated, memory);
        let read_as_hand =
            rkyv::from_bytes::<hand::Memory, rkyv::rancor::Error>(&through_generated).unwrap();
        assert_eq!(read_as_hand, hand);
    }
}
