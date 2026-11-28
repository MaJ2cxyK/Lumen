use crate::approval;
use crate::db::{insert_message, log_event, new_id, now_ms};
use crate::detect;
use crate::error::{AppError, Result};
use crate::models::*;
use crate::orchestrator::{self, PipelineDeps};
use crate::state::AppState;
use crate::task_lifecycle::{self, TaskStatus, TaskTransition};
use crate::usage as usage_stats;
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tauri::State;
use tauri::{AppHandle, Emitter};

mod agents;
mod approvals;
mod data;
mod extensions;
mod history;
mod projects;
mod providers;
mod runs;
mod system;
mod task_runs;
mod tasks;
mod teams;
mod usage;

pub use agents::*;
pub use approvals::*;
pub use data::*;
pub use extensions::*;
pub use history::*;
pub use projects::*;
pub use providers::*;
pub use runs::*;
pub use system::*;
pub use task_runs::*;
pub use tasks::*;
pub use teams::*;
pub use usage::*;

fn conn(state: &State<'_, AppState>) -> Result<Connection> {
    state.conn()
}

fn map_message(r: &rusqlite::Row) -> rusqlite::Result<Message> {
    let metadata: String = r.get(10)?;
    Ok(Message {
        id: r.get(0)?,
        task_id: r.get(1)?,
        run_id: r.get(2)?,
        seat_id: r.get(3)?,
        agent_id: r.get(4)?,
        seat_name: r.get(5)?,
        agent_name: r.get(6)?,
        kind: r.get(7)?,
        role: r.get(8)?,
        content: r.get(9)?,
        metadata: serde_json::from_str(&metadata).unwrap_or(Value::Null),
        created_at: r.get(11)?,
    })
}

const MESSAGE_SQL: &str = "SELECT m.id, m.task_id, m.run_id, m.seat_id, m.agent_id,
        s.role_name, a.name, m.kind, m.role, m.content, m.metadata_json, m.created_at
     FROM messages m
     LEFT JOIN seats s ON s.id = m.seat_id
     LEFT JOIN agents a ON a.id = m.agent_id";

const TASK_SELECT: &str =
    "SELECT t.id, t.project_id, p.name, t.team_id, tm.name, t.title, t.description, t.acceptance,
        t.model, t.reasoning, COALESCE(t.mandate, 'ask'), t.work_dir,
        t.status, t.priority, t.assignee_seat_id, t.stage_index, t.stage_key,
        t.actual_tokens, t.actual_cost_micros, t.created_at, t.started_at, t.finished_at,
        t.review_pending, t.parent_id
     FROM tasks t LEFT JOIN projects p ON p.id = t.project_id
     LEFT JOIN teams tm ON tm.id = t.team_id";

