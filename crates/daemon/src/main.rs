use energy_nexus_core::{
    Configuring, LevelChange, Levelling, Memory, Pacing, RedeemDecision, Redeeming, Remembering,
    Reminding,
};
use energy_nexus_emission::{
    ClaudeUsage, CodexConsumeReset, CodexRateLimits, Consumed, Emission, Idempotency,
    Outcome as ExternalOutcome, Query as EmissionQuery, Response as EmissionResponse, Send,
};
use energy_nexus_wire::{Framing, Place, SocketPathing};
use signal::{ByteViewable, Restorable, Signal, Signalizable};
use signal_energy_nexus::{Call, Changed_Data, Provider, Query, Read_Data, Refused, Response};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};

struct Nexus {
    memory: Memory,
    subscribers: Vec<UnixStream>,
    place: Place,
}
enum Operation {
    Reading(Provider),
    Levelling(signal_energy_nexus::Reading),
    Reminding(signal_energy_nexus::Reminder, signal_energy_nexus::Reading),
    Redeeming(Provider),
}
enum OperationOutcome {
    Read(signal_energy_nexus::Reading),
    Levelled(LevelChange),
    Reminded,
    Redeemed(signal_energy_nexus::Reset),
    Failed(Refused),
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
impl Nexus {
    fn persist(&self) -> Result<(), String> {
        self.memory.save(&self.place.state.join("memory.rkyv"))
    }
    fn attempt(&mut self, operation: Operation) -> OperationOutcome {
        match operation {
            Operation::Reading(provider) => {
                let emission = match provider {
                    Provider::Codex => Emission::CodexRateLimits(CodexRateLimits),
                    Provider::Claude => Emission::ClaudeUsage(ClaudeUsage),
                };
                match self.emit(emission) {
                    Ok(ExternalOutcome::Observed(reading)) => OperationOutcome::Read(reading),
                    _ => OperationOutcome::Failed(Refused::ProviderDown(provider)),
                }
            }
            Operation::Levelling(reading) => {
                let change = self.memory.remember(reading);
                let _ = self.persist();
                OperationOutcome::Levelled(change)
            }
            Operation::Reminding(reminder, reading) => {
                let provider = reminder.provider.clone();
                let level = reading.level(self.memory.configuration());
                match self.emit(Emission::Send(Send {
                    recipient: reminder.recipient,
                    provider: reading.provider,
                    level,
                })) {
                    Ok(ExternalOutcome::Sent) => OperationOutcome::Reminded,
                    _ => OperationOutcome::Failed(Refused::ProviderDown(provider)),
                }
            }
            Operation::Redeeming(provider) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_secs() as i64)
                    .unwrap_or(0);
                match self.memory.redeem_decision(&provider, now) {
                    RedeemDecision::Allowed(reset) => {
                        let key = Idempotency(format!("energy:{}:{}", reset.reset_id.0, now));
                        match self.emit(Emission::CodexConsumeReset(CodexConsumeReset {
                            reset_id: reset.reset_id.clone(),
                            idempotency: key,
                        })) {
                            Ok(ExternalOutcome::Consumed(Consumed::Reset(value))) => {
                                OperationOutcome::Redeemed(value)
                            }
                            Ok(ExternalOutcome::Consumed(Consumed::AlreadyRedeemed)) => {
                                OperationOutcome::Redeemed(reset)
                            }
                            _ => OperationOutcome::Failed(Refused::ProviderDown(provider)),
                        }
                    }
                    RedeemDecision::NoReset => OperationOutcome::Failed(Refused::NoReset),
                    RedeemDecision::Refused(reason) => {
                        OperationOutcome::Failed(Refused::RuleRefused(reason))
                    }
                }
            }
        }
    }
}
trait Answering {
    fn answer(&mut self, call: Call) -> Response;
    fn read(&mut self, provider: Provider) -> Response;
    fn redeem(&mut self, provider: Provider) -> Response;
}
impl Answering for Nexus {
    fn answer(&mut self, call: Call) -> Response {
        match call.input {
            Query::Read(provider) => self.read(provider),
            Query::Subscribe => Response::Subscribed,
            Query::Redeem(provider) => self.redeem(provider),
        }
    }
    fn read(&mut self, provider: Provider) -> Response {
        match self.attempt(Operation::Reading(provider.clone())) {
            OperationOutcome::Read(reading) => {
                let OperationOutcome::Levelled(change) =
                    self.attempt(Operation::Levelling(reading.clone()))
                else {
                    return Response::Refused(Refused::ProviderDown(provider));
                };
                if change.old.as_ref() != Some(&change.latest.level) {
                    let changed = Response::Changed(Changed_Data {
                        provider: provider.clone(),
                        level: change.latest.level.clone(),
                        reading: reading.clone(),
                    });
                    push(&mut self.subscribers, &changed);
                    for reminder in self.memory.reminders_for(&provider, &change.latest.level) {
                        let _ = self.attempt(Operation::Reminding(reminder, reading.clone()));
                    }
                }
                let pace = reading.pace(self.memory.configuration());
                Response::Read(Read_Data { reading, pace })
            }
            OperationOutcome::Failed(refused) => Response::Refused(refused),
            _ => Response::Refused(Refused::ProviderDown(provider)),
        }
    }
    fn redeem(&mut self, provider: Provider) -> Response {
        match self.attempt(Operation::Redeeming(provider)) {
            OperationOutcome::Redeemed(reset) => Response::Redeemed(reset),
            OperationOutcome::Failed(refused) => Response::Refused(refused),
            _ => Response::Refused(Refused::NoReset),
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
        std::fs::create_dir_all(&self.state)?;
        for path in [self.public(), self.meta()] {
            let _ = std::fs::remove_file(path);
        }
        let public = UnixListener::bind(self.public())?;
        let meta = UnixListener::bind(self.meta())?;
        let nexus = Arc::new(Mutex::new(Nexus {
            memory: Memory::load(&self.state.join("memory.rkyv")).unwrap_or_default(),
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
    let Ok(call) = Signal::<Call>::from(bytes).restore() else {
        return;
    };
    let subscribe = matches!(call.input, Query::Subscribe);
    let response = nexus.lock().ok().map(|mut n| n.answer(call));
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
    let Ok(call) = Signal::<meta_signal_energy_nexus::Call>::from(bytes).restore() else {
        return;
    };
    let meta_signal_energy_nexus::Query::Configure(set) = call.input;
    let response = match nexus.lock() {
        Ok(mut n) => match n.memory.configure(set) {
            Ok(applied) => {
                let _ = n.persist();
                meta_signal_energy_nexus::Response::Configured(applied)
            }
            Err(reason) => meta_signal_energy_nexus::Response::ConfigureRefused(reason),
        },
        Err(_) => meta_signal_energy_nexus::Response::ConfigureRefused("memory unavailable".into()),
    };
    if let Ok(signal) = response.signalize() {
        let _ = stream.write_frame(signal.bytes());
    }
}
fn main() {
    let mut arguments = std::env::args().skip(1);
    let name = arguments
        .next()
        .expect("usage: energy-daemon NAME RUNTIME STATE");
    let runtime = arguments
        .next()
        .map(std::path::PathBuf::from)
        .expect("runtime");
    let state = arguments
        .next()
        .map(std::path::PathBuf::from)
        .expect("state");
    Place {
        name: energy_nexus_wire::PlaceName(name),
        runtime,
        state,
    }
    .serve()
    .expect("serve energy nexus")
}
