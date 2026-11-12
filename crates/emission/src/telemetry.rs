#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    clippy::type_complexity
)]
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum Trigger {
    Subscription,
    TurnEnded,
    WindowReset,
    ExplicitRead,
    Redemption,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct DemandCount(pub i64);
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct TriggerDemand {
    pub trigger: Trigger,
    pub demand_count: DemandCount,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct AttemptContext {
    pub attempt_id: signal_energy_nexus::library::AttemptId,
    pub trigger_demand_vector: std::vec::Vec<TriggerDemand>,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct DurationMilliseconds(pub i64);
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum AttemptOutcome {
    Observed,
    Failure(signal_energy_nexus::library::Failure),
    RedemptionResult(signal_energy_nexus::library::RedemptionResult),
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct Started_Data {
    pub provider: signal_energy_nexus::library::Provider,
    pub attempt_context: AttemptContext,
    pub instant: signal_energy_nexus::library::Instant,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct Rejected_Data {
    pub provider: signal_energy_nexus::library::Provider,
    pub attempt_context: AttemptContext,
    pub instant: signal_energy_nexus::library::Instant,
    pub failure: signal_energy_nexus::library::Failure,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct Completed_Data {
    pub attempt_id: signal_energy_nexus::library::AttemptId,
    pub instant: signal_energy_nexus::library::Instant,
    pub duration_milliseconds: DurationMilliseconds,
    pub attempt_outcome: AttemptOutcome,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum AttemptRecord {
    Started(Started_Data),
    Rejected(Rejected_Data),
    Completed(Completed_Data),
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum ReportingFailure {
    SinkUnavailable,
    CountOverflow,
    ClockUnavailable,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum ReportResult {
    Written,
    ReportingFailure(ReportingFailure),
}
#[rustfmt::skip]
pub trait AttemptReporting {
    fn record(&self) -> AttemptRecord;
}
#[rustfmt::skip]
pub trait AttemptReportable {
    fn report<N: AttemptReporting>(&mut self, input: N) -> ReportResult;
}
#[rustfmt::skip]
const _: () = {
    fn assert_attemptrecord_attemptreporting<T: AttemptReporting>() {}
    let _ = assert_attemptrecord_attemptreporting::<AttemptRecord>;
};
