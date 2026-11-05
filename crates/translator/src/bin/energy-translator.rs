use energy_nexus_translator::{Programs, Translating, Translator};
use energy_nexus_wire::Place;
fn main() {
    let runtime = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("usage: energy-translator RUNTIME");
    let mut programs = Programs::default();
    if let Ok(program) = std::env::var("ENERGY_CODEX") {
        programs.codex = program;
    }
    if let Ok(program) = std::env::var("ENERGY_FLOW") {
        programs.flow = program;
    }
    Translator {
        place: Place { runtime },
        programs,
    }
    .serve()
    .expect("serve translator")
}
