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
mod system;
mod teams;
mod usage;

pub use agents::*;
pub use approvals::*;
pub use data::*;
pub use extensions::*;
pub use history::*;
pub use projects::*;
pub use providers::*;
pub use system::*;
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

#[tauri::command]
pub fn tasks_list(
    state: State<'_, AppState>,
    project_id: Option<String>,
    status: Option<String>,
) -> Result<Vec<Task>> {
    let conn = conn(&state)?;
    let mut sql = String::from(TASK_SELECT);
    sql.push_str(" WHERE 1=1");
    let mut values: Vec<SqlValue> = Vec::new();
    if let Some(pid) = project_id {
        sql.push_str(" AND t.project_id = ?");
        values.push(pid.into());
    }
    if let Some(st) = status {
        sql.push_str(" AND t.status = ?");
        values.push(st.into());
    }
    sql.push_str(" ORDER BY t.priority ASC, t.created_at DESC LIMIT 300");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values), map_task_row)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// 游标格式：`priority:createdAt:id`，与列表排序（优先级升序、创建时间倒序、ID 倒序）一致。
fn parse_task_cursor(raw: &str) -> Option<(i64, i64, String)> {
    let mut parts = raw.splitn(3, ':');
    let priority = parts.next()?.parse().ok()?;
    let created_at = parts.next()?.parse().ok()?;
    let id = parts.next()?.trim();
    if id.is_empty() {
        return None;
    }
    Some((priority, created_at, id.to_string()))
}

fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// 任务分页查询：看板用它替代固定 300 条的全量拉取。
fn query_tasks_page(
    conn: &Connection,
    project_id: Option<String>,
    status: Option<String>,
    query: Option<String>,
    attention_only: Option<bool>,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<TaskPage> {
    let mut filter = String::from(" WHERE 1=1");
    let mut values: Vec<SqlValue> = Vec::new();
    if let Some(pid) = project_id.filter(|v| !v.trim().is_empty()) {
        filter.push_str(" AND t.project_id = ?");
        values.push(pid.into());
    }
    if let Some(st) = status.filter(|v| !v.trim().is_empty()) {
        filter.push_str(" AND t.status = ?");
        values.push(st.into());
    }
    if let Some(q) = query
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        let pattern = like_pattern(&q);
        filter.push_str(
            " AND (t.title LIKE ? ESCAPE '\\' OR COALESCE(t.description, '') LIKE ? ESCAPE '\\')",
        );
        values.push(pattern.clone().into());
        values.push(pattern.into());
    }
    if attention_only.unwrap_or(false) {
        filter.push_str(&format!(
            " AND (t.status IN ('inbox', 'waiting_approval', 'paused', 'blocked')
                   OR (t.review_pending = 1 AND t.status NOT IN ({})))",
            task_lifecycle::ACTIVE_STATUSES_SQL
        ));
    }

    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM tasks t{filter}"),
        params_from_iter(values.clone()),
        |r| r.get(0),
    )?;
    let status_counts: BTreeMap<String, i64> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT t.status, COUNT(*) FROM tasks t{filter} GROUP BY t.status"
        ))?;
        let rows = stmt.query_map(params_from_iter(values.clone()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?
    };
    let review_count: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM tasks t{filter} AND t.review_pending = 1
             AND t.status NOT IN ({})",
            task_lifecycle::ACTIVE_STATUSES_SQL
        ),
        params_from_iter(values.clone()),
        |r| r.get(0),
    )?;

    let page_limit = limit.unwrap_or(60).clamp(1, 200);
    let mut sql = format!("{TASK_SELECT}{filter}");
    let mut page_values = values;
    if let Some(raw) = cursor.filter(|v| !v.trim().is_empty()) {
        if let Some((priority, created_at, last_id)) = parse_task_cursor(&raw) {
            sql.push_str(
                " AND (t.priority > ? OR (t.priority = ? AND (t.created_at < ? OR (t.created_at = ? AND t.id < ?))))",
            );
            page_values.push(priority.into());
            page_values.push(priority.into());
            page_values.push(created_at.into());
            page_values.push(created_at.into());
            page_values.push(last_id.into());
        }
    }
    sql.push_str(" ORDER BY t.priority ASC, t.created_at DESC, t.id DESC LIMIT ?");
    page_values.push((page_limit + 1).into());

    let mut stmt = conn.prepare(&sql)?;
    let mut items = stmt
        .query_map(params_from_iter(page_values), map_task_row)?
        .collect::<rusqlite::Result<Vec<Task>>>()?;
    let has_more = items.len() > page_limit as usize;
    if has_more {
        items.truncate(page_limit as usize);
    }
    let next_cursor = if has_more {
        items
            .last()
            .map(|task| format!("{}:{}:{}", task.priority, task.created_at, task.id))
    } else {
        None
    };

    Ok(TaskPage {
        items,
        total,
        status_counts,
        review_count,
        has_more,
        next_cursor,
    })
}

