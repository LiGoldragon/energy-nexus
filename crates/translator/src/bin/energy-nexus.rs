use energy_nexus_translator::{Programs, TextClienting, Translator};
use energy_nexus_wire::Place;
fn main() {
    let mut args = std::env::args().skip(1);
    let runtime = args
        .next()
        .map(std::path::PathBuf::from)
        .expect("usage: energy-nexus RUNTIME DATOM");
    let text = args.next().expect("datom");
    let follow = text == "Subscribe";
    let client = Translator {
        place: Place { runtime },
        programs: Programs::default(),
    };
    for answer in client.request(&text, false, follow).expect("request") {
        println!("{answer}");
    }
}
