use meta_signal_energy_nexus::{ProviderTarget, Set, WindowTarget};
use signal_energy_nexus::{
    Level, Minutes, Pace, Percent, Provider, Reading, Reminder, Reset, ResetStatus, Standing,
    Target, Threshold,
};

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Latest {
    pub provider: Provider,
    pub reading: Reading,
    pub level: Level,
}
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Configuration {
    pub polling: Minutes,
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
            polling: Minutes(5),
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
impl Memory {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        rkyv::from_bytes::<Self, rkyv::rancor::Error>(&bytes).map_err(|error| error.to_string())
    }
    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let bytes =
            rkyv::to_bytes::<rkyv::rancor::Error>(self).map_err(|error| error.to_string())?;
        let temporary = path.with_extension("new");
        std::fs::write(&temporary, bytes.as_slice()).map_err(|error| error.to_string())?;
        std::fs::rename(temporary, path).map_err(|error| error.to_string())
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
            Set::Polling(value) | Set::RedeemExpiring(value) if value.0 <= 0 => {
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
            Set::Polling(value) => self.configuration.polling = value,
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

        let restored = Memory::load(&path).unwrap();
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
}
