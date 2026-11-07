//! Memory admission is fail-closed: only a missing entry starts a fresh memory; an old unversioned
//! archive, a damaged or foreign envelope, a non-regular entry and an unreadable file are refused
//! with the bytes untouched, and a refused file is never overwritten.

use std::path::{Path, PathBuf};

use energy_nexus_core::{Admission, Configuring, Memory, MemoryRefusal};

struct Root(PathBuf);

impl Root {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("energy-memory-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("test root");
        Self(path)
    }
    fn memory(&self) -> PathBuf {
        self.0.join("memory.rkyv")
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(
            self.0.join("memory.rkyv"),
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        );
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The archive the previous release wrote for a Memory with one reminder whose recipient carries
/// each Layer, Primary to Quaternary, as raw rkyv bytes with no envelope. Provenance: produced by
/// the base source energy-nexus e5722529cd80ea23b3ce3f2b6ef45104a692651c (energy-nexus-core with
/// signal-energy-nexus 226f, meta-signal-energy-nexus c20f, signal-flow 74f4), by a throwaway test
/// in a separate worktree that archived Memory::default() after configure(Set::Reminder) with the
/// recipient Mind energyNexus and the layer named, printing each archive in hex. The four archives
/// are 142 bytes and differ at exactly one offset, 49, the Layer discriminant 0 to 3.
const OLD_ARCHIVES: [&str; 4] = [
    "003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff000000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000",
    "003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff010000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000",
    "003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff020000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000",
    "003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff030000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000",
];

fn old_archive(index: usize) -> Vec<u8> {
    OLD_ARCHIVES[index]
        .as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn current_file(root: &Root) -> Vec<u8> {
    Memory::default().save(&root.memory()).expect("save");
    std::fs::read(root.memory()).expect("read")
}

/// The bytes of a directory: names and contents.
fn snapshot(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found: Vec<_> = std::fs::read_dir(directory)
        .expect("read directory")
        .map(|entry| {
            let entry = entry.expect("entry");
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap_or_default(),
            )
        })
        .collect();
    found.sort();
    found
}

/// Seed the file, then require that load and save both refuse it and change nothing.
fn refused(root: &Root, bytes: &[u8]) -> MemoryRefusal {
    std::fs::write(root.memory(), bytes).expect("seed");
    let before = snapshot(&root.0);
    let refusal = Memory::load(&root.memory()).expect_err("load refuses the file");
    assert!(
        Memory::default().save(&root.memory()).is_err(),
        "save refuses to replace it"
    );
    assert_eq!(
        snapshot(&root.0),
        before,
        "a refusal changes nothing and leaves no temporary file"
    );
    refusal
}

#[test]
fn the_archives_the_previous_release_wrote_are_refused_for_every_layer() {
    for index in 0..4 {
        let root = Root::new("old");
        let bytes = old_archive(index);
        assert_eq!(bytes.len(), 142);
        assert_eq!(
            bytes[49], index as u8,
            "the Layer discriminant is the one differing byte"
        );
        // the hazard: the current reader would accept these bytes structurally, reading the Layer
        // byte as a Model, which is why admission refuses them before any restore
        assert!(rkyv::from_bytes::<Memory, rkyv::rancor::Error>(&bytes).is_ok());
        assert!(matches!(
            refused(&root, &bytes),
            MemoryRefusal::Unrecognized { .. }
        ));
        assert_eq!(
            std::fs::read(root.memory()).unwrap(),
            bytes,
            "never rewritten"
        );
    }
}

#[test]
fn empty_short_and_foreign_inputs_are_refused_deterministically() {
    let root = Root::new("short");
    assert!(
        matches!(refused(&root, b""), MemoryRefusal::Truncated { .. }),
        "an empty file is not a fresh memory"
    );
    assert!(matches!(
        refused(&root, b"ENXME"),
        MemoryRefusal::Truncated { .. }
    ));
    assert!(matches!(
        refused(&root, b"hello"),
        MemoryRefusal::Unrecognized { .. }
    ));
    assert!(matches!(
        refused(&root, b"hello world, not a memory"),
        MemoryRefusal::Unrecognized { .. }
    ));
    assert!(matches!(
        refused(&root, b"ENXMEMRY\x01"),
        MemoryRefusal::Truncated { .. }
    ));
}

#[test]
fn envelope_lengths_versions_and_payloads_are_checked() {
    let root = Root::new("envelope");
    let valid = current_file(&root);
    std::fs::remove_file(root.memory()).unwrap();

    let mut version = valid.clone();
    version[8..12].copy_from_slice(&9u32.to_le_bytes());
    assert!(matches!(
        refused(&root, &version),
        MemoryRefusal::UnsupportedVersion { found: 9, .. }
    ));

    let mut short = valid.clone();
    short.pop();
    assert!(matches!(
        refused(&root, &short),
        MemoryRefusal::Truncated { .. }
    ));
    let mut long = valid.clone();
    long.push(0);
    assert!(matches!(
        refused(&root, &long),
        MemoryRefusal::Truncated { .. }
    ));
    let mut overflow = valid.clone();
    overflow[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(
        refused(&root, &overflow),
        MemoryRefusal::Truncated { .. }
    ));

    let mut corrupt = valid.clone();
    for byte in corrupt[20..].iter_mut() {
        *byte = 0xff;
    }
    let refusal = refused(&root, &corrupt);
    assert!(
        matches!(refusal, MemoryRefusal::Payload { .. }),
        "a current header over a corrupt payload: {refusal}"
    );
    assert_eq!(
        std::fs::read(root.memory()).unwrap(),
        corrupt,
        "the corrupt current file is preserved"
    );
}

#[test]
fn a_dangling_symlink_a_directory_and_an_unreadable_file_are_read_refusals() {
    let root = Root::new("entries");
    std::os::unix::fs::symlink(root.0.join("nowhere"), root.memory()).expect("dangling link");
    assert!(
        matches!(
            Memory::load(&root.memory()),
            Err(MemoryRefusal::Read { .. })
        ),
        "not a fresh start"
    );
    assert!(matches!(
        Memory::default().save(&root.memory()),
        Err(MemoryRefusal::Read { .. })
    ));
    assert!(
        !root.0.join("nowhere").exists(),
        "the link target was not created"
    );
    std::fs::remove_file(root.memory()).unwrap();

    std::fs::create_dir(root.memory()).expect("a directory in place of the file");
    assert!(matches!(
        Memory::load(&root.memory()),
        Err(MemoryRefusal::Read { .. })
    ));
    std::fs::remove_dir(root.memory()).unwrap();

    if std::os::unix::fs::MetadataExt::uid(&std::fs::metadata("/proc/self").unwrap()) == 0 {
        return;
    }
    std::fs::write(root.memory(), b"x").unwrap();
    std::fs::set_permissions(
        root.memory(),
        std::os::unix::fs::PermissionsExt::from_mode(0o000),
    )
    .unwrap();
    assert!(matches!(
        Memory::load(&root.memory()),
        Err(MemoryRefusal::Read { .. })
    ));
}

#[test]
fn only_a_missing_entry_is_a_fresh_memory_and_load_creates_nothing() {
    let root = Root::new("missing");
    assert!(matches!(
        Memory::admit(&root.memory()),
        Ok(Admission::Absent)
    ));
    let memory = Memory::load(&root.memory()).expect("a missing file is a fresh memory");
    assert!(memory.configuration().reminders.is_empty());
    assert!(snapshot(&root.0).is_empty(), "load creates nothing");
}

#[test]
fn the_current_envelope_round_trips_reopens_and_a_failed_save_changes_nothing() {
    let root = Root::new("current");
    let memory = Memory::default();
    memory.save(&root.memory()).expect("save");
    let bytes = std::fs::read(root.memory()).unwrap();
    assert_eq!(&bytes[..8], b"ENXMEMRY");
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 1);
    assert_eq!(
        u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize,
        bytes.len() - 20
    );
    let restored = Memory::load(&root.memory()).expect("load");
    assert_eq!(restored.configuration(), memory.configuration());
    restored
        .save(&root.memory())
        .expect("save over a valid current file");
    assert_eq!(
        Memory::load(&root.memory()).unwrap().configuration(),
        memory.configuration()
    );

    std::fs::create_dir(root.0.join("memory.new"))
        .expect("the temporary name is taken by a directory");
    let before = std::fs::read(root.memory()).unwrap();
    assert!(matches!(
        memory.save(&root.memory()),
        Err(MemoryRefusal::Write { .. })
    ));
    assert_eq!(
        std::fs::read(root.memory()).unwrap(),
        before,
        "a failed save leaves the target unchanged"
    );
}
