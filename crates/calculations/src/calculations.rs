use crate::selection::*;
use signal_energy_nexus::library::*;

impl CreditSelectable for CreditSelection {
    fn select(&self) -> SelectionResult {
        if self.instant.0 < 0 {
            return SelectionResult::Unproved;
        }
        if self.redeem_expiring.0 <= 0 || !(0..=100).contains(&self.redeem_used_at_least.0) {
            return SelectionResult::RuleRefused;
        }
        let known = match &self.credits {
            Credits::Known(known) => known,
            Credits::NotApplicable => return SelectionResult::NoReset,
            Credits::Unknown => return SelectionResult::Unproved,
        };
        if known.available_count.0 < 0 {
            return SelectionResult::Unproved;
        }
        let resets = match &known.reset_details {
            ResetDetails::Complete(resets) => resets,
            ResetDetails::Partial(_) => return SelectionResult::Unproved,
            ResetDetails::Unknown => {
                return if known.available_count.0 == 0 {
                    SelectionResult::NoReset
                } else {
                    SelectionResult::Unproved
                };
            }
        };
        let mut identifiers = std::collections::HashSet::new();
        let mut available = 0_i64;
        let mut selected: Option<&Reset> = None;
        for reset in resets {
            if reset.reset_id.0.is_empty() || !identifiers.insert(&reset.reset_id.0) {
                return SelectionResult::Unproved;
            }
            match reset.reset_status {
                ResetStatus::Unknown => return SelectionResult::Unproved,
                ResetStatus::Redeeming | ResetStatus::Redeemed => continue,
                ResetStatus::Available => available += 1,
            }
            if reset.reset_kind != ResetKind::RateLimits {
                return SelectionResult::Unproved;
            }
            match reset.expiry {
                Expiry::Unknown => return SelectionResult::Unproved,
                Expiry::Instant(instant) if instant.0 < 0 => return SelectionResult::Unproved,
                Expiry::Instant(instant) if instant.0 <= self.instant.0 => continue,
                Expiry::Instant(_) | Expiry::NoExpiry => {}
            }
            let earlier = match selected {
                None => true,
                Some(previous) => match (&reset.expiry, &previous.expiry) {
                    (Expiry::Instant(left), Expiry::Instant(right)) => {
                        left < right || (left == right && reset.reset_id < previous.reset_id)
                    }
                    (Expiry::Instant(_), Expiry::NoExpiry) => true,
                    (Expiry::NoExpiry, Expiry::Instant(_)) => false,
                    (Expiry::NoExpiry, Expiry::NoExpiry) => reset.reset_id < previous.reset_id,
                    (Expiry::Unknown, _) | (_, Expiry::Unknown) => unreachable!(),
                },
            };
            if earlier {
                selected = Some(reset);
            }
        }
        if available != known.available_count.0 {
            return SelectionResult::Unproved;
        }
        let Some(reset) = selected else {
            return SelectionResult::NoReset;
        };
        let expiring = match reset.expiry {
            Expiry::Instant(instant) => {
                i128::from(instant.0) - i128::from(self.instant.0)
                    <= i128::from(self.redeem_expiring.0) * 60
            }
            Expiry::NoExpiry | Expiry::Unknown => false,
        };
        let used_enough = match self.week_usage {
            WeekUsage::Percent(percent) => {
                (0..=100).contains(&percent.0) && percent.0 >= self.redeem_used_at_least.0
            }
            WeekUsage::Unknown => false,
        };
        if self.allowance == Allowance::Disallowed || (expiring && used_enough) {
            SelectionResult::Reset(reset.clone())
        } else {
            SelectionResult::RuleRefused
        }
    }
}