#[tauri::command]
pub fn tasks_page(
    state: State<'_, AppState>,
    project_id: Option<String>,
    status: Option<String>,
    query: Option<String>,
    attention_only: Option<bool>,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<TaskPage> {
    let conn = conn(&state)?;
    query_tasks_page(
        &conn,
        project_id,
        status,
        query,
        attention_only,
        cursor,
        limit,
    )
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
        let first =
            query_tasks_page(&conn, Some("p1".into()), None, None, None, None, Some(2)).unwrap();
        assert_eq!(first.total, 5);
        assert_eq!(first.items.len(), 2);
        assert!(first.has_more);
        let cursor = first.next_cursor.clone().unwrap();

        let second = query_tasks_page(
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
        let searched =
            query_tasks_page(&conn, None, None, Some("二".into()), None, None, Some(10)).unwrap();
        assert_eq!(searched.total, 1);
        assert_eq!(searched.items[0].id, "t2");

        let attention =
            query_tasks_page(&conn, None, None, None, Some(true), None, Some(10)).unwrap();
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
            query_tasks_page(&conn, None, None, None, Some(true), None, Some(10)).unwrap();
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

#[tauri::command]
pub fn task_get(state: State<'_, AppState>, id: String) -> Result<Task> {
    let conn = conn(&state)?;
    let task = conn.query_row(
        &format!("{TASK_SELECT} WHERE t.id = ?1"),
        params![id],
        map_task_row,
    )?;
    Ok(task)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn task_create(
    state: State<'_, AppState>,
    project_id: String,
    team_id: Option<String>,
    title: String,
    description: Option<String>,
    acceptance: Option<String>,
    model: Option<String>,
    reasoning: Option<String>,
    mandate: Option<String>,
    isolated: Option<bool>,
    priority: Option<i64>,
    parent_id: Option<String>,
) -> Result<String> {
    if title.trim().is_empty() {
        return Err("任务标题不能为空".into());
    }
    let conn = conn(&state)?;
    let id = new_id();
    let project_path: String = conn
        .query_row(
            "SELECT path FROM projects WHERE id = ?1",
            params![project_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("项目不存在".into()))?;
    // 子任务必须与父任务同项目；未显式选团队时沿用父任务团队。
    let mut team_id = team_id;
    if let Some(parent_id) = parent_id.as_deref() {
        let parent: (String, Option<String>) = conn
            .query_row(
                "SELECT project_id, team_id FROM tasks WHERE id = ?1",
                params![parent_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| AppError::Msg("父任务不存在".into()))?;
        if parent.0 != project_id {
            return Err("子任务必须与父任务属于同一项目".into());
        }
        if team_id.is_none() {
            team_id = parent.1;
        }
    }
    let mandate = if mandate.as_deref() == Some("auto") {
        "auto"
    } else {
        "ask"
    };
    // 隔离工作区只在任务创建时创建一次；失败直接报错，不落任务，避免静默写入项目目录。
    let work_dir = if isolated.unwrap_or(false) {
        Some(crate::worktree::create_detached_worktree(
            std::path::Path::new(&project_path),
            &state.base_dir,
            &id,
        )?)
    } else {
        None
    };
    conn.execute(
        "INSERT INTO tasks (id, project_id, team_id, title, description, acceptance, model, reasoning, mandate, work_dir, priority, parent_id, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'inbox', ?13)",
        params![
            id,
            project_id,
            team_id,
            title.trim(),
            description,
            acceptance,
            model,
            reasoning,
            mandate,
            work_dir,
            priority.unwrap_or(2),
            parent_id,
            now_ms()
        ],
    )?;
    log_event(
        &conn,
        Some(&project_id),
        Some(&id),
        None,
        "task.created",
        Some("user"),
        json!({ "title": title, "parentId": parent_id }),
    )?;
    Ok(id)
}

/// 子任务列表：任务详情页的协作件区域。
#[tauri::command]
pub fn task_children(state: State<'_, AppState>, task_id: String) -> Result<Vec<Task>> {
    let conn = conn(&state)?;
    let mut stmt = conn.prepare(&format!(
        "{TASK_SELECT} WHERE t.parent_id = ?1 ORDER BY t.created_at ASC"
    ))?;
    let rows = stmt.query_map(params![task_id], map_task_row)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// 依赖检查：存在未完成的依赖任务时不允许启动，避免产出建立在缺失前置上。
fn ensure_dependencies_done(conn: &Connection, task_id: &str) -> Result<()> {
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM task_dependencies d
         JOIN tasks t ON t.id = d.depends_on_id
         WHERE d.task_id = ?1 AND t.status <> 'completed'",
        params![task_id],
        |r| r.get(0),
    )?;
    if pending > 0 {
        return Err(format!("还有 {pending} 个依赖任务未完成，完成后才能运行").into());
    }
    Ok(())
}

#[tauri::command]
pub fn task_dependencies(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<TaskDependency>> {
    let conn = conn(&state)?;
    let mut stmt = conn.prepare(
        "SELECT d.task_id, d.depends_on_id, t.title, t.status
         FROM task_dependencies d JOIN tasks t ON t.id = d.depends_on_id
         WHERE d.task_id = ?1 ORDER BY t.created_at ASC",
    )?;
    let rows = stmt.query_map(params![task_id], |r| {
        Ok(TaskDependency {
            task_id: r.get(0)?,
            depends_on_id: r.get(1)?,
            title: r.get(2)?,
            status: r.get(3)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn task_dependency_add(
    state: State<'_, AppState>,
    task_id: String,
    depends_on_id: String,
) -> Result<()> {
    if task_id == depends_on_id {
        return Err("任务不能依赖自身".into());
    }
    let conn = conn(&state)?;
    let project_id: String = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    let dep_project: String = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![depends_on_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("依赖任务不存在".into()))?;
    if project_id != dep_project {
        return Err("只能依赖同一项目下的任务".into());
    }
    // 沿依赖链检查 depends_on 是否已能回到当前任务，避免形成环。
    let cycle: i64 = conn.query_row(
        "WITH RECURSIVE chain(id) AS (
           SELECT depends_on_id FROM task_dependencies WHERE task_id = ?2
           UNION
           SELECT d.depends_on_id FROM task_dependencies d JOIN chain c ON d.task_id = c.id
         )
         SELECT COUNT(*) FROM chain WHERE id = ?1",
        params![task_id, depends_on_id],
        |r| r.get(0),
    )?;
    if cycle > 0 {
        return Err("该依赖会形成循环".into());
    }
    conn.execute(
        "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id, created_at)
         VALUES (?1, ?2, ?3)",
        params![task_id, depends_on_id, now_ms()],
    )?;
    log_event(
        &conn,
        Some(&project_id),
        Some(&task_id),
        None,
        "task.dependency_added",
        Some("user"),
        json!({ "dependsOnId": depends_on_id }),
    )?;
    Ok(())
}

#[tauri::command]
pub fn task_dependency_remove(
    state: State<'_, AppState>,
    task_id: String,
    depends_on_id: String,
) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute(
        "DELETE FROM task_dependencies WHERE task_id = ?1 AND depends_on_id = ?2",
        params![task_id, depends_on_id],
    )?;
    let project_id: Option<String> = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .optional()?;
    log_event(
        &conn,
        project_id.as_deref(),
        Some(&task_id),
        None,
        "task.dependency_removed",
        Some("user"),
        json!({ "dependsOnId": depends_on_id }),
    )?;
    Ok(())
}

#[tauri::command]
pub fn task_checklist_get(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<ChecklistItem>> {
    let conn = conn(&state)?;
    let raw: String = conn
        .query_row(
            "SELECT COALESCE(acceptance_json, '[]') FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

#[tauri::command]
pub fn task_checklist_save(
    state: State<'_, AppState>,
    task_id: String,
    items: Vec<ChecklistItem>,
) -> Result<Vec<ChecklistItem>> {
    let conn = conn(&state)?;
    let mut cleaned = Vec::new();
    for item in items {
        let text = item.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        cleaned.push(ChecklistItem {
            id: if item.id.trim().is_empty() {
                new_id()
            } else {
                item.id
            },
            text,
            done: item.done,
        });
    }
    let raw = serde_json::to_string(&cleaned)
        .map_err(|e| AppError::Msg(format!("验收清单序列化失败：{e}")))?;
    let changed = conn.execute(
        "UPDATE tasks SET acceptance_json = ?1 WHERE id = ?2",
        params![raw, task_id],
    )?;
    if changed == 0 {
        return Err("任务不存在".into());
    }
    let project_id: Option<String> = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .optional()?;
    let done = cleaned.iter().filter(|item| item.done).count();
    log_event(
        &conn,
        project_id.as_deref(),
        Some(&task_id),
        None,
        "task.checklist_updated",
        Some("user"),
        json!({ "total": cleaned.len(), "done": done }),
    )?;
    Ok(cleaned)
}

#[tauri::command]
pub fn task_attachments(state: State<'_, AppState>, task_id: String) -> Result<Vec<Attachment>> {
    let conn = conn(&state)?;
    let mut stmt = conn.prepare(
        "SELECT id, task_id, name, path, size, created_at FROM attachments
         WHERE task_id = ?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![task_id], |r| {
        Ok(Attachment {
            id: r.get(0)?,
            task_id: r.get(1)?,
            name: r.get(2)?,
            path: r.get(3)?,
            size: r.get(4)?,
            created_at: r.get(5)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

/// 复制本地文件到数据目录，避免运行期外部文件被移动导致附件失效。
#[tauri::command]
pub fn task_attachment_add(
    state: State<'_, AppState>,
    task_id: String,
    source_path: String,
) -> Result<Attachment> {
    let src = std::path::PathBuf::from(source_path.trim());
    if !src.is_file() {
        return Err("文件不存在或不是普通文件".into());
    }
    let conn = conn(&state)?;
    let project_id: String = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    let size = std::fs::metadata(&src).map(|m| m.len() as i64).unwrap_or(0);
    if size > 50 * 1024 * 1024 {
        return Err("单个附件不能超过 50 MB".into());
    }
    let name = src
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("attachment")
        .to_string();
    let dir = state.base_dir.join("attachments").join(&task_id);
    std::fs::create_dir_all(&dir)?;
    let id = new_id();
    let dest = dir.join(format!("{}_{}", id, safe_file_name(&name)));
    std::fs::copy(&src, &dest)?;
    let attachment = Attachment {
        id,
        task_id: task_id.clone(),
        name,
        path: dest.to_string_lossy().to_string(),
        size,
        created_at: now_ms(),
    };
    conn.execute(
        "INSERT INTO attachments (id, task_id, name, path, size, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            attachment.id,
            attachment.task_id,
            attachment.name,
            attachment.path,
            attachment.size,
            attachment.created_at
        ],
    )?;
    log_event(
        &conn,
        Some(&project_id),
        Some(&task_id),
        None,
        "task.attachment_added",
        Some("user"),
        json!({ "name": attachment.name, "size": size }),
    )?;
    Ok(attachment)
}

#[tauri::command]
pub fn task_attachment_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    let row: Option<(String, String, String)> = conn
        .query_row(
            "SELECT task_id, path, name FROM attachments WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((task_id, path, name)) = row else {
        return Ok(());
    };
    conn.execute("DELETE FROM attachments WHERE id = ?1", params![id])?;
    let _ = std::fs::remove_file(&path);
    let project_id: Option<String> = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .optional()?;
    log_event(
        &conn,
        project_id.as_deref(),
        Some(&task_id),
        None,
        "task.attachment_removed",
        Some("user"),
        json!({ "name": name }),
    )?;
    Ok(())
}

#[tauri::command]
pub fn task_attachment_reveal(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    let path: String = conn
        .query_row(
            "SELECT path FROM attachments WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("附件不存在".into()))?;
    #[cfg(windows)]
    std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn()
        .map_err(|e| AppError::Msg(format!("打开附件位置失败：{e}")))?;
    #[cfg(not(windows))]
    {
        let _ = path;
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub acceptance: Option<String>,
    pub priority: Option<i64>,
    pub status: Option<String>,
    pub team_id: Option<String>,
}

#[tauri::command]
pub fn task_update(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    patch: TaskPatch,
) -> Result<()> {
    let conn = conn(&state)?;
    let (title, description, acceptance, priority, current_status): (
        String,
        Option<String>,
        Option<String>,
        i64,
        String,
    ) = conn.query_row(
        "SELECT title, description, acceptance, priority, status FROM tasks WHERE id = ?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    if let Some(status) = patch.status.as_deref() {
        let from = TaskStatus::parse(&current_status).ok_or_else(|| {
            AppError::Msg(format!("未知任务状态「{current_status}」，请先重置任务"))
        })?;
        let to = TaskStatus::parse(status)
            .ok_or_else(|| AppError::Msg(format!("不允许手动把任务状态改为「{status}」")))?;
        if !TaskStatus::can_change_manual(from, to) {
            return Err(if from.is_active() {
                "任务正在运行，请先停止后再修改状态".into()
            } else {
                format!("不允许手动把任务状态改为「{status}」").into()
            });
        }
    }
    let status_changed = patch.status.is_some();
    let next_status = patch
        .status
        .clone()
        .unwrap_or_else(|| current_status.clone());
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE tasks SET title = ?1, description = ?2, acceptance = ?3, priority = ?4,
         team_id = COALESCE(?5, team_id) WHERE id = ?6",
        params![
            patch.title.unwrap_or(title),
            patch.description.or(description),
            patch.acceptance.or(acceptance),
            patch.priority.unwrap_or(priority),
            patch.team_id,
            id
        ],
    )?;
    // 手动置为终态/收件箱后，旧审批不得继续挂在待处理列表里。
    let expired = match patch.status.as_deref() {
        Some(status) => {
            let next = TaskStatus::parse(status)
                .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
            let outcome = task_lifecycle::apply(
                &tx,
                &id,
                TaskTransition::Manual { status: next },
                "任务状态已手动变更",
            )?;
            if !outcome.changed {
                return Err("任务状态已变化，请刷新后重试".into());
            }
            outcome.expired
        }
        None => 0,
    };
    tx.commit()?;
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": id, "reason": "manual_status" }),
        );
    }
    if status_changed {
        let _ = app.emit(
            "task.updated",
            json!({ "taskId": id, "status": next_status }),
        );
    }
    Ok(())
}

#[tauri::command]
pub fn task_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    let worktree: Option<(String, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT t.status, t.work_dir, p.path FROM tasks t
               LEFT JOIN projects p ON p.id = t.project_id WHERE t.id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((status, _, _)) = &worktree {
        if TaskStatus::parse(status)
            .map(TaskStatus::is_active)
            .unwrap_or(false)
        {
            return Err("任务正在运行，请先停止后再删除".into());
        }
    }
    let attachment_paths: Vec<String> = {
        let mut stmt = conn.prepare("SELECT path FROM attachments WHERE task_id = ?1")?;
        let rows: Vec<String> = stmt
            .query_map(params![id], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };
    let tx = conn.unchecked_transaction()?;
    // 子任务保留但解除父子关系，避免删除父任务时孤立不可见。
    tx.execute(
        "UPDATE tasks SET parent_id = NULL WHERE parent_id = ?1",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM task_dependencies WHERE task_id = ?1 OR depends_on_id = ?1",
        params![id],
    )?;
    tx.execute("DELETE FROM attachments WHERE task_id = ?1", params![id])?;
    tx.execute("DELETE FROM messages WHERE task_id = ?1", params![id])?;
    tx.execute("DELETE FROM approvals WHERE task_id = ?1", params![id])?;
    tx.execute(
        "DELETE FROM events WHERE task_id = ?1
           OR run_id IN (SELECT id FROM runs WHERE task_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM artifacts WHERE task_id = ?1
           OR run_id IN (SELECT id FROM runs WHERE task_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM budget_reservations WHERE run_id IN (SELECT id FROM runs WHERE task_id = ?1)",
        params![id],
    )?;
    tx.execute("DELETE FROM usage_events WHERE task_id = ?1", params![id])?;
    tx.execute(
        "DELETE FROM budgets WHERE scope_type = 'task' AND scope_id = ?1",
        params![id],
    )?;
    tx.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
    tx.commit()?;
    for path in attachment_paths {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_dir_all(state.base_dir.join("attachments").join(&id));
    if let Some((_, work_dir, project_path)) = worktree {
        crate::review::cleanup_worktree_best_effort(
            &state,
            work_dir.as_deref(),
            project_path.as_deref(),
        );
    }
    Ok(())
}

#[tauri::command]
pub fn task_dispatch(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<String>> {
    let settings = state.settings();
    if state.registry.active_count() >= settings.max_concurrent_runs.max(1) as usize {
        return Err("并发运行已达上限，请等待当前任务结束或调整设置".into());
    }
    let (status, team_id, stage_index, project_id, work_dir, project_path): (
        String,
        Option<String>,
        i64,
        String,
        Option<String>,
        Option<String>,
    ) = {
        let conn = conn(&state)?;
        conn.query_row(
            "SELECT t.status, t.team_id, t.stage_index, t.project_id, t.work_dir, p.path
               FROM tasks t LEFT JOIN projects p ON p.id = t.project_id WHERE t.id = ?1",
            params![task_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )?
    };
    {
        let conn = conn(&state)?;
        ensure_dependencies_done(&conn, &task_id)?;
    }
    let status_kind = TaskStatus::parse(&status)
        .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
    if !task_lifecycle::permits(status_kind, TaskTransition::Dispatch { start_stage: 0 }) {
        return Err("任务正在运行或等待审批，不能重复派发".into());
    }
    let Some(_) = team_id else {
        return Err("请先为任务选择团队".into());
    };
    // 非隔离运行直接改项目目录：Git 项目先留一份只读补丁快照，非 Git 项目由前端明确警告。
    let snapshot = if work_dir.is_none() {
        match project_path.as_deref() {
            Some(path) if !path.trim().is_empty() => {
                match crate::snapshot::create_pre_run_snapshot(
                    std::path::Path::new(path),
                    &state.base_dir,
                    &task_id,
                ) {
                    Ok(path) => path,
                    Err(err) => {
                        tracing::warn!("任务 {task_id} 的运行前快照生成失败：{err}");
                        None
                    }
                }
            }
            _ => None,
        }
    } else {
        None
    };
    let start = if status == "completed" {
        0
    } else {
        stage_index
    };
    {
        let conn = conn(&state)?;
        let outcome = task_lifecycle::apply(
            &conn,
            &task_id,
            TaskTransition::Dispatch { start_stage: start },
            "任务重新派发",
        )?;
        if !outcome.changed {
            return Err("任务状态已变化，派发已取消".into());
        }
        if outcome.expired > 0 {
            let _ = app.emit(
                "approval.changed",
                json!({ "taskId": task_id, "reason": "redispatch" }),
            );
        }
        log_event(
            &conn,
            Some(&project_id),
            Some(&task_id),
            None,
            "task.dispatched",
            Some("user"),
            json!({ "stageIndex": start, "snapshot": snapshot.clone() }),
        )?;
        if let Some(path) = &snapshot {
            log_event(
                &conn,
                Some(&project_id),
                Some(&task_id),
                None,
                "task.snapshot",
                Some("system"),
                json!({ "path": path, "isolated": false }),
            )?;
        }
    }
    let deps = PipelineDeps::from_state(&app, &state);
    orchestrator::spawn_pipeline(deps, task_id, start);
    Ok(snapshot)
}

#[tauri::command]
pub fn task_cancel(app: AppHandle, state: State<'_, AppState>, task_id: String) -> Result<()> {
    let conn = conn(&state)?;
    let mut stmt = conn
        .prepare("SELECT id FROM runs WHERE task_id = ?1 AND status IN ('starting', 'running')")?;
    let run_ids: Vec<String> = stmt
        .query_map(params![task_id], |r| r.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    for run_id in &run_ids {
        state.registry.cancel(run_id);
    }
    let tx = conn.unchecked_transaction()?;
    let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Cancel, "任务被取消")?;
    let expired = outcome.expired;
    tx.execute(
        "UPDATE runs SET status = 'cancelled', ended_at = ?1
         WHERE task_id = ?2 AND status IN ('starting', 'running')",
        params![now_ms(), task_id],
    )?;
    let project_id: Option<String> = tx
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .ok();
    log_event(
        &tx,
        project_id.as_deref(),
        Some(&task_id),
        None,
        "task.cancelled",
        Some("user"),
        json!({ "runs": run_ids }),
    )?;
    tx.commit()?;
    // 停止运行后隔离区可能留有改动，立即刷新待审查标记。
    if let Err(err) = crate::review::refresh_review_flag(&conn, &task_id) {
        tracing::warn!("刷新待审查标记失败：{err}");
    }
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "blocked" }),
    );
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": task_id, "reason": "cancelled" }),
        );
    }
    Ok(())
}

/// 非隔离运行会直接改项目目录，重新运行前留一份只读补丁快照。
fn pre_run_snapshot(
    state: &State<'_, AppState>,
    task_id: &str,
    work_dir: Option<&str>,
    project_path: Option<&str>,
) -> Option<String> {
    if work_dir
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return None;
    }
    let path = project_path.filter(|value| !value.trim().is_empty())?;
    match crate::snapshot::create_pre_run_snapshot(
        std::path::Path::new(path),
        &state.base_dir,
        task_id,
    ) {
        Ok(snapshot) => snapshot,
        Err(err) => {
            tracing::warn!("任务 {task_id} 的运行前快照生成失败：{err}");
            None
        }
    }
}

type ActiveTaskRow = (
    String,
    Option<String>,
    i64,
    String,
    Option<String>,
    Option<String>,
);

fn active_task_row(conn: &Connection, task_id: &str) -> Result<ActiveTaskRow> {
    conn.query_row(
        "SELECT t.status, t.team_id, t.stage_index, t.project_id, t.work_dir, p.path
           FROM tasks t LEFT JOIN projects p ON p.id = t.project_id WHERE t.id = ?1",
        params![task_id],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )
    .map_err(|_| AppError::Msg("任务不存在".into()))
}

/// 立即暂停：终止当前阶段子进程并标记中断，任务进入 paused；继续时从当前阶段重跑。
#[tauri::command]
pub fn task_pause(app: AppHandle, state: State<'_, AppState>, task_id: String) -> Result<()> {
    let conn = conn(&state)?;
    let (status, project_id): (String, String) = conn
        .query_row(
            "SELECT status, project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    let status_kind = TaskStatus::parse(&status)
        .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
    if !task_lifecycle::permits(status_kind, TaskTransition::Pause) {
        return Err("只有运行中或等待审批的任务可以暂停".into());
    }
    // 先终止该任务的进程树；paused 状态会阻止流水线继续推进下一阶段。
    let cancelled = state.registry.cancel_task(&task_id);
    let tx = conn.unchecked_transaction()?;
    let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Pause, "任务已暂停")?;
    if !outcome.changed {
        return Err("任务状态已变化，暂停已取消".into());
    }
    // 只结束仍在进行的运行；已自然完成的运行保留真实结果。
    tx.execute(
        "UPDATE runs SET status = 'cancelled', error = COALESCE(error, '用户暂停'), ended_at = ?1
         WHERE task_id = ?2 AND status IN ('starting', 'running')",
        params![now_ms(), task_id],
    )?;
    let expired = outcome.expired;
    log_event(
        &tx,
        Some(&project_id),
        Some(&task_id),
        None,
        "task.paused",
        Some("user"),
        json!({ "runs": cancelled }),
    )?;
    tx.commit()?;
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "paused" }),
    );
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": task_id, "reason": "paused" }),
        );
    }
    Ok(())
}

/// 从暂停处继续：沿用 stage_index 从当前阶段重跑，已完成的阶段不重复执行。
#[tauri::command]
pub fn task_resume(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<String>> {
    let settings = state.settings();
    if state.registry.active_count() >= settings.max_concurrent_runs.max(1) as usize {
        return Err("并发运行已达上限，请等待当前任务结束或调整设置".into());
    }
    let db_conn = conn(&state)?;
    let (status, team_id, stage_index, project_id, work_dir, project_path) =
        active_task_row(&db_conn, &task_id)?;
    let status_kind = TaskStatus::parse(&status)
        .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
    if !task_lifecycle::permits(status_kind, TaskTransition::Resume) {
        return Err("只有已暂停的任务可以继续".into());
    }
    if team_id.is_none() {
        return Err("请先为任务选择团队".into());
    }
    ensure_dependencies_done(&db_conn, &task_id)?;
    drop(db_conn);
    let snapshot = pre_run_snapshot(
        &state,
        &task_id,
        work_dir.as_deref(),
        project_path.as_deref(),
    );
    let conn = conn(&state)?;
    let tx = conn.unchecked_transaction()?;
    let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Resume, "任务继续")?;
    if !outcome.changed {
        return Err("任务状态已变化，继续已取消".into());
    }
    let expired = outcome.expired;
    log_event(
        &tx,
        Some(&project_id),
        Some(&task_id),
        None,
        "task.resumed",
        Some("user"),
        json!({ "stageIndex": stage_index, "snapshot": snapshot.clone() }),
    )?;
    if let Some(path) = &snapshot {
        log_event(
            &tx,
            Some(&project_id),
            Some(&task_id),
            None,
            "task.snapshot",
            Some("system"),
            json!({ "path": path, "isolated": false }),
        )?;
    }
    tx.commit()?;
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "queued" }),
    );
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": task_id, "reason": "resumed" }),
        );
    }
    let deps = PipelineDeps::from_state(&app, &state);
    orchestrator::spawn_pipeline(deps, task_id, stage_index.max(0));
    Ok(snapshot)
}

/// 改派当前阶段的席位 Agent，并从当前阶段重跑；支持暂停中与阻塞中的任务。
#[tauri::command]
pub fn task_reassign(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    agent_id: String,
) -> Result<Option<String>> {
    let settings = state.settings();
    if state.registry.active_count() >= settings.max_concurrent_runs.max(1) as usize {
        return Err("并发运行已达上限，请等待当前任务结束或调整设置".into());
    }
    let db_conn = conn(&state)?;
    let (status, team_id, stage_index, project_id, work_dir, project_path) =
        active_task_row(&db_conn, &task_id)?;
    let status_kind = TaskStatus::parse(&status)
        .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
    if !task_lifecycle::permits(status_kind, TaskTransition::Reassign) {
        return Err("只有暂停或阻塞的任务可以改派后重跑".into());
    }
    let Some(team_id) = team_id else {
        return Err("请先为任务选择团队".into());
    };
    let agent_name: String = db_conn
        .query_row(
            "SELECT name FROM agents WHERE id = ?1",
            params![agent_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("Agent 不存在".into()))?;
    let seat: Option<(String, String)> = db_conn
        .query_row(
            "SELECT id, role_name FROM seats WHERE team_id = ?1
             ORDER BY order_index ASC LIMIT 1 OFFSET ?2",
            params![team_id, stage_index.max(0)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((seat_id, role_name)) = seat else {
        return Err("当前阶段没有对应席位，请先在团队页调整席位".into());
    };
    ensure_dependencies_done(&db_conn, &task_id)?;
    drop(db_conn);
    let snapshot = pre_run_snapshot(
        &state,
        &task_id,
        work_dir.as_deref(),
        project_path.as_deref(),
    );
    let conn = conn(&state)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE seats SET agent_id = ?1 WHERE id = ?2",
        params![agent_id, seat_id],
    )?;
    let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Reassign, "任务改派")?;
    if !outcome.changed {
        return Err("任务状态已变化，改派已取消".into());
    }
    let expired = outcome.expired;
    log_event(
        &tx,
        Some(&project_id),
        Some(&task_id),
        None,
        "task.reassigned",
        Some("user"),
        json!({
            "stageIndex": stage_index,
            "seatId": seat_id,
            "seatName": role_name,
            "agentId": agent_id,
            "agentName": agent_name,
            "snapshot": snapshot.clone()
        }),
    )?;
    if let Some(path) = &snapshot {
        log_event(
            &tx,
            Some(&project_id),
            Some(&task_id),
            None,
            "task.snapshot",
            Some("system"),
            json!({ "path": path, "isolated": false }),
        )?;
    }
    tx.commit()?;
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "queued" }),
    );
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": task_id, "reason": "reassigned" }),
        );
    }
    let deps = PipelineDeps::from_state(&app, &state);
    orchestrator::spawn_pipeline(deps, task_id, stage_index.max(0));
    Ok(snapshot)
}

#[tauri::command]
pub fn task_reset(app: AppHandle, state: State<'_, AppState>, task_id: String) -> Result<()> {
    let conn = conn(&state)?;
    let status: String = conn
        .query_row(
            "SELECT status FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    // Unknown persisted values are repairable only through this reset path.
    if let Some(status_kind) = TaskStatus::parse(&status) {
        if !task_lifecycle::permits(status_kind, TaskTransition::Reset) {
            return Err("任务正在运行，请先取消后再重置".into());
        }
    }
    let tx = conn.unchecked_transaction()?;
    let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Reset, "任务已重置")?;
    if !outcome.changed {
        return Err("任务状态已变化，请刷新后重试".into());
    }
    let expired = outcome.expired;
    tx.commit()?;
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "inbox" }),
    );
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": task_id, "reason": "reset" }),
        );
    }
    Ok(())
}

