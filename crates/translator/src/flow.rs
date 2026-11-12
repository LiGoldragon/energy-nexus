//! Flow's turn ends, held for the daemon across Flow going and coming back.
//!
//! The translator subscribes to the Flow Nexus at the place, `Observe.TurnEnds`
//! on its ordinary socket, and hands each frame to the daemon. When Flow
//! goes, the subscription closes and Energy keeps running: the daemon's
//! connection stays open, readings and questions go on from memory, and the
//! translator waits for Flow's socket to appear in the place's runtime
//! directory, watched by inotify. It subscribes again on that event, and the
//! new opening (the last turn end on each provider) reaches the daemon as the
//! first one did at start. Nothing is tried on a timer: the watch is armed
//! before Flow is first looked for, so a socket made at any later moment is
//! an event, and every connection attempt after the first follows one.
//!
//! Every subscription greets first, as signal-flow 61 requires of an
//! ordinary connection: the translator writes signal-flow's `Handshake`,
//! reads the receipt, and writes its query only on `Greeted` naming the
//! digest of the contract it was built with. Any other answer is a typed
//! `SubscriptionFailure` and the connection is closed with no query sent.
use crate::{turn_end_outcome, FlowFraming};
use energy_nexus_emission::{Outcome, Response as EmissionResponse};
use energy_nexus_wire::Framing;
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use signal::{ByteViewable, Signalizable};
use signal_flow::Contracted;
use std::ffi::OsString;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long Flow has to answer the greeting. A Flow that accepted the
/// connection and says nothing is a failed subscription, which waits for
/// the socket's next event like any other.
const GREETING_WAIT: Duration = Duration::from_secs(30);

/// Why one subscription was not opened. Each is a failed attempt; the
/// relay waits for the next event of Flow's socket.
#[derive(Debug)]
pub enum SubscriptionFailure {
    /// The socket could not be reached, or a frame could not be written or
    /// read on it.
    Transport(std::io::Error),
    /// Flow answered the greeting with `GreetingRefused`: its contract is
    /// not signal-flow 61's. The rejection carries Flow's own digest.
    GreetingRefused(signal_flow::HandshakeRejection),
    /// Flow answered `Greeted` naming a contract other than this
    /// translator's.
    GreetedAnotherContract(signal_flow::ContractDigest),
    /// Flow's answer to the greeting is no `HandshakeReceipt`.
    ReceiptUnreadable(String),
}
impl From<std::io::Error> for SubscriptionFailure {
    fn from(error: std::io::Error) -> Self {
        Self::Transport(error)
    }
}
impl std::fmt::Display for SubscriptionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "{error}"),
            Self::GreetingRefused(rejection) => {
                write!(formatter, "ordinary greeting refused: {rejection:?}")
            }
            Self::GreetedAnotherContract(digest) => {
                write!(formatter, "Flow greeted with another contract: {digest:?}")
            }
            Self::ReceiptUnreadable(reason) => {
                write!(formatter, "Flow's greeting receipt is unreadable: {reason}")
            }
        }
    }
}

/// Writes signal-flow's greeting and reads the receipt: Ok on `Greeted`
/// naming the digest of the contract this translator was built with.
pub trait GreetsFlowNexus {
    fn greet_flow(&mut self) -> Result<(), SubscriptionFailure>;
}
impl GreetsFlowNexus for UnixStream {
    fn greet_flow(&mut self) -> Result<(), SubscriptionFailure> {
        let greeting = rkyv::to_bytes::<rkyv::rancor::Error>(&signal_flow::Query::greeting())
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        self.write_flow_frame(&greeting)?;
        self.set_read_timeout(Some(GREETING_WAIT))?;
        let frame = self.read_flow_frame()?;
        self.set_read_timeout(None)?;
        match rkyv::from_bytes::<signal_flow::HandshakeReceipt, rkyv::rancor::Error>(&frame) {
            Ok(signal_flow::HandshakeReceipt::Greeted(digest))
                if digest == signal_flow::Query::contract_digest() =>
            {
                Ok(())
            }
            Ok(signal_flow::HandshakeReceipt::Greeted(digest)) => {
                Err(SubscriptionFailure::GreetedAnotherContract(digest))
            }
            Ok(signal_flow::HandshakeReceipt::GreetingRefused(rejection)) => {
                Err(SubscriptionFailure::GreetingRefused(rejection))
            }
            Err(error) => Err(SubscriptionFailure::ReceiptUnreadable(error.to_string())),
        }
    }
}

