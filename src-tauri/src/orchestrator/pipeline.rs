//! Stage loop for a task pipeline.

use super::context::{
    agent_of, build_prompt, previous_context, seats_of, task_ctx, AgentCtx, SeatCtx, TaskCtx,
};
use super::gate::wait_run_approval;
use super::{emit_task, mark_blocked, PipelineDeps};
use crate::db::{log_event, new_id, now_ms};
use crate::error::{AppError, Result};
use crate::runtime::{self, RunRequest};
use rusqlite::{params, Connection};
use serde_json::json;
use std::path::PathBuf;
use tauri::Emitter;

fn insert_stage_message(conn: &Connection, task: &TaskCtx, seat: &SeatCtx, agent: &AgentCtx) {
    let content = crate::i18n::tr(
        &format!("▶ 阶段「{}」开始 · {}", seat.role_name, agent.name),
        &format!("▶ Stage \"{}\" started · {}", seat.role_name, agent.name),
    );
    let _ = crate::db::insert_message(
        conn,
        Some(&task.id),
        None,
        Some(&seat.id),
        Some(&agent.id),
        "system",
        Some("system"),
        &content,
        json!({ "stageKey": seat.role_key }),
    );
}

pub(super) async fn run_pipeline(
    deps: PipelineDeps,
    task_id: &str,
    start_stage: i64,
) -> Result<()> {
    let mut stage = start_stage.max(0);
    loop {
        let (task, seats, current_status) = {
            let conn = deps.db.open()?;
            let task = task_ctx(&conn, task_id)?;
            if task.team_id.is_empty() {
                return Err(AppError::Msg("任务未绑定团队，请先在任务中指定团队".into()));
            }
            let seats = seats_of(&conn, &task.team_id)?;
            if seats.is_empty() {
                return Err(AppError::Msg("团队没有席位，请先配置团队".into()));
            }
            let current_status: String = conn.query_row(
                "SELECT status FROM tasks WHERE id = ?1",
                params![task_id],
                |r| r.get(0),
            )?;
            (task, seats, current_status)
        };

        // 取消/重置/手动终态后，流水线不得继续推进下一阶段。
        if !crate::task_lifecycle::TaskStatus::parse(&current_status)
            .map(crate::task_lifecycle::TaskStatus::is_active)
            .unwrap_or(false)
        {
            let conn = deps.db.open()?;
            let _ = log_event(
                &conn,
                Some(task.project_id.as_str()),
                Some(task_id),
                None,
                "task.stale",
                Some("orchestrator"),
                json!({ "status": current_status, "stageIndex": stage }),
            );
            return Ok(());
        }

        if stage as usize >= seats.len() {
            let conn = deps.db.open()?;
            let result = json!({ "stages": seats.len() }).to_string();
            let outcome = crate::task_lifecycle::apply(
                &conn,
                task_id,
                crate::task_lifecycle::TaskTransition::Complete {
                    stage: None,
                    result_json: &result,
                },
                "",
            )?;
            let changed = outcome.changed;
            if changed {
                let _ = log_event(
                    &conn,
                    Some(task.project_id.as_str()),
                    Some(task_id),
                    None,
                    "task.completed",
                    Some("orchestrator"),
                    json!({ "stages": seats.len() }),
                );
            }
            if let Err(err) = crate::review::refresh_review_flag(&conn, task_id) {
                tracing::warn!("刷新待审查标记失败：{err}");
            }
            if changed {
                emit_task(&deps.app, task_id, "completed", stage, json!({}));
                crate::notify::send_task(
                    &deps.app,
                    &crate::i18n::tr("Lumen 任务完成", "Lumen task completed"),
                    &crate::i18n::tr(
                        &format!("「{}」已完成，点击查看结果", task.title),
                        &format!("\"{}\" is complete. Click to view the result", task.title),
                    ),
                    Some(task_id),
                    "task_completed",
                );
            }
            return Ok(());
        }

        let seat = &seats[stage as usize];
        let Some(agent_id) = seat.agent_id.clone() else {
            return Err(AppError::Msg(format!(
                "阶段「{}」尚未指派 Agent，请先到团队席位页完成指派",
                seat.role_name
            )));
        };

        let agent = {
            let conn = deps.db.open()?;
            agent_of(&conn, &agent_id)?
        };
        if agent.command.trim().is_empty() {
            return Err(AppError::Msg(format!(
                "Agent「{}」未配置启动命令",
                agent.name
            )));
        }
        let project_path = PathBuf::from(&task.project_path);
        if !project_path.is_dir() {
            return Err(AppError::Msg(format!(
                "项目目录不存在：{}",
                task.project_path
            )));
        }

        // 非 Claude 适配器没有工具级 Hook，运行前补一道人工放行；
        // 任务授权为 auto 时视为用户已预授权，不再逐阶段打断。
        if deps.settings.non_claude_run_approval
            && agent.adapter != "claude-code"
            && task.mandate != "auto"
        {
            wait_run_approval(&deps, &task, seat, &agent, stage).await?;
        }

        let run_id = new_id();
        let budget_usd;
        let stage_summary;
        {
            let conn = deps.db.open()?;
            crate::budget::check_dispatch(
                &conn,
                &task.project_id,
                &task.team_id,
                &agent.id,
                task_id,
            )?;
            budget_usd = crate::budget::usd_headroom(
                &conn,
                &task.project_id,
                &task.team_id,
                &agent.id,
                task_id,
            );
            let started = crate::task_lifecycle::apply(
                &conn,
                task_id,
                crate::task_lifecycle::TaskTransition::StartStage { stage },
                "",
            )?;
            if !started.changed {
                return Ok(());
            }
            conn.execute(
                "UPDATE tasks SET started_at = COALESCE(started_at, ?1), stage_key = ?2 WHERE id = ?3",
                params![now_ms(), seat.role_key, task_id],
            )?;
            insert_stage_message(&conn, &task, seat, &agent);
            log_event(
                &conn,
                Some(&task.project_id),
                Some(task_id),
                Some(&run_id),
                "task.stage_started",
                Some("orchestrator"),
                json!({ "stageIndex": stage, "role": seat.role_name, "agent": agent.name }),
            )?;
            let previous = previous_context(&conn, task_id);
            let prompt = build_prompt(&task, seat, &agent, &previous);
            // 同一任务同一席位复用上次成功会话，避免重复计费与丢失上下文。
            let resume_session = if agent.adapter == "claude-code" {
                conn.query_row(
                    "SELECT external_session_id FROM runs
                     WHERE task_id = ?1 AND seat_id = ?2 AND status = 'completed'
                       AND external_session_id IS NOT NULL
                     ORDER BY started_at DESC LIMIT 1",
                    params![task_id, seat.id],
                    |r| r.get::<_, String>(0),
                )
                .ok()
            } else {
                None
            };
            drop(conn);

            emit_task(
                &deps.app,
                task_id,
                "running",
                stage,
                json!({ "runId": run_id, "roleName": seat.role_name }),
            );

            let req = RunRequest {
                run_id: run_id.clone(),
                task_id: task_id.to_string(),
                project_id: task.project_id.clone(),
                team_id: task.team_id.clone(),
                seat_id: Some(seat.id.clone()),
                seat_name: seat.role_name.clone(),
                agent_id: agent.id.clone(),
                agent_name: agent.name.clone(),
                adapter: agent.adapter.clone(),
                command: agent.command.clone(),
                args: agent.args.clone(),
                model: if !task.model.is_empty() {
                    task.model.clone()
                } else if agent.model.is_empty() {
                    "claude".into()
                } else {
                    agent.model.clone()
                },
                project_path,
                stage_key: Some(seat.role_key.clone()),
                prompt,
                resume_session,
                budget_usd,
            };
            let run_deps = runtime::RunDeps::for_app(
                deps.app.clone(),
                deps.db.clone(),
                deps.base_dir.clone(),
                deps.settings.clone(),
                deps.gate.clone(),
                deps.registry.clone(),
                deps.proxy.clone(),
            );
            let outcome = match runtime::RunSession::run(&run_deps, req).await {
                Ok(outcome) => outcome,
                Err(err) => {
                    // runtime 在占用并发槽之前失败时没有守卫，需要在这里归还请求级预留。
                    if let Ok(conn) = deps.db.open() {
                        let _ = crate::budget::release(&conn, &run_id);
                    }
                    return Err(err);
                }
            };

            if !outcome.success {
                let reason = outcome
                    .error
                    .clone()
                    .unwrap_or_else(|| "Agent 运行未成功完成".to_string());
                let paused = {
                    let conn = deps.db.open()?;
                    let _ = log_event(
                        &conn,
                        Some(&task.project_id),
                        Some(task_id),
                        Some(&run_id),
                        "task.stage_failed",
                        Some("orchestrator"),
                        json!({ "stageIndex": stage, "reason": reason }),
                    );
                    conn.query_row(
                        "SELECT status FROM tasks WHERE id = ?1",
                        params![task_id],
                        |r| r.get::<_, String>(0),
                    )
                    .map(|status| {
                        crate::task_lifecycle::TaskStatus::parse(&status)
                            == Some(crate::task_lifecycle::TaskStatus::Paused)
                    })
                    .unwrap_or(false)
                };
                // 用户主动暂停会终止子进程，运行失败属预期行为，不转为阻塞。
                if paused {
                    return Ok(());
                }
                mark_blocked(&deps, task_id, &reason, true);
                return Ok(());
            }
            stage_summary = outcome.final_text.chars().take(600).collect::<String>();
        }

        if seat.gate {
            let conn = deps.db.open()?;
            let title = crate::i18n::tr(
                &format!("阶段门禁：{} · {}", seat.role_name, task.title),
                &format!("Stage gate: {} · {}", seat.role_name, task.title),
            );
            let detail = json!({
                "stageIndex": stage,
                "nextStageIndex": stage + 1,
                "roleName": seat.role_name,
                "runId": run_id,
                "summary": stage_summary,
            });
            // 建审批与置等待态放在同一事务里：任务已被取消/重置时不再产生新的 pending 审批。
            let tx = conn.unchecked_transaction()?;
            let changed = crate::task_lifecycle::apply(
                &tx,
                task_id,
                crate::task_lifecycle::TaskTransition::WaitApproval { stage },
                "",
            )?
            .changed;
            if !changed {
                let _ = tx.rollback();
                let _ = log_event(
                    &conn,
                    Some(task.project_id.as_str()),
                    Some(task_id),
                    Some(&run_id),
                    "task.gate_skipped",
                    Some("orchestrator"),
                    json!({ "stageIndex": stage }),
                );
                return Ok(());
            }
            let approval_id = crate::approval::create_stage_approval(
                &tx,
                task_id,
                Some(&run_id),
                &title,
                detail,
            )?;
            tx.commit()?;
            let _ = deps.app.emit(
                "approval.requested",
                json!({
                    "approvalId": approval_id,
                    "type": "stage_gate",
                    "title": title,
                    "taskId": task_id,
                    "stageIndex": stage
                }),
            );
            emit_task(
                &deps.app,
                task_id,
                "waiting_approval",
                stage,
                json!({ "approvalId": approval_id }),
            );
            crate::notify::send_task(
                &deps.app,
                &crate::i18n::tr("Lumen 审批请求", "Lumen approval request"),
                &crate::i18n::tr(
                    &format!("「{}」第 {} 阶段等待你的确认", task.title, stage + 1),
                    &format!(
                        "\"{}\" is waiting for your confirmation at stage {}",
                        task.title,
                        stage + 1
                    ),
                ),
                Some(task_id),
                "approval",
            );
            return Ok(());
        }

        stage += 1;
        {
            let conn = deps.db.open()?;
            if stage as usize >= seats.len() {
                let result = json!({ "stages": seats.len() }).to_string();
                let changed = crate::task_lifecycle::apply(
                    &conn,
                    task_id,
                    crate::task_lifecycle::TaskTransition::Complete {
                        stage: Some(stage),
                        result_json: &result,
                    },
                    "",
                )?
                .changed;
                if changed {
                    let _ = log_event(
                        &conn,
                        Some(&task.project_id),
                        Some(task_id),
                        Some(&run_id),
                        "task.completed",
                        Some("orchestrator"),
                        json!({ "stages": seats.len() }),
                    );
                }
                if let Err(err) = crate::review::refresh_review_flag(&conn, task_id) {
                    tracing::warn!("刷新待审查标记失败：{err}");
                }
                if changed {
                    emit_task(&deps.app, task_id, "completed", stage, json!({}));
                    crate::notify::send_task(
                        &deps.app,
                        &crate::i18n::tr("Lumen 任务完成", "Lumen task completed"),
                        &crate::i18n::tr(
                            &format!("「{}」已完成，点击查看结果", task.title),
                            &format!("\"{}\" is complete. Click to view the result", task.title),
                        ),
                        Some(task_id),
                        "task_completed",
                    );
                }
                return Ok(());
            }
            let advanced = crate::task_lifecycle::apply(
                &conn,
                task_id,
                crate::task_lifecycle::TaskTransition::AdvanceStage { stage },
                "",
            )?;
            if !advanced.changed {
                return Ok(());
            }
        }
        emit_task(&deps.app, task_id, "running", stage, json!({}));
    }
}