/// 从当前阶段重试：仅清理挂起审批并沿用 stage_index，不再整体回到收件箱。
#[tauri::command]
pub fn task_retry_stage(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<String>> {
    let settings = state.settings();
    if state.registry.active_count() >= settings.max_concurrent_runs.max(1) as usize {
        return Err("并发运行已达上限，请等待当前任务结束或调整设置".into());
    }
    let (status, team_id, stage_index, project_id, work_dir, project_path): (
        String,
        Option<String>,
        i64,
        String,
        Option<String>,
        Option<String>,
    ) = {
        let conn = conn(&state)?;
        conn.query_row(
            "SELECT t.status, t.team_id, t.stage_index, t.project_id, t.work_dir, p.path
               FROM tasks t LEFT JOIN projects p ON p.id = t.project_id WHERE t.id = ?1",
            params![task_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )?
    };
    let status_kind = TaskStatus::parse(&status)
        .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
    if !task_lifecycle::permits(status_kind, TaskTransition::Retry) {
        return Err("只有已阻塞的任务可以从当前阶段重试".into());
    }
    {
        let conn = conn(&state)?;
        ensure_dependencies_done(&conn, &task_id)?;
    }
    if team_id.is_none() {
        return Err("请先为任务选择团队".into());
    }
    // 重试同样是一次真实运行：非隔离时也留一份运行前快照。
    let snapshot = if work_dir.is_none() {
        match project_path.as_deref() {
            Some(path) if !path.trim().is_empty() => {
                match crate::snapshot::create_pre_run_snapshot(
                    std::path::Path::new(path),
                    &state.base_dir,
                    &task_id,
                ) {
                    Ok(path) => path,
                    Err(err) => {
                        tracing::warn!("任务 {task_id} 的重试前快照生成失败：{err}");
                        None
                    }
                }
            }
            _ => None,
        }
    } else {
        None
    };
    {
        let conn = conn(&state)?;
        let tx = conn.unchecked_transaction()?;
        let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Retry, "任务重试阶段")?;
        if !outcome.changed {
            return Err("任务状态已变化，重试已取消".into());
        }
        let expired = outcome.expired;
        log_event(
            &tx,
            Some(&project_id),
            Some(&task_id),
            None,
            "task.retried",
            Some("user"),
            json!({ "stageIndex": stage_index, "snapshot": snapshot.clone() }),
        )?;
        if let Some(path) = &snapshot {
            log_event(
                &tx,
                Some(&project_id),
                Some(&task_id),
                None,
                "task.snapshot",
                Some("system"),
                json!({ "path": path, "isolated": false }),
            )?;
        }
        tx.commit()?;
        if expired > 0 {
            let _ = app.emit(
                "approval.changed",
                json!({ "taskId": task_id, "reason": "retry" }),
            );
        }
    }
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "queued" }),
    );
    let deps = PipelineDeps::from_state(&app, &state);
    orchestrator::spawn_pipeline(deps, task_id, stage_index);
    Ok(snapshot)
}

