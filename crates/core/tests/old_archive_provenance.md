# Provenance of the archives written before the envelope

Every fixture here was written by energy-nexus e5722529cd80ea23b3ce3f2b6ef45104a692651c, the
revision the proof of concept runs on ouranos (its unit started on 2026-11-06 at 13:39:26, 37
seconds after that commit; its memory is ~/.local/state/energy-nexus/memory.rkyv and
translator.rkyv). That code reads and writes bare rkyv archives with signal-energy-nexus 226ff14f,
meta-signal-energy-nexus c20f401c and signal-flow 74f4dae4, whose Address carries a Layer. No live
file was read to make them.

## `OLD_ARCHIVES` (memory_admission.rs)

Four archives of `Memory::default()` after one `configure(Set::Reminder)` with the recipient `Mind
energyNexus` and the layer Primary, Secondary, Tertiary, Quaternary, printed by a throwaway test in
a worktree of e5722529 with one dev-dependency, signal-flow 74f4dae4. 142 bytes each; they differ at
one offset, 49, the Layer discriminant 0 to 3.

- layer 0: sha256 fa9a442a82f4a0d61e751e1d5531155ff4ee1da52a17c13f8f60876c7bce7f30
- layer 1: sha256 27db5fd741bf0be23b0ee238ac828ec35c70bbf0ebeb5ebdf95b0d7e6bbdaadc
- layer 2: sha256 bd3370a7273169496efce6dac5ced18d31d5425a2f158a765cf401f5659119b1
- layer 3: sha256 7377c0595d0ec957c48a36a4d5f0da676d3f79f2cf1b871d2fdff393fb5aec8c

## `fixtures/energy-e5722529.rkyv` and `../../translator/tests/fixtures/energy-translator-e5722529.rkyv`

Made on 2026-11-09 on the remote builder by a clone of e5722529 with two throwaway tests and one
dev-dependency (signal-flow 74f4dae4 for energy-nexus-core, one line added to Cargo.lock), built
through the flake check `test` with `-L`; each test printed its file in hex, copied from the log.

- `energy-e5722529.rkyv`, 617 bytes, sha256
  1715a4770845a4ab172ef19f43c3261107eaf49ecb64fda94eba6bd29690ab3d: the file e5722529's own
  `Memory::save` wrote for a Memory configured with Polling 5, Threshold Fair 25, the reminders
  `Codex Low { Mind energyNexus Secondary }` and `Claude Out { Psyche core Primary }`,
  RedeemExpiring 720, RedeemUsedAtLeast 3, Target Codex SpendBy 1762450320 and WindowTarget Claude
  FableWeek Even, after three readings: Claude (Session, Week, FableWeek), Codex allowed with two
  resets, and Codex not allowed (Out).
- `energy-translator-e5722529.rkyv`, 157 bytes, sha256
  bc0b57c6d6140a49431161bf798be37de030db0deddda1367cac1fe4191d6270: `rkyv::to_bytes` of
  `Programs`, the call e5722529's `Translator::configure` writes the file with, holding the three
  paths the live file names (flows/ff3882/activation-places.md): the Codex program, Flow
  `/home/li/.local/bin/flow-0.51` and the Claude credentials.

```rust
// crates/core/tests/old_fixture.rs at e5722529, abridged: every Set above through
// Memory::configure, the three readings through Memory::remember, then
let path = std::env::temp_dir().join(format!("old-fixture-{}.rkyv", std::process::id()));
memory.save(&path).unwrap();
let bytes = std::fs::read(&path).unwrap();
println!("MEMORYFIXTURE {} {}", bytes.len(), hex(&bytes));

// crates/translator/tests/old_programs.rs at e5722529
let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&programs).unwrap();
println!("PROGRAMSFIXTURE {} {}", bytes.len(), hex(&bytes));
```
