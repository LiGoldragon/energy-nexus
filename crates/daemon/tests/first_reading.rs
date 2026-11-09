//! The first reading at start is not lost. The translator listens before the
//! daemon starts (its unit is ordered first); the daemon, once ready, opens
//! Flow's turn ends through it, and the opening, the last turn end on each
//! provider, is the event of its first reading. A fake Flow answers the
//! subscription and a fake Codex the reading; every wait is on an event and
//! bounded by EVENT_WAIT.
use energy_nexus_translator::{Programs, Translating, Translator};
use energy_nexus_wire::{Framing, Place, PlaceName, SocketPathing};
use nix::sys::signal::{kill, Signal as Kill};
use nix::unistd::Pid;
use signal::{ByteViewable, Restorable, Signal, Signalizable};
use signal_energy_nexus::{Call, Caller, Provider, Query, Response};
use std::io::{Read, Write};
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const DAEMON: &str = env!("CARGO_BIN_EXE_energy-nexus-daemon");
const EVENT_WAIT: Duration = Duration::from_secs(60);

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn flow_frame(stream: &mut UnixStream) -> Vec<u8> {
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).unwrap();
    let mut bytes = vec![0; u32::from_be_bytes(head) as usize];
    stream.read_exact(&mut bytes).unwrap();
    bytes
}

fn write_flow_frame(stream: &mut UnixStream, bytes: &[u8]) {
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(bytes).unwrap();
}

/// A Codex stand-in: notes each rate-limit read in `log`, then answers one
/// window that resets in an hour.
fn fake_codex(directory: &std::path::Path, log: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let program = directory.join("codex");
    std::fs::write(
        &program,
        format!(
            r#"#!/bin/sh
read a; read b; read c
echo read >> {}
resets=$(( $(date +%s) + 3600 ))
printf '{{"jsonrpc":"2.0","id":2,"result":{{"rateLimits":{{"primary":{{"usedPercent":8,"resetsAt":%s,"windowDurationMins":300}}}}}}}}\n' "$resets"
"#,
            log.display()
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&program).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&program, permissions).unwrap();
    program
}

fn call(input: Query) -> Vec<u8> {
    Call {
        caller: Caller {
            process_id: std::process::id() as i64,
            user_id: 0,
        },
        input,
    }
    .signalize()
    .unwrap()
    .bytes()
    .to_vec()
}

fn response(stream: &mut UnixStream) -> Response {
    Signal::<Response>::from(stream.read_frame().unwrap())
        .restore()
        .unwrap()
}

#[test]
fn the_first_reading_is_made_on_the_turn_ends_opening_and_not_lost() {
    let directory = tempfile::tempdir().unwrap();
    let place = Place {
        name: PlaceName("main".into()),
        runtime: directory.path().join("runtime"),
        state: directory.path().join("state"),
    };
    std::fs::create_dir(&place.runtime).unwrap();
    std::fs::create_dir(&place.state).unwrap();
    let log = directory.path().join("codex.log");
    let codex = fake_codex(directory.path(), &log);

    // Flow at the place: it takes Observe.TurnEnds, says so, and sends its
    // opening when the test lets it.
    let flow = UnixListener::bind(place.runtime.join("flow.sock")).unwrap();
    let (subscribed, subscription) = mpsc::channel();
    let (open, opening) = mpsc::channel::<()>();
    let turn_ended_at = unix_now() - 5;
    std::thread::spawn(move || {
        let (mut peer, _) = flow.accept().unwrap();
        let query =
            rkyv::from_bytes::<signal_flow::Query, rkyv::rancor::Error>(&flow_frame(&mut peer))
                .unwrap();
        let _ = subscribed.send(query);
        opening.recv().unwrap();
        let codex_end = signal_flow::TurnEnd {
            address: signal_flow::Address {
                aspect: signal_flow::Aspect::Mind,
                topic: "energyNexus".into(),
                model: signal_flow::Model::Sol,
            },
            harness_kind: signal_flow::HarnessKind::Codex,
            flow_id: "ff3882".into(),
            observed_at: turn_ended_at,
        };
        let frame = rkyv::to_bytes::<rkyv::rancor::Error>(
            &signal_flow::Response::TurnEndsObserved(vec![codex_end]),
        )
        .unwrap();
        write_flow_frame(&mut peer, &frame);
        let mut rest = Vec::new();
        let _ = peer.read_to_end(&mut rest);
    });

    // The translator listens first, as its unit is ordered before the
    // daemon's.
    let translator = Translator {
        place: place.clone(),
        programs: Arc::new(Mutex::new(Programs {
            codex: Some(codex.to_str().unwrap().into()),
            ..Programs::default()
        })),
    };
    let listening = translator.bind().unwrap();
    std::thread::spawn(move || listening.run());

    let notify_directory = tempfile::tempdir().unwrap();
    let notify_path = notify_directory.path().join("notify.sock");
    let notify = UnixDatagram::bind(&notify_path).unwrap();
    notify.set_read_timeout(Some(EVENT_WAIT)).unwrap();
    let mut daemon = Command::new(DAEMON)
        .arg(&place.name.0)
        .arg(&place.runtime)
        .arg(&place.state)
        .env("NOTIFY_SOCKET", &notify_path)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let pid = Pid::from_raw(daemon.id() as i32);
    let mut message = [0u8; 64];
    let length = notify.recv(&mut message).unwrap();
    assert_eq!(&message[..length], b"READY=1");

    // The daemon, ready, opened Flow's turn ends through the translator.
    let query = subscription
        .recv_timeout(EVENT_WAIT)
        .expect("the daemon subscribes to Flow's turn ends at start");
    assert_eq!(
        query,
        signal_flow::Query::Observe(signal_flow::ObserveSelection::TurnEnds)
    );
    assert!(!log.exists(), "nothing is read before an event");

    // Watch the daemon's level changes, then let Flow open.
    let mut changes = UnixStream::connect(place.public()).unwrap();
    changes.set_read_timeout(Some(EVENT_WAIT)).unwrap();
    changes.write_frame(&call(Query::Subscribe)).unwrap();
    assert_eq!(response(&mut changes), Response::Subscribed);
    open.send(()).unwrap();
    match response(&mut changes) {
        Response::Changed(changed) => {
            assert_eq!(changed.provider, Provider::Codex);
            assert!(changed.reading.at.0 > turn_ended_at);
        }
        other => panic!("the first reading pushes its level, got {other:?}"),
    }
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "read\n");

    // Asked now, the reading is newer than the last turn end: answered from
    // memory, with no second read.
    let mut question = UnixStream::connect(place.public()).unwrap();
    question.set_read_timeout(Some(EVENT_WAIT)).unwrap();
    question.write_frame(&call(Query::Read(Provider::Codex))).unwrap();
    assert!(matches!(response(&mut question), Response::Read(_)));
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "read\n");

    let (exited, exit) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = exited.send(daemon.wait().unwrap());
    });
    kill(pid, Kill::SIGTERM).unwrap();
    assert!(exit.recv_timeout(EVENT_WAIT).unwrap().success());
}
