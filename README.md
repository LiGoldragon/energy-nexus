# Energy Nexus v1

Energy Nexus reads Claude and Codex quota usage on events, levels the typed readings, retains history across restarts, pushes level changes, sends configured reminders through Flow, compares each provider with its pace target, and applies the Codex reset rule.

The processes stay separated:

- `energy-nexus-daemon` owns typed Memory, Operation, public Signal, and meta Signal.
- `energy-translator` alone holds provider API words, JSON, credentials, Datom, process calls, and reminder prose.
- `energy-nexus` and `energy-nexus-meta` are text-only clients.
- `energy-pace` shows both providers' usage per period, target, standing, and daily share.

All process-to-process traffic behind the translator is length-prefixed, validated rkyv signal. A place names an instance; its runtime and state paths are passed once when its processes start. The text clients take the datom and nothing else: the place is their configuration's, one datom at `$XDG_CONFIG_HOME/nexus/energy-nexus.datom` (`~/.config/nexus/energy-nexus.datom` when the variable is unset or empty), `CliConfiguration.{ PlaceName }`, and the place name is resolved below the standard XDG runtime and state roots. No place is typed: a second argument is refused. A configuration that is missing, cannot be read or names no place is a typed refusal naming the file (`Missing.<path>`, `Unreadable.{ <path> <reason> }`, `Invalid.{ <path> <reason> }`, exit 2), and nothing is sent.

```sh
energy-nexus-daemon main /run/user/1001/nexus/main ~/.local/state/nexus/main
energy-translator main /run/user/1001/nexus/main ~/.local/state/nexus/main
# optionally a fourth argument, whole seconds: how long each read of Flow's greeting receipt waits (30 when absent)
energy-translator main /run/user/1001/nexus/main ~/.local/state/nexus/main 30
# ~/.config/nexus/energy-nexus.datom
{ main }
energy-nexus 'Read.Codex'
energy-nexus-meta 'Configure.RedeemExpiring.720'
energy-nexus-meta 'Configure.Program.{ Flow /home/li/.local/bin/flow-nexus }'
energy-pace
```

A place's two directories are shared with every other Nexus at the place and are declared by the environment (CriomOS-home declares them as user tmpfiles rules); no Energy process makes or removes them, and a process whose place is absent does not start. Each process makes only its own sockets there and removes only those when it is stopped (SIGTERM or SIGINT): the daemon `energy.sock` and `energy-meta.sock`, the translator `energy-translator.sock`, `energy-meta-translator.sock` and `energy-emission.sock`. Every Nexus at a place places a socket at its name only once it listens: it is bound under a temporary name beside it, `.<name>.<pid>`, its mode set to `0600`, made to listen, then renamed to its name. A stale socket a crashed process left at the name is replaced by the rename; one that accepts a connection is refused. A peer waiting for a Nexus waits on its socket's name moved in (inotify `IN_MOVED_TO`) and connects at once. Its memory is `energy.rkyv` (the daemon) and `energy-translator.rkyv` (the translator's configured programs) in State. Each file is an envelope (its kind, a format version, the archive's length) around an rkyv archive. A file written before the envelope, by the proof of concept, is upgraded on open; every reading and setting is carried, and each reminder, whose recipient named a Flow layer, is named on the daemon's standard error to be configured again with a model. Only a missing file makes a memory new; a file that cannot be read keeps the process from starting and is left as it is. Every change is saved before it is used, and a failed save is answered as a refusal.

Each process sends `READY=1` to `$NOTIFY_SOCKET`, when it is set, once all its sockets listen, so each runs as a systemd unit of `Type=notify`. The daemon's unit is ordered after the translator's and bound to it. The translator's unit wants the Flow Nexus's unit at the place and is ordered after it, and is not bound to it: Energy keeps running when Flow stops, and takes Flow back when it returns.

Energy reads a provider on three events and never on an interval:

- A turn ends on it. The daemon holds `Emit.TurnEnds` open on the translator, which subscribes to the Flow Nexus at the place (`Observe.TurnEnds` on `flow.sock`, signal, after the Signal greeting that signal-flow 61 requires of every ordinary connection: the translator writes signal-flow's `Handshake`, reads the receipt, and sends its query only on `Greeted` naming its own contract's digest; a `GreetingRefused`, a receipt that never comes, or any other answer is a typed subscription failure that closes the connection and waits for the next event below; the receipt's reads each wait the configured greeting wait, the translator's optional fourth start argument in whole seconds, thirty by default: a per-read timeout, not a deadline for the whole handshake) and hands each turn end on as the provider its harness spends. A turn end that arrives while a read of that provider is in flight owes one more read when it ends: a burst is one read and at most one follow-up. The subscription's opening, the last turn end on each provider, reads each provider whose latest reading is older than it; this is the first reading at start. When Flow goes the subscription closes and Energy runs on, answering from memory and reading nothing for Flow's absence. The translator watches the place's runtime directory with inotify, armed before it first looks for Flow, and subscribes again on the event of `flow.sock` being moved to its name (`IN_MOVED_TO`), which Flow does only once the socket listens; an attempt the socket refuses waits for the next event. Nothing is tried on a timer. The new opening reaches the daemon on the same connection and is taken as the first one was.
- Its window resets. Each reading names when its provider's next window resets; the provider's alarm is set for that instant and reads once when it fires, and a newer reading replaces it. The alarm is a timerfd on `CLOCK_REALTIME` set with `TFD_TIMER_ABSTIME` at the reset's Unix time: it fires at that wall-clock moment across a suspend or a clock step, at once on resume when the moment passed during it.
- It is asked. `Read.Provider` answers the latest reading, read first when it is older than the last turn end on the provider or there is none.

When nothing happens, nothing is read.

The configured Flow program is run with one argument, the `Send` datom, and reaches the Flow Nexus at the place its own configuration names: once Flow 0.53.3 is at main, Home's `~/.local/bin/flow-nexus` is Flow 0.53.3's `flow-nexus`, which reads `~/.config/nexus/flow-nexus.datom`, `{ main }`.

An explicit `Redeem.Codex` is treated as evidence that work is waiting. It never redeems a later available reset while an earlier one stands. Tests use recorded provider documents and a fake Flow peer; they do not read live providers or consume a real reset.