/// Opens the subscription on Flow's socket: one connection, the greeting on
/// it, and the `Observe.TurnEnds` query once Flow has greeted.
pub trait SubscribesToFlow: Send + 'static {
    fn subscribe(&self, socket: &Path) -> Result<UnixStream, SubscriptionFailure>;
}
/// The subscription as Flow 0.54.7 takes it (signal-flow 61): the greeting,
/// then `Observe.TurnEnds`.
pub struct ObserveTurnEnds;
impl SubscribesToFlow for ObserveTurnEnds {
    fn subscribe(&self, socket: &Path) -> Result<UnixStream, SubscriptionFailure> {
        let query = signal_flow::Query::Observe(signal_flow::ObserveSelection::TurnEnds);
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&query)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let mut flow = UnixStream::connect(socket)?;
        flow.greet_flow()?;
        flow.write_flow_frame(&bytes)?;
        Ok(flow)
    }
}

/// The watch on Flow's socket in the place's runtime directory. It names
/// the socket moved to its name (`IN_MOVED_TO`) and nothing else: every
/// Nexus at a place makes its socket under a temporary name and renames it
/// to its name once it listens, so that event is Flow's socket listening.
/// Flow places it so from 0.53.3, the first Flow at a place.
pub struct FlowWatch {
    inotify: Inotify,
    socket: PathBuf,
    name: OsString,
}
/// What one batch of the watch's events says.
#[derive(Debug, PartialEq, Eq)]
pub enum Watched {
    /// Flow's socket was moved to its name.
    Appeared,
    /// Only other files changed.
    Other,
    /// The directory is gone or the watch was removed: nothing more will come.
    Lost,
}
impl FlowWatch {
    /// Arms the watch on the socket's directory.
    pub fn arm(socket: PathBuf) -> std::io::Result<Self> {
        let directory = socket
            .parent()
            .ok_or_else(|| std::io::Error::other("Flow's socket has no directory"))?
            .to_owned();
        let name = socket
            .file_name()
            .ok_or_else(|| std::io::Error::other("Flow's socket has no name"))?
            .to_owned();
        let inotify = Inotify::init(InitFlags::IN_CLOEXEC | InitFlags::IN_NONBLOCK)?;
        inotify.add_watch(
            &directory,
            AddWatchFlags::IN_MOVED_TO
                | AddWatchFlags::IN_DELETE_SELF
                | AddWatchFlags::IN_MOVE_SELF
                | AddWatchFlags::IN_ONLYDIR,
        )?;
        Ok(Self {
            inotify,
            socket,
            name,
        })
    }
    pub fn socket(&self) -> &Path {
        &self.socket
    }
    /// Takes the events queued now; called when the watch is readable.
    fn take(&self) -> Watched {
        let events = match self.inotify.read_events() {
            Ok(events) => events,
            Err(nix::errno::Errno::EAGAIN) => return Watched::Other,
            Err(_) => return Watched::Lost,
        };
        let gone = AddWatchFlags::IN_IGNORED
            | AddWatchFlags::IN_DELETE_SELF
            | AddWatchFlags::IN_MOVE_SELF
            | AddWatchFlags::IN_UNMOUNT;
        if events.iter().any(|event| event.mask.intersects(gone)) {
            return Watched::Lost;
        }
        if events
            .iter()
            .any(|event| event.name.as_deref() == Some(self.name.as_os_str()))
        {
            Watched::Appeared
        } else {
            Watched::Other
        }
    }
}

/// Why the relay stopped forwarding one subscription.
enum Forwarded {
    /// The daemon closed its end: the relay is over.
    DaemonGone,
    /// Flow closed the subscription or answered otherwise: Flow went.
    FlowClosed,
}
/// Why the relay stopped waiting for Flow.
enum Waited {
    Subscribed(UnixStream),
    DaemonGone,
    WatchLost,
}

