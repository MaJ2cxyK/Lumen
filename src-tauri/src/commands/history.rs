//! Event history, message search and dashboard summary commands.

use super::*;

#[tauri::command]
pub fn events_list(
    state: State<'_, AppState>,
    project_id: Option<String>,
    task_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<EventItem>> {
    let conn = conn(&state)?;
    let mut sql = String::from(
        "SELECT id, seq, project_id, task_id, run_id, type, actor, payload_json, created_at FROM events WHERE 1=1",
    );
    let mut values: Vec<SqlValue> = Vec::new();
    if let Some(pid) = project_id {
        sql.push_str(" AND project_id = ?");
        values.push(pid.into());
    }
    if let Some(tid) = task_id {
        sql.push_str(" AND task_id = ?");
        values.push(tid.into());
    }
    sql.push_str(" ORDER BY seq DESC LIMIT ?");
    values.push(limit.unwrap_or(100).clamp(1, 500).into());
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values), |r| {
        let payload: String = r.get(7)?;
        Ok(EventItem {
            id: r.get(0)?,
            seq: r.get(1)?,
            project_id: r.get(2)?,
            task_id: r.get(3)?,
            run_id: r.get(4)?,
            r#type: r.get(5)?,
            actor: r.get(6)?,
            payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
            created_at: r.get(8)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn search_history(
    state: State<'_, AppState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<Message>> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let conn = conn(&state)?;
    let sql = format!("{MESSAGE_SQL} WHERE m.content LIKE ?1 ORDER BY m.created_at DESC LIMIT ?2");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![format!("%{q}%"), limit.unwrap_or(80).clamp(1, 300)],
        map_message,
    )?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn dashboard_summary(state: State<'_, AppState>) -> Result<DashboardSummary> {
    let conn = conn(&state)?;
    let usage = usage_stats::summary(&conn)?;
    let pending_approvals = approval::pending_count(&conn)?;
    let pending_reviews: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE review_pending = 1",
        [],
        |r| r.get(0),
    )?;
    let agents = usage_stats::agent_stats(&conn)?;
    let series = usage_stats::series(&conn, 7)?;
    let mut teams = super::teams::teams_query(&conn, None)?;
    teams.truncate(5);
    Ok(DashboardSummary {
        usage,
        pending_approvals,
        pending_reviews,
        agents,
        teams,
        series,
    })
}
