use crate::approval::{GateRunCtx, GateState};
use crate::db::{log_event, now_ms, Db};
use crate::error::{AppError, Result};
use crate::llm_proxy::{ProxyRun, ProxyState};
use crate::models::AppSettings;
use crate::sandbox::JobHandle;
use crate::usage::UsageInput;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

mod events;
mod policy;
mod store;

use events::{RunEvents, TauriEvents};
use store::{FinishRecord, MessageRecord, RunStore};

#[cfg(windows)]
use crate::sandbox::session::SandboxSession;
/// 非 Windows 平台不提供 AppContainer，仅用于保持类型可编译。
#[cfg(not(windows))]
struct SandboxSession;

#[derive(Debug, Clone)]
pub struct RunRequest {
    pub run_id: String,
    pub task_id: String,
    pub project_id: String,
    pub team_id: String,
    pub seat_id: Option<String>,
    pub seat_name: String,
    pub agent_id: String,
    pub agent_name: String,
    pub adapter: String,
    pub command: String,
    pub args: Vec<String>,
    pub model: String,
    pub project_path: PathBuf,
    pub stage_key: Option<String>,
    pub prompt: String,
    pub resume_session: Option<String>,
    pub budget_usd: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    pub success: bool,
    pub final_text: String,
    pub session_id: Option<String>,
    pub error: Option<String>,
}

pub struct RunControl {
    pub task_id: String,
    pub pid: Option<u32>,
    pub cancelled: Arc<AtomicBool>,
    /// 每次运行独占的 Job Object：取消或结束时终止整棵进程树。
    pub job: Option<Arc<JobHandle>>,
}

#[derive(Default)]
pub struct RunRegistry {
    inner: Mutex<HashMap<String, RunControl>>,
}

impl RunRegistry {
    /// 原子占用并发槽位；达到上限时返回 None，避免“先检查后启动”的竞态。
    pub fn register(
        &self,
        run_id: &str,
        task_id: &str,
        max_concurrent: usize,
    ) -> Option<Arc<AtomicBool>> {
        let mut map = self.inner.lock().ok()?;
        if map.len() >= max_concurrent.max(1) {
            return None;
        }
        let flag = Arc::new(AtomicBool::new(false));
        map.insert(
            run_id.to_string(),
            RunControl {
                task_id: task_id.to_string(),
                pid: None,
                cancelled: flag.clone(),
                job: None,
            },
        );
        Some(flag)
    }

    /// 按任务批量取消：暂停时需要立刻终止该任务当前阶段的所有子进程。
    pub fn cancel_task(&self, task_id: &str) -> Vec<String> {
        let ids: Vec<String> = match self.inner.lock() {
            Ok(map) => map
                .iter()
                .filter(|(_, control)| control.task_id == task_id)
                .map(|(id, _)| id.clone())
                .collect(),
            Err(_) => Vec::new(),
        };
        for id in &ids {
            self.cancel(id);
        }
        ids
    }

    pub fn set_pid(&self, run_id: &str, pid: u32) {
        if let Ok(mut map) = self.inner.lock() {
            if let Some(control) = map.get_mut(run_id) {
                control.pid = Some(pid);
            }
        }
    }

    pub fn set_job(&self, run_id: &str, job: Arc<JobHandle>) {
        if let Ok(mut map) = self.inner.lock() {
            if let Some(control) = map.get_mut(run_id) {
                control.job = Some(job);
            }
        }
    }

    pub fn cancel(&self, run_id: &str) {
        let (pid, job) = match self.inner.lock() {
            Ok(map) => match map.get(run_id) {
                Some(c) => {
                    c.cancelled.store(true, Ordering::SeqCst);
                    (c.pid, c.job.clone())
                }
                None => (None, None),
            },
            Err(_) => (None, None),
        };
        // 先终止作业对象（覆盖 cmd 拉起的整棵子树），再用 taskkill 兜底。
        if let Some(job) = job {
            job.terminate();
        }
        if let Some(pid) = pid {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .creation_flags(0x08000000)
                .output();
        }
    }

    /// 应用退出时终止所有仍在运行的进程树，避免残留 Agent 继续改文件与计费。
    pub fn cancel_all(&self) {
        let ids: Vec<String> = self
            .inner
            .lock()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default();
        for id in ids {
            self.cancel(&id);
        }
    }

    pub fn finish(&self, run_id: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.remove(run_id);
        }
    }

    pub fn active_count(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
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

fn hash_line(line: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(line.as_bytes());
    format!("{:x}", hasher.finalize())[..16].to_string()
}

fn text_of_content(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    let mut parts = Vec::new();
    if let Some(arr) = content.as_array() {
        for block in arr {
            if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    parts.push(text.to_string());
                }
            }
        }
    }
    parts.join("\n")
}

