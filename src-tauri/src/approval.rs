use crate::db::{log_event, new_id, now_ms, Db};
use crate::error::Result;
use crate::models::{Approval, ApprovalRule};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use rand::{distributions::Alphanumeric, Rng};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone)]
pub struct GateRunCtx {
    pub run_id: String,
    pub task_id: String,
    pub project_id: String,
    pub project_path: String,
    pub seat_name: String,
    pub agent_name: String,
}

#[derive(Debug, Clone)]
pub struct Decision {
    pub allow: bool,
    pub reason: String,
    pub by: String,
}

/// 审批网关状态。
///
/// 安全约束：token 只存在于内存，并按 run 隔离；不写入任何配置文件，
/// 也不出现在子进程命令行中，只通过环境变量传给 hook 进程。
#[derive(Default)]
pub struct GateState {
    pub port: u16,
    runs: Mutex<HashMap<String, GateRunCtx>>,
    tokens: Mutex<HashMap<String, String>>,
    pending: Mutex<HashMap<String, tokio::sync::oneshot::Sender<Decision>>>,
    /// 每个 run 实际经过网关的（工具名，输入哈希），用于运行结束时的绕过检测。
    records: Mutex<HashMap<String, HashSet<(String, String)>>>,
}

impl GateState {
    /// 注册一次运行并生成一次性 token，返回给 runtime 注入子进程环境变量。
    pub fn register(&self, ctx: GateRunCtx) -> String {
        let token: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(48)
            .map(char::from)
            .collect();
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.insert(ctx.run_id.clone(), token.clone());
        }
        if let Ok(mut runs) = self.runs.lock() {
            runs.insert(ctx.run_id.clone(), ctx);
        }
        token
    }

    pub fn unregister(&self, run_id: &str) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.remove(run_id);
        }
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.remove(run_id);
        }
        if let Ok(mut records) = self.records.lock() {
            records.remove(run_id);
        }
    }

    /// 记录一次真实到达网关的工具调用，供事后绕过检测比对。
    pub fn record_tool(&self, run_id: &str, tool: &str, input_hash: &str) {
        if let Ok(mut records) = self.records.lock() {
            records
                .entry(run_id.to_string())
                .or_default()
                .insert((tool.to_string(), input_hash.to_string()));
        }
    }

    /// 返回流里出现过、但从未经过网关的调用。
    pub fn unseen_tools(&self, run_id: &str, calls: &[(String, String)]) -> Vec<(String, String)> {
        let records = self.records.lock().ok();
        let empty = HashSet::new();
        let seen = records
            .as_ref()
            .and_then(|r| r.get(run_id))
            .unwrap_or(&empty);
        calls
            .iter()
            .filter(|c| !seen.contains(*c))
            .cloned()
            .collect()
    }

    /// 按 run 校验 token，常量时间比较，避免跨 run 复用与计时侧信道。
    pub fn authorize(&self, run_id: &str, candidate: &str) -> bool {
        let expected = self
            .tokens
            .lock()
            .ok()
            .and_then(|tokens| tokens.get(run_id).cloned());
        match expected {
            Some(expected) => constant_time_eq(expected.as_bytes(), candidate.as_bytes()),
            None => false,
        }
    }

    pub fn decide_channel(&self, approval_id: &str, decision: Decision) -> bool {
        let sender = self
            .pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(approval_id));
        if let Some(tx) = sender {
            let _ = tx.send(decision);
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
struct GateCtx {
    gate: Arc<GateState>,
    db: Db,
    app: AppHandle,
    timeout_secs: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolGateRequest {
    run_id: String,
    tool_name: String,
    tool_input: Value,
    cwd: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolGateResponse {
    decision: String,
    reason: String,
    approval_id: Option<String>,
}

pub fn start_server(app: AppHandle, db: Db, timeout_secs: i64) -> Result<Arc<GateState>> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let gate = Arc::new(GateState {
        port,
        runs: Mutex::new(HashMap::new()),
        tokens: Mutex::new(HashMap::new()),
        pending: Mutex::new(HashMap::new()),
        records: Mutex::new(HashMap::new()),
    });
    let ctx = Arc::new(GateCtx {
        gate: gate.clone(),
        db,
        app,
        timeout_secs,
    });
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(err) => {
                tracing::error!("gate listener error: {err}");
                return;
            }
        };
        let router = Router::new()
            .route("/gate/health", get(|| async { "ok" }))
            .route("/gate/tool", post(gate_tool))
            .with_state(ctx);
        if let Err(err) = axum::serve(listener, router).await {
            tracing::error!("gate server stopped: {err}");
        }
    });
    Ok(gate)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

