//! Opens copies of live `energy.rkyv` files, never a live file: the
//! directory named by `ENERGY_LIVE_STORES` holds one directory per host,
//! each with a copy of that host's memory. Every record is read, and the
//! memory is written back: a current file is written back identical, an
//! earlier one upgraded and read back as it was carried. Ignored unless run
//! by hand with the copies at hand.
use energy_nexus_core::{Memory, Remembering, Configuring, Validating};
use energy_nexus_wire::{Admission, Persisted};

#[test]
#[ignore]
fn every_live_copy_is_read_whole_and_written_back() {
    let root = std::path::PathBuf::from(
        std::env::var_os("ENERGY_LIVE_STORES").expect("ENERGY_LIVE_STORES names the copies"),
    );
    let scratch = std::env::temp_dir().join(format!("energy-live-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut hosts: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().collect();
    hosts.sort_by_key(|entry| entry.file_name());
    for host in hosts {
        let copy = host.path().join("energy.rkyv");
        if !copy.exists() {
            println!("{}: no energy.rkyv", host.file_name().to_string_lossy());
            continue;
        }
        let original = std::fs::read(&copy).unwrap();
        let path = scratch.join(format!("{}.rkyv", host.file_name().to_string_lossy()));
        std::fs::write(&path, &original).unwrap();
        let admission = Memory::load(&path).expect("the copy opens");
        let (memory, how) = match admission {
            Admission::Admitted(memory) => (memory, "admitted as it is".to_string()),
            Admission::Upgraded(earlier, upgrade) => {
                (upgrade.memory, format!("upgraded from {earlier}, {} uncarried", upgrade.uncarried.len()))
            }
            Admission::Absent => panic!("the copy is absent"),
        };
        memory.configuration().validate().expect("its rules hold");
        let mut latest = 0;
        for provider in [signal_energy_nexus::Provider::Claude, signal_energy_nexus::Provider::Codex] {
            if let Some(record) = memory.latest(&provider) {
                assert_eq!(record.provider, provider);
                latest += 1;
            }
        }
        let windows: usize = memory.history().iter().map(|reading| reading.window_vector.len()).sum();
        println!(
            "{}: {} bytes, {how}; {latest} latest, {} readings ({windows} windows), {} thresholds, {} reminders, {} targets, {} window targets",
            host.file_name().to_string_lossy(),
            original.len(),
            memory.history().len(),
            memory.configuration().threshold_vector.len(),
            memory.configuration().reminder_vector.len(),
            memory.configuration().targets.0.len(),
            memory.configuration().window_targets.0.len(),
        );
        let written = scratch.join("written.rkyv");
        memory.save(&written).expect("written back");
        let rewritten = std::fs::read(&written).unwrap();
        match Memory::load(&written).expect("the written file opens") {
            Admission::Admitted(reread) => assert_eq!(reread, memory, "read back as written"),
            other => panic!("the written file is current, got {other:?}"),
        }
        if std::fs::read(&path).unwrap() == original {
            assert_eq!(rewritten, original, "a current file is written back identical");
            println!("  written back byte-identical");
        } else {
            assert_eq!(rewritten, std::fs::read(&path).unwrap(), "the upgrade on open wrote what save writes");
            println!("  upgraded on open; written back as the upgrade wrote it, {} bytes", rewritten.len());
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
}
