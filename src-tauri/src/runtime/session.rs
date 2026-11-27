//! 一次运行的完整执行流程。
//!
//! 阶段顺序固定：占用并发槽 → 落库 → 启动进程 → 消费输出流 → 终态判定 → 收尾。
//! 事件顺序也固定：`run.status(running)` → `run.delta` / `usage.updated` → 恰好一次
//! 终态 `run.status`。阶段循环、阶段门禁与任务状态流转仍由 orchestrator 负责。

use super::events::RunEvents;
use super::launch::{
    is_cmd_program, opencode_config, opencode_model, opencode_model_ref, spawn_spec, with_v1,
};
use super::policy;
use super::store::{FinishRecord, MessageRecord, RunStore};
use super::stream::{MessageOrigin, RunEffect, StreamInterpreter, UsageDelta};
use super::{
    handle_alerts, spawn_watchdog, RunOutcome, RunRegistry, RunRequest, SandboxSession,
    WatchdogGuard,
};
#[cfg(windows)]
use super::{log_sandbox_fallback, prepare_sandbox};
use crate::approval::{GateRunCtx, GateState};
use crate::db::{now_ms, Db};
use crate::error::{AppError, Result};
use crate::llm_proxy::{ProxyRun, ProxyState};
use crate::models::AppSettings;
use crate::usage::UsageInput;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// 一次运行所需的依赖。
///
/// 生产环境由 `execute` 传入 `Some(app)`；测试可传 `None`，此时跳过界面告警、
/// 系统通知与看门狗，但用量记账和预算终检仍然执行。
pub struct RunDeps {
    pub(crate) app: Option<AppHandle>,
    pub(crate) db: Db,
    pub(crate) base_dir: PathBuf,
    pub(crate) settings: AppSettings,
    pub(crate) gate: Arc<GateState>,
    pub(crate) registry: Arc<RunRegistry>,
    pub(crate) proxy: Arc<ProxyState>,
    pub(crate) events: Arc<dyn RunEvents>,
}

/// 只负责一次执行尝试；不推进阶段，也不修改任务状态。
pub struct RunSession;