impl Levellable for LevelCalculation {
    fn level(&self) -> Level {
        match self.reading.allowance {
            Allowance::Disallowed => return Level::Out,
            Allowance::Unknown => return Level::Unknown,
            Allowance::Allowed => {}
        }
        let windows = match &self.reading.windows {
            Windows::Known(windows) if !windows.is_empty() => windows,
            Windows::Known(_) | Windows::Partial(_) | Windows::MissingEvidence(_) => {
                return Level::Unknown;
            }
        };
        let mut identities = std::collections::HashSet::new();
        let mut maximum_used = 0;
        for window in windows {
            if window.name == Name::Unknown || !identities.insert(&window.name) {
                return Level::Unknown;
            }
            match window.usage {
                Usage::Percent(percent) if (0..=100).contains(&percent.0) => {
                    maximum_used = maximum_used.max(percent.0);
                }
                Usage::Percent(_) | Usage::Unknown => return Level::Unknown,
            }
        }
        let mut boundaries = std::collections::HashMap::new();
        for threshold in &self.threshold_vector {
            if matches!(threshold.level, Level::Out | Level::Unknown)
                || !(0..=100).contains(&threshold.left.0)
                || boundaries
                    .insert(threshold.left.0, &threshold.level)
                    .is_some_and(|previous| previous != &threshold.level)
            {
                return Level::Unknown;
            }
        }
        let left = 100 - maximum_used;
        self.threshold_vector
            .iter()
            .filter(|threshold| left >= threshold.left.0)
            .max_by_key(|threshold| threshold.left.0)
            .map(|threshold| threshold.level.clone())
            .unwrap_or(Level::Low)
    }
}

impl PaceCalculable for PaceCalculation {
    fn pace(&self) -> Pace {
        let unavailable = |reason| {
            Pace::Unavailable(Unavailable_Data {
                provider: self.provider.clone(),
                name: self.window.name.clone(),
                pace_unavailable: reason,
            })
        };
        if self.window.name == Name::Unknown {
            return unavailable(PaceUnavailable::IdentityMissing);
        }
        let used = match self.window.usage {
            Usage::Percent(percent) if (0..=100).contains(&percent.0) => percent.0,
            Usage::Percent(_) | Usage::Unknown => {
                return unavailable(PaceUnavailable::UsageMissing)
            }
        };
        let resets = match self.window.reset_time {
            ResetTime::Instant(instant) if instant.0 >= 0 => i128::from(instant.0),
            ResetTime::Instant(_) | ResetTime::Unknown => {
                return unavailable(PaceUnavailable::ResetTimeMissing);
            }
        };
        let duration = match self.window.window_duration {
            WindowDuration::Minutes(minutes) if minutes.0 > 0 => i128::from(minutes.0) * 60,
            WindowDuration::Minutes(_) | WindowDuration::Unknown => {
                return unavailable(PaceUnavailable::DurationMissing);
            }
        };
        let deadline = match self.target {
            Target::Even => resets,
            Target::SpendBy(instant) if instant.0 >= 0 => i128::from(instant.0),
            Target::SpendBy(_) => return unavailable(PaceUnavailable::ResetTimeMissing),
        };
        if self.instant.0 < 0 {
            return unavailable(PaceUnavailable::ResetTimeMissing);
        }
        let now = i128::from(self.instant.0);
        let start = resets - duration;
        let span = (deadline - start).max(1);
        let elapsed = (now - start).clamp(0, span);
        let expected = ((elapsed * 100 + span / 2) / span).clamp(0, 100) as i64;
        let seconds_left = (deadline - now).max(1);
        let per_day = if used >= 100 || deadline <= now {
            0
        } else {
            (((i128::from(100 - used) * 86_400 + seconds_left / 2) / seconds_left).max(1)) as i64
        };
        Pace::Calculated(Calculated_Data {
            provider: self.provider.clone(),
            name: self.window.name.clone(),
            target: self.target.clone(),
            expected: Expected(expected),
            used: Used(used),
            standing: match used.cmp(&expected) {
                std::cmp::Ordering::Greater => Standing::Over,
                std::cmp::Ordering::Less => Standing::Under,
                std::cmp::Ordering::Equal => Standing::On,
            },
            per_day: PerDay(per_day),
        })
    }
}
