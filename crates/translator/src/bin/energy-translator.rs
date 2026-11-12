use energy_nexus_translator::flow::GreetingWait;
use energy_nexus_translator::{Translating, Translator};
use energy_nexus_wire::{Place, PlaceName};
fn main() {
    let mut arguments = std::env::args().skip(1);
    let name = arguments
        .next()
        .expect("usage: energy-translator NAME RUNTIME STATE [GREETING_WAIT_SECONDS]");
    let runtime = arguments
        .next()
        .map(std::path::PathBuf::from)
        .expect("runtime");
    let state = arguments
        .next()
        .map(std::path::PathBuf::from)
        .expect("state");
    // How long each read of Flow's greeting receipt waits, in whole seconds;
    // thirty where none is given.
    let greeting_wait = match arguments.next() {
        Some(seconds) => seconds
            .parse::<GreetingWait>()
            .unwrap_or_else(|refusal| panic!("greeting wait: {refusal}")),
        None => GreetingWait::default(),
    };
    Translator::open(
        Place {
            name: PlaceName(name),
            runtime,
            state,
        },
        greeting_wait,
    )
    .expect("open the translator's memory")
    .serve()
    .expect("serve translator")
}