/// The daemon's `Emit.TurnEnds`, held open for as long as the daemon holds
/// it, across every time Flow goes and comes back.
pub struct TurnEndRelay<S: SubscribesToFlow> {
    watch: FlowWatch,
    subscriber: S,
    daemon: UnixStream,
}
const DAEMON: u64 = 0;
const OTHER: u64 = 1;
/// One outcome to the daemon, as the emission socket answers.
fn send(daemon: &mut UnixStream, outcome: Outcome) -> Result<(), String> {
    EmissionResponse::Emitted(outcome)
        .signalize()
        .map_err(|error| format!("{error:?}"))
        .and_then(|signal| {
            daemon
                .write_frame(signal.bytes())
                .map_err(|error| error.to_string())
        })
}
impl<S: SubscribesToFlow> TurnEndRelay<S> {
    /// Arms the watch on Flow's socket, then relays (`run`); a watch that
    /// cannot be armed is answered `Down`.
    pub fn start(socket: PathBuf, subscriber: S, mut daemon: UnixStream) {
        match FlowWatch::arm(socket.clone()) {
            Ok(watch) => Self {
                watch,
                subscriber,
                daemon,
            }
            .run(),
            Err(error) => {
                eprintln!(
                    "energy-translator: no watch on {}: {error}; Flow's turn ends are not held",
                    socket.display()
                );
                let _ = send(&mut daemon, Outcome::Down);
            }
        }
    }
    /// Looks for Flow once, the watch already armed; then forwards each
    /// subscription and waits for Flow's socket between them. Returns when
    /// the daemon goes, or sends `Down` and returns when the place's
    /// directory is lost.
    fn run(mut self) {
        let mut subscribed = match self.subscriber.subscribe(self.watch.socket()) {
            Ok(flow) => Some(flow),
            Err(error) => {
                eprintln!(
                    "energy-translator: Flow at {} absent ({error}); waiting for its socket",
                    self.watch.socket().display()
                );
                None
            }
        };
        loop {
            let flow = match subscribed.take() {
                Some(flow) => flow,
                None => match self.wait() {
                    Waited::Subscribed(flow) => flow,
                    Waited::DaemonGone => return,
                    Waited::WatchLost => {
                        eprintln!(
                            "energy-translator: the watch on {} is lost; Flow's turn ends end",
                            self.watch.socket().display()
                        );
                        let _ = send(&mut self.daemon, Outcome::Down);
                        return;
                    }
                },
            };
            eprintln!(
                "energy-translator: subscribed to Flow's turn ends at {}",
                self.watch.socket().display()
            );
            match self.forward(flow) {
                Forwarded::DaemonGone => return,
                Forwarded::FlowClosed => eprintln!(
                    "energy-translator: Flow's turn ends closed; waiting for its socket at {}",
                    self.watch.socket().display()
                ),
            }
        }
    }
    /// One epoll over the daemon's connection and one other descriptor;
    /// the daemon sends nothing after its query, so its end readable is the
    /// daemon gone.
    fn epoll(&self, other: &impl std::os::fd::AsFd) -> std::io::Result<Epoll> {
        let epoll = Epoll::new(EpollCreateFlags::EPOLL_CLOEXEC)?;
        epoll.add(
            &self.daemon,
            EpollEvent::new(EpollFlags::EPOLLIN | EpollFlags::EPOLLRDHUP, DAEMON),
        )?;
        epoll.add(other, EpollEvent::new(EpollFlags::EPOLLIN, OTHER))?;
        Ok(epoll)
    }
    /// Blocks on the next event and says which side it came from.
    fn next(epoll: &Epoll) -> Option<Vec<u64>> {
        let mut events = [EpollEvent::empty(); 2];
        loop {
            match epoll.wait(&mut events, EpollTimeout::NONE) {
                Ok(count) => return Some(events[..count].iter().map(EpollEvent::data).collect()),
                Err(nix::errno::Errno::EINTR) => continue,
                Err(_) => return None,
            }
        }
    }
    /// Waits for Flow's socket to be moved to its name and subscribes on
    /// that event. A subscription refused after the event (something moved
    /// to the name that does not listen) waits for the next one.
    fn wait(&mut self) -> Waited {
        let Ok(epoll) = self.epoll(&self.watch.inotify) else {
            return Waited::WatchLost;
        };
        loop {
            let Some(ready) = Self::next(&epoll) else {
                return Waited::WatchLost;
            };
            if ready.contains(&DAEMON) {
                return Waited::DaemonGone;
            }
            match self.watch.take() {
                Watched::Lost => return Waited::WatchLost,
                Watched::Other => {}
                Watched::Appeared => match self.subscriber.subscribe(self.watch.socket()) {
                    Ok(flow) => return Waited::Subscribed(flow),
                    Err(error) => eprintln!(
                        "energy-translator: Flow's socket at {} did not take the subscription \
                         ({error}); waiting for its next event",
                        self.watch.socket().display()
                    ),
                },
            }
        }
    }
    /// Hands each frame of one subscription to the daemon, until Flow closes
    /// it or the daemon goes; the Flow connection is closed with it.
    fn forward(&mut self, mut flow: UnixStream) -> Forwarded {
        let Ok(epoll) = self.epoll(&flow) else {
            return Forwarded::FlowClosed;
        };
        loop {
            let Some(ready) = Self::next(&epoll) else {
                return Forwarded::FlowClosed;
            };
            if ready.contains(&DAEMON) {
                let _ = flow.shutdown(std::net::Shutdown::Both);
                return Forwarded::DaemonGone;
            }
            let outcome = match flow.read_flow_frame() {
                Ok(frame) => turn_end_outcome(&frame),
                Err(_) => Outcome::Down,
            };
            if matches!(outcome, Outcome::Down) {
                return Forwarded::FlowClosed;
            }
            if send(&mut self.daemon, outcome).is_err() {
                let _ = flow.shutdown(std::net::Shutdown::Both);
                return Forwarded::DaemonGone;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! The relay against a Flow played by the test in a scratch runtime
    //! directory. Every wait is on an event (a subscription attempt, an
    //! accepted connection, a frame) and bounded by EVENT_WAIT.
    use super::*;
    use crate::FlowFraming;
    use energy_nexus_emission::Response as EmissionResponse;
    use energy_nexus_wire::{PlacedSocket, PlacesSocket};
    use signal::{Restorable, Signal};
    use std::io::ErrorKind;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;
    use std::time::Duration;

    const EVENT_WAIT: Duration = Duration::from_secs(60);

    /// How one attempt failed, as the tests compare it.
    #[derive(Debug, PartialEq, Eq)]
    enum Failed {
        Transport(ErrorKind),
        GreetingRefused(signal_flow::HandshakeRejection),
        Other,
    }

    /// The real subscription, each attempt and its result reported to the
    /// test as it is made.
    struct Counted(mpsc::Sender<Result<(), Failed>>);
    impl SubscribesToFlow for Counted {
        fn subscribe(&self, socket: &Path) -> Result<UnixStream, SubscriptionFailure> {
            let result = ObserveTurnEnds.subscribe(socket);
            let _ = self.0.send(result.as_ref().map(|_| ()).map_err(|failure| match failure {
                SubscriptionFailure::Transport(error) => Failed::Transport(error.kind()),
                SubscriptionFailure::GreetingRefused(rejection) => {
                    Failed::GreetingRefused(rejection.clone())
                }
                _ => Failed::Other,
            }));
            result
        }
    }

    /// A socket bound at `socket` and not listening.
    fn bound(socket: &Path) -> std::os::fd::OwnedFd {
        use nix::sys::socket::{bind, socket as open, AddressFamily, SockFlag, SockType, UnixAddr};
        use std::os::fd::AsRawFd;
        let fd = open(AddressFamily::Unix, SockType::Stream, SockFlag::SOCK_CLOEXEC, None).unwrap();
        bind(fd.as_raw_fd(), &UnixAddr::new(socket).unwrap()).unwrap();
        fd
    }

    /// Flow's socket placed as every Nexus at a place places its own: made
    /// under a temporary name, its mode set, listening, then renamed to its
    /// name.
    fn placed(socket: &Path) -> UnixListener {
        PlacedSocket::at(socket).unwrap().claim().unwrap()
    }

    /// The other files of a shared place changing: no event of Flow's.
    /// Another Nexus places its socket and removes it; a temporary name of
    /// Flow's own is made, its mode set, and removed, as a crash between
    /// bind and rename leaves one; another file is written beside them.
    fn others_change(runtime: &Path) {
        let other = runtime.join("energy.sock");
        drop(placed(&other));
        std::fs::remove_file(&other).unwrap();
        let leftover = runtime.join(".flow.sock.1");
        drop(bound(&leftover));
        std::fs::set_permissions(&leftover, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::remove_file(&leftover).unwrap();
        std::fs::write(runtime.join("flow.sock.part"), "not Flow's").unwrap();
        std::fs::remove_file(runtime.join("flow.sock.part")).unwrap();
    }

    /// Flow, as signal-flow 61 plays it, reads the peer's greeting and
    /// answers `receipt` to it.
    fn greeted(
        peer: &mut UnixStream,
        receipt: impl FnOnce(&signal_flow::Handshake) -> signal_flow::HandshakeReceipt,
    ) -> signal_flow::Handshake {
        let greeting = rkyv::from_bytes::<signal_flow::Handshake, rkyv::rancor::Error>(
            &peer.read_flow_frame().unwrap(),
        )
        .unwrap();
        let receipt = rkyv::to_bytes::<rkyv::rancor::Error>(&receipt(&greeting)).unwrap();
        peer.write_flow_frame(&receipt).unwrap();
        greeting
    }

    /// Flow takes one subscription: the accepted connection greets, is
    /// answered `Greeted`, and then sends its query. Flow is listening from
    /// here on, so the greeting is answered while the relay's attempt waits
    /// for it; `accepted` waits for the connection and its query.
    fn accepting(
        listener: &UnixListener,
    ) -> mpsc::Receiver<(UnixStream, signal_flow::Query)> {
        let (taken, take) = mpsc::channel();
        let listener = listener.try_clone().unwrap();
        std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let greeting = greeted(&mut peer, signal_flow::Query::receipt);
            assert_eq!(greeting, signal_flow::Query::greeting());
            let query = rkyv::from_bytes::<signal_flow::Query, rkyv::rancor::Error>(
                &peer.read_flow_frame().unwrap(),
            )
            .unwrap();
            let _ = taken.send((peer, query));
        });
        take
    }
    fn accepted(
        taking: mpsc::Receiver<(UnixStream, signal_flow::Query)>,
    ) -> (UnixStream, signal_flow::Query) {
        taking.recv_timeout(EVENT_WAIT).expect("Flow takes a subscription")
    }

    /// A Flow that answers the greeting with `receipt` whatever it is, and
    /// reports what else the peer sent before it closed.
    fn answering(
        listener: &UnixListener,
        receipt: signal_flow::HandshakeReceipt,
    ) -> mpsc::Receiver<Vec<u8>> {
        let (rest, read) = mpsc::channel();
        let listener = listener.try_clone().unwrap();
        std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            greeted(&mut peer, |_| receipt);
            let mut after = Vec::new();
            let _ = std::io::Read::read_to_end(&mut peer, &mut after);
            let _ = rest.send(after);
        });
        read
    }