impl RunSession {
    pub async fn run(deps: &RunDeps, req: RunRequest) -> Result<RunOutcome> {
        let Some(cancelled) = deps.registry.register(
            &req.run_id,
            &req.task_id,
            deps.settings.max_concurrent_runs.max(1) as usize,
        ) else {
            return Err(AppError::Msg(
                "并发运行已达上限，请等待当前任务结束或调整设置".into(),
            ));
        };
        let store = RunStore::new(deps.db.clone());
        // 守卫先于任何可能失败的 IO 建立，保证早退路径也释放槽位与预算。
        let mut guard = RunGuard {
            gate: deps.gate.clone(),
            registry: deps.registry.clone(),
            proxy: deps.proxy.clone(),
            store: store.clone(),
            run_id: req.run_id.clone(),
            agent_id: req.agent_id.clone(),
            finalized: false,
        };
        let tpath = transcript_path(&deps.base_dir, &req.run_id);
        let run_dir = deps.base_dir.join("sessions").join(&req.run_id);
        std::fs::create_dir_all(&run_dir)?;
        let _run_dir_guard = TempRunDir(run_dir.clone());

        store.record_start(&req)?;
        deps.events.emit(
            "run.status",
            json!({ "runId": req.run_id, "status": "running", "taskId": req.task_id }),
        );

        // 一次性 token 只存在于内存，通过环境变量交给 hook 进程；不落盘、不进命令行。
        let gate_token = deps.gate.register(GateRunCtx {
            run_id: req.run_id.clone(),
            task_id: req.task_id.clone(),
            project_id: req.project_id.clone(),
            project_path: req.project_path.to_string_lossy().to_string(),
            seat_name: req.seat_name.clone(),
            agent_name: req.agent_name.clone(),
        });

        let settings_path = run_dir.join("settings.json");
        let is_claude = req.adapter == "claude-code";
        let exe = std::env::current_exe().unwrap_or_default();
        let hook_cmd = format!(
            "\"{}\" --hook-gate --run-id {}",
            exe.to_string_lossy(),
            req.run_id
        );
        let mut hooks = json!({
            "PreToolUse": [{
                // 覆盖内置写/执行工具与全部 MCP 工具，MCP 调用同样要经过审批网关。
                "matcher": "Bash|Write|Edit|MultiEdit|NotebookEdit|WebFetch|mcp__.*",
                "hooks": [{ "type": "command", "command": hook_cmd, "timeout": 600 }]
            }]
        });
        if is_claude {
            let conn = deps.db.open()?;
            let extra = crate::extensions::extra_hooks(&conn)?;
            merge_hooks(&mut hooks, &extra);
        }
        let settings_json = json!({ "hooks": hooks });
        std::fs::write(&settings_path, settings_json.to_string())?;

        let provider = {
            let conn = deps.db.open()?;
            crate::providers::resolved(&conn, &req.agent_id)?
        };
        let use_proxy = deps.settings.proxy_enabled && provider.is_some();
        // 关闭代理时直接注入供应商环境；真实 Key 因此会进入子进程，不能启用文件级沙箱。
        let direct_provider_env = if !deps.settings.proxy_enabled && provider.is_some() {
            let conn = deps.db.open()?;
            Some(crate::providers::env_for_agent(&conn, &req.agent_id)?)
        } else {
            None
        };

        let mut mcp_config_path: Option<PathBuf> = None;
        let mut plugin_dirs: Vec<PathBuf> = Vec::new();
        if is_claude {
            let conn = deps.db.open()?;
            mcp_config_path = crate::extensions::mcp_config(&conn, &run_dir)?;
            plugin_dirs = crate::extensions::plugin_dirs(&conn, &deps.base_dir)?;
        }

        let mut args: Vec<String> = Vec::new();
        if is_claude {
            args.extend([
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--include-partial-messages".to_string(),
                "--verbose".to_string(),
                "--permission-mode".to_string(),
                "default".to_string(),
                "--settings".to_string(),
                settings_path.to_string_lossy().to_string(),
            ]);
            if let Some(path) = &mcp_config_path {
                args.push("--mcp-config".to_string());
                args.push(path.to_string_lossy().to_string());
                args.push("--strict-mcp-config".to_string());
            }
            for dir in &plugin_dirs {
                args.push("--plugin-dir".to_string());
                args.push(dir.to_string_lossy().to_string());
            }
            if let Some(session) = &req.resume_session {
                args.push("--resume".to_string());
                args.push(session.clone());
            } else {
                args.push("--session-id".to_string());
                args.push(req.run_id.clone());
            }
            if let Some(budget) = req.budget_usd {
                args.push("--max-budget-usd".to_string());
                args.push(format!("{:.4}", budget.max(0.01)));
            }
        } else {
            args.extend(req.args.iter().cloned());
            if req.adapter == "opencode" {
                if let Some(provider) = provider.as_ref() {
                    if let Some(model) = opencode_model(provider.model.as_deref(), &req.model) {
                        if let Some(reference) =
                            opencode_model_ref(use_proxy, &provider.protocol, &model)
                        {
                            args.push("-m".into());
                            args.push(reference);
                        }
                    }
                }
            }
        }

        tracing::info!(
            "spawning run {} via {} ({} args)",
            req.run_id,
            req.command,
            args.len()
        );
        // 直接 spawn 可执行文件；只有 .cmd/.bat 或用户填写的完整命令行才经过 cmd，
        // 任务文本（prompt）永远不再拼进命令行，杜绝 shell 注入。
        let (program, proc_args) = spawn_spec(&req.command, &args);
        let mut proc_args = proc_args;
        let mut program_path = PathBuf::from(&program);
        let mut via_cmd = is_cmd_program(&program);

        let mut proxy_token: Option<String> = None;
        if use_proxy {
            let provider = provider.as_ref().expect("provider checked");
            let token = deps.proxy.register(ProxyRun {
                run_id: req.run_id.clone(),
                agent_id: req.agent_id.clone(),
                project_id: req.project_id.clone(),
                team_id: req.team_id.clone(),
                task_id: req.task_id.clone(),
                seat_id: req.seat_id.clone(),
                protocol: provider.protocol.clone(),
                base_url: provider.base_url.clone(),
                api_key: provider.api_key.clone(),
                model: provider.model.clone().unwrap_or_else(|| req.model.clone()),
                notify: deps.settings.notify,
            });
            proxy_token = Some(token);
        }

        // 沙箱准备：仅 Windows + 设置开启 + 代理模式时启用。
        let mut sandbox: Option<SandboxSession> = None;
        let mut isolation = "process";
        let mut isolation_reason: Option<String> = None;
        #[cfg(windows)]
        if deps.settings.sandbox_enabled {
            if !use_proxy {
                let (reason, warning) = if direct_provider_env.is_some() {
                    (
                        "proxy_disabled",
                        "模型代理已关闭，真实 Key 不应进入文件隔离容器；本次使用进程隔离",
                    )
                } else {
                    (
                        "provider_unavailable",
                        "未找到可用的模型供应商；本次使用进程隔离",
                    )
                };
                log_sandbox_fallback(deps.events.as_ref(), &store, &req, reason, warning);
                isolation_reason = Some(reason.to_string());
            } else {
                match prepare_sandbox(
                    &req,
                    &deps.base_dir,
                    &run_dir,
                    &program,
                    &proc_args,
                    &plugin_dirs,
                    is_claude,
                    deps.gate.port,
                    deps.proxy.port,
                ) {
                    Ok(run) => {
                        isolation = "appcontainer";
                        program_path = run.program;
                        proc_args = run.args;
                        via_cmd = run.via_cmd;
                        sandbox = Some(run.session);
                    }
                    Err(fallback) => {
                        log_sandbox_fallback(
                            deps.events.as_ref(),
                            &store,
                            &req,
                            fallback.code,
                            &fallback.message,
                        );
                        isolation_reason = Some(fallback.code.to_string());
                    }
                }
            }
        }
        if !is_claude && !via_cmd {
            proc_args.push(req.prompt.clone());
        }

        let mut env_overrides: HashMap<String, String> = HashMap::new();
        let mut env_removals: Vec<String> = [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_MODEL",
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "OPENAI_API_BASE",
            "OPENAI_MODEL",
            "OPENCODE_CONFIG_CONTENT",
            "LUMEN_GATE_URL",
            "LUMEN_GATE_TOKEN",
            "LUMEN_RUN_ID",
        ]
        .iter()
        .map(|name| name.to_string())
        .collect();
        let mut proxy_base: Option<String> = None;
        if use_proxy {
            let token = proxy_token.clone().unwrap_or_default();
            // 沙箱内必须连接容器 relay 的端口，宿主再桥接到真实代理端口。
            #[cfg(windows)]
            let port = sandbox
                .as_ref()
                .and_then(|s| s.container_port("proxy"))
                .unwrap_or(deps.proxy.port);
            #[cfg(not(windows))]
            let port = deps.proxy.port;
            let base = format!("http://127.0.0.1:{port}/proxy/{token}");
            proxy_base = Some(base.clone());
            let provider = provider.as_ref().expect("provider checked");
            match provider.protocol.as_str() {
                "anthropic" => {
                    env_overrides.insert("ANTHROPIC_BASE_URL".into(), base);
                    env_overrides.insert("ANTHROPIC_AUTH_TOKEN".into(), token);
                    if let Some(model) = &provider.model {
                        env_overrides.insert("ANTHROPIC_MODEL".into(), model.clone());
                    }
                }
                "openai" => {
                    env_overrides.insert("OPENAI_BASE_URL".into(), base);
                    env_overrides.insert("OPENAI_API_KEY".into(), token);
                    if let Some(model) = &provider.model {
                        env_overrides.insert("OPENAI_MODEL".into(), model.clone());
                    }
                }
                _ => {}
            }
        } else if let Some((provider_env, provider_removals)) = direct_provider_env {
            env_removals.extend(provider_removals);
            env_overrides.extend(provider_env);
        }
        if is_claude {
            // 审批网关上下文只交给 Claude Code：hook 由它的 settings.json 触发。
            #[cfg(windows)]
            let gate_port = sandbox
                .as_ref()
                .and_then(|s| s.container_port("gate"))
                .unwrap_or(deps.gate.port);
            #[cfg(not(windows))]
            let gate_port = deps.gate.port;
            env_overrides.insert(
                "LUMEN_GATE_URL".into(),
                format!("http://127.0.0.1:{gate_port}"),
            );
            env_overrides.insert("LUMEN_GATE_TOKEN".into(), gate_token.clone());
            env_overrides.insert("LUMEN_RUN_ID".into(), req.run_id.clone());
        }
        // OpenCode 1.x 不读取 OPENAI_MODEL；模型必须在 argv 中显式指定，并在
        // 临时配置里登记，否则会回退到它自带目录中的默认模型。
        if req.adapter == "opencode" {
            if let Some(provider) = provider.as_ref() {
                if let Some(model) = opencode_model(provider.model.as_deref(), &req.model) {
                    if provider.protocol == "anthropic" && use_proxy {
                        if let Some(token) = proxy_token.as_ref() {
                            env_overrides.insert("ANTHROPIC_API_KEY".into(), token.clone());
                        }
                    }
                    let base = if let Some(base) = proxy_base.as_deref() {
                        format!("{}/v1", base.trim_end_matches('/'))
                    } else {
                        with_v1(&provider.base_url)
                    };
                    let config = opencode_config(use_proxy, &provider.protocol, &base, &model);
                    env_overrides.insert("OPENCODE_CONFIG_CONTENT".into(), config.to_string());
                }
            }
        }
        if sandbox.is_some() {
            let home = deps.base_dir.join("sandbox-home");
            let home_text = home.to_string_lossy().to_string();
            let tmp = home.join("tmp");
            std::fs::create_dir_all(&tmp)
                .map_err(|e| AppError::Msg(format!("创建沙箱 TEMP {} 失败：{e}", tmp.display())))?;
            env_overrides.insert("USERPROFILE".into(), home_text.clone());
            env_overrides.insert("HOME".into(), home_text);
            env_overrides.insert("TEMP".into(), tmp.to_string_lossy().to_string());
            env_overrides.insert("TMP".into(), tmp.to_string_lossy().to_string());
        }

        // 沙箱模式下启动宿主进程，由它把容器 CLI 的标准输入输出转发回来。
        #[cfg(windows)]
        let sandbox_spec_path = sandbox.as_ref().map(|session| {
            let spec_path = run_dir.join("host.json");
            let spec = session.host_spec(
                &program_path,
                &proc_args,
                &req.project_path,
                env_overrides.clone(),
                &run_dir.join("relay-ready.json"),
                &run_dir.join("host-status.json"),
            );
            if let Err(err) =
                std::fs::write(&spec_path, serde_json::to_vec(&spec).unwrap_or_default())
            {
                tracing::warn!("写入沙箱宿主配置失败：{err}");
            }
            spec_path
        });
        #[cfg(not(windows))]
        let sandbox_spec_path: Option<PathBuf> = None;

        let mut command = if let Some(spec_path) = &sandbox_spec_path {
            let mut cmd = tokio::process::Command::new(&exe);
            cmd.arg("--sandbox-host").arg(spec_path);
            cmd
        } else {
            let mut cmd = tokio::process::Command::new(&program_path);
            for arg in &proc_args {
                cmd.arg(arg);
            }
            cmd
        };
        // P0-1：先清掉继承环境里的真实密钥，任何适配器都不能再直接拿到 Key。
        for name in &env_removals {
            command.env_remove(name);
        }
        for (name, value) in &env_overrides {
            command.env(name, value);
        }
        command
            .current_dir(&req.project_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .map_err(|e| AppError::Msg(format!("启动 Agent 失败：{e}")))?;
        if isolation != "process" {
            store.set_isolation(&req.run_id, isolation);
            deps.events.emit(
                "run.status",
                json!({
                    "runId": req.run_id,
                    "status": "running",
                    "taskId": req.task_id,
                    "isolation": isolation,
                    "isolationReason": isolation_reason
                }),
            );
        }
        if let Some(pid) = child.id() {
            deps.registry.set_pid(&req.run_id, pid);
            // P0-3：加入 Job Object，退出/取消时由内核终止整棵进程树。
            if let Some(job) = crate::sandbox::create_job() {
                if !job.assign(pid) {
                    tracing::warn!("assign process {pid} to job object failed");
                }
                deps.registry.set_job(&req.run_id, Arc::new(job));
            }
        }
        let _watchdog = WatchdogGuard(spawn_watchdog(
            deps.app.clone(),
            deps.events.clone(),
            deps.db.clone(),
            deps.registry.clone(),
            req.clone(),
            deps.settings.clone(),
        ));
        if is_claude || via_cmd {
            if let Some(mut stdin) = child.stdin.take() {
                let prompt = req.prompt.clone();
                tokio::spawn(async move {
                    let _ = stdin.write_all(prompt.as_bytes()).await;
                    let _ = stdin.shutdown().await;
                });
            }
        } else if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.shutdown().await;
        }

        if let Some(stderr) = child.stderr.take() {
            let tpath2 = tpath.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    append_line(&tpath2, &line);
                }
            });
        }

        let mut progress = RunProgress::default();
        let mut interpreter = StreamInterpreter::new(
            &req.adapter,
            &req.run_id,
            &req.model,
            &req.prompt,
            use_proxy,
        );

        if let Some(stdout) = child.stdout.take() {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                append_line(&tpath, &line);
                if line.trim().is_empty() {
                    continue;
                }
                for effect in interpreter.feed(&line) {
                    apply_run_effect(effect, deps, &req, &store, &mut progress);
                }
            }
        }
        for effect in interpreter.finish() {
            apply_run_effect(effect, deps, &req, &store, &mut progress);
        }
        let status = child.wait().await;
        let exit_code = status.as_ref().ok().and_then(|s| s.code()).unwrap_or(-1);

        // 宿主在 relay 启动失败等场景会写状态文件，优先展示更具体的原因。
        #[cfg(windows)]
        if let Some(spec_path) = &sandbox_spec_path {
            let status_path = spec_path.with_file_name("host-status.json");
            if let Ok(text) = std::fs::read_to_string(&status_path) {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
                        if progress.outcome.error.is_none() {
                            progress.outcome.error = Some(format!("沙箱宿主错误：{err}"));
                        }
                    }
                }
            }
        }

        // 代理已开启但没有捕获到任何模型流量（适配器不识别注入变量等）：回退估算，避免完全漏计。
        if use_proxy && !deps.proxy.usage_recorded(&req.run_id) {
            let text = if interpreter.text_buffer().trim().is_empty() {
                progress.outcome.final_text.clone()
            } else {
                interpreter.text_buffer().to_string()
            };
            if let Ok(conn) = deps.db.open() {
                let input_tokens =
                    ((req.prompt.chars().count() as f64) / 4.0).ceil().max(1.0) as i64;
                let output_tokens = ((text.chars().count() as f64) / 4.0).ceil() as i64;
                let input = UsageInput {
                    run_id: Some(req.run_id.clone()),
                    task_id: Some(req.task_id.clone()),
                    project_id: Some(req.project_id.clone()),
                    team_id: Some(req.team_id.clone()),
                    seat_id: req.seat_id.clone(),
                    agent_id: Some(req.agent_id.clone()),
                    provider: crate::usage::provider_for(&req.model),
                    model: req.model.clone(),
                    input_tokens,
                    output_tokens,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    source: "estimate".into(),
                    dedupe_key: format!("estimate:{}:fallback", req.run_id),
                };
                if store.record_usage_on(&conn, &input).is_some() {
                    deps.events
                        .emit("usage.updated", json!({ "runId": req.run_id }));
                    progress.budget_blocked |= handle_alerts(
                        deps.app.as_ref(),
                        &conn,
                        &req,
                        &deps.registry,
                        deps.settings.notify,
                    );
                }
            }
        }

        // 预算终检：代理侧可能在本次运行中已触发 402 硬停，即便进程正常退出也要暂停。
        if let Ok(conn) = deps.db.open() {
            let scope = crate::budget::BudgetScope {
                project_id: &req.project_id,
                team_id: &req.team_id,
                agent_id: &req.agent_id,
                task_id: &req.task_id,
                run_id: &req.run_id,
            };
            progress.budget_blocked |= deps.proxy.was_blocked(&req.run_id)
                || crate::budget::spent_exhausted(&conn, &scope);
        }

        // P1：hook 绕过检测——流里出现过的受管工具，必须都能在网关侧找到同输入记录。
        let unseen = deps.gate.unseen_tools(&req.run_id, &progress.gated_calls);
        let hook_bypass = !unseen.is_empty();
        if hook_bypass {
            let calls: Vec<Value> = unseen
                .iter()
                .take(20)
                .map(|(tool, hash)| json!({ "tool": tool, "inputHash": hash }))
                .collect();
            store.record_event(
                &req,
                "hook.bypass",
                Some("gate"),
                json!({ "calls": calls, "count": unseen.len() }),
            );
            deps.events.emit(
                "run.delta",
                json!({
                    "runId": req.run_id,
                    "kind": "warning",
                    "warning": format!("检测到 {} 次未经审批网关的工具调用", unseen.len())
                }),
            );
        }

        let cancelled_now = cancelled.load(Ordering::SeqCst);
        let (run_status, error) = policy::evaluate(policy::OutcomeInputs {
            budget_blocked: progress.budget_blocked,
            hook_bypass,
            hook_strict: deps.settings.hook_strict,
            cancelled: cancelled_now,
            exit_code,
            reported_error: progress.outcome.error.as_deref(),
        });
        let run_status = run_status.as_db_str();
        progress.outcome.success = run_status == "completed";
        if progress.outcome.error.is_none() {
            progress.outcome.error = error.clone();
        }

        if store.finish(FinishRecord {
            project_id: &req.project_id,
            task_id: &req.task_id,
            run_id: &req.run_id,
            agent_id: &req.agent_id,
            agent_name: &req.agent_name,
            status: run_status,
            exit_code,
            error: error.as_deref(),
            session_id: progress.outcome.session_id.as_deref(),
            isolation,
            isolation_reason: isolation_reason.as_deref(),
        }) {
            deps.events.emit(
                "run.status",
                json!({
                    "runId": req.run_id,
                    "status": run_status,
                    "taskId": req.task_id,
                    "error": error,
                    "isolation": isolation,
                    "isolationReason": isolation_reason
                }),
            );
        }
        // 正常结束：标记守卫已完成，避免 Drop 再写一次 failed。
        guard.finalized = true;

        Ok(progress.outcome)
    }
}

