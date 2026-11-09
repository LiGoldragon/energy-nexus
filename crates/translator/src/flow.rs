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
use crate::{turn_end_outcome, FlowFraming};
use energy_nexus_emission::{Outcome, Response as EmissionResponse};
use energy_nexus_wire::Framing;
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use signal::{ByteViewable, Signalizable};
use std::ffi::OsString;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// Opens the subscription on Flow's socket: one connection and the
/// `Observe.TurnEnds` query on it.
pub trait SubscribesToFlow: Send + 'static {
    fn subscribe(&self, socket: &Path) -> std::io::Result<UnixStream>;
}
/// The subscription as Flow 0.53.3 takes it: `Observe.TurnEnds`, signal.
pub struct ObserveTurnEnds;
impl SubscribesToFlow for ObserveTurnEnds {
    fn subscribe(&self, socket: &Path) -> std::io::Result<UnixStream> {
        let query = signal_flow::Query::Observe(signal_flow::ObserveSelection::TurnEnds);
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&query)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let mut flow = UnixStream::connect(socket)?;
        flow.write_flow_frame(&bytes)?;
        Ok(flow)
    }
}

/// The watch on Flow's socket in the place's runtime directory. It names
/// the socket made there (`IN_CREATE`), moved there (`IN_MOVED_TO`) or
/// changed there (`IN_ATTRIB`): Flow sets its socket's mode once it listens,
/// so a socket found made but not yet listening is followed by that event.
pub struct FlowWatch {
    inotify: Inotify,
    socket: PathBuf,
    name: OsString,
}
/// What one batch of the watch's events says.
#[derive(Debug, PartialEq, Eq)]
pub enum Watched {
    /// Flow's socket was made, moved in or changed.
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
            AddWatchFlags::IN_CREATE
                | AddWatchFlags::IN_MOVED_TO
                | AddWatchFlags::IN_ATTRIB
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
    /// Waits for Flow's socket to appear and subscribes on that event. A
    /// subscription refused after the event (the socket made but not yet
    /// listening) waits for the next one.
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
    use signal::{Restorable, Signal};
    use std::io::ErrorKind;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;
    use std::time::Duration;

    const EVENT_WAIT: Duration = Duration::from_secs(60);

    /// The real subscription, each attempt and its result reported to the
    /// test as it is made.
    struct Counted(mpsc::Sender<Result<(), ErrorKind>>);
    impl SubscribesToFlow for Counted {
        fn subscribe(&self, socket: &Path) -> std::io::Result<UnixStream> {
            let result = ObserveTurnEnds.subscribe(socket);
            let _ = self
                .0
                .send(result.as_ref().map(|_| ()).map_err(|error| error.kind()));
            result
        }
    }

    /// A Flow socket bound and not yet listening, as Flow's is for the
    /// moment between bind and listen.
    fn bound(socket: &Path) -> std::os::fd::OwnedFd {
        use nix::sys::socket::{bind, socket as open, AddressFamily, SockFlag, SockType, UnixAddr};
        use std::os::fd::AsRawFd;
        let fd = open(AddressFamily::Unix, SockType::Stream, SockFlag::SOCK_CLOEXEC, None).unwrap();
        bind(fd.as_raw_fd(), &UnixAddr::new(socket).unwrap()).unwrap();
        fd
    }

    /// As Flow 0.53.3 does once its socket listens.
    fn protect(socket: &Path) {
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    /// The other files of a shared place changing: no event of Flow's.
    fn others_change(runtime: &Path) {
        let other = runtime.join("energy.sock");
        let listener = UnixListener::bind(&other).unwrap();
        protect(&other);
        drop(listener);
        std::fs::remove_file(&other).unwrap();
        std::fs::write(runtime.join("flow.sock.part"), "not Flow's").unwrap();
        std::fs::remove_file(runtime.join("flow.sock.part")).unwrap();
    }

    /// Flow takes one subscription: the accepted connection and its query.
    fn accepted(listener: &UnixListener) -> (UnixStream, signal_flow::Query) {
        let (taken, take) = mpsc::channel();
        let listener = listener.try_clone().unwrap();
        std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let query = rkyv::from_bytes::<signal_flow::Query, rkyv::rancor::Error>(
                &peer.read_flow_frame().unwrap(),
            )
            .unwrap();
            let _ = taken.send((peer, query));
        });
        take.recv_timeout(EVENT_WAIT).expect("Flow takes a subscription")
    }

    fn opening(at: i64) -> Vec<u8> {
        let end = signal_flow::TurnEnd {
            address: signal_flow::Address {
                aspect: signal_flow::Aspect::Psyche,
                topic: "core".into(),
                model: signal_flow::Model::Opus,
            },
            harness_kind: signal_flow::HarnessKind::Claude,
            flow_id: "ff3882".into(),
            observed_at: at,
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
        assert_eq!(attempt(), Err(ErrorKind::NotFound));
        // Other files of the place change: no attempt. (Shown by the next
        // attempt, which follows them in the watch's order: it finds the
        // socket bound, so it was made on that event, not on these.)
        others_change(&runtime);
        // Flow's socket made, not yet listening: one attempt, refused, and
        // the relay waits for the next event, not on a timer.
        let flow = bound(&socket);
        assert_eq!(attempt(), Err(ErrorKind::ConnectionRefused));
        // Flow listens (no event), then sets its socket's mode: that event
        // subscribes.
        nix::sys::socket::listen(&flow, nix::sys::socket::Backlog::new(16).unwrap()).unwrap();
        protect(&socket);
        assert_eq!(attempt(), Ok(()));
        let first = UnixListener::from(flow);
        let (mut peer, query) = accepted(&first);
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

        // Flow comes back as Flow binds: bound and listening, then its mode
        // set. One subscription; its opening reaches the daemon.
        let second = UnixListener::bind(&socket).unwrap();
        protect(&socket);
        assert_eq!(attempt(), Ok(()));
        let (mut peer, query) = accepted(&second);
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
                topic: "energyNexus".into(),
                model: signal_flow::Model::Sol,
            },
            harness_kind: signal_flow::HarnessKind::Codex,
            flow_id: "ff3882".into(),
            observed_at: 201,
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
        // each appearance; no other.
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
        let listener = UnixListener::bind(&socket).unwrap();
        let (mut daemon, relay_end) = UnixStream::pair().unwrap();
        daemon.set_read_timeout(Some(EVENT_WAIT)).unwrap();
        let (attempted, attempts) = mpsc::channel();
        let relay = std::thread::spawn({
            let socket = socket.clone();
            move || TurnEndRelay::start(socket, Counted(attempted), relay_end)
        });
        assert_eq!(attempts.recv_timeout(EVENT_WAIT).unwrap(), Ok(()));
        let (peer, _) = accepted(&listener);
        // Flow goes and the place's runtime directory is removed: the watch
        // is lost, and the daemon is told the turn ends ended.
        drop(peer);
        drop(listener);
        std::fs::remove_dir_all(&runtime).unwrap();
        assert_eq!(outcome(&mut daemon), Outcome::Down);
        relay.join().unwrap();
        assert!(attempts.try_recv().is_err(), "no attempt after the first");
    }
}
