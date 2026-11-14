//! Opens copies of live `energy-translator.rkyv` files, never a live file:
//! the directory named by `ENERGY_LIVE_STORES` holds one directory per host,
//! each with a copy of that host's translator memory. The memory is read
//! whole and written back: a current file identical, an earlier one as its
//! upgrade wrote it. Ignored unless run by hand with the copies at hand.
use energy_nexus_translator::Programs;
use energy_nexus_wire::{Admission, Persisted};

#[test]
#[ignore]
fn every_live_copy_is_read_whole_and_written_back() {
    let root = std::path::PathBuf::from(
        std::env::var_os("ENERGY_LIVE_STORES").expect("ENERGY_LIVE_STORES names the copies"),
    );
    let scratch = tempfile::tempdir().unwrap();
    let mut hosts: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().collect();
    hosts.sort_by_key(|entry| entry.file_name());
    for host in hosts {
        let name = host.file_name().to_string_lossy().into_owned();
        let copy = host.path().join("energy-translator.rkyv");
        if !copy.exists() {
            println!("{name}: no energy-translator.rkyv");
            continue;
        }
        let original = std::fs::read(&copy).unwrap();
        let path = scratch.path().join(format!("{name}.rkyv"));
        std::fs::write(&path, &original).unwrap();
        let (programs, how) = match Programs::load(&path).expect("the copy opens") {
            Admission::Admitted(programs) => (programs, "admitted as it is".to_string()),
            Admission::Upgraded(earlier, upgrade) => (upgrade.memory, format!("upgraded from {earlier}")),
            Admission::Absent => panic!("the copy is absent"),
        };
        println!(
            "{name}: {} bytes, {how}; codex program {}, flow program {}, claude credentials {}",
            original.len(),
            if programs.codex_program.0.is_some() { "set" } else { "unset" },
            if programs.flow_program.0.is_some() { "set" } else { "unset" },
            if programs.claude_credentials.0.is_some() { "set" } else { "unset" },
        );
        let written = scratch.path().join("written.rkyv");
        programs.save(&written).expect("written back");
        let rewritten = std::fs::read(&written).unwrap();
        match Programs::load(&written).expect("the written file opens") {
            Admission::Admitted(reread) => assert_eq!(reread, programs),
            other => panic!("the written file is current, got {other:?}"),
        }
        if std::fs::read(&path).unwrap() == original {
            assert_eq!(rewritten, original, "a current file is written back identical");
            println!("  written back byte-identical");
        } else {
            assert_eq!(rewritten, std::fs::read(&path).unwrap());
            println!("  upgraded on open; written back as the upgrade wrote it, {} bytes", rewritten.len());
        }
    }
}
