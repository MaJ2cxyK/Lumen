use super::*;

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
        tasks::ensure_dependencies_done(&conn, &task_id)?;
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
    tasks::ensure_dependencies_done(&db_conn, &task_id)?;
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
    tasks::ensure_dependencies_done(&db_conn, &task_id)?;
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
        tasks::ensure_dependencies_done(&conn, &task_id)?;
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
