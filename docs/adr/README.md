# Architecture decision records

Each record keeps one decision: its context, the decision, the alternatives weighed, the fallback if it fails, and its consequences. A record's decision is never rewritten; when it changes, the record gets a dated status update and, if needed, a new record that supersedes it.

## The records

- [ADR-0001: Workspace and dependencies](0001-workspace-and-dependencies.md)
- [ADR-0002: The core model](0002-the-core-model.md)
- [ADR-0003: The provider trait](0003-the-provider-trait.md)
- [ADR-0004: The job engine](0004-the-job-engine.md)
- [ADR-0005: Commands, keys and menus from one model](0005-commands-keys-and-menus.md)
- [ADR-0006: The plugin protocol](0006-the-plugin-protocol.md)
- [ADR-0007: Frontends over one core](0007-frontends-over-one-core.md)
- [ADR-0008: Safety rules](0008-safety-rules.md)

## The template

```markdown
# ADR-NNNN: Title

Status: Accepted, Weekday, Month D, YYYY (who).

## Context
## Decision
## Alternatives
## Fallback
## Consequences
## See also
```

Dates come from the machine's clock, written in full with the weekday.

## See also

- [Architecture](../dev/architecture.md)
- [The plan](../plan.md)