    /// The receipt of a Flow built on another contract, naming its digest.
    fn refusal() -> signal_flow::HandshakeReceipt {
        signal_flow::HandshakeReceipt::GreetingRefused(
            signal_flow::HandshakeRejection::ContractMismatch(signal_flow::ContractDigest(0x60)),
        )
    }

    fn opening(at: i64) -> Vec<u8> {
        let end = signal_flow::TurnEnd {
            address: signal_flow::Address {
                aspect: signal_flow::Aspect::Psyche,
                topic: signal_flow::Topic("core".into()),
                model: signal_flow::Model::Opus,
            },
            harness_kind: signal_flow::HarnessKind::Claude,
            flow_id: signal_flow::FlowId("ff3882".into()),
            observed_at: signal_flow::ObservedAt(at),
        };
        rkyv::to_bytes::<rkyv::rancor::Error>(&signal_flow::Response::TurnEndsObserved(vec![end]))
            .unwrap()
            .to_vec()
    }

    fn outcome(daemon: &mut UnixStream) -> Outcome {
        let EmissionResponse::Emitted(outcome) =
            Signal::<EmissionResponse>::from(daemon.read_frame().unwrap())
                .restore()
                .unwrap();
        outcome
    }

    #[test]
    fn flow_absent_is_waited_for_and_each_appearance_subscribes_once() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = directory.path().to_owned();
        let socket = runtime.join("flow.sock");
        let (daemon_end, relay_end) = UnixStream::pair().unwrap();
        let mut daemon = daemon_end;
        daemon.set_read_timeout(Some(EVENT_WAIT)).unwrap();
        let (attempted, attempts) = mpsc::channel();
        let relay = {
            let socket = socket.clone();
            std::thread::spawn(move || {
                TurnEndRelay::start(socket, Counted(attempted), relay_end)
            })
        };
        let attempt = || attempts.recv_timeout(EVENT_WAIT).expect("an attempt");

