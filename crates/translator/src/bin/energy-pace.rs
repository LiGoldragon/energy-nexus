use datom_codec::{Actualizing, Potential};
use energy_nexus_translator::{TextClienting, Translator};
use energy_nexus_wire::{Place, PlaceName};
use signal_energy_nexus::{Provider, Response, Target};

fn read(client: &Translator, provider: Provider) -> Result<signal_energy_nexus::Read_Data, String> {
    let text = match provider {
        Provider::Claude => "Read.Claude",
        Provider::Codex => "Read.Codex",
    };
    let answer = client
        .request(text, false, false)?
        .into_iter()
        .next()
        .ok_or("no answer")?;
    let mut budget = datom_codec::Budget {
        remaining: 100_000,
        reader: protos::ReaderBudget::try_from(100_000).expect("the reader budget is within range"),
        depth: 0,
        maximum_depth: 128,
    };
    let mut potential = Potential::<Response>::from(answer.as_str());
    match potential
        .actualize(&mut budget)
        .map_err(|error| format!("{error:?}"))?
    {
        Response::Read(read) => Ok(read),
        other => Err(format!("read refused: {other:?}")),
    }
}

fn show(read: signal_energy_nexus::Read_Data) {
    for pace in read.pace_vector {
        let window = read
            .reading
            .window_vector
            .iter()
            .find(|window| window.name == pace.name);
        let period = window
            .map(|window| format!("{} days", window.duration.0 as f64 / 1_440.0))
            .unwrap_or_else(|| "the current period".into());
        let target = match pace.target {
            Target::Even => "evenly".into(),
            Target::SpendBy(instant) => format!("by {}", instant.0),
        };
        println!(
            "{:?} {:?}: {}% used over {}; target {} expects {}% now; {:?}, with {}% per day from here.",
            pace.provider,
            pace.name,
            pace.used.0,
            period,
            target,
            pace.expected.0,
            pace.standing,
            pace.per_day.0
        );
    }
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let name = arguments
        .next()
        .expect("usage: energy-pace PLACE [Claude|Codex]");
    let providers = match arguments.next().as_deref() {
        Some("Claude") => vec![Provider::Claude],
        Some("Codex") => vec![Provider::Codex],
        Some(_) => panic!("provider is Claude or Codex"),
        None => vec![Provider::Claude, Provider::Codex],
    };
    let client = Translator::open(Place::resolve(PlaceName(name)).expect("resolve place"));
    for provider in providers {
        show(read(&client, provider).expect("read energy pace"));
    }
}
