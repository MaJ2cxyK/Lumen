//! Manual run approval for adapters without tool-level hooks.

use super::context::{AgentCtx, SeatCtx, TaskCtx};
use super::PipelineDeps;
use crate::db::{log_event, new_id, now_ms};
use crate::error::{AppError, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::json;
use tauri::Emitter;

/// 非 Claude 适配器没有 PreToolUse Hook，运行前补一道人工放行：
/// 创建 `run_permission` 审批并轮询等待，批准后继续，拒绝/超时/取消则返回错误。
pub(super) async fn wait_run_approval(
    deps: &PipelineDeps,
    task: &TaskCtx,
    seat: &SeatCtx,
    agent: &AgentCtx,
    stage: i64,
) -> Result<()> {
    let (approval_id, created) = {
        let conn = deps.db.open()?;
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM approvals
                 WHERE task_id = ?1 AND type = 'run_permission' AND status = 'pending'
                   AND json_extract(detail_json, '$.stageIndex') = ?2
                 ORDER BY created_at DESC LIMIT 1",
                params![task.id, stage],
                |r| r.get(0),
            )
            .optional()?;
        match existing {
            Some(id) => (id, false),
            None => {
                let id = new_id();
                let title = crate::i18n::tr(
                    &format!("运行授权：{} · {}", agent.name, task.title),
                    &format!("Run approval: {} · {}", agent.name, task.title),
                );
                let detail = json!({
                    "stageIndex": stage,
                    "adapter": agent.adapter,
                    "agentName": agent.name,
                    "seatName": seat.role_name,
                    "reason": crate::i18n::tr(
                        "该适配器不支持工具级 Hook 审批，需要运行前人工确认",
                        "This adapter does not support tool-level hook approval; manual confirmation is required before the run"
                    )
                });
                conn.execute(
                    "INSERT INTO approvals (id, task_id, run_id, type, risk_level, title, detail_json, status, requested_by, created_at, expires_at)
                     VALUES (?1, ?2, NULL, 'run_permission', 'high', ?3, ?4, 'pending', ?5, ?6, ?7)",
                    params![
                        id,
                        task.id,
                        title,
                        detail.to_string(),
                        format!("agent:{}", agent.name),
                        now_ms(),
                        now_ms() + deps.settings.approval_timeout_secs * 1000
                    ],
                )?;
                let outcome = crate::task_lifecycle::apply(
                    &conn,
                    &task.id,
                    crate::task_lifecycle::TaskTransition::WaitApproval { stage },
                    "",
                )?;
                if !outcome.changed {
                    let _ = conn.execute(
                        "UPDATE approvals SET status = 'expired', decided_at = ?1, decision_note = '任务已结束'
                         WHERE id = ?2 AND status = 'pending'",
                        params![now_ms(), id],
                    );
                    return Err(AppError::Msg("任务已结束，运行授权未创建".into()));
                }
                (id, true)
            }
        }
    };

    if created {
        let _ = deps.app.emit(
            "approval.requested",
            json!({ "approvalId": approval_id, "type": "run_permission", "title": crate::i18n::tr(&format!("运行授权：{}", agent.name), &format!("Run approval: {}", agent.name)) }),
        );
        let _ = deps.app.emit(
            "task.updated",
            json!({ "taskId": task.id, "status": "waiting_approval" }),
        );
        crate::notify::send_task(
            &deps.app,
            &crate::i18n::tr("Lumen 运行授权", "Lumen run approval"),
            &crate::i18n::tr(
                &format!("「{}」等待你授权启动 {}", task.title, agent.name),
                &format!(
                    "\"{}\" is waiting for your approval to start {}",
                    task.title, agent.name
                ),
            ),
            Some(&task.id),
            "approval",
        );
    }

    let deadline = now_ms() + deps.settings.approval_timeout_secs * 1000;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let conn = deps.db.open()?;
        let (status, note): (String, Option<String>) = conn.query_row(
            "SELECT status, decision_note FROM approvals WHERE id = ?1",
            params![approval_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        match status.as_str() {
            "approved" => {
                let outcome = crate::task_lifecycle::apply(
                    &conn,
                    &task.id,
                    crate::task_lifecycle::TaskTransition::ApproveRun { stage },
                    "",
                )?;
                if !outcome.changed {
                    return Err(AppError::Msg(
                        "任务已结束或阶段已变化，运行授权未生效".into(),
                    ));
                }
                log_event(
                    &conn,
                    Some(&task.project_id),
                    Some(&task.id),
                    None,
                    "run.approved",
                    Some("user"),
                    json!({ "approvalId": approval_id, "agent": agent.name }),
                )?;
                return Ok(());
            }
            "denied" => {
                return Err(AppError::Msg(
                    note.unwrap_or_else(|| "运行授权被拒绝".into()),
                ))
            }
            "expired" => return Err(AppError::Msg("运行授权已过期，任务已暂停".into())),
            _ => {}
        }
        if now_ms() > deadline {
            let changed = conn.execute(
                "UPDATE approvals SET status = 'expired', decided_at = ?1, decision_note = '等待超时，运行未启动'
                 WHERE id = ?2 AND status = 'pending'",
                params![now_ms(), approval_id],
            )?;
            if changed > 0 {
                let _ = deps.app.emit(
                    "approval.changed",
                    json!({ "approvalId": approval_id, "reason": "timeout" }),
                );
            }
            return Err(AppError::Msg("运行授权等待超时，任务已暂停".into()));
        }
    }
}
