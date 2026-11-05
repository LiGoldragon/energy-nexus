use energy_nexus_core::{Configuring, Memory, RedeemDecision, Redeeming, Remembering, Reminding};
use energy_nexus_emission::{
    Consumed, Emission, Idempotency, Outcome as ExternalOutcome, Query as EmissionQuery,
    Response as EmissionResponse,
};
use energy_nexus_wire::{Framing, Place, SocketPathing};
use signal::{ByteViewable, Restorable, Signal, Signalizable};
use signal_energy_nexus::{Changed_Data, Provider, Query, Refused, Response};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};

struct Nexus {
    memory: Memory,
    subscribers: Vec<UnixStream>,
    place: Place,
}
trait Emitting {
    fn emit(&self, emission: Emission) -> Result<ExternalOutcome, String>;
}
impl Emitting for Nexus {
    fn emit(&self, emission: Emission) -> Result<ExternalOutcome, String> {
        let mut stream =
            UnixStream::connect(self.place.emission()).map_err(|error| error.to_string())?;
        let signal = EmissionQuery::Emit(emission)
            .signalize()
            .map_err(|error| format!("{error:?}"))?;
        stream
            .write_frame(signal.bytes())
            .map_err(|error| error.to_string())?;
        let bytes = stream.read_frame().map_err(|error| error.to_string())?;
        let response = Signal::<EmissionResponse>::from(bytes)
            .restore()
            .map_err(|error| format!("{error:?}"))?;
        let EmissionResponse::Emitted(outcome) = response;
        Ok(outcome)
    }
}
trait Answering {
    fn answer(&mut self, query: Query) -> Response;
    fn read(&mut self, provider: Provider) -> Response;
    fn redeem(&mut self, provider: Provider) -> Response;
}
impl Answering for Nexus {
    fn answer(&mut self, query: Query) -> Response {
        match query {
            Query::Read(provider) => self.read(provider),
            Query::Subscribe => Response::Subscribed,
            Query::Redeem(provider) => self.redeem(provider),
        }
    }
    fn read(&mut self, provider: Provider) -> Response {
        let emission = match provider {
            Provider::Codex => Emission::CodexRateLimits,
            Provider::Claude => Emission::ClaudeUsage,
        };
        match self.emit(emission) {
            Ok(ExternalOutcome::Observed(reading)) => {
                let change = self.memory.remember(reading.clone());
                if change.old.as_ref() != Some(&change.latest.level) {
                    let changed = Response::Changed(Changed_Data {
                        provider: provider.clone(),
                        level: change.latest.level.clone(),
                        reading: reading.clone(),
                    });
                    push(&mut self.subscribers, &changed);
                    for reminder in self.memory.reminders_for(&provider, &change.latest.level) {
                        let _ = self.emit(Emission::Send {
                            recipient: reminder.recipient,
                            provider: provider.clone(),
                            level: change.latest.level.clone(),
                        });
                    }
                }
                Response::Read(reading)
            }
            _ => Response::Refused(Refused::ProviderDown(provider)),
        }
    }
    fn redeem(&mut self, provider: Provider) -> Response {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        match self.memory.redeem_decision(&provider, now) {
            RedeemDecision::Allowed(reset) => {
                let key = Idempotency(format!("energy:{}:{}", reset.reset_id.0, now));
                match self.emit(Emission::CodexConsumeReset {
                    reset_id: reset.reset_id.clone(),
                    idempotency: key,
                }) {
                    Ok(ExternalOutcome::Consumed(Consumed::Reset(redeemed))) => {
                        Response::Redeemed(redeemed)
                    }
                    Ok(ExternalOutcome::Consumed(Consumed::AlreadyRedeemed)) => {
                        Response::Redeemed(reset)
                    }
                    _ => Response::Refused(Refused::ProviderDown(provider)),
                }
            }
            RedeemDecision::NoReset => Response::Refused(Refused::NoReset),
            RedeemDecision::Refused(reason) => Response::Refused(Refused::RuleRefused(reason)),
        }
    }
}
fn push(subscribers: &mut Vec<UnixStream>, response: &Response) {
    let Ok(signal) = response.signalize() else {
        return;
    };
    subscribers.retain_mut(|stream| stream.write_frame(signal.bytes()).is_ok());
}
trait Serving {
    fn serve(self) -> std::io::Result<()>;
}
impl Serving for Place {
    fn serve(self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.runtime)?;
        for path in [self.public(), self.meta()] {
            let _ = std::fs::remove_file(path);
        }
        let public = UnixListener::bind(self.public())?;
        let meta = UnixListener::bind(self.meta())?;
        let nexus = Arc::new(Mutex::new(Nexus {
            memory: Memory::default(),
            subscribers: Vec::new(),
            place: self.clone(),
        }));
        {
            let nexus = Arc::clone(&nexus);
            std::thread::spawn(move || loop {
                let minutes = nexus
                    .lock()
                    .ok()
                    .map(|guard| guard.memory.configuration().polling.0)
                    .unwrap_or(5)
                    .max(1);
                if let Ok(mut guard) = nexus.lock() {
                    let _ = guard.read(Provider::Claude);
                    let _ = guard.read(Provider::Codex);
                }
                std::thread::sleep(std::time::Duration::from_secs(
                    (minutes as u64).saturating_mul(60),
                ));
            });
        }
        {
            let nexus = Arc::clone(&nexus);
            std::thread::spawn(move || {
                for stream in meta.incoming().flatten() {
                    let nexus = Arc::clone(&nexus);
                    std::thread::spawn(move || serve_meta(stream, nexus));
                }
            });
        }
        for stream in public.incoming().flatten() {
            let nexus = Arc::clone(&nexus);
            std::thread::spawn(move || serve_public(stream, nexus));
        }
        Ok(())
    }
}
fn serve_public(mut stream: UnixStream, nexus: Arc<Mutex<Nexus>>) {
    let Ok(bytes) = stream.read_frame() else {
        return;
    };
    let Ok(query) = Signal::<Query>::from(bytes).restore() else {
        return;
    };
    let subscribe = matches!(query, Query::Subscribe);
    let response = nexus.lock().ok().map(|mut n| n.answer(query));
    if let Some(response) = response {
        if let Ok(signal) = response.signalize() {
            if stream.write_frame(signal.bytes()).is_ok() && subscribe {
                if let Ok(mut n) = nexus.lock() {
                    n.subscribers.push(stream)
                }
            }
        }
    }
}
fn serve_meta(mut stream: UnixStream, nexus: Arc<Mutex<Nexus>>) {
    let Ok(bytes) = stream.read_frame() else {
        return;
    };
    let Ok(query) = Signal::<meta_signal_energy_nexus::Query>::from(bytes).restore() else {
        return;
    };
    let meta_signal_energy_nexus::Query::Configure(set) = query;
    let response = match nexus.lock() {
        Ok(mut n) => match n.memory.configure(set) {
            Ok(applied) => meta_signal_energy_nexus::Response::Configured(applied),
            Err(reason) => meta_signal_energy_nexus::Response::ConfigureRefused(reason),
        },
        Err(_) => meta_signal_energy_nexus::Response::ConfigureRefused("memory unavailable".into()),
    };
    if let Ok(signal) = response.signalize() {
        let _ = stream.write_frame(signal.bytes());
    }
}
fn main() {
    let runtime = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("usage: energy-daemon RUNTIME");
    Place { runtime }.serve().expect("serve energy nexus")
}
