//! Typed external-call vocabulary; execution remains the translator's responsibility.
mod contract;
pub mod telemetry;
pub use contract::*;

pub const ETHOS: &str = concat!(
    include_str!("../Cargo.toml"),
    "\n",
    include_str!("../../../ethos/telemetry.ethos"),
    "\n",
    include_str!("../../../ethos/emission-stage-one.ethos"),
);
impl signal::Contracted for Query {
    const CONTRACT_SOURCE: &'static str = ETHOS;
}
impl telemetry::AttemptReporting for telemetry::AttemptRecord {
    fn record(&self) -> telemetry::AttemptRecord {
        self.clone()
    }
}
