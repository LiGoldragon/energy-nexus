# Energy Nexus v1

Energy Nexus reads Claude and Codex quota usage, levels the typed readings, retains history across restarts, pushes level changes, sends configured reminders through Flow, compares each provider with its pace target, and applies the Codex reset rule.

The processes stay separated:

- `energy-nexus-daemon` owns typed Memory, Operation, public Signal, and meta Signal.
- `energy-translator` alone holds provider API words, JSON, credentials, Datom, process calls, and reminder prose.
- `energy-nexus` and `energy-nexus-meta` are text-only clients.
- `energy-pace` shows both providers' usage per period, target, standing, and daily share.
- `energy-nexus-poc` runs the separated processes together for an isolated transient deployment; a deployment runs each process as its own unit.

All process-to-process traffic behind the translator is length-prefixed, validated rkyv signal. A place names an instance; its runtime and state paths are passed once when its processes start. The text clients resolve the place name below the standard XDG runtime and state roots:

```sh
energy-nexus-daemon main /run/user/1001/nexus/main ~/.local/state/nexus/main
energy-translator main /run/user/1001/nexus/main ~/.local/state/nexus/main
energy-nexus main 'Read.Codex'
energy-nexus-meta main 'Configure.Polling.5'
energy-nexus-meta main 'Configure.Program.{ Flow /home/li/.local/bin/flow }'
energy-pace main
```

A place's two directories are shared with every other Nexus at the place and are declared by the environment (CriomOS-home declares them as user tmpfiles rules); no Energy process makes or removes them, and a process whose place is absent does not start. Each process makes only its own sockets there and removes only those when it is stopped (SIGTERM or SIGINT): the daemon `energy.sock` and `energy-meta.sock`, the translator `energy-translator.sock`, `energy-meta-translator.sock` and `energy-emission.sock`. Its memory is `energy.rkyv` (the daemon) and `energy-translator.rkyv` (the translator's configured programs) in State.

Each process sends `READY=1` to `$NOTIFY_SOCKET`, when it is set, once all its sockets listen, so each runs as a systemd unit of `Type=notify`; the translator's unit is ordered after the daemon's and bound to it.

The configured Flow program is run with one argument, the `Send` datom, and reaches the Flow Nexus at the place it names: once Flow 0.53.3 is at main, Home's `~/.local/bin/flow` is `flow main`.

An explicit `Redeem.Codex` is treated as evidence that work is waiting. It never redeems a later available reset while an earlier one stands. Tests use recorded provider documents and a fake Flow peer; they do not read live providers or consume a real reset.
