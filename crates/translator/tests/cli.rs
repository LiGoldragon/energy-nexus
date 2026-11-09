//! The Energy CLIs take the datom and nothing else (vision:nexus-v1): the
//! place is the configuration's, `$XDG_CONFIG_HOME/nexus/energy-nexus.datom`,
//! resolved under `$XDG_RUNTIME_DIR/nexus/` to that place's translator
//! sockets and to no other. A typed place is refused; a missing
//! configuration is a typed refusal naming the file.
use energy_nexus_wire::Framing;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Output};

const CLIENT: &str = env!("CARGO_BIN_EXE_energy-nexus");
const META: &str = env!("CARGO_BIN_EXE_energy-nexus-meta");
const PACE: &str = env!("CARGO_BIN_EXE_energy-pace");

struct World {
    root: tempfile::TempDir,
}
impl World {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for directory in ["run/nexus/main", "run/nexus/next", "config/nexus", "home"] {
            std::fs::create_dir_all(root.path().join(directory)).unwrap();
        }
        Self { root }
    }
    fn path(&self, relative: &str) -> std::path::PathBuf {
        self.root.path().join(relative)
    }
    fn configure(&self, text: &str) {
        std::fs::write(self.path("config/nexus/energy-nexus.datom"), text).unwrap();
    }
    fn run(&self, client: &str, arguments: &[&str]) -> Output {
        Command::new(client)
            .env_clear()
            .env("XDG_RUNTIME_DIR", self.path("run"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("HOME", self.path("home"))
            .args(arguments)
            .output()
            .unwrap()
    }
    /// A translator socket at the place that answers one frame with `reply`
    /// and hands over the text it was sent.
    fn answering(&self, socket: &Path, reply: &'static str) -> std::thread::JoinHandle<String> {
        let listener = UnixListener::bind(socket).unwrap();
        std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let text = String::from_utf8(peer.read_frame().unwrap()).unwrap();
            peer.write_frame(reply.as_bytes()).unwrap();
            text
        })
    }
    /// A socket at a place no configuration names: nothing may reach it.
    fn unreached(&self, socket: &Path) -> UnixListener {
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        listener
    }
}
fn assert_untouched(listener: &UnixListener) {
    assert_eq!(
        listener.accept().expect_err("nothing reached it").kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn each_client_reaches_the_place_its_configuration_names() {
    let world = World::new();
    world.configure("{ next }\n");
    let main_public = world.unreached(&world.path("run/nexus/main/energy-translator.sock"));
    let main_meta = world.unreached(&world.path("run/nexus/main/energy-meta-translator.sock"));
    let public = world.answering(
        &world.path("run/nexus/next/energy-translator.sock"),
        "Refused.ProviderDown.Codex",
    );
    let output = world.run(CLIENT, &["Read.Codex"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(public.join().unwrap(), "Read.Codex");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Refused.ProviderDown.Codex"
    );
    let meta = world.answering(
        &world.path("run/nexus/next/energy-meta-translator.sock"),
        "Configured.RedeemExpiring.720",
    );
    let output = world.run(META, &["Configure.RedeemExpiring.720"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(meta.join().unwrap(), "Configure.RedeemExpiring.720");
    assert_untouched(&main_public);
    assert_untouched(&main_meta);
}

#[test]
fn a_typed_place_is_refused() {
    let world = World::new();
    world.configure("{ main }\n");
    let public = world.unreached(&world.path("run/nexus/main/energy-translator.sock"));
    let meta = world.unreached(&world.path("run/nexus/main/energy-meta-translator.sock"));
    for (client, arguments) in [
        (CLIENT, &["main", "Read.Codex"][..]),
        (META, &["main", "Configure.RedeemExpiring.720"][..]),
        (PACE, &["main"][..]),
        (PACE, &["main", "Codex"][..]),
    ] {
        let refused = world.run(client, arguments);
        assert_eq!(refused.status.code(), Some(2), "{arguments:?}: {refused:?}");
        assert!(
            String::from_utf8_lossy(&refused.stderr).starts_with("usage: "),
            "{arguments:?}: {refused:?}"
        );
    }
    assert_untouched(&public);
    assert_untouched(&meta);
}

#[test]
fn a_missing_configuration_is_refused_naming_its_path() {
    let world = World::new();
    let public = world.unreached(&world.path("run/nexus/main/energy-translator.sock"));
    let path = world.path("config/nexus/energy-nexus.datom");
    for (client, arguments) in [
        (CLIENT, &["Read.Codex"][..]),
        (META, &["Configure.RedeemExpiring.720"][..]),
        (PACE, &[][..]),
    ] {
        let refused = world.run(client, arguments);
        assert_eq!(refused.status.code(), Some(2), "{refused:?}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(
            stderr.starts_with("Missing.") && stderr.contains(&*path.to_string_lossy()),
            "{stderr}"
        );
    }
    // With no XDG_CONFIG_HOME the file is ~/.config/nexus/energy-nexus.datom.
    let refused = Command::new(CLIENT)
        .env_clear()
        .env("XDG_RUNTIME_DIR", world.path("run"))
        .env("HOME", world.path("home"))
        .arg("Read.Codex")
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");
    let unset = world.path("home/.config/nexus/energy-nexus.datom");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.starts_with("Missing.") && stderr.contains(&*unset.to_string_lossy()),
        "{stderr}"
    );
    assert_untouched(&public);
}
