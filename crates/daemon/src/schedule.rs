//! When Energy reads a provider: on three events, never on an interval.
//!
//! - A turn ends on the provider (Flow's `Observe.TurnEnds`, through the
//!   translator). A turn end that arrives while a read of that provider is in
//!   flight marks it for one more read when that one ends, so a burst of turn
//!   ends is one read and at most one follow-up. When Flow comes back after
//!   going, its new opening is taken as the first one was.
//! - Its window resets. Each reading says when its provider's next window
//!   resets; the provider's alarm is set for that wall-clock instant and reads
//!   once when it fires. A newer reading replaces the alarm. The alarm is on
//!   the wall clock, absolute, so a suspend or a clock step does not make it
//!   late: it fires at the instant, or at once on waking when that has passed.
//! - A question: `Read.Provider` is answered from memory, unless the latest
//!   reading is older than the last turn end on the provider (or there is
//!   none), when it is read first.
//!
//! When nothing happens, nothing is read.
use energy_nexus_core::Resetting;
use energy_nexus_emission::TurnEnd;
use signal_energy_nexus::{Provider, Reading, Refused, Response};
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// What wakes the schedule.
#[derive(Debug)]
pub enum Event {
    /// The subscription opened, at start or when Flow came back: the last
    /// turn end on each provider.
    TurnEndsObserved(Vec<TurnEnd>),
    /// A turn ended.
    TurnEnded(TurnEnd),
    /// A question about a provider, answered on the sender.
    Question(Provider, oneshot::Sender<Response>),
    /// A read the schedule started has ended.
    Finished(Provider, Response),
    /// The provider's alarm fired: its window reset, if the alarm is still
    /// the one set (a newer reading may have moved it).
    WindowReset(Provider),
}

/// The reads the schedule asks for, and the answers from memory.
pub trait Reads: Clone + Send + Sync + 'static {
    /// One read of the provider: `Read` with the reading, or `Refused`.
    fn read(&self, provider: Provider) -> impl Future<Output = Response> + Send;
    /// The latest reading in memory, answered as a `Read`.
    fn recall(&self, provider: Provider) -> impl Future<Output = Response> + Send;
}

/// Wall time in Unix seconds, which readings and turn ends are stamped in,
/// and one alarm per provider at a wall-clock instant.
pub trait Clock: Send + 'static {
    fn unix_now(&self) -> i64;
    /// Sets the provider's alarm at `at`, Unix seconds, replacing the one
    /// set before, or clears it (`None`). A fired alarm is sent on `events`
    /// as `WindowReset`.
    fn alarm(&mut self, provider: &Provider, at: Option<i64>, events: &mpsc::UnboundedSender<Event>);
}
/// The system's wall clock. Each provider's alarm is a timerfd on
/// `CLOCK_REALTIME`, set with `TFD_TIMER_ABSTIME` at the reset's Unix instant:
/// the kernel holds the instant, not a length of time, so it fires at that
/// wall-clock moment whatever happens between (a suspend, an NTP step), or
/// at once on resume when the moment passed during it. One thread per
/// provider blocks on its timer's expiry and sends it to the schedule.
#[derive(Default)]
pub struct WallClock {
    alarms: Vec<(Provider, Arc<nix::sys::timerfd::TimerFd>)>,
}
impl WallClock {
    fn timer(
        &mut self,
        provider: &Provider,
        events: &mpsc::UnboundedSender<Event>,
    ) -> Option<Arc<nix::sys::timerfd::TimerFd>> {
        use nix::sys::timerfd::{ClockId, TimerFd, TimerFlags};
        if let Some((_, timer)) = self.alarms.iter().find(|(known, _)| known == provider) {
            return Some(Arc::clone(timer));
        }
        let timer = match TimerFd::new(ClockId::CLOCK_REALTIME, TimerFlags::TFD_CLOEXEC) {
            Ok(timer) => Arc::new(timer),
            Err(error) => {
                eprintln!("energy-nexus: no window-reset alarm for {provider:?}: {error}");
                return None;
            }
        };
        let fired = Arc::clone(&timer);
        let events = events.clone();
        let named = provider.clone();
        // Each pass blocks in read(2) until the alarm's instant.
        std::thread::spawn(move || {
            while fired.wait().is_ok() {
                if events.send(Event::WindowReset(named.clone())).is_err() {
                    return;
                }
            }
        });
        self.alarms.push((provider.clone(), Arc::clone(&timer)));
        Some(timer)
    }
}
impl Clock for WallClock {
    fn unix_now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0)
    }
    fn alarm(&mut self, provider: &Provider, at: Option<i64>, events: &mpsc::UnboundedSender<Event>) {
        use nix::sys::time::TimeSpec;
        use nix::sys::timerfd::{Expiration, TimerSetTimeFlags};
        let Some(timer) = self.timer(provider, events) else {
            return;
        };
        let set = match at {
            // An absolute zero would disarm the timer; any instant passed
            // fires at once.
            Some(at) => timer.set(
                Expiration::OneShot(TimeSpec::new(at.max(1), 0)),
                TimerSetTimeFlags::TFD_TIMER_ABSTIME,
            ),
            None => timer.unset(),
        };
        if let Err(error) = set {
            eprintln!("energy-nexus: the window-reset alarm for {provider:?} is not set: {error}");
        }
    }
}

