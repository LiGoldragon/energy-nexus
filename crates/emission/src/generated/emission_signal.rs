#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    clippy::type_complexity
)]
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct TurnEnd {
    pub provider: signal_energy_nexus::Provider,
    pub at: signal_energy_nexus::At,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum Input {
    Emit(crate::Emission),
    TurnEnds,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum Output {
    Emitted(crate::Outcome),
    TurnEndsObserved(std::vec::Vec<TurnEnd>),
    TurnEnded(TurnEnd),
    Down,
    TurnEndsRefused(signal_energy_nexus::ContractMismatch),
}
#[rustfmt::skip]
#[allow(clippy::result_large_err, clippy::ptr_arg, clippy::borrowed_box)]
pub trait Answering {
    fn emit(
        &self,
        emission: &crate::Emission,
    ) -> std::result::Result<crate::Outcome, Output>;
    fn turn_ends(&self) -> std::result::Result<std::vec::Vec<TurnEnd>, Output>;
}
#[rustfmt::skip]
pub trait Answerable {
    fn answer<Nexus: Answering>(&self, nexus: &Nexus) -> Output;
}
#[rustfmt::skip]
impl Answerable for Input {
    fn answer<Nexus: Answering>(&self, nexus: &Nexus) -> Output {
        match self {
            Input::Emit(carried) => {
                nexus.emit(carried).map_or_else(|refusal| refusal, Output::Emitted)
            }
            Input::TurnEnds => {
                nexus
                    .turn_ends()
                    .map_or_else(|refusal| refusal, Output::TurnEndsObserved)
            }
        }
    }
}
