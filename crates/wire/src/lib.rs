use std::io::{Read, Write};
mod memory;
pub use memory::{Admission, Envelope, MemoryRefusal, Persisted, Upgrade};
pub const CAPACITY: usize = 4 * 1024 * 1024;
pub trait Framing {
    fn read_frame(&mut self) -> std::io::Result<Vec<u8>>;
    fn write_frame(&mut self, bytes: &[u8]) -> std::io::Result<()>;
}
impl<T: Read + Write> Framing for T {
    fn read_frame(&mut self) -> std::io::Result<Vec<u8>> {
        let mut head = [0u8; 4];
        self.read_exact(&mut head)?;
        let size = u32::from_le_bytes(head) as usize;
        if size > CAPACITY {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame too large",
            ));
        }
        let mut bytes = vec![0; size];
        self.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    fn write_frame(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if bytes.len() > CAPACITY {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "frame too large",
            ));
        }
        self.write_all(&(bytes.len() as u32).to_le_bytes())?;
        self.write_all(bytes)?;
        self.flush()
    }
}
pub trait SocketPathing {
    fn public(&self) -> std::path::PathBuf;
    fn meta(&self) -> std::path::PathBuf;
    fn emission(&self) -> std::path::PathBuf;
    fn public_translator(&self) -> std::path::PathBuf;
    fn meta_translator(&self) -> std::path::PathBuf;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceName(pub String);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub name: PlaceName,
    pub runtime: std::path::PathBuf,
    pub state: std::path::PathBuf,
}
impl Place {
    pub fn resolve(name: PlaceName) -> Result<Self, String> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .ok_or("XDG_RUNTIME_DIR is unset")?
            .join("nexus")
            .join(&name.0);
        let state_root = std::env::var_os("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::PathBuf::from(home).join(".local/state"))
            })
            .ok_or("HOME and XDG_STATE_HOME are unset")?;
        Ok(Self {
            state: state_root.join("nexus").join(&name.0),
            runtime,
            name,
        })
    }
}
impl SocketPathing for Place {
    fn public(&self) -> std::path::PathBuf {
        self.runtime.join("energy.sock")
    }
    fn meta(&self) -> std::path::PathBuf {
        self.runtime.join("energy-meta.sock")
    }
    fn emission(&self) -> std::path::PathBuf {
        self.runtime.join("energy-emission.sock")
    }
    fn public_translator(&self) -> std::path::PathBuf {
        self.runtime.join("energy-translator.sock")
    }
    fn meta_translator(&self) -> std::path::PathBuf {
        self.runtime.join("energy-meta-translator.sock")
    }
}
/// The files Energy keeps in its place's State, each under Energy's own name:
/// the place is shared with every other Nexus at it.
pub trait MemoryPathing {
    /// `<State>/energy.rkyv`: the daemon's Memory.
    fn memory(&self) -> std::path::PathBuf;
    /// `<State>/energy-translator.rkyv`: the translator's configured programs.
    fn translator_memory(&self) -> std::path::PathBuf;
}
impl MemoryPathing for Place {
    fn memory(&self) -> std::path::PathBuf {
        self.state.join("energy.rkyv")
    }
    fn translator_memory(&self) -> std::path::PathBuf {
        self.state.join("energy-translator.rkyv")
    }
}
/// A place's two directories are declared by the environment and belong to
/// no Nexus: a process at the place finds them, and never makes or removes
/// them.
pub trait Declared {
    /// Both directories exist; otherwise the process does not start.
    fn confirm(&self) -> std::io::Result<()>;
}
impl Declared for Place {
    fn confirm(&self) -> std::io::Result<()> {
        for directory in [&self.runtime, &self.state] {
            if !directory.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "place {}: {} is not a directory; the place is declared by the environment",
                        self.name.0,
                        directory.display()
                    ),
                ));
            }
        }
        Ok(())
    }
}
/// The sockets one process makes at its place, and nothing else there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedSockets(pub Vec<std::path::PathBuf>);
pub trait OwningSockets {
    /// Removes the sockets, each if present.
    fn remove(&self);
    /// Takes SIGTERM and SIGINT from the calling thread and every thread it
    /// starts after, and waits for them on a thread of its own: on either,
    /// the sockets are removed and the process exits 0. Called before any
    /// other thread is started.
    fn remove_at_stop(&self) -> std::io::Result<()>;
}
impl OwningSockets for OwnedSockets {
    fn remove(&self) {
        for socket in &self.0 {
            let _ = std::fs::remove_file(socket);
        }
    }
    fn remove_at_stop(&self) -> std::io::Result<()> {
        use nix::sys::signal::{SigSet, Signal};
        let mut stopping = SigSet::empty();
        stopping.add(Signal::SIGTERM);
        stopping.add(Signal::SIGINT);
        stopping.thread_block().map_err(std::io::Error::from)?;
        let sockets = self.clone();
        // A child started through std::process begins with an empty mask:
        // std resets it before exec.
        std::thread::spawn(move || loop {
            if stopping.wait().is_ok() {
                sockets.remove();
                std::process::exit(0);
            }
        });
        Ok(())
    }
}
/// systemd's readiness protocol: a unit of Type=notify has started once its
/// main process sends `READY=1` to the datagram socket `NOTIFY_SOCKET`
/// names, a path or, after `@`, an abstract name.
pub struct Readiness;
pub trait AnnouncesReadiness {
    /// Sends `READY=1` to `$NOTIFY_SOCKET` when it is set; does nothing when
    /// it is not. Called once every socket the process serves is bound.
    fn announce_ready() -> std::io::Result<()>;
}
impl AnnouncesReadiness for Readiness {
    fn announce_ready() -> std::io::Result<()> {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::net::{SocketAddr, UnixDatagram};
        let Some(target) = std::env::var_os("NOTIFY_SOCKET") else {
            return Ok(());
        };
        let address = match target.as_encoded_bytes().strip_prefix(b"@") {
            Some(name) => SocketAddr::from_abstract_name(name),
            None => SocketAddr::from_pathname(std::path::Path::new(&target)),
        }?;
        UnixDatagram::unbound()?.send_to_addr(b"READY=1", &address)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::{MemoryPathing, OwnedSockets, OwningSockets, Place, PlaceName, SocketPathing};

    fn place(directory: &std::path::Path) -> Place {
        Place {
            name: PlaceName("main".into()),
            runtime: directory.join("runtime"),
            state: directory.join("state"),
        }
    }

    #[test]
    fn energy_files_in_a_shared_place_carry_energy_names() {
        let place = place(std::path::Path::new("/place"));
        assert_eq!(place.memory(), std::path::Path::new("/place/state/energy.rkyv"));
        assert_eq!(
            place.translator_memory(),
            std::path::Path::new("/place/state/energy-translator.rkyv")
        );
        assert_eq!(place.public(), std::path::Path::new("/place/runtime/energy.sock"));
    }

    #[test]
    fn removing_owned_sockets_leaves_the_place_and_every_other_file() {
        let directory = tempfile::tempdir().unwrap();
        let place = place(directory.path());
        std::fs::create_dir(&place.runtime).unwrap();
        let own = OwnedSockets(vec![place.public(), place.meta()]);
        for socket in &own.0 {
            std::fs::write(socket, "").unwrap();
        }
        let others = [place.public_translator(), place.runtime.join("flow.sock")];
        for other in &others {
            std::fs::write(other, "").unwrap();
        }
        own.remove();
        assert!(own.0.iter().all(|socket| !socket.exists()));
        assert!(others.iter().all(|other| other.exists()));
        assert!(place.runtime.is_dir());
    }

    #[test]
    fn an_absent_place_is_refused_and_not_made() {
        use super::Declared;
        let directory = tempfile::tempdir().unwrap();
        let place = place(directory.path());
        assert!(place.confirm().is_err());
        assert!(!place.runtime.exists() && !place.state.exists());
        std::fs::create_dir(&place.runtime).unwrap();
        std::fs::create_dir(&place.state).unwrap();
        assert!(place.confirm().is_ok());
    }
}