/// 运行生命周期守卫：任何早退、失败或取消都会归还并发槽、审批上下文、代理上下文与预算预留。
struct RunGuard {
    gate: Arc<GateState>,
    registry: Arc<RunRegistry>,
    proxy: Arc<ProxyState>,
    store: RunStore,
    run_id: String,
    agent_id: String,
    finalized: bool,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.gate.unregister(&self.run_id);
        self.proxy.unregister(&self.run_id);
        self.registry.finish(&self.run_id);
        if self.finalized {
            return;
        }
        self.store.mark_interrupted(&self.run_id, &self.agent_id);
    }
}

fn transcript_path(base: &Path, run_id: &str) -> PathBuf {
    base.join("sessions").join(format!("{run_id}.ndjson"))
}

/// 运行结束（含失败/取消）后自动删除本次运行生成的临时配置。
struct TempRunDir(PathBuf);

impl Drop for TempRunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn merge_hooks(base: &mut Value, extra: &Value) {
    let (Some(base_map), Some(extra_map)) = (base.as_object_mut(), extra.as_object()) else {
        return;
    };
    for (event, entries) in extra_map {
        let Some(extra_arr) = entries.as_array() else {
            continue;
        };
        let slot = base_map.entry(event.clone()).or_insert_with(|| json!([]));
        if let Some(arr) = slot.as_array_mut() {
            arr.extend(extra_arr.iter().cloned());
        }
    }
}

