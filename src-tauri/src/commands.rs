use crate::approval;
use crate::db::{insert_message, log_event, new_id, now_ms};
use crate::detect;
use crate::error::{AppError, Result};
use crate::models::*;
use crate::orchestrator::{self, PipelineDeps};
use crate::state::AppState;
use crate::task_lifecycle::{self, TaskStatus, TaskTransition};
use crate::usage;
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tauri::State;
use tauri::{AppHandle, Emitter};

fn conn(state: &State<'_, AppState>) -> Result<Connection> {
    state.conn()
}

fn map_message(r: &rusqlite::Row) -> rusqlite::Result<Message> {
    let metadata: String = r.get(10)?;
    Ok(Message {
        id: r.get(0)?,
        task_id: r.get(1)?,
        run_id: r.get(2)?,
        seat_id: r.get(3)?,
        agent_id: r.get(4)?,
        seat_name: r.get(5)?,
        agent_name: r.get(6)?,
        kind: r.get(7)?,
        role: r.get(8)?,
        content: r.get(9)?,
        metadata: serde_json::from_str(&metadata).unwrap_or(Value::Null),
        created_at: r.get(11)?,
    })
}

const MESSAGE_SQL: &str = "SELECT m.id, m.task_id, m.run_id, m.seat_id, m.agent_id,
        s.role_name, a.name, m.kind, m.role, m.content, m.metadata_json, m.created_at
     FROM messages m
     LEFT JOIN seats s ON s.id = m.seat_id
     LEFT JOIN agents a ON a.id = m.agent_id";

const TASK_SELECT: &str =
    "SELECT t.id, t.project_id, p.name, t.team_id, tm.name, t.title, t.description, t.acceptance,
        t.model, t.reasoning, COALESCE(t.mandate, 'ask'), t.work_dir,
        t.status, t.priority, t.assignee_seat_id, t.stage_index, t.stage_key,
        t.actual_tokens, t.actual_cost_micros, t.created_at, t.started_at, t.finished_at,
        t.review_pending, t.parent_id
     FROM tasks t LEFT JOIN projects p ON p.id = t.project_id
     LEFT JOIN teams tm ON tm.id = t.team_id";

fn map_task_row(r: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        project_id: r.get(1)?,
        project_name: r.get(2)?,
        team_id: r.get(3)?,
        team_name: r.get(4)?,
        parent_id: r.get(23)?,
        title: r.get(5)?,
        description: r.get(6)?,
        acceptance: r.get(7)?,
        model: r.get(8)?,
        reasoning: r.get(9)?,
        mandate: r.get(10)?,
        work_dir: r.get(11)?,
        status: r.get(12)?,
        priority: r.get(13)?,
        assignee_seat_id: r.get(14)?,
        stage_index: r.get(15)?,
        stage_key: r.get(16)?,
        actual_tokens: r.get(17)?,
        actual_cost_micros: r.get(18)?,
        created_at: r.get(19)?,
        started_at: r.get(20)?,
        finished_at: r.get(21)?,
        review_pending: r.get::<_, i64>(22)? != 0,
    })
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> Result<AppInfo> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: state.base_dir.to_string_lossy().to_string(),
        db_path: state.db.path.to_string_lossy().to_string(),
        gate_port: state.gate.port,
        platform: std::env::consts::OS.to_string(),
    })
}

#[tauri::command]
pub fn app_settings_get(state: State<'_, AppState>) -> Result<AppSettings> {
    Ok(state.settings())
}

#[tauri::command]
pub fn app_settings_save(state: State<'_, AppState>, settings: AppSettings) -> Result<()> {
    let previous = state.settings();
    crate::settings::save(&state.base_dir, &settings)?;
    if let Ok(mut current) = state.settings.lock() {
        *current = settings.clone();
    }
    // 关闭沙箱时回收已授予的 ACL 与运行时 profile，避免留下可被容器访问的目录。
    #[cfg(windows)]
    if previous.sandbox_enabled && !settings.sandbox_enabled {
        if state.registry.active_count() > 0 {
            tracing::warn!("仍有运行进行中，跳过沙箱清理；下次启动时会再次尝试");
        } else {
            match crate::sandbox::session::cleanup(
                &state.base_dir,
                crate::sandbox::session::RUN_PROFILE,
            ) {
                Ok(count) => tracing::info!("沙箱已清理，回收 {count} 项 ACL 记录"),
                Err(err) => tracing::warn!("清理沙箱资源失败：{err}"),
            }
        }
    }
    Ok(())
}

/// 前端切换语言时同步后端，保证之后生成的通知、审批与系统消息使用同一语言。
#[tauri::command]
pub fn set_backend_lang(app: AppHandle, lang: String) {
    let lang = if lang == "en" { "en" } else { "zh" };
    crate::i18n::set_lang(lang);
    crate::i18n::save(&app, lang);
}

#[tauri::command]
pub fn open_data_dir(state: State<'_, AppState>) -> Result<()> {
    std::process::Command::new("explorer")
        .arg(state.base_dir.as_os_str())
        .spawn()
        .map_err(|e| AppError::Msg(format!("打开目录失败：{e}")))?;
    Ok(())
}

/// AppContainer 文件级隔离自检：验证工作目录可读写、外部路径被拦截、relay 回路可用。
#[tauri::command]
pub async fn sandbox_probe(state: State<'_, AppState>) -> Result<Value> {
    #[cfg(windows)]
    {
        let base = state.base_dir.clone();
        let report =
            tauri::async_runtime::spawn_blocking(move || crate::sandbox::probe::run(&base))
                .await
                .map_err(|e| AppError::Msg(format!("沙箱自检任务失败：{e}")))?
                .map_err(AppError::Msg)?;
        Ok(serde_json::to_value(report)?)
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        Err(AppError::Msg("沙箱自检仅支持 Windows".to_string()))
    }
}

#[tauri::command]
pub async fn cli_detect(with_version: bool) -> Result<Vec<DetectedCli>> {
    tauri::async_runtime::spawn_blocking(move || detect::detect_all(with_version))
        .await
        .map_err(|e| AppError::Msg(e.to_string()))
}

#[tauri::command]
pub async fn cli_install(name: String) -> Result<String> {
    tauri::async_runtime::spawn_blocking(move || detect::install(&name))
        .await
        .map_err(|e| AppError::Msg(e.to_string()))?
        .map_err(AppError::Msg)
}

#[tauri::command]
pub async fn cli_latest_versions(names: Vec<String>) -> Result<Vec<CliVersionInfo>> {
    tauri::async_runtime::spawn_blocking(move || detect::latest_versions(&names))
        .await
        .map_err(|e| AppError::Msg(e.to_string()))
}

#[tauri::command]
pub fn agents_list(state: State<'_, AppState>) -> Result<Vec<Agent>> {
    let conn = conn(&state)?;
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, adapter, command, model, provider_id, status, version, capabilities_json, last_seen_at, created_at
         FROM agents ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        let caps: String = r.get(9)?;
        Ok(Agent {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
            adapter: r.get(3)?,
            command: r.get(4)?,
            model: r.get(5)?,
            provider_id: r.get(6)?,
            status: r.get(7)?,
            version: r.get(8)?,
            capabilities: serde_json::from_str(&caps).unwrap_or(Value::Null),
            last_seen_at: r.get(10)?,
            created_at: r.get(11)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn default_args(adapter: &str) -> Vec<String> {
    if adapter == "opencode" {
        vec!["run".to_string()]
    } else {
        Vec::new()
    }
}

#[tauri::command]
pub fn agent_create(
    state: State<'_, AppState>,
    name: String,
    adapter: String,
    command: String,
    model: Option<String>,
    args: Option<Vec<String>>,
    provider_id: Option<String>,
) -> Result<String> {
    if name.trim().is_empty() {
        return Err("Agent 名称不能为空".into());
    }
    let cmd = if command.trim().is_empty() {
        let cli = match adapter.as_str() {
            "claude-code" => "claude",
            "opencode" => "opencode",
            _ => return Err("该适配器需要填写启动命令".into()),
        };
        detect::resolve(cli)
            .map(|p| p.to_string_lossy().to_string())
            .ok_or_else(|| AppError::Msg(format!("未检测到 {cli}，请先安装或手动填写命令")))?
    } else {
        command.trim().to_string()
    };
    let args = args.unwrap_or_else(|| default_args(&adapter));
    let capabilities = detect::capabilities_for(&adapter);
    let conn = conn(&state)?;
    let id = new_id();
    conn.execute(
        "INSERT INTO agents (id, name, kind, adapter, command, args_json, model, provider_id, capabilities_json, status, created_at)
         VALUES (?1, ?2, 'cli', ?3, ?4, ?5, ?6, ?7, ?8, 'idle', ?9)",
        params![
            id,
            name.trim(),
            adapter,
            cmd,
            serde_json::to_string(&args)?,
            model.filter(|m| !m.trim().is_empty()),
            provider_id.filter(|p| !p.trim().is_empty()),
            capabilities.to_string(),
            now_ms()
        ],
    )?;
    Ok(id)
}

#[tauri::command]
pub fn agent_update(
    state: State<'_, AppState>,
    id: String,
    name: String,
    command: String,
    model: Option<String>,
    args: Option<Vec<String>>,
    provider_id: Option<String>,
) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute(
        "UPDATE agents SET name = ?1, command = ?2, model = ?3, provider_id = ?4 WHERE id = ?5",
        params![
            name.trim(),
            command.trim(),
            model.filter(|m| !m.trim().is_empty()),
            provider_id.filter(|p| !p.trim().is_empty()),
            id
        ],
    )?;
    if let Some(args) = args {
        conn.execute(
            "UPDATE agents SET args_json = ?1 WHERE id = ?2",
            params![serde_json::to_string(&args)?, id],
        )?;
    }
    Ok(())
}

#[tauri::command]
pub fn agent_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    let tx = conn.unchecked_transaction()?;
    // 历史运行保留，但解除 Agent 关联；避免外键阻断删除。
    tx.execute(
        "UPDATE runs SET agent_id = NULL WHERE agent_id = ?1",
        params![id],
    )?;
    tx.execute(
        "UPDATE messages SET agent_id = NULL WHERE agent_id = ?1",
        params![id],
    )?;
    tx.execute(
        "UPDATE seats SET agent_id = NULL WHERE agent_id = ?1",
        params![id],
    )?;
    tx.execute("DELETE FROM agents WHERE id = ?1", params![id])?;
    tx.commit()?;
    Ok(())
}

#[tauri::command]
pub fn projects_list(state: State<'_, AppState>) -> Result<Vec<Project>> {
    let conn = conn(&state)?;
    let mut stmt = conn.prepare(
        "SELECT id, name, path, kind, description, created_at, updated_at FROM projects ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Project {
            id: r.get(0)?,
            name: r.get(1)?,
            path: r.get(2)?,
            kind: r.get(3)?,
            description: r.get(4)?,
            is_git: false,
            git_reason: None,
            created_at: r.get(5)?,
            updated_at: r.get(6)?,
        })
    })?;
    let mut items: Vec<Project> = rows.filter_map(|r| r.ok()).collect();
    // Git 探测只用于展示与派发提示，不写入数据库。
    for project in &mut items {
        let probe = crate::review::git_state(std::path::Path::new(&project.path));
        project.is_git = probe.is_git;
        project.git_reason = probe.reason;
    }
    Ok(items)
}

