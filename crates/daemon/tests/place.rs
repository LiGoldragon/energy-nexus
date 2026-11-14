//! The daemon at a place shared with other Nexuses: it finds the place's
//! directories, makes only its own sockets in them, announces readiness
//! once they listen, and removes only those sockets when it is stopped.
//! Every wait here is on an event, READY=1 or the child's exit, and each is
//! bounded by EVENT_WAIT: a daemon that never announces or never exits fails
//! the test there.
use energy_nexus_wire::{Place, PlaceName, SocketPathing};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const DAEMON: &str = env!("CARGO_BIN_EXE_energy-nexus-daemon");
/// How long a test waits for any one event before it fails.
const EVENT_WAIT: Duration = Duration::from_secs(60);

struct SharedPlace {
    _directory: tempfile::TempDir,
    place: Place,
}
/// A started daemon: its pid and its exit, handed over by the thread that
/// waits on it.
struct Running {
    pid: Pid,
    exit: mpsc::Receiver<ExitStatus>,
}
impl Running {
    fn started(mut child: Child) -> Self {
        let pid = Pid::from_raw(child.id() as i32);
        let (exited, exit) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = exited.send(child.wait().unwrap());
        });
        Self { pid, exit }
    }
    /// The exit, or the daemon killed and the test failed after EVENT_WAIT.
    fn exit(self) -> ExitStatus {
        match self.exit.recv_timeout(EVENT_WAIT) {
            Ok(status) => status,
            Err(error) => {
                let _ = kill(self.pid, Signal::SIGKILL);
                panic!("the daemon did not exit within {EVENT_WAIT:?}: {error}")
            }
        }
    }
    /// Stops the daemon as systemd does, and waits for its exit.
    fn stop(self) -> ExitStatus {
        kill(self.pid, Signal::SIGTERM).unwrap();
        self.exit()
    }
}
impl SharedPlace {
    fn declared() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let place = Place {
            name: PlaceName("main".into()),
            runtime: directory.path().join("runtime"),
            state: directory.path().join("state"),
        };
        std::fs::create_dir(&place.runtime).unwrap();
        std::fs::create_dir(&place.state).unwrap();
        Self {
            _directory: directory,
            place,
        }
    }
    fn own(&self) -> Vec<std::path::PathBuf> {
        vec![self.place.public(), self.place.meta()]
    }
    /// Files of other processes and Nexuses at the same place.
    fn others(&self) -> Vec<std::path::PathBuf> {
        let others = vec![
            self.place.public_translator(),
            self.place.runtime.join("flow.sock"),
            self.place.state.join("flow.sema"),
        ];
        for other in &others {
            std::fs::write(other, "another's").unwrap();
        }
        others
    }
    fn command(place: &Place) -> Command {
        let mut command = Command::new(DAEMON);
        command
            .arg(&place.name.0)
            .arg(&place.runtime)
            .arg(&place.state)
            .stdin(Stdio::null());
        command
    }
    /// Starts the daemon with the notify socket systemd would give it, and
    /// returns once it announced READY=1.
    fn start(&self) -> Running {
        self.launch(Stdio::inherit()).0
    }
    /// As `start`, with the daemon's standard error taken as given.
    fn launch(&self, stderr: Stdio) -> (Running, Option<std::process::ChildStderr>) {
        let notify_directory = tempfile::tempdir().unwrap();
        let notify_path = notify_directory.path().join("notify.sock");
        let notify = UnixDatagram::bind(&notify_path).unwrap();
        notify.set_read_timeout(Some(EVENT_WAIT)).unwrap();
        let mut child = Self::command(&self.place)
            .env("NOTIFY_SOCKET", &notify_path)
            .stderr(stderr)
            .spawn()
            .unwrap();
        let errors = child.stderr.take();
        let running = Running::started(child);
        let mut message = [0_u8; 64];
        match notify.recv(&mut message) {
            Ok(length) => assert_eq!(&message[..length], b"READY=1"),
            Err(error) => {
                let _ = kill(running.pid, Signal::SIGKILL);
                panic!("the daemon announced no readiness: {error}");
            }
        }
        (running, errors)
    }
}

