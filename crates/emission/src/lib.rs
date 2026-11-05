use signal_energy_nexus::{Level, Provider, Reading, Recipient, Reset, ResetId};
macro_rules! wire {
    ($item:item) => {
        #[derive(
            rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq,
        )]
        $item
    };
}
wire! {pub struct Idempotency(pub String);}
wire! {pub enum Emission { CodexRateLimits, CodexConsumeReset { reset_id: ResetId, idempotency: Idempotency }, ClaudeUsage, Send { recipient: Recipient, provider: Provider, level: Level } }}
wire! {pub enum Consumed { Reset(Reset), NothingToReset, NoCredit, AlreadyRedeemed }}
wire! {pub enum Outcome { Observed(Reading), Consumed(Consumed), Sent, Down }}
pub trait Emittable {
    type Outcome;
    fn emission(&self) -> Emission;
}
pub enum Never {}
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
