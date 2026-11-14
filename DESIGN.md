# Design assumptions

- `Percent` in Meta is the same library type imported by the ordinary contract; its missing source import is compiler bookkeeping only.
- Explicit `Redeem.Codex` is evidence of waiting work for the provider-denied branch of the rule.
- A subscription receives `Subscribed` immediately and `Changed` whenever the level moves; readings that keep the same level remain available through `Read` and History rather than producing the declared `Changed` reaction.
- Provider parsing and every provider/API/Flow word live in `energy-nexus-translator`. The daemon dependency closure has neither Datom nor `serde_json`.
- The owner's local time is the system's time zone, read as the daemon computes pace.
- Pace stays per window (`Read.{ Reading Vector<Pace> }`), each window's pace counted in useful hours; the vision's Pace has no window name.
- An overlap the vision leaves open is settled by specificity and then by order: two spans naming as many days and as many hours, the later configured; two overrides, the shorter, then the later configured.
- A span whose To is before its From runs past midnight; From equal to To is refused.
- The baseline is 100 until configured, so a Nexus with no schedule paces as before.
- The Ethos `Override` names its instants `Since` and `Until`: ethos-zero refuses `From` declared both as an `Hour` (in `Span`) and as an `Instant`. The Rust fields and the datom text are positional and unchanged.