#[allow(clippy::too_many_arguments)]
pub async fn execute(
    app: &AppHandle,
    db: &Db,
    base_dir: &Path,
    settings: &AppSettings,
    gate: &Arc<GateState>,
    registry: &Arc<RunRegistry>,
    proxy: &Arc<ProxyState>,
    req: RunRequest,
) -> Result<RunOutcome> {
    let Some(cancelled) = registry.register(
        &req.run_id,
        &req.task_id,
        settings.max_concurrent_runs.max(1) as usize,
    ) else {
        return Err(AppError::Msg(
            "并发运行已达上限，请等待当前任务结束或调整设置".into(),
        ));
    };
    let store = RunStore::new(db.clone());
    // 守卫先于任何可能失败的 IO 建立，保证早退路径也释放槽位与预算。
    let mut guard = RunGuard {
        gate: gate.clone(),
        registry: registry.clone(),
        proxy: proxy.clone(),
        store: store.clone(),
        run_id: req.run_id.clone(),
        agent_id: req.agent_id.clone(),
        finalized: false,
    };
    let events: Arc<dyn RunEvents> = Arc::new(TauriEvents::new(app.clone()));
    let tpath = transcript_path(base_dir, &req.run_id);
    let run_dir = base_dir.join("sessions").join(&req.run_id);
    std::fs::create_dir_all(&run_dir)?;
    let _run_dir_guard = TempRunDir(run_dir.clone());

    store.record_start(&req)?;
    events.emit(
        "run.status",
        json!({ "runId": req.run_id, "status": "running", "taskId": req.task_id }),
    );

    // 一次性 token 只存在于内存，通过环境变量交给 hook 进程；不落盘、不进命令行。
    let gate_token = gate.register(GateRunCtx {
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
        let conn = db.open()?;
        let extra = crate::extensions::extra_hooks(&conn)?;
        merge_hooks(&mut hooks, &extra);
    }
    let settings_json = json!({ "hooks": hooks });
    std::fs::write(&settings_path, settings_json.to_string())?;

    let provider = {
        let conn = db.open()?;
        crate::providers::resolved(&conn, &req.agent_id)?
    };
    let use_proxy = settings.proxy_enabled && provider.is_some();
    // 关闭代理时直接注入供应商环境；真实 Key 因此会进入子进程，不能启用文件级沙箱。
    let direct_provider_env = if !settings.proxy_enabled && provider.is_some() {
        let conn = db.open()?;
        Some(crate::providers::env_for_agent(&conn, &req.agent_id)?)
    } else {
        None
    };

    let mut mcp_config_path: Option<PathBuf> = None;
    let mut plugin_dirs: Vec<PathBuf> = Vec::new();
    if is_claude {
        let conn = db.open()?;
        mcp_config_path = crate::extensions::mcp_config(&conn, &run_dir)?;
        plugin_dirs = crate::extensions::plugin_dirs(&conn, base_dir)?;
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
        let token = proxy.register(ProxyRun {
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
            notify: settings.notify,
        });
        proxy_token = Some(token);
    }

    // 沙箱准备：仅 Windows + 设置开启 + 代理模式时启用。
    let mut sandbox: Option<SandboxSession> = None;
    let mut isolation = "process";
    let mut isolation_reason: Option<String> = None;
    #[cfg(windows)]
    if settings.sandbox_enabled {
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
            log_sandbox_fallback(events.as_ref(), &store, &req, reason, warning);
            isolation_reason = Some(reason.to_string());
        } else {
            match prepare_sandbox(
                &req,
                base_dir,
                &run_dir,
                &program,
                &proc_args,
                &plugin_dirs,
                is_claude,
                gate.port,
                proxy.port,
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
                        events.as_ref(),
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
            .unwrap_or(proxy.port);
        #[cfg(not(windows))]
        let port = proxy.port;
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
            .unwrap_or(gate.port);
        #[cfg(not(windows))]
        let gate_port = gate.port;
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
        let home = base_dir.join("sandbox-home");
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
        if let Err(err) = std::fs::write(&spec_path, serde_json::to_vec(&spec).unwrap_or_default())
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
        events.emit(
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
        registry.set_pid(&req.run_id, pid);
        // P0-3：加入 Job Object，退出/取消时由内核终止整棵进程树。
        if let Some(job) = crate::sandbox::create_job() {
            if !job.assign(pid) {
                tracing::warn!("assign process {pid} to job object failed");
            }
            registry.set_job(&req.run_id, Arc::new(job));
        }
    }
    let _watchdog = WatchdogGuard(spawn_watchdog(
        app.clone(),
        events.clone(),
        db.clone(),
        registry.clone(),
        req.clone(),
        settings.clone(),
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

    let mut outcome = RunOutcome::default();
    let mut recorded_any = false;
    let mut budget_blocked = false;
    let mut generic_output = String::new();
    // 记录流里出现过的受管工具调用，运行结束时与网关日志比对，识别 hook 绕过。
    let mut gated_calls: Vec<(String, String)> = Vec::new();
    let mut est_input_recorded = false;
    let mut est_output_recorded: i64 = 0;
    let mut est_bytes: usize = 0;

    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            append_line(&tpath, &line);
            if line.trim().is_empty() {
                continue;
            }
            if is_claude {
                let parsed: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let kind = parsed
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                match kind {
                    "system" => {
                        if let Some(sid) = parsed.get("session_id").and_then(|v| v.as_str()) {
                            outcome.session_id = Some(sid.to_string());
                            store.set_session(&req.run_id, sid);
                        }
                    }
                    "assistant" => {
                        let Some(message) = parsed.get("message") else {
                            continue;
                        };
                        let model = message
                            .get("model")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&req.model)
                            .to_string();
                        let content = message.get("content").cloned().unwrap_or(Value::Null);
                        let text = text_of_content(&content);
                        if !text.trim().is_empty() {
                            if let Ok(msg_id) = store.insert_message(MessageRecord {
                                task_id: &req.task_id,
                                run_id: &req.run_id,
                                seat_id: req.seat_id.as_deref(),
                                agent_id: &req.agent_id,
                                kind: "chat",
                                role: Some("assistant"),
                                content: &text,
                                meta: json!({ "model": model, "sessionId": outcome.session_id }),
                            }) {
                                store.record_event(
                                    &req,
                                    "agent.message",
                                    Some(&req.agent_name),
                                    json!({ "messageId": msg_id, "chars": text.chars().count() }),
                                );
                                events.emit(
                                    "run.delta",
                                    json!({
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
                                            "sessionId": outcome.session_id,
                                            "createdAt": now_ms()
                                        }
                                    }),
                                );
                            }
                        }
                        if let Some(blocks) = content.as_array() {
                            for block in blocks {
                                if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                                    let tool_name = block
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("tool");
                                    let tool_id = block
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or_default();
                                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                                    if gate_tool_matched(tool_name) {
                                        gated_calls.push((
                                            tool_name.to_string(),
                                            crate::approval::input_hash(&input),
                                        ));
                                    }
                                    let call_id = format!("{}:{}", req.run_id, tool_id);
                                    store.insert_tool_call(
                                        &call_id,
                                        &req.run_id,
                                        tool_name,
                                        &input,
                                    );
                                    events.emit(
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
                            }
                        }
                        if !use_proxy {
                            if let Some(usage) = message.get("usage") {
                                let input_tokens = usage
                                    .get("input_tokens")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0);
                                let output_tokens = usage
                                    .get("output_tokens")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0);
                                let cache_read = usage
                                    .get("cache_read_input_tokens")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0);
                                let cache_write = usage
                                    .get("cache_creation_input_tokens")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0);
                                let message_id = message
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| hash_line(&line));
                                if let Ok(conn) = db.open() {
                                    let input = UsageInput {
                                        run_id: Some(req.run_id.clone()),
                                        task_id: Some(req.task_id.clone()),
                                        project_id: Some(req.project_id.clone()),
                                        team_id: Some(req.team_id.clone()),
                                        seat_id: req.seat_id.clone(),
                                        agent_id: Some(req.agent_id.clone()),
                                        provider: crate::usage::provider_for(&model),
                                        model: model.clone(),
                                        input_tokens,
                                        output_tokens,
                                        cache_read_tokens: cache_read,
                                        cache_write_tokens: cache_write,
                                        source: "stream".into(),
                                        dedupe_key: format!("claude:{}:{}", req.run_id, message_id),
                                    };
                                    if let Some(cost) = store.record_usage_on(&conn, &input) {
                                        recorded_any = true;
                                        events
                                            .emit("usage.updated", json!({ "runId": req.run_id }));
                                        budget_blocked |= handle_alerts(
                                            app,
                                            &conn,
                                            &req,
                                            registry,
                                            settings.notify,
                                        );
                                        events.emit(
                                            "run.delta",
                                            json!({
                                                "runId": req.run_id,
                                                "kind": "usage",
                                                "usage": {
                                                    "inputTokens": input_tokens,
                                                    "outputTokens": output_tokens,
                                                    "cacheReadTokens": cache_read,
                                                    "cacheWriteTokens": cache_write,
                                                    "costMicros": cost
                                                }
                                            }),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    "user" => {
                        if let Some(message) = parsed.get("message") {
                            if let Some(blocks) = message.get("content").and_then(|v| v.as_array())
                            {
                                for block in blocks {
                                    if block.get("type").and_then(|v| v.as_str())
                                        == Some("tool_result")
                                    {
                                        let tool_id = block
                                            .get("tool_use_id")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or_default();
                                        let call_id = format!("{}:{}", req.run_id, tool_id);
                                        let output =
                                            block.get("content").cloned().unwrap_or(Value::Null);
                                        store.complete_tool_call(&call_id, &output);
                                        events.emit(
                                            "run.delta",
                                            json!({ "runId": req.run_id, "kind": "tool_result", "toolCallId": call_id }),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    "result" => {
                        if let Some(result) = parsed.get("result").and_then(|v| v.as_str()) {
                            if !result.trim().is_empty() {
                                outcome.final_text = result.to_string();
                            }
                        }
                        if let Some(sid) = parsed.get("session_id").and_then(|v| v.as_str()) {
                            outcome.session_id = Some(sid.to_string());
                        }
                        let is_error = parsed
                            .get("is_error")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if is_error {
                            outcome.error = parsed
                                .get("result")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .or_else(|| Some("Agent 返回错误".into()));
                        }
                        if !recorded_any && !use_proxy {
                            if let Some(usage) = parsed.get("usage") {
                                let input_tokens = usage
                                    .get("input_tokens")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0);
                                let output_tokens = usage
                                    .get("output_tokens")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0);
                                let cache_read = usage
                                    .get("cache_read_input_tokens")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0);
                                let cache_write = usage
                                    .get("cache_creation_input_tokens")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0);
                                if let Ok(conn) = db.open() {
                                    let model = parsed
                                        .get("modelUsage")
                                        .and_then(|v| v.as_object())
                                        .and_then(|m| m.keys().next().cloned())
                                        .unwrap_or_else(|| req.model.clone());
                                    let input = UsageInput {
                                        run_id: Some(req.run_id.clone()),
                                        task_id: Some(req.task_id.clone()),
                                        project_id: Some(req.project_id.clone()),
                                        team_id: Some(req.team_id.clone()),
                                        seat_id: req.seat_id.clone(),
                                        agent_id: Some(req.agent_id.clone()),
                                        provider: crate::usage::provider_for(&model),
                                        model,
                                        input_tokens,
                                        output_tokens,
                                        cache_read_tokens: cache_read,
                                        cache_write_tokens: cache_write,
                                        source: "stream".into(),
                                        dedupe_key: format!("claude:{}:result", req.run_id),
                                    };
                                    let _ = store.record_usage_on(&conn, &input);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                generic_output.push_str(&line);
                generic_output.push('\n');
                events.emit(
                    "run.delta",
                    json!({ "runId": req.run_id, "kind": "raw", "line": line }),
                );
                // 非代理路径每约 4KB 增量落一次估算用量，让看门狗与预算看板及时可见。
                if !use_proxy {
                    est_bytes += line.len() + 1;
                    while est_bytes as i64 - est_output_recorded * 4 >= 4096 {
                        let Ok(conn) = db.open() else { break };
                        if !est_input_recorded {
                            let input_tokens =
                                ((req.prompt.chars().count() as f64) / 4.0).ceil().max(1.0) as i64;
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
                                output_tokens: 0,
                                cache_read_tokens: 0,
                                cache_write_tokens: 0,
                                source: "estimate".into(),
                                dedupe_key: format!("estimate:{}:input", req.run_id),
                            };
                            let _ = store.record_usage_on(&conn, &input);
                            est_input_recorded = true;
                        }
                        let input = UsageInput {
                            run_id: Some(req.run_id.clone()),
                            task_id: Some(req.task_id.clone()),
                            project_id: Some(req.project_id.clone()),
                            team_id: Some(req.team_id.clone()),
                            seat_id: req.seat_id.clone(),
                            agent_id: Some(req.agent_id.clone()),
                            provider: crate::usage::provider_for(&req.model),
                            model: req.model.clone(),
                            input_tokens: 0,
                            output_tokens: 1024,
                            cache_read_tokens: 0,
                            cache_write_tokens: 0,
                            source: "estimate".into(),
                            dedupe_key: format!(
                                "estimate:{}:out:{}",
                                req.run_id, est_output_recorded
                            ),
                        };
                        let _ = store.record_usage_on(&conn, &input);
                        est_output_recorded += 1024;
                        events.emit("usage.updated", json!({ "runId": req.run_id }));
                    }
                }
            }
        }
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
                    if outcome.error.is_none() {
                        outcome.error = Some(format!("沙箱宿主错误：{err}"));
                    }
                }
            }
        }
    }

    if !is_claude && !generic_output.trim().is_empty() {
        if store
            .insert_message(MessageRecord {
                task_id: &req.task_id,
                run_id: &req.run_id,
                seat_id: req.seat_id.as_deref(),
                agent_id: &req.agent_id,
                kind: "chat",
                role: Some("assistant"),
                content: generic_output.trim(),
                meta: json!({ "source": "stdout" }),
            })
            .is_ok()
        {
            events.emit(
                "run.delta",
                json!({
                    "runId": req.run_id,
                    "kind": "message",
                    "message": {
                        "taskId": req.task_id,
                        "runId": req.run_id,
                        "seatId": req.seat_id,
                        "agentId": req.agent_id,
                        "kind": "chat",
                        "role": "assistant",
                        "content": generic_output.trim(),
                        "createdAt": now_ms()
                    }
                }),
            );
        }
        if outcome.final_text.is_empty() {
            outcome.final_text = generic_output.trim().to_string();
        }
        // 非 Claude 适配器没有结构化 usage，按字符数增量估算并标记来源，避免重复计费。
        if !use_proxy {
            if let Ok(conn) = db.open() {
                let input_tokens =
                    ((req.prompt.chars().count() as f64) / 4.0).ceil().max(1.0) as i64;
                if !est_input_recorded {
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
                        output_tokens: 0,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                        source: "estimate".into(),
                        dedupe_key: format!("estimate:{}:input", req.run_id),
                    };
                    let _ = store.record_usage_on(&conn, &input);
                }
                let total_output = ((generic_output.chars().count() as f64) / 4.0)
                    .ceil()
                    .max(1.0) as i64;
                let delta = (total_output - est_output_recorded).max(0);
                if delta > 0 {
                    let input = UsageInput {
                        run_id: Some(req.run_id.clone()),
                        task_id: Some(req.task_id.clone()),
                        project_id: Some(req.project_id.clone()),
                        team_id: Some(req.team_id.clone()),
                        seat_id: req.seat_id.clone(),
                        agent_id: Some(req.agent_id.clone()),
                        provider: crate::usage::provider_for(&req.model),
                        model: req.model.clone(),
                        input_tokens: 0,
                        output_tokens: delta,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                        source: "estimate".into(),
                        dedupe_key: format!("estimate:{}:out:final", req.run_id),
                    };
                    if store.record_usage_on(&conn, &input).is_some() {
                        events.emit("usage.updated", json!({ "runId": req.run_id }));
                        budget_blocked |=
                            handle_alerts(app, &conn, &req, registry, settings.notify);
                    }
                }
            }
        }
    }

    // 代理已开启但没有捕获到任何模型流量（适配器不识别注入变量等）：回退估算，避免完全漏计。
    if use_proxy && !proxy.usage_recorded(&req.run_id) {
        let text = if generic_output.trim().is_empty() {
            outcome.final_text.clone()
        } else {
            generic_output.clone()
        };
        if let Ok(conn) = db.open() {
            let input_tokens = ((req.prompt.chars().count() as f64) / 4.0).ceil().max(1.0) as i64;
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
                events.emit("usage.updated", json!({ "runId": req.run_id }));
                budget_blocked |= handle_alerts(app, &conn, &req, registry, settings.notify);
            }
        }
    }

    // 预算终检：代理侧可能在本次运行中已触发 402 硬停，即便进程正常退出也要暂停。
    if let Ok(conn) = db.open() {
        let scope = crate::budget::BudgetScope {
            project_id: &req.project_id,
            team_id: &req.team_id,
            agent_id: &req.agent_id,
            task_id: &req.task_id,
            run_id: &req.run_id,
        };
        budget_blocked |=
            proxy.was_blocked(&req.run_id) || crate::budget::spent_exhausted(&conn, &scope);
    }

    // P1：hook 绕过检测——流里出现过的受管工具，必须都能在网关侧找到同输入记录。
    let unseen = gate.unseen_tools(&req.run_id, &gated_calls);
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
        events.emit(
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
        budget_blocked,
        hook_bypass,
        hook_strict: settings.hook_strict,
        cancelled: cancelled_now,
        exit_code,
        reported_error: outcome.error.as_deref(),
    });
    let run_status = run_status.as_db_str();
    outcome.success = run_status == "completed";
    if outcome.error.is_none() {
        outcome.error = error.clone();
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
        session_id: outcome.session_id.as_deref(),
        isolation,
        isolation_reason: isolation_reason.as_deref(),
    }) {
        events.emit(
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

    Ok(outcome)
}

/// 判断程序名是否为 cmd（含 cmd.exe）。
fn is_cmd_program(program: &str) -> bool {
    let name = Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(program);
    name.eq_ignore_ascii_case("cmd") || name.eq_ignore_ascii_case("cmd.exe")
}

/// OpenCode 只接受 `provider/model` 形式的显式模型参数；没有真实模型时宁可不传，
/// 也不要把它内置目录里的默认模型当作供应商模型。
fn opencode_model(provider_model: Option<&str>, request_model: &str) -> Option<String> {
    let model = provider_model.unwrap_or(request_model).trim();
    if model.is_empty()
        || model.eq_ignore_ascii_case("claude")
        || model.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, '&' | '|' | '<' | '>' | '^' | '"' | '%')
        })
    {
        return None;
    }
    Some(model.to_string())
}

/// 代理模式下为 OpenCode 使用 Lumen 临时 provider；直连模式沿用内置 provider。
fn opencode_model_ref(use_proxy: bool, protocol: &str, model: &str) -> Option<String> {
    let provider_id = match protocol {
        "openai" if use_proxy => "lumen",
        "openai" => "openai",
        "anthropic" => "anthropic",
        _ => return None,
    };
    Some(format!("{provider_id}/{model}"))
}

/// OpenAI 兼容端点通常要求 `/v1` 作为 base URL 的一部分。
fn with_v1(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1") {
        base.to_string()
    } else {
        format!("{base}/v1")
    }
}

