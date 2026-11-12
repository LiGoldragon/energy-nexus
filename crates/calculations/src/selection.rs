#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    clippy::type_complexity
)]
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum WeekUsage {
    Percent(signal_energy_nexus::library::Percent),
    Unknown,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct CreditSelection {
    pub credits: signal_energy_nexus::library::Credits,
    pub allowance: signal_energy_nexus::library::Allowance,
    pub instant: signal_energy_nexus::library::Instant,
    pub redeem_expiring: meta_signal_energy_nexus::library::RedeemExpiring,
    pub redeem_used_at_least: meta_signal_energy_nexus::library::RedeemUsedAtLeast,
    pub week_usage: WeekUsage,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum SelectionResult {
    Reset(signal_energy_nexus::library::Reset),
    NoReset,
    Unproved,
    RuleRefused,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct LevelCalculation {
    pub reading: signal_energy_nexus::library::Reading,
    pub threshold_vector: std::vec::Vec<signal_energy_nexus::library::Threshold>,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct PaceCalculation {
    pub provider: signal_energy_nexus::library::Provider,
    pub window: signal_energy_nexus::library::Window,
    pub target: signal_energy_nexus::library::Target,
    pub instant: signal_energy_nexus::library::Instant,
}
#[rustfmt::skip]
pub trait CreditSelectable {
    fn select(&self) -> SelectionResult;
}
#[rustfmt::skip]
pub trait Levellable {
    fn level(&self) -> signal_energy_nexus::library::Level;
}
#[rustfmt::skip]
pub trait PaceCalculable {
    fn pace(&self) -> signal_energy_nexus::library::Pace;
}
#[rustfmt::skip]
const _: () = {
    fn assert_creditselection_creditselectable<T: CreditSelectable>() {}
    let _ = assert_creditselection_creditselectable::<CreditSelection>;
};
#[rustfmt::skip]
const _: () = {
    fn assert_levelcalculation_levellable<T: Levellable>() {}
    let _ = assert_levelcalculation_levellable::<LevelCalculation>;
};
#[rustfmt::skip]
const _: () = {
    fn assert_pacecalculation_pacecalculable<T: PaceCalculable>() {}
    let _ = assert_pacecalculation_pacecalculable::<PaceCalculation>;
};