#[test]
fn a_daemon_announces_readiness_once_its_sockets_listen() {
    let shared = SharedPlace::declared();
    let running = shared.start();
    for socket in shared.own() {
        UnixStream::connect(&socket)
            .unwrap_or_else(|error| panic!("{} listens at readiness: {error}", socket.display()));
    }
    assert!(running.stop().success());
}

#[test]
fn a_stopped_daemon_removes_its_own_sockets_and_nothing_else() {
    let shared = SharedPlace::declared();
    let others = shared.others();
    let running = shared.start();
    assert!(shared.own().iter().all(|socket| socket.exists()));
    let status = running.stop();
    println!("daemon stopped by SIGTERM: {status}");
    assert!(status.success());
    assert!(shared.own().iter().all(|socket| !socket.exists()));
    assert!(others.iter().all(|other| other.exists()));
    assert!(shared.place.runtime.is_dir() && shared.place.state.is_dir());
}

#[test]
fn a_daemon_whose_place_is_absent_does_not_start_or_make_it() {
    let directory = tempfile::tempdir().unwrap();
    let place = Place {
        name: PlaceName("main".into()),
        runtime: directory.path().join("runtime"),
        state: directory.path().join("state"),
    };
    std::fs::create_dir(&place.state).unwrap();
    let child = SharedPlace::command(&place)
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert!(!Running::started(child).exit().success());
    assert!(!place.runtime.exists());
}

/// The file energy-nexus e5722529, the proof of concept, wrote as its
/// Memory (provenance in crates/core/tests/old_archive_provenance.md).
const PROOF_OF_CONCEPT_MEMORY: &[u8] =
    include_bytes!("../../core/tests/fixtures/energy-e5722529.rkyv");

#[test]
fn the_proof_of_concept_memory_moved_into_the_place_is_upgraded_and_served() {
    use energy_nexus_core::{Configuring, Memory, Remembering};
    use energy_nexus_wire::{Admission, MemoryPathing, Persisted};
    use std::io::Read;
    let shared = SharedPlace::declared();
    // activation-places.md step 3: memory.rkyv moved to <State>/energy.rkyv
    std::fs::write(shared.place.memory(), PROOF_OF_CONCEPT_MEMORY).unwrap();
    let (running, errors) = shared.launch(Stdio::piped());
    for socket in shared.own() {
        UnixStream::connect(&socket)
            .unwrap_or_else(|error| panic!("{} listens at readiness: {error}", socket.display()));
    }
    assert!(running.stop().success());
    let mut report = String::new();
    errors.unwrap().read_to_string(&mut report).unwrap();
    println!("daemon over the proof of concept memory:\n{report}");
    assert!(report.contains("upgraded from the archive written before the envelope"));
    assert!(report.contains("Codex Low { Mind energyNexus Secondary }"));
    assert!(report.contains("Claude Out { Psyche core Primary }"));
    let bytes = std::fs::read(shared.place.memory()).unwrap();
    assert_eq!(&bytes[..8], b"ENXMEMRY");
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 3);
    match Memory::load(&shared.place.memory()).unwrap() {
        Admission::Admitted(memory) => {
            assert_eq!(memory.history().len(), 3);
            assert_eq!(memory.configuration().redeem_expiring.0, 720);
            assert!(memory.configuration().reminders.is_empty());
        }
        other => panic!("the upgraded memory is current, got {other:?}"),
    }
}

#[test]
fn a_daemon_over_a_foreign_memory_does_not_start_and_leaves_it() {
    use energy_nexus_wire::MemoryPathing;
    let shared = SharedPlace::declared();
    let foreign: Vec<u8> = (0..4096u32).map(|n| (n % 251) as u8).collect();
    std::fs::write(shared.place.memory(), &foreign).unwrap();
    let child = SharedPlace::command(&shared.place)
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert!(!Running::started(child).exit().success());
    assert_eq!(std::fs::read(shared.place.memory()).unwrap(), foreign);
    assert_eq!(std::fs::read_dir(&shared.place.runtime).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&shared.place.state).unwrap().count(), 1);
}