/// 生成仅存在于本次运行环境中的 OpenCode 配置。密钥使用 `{env:...}` 占位，
/// 真实 Key 或一次性代理 token 不会写进配置文本或磁盘。
fn opencode_config(use_proxy: bool, protocol: &str, base: &str, model: &str) -> Value {
    let mut models = serde_json::Map::new();
    models.insert(
        model.to_string(),
        json!({
            "name": model,
            "limit": { "context": 200000, "output": 32000 }
        }),
    );
    let models = Value::Object(models);
    match (protocol, use_proxy) {
        ("openai", true) => json!({
            "provider": {
                "lumen": {
                    "npm": "@ai-sdk/openai-compatible",
                    "name": "Lumen",
                    "options": {
                        "baseURL": base,
                        "apiKey": "{env:OPENAI_API_KEY}"
                    },
                    "models": models
                }
            }
        }),
        ("openai", false) => json!({
            "provider": {
                "openai": { "models": models }
            }
        }),
        ("anthropic", true) => json!({
            "provider": {
                "anthropic": {
                    "options": {
                        "baseURL": base,
                        "apiKey": "{env:ANTHROPIC_API_KEY}"
                    },
                    "models": models
                }
            }
        }),
        ("anthropic", false) => json!({
            "provider": {
                "anthropic": { "models": models }
            }
        }),
        _ => json!({}),
    }
}

