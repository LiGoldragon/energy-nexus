//! Flow goes and comes back while Energy runs. The daemon (the real binary)
//! and the translator keep running when Flow's subscription closes; the
//! daemon answers from memory and reads nothing while Flow is absent; when
//! Flow's socket appears again the translator subscribes once, and the new
//! opening reads the provider whose reading is older than its last turn
//! end, as the first opening did. A fake Flow plays both Flows, placing its
//! socket as every Nexus at a place does, and a fake Codex answers the
//! readings; every wait is on an event and bounded by EVENT_WAIT.
use energy_nexus_translator::{Programs, Translating, Translator};
use energy_nexus_wire::{Framing, Place, PlaceName, PlacedSocket, PlacesSocket, SocketPathing};
use nix::sys::signal::{kill, Signal as Kill};
use nix::unistd::Pid;
use signal::{ByteViewable, Restorable, Signal, Signalizable};
use signal_energy_nexus::{Call, Caller, Percent, Provider, Query, Response};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
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

/// Flow as signal-flow 61 plays it: the connection greets first, and is
/// answered with the receipt for the contract Flow was built with.
fn greeted(stream: &mut UnixStream) {
    use signal_flow::Contracted;
    let greeting = rkyv::from_bytes::<signal_flow::Handshake, rkyv::rancor::Error>(
        &flow_frame(stream),
    )
    .unwrap();
    assert_eq!(greeting, signal_flow::Query::greeting());
    let receipt =
        rkyv::to_bytes::<rkyv::rancor::Error>(&signal_flow::Query::receipt(&greeting)).unwrap();
    write_flow_frame(stream, &receipt);
}

/// A Codex stand-in: notes each rate-limit read in `log`; the first answers
/// 8% used, every later one 97%, each with a window that resets in an hour.
fn fake_codex(directory: &std::path::Path, log: &std::path::Path) -> std::path::PathBuf {
    let program = directory.join("codex");
    std::fs::write(
        &program,
        format!(
            r#"#!/bin/sh
read a; read b; read c
if [ -e {log} ]; then used=97; else used=8; fi
echo read >> {log}
resets=$(( $(date +%s) + 3600 ))
printf '{{"jsonrpc":"2.0","id":2,"result":{{"rateLimits":{{"primary":{{"usedPercent":%s,"resetsAt":%s,"windowDurationMins":300}}}}}}}}\n' "$used" "$resets"
"#,
            log = log.display()
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

fn ask(place: &Place, input: Query) -> Response {
    let mut question = UnixStream::connect(place.public()).unwrap();
    question.set_read_timeout(Some(EVENT_WAIT)).unwrap();
    question.write_frame(&call(input)).unwrap();
    response(&mut question)
}

/// Flow's socket at the place as every Nexus at it places its own: made
/// under a temporary name, its mode set, listening, then moved to its name.
fn flow_binds(socket: &std::path::Path) -> UnixListener {
    PlacedSocket::at(socket).unwrap().claim().unwrap()
}

/// One subscription Flow takes, with its query.
fn accepted(listener: &UnixListener) -> (UnixStream, signal_flow::Query) {
    let (taken, take) = mpsc::channel();
    let listener = listener.try_clone().unwrap();
    std::thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        greeted(&mut peer);
        let query =
            rkyv::from_bytes::<signal_flow::Query, rkyv::rancor::Error>(&flow_frame(&mut peer))
                .unwrap();
        let _ = taken.send((peer, query));
    });
    take.recv_timeout(EVENT_WAIT)
        .expect("the translator subscribes to Flow's turn ends")
}

/// The opening Flow sends: the last turn end, on Codex, at `at`.
fn opening(at: i64) -> Vec<u8> {
    let end = signal_flow::TurnEnd {
        address: signal_flow::Address {
            aspect: signal_flow::Aspect::Mind,
            topic: signal_flow::Topic("energyNexus".into()),
            model: signal_flow::Model::Sol,
        },
        harness_kind: signal_flow::HarnessKind::Codex,
        flow_id: signal_flow::FlowId("ff3882".into()),
        observed_at: signal_flow::ObservedAt(at),
    };
    rkyv::to_bytes::<rkyv::rancor::Error>(&signal_flow::Response::TurnEndsObserved(vec![end]))
        .unwrap()
        .to_vec()
}

fn reads(log: &std::path::Path) -> usize {
    std::fs::read_to_string(log).map_or(0, |text| text.lines().count())
}

#[test]
fn flow_going_leaves_energy_running_and_its_return_resubscribes_once_and_reads_what_is_stale() {
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
    let socket = place.runtime.join("flow.sock");

    // Flow at the place, then the translator, then the daemon, as their
    // units are ordered.
    let first = flow_binds(&socket);
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

    let mut changes = UnixStream::connect(place.public()).unwrap();
    changes.set_read_timeout(Some(EVENT_WAIT)).unwrap();
    changes.write_frame(&call(Query::Subscribe)).unwrap();
    assert_eq!(response(&mut changes), Response::Subscribed);

    // The first Flow: its opening reads Codex once.
    let (mut peer, query) = accepted(&first);
    assert_eq!(
        query,
        signal_flow::Query::Observe(signal_flow::ObserveSelection::TurnEnds)
    );
    write_flow_frame(&mut peer, &opening(unix_now() - 5));
    let read_first = match response(&mut changes) {
        Response::Changed(changed) => changed.reading,
        other => panic!("the first reading pushes its level, got {other:?}"),
    };
    assert_eq!(read_first.window_vector[0].used, Percent(8));
    assert_eq!(reads(&log), 1);

    // Flow goes: it closes the subscription and removes its socket.
    drop(peer);
    drop(first);
    std::fs::remove_file(&socket).unwrap();

    // Energy runs on: asked, the daemon answers from memory and reads
    // nothing, since no turn end is newer than its reading.
    match ask(&place, Query::Read(Provider::Codex)) {
        Response::Read(read) => assert_eq!(read.reading, read_first),
        other => panic!("answered from memory, got {other:?}"),
    }
    assert_eq!(reads(&log), 1, "no read while Flow is absent");
    assert!(daemon.try_wait().unwrap().is_none(), "the daemon runs");

    // Flow comes back. The translator subscribes on its socket's
    // appearance; the opening names a Codex turn end newer than the
    // reading, which is read again.
    let second = flow_binds(&socket);
    let (mut peer, query) = accepted(&second);
    assert_eq!(
        query,
        signal_flow::Query::Observe(signal_flow::ObserveSelection::TurnEnds)
    );
    write_flow_frame(&mut peer, &opening(unix_now()));
    match response(&mut changes) {
        Response::Changed(changed) => {
            assert_eq!(changed.provider, Provider::Codex);
            assert_eq!(changed.reading.window_vector[0].used, Percent(97));
        }
        other => panic!("the reopening reads the stale provider, got {other:?}"),
    }
    assert_eq!(reads(&log), 2);
    // One subscription on Flow's return.
    second.set_nonblocking(true).unwrap();
    assert_eq!(
        second.accept().map(|_| ()).map_err(|error| error.kind()),
        Err(std::io::ErrorKind::WouldBlock)
    );

    let (exited, exit) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = exited.send(daemon.wait().unwrap());
    });
    kill(pid, Kill::SIGTERM).unwrap();
    assert!(exit.recv_timeout(EVENT_WAIT).unwrap().success());
}
