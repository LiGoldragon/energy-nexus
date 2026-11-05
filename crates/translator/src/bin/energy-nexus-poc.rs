use std::process::{Command, Stdio};

fn main() {
    let mut arguments = std::env::args().skip(1);
    let runtime = arguments
        .next()
        .map(std::path::PathBuf::from)
        .expect("usage: energy-nexus-poc RUNTIME STATE");
    let state = arguments
        .next()
        .map(std::path::PathBuf::from)
        .expect("usage: energy-nexus-poc RUNTIME STATE");
    std::fs::create_dir_all(&runtime).expect("create runtime directory");
    std::fs::create_dir_all(&state).expect("create state directory");
    std::fs::write(state.join("deployment"), "Energy Nexus proof of concept\n")
        .expect("write deployment record");
    let directory = std::env::current_exe()
        .expect("current executable")
        .parent()
        .expect("binary directory")
        .to_owned();
    let mut translator = Command::new(directory.join("energy-translator"))
        .arg(&runtime)
        .stdin(Stdio::null())
        .spawn()
        .expect("start translator");
    let mut daemon = Command::new(directory.join("energy-nexus-daemon"))
        .arg(&runtime)
        .stdin(Stdio::null())
        .spawn()
        .expect("start daemon");
    let status = daemon.wait().expect("wait for daemon");
    let _ = translator.kill();
    std::process::exit(status.code().unwrap_or(1));
}
