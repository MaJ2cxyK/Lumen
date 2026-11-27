use serde::{Deserialize, Serialize};
use serde_json::Value;

fn v_string(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

fn v_bool(v: &Value, key: &str) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(false)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub kind: String,
    pub description: Option<String>,
    /// 项目目录是否为 Git 仓库（仅探测，不入库）。
    pub is_git: bool,
    pub git_reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub adapter: String,
    pub command: Option<String>,
    pub model: Option<String>,
    pub provider_id: Option<String>,
    pub status: String,
    pub version: Option<String>,
    pub capabilities: Value,
    pub last_seen_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedCli {
    pub name: String,
    pub command: String,
    pub adapter: String,
    pub version: Option<String>,
    pub installed: bool,
    pub recommended: bool,
    pub installable: bool,
    pub install_hint: Option<String>,
    /// 适配器真实能力（streaming / streamUsage / toolApproval / resume / budgetEnforcement / usageSource）。
    pub capabilities: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliVersionInfo {
    pub name: String,
    pub latest_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub model: Option<String>,
    /// 供应商返回并勾选保存的模型列表；`model` 为其中的默认项。
    pub models: Vec<String>,
    pub key_hint: Option<String>,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestResult {
    pub ok: bool,
    pub status: i64,
    pub latency_ms: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub description: Option<String>,
    pub config: Value,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamTemplateRole {
    pub role_key: String,
    pub role_name: String,
    pub responsibility: String,
    pub capability: String,
    pub gate: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamTemplate {
    pub key: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub roles: Vec<TeamTemplateRole>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub id: String,
    pub project_id: String,
    pub project_name: Option<String>,
    pub name: String,
    pub template_key: Option<String>,
    pub description: Option<String>,
    pub seats_count: i64,
    pub assigned_count: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seat {
    pub id: String,
    pub team_id: String,
    pub role_key: String,
    pub role_name: String,
    pub responsibility: Option<String>,
    pub agent_id: Option<String>,
    pub agent_name: Option<String>,
    pub order_index: i64,
    pub gate: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub project_name: Option<String>,
    pub team_id: Option<String>,
    pub team_name: Option<String>,
    /// 父任务：子任务共享项目与团队，但独立运行与计费。
    pub parent_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub acceptance: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    /// 任务授权：`ask` 逐次确认，`auto` 任务内自动。
    pub mandate: String,
    /// 隔离工作区路径；为空时使用项目目录。
    pub work_dir: Option<String>,
    /// 隔离工作区仍有未提交或未应用的改动，等待用户审查。
    pub review_pending: bool,
    pub status: String,
    pub priority: i64,
    pub assignee_seat_id: Option<String>,
    pub stage_index: i64,
    pub stage_key: Option<String>,
    pub actual_tokens: i64,
    pub actual_cost_micros: i64,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

/// 任务附件：文件复制到应用数据目录，任务删除时记录级联清理。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub task_id: String,
    pub name: String,
    pub path: String,
    pub size: i64,
    pub created_at: i64,
}

/// 任务依赖：只有被依赖任务完成后才允许派发。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDependency {
    pub task_id: String,
    pub depends_on_id: String,
    pub title: String,
    pub status: String,
}

/// 验收清单条目，持久化为 tasks.acceptance_json。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecklistItem {
    pub id: String,
    pub text: String,
    pub done: bool,
}

/// 任务列表分页结果：items 只含当前页，total/status_counts 为过滤后的全量统计。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPage {
    pub items: Vec<Task>,
    pub total: i64,
    pub status_counts: std::collections::BTreeMap<String, i64>,
    pub review_count: i64,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub task_id: String,
    pub seat_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_name: Option<String>,
    pub seat_name: Option<String>,
    pub stage_key: Option<String>,
    pub status: String,
    pub external_session_id: Option<String>,
    /// 隔离级别：`appcontainer` 或 `process`（Job Object）。
    pub isolation: String,
    /// 未能使用 AppContainer 时的回退原因代码；正常运行为空。
    pub isolation_reason: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_micros: i64,
    pub error: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub run_id: String,
    pub tool_name: String,
    pub input: Value,
    pub output: Option<Value>,
    pub status: String,
    pub risk_level: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub seat_id: Option<String>,
    pub agent_id: Option<String>,
    pub seat_name: Option<String>,
    pub agent_name: Option<String>,
    pub kind: String,
    pub role: Option<String>,
    pub content: String,
    pub metadata: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEvent {
    pub id: String,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub cost_micros: i64,
    pub usage_source: String,
    pub occurred_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePoint {
    pub date: String,
    pub tokens: i64,
    pub cost_micros: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedUsage {
    pub key: String,
    pub name: String,
    pub tokens: i64,
    pub cost_micros: i64,
    pub rounds: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub today_tokens: i64,
    pub total_tokens: i64,
    pub today_cost_micros: i64,
    pub total_cost_micros: i64,
    pub cache_hit_pct: f64,
    pub rounds: i64,
    pub today_rounds: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStat {
    pub id: String,
    pub name: String,
    pub adapter: String,
    pub status: String,
    pub tokens: i64,
    pub today_tokens: i64,
    pub cost_micros: i64,
    pub rounds: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSummary {
    pub usage: UsageSummary,
    pub pending_approvals: i64,
    /// 待审查的隔离工作区任务数。
    pub pending_reviews: i64,
    pub agents: Vec<AgentStat>,
    pub teams: Vec<Team>,
    pub series: Vec<UsagePoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub id: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub scope_name: String,
    pub unit: String,
    pub period: String,
    pub limit_value: i64,
    pub warn_pct: i64,
    pub hard_stop: bool,
    pub enabled: bool,
    pub spent: i64,
    pub reserved: i64,
    pub pct: f64,
    pub blocked: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: String,
    pub task_id: Option<String>,
    pub task_title: Option<String>,
    pub run_id: Option<String>,
    pub r#type: String,
    pub risk_level: Option<String>,
    pub title: String,
    pub detail: Value,
    pub status: String,
    pub requested_by: Option<String>,
    pub decided_by: Option<String>,
    pub decision_note: Option<String>,
    pub created_at: i64,
    pub decided_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRule {
    pub id: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub r#match: Value,
    pub action: String,
    pub priority: i64,
    pub enabled: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventItem {
    pub id: String,
    pub seq: i64,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub r#type: String,
    pub actor: Option<String>,
    pub payload: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub db_path: String,
    pub gate_port: u16,
    pub platform: String,
}

fn default_true() -> bool {
    true
}

fn default_run_max_minutes() -> i64 {
    180
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub max_concurrent_runs: i64,
    pub approval_timeout_secs: i64,
    pub transcript_retention_days: i64,
    pub notify: bool,
    /// 真实 API Key 只留在本地代理，子进程只拿一次性令牌。
    #[serde(default = "default_true")]
    pub proxy_enabled: bool,
    /// 单次运行时长上限（分钟），0 表示不限制。
    #[serde(default = "default_run_max_minutes")]
    pub run_max_minutes: i64,
    /// 检测到未经网关的工具调用时，是否直接判定运行失败。
    #[serde(default)]
    pub hook_strict: bool,
    /// 非 Claude 适配器没有工具级 Hook，运行前是否要求人工放行。
    #[serde(default = "default_true")]
    pub non_claude_run_approval: bool,
    /// Windows AppContainer 文件级隔离；默认开启，仅在模型代理可用时实际生效。
    #[serde(default = "default_true")]
    pub sandbox_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            max_concurrent_runs: 3,
            approval_timeout_secs: 600,
            transcript_retention_days: 90,
            notify: true,
            proxy_enabled: true,
            run_max_minutes: 180,
            hook_strict: false,
            non_claude_run_approval: true,
            sandbox_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamTemplateRoleDef {
    pub role_key: String,
    pub role_name: String,
    pub responsibility: String,
    pub capability: String,
    pub gate: bool,
}

pub fn parse_template_role(v: &Value) -> TeamTemplateRoleDef {
    TeamTemplateRoleDef {
        role_key: v_string(v, "roleKey"),
        role_name: v_string(v, "roleName"),
        responsibility: v_string(v, "responsibility"),
        capability: v_string(v, "capability"),
        gate: v_bool(v, "gate"),
    }
}

pub fn template_roles(template_key: &str) -> Vec<TeamTemplateRoleDef> {
    let raw = match template_key {
        "software-dev" => include_str!("../templates/teams/software-dev.json"),
        "quant-research" => include_str!("../templates/teams/quant-research.json"),
        "office-ops" => include_str!("../templates/teams/office-ops.json"),
        _ => include_str!("../templates/teams/office-ops.json"),
    };
    let value: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    value
        .get("roles")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().map(parse_template_role).collect())
        .unwrap_or_default()
}

pub fn template_meta(template_key: &str) -> (String, String, String, String) {
    let raw = match template_key {
        "software-dev" => include_str!("../templates/teams/software-dev.json"),
        "quant-research" => include_str!("../templates/teams/quant-research.json"),
        "office-ops" => include_str!("../templates/teams/office-ops.json"),
        _ => include_str!("../templates/teams/office-ops.json"),
    };
    let v: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    (
        v_string(&v, "key"),
        v_string(&v, "name"),
        v_string(&v, "description"),
        v_string(&v, "icon"),
    )
}

impl DashboardSummary {
    pub fn empty() -> Self {
        Self {
            usage: UsageSummary {
                today_tokens: 0,
                total_tokens: 0,
                today_cost_micros: 0,
                total_cost_micros: 0,
                cache_hit_pct: 0.0,
                rounds: 0,
                today_rounds: 0,
            },
            pending_approvals: 0,
            pending_reviews: 0,
            agents: vec![],
            teams: vec![],
            series: vec![],
        }
    }
}
