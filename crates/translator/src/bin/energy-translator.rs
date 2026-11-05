use energy_nexus_translator::{Programs, Translating, Translator};
use energy_nexus_wire::Place;
fn main() {
    let runtime = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("usage: energy-translator RUNTIME");
    Translator {
        place: Place { runtime },
        programs: Programs::default(),
    }
    .serve()
    .expect("serve translator")
}
