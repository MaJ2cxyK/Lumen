# Task status is persisted; stage is derived

- Status: Accepted
- Date: 2026-11-27

## Context

Task status had drifted into several frontend maps and a duplicated review guard.
`failed` and `paused_budget` were present in the task-status vocabulary but had
no task lifecycle writer, while the board could silently omit or mislabel statuses.

## Decision

Task status is the persisted lifecycle vocabulary: `inbox`, `queued`, `running`,
`waiting_approval`, `paused`, `blocked`, `completed`, and `cancelled`.

The board stage is a derived grouping: `inbox`, `queued`, `running`,
`waiting_approval`, `paused`, `blocked`, and `closed`. `closed` groups
`completed` and `cancelled` while preserving their distinct status labels.

`failed` and `paused_budget` remain run statuses only. Historical task values
outside the vocabulary are shown as unknown rather than Inbox.

Manual task-status changes are allowed only while the task is inactive, and
all backend and browser-preview writes use the same lifecycle guard.
An unknown task status has one repair path: an explicit compare-and-swap reset
to `inbox`; every other lifecycle action fails closed.

## Consequences

- Persisted status strings do not migrate.
- Board grouping no longer drops known or unknown statuses.
- The review guard consumes the lifecycle predicate instead of repeating status literals.
- Task-level failure remains `blocked`; a distinct budget-paused task state requires a future product decision and a real writer.
- Unknown legacy statuses remain visible and can be quarantined without pretending they completed.
