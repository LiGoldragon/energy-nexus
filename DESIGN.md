# Design assumptions

- `Percent` in Meta is the same library type imported by the ordinary contract; its missing source import is compiler bookkeeping only.
- Explicit `Redeem.Codex` is evidence of waiting work for the provider-denied branch of the rule.
- A subscription receives `Subscribed` immediately and `Changed` whenever the level moves; readings that keep the same level remain available through `Read` and History rather than producing the declared `Changed` reaction.
- Provider parsing and every provider/API/Flow word live in `energy-nexus-translator`. The daemon dependency closure has neither Datom nor `serde_json`.
