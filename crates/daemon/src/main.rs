use energy_nexus_core::{
    Configuring, LevelChange, Levelling, Memory, MemoryRefusal, Pacing, RedeemDecision, Redeeming,
    Remembering, Reminding,
};
use energy_nexus_emission::{
    ClaudeUsage, CodexConsumeReset, CodexRateLimits, Consumed, Emission, Idempotency,
    Outcome as ExternalOutcome, Query as EmissionQuery, Response as EmissionResponse, Send,
};
use energy_nexus_wire::{
    AnnouncesReadiness, Declared, Framing, MemoryPathing, OwnedSockets, OwningSockets, Place,
    Readiness, SocketPathing,
};
use signal::{ByteViewable, Restorable, Signal, Signalizable};
use signal_energy_nexus::{Call, Changed_Data, Provider, Query, Read_Data, Refused, Response};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};

struct Nexus {
    memory: Memory,
    subscribers: Vec<UnixStream>,
    place: Place,
    halted: Option<String>,
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
    /// Saves the candidate memory and only then makes it the memory in use, so the memory in RAM
    /// never changes ahead of the file.
    fn commit(&mut self, candidate: Memory) -> Result<(), MemoryRefusal> {
        candidate.save(&self.place.memory())?;
        self.memory = candidate;
        Ok(())
    }
    /// After a failed save the Nexus stops every mutation, provider and reminder entry; the
    /// memory in RAM is unchanged and nothing is rolled back.
    fn halt(&mut self, refusal: &MemoryRefusal) -> String {
        let reason = format!("memory not persisted: {refusal}");
        eprintln!("energy-nexus: halted, {reason}");
        self.halted = Some(reason.clone());
        reason
    }
    fn attempt(&mut self, operation: Operation) -> OperationOutcome {
        if let Some(reason) = &self.halted {
            return OperationOutcome::Failed(Refused::RuleRefused(format!("halted: {reason}")));
        }
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
                let mut candidate = self.memory.clone();
                let change = candidate.remember(reading);
                match self.commit(candidate) {
                    Ok(()) => OperationOutcome::Levelled(change),
                    Err(refusal) => {
                        let reason = self.halt(&refusal);
                        OperationOutcome::Failed(Refused::RuleRefused(reason))
                    }
                }
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
        if let Some(reason) = &self.halted {
            // Halted: no provider call and no mutation. The committed reading, its level and its
            // pace, and the halt cause, are carried as text in the typed refusal, the wire having
            // no status field and Read_Data no room for a cause.
            let snapshot = match self.memory.latest(&provider) {
                Some(latest) => format!(
                    "level {:?}, reading {:?}, pace {:?}",
                    latest.level,
                    latest.reading,
                    latest.reading.pace(self.memory.configuration())
                ),
                None => "none".to_owned(),
            };
            return Response::Refused(Refused::RuleRefused(format!(
                "halted: {reason}; committed snapshot: {snapshot}"
            )));
        }
        match self.attempt(Operation::Reading(provider.clone())) {
            OperationOutcome::Read(reading) => {
                let levelled = self.attempt(Operation::Levelling(reading.clone()));
                let OperationOutcome::Levelled(change) = levelled else {
                    return match levelled {
                        OperationOutcome::Failed(refused) => Response::Refused(refused),
                        _ => Response::Refused(Refused::ProviderDown(provider)),
                    };
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
                let pace_vector = reading.pace(self.memory.configuration());
                Response::Read(Read_Data {
                    reading,
                    pace_vector,
                })
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
        self.confirm()?;
        // Admission next: a refused memory means no socket or thread is created.
        let memory = Memory::load(&self.memory()).map_err(|refusal| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, refusal.to_string())
        })?;
        let own = OwnedSockets(vec![self.public(), self.meta()]);
        own.remove_at_stop()?;
        own.remove();
        let public = UnixListener::bind(self.public())?;
        let meta = UnixListener::bind(self.meta())?;
        let nexus = Arc::new(Mutex::new(Nexus {
            memory,
            subscribers: Vec::new(),
            place: self.clone(),
            halted: None,
        }));
        // Both sockets listen and Memory is loaded: a unit ordered after
        // this one (the translator) finds the daemon serving.
        if let Err(error) = Readiness::announce_ready() {
            eprintln!("readiness not announced: {error}");
        }
        {
            let nexus = Arc::clone(&nexus);
            std::thread::spawn(move || loop {
                if nexus
                    .lock()
                    .map(|guard| guard.halted.is_some())
                    .unwrap_or(true)
                {
                    break;
                }
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
        Ok(mut n) => {
            if let Some(reason) = n.halted.clone() {
                meta_signal_energy_nexus::Response::ConfigureRefused(format!("halted: {reason}"))
            } else {
                let mut candidate = n.memory.clone();
                match candidate.configure(set) {
                    Ok(applied) => match n.commit(candidate) {
                        Ok(()) => meta_signal_energy_nexus::Response::Configured(applied),
                        Err(refusal) => {
                            meta_signal_energy_nexus::Response::ConfigureRefused(n.halt(&refusal))
                        }
                    },
                    Err(reason) => meta_signal_energy_nexus::Response::ConfigureRefused(reason),
                }
            }
        }
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
#[cfg(test)]
mod tests {
    use super::*;
    use energy_nexus_core::Levelling;
    use signal_energy_nexus::{Instant, Minutes, Name, Percent, Reading, Window};

    fn nexus(root: &std::path::Path) -> Nexus {
        std::fs::create_dir_all(root).unwrap();
        Nexus {
            memory: Memory::default(),
            subscribers: Vec::new(),
            place: Place {
                name: energy_nexus_wire::PlaceName("test".into()),
                runtime: root.join("run"),
                state: root.to_owned(),
            },
            halted: None,
        }
    }

    fn reading() -> Reading {
        Reading {
            provider: Provider::Codex,
            allowed: true,
            window_vector: vec![Window {
                name: Name::Week,
                used: Percent(10),
                resets: Instant(1_000),
                duration: Minutes(10_080),
            }],
            reset_vector: Vec::new(),
            at: Instant(1),
        }
    }

    #[test]
    fn a_failed_save_leaves_the_memory_halts_the_nexus_and_refuses_further_work() {
        let root = std::env::temp_dir().join(format!("energy-halt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut nexus = nexus(&root);
        // a committed snapshot first
        assert!(matches!(
            nexus.attempt(Operation::Levelling(reading())),
            OperationOutcome::Levelled(_)
        ));
        let committed = std::fs::read(root.join("energy.rkyv")).unwrap();
        let before = nexus.memory.clone();
        // the temporary name is taken by a directory: the next save fails
        std::fs::create_dir(root.join("energy.new")).unwrap();
        let mut worse = reading();
        worse.window_vector[0].used = Percent(95);
        let outcome = nexus.attempt(Operation::Levelling(worse));
        assert!(
            matches!(&outcome, OperationOutcome::Failed(Refused::RuleRefused(reason)) if reason.contains("memory not persisted")),
        );
        assert!(nexus.halted.is_some());
        assert_eq!(nexus.memory.configuration(), before.configuration());
        assert_eq!(
            nexus
                .memory
                .latest(&Provider::Codex)
                .unwrap()
                .reading
                .window_vector[0]
                .used,
            Percent(10),
            "the committed snapshot is kept, the failed one never swapped in"
        );
        assert_eq!(
            std::fs::read(root.join("energy.rkyv")).unwrap(),
            committed,
            "the committed file is untouched"
        );
        for _ in 0..2 {
            let again = nexus.attempt(Operation::Levelling(reading()));
            assert!(
                matches!(&again, OperationOutcome::Failed(Refused::RuleRefused(reason)) if reason.starts_with("halted:")),
                "repeated refusals"
            );
            let read = nexus.attempt(Operation::Reading(Provider::Codex));
            assert!(
                matches!(&read, OperationOutcome::Failed(Refused::RuleRefused(reason)) if reason.starts_with("halted:")),
                "no provider call is attempted once halted"
            );
            let reminding = nexus.attempt(Operation::Reminding(
                signal_energy_nexus::Reminder {
                    provider: Provider::Codex,
                    level: signal_energy_nexus::Level::Low,
                    recipient: signal_energy_nexus::Recipient(signal_energy_nexus::Address {
                        aspect: signal_flow::Aspect::Mind,
                        topic: "energyNexus".into(),
                        model: signal_flow::Model::Sol,
                    }),
                },
                reading(),
            ));
            assert!(matches!(
                reminding,
                OperationOutcome::Failed(Refused::RuleRefused(_))
            ));
            let redeeming = nexus.attempt(Operation::Redeeming(Provider::Codex));
            assert!(matches!(
                redeeming,
                OperationOutcome::Failed(Refused::RuleRefused(_))
            ));
        }
        match nexus.read(Provider::Codex) {
            Response::Refused(Refused::RuleRefused(text)) => {
                assert!(text.starts_with("halted: memory not persisted"), "{text}");
                assert!(text.contains("committed snapshot: level"), "{text}");
                assert!(
                    text.contains("window_vector") && text.contains("pace"),
                    "{text}"
                );
            }
            other => panic!("a halted read reports the cause and the snapshot, got {other:?}"),
        }
        // restart: the full admission of the untouched file reads the committed snapshot
        let restarted = Memory::load(&root.join("energy.rkyv")).unwrap();
        assert_eq!(
            restarted
                .latest(&Provider::Codex)
                .unwrap()
                .reading
                .window_vector[0]
                .used,
            Percent(10)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_successful_save_replaces_the_memory_in_use() {
        let root = std::env::temp_dir().join(format!("energy-commit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut nexus = nexus(&root);
        let outcome = nexus.attempt(Operation::Levelling(reading()));
        assert!(matches!(outcome, OperationOutcome::Levelled(_)));
        assert!(nexus.halted.is_none());
        let reloaded = Memory::load(&root.join("energy.rkyv")).unwrap();
        assert_eq!(reloaded.configuration(), nexus.memory.configuration());
        let _ = std::fs::remove_dir_all(&root);
    }
}
