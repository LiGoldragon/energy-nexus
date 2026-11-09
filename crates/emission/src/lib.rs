use signal_energy_nexus::{Instant, Level, Provider, Reading, Recipient, Reset, ResetId};
macro_rules! wire {
    ($item:item) => {
        #[derive(
            rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq,
        )]
        $item
    };
}
wire! {pub struct Idempotency(pub String);}
wire! {pub struct CodexRateLimits;}
wire! {pub struct CodexConsumeReset { pub reset_id: ResetId, pub idempotency: Idempotency }}
wire! {pub struct ClaudeUsage;}
wire! {pub struct Send { pub recipient: Recipient, pub provider: Provider, pub level: Level }}
wire! {
    /// The Flow Nexus's turn ends at the place, subscribed to: answered with
    /// the last turn end on each provider, then each turn end as it comes,
    /// until either side closes or Flow goes (`Down`).
    pub struct TurnEnds;
}
wire! {
    /// A turn ended on a provider: a flow of its harness finished a turn.
    pub struct TurnEnd { pub provider: Provider, pub at: Instant }
}
wire! {pub enum Emission { CodexRateLimits(CodexRateLimits), CodexConsumeReset(CodexConsumeReset), ClaudeUsage(ClaudeUsage), Send(Send), TurnEnds(TurnEnds) }}
wire! {pub enum Consumed { Reset(Reset), NothingToReset, NoCredit, AlreadyRedeemed }}
wire! {pub enum Outcome { Observed(Reading), Consumed(Consumed), Sent, TurnEndsObserved(Vec<TurnEnd>), TurnEnded(TurnEnd), Down }}
pub trait Emittable<T> {
    fn emit(&self, translator: &T) -> Outcome;
}
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Query {
    Emit(Emission),
}
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Emitted(Outcome),
}
pub const ETHOS: &str = include_str!("../../../ethos/emission.ethos");
impl signal::Contracted for Query {
    const CONTRACT_SOURCE: &'static str = ETHOS;
}
