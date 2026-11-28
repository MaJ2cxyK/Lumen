use super::*;

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
pub(super) fn query_tasks_page(
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
pub(super) fn ensure_dependencies_done(conn: &Connection, task_id: &str) -> Result<()> {
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