        // Flow absent: looked for once, after the watch is armed.
        assert_eq!(attempt(), Err(Failed::Transport(ErrorKind::NotFound)));
        // Other files of the place change: no attempt. (Shown by the next
        // attempt, which follows them in the watch's order: it finds the
        // socket bound, so it was made on that event, not on these.)
        others_change(&runtime);
        // A socket that does not listen moved to Flow's name: one attempt,
        // refused, and the relay waits for the next event, not on a timer.
        let moved = runtime.join("unlistened");
        let unlistened = bound(&moved);
        std::fs::rename(&moved, &socket).unwrap();
        assert_eq!(attempt(), Err(Failed::Transport(ErrorKind::ConnectionRefused)));
        // Flow places its socket over the stale one: made under its
        // temporary name, its mode set, listening (no event of the name's),
        // then moved to its name: that event subscribes, at once.
        let first = placed(&socket);
        let taking = accepting(&first);
        drop(unlistened);
        assert_eq!(attempt(), Ok(()));
        let (mut peer, query) = accepted(taking);
        assert_eq!(
            query,
            signal_flow::Query::Observe(signal_flow::ObserveSelection::TurnEnds)
        );
        peer.write_flow_frame(&opening(100)).unwrap();
        assert!(matches!(outcome(&mut daemon), Outcome::TurnEndsObserved(ends) if ends[0].at.0 == 100));

