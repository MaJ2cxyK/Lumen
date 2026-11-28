use crate::approval::GateState;
use crate::db::{log_event, Db};
use crate::llm_proxy::ProxyState;
use crate::models::AppSettings;
use crate::runtime::RunRegistry;
use crate::state::AppState;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

mod context;
mod gate;
mod pipeline;

use pipeline::run_pipeline;

#[derive(Clone)]
pub struct PipelineDeps {
    pub app: AppHandle,
    pub db: Db,
    pub base_dir: PathBuf,
    pub settings: AppSettings,
    pub gate: Arc<GateState>,
    pub registry: Arc<RunRegistry>,
    pub proxy: Arc<ProxyState>,
}

impl PipelineDeps {
    pub fn from_state(app: &AppHandle, state: &AppState) -> Self {
        Self {
            app: app.clone(),
            db: state.db.clone(),
            base_dir: state.base_dir.clone(),
            settings: state.settings(),
            gate: state.gate.clone(),
            registry: state.registry.clone(),
            proxy: state.proxy.clone(),
        }
    }
}

fn emit_task(app: &AppHandle, task_id: &str, status: &str, stage_index: i64, extra: Value) {
    let mut payload = json!({ "taskId": task_id, "status": status, "stageIndex": stage_index });
    if let Some(obj) = extra.as_object() {
        if let Some(map) = payload.as_object_mut() {
            for (k, v) in obj {
                map.insert(k.clone(), v.clone());
            }
        }
    }
    let _ = app.emit("task.updated", payload);
}

pub fn mark_blocked(deps: &PipelineDeps, task_id: &str, reason: &str, notify_user: bool) {
    let mut transitioned = false;
    let mut expired = 0usize;
    let mut title = String::new();
    if let Ok(conn) = deps.db.open() {
        if let Ok(tx) = conn.unchecked_transaction() {
            let row: Option<(String, String)> = tx
                .query_row(
                    "SELECT project_id, title FROM tasks WHERE id = ?1",
                    params![task_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .unwrap_or(None);
            if let Some((project_id, task_title)) = row {
                title = task_title;
                match crate::task_lifecycle::apply(
                    &tx,
                    task_id,
                    crate::task_lifecycle::TaskTransition::Block,
                    "任务已阻塞",
                ) {
                    Ok(outcome) => {
                        transitioned = outcome.changed;
                        expired = outcome.expired;
                    }
                    Err(err) => tracing::warn!("任务 {task_id} 置为阻塞失败：{err}"),
                }
                let _ = log_event(
                    &tx,
                    Some(&project_id),
                    Some(task_id),
                    None,
                    "task.blocked",
                    Some("orchestrator"),
                    json!({ "reason": reason }),
                );
            }
            let _ = tx.commit();
        }
        // 失败或中断时隔离区可能已有改动，同步待审查标记，避免看板漏报。
        if let Err(err) = crate::review::refresh_review_flag(&conn, task_id) {
            tracing::warn!("刷新待审查标记失败：{err}");
        }
    }
    let _ = deps.app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "blocked", "reason": reason }),
    );
    if expired > 0 {
        let _ = deps.app.emit(
            "approval.changed",
            json!({ "taskId": task_id, "reason": "blocked" }),
        );
    }
    // 用户主动取消也会走到这里，状态未发生 blocked 迁移时不重复提醒。
    if transitioned && notify_user {
        let brief: String = reason.chars().take(120).collect();
        crate::notify::send_task(
            &deps.app,
            &crate::i18n::tr("Lumen 任务需要处理", "Lumen task needs attention"),
            &crate::i18n::tr(
                &format!("「{title}」已阻塞：{brief}"),
                &format!("\"{title}\" is blocked: {brief}"),
            ),
            Some(task_id),
            "task_blocked",
        );
    }
}

pub fn spawn_pipeline(deps: PipelineDeps, task_id: String, start_stage: i64) {
    tauri::async_runtime::spawn(async move {
        if let Err(err) = run_pipeline(deps.clone(), &task_id, start_stage).await {
            mark_blocked(&deps, &task_id, &err.to_string(), true);
        }
    });
}

pub fn resume_after_gate(deps: PipelineDeps, task_id: String, approved_stage: i64) -> bool {
    let next = approved_stage + 1;
    let changed = match deps.db.open() {
        Ok(conn) => crate::task_lifecycle::apply(
            &conn,
            &task_id,
            crate::task_lifecycle::TaskTransition::ResumeAfterGate { approved_stage },
            "",
        )
        .map(|outcome| outcome.changed)
        .unwrap_or(false),
        Err(_) => false,
    };
    if !changed {
        if let Ok(conn) = deps.db.open() {
            let _ = log_event(
                &conn,
                None,
                Some(&task_id),
                None,
                "task.gate_stale",
                Some("orchestrator"),
                json!({ "stageIndex": approved_stage }),
            );
        }
        return false;
    }
    emit_task(&deps.app, &task_id, "queued", next, json!({}));
    spawn_pipeline(deps, task_id, next);
    true
}