fn map_task_row(r: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        project_id: r.get(1)?,
        project_name: r.get(2)?,
        team_id: r.get(3)?,
        team_name: r.get(4)?,
        parent_id: r.get(23)?,
        title: r.get(5)?,
        description: r.get(6)?,
        acceptance: r.get(7)?,
        model: r.get(8)?,
        reasoning: r.get(9)?,
        mandate: r.get(10)?,
        work_dir: r.get(11)?,
        status: r.get(12)?,
        priority: r.get(13)?,
        assignee_seat_id: r.get(14)?,
        stage_index: r.get(15)?,
        stage_key: r.get(16)?,
        actual_tokens: r.get(17)?,
        actual_cost_micros: r.get(18)?,
        created_at: r.get(19)?,
        started_at: r.get(20)?,
        finished_at: r.get(21)?,
        review_pending: r.get::<_, i64>(22)? != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_tasks() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT);
             CREATE TABLE teams (id TEXT PRIMARY KEY, name TEXT);
             CREATE TABLE tasks (
               id TEXT PRIMARY KEY, project_id TEXT NOT NULL, team_id TEXT,
               title TEXT NOT NULL, description TEXT, acceptance TEXT,
               model TEXT, reasoning TEXT, mandate TEXT, work_dir TEXT,
               status TEXT NOT NULL, priority INTEGER NOT NULL,
               assignee_seat_id TEXT, stage_index INTEGER NOT NULL DEFAULT 0,
               stage_key TEXT, actual_tokens INTEGER NOT NULL DEFAULT 0,
               actual_cost_micros INTEGER NOT NULL DEFAULT 0,
               created_at INTEGER NOT NULL, started_at INTEGER, finished_at INTEGER,
               review_pending INTEGER NOT NULL DEFAULT 0,
               parent_id TEXT, acceptance_json TEXT NOT NULL DEFAULT '[]'
             );
             INSERT INTO projects VALUES ('p1', '项目一');
             INSERT INTO tasks (id, project_id, title, description, status, priority, created_at)
             VALUES ('t1', 'p1', '一', 'a', 'inbox', 3, 100),
                    ('t2', 'p1', '二', 'b', 'inbox', 3, 100),
                    ('t3', 'p1', '三', 'c', 'inbox', 3, 90),
                    ('t4', 'p1', '四', 'd', 'completed', 3, 80),
                    ('t5', 'p1', '五', 'e', 'blocked', 2, 70);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn tasks_page_cursor_has_no_overlap_or_gap() {
        let conn = seed_tasks();
        let first = super::tasks::query_tasks_page(
            &conn,
            Some("p1".into()),
            None,
            None,
            None,
            None,
            Some(2),
        )
        .unwrap();
        assert_eq!(first.total, 5);
        assert_eq!(first.items.len(), 2);
        assert!(first.has_more);
        let cursor = first.next_cursor.clone().unwrap();

        let second = super::tasks::query_tasks_page(
            &conn,
            Some("p1".into()),
            None,
            None,
            None,
            Some(cursor),
            Some(2),
        )
        .unwrap();
        assert_eq!(second.total, 5);
        assert!(second.has_more);

        let ids: Vec<&str> = first
            .items
            .iter()
            .chain(second.items.iter())
            .map(|task| task.id.as_str())
            .collect();
        let unique: std::collections::HashSet<&&str> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "分页结果不应重复：{ids:?}");
    }

    #[test]
    fn tasks_page_filters_search_and_attention() {
        let conn = seed_tasks();
        let searched = super::tasks::query_tasks_page(
            &conn,
            None,
            None,
            Some("二".into()),
            None,
            None,
            Some(10),
        )
        .unwrap();
        assert_eq!(searched.total, 1);
        assert_eq!(searched.items[0].id, "t2");

        let attention =
            super::tasks::query_tasks_page(&conn, None, None, None, Some(true), None, Some(10))
                .unwrap();
        assert_eq!(attention.total, 4);
        assert_eq!(attention.status_counts.get("inbox"), Some(&3));
        assert_eq!(attention.review_count, 0);
    }

    #[test]
    fn attention_includes_paused_and_inactive_review_but_not_active_review() {
        let conn = seed_tasks();
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, description, status, priority, created_at)
             VALUES ('t6', 'p1', '六', 'f', 'paused', 3, 60),
                    ('t7', 'p1', '七', 'g', 'running', 3, 50)",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE tasks SET review_pending = 1 WHERE id IN ('t4', 't7')",
            [],
        )
        .unwrap();

        let attention =
            super::tasks::query_tasks_page(&conn, None, None, None, Some(true), None, Some(10))
                .unwrap();
        assert_eq!(attention.total, 6);
        assert_eq!(attention.status_counts.get("inbox"), Some(&3));
        assert_eq!(attention.status_counts.get("blocked"), Some(&1));
        assert_eq!(attention.status_counts.get("paused"), Some(&1));
        assert_eq!(attention.status_counts.get("completed"), Some(&1));
        assert_eq!(attention.status_counts.get("running"), None);
        assert_eq!(attention.review_count, 1);
    }

    #[test]
    fn project_delete_counts_only_active_tasks() {
        let conn = seed_tasks();
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, description, status, priority, created_at)
             VALUES ('t6', 'p1', '六', 'f', 'queued', 3, 60),
                    ('t7', 'p1', '七', 'g', 'running', 3, 50),
                    ('t8', 'p1', '八', 'h', 'waiting_approval', 3, 40),
                    ('t9', 'p1', '九', 'i', 'paused', 3, 30)",
            [],
        )
        .unwrap();

        let active = super::projects::project_active_task_count(&conn, "p1").unwrap();
        assert_eq!(active, 3);
    }
}
