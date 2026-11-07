//! A daemon started over a legacy memory file refuses to start: it binds nothing and leaves the
//! file as it was.

use std::process::{Command, Stdio};

const OLD_SECONDARY: &str = "003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff010000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000";

#[test]
fn a_legacy_memory_stops_the_daemon_before_any_socket_and_leaves_the_file() {
    let root = std::env::temp_dir().join(format!("energy-daemon-legacy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (runtime, state) = (root.join("run/nexus/main"), root.join("state/nexus/main"));
    std::fs::create_dir_all(&state).expect("state");
    std::fs::create_dir_all(&runtime).expect("runtime");
    // Two inputs: the archive the previous release wrote with the Secondary layer (provenance in
    // crates/core/tests/old_archive_provenance.md), and 4096 synthetic bytes that are no archive.
    let old: Vec<u8> = (0..OLD_SECONDARY.len() / 2)
        .map(|index| u8::from_str_radix(&OLD_SECONDARY[2 * index..2 * index + 2], 16).unwrap())
        .collect();
    let synthetic: Vec<u8> = (0..4096u32).map(|n| (n % 251) as u8).collect();
    for legacy in [old, synthetic] {
        std::fs::write(state.join("energy.rkyv"), &legacy).expect("legacy memory");

        let status = Command::new(env!("CARGO_BIN_EXE_energy-nexus-daemon"))
            .arg("main")
            .arg(&runtime)
            .arg(&state)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run the daemon");
        assert!(!status.success(), "the daemon refuses to start");
        assert_eq!(
            std::fs::read(state.join("energy.rkyv")).unwrap(),
            legacy,
            "the legacy bytes are untouched"
        );
        assert_eq!(
            std::fs::read_dir(&runtime).unwrap().count(),
            0,
            "no socket was created"
        );
        let names: Vec<_> = std::fs::read_dir(&state)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            names.len(),
            1,
            "no temporary file appeared beside the memory"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
