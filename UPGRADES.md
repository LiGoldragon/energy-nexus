# Upgrades

## 0.2.0 — Protos 0.33.0 and a model-addressed Flow recipient

All five packages move to 0.2.0. Branch-only preparation: the Flow dependency, signal-flow 57.0.0 (`065730f`, the contract Flow 0.53.3 is built with), is a branch revision and not yet a main.

**Pins.** signal-energy-nexus 0.3.0 (`e24e9b0`), meta-signal-energy-nexus 0.3.0 (`bb5fd0d`), signal 10.3.0 (`7afb`), signal-flow 57.0.0 (`065730f`), datom-codec 0.33.0 (`eabe`) and protos 0.33.0 (`3c7a`); the generator is ethos-zero 19.2.0 (`5a5d`). `ReaderBudget` is built with `try_from`.

**Reminder recipient.** Flow addresses a flow by model, so the recipient of an Energy reminder is `{ Mind energyNexus Sol }` where it was `{ Mind energyNexus Secondary }`: the translator prints the recipient's model into the `Send` it hands to the Flow command, and the recipient configured through the meta socket carries a model.

**Text form of minutes.** protos 0.33.0 prints a one-field newtype as its inner value: `Configure.Polling.5` where the previous form was `Configure.Polling.{ 5 }`. The same holds for a path: `Configure.Program.{ Flow /home/li/.local/bin/flow }` where it was `Configure.Program.{ Flow { /home/li/.local/bin/flow } }`. The new reader accepts the bare form and refuses the braced one; update scripts that call the meta command with the old text.

**Known and inherited.** signal-flow 57.0.0 and its flow-ethos list ethos-zero 17.0.0 (`07714b0`) as a build dependency, which carries protos 0.32.2 and datom-codec 0.32.2 into the lock beside the 0.33.0 pair until signal-flow repins its generator.


**Memory file: versioned envelope, fail-closed admission.** `energy.rkyv` now begins with a 20-byte header, the magic `ENXMEMRY`, a little-endian format version (1) and the payload length, then the rkyv archive. A memory written by an earlier release is a bare archive with no header; the daemon refuses it at start with `does not begin with the supported envelope`, leaves it byte for byte as it was, and does not start: no directory, socket or write. Only a missing `energy.rkyv` starts a fresh memory; an empty or truncated file, an unsupported version, a length that disagrees with the file, an invalid archive, a configuration that breaks the rules `Configure` enforces, a symlink or other non-regular entry, and an unreadable file are each refused with a typed reason and never replaced by an empty memory. The previous layout is unchanged except for the reminder recipient: its `Layer` byte now reads as a `Model`, which is why an old file is refused instead of read. To start fresh, move the old `energy.rkyv` aside; carrying its settings forward needs an explicit mapping from layer to model that this release does not provide. Nothing is converted or rewritten.



**Persistence is committed before it is used.** A level change or a `Configure` is computed on a copy, saved, and only then made the memory in use and acknowledged. When the save fails the memory in use is unchanged, nothing is scheduled or acknowledged as applied, and the daemon is halted until it is restarted: every later reading, reminder, redeem and `Configure` is refused with `halted: memory not persisted` and the cause, the polling stops at its next turn, and a read answers with the cause and the last committed snapshot. Nothing retries or clears the halt; a restart admits the file on disk again. Effects already issued before the failure cannot be undone. The replacement writes `energy.new`, syncs it and renames it over the target; it assumes one cooperating writer, may leave `energy.new` after a crash, and does not sync the directory.