#[tauri::command]
pub fn project_git_check(path: String) -> crate::review::GitState {
    crate::review::git_state(std::path::Path::new(path.trim()))
}

#[tauri::command]
pub fn project_create(
    state: State<'_, AppState>,
    name: String,
    path: String,
    kind: String,
    description: Option<String>,
) -> Result<String> {
    if name.trim().is_empty() {
        return Err("项目名称不能为空".into());
    }
    if !std::path::Path::new(&path).is_dir() {
        return Err("项目目录不存在".into());
    }
    let conn = conn(&state)?;
    let id = new_id();
    let now = now_ms();
    conn.execute(
        "INSERT INTO projects (id, name, path, kind, description, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![id, name.trim(), path.trim(), kind, description, now],
    )?;
    log_event(
        &conn,
        Some(&id),
        None,
        None,
        "project.created",
        Some("user"),
        json!({ "name": name }),
    )?;
    Ok(id)
}

#[tauri::command]
pub fn project_update(
    state: State<'_, AppState>,
    id: String,
    name: String,
    path: String,
    kind: String,
    description: Option<String>,
) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute(
        "UPDATE projects SET name = ?1, path = ?2, kind = ?3, description = ?4, updated_at = ?5 WHERE id = ?6",
        params![name.trim(), path.trim(), kind, description, now_ms(), id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn project_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    let running: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE project_id = ?1
           AND status IN ('running', 'queued', 'waiting_approval')",
        params![id],
        |r| r.get(0),
    )?;
    if running > 0 {
        return Err("项目下有任务正在运行，请先停止后再删除项目".into());
    }
    let project_path: Option<String> = conn
        .query_row(
            "SELECT path FROM projects WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    let worktrees: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT work_dir FROM tasks WHERE project_id = ?1 AND work_dir IS NOT NULL")?;
        let rows = stmt.query_map(params![id], |r| r.get(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM messages WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM approvals WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM events WHERE project_id = ?1
           OR task_id IN (SELECT id FROM tasks WHERE project_id = ?1)
           OR run_id IN (SELECT r.id FROM runs r JOIN tasks t ON t.id = r.task_id WHERE t.project_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM artifacts WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?1)
           OR run_id IN (SELECT r.id FROM runs r JOIN tasks t ON t.id = r.task_id WHERE t.project_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM budget_reservations WHERE run_id IN (
           SELECT r.id FROM runs r JOIN tasks t ON t.id = r.task_id WHERE t.project_id = ?1
         ) OR budget_id IN (SELECT id FROM budgets WHERE (scope_type = 'project' AND scope_id = ?1)
           OR (scope_type = 'team' AND scope_id IN (SELECT id FROM teams WHERE project_id = ?1)))",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM usage_events WHERE project_id = ?1",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM budgets WHERE (scope_type = 'project' AND scope_id = ?1)
           OR (scope_type = 'team' AND scope_id IN (SELECT id FROM teams WHERE project_id = ?1))
           OR (scope_type = 'task' AND scope_id IN (SELECT id FROM tasks WHERE project_id = ?1))",
        params![id],
    )?;
    tx.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
    tx.commit()?;
    for dir in worktrees {
        crate::review::cleanup_worktree_best_effort(&state, Some(&dir), project_path.as_deref());
    }
    Ok(())
}

#[tauri::command]
pub fn team_templates() -> Vec<TeamTemplate> {
    ["software-dev", "office-ops", "quant-research"]
        .iter()
        .map(|key| {
            let (k, name, description, icon) = template_meta(key);
            let roles = template_roles(key)
                .into_iter()
                .map(|r| TeamTemplateRole {
                    role_key: r.role_key,
                    role_name: r.role_name,
                    responsibility: r.responsibility,
                    capability: r.capability,
                    gate: r.gate,
                })
                .collect();
            TeamTemplate {
                key: k,
                name,
                description,
                icon,
                roles,
            }
        })
        .collect()
}

#[tauri::command]
pub fn teams_list(state: State<'_, AppState>, project_id: Option<String>) -> Result<Vec<Team>> {
    let conn = conn(&state)?;
    teams_query(&conn, project_id)
}

#[tauri::command]
pub fn team_create(
    state: State<'_, AppState>,
    project_id: String,
    name: String,
    template_key: String,
    description: Option<String>,
) -> Result<String> {
    let roles = template_roles(&template_key);
    if roles.is_empty() {
        return Err("团队模板不存在或没有角色".into());
    }
    let conn = conn(&state)?;
    let tx = conn.unchecked_transaction()?;
    let id = new_id();
    tx.execute(
        "INSERT INTO teams (id, project_id, name, template_key, description, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            project_id,
            name.trim(),
            template_key,
            description,
            now_ms()
        ],
    )?;
    for (i, role) in roles.iter().enumerate() {
        tx.execute(
            "INSERT INTO seats (id, team_id, role_key, role_name, responsibility, order_index, gate)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new_id(),
                id,
                role.role_key,
                role.role_name,
                role.responsibility,
                i as i64,
                role.gate as i64
            ],
        )?;
    }
    tx.commit()?;
    Ok(id)
}

#[tauri::command]
pub fn team_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute("DELETE FROM teams WHERE id = ?1", params![id])?;
    Ok(())
}