/// 沙箱启动所需的解析结果。
#[cfg(windows)]
struct SandboxRun {
    session: SandboxSession,
    program: PathBuf,
    args: Vec<String>,
    via_cmd: bool,
}

/// 沙箱无法准备时仍应继续运行，但必须留下可查询的原因。
#[cfg(windows)]
struct SandboxFallback {
    code: &'static str,
    message: String,
}

#[cfg(windows)]
impl From<String> for SandboxFallback {
    fn from(message: String) -> Self {
        Self {
            code: "prepare_failed",
            message,
        }
    }
}

/// 从项目目录向上查找 Git 工作区；CLI 的 Git 发现同样会向上遍历。
#[cfg(windows)]
fn has_git_worktree(project_path: &Path) -> bool {
    project_path
        .ancestors()
        .any(|dir| dir.join(".git").exists())
}

/// Windows AppContainer 默认无权访问 NUL，而 Git for Windows 及其 CLI 包装器
/// 依赖该设备。Git 工作区统一回退到进程隔离，避免运行在 Git 发现或首次
/// 工具调用时才以 EPERM 失败。
#[cfg(windows)]
fn git_needs_process_isolation(project_path: &Path) -> bool {
    has_git_worktree(project_path)
}

/// Git 工作区无法同时启用严格 AppContainer，提前返回可审计的回退原因。
#[cfg(windows)]
fn git_sandbox_fallback(project_path: &Path) -> Option<SandboxFallback> {
    git_needs_process_isolation(project_path).then(|| SandboxFallback {
        code: "git_nul",
        message:
            "Windows AppContainer 禁止访问 Git for Windows 依赖的 NUL 设备；Git 工作区统一使用进程隔离"
                .to_string(),
    })
}

