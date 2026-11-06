# Upgrades

## 0.2.0 — Protos 0.33.0 and a model-addressed Flow recipient

All five packages move to 0.2.0. Branch-only preparation: the Flow dependency, signal-flow 57.0.0 (`065730f`, the contract Flow 0.53.3 is built with), is a branch revision and not yet a main.

**Pins.** signal-energy-nexus 0.3.0 (`e24e9b0`), meta-signal-energy-nexus 0.3.0 (`bb5fd0d`), signal 10.3.0 (`7afb`), signal-flow 57.0.0 (`065730f`), datom-codec 0.33.0 (`eabe`) and protos 0.33.0 (`3c7a`); the generator is ethos-zero 19.2.0 (`5a5d`). `ReaderBudget` is built with `try_from`.

**Reminder recipient.** Flow addresses a flow by model, so the recipient of an Energy reminder is `{ Mind energyNexus Sol }` where it was `{ Mind energyNexus Secondary }`: the translator prints the recipient's model into the `Send` it hands to the Flow command, and the recipient configured through the meta socket carries a model.

**Text form of minutes.** protos 0.33.0 prints a one-field newtype as its inner value: `Configure.Polling.5` where the previous form was `Configure.Polling.{ 5 }`. The same holds for a path: `Configure.Program.{ Flow /home/li/.local/bin/flow }` where it was `Configure.Program.{ Flow { /home/li/.local/bin/flow } }`. The new reader accepts the bare form and refuses the braced one; update scripts that call the meta command with the old text.

**Known and inherited.** signal-flow 57.0.0 and its flow-ethos list ethos-zero 17.0.0 (`07714b0`) as a build dependency, which carries protos 0.32.2 and datom-codec 0.32.2 into the lock beside the 0.33.0 pair until signal-flow repins its generator.
