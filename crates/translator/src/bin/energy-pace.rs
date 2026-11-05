use energy_nexus_wire::{Framing, Place, SocketPathing};
use signal::{ByteViewable, Restorable, Signal, Signalizable};
use signal_energy_nexus::{Name, Provider, Query, Response, Target};
use std::os::unix::net::UnixStream;

fn read(place: &Place, provider: Provider) -> Result<signal_energy_nexus::Read_Data, String> {
    let mut stream = UnixStream::connect(place.public()).map_err(|error| error.to_string())?;
    let query = Query::Read(provider)
        .signalize()
        .map_err(|error| format!("{error:?}"))?;
    stream
        .write_frame(query.bytes())
        .map_err(|error| error.to_string())?;
    let bytes = stream.read_frame().map_err(|error| error.to_string())?;
    match Signal::<Response>::from(bytes)
        .restore()
        .map_err(|error| format!("{error:?}"))?
    {
        Response::Read(read) => Ok(read),
        other => Err(format!("read refused: {other:?}")),
    }
}

fn show(read: signal_energy_nexus::Read_Data) {
    let pace = read.pace;
    let window = read
        .reading
        .window_vector
        .iter()
        .find(|window| window.name == Name::Week)
        .or_else(|| read.reading.window_vector.first());
    let period = window
        .map(|window| format!("{} days", window.duration.0 as f64 / 1_440.0))
        .unwrap_or_else(|| "the current period".into());
    let target = match pace.target {
        Target::Even => "evenly".into(),
        Target::SpendBy(instant) => format!("by {}", instant.0),
    };
    println!(
        "{:?}: {}% used over {}; target {} expects {}% now; {:?}, with {}% per day from here.",
        pace.provider, pace.used.0, period, target, pace.expected.0, pace.standing, pace.per_day.0
    );
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let runtime = arguments
        .next()
        .map(std::path::PathBuf::from)
        .expect("usage: energy-pace RUNTIME [Claude|Codex]");
    let providers = match arguments.next().as_deref() {
        Some("Claude") => vec![Provider::Claude],
        Some("Codex") => vec![Provider::Codex],
        Some(_) => panic!("provider is Claude or Codex"),
        None => vec![Provider::Claude, Provider::Codex],
    };
    let place = Place { runtime };
    for provider in providers {
        show(read(&place, provider).expect("read energy pace"));
    }
}
