//! Isolated local persistence owner. Runtime handlers are not connected yet.
use crate::{Admission, Earlier, MemoryRefusal, Persisted};
use sha2::{Digest, Sha256};
use signal_energy_nexus::library::{
    AfterReplacement, BeforeReplacement, LastSave, PersistenceReadback, PersistenceStatus,
    ReplacedDurabilityUncertain_Data, SaveId, SaveOutcome, SaveReceipt, StoredStateId,
};
use std::{
    fs::File,
    io,
    io::Write,
    path::{Path, PathBuf},
};

/// The encoded candidate bears its identity; no second copy is configured.
pub struct EncodedMemory {
    bytes: Vec<u8>,
}
pub trait IdentifiableMemory {
    fn identity(&self) -> StoredStateId;
}
impl IdentifiableMemory for EncodedMemory {
    fn identity(&self) -> StoredStateId {
        StoredStateId(format!("{:x}", Sha256::digest(&self.bytes)))
    }
}

/// Local native evidence remains in process; only bounded stages go on the wire.
#[derive(Debug)]
pub enum NativeSaveFailure {
    Validation(String),
    Serialization(String),
    Io(io::Error),
}
#[derive(Debug)]
pub struct SaveObservation {
    pub receipt: SaveReceipt,
    pub native_failure: Option<NativeSaveFailure>,
}

/// A temporary file owns only a name it successfully created exclusively.
pub struct TemporaryFile {
    file: File,
    owned_path: Option<PathBuf>,
}
pub trait TemporaryWriting {
    fn write_contents(&mut self, contents: &[u8]) -> io::Result<()>;
    fn synchronize_contents(&self) -> io::Result<()>;
}
impl TemporaryWriting for TemporaryFile {
    fn write_contents(&mut self, contents: &[u8]) -> io::Result<()> {
        self.file.write_all(contents)
    }
    fn synchronize_contents(&self) -> io::Result<()> {
        self.file.sync_all()
    }
}
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if let Some(path) = &self.owned_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub trait DirectorySynchronizing {
    fn synchronize_directory(&self) -> io::Result<()>;
}
impl DirectorySynchronizing for File {
    fn synchronize_directory(&self) -> io::Result<()> {
        self.sync_all()
    }
}

/// This capability is local: an unsuccessful replacement must be a definite
/// local failure, never an absent acknowledgement from a delegated saver.
pub trait SaveTargeting {
    type Temporary: TemporaryWriting;
    type Directory: DirectorySynchronizing;
    fn create_temporary(&self, identity: &SaveId) -> io::Result<Self::Temporary>;
    fn replace(&self, temporary: &mut Self::Temporary) -> io::Result<()>;
    fn open_parent(&self) -> io::Result<Self::Directory>;
}
pub struct LocalSaveTarget {
    pub path: PathBuf,
}
impl SaveTargeting for LocalSaveTarget {
    type Temporary = TemporaryFile;
    type Directory = File;
    fn create_temporary(&self, identity: &SaveId) -> io::Result<TemporaryFile> {
        let name = self
            .path
            .file_name()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        let mut temporary_name = std::ffi::OsString::from(".");
        temporary_name.push(name);
        temporary_name.push(format!(".{:x}.new", Sha256::digest(identity.0.as_bytes())));
        let path = self.path.with_file_name(temporary_name);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(TemporaryFile {
            file,
            owned_path: Some(path),
        })
    }
    fn replace(&self, temporary: &mut TemporaryFile) -> io::Result<()> {
        let source = temporary
            .owned_path
            .as_ref()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        std::fs::rename(source, &self.path)?;
        temporary.owned_path = None;
        Ok(())
    }
    fn open_parent(&self) -> io::Result<File> {
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        File::open(parent)
    }
}

