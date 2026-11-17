use datom_codec::{Actualizing, Potential};
use energy_nexus_translator::cli::ConfiguredPlace;
use energy_nexus_translator::{TextClienting, Translator};
use energy_nexus_wire::Place;
use signal_energy_nexus::{Decimating, Output, Provider, Standing, Target, TurnEndSubscription};

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
    let mut potential = Potential::<Output>::from(answer.as_str());
    match potential
        .actualize(&mut budget)
        .map_err(|error| format!("{error:?}"))?
    {
        Output::Read(read) => Ok(read),
        other => Err(format!("read refused: {other:?}")),
    }
}

/// When the reading was read, and how long ago, in the system's zone; and,
/// when Flow's turn ends do not reach the Nexus, that the reading is stale
/// and since when.
fn when(read: &signal_energy_nexus::Read_Data) {
    let zone = jiff::tz::TimeZone::system();
    let local = |seconds: i64| match jiff::Timestamp::from_second(seconds) {
        Ok(instant) => instant.to_zoned(zone.clone()).strftime("%Y-%m-%d %H:%M %Z").to_string(),
        Err(_) => format!("instant {seconds}"),
    };
    let at = read.reading.at.0 .0;
    let age = jiff::Timestamp::now().as_second() - at;
    let age = if age < 3_600 {
        format!("{} minutes", age / 60)
    } else {
        format!("{:.1} hours", age as f64 / 3_600.0)
    };
    match &read.turn_end_subscription {
        TurnEndSubscription::Held => {
            println!("{:?} read at {}, {age} ago.", read.reading.provider, local(at))
        }
        TurnEndSubscription::Mismatched(mismatch) => println!(
            "{:?} STALE: read at {}, {age} ago. Flow has refused Energy's turn-end subscription \
             since {}, contract mismatch (Flow {}, Energy {}): no reading is taken until both \
             speak one contract.",
            read.reading.provider,
            local(at),
            local(mismatch.since.0 .0),
            mismatch.flow_contract.0 .0,
            mismatch.own_contract.0 .0
        ),
    }
}

fn show(read: signal_energy_nexus::Read_Data) {
    when(&read);
    for pace in read.pace_vector {
        let window = read
            .reading
            .window_vector
            .iter()
            .find(|window| window.name == pace.name);
        let period = window
            .map(|window| format!("{} days", window.duration.0.0 as f64 / 1_440.0))
            .unwrap_or_else(|| "the current period".into());
        let target = match pace.target {
            Target::Even => "evenly".into(),
            Target::SpendBy(spend) => format!("{}% by {} hours before the window resets", spend.percent.0, spend.hours.0),
        };
        // A Session spent out is a fault in the pace of work.
        let standing = match pace.standing {
            Standing::Spent => "Spent, a fault in the pace of work".to_string(),
            other => format!("{other:?}"),
        };
        println!(
            "{:?} {:?}: {}% used over {}; target {} expects {}% now, counted in useful hours; {:?}. {} hours left, {} useful hours; {}% per useful hour from here.",
            pace.provider,
            pace.name,
            pace.used.0.0,
            period,
            target,
            pace.expected.0.0,
            standing,
            pace.hours_left.0.0,
            pace.useful_left.0.float(),
            pace.per_useful_hour.0.float()
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
