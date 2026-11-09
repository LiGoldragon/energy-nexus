use energy_nexus_wire::{Earlier, Envelope, Persisted, Upgrade};
use meta_signal_energy_nexus::{ProviderTarget, Set, WindowTarget};
use signal_energy_nexus::{
    Instant, Level, Minutes, Pace, Percent, Provider, Reading, Reminder, Reset, ResetStatus,
    Standing, Target, Threshold,
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
impl Default for Configuration {
    fn default() -> Self {
        Self {
            thresholds: vec![
                Threshold {
                    level: Level::Plenty,
                    left: Percent(50),
                },
                Threshold {
                    level: Level::Fair,
                    left: Percent(20),
                },
                Threshold {
                    level: Level::Low,
                    left: Percent(0),
                },
            ],
            reminders: Vec::new(),
            redeem_expiring: Minutes(1440),
            redeem_used_at_least: Percent(1),
            targets: vec![
                ProviderTarget {
                    provider: Provider::Claude,
                    target: Target::Even,
                },
                ProviderTarget {
                    provider: Provider::Codex,
                    target: Target::Even,
                },
            ],
            window_targets: Vec::new(),
        }
    }
}
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, Default)]
pub struct Memory {
    latest: Vec<Latest>,
    history: Vec<Reading>,
    configuration: Configuration,
}
impl Configuration {
    /// The rules `Memory::configure` enforces, kept by every memory read or
    /// written.
    fn validate(&self) -> Result<(), String> {
        if self.redeem_expiring.0 <= 0 {
            return Err("minutes must be positive".into());
        }
        if !(0..=100).contains(&self.redeem_used_at_least.0) {
            return Err("percent must be between 0 and 100".into());
        }
        for threshold in &self.thresholds {
            if threshold.level == Level::Out || !(0..=100).contains(&threshold.left.0) {
                return Err(
                    "Out follows provider allowance; threshold percent must be between 0 and 100"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

/// The daemon's Memory, `energy.rkyv`, under its envelope. Version 2 has
/// no polling interval: Energy reads a provider on events. Version 1 and the
/// archive written before the envelope are upgraded on open.
impl Persisted for Memory {
    const ENVELOPE: Envelope = Envelope {
        magic: *b"ENXMEMRY",
        version: 2,
    };
    const EARLIER_VERSIONS: &'static [u32] = &[1];
    type Uncarried = unversioned::Reminder;
    fn from_archive(archive: &[u8]) -> Result<Self, String> {
        rkyv::from_bytes::<Self, rkyv::rancor::Error>(archive).map_err(|error| error.to_string())
    }
    fn upgrade(earlier: Earlier, archive: &[u8]) -> Result<Upgrade<Self>, String> {
        match earlier {
            Earlier::Unenveloped => {
                rkyv::from_bytes::<unversioned::Memory, rkyv::rancor::Error>(archive)
                    .map(Upgrade::from)
                    .map_err(|error| error.to_string())
            }
            Earlier::Version(1) => {
                rkyv::from_bytes::<version1::Memory, rkyv::rancor::Error>(archive)
                    .map(Upgrade::from)
                    .map_err(|error| error.to_string())
            }
            Earlier::Version(version) => Err(format!("no layout of format version {version}")),
        }
    }
    fn to_archive(&self) -> Result<rkyv::util::AlignedVec, String> {
        rkyv::to_bytes::<rkyv::rancor::Error>(self).map_err(|error| error.to_string())
    }
    fn validate(&self) -> Result<(), String> {
        self.configuration.validate()
    }
}

/// The layout the daemon wrote before its Memory had an envelope, up to
/// energy-nexus e5722529, which the proof of concept runs: the current
/// layout but for a reminder's recipient, whose Flow address named a Layer
/// (signal-flow 74f4dae). Flow now addresses a flow by its Model, and which
/// model serves a layer is Flow's configuration, not Energy's; so a reminder
/// is not carried into the current layout. It is answered, for the signals
/// that repopulate it with a model: `Configure.Reminder`. Its polling
/// interval is dropped: Energy reads on events.
pub mod unversioned {
    use meta_signal_energy_nexus::{ProviderTarget, WindowTarget};
    use signal_energy_nexus::{Level, Minutes, Percent, Provider, Reading, Threshold};

    #[derive(rkyv::Archive, rkyv::Deserialize, Debug)]
    pub struct Memory {
        pub latest: Vec<super::Latest>,
        pub history: Vec<Reading>,
        pub configuration: Configuration,
    }
    #[derive(rkyv::Archive, rkyv::Deserialize, Debug)]
    pub struct Configuration {
        pub polling: Minutes,
        pub thresholds: Vec<Threshold>,
        pub reminders: Vec<Reminder>,
        pub redeem_expiring: Minutes,
        pub redeem_used_at_least: Percent,
        pub targets: Vec<ProviderTarget>,
        pub window_targets: Vec<WindowTarget>,
    }
    #[derive(rkyv::Archive, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
    pub struct Reminder {
        pub provider: Provider,
        pub level: Level,
        pub recipient: Recipient,
    }
    #[derive(rkyv::Archive, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
    pub struct Recipient(pub Address);
    #[derive(rkyv::Archive, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
    pub struct Address {
        pub aspect: Aspect,
        pub topic: String,
        pub layer: Layer,
    }
    #[derive(rkyv::Archive, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
    pub enum Aspect {
        Psyche,
        Mind,
        Field,
    }
    #[derive(rkyv::Archive, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
    pub enum Layer {
        Primary,
        Secondary,
        Tertiary,
        Quaternary,
    }
    /// The reminder in the text form it was configured with:
    /// `Codex Low { Mind energyNexus Secondary }`.
    impl std::fmt::Display for Reminder {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let address = &self.recipient.0;
            write!(
                formatter,
                "{:?} {:?} {{ {:?} {} {:?} }}",
                self.provider, self.level, address.aspect, address.topic, address.layer
            )
        }
    }
    impl From<Memory> for super::Upgrade<super::Memory> {
        /// Every reading, level and setting is carried but the polling
        /// interval, which is dropped; every reminder is answered as
        /// uncarried.
        fn from(old: Memory) -> Self {
            let configuration = old.configuration;
            Self {
                memory: super::Memory {
                    latest: old.latest,
                    history: old.history,
                    configuration: super::Configuration {
                        thresholds: configuration.thresholds,
                        reminders: Vec::new(),
                        redeem_expiring: configuration.redeem_expiring,
                        redeem_used_at_least: configuration.redeem_used_at_least,
                        targets: configuration.targets,
                        window_targets: configuration.window_targets,
                    },
                },
                uncarried: configuration.reminders,
            }
        }
    }
}
/// The layout of format version 1, written by energy-nexus 0.2.0 (4e4742e):
/// the current layout and a polling interval. The interval is dropped on
/// upgrade, since Energy reads on events; everything else is carried.
pub mod version1 {
    use meta_signal_energy_nexus::{ProviderTarget, WindowTarget};
    use signal_energy_nexus::{Minutes, Percent, Reading, Reminder, Threshold};

    #[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Debug)]
    pub struct Memory {
        pub latest: Vec<super::Latest>,
        pub history: Vec<Reading>,
        pub configuration: Configuration,
    }
    #[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Debug)]
    pub struct Configuration {
        pub polling: Minutes,
        pub thresholds: Vec<Threshold>,
        pub reminders: Vec<Reminder>,
        pub redeem_expiring: Minutes,
        pub redeem_used_at_least: Percent,
        pub targets: Vec<ProviderTarget>,
        pub window_targets: Vec<WindowTarget>,
    }
    impl From<Memory> for super::Upgrade<super::Memory> {
        fn from(old: Memory) -> Self {
            let configuration = old.configuration;
            Self {
                memory: super::Memory {
                    latest: old.latest,
                    history: old.history,
                    configuration: super::Configuration {
                        thresholds: configuration.thresholds,
                        reminders: configuration.reminders,
                        redeem_expiring: configuration.redeem_expiring,
                        redeem_used_at_least: configuration.redeem_used_at_least,
                        targets: configuration.targets,
                        window_targets: configuration.window_targets,
                    },
                },
                uncarried: Vec::new(),
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelChange {
    pub old: Option<Level>,
    pub latest: Latest,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RedeemDecision {
    Allowed(Reset),
    Refused(String),
    NoReset,
}

pub trait Pacing {
    fn pace(&self, configuration: &Configuration) -> Vec<Pace>;
}
impl Pacing for Reading {
    fn pace(&self, configuration: &Configuration) -> Vec<Pace> {
        self.window_vector
            .iter()
            .map(|window| {
                let target = configuration
                    .window_targets
                    .iter()
                    .find(|configured| {
                        configured.provider == self.provider && configured.name == window.name
                    })
                    .map(|configured| configured.target.clone())
                    .or_else(|| {
                        configuration
                            .targets
                            .iter()
                            .find(|configured| configured.provider == self.provider)
                            .map(|configured| configured.target.clone())
                    })
                    .unwrap_or(Target::Even);
                let used = window.used.0.clamp(0, 100);
                let start = window.resets.0 - window.duration.0 * 60;
                let deadline = match &target {
                    Target::Even => window.resets.0,
                    Target::SpendBy(instant) => instant.0,
                };
                let span = (deadline - start).max(1);
                let elapsed = (self.at.0 - start).clamp(0, span);
                let expected = ((elapsed * 100 + span / 2) / span).clamp(0, 100);
                let seconds_left = (deadline - self.at.0).max(1);
                let per_day = if used >= 100 || deadline <= self.at.0 {
                    0
                } else {
                    (((100 - used) * 86_400 + seconds_left / 2) / seconds_left).max(1)
                };
                Pace {
                    provider: self.provider.clone(),
                    name: window.name.clone(),
                    target,
                    expected: Percent(expected),
                    used: Percent(used),
                    standing: match used.cmp(&expected) {
                        std::cmp::Ordering::Greater => Standing::Over,
                        std::cmp::Ordering::Less => Standing::Under,
                        std::cmp::Ordering::Equal => Standing::On,
                    },
                    per_day: Percent(per_day),
                }
            })
            .collect()
    }
}

/// When a reading says its provider's next window resets.
pub trait Resetting {
    /// The soonest window reset after the reading was taken; none when no
    /// window names a later one.
    fn next_reset(&self) -> Option<Instant>;
}
impl Resetting for Reading {
    fn next_reset(&self) -> Option<Instant> {
        self.window_vector
            .iter()
            .map(|window| window.resets.0)
            .filter(|resets| *resets > self.at.0)
            .min()
            .map(Instant)
    }
}

pub trait Levelling {
    fn level(&self, configuration: &Configuration) -> Level;
}
impl Levelling for Reading {
    fn level(&self, configuration: &Configuration) -> Level {
        if !self.allowed {
            return Level::Out;
        }
        let left = 100
            - self
                .window_vector
                .iter()
                .map(|window| window.used.0)
                .max()
                .unwrap_or(0);
        configuration
            .thresholds
            .iter()
            .filter(|threshold| threshold.level != Level::Out && left >= threshold.left.0)
            .max_by_key(|threshold| threshold.left.0)
            .map(|threshold| threshold.level.clone())
            .unwrap_or(Level::Low)
    }
}
pub trait Remembering {
    fn remember(&mut self, reading: Reading) -> LevelChange;
    fn latest(&self, provider: &Provider) -> Option<&Latest>;
    fn history(&self) -> &[Reading];
}
impl Remembering for Memory {
    fn remember(&mut self, reading: Reading) -> LevelChange {
        let level = reading.level(&self.configuration);
        let old = self
            .latest(&reading.provider)
            .map(|latest| latest.level.clone());
        self.history.push(reading.clone());
        self.latest
            .retain(|latest| latest.provider != reading.provider);
        let latest = Latest {
            provider: reading.provider.clone(),
            reading,
            level,
        };
        self.latest.push(latest.clone());
        LevelChange { old, latest }
    }
    fn latest(&self, provider: &Provider) -> Option<&Latest> {
        self.latest
            .iter()
            .find(|latest| &latest.provider == provider)
    }
    fn history(&self) -> &[Reading] {
        &self.history
    }
}
pub trait Configuring {
    fn configure(&mut self, set: Set) -> Result<Set, String>;
    fn configuration(&self) -> &Configuration;
}
impl Configuring for Memory {
    fn configure(&mut self, set: Set) -> Result<Set, String> {
        match &set {
            Set::RedeemExpiring(value) if value.0 <= 0 => {
                return Err("minutes must be positive".into())
            }
            Set::RedeemUsedAtLeast(value) if !(0..=100).contains(&value.0) => {
                return Err("percent must be between 0 and 100".into())
            }
            Set::Threshold(value)
                if value.level == Level::Out || !(0..=100).contains(&value.left.0) =>
            {
                return Err(
                    "Out follows provider allowance; threshold percent must be between 0 and 100"
                        .into(),
                )
            }
            _ => {}
        }
        match set.clone() {
            Set::Threshold(value) => {
                self.configuration
                    .thresholds
                    .retain(|known| known.level != value.level);
                self.configuration.thresholds.push(value)
            }
            Set::Reminder(value) => {
                if !self.configuration.reminders.contains(&value) {
                    self.configuration.reminders.push(value)
                }
            }
            Set::RemoveReminder(value) => {
                self.configuration.reminders.retain(|known| known != &value)
            }
            Set::RedeemExpiring(value) => self.configuration.redeem_expiring = value,
            Set::RedeemUsedAtLeast(value) => self.configuration.redeem_used_at_least = value,
            Set::Target(value) => {
                self.configuration
                    .targets
                    .retain(|known| known.provider != value.provider);
                self.configuration.targets.push(value)
            }
            Set::WindowTarget(value) => {
                self.configuration
                    .window_targets
                    .retain(|known| known.provider != value.provider || known.name != value.name);
                self.configuration.window_targets.push(value)
            }
            Set::Program(_) | Set::Credentials(_) => {
                return Err("translator configuration belongs to the translator".into())
            }
        }
        Ok(set)
    }
    fn configuration(&self) -> &Configuration {
        &self.configuration
    }
}
pub trait Redeeming {
    fn redeem_decision(&self, provider: &Provider, now: i64) -> RedeemDecision;
}
impl Redeeming for Memory {
    fn redeem_decision(&self, provider: &Provider, now: i64) -> RedeemDecision {
        if provider != &Provider::Codex {
            return RedeemDecision::Refused("only Codex supplies resets".into());
        }
        let Some(latest) = self.latest(provider) else {
            return RedeemDecision::Refused("no reading".into());
        };
        let Some(reset) = latest
            .reading
            .reset_vector
            .iter()
            .filter(|reset| reset.reset_status == ResetStatus::Available)
            .min_by_key(|reset| reset.expires.0)
            .cloned()
        else {
            return RedeemDecision::NoReset;
        };
        let week_used = latest
            .reading
            .window_vector
            .iter()
            .find(|window| window.name == signal_energy_nexus::Name::Week)
            .map(|window| window.used.0)
            .unwrap_or(0);
        let expiring = reset.expires.0 - now <= self.configuration.redeem_expiring.0 * 60;
        if !latest.reading.allowed
            || (expiring && week_used >= self.configuration.redeem_used_at_least.0)
        {
            RedeemDecision::Allowed(reset)
        } else {
            RedeemDecision::Refused(
                "usage is allowed and the soonest reset is outside the configured rule".into(),
            )
        }
    }
}
pub trait Reminding {
    fn reminders_for(&self, provider: &Provider, level: &Level) -> Vec<Reminder>;
}
impl Reminding for Memory {
    fn reminders_for(&self, provider: &Provider, level: &Level) -> Vec<Reminder> {
        self.configuration
            .reminders
            .iter()
            .filter(|r| &r.provider == provider && &r.level == level)
            .cloned()
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use energy_nexus_wire::{Admission, MemoryRefusal};
    use ethos_zero::{Actualizing, File, Potential, Validating};
    use signal_energy_nexus::{Instant, Name, Window};

    fn assert_ethos(source: &str) {
        let file = Potential::<File>::from(source).actualize().unwrap();
        file.validate().unwrap();
    }

    #[test]
    fn ethos_declarations_check() {
        for source in [
            include_str!("../../../ethos/library.ethos"),
            include_str!("../../../ethos/operation.ethos"),
            include_str!("../../../ethos/memory.ethos"),
            include_str!("../../../ethos/emission.ethos"),
        ] {
            assert_ethos(source);
        }
    }
    fn reading(allowed: bool, used: i64) -> Reading {
        Reading {
            provider: Provider::Codex,
            allowed,
            window_vector: vec![Window {
                name: Name::Week,
                used: Percent(used),
                resets: Instant(200),
                duration: Minutes(10080),
            }],
            reset_vector: vec![Reset {
                reset_id: signal_energy_nexus::ResetId("soon".into()),
                reset_status: ResetStatus::Available,
                expires: Instant(130),
            }],
            at: Instant(100),
        }
    }
    #[test]
    fn provider_denial_is_out() {
        assert_eq!(
            reading(false, 10).level(&Configuration::default()),
            Level::Out
        )
    }
    #[test]
    fn remembers_only_latest_and_all_history() {
        let mut m = Memory::default();
        m.remember(reading(true, 10));
        m.remember(reading(true, 90));
        assert_eq!(m.history().len(), 2);
        assert_eq!(m.latest(&Provider::Codex).unwrap().level, Level::Low)
    }
    #[test]
    fn explicit_redeem_is_waiting_work() {
        let mut m = Memory::default();
        m.remember(reading(false, 10));
        assert!(matches!(
            m.redeem_decision(&Provider::Codex, 100),
            RedeemDecision::Allowed(_)
        ))
    }
    #[test]
    fn pace_compares_week_usage_to_spend_by_target() {
        let mut value = reading(true, 92);
        value.at = Instant(50);
        value.window_vector[0].resets = Instant(150);
        value.window_vector[0].duration = Minutes(2);
        let mut configuration = Configuration::default();
        configuration.targets = vec![ProviderTarget {
            provider: Provider::Codex,
            target: Target::SpendBy(Instant(170)),
        }];
        let pace = value.pace(&configuration).remove(0);
        assert_eq!(pace.used, Percent(92));
        assert_eq!(pace.standing, Standing::Over);
        assert!(pace.expected.0 < pace.used.0);
    }
    #[test]
    fn the_next_reset_is_the_soonest_window_reset_after_the_reading() {
        let mut value = reading(true, 10);
        value.window_vector.push(Window {
            name: Name::Session,
            used: Percent(5),
            resets: Instant(150),
            duration: Minutes(300),
        });
        value.window_vector.push(Window {
            name: Name::FableWeek,
            used: Percent(5),
            resets: Instant(0),
            duration: Minutes(10080),
        });
        assert_eq!(value.next_reset(), Some(Instant(150)));
        value.at = Instant(150);
        assert_eq!(value.next_reset(), Some(Instant(200)), "a reset at the reading is past");
        value.at = Instant(200);
        assert_eq!(value.next_reset(), None);
    }
    #[test]
    fn window_target_overrides_provider_default() {
        let mut value = reading(true, 40);
        value.window_vector.push(Window {
            name: Name::FableWeek,
            used: Percent(60),
            resets: Instant(200),
            duration: Minutes(10080),
        });
        let mut configuration = Configuration::default();
        configuration.window_targets = vec![WindowTarget {
            provider: Provider::Codex,
            name: Name::FableWeek,
            target: Target::SpendBy(Instant(220)),
        }];
        let pace = value.pace(&configuration);
        assert_eq!(pace.len(), 2);
        assert_eq!(pace[0].name, Name::Week);
        assert_eq!(pace[0].target, Target::Even);
        assert_eq!(pace[1].name, Name::FableWeek);
        assert_eq!(pace[1].target, Target::SpendBy(Instant(220)));
    }
    #[test]
    fn memory_survives_a_restart() {
        let path = std::env::temp_dir().join(format!(
            "energy-nexus-memory-{}-{}.rkyv",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let mut memory = Memory::default();
        memory.remember(reading(true, 42));
        memory
            .configure(Set::Target(ProviderTarget {
                provider: Provider::Codex,
                target: Target::SpendBy(Instant(170)),
            }))
            .unwrap();
        memory.save(&path).unwrap();

        let restored = match Memory::load(&path).unwrap() {
            Admission::Admitted(memory) => memory,
            other => panic!("a saved memory is admitted as it is, got {other:?}"),
        };
        assert_eq!(restored.history().len(), 1);
        assert_eq!(
            restored.latest(&Provider::Codex).unwrap().reading,
            reading(true, 42)
        );
        assert_eq!(restored.configuration().targets.len(), 2);
        assert_eq!(
            restored
                .configuration()
                .targets
                .iter()
                .find(|target| target.provider == Provider::Codex)
                .unwrap()
                .target,
            Target::SpendBy(Instant(170))
        );
        std::fs::remove_file(path).unwrap();
    }
    /// The current envelope around a Memory, built without `save`, so an invalid configuration
    /// can be placed on disk to prove admission refuses it.
    fn envelope(memory: &Memory) -> Vec<u8> {
        let archive = rkyv::to_bytes::<rkyv::rancor::Error>(memory).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&Memory::ENVELOPE.magic);
        bytes.extend_from_slice(&Memory::ENVELOPE.version.to_le_bytes());
        bytes.extend_from_slice(&(archive.len() as u64).to_le_bytes());
        bytes.extend_from_slice(archive.as_slice());
        bytes
    }

    #[test]
    fn configuration_rules_are_enforced_on_admission_and_before_any_write() {
        let directory = std::env::temp_dir().join(format!("energy-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        for broken in 0..3 {
            let mut memory = Memory::default();
            match broken {
                0 => memory.configuration.redeem_expiring = Minutes(0),
                1 => memory.configuration.redeem_used_at_least = Percent(101),
                _ => memory.configuration.thresholds.push(Threshold {
                    level: Level::Out,
                    left: Percent(10),
                }),
            }
            // admission of a file holding the invalid configuration, built by test encoding
            let path = directory.join(format!("memory-{broken}.rkyv"));
            std::fs::write(&path, envelope(&memory)).unwrap();
            let before = std::fs::read(&path).unwrap();
            assert!(
                matches!(Memory::load(&path), Err(MemoryRefusal::Payload { .. })),
                "rule {broken}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), before);
            // save of the invalid candidate: refused before any file is made
            let fresh = directory.join(format!("never-{broken}.rkyv"));
            assert!(matches!(
                memory.save(&fresh),
                Err(MemoryRefusal::Payload { .. })
            ));
            assert!(!fresh.exists(), "no memory was written");
            assert!(!path.with_extension("new").exists(), "no temporary file");
            assert!(
                matches!(memory.save(&path), Err(MemoryRefusal::Payload { .. })),
                "an invalid candidate over a refused file is refused too"
            );
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
        let _ = std::fs::remove_dir_all(&directory);
    }
}
