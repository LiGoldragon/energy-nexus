//! The translator at a place shared with other Nexuses: it finds the place's
//! directories, makes only its own sockets in them, announces readiness
//! once they listen, and removes only those sockets when it is stopped.
//! Every wait here is on an event, READY=1 or the child's exit, and each is
//! bounded by EVENT_WAIT: a translator that never announces or never exits fails
//! the test there.
use energy_nexus_wire::{Place, PlaceName, SocketPathing};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const TRANSLATOR: &str = env!("CARGO_BIN_EXE_energy-translator");
/// How long a test waits for any one event before it fails.
const EVENT_WAIT: Duration = Duration::from_secs(60);

struct SharedPlace {
    _directory: tempfile::TempDir,
    place: Place,
}
/// A started translator: its pid and its exit, handed over by the thread that
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
    /// The exit, or the translator killed and the test failed after EVENT_WAIT.
    fn exit(self) -> ExitStatus {
        match self.exit.recv_timeout(EVENT_WAIT) {
            Ok(status) => status,
            Err(error) => {
                let _ = kill(self.pid, Signal::SIGKILL);
                panic!("the translator did not exit within {EVENT_WAIT:?}: {error}")
            }
        }
    }
    /// Stops the translator as systemd does, and waits for its exit.
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
        vec![
            self.place.public_translator(),
            self.place.meta_translator(),
            self.place.emission(),
        ]
    }
    /// Files of other processes and Nexuses at the same place.
    fn others(&self) -> Vec<std::path::PathBuf> {
        let others = vec![
            self.place.public(),
            self.place.runtime.join("flow.sock"),
            self.place.state.join("flow.sema"),
        ];
        for other in &others {
            std::fs::write(other, "another's").unwrap();
        }
        others
    }
    fn command(place: &Place) -> Command {
        let mut command = Command::new(TRANSLATOR);
        command
            .arg(&place.name.0)
            .arg(&place.runtime)
            .arg(&place.state)
            .stdin(Stdio::null());
        command
    }
    /// Starts the translator with the notify socket systemd would give it, and
    /// returns once it announced READY=1.
    fn start(&self) -> Running {
        let notify_directory = tempfile::tempdir().unwrap();
        let notify_path = notify_directory.path().join("notify.sock");
        let notify = UnixDatagram::bind(&notify_path).unwrap();
        notify.set_read_timeout(Some(EVENT_WAIT)).unwrap();
        let child = Self::command(&self.place)
            .env("NOTIFY_SOCKET", &notify_path)
            .spawn()
            .unwrap();
        let running = Running::started(child);
        let mut message = [0_u8; 64];
        match notify.recv(&mut message) {
            Ok(length) => assert_eq!(&message[..length], b"READY=1"),
            Err(error) => {
                let _ = kill(running.pid, Signal::SIGKILL);
                panic!("the translator announced no readiness: {error}");
            }
        }
        running
    }
}

#[test]
fn a_translator_announces_readiness_once_its_sockets_listen() {
    let shared = SharedPlace::declared();
    let running = shared.start();
    for socket in shared.own() {
        UnixStream::connect(&socket)
            .unwrap_or_else(|error| panic!("{} listens at readiness: {error}", socket.display()));
    }
    assert!(running.stop().success());
}

#[test]
fn a_stopped_translator_removes_its_own_sockets_and_nothing_else() {
    let shared = SharedPlace::declared();
    let others = shared.others();
    let running = shared.start();
    assert!(shared.own().iter().all(|socket| socket.exists()));
    let status = running.stop();
    println!("translator stopped by SIGTERM: {status}");
    assert!(status.success());
    assert!(shared.own().iter().all(|socket| !socket.exists()));
    assert!(others.iter().all(|other| other.exists()));
    assert!(shared.place.runtime.is_dir() && shared.place.state.is_dir());
}

#[test]
fn a_translator_whose_place_is_absent_does_not_start_or_make_it() {
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

/// The translator memory energy-nexus e5722529, the proof of concept,
/// wrote: its three configured paths, a bare rkyv archive (provenance in
/// crates/core/tests/old_archive_provenance.md).
const PROOF_OF_CONCEPT_MEMORY: &[u8] =
    include_bytes!("fixtures/energy-translator-e5722529.rkyv");

#[test]
fn the_proof_of_concept_memory_moved_into_the_place_is_upgraded_and_configured() {
    use energy_nexus_translator::Programs;
    use energy_nexus_wire::{Admission, Framing, MemoryPathing, Persisted};
    let shared = SharedPlace::declared();
    // activation-places.md step 3: translator.rkyv moved to <State>/energy-translator.rkyv
    let path = shared.place.translator_memory();
    std::fs::write(&path, PROOF_OF_CONCEPT_MEMORY).unwrap();
    let running = shared.start();
    let read = || match Programs::load(&path).unwrap() {
        Admission::Admitted(programs) => programs,
        other => panic!("the translator memory is current, got {other:?}"),
    };
    assert_eq!(&std::fs::read(&path).unwrap()[..8], b"ENXTRNSL");
    assert_eq!(
        read(),
        Programs {
            codex: Some(
                "/nix/store/1q41fx3vb1hqzdgkr0nk5b0pc4jvyhl1-codex-0.166.0/bin/codex".into()
            ),
            flow: Some("/home/li/.local/bin/flow-0.51".into()),
            credentials: Some("/home/li/.claude/.credentials.json".into()),
        }
    );
    // activation-places.md step 6, as energy-nexus-meta sends it
    use datom_codec::Datomizable;
    use protos::{Protosizable, Textualizable};
    let set = meta_signal_energy_nexus::Set::Program(meta_signal_energy_nexus::ProgramPath {
        program: meta_signal_energy_nexus::Program::Flow,
        path: meta_signal_energy_nexus::Path("/home/li/.local/bin/flow".into()),
    });
    let text = meta_signal_energy_nexus::Query::Configure(set.clone())
        .datomize(Vec::new())
        .protosize()
        .textualize();
    println!("step 6 text: {text}");
    let mut meta = UnixStream::connect(shared.place.meta_translator()).unwrap();
    meta.write_frame(text.as_bytes()).unwrap();
    let answer = String::from_utf8(meta.read_frame().unwrap()).unwrap();
    let configured = meta_signal_energy_nexus::Response::Configured(set)
        .datomize(Vec::new())
        .protosize()
        .textualize();
    assert_eq!(answer, configured);
    assert_eq!(read().flow.as_deref(), Some("/home/li/.local/bin/flow"));
    assert!(running.stop().success());
}
