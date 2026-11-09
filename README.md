# Energy Nexus v1

Energy Nexus reads Claude and Codex quota usage on events, levels the typed readings, retains history across restarts, pushes level changes, sends configured reminders through Flow, compares each provider with its pace target, and applies the Codex reset rule.

The processes stay separated:

- `energy-nexus-daemon` owns typed Memory, Operation, public Signal, and meta Signal.
- `energy-translator` alone holds provider API words, JSON, credentials, Datom, process calls, and reminder prose.
- `energy-nexus` and `energy-nexus-meta` are text-only clients.
- `energy-pace` shows both providers' usage per period, target, standing, and daily share.

All process-to-process traffic behind the translator is length-prefixed, validated rkyv signal. A place names an instance; its runtime and state paths are passed once when its processes start. The text clients resolve the place name below the standard XDG runtime and state roots:

```sh
energy-nexus-daemon main /run/user/1001/nexus/main ~/.local/state/nexus/main
energy-translator main /run/user/1001/nexus/main ~/.local/state/nexus/main
energy-nexus main 'Read.Codex'
energy-nexus-meta main 'Configure.RedeemExpiring.720'
energy-nexus-meta main 'Configure.Program.{ Flow /home/li/.local/bin/flow }'
energy-pace main
```

A place's two directories are shared with every other Nexus at the place and are declared by the environment (CriomOS-home declares them as user tmpfiles rules); no Energy process makes or removes them, and a process whose place is absent does not start. Each process makes only its own sockets there and removes only those when it is stopped (SIGTERM or SIGINT): the daemon `energy.sock` and `energy-meta.sock`, the translator `energy-translator.sock`, `energy-meta-translator.sock` and `energy-emission.sock`. Its memory is `energy.rkyv` (the daemon) and `energy-translator.rkyv` (the translator's configured programs) in State. Each file is an envelope (its kind, a format version, the archive's length) around an rkyv archive. A file written before the envelope, by the proof of concept, is upgraded on open; every reading and setting is carried, and each reminder, whose recipient named a Flow layer, is named on the daemon's standard error to be configured again with a model. Only a missing file makes a memory new; a file that cannot be read keeps the process from starting and is left as it is. Every change is saved before it is used, and a failed save is answered as a refusal.

Each process sends `READY=1` to `$NOTIFY_SOCKET`, when it is set, once all its sockets listen, so each runs as a systemd unit of `Type=notify`. The daemon's unit is ordered after the translator's and bound to it, and the translator's is ordered after the Flow Nexus's unit at the place and part of it: Flow (re)started brings the translator up after Flow's readiness, then the daemon after the translator's.

Energy reads a provider on three events and never on an interval:

- A turn ends on it. The daemon holds `Emit.TurnEnds` open on the translator, which subscribes to the Flow Nexus at the place (`Observe.TurnEnds` on `flow.sock`, signal) and hands each turn end on as the provider its harness spends. A turn end that arrives while a read of that provider is in flight owes one more read when it ends: a burst is one read and at most one follow-up. The subscription's opening, the last turn end on each provider, reads each provider whose latest reading is older than it; this is the first reading at start. When Flow goes the subscription ends, and it is opened again when Energy restarts with Flow.
- Its window resets. Each reading names when its provider's next window resets; one timer is set for that moment and reads once when it fires, and a newer reading replaces it.
- It is asked. `Read.Provider` answers the latest reading, read first when it is older than the last turn end on the provider or there is none.

When nothing happens, nothing is read.

The configured Flow program is run with one argument, the `Send` datom, and reaches the Flow Nexus at the place it names: once Flow 0.53.3 is at main, Home's `~/.local/bin/flow` is `flow main`.

An explicit `Redeem.Codex` is treated as evidence that work is waiting. It never redeems a later available reset while an earlier one stands. Tests use recorded provider documents and a fake Flow peer; they do not read live providers or consume a real reset.