        // Flow goes: it closes the subscription and removes its socket. The
        // daemon hears nothing of it, and nothing is attempted.
        drop(peer);
        drop(first);
        std::fs::remove_file(&socket).unwrap();
        others_change(&runtime);

        // Flow comes back and places its socket. One subscription; its
        // opening reaches the daemon.
        let second = placed(&socket);
        let taking = accepting(&second);
        assert_eq!(attempt(), Ok(()));
        let (mut peer, query) = accepted(taking);
        assert_eq!(
            query,
            signal_flow::Query::Observe(signal_flow::ObserveSelection::TurnEnds)
        );
        peer.write_flow_frame(&opening(200)).unwrap();
        assert!(matches!(outcome(&mut daemon), Outcome::TurnEndsObserved(ends) if ends[0].at.0 == 200));
        // A turn end on the new subscription is relayed as before.
        let ended = signal_flow::Response::TurnEnded(signal_flow::TurnEnd {
            address: signal_flow::Address {
                aspect: signal_flow::Aspect::Mind,
                topic: signal_flow::Topic("energyNexus".into()),
                model: signal_flow::Model::Sol,
            },
            harness_kind: signal_flow::HarnessKind::Codex,
            flow_id: signal_flow::FlowId("ff3882".into()),
            observed_at: signal_flow::ObservedAt(201),
        });
        peer.write_flow_frame(&rkyv::to_bytes::<rkyv::rancor::Error>(&ended).unwrap())
            .unwrap();
        assert!(matches!(outcome(&mut daemon), Outcome::TurnEnded(end) if end.at.0 == 201));

