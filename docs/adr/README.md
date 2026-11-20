# Architecture decision records

`docs/PLAN.md` is the source of truth; ADRs record the reasoning behind its structural decisions and how each held up in
implementation. Format: Status, Context, Decision, Consequences, What would change it, Implementation notes.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-shell-and-playback-integration.md) | Shell and playback integration | Accepted (Linux conditional, see notes) |
| [0002](0002-no-custom-av-pipeline.md) | No custom A/V pipeline | Accepted |
| [0003](0003-sqlite-read-model.md) | SQLite is the read model | Accepted |
| 0004 | Reserved, unused | The plan defines no ADR-0004 |
| 0005 | Reserved, unused | The plan defines no ADR-0005 |
| [0006](0006-bundled-libmpv.md) | libmpv bundled in every installer | Accepted |

Numbers 0004 and 0005 are kept free so the numbering the plan already uses (0006) stays stable. New ADRs start at 0007.
