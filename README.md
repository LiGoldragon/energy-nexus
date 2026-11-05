# Energy Nexus v1

Energy Nexus reads Claude and Codex quota usage, levels the typed readings, retains history across restarts, pushes level changes, sends configured reminders through Flow 0.51, compares each provider with its pace target, and applies the Codex reset rule.

The processes stay separated:

- `energy-nexus-daemon` owns typed Memory, Operation, public Signal, and meta Signal.
- `energy-translator` alone holds provider API words, JSON, credentials, Datom, process calls, and reminder prose.
- `energy-nexus` and `energy-nexus-meta` are text-only clients.
- `energy-pace` shows both providers' usage per period, target, standing, and daily share.
- `energy-nexus-poc` runs the separated processes together for an isolated transient deployment.

All process-to-process traffic behind the translator is length-prefixed, validated rkyv signal. A place names an instance; its runtime and state paths are passed once when its processes start. The text clients resolve the place name below the standard XDG runtime and state roots:

```sh
energy-nexus-poc main /run/user/1001/nexus/main ~/.local/state/nexus/main
energy-nexus main 'Read.Codex'
energy-nexus-meta main 'Configure.Polling.5'
energy-nexus-meta main 'Configure.Program.{ Flow /home/li/.local/bin/flow-0.51 }'
energy-pace main
```

An explicit `Redeem.Codex` is treated as evidence that work is waiting. It never redeems a later available reset while an earlier one stands. Tests use recorded provider documents and a fake Flow peer; they do not read live providers or consume a real reset.
