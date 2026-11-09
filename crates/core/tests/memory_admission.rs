//! Memory is read through its envelope. Only a missing entry makes a memory
//! new; the archive written before the envelope is upgraded on open; a
//! damaged or foreign file is refused and left as it is.

use std::path::{Path, PathBuf};

use energy_nexus_core::{unversioned, Configuring, Memory, Remembering};
use energy_nexus_wire::{Admission, MemoryRefusal, Persisted};
use meta_signal_energy_nexus::{ProviderTarget, WindowTarget};
use signal_energy_nexus::{Instant, Level, Minutes, Name, Percent, Provider, Target, Threshold};

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
        self.0.join("energy.rkyv")
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(
            self.memory(),
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        );
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The archive energy-nexus e5722529 wrote for a Memory with one reminder
/// whose recipient carries each Layer, Primary to Quaternary, as raw rkyv
/// bytes with no envelope (provenance in old_archive_provenance.md). The
/// four are 142 bytes and differ at one offset, 49, the Layer discriminant.
const OLD_ARCHIVES: [&str; 4] = [
    "003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff000000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000",
    "003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff010000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000",
    "003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff020000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000",
    "003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff030000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000",
];

/// The file energy-nexus e5722529's own `Memory::save` wrote for a Memory
/// with every field set: two reminders, every setting, three readings
/// (provenance in old_archive_provenance.md).
const OLD_FULL: &[u8] = include_bytes!("fixtures/energy-e5722529.rkyv");

fn old_archive(index: usize) -> Vec<u8> {
    OLD_ARCHIVES[index]
        .as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

/// The names and contents of a directory.
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

/// Seeds the file, then requires that opening it refuses it and changes nothing.
fn refused(root: &Root, bytes: &[u8]) -> MemoryRefusal {
    std::fs::write(root.memory(), bytes).expect("seed");
    let before = snapshot(&root.0);
    let refusal = Memory::load(&root.memory()).expect_err("load refuses the file");
    assert_eq!(
        snapshot(&root.0),
        before,
        "a refusal changes nothing and leaves no temporary file"
    );
    refusal
}

fn upgraded(root: &Root, bytes: &[u8]) -> (Memory, Vec<unversioned::Reminder>) {
    std::fs::write(root.memory(), bytes).expect("seed");
    match Memory::load(&root.memory()).expect("the unversioned archive opens") {
        Admission::Upgraded(upgrade) => (upgrade.memory, upgrade.uncarried),
        other => panic!("an unversioned archive is upgraded, got {other:?}"),
    }
}

/// The file now holds the current envelope, and reads back as `memory`.
fn written_in_the_envelope(root: &Root, memory: &Memory) {
    let bytes = std::fs::read(root.memory()).unwrap();
    assert_eq!(&bytes[..8], b"ENXMEMRY");
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 1);
    match Memory::load(&root.memory()).unwrap() {
        Admission::Admitted(reopened) => {
            assert_eq!(reopened.configuration(), memory.configuration());
            assert_eq!(reopened.history(), memory.history());
            for provider in [Provider::Claude, Provider::Codex] {
                assert_eq!(reopened.latest(&provider), memory.latest(&provider));
            }
        }
        other => panic!("the upgraded file is current, got {other:?}"),
    }
    assert!(!root.memory().with_extension("new").exists());
}

#[test]
fn the_archive_written_before_the_envelope_is_upgraded_for_every_layer() {
    for (index, layer) in [
        unversioned::Layer::Primary,
        unversioned::Layer::Secondary,
        unversioned::Layer::Tertiary,
        unversioned::Layer::Quaternary,
    ]
    .into_iter()
    .enumerate()
    {
        let root = Root::new("old");
        let bytes = old_archive(index);
        assert_eq!(bytes.len(), 142);
        assert_eq!(bytes[49], index as u8, "the Layer discriminant");
        // The current layout would take these bytes too, reading the Layer
        // as a Model; the missing envelope is what says they are older.
        assert!(rkyv::from_bytes::<Memory, rkyv::rancor::Error>(&bytes).is_ok());
        let (memory, uncarried) = upgraded(&root, &bytes);
        assert!(memory.configuration().reminders.is_empty());
        assert_eq!(uncarried.len(), 1);
        assert_eq!(uncarried[0].recipient.0.layer, layer);
        assert_eq!(
            uncarried[0].to_string(),
            format!("Codex Low {{ Mind energyNexus {layer:?} }}")
        );
        written_in_the_envelope(&root, &memory);
    }
}

