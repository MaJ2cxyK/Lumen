use crate::db::{log_event, Db};
use crate::models::AppSettings;
use crate::sandbox::JobHandle;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::AppHandle;

mod events;
mod launch;
mod policy;
mod session;
mod store;
mod stream;

use events::RunEvents;
use launch::is_cmd_program;
pub use session::{RunDeps, RunSession};
use store::RunStore;

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

fn hash_line(line: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(line.as_bytes());
    format!("{:x}", hasher.finalize())[..16].to_string()
}

/// 生产入口：组装依赖后交给 `RunSession`，本身不包含执行逻辑。
#[allow(clippy::too_many_arguments)]
/// 判断程序名是否为 cmd（含 cmd.exe）。
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
fn handle_alerts(
    app: Option<&AppHandle>,
    conn: &rusqlite::Connection,
    req: &RunRequest,
    registry: &Arc<RunRegistry>,
    notify: bool,
) -> bool {
    let Some(app) = app else {
        return false;
    };
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
    app: Option<AppHandle>,
    events: Arc<dyn RunEvents>,
    db: Db,
    registry: Arc<RunRegistry>,
    req: RunRequest,
    settings: AppSettings,
) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let Some(app) = app else {
        return stop;
    };
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
    use serde_json::Value;

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
}

#[cfg(test)]
mod app_optional_tests {
    use super::*;

    fn request() -> RunRequest {
        RunRequest {
            run_id: "r1".into(),
            task_id: "task1".into(),
            project_id: "p1".into(),
            team_id: "t1".into(),
            seat_id: None,
            seat_name: "Seat".into(),
            agent_id: "a1".into(),
            agent_name: "Agent".into(),
            adapter: "generic".into(),
            command: "cmd".into(),
            args: vec![],
            model: "claude".into(),
            project_path: PathBuf::from("C:/tmp"),
            stage_key: None,
            prompt: "hello".into(),
            resume_session: None,
            budget_usd: None,
        }
    }

    #[test]
    fn alerts_are_skipped_without_app_handle() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let registry = Arc::new(RunRegistry::default());
        assert!(!handle_alerts(None, &conn, &request(), &registry, true));
    }

    #[test]
    fn watchdog_without_app_handle_does_not_start() {
        let stop = spawn_watchdog(
            None,
            Arc::new(events::RecordingEvents::new()),
            Db::new(std::env::temp_dir().join("lumen-watchdog-test.db")),
            Arc::new(RunRegistry::default()),
            request(),
            AppSettings::default(),
        );
        assert!(!stop.load(Ordering::SeqCst));
    }
}
