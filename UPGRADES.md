# Upgrades

## 0.4.0 — pace in useful hours; the usefulness schedule

All five packages move to 0.4.0. Branch-only (schedule-pace); the vision is energy-nexus-v1, "Pace: over or under, per day" (psyche-belief 90b5bf1, 09947be, d0b875d).

**Pins.** signal-energy-nexus 0.6.0 (`d7d9f8d`) and meta-signal-energy-nexus 0.6.0 (`dd6ec2b`): `Pace` carries `hours_left`, `useful_left` and `per_useful_hour` where it carried `per_day`, and `Set` carries `Span`, `RemoveSpan`, `Override`, `RemoveOverride` and `Baseline`. Both contract digests move: a 0.3.x client is refused at the greeting, so the daemon, the translator and the text clients deploy together. jiff 0.2 in the core, for the owner's time zone; the daemon's closure stays free of Datom, Protos and JSON.

**Useful hours.** `Pacing::pace` takes the time zone the owner's hours are read in; the daemon passes the system's (`TimeZone::system()`: `TZ`, else `/etc/localtime`). Time is weighed exactly, in percent-seconds, segment by segment between local hour boundaries and override edges (`UsefulTime`). Expected is the useful time spent from the window's start over the useful time from its start to the target's end (the reset for `Even`, the instant for `SpendBy`), rounded half up; a window with no useful time at all counts time plainly. `hours_left` is the wall-clock hours to the target's end, rounded; `useful_left` is the useful hours from the reading to that end, to a tenth; `per_useful_hour` is the percent left over the unrounded useful hours left, to a hundredth, and the whole percent left when no useful hour is left. With no schedule the baseline is 100 and pace reads as 0.3.x did.

**The schedule.** `Configure.Span` adds one span, its days kept in week order and each once; an equal span is not added twice, and `RemoveSpan` removes the equal one. A span's hours are 0 to 23 and differ; To before From runs past midnight into the next day. `Override` adds one stretch, From before To; `RemoveOverride` removes it. `Baseline` sets the usefulness of every hour nothing names. Every usefulness is 0 to 100. Where spans overlap the one naming fewer days counts, then fewer hours, then the later configured; an override counts over every span, the shorter of two overlapping overrides first, then the later configured.

**Memory: format version 3.** `energy.rkyv` is written at format version 3, whose configuration holds `schedule`, `overrides` and `baseline`. Version 2 (0.3.0 to 0.3.2) is upgraded on open with everything carried and the schedule empty at baseline 100; version 1 and the archive written before the envelope are upgraded as before, to version 3. The schedule's rules are checked on every read and before every write. The translator's file is unchanged.

**energy-pace** prints the hours left, the useful hours left and the percent per useful hour in place of the daily share.

**Tests.** `crates/core/tests/useful_hours.rs` holds the module's worked examples: his Codex week (Baseline 20, every day 8 to 22 at 100, 35% used at 13 November 16:50) reads `Even 25 35 Over 124 83.5 0.78`; the weekend keeper, the evening job, the gradient and the dentist's afternoon read hour by hour as the module says; the more specific span counts, an override counts over every span, a span may run past midnight, and the hours follow daylight saving. `memory_admission.rs` upgrades a format version 2 file and keeps a schedule across a restart.

## 0.3.2 — signal-flow 61 and the Signal greeting

All five packages move to 0.3.2. No Energy contract or memory change. Branch-only preparation: it is the Energy side of the Flow 0.54.7 rollout and is deployed with it, in the same Home generation.

**Pins.** signal-flow 57.0.0 (`3d0a4cd`) moves to 61.0.0 (`4871bb8`), the contract Flow 0.54.7 is built with. signal moves from 10.3.0 to 11.0.0 (`6d4eb74`): signal-flow 61 depends on it and re-exports the greeting types from it (`Contracted`, `ContractDigest`, `Handshake`, `HandshakeReceipt`, `HandshakeRejection`), and `signal` declares `links = "signal"`, so no second signal can sit beside it. signal-energy-nexus and meta-signal-energy-nexus, which carry signal-flow in their own contracts, move with it to 0.5.0 (`7ca10eb`, `3264d6f`; wire unchanged, Rust types of the recipient change), and the translator's datom-codec and protos move to 09a0526 and c93b598, the pair signal-flow 61 builds with. Energy builds with rust-build and carries no Cargo.nix or build-base pin; Cargo.lock follows.

**Flow's types.** Under signal-flow 61 an `Address` holds a `Topic` newtype, a turn end holds a `FlowId` and an `ObservedAt`, and a `Send` holds `address` and `sendable_message`. The translator reads `observed_at.0` and prints `topic.0` in the reminder's `Send`; the text of the reminder is unchanged.

**The translator greets Flow.** Flow 0.54.7 greets on its ordinary socket as it has on its meta socket since 0.54.6: the first frame of a connection is a `Handshake`, and a Query before the greeting is answered `GreetingRefused.ContractMismatch(<digest>)` and the connection is closed. The translator's subscription now writes `signal_flow::Query::greeting()`, reads one `HandshakeReceipt`, and writes `Observe.TurnEnds` only after `Greeted` naming `signal_flow::Query::contract_digest()`. Each read of the receipt waits at most the configured greeting wait (below); the wait is per read and not a deadline for the whole handshake.