async fn gate_tool(
    State(ctx): State<Arc<GateCtx>>,
    headers: HeaderMap,
    Json(req): Json<ToolGateRequest>,
) -> std::result::Result<Json<ToolGateResponse>, (StatusCode, String)> {
    let candidate = headers
        .get("x-agenthub-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !ctx.gate.authorize(&req.run_id, candidate) {
        return Err((StatusCode::UNAUTHORIZED, "bad token".into()));
    }
    // 先登记再到策略判断：无论最终放行、拒绝还是待审批，都算“经过网关”。
    ctx.gate
        .record_tool(&req.run_id, &req.tool_name, &input_hash(&req.tool_input));
    let run_ctx = {
        let runs = ctx
            .gate
            .runs
            .lock()
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "lock".into()))?;
        runs.get(&req.run_id).cloned()
    };
    let Some(run_ctx) = run_ctx else {
        return Ok(Json(ToolGateResponse {
            decision: "deny".into(),
            reason: "运行上下文不存在".into(),
            approval_id: None,
        }));
    };
    let outcome = {
        let conn = ctx
            .db
            .open()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        evaluate(&conn, &run_ctx, &req.tool_name, &req.tool_input)
    };
    match outcome {
        PolicyOutcome::Allow(reason) => Ok(Json(ToolGateResponse {
            decision: "allow".into(),
            reason,
            approval_id: None,
        })),
        PolicyOutcome::Deny(reason) => Ok(Json(ToolGateResponse {
            decision: "deny".into(),
            reason,
            approval_id: None,
        })),
        PolicyOutcome::Ask { risk, reason } => {
            let approval_id = {
                let conn = ctx
                    .db
                    .open()
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                insert_tool_approval(&conn, &run_ctx, &req, &risk, &reason)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            };
            let (tx, rx) = tokio::sync::oneshot::channel();
            if let Ok(mut pending) = ctx.gate.pending.lock() {
                pending.insert(approval_id.clone(), tx);
            }
            let _ = ctx.app.emit(
                "approval.requested",
                json!({
                    "approvalId": approval_id,
                    "type": "tool_permission",
                    "riskLevel": risk,
                    "title": crate::i18n::tr(
                        &format!("工具授权：{}", req.tool_name),
                        &format!("Tool approval: {}", req.tool_name)
                    ),
                    "taskId": run_ctx.task_id,
                }),
            );
            crate::notify::send_task(
                &ctx.app,
                &crate::i18n::tr("Lumen 审批请求", "Lumen approval request"),
                &crate::i18n::tr(
                    &format!(
                        "{} 请求执行 {}，等待你的决定",
                        run_ctx.agent_name, req.tool_name
                    ),
                    &format!(
                        "{} requested to run {}. Waiting for your decision",
                        run_ctx.agent_name, req.tool_name
                    ),
                ),
                Some(&run_ctx.task_id),
                "approval",
            );
            let timeout = Duration::from_secs(ctx.timeout_secs.max(30) as u64);
            let decision = match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(d)) => d,
                _ => {
                    let mut expired = false;
                    if let Ok(conn) = ctx.db.open() {
                        let changed = conn
                            .execute(
                                "UPDATE approvals SET status = 'expired', decided_at = ?1, decision_note = '等待超时，自动拒绝'
                                 WHERE id = ?2 AND status = 'pending'",
                                params![now_ms(), approval_id],
                            )
                            .unwrap_or(0);
                        expired = changed > 0;
                        let _ = log_event(
                            &conn,
                            Some(&run_ctx.project_id),
                            Some(&run_ctx.task_id),
                            Some(&run_ctx.run_id),
                            "approval.expired",
                            Some("system"),
                            json!({ "approvalId": approval_id }),
                        );
                    }
                    if expired {
                        let _ = ctx.app.emit(
                            "approval.changed",
                            json!({ "approvalId": approval_id, "reason": "timeout" }),
                        );
                    }
                    Decision {
                        allow: false,
                        reason: "审批超时，自动拒绝".into(),
                        by: "timeout".into(),
                    }
                }
            };
            if let Ok(mut pending) = ctx.gate.pending.lock() {
                pending.remove(&approval_id);
            }
            Ok(Json(ToolGateResponse {
                decision: if decision.allow {
                    "allow".into()
                } else {
                    "deny".into()
                },
                reason: decision.reason,
                approval_id: Some(approval_id),
            }))
        }
    }
}

pub enum PolicyOutcome {
    Allow(String),
    Deny(String),
    Ask { risk: String, reason: String },
}

macro_rules! regex_lazy {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            use std::sync::OnceLock;
            static RE: OnceLock<Regex> = OnceLock::new();
            RE.get_or_init(|| Regex::new($pat).unwrap())
        }
    };
}

regex_lazy!(
    danger_re,
    r"(?i)(rm\s+-[a-z]*r|remove-item.*-recurse|\brd\s+/s\b|\bdel\s+/f\b|format\s+[a-z]:|shutdown\b|reg\s+delete|bcdedit|diskpart|git\s+(reset\s+--hard|clean\s+-[a-z]*f|checkout\s+--\s+\.))"
);
regex_lazy!(
    publish_re,
    r"(?i)(git\s+push|npm\s+publish|pnpm\s+publish|yarn\s+publish|docker\s+push|kubectl\s+(apply|delete)|terraform\s+(apply|destroy)|vercel\s+deploy|\baws\s+)"
);
regex_lazy!(
    netexec_re,
    r"(?i)(curl|wget|invoke-webrequest|\biwr\b).*(\||>|\biex\b|\bsh\b)"
);
regex_lazy!(priv_re, r"(?i)(^|\s)(sudo|runas)\s");

