use meta_signal_energy_nexus::Set;
use signal_energy_nexus::{
    Level, Minutes, Percent, Provider, Reading, Reminder, Reset, ResetStatus, Threshold,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Latest {
    pub provider: Provider,
    pub reading: Reading,
    pub level: Level,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configuration {
    pub polling: Minutes,
    pub thresholds: Vec<Threshold>,
    pub reminders: Vec<Reminder>,
    pub redeem_expiring: Minutes,
    pub redeem_used_at_least: Percent,
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
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Memory {
    latest: Vec<Latest>,
    history: Vec<Reading>,
    configuration: Configuration,
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
    use signal_energy_nexus::{Instant, Name, Window};
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
}
