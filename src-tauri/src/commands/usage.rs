//! Usage and budget commands.

use super::*;
use crate::usage;

#[tauri::command]
pub fn usage_summary(state: State<'_, AppState>) -> Result<UsageSummary> {
    let conn = conn(&state)?;
    usage::summary(&conn)
}

#[tauri::command]
pub fn usage_series(state: State<'_, AppState>, days: Option<i64>) -> Result<Vec<UsagePoint>> {
    let conn = conn(&state)?;
    usage::series(&conn, days.unwrap_or(7).clamp(1, 90))
}

#[tauri::command]
pub fn usage_by_agent(state: State<'_, AppState>) -> Result<Vec<ScopedUsage>> {
    let conn = conn(&state)?;
    usage::by_agent(&conn)
}

#[tauri::command]
pub fn usage_by_project(state: State<'_, AppState>) -> Result<Vec<ScopedUsage>> {
    let conn = conn(&state)?;
    usage::by_project(&conn)
}

#[tauri::command]
pub fn usage_by_task(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<Vec<ScopedUsage>> {
    let conn = conn(&state)?;
    let base = "SELECT t.id, t.title,
            COALESCE(SUM(u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens), 0),
            COALESCE(SUM(u.cost_micros), 0),
            COALESCE(COUNT(u.id), 0)
         FROM tasks t LEFT JOIN usage_events u ON u.task_id = t.id";
    let sql = if project_id.is_some() {
        format!("{base} WHERE t.project_id = ?1 GROUP BY t.id ORDER BY 3 DESC LIMIT 100")
    } else {
        format!("{base} GROUP BY t.id ORDER BY 3 DESC LIMIT 100")
    };
    let mut stmt = conn.prepare(&sql)?;
    let mapper = |r: &rusqlite::Row| -> rusqlite::Result<ScopedUsage> {
        Ok(ScopedUsage {
            key: r.get(0)?,
            name: r.get(1)?,
            tokens: r.get(2)?,
            cost_micros: r.get(3)?,
            rounds: r.get(4)?,
        })
    };
    let items: Vec<ScopedUsage> = if let Some(pid) = project_id {
        stmt.query_map(params![pid], mapper)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map([], mapper)?.filter_map(|r| r.ok()).collect()
    };
    Ok(items)
}

#[tauri::command]
pub fn budgets_list(state: State<'_, AppState>) -> Result<Vec<Budget>> {
    let conn = conn(&state)?;
    crate::budget::list(&conn)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn budget_upsert(
    state: State<'_, AppState>,
    id: Option<String>,
    scope_type: String,
    scope_id: Option<String>,
    unit: String,
    period: String,
    limit_value: i64,
    warn_pct: Option<i64>,
    hard_stop: bool,
    enabled: Option<bool>,
) -> Result<String> {
    if limit_value <= 0 {
        return Err("预算上限必须大于 0".into());
    }
    if scope_type != "global" && scope_id.as_deref().unwrap_or_default().is_empty() {
        return Err("请选择预算作用对象".into());
    }
    let conn = conn(&state)?;
    crate::budget::upsert(
        &conn,
        id,
        &scope_type,
        scope_id,
        &unit,
        &period,
        limit_value,
        warn_pct.unwrap_or(80).clamp(1, 100),
        hard_stop,
        enabled,
    )
}

#[tauri::command]
pub fn budget_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    crate::budget::delete(&conn, &id)
}
