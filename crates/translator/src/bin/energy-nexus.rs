use energy_nexus_translator::cli::ConfiguredPlace;
use energy_nexus_translator::{TextClienting, Translator};
use energy_nexus_wire::Place;
/// `energy-nexus '<datom>'`: the datom and nothing else; the place is the
/// configuration's, `$XDG_CONFIG_HOME/nexus/energy-nexus.datom`.
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [text] = arguments.as_slice() else {
        eprintln!("usage: energy-nexus '<datom>'; the place is read from $XDG_CONFIG_HOME/nexus/energy-nexus.datom");
        std::process::exit(2)
    };
    let place = Place::configured().unwrap_or_else(|refusal| {
        eprintln!("{refusal}");
        std::process::exit(2)
    });
    let follow = text == "Subscribe";
    let client = Translator::client(place);
    for answer in client.request(text, false, follow).expect("request") {
        println!("{answer}");
    }
}