        // The daemon goes: the relay closes Flow's connection and ends.
        drop(daemon);
        peer.set_read_timeout(Some(EVENT_WAIT)).unwrap();
        let mut rest = Vec::new();
        assert_eq!(std::io::Read::read_to_end(&mut peer, &mut rest).unwrap(), 0);
        relay.join().unwrap();
        // Four attempts in all: the first look, the refused one, and one on
        // each placing of Flow's socket; no other.
        assert!(attempts.try_recv().is_err(), "no further attempt");
        second.set_nonblocking(true).unwrap();
        assert_eq!(
            second.accept().map(|_| ()).map_err(|error| error.kind()),
            Err(ErrorKind::WouldBlock),
            "one subscription on Flow's return"
        );
    }

    #[test]
    fn flow_already_listening_is_subscribed_at_once_and_a_lost_place_ends_the_relay() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = directory.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let socket = runtime.join("flow.sock");
        let listener = placed(&socket);
        let taking = accepting(&listener);
        let (mut daemon, relay_end) = UnixStream::pair().unwrap();
        daemon.set_read_timeout(Some(EVENT_WAIT)).unwrap();
        let (attempted, attempts) = mpsc::channel();
        let relay = std::thread::spawn({
            let socket = socket.clone();
            move || TurnEndRelay::start(socket, Counted(attempted), relay_end)
        });
        assert_eq!(attempts.recv_timeout(EVENT_WAIT).unwrap(), Ok(()));
        let (peer, _) = accepted(taking);
        // Flow goes and the place's runtime directory is removed: the watch
        // is lost, and the daemon is told the turn ends ended.
        drop(peer);
        drop(listener);
        std::fs::remove_dir_all(&runtime).unwrap();
        assert_eq!(outcome(&mut daemon), Outcome::Down);
        relay.join().unwrap();
        assert!(attempts.try_recv().is_err(), "no attempt after the first");
    }

    #[test]
    fn a_refused_greeting_is_a_typed_failure_and_no_query_is_sent() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("flow.sock");
        let listener = placed(&socket);
        let rest = answering(&listener, refusal());
        match ObserveTurnEnds.subscribe(&socket) {
            Err(SubscriptionFailure::GreetingRefused(
                signal_flow::HandshakeRejection::ContractMismatch(digest),
            )) => assert_eq!(digest, signal_flow::ContractDigest(0x60)),
            other => panic!("a typed refusal, got {other:?}"),
        }
        assert!(
            rest.recv_timeout(EVENT_WAIT).expect("Flow's connection ends").is_empty(),
            "nothing follows a refused greeting"
        );
    }

    #[test]
    fn a_greeting_answered_with_another_contract_is_a_typed_failure() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("flow.sock");
        let listener = placed(&socket);
        let other = signal_flow::ContractDigest(0x61);
        let rest = answering(&listener, signal_flow::HandshakeReceipt::Greeted(other));
        match ObserveTurnEnds.subscribe(&socket) {
            Err(SubscriptionFailure::GreetedAnotherContract(digest)) => assert_eq!(digest, other),
            other => panic!("a typed failure, got {other:?}"),
        }
        assert!(rest.recv_timeout(EVENT_WAIT).unwrap().is_empty());
    }

    #[test]
    fn a_flow_that_refuses_the_greeting_is_waited_on_until_its_socket_is_placed_again() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = directory.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let socket = runtime.join("flow.sock");
        let refusing = placed(&socket);
        let rest = answering(&refusing, refusal());
        let (mut daemon, relay_end) = UnixStream::pair().unwrap();
        daemon.set_read_timeout(Some(EVENT_WAIT)).unwrap();
        let (attempted, attempts) = mpsc::channel();
        let relay = std::thread::spawn({
            let socket = socket.clone();
            move || TurnEndRelay::start(socket, Counted(attempted), relay_end)
        });
        let attempt = || attempts.recv_timeout(EVENT_WAIT).expect("an attempt");
        // The first look is refused at the greeting, and no query follows.
        assert_eq!(
            attempt(),
            Err(Failed::GreetingRefused(
                signal_flow::HandshakeRejection::ContractMismatch(signal_flow::ContractDigest(
                    0x60
                ))
            ))
        );
        assert!(rest.recv_timeout(EVENT_WAIT).unwrap().is_empty());
        // The relay waits on the socket's next event, not on a timer. A
        // Flow that greets properly is placed; the next attempt succeeds.
        std::fs::remove_file(&socket).unwrap();
        drop(refusing);
        let second = placed(&socket);
        let taking = accepting(&second);
        assert_eq!(attempt(), Ok(()));
        let (mut peer, query) = accepted(taking);
        assert_eq!(
            query,
            signal_flow::Query::Observe(signal_flow::ObserveSelection::TurnEnds)
        );
        peer.write_flow_frame(&opening(300)).unwrap();
        assert!(matches!(outcome(&mut daemon), Outcome::TurnEndsObserved(ends) if ends[0].at.0 == 300));
        drop(daemon);
        peer.set_read_timeout(Some(EVENT_WAIT)).unwrap();
        let mut closed = Vec::new();
        assert_eq!(std::io::Read::read_to_end(&mut peer, &mut closed).unwrap(), 0);
        relay.join().unwrap();
        assert!(attempts.try_recv().is_err(), "no further attempt");
    }
}
