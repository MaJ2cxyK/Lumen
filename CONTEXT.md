# Lumen

Lumen is a local-first multi-agent workbench. Tasks move through a persisted lifecycle, while the board presents that lifecycle as coarser operational stages.

## Language

**Task status**:
The persisted lifecycle state of a task. A task has exactly one of `inbox`, `queued`, `running`, `waiting_approval`, `paused`, `blocked`, `completed`, or `cancelled`.
_Avoid_: task phase, task stage

**Unknown task status**:
A persisted task value outside the task-status vocabulary. It is displayed as Unknown and may only be repaired to `inbox`.

**Task stage**:
A board grouping derived from task status. Task stages are `inbox`, `queued`, `running`, `waiting_approval`, `paused`, `blocked`, and `closed`. A stage is not persisted.
_Avoid_: task status, board status

**Closed**:
The terminal task stage that groups `completed` and `cancelled` tasks. The status badge still distinguishes the two.

**Needs you**:
The task requires user action. This is true for `inbox`, `waiting_approval`, `paused`, and `blocked`, or when an inactive task has a pending review.

**Run status**:
The lifecycle state of one execution attempt. Run status includes values that are not task statuses, including `paused_budget` and `failed`.
_Avoid_: task status

**Review pending**:
An inactive task's workspace still contains changes that have not been applied or reverted.
