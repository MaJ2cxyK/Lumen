//! Task lifecycle: the single place that decides which status transitions are legal.
//!
//! Status values are persisted as the existing lowercase strings; this module is a
//! behavioural seam, not a schema migration.

use crate::error::{AppError, Result};
use rusqlite::{params, Connection};

/// SQL literal for [`TaskStatus::is_active`]. Kept next to the enum so SQL and
/// Rust guards move together; a test asserts the two agree.
pub const ACTIVE_STATUSES_SQL: &str = "'queued','running','waiting_approval'";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Inbox,
    Queued,
    Running,
    WaitingApproval,
    Paused,
    Blocked,
    Completed,
    Cancelled,
}

impl TaskStatus {
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "inbox" => Self::Inbox,
            "queued" => Self::Queued,
            "running" => Self::Running,
            "waiting_approval" => Self::WaitingApproval,
            "paused" => Self::Paused,
            "blocked" => Self::Blocked,
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Statuses that own a live run slot or an in-flight pipeline.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::WaitingApproval)
    }

    /// Review actions must not race a stage that is still moving.
    pub fn is_review_locked(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::WaitingApproval)
    }

    /// Statuses a user may set directly from the task edit form.
    pub fn manual_targets() -> &'static [TaskStatus] {
        &[Self::Inbox, Self::Blocked, Self::Completed, Self::Cancelled]
    }

    /// Manual status edits never race a live run, and unknown target values
    /// are rejected by [`TaskStatus::parse`] before this predicate runs.
    pub fn can_change_manual(from: TaskStatus, to: TaskStatus) -> bool {
        !from.is_active() && Self::manual_targets().contains(&to)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TaskTransition<'a> {
    /// User dispatch / re-dispatch from any resting state.
    Dispatch {
        start_stage: i64,
    },
    Pause,
    Resume,
    /// Cancel is intentionally lenient: it always expires approvals, and only
    /// rewrites the status when the task was still active.
    Cancel,
    Retry,
    Reset,
    Reassign,
    Block,
    StartStage {
        stage: i64,
    },
    AdvanceStage {
        stage: i64,
    },
    WaitApproval {
        stage: i64,
    },
    ApproveRun {
        stage: i64,
    },
    ResumeAfterGate {
        approved_stage: i64,
    },
    Complete {
        stage: Option<i64>,
        result_json: &'a str,
    },
    StartupReconcile,
    Manual {
        status: TaskStatus,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionOutcome {
    pub changed: bool,
    pub expired: usize,
}

/// Legal transitions, independent of the database.
pub fn permits(from: TaskStatus, transition: TaskTransition<'_>) -> bool {
    match transition {
        // Re-dispatching an already queued task is allowed; a live run or a
        // pending approval must be stopped first.
        TaskTransition::Dispatch { .. } => {
            !matches!(from, TaskStatus::Running | TaskStatus::WaitingApproval)
        }
        TaskTransition::Pause => from.is_active(),
        TaskTransition::Resume => from == TaskStatus::Paused,
        // Cancel is lenient by design: it always expires approvals, and only
        // rewrites the status while the task is active or paused.
        TaskTransition::Cancel => true,
        TaskTransition::Retry => from == TaskStatus::Blocked,
        TaskTransition::Reset => !from.is_active(),
        TaskTransition::Reassign => matches!(from, TaskStatus::Paused | TaskStatus::Blocked),
        TaskTransition::Block => !matches!(from, TaskStatus::Blocked | TaskStatus::Paused),
        TaskTransition::StartStage { .. } => {
            matches!(from, TaskStatus::Queued | TaskStatus::Running)
        }
        TaskTransition::AdvanceStage { .. } => matches!(
            from,
            TaskStatus::Queued | TaskStatus::Running | TaskStatus::WaitingApproval
        ),
        TaskTransition::WaitApproval { .. } => {
            matches!(from, TaskStatus::Queued | TaskStatus::Running)
        }
        TaskTransition::ApproveRun { .. } => from == TaskStatus::WaitingApproval,
        TaskTransition::ResumeAfterGate { .. } => from == TaskStatus::WaitingApproval,
        TaskTransition::Complete { .. } => matches!(
            from,
            TaskStatus::Queued | TaskStatus::Running | TaskStatus::WaitingApproval
        ),
        TaskTransition::StartupReconcile => from.is_active(),
        TaskTransition::Manual { status } => TaskStatus::can_change_manual(from, status),
    }
}

/// Applies a transition with a compare-and-swap on the observed status, so a
/// concurrent stage change can never be overwritten by a stale decision.
///
/// The caller owns the surrounding transaction. Pending approvals are expired
/// only when the status actually moved (cancel is the exception: it always
/// clears approvals even if the task had already settled).
pub fn apply(
    conn: &Connection,
    task_id: &str,
    transition: TaskTransition<'_>,
    expiry_note: &str,
) -> Result<TransitionOutcome> {
    let current: String = conn
        .query_row(
            "SELECT status FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    let status = match TaskStatus::parse(&current) {
        Some(status) => status,
        // Unknown persisted values have exactly one repair path: an explicit
        // compare-and-swap reset back to inbox. Every other transition fails
        // closed below.
        None if matches!(transition, TaskTransition::Reset) => {
            let changed = conn.execute(
                "UPDATE tasks SET status = 'inbox', stage_index = 0, stage_key = NULL,
                        started_at = NULL, finished_at = NULL
                 WHERE id = ?1 AND status = ?2",
                params![task_id, current],
            )?;
            let expired = if changed > 0 {
                crate::approval::expire_pending_for_task(conn, task_id, expiry_note)?
            } else {
                0
            };
            return Ok(TransitionOutcome {
                changed: changed > 0,
                expired,
            });
        }
        None => return Err(AppError::Msg(format!("未知任务状态「{current}」"))),
    };
    if !permits(status, transition) {
        return Ok(TransitionOutcome {
            changed: false,
            expired: 0,
        });
    }

    let changed = match transition {
        TaskTransition::Dispatch { start_stage } => conn.execute(
            "UPDATE tasks SET status = 'queued', stage_index = ?1, finished_at = NULL
             WHERE id = ?2 AND status = ?3",
            params![start_stage, task_id, current],
        )?,
        TaskTransition::Pause => conn.execute(
            "UPDATE tasks SET status = 'paused' WHERE id = ?1 AND status = ?2",
            params![task_id, current],
        )?,
        TaskTransition::Resume | TaskTransition::Retry | TaskTransition::Reassign => conn.execute(
            "UPDATE tasks SET status = 'queued', finished_at = NULL
             WHERE id = ?1 AND status = ?2",
            params![task_id, current],
        )?,
        TaskTransition::Cancel => {
            if !matches!(status, TaskStatus::Paused) && !status.is_active() {
                0
            } else {
                conn.execute(
                    "UPDATE tasks SET status = 'blocked' WHERE id = ?1 AND status = ?2",
                    params![task_id, current],
                )?
            }
        }
        TaskTransition::Reset => conn.execute(
            "UPDATE tasks SET status = 'inbox', stage_index = 0, stage_key = NULL,
                    started_at = NULL, finished_at = NULL
             WHERE id = ?1 AND status = ?2",
            params![task_id, current],
        )?,
        TaskTransition::Block | TaskTransition::StartupReconcile => conn.execute(
            "UPDATE tasks SET status = 'blocked' WHERE id = ?1 AND status = ?2",
            params![task_id, current],
        )?,
        TaskTransition::StartStage { stage } | TaskTransition::AdvanceStage { stage } => conn
            .execute(
                "UPDATE tasks SET status = 'running', stage_index = ?1
                 WHERE id = ?2 AND status = ?3",
                params![stage, task_id, current],
            )?,
        TaskTransition::WaitApproval { stage } => conn.execute(
            "UPDATE tasks SET status = 'waiting_approval', stage_index = ?1
             WHERE id = ?2 AND status = ?3",
            params![stage, task_id, current],
        )?,
        TaskTransition::ApproveRun { stage } => conn.execute(
            "UPDATE tasks SET status = 'queued', stage_index = ?1
             WHERE id = ?2 AND status = ?3 AND stage_index = ?1",
            params![stage, task_id, current],
        )?,
        TaskTransition::ResumeAfterGate { approved_stage } => {
            let next = approved_stage + 1;
            conn.execute(
                "UPDATE tasks SET status = 'queued', stage_index = ?1
                 WHERE id = ?2 AND status = ?3 AND stage_index = ?4",
                params![next, task_id, current, approved_stage],
            )?
        }
        TaskTransition::Complete { stage, result_json } => match stage {
            Some(stage) => conn.execute(
                "UPDATE tasks SET status = 'completed', stage_index = ?1,
                        finished_at = ?2, result_json = ?3
                 WHERE id = ?4 AND status = ?5",
                params![stage, crate::db::now_ms(), result_json, task_id, current],
            )?,
            None => conn.execute(
                "UPDATE tasks SET status = 'completed', finished_at = ?1, result_json = ?2
                 WHERE id = ?3 AND status = ?4",
                params![crate::db::now_ms(), result_json, task_id, current],
            )?,
        },
        TaskTransition::Manual { status } => conn.execute(
            "UPDATE tasks SET status = ?1,
                    finished_at = CASE WHEN ?1 = 'completed'
                                       THEN COALESCE(finished_at, ?2) ELSE finished_at END
             WHERE id = ?3 AND status = ?4",
            params![status.as_str(), crate::db::now_ms(), task_id, current],
        )?,
    };
    let should_expire = match transition {
        TaskTransition::Dispatch { .. }
        | TaskTransition::Pause
        | TaskTransition::Resume
        | TaskTransition::Retry
        | TaskTransition::Reset
        | TaskTransition::Reassign
        | TaskTransition::Block
        | TaskTransition::Manual { .. } => changed > 0,
        TaskTransition::Cancel => true,
        _ => false,
    };
    let expired = if should_expire {
        crate::approval::expire_pending_for_task(conn, task_id, expiry_note)?
    } else {
        0
    };
    Ok(TransitionOutcome {
        changed: changed > 0,
        expired,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_connection;
    use serde_json::json;

    fn seed_task(conn: &Connection, id: &str, status: &str) -> String {
        conn.execute(
            "INSERT INTO projects (id, name, path, created_at, updated_at)
             VALUES ('p1', 'Project', 'C:/tmp', 1, 1)",
            [],
        )
        .ok();
        conn.execute(
            "INSERT INTO teams (id, project_id, name, created_at) VALUES ('tm1', 'p1', 'Team', 1)",
            [],
        )
        .ok();
        conn.execute(
            "INSERT INTO tasks (id, project_id, team_id, title, status, stage_index, created_at)
             VALUES (?1, 'p1', 'tm1', 'Task', ?2, 1, 1)",
            params![id, status],
        )
        .unwrap();
        "p1".to_string()
    }

    fn status_of(conn: &Connection, id: &str) -> (String, i64) {
        conn.query_row(
            "SELECT status, stage_index FROM tasks WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }

    fn pending_approval(conn: &Connection, task_id: &str) -> String {
        conn.execute(
            "INSERT INTO approvals (id, task_id, type, title, status, created_at)
             VALUES ('ap1', ?1, 'stage_gate', 'Gate', 'pending', 1)",
            params![task_id],
        )
        .unwrap();
        "ap1".to_string()
    }

    #[test]
    fn guard_matrix_matches_documented_lifecycle() {
        assert!(permits(TaskStatus::Paused, TaskTransition::Resume));
        assert!(!permits(TaskStatus::Running, TaskTransition::Reset));
        assert!(permits(
            TaskStatus::Queued,
            TaskTransition::Dispatch { start_stage: 0 }
        ));
        assert!(permits(TaskStatus::Blocked, TaskTransition::Retry));
        assert!(!permits(TaskStatus::Cancelled, TaskTransition::Resume));
        assert!(permits(
            TaskStatus::WaitingApproval,
            TaskTransition::ApproveRun { stage: 1 }
        ));
        assert!(!permits(
            TaskStatus::Queued,
            TaskTransition::ApproveRun { stage: 1 }
        ));
        assert!(!permits(
            TaskStatus::Running,
            TaskTransition::Dispatch { start_stage: 0 }
        ));
        assert!(permits(
            TaskStatus::Inbox,
            TaskTransition::Manual {
                status: TaskStatus::Completed
            }
        ));
        assert!(!permits(
            TaskStatus::Inbox,
            TaskTransition::Manual {
                status: TaskStatus::Running
            }
        ));
        assert!(!permits(
            TaskStatus::Running,
            TaskTransition::Manual {
                status: TaskStatus::Completed
            }
        ));
        assert!(TaskStatus::can_change_manual(
            TaskStatus::Paused,
            TaskStatus::Inbox
        ));
    }

    #[test]
    fn sql_active_status_list_matches_the_enum() {
        let all = [
            TaskStatus::Inbox,
            TaskStatus::Queued,
            TaskStatus::Running,
            TaskStatus::WaitingApproval,
            TaskStatus::Paused,
            TaskStatus::Blocked,
            TaskStatus::Completed,
            TaskStatus::Cancelled,
        ];
        let expected = all
            .iter()
            .filter(|status| status.is_active())
            .map(|status| format!("'{}'", status.as_str()))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(ACTIVE_STATUSES_SQL, expected);
    }

    #[test]
    fn resume_requeues_and_expires_pending_approvals() {
        let conn = test_connection();
        seed_task(&conn, "t1", "paused");
        pending_approval(&conn, "t1");
        let outcome = apply(&conn, "t1", TaskTransition::Resume, "resumed").unwrap();
        assert!(outcome.changed);
        assert_eq!(outcome.expired, 1);
        assert_eq!(status_of(&conn, "t1").0, "queued");
        let approval: String = conn
            .query_row("SELECT status FROM approvals WHERE id = 'ap1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(approval, "expired");
    }

    #[test]
    fn stale_dispatch_is_rejected_without_touching_the_task() {
        let conn = test_connection();
        seed_task(&conn, "t1", "running");
        let outcome = apply(
            &conn,
            "t1",
            TaskTransition::Dispatch { start_stage: 0 },
            "dispatch",
        )
        .unwrap();
        assert!(!outcome.changed);
        assert_eq!(status_of(&conn, "t1").0, "running");
    }

    #[test]
    fn unknown_status_can_only_be_repaired_by_reset() {
        let conn = test_connection();
        seed_task(&conn, "t1", "legacy_value");
        pending_approval(&conn, "t1");

        let manual = apply(
            &conn,
            "t1",
            TaskTransition::Manual {
                status: TaskStatus::Completed,
            },
            "manual",
        );
        assert!(manual.is_err());
        let dispatch = apply(
            &conn,
            "t1",
            TaskTransition::Dispatch { start_stage: 0 },
            "dispatch",
        );
        assert!(dispatch.is_err());
        assert_eq!(status_of(&conn, "t1").0, "legacy_value");

        let outcome = apply(&conn, "t1", TaskTransition::Reset, "reset").unwrap();
        assert!(outcome.changed);
        assert_eq!(outcome.expired, 1);
        assert_eq!(status_of(&conn, "t1"), ("inbox".to_string(), 0));
    }

    #[test]
    fn manual_status_change_is_rejected_while_a_run_is_live() {
        let conn = test_connection();
        seed_task(&conn, "t1", "running");
        let outcome = apply(
            &conn,
            "t1",
            TaskTransition::Manual {
                status: TaskStatus::Completed,
            },
            "manual",
        )
        .unwrap();
        assert!(!outcome.changed);
        assert_eq!(status_of(&conn, "t1").0, "running");
    }

    #[test]
    fn approving_a_run_requires_the_observed_stage() {
        let conn = test_connection();
        seed_task(&conn, "t1", "waiting_approval");
        let stale = apply(
            &conn,
            "t1",
            TaskTransition::ApproveRun { stage: 0 },
            "approve",
        )
        .unwrap();
        assert!(!stale.changed);
        let ok = apply(
            &conn,
            "t1",
            TaskTransition::ApproveRun { stage: 1 },
            "approve",
        )
        .unwrap();
        assert!(ok.changed);
        assert_eq!(status_of(&conn, "t1"), ("queued".to_string(), 1));
    }

    #[test]
    fn reset_clears_stage_progress() {
        let conn = test_connection();
        seed_task(&conn, "t1", "blocked");
        conn.execute(
            "UPDATE tasks SET stage_key = 'review', started_at = 10, finished_at = 20 WHERE id = 't1'",
            [],
        )
        .unwrap();
        let outcome = apply(&conn, "t1", TaskTransition::Reset, "reset").unwrap();
        assert!(outcome.changed);
        let row: (String, i64, Option<String>, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT status, stage_index, stage_key, started_at, finished_at
                 FROM tasks WHERE id = 't1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(row, ("inbox".to_string(), 0, None, None, None));
    }

    #[test]
    fn manual_completion_records_finish_time() {
        let conn = test_connection();
        seed_task(&conn, "t1", "blocked");
        let outcome = apply(
            &conn,
            "t1",
            TaskTransition::Manual {
                status: TaskStatus::Completed,
            },
            "manual",
        )
        .unwrap();
        assert!(outcome.changed);
        let (status, finished): (String, Option<i64>) = conn
            .query_row(
                "SELECT status, finished_at FROM tasks WHERE id = 't1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "completed");
        assert!(finished.is_some());
    }

    #[test]
    fn completion_from_an_active_stage_is_recorded_once() {
        let conn = test_connection();
        seed_task(&conn, "t1", "running");
        let result = json!({ "stages": 2 }).to_string();
        let outcome = apply(
            &conn,
            "t1",
            TaskTransition::Complete {
                stage: None,
                result_json: &result,
            },
            "complete",
        )
        .unwrap();
        assert!(outcome.changed);
        assert_eq!(status_of(&conn, "t1").0, "completed");
        let stored: Option<String> = conn
            .query_row("SELECT result_json FROM tasks WHERE id = 't1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored.as_deref(), Some(result.as_str()));
    }
}