/// One provider's place in the schedule.
#[derive(Debug, Default)]
struct Track {
    /// When the latest reading was taken.
    read_at: Option<i64>,
    /// The last turn end on the provider.
    turn_ended_at: Option<i64>,
    in_flight: bool,
    /// A read is owed once the one in flight ends.
    again: bool,
    /// Questions waiting on the read in flight.
    waiting: Vec<oneshot::Sender<Response>>,
    /// The moment the next window resets, from the latest reading: the
    /// instant the provider's alarm is set at.
    resets_at: Option<i64>,
}
impl Track {
    /// The latest reading is not known to be newer than the last turn end.
    /// Readings and turn ends are stamped in whole seconds, so a reading in
    /// the same second as a turn end may predate it, and is read again.
    fn stale(&self) -> bool {
        match (self.read_at, self.turn_ended_at) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(read), Some(ended)) => read <= ended,
        }
    }
}

pub struct Schedule<R: Reads, C: Clock> {
    reads: R,
    clock: C,
    claude: Track,
    codex: Track,
    events: mpsc::UnboundedSender<Event>,
    inbox: mpsc::UnboundedReceiver<Event>,
}

impl<R: Reads, C: Clock> Schedule<R, C> {
    /// A schedule over `reads`, knowing the latest reading of each provider;
    /// each one's alarm is set from it.
    pub fn new(reads: R, clock: C, latest: Vec<Reading>) -> Self {
        let (events, inbox) = mpsc::unbounded_channel();
        let mut schedule = Self {
            reads,
            clock,
            claude: Track::default(),
            codex: Track::default(),
            events,
            inbox,
        };
        for reading in &latest {
            schedule.reading(reading);
        }
        schedule
    }
    /// Where events are sent from: the subscription and the questions.
    pub fn sender(&self) -> mpsc::UnboundedSender<Event> {
        self.events.clone()
    }
    fn track(&mut self, provider: &Provider) -> &mut Track {
        match provider {
            Provider::Claude => &mut self.claude,
            Provider::Codex => &mut self.codex,
        }
    }
    /// Runs for as long as the daemon does, taking each event as it comes;
    /// window resets come as events from the clock's alarms.
    pub async fn run(mut self) {
        while let Some(event) = self.inbox.recv().await {
            self.on(event);
        }
    }
    /// A reading is the provider's latest: its time, and its next window
    /// reset as the provider's alarm.
    fn reading(&mut self, reading: &Reading) {
        let resets = reading.next_reset().map(|instant| instant.0);
        let track = self.track(&reading.provider);
        track.read_at = Some(reading.at.0);
        self.resets(&reading.provider, resets);
    }
    fn resets(&mut self, provider: &Provider, at: Option<i64>) {
        self.track(provider).resets_at = at;
        self.clock.alarm(provider, at, &self.events);
    }
    fn on(&mut self, event: Event) {
        match event {
            Event::TurnEndsObserved(ends) => {
                for end in ends {
                    let track = self.track(&end.provider);
                    track.turn_ended_at = track.turn_ended_at.max(Some(end.at.0));
                    if track.stale() {
                        self.want(end.provider);
                    }
                }
            }
            Event::TurnEnded(end) => {
                let track = self.track(&end.provider);
                track.turn_ended_at = track.turn_ended_at.max(Some(end.at.0));
                self.want(end.provider);
            }
            Event::Question(provider, answer) => {
                let track = self.track(&provider);
                if track.in_flight {
                    // Answered by the read in flight, or by the follow-up a
                    // later turn end owes.
                    track.waiting.push(answer);
                } else if track.stale() {
                    track.waiting.push(answer);
                    self.start(provider);
                } else {
                    let reads = self.reads.clone();
                    tokio::spawn(async move {
                        let _ = answer.send(reads.recall(provider).await);
                    });
                }
            }
            Event::Finished(provider, response) => {
                self.track(&provider).in_flight = false;
                if let Response::Read(read) = &response {
                    self.reading(&read.reading);
                }
                let track = self.track(&provider);
                if track.again {
                    track.again = false;
                    self.start(provider);
                } else {
                    for answer in track.waiting.drain(..) {
                        let _ = answer.send(response.clone());
                    }
                }
            }
            Event::WindowReset(provider) => {
                let now = self.clock.unix_now();
                if self.track(&provider).resets_at.is_some_and(|at| at <= now) {
                    self.reset(provider);
                }
            }
        }
    }
    /// The provider's window reset: its alarm is spent, and it is read.
    fn reset(&mut self, provider: Provider) {
        self.resets(&provider, None);
        self.want(provider);
    }
    /// A read is wanted: started now, or owed once the one in flight ends.
    fn want(&mut self, provider: Provider) {
        let track = self.track(&provider);
        if track.in_flight {
            track.again = true;
        } else {
            self.start(provider);
        }
    }
    fn start(&mut self, provider: Provider) {
        self.track(&provider).in_flight = true;
        let reads = self.reads.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let response = reads.read(provider.clone()).await;
            let _ = events.send(Event::Finished(provider, response));
        });
    }
}