fn append_line(path: &Path, line: &str) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

fn usage_input(req: &RunRequest, delta: &UsageDelta) -> UsageInput {
    UsageInput {
        run_id: Some(req.run_id.clone()),
        task_id: Some(req.task_id.clone()),
        project_id: Some(req.project_id.clone()),
        team_id: Some(req.team_id.clone()),
        seat_id: req.seat_id.clone(),
        agent_id: Some(req.agent_id.clone()),
        provider: crate::usage::provider_for(&delta.model),
        model: delta.model.clone(),
        input_tokens: delta.input_tokens,
        output_tokens: delta.output_tokens,
        cache_read_tokens: delta.cache_read_tokens,
        cache_write_tokens: delta.cache_write_tokens,
        source: delta.source.to_string(),
        dedupe_key: delta.dedupe_key.clone(),
    }
}

#[derive(Default)]
struct RunProgress {
    outcome: RunOutcome,
    usage_recorded: bool,
    budget_blocked: bool,
    gated_calls: Vec<(String, String)>,
}

fn apply_run_effect(
    effect: RunEffect,
    deps: &RunDeps,
    req: &RunRequest,
    store: &RunStore,
    progress: &mut RunProgress,
) {
    match effect {
        RunEffect::SessionObserved { session_id } => {
            progress.outcome.session_id = Some(session_id.clone());
            store.set_session(&req.run_id, &session_id);
        }
        RunEffect::ChatMessage {
            model,
            text,
            origin,
        } => {
            let meta = match origin {
                MessageOrigin::Claude => json!({
                    "model": model,
                    "sessionId": progress.outcome.session_id.as_deref()
                }),
                MessageOrigin::Stdout => json!({ "source": "stdout" }),
            };
            if let Ok(msg_id) = store.insert_message(MessageRecord {
                task_id: &req.task_id,
                run_id: &req.run_id,
                seat_id: req.seat_id.as_deref(),
                agent_id: &req.agent_id,
                kind: "chat",
                role: Some("assistant"),
                content: &text,
                meta,
            }) {
                if matches!(origin, MessageOrigin::Claude) {
                    store.record_event(
                        req,
                        "agent.message",
                        Some(&req.agent_name),
                        json!({ "messageId": msg_id, "chars": text.chars().count() }),
                    );
                }
                let payload = match origin {
                    MessageOrigin::Claude => json!({
                        "runId": req.run_id,
                        "kind": "message",
                        "message": {
                            "id": msg_id,
                            "taskId": req.task_id,
                            "runId": req.run_id,
                            "seatId": req.seat_id,
                            "agentId": req.agent_id,
                            "kind": "chat",
                            "role": "assistant",
                            "content": text,
                            "sessionId": progress.outcome.session_id.as_deref(),
                            "createdAt": now_ms()
                        }
                    }),
                    MessageOrigin::Stdout => json!({
                        "runId": req.run_id,
                        "kind": "message",
                        "message": {
                            "taskId": req.task_id,
                            "runId": req.run_id,
                            "seatId": req.seat_id,
                            "agentId": req.agent_id,
                            "kind": "chat",
                            "role": "assistant",
                            "content": text,
                            "createdAt": now_ms()
                        }
                    }),
                };
                deps.events.emit("run.delta", payload);
                if matches!(origin, MessageOrigin::Stdout) && progress.outcome.final_text.is_empty()
                {
                    progress.outcome.final_text = text.clone();
                }
            }
        }
        RunEffect::ToolCall {
            tool_id,
            tool_name,
            input,
            gated,
        } => {
            if gated {
                progress
                    .gated_calls
                    .push((tool_name.clone(), crate::approval::input_hash(&input)));
            }
            let call_id = format!("{}:{}", req.run_id, tool_id);
            store.insert_tool_call(&call_id, &req.run_id, &tool_name, &input);
            deps.events.emit(
                "run.delta",
                json!({
                    "runId": req.run_id,
                    "kind": "tool_call",
                    "toolCall": {
                        "id": call_id,
                        "toolName": tool_name,
                        "input": input,
                        "status": "running"
                    }
                }),
            );
        }
        RunEffect::ToolResult { tool_id, output } => {
            let call_id = format!("{}:{}", req.run_id, tool_id);
            store.complete_tool_call(&call_id, &output);
            deps.events.emit(
                "run.delta",
                json!({
                    "runId": req.run_id,
                    "kind": "tool_result",
                    "toolCallId": call_id
                }),
            );
        }
        RunEffect::Usage(delta) => {
            let input = usage_input(req, &delta);
            if let Ok(conn) = deps.db.open() {
                if let Some(cost) = store.record_usage_on(&conn, &input) {
                    if delta.report_delta {
                        progress.usage_recorded = true;
                    }
                    if delta.report_updated {
                        deps.events
                            .emit("usage.updated", json!({ "runId": req.run_id }));
                    }
                    if delta.report_alert {
                        progress.budget_blocked |= handle_alerts(
                            deps.app.as_ref(),
                            &conn,
                            req,
                            &deps.registry,
                            deps.settings.notify,
                        );
                    }
                    if delta.report_delta {
                        deps.events.emit(
                            "run.delta",
                            json!({
                                "runId": req.run_id,
                                "kind": "usage",
                                "usage": {
                                    "inputTokens": delta.input_tokens,
                                    "outputTokens": delta.output_tokens,
                                    "cacheReadTokens": delta.cache_read_tokens,
                                    "cacheWriteTokens": delta.cache_write_tokens,
                                    "costMicros": cost
                                }
                            }),
                        );
                    }
                }
            }
        }
        RunEffect::RawLine { line } => {
            deps.events.emit(
                "run.delta",
                json!({ "runId": req.run_id, "kind": "raw", "line": line }),
            );
        }
        RunEffect::Result {
            final_text,
            session_id,
            error,
            fallback_usage,
        } => {
            if let Some(text) = final_text {
                progress.outcome.final_text = text;
            }
            if let Some(session_id) = session_id {
                progress.outcome.session_id = Some(session_id);
            }
            if let Some(error) = error {
                progress.outcome.error = Some(error);
            }
            if let Some(delta) = fallback_usage {
                if !progress.usage_recorded {
                    let input = usage_input(req, &delta);
                    if let Ok(conn) = deps.db.open() {
                        let _ = store.record_usage_on(&conn, &input);
                    }
                }
            }
        }
    }
}