/// 记录一次显式回退，避免“设置显示已开启、实际却未隔离”时无迹可查。
#[cfg(windows)]
fn log_sandbox_fallback(
    events: &dyn RunEvents,
    store: &RunStore,
    req: &RunRequest,
    reason: &str,
    warning: &str,
) {
    tracing::warn!("{warning}");
    store.fallback_to_process(req, reason, warning);
    events.emit(
        "run.delta",
        json!({
            "runId": req.run_id,
            "taskId": req.task_id,
            "kind": "warning",
            "warning": warning
        }),
    );
}

/// 为一次运行准备沙箱：解析真实程序路径、收集额外授权路径并建立 relay 通道。
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn prepare_sandbox(
    req: &RunRequest,
    base_dir: &Path,
    run_dir: &Path,
    program: &str,
    args: &[String],
    plugin_dirs: &[PathBuf],
    is_claude: bool,
    gate_port: u16,
    proxy_port: u16,
) -> std::result::Result<SandboxRun, SandboxFallback> {
    // 这里提前回退到进程隔离，避免用户只能看到“阻塞”而不知道原因。
    // 非 Git 项目仍可使用 AppContainer。
    if let Some(fallback) = git_sandbox_fallback(&req.project_path) {
        return Err(fallback);
    }
    let exe = std::env::current_exe().map_err(|e| format!("无法定位主程序：{e}"))?;
    let plan = crate::sandbox::session::plan_launch(program, args)?;
    let mut extra = plan.extra_paths;
    extra.extend(plugin_dirs.iter().cloned());

    let sandbox_home = base_dir.join("sandbox-home");
    std::fs::create_dir_all(&sandbox_home)
        .map_err(|e| format!("创建沙箱 HOME {} 失败：{e}", sandbox_home.display()))?;
    let sandbox_temp = sandbox_home.join("tmp");
    std::fs::create_dir_all(&sandbox_temp)
        .map_err(|e| format!("创建沙箱 TEMP {} 失败：{e}", sandbox_temp.display()))?;
    if is_claude {
        crate::sandbox::session::bootstrap_claude_home(&sandbox_home)?;
    }

    let mut upstreams: Vec<(&str, u16)> = vec![("proxy", proxy_port)];
    if is_claude {
        upstreams.push(("gate", gate_port));
    }
    let session = crate::sandbox::session::SandboxSession::prepare(
        &req.run_id,
        run_dir,
        &req.project_path,
        &sandbox_home,
        &exe,
        &extra,
        &upstreams,
    )?;
    Ok(SandboxRun {
        session,
        via_cmd: is_cmd_program(&plan.program.to_string_lossy()),
        program: plan.program,
        args: plan.args,
    })
}

