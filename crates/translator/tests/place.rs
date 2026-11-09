//! The translator at a place shared with other Nexuses: it finds the place's
//! directories, makes only its own sockets in them, and removes only those
//! sockets when it is stopped.
use energy_nexus_wire::{Place, PlaceName, SocketPathing};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::process::{Command, ExitStatus, Stdio};

const TRANSLATOR: &str = env!("CARGO_BIN_EXE_energy-translator");
/// Written into Runtime by the test when the translator exits, so one that
/// ends before its sockets exist ends the wait for them.
const EXITED: &str = "translator-exited";

struct SharedPlace {
    _directory: tempfile::TempDir,
    place: Place,
}
struct Running {
    pid: Pid,
    waiter: std::thread::JoinHandle<ExitStatus>,
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
    fn command(&self) -> Command {
        let mut command = Command::new(TRANSLATOR);
        command
            .arg(&self.place.name.0)
            .arg(&self.place.runtime)
            .arg(&self.place.state)
            .stdin(Stdio::null());
        command
    }
    /// Starts the translator and returns once its three sockets are made: the
    /// wait reads creation events in Runtime.
    fn start(&self) -> Running {
        let events = Inotify::init(InitFlags::empty()).unwrap();
        events
            .add_watch(&self.place.runtime, AddWatchFlags::IN_CREATE)
            .unwrap();
        let mut child = self.command().spawn().unwrap();
        let pid = Pid::from_raw(child.id() as i32);
        let exited = self.place.runtime.join(EXITED);
        let waiter = std::thread::spawn(move || {
            let status = child.wait().unwrap();
            std::fs::write(exited, "").unwrap();
            status
        });
        let mut made: Vec<std::ffi::OsString> = Vec::new();
        let own: Vec<std::ffi::OsString> = self
            .own()
            .iter()
            .map(|socket| socket.file_name().unwrap().to_owned())
            .collect();
        while !own.iter().all(|name| made.contains(name)) {
            for event in events.read_events().unwrap() {
                let name = event.name.unwrap();
                assert!(
                    name != EXITED,
                    "the translator ended before its sockets were made: {:?}",
                    waiter.join()
                );
                made.push(name);
            }
        }
        Running { pid, waiter }
    }
}
impl Running {
    fn stop(self) -> ExitStatus {
        kill(self.pid, Signal::SIGTERM).unwrap();
        self.waiter.join().unwrap()
    }
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
    let status = Command::new(TRANSLATOR)
        .arg(&place.name.0)
        .arg(&place.runtime)
        .arg(&place.state)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    assert!(!place.runtime.exists());
}