/// 可直接放行的可执行文件（含 Windows 常见只读命令）。
const SAFE_HEADS: &[&str] = &[
    "ls",
    "dir",
    "pwd",
    "cd",
    "cat",
    "type",
    "head",
    "tail",
    "echo",
    "where",
    "which",
    "whoami",
    "rg",
    "findstr",
    "get-childitem",
    "get-content",
    "select-string",
    "test-path",
    "resolve-path",
    "git",
    "npm",
    "pnpm",
    "yarn",
    "npx",
    "node",
    "python",
    "python3",
    "pytest",
    "tsc",
    "vite",
    "cargo",
    "rustc",
    "go",
    "dotnet",
    "java",
    "mvn",
    "gradle",
    "make",
];

/// 只在拥有精确命令规则时才允许通过的控制操作符。
fn has_control_ops(command: &str) -> bool {
    command.contains("&&")
        || command.contains("||")
        || command.contains('|')
        || command.contains(';')
        || command.contains('&')
        || command.contains('>')
        || command.contains('<')
        || command.contains('^')
        || command.contains('`')
        || command.contains("$(")
        || command.contains('\n')
        || command.contains('\r')
}

fn normalize_command(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn hard_risk(command: &str) -> Option<(String, String)> {
    let note = |label: &str, label_en: &str| {
        Some((
            "high".to_string(),
            crate::i18n::tr(
                &format!("{label}需要确认：{}", truncate(command, 180)),
                &format!(
                    "{label_en} requires confirmation: {}",
                    truncate(command, 180)
                ),
            ),
        ))
    };
    if danger_re().is_match(command) {
        note("高风险命令", "High-risk command")
    } else if publish_re().is_match(command) {
        note("发布/推送类命令", "Publish/push command")
    } else if netexec_re().is_match(command) {
        note("下载并执行类命令", "Download-and-execute command")
    } else if priv_re().is_match(command) {
        note("提权命令", "Privilege escalation command")
    } else {
        None
    }
}

/// 把命令按 &&、||、;、|、换行切开，逐个检查可执行文件是否在白名单内。
fn command_segments(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            cur.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                cur.push(c);
            }
            '&' | '|' | ';' | '\n' | '\r' => {
                if let Some(next) = chars.peek() {
                    if *next == c {
                        chars.next();
                    }
                }
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn segment_allowed(segment: &str) -> bool {
    let segment = segment.trim();
    if segment.is_empty() {
        return true;
    }
    let head = segment
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|c| c == '"' || c == '\'')
        .to_lowercase();
    let head = head
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(&head)
        .trim_end_matches(".exe")
        .trim_end_matches(".cmd")
        .trim_end_matches(".bat")
        .to_string();
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    // 内联执行代码等同于任意命令，不能因为可执行文件在白名单里就放行。
    if head == "node"
        && tokens
            .iter()
            .skip(1)
            .any(|t| t.starts_with("-e") || t.starts_with("--eval"))
    {
        return false;
    }
    if (head == "python" || head == "python3") && tokens.iter().skip(1).any(|t| t.starts_with("-c"))
    {
        return false;
    }
    SAFE_HEADS.contains(&head.as_str())
}

fn safe_command(command: &str) -> bool {
    if command.trim().is_empty() {
        return false;
    }
    if has_control_ops(command) {
        return false;
    }
    command_segments(command).iter().all(|s| segment_allowed(s))
}

pub fn evaluate(conn: &Connection, ctx: &GateRunCtx, tool: &str, input: &Value) -> PolicyOutcome {
    let command = input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    // 任务级授权：auto 只放大“普通询问”，不越过硬风险、拒绝规则与目录外写入。
    let mandate_auto = conn
        .query_row(
            "SELECT COALESCE(mandate, 'ask') FROM tasks WHERE id = ?1",
            params![ctx.task_id],
            |r| r.get::<_, String>(0),
        )
        .map(|value| value == "auto")
        .unwrap_or(false);

    // 1) 硬性危险检查必须前置于任何规则，避免“记住过一次”把危险命令一起放行。
    if tool == "Bash" {
        if let Some((risk, reason)) = hard_risk(&command) {
            return PolicyOutcome::Ask { risk, reason };
        }
        if has_control_ops(&command) {
            // 组合命令只接受“完全相同命令”的精确规则，其余默认人工确认。
            if let Some(outcome) = exact_rule(conn, tool, input) {
                return outcome;
            }
            // 用户拒绝规则必须先于任务授权生效。
            if let Ok(rules) = rule_rows(conn) {
                for rule in rules {
                    if !rule.enabled || rule.action != "deny" {
                        continue;
                    }
                    if let Some((m, _)) = rule_matches(&rule, tool, input) {
                        return PolicyOutcome::Deny(m);
                    }
                }
            }
            if mandate_auto {
                return PolicyOutcome::Allow("任务授权：自动".into());
            }
            return PolicyOutcome::Ask {
                risk: "medium".into(),
                reason: crate::i18n::tr(
                    &format!(
                        "命令包含组合或重定向操作符，需要确认：{}",
                        truncate(&command, 180)
                    ),
                    &format!(
                        "Command contains chained or redirected operations and needs confirmation: {}",
                        truncate(&command, 180)
                    ),
                ),
            };
        }
    }

    // 2) 用户规则（allow 规则不能是“所有 Bash”这类宽泛匹配）。
    if let Ok(rules) = rule_rows(conn) {
        for rule in rules {
            if !rule.enabled {
                continue;
            }
            if let Some((m, has_constraint)) = rule_matches(&rule, tool, input) {
                if rule.action == "allow" && tool == "Bash" && !has_constraint {
                    continue;
                }
                return match rule.action.as_str() {
                    "allow" => PolicyOutcome::Allow(m),
                    "deny" => PolicyOutcome::Deny(m),
                    _ => {
                        if mandate_auto {
                            PolicyOutcome::Allow("任务授权：自动".into())
                        } else {
                            PolicyOutcome::Ask {
                                risk: "medium".into(),
                                reason: m,
                            }
                        }
                    }
                };
            }
        }
    }

    // 3) 默认策略：未知工具一律询问，不再默认放行。
    match tool {
        "Read" | "Glob" | "Grep" | "LS" | "TodoWrite" | "TodoRead" | "NotebookRead"
        | "BashOutput" | "KillShell" => PolicyOutcome::Allow("只读工具，自动放行".into()),
        "Bash" => {
            if safe_command(&command) {
                PolicyOutcome::Allow("白名单内的常规命令，自动放行".into())
            } else if mandate_auto {
                PolicyOutcome::Allow("任务授权：自动".into())
            } else {
                PolicyOutcome::Ask {
                    risk: "medium".into(),
                    reason: crate::i18n::tr(
                        &format!("命令不在自动放行清单内：{}", truncate(&command, 180)),
                        &format!(
                            "Command is not on the auto-approve list: {}",
                            truncate(&command, 180)
                        ),
                    ),
                }
            }
        }
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let path = input
                .get("file_path")
                .or_else(|| input.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if path.is_empty() || inside(Path::new(path), Path::new(&ctx.project_path)) {
                PolicyOutcome::Allow("项目目录内写入，自动放行".into())
            } else {
                PolicyOutcome::Ask {
                    risk: "medium".into(),
                    reason: crate::i18n::tr(
                        &format!("写入项目目录之外：{path}"),
                        &format!("Write outside the project directory: {path}"),
                    ),
                }
            }
        }
        "WebFetch" | "WebSearch" => {
            if mandate_auto {
                PolicyOutcome::Allow("任务授权：自动".into())
            } else {
                PolicyOutcome::Ask {
                    risk: "medium".into(),
                    reason: crate::i18n::tr(
                        "访问外部网络需要确认",
                        "External network access requires confirmation",
                    ),
                }
            }
        }
        _ => {
            if mandate_auto {
                PolicyOutcome::Allow("任务授权：自动".into())
            } else {
                PolicyOutcome::Ask {
                    risk: "medium".into(),
                    reason: crate::i18n::tr(
                        &format!("未识别的工具「{tool}」，默认请求确认"),
                        &format!(
                            "Unrecognized tool \"{tool}\"; requesting confirmation by default"
                        ),
                    ),
                }
            }
        }
    }
}

/// 只匹配 commandExact 的规则，用于组合命令。
fn exact_rule(conn: &Connection, tool: &str, input: &Value) -> Option<PolicyOutcome> {
    let command = input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let normalized = normalize_command(command);
    if let Ok(rules) = rule_rows(conn) {
        for rule in rules {
            if !rule.enabled {
                continue;
            }
            let Ok(m) = serde_json::from_str::<Value>(&rule.match_json) else {
                continue;
            };
            if m.get("tool").and_then(|v| v.as_str()) != Some(tool) {
                continue;
            }
            let Some(exact) = m.get("commandExact").and_then(|v| v.as_str()) else {
                continue;
            };
            if exact != normalized {
                continue;
            }
            return Some(match rule.action.as_str() {
                "allow" => PolicyOutcome::Allow("命中精确命令规则：allow".into()),
                "deny" => PolicyOutcome::Deny("命中精确命令规则：deny".into()),
                _ => PolicyOutcome::Ask {
                    risk: "medium".into(),
                    reason: "命中精确命令规则：ask".into(),
                },
            });
        }
    }
    None
}

fn lexical_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn inside(child: &Path, parent: &Path) -> bool {
    let p = parent
        .canonicalize()
        .unwrap_or_else(|_| lexical_normalize(parent));
    let c = child.canonicalize().unwrap_or_else(|_| {
        let mut existing = child;
        let mut tail = PathBuf::new();
        while !existing.exists() {
            let Some(name) = existing.file_name() else {
                break;
            };
            tail = Path::new(name).join(tail);
            let Some(parent_dir) = existing.parent() else {
                break;
            };
            existing = parent_dir;
        }
        let base = existing
            .canonicalize()
            .unwrap_or_else(|_| lexical_normalize(existing));
        lexical_normalize(&base.join(tail))
    });
    let strip = |path: &Path| -> String {
        path.to_string_lossy()
            .to_lowercase()
            .trim_start_matches("\\\\?\\")
            .trim_end_matches('\\')
            .to_string()
    };
    let cs = strip(&c);
    let ps = strip(&p);
    !ps.is_empty() && (cs == ps || cs.starts_with(&format!("{ps}\\")))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

/// 工具输入哈希：stream-json 解析侧与网关侧共用，保证两边可以精确比对。
pub fn input_hash(input: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.to_string().as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    digest[..16].to_string()
}

struct RuleRow {
    match_json: String,
    action: String,
    enabled: bool,
}

fn rule_rows(conn: &Connection) -> Result<Vec<RuleRow>> {
    let mut stmt = conn.prepare(
        "SELECT match_json, action, enabled
         FROM approval_rules ORDER BY priority ASC, created_at ASC",
    )?;
    let items = stmt.query_map([], |r| {
        Ok(RuleRow {
            match_json: r.get(0)?,
            action: r.get(1)?,
            enabled: r.get::<_, i64>(2)? != 0,
        })
    })?;
    Ok(items.filter_map(|r| r.ok()).collect())
}

/// 返回（命中原因，是否包含命令/路径约束）。
fn rule_matches(rule: &RuleRow, tool: &str, input: &Value) -> Option<(String, bool)> {
    let m: Value = serde_json::from_str(&rule.match_json).ok()?;
    if let Some(t) = m.get("tool").and_then(|v| v.as_str()) {
        if t != tool {
            return None;
        }
    }
    let command = input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let mut constrained = false;
    if let Some(needle) = m.get("commandContains").and_then(|v| v.as_str()) {
        if !command.contains(needle) {
            return None;
        }
        constrained = true;
    }
    if let Some(exact) = m.get("commandExact").and_then(|v| v.as_str()) {
        if normalize_command(command) != exact {
            return None;
        }
        constrained = true;
    }
    if let Some(pattern) = m.get("commandRegex").and_then(|v| v.as_str()) {
        match Regex::new(pattern) {
            Ok(re) if re.is_match(command) => {}
            _ => return None,
        }
        constrained = true;
    }
    if let Some(glob) = m.get("pathGlob").and_then(|v| v.as_str()) {
        let path = input
            .get("file_path")
            .or_else(|| input.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !glob_simple_match(glob, path) {
            return None;
        }
        constrained = true;
    }
    Some((format!("命中规则：{}", rule.action), constrained))
}

fn glob_simple_match(pattern: &str, text: &str) -> bool {
    let p = pattern.replace("**", "\u{0}").replace(['*', '\u{0}'], ".*");
    Regex::new(&format!("(?i)^{p}$"))
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

fn insert_tool_approval(
    conn: &Connection,
    run_ctx: &GateRunCtx,
    req: &ToolGateRequest,
    risk: &str,
    reason: &str,
) -> Result<String> {
    let id = new_id();
    let task_id = if run_ctx.task_id.is_empty() {
        None
    } else {
        Some(run_ctx.task_id.clone())
    };
    conn.execute(
        "INSERT INTO approvals (id, task_id, run_id, type, risk_level, title, detail_json, status, requested_by, created_at, expires_at)
         VALUES (?1, ?2, ?3, 'tool_permission', ?4, ?5, ?6, 'pending', ?7, ?8, ?9)",
        params![
            id,
            task_id,
            run_ctx.run_id,
            risk,
            crate::i18n::tr(
                &format!("工具授权：{}", req.tool_name),
                &format!("Tool approval: {}", req.tool_name),
            ),
            json!({
                "toolName": req.tool_name,
                "toolInput": req.tool_input,
                "cwd": req.cwd.clone().unwrap_or_else(|| run_ctx.project_path.clone()),
                "reason": reason,
                "seatName": run_ctx.seat_name,
                "agentName": run_ctx.agent_name
            })
            .to_string(),
            run_ctx.agent_name,
            now_ms(),
            now_ms() + 30 * 60 * 1000
        ],
    )?;
    log_event(
        conn,
        Some(&run_ctx.project_id),
        Some(&run_ctx.task_id),
        Some(&run_ctx.run_id),
        "approval.requested",
        Some("gate"),
        json!({ "approvalId": id, "toolName": req.tool_name, "risk": risk }),
    )?;
    Ok(id)
}

/// 将某个任务下仍处于 pending 的审批统一置为过期，返回影响行数。
pub fn expire_pending_for_task(conn: &Connection, task_id: &str, note: &str) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE approvals SET status = 'expired', decided_at = ?1, decision_note = ?2
         WHERE task_id = ?3 AND status = 'pending'",
        params![now_ms(), note, task_id],
    )?)
}

/// 原子写入审批决定：只有审批仍 pending、所属任务仍有效、阶段门禁阶段匹配时才生效。
fn decide_if_active(
    conn: &Connection,
    approval_id: &str,
    new_status: &str,
    note: &str,
) -> Result<usize> {
    Ok(conn.execute(
        &format!(
            "UPDATE approvals SET status = ?1, decided_by = 'user', decision_note = ?2, decided_at = ?3
         WHERE id = ?4 AND status = 'pending'
           AND (task_id IS NULL OR EXISTS (
             SELECT 1 FROM tasks t
             WHERE t.id = approvals.task_id
               AND t.status IN ({})
               AND (approvals.type <> 'stage_gate'
                    OR (t.status = 'waiting_approval'
                        AND json_extract(approvals.detail_json, '$.stageIndex') = t.stage_index))
           ))",
            crate::task_lifecycle::ACTIVE_STATUSES_SQL
        ),
        params![new_status, note, now_ms(), approval_id],
    )?)
}

pub fn decide(
    app: &AppHandle,
    gate: &GateState,
    db: &Db,
    approval_id: &str,
    approve: bool,
    note: &str,
    remember: bool,
) -> Result<()> {
    let conn = db.open()?;
    let (status, detail_json, r#type, task_id, run_id, expires_at): (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<i64>,
    ) = conn.query_row(
        "SELECT status, detail_json, type, task_id, run_id, expires_at FROM approvals WHERE id = ?1",
        params![approval_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
    )?;
    if status != "pending" {
        return Err("该审批已处理".into());
    }
    if let Some(expires_at) = expires_at {
        if expires_at <= now_ms() {
            let tx = conn.unchecked_transaction()?;
            let changed = tx.execute(
                "UPDATE approvals SET status = 'expired', decided_at = ?1, decision_note = '审批已过期'
                 WHERE id = ?2 AND status = 'pending'",
                params![now_ms(), approval_id],
            )?;
            let _ = log_event(
                &tx,
                None,
                task_id.as_deref(),
                run_id.as_deref(),
                "approval.expired",
                Some("system"),
                json!({ "approvalId": approval_id }),
            );
            tx.commit()?;
            if changed > 0 {
                let _ = app.emit(
                    "approval.changed",
                    json!({ "approvalId": approval_id, "reason": "expired" }),
                );
            }
            return Err("该审批已过期，请重新发起".into());
        }
    }
    let new_status = if approve { "approved" } else { "denied" };

    let tx = conn.unchecked_transaction()?;
    let changed = decide_if_active(&tx, approval_id, new_status, note)?;
    if changed == 0 {
        let current: Option<String> = tx
            .query_row(
                "SELECT status FROM approvals WHERE id = ?1",
                params![approval_id],
                |r| r.get(0),
            )
            .optional()?;
        if current.as_deref() == Some("pending") {
            // 审批仍 pending 却被条件更新拒绝：任务已结束或阶段已变化，不能复活任务。
            tx.execute(
                "UPDATE approvals SET status = 'expired', decided_at = ?1, decision_note = '任务已结束或阶段已变化'
                 WHERE id = ?2 AND status = 'pending'",
                params![now_ms(), approval_id],
            )?;
            let _ = log_event(
                &tx,
                None,
                task_id.as_deref(),
                run_id.as_deref(),
                "approval.expired",
                Some("system"),
                json!({ "approvalId": approval_id, "reason": "inactive" }),
            );
            tx.commit()?;
            let _ = app.emit(
                "approval.changed",
                json!({ "approvalId": approval_id, "reason": "inactive" }),
            );
            return Err("任务已结束或阶段已变化，该审批已失效".into());
        }
        return Err("该审批已处理".into());
    }
    let detail: Value = serde_json::from_str(&detail_json).unwrap_or(Value::Null);
    if remember && r#type == "tool_permission" {
        if let Some(tool) = detail.get("toolName").and_then(|v| v.as_str()) {
            let mut matcher = json!({ "tool": tool });
            if tool == "Bash" {
                let command = detail
                    .get("toolInput")
                    .and_then(|v| v.get("command"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if !command.trim().is_empty() {
                    // 精确命令匹配，杜绝“前两个词”模糊规则被组合命令绕过。
                    matcher["commandExact"] = json!(normalize_command(command));
                }
            } else if let Some(path) = detail
                .get("toolInput")
                .and_then(|v| v.get("file_path").or_else(|| v.get("path")))
                .and_then(|v| v.as_str())
            {
                if !path.trim().is_empty() {
                    matcher["pathGlob"] = json!(path);
                }
            }
            let usable = matcher.get("commandExact").is_some()
                || matcher.get("pathGlob").is_some()
                || tool != "Bash";
            if usable {
                tx.execute(
                    "INSERT INTO approval_rules (id, scope_type, scope_id, match_json, action, priority, enabled, created_at)
                     VALUES (?1, 'global', NULL, ?2, ?3, 10, 1, ?4)",
                    params![new_id(), matcher.to_string(), if approve { "allow" } else { "deny" }, now_ms()],
                )?;
            }
        }
    }
    log_event(
        &tx,
        None,
        task_id.as_deref(),
        run_id.as_deref(),
        "approval.decided",
        Some("user"),
        json!({ "approvalId": approval_id, "approved": approve, "remember": remember, "note": note }),
    )?;
    tx.commit()?;
    let _ = app.emit(
        "approval.decided",
        json!({ "approvalId": approval_id, "approved": approve }),
    );
    gate.decide_channel(
        approval_id,
        Decision {
            allow: approve,
            reason: if note.is_empty() {
                if approve {
                    "用户已批准".into()
                } else {
                    "用户已拒绝".into()
                }
            } else {
                note.to_string()
            },
            by: "user".into(),
        },
    );
    Ok(())
}

pub fn list(conn: &Connection, status: Option<&str>) -> Result<Vec<Approval>> {
    let sql = if status.is_some() {
        "SELECT a.id, a.task_id, t.title, a.run_id, a.type, a.risk_level, a.title, a.detail_json,
                a.status, a.requested_by, a.decided_by, a.decision_note, a.created_at, a.decided_at
         FROM approvals a LEFT JOIN tasks t ON t.id = a.task_id
         WHERE a.status = ?1 ORDER BY a.created_at DESC LIMIT 200"
    } else {
        "SELECT a.id, a.task_id, t.title, a.run_id, a.type, a.risk_level, a.title, a.detail_json,
                a.status, a.requested_by, a.decided_by, a.decision_note, a.created_at, a.decided_at
         FROM approvals a LEFT JOIN tasks t ON t.id = a.task_id
         ORDER BY a.created_at DESC LIMIT 200"
    };
    let mut stmt = conn.prepare(sql)?;
    let mapper = |r: &rusqlite::Row| -> rusqlite::Result<Approval> {
        let detail: String = r.get(7)?;
        Ok(Approval {
            id: r.get(0)?,
            task_id: r.get(1)?,
            task_title: r.get(2)?,
            run_id: r.get(3)?,
            r#type: r.get(4)?,
            risk_level: r.get(5)?,
            title: r.get(6)?,
            detail: serde_json::from_str(&detail).unwrap_or(Value::Null),
            status: r.get(8)?,
            requested_by: r.get(9)?,
            decided_by: r.get(10)?,
            decision_note: r.get(11)?,
            created_at: r.get(12)?,
            decided_at: r.get(13)?,
        })
    };
    let items: Vec<Approval> = if let Some(s) = status {
        stmt.query_map(params![s], mapper)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map([], mapper)?.filter_map(|r| r.ok()).collect()
    };
    Ok(items)
}

pub fn pending_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM approvals WHERE status = 'pending'",
        [],
        |r| r.get(0),
    )?)
}