/// 解析启动方式：真实存在的可执行文件直接 spawn；.cmd/.bat/.ps1 通过 cmd /C；
/// 用户在配置里填写的完整命令行也交给 cmd（该字符串来自用户配置，不含任务文本）。
fn spawn_spec(command: &str, extra_args: &[String]) -> (String, Vec<String>) {
    let path = Path::new(command);
    if path.is_file() {
        let (program, mut args) = crate::detect::command_for(path);
        args.extend(extra_args.iter().cloned());
        (program, args)
    } else if command.chars().any(|c| c.is_whitespace()) {
        let mut args = vec!["/C".to_string(), command.to_string()];
        args.extend(extra_args.iter().cloned());
        ("cmd".to_string(), args)
    } else {
        (command.to_string(), extra_args.to_vec())
    }
}

fn handle_alerts(
    app: &AppHandle,
    conn: &rusqlite::Connection,
    req: &RunRequest,
    registry: &Arc<RunRegistry>,
    notify: bool,
) -> bool {
    let scope = crate::budget::BudgetScope {
        project_id: &req.project_id,
        team_id: &req.team_id,
        agent_id: &req.agent_id,
        task_id: &req.task_id,
        run_id: &req.run_id,
    };
    crate::budget::enforce(app, conn, &scope, registry, notify)
}