#[tauri::command]
pub fn seats_list(state: State<'_, AppState>, team_id: String) -> Result<Vec<Seat>> {
    let conn = conn(&state)?;
    let mut stmt = conn.prepare(
        "SELECT s.id, s.team_id, s.role_key, s.role_name, s.responsibility, s.agent_id, a.name,
                s.order_index, s.gate
         FROM seats s LEFT JOIN agents a ON a.id = s.agent_id
         WHERE s.team_id = ?1 ORDER BY s.order_index ASC",
    )?;
    let rows = stmt.query_map(params![team_id], |r| {
        Ok(Seat {
            id: r.get(0)?,
            team_id: r.get(1)?,
            role_key: r.get(2)?,
            role_name: r.get(3)?,
            responsibility: r.get(4)?,
            agent_id: r.get(5)?,
            agent_name: r.get(6)?,
            order_index: r.get(7)?,
            gate: r.get::<_, i64>(8)? != 0,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn seat_assign(
    state: State<'_, AppState>,
    seat_id: String,
    agent_id: Option<String>,
) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute(
        "UPDATE seats SET agent_id = ?1 WHERE id = ?2",
        params![agent_id, seat_id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn seat_update(
    state: State<'_, AppState>,
    seat_id: String,
    role_name: String,
    responsibility: Option<String>,
    gate: bool,
) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute(
        "UPDATE seats SET role_name = ?1, responsibility = ?2, gate = ?3 WHERE id = ?4",
        params![role_name, responsibility, gate as i64, seat_id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn seat_reorder(
    state: State<'_, AppState>,
    team_id: String,
    seat_ids: Vec<String>,
) -> Result<()> {
    let mut conn = conn(&state)?;
    let tx = conn.transaction()?;
    for (index, seat_id) in seat_ids.iter().enumerate() {
        tx.execute(
            "UPDATE seats SET order_index = ?1 WHERE id = ?2 AND team_id = ?3",
            params![index as i64, seat_id, team_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[tauri::command]
pub fn seat_auto_assign(state: State<'_, AppState>, team_id: String) -> Result<i64> {
    let conn = conn(&state)?;
    let mut agents: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM agents ORDER BY created_at ASC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    if agents.is_empty() {
        return Err("还没有可用的 Agent，请先添加".into());
    }
    let seats: Vec<String> = {
        let mut stmt =
            conn.prepare("SELECT id FROM seats WHERE team_id = ?1 ORDER BY order_index ASC")?;
        let rows = stmt.query_map(params![team_id], |r| r.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let mut assigned = 0i64;
    for seat_id in seats.iter() {
        let agent = agents.remove(0);
        agents.push(agent.clone());
        let changed = conn.execute(
            "UPDATE seats SET agent_id = ?1 WHERE id = ?2 AND agent_id IS NULL",
            params![agent, seat_id],
        )?;
        assigned += changed as i64;
    }
    Ok(assigned)
}

#[tauri::command]
pub fn tasks_list(
    state: State<'_, AppState>,
    project_id: Option<String>,
    status: Option<String>,
) -> Result<Vec<Task>> {
    let conn = conn(&state)?;
    let mut sql = String::from(TASK_SELECT);
    sql.push_str(" WHERE 1=1");
    let mut values: Vec<SqlValue> = Vec::new();
    if let Some(pid) = project_id {
        sql.push_str(" AND t.project_id = ?");
        values.push(pid.into());
    }
    if let Some(st) = status {
        sql.push_str(" AND t.status = ?");
        values.push(st.into());
    }
    sql.push_str(" ORDER BY t.priority ASC, t.created_at DESC LIMIT 300");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values), map_task_row)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// 游标格式：`priority:createdAt:id`，与列表排序（优先级升序、创建时间倒序、ID 倒序）一致。
fn parse_task_cursor(raw: &str) -> Option<(i64, i64, String)> {
    let mut parts = raw.splitn(3, ':');
    let priority = parts.next()?.parse().ok()?;
    let created_at = parts.next()?.parse().ok()?;
    let id = parts.next()?.trim();
    if id.is_empty() {
        return None;
    }
    Some((priority, created_at, id.to_string()))
}

fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// 任务分页查询：看板用它替代固定 300 条的全量拉取。
fn query_tasks_page(
    conn: &Connection,
    project_id: Option<String>,
    status: Option<String>,
    query: Option<String>,
    attention_only: Option<bool>,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<TaskPage> {
    let mut filter = String::from(" WHERE 1=1");
    let mut values: Vec<SqlValue> = Vec::new();
    if let Some(pid) = project_id.filter(|v| !v.trim().is_empty()) {
        filter.push_str(" AND t.project_id = ?");
        values.push(pid.into());
    }
    if let Some(st) = status.filter(|v| !v.trim().is_empty()) {
        filter.push_str(" AND t.status = ?");
        values.push(st.into());
    }
    if let Some(q) = query
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        let pattern = like_pattern(&q);
        filter.push_str(
            " AND (t.title LIKE ? ESCAPE '\\' OR COALESCE(t.description, '') LIKE ? ESCAPE '\\')",
        );
        values.push(pattern.clone().into());
        values.push(pattern.into());
    }
    if attention_only.unwrap_or(false) {
        filter.push_str(&format!(
            " AND (t.status IN ('inbox', 'waiting_approval', 'blocked')
                   OR (t.review_pending = 1 AND t.status NOT IN ({})))",
            task_lifecycle::ACTIVE_STATUSES_SQL
        ));
    }

    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM tasks t{filter}"),
        params_from_iter(values.clone()),
        |r| r.get(0),
    )?;
    let status_counts: BTreeMap<String, i64> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT t.status, COUNT(*) FROM tasks t{filter} GROUP BY t.status"
        ))?;
        let rows = stmt.query_map(params_from_iter(values.clone()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?
    };
    let review_count: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM tasks t{filter} AND t.review_pending = 1
             AND t.status NOT IN ({})",
            task_lifecycle::ACTIVE_STATUSES_SQL
        ),
        params_from_iter(values.clone()),
        |r| r.get(0),
    )?;

    let page_limit = limit.unwrap_or(60).clamp(1, 200);
    let mut sql = format!("{TASK_SELECT}{filter}");
    let mut page_values = values;
    if let Some(raw) = cursor.filter(|v| !v.trim().is_empty()) {
        if let Some((priority, created_at, last_id)) = parse_task_cursor(&raw) {
            sql.push_str(
                " AND (t.priority > ? OR (t.priority = ? AND (t.created_at < ? OR (t.created_at = ? AND t.id < ?))))",
            );
            page_values.push(priority.into());
            page_values.push(priority.into());
            page_values.push(created_at.into());
            page_values.push(created_at.into());
            page_values.push(last_id.into());
        }
    }
    sql.push_str(" ORDER BY t.priority ASC, t.created_at DESC, t.id DESC LIMIT ?");
    page_values.push((page_limit + 1).into());

    let mut stmt = conn.prepare(&sql)?;
    let mut items = stmt
        .query_map(params_from_iter(page_values), map_task_row)?
        .collect::<rusqlite::Result<Vec<Task>>>()?;
    let has_more = items.len() > page_limit as usize;
    if has_more {
        items.truncate(page_limit as usize);
    }
    let next_cursor = if has_more {
        items
            .last()
            .map(|task| format!("{}:{}:{}", task.priority, task.created_at, task.id))
    } else {
        None
    };

    Ok(TaskPage {
        items,
        total,
        status_counts,
        review_count,
        has_more,
        next_cursor,
    })
}

#[tauri::command]
pub fn tasks_page(
    state: State<'_, AppState>,
    project_id: Option<String>,
    status: Option<String>,
    query: Option<String>,
    attention_only: Option<bool>,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<TaskPage> {
    let conn = conn(&state)?;
    query_tasks_page(
        &conn,
        project_id,
        status,
        query,
        attention_only,
        cursor,
        limit,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_tasks() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT);
             CREATE TABLE teams (id TEXT PRIMARY KEY, name TEXT);
             CREATE TABLE tasks (
               id TEXT PRIMARY KEY, project_id TEXT NOT NULL, team_id TEXT,
               title TEXT NOT NULL, description TEXT, acceptance TEXT,
               model TEXT, reasoning TEXT, mandate TEXT, work_dir TEXT,
               status TEXT NOT NULL, priority INTEGER NOT NULL,
               assignee_seat_id TEXT, stage_index INTEGER NOT NULL DEFAULT 0,
               stage_key TEXT, actual_tokens INTEGER NOT NULL DEFAULT 0,
               actual_cost_micros INTEGER NOT NULL DEFAULT 0,
               created_at INTEGER NOT NULL, started_at INTEGER, finished_at INTEGER,
               review_pending INTEGER NOT NULL DEFAULT 0,
               parent_id TEXT, acceptance_json TEXT NOT NULL DEFAULT '[]'
             );
             INSERT INTO projects VALUES ('p1', '项目一');
             INSERT INTO tasks (id, project_id, title, description, status, priority, created_at)
             VALUES ('t1', 'p1', '一', 'a', 'inbox', 3, 100),
                    ('t2', 'p1', '二', 'b', 'inbox', 3, 100),
                    ('t3', 'p1', '三', 'c', 'inbox', 3, 90),
                    ('t4', 'p1', '四', 'd', 'completed', 3, 80),
                    ('t5', 'p1', '五', 'e', 'blocked', 2, 70);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn tasks_page_cursor_has_no_overlap_or_gap() {
        let conn = seed_tasks();
        let first =
            query_tasks_page(&conn, Some("p1".into()), None, None, None, None, Some(2)).unwrap();
        assert_eq!(first.total, 5);
        assert_eq!(first.items.len(), 2);
        assert!(first.has_more);
        let cursor = first.next_cursor.clone().unwrap();

        let second = query_tasks_page(
            &conn,
            Some("p1".into()),
            None,
            None,
            None,
            Some(cursor),
            Some(2),
        )
        .unwrap();
        assert_eq!(second.total, 5);
        assert!(second.has_more);

        let ids: Vec<&str> = first
            .items
            .iter()
            .chain(second.items.iter())
            .map(|task| task.id.as_str())
            .collect();
        let unique: std::collections::HashSet<&&str> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "分页结果不应重复：{ids:?}");
    }

    #[test]
    fn tasks_page_filters_search_and_attention() {
        let conn = seed_tasks();
        let searched =
            query_tasks_page(&conn, None, None, Some("二".into()), None, None, Some(10)).unwrap();
        assert_eq!(searched.total, 1);
        assert_eq!(searched.items[0].id, "t2");

        let attention =
            query_tasks_page(&conn, None, None, None, Some(true), None, Some(10)).unwrap();
        assert_eq!(attention.total, 4);
        assert_eq!(attention.status_counts.get("inbox"), Some(&3));
        assert_eq!(attention.review_count, 0);
    }
}

#[tauri::command]
pub fn task_get(state: State<'_, AppState>, id: String) -> Result<Task> {
    let conn = conn(&state)?;
    let task = conn.query_row(
        &format!("{TASK_SELECT} WHERE t.id = ?1"),
        params![id],
        map_task_row,
    )?;
    Ok(task)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn task_create(
    state: State<'_, AppState>,
    project_id: String,
    team_id: Option<String>,
    title: String,
    description: Option<String>,
    acceptance: Option<String>,
    model: Option<String>,
    reasoning: Option<String>,
    mandate: Option<String>,
    isolated: Option<bool>,
    priority: Option<i64>,
    parent_id: Option<String>,
) -> Result<String> {
    if title.trim().is_empty() {
        return Err("任务标题不能为空".into());
    }
    let conn = conn(&state)?;
    let id = new_id();
    let project_path: String = conn
        .query_row(
            "SELECT path FROM projects WHERE id = ?1",
            params![project_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("项目不存在".into()))?;
    // 子任务必须与父任务同项目；未显式选团队时沿用父任务团队。
    let mut team_id = team_id;
    if let Some(parent_id) = parent_id.as_deref() {
        let parent: (String, Option<String>) = conn
            .query_row(
                "SELECT project_id, team_id FROM tasks WHERE id = ?1",
                params![parent_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| AppError::Msg("父任务不存在".into()))?;
        if parent.0 != project_id {
            return Err("子任务必须与父任务属于同一项目".into());
        }
        if team_id.is_none() {
            team_id = parent.1;
        }
    }
    let mandate = if mandate.as_deref() == Some("auto") {
        "auto"
    } else {
        "ask"
    };
    // 隔离工作区只在任务创建时创建一次；失败直接报错，不落任务，避免静默写入项目目录。
    let work_dir = if isolated.unwrap_or(false) {
        Some(crate::worktree::create_detached_worktree(
            std::path::Path::new(&project_path),
            &state.base_dir,
            &id,
        )?)
    } else {
        None
    };
    conn.execute(
        "INSERT INTO tasks (id, project_id, team_id, title, description, acceptance, model, reasoning, mandate, work_dir, priority, parent_id, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'inbox', ?13)",
        params![
            id,
            project_id,
            team_id,
            title.trim(),
            description,
            acceptance,
            model,
            reasoning,
            mandate,
            work_dir,
            priority.unwrap_or(2),
            parent_id,
            now_ms()
        ],
    )?;
    log_event(
        &conn,
        Some(&project_id),
        Some(&id),
        None,
        "task.created",
        Some("user"),
        json!({ "title": title, "parentId": parent_id }),
    )?;
    Ok(id)
}

/// 子任务列表：任务详情页的协作件区域。
#[tauri::command]
pub fn task_children(state: State<'_, AppState>, task_id: String) -> Result<Vec<Task>> {
    let conn = conn(&state)?;
    let mut stmt = conn.prepare(&format!(
        "{TASK_SELECT} WHERE t.parent_id = ?1 ORDER BY t.created_at ASC"
    ))?;
    let rows = stmt.query_map(params![task_id], map_task_row)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// 依赖检查：存在未完成的依赖任务时不允许启动，避免产出建立在缺失前置上。
fn ensure_dependencies_done(conn: &Connection, task_id: &str) -> Result<()> {
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM task_dependencies d
         JOIN tasks t ON t.id = d.depends_on_id
         WHERE d.task_id = ?1 AND t.status <> 'completed'",
        params![task_id],
        |r| r.get(0),
    )?;
    if pending > 0 {
        return Err(format!("还有 {pending} 个依赖任务未完成，完成后才能运行").into());
    }
    Ok(())
}

#[tauri::command]
pub fn task_dependencies(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<TaskDependency>> {
    let conn = conn(&state)?;
    let mut stmt = conn.prepare(
        "SELECT d.task_id, d.depends_on_id, t.title, t.status
         FROM task_dependencies d JOIN tasks t ON t.id = d.depends_on_id
         WHERE d.task_id = ?1 ORDER BY t.created_at ASC",
    )?;
    let rows = stmt.query_map(params![task_id], |r| {
        Ok(TaskDependency {
            task_id: r.get(0)?,
            depends_on_id: r.get(1)?,
            title: r.get(2)?,
            status: r.get(3)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn task_dependency_add(
    state: State<'_, AppState>,
    task_id: String,
    depends_on_id: String,
) -> Result<()> {
    if task_id == depends_on_id {
        return Err("任务不能依赖自身".into());
    }
    let conn = conn(&state)?;
    let project_id: String = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    let dep_project: String = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![depends_on_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("依赖任务不存在".into()))?;
    if project_id != dep_project {
        return Err("只能依赖同一项目下的任务".into());
    }
    // 沿依赖链检查 depends_on 是否已能回到当前任务，避免形成环。
    let cycle: i64 = conn.query_row(
        "WITH RECURSIVE chain(id) AS (
           SELECT depends_on_id FROM task_dependencies WHERE task_id = ?2
           UNION
           SELECT d.depends_on_id FROM task_dependencies d JOIN chain c ON d.task_id = c.id
         )
         SELECT COUNT(*) FROM chain WHERE id = ?1",
        params![task_id, depends_on_id],
        |r| r.get(0),
    )?;
    if cycle > 0 {
        return Err("该依赖会形成循环".into());
    }
    conn.execute(
        "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id, created_at)
         VALUES (?1, ?2, ?3)",
        params![task_id, depends_on_id, now_ms()],
    )?;
    log_event(
        &conn,
        Some(&project_id),
        Some(&task_id),
        None,
        "task.dependency_added",
        Some("user"),
        json!({ "dependsOnId": depends_on_id }),
    )?;
    Ok(())
}

#[tauri::command]
pub fn task_dependency_remove(
    state: State<'_, AppState>,
    task_id: String,
    depends_on_id: String,
) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute(
        "DELETE FROM task_dependencies WHERE task_id = ?1 AND depends_on_id = ?2",
        params![task_id, depends_on_id],
    )?;
    let project_id: Option<String> = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .optional()?;
    log_event(
        &conn,
        project_id.as_deref(),
        Some(&task_id),
        None,
        "task.dependency_removed",
        Some("user"),
        json!({ "dependsOnId": depends_on_id }),
    )?;
    Ok(())
}

#[tauri::command]
pub fn task_checklist_get(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<ChecklistItem>> {
    let conn = conn(&state)?;
    let raw: String = conn
        .query_row(
            "SELECT COALESCE(acceptance_json, '[]') FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

#[tauri::command]
pub fn task_checklist_save(
    state: State<'_, AppState>,
    task_id: String,
    items: Vec<ChecklistItem>,
) -> Result<Vec<ChecklistItem>> {
    let conn = conn(&state)?;
    let mut cleaned = Vec::new();
    for item in items {
        let text = item.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        cleaned.push(ChecklistItem {
            id: if item.id.trim().is_empty() {
                new_id()
            } else {
                item.id
            },
            text,
            done: item.done,
        });
    }
    let raw = serde_json::to_string(&cleaned)
        .map_err(|e| AppError::Msg(format!("验收清单序列化失败：{e}")))?;
    let changed = conn.execute(
        "UPDATE tasks SET acceptance_json = ?1 WHERE id = ?2",
        params![raw, task_id],
    )?;
    if changed == 0 {
        return Err("任务不存在".into());
    }
    let project_id: Option<String> = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .optional()?;
    let done = cleaned.iter().filter(|item| item.done).count();
    log_event(
        &conn,
        project_id.as_deref(),
        Some(&task_id),
        None,
        "task.checklist_updated",
        Some("user"),
        json!({ "total": cleaned.len(), "done": done }),
    )?;
    Ok(cleaned)
}

#[tauri::command]
pub fn task_attachments(state: State<'_, AppState>, task_id: String) -> Result<Vec<Attachment>> {
    let conn = conn(&state)?;
    let mut stmt = conn.prepare(
        "SELECT id, task_id, name, path, size, created_at FROM attachments
         WHERE task_id = ?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![task_id], |r| {
        Ok(Attachment {
            id: r.get(0)?,
            task_id: r.get(1)?,
            name: r.get(2)?,
            path: r.get(3)?,
            size: r.get(4)?,
            created_at: r.get(5)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

/// 复制本地文件到数据目录，避免运行期外部文件被移动导致附件失效。
#[tauri::command]
pub fn task_attachment_add(
    state: State<'_, AppState>,
    task_id: String,
    source_path: String,
) -> Result<Attachment> {
    let src = std::path::PathBuf::from(source_path.trim());
    if !src.is_file() {
        return Err("文件不存在或不是普通文件".into());
    }
    let conn = conn(&state)?;
    let project_id: String = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    let size = std::fs::metadata(&src).map(|m| m.len() as i64).unwrap_or(0);
    if size > 50 * 1024 * 1024 {
        return Err("单个附件不能超过 50 MB".into());
    }
    let name = src
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("attachment")
        .to_string();
    let dir = state.base_dir.join("attachments").join(&task_id);
    std::fs::create_dir_all(&dir)?;
    let id = new_id();
    let dest = dir.join(format!("{}_{}", id, safe_file_name(&name)));
    std::fs::copy(&src, &dest)?;
    let attachment = Attachment {
        id,
        task_id: task_id.clone(),
        name,
        path: dest.to_string_lossy().to_string(),
        size,
        created_at: now_ms(),
    };
    conn.execute(
        "INSERT INTO attachments (id, task_id, name, path, size, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            attachment.id,
            attachment.task_id,
            attachment.name,
            attachment.path,
            attachment.size,
            attachment.created_at
        ],
    )?;
    log_event(
        &conn,
        Some(&project_id),
        Some(&task_id),
        None,
        "task.attachment_added",
        Some("user"),
        json!({ "name": attachment.name, "size": size }),
    )?;
    Ok(attachment)
}

#[tauri::command]
pub fn task_attachment_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    let row: Option<(String, String, String)> = conn
        .query_row(
            "SELECT task_id, path, name FROM attachments WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((task_id, path, name)) = row else {
        return Ok(());
    };
    conn.execute("DELETE FROM attachments WHERE id = ?1", params![id])?;
    let _ = std::fs::remove_file(&path);
    let project_id: Option<String> = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .optional()?;
    log_event(
        &conn,
        project_id.as_deref(),
        Some(&task_id),
        None,
        "task.attachment_removed",
        Some("user"),
        json!({ "name": name }),
    )?;
    Ok(())
}

#[tauri::command]
pub fn task_attachment_reveal(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    let path: String = conn
        .query_row(
            "SELECT path FROM attachments WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("附件不存在".into()))?;
    #[cfg(windows)]
    std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn()
        .map_err(|e| AppError::Msg(format!("打开附件位置失败：{e}")))?;
    #[cfg(not(windows))]
    {
        let _ = path;
    }
    Ok(())
}

#[tauri::command]
pub fn extensions_health(
    state: State<'_, AppState>,
) -> Result<Vec<crate::extensions::ExtensionHealth>> {
    let conn = conn(&state)?;
    crate::extensions::health(&conn)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub acceptance: Option<String>,
    pub priority: Option<i64>,
    pub status: Option<String>,
    pub team_id: Option<String>,
}

#[tauri::command]
pub fn task_update(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    patch: TaskPatch,
) -> Result<()> {
    if let Some(status) = patch.status.as_deref() {
        let allowed = TaskStatus::parse(status)
            .map(|value| TaskStatus::manual_targets().contains(&value))
            .unwrap_or(false);
        if !allowed {
            return Err(format!("不允许手动把任务状态改为「{status}」").into());
        }
    }
    let conn = conn(&state)?;
    let (title, description, acceptance, priority, status): (
        String,
        Option<String>,
        Option<String>,
        i64,
        String,
    ) = conn.query_row(
        "SELECT title, description, acceptance, priority, status FROM tasks WHERE id = ?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    let status_changed = patch.status.is_some();
    let next_status = patch.status.clone().unwrap_or_else(|| status.clone());
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE tasks SET title = ?1, description = ?2, acceptance = ?3, priority = ?4,
         team_id = COALESCE(?5, team_id) WHERE id = ?6",
        params![
            patch.title.unwrap_or(title),
            patch.description.or(description),
            patch.acceptance.or(acceptance),
            patch.priority.unwrap_or(priority),
            patch.team_id,
            id
        ],
    )?;
    // 手动置为终态/收件箱后，旧审批不得继续挂在待处理列表里。
    let expired = match patch.status.as_deref() {
        Some(status) => {
            let next = TaskStatus::parse(status)
                .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
            task_lifecycle::apply(
                &tx,
                &id,
                TaskTransition::Manual { status: next },
                "任务状态已手动变更",
            )?
            .expired
        }
        None => 0,
    };
    tx.commit()?;
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": id, "reason": "manual_status" }),
        );
    }
    if status_changed {
        let _ = app.emit(
            "task.updated",
            json!({ "taskId": id, "status": next_status }),
        );
    }
    Ok(())
}

#[tauri::command]
pub fn task_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    let worktree: Option<(String, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT t.status, t.work_dir, p.path FROM tasks t
               LEFT JOIN projects p ON p.id = t.project_id WHERE t.id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((status, _, _)) = &worktree {
        if TaskStatus::parse(status)
            .map(TaskStatus::is_active)
            .unwrap_or(false)
        {
            return Err("任务正在运行，请先停止后再删除".into());
        }
    }
    let attachment_paths: Vec<String> = {
        let mut stmt = conn.prepare("SELECT path FROM attachments WHERE task_id = ?1")?;
        let rows: Vec<String> = stmt
            .query_map(params![id], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };
    let tx = conn.unchecked_transaction()?;
    // 子任务保留但解除父子关系，避免删除父任务时孤立不可见。
    tx.execute(
        "UPDATE tasks SET parent_id = NULL WHERE parent_id = ?1",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM task_dependencies WHERE task_id = ?1 OR depends_on_id = ?1",
        params![id],
    )?;
    tx.execute("DELETE FROM attachments WHERE task_id = ?1", params![id])?;
    tx.execute("DELETE FROM messages WHERE task_id = ?1", params![id])?;
    tx.execute("DELETE FROM approvals WHERE task_id = ?1", params![id])?;
    tx.execute(
        "DELETE FROM events WHERE task_id = ?1
           OR run_id IN (SELECT id FROM runs WHERE task_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM artifacts WHERE task_id = ?1
           OR run_id IN (SELECT id FROM runs WHERE task_id = ?1)",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM budget_reservations WHERE run_id IN (SELECT id FROM runs WHERE task_id = ?1)",
        params![id],
    )?;
    tx.execute("DELETE FROM usage_events WHERE task_id = ?1", params![id])?;
    tx.execute(
        "DELETE FROM budgets WHERE scope_type = 'task' AND scope_id = ?1",
        params![id],
    )?;
    tx.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
    tx.commit()?;
    for path in attachment_paths {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_dir_all(state.base_dir.join("attachments").join(&id));
    if let Some((_, work_dir, project_path)) = worktree {
        crate::review::cleanup_worktree_best_effort(
            &state,
            work_dir.as_deref(),
            project_path.as_deref(),
        );
    }
    Ok(())
}

#[tauri::command]
pub fn task_dispatch(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<String>> {
    let settings = state.settings();
    if state.registry.active_count() >= settings.max_concurrent_runs.max(1) as usize {
        return Err("并发运行已达上限，请等待当前任务结束或调整设置".into());
    }
    let (status, team_id, stage_index, project_id, work_dir, project_path): (
        String,
        Option<String>,
        i64,
        String,
        Option<String>,
        Option<String>,
    ) = {
        let conn = conn(&state)?;
        conn.query_row(
            "SELECT t.status, t.team_id, t.stage_index, t.project_id, t.work_dir, p.path
               FROM tasks t LEFT JOIN projects p ON p.id = t.project_id WHERE t.id = ?1",
            params![task_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )?
    };
    {
        let conn = conn(&state)?;
        ensure_dependencies_done(&conn, &task_id)?;
    }
    let status_kind = TaskStatus::parse(&status)
        .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
    if !task_lifecycle::permits(status_kind, TaskTransition::Dispatch { start_stage: 0 }) {
        return Err("任务正在运行或等待审批，不能重复派发".into());
    }
    let Some(_) = team_id else {
        return Err("请先为任务选择团队".into());
    };
    // 非隔离运行直接改项目目录：Git 项目先留一份只读补丁快照，非 Git 项目由前端明确警告。
    let snapshot = if work_dir.is_none() {
        match project_path.as_deref() {
            Some(path) if !path.trim().is_empty() => {
                match crate::snapshot::create_pre_run_snapshot(
                    std::path::Path::new(path),
                    &state.base_dir,
                    &task_id,
                ) {
                    Ok(path) => path,
                    Err(err) => {
                        tracing::warn!("任务 {task_id} 的运行前快照生成失败：{err}");
                        None
                    }
                }
            }
            _ => None,
        }
    } else {
        None
    };
    let start = if status == "completed" {
        0
    } else {
        stage_index
    };
    {
        let conn = conn(&state)?;
        let outcome = task_lifecycle::apply(
            &conn,
            &task_id,
            TaskTransition::Dispatch { start_stage: start },
            "任务重新派发",
        )?;
        if !outcome.changed {
            return Err("任务状态已变化，派发已取消".into());
        }
        if outcome.expired > 0 {
            let _ = app.emit(
                "approval.changed",
                json!({ "taskId": task_id, "reason": "redispatch" }),
            );
        }
        log_event(
            &conn,
            Some(&project_id),
            Some(&task_id),
            None,
            "task.dispatched",
            Some("user"),
            json!({ "stageIndex": start, "snapshot": snapshot.clone() }),
        )?;
        if let Some(path) = &snapshot {
            log_event(
                &conn,
                Some(&project_id),
                Some(&task_id),
                None,
                "task.snapshot",
                Some("system"),
                json!({ "path": path, "isolated": false }),
            )?;
        }
    }
    let deps = PipelineDeps::from_state(&app, &state);
    orchestrator::spawn_pipeline(deps, task_id, start);
    Ok(snapshot)
}

#[tauri::command]
pub fn task_cancel(app: AppHandle, state: State<'_, AppState>, task_id: String) -> Result<()> {
    let conn = conn(&state)?;
    let mut stmt = conn
        .prepare("SELECT id FROM runs WHERE task_id = ?1 AND status IN ('starting', 'running')")?;
    let run_ids: Vec<String> = stmt
        .query_map(params![task_id], |r| r.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    for run_id in &run_ids {
        state.registry.cancel(run_id);
    }
    let tx = conn.unchecked_transaction()?;
    let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Cancel, "任务被取消")?;
    let expired = outcome.expired;
    tx.execute(
        "UPDATE runs SET status = 'cancelled', ended_at = ?1
         WHERE task_id = ?2 AND status IN ('starting', 'running')",
        params![now_ms(), task_id],
    )?;
    let project_id: Option<String> = tx
        .query_row(
            "SELECT project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .ok();
    log_event(
        &tx,
        project_id.as_deref(),
        Some(&task_id),
        None,
        "task.cancelled",
        Some("user"),
        json!({ "runs": run_ids }),
    )?;
    tx.commit()?;
    // 停止运行后隔离区可能留有改动，立即刷新待审查标记。
    if let Err(err) = crate::review::refresh_review_flag(&conn, &task_id) {
        tracing::warn!("刷新待审查标记失败：{err}");
    }
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "blocked" }),
    );
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": task_id, "reason": "cancelled" }),
        );
    }
    Ok(())
}

/// 非隔离运行会直接改项目目录，重新运行前留一份只读补丁快照。
fn pre_run_snapshot(
    state: &State<'_, AppState>,
    task_id: &str,
    work_dir: Option<&str>,
    project_path: Option<&str>,
) -> Option<String> {
    if work_dir
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return None;
    }
    let path = project_path.filter(|value| !value.trim().is_empty())?;
    match crate::snapshot::create_pre_run_snapshot(
        std::path::Path::new(path),
        &state.base_dir,
        task_id,
    ) {
        Ok(snapshot) => snapshot,
        Err(err) => {
            tracing::warn!("任务 {task_id} 的运行前快照生成失败：{err}");
            None
        }
    }
}

type ActiveTaskRow = (
    String,
    Option<String>,
    i64,
    String,
    Option<String>,
    Option<String>,
);

fn active_task_row(conn: &Connection, task_id: &str) -> Result<ActiveTaskRow> {
    conn.query_row(
        "SELECT t.status, t.team_id, t.stage_index, t.project_id, t.work_dir, p.path
           FROM tasks t LEFT JOIN projects p ON p.id = t.project_id WHERE t.id = ?1",
        params![task_id],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )
    .map_err(|_| AppError::Msg("任务不存在".into()))
}

/// 立即暂停：终止当前阶段子进程并标记中断，任务进入 paused；继续时从当前阶段重跑。
#[tauri::command]
pub fn task_pause(app: AppHandle, state: State<'_, AppState>, task_id: String) -> Result<()> {
    let conn = conn(&state)?;
    let (status, project_id): (String, String) = conn
        .query_row(
            "SELECT status, project_id FROM tasks WHERE id = ?1",
            params![task_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    let status_kind = TaskStatus::parse(&status)
        .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
    if !task_lifecycle::permits(status_kind, TaskTransition::Pause) {
        return Err("只有运行中或等待审批的任务可以暂停".into());
    }
    // 先终止该任务的进程树；paused 状态会阻止流水线继续推进下一阶段。
    let cancelled = state.registry.cancel_task(&task_id);
    let tx = conn.unchecked_transaction()?;
    let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Pause, "任务已暂停")?;
    if !outcome.changed {
        return Err("任务状态已变化，暂停已取消".into());
    }
    // 只结束仍在进行的运行；已自然完成的运行保留真实结果。
    tx.execute(
        "UPDATE runs SET status = 'cancelled', error = COALESCE(error, '用户暂停'), ended_at = ?1
         WHERE task_id = ?2 AND status IN ('starting', 'running')",
        params![now_ms(), task_id],
    )?;
    let expired = outcome.expired;
    log_event(
        &tx,
        Some(&project_id),
        Some(&task_id),
        None,
        "task.paused",
        Some("user"),
        json!({ "runs": cancelled }),
    )?;
    tx.commit()?;
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "paused" }),
    );
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": task_id, "reason": "paused" }),
        );
    }
    Ok(())
}

/// 从暂停处继续：沿用 stage_index 从当前阶段重跑，已完成的阶段不重复执行。
#[tauri::command]
pub fn task_resume(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<String>> {
    let settings = state.settings();
    if state.registry.active_count() >= settings.max_concurrent_runs.max(1) as usize {
        return Err("并发运行已达上限，请等待当前任务结束或调整设置".into());
    }
    let db_conn = conn(&state)?;
    let (status, team_id, stage_index, project_id, work_dir, project_path) =
        active_task_row(&db_conn, &task_id)?;
    let status_kind = TaskStatus::parse(&status)
        .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
    if !task_lifecycle::permits(status_kind, TaskTransition::Resume) {
        return Err("只有已暂停的任务可以继续".into());
    }
    if team_id.is_none() {
        return Err("请先为任务选择团队".into());
    }
    ensure_dependencies_done(&db_conn, &task_id)?;
    drop(db_conn);
    let snapshot = pre_run_snapshot(
        &state,
        &task_id,
        work_dir.as_deref(),
        project_path.as_deref(),
    );
    let conn = conn(&state)?;
    let tx = conn.unchecked_transaction()?;
    let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Resume, "任务继续")?;
    if !outcome.changed {
        return Err("任务状态已变化，继续已取消".into());
    }
    let expired = outcome.expired;
    log_event(
        &tx,
        Some(&project_id),
        Some(&task_id),
        None,
        "task.resumed",
        Some("user"),
        json!({ "stageIndex": stage_index, "snapshot": snapshot.clone() }),
    )?;
    if let Some(path) = &snapshot {
        log_event(
            &tx,
            Some(&project_id),
            Some(&task_id),
            None,
            "task.snapshot",
            Some("system"),
            json!({ "path": path, "isolated": false }),
        )?;
    }
    tx.commit()?;
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "queued" }),
    );
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": task_id, "reason": "resumed" }),
        );
    }
    let deps = PipelineDeps::from_state(&app, &state);
    orchestrator::spawn_pipeline(deps, task_id, stage_index.max(0));
    Ok(snapshot)
}

/// 改派当前阶段的席位 Agent，并从当前阶段重跑；支持暂停中与阻塞中的任务。
#[tauri::command]
pub fn task_reassign(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    agent_id: String,
) -> Result<Option<String>> {
    let settings = state.settings();
    if state.registry.active_count() >= settings.max_concurrent_runs.max(1) as usize {
        return Err("并发运行已达上限，请等待当前任务结束或调整设置".into());
    }
    let db_conn = conn(&state)?;
    let (status, team_id, stage_index, project_id, work_dir, project_path) =
        active_task_row(&db_conn, &task_id)?;
    let status_kind = TaskStatus::parse(&status)
        .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
    if !task_lifecycle::permits(status_kind, TaskTransition::Reassign) {
        return Err("只有暂停或阻塞的任务可以改派后重跑".into());
    }
    let Some(team_id) = team_id else {
        return Err("请先为任务选择团队".into());
    };
    let agent_name: String = db_conn
        .query_row(
            "SELECT name FROM agents WHERE id = ?1",
            params![agent_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("Agent 不存在".into()))?;
    let seat: Option<(String, String)> = db_conn
        .query_row(
            "SELECT id, role_name FROM seats WHERE team_id = ?1
             ORDER BY order_index ASC LIMIT 1 OFFSET ?2",
            params![team_id, stage_index.max(0)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((seat_id, role_name)) = seat else {
        return Err("当前阶段没有对应席位，请先在团队页调整席位".into());
    };
    ensure_dependencies_done(&db_conn, &task_id)?;
    drop(db_conn);
    let snapshot = pre_run_snapshot(
        &state,
        &task_id,
        work_dir.as_deref(),
        project_path.as_deref(),
    );
    let conn = conn(&state)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE seats SET agent_id = ?1 WHERE id = ?2",
        params![agent_id, seat_id],
    )?;
    let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Reassign, "任务改派")?;
    if !outcome.changed {
        return Err("任务状态已变化，改派已取消".into());
    }
    let expired = outcome.expired;
    log_event(
        &tx,
        Some(&project_id),
        Some(&task_id),
        None,
        "task.reassigned",
        Some("user"),
        json!({
            "stageIndex": stage_index,
            "seatId": seat_id,
            "seatName": role_name,
            "agentId": agent_id,
            "agentName": agent_name,
            "snapshot": snapshot.clone()
        }),
    )?;
    if let Some(path) = &snapshot {
        log_event(
            &tx,
            Some(&project_id),
            Some(&task_id),
            None,
            "task.snapshot",
            Some("system"),
            json!({ "path": path, "isolated": false }),
        )?;
    }
    tx.commit()?;
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "queued" }),
    );
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": task_id, "reason": "reassigned" }),
        );
    }
    let deps = PipelineDeps::from_state(&app, &state);
    orchestrator::spawn_pipeline(deps, task_id, stage_index.max(0));
    Ok(snapshot)
}

#[tauri::command]
pub fn task_reset(app: AppHandle, state: State<'_, AppState>, task_id: String) -> Result<()> {
    let conn = conn(&state)?;
    let status: String = conn
        .query_row(
            "SELECT status FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    let status_kind = TaskStatus::parse(&status)
        .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
    if !task_lifecycle::permits(status_kind, TaskTransition::Reset) {
        return Err("任务正在运行，请先取消后再重置".into());
    }
    let tx = conn.unchecked_transaction()?;
    let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Reset, "任务已重置")?;
    if !outcome.changed {
        return Err("任务状态已变化，请刷新后重试".into());
    }
    let expired = outcome.expired;
    tx.commit()?;
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "inbox" }),
    );
    if expired > 0 {
        let _ = app.emit(
            "approval.changed",
            json!({ "taskId": task_id, "reason": "reset" }),
        );
    }
    Ok(())
}

/// 从当前阶段重试：仅清理挂起审批并沿用 stage_index，不再整体回到收件箱。
#[tauri::command]
pub fn task_retry_stage(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<String>> {
    let settings = state.settings();
    if state.registry.active_count() >= settings.max_concurrent_runs.max(1) as usize {
        return Err("并发运行已达上限，请等待当前任务结束或调整设置".into());
    }
    let (status, team_id, stage_index, project_id, work_dir, project_path): (
        String,
        Option<String>,
        i64,
        String,
        Option<String>,
        Option<String>,
    ) = {
        let conn = conn(&state)?;
        conn.query_row(
            "SELECT t.status, t.team_id, t.stage_index, t.project_id, t.work_dir, p.path
               FROM tasks t LEFT JOIN projects p ON p.id = t.project_id WHERE t.id = ?1",
            params![task_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )?
    };
    let status_kind = TaskStatus::parse(&status)
        .ok_or_else(|| AppError::Msg(format!("未知任务状态「{status}」")))?;
    if !task_lifecycle::permits(status_kind, TaskTransition::Retry) {
        return Err("只有已阻塞的任务可以从当前阶段重试".into());
    }
    {
        let conn = conn(&state)?;
        ensure_dependencies_done(&conn, &task_id)?;
    }
    if team_id.is_none() {
        return Err("请先为任务选择团队".into());
    }
    // 重试同样是一次真实运行：非隔离时也留一份运行前快照。
    let snapshot = if work_dir.is_none() {
        match project_path.as_deref() {
            Some(path) if !path.trim().is_empty() => {
                match crate::snapshot::create_pre_run_snapshot(
                    std::path::Path::new(path),
                    &state.base_dir,
                    &task_id,
                ) {
                    Ok(path) => path,
                    Err(err) => {
                        tracing::warn!("任务 {task_id} 的重试前快照生成失败：{err}");
                        None
                    }
                }
            }
            _ => None,
        }
    } else {
        None
    };
    {
        let conn = conn(&state)?;
        let tx = conn.unchecked_transaction()?;
        let outcome = task_lifecycle::apply(&tx, &task_id, TaskTransition::Retry, "任务重试阶段")?;
        if !outcome.changed {
            return Err("任务状态已变化，重试已取消".into());
        }
        let expired = outcome.expired;
        log_event(
            &tx,
            Some(&project_id),
            Some(&task_id),
            None,
            "task.retried",
            Some("user"),
            json!({ "stageIndex": stage_index, "snapshot": snapshot.clone() }),
        )?;
        if let Some(path) = &snapshot {
            log_event(
                &tx,
                Some(&project_id),
                Some(&task_id),
                None,
                "task.snapshot",
                Some("system"),
                json!({ "path": path, "isolated": false }),
            )?;
        }
        tx.commit()?;
        if expired > 0 {
            let _ = app.emit(
                "approval.changed",
                json!({ "taskId": task_id, "reason": "retry" }),
            );
        }
    }
    let _ = app.emit(
        "task.updated",
        json!({ "taskId": task_id, "status": "queued" }),
    );
    let deps = PipelineDeps::from_state(&app, &state);
    orchestrator::spawn_pipeline(deps, task_id, stage_index);
    Ok(snapshot)
}

#[tauri::command]
pub fn task_add_note(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    content: String,
) -> Result<()> {
    if content.trim().is_empty() {
        return Err("内容不能为空".into());
    }
    let conn = conn(&state)?;
    let status: String = conn
        .query_row(
            "SELECT status FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    // 运行中的追加内容不会打断当前 Agent，而是写入时间线供下一阶段或重试时使用。
    let during_run = TaskStatus::parse(&status)
        .map(TaskStatus::is_active)
        .unwrap_or(false);
    let id = insert_message(
        &conn,
        Some(&task_id),
        None,
        None,
        None,
        "chat",
        Some("user"),
        content.trim(),
        json!({ "source": "user", "duringRun": during_run }),
    )?;
    let _ = app.emit(
        "run.delta",
        json!({
            "taskId": task_id,
            "kind": "message",
            "message": {
                "id": id,
                "taskId": task_id,
                "kind": "chat",
                "role": "user",
                "content": content.trim(),
                "createdAt": now_ms()
            }
        }),
    );
    Ok(())
}

#[tauri::command]
pub fn task_messages(
    state: State<'_, AppState>,
    task_id: String,
    before: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<Message>> {
    let conn = conn(&state)?;
    let lim = limit.unwrap_or(200).clamp(1, 500);
    // 倒序取最近 N 条再回正，避免超过上限后新消息完全不可见；before 作为游标加载更早。
    let (sql, params_vec): (String, Vec<SqlValue>) = match before {
        Some(cursor) => (
            format!(
                "{MESSAGE_SQL} WHERE m.task_id = ?1 AND m.created_at < ?2 ORDER BY m.created_at DESC, m.rowid DESC LIMIT ?3"
            ),
            vec![task_id.into(), cursor.into(), lim.into()],
        ),
        None => (
            format!("{MESSAGE_SQL} WHERE m.task_id = ?1 ORDER BY m.created_at DESC, m.rowid DESC LIMIT ?2"),
            vec![task_id.into(), lim.into()],
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params_vec), map_message)?;
    let mut items: Vec<Message> = rows.filter_map(|r| r.ok()).collect();
    items.reverse();
    Ok(items)
}

#[tauri::command]
pub fn tool_calls_list(
    state: State<'_, AppState>,
    task_id: String,
    limit: Option<i64>,
) -> Result<Vec<ToolCall>> {
    let conn = conn(&state)?;
    let lim = limit.unwrap_or(200).clamp(1, 500);
    let mut stmt = conn.prepare(
        "SELECT t.id, t.run_id, t.tool_name, t.input_json, t.output_json, t.status,
                t.risk_level, t.created_at, t.finished_at
         FROM tool_calls t JOIN runs r ON r.id = t.run_id
         WHERE r.task_id = ?1
         ORDER BY t.created_at DESC, t.rowid DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![task_id, lim], |r| {
        let input: String = r.get(3)?;
        let output: Option<String> = r.get(4)?;
        Ok(ToolCall {
            id: r.get(0)?,
            run_id: r.get(1)?,
            tool_name: r.get(2)?,
            input: serde_json::from_str(&input).unwrap_or(Value::Null),
            output: output.and_then(|text| serde_json::from_str(&text).ok()),
            status: r.get(5)?,
            risk_level: r.get(6)?,
            created_at: r.get(7)?,
            finished_at: r.get(8)?,
        })
    })?;
    let mut items: Vec<ToolCall> = rows.filter_map(|r| r.ok()).collect();
    items.reverse();
    Ok(items)
}

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

#[tauri::command]
pub fn usage_summary(state: State<'_, AppState>) -> Result<UsageSummary> {
    let conn = conn(&state)?;
    usage::summary(&conn)
}

#[tauri::command]
pub fn usage_series(state: State<'_, AppState>, days: Option<i64>) -> Result<Vec<UsagePoint>> {
    let conn = conn(&state)?;
    usage::series(&conn, days.unwrap_or(7).clamp(1, 90))
}

#[tauri::command]
pub fn usage_by_agent(state: State<'_, AppState>) -> Result<Vec<ScopedUsage>> {
    let conn = conn(&state)?;
    usage::by_agent(&conn)
}

#[tauri::command]
pub fn usage_by_project(state: State<'_, AppState>) -> Result<Vec<ScopedUsage>> {
    let conn = conn(&state)?;
    usage::by_project(&conn)
}

#[tauri::command]
pub fn usage_by_task(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<Vec<ScopedUsage>> {
    let conn = conn(&state)?;
    let base = "SELECT t.id, t.title,
            COALESCE(SUM(u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens), 0),
            COALESCE(SUM(u.cost_micros), 0),
            COALESCE(COUNT(u.id), 0)
         FROM tasks t LEFT JOIN usage_events u ON u.task_id = t.id";
    let sql = if project_id.is_some() {
        format!("{base} WHERE t.project_id = ?1 GROUP BY t.id ORDER BY 3 DESC LIMIT 100")
    } else {
        format!("{base} GROUP BY t.id ORDER BY 3 DESC LIMIT 100")
    };
    let mut stmt = conn.prepare(&sql)?;
    let mapper = |r: &rusqlite::Row| -> rusqlite::Result<ScopedUsage> {
        Ok(ScopedUsage {
            key: r.get(0)?,
            name: r.get(1)?,
            tokens: r.get(2)?,
            cost_micros: r.get(3)?,
            rounds: r.get(4)?,
        })
    };
    let items: Vec<ScopedUsage> = if let Some(pid) = project_id {
        stmt.query_map(params![pid], mapper)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map([], mapper)?.filter_map(|r| r.ok()).collect()
    };
    Ok(items)
}

#[tauri::command]
pub fn budgets_list(state: State<'_, AppState>) -> Result<Vec<Budget>> {
    let conn = conn(&state)?;
    crate::budget::list(&conn)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn budget_upsert(
    state: State<'_, AppState>,
    id: Option<String>,
    scope_type: String,
    scope_id: Option<String>,
    unit: String,
    period: String,
    limit_value: i64,
    warn_pct: Option<i64>,
    hard_stop: bool,
    enabled: Option<bool>,
) -> Result<String> {
    if limit_value <= 0 {
        return Err("预算上限必须大于 0".into());
    }
    if scope_type != "global" && scope_id.as_deref().unwrap_or_default().is_empty() {
        return Err("请选择预算作用对象".into());
    }
    let conn = conn(&state)?;
    crate::budget::upsert(
        &conn,
        id,
        &scope_type,
        scope_id,
        &unit,
        &period,
        limit_value,
        warn_pct.unwrap_or(80).clamp(1, 100),
        hard_stop,
        enabled,
    )
}

#[tauri::command]
pub fn budget_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    crate::budget::delete(&conn, &id)
}

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

#[tauri::command]
pub fn events_list(
    state: State<'_, AppState>,
    project_id: Option<String>,
    task_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<EventItem>> {
    let conn = conn(&state)?;
    let mut sql = String::from(
        "SELECT id, seq, project_id, task_id, run_id, type, actor, payload_json, created_at FROM events WHERE 1=1",
    );
    let mut values: Vec<SqlValue> = Vec::new();
    if let Some(pid) = project_id {
        sql.push_str(" AND project_id = ?");
        values.push(pid.into());
    }
    if let Some(tid) = task_id {
        sql.push_str(" AND task_id = ?");
        values.push(tid.into());
    }
    sql.push_str(" ORDER BY seq DESC LIMIT ?");
    values.push(limit.unwrap_or(100).clamp(1, 500).into());
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values), |r| {
        let payload: String = r.get(7)?;
        Ok(EventItem {
            id: r.get(0)?,
            seq: r.get(1)?,
            project_id: r.get(2)?,
            task_id: r.get(3)?,
            run_id: r.get(4)?,
            r#type: r.get(5)?,
            actor: r.get(6)?,
            payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
            created_at: r.get(8)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn search_history(
    state: State<'_, AppState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<Message>> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let conn = conn(&state)?;
    let sql = format!("{MESSAGE_SQL} WHERE m.content LIKE ?1 ORDER BY m.created_at DESC LIMIT ?2");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![format!("%{q}%"), limit.unwrap_or(80).clamp(1, 300)],
        map_message,
    )?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn dashboard_summary(state: State<'_, AppState>) -> Result<DashboardSummary> {
    let conn = conn(&state)?;
    let usage = usage::summary(&conn)?;
    let pending_approvals = approval::pending_count(&conn)?;
    let pending_reviews: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE review_pending = 1",
        [],
        |r| r.get(0),
    )?;
    let agents = usage::agent_stats(&conn)?;
    let series = usage::series(&conn, 7)?;
    let mut teams = teams_query(&conn, None)?;
    teams.truncate(5);
    Ok(DashboardSummary {
        usage,
        pending_approvals,
        pending_reviews,
        agents,
        teams,
        series,
    })
}

#[tauri::command]
pub fn providers_list(state: State<'_, AppState>) -> Result<Vec<Provider>> {
    let conn = conn(&state)?;
    crate::providers::list(&conn)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn provider_upsert(
    state: State<'_, AppState>,
    id: Option<String>,
    name: String,
    protocol: String,
    base_url: String,
    model: Option<String>,
    models: Option<Vec<String>>,
    api_key: Option<String>,
    enabled: bool,
) -> Result<String> {
    let conn = conn(&state)?;
    crate::providers::upsert(
        &conn, id, name, protocol, base_url, model, models, api_key, enabled,
    )
}

#[tauri::command]
pub async fn provider_models(
    state: State<'_, AppState>,
    id: Option<String>,
    protocol: String,
    base_url: String,
    api_key: Option<String>,
) -> Result<Vec<String>> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::providers::fetch_models(&db, id, protocol, base_url, api_key)
    })
    .await
    .map_err(|e| AppError::Msg(e.to_string()))?
}

#[tauri::command]
pub fn provider_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    crate::providers::delete(&conn, &id)
}

#[tauri::command]
pub async fn provider_test(state: State<'_, AppState>, id: String) -> Result<ProviderTestResult> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || crate::providers::test(&db, &id))
        .await
        .map_err(|e| AppError::Msg(e.to_string()))?
}

#[tauri::command]
pub fn data_export(state: State<'_, AppState>) -> Result<String> {
    let conn = conn(&state)?;
    let dir = state.base_dir.join("exports");
    std::fs::create_dir_all(&dir)?;
    let tables = [
        "projects",
        "teams",
        "seats",
        "agents",
        "tasks",
        "messages",
        "runs",
        "tool_calls",
        "events",
        "usage_events",
        "budgets",
        "budget_reservations",
        "approvals",
        "approval_rules",
        "artifacts",
        "providers",
        "extensions",
    ];
    let mut out = serde_json::Map::new();
    for table in tables {
        out.insert(table.to_string(), table_rows(&conn, table)?);
    }
    let path = dir.join(format!("lumen-export-{}.json", now_ms()));
    std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(out))?)?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn data_backup(state: State<'_, AppState>) -> Result<String> {
    let conn = conn(&state)?;
    crate::backup::create(&state.base_dir, &conn)
}

#[tauri::command]
pub fn backups_list(state: State<'_, AppState>) -> Result<Vec<crate::backup::BackupFile>> {
    crate::backup::list(&state.base_dir)
}

#[tauri::command]
pub fn data_restore(state: State<'_, AppState>, name: String) -> Result<()> {
    if state.registry.active_count() > 0 {
        return Err("仍有运行进行中，请先停止后再恢复备份".into());
    }
    crate::backup::restore(&state.base_dir, &state.db.path, &name)?;
    // 恢复等同于换库：补齐迁移并清理可能残留的运行中状态。
    state.db.init()?;
    state.db.reconcile_on_startup()?;
    Ok(())
}

#[tauri::command]
pub fn open_backup_dir(state: State<'_, AppState>) -> Result<()> {
    let dir = state.base_dir.join("backups");
    std::fs::create_dir_all(&dir)?;
    std::process::Command::new("explorer")
        .arg(dir.as_os_str())
        .spawn()
        .map_err(|e| AppError::Msg(format!("打开备份目录失败：{e}")))?;
    Ok(())
}

fn table_rows(conn: &rusqlite::Connection, table: &str) -> Result<Value> {
    let mut stmt = conn.prepare(&format!("SELECT * FROM {table}"))?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mut rows = stmt.query([])?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        let mut obj = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(v) => json!(v),
                rusqlite::types::ValueRef::Real(v) => json!(v),
                rusqlite::types::ValueRef::Text(v) => {
                    let text = String::from_utf8_lossy(v).to_string();
                    // 导出时排除密钥：供应商密文直接置空，Agent 环境与扩展配置递归脱敏。
                    if table == "providers" && name == "key_cipher" {
                        Value::Null
                    } else if (table == "agents" && name == "env_json")
                        || (table == "extensions" && name == "config_json")
                    {
                        crate::backup::redact_json_text(&text)
                    } else {
                        Value::String(text)
                    }
                }
                rusqlite::types::ValueRef::Blob(_) => Value::String("<blob>".into()),
            };
            obj.insert(name.clone(), value);
        }
        items.push(Value::Object(obj));
    }
    Ok(Value::Array(items))
}

#[tauri::command]
pub fn extensions_list(state: State<'_, AppState>) -> Result<Vec<Extension>> {
    let conn = conn(&state)?;
    crate::extensions::list(&conn)
}

#[tauri::command]
pub fn extension_upsert(
    state: State<'_, AppState>,
    id: Option<String>,
    kind: String,
    name: String,
    description: Option<String>,
    config: Value,
    enabled: bool,
) -> Result<String> {
    let conn = conn(&state)?;
    crate::extensions::upsert(&conn, id, kind, name, description, config, enabled)
}

#[tauri::command]
pub fn extension_toggle(state: State<'_, AppState>, id: String, enabled: bool) -> Result<()> {
    let conn = conn(&state)?;
    crate::extensions::toggle(&conn, &id, enabled)
}

#[tauri::command]
pub fn extension_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    crate::extensions::delete(&conn, &state.base_dir, &id)
}

fn teams_query(conn: &Connection, project_id: Option<String>) -> Result<Vec<Team>> {
    let base = "SELECT t.id, t.project_id, p.name, t.name, t.template_key, t.description,
            (SELECT COUNT(*) FROM seats s WHERE s.team_id = t.id),
            (SELECT COUNT(*) FROM seats s WHERE s.team_id = t.id AND s.agent_id IS NOT NULL),
            t.created_at
         FROM teams t LEFT JOIN projects p ON p.id = t.project_id";
    let sql = if project_id.is_some() {
        format!("{base} WHERE t.project_id = ?1 ORDER BY t.created_at DESC")
    } else {
        format!("{base} ORDER BY t.created_at DESC")
    };
    let mut stmt = conn.prepare(&sql)?;
    let mapper = |r: &rusqlite::Row| -> rusqlite::Result<Team> {
        Ok(Team {
            id: r.get(0)?,
            project_id: r.get(1)?,
            project_name: r.get(2)?,
            name: r.get(3)?,
            template_key: r.get(4)?,
            description: r.get(5)?,
            seats_count: r.get(6)?,
            assigned_count: r.get(7)?,
            created_at: r.get(8)?,
        })
    };
    let items: Vec<Team> = if let Some(pid) = project_id {
        stmt.query_map(params![pid], mapper)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map([], mapper)?.filter_map(|r| r.ok()).collect()
    };
    Ok(items)
}