pub fn create_stage_approval(
    conn: &Connection,
    task_id: &str,
    run_id: Option<&str>,
    title: &str,
    detail: Value,
) -> Result<String> {
    let id = new_id();
    conn.execute(
        "INSERT INTO approvals (id, task_id, run_id, type, risk_level, title, detail_json, status, requested_by, created_at, expires_at)
         VALUES (?1, ?2, ?3, 'stage_gate', 'medium', ?4, ?5, 'pending', 'orchestrator', ?6, ?7)",
        params![id, task_id, run_id, title, detail.to_string(), now_ms(), now_ms() + 24 * 3600 * 1000],
    )?;
    log_event(
        conn,
        None,
        Some(task_id),
        run_id,
        "approval.requested",
        Some("orchestrator"),
        json!({ "approvalId": id, "title": title }),
    )?;
    Ok(id)
}

pub fn list_rules(conn: &Connection) -> Result<Vec<ApprovalRule>> {
    let mut stmt = conn.prepare(
        "SELECT id, scope_type, scope_id, match_json, action, priority, enabled, created_at
         FROM approval_rules ORDER BY priority ASC, created_at DESC",
    )?;
    let items = stmt.query_map([], |r| {
        let m: String = r.get(3)?;
        Ok(ApprovalRule {
            id: r.get(0)?,
            scope_type: r.get(1)?,
            scope_id: r.get(2)?,
            r#match: serde_json::from_str(&m).unwrap_or(Value::Null),
            action: r.get(4)?,
            priority: r.get(5)?,
            enabled: r.get::<_, i64>(6)? != 0,
            created_at: r.get(7)?,
        })
    })?;
    Ok(items.filter_map(|r| r.ok()).collect())
}

