mod schedule;

use energy_nexus_core::operation::{Operation, Outcome as OperationOutcome, Reminding_Data};
use energy_nexus_core::{
    Configuring, Levelling, Memory, OwnerZone, Pacing, RedeemDecision, Redeeming, Remembering,
    Reminding,
};
use energy_nexus_emission::{
    ClaudeUsage, CodexConsumeReset, CodexRateLimits, Consumption, Delivery, Emission, Failure,
    Idempotency, Input as EmissionInput, Observation, Outcome as ExternalOutcome,
    Output as EmissionOutput, Send,
};
use schedule::{Clock, Event, Reads, Schedule, WallClock};
use energy_nexus_wire::{
    AnnouncesReadiness, Declared, Framing, MemoryPathing, MemoryRefusal, OwnedSockets,
    OwningSockets, Persisted, Place, PlacedSocket, PlacesSocket, Readiness, SocketPathing,
    Upgrade,
};
use signal::{ByteViewable, Restorable, Signal, Signalizable};
use signal_energy_nexus::{
    Answerable, Call, Changed_Data, Instant, Output, Provider, Read_Data, Refused, Reset,
};
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

struct Nexus {
    memory: Memory,
    subscribers: Vec<UnixStream>,
    place: Place,
    /// Each provider that said to wait, and the instant it may be read
    /// again: its failed reading's `retry-after` from when it came.
    withheld: HashMap<Provider, Instant>,
    /// Unix seconds now: the wall clock, and in a test one it sets.
    now: fn() -> i64,
}
trait Emitting {
    fn emit(&self, emission: Emission) -> Result<ExternalOutcome, String>;
}
impl Emitting for Nexus {
    fn emit(&self, emission: Emission) -> Result<ExternalOutcome, String> {
        let mut stream =
            UnixStream::connect(self.place.emission()).map_err(|error| error.to_string())?;
        let signal = EmissionInput::Emit(emission)
            .signalize()
            .map_err(|error| format!("{error:?}"))?;
        stream
            .write_frame(signal.bytes())
            .map_err(|error| error.to_string())?;
        let bytes = stream.read_frame().map_err(|error| error.to_string())?;
        match Signal::<EmissionOutput>::from(bytes)
            .restore()
            .map_err(|error| format!("{error:?}"))?
        {
            EmissionOutput::Emitted(outcome) => Ok(outcome),
            other => Err(format!("the translator answered an emission with {other:?}")),
        }
    }
}
fn wall_now() -> i64 {
    WallClock::default().unix_now()
}
/// What the Nexus does with a provider that said to wait.
trait Withholding {
    /// Notes the instant the provider may be read again, when its failed
    /// reading carried a `retry-after`.
    fn withhold(&mut self, provider: &Provider, failure: &Failure);
    /// The provider is not read before it said to: the instant it may be, when
    /// that is after `now`.
    fn withheld_until(&self, provider: &Provider, now: i64) -> Option<i64>;
}
impl Withholding for Nexus {
    fn withhold(&mut self, provider: &Provider, failure: &Failure) {
        let now = (self.now)();
        match failure.retry_after_seconds() {
            Some(seconds) => {
                self.withheld
                    .insert(provider.clone(), Instant(now.saturating_add(seconds)));
            }
            None => {
                self.withheld.remove(provider);
            }
        }
    }
    fn withheld_until(&self, provider: &Provider, now: i64) -> Option<i64> {
        self.withheld
            .get(provider)
            .map(|instant| instant.0)
            .filter(|until| *until > now)
    }
}
impl Nexus {
    /// Saves the candidate memory and only then makes it the memory in use:
    /// the memory in use never runs ahead of the file, and a failed save
    /// leaves both as they were.
    fn commit(&mut self, candidate: Memory) -> Result<(), MemoryRefusal> {
        candidate.save(&self.place.memory())?;
        self.memory = candidate;
        Ok(())
    }
    fn attempt(&mut self, operation: Operation) -> OperationOutcome {
        match operation {
            Operation::Reading(provider) => {
                let emission = match provider {
                    Provider::Codex => Emission::CodexRateLimits(CodexRateLimits {}),
                    Provider::Claude => Emission::ClaudeUsage(ClaudeUsage {}),
                };
                match self.emit(emission) {
                    Ok(ExternalOutcome::CodexRateLimitsObserved(Observation::Observed(reading)))
                    | Ok(ExternalOutcome::ClaudeUsageObserved(Observation::Observed(reading))) => {
                        self.withheld.remove(&provider);
                        OperationOutcome::Read(reading)
                    }
                    Ok(ExternalOutcome::CodexRateLimitsObserved(Observation::Down(failure)))
                    | Ok(ExternalOutcome::ClaudeUsageObserved(Observation::Down(failure))) => {
                        self.withhold(&provider, &failure);
                        OperationOutcome::Failed(Refused::ProviderDown(provider))
                    }
                    _ => OperationOutcome::Failed(Refused::ProviderDown(provider)),
                }
            }
            Operation::Levelling(reading) => {
                let mut candidate = self.memory.clone();
                let change = candidate.remember(reading);
                match self.commit(candidate) {
                    Ok(()) => OperationOutcome::Levelled(change),
                    Err(refusal) => OperationOutcome::Failed(Refused::RuleRefused(format!(
                        "memory not persisted: {refusal}"
                    ))),
                }
            }
            Operation::Reminding(Reminding_Data { reminder, reading }) => {
                let provider = reminder.provider.clone();
                let level = reading.level(self.memory.configuration());
                match self.emit(Emission::Send(Send {
                    recipient: reminder.recipient,
                    provider: reading.provider,
                    level,
                })) {
                    Ok(ExternalOutcome::ReminderSent(Delivery::Sent)) => OperationOutcome::Reminded,
                    _ => OperationOutcome::Failed(Refused::ProviderDown(provider)),
                }
            }
            Operation::Redeeming(provider) => {
                let now = WallClock::default().unix_now();
                match self.memory.redeem_decision(&provider, now) {
                    RedeemDecision::Allowed(reset) => {
                        let key = Idempotency(format!("energy:{}:{}", reset.reset_id.0, now));
                        match self.emit(Emission::CodexConsumeReset(CodexConsumeReset {
                            reset_id: reset.reset_id.clone(),
                            idempotency: key,
                        })) {
                            Ok(ExternalOutcome::CodexResetConsumed(Consumption::Reset(value))) => {
                                OperationOutcome::Redeemed(value)
                            }
                            Ok(ExternalOutcome::CodexResetConsumed(
                                Consumption::AlreadyRedeemed,
                            )) => OperationOutcome::Redeemed(reset),
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
/// What the Nexus does to answer: the reads the schedule asks for and a
/// redeem.
trait Acting {
    /// Reads the provider now: levels the reading, pushes a level change and
    /// sends its reminders. Only the schedule asks for it.
    fn read(&mut self, provider: Provider) -> Output;
    /// The latest reading in memory, with its pace.
    fn recall(&self, provider: Provider) -> Output;
    /// The reading held, for the journal: its age at `now`, or none held.
    fn held(&self, provider: &Provider, now: i64) -> String;
    fn redeem(&mut self, provider: Provider) -> Output;
}
impl Acting for Nexus {
    fn held(&self, provider: &Provider, now: i64) -> String {
        match self.memory.latest(provider) {
            Some(latest) => format!("the reading held, {} seconds old", now - latest.reading.at.0.0),
            None => "no reading held".to_string(),
        }
    }
    fn recall(&self, provider: Provider) -> Output {
        match self.memory.latest(&provider) {
            Some(latest) => Output::Read(Read_Data {
                pace_vector: latest.reading.pace(
                    self.memory.configuration(),
                    &self.memory.configuration().owner_zone(),
                ),
                reading: latest.reading.clone(),
            }),
            None => Output::Refused(Refused::ProviderDown(provider)),
        }
    }
    fn read(&mut self, provider: Provider) -> Output {
        let now = (self.now)();
        if let Some(until) = self.withheld_until(&provider, now) {
            eprintln!(
                "energy-nexus: {provider:?} not read until {until}, as it said; answering with {}",
                self.held(&provider, now)
            );
            return self.recall(provider);
        }
        match self.attempt(Operation::Reading(provider.clone())) {
            OperationOutcome::Read(reading) => {
                let levelled = self.attempt(Operation::Levelling(reading.clone()));
                let OperationOutcome::Levelled(change) = levelled else {
                    return match levelled {
                        OperationOutcome::Failed(refused) => Output::Refused(refused),
                        _ => Output::Refused(Refused::ProviderDown(provider)),
                    };
                };
                if change.old.0.as_ref() != Some(&change.latest.level) {
                    let changed = Output::Changed(Changed_Data {
                        provider: provider.clone(),
                        level: change.latest.level.clone(),
                        reading: reading.clone(),
                    });
                    push(&mut self.subscribers, &changed);
                    for reminder in self.memory.reminders_for(&provider, &change.latest.level) {
                        let _ = self.attempt(Operation::Reminding(Reminding_Data {
                            reminder,
                            reading: reading.clone(),
                        }));
                    }
                }
                let configuration = self.memory.configuration();
                let pace_vector = reading.pace(configuration, &configuration.owner_zone());
                Output::Read(Read_Data {
                    reading,
                    pace_vector,
                })
            }
            // The fresh reading failed: the reading held answers, its own
            // instant its age; the provider is down only when none is held.
            OperationOutcome::Failed(Refused::ProviderDown(down)) => {
                eprintln!(
                    "energy-nexus: {down:?} reading failed; answering with {}",
                    self.held(&down, now)
                );
                self.recall(down)
            }
            OperationOutcome::Failed(refused) => Output::Refused(refused),
            _ => Output::Refused(Refused::ProviderDown(provider)),
        }
    }
    fn redeem(&mut self, provider: Provider) -> Output {
        match self.attempt(Operation::Redeeming(provider)) {
            OperationOutcome::Redeemed(reset) => Output::Redeemed(reset),
            OperationOutcome::Failed(refused) => Output::Refused(refused),
            _ => Output::Refused(Refused::NoReset),
        }
    }
}
/// The schedule's reads and answers, made on the Nexus off the schedule's
/// thread: a provider call blocks.
#[derive(Clone)]
struct Daemon(Arc<Mutex<Nexus>>);
impl Reads for Daemon {
    async fn read(&self, provider: Provider) -> Output {
        let nexus = Arc::clone(&self.0);
        let down = Output::Refused(Refused::ProviderDown(provider.clone()));
        tokio::task::spawn_blocking(move || match nexus.lock() {
            Ok(mut nexus) => nexus.read(provider),
            Err(_) => Output::Refused(Refused::ProviderDown(provider)),
        })
        .await
        .unwrap_or(down)
    }
    async fn recall(&self, provider: Provider) -> Output {
        let nexus = Arc::clone(&self.0);
        let down = Output::Refused(Refused::ProviderDown(provider.clone()));
        tokio::task::spawn_blocking(move || match nexus.lock() {
            Ok(nexus) => nexus.recall(provider),
            Err(_) => Output::Refused(Refused::ProviderDown(provider)),
        })
        .await
        .unwrap_or(down)
    }
}
/// Holds Flow's turn ends open through the translator, `TurnEnds`, and
/// hands each to the schedule. Flow going does not end it: the translator
/// waits for Flow's socket and subscribes again, and the new opening comes
/// on this same connection, taken as the first. It ends when the translator
/// answers `Down` (its watch on the place is lost) or goes, and the daemon's
/// unit is bound to the translator's.
fn subscribe(place: &Place, events: &tokio::sync::mpsc::UnboundedSender<Event>) {
    let opened = UnixStream::connect(place.emission())
        .map_err(|error| error.to_string())
        .and_then(|mut stream| {
            let signal = EmissionInput::TurnEnds
                .signalize()
                .map_err(|error| format!("{error:?}"))?;
            stream
                .write_frame(signal.bytes())
                .map_err(|error| error.to_string())?;
            Ok(stream)
        });
    let mut stream = match opened {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("energy-nexus: turn ends not subscribed, the translator is unreachable: {error}");
            return;
        }
    };
    loop {
        let event = stream
            .read_frame()
            .ok()
            .and_then(|bytes| Signal::<EmissionOutput>::from(bytes).restore().ok());
        let event = match event {
            Some(EmissionOutput::TurnEndsObserved(ends)) => Event::TurnEndsObserved(ends),
            Some(EmissionOutput::TurnEnded(end)) => Event::TurnEnded(end),
            _ => {
                eprintln!(
                    "energy-nexus: the translator closed Flow's turn ends; window resets and \
                     questions still read"
                );
                return;
            }
        };
        if events.send(event).is_err() {
            return;
        }
    }
}
fn push(subscribers: &mut Vec<UnixStream>, response: &Output) {
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
        // Memory next: a refused memory means no socket or thread is made.
        let path = self.memory();
        let admission = Memory::load(&path).map_err(|refusal| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, refusal.to_string())
        })?;
        if let energy_nexus_wire::Admission::Upgraded(earlier, _) = &admission {
            eprintln!("energy-nexus: {} upgraded from {earlier}", path.display());
        }
        let Upgrade { memory, uncarried } = Upgrade::from(admission);
        for reminder in &uncarried {
            let address = &reminder.recipient.0;
            eprintln!(
                "energy-nexus: reminder not carried, its recipient names a layer and Flow \
                 addresses a model: {reminder}; configure it again with the model that serves \
                 it: energy-nexus-meta 'Configure.Reminder.{{ {:?} {:?} {{ {:?} {} <Model> }} }}'",
                reminder.provider, reminder.level, address.aspect, address.topic
            );
        }
        let own = OwnedSockets(vec![self.public(), self.meta()]);
        own.remove_at_stop()?;
        // Each socket appears at its name only once it listens, replacing a
        // stale one a crashed daemon left there.
        let public = PlacedSocket::at(&self.public())?.claim()?;
        let meta = PlacedSocket::at(&self.meta())?.claim()?;
        let latest = [Provider::Claude, Provider::Codex]
            .iter()
            .filter_map(|provider| memory.latest(provider))
            .map(|latest| latest.reading.clone())
            .collect();
        let nexus = Arc::new(Mutex::new(Nexus {
            memory,
            subscribers: Vec::new(),
            place: self.clone(),
            withheld: HashMap::new(),
            now: wall_now,
        }));
        // The schedule: every read Energy makes is its, on an event.
        let schedule = Schedule::new(Daemon(Arc::clone(&nexus)), WallClock::default(), latest);
        let events = schedule.sender();
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        std::thread::spawn(move || runtime.block_on(schedule.run()));
        // Both sockets listen and Memory is loaded.
        if let Err(error) = Readiness::announce_ready() {
            eprintln!("readiness not announced: {error}");
        }
        // The translator's unit is ordered before this one: its readiness
        // came first, so the subscription finds it, and its opening, the
        // last turn end on each provider, is the first reading's event.
        {
            let place = self.clone();
            let events = events.clone();
            std::thread::spawn(move || subscribe(&place, &events));
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
            let events = events.clone();
            std::thread::spawn(move || serve_public(stream, nexus, events));
        }
        Ok(())
    }
}
/// One public call, answered through the contract's generated dispatch: a
/// question is the schedule's, a redeem the Nexus's.
struct Public<'a> {
    nexus: &'a Arc<Mutex<Nexus>>,
    events: &'a tokio::sync::mpsc::UnboundedSender<Event>,
}
impl signal_energy_nexus::Answering for Public<'_> {
    /// The question: answered by the schedule, which reads first when the
    /// latest reading is older than the last turn end on the provider.
    fn read(&self, provider: &Provider) -> Result<Read_Data, Output> {
        match schedule::ask(self.events, provider.clone()) {
            Output::Read(read) => Ok(read),
            other => Err(other),
        }
    }
    /// The subscriber is kept once its answer is written.
    fn subscribe(&self) -> Result<(), Output> {
        Ok(())
    }
    fn redeem(&self, provider: &Provider) -> Result<Reset, Output> {
        let mut nexus = self
            .nexus
            .lock()
            .map_err(|_| Output::Refused(Refused::ProviderDown(provider.clone())))?;
        match nexus.redeem(provider.clone()) {
            Output::Redeemed(reset) => Ok(reset),
            other => Err(other),
        }
    }
}
fn serve_public(
    mut stream: UnixStream,
    nexus: Arc<Mutex<Nexus>>,
    events: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let Ok(bytes) = stream.read_frame() else {
        return;
    };
    let Ok(call) = Signal::<Call>::from(bytes).restore() else {
        return;
    };
    let response = call.input.answer(&Public {
        nexus: &nexus,
        events: &events,
    });
    let subscribed = matches!(response, Output::Subscribed);
    if let Ok(signal) = response.signalize() {
        if stream.write_frame(signal.bytes()).is_ok() && subscribed {
            if let Ok(mut n) = nexus.lock() {
                n.subscribers.push(stream)
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
    let meta_signal_energy_nexus::Input::Configure(set) = call.input;
    let response = match nexus.lock() {
        Ok(mut n) => {
            let mut candidate = n.memory.clone();
            match candidate.configure(set, Instant(WallClock::default().unix_now())) {
                Ok(applied) => match n.commit(candidate) {
                    Ok(()) => meta_signal_energy_nexus::Output::Configured(applied),
                    Err(refusal) => meta_signal_energy_nexus::Output::ConfigureRefused(format!(
                        "memory not persisted: {refusal}"
                    )),
                },
                Err(reason) => meta_signal_energy_nexus::Output::ConfigureRefused(reason),
            }
        }
        Err(_) => meta_signal_energy_nexus::Output::ConfigureRefused("memory unavailable".into()),
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
    use energy_nexus_wire::Admission;
    use signal_energy_nexus::{Instant, Minutes, Name, Percent, Reading, Window};

    fn nexus(root: &std::path::Path) -> Nexus {
        std::fs::create_dir_all(root).unwrap();
        Nexus {
            memory: Memory::default(),
            subscribers: Vec::new(),
            withheld: HashMap::new(),
            now: wall_now,
            place: Place {
                name: energy_nexus_wire::PlaceName("test".into()),
                runtime: root.join("run"),
                state: root.to_owned(),
            },
        }
    }

    fn reading(used: i64) -> Reading {
        Reading {
            provider: Provider::Codex,
            allowed: signal_energy_nexus::Allowed(true),
            window_vector: vec![Window {
                name: Name::Week,
                used: signal_energy_nexus::Used(Percent(used)),
                resets: signal_energy_nexus::Resets(Instant(1_000)),
                duration: signal_energy_nexus::Duration(Minutes(10_080)),
            }],
            reset_vector: Vec::new(),
            at: signal_energy_nexus::At(Instant(1)),
        }
    }

    fn used(memory: &Memory) -> Percent {
        memory.latest(&Provider::Codex).unwrap().reading.window_vector[0]
            .used
            .0
            .clone()
    }

    fn on_disk(root: &std::path::Path) -> Memory {
        match Memory::load(&root.join("energy.rkyv")).unwrap() {
            Admission::Admitted(memory) => memory,
            other => panic!("the committed file is current, got {other:?}"),
        }
    }

    thread_local! {
        static NOW: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    }
    fn fixed_now() -> i64 {
        NOW.with(|now| now.get())
    }
    fn at(now: i64) {
        NOW.with(|cell| cell.set(now));
    }

    /// The translator as the daemon meets it on the emission socket: it counts
    /// each call and answers every one with what the test last set.
    struct Translating {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        answer: Arc<Mutex<ExternalOutcome>>,
    }
    impl Translating {
        fn at(nexus: &Nexus) -> Self {
            std::fs::create_dir_all(&nexus.place.runtime).unwrap();
            let listener = std::os::unix::net::UnixListener::bind(nexus.place.emission()).unwrap();
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let answer = Arc::new(Mutex::new(ExternalOutcome::CodexRateLimitsObserved(
                Observation::Down(Failure::of(energy_nexus_emission::Cause::Unreached)),
            )));
            let (counted, answering) = (Arc::clone(&calls), Arc::clone(&answer));
            std::thread::spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    if stream.read_frame().is_err() {
                        continue;
                    }
                    counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let outcome = answering.lock().unwrap().clone();
                    let signal = EmissionOutput::Emitted(outcome).signalize().unwrap();
                    let _ = stream.write_frame(signal.bytes());
                }
            });
            Self { calls, answer }
        }
        fn answers(&self, outcome: ExternalOutcome) {
            *self.answer.lock().unwrap() = outcome;
        }
        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    fn down(provider: &Provider, status: i64, retry_after: Option<i64>) -> ExternalOutcome {
        let failure = Failure {
            cause: energy_nexus_emission::Cause::Refused,
            status: energy_nexus_emission::Status(Some(energy_nexus_emission::HttpStatus(status))),
            retry_after: energy_nexus_emission::RetryAfter(
                retry_after.map(energy_nexus_emission::Seconds),
            ),
        };
        match provider {
            Provider::Codex => ExternalOutcome::CodexRateLimitsObserved(Observation::Down(failure)),
            Provider::Claude => ExternalOutcome::ClaudeUsageObserved(Observation::Down(failure)),
        }
    }

    fn held_nexus(name: &str, provider: Provider, used: i64, taken: i64) -> (Nexus, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("energy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut nexus = nexus(&root);
        nexus.now = fixed_now;
        let mut held = reading(used);
        held.provider = provider;
        held.at = signal_energy_nexus::At(Instant(taken));
        assert!(matches!(
            nexus.attempt(Operation::Levelling(held)),
            OperationOutcome::Levelled(_)
        ));
        (nexus, root)
    }

    #[test]
    fn a_failed_reading_is_answered_with_the_reading_held_and_not_with_down() {
        at(5_000);
        let (mut nexus, root) = held_nexus("answers-held", Provider::Claude, 10, 4_700);
        let translator = Translating::at(&nexus);
        translator.answers(down(&Provider::Claude, 429, None));
        let answer = nexus.read(Provider::Claude);
        let Output::Read(read) = answer else {
            panic!("the held reading answers, got {answer:?}");
        };
        assert_eq!(translator.calls(), 1, "the fresh reading was tried");
        assert_eq!(read.reading.provider, Provider::Claude);
        assert_eq!(read.reading.at.0, Instant(4_700), "its own instant is its age");
        assert_eq!(used_by(&nexus.memory, &Provider::Claude), Percent(10), "nothing was levelled");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_reading_with_none_held_is_the_provider_down() {
        at(5_000);
        let (mut nexus, root) = held_nexus("answers-down", Provider::Codex, 10, 4_700);
        let translator = Translating::at(&nexus);
        translator.answers(down(&Provider::Claude, 500, None));
        assert_eq!(
            nexus.read(Provider::Claude),
            Output::Refused(Refused::ProviderDown(Provider::Claude)),
            "Codex is held, Claude is not"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_provider_that_said_to_wait_is_not_read_before_it_said_and_is_after() {
        at(1_000);
        let (mut nexus, root) = held_nexus("retry-after", Provider::Claude, 10, 900);
        let translator = Translating::at(&nexus);
        translator.answers(down(&Provider::Claude, 429, Some(60)));
        assert!(matches!(nexus.read(Provider::Claude), Output::Read(_)));
        assert_eq!(translator.calls(), 1);
        // not read again while the 60 seconds run, and answered from memory
        for now in [1_001, 1_030, 1_059] {
            at(now);
            let answer = nexus.read(Provider::Claude);
            assert!(
                matches!(&answer, Output::Read(read) if read.reading.at.0 == Instant(900)),
                "{answer:?}"
            );
            assert_eq!(translator.calls(), 1, "not read at {now}");
        }
        // once it has passed, read again, and a reading taken is the answer
        at(1_060);
        let mut fresh = reading(55);
        fresh.provider = Provider::Claude;
        fresh.at = signal_energy_nexus::At(Instant(1_060));
        translator.answers(ExternalOutcome::ClaudeUsageObserved(Observation::Observed(
            fresh.clone(),
        )));
        let answer = nexus.read(Provider::Claude);
        assert!(matches!(&answer, Output::Read(read) if read.reading == fresh), "{answer:?}");
        assert_eq!(translator.calls(), 2, "read once the retry-after had passed");
        assert!(nexus.withheld.is_empty(), "a reading taken ends the wait");
        at(1_061);
        let _ = nexus.read(Provider::Claude);
        assert_eq!(translator.calls(), 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failure_that_names_no_retry_after_is_read_again_at_the_next_question() {
        at(1_000);
        let (mut nexus, root) = held_nexus("no-retry-after", Provider::Claude, 10, 900);
        let translator = Translating::at(&nexus);
        translator.answers(down(&Provider::Claude, 500, None));
        let _ = nexus.read(Provider::Claude);
        let _ = nexus.read(Provider::Claude);
        assert_eq!(translator.calls(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_provider_that_said_to_wait_holds_back_that_provider_only() {
        at(1_000);
        let (mut nexus, root) = held_nexus("only-it", Provider::Claude, 10, 900);
        let translator = Translating::at(&nexus);
        translator.answers(down(&Provider::Claude, 429, Some(600)));
        let _ = nexus.read(Provider::Claude);
        let mut codex = reading(20);
        codex.at = signal_energy_nexus::At(Instant(1_000));
        translator.answers(ExternalOutcome::CodexRateLimitsObserved(Observation::Observed(codex)));
        let answer = nexus.read(Provider::Codex);
        assert!(matches!(&answer, Output::Read(read) if read.reading.provider == Provider::Codex));
        assert_eq!(translator.calls(), 2, "Codex was read while Claude waited");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn used_by(memory: &Memory, provider: &Provider) -> Percent {
        memory.latest(provider).unwrap().reading.window_vector[0].used.0.clone()
    }

    #[test]
    fn a_failed_save_leaves_the_memory_and_the_file_and_answers_the_failure() {
        let root = std::env::temp_dir().join(format!("energy-commit-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut nexus = nexus(&root);
        assert!(matches!(
            nexus.attempt(Operation::Levelling(reading(10))),
            OperationOutcome::Levelled(_)
        ));
        let committed = std::fs::read(root.join("energy.rkyv")).unwrap();
        // the temporary name is taken by a directory: the next save fails
        std::fs::create_dir(root.join("energy.new")).unwrap();
        let outcome = nexus.attempt(Operation::Levelling(reading(95)));
        assert!(
            matches!(&outcome, OperationOutcome::Failed(Refused::RuleRefused(reason)) if reason.starts_with("memory not persisted")),
        );
        assert_eq!(used(&nexus.memory), Percent(10), "the memory in use is the committed one");
        assert_eq!(std::fs::read(root.join("energy.rkyv")).unwrap(), committed);
        // the next attempt is answered on its own: once the save succeeds, it commits
        std::fs::remove_dir(root.join("energy.new")).unwrap();
        assert!(matches!(
            nexus.attempt(Operation::Levelling(reading(95))),
            OperationOutcome::Levelled(_)
        ));
        assert_eq!(used(&nexus.memory), Percent(95));
        assert_eq!(used(&on_disk(&root)), Percent(95));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_successful_save_replaces_the_memory_in_use() {
        let root = std::env::temp_dir().join(format!("energy-commit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut nexus = nexus(&root);
        let outcome = nexus.attempt(Operation::Levelling(reading(10)));
        assert!(matches!(outcome, OperationOutcome::Levelled(_)));
        assert_eq!(on_disk(&root).configuration(), nexus.memory.configuration());
        assert_eq!(used(&on_disk(&root)), Percent(10));
        let _ = std::fs::remove_dir_all(&root);
    }
}