/// A question asked from a thread outside the schedule: the answer, once
/// the schedule gives it.
pub fn ask(events: &mpsc::UnboundedSender<Event>, provider: Provider) -> Response {
    let (answer, answered) = oneshot::channel();
    if events
        .send(Event::Question(provider.clone(), answer))
        .is_err()
    {
        return Response::Refused(Refused::ProviderDown(provider));
    }
    answered
        .blocking_recv()
        .unwrap_or(Response::Refused(Refused::ProviderDown(provider)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use signal_energy_nexus::{Instant, Minutes, Name, Percent, Read_Data, Window};
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::sync::Semaphore;

    /// The test's wall clock: a fixed origin advanced by tokio's paused
    /// time, where wall time and the monotonic clock move together. Each
    /// alarm is a task sleeping on paused time until its instant.
    struct PausedClock {
        unix: i64,
        origin: tokio::time::Instant,
        alarms: Vec<(Provider, tokio::task::JoinHandle<()>)>,
    }
    impl PausedClock {
        fn at(unix: i64) -> Self {
            Self {
                unix,
                origin: tokio::time::Instant::now(),
                alarms: Vec::new(),
            }
        }
    }
    impl Clock for PausedClock {
        fn unix_now(&self) -> i64 {
            self.unix + (tokio::time::Instant::now() - self.origin).as_secs() as i64
        }
        fn alarm(&mut self, provider: &Provider, at: Option<i64>, events: &mpsc::UnboundedSender<Event>) {
            self.alarms.retain(|(known, task)| {
                let replaced = known == provider;
                if replaced {
                    task.abort();
                }
                !replaced
            });
            if let Some(at) = at {
                let deadline =
                    self.origin + Duration::from_secs((at - self.unix).max(0) as u64);
                let events = events.clone();
                let named = provider.clone();
                let task = tokio::spawn(async move {
                    tokio::time::sleep_until(deadline).await;
                    let _ = events.send(Event::WindowReset(named));
                });
                self.alarms.push((provider.clone(), task));
            }
        }
    }

    /// A wall clock that jumps while the monotonic clock stands still, as
    /// over a suspend or a clock step: each alarm is held as its wall-clock
    /// instant, as the kernel holds an absolute `CLOCK_REALTIME` timer, and
    /// fires when the wall clock reaches or passes it.
    #[derive(Clone)]
    struct SteppedClock {
        wall: Arc<Mutex<i64>>,
        alarms: Arc<Mutex<Vec<(Provider, i64, mpsc::UnboundedSender<Event>)>>>,
    }
    impl SteppedClock {
        fn at(unix: i64) -> Self {
            Self {
                wall: Arc::new(Mutex::new(unix)),
                alarms: Arc::default(),
            }
        }
        /// The wall clock is set to `unix`; every alarm passed fires.
        fn step(&self, unix: i64) {
            *self.wall.lock().unwrap() = unix;
            self.alarms.lock().unwrap().retain(|(provider, at, events)| {
                let due = *at <= unix;
                if due {
                    let _ = events.send(Event::WindowReset(provider.clone()));
                }
                !due
            });
        }
    }
    impl Clock for SteppedClock {
        fn unix_now(&self) -> i64 {
            *self.wall.lock().unwrap()
        }
        fn alarm(&mut self, provider: &Provider, at: Option<i64>, events: &mpsc::UnboundedSender<Event>) {
            let mut alarms = self.alarms.lock().unwrap();
            alarms.retain(|(known, _, _)| known != provider);
            if let Some(at) = at {
                alarms.push((provider.clone(), at, events.clone()));
            }
        }
    }

    const START: i64 = 1_762_200_000;

    fn reading(provider: Provider, at: i64, resets: i64) -> Reading {
        Reading {
            provider,
            allowed: true,
            window_vector: vec![Window {
                name: Name::Session,
                used: Percent(10),
                resets: Instant(resets),
                duration: Minutes(300),
            }],
            reset_vector: Vec::new(),
            at: Instant(at),
        }
    }

    /// A provider fake: it counts the reads it is asked for, holds each one
    /// until the test lets it finish when `held`, and answers with a reading
    /// taken at the paused clock whose window resets `next_reset` seconds
    /// later (none when zero), or refuses when `down`.
    #[derive(Clone)]
    struct Provided {
        start: tokio::time::Instant,
        reads: Arc<Mutex<Vec<(Provider, i64)>>>,
        latest: Arc<Mutex<Vec<Reading>>>,
        gate: Option<Arc<Semaphore>>,
        next_reset: i64,
        down: Arc<Mutex<bool>>,
    }
    impl Provided {
        fn new(next_reset: i64) -> Self {
            Self {
                start: tokio::time::Instant::now(),
                reads: Arc::default(),
                latest: Arc::default(),
                gate: None,
                next_reset,
                down: Arc::default(),
            }
        }
        fn held(mut self) -> Self {
            self.gate = Some(Arc::new(Semaphore::new(0)));
            self
        }
        fn release(&self) {
            self.gate.as_ref().unwrap().add_permits(1);
        }
        fn now(&self) -> i64 {
            START + (tokio::time::Instant::now() - self.start).as_secs() as i64
        }
        fn reads(&self) -> Vec<(Provider, i64)> {
            self.reads.lock().unwrap().clone()
        }
        fn count(&self, provider: &Provider) -> usize {
            self.reads().iter().filter(|(read, _)| read == provider).count()
        }
        fn remember(&self, reading: Reading) {
            let mut latest = self.latest.lock().unwrap();
            latest.retain(|known| known.provider != reading.provider);
            latest.push(reading);
        }
    }
    impl Reads for Provided {
        async fn read(&self, provider: Provider) -> Response {
            self.reads.lock().unwrap().push((provider.clone(), self.now()));
            if let Some(gate) = &self.gate {
                gate.acquire().await.unwrap().forget();
            }
            if *self.down.lock().unwrap() {
                return Response::Refused(Refused::ProviderDown(provider));
            }
            let at = self.now();
            let resets = if self.next_reset > 0 { at + self.next_reset } else { 0 };
            let reading = reading(provider, at, resets);
            self.remember(reading.clone());
            Response::Read(Read_Data {
                reading,
                pace_vector: Vec::new(),
            })
        }
        async fn recall(&self, provider: Provider) -> Response {
            let latest = self.latest.lock().unwrap();
            match latest.iter().find(|known| known.provider == provider) {
                Some(reading) => Response::Read(Read_Data {
                    reading: reading.clone(),
                    pace_vector: Vec::new(),
                }),
                None => Response::Refused(Refused::ProviderDown(provider)),
            }
        }
    }

    /// Lets every task run until all wait: with time paused, the clock only
    /// moves once nothing else can run, so after this sleep the schedule has
    /// taken every event sent before it.
    async fn settle() {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    fn started(provided: &Provided, latest: Vec<Reading>) -> mpsc::UnboundedSender<Event> {
        for reading in &latest {
            provided.remember(reading.clone());
        }
        let schedule = Schedule::new(provided.clone(), PausedClock::at(START), latest);
        let events = schedule.sender();
        tokio::spawn(schedule.run());
        events
    }

    fn ended(provider: Provider, at: i64) -> Event {
        Event::TurnEnded(TurnEnd {
            provider,
            at: Instant(at),
        })
    }

    #[tokio::test(start_paused = true)]
    async fn a_turn_end_reads_its_provider_once() {
        let provided = Provided::new(0);
        let events = started(&provided, Vec::new());
        events.send(ended(Provider::Codex, START)).unwrap();
        settle().await;
        assert_eq!(provided.count(&Provider::Codex), 1);
        assert_eq!(provided.count(&Provider::Claude), 0, "only its provider");
        events.send(ended(Provider::Claude, START)).unwrap();
        settle().await;
        assert_eq!(provided.count(&Provider::Claude), 1);
        assert_eq!(provided.count(&Provider::Codex), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_turn_ends_is_one_read_and_at_most_one_more() {
        let provided = Provided::new(0).held();
        let events = started(&provided, Vec::new());
        events.send(ended(Provider::Claude, START)).unwrap();
        settle().await;
        assert_eq!(provided.count(&Provider::Claude), 1, "the first starts a read");
        for second in 1..=5 {
            events.send(ended(Provider::Claude, START + second)).unwrap();
        }
        settle().await;
        assert_eq!(provided.count(&Provider::Claude), 1, "the burst waits on it");
        provided.release();
        settle().await;
        assert_eq!(provided.count(&Provider::Claude), 2, "one follow-up for the burst");
        provided.release();
        settle().await;
        assert_eq!(provided.count(&Provider::Claude), 2, "and nothing after it");
        assert_eq!(provided.count(&Provider::Codex), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_read_while_nothing_happens() {
        // A reading in memory whose windows name no later reset, and no
        // turn end, no question: a week passes quiet.
        let provided = Provided::new(0);
        let _events = started(&provided, vec![reading(Provider::Codex, START - 60, 0)]);
        tokio::time::sleep(Duration::from_secs(7 * 86_400)).await;
        assert!(provided.reads().is_empty(), "read: {:?}", provided.reads());
    }

    #[tokio::test(start_paused = true)]
    async fn a_window_reset_reads_once_at_its_moment_and_the_next_reading_resets_the_timer() {
        // In memory: a Claude reading whose window resets an hour from now.
        // Each read answers a window that resets five hours after it.
        let provided = Provided::new(5 * 3_600);
        let _events = started(
            &provided,
            vec![reading(Provider::Claude, START - 60, START + 3_600)],
        );
        tokio::time::sleep(Duration::from_secs(3_599)).await;
        assert!(provided.reads().is_empty(), "not before the reset");
        tokio::time::sleep(Duration::from_secs(1)).await;
        settle().await;
        assert_eq!(provided.reads(), vec![(Provider::Claude, START + 3_600)]);
        // The new reading's reset replaces the timer: one read five hours on.
        tokio::time::sleep(Duration::from_secs(5 * 3_600 - 2)).await;
        assert_eq!(provided.reads().len(), 1);
        tokio::time::sleep(Duration::from_secs(2)).await;
        settle().await;
        assert_eq!(
            provided.reads(),
            vec![
                (Provider::Claude, START + 3_600),
                (Provider::Claude, START + 3_600 + 5 * 3_600)
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_newer_reading_replaces_the_reset_timer() {
        // Memory says Codex resets in an hour; a turn end reads it first,
        // and that reading says its window resets in two hours.
        let provided = Provided::new(2 * 3_600);
        let events = started(
            &provided,
            vec![reading(Provider::Codex, START - 60, START + 3_600)],
        );
        tokio::time::sleep(Duration::from_secs(600)).await;
        events.send(ended(Provider::Codex, START + 600)).unwrap();
        settle().await;
        assert_eq!(provided.reads(), vec![(Provider::Codex, START + 600)]);
        // The hour passes: the old timer is gone.
        tokio::time::sleep(Duration::from_secs(3_000 + 60)).await;
        assert_eq!(provided.reads().len(), 1);
        // Two hours after that reading, the new one fires.
        tokio::time::sleep(Duration::from_secs(2 * 3_600 - 3_060)).await;
        settle().await;
        assert_eq!(
            provided.reads(),
            vec![
                (Provider::Codex, START + 600),
                (Provider::Codex, START + 600 + 2 * 3_600)
            ]
        );
    }

    async fn asked(events: &mpsc::UnboundedSender<Event>, provider: Provider) -> Response {
        let (answer, answered) = oneshot::channel();
        events.send(Event::Question(provider, answer)).unwrap();
        answered.await.unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn a_question_reads_when_the_reading_is_older_than_the_last_turn_end() {
        let provided = Provided::new(0);
        let older = reading(Provider::Codex, START - 600, 0);
        let events = started(&provided, vec![older.clone()]);
        // Fresh: the last turn end is older than the reading. The opening
        // reads nothing, and the question is answered from memory.
        events
            .send(Event::TurnEndsObserved(vec![TurnEnd {
                provider: Provider::Codex,
                at: Instant(START - 900),
            }]))
            .unwrap();
        let answer = asked(&events, Provider::Codex).await;
        assert!(provided.reads().is_empty(), "a fresh reading is not read again");
        assert!(matches!(answer, Response::Read(read) if read.reading == older));
        // A turn end whose read fails leaves the memory older than it: the
        // next question reads.
        *provided.down.lock().unwrap() = true;
        events.send(ended(Provider::Codex, START)).unwrap();
        settle().await;
        assert_eq!(provided.count(&Provider::Codex), 1);
        *provided.down.lock().unwrap() = false;
        tokio::time::sleep(Duration::from_secs(5)).await;
        let answer = asked(&events, Provider::Codex).await;
        assert_eq!(provided.count(&Provider::Codex), 2, "a stale reading is read first");
        assert!(
            matches!(&answer, Response::Read(read) if read.reading.at.0 > START),
            "answered with the new reading: {answer:?}"
        );
        // Now fresh again: answered from memory.
        let _ = asked(&events, Provider::Codex).await;
        assert_eq!(provided.count(&Provider::Codex), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_question_with_no_reading_reads_and_one_in_flight_waits_for_it() {
        let provided = Provided::new(0).held();
        let events = started(&provided, Vec::new());
        let first = tokio::spawn({
            let events = events.clone();
            async move { asked(&events, Provider::Claude).await }
        });
        settle().await;
        assert_eq!(provided.count(&Provider::Claude), 1);
        let second = tokio::spawn({
            let events = events.clone();
            async move { asked(&events, Provider::Claude).await }
        });
        settle().await;
        assert_eq!(provided.count(&Provider::Claude), 1, "the second waits on the read in flight");
        provided.release();
        assert!(matches!(first.await.unwrap(), Response::Read(_)));
        assert!(matches!(second.await.unwrap(), Response::Read(_)));
        assert_eq!(provided.count(&Provider::Claude), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_opening_reads_each_provider_whose_reading_predates_its_turn_end() {
        let provided = Provided::new(0);
        let events = started(
            &provided,
            vec![
                reading(Provider::Claude, START - 600, 0),
                reading(Provider::Codex, START - 600, 0),
            ],
        );
        events
            .send(Event::TurnEndsObserved(vec![
                TurnEnd {
                    provider: Provider::Claude,
                    at: Instant(START - 60),
                },
                TurnEnd {
                    provider: Provider::Codex,
                    at: Instant(START - 900),
                },
            ]))
            .unwrap();
        settle().await;
        assert_eq!(provided.reads(), vec![(Provider::Claude, START)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_window_reset_fires_at_its_wall_clock_instant_across_a_jump() {
        // In memory: a Codex reading whose window resets an hour from now.
        // The machine sleeps two hours: the wall clock jumps, and the
        // monotonic clock (tokio's paused time) does not move at all. A timer
        // counted in monotonic time would still be an hour away; the alarm,
        // held as its wall-clock instant, has passed, and reads once.
        let provided = Provided::new(5 * 3_600);
        let clock = SteppedClock::at(START);
        provided.remember(reading(Provider::Codex, START - 60, START + 3_600));
        let schedule = Schedule::new(
            provided.clone(),
            clock.clone(),
            vec![reading(Provider::Codex, START - 60, START + 3_600)],
        );
        let _events = schedule.sender();
        tokio::spawn(schedule.run());
        settle().await;
        clock.step(START + 3_599);
        settle().await;
        assert!(provided.reads().is_empty(), "not before its instant");
        let monotonic = tokio::time::Instant::now();
        clock.step(START + 2 * 3_600);
        settle().await;
        assert_eq!(provided.count(&Provider::Codex), 1, "read once on waking");
        assert!(tokio::time::Instant::now() - monotonic < Duration::from_secs(1));
        // The reading's own reset (five hours after it, on the provided
        // fake's clock) is the next alarm: set, and not fired by the jump.
        let alarms = clock.alarms.lock().unwrap();
        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].0, Provider::Codex);
        assert!(alarms[0].1 > START + 2 * 3_600);
    }

    #[tokio::test(start_paused = true)]
    async fn a_replaced_alarm_that_fired_reads_nothing() {
        // An alarm fires for an instant a newer reading has already moved:
        // the schedule reads only when the provider's reset has come.
        let provided = Provided::new(0);
        let clock = SteppedClock::at(START);
        let schedule = Schedule::new(
            provided.clone(),
            clock.clone(),
            vec![reading(Provider::Claude, START - 60, START + 3_600)],
        );
        let events = schedule.sender();
        tokio::spawn(schedule.run());
        events.send(Event::WindowReset(Provider::Claude)).unwrap();
        settle().await;
        assert!(provided.reads().is_empty());
    }

    #[tokio::test]
    async fn the_wall_clock_alarm_is_an_absolute_realtime_timer_that_fires_at_its_instant() {
        let mut clock = WallClock::default();
        let (events, mut inbox) = mpsc::unbounded_channel();
        let instant = clock.unix_now() + 2;
        clock.alarm(&Provider::Claude, Some(instant), &events);
        // The kernel holds it on CLOCK_REALTIME (clockid 0) with
        // TFD_TIMER_ABSTIME (settime flags 01): an instant, not a length.
        use std::os::fd::{AsFd, AsRawFd};
        let fd = clock.alarms[0].1.as_fd().as_raw_fd();
        let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).unwrap();
        println!("alarm fdinfo:\n{info}");
        assert!(info.contains("clockid: 0\n"), "{info}");
        assert!(info.contains("settime flags: 01\n"), "{info}");
        let fired = tokio::time::timeout(Duration::from_secs(30), inbox.recv())
            .await
            .expect("the alarm fires");
        assert!(matches!(fired, Some(Event::WindowReset(Provider::Claude))));
        assert!(clock.unix_now() >= instant, "not before its instant");
        // An instant already passed fires at once.
        clock.alarm(&Provider::Codex, Some(instant - 600), &events);
        let fired = tokio::time::timeout(Duration::from_secs(30), inbox.recv())
            .await
            .expect("a passed instant fires at once");
        assert!(matches!(fired, Some(Event::WindowReset(Provider::Codex))));
    }
}
