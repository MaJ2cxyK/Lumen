use super::*;

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
