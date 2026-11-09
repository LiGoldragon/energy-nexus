use datom_codec::{Actualizing, Potential};
use energy_nexus_translator::cli::ConfiguredPlace;
use energy_nexus_translator::{TextClienting, Translator};
use energy_nexus_wire::Place;
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

/// `energy-pace [Claude|Codex]`: no place is typed; it is the
/// configuration's, `$XDG_CONFIG_HOME/nexus/energy-nexus.datom`.
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let providers = match arguments.as_slice() {
        [] => vec![Provider::Claude, Provider::Codex],
        [only] if only == "Claude" => vec![Provider::Claude],
        [only] if only == "Codex" => vec![Provider::Codex],
        _ => {
            eprintln!("usage: energy-pace [Claude|Codex]; the place is read from $XDG_CONFIG_HOME/nexus/energy-nexus.datom");
            std::process::exit(2)
        }
    };
    let place = Place::configured().unwrap_or_else(|refusal| {
        eprintln!("{refusal}");
        std::process::exit(2)
    });
    let client = Translator::client(place);
    for provider in providers {
        show(read(&client, provider).expect("read energy pace"));
    }
}
