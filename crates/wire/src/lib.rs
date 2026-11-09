use std::io::{Read, Write};
mod memory;
pub use memory::{Admission, Earlier, Envelope, MemoryRefusal, Persisted, Upgrade};
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
/// A socket placed at its name in the place's runtime directory only once it
/// listens, as every Nexus at a place places its sockets: bound under a
/// temporary name beside its name, `.<name>.<pid>`, its mode set to `0600`,
/// made to listen, then renamed to its name (rename(2) is atomic). A peer
/// waiting for the socket waits on its name moved in (`IN_MOVED_TO`) and
/// connects at once.
pub struct PlacedSocket {
    name: std::path::PathBuf,
    temporary: std::path::PathBuf,
}
/// What stands at a socket's name before the socket is placed there.
pub enum AtSocketName {
    /// Nothing: the rename replaces nothing, so a socket another process
    /// placed there since is never replaced.
    Absent,
    /// A socket nobody accepts on, left by a process that died: the rename
    /// replaces it, with no moment at which the name is absent.
    Stale,
}
pub trait PlacesSocket: Sized {
    /// The socket to be placed at `name`.
    fn at(name: &std::path::Path) -> std::io::Result<Self>;
    /// The temporary name the socket is made under.
    fn temporary(&self) -> &std::path::Path;
    /// What stands at the name. A socket that accepts a connection belongs
    /// to a live process and is refused, as is a file that is no socket.
    fn found(&self) -> std::io::Result<AtSocketName>;
    /// A socket bound at the temporary name, its mode `0600`, listening. A
    /// leftover at this process's own temporary name is a dead process's
    /// that had the same id, and is removed first.
    fn listening(&self) -> std::io::Result<std::os::unix::net::UnixListener>;
    /// Renames the temporary name to the name, over a stale socket or into
    /// an absent name.
    fn place(&self, found: AtSocketName) -> std::io::Result<()>;
    /// The socket listening at its name; on any refusal nothing of it is
    /// left at either name.
    fn claim(&self) -> std::io::Result<std::os::unix::net::UnixListener> {
        let found = self.found()?;
        let listener = self.listening()?;
        if let Err(error) = self.place(found) {
            drop(listener);
            let _ = std::fs::remove_file(self.temporary());
            return Err(error);
        }
        Ok(listener)
    }
}
impl PlacesSocket for PlacedSocket {
    fn at(name: &std::path::Path) -> std::io::Result<Self> {
        let unplaced = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("socket path {} names no file in a directory", name.display()),
            )
        };
        let directory = name.parent().ok_or_else(unplaced)?;
        let file_name = name.file_name().ok_or_else(unplaced)?;
        let mut temporary = std::ffi::OsString::from(".");
        temporary.push(file_name);
        temporary.push(format!(".{}", std::process::id()));
        Ok(Self {
            name: name.to_owned(),
            temporary: directory.join(temporary),
        })
    }
    fn temporary(&self) -> &std::path::Path {
        &self.temporary
    }
    fn found(&self) -> std::io::Result<AtSocketName> {
        use std::io::{Error, ErrorKind};
        use std::os::unix::fs::FileTypeExt;
        let name = &self.name;
        match std::fs::symlink_metadata(name) {
            Ok(metadata) if metadata.file_type().is_socket() => {
                match std::os::unix::net::UnixStream::connect(name) {
                    Ok(_peer) => Err(Error::new(
                        ErrorKind::AddrInUse,
                        format!("socket {} is already accepting connections", name.display()),
                    )),
                    Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
                        Ok(AtSocketName::Stale)
                    }
                    Err(error) if error.kind() == ErrorKind::NotFound => Ok(AtSocketName::Absent),
                    Err(error) => Err(error),
                }
            }
            Ok(_) => Err(Error::new(
                ErrorKind::AlreadyExists,
                format!("socket path {} exists and is not a socket", name.display()),
            )),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(AtSocketName::Absent),
            Err(error) => Err(error),
        }
    }
    fn listening(&self) -> std::io::Result<std::os::unix::net::UnixListener> {
        use nix::sys::socket::{
            bind, listen, socket, AddressFamily, Backlog, SockFlag, SockType, UnixAddr,
        };
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::PermissionsExt;
        let temporary = &self.temporary;
        match std::fs::remove_file(temporary) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let descriptor = socket(
            AddressFamily::Unix,
            SockType::Stream,
            SockFlag::SOCK_CLOEXEC,
            None,
        )?;
        bind(descriptor.as_raw_fd(), &UnixAddr::new(temporary)?)?;
        let made = std::fs::set_permissions(temporary, std::fs::Permissions::from_mode(0o600))
            .and_then(|()| listen(&descriptor, Backlog::MAXCONN).map_err(std::io::Error::from));
        if let Err(error) = made {
            let _ = std::fs::remove_file(temporary);
            return Err(error);
        }
        Ok(std::os::unix::net::UnixListener::from(descriptor))
    }
    fn place(&self, found: AtSocketName) -> std::io::Result<()> {
        use nix::fcntl::{renameat2, RenameFlags, AT_FDCWD};
        let flags = match found {
            AtSocketName::Absent => RenameFlags::RENAME_NOREPLACE,
            AtSocketName::Stale => RenameFlags::empty(),
        };
        renameat2(AT_FDCWD, &self.temporary, AT_FDCWD, &self.name, flags)
            .map_err(std::io::Error::from)
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

    /// A socket bound at `path` that never listened, then closed: a
    /// connection to it is refused.
    fn stale(path: &std::path::Path) {
        use nix::sys::socket::{bind, socket, AddressFamily, SockFlag, SockType, UnixAddr};
        use std::os::fd::AsRawFd;
        let descriptor =
            socket(AddressFamily::Unix, SockType::Stream, SockFlag::SOCK_CLOEXEC, None).unwrap();
        bind(descriptor.as_raw_fd(), &UnixAddr::new(path).unwrap()).unwrap();
    }

    fn names(directory: &std::path::Path) -> Vec<String> {
        let mut names = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn a_socket_appears_at_its_name_listening_and_owner_only() {
        use super::{PlacedSocket, PlacesSocket};
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        let runtime = tempfile::tempdir().unwrap();
        let name = runtime.path().join("energy.sock");
        let placed = PlacedSocket::at(&name).unwrap();
        assert_eq!(
            placed.temporary(),
            runtime.path().join(format!(".energy.sock.{}", std::process::id()))
        );
        let _listener = placed.claim().unwrap();
        let metadata = std::fs::symlink_metadata(&name).unwrap();
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        std::os::unix::net::UnixStream::connect(&name).expect("the name listens");
        assert_eq!(names(runtime.path()), ["energy.sock"]);
    }

    #[test]
    fn a_stale_socket_is_replaced_by_the_rename_and_a_live_one_refused() {
        use super::{PlacedSocket, PlacesSocket};
        use std::os::unix::fs::MetadataExt;
        let runtime = tempfile::tempdir().unwrap();
        let name = runtime.path().join("energy.sock");
        stale(&name);
        let before = std::fs::symlink_metadata(&name).unwrap().ino();
        let listener = PlacedSocket::at(&name).unwrap().claim().unwrap();
        assert_ne!(std::fs::symlink_metadata(&name).unwrap().ino(), before);
        std::os::unix::net::UnixStream::connect(&name).expect("the replacement listens");
        assert!(
            PlacedSocket::at(&name).unwrap().claim().is_err(),
            "a live socket is refused"
        );
        std::os::unix::net::UnixStream::connect(&name).expect("the live socket still listens");
        drop(listener);
        std::fs::write(runtime.path().join("other"), "not a socket").unwrap();
        assert!(PlacedSocket::at(&runtime.path().join("other"))
            .unwrap()
            .claim()
            .is_err());
        assert_eq!(names(runtime.path()), ["energy.sock", "other"]);
    }

    /// A crash between bind and rename leaves a temporary name. One of this
    /// process's own id is a dead process's and is replaced; another's is
    /// left as it is, and neither is the socket.
    #[test]
    fn a_leftover_temporary_name_is_never_the_socket() {
        use super::{PlacedSocket, PlacesSocket};
        let runtime = tempfile::tempdir().unwrap();
        let name = runtime.path().join("energy.sock");
        let placed = PlacedSocket::at(&name).unwrap();
        stale(placed.temporary());
        let foreign = runtime.path().join(".energy.sock.1");
        stale(&foreign);
        let _listener = placed.claim().unwrap();
        std::os::unix::net::UnixStream::connect(&name).expect("the name listens");
        assert!(std::os::unix::net::UnixStream::connect(&foreign).is_err());
        assert_eq!(names(runtime.path()), [".energy.sock.1", "energy.sock"]);
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
