use std::process::{Command, Stdio};

fn main() {
    let mut arguments = std::env::args().skip(1);
    let name = arguments
        .next()
        .expect("usage: energy-nexus-poc NAME RUNTIME STATE");
    let runtime = arguments
        .next()
        .map(std::path::PathBuf::from)
        .expect("usage: energy-nexus-poc NAME RUNTIME STATE");
    let state = arguments
        .next()
        .map(std::path::PathBuf::from)
        .expect("usage: energy-nexus-poc NAME RUNTIME STATE");
    let directory = std::env::current_exe()
        .expect("current executable")
        .parent()
        .expect("binary directory")
        .to_owned();
    let mut translator = Command::new(directory.join("energy-translator"))
        .arg(&name)
        .arg(&runtime)
        .arg(&state)
        .stdin(Stdio::null())
        .spawn()
        .expect("start translator");
    let mut daemon = Command::new(directory.join("energy-nexus-daemon"))
        .arg(&name)
        .arg(&runtime)
        .arg(&state)
        .stdin(Stdio::null())
        .spawn()
        .expect("start daemon");
    let status = daemon.wait().expect("wait for daemon");
    // SIGTERM, so the translator removes its own sockets as it stops.
    let _ = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(translator.id() as i32),
        nix::sys::signal::Signal::SIGTERM,
    );
    let _ = translator.wait();
    std::process::exit(status.code().unwrap_or(1));
}
