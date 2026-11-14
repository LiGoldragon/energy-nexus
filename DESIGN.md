# Design assumptions

- `Percent` in Meta is the same library type imported by the ordinary contract; its missing source import is compiler bookkeeping only.
- Explicit `Redeem.Codex` is evidence of waiting work for the provider-denied branch of the rule.
- A subscription receives `Subscribed` immediately and `Changed` whenever the level moves; readings that keep the same level remain available through `Read` and History rather than producing the declared `Changed` reaction.
- Provider parsing and every provider/API/Flow word live in `energy-nexus-translator`. The daemon dependency closure has neither Datom nor `serde_json`.
- Pace stays per window (`Read.{ Reading Vector<Pace> }`), each window's pace counted in useful hours, with its `Name` (Fable's ruling 6).
- The owner's zone is `Configure.TimeZone`, the system's until one is set (ruling 3); a configured name is resolved in jiff's bundled zone database, so it resolves alike on every host and in every build.
- An overlap the vision leaves open is settled by specificity and then by order: two spans naming as many days and as many hours, the later configured; two overrides, the shorter, then the later configured.
- A span whose To is before its From runs past midnight; From equal to To is refused.
- The baseline is 100 until configured, so a Nexus with no schedule paces as before. Usefulness and the baseline are each a Percent, 100 the ceiling (ruling 4).
- An override takes Since and Until (ruling 2); one whose Until has passed is removed at the next change of the schedule (ruling 5), the change being any Span, RemoveSpan, Override, RemoveOverride, Baseline or TimeZone setting.
- `SpendBy.{ Hours Percent }`: the target falls that many whole hours before the window resets, and so recurs with every reset; expected is that percent of the useful time from the window's start to the target elapsed, and the share per useful hour is the percent left to it per useful hour left. A target is the Week window alone; the Session takes none, and Claude's FableWeek is paced Even. A Week shorter than the hours (accepted before a reading was held) is paced as Even; `Configure` refuses hours beyond the Week as the held reading says. A Session used to 100 percent stands `Spent`: a fault in the pace of work, told once per session window to whoever a reminder names.
