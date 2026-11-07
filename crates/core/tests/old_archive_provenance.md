# Provenance of the old-schema memory archives

`OLD_ARCHIVES` in `memory_admission.rs` and `OLD_SECONDARY` in `crates/daemon/tests/legacy_memory.rs`
are raw rkyv archives of the `Memory` the previous release wrote, one reminder with the recipient
`Mind energyNexus` and the layer Primary, Secondary, Tertiary, Quaternary.

Source: energy-nexus commit e5722529cd80ea23b3ce3f2b6ef45104a692651c (origin main when this was
made), crate energy-nexus-core with its own Cargo.lock, signal-energy-nexus 226ff14f, 
meta-signal-energy-nexus c20f401c, signal-flow 74f4dae4 (Address carries a Layer). A throwaway
worktree of that commit gained one dev-dependency, signal-flow 74f4dae4 (default features off), and
this test, `crates/core/tests/old_fixture.rs`, which was built and run through the repository
flake check `test` on the remote builder with `-L`; its output lines were copied from the log.

```rust
use energy_nexus_core::{Configuring, Memory};
use meta_signal_energy_nexus::Set;
use signal_energy_nexus::{Address, Level, Provider, Recipient, Reminder};

#[test]
fn print_old_fixtures() {
    for (index, layer) in [
        signal_flow::Layer::Primary,
        signal_flow::Layer::Secondary,
        signal_flow::Layer::Tertiary,
        signal_flow::Layer::Quaternary,
    ]
    .into_iter()
    .enumerate()
    {
        let mut memory = Memory::default();
        memory
            .configure(Set::Reminder(Reminder {
                provider: Provider::Codex,
                level: Level::Low,
                recipient: Recipient(Address {
                    aspect: signal_flow::Aspect::Mind,
                    topic: "energyNexus".into(),
                    layer,
                }),
            }))
            .unwrap();
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&memory).unwrap();
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        println!("FIXTURE {index} {} {hex}", bytes.len());
    }
}
```

Output (`FIXTURE <index> <length> <hex>`), and the SHA-256 of each archive:

```
FIXTURE 0 142 003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff000000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000
FIXTURE 1 142 003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff010000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000
FIXTURE 2 142 003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff020000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000
FIXTURE 3 142 003200000000000000011400000000000000020000000000000000656e657267794e657875730102018b000000f2ffffff030000000000000000000001000000000000000000baffffff00000000b2ffffff000000000500000000000000a2ffffff03000000c0ffffff01000000a0050000000000000100000000000000b4ffffff02000000c0ffffff00000000
```

- layer index 0: 142 bytes, sha256 fa9a442a82f4a0d61e751e1d5531155ff4ee1da52a17c13f8f60876c7bce7f30
- layer index 1: 142 bytes, sha256 27db5fd741bf0be23b0ee238ac828ec35c70bbf0ebeb5ebdf95b0d7e6bbdaadc
- layer index 2: 142 bytes, sha256 bd3370a7273169496efce6dac5ced18d31d5425a2f158a765cf401f5659119b1
- layer index 3: 142 bytes, sha256 7377c0595d0ec957c48a36a4d5f0da676d3f79f2cf1b871d2fdff393fb5aec8c

The four archives differ at exactly one offset, 49, the layer discriminant 0 to 3. The derivation
log is retained outside the repository as `old.log` in the author's scratchpad.