#[tauri::command]
pub fn task_add_note(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    content: String,
) -> Result<()> {
    if content.trim().is_empty() {
        return Err("内容不能为空".into());
    }
    let conn = conn(&state)?;
    let status: String = conn
        .query_row(
            "SELECT status FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    // 运行中的追加内容不会打断当前 Agent，而是写入时间线供下一阶段或重试时使用。
    let during_run = TaskStatus::parse(&status)
        .map(TaskStatus::is_active)
        .unwrap_or(false);
    let id = insert_message(
        &conn,
        Some(&task_id),
        None,
        None,
        None,
        "chat",
        Some("user"),
        content.trim(),
        json!({ "source": "user", "duringRun": during_run }),
    )?;
    let _ = app.emit(
        "run.delta",
        json!({
            "taskId": task_id,
            "kind": "message",
            "message": {
                "id": id,
                "taskId": task_id,
                "kind": "chat",
                "role": "user",
                "content": content.trim(),
                "createdAt": now_ms()
            }
        }),
    );
    Ok(())
}

#[tauri::command]
pub fn task_messages(
    state: State<'_, AppState>,
    task_id: String,
    before: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<Message>> {
    let conn = conn(&state)?;
    let lim = limit.unwrap_or(200).clamp(1, 500);
    // 倒序取最近 N 条再回正，避免超过上限后新消息完全不可见；before 作为游标加载更早。
    let (sql, params_vec): (String, Vec<SqlValue>) = match before {
        Some(cursor) => (
            format!(
                "{MESSAGE_SQL} WHERE m.task_id = ?1 AND m.created_at < ?2 ORDER BY m.created_at DESC, m.rowid DESC LIMIT ?3"
            ),
            vec![task_id.into(), cursor.into(), lim.into()],
        ),
        None => (
            format!("{MESSAGE_SQL} WHERE m.task_id = ?1 ORDER BY m.created_at DESC, m.rowid DESC LIMIT ?2"),
            vec![task_id.into(), lim.into()],
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params_vec), map_message)?;
    let mut items: Vec<Message> = rows.filter_map(|r| r.ok()).collect();
    items.reverse();
    Ok(items)
}

#[tauri::command]
pub fn tool_calls_list(
    state: State<'_, AppState>,
    task_id: String,
    limit: Option<i64>,
) -> Result<Vec<ToolCall>> {
    let conn = conn(&state)?;
    let lim = limit.unwrap_or(200).clamp(1, 500);
    let mut stmt = conn.prepare(
        "SELECT t.id, t.run_id, t.tool_name, t.input_json, t.output_json, t.status,
                t.risk_level, t.created_at, t.finished_at
         FROM tool_calls t JOIN runs r ON r.id = t.run_id
         WHERE r.task_id = ?1
         ORDER BY t.created_at DESC, t.rowid DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![task_id, lim], |r| {
        let input: String = r.get(3)?;
        let output: Option<String> = r.get(4)?;
        Ok(ToolCall {
            id: r.get(0)?,
            run_id: r.get(1)?,
            tool_name: r.get(2)?,
            input: serde_json::from_str(&input).unwrap_or(Value::Null),
            output: output.and_then(|text| serde_json::from_str(&text).ok()),
            status: r.get(5)?,
            risk_level: r.get(6)?,
            created_at: r.get(7)?,
            finished_at: r.get(8)?,
        })
    })?;
    let mut items: Vec<ToolCall> = rows.filter_map(|r| r.ok()).collect();
    items.reverse();
    Ok(items)
}

#[tauri::command]
pub fn runs_list(
    state: State<'_, AppState>,
    task_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<Run>> {
    let conn = conn(&state)?;
    let lim = limit.unwrap_or(100).clamp(1, 500);
    let base = "SELECT r.id, r.task_id, r.seat_id, r.agent_id, a.name, s.role_name, r.stage_key,
            r.status, r.external_session_id, r.isolation, r.isolation_reason,
            r.input_tokens, r.output_tokens, r.cost_micros, r.error, r.started_at, r.ended_at
         FROM runs r LEFT JOIN agents a ON a.id = r.agent_id
         LEFT JOIN seats s ON s.id = r.seat_id";
    let sql = if task_id.is_some() {
        format!("{base} WHERE r.task_id = ?1 ORDER BY r.started_at DESC LIMIT ?2")
    } else {
        format!("{base} ORDER BY r.started_at DESC LIMIT ?1")
    };
    let mut stmt = conn.prepare(&sql)?;
    let mapper = |r: &rusqlite::Row| -> rusqlite::Result<Run> {
        Ok(Run {
            id: r.get(0)?,
            task_id: r.get(1)?,
            seat_id: r.get(2)?,
            agent_id: r.get(3)?,
            agent_name: r.get(4)?,
            seat_name: r.get(5)?,
            stage_key: r.get(6)?,
            status: r.get(7)?,
            external_session_id: r.get(8)?,
            isolation: r.get(9)?,
            isolation_reason: r.get(10)?,
            input_tokens: r.get(11)?,
            output_tokens: r.get(12)?,
            cost_micros: r.get(13)?,
            error: r.get(14)?,
            started_at: r.get(15)?,
            ended_at: r.get(16)?,
        })
    };
    let items: Vec<Run> = if let Some(tid) = task_id {
        stmt.query_map(params![tid, lim], mapper)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map(params![lim], mapper)?
            .filter_map(|r| r.ok())
            .collect()
    };
    Ok(items)
}

#[tauri::command]
pub fn run_prompt(state: State<'_, AppState>, run_id: String) -> Result<Option<String>> {
    let conn = conn(&state)?;
    let prompt = conn
        .query_row(
            "SELECT prompt_text FROM runs WHERE id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(prompt)
}

#[tauri::command]
pub fn run_cancel(state: State<'_, AppState>, run_id: String) -> Result<()> {
    state.registry.cancel(&run_id);
    Ok(())
}

#[tauri::command]
pub fn run_transcript(
    state: State<'_, AppState>,
    run_id: String,
    limit: Option<i64>,
) -> Result<Vec<String>> {
    // 只接受 UUID，且最终路径必须落在 sessions 目录内，杜绝 ../../ 路径穿越。
    let id = uuid::Uuid::parse_str(&run_id).map_err(|_| "非法的运行 ID")?;
    let sessions = state.base_dir.join("sessions");
    let path = sessions.join(format!("{id}.ndjson"));
    let canonical_sessions = sessions.canonicalize().unwrap_or_else(|_| sessions.clone());
    let canonical = match path.canonicalize() {
        Ok(p) if p.starts_with(&canonical_sessions) => p,
        _ => return Ok(Vec::new()),
    };
    let text = std::fs::read_to_string(canonical).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let take = limit.unwrap_or(200).clamp(1, 2000) as usize;
    let start = lines.len().saturating_sub(take);
    Ok(lines[start..].iter().map(|s| s.to_string()).collect())
}
