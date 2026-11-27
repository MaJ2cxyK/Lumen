# ADR 0001: Task lifecycle owns all status transitions

- Status: Accepted
- Date: 2026-11-26

## Context

Task status values were written from commands, the orchestrator, approvals,
budget handling, and startup recovery. Every call site repeated its own
`IN (...)` list, so guards drifted: manual status changes, cancellation, and
review locking each had a slightly different idea of which statuses were
"active". Review actions could also race a stage that was still moving.

## Decision

`src-tauri/src/task_lifecycle.rs` is the single seam for task status policy:

- `TaskStatus` maps the existing persisted strings (no value migration).
- `permits(from, transition)` is the guard matrix for every transition.
- `apply(conn, task_id, transition, note)` performs a compare-and-swap on the
  observed status inside the caller's transaction, then expires pending
  approvals only for transitions that invalidate them.
- `ACTIVE_STATUSES_SQL` mirrors `is_active` for the few queries that must
  filter in SQL; a unit test keeps the two in sync.

The frontend mirrors the same guards in `src/lib/taskLifecycle.ts` so buttons,
board stages, and backend enforcement cannot disagree. ADR-0002 narrows the
task-status vocabulary and defines the unknown-status repair path.

## Consequences

- A stale decision can no longer overwrite a newer status: the conditional
  update changes zero rows and the caller reports a conflict.
- Adding a status or transition requires touching one Rust module, one TS
  module, and the tests that pin the matrix.
- Stage-gate approvals carry the observed `stage_index` in the transition,
  making "approve the wrong stage" impossible.
- Status strings remain stable, so databases and existing releases keep
  working without a migration.
