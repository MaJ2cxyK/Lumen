//! Approval queue and approval rule commands.

use super::*;

#[tauri::command]
pub fn approvals_list(state: State<'_, AppState>, status: Option<String>) -> Result<Vec<Approval>> {
    let conn = conn(&state)?;
    approval::list(&conn, status.as_deref())
}

#[tauri::command]
pub fn approvals_pending(state: State<'_, AppState>) -> Result<i64> {
    let conn = conn(&state)?;
    approval::pending_count(&conn)
}

#[tauri::command]
pub fn approval_decide(
    app: AppHandle,
    state: State<'_, AppState>,
    approval_id: String,
    approve: bool,
    note: Option<String>,
    remember: Option<bool>,
) -> Result<()> {
    let (r#type, detail_json, task_id): (String, String, Option<String>) = {
        let conn = conn(&state)?;
        conn.query_row(
            "SELECT type, detail_json, task_id FROM approvals WHERE id = ?1",
            params![approval_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?
    };
    if r#type == "stage_gate" {
        if let Some(tid) = task_id.as_deref() {
            let detail: Value = serde_json::from_str(&detail_json).unwrap_or(Value::Null);
            let stage = detail
                .get("stageIndex")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let (status, stage_index): (String, i64) = {
                let conn = conn(&state)?;
                conn.query_row(
                    "SELECT status, stage_index FROM tasks WHERE id = ?1",
                    params![tid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?
            };
            if status != "waiting_approval" || stage_index != stage {
                return Err("任务当前状态或阶段已变化，该门禁审批已失效".into());
            }
        }
    }
    let note = note.unwrap_or_default();
    approval::decide(
        &app,
        &state.gate,
        &state.db,
        &approval_id,
        approve,
        &note,
        remember.unwrap_or(false),
    )?;

    if r#type == "stage_gate" {
        if let Some(task_id) = task_id {
            let detail: Value = serde_json::from_str(&detail_json).unwrap_or(Value::Null);
            let stage = detail
                .get("stageIndex")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let deps = PipelineDeps::from_state(&app, &state);
            if approve {
                if !orchestrator::resume_after_gate(deps, task_id, stage) {
                    return Err("任务已结束或阶段已变化，审批未生效".into());
                }
            } else {
                orchestrator::mark_blocked(
                    &deps,
                    &task_id,
                    if note.is_empty() {
                        "阶段门禁被拒绝"
                    } else {
                        &note
                    },
                    false,
                );
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn approval_rules_list(state: State<'_, AppState>) -> Result<Vec<ApprovalRule>> {
    let conn = conn(&state)?;
    approval::list_rules(&conn)
}

#[tauri::command]
pub fn approval_rule_upsert(
    state: State<'_, AppState>,
    id: Option<String>,
    matcher: Value,
    action: String,
    priority: Option<i64>,
) -> Result<String> {
    if !["allow", "deny", "ask"].contains(&action.as_str()) {
        return Err("规则动作必须是 allow/deny/ask".into());
    }
    let conn = conn(&state)?;
    approval::upsert_rule(&conn, id, matcher, &action, priority.unwrap_or(100))
}

#[tauri::command]
pub fn approval_rule_toggle(state: State<'_, AppState>, id: String, enabled: bool) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute(
        "UPDATE approval_rules SET enabled = ?1 WHERE id = ?2",
        params![enabled as i64, id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn approval_rule_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute("DELETE FROM approval_rules WHERE id = ?1", params![id])?;
    Ok(())
}
