# Upgrades

## 0.3.1 — Energy outlives Flow; window resets on the wall clock

All five packages move to 0.3.1. No contract or memory change.

**Flow goes, Energy stays.** The translator holds the daemon's `Emit.TurnEnds` across Flow going and coming back. When Flow's subscription closes it waits on an inotify watch of the place's runtime directory, armed before it first looked for Flow, and subscribes again on `flow.sock` being made (`IN_CREATE`), moved in (`IN_MOVED_TO`) or having its mode set (`IN_ATTRIB`, which Flow 0.53.3 does once the socket listens). An attempt refused after an event waits for the next one; there is no retry on a timer. The new opening, the last turn end on each provider, reads each provider whose reading is older than it, as at start. While Flow is absent the daemon answers from memory and reads nothing for it. `Down` now means the watch on the place is lost.

**Units.** The translator's unit wants the Flow unit and is ordered after it (`Wants=`, `After=`); it is no longer part of it and no longer wanted by it, so stopping or restarting Flow leaves Energy running. The daemon stays ordered after the translator and bound to it.

**Window resets survive a suspend.** Each provider's reset alarm is a timerfd on `CLOCK_REALTIME`, set with `TFD_TIMER_ABSTIME` at the reading's next reset: it fires at that wall-clock instant, and at once on resume when the instant passed during a suspend. The monotonic timer it replaces ran late by the length of the suspend.

## 0.3.0 — reads on events; no polling interval

All five packages move to 0.3.0. Branch-only preparation, like 0.2.0.

**Pins.** signal-energy-nexus 0.4.0, meta-signal-energy-nexus 0.4.0 and signal-flow 57.0.0 at `8e56457`, which carries `Observe.TurnEnds`; tokio 1 in the daemon, for its schedule.

**Events, not an interval.** The five-minute reading loop and its `Polling` setting are gone. A provider is read when a turn ends on it (Flow's turn ends, subscribed through the translator; a burst on one provider is one read and at most one follow-up), when its window resets (one timer, set from the latest reading's next reset), and when `Read` is asked while the latest reading is older than the last turn end on it. `Read` otherwise answers from memory. `Configure.Polling.5` no longer reads.

**The first reading.** The daemon used to read at start before the translator listened, and that reading was lost. The daemon's unit now follows the translator's, and the first reading is made on the opening of Flow's turn ends, which arrives through the translator: there is no read before the translator serves, and no retry.

**Units.** The daemon's unit is ordered after the translator's and bound to it, wanted by it; the translator's is ordered after the Flow Nexus's unit at the place, part of it and wanted by it. A Flow restart restarts the translator and then the daemon, which subscribes again; stopping Flow stops Energy.

**Memory: format version 2.** `energy.rkyv` is written at format version 2, which has no polling interval. A file of format version 1 (0.2.0) is upgraded on open with every reading, level, setting and reminder carried and its interval dropped; the archive written before the envelope (the proof of concept) is upgraded as in 0.2.0 and its interval dropped too. The translator's file stays at version 1.

## 0.2.0 — Protos 0.33.0 and a model-addressed Flow recipient

All five packages move to 0.2.0. Branch-only preparation: the Flow dependency, signal-flow 57.0.0 (`065730f`, the contract Flow 0.53.3 is built with), is a branch revision and not yet a main.

**Pins.** signal-energy-nexus 0.3.0 (`e24e9b0`), meta-signal-energy-nexus 0.3.0 (`bb5fd0d`), signal 10.3.0 (`7afb`), signal-flow 57.0.0 (`065730f`), datom-codec 0.33.0 (`eabe`) and protos 0.33.0 (`3c7a`); the generator is ethos-zero 19.2.0 (`5a5d`). `ReaderBudget` is built with `try_from`.

**Reminder recipient.** Flow addresses a flow by model, so the recipient of an Energy reminder is `{ Mind energyNexus Sol }` where it was `{ Mind energyNexus Secondary }`: the translator prints the recipient's model into the `Send` it hands to the Flow command, and the recipient configured through the meta socket carries a model.

**Text form of minutes.** protos 0.33.0 prints a one-field newtype as its inner value: `Configure.Polling.5` where the previous form was `Configure.Polling.{ 5 }`. The same holds for a path: `Configure.Program.{ Flow /home/li/.local/bin/flow }` where it was `Configure.Program.{ Flow { /home/li/.local/bin/flow } }`. The new reader accepts the bare form and refuses the braced one; update scripts that call the meta command with the old text.

**Known and inherited.** signal-flow 57.0.0 and its flow-ethos list ethos-zero 17.0.0 (`07714b0`) as a build dependency, which carries protos 0.32.2 and datom-codec 0.32.2 into the lock beside the 0.33.0 pair until signal-flow repins its generator.

**Memory files: an envelope, and the earlier archive upgraded on open.** `energy.rkyv` (the daemon's Memory) and `energy-translator.rkyv` (the translator's programs) each begin with a 20-byte envelope: eight bytes naming the file (`ENXMEMRY`, `ENXTRNSL`), a little-endian format version (1) and the archive's length, then the rkyv archive. A file without the envelope was written before it, by 0.1.0 (the proof of concept, e5722529); it is read in that layout and upgraded on open: written in the envelope before the process serves, the old file left as it was when that write fails. Every reading, level and setting is carried. A reminder is not: its recipient names a Flow layer, Flow now addresses a model, and which model serves a layer is Flow's configuration. The daemon names each reminder it did not carry on standard error, as `Codex Low { Mind energyNexus Secondary }`; configure it again with a model, `Configure.Reminder.{ Codex Low { Mind energyNexus Sol } }`. The translator's programs are carried whole. Only a missing file makes a memory new; an empty or cut-off file, an unsupported version, a length that disagrees with the file, an invalid archive, a configuration that breaks the rules `Configure` enforces, foreign bytes and an entry that cannot be read are each refused with a typed reason, the process does not start, and the file is left as it is. A link is read through.

**Persistence is committed before it is used.** A level change, a `Configure` and a translator setting are made on a copy, saved, and only then made the memory in use and acknowledged. A failed save leaves the memory in use and the file as they were and is answered: the read is refused with `memory not persisted` and the cause, no level change is pushed and no reminder sent; the `Configure` is refused the same way. The next attempt is answered on its own. The write goes to `<file>.new`, is synced and renamed over the file; the Nexus is its memory's one writer.

**energy-nexus-poc is gone.** A deployment runs `energy-nexus-daemon` and `energy-translator` as their own units.