/// 受审批网关管理的工具集合，必须与 PreToolUse matcher 保持一致。
fn gate_tool_matched(name: &str) -> bool {
    matches!(
        name,
        "Bash" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "WebFetch"
    ) || name.starts_with("mcp__")
}

/// 预算/时长看门狗：每 2 秒检查一次，避免长时间无输出的 Agent 把预算跑穿。
struct WatchdogGuard(Arc<AtomicBool>);

impl Drop for WatchdogGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn spawn_watchdog(
    app: AppHandle,
    events: Arc<dyn RunEvents>,
    db: Db,
    registry: Arc<RunRegistry>,
    req: RunRequest,
    settings: AppSettings,
) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        let max_secs = settings.run_max_minutes.max(0) as u64 * 60;
        let mut ticker = tokio::time::interval(Duration::from_secs(2));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if flag.load(Ordering::SeqCst) {
                break;
            }
            if max_secs > 0 && started.elapsed().as_secs() >= max_secs {
                if let Ok(conn) = db.open() {
                    let _ = log_event(
                        &conn,
                        Some(&req.project_id),
                        Some(&req.task_id),
                        Some(&req.run_id),
                        "run.timeout",
                        Some("watchdog"),
                        json!({ "maxMinutes": settings.run_max_minutes }),
                    );
                }
                registry.cancel(&req.run_id);
                events.emit(
                    "run.delta",
                    json!({
                        "runId": req.run_id,
                        "kind": "warning",
                        "warning": format!("运行超过 {} 分钟上限，正在终止", settings.run_max_minutes)
                    }),
                );
                break;
            }
            let Ok(conn) = db.open() else { continue };
            let scope = crate::budget::BudgetScope {
                project_id: &req.project_id,
                team_id: &req.team_id,
                agent_id: &req.agent_id,
                task_id: &req.task_id,
                run_id: &req.run_id,
            };
            // 只看已实际发生的用量：在途请求的预留额度不应把运行误杀。
            if crate::budget::spent_exhausted(&conn, &scope) {
                crate::budget::enforce(&app, &conn, &scope, &registry, settings.notify);
                break;
            }
        }
    });
    stop
}

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(test)]
mod sandbox_home_tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn claude_bootstrap_contains_no_credentials() {
        let home = std::env::temp_dir().join("lumen-claude-sandbox-home-test");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();

