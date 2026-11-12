//! Energy's memory files on disk. Each is an envelope, then one rkyv
//! archive: eight bytes naming the kind of file, the format version (u32,
//! little-endian) and the archive's length (u64, little-endian). A file
//! without the envelope was written before the envelope existed, and an
//! envelope of an earlier version holds an earlier layout; its owner reads
//! either in that layout and upgrades it on open.
use std::path::{Path, PathBuf};

/// The envelope of one kind of memory file: the bytes that open it and the
/// format version this build writes and reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub magic: [u8; 8],
    pub version: u32,
}
/// Magic, version, archive length.
const HEADER: usize = 8 + 4 + 8;

/// Why a memory file was not read or not written. A refusal never changes
/// the file and never stands in for a memory created new.
#[derive(Debug)]
pub enum MemoryRefusal {
    /// The entry exists and could not be read: a directory, a dangling
    /// link, a file without permission.
    Read { path: PathBuf, source: std::io::Error },
    /// Empty, shorter than its envelope, or an archive whose length
    /// differs from the length the envelope declares.
    Truncated { path: PathBuf, found: u64 },
    /// No envelope, and not an archive of the layout written before it.
    Unrecognized { path: PathBuf, bytes: u64 },
    /// The envelope names a format version this build does not read.
    UnsupportedVersion {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
    /// The archive, or a rule its content must keep, is invalid.
    Payload { path: PathBuf, reason: String },
    /// The memory could not be written.
    Write { path: PathBuf, source: std::io::Error },
}
impl std::fmt::Display for MemoryRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "memory {} could not be read: {source}", path.display())
            }
            Self::Truncated { path, found } => write!(
                formatter,
                "memory {} is empty, truncated or longer than its envelope declares ({found} bytes)",
                path.display()
            ),
            Self::Unrecognized { path, bytes } => write!(
                formatter,
                "memory {} ({bytes} bytes) has no envelope and is no archive of the layout \
                 written before the envelope; it is left as it is",
                path.display()
            ),
            Self::UnsupportedVersion {
                path,
                found,
                expected,
            } => write!(
                formatter,
                "memory {} has format version {found}; this build reads version {expected}",
                path.display()
            ),
            Self::Payload { path, reason } => {
                write!(formatter, "memory {} is invalid: {reason}", path.display())
            }
            Self::Write { path, source } => {
                write!(formatter, "memory {} could not be written: {source}", path.display())
            }
        }
    }
}
impl std::error::Error for MemoryRefusal {}

/// The earlier layout an archive was read in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Earlier {
    /// Written before the envelope existed.
    Unenveloped,
    /// An envelope of this earlier format version.
    Version(u32),
}
impl std::fmt::Display for Earlier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unenveloped => write!(formatter, "the archive written before the envelope"),
            Self::Version(version) => write!(formatter, "format version {version}"),
        }
    }
}
/// What a memory path held.
#[derive(Debug)]
pub enum Admission<M: Persisted> {
    /// No entry: the only case that makes a memory new, from the defaults.
    Absent,
    /// A current envelope around a valid archive.
    Admitted(M),
    /// An archive of an earlier layout, read in that layout and carried
    /// into this one.
    Upgraded(Earlier, Upgrade<M>),
}
/// The memory an earlier archive upgrades to, and what of it has no place
/// in this layout.
#[derive(Debug)]
pub struct Upgrade<M: Persisted> {
    pub memory: M,
    pub uncarried: Vec<M::Uncarried>,
}
impl<M: Persisted + Default> From<Admission<M>> for Upgrade<M> {
    /// The memory in use after opening: the defaults when absent, nothing
    /// uncarried unless upgraded.
    fn from(admission: Admission<M>) -> Self {
        match admission {
            Admission::Absent => Self {
                memory: M::default(),
                uncarried: Vec::new(),
            },
            Admission::Admitted(memory) => Self {
                memory,
                uncarried: Vec::new(),
            },
            Admission::Upgraded(_, upgrade) => upgrade,
        }
    }
}

/// A memory kept in one file under an envelope. The kind supplies its
/// envelope, its archive in the current and every earlier layout and its
/// rules; reading, the upgrade on open and the write are the defaults here.
pub trait Persisted: Sized {
    const ENVELOPE: Envelope;
    /// The earlier envelope versions this kind upgrades on open.
    const EARLIER_VERSIONS: &'static [u32] = &[];
    /// What an earlier archive held that this layout has no place for.
    type Uncarried: std::fmt::Display + std::fmt::Debug;
    /// A current archive; the bytes are aligned to 16.
    fn from_archive(archive: &[u8]) -> Result<Self, String>;
    /// An archive of an earlier layout: the one written before the envelope,
    /// or one of `EARLIER_VERSIONS`; the bytes are aligned to 16.
    fn upgrade(earlier: Earlier, archive: &[u8]) -> Result<Upgrade<Self>, String>;
    fn to_archive(&self) -> Result<rkyv::util::AlignedVec, String>;
    /// The rules the content keeps, checked on every read and before every
    /// write, so a file this build writes is one it reads.
    fn validate(&self) -> Result<(), String>;