/// Initiators provide this identity; neither the saver nor a forwarder mints it.
pub struct SaveAttempt<'a, M, T> {
    pub save_id: SaveId,
    pub candidate: &'a M,
    pub target: &'a T,
}
pub trait LocalSaveable {
    fn save(&self) -> SaveObservation;
}
impl<M: Persisted, T: SaveTargeting> LocalSaveable for SaveAttempt<'_, M, T> {
    fn save(&self) -> SaveObservation {
        let rejected = |stage, native_failure| SaveObservation {
            receipt: SaveReceipt {
                save_id: self.save_id.clone(),
                save_outcome: SaveOutcome::NotReplaced(stage),
            },
            native_failure: Some(native_failure),
        };
        if let Err(error) = self.candidate.validate() {
            return rejected(
                BeforeReplacement::Validation,
                NativeSaveFailure::Validation(error),
            );
        }
        let archive = match self.candidate.to_archive() {
            Ok(archive) => archive,
            Err(error) => {
                return rejected(
                    BeforeReplacement::Serialization,
                    NativeSaveFailure::Serialization(error),
                )
            }
        };
        let mut bytes = Vec::with_capacity(20 + archive.len());
        bytes.extend_from_slice(&M::ENVELOPE.magic);
        bytes.extend_from_slice(&M::ENVELOPE.version.to_le_bytes());
        bytes.extend_from_slice(&(archive.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&archive);
        let encoded = EncodedMemory { bytes };
        let stored_state_id = encoded.identity();
        let mut temporary = match self.target.create_temporary(&self.save_id) {
            Ok(temporary) => temporary,
            Err(error) => {
                return rejected(
                    BeforeReplacement::TemporaryCreate,
                    NativeSaveFailure::Io(error),
                )
            }
        };
        if let Err(error) = temporary.write_contents(&encoded.bytes) {
            return rejected(
                BeforeReplacement::TemporaryWrite,
                NativeSaveFailure::Io(error),
            );
        }
        if let Err(error) = temporary.synchronize_contents() {
            return rejected(
                BeforeReplacement::TemporarySync,
                NativeSaveFailure::Io(error),
            );
        }
        if let Err(error) = self.target.replace(&mut temporary) {
            return rejected(BeforeReplacement::Replacement, NativeSaveFailure::Io(error));
        }
        let uncertain = |stage, error| SaveObservation {
            receipt: SaveReceipt {
                save_id: self.save_id.clone(),
                save_outcome: SaveOutcome::ReplacedDurabilityUncertain(
                    ReplacedDurabilityUncertain_Data {
                        stored_state_id: stored_state_id.clone(),
                        after_replacement: stage,
                    },
                ),
            },
            native_failure: Some(NativeSaveFailure::Io(error)),
        };
        let directory = match self.target.open_parent() {
            Ok(directory) => directory,
            Err(error) => return uncertain(AfterReplacement::ParentOpen, error),
        };
        if let Err(error) = directory.synchronize_directory() {
            return uncertain(AfterReplacement::ParentSync, error);
        }
        SaveObservation {
            receipt: SaveReceipt {
                save_id: self.save_id.clone(),
                save_outcome: SaveOutcome::Durable(stored_state_id),
            },
            native_failure: None,
        }
    }
}

/// Runtime evidence is held beside Memory, not serialized into its candidate.
pub struct LocalMemory<M> {
    memory: M,
    readback: PersistenceReadback,
}
pub trait LocalObservable<M> {
    fn memory(&self) -> &M;
    fn readback(&self) -> &PersistenceReadback;
}
impl<M> LocalObservable<M> for LocalMemory<M> {
    fn memory(&self) -> &M {
        &self.memory
    }
    fn readback(&self) -> &PersistenceReadback {
        &self.readback
    }
}
#[derive(Debug)]
pub enum LocalAdmissionRefusal {
    Memory(MemoryRefusal),
    UpgradeRequired(Earlier),
}
pub trait LocalAdmissible: Sized {
    fn open(path: &Path) -> Result<Self, LocalAdmissionRefusal>;
}
impl<M: Persisted + Default> LocalAdmissible for LocalMemory<M> {
    fn open(path: &Path) -> Result<Self, LocalAdmissionRefusal> {
        // Read once: the identity describes the same bytes that were validated.
        let bytes = match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    memory: M::default(),
                    readback: PersistenceReadback {
                        persistence_status: PersistenceStatus::Volatile,
                        last_save: LastSave::Never,
                    },
                })
            }
            Err(source) => {
                return Err(LocalAdmissionRefusal::Memory(MemoryRefusal::Read {
                    path: path.to_owned(),
                    source,
                }))
            }
            Ok(_) => std::fs::read(path).map_err(|source| {
                LocalAdmissionRefusal::Memory(MemoryRefusal::Read {
                    path: path.to_owned(),
                    source,
                })
            })?,
        };
        let admission = M::admit_bytes(path, &bytes).map_err(LocalAdmissionRefusal::Memory)?;
        match admission {
            Admission::Admitted(memory) => Ok(Self {
                memory,
                readback: PersistenceReadback {
                    persistence_status: PersistenceStatus::Unproved(
                        EncodedMemory { bytes }.identity(),
                    ),
                    last_save: LastSave::Never,
                },
            }),
            Admission::Upgraded(earlier, _) => Err(LocalAdmissionRefusal::UpgradeRequired(earlier)),
            Admission::Absent => Err(LocalAdmissionRefusal::Memory(MemoryRefusal::Unrecognized {
                path: path.to_owned(),
                bytes: bytes.len() as u64,
            })),
        }
    }
}
pub trait LocalCommittable<M> {
    fn commit<T: SaveTargeting>(
        &mut self,
        save_id: SaveId,
        candidate: M,
        target: &T,
    ) -> SaveObservation;
}
impl<M: Persisted> LocalCommittable<M> for LocalMemory<M> {
    fn commit<T: SaveTargeting>(
        &mut self,
        save_id: SaveId,
        candidate: M,
        target: &T,
    ) -> SaveObservation {
        let observation = SaveAttempt {
            save_id,
            candidate: &candidate,
            target,
        }
        .save();
        match &observation.receipt.save_outcome {
            SaveOutcome::NotReplaced(_) => {}
            SaveOutcome::ReplacedDurabilityUncertain(replaced) => {
                self.memory = candidate;
                self.readback.persistence_status =
                    PersistenceStatus::Unproved(replaced.stored_state_id.clone());
            }
            SaveOutcome::Durable(identity) => {
                self.memory = candidate;
                self.readback.persistence_status = PersistenceStatus::Durable(identity.clone());
            }
        }
        self.readback.last_save = LastSave::SaveReceipt(observation.receipt.clone());
        observation
    }
}