        crate::sandbox::session::bootstrap_claude_home(&home).unwrap();
        crate::sandbox::session::bootstrap_claude_home(&home).unwrap();

        let config: Value =
            serde_json::from_slice(&std::fs::read(home.join(".claude.json")).unwrap()).unwrap();
        assert_eq!(config["hasCompletedOnboarding"], true);

        let settings: Value = serde_json::from_slice(
            &std::fs::read(home.join(".claude").join("settings.json")).unwrap(),
        )
        .unwrap();
        let env = settings["env"].as_object().unwrap();
        assert_eq!(
            env.get("DISABLE_TELEMETRY")
                .and_then(|value| value.as_str()),
            Some("1")
        );
        assert!(!env
            .keys()
            .any(|key| { key.starts_with("ANTHROPIC_") || key.starts_with("OPENAI_") }));

        let _ = std::fs::remove_dir_all(&home);
    }
}

#[cfg(test)]
mod opencode_tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn git_project_requires_process_isolation() {
        let root = std::env::temp_dir().join("lumen-opencode-git-sandbox-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let nested = root.join("crates").join("core");
        std::fs::create_dir_all(&nested).unwrap();

        assert!(git_needs_process_isolation(&root));
        assert!(git_needs_process_isolation(&nested));
        assert_eq!(
            git_sandbox_fallback(&root).map(|fallback| fallback.code),
            Some("git_nul")
        );

        let plain = std::env::temp_dir().join("lumen-opencode-plain-sandbox-test");
        let _ = std::fs::remove_dir_all(&plain);
        std::fs::create_dir_all(&plain).unwrap();
        assert!(!git_needs_process_isolation(&plain));
        assert!(git_sandbox_fallback(&plain).is_none());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&plain);
    }

    #[cfg(windows)]
    #[test]
    fn opencode_git_file_marker_counts_as_worktree() {
        let root = std::env::temp_dir().join("lumen-opencode-git-file-sandbox-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join(".git"), "gitdir: ../actual/.git/worktrees/sub").unwrap();

        assert!(git_needs_process_isolation(&root.join("sub")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn provider_model_wins_and_placeholder_is_rejected() {
        assert_eq!(
            opencode_model(Some("deepseek-flash"), "claude").as_deref(),
            Some("deepseek-flash")
        );
        assert_eq!(
            opencode_model(None, "deepseek-flash").as_deref(),
            Some("deepseek-flash")
        );
        assert_eq!(opencode_model(None, "claude"), None);
        assert_eq!(opencode_model(Some("bad&model"), "claude"), None);
    }

    #[test]
    fn model_reference_matches_transport() {
        assert_eq!(
            opencode_model_ref(true, "openai", "deepseek-flash").as_deref(),
            Some("lumen/deepseek-flash")
        );
        assert_eq!(
            opencode_model_ref(false, "openai", "deepseek-flash").as_deref(),
            Some("openai/deepseek-flash")
        );
        assert_eq!(
            opencode_model_ref(true, "anthropic", "claude-sonnet").as_deref(),
            Some("anthropic/claude-sonnet")
        );
    }

    #[test]
    fn base_url_gets_v1_once() {
        assert_eq!(
            with_v1("https://api.deepseek.com"),
            "https://api.deepseek.com/v1"
        );
        assert_eq!(
            with_v1("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1"
        );
    }

    #[test]
    fn proxy_config_registers_dynamic_model_without_secret() {
        let config = opencode_config(
            true,
            "openai",
            "http://127.0.0.1:1234/proxy/token/v1",
            "deepseek-flash",
        );
        assert_eq!(
            config["provider"]["lumen"]["models"]["deepseek-flash"]["name"],
            "deepseek-flash"
        );
        assert_eq!(
            config["provider"]["lumen"]["options"]["apiKey"],
            "{env:OPENAI_API_KEY}"
        );
        assert_eq!(
            config["provider"]["lumen"]["options"]["baseURL"],
            "http://127.0.0.1:1234/proxy/token/v1"
        );
        assert!(!config.to_string().contains("sk-"));
    }

    #[test]
    fn direct_config_uses_builtin_provider() {
        let config = opencode_config(
            false,
            "openai",
            "https://api.deepseek.com/v1",
            "deepseek-flash",
        );
        assert!(config["provider"]["lumen"].is_null());
        assert_eq!(
            config["provider"]["openai"]["models"]["deepseek-flash"]["name"],
            "deepseek-flash"
        );
    }
}
