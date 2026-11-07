//! A daemon started over a legacy memory file refuses to start: it binds nothing and leaves the
//! file as it was.

use std::process::{Command, Stdio};

#[test]
fn a_legacy_memory_stops_the_daemon_before_any_socket_and_leaves_the_file() {
    let root = std::env::temp_dir().join(format!("energy-daemon-legacy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (runtime, state) = (root.join("run/nexus/main"), root.join("state/nexus/main"));
    std::fs::create_dir_all(&state).expect("state");
    let legacy: Vec<u8> = (0..4096u32).map(|n| (n % 251) as u8).collect();
    std::fs::write(state.join("memory.rkyv"), &legacy).expect("legacy memory");

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
        std::fs::read(state.join("memory.rkyv")).unwrap(),
        legacy,
        "the legacy bytes are untouched"
    );
    assert!(
        !runtime.exists(),
        "no runtime directory or socket was created"
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
    let _ = std::fs::remove_dir_all(&root);
}