#[test]
fn every_reading_and_setting_of_the_proof_of_concept_memory_is_carried() {
    let root = Root::new("full");
    assert_eq!(OLD_FULL.len(), 617);
    let (memory, uncarried) = upgraded(&root, OLD_FULL);
    let configuration = memory.configuration();
    assert_eq!(configuration.polling, Minutes(5));
    assert!(configuration.thresholds.contains(&Threshold {
        level: Level::Fair,
        left: Percent(25)
    }));
    assert_eq!(configuration.redeem_expiring, Minutes(720));
    assert_eq!(configuration.redeem_used_at_least, Percent(3));
    assert!(configuration.targets.contains(&ProviderTarget {
        provider: Provider::Codex,
        target: Target::SpendBy(Instant(1762450320)),
    }));
    assert_eq!(
        configuration.window_targets,
        vec![WindowTarget {
            provider: Provider::Claude,
            name: Name::FableWeek,
            target: Target::Even,
        }]
    );
    assert!(configuration.reminders.is_empty());
    assert_eq!(memory.history().len(), 3);
    let claude = memory.latest(&Provider::Claude).expect("Claude's latest");
    assert_eq!(claude.reading.window_vector.len(), 3);
    let codex = memory.latest(&Provider::Codex).expect("Codex's latest");
    assert_eq!(codex.level, Level::Out);
    assert_eq!(codex.reading.at, Instant(1762210360));
    assert_eq!(
        uncarried.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "Codex Low { Mind energyNexus Secondary }",
            "Claude Out { Psyche core Primary }"
        ]
    );
    written_in_the_envelope(&root, &memory);
}

#[test]
fn an_upgrade_that_cannot_be_written_leaves_the_old_file() {
    let root = Root::new("unwritable");
    std::fs::write(root.memory(), OLD_FULL).unwrap();
    std::fs::create_dir(root.memory().with_extension("new")).unwrap();
    assert!(matches!(
        Memory::load(&root.memory()),
        Err(MemoryRefusal::Write { .. })
    ));
    assert_eq!(std::fs::read(root.memory()).unwrap(), OLD_FULL);
}

#[test]
fn empty_short_and_foreign_inputs_are_refused() {
    let root = Root::new("short");
    assert!(
        matches!(refused(&root, b""), MemoryRefusal::Truncated { .. }),
        "an empty file is not a memory made new"
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
    let synthetic: Vec<u8> = (0..4096u32).map(|n| (n % 251) as u8).collect();
    assert!(matches!(
        refused(&root, &synthetic),
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
    Memory::default().save(&root.memory()).expect("save");
    let valid = std::fs::read(root.memory()).expect("read");
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
        "a current header over a corrupt archive: {refusal}"
    );
}

#[test]
fn an_entry_that_cannot_be_read_is_refused_and_a_link_is_read_through() {
    let root = Root::new("entries");
    std::os::unix::fs::symlink(root.0.join("nowhere"), root.memory()).expect("dangling link");
    assert!(
        matches!(
            Memory::load(&root.memory()),
            Err(MemoryRefusal::Read { .. })
        ),
        "a dangling link is no memory made new"
    );
    std::fs::remove_file(root.memory()).unwrap();

    std::fs::create_dir(root.memory()).expect("a directory in place of the file");
    assert!(matches!(
        Memory::load(&root.memory()),
        Err(MemoryRefusal::Read { .. })
    ));
    std::fs::remove_dir(root.memory()).unwrap();

    let target = root.0.join("elsewhere.rkyv");
    let mut memory = Memory::default();
    memory
        .configure(meta_signal_energy_nexus::Set::Polling(Minutes(9)))
        .unwrap();
    memory.save(&target).unwrap();
    std::os::unix::fs::symlink(&target, root.memory()).expect("link to a memory");
    match Memory::load(&root.memory()).unwrap() {
        Admission::Admitted(read) => assert_eq!(read.configuration().polling, Minutes(9)),
        other => panic!("a link to a memory is read through, got {other:?}"),
    }
    std::fs::remove_file(root.memory()).unwrap();

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
fn only_a_missing_entry_is_a_memory_made_new_and_load_creates_nothing() {
    let root = Root::new("missing");
    assert!(matches!(
        Memory::load(&root.memory()),
        Ok(Admission::Absent)
    ));
    assert!(snapshot(&root.0).is_empty(), "load creates nothing");
}

#[test]
fn the_current_envelope_round_trips_and_a_failed_save_changes_nothing() {
    let root = Root::new("current");
    let memory = Memory::default();
    memory.save(&root.memory()).expect("save");
    let bytes = std::fs::read(root.memory()).unwrap();
    assert_eq!(
        u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize,
        bytes.len() - 20
    );
    written_in_the_envelope(&root, &memory);

    std::fs::create_dir(root.memory().with_extension("new"))
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
