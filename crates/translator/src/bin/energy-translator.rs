use energy_nexus_translator::{Translating, Translator};
use energy_nexus_wire::{Place, PlaceName};
fn main() {
    let mut arguments = std::env::args().skip(1);
    let name = arguments
        .next()
        .expect("usage: energy-translator NAME RUNTIME STATE");
    let runtime = arguments
        .next()
        .map(std::path::PathBuf::from)
        .expect("runtime");
    let state = arguments
        .next()
        .map(std::path::PathBuf::from)
        .expect("state");
    Translator::open(Place {
        name: PlaceName(name),
        runtime,
        state,
    })
    .expect("open the translator's memory")
    .serve()
    .expect("serve translator")
}