    /// Reads the path and changes nothing.
    fn admit(path: &Path) -> Result<Admission<Self>, MemoryRefusal> {
        let bytes = match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Admission::Absent)
            }
            Err(source) => Err(source),
            Ok(_) => std::fs::read(path),
        }
        .map_err(|source| MemoryRefusal::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::admit_bytes(path, &bytes)
    }
    /// Validates exactly these already-read bytes, preserving their identity.
    fn admit_bytes(path: &Path, bytes: &[u8]) -> Result<Admission<Self>, MemoryRefusal> {
        let truncated = |found: usize| MemoryRefusal::Truncated {
            path: path.to_owned(),
            found: found as u64,
        };
        let payload = |reason: String| MemoryRefusal::Payload {
            path: path.to_owned(),
            reason,
        };
        let magic = &Self::ENVELOPE.magic;
        // Empty, or a proper prefix of the magic: a cut-off envelope.
        if bytes.len() < magic.len() && magic.starts_with(&bytes) {
            return Err(truncated(bytes.len()));
        }
        let mut aligned = rkyv::util::AlignedVec::<16>::new();
        if !bytes.starts_with(magic) {
            aligned.extend_from_slice(&bytes);
            let upgrade = Self::upgrade(Earlier::Unenveloped, &aligned).map_err(|_| {
                MemoryRefusal::Unrecognized {
                    path: path.to_owned(),
                    bytes: bytes.len() as u64,
                }
            })?;
            upgrade.memory.validate().map_err(payload)?;
            return Ok(Admission::Upgraded(Earlier::Unenveloped, upgrade));
        }
        if bytes.len() < HEADER {
            return Err(truncated(bytes.len()));
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().expect("four bytes"));
        let current = version == Self::ENVELOPE.version;
        if !current && !Self::EARLIER_VERSIONS.contains(&version) {
            return Err(MemoryRefusal::UnsupportedVersion {
                path: path.to_owned(),
                found: version,
                expected: Self::ENVELOPE.version,
            });
        }
        let declared = u64::from_le_bytes(bytes[12..20].try_into().expect("eight bytes"));
        if u64::try_from(bytes.len() - HEADER).map_or(true, |actual| actual != declared) {
            return Err(truncated(bytes.len()));
        }
        aligned.extend_from_slice(&bytes[HEADER..]);
        if !current {
            let earlier = Earlier::Version(version);
            let upgrade = Self::upgrade(earlier, &aligned).map_err(payload)?;
            upgrade.memory.validate().map_err(payload)?;
            return Ok(Admission::Upgraded(earlier, upgrade));
        }
        let memory = Self::from_archive(&aligned).map_err(payload)?;
        memory.validate().map_err(payload)?;
        Ok(Admission::Admitted(memory))
    }
    /// Reads the path; an earlier archive is upgraded on open: written in
    /// the current envelope before it is answered, the file left as it was
    /// when that write fails. Nothing is written for an absent entry.
    fn load(path: &Path) -> Result<Admission<Self>, MemoryRefusal> {
        let admission = Self::admit(path)?;
        if let Admission::Upgraded(_, upgrade) = &admission {
            upgrade.memory.save(path)?;
        }
        Ok(admission)
    }
    /// Writes the envelope and the archive to `<path>.new`, syncs it and
    /// renames it over the path. The Nexus is its memory's one writer.
    fn save(&self, path: &Path) -> Result<(), MemoryRefusal> {
        self.validate().map_err(|reason| MemoryRefusal::Payload {
            path: path.to_owned(),
            reason,
        })?;
        let failure = |source| MemoryRefusal::Write {
            path: path.to_owned(),
            source,
        };
        let archive = self
            .to_archive()
            .map_err(|reason| failure(std::io::Error::new(std::io::ErrorKind::InvalidData, reason)))?;
        let mut bytes = Vec::with_capacity(HEADER + archive.len());
        bytes.extend_from_slice(&Self::ENVELOPE.magic);
        bytes.extend_from_slice(&Self::ENVELOPE.version.to_le_bytes());
        bytes.extend_from_slice(&(archive.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&archive);
        let temporary = path.with_extension("new");
        let written = (|| {
            use std::io::Write;
            let mut file = std::fs::File::create(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()
        })();
        if let Err(source) = written {
            let _ = std::fs::remove_file(&temporary);
            return Err(failure(source));
        }
        std::fs::rename(&temporary, path).map_err(|source| {
            let _ = std::fs::remove_file(&temporary);
            failure(source)
        })
    }
}
