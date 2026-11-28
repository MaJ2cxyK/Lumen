use super::*;

#[tauri::command]
pub fn projects_list(state: State<'_, AppState>) -> Result<Vec<Project>> {
    let conn = conn(&state)?;
    let mut stmt = conn.prepare(
        "SELECT id, name, path, kind, description, created_at, updated_at FROM projects ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Project {
            id: r.get(0)?,
            name: r.get(1)?,
            path: r.get(2)?,
            kind: r.get(3)?,
            description: r.get(4)?,
            is_git: false,
            git_reason: None,
            created_at: r.get(5)?,
            updated_at: r.get(6)?,
        })
    })?;
    let mut items: Vec<Project> = rows.filter_map(|r| r.ok()).collect();
    // Git 探测只用于展示与派发提示，不写入数据库。
    for project in &mut items {
        let probe = crate::review::git_state(std::path::Path::new(&project.path));
        project.is_git = probe.is_git;
        project.git_reason = probe.reason;
    }
    Ok(items)
}

#[tauri::command]
pub fn project_git_check(path: String) -> crate::review::GitState {
    crate::review::git_state(std::path::Path::new(path.trim()))
}

#[tauri::command]
pub fn project_create(
    state: State<'_, AppState>,
    name: String,
    path: String,
    kind: String,
    description: Option<String>,
) -> Result<String> {
    if name.trim().is_empty() {
        return Err("项目名称不能为空".into());
    }
    if !std::path::Path::new(&path).is_dir() {
        return Err("项目目录不存在".into());
    }
    let conn = conn(&state)?;
    let id = new_id();
    let now = now_ms();
    conn.execute(
        "INSERT INTO projects (id, name, path, kind, description, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![id, name.trim(), path.trim(), kind, description, now],
    )?;
    log_event(
        &conn,
        Some(&id),
        None,
        None,
        "project.created",
        Some("user"),
        json!({ "name": name }),
    )?;
    Ok(id)
}

#[tauri::command]
pub fn project_update(
    state: State<'_, AppState>,
    id: String,
    name: String,
    path: String,
    kind: String,
    description: Option<String>,
) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute(
        "UPDATE projects SET name = ?1, path = ?2, kind = ?3, description = ?4, updated_at = ?5 WHERE id = ?6",
        params![name.trim(), path.trim(), kind, description, now_ms(), id],
    )?;
    Ok(())
}

/// Counts tasks that still own a run slot, using the lifecycle vocabulary so
/// project deletion can never disagree with the transition guards.
pub(super) fn project_active_task_count(conn: &Connection, project_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM tasks WHERE project_id = ?1 AND status IN ({})",
            task_lifecycle::ACTIVE_STATUSES_SQL
        ),
        params![project_id],
        |r| r.get(0),
    )?)
}

#[tauri::command]
pub fn project_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    let running = project_active_task_count(&conn, &id)?;
    if running > 0 {
        return Err("项目下有任务正在运行，请先停止后再删除项目".into());
    }
    let project_path: Option<String> = conn
        .query_row(
            "SELECT path FROM projects WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    let worktrees: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT work_dir FROM tasks WHERE project_id = ?1 AND work_dir IS NOT NULL")?;
        let rows = stmt.query_map(params![id], |r| r.get(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM messages WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM approvals WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM events WHERE project_id = ?1
           OR task_id IN (SELECT id FROM tasks WHERE project_id = ?1)
           OR run_id IN (SELECT r.id FROM runs r JOIN tasks t ON t.id = r.task_id WHERE t.project_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM artifacts WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?1)
           OR run_id IN (SELECT r.id FROM runs r JOIN tasks t ON t.id = r.task_id WHERE t.project_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM budget_reservations WHERE run_id IN (
           SELECT r.id FROM runs r JOIN tasks t ON t.id = r.task_id WHERE t.project_id = ?1
         ) OR budget_id IN (SELECT id FROM budgets WHERE (scope_type = 'project' AND scope_id = ?1)
           OR (scope_type = 'team' AND scope_id IN (SELECT id FROM teams WHERE project_id = ?1)))",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM usage_events WHERE project_id = ?1",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM budgets WHERE (scope_type = 'project' AND scope_id = ?1)
           OR (scope_type = 'team' AND scope_id IN (SELECT id FROM teams WHERE project_id = ?1))
           OR (scope_type = 'task' AND scope_id IN (SELECT id FROM tasks WHERE project_id = ?1))",
        params![id],
    )?;
    tx.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
    tx.commit()?;
    for dir in worktrees {
        crate::review::cleanup_worktree_best_effort(&state, Some(&dir), project_path.as_deref());
    }
    Ok(())
}