**The greeting wait is configured.** `energy-translator NAME RUNTIME STATE [GREETING_WAIT_SECONDS]`: the optional fourth start argument is whole seconds, and thirty seconds where none is given (`GreetingWait::default()`); zero or anything that is not whole seconds keeps the translator from starting, naming the greeting wait. It is the connection's read timeout (`SO_RCVTIMEO`) on the reads of Flow's receipt, so it bounds each read, not the greeting as a whole: a Flow that sends no byte for the wait fails the subscription, and a Flow that sends the receipt a byte at a time, each sooner than the wait, is waited on for as long as it goes on. The handshake has no absolute deadline. A start argument is the translator's existing process-level configuration (its place paths are passed so); a setting changed at run time through `energy-nexus-meta` would need a new `Set` variant in meta-signal-energy-nexus, a wire change to that contract, and is not made here. `Translator::open` takes the `GreetingWait`, and `Translator` holds it.

**A refused greeting is a typed failure.** `subscribe` returns `SubscriptionFailure`: `Transport`, `GreetingRefused(HandshakeRejection)` (Flow's own digest is in the rejection), `GreetedAnotherContract(ContractDigest)`, `ReceiptUnreadable` and `GreetingUnanswered(GreetingWait)` (a read of the receipt waited the configured wait and got no byte). A failed subscription is handled as any refused attempt is: the relay logs it and waits for the next `IN_MOVED_TO` of `flow.sock`, so Energy runs on from memory without Flow's turn ends. A translator built on signal-flow 57 against Flow 0.54.7 is such a refused peer, which is why this change deploys with Flow 0.54.7.

**Tests.** The fake Flows of the translator and the daemon answer the greeting before reading the subscription. New: a refused greeting and a `Greeted` naming another contract are typed failures with no query sent, a Flow that never answers the greeting is the typed `GreetingUnanswered` with no query sent, the configured wait is the one honoured (a 200 ms wait times out on an answer 1.5 s late that a 20 s wait takes), the wait parses as whole seconds with thirty as its default, the translator takes the fourth start argument and refuses a bad one before any socket is placed, and a relay refused at the greeting waits for the socket's next placing and then subscribes.

## 0.3.1 — Energy outlives Flow; window resets on the wall clock

All five packages move to 0.3.1. No contract or memory change.

**Flow goes, Energy stays.** The translator holds the daemon's `Emit.TurnEnds` across Flow going and coming back. When Flow's subscription closes it waits on an inotify watch of the place's runtime directory, armed before it first looked for Flow, and subscribes again on `flow.sock` being moved to its name (`IN_MOVED_TO`), which Flow 0.53.3 does only once the socket listens. An attempt refused after an event waits for the next one; there is no retry on a timer. The new opening, the last turn end on each provider, reads each provider whose reading is older than it, as at start. While Flow is absent the daemon answers from memory and reads nothing for it. `Down` now means the watch on the place is lost.

**A socket appears at its name only listening.** The daemon and the translator place each socket as Flow 0.53.3 does: bound under `<runtime>/.<name>.<pid>`, its mode set to `0600`, made to listen, then renamed to its name. A stale socket at the name is replaced by the rename, with no moment at which the name is absent; a socket that accepts a connection is refused, and the process does not start (before, it was removed and taken over).

**Units.** The translator's unit wants the Flow unit and is ordered after it (`Wants=`, `After=`); it is no longer part of it and no longer wanted by it, so stopping or restarting Flow leaves Energy running. The daemon stays ordered after the translator and bound to it.

**The CLIs take the datom alone.** `energy-nexus '<datom>'`, `energy-nexus-meta '<datom>'` and `energy-pace [Claude|Codex]` type no place: it is read from `$XDG_CONFIG_HOME/nexus/energy-nexus.datom` (`~/.config/nexus/energy-nexus.datom` when unset or empty), `CliConfiguration.{ PlaceName }`, e.g. `{ main }` (vision:nexus-v1). A second argument is refused; a configuration missing, unreadable or naming no place is a typed refusal naming the file, exit 2. The daemon's note on an uncarried reminder names `energy-nexus-meta 'Configure.Reminder.{ … }'` without a place.

**Window resets survive a suspend.** Each provider's reset alarm is a timerfd on `CLOCK_REALTIME`, set with `TFD_TIMER_ABSTIME` at the reading's next reset: it fires at that wall-clock instant, and at once on resume when the instant passed during a suspend. The monotonic timer it replaces ran late by the length of the suspend.

## 0.3.0 — reads on events; no polling interval

All five packages move to 0.3.0. Branch-only preparation, like 0.2.0.

**Pins.** signal-energy-nexus 0.4.0, meta-signal-energy-nexus 0.4.0 and signal-flow 57.0.0 at `3d0a4cd`, which carries `Observe.TurnEnds` (and `Observe.Metaflows`, which Energy does not ask); tokio 1 in the daemon, for its schedule.

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