pub fn upsert_rule(
    conn: &Connection,
    id: Option<String>,
    matcher: Value,
    action: &str,
    priority: i64,
) -> Result<String> {
    if let Some(existing) = id {
        conn.execute(
            "UPDATE approval_rules SET match_json = ?1, action = ?2, priority = ?3 WHERE id = ?4",
            params![matcher.to_string(), action, priority, existing],
        )?;
        Ok(existing)
    } else {
        let new = new_id();
        conn.execute(
            "INSERT INTO approval_rules (id, scope_type, scope_id, match_json, action, priority, enabled, created_at)
             VALUES (?1, 'global', NULL, ?2, ?3, ?4, 1, ?5)",
            params![new, matcher.to_string(), action, priority, now_ms()],
        )?;
        Ok(new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::fs;

    fn ctx() -> GateRunCtx {
        GateRunCtx {
            run_id: "run-1".into(),
            task_id: "task-1".into(),
            project_id: "project-1".into(),
            project_path: std::env::temp_dir().to_string_lossy().to_string(),
            seat_name: "dev".into(),
            agent_name: "claude".into(),
        }
    }

    fn conn_with_rule(matcher: &str, action: &str) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE approval_rules (
               match_json TEXT NOT NULL, action TEXT NOT NULL, enabled INTEGER NOT NULL,
               priority INTEGER NOT NULL, created_at INTEGER NOT NULL
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO approval_rules (match_json, action, enabled, priority, created_at)
             VALUES (?1, ?2, 1, 10, 0)",
            params![matcher, action],
        )
        .unwrap();
        conn
    }

    fn bash(command: &str) -> Value {
        json!({ "command": command })
    }

    #[test]
    fn inside_accepts_new_file_and_dotdot_paths() {
        let base = std::env::temp_dir().join(format!("agenthub-inside-{}", std::process::id()));
        let proj = base.join("proj");
        fs::create_dir_all(&proj).unwrap();

        assert!(inside(&proj.join("new-file.txt"), &proj));
        assert!(inside(&proj.join("sub").join("a.txt"), &proj));
        assert!(inside(&proj.join("..").join("proj").join("x.txt"), &proj));
        assert!(!inside(&base.join("outside.txt"), &proj));
        assert!(!inside(&base, &proj));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn unknown_tool_defaults_to_ask() {
        let conn = Connection::open_in_memory().unwrap();
        let outcome = evaluate(&conn, &ctx(), "mcp__custom__deploy", &json!({}));
        assert!(matches!(outcome, PolicyOutcome::Ask { .. }));
    }

    #[test]
    fn remembered_fuzzy_rule_cannot_bypass_compound_command() {
        let conn = conn_with_rule(r#"{"tool":"Bash","commandContains":"git status"}"#, "allow");
        let outcome = evaluate(&conn, &ctx(), "Bash", &bash("evil && git status"));
        assert!(matches!(outcome, PolicyOutcome::Ask { .. }));
    }

    #[test]
    fn exact_rule_allows_the_exact_command_only() {
        let conn = conn_with_rule(r#"{"tool":"Bash","commandExact":"npm run build"}"#, "allow");
        assert!(matches!(
            evaluate(&conn, &ctx(), "Bash", &bash("npm run build")),
            PolicyOutcome::Allow(_)
        ));
        assert!(matches!(
            evaluate(&conn, &ctx(), "Bash", &bash("npm run build && rm -rf .")),
            PolicyOutcome::Ask { .. }
        ));
    }

    #[test]
    fn danger_check_runs_before_allow_rules() {
        let conn = conn_with_rule(r#"{"tool":"Bash","commandContains":"rm"}"#, "allow");
        let outcome = evaluate(&conn, &ctx(), "Bash", &bash("rm -rf ./src"));
        match outcome {
            PolicyOutcome::Ask { risk, .. } => assert_eq!(risk, "high"),
            _ => panic!("dangerous command must ask"),
        }
    }

    #[test]
    fn safe_build_commands_are_allowed() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(matches!(
            evaluate(&conn, &ctx(), "Bash", &bash("git status --short")),
            PolicyOutcome::Allow(_)
        ));
        assert!(matches!(
            evaluate(&conn, &ctx(), "Bash", &bash("npm run build")),
            PolicyOutcome::Allow(_)
        ));
        assert!(matches!(
            evaluate(&conn, &ctx(), "Bash", &bash("some-unknown-binary --flag")),
            PolicyOutcome::Ask { .. }
        ));
        assert!(matches!(
            evaluate(
                &conn,
                &ctx(),
                "Bash",
                &bash("node -e \"require('fs').rmSync('.', {recursive:true})\"")
            ),
            PolicyOutcome::Ask { .. }
        ));
        assert!(matches!(
            evaluate(&conn, &ctx(), "Bash", &bash("python -c \"print(1)\"")),
            PolicyOutcome::Ask { .. }
        ));
    }

    fn conn_with_approval_tables() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (
               id TEXT PRIMARY KEY, status TEXT NOT NULL, stage_index INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE approvals (
               id TEXT PRIMARY KEY, task_id TEXT, run_id TEXT, type TEXT NOT NULL,
               risk_level TEXT, title TEXT, detail_json TEXT NOT NULL, status TEXT NOT NULL,
               requested_by TEXT, decision_note TEXT, decided_by TEXT,
               created_at INTEGER NOT NULL, expires_at INTEGER, decided_at INTEGER
             );
             INSERT INTO tasks (id, status, stage_index) VALUES
               ('task-running', 'running', 0),
               ('task-blocked', 'blocked', 0),
               ('task-waiting', 'waiting_approval', 1);
             INSERT INTO approvals (id, task_id, type, detail_json, status, created_at) VALUES
               ('a-running', 'task-running', 'tool_permission', '{}', 'pending', 0),
               ('a-running-done', 'task-running', 'tool_permission', '{}', 'approved', 0),
               ('a-blocked', 'task-blocked', 'tool_permission', '{}', 'pending', 0),
               ('a-gate-ok', 'task-waiting', 'stage_gate', '{\"stageIndex\":1}', 'pending', 0),
               ('a-gate-bad', 'task-waiting', 'stage_gate', '{\"stageIndex\":2}', 'pending', 0);",
        )
        .unwrap();
        conn
    }

    fn approval_status(conn: &Connection, id: &str) -> String {
        conn.query_row(
            "SELECT status FROM approvals WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn expire_pending_only_touches_target_task_pending_rows() {
        let conn = conn_with_approval_tables();
        let changed = expire_pending_for_task(&conn, "task-running", "任务已取消").unwrap();
        assert_eq!(changed, 1);
        assert_eq!(approval_status(&conn, "a-running"), "expired");
        assert_eq!(approval_status(&conn, "a-running-done"), "approved");
        assert_eq!(approval_status(&conn, "a-blocked"), "pending");
    }

    #[test]
    fn decide_rejects_terminal_task_and_mismatched_gate() {
        let conn = conn_with_approval_tables();
        assert_eq!(
            decide_if_active(&conn, "a-running", "approved", "").unwrap(),
            1
        );
        assert_eq!(
            decide_if_active(&conn, "a-blocked", "approved", "").unwrap(),
            0
        );
        assert_eq!(approval_status(&conn, "a-blocked"), "pending");
        assert_eq!(
            decide_if_active(&conn, "a-gate-ok", "approved", "").unwrap(),
            1
        );
        assert_eq!(
            decide_if_active(&conn, "a-gate-bad", "approved", "").unwrap(),
            0
        );
        // 已决定的审批不能再次生效。
        assert_eq!(
            decide_if_active(&conn, "a-running", "denied", "").unwrap(),
            0
        );
    }
}
