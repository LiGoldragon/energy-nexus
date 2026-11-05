use energy_nexus_translator::{Programs, TextClienting, Translator};
use energy_nexus_wire::Place;
fn main() {
    let mut args = std::env::args().skip(1);
    let runtime = args
        .next()
        .map(std::path::PathBuf::from)
        .expect("usage: energy-nexus-meta RUNTIME DATOM");
    let text = args.next().expect("datom");
    let client = Translator {
        place: Place { runtime },
        programs: Programs::default(),
    };
    for answer in client.request(&text, true, false).expect("request") {
        println!("{answer}");
    }
}
