use energy_nexus_translator::{TextClienting, Translator};
use energy_nexus_wire::{Place, PlaceName};
fn main() {
    let mut args = std::env::args().skip(1);
    let name = args.next().expect("usage: energy-nexus-meta PLACE DATOM");
    let text = args.next().expect("datom");
    let client = Translator::client(Place::resolve(PlaceName(name)).expect("resolve place"));
    for answer in client.request(&text, true, false).expect("request") {
        println!("{answer}");
    }
}
