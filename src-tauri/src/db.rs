use crate::error::Result;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SCHEMA_VERSION: i64 = 8;

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

#[derive(Clone)]
pub struct Db {
    pub path: PathBuf,
}

impl Db {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn open(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;",
        )?;
        Ok(conn)
    }

    pub fn init(&self) -> Result<()> {
        let mut conn = self.open()?;
        conn.execute_batch(SCHEMA_SQL)?;
        migrate(&mut conn)?;
        let version: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_meta",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if version < SCHEMA_VERSION {
            conn.execute(
                "INSERT INTO schema_meta (version, applied_at) VALUES (?1, ?2)",
                params![SCHEMA_VERSION, now_ms()],
            )?;
        }
        seed_prices(&conn)?;
        Ok(())
    }

    /// 应用重启后，清理上次未完成的运行、任务与审批，避免界面停留在假运行状态。
    pub fn reconcile_on_startup(&self) -> Result<()> {
        let conn = self.open()?;
        // 先记录被中断的任务，再改状态，最后逐条留下可追溯的系统消息。
        let interrupted: Vec<String> = {
            let mut stmt = conn.prepare(&format!(
                "SELECT id FROM tasks WHERE status IN ({})",
                crate::task_lifecycle::ACTIVE_STATUSES_SQL
            ))?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        conn.execute(
            "UPDATE runs SET status = 'failed', error = COALESCE(error, '宿主重启，运行中断'), ended_at = ?1
             WHERE status IN ('starting', 'running', 'paused_budget')",
            params![now_ms()],
        )?;
        for task_id in &interrupted {
            if let Err(err) = crate::task_lifecycle::apply(
                &conn,
                task_id,
                crate::task_lifecycle::TaskTransition::StartupReconcile,
                "应用重启，任务已中断",
            ) {
                tracing::warn!("任务 {task_id} 的重启状态回收失败：{err}");
                continue;
            }
            let _ = insert_message(
                &conn,
                Some(task_id),
                None,
                None,
                None,
                "system",
                Some("system"),
                &crate::i18n::tr(
                    "宿主重启，运行已中断；重新派发时会先核对目录与外部状态。",
                    "The host restarted and the run was interrupted. When you dispatch again, the directory and external state will be checked first.",
                ),
                json!({ "recovered": true }),
            );
        }
        conn.execute(
            "UPDATE approvals SET status = 'expired', decided_at = ?1, decision_note = '应用重启，审批已失效'
             WHERE status = 'pending'",
            params![now_ms()],
        )?;
        // 重启后不存在任何活动运行，所有预留都必须归还，否则预算会被永久占用。
        conn.execute(
            "UPDATE budget_reservations SET status = 'released'
             WHERE status = 'active'",
            [],
        )?;
        // 隔离工作区可能在上次运行中留下了改动；启动时全量重算待审查标记，
        // 覆盖失败、阻塞与中断任务，避免看板与导航漏报。
        let with_work_dir: Vec<String> = {
            let mut stmt = conn.prepare(&format!(
                "SELECT id FROM tasks
                 WHERE work_dir IS NOT NULL AND status NOT IN ({})",
                crate::task_lifecycle::ACTIVE_STATUSES_SQL
            ))?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        for task_id in with_work_dir {
            if let Err(err) = crate::review::refresh_review_flag(&conn, &task_id) {
                tracing::warn!("刷新任务 {task_id} 的待审查标记失败：{err}");
            }
        }
        if let Some(base) = self.path.parent() {
            cleanup_temp_runs(&base.join("sessions"));
        }
        Ok(())
    }
}

/// 按保留策略清理历史 transcript 文件，避免 sessions 目录无界增长。
pub fn cleanup_transcripts(base: &Path, retention_days: i64) -> Result<()> {
    if retention_days <= 0 {
        return Ok(());
    }
    let dir = base.join("sessions");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    let max_age = Duration::from_secs(retention_days as u64 * 86_400);
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_transcript = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("ndjson"))
            .unwrap_or(false);
        if !is_transcript {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| age > max_age)
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

/// 运行期临时配置目录统一清理，不在系统中留下残留。
fn cleanup_temp_runs(sessions: &Path) {
    if let Ok(entries) = std::fs::read_dir(sessions) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // 只清理一小时前的残留，避免误删另一个实例正在使用的目录。
            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|age| age.as_secs() > 3600)
                .unwrap_or(false);
            if stale {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
}

fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({table})");
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) else {
        return false;
    };
    let names: Vec<String> = rows.filter_map(|r| r.ok()).collect();
    names.iter().any(|name| name == column)
}

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0)
}

fn fk_on_delete(conn: &Connection, table: &str, column: &str) -> String {
    let sql = format!("PRAGMA foreign_key_list({table})");
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return String::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(3)?, r.get::<_, Option<String>>(6)?))
    }) else {
        return String::new();
    };
    for row in rows.flatten() {
        if row.0 == column {
            return row.1.unwrap_or_default().to_uppercase();
        }
    }
    String::new()
}

/// 版本化迁移：每一步都在事务内完成，失败不会留下半套 schema。
fn migrate(conn: &mut Connection) -> Result<()> {
    if !has_column(conn, "agents", "provider_id") {
        conn.execute("ALTER TABLE agents ADD COLUMN provider_id TEXT", [])?;
    }
    if !has_column(conn, "tasks", "model") {
        conn.execute("ALTER TABLE tasks ADD COLUMN model TEXT", [])?;
    }
    if !has_column(conn, "tasks", "reasoning") {
        conn.execute("ALTER TABLE tasks ADD COLUMN reasoning TEXT", [])?;
    }
    if !has_column(conn, "tasks", "mandate") {
        conn.execute(
            "ALTER TABLE tasks ADD COLUMN mandate TEXT NOT NULL DEFAULT 'ask'",
            [],
        )?;
    }
    if !has_column(conn, "tasks", "work_dir") {
        conn.execute("ALTER TABLE tasks ADD COLUMN work_dir TEXT", [])?;
    }
    if !has_column(conn, "runs", "prompt_text") {
        conn.execute("ALTER TABLE runs ADD COLUMN prompt_text TEXT", [])?;
    }
    let mut version = user_version(conn).min(SCHEMA_VERSION);

    if version < 1 {
        // v1：历史库补齐 agents.provider_id（已在上面完成）与基础索引。
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_messages_task_time ON messages(task_id, created_at);
             CREATE INDEX IF NOT EXISTS idx_runs_task_time ON runs(task_id, started_at);
             CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status);
             CREATE INDEX IF NOT EXISTS idx_approvals_status ON approvals(status, created_at);
             CREATE INDEX IF NOT EXISTS idx_approvals_run ON approvals(run_id);
             CREATE INDEX IF NOT EXISTS idx_reservations_run ON budget_reservations(run_id);",
        )?;
        conn.pragma_update(None, "user_version", 1)?;
        version = 1;
    }

    if version < 2 {
        // v2：events 自增计数器 + seq 唯一索引（先把历史重复 seq 重排）。
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS counters (name TEXT PRIMARY KEY, value INTEGER NOT NULL);
             WITH ordered AS (
               SELECT id, ROW_NUMBER() OVER (ORDER BY seq, created_at, id) AS rn FROM events
             )
             UPDATE events SET seq = (SELECT rn FROM ordered WHERE ordered.id = events.id);
             INSERT OR REPLACE INTO counters(name, value)
               VALUES ('events_seq', (SELECT COALESCE(MAX(seq), 0) FROM events));
             CREATE UNIQUE INDEX IF NOT EXISTS idx_events_seq ON events(seq);",
        )?;
        tx.commit()?;
        conn.pragma_update(None, "user_version", 2)?;
        version = 2;
    }

    if version < 3 {
        // v3：runs.agent_id 改为 ON DELETE SET NULL，并修正能力标签。
        if fk_on_delete(conn, "runs", "agent_id") != "SET NULL" {
            conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(
                "DROP TABLE IF EXISTS runs_new;
                 CREATE TABLE runs_new (
                   id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                   seat_id TEXT REFERENCES seats(id) ON DELETE SET NULL,
                   agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
                   external_session_id TEXT, stage_key TEXT,
                   status TEXT NOT NULL DEFAULT 'starting',
                   exit_code INTEGER, error TEXT,
                   input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
                   cost_micros INTEGER NOT NULL DEFAULT 0,
                   transcript_path TEXT, transcript_bytes INTEGER NOT NULL DEFAULT 0,
                   started_at INTEGER NOT NULL, ended_at INTEGER
                 );
                 INSERT INTO runs_new
                   SELECT id, task_id, seat_id, agent_id, external_session_id, stage_key, status,
                          exit_code, error, input_tokens, output_tokens, cost_micros,
                          transcript_path, transcript_bytes, started_at, ended_at
                   FROM runs;
                 DROP TABLE runs;
                 ALTER TABLE runs_new RENAME TO runs;
                 CREATE INDEX IF NOT EXISTS idx_runs_task_time ON runs(task_id, started_at);
                 CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status);",
            )?;
            tx.commit()?;
            conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        }
        // 让 UI 的能力标注与真实适配器一致：只有 Claude Code 具备审批与结构化 usage。
        conn.execute(
            "UPDATE agents SET capabilities_json = ?1
             WHERE adapter = 'claude-code' AND capabilities_json LIKE '%\"toolApproval\":false%'",
            params!["{\"streamUsage\":true,\"toolApproval\":true,\"budgetEnforcement\":true,\"usageSource\":\"provider\"}"],
        )?;
        conn.execute(
            "UPDATE agents SET capabilities_json = ?1
             WHERE adapter <> 'claude-code' AND capabilities_json LIKE '%\"toolApproval\":true%'",
            params!["{\"streamUsage\":false,\"toolApproval\":false,\"budgetEnforcement\":false,\"usageSource\":\"estimate\"}"],
        )?;
        conn.pragma_update(None, "user_version", 3)?;
        version = 3;
    }

    if version < 4 {
        // v4：记录每次运行的隔离级别，供运行详情与审计使用。
        if !has_column(conn, "runs", "isolation") {
            conn.execute(
                "ALTER TABLE runs ADD COLUMN isolation TEXT NOT NULL DEFAULT 'process'",
                [],
            )?;
        }
        conn.pragma_update(None, "user_version", 4)?;
    }

    if version < 5 {
        // v5：保存从供应商拉取并勾选的模型列表，供 Agent 选择。
        // 早期测试夹具可能没有 providers 表，缺少时跳过（真实库由 SCHEMA_SQL 创建）。
        let has_providers: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'providers'",
            [],
            |r| r.get(0),
        )?;
        if has_providers > 0 && !has_column(conn, "providers", "models_json") {
            conn.execute(
                "ALTER TABLE providers ADD COLUMN models_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }
        conn.pragma_update(None, "user_version", 5)?;
        version = 5;
    }

    if version < 6 {
        // v6：记录任务是否还有待审查的隔离工作区改动（未提交或未应用）。
        if !has_column(conn, "tasks", "review_pending") {
            conn.execute(
                "ALTER TABLE tasks ADD COLUMN review_pending INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        conn.pragma_update(None, "user_version", 6)?;
    }

    if version < 7 {
        // v7：任务协作件——父子任务、依赖关系、附件与验收清单。
        if !has_column(conn, "tasks", "parent_id") {
            conn.execute("ALTER TABLE tasks ADD COLUMN parent_id TEXT", [])?;
        }
        if !has_column(conn, "tasks", "acceptance_json") {
            conn.execute(
                "ALTER TABLE tasks ADD COLUMN acceptance_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS task_dependencies (
               task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
               depends_on_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
               created_at INTEGER NOT NULL,
               PRIMARY KEY (task_id, depends_on_id)
             );
             CREATE INDEX IF NOT EXISTS idx_task_deps_parent ON task_dependencies(depends_on_id);
             CREATE TABLE IF NOT EXISTS attachments (
               id TEXT PRIMARY KEY,
               task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
               name TEXT NOT NULL, path TEXT NOT NULL, size INTEGER NOT NULL DEFAULT 0,
               created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_attachments_task ON attachments(task_id, created_at);
             CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id);",
        )?;
        // 能力字段补齐：旧库只记录 streamUsage/toolApproval，按适配器回填真实能力。
        let claude_caps = crate::detect::capabilities_for("claude-code").to_string();
        let other_caps = crate::detect::capabilities_for("other").to_string();
        conn.execute(
            "UPDATE agents SET capabilities_json = ?1 WHERE adapter = 'claude-code'",
            [claude_caps],
        )?;
        conn.execute(
            "UPDATE agents SET capabilities_json = ?1 WHERE adapter <> 'claude-code'",
            [other_caps],
        )?;
        conn.pragma_update(None, "user_version", 7)?;
        version = 7;
    }

    if version < 8 {
        // v8：记录沙箱回退到进程隔离的原因，便于运行历史与审计查询。
        if !has_column(conn, "runs", "isolation_reason") {
            conn.execute("ALTER TABLE runs ADD COLUMN isolation_reason TEXT", [])?;
        }
        // 回填历史运行：只依据已记录的 sandbox.fallback 事件，不改变运行状态。
        // 极早期的测试夹具可能没有完整 events 列，此时跳过回填。
        if has_column(conn, "events", "run_id")
            && has_column(conn, "events", "type")
            && has_column(conn, "events", "payload_json")
        {
            conn.execute_batch(
                r#"
            UPDATE runs SET isolation_reason = 'opencode_git_nul'
            WHERE isolation = 'process' AND isolation_reason IS NULL
              AND EXISTS (
                SELECT 1 FROM events e
                WHERE e.run_id = runs.id AND e.type = 'sandbox.fallback'
                  AND (e.payload_json LIKE '%"opencode_git_nul"%'
                       OR e.payload_json LIKE '%NUL%')
              );
            UPDATE runs SET isolation_reason = 'proxy_disabled'
            WHERE isolation = 'process' AND isolation_reason IS NULL
              AND EXISTS (
                SELECT 1 FROM events e
                WHERE e.run_id = runs.id AND e.type = 'sandbox.fallback'
                  AND e.payload_json LIKE '%"proxy_disabled"%'
              );
            UPDATE runs SET isolation_reason = 'provider_unavailable'
            WHERE isolation = 'process' AND isolation_reason IS NULL
              AND EXISTS (
                SELECT 1 FROM events e
                WHERE e.run_id = runs.id AND e.type = 'sandbox.fallback'
                  AND e.payload_json LIKE '%"provider_unavailable"%'
              );
            "#,
            )?;
        }
        conn.pragma_update(None, "user_version", 8)?;
    }
    Ok(())
}

pub fn log_event(
    conn: &Connection,
    project_id: Option<&str>,
    task_id: Option<&str>,
    run_id: Option<&str>,
    event_type: &str,
    actor: Option<&str>,
    payload: Value,
) -> Result<()> {
    // 单语句原子自增，避免并发运行取到相同 seq；counters 在迁移中初始化。
    let seq: i64 = conn
        .query_row(
            "UPDATE counters SET value = value + 1 WHERE name = 'events_seq' RETURNING value",
            [],
            |r| r.get(0),
        )
        .or_else(|_| {
            conn.execute(
                "INSERT OR IGNORE INTO counters (name, value) VALUES ('events_seq', 0)",
                [],
            )?;
            conn.query_row(
                "UPDATE counters SET value = value + 1 WHERE name = 'events_seq' RETURNING value",
                [],
                |r| r.get(0),
            )
        })
        .unwrap_or(1);
    conn.execute(
        "INSERT INTO events (id, seq, project_id, task_id, run_id, type, actor, payload_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            new_id(),
            seq,
            project_id,
            task_id,
            run_id,
            event_type,
            actor,
            payload.to_string(),
            now_ms()
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn insert_message(
    conn: &Connection,
    task_id: Option<&str>,
    run_id: Option<&str>,
    seat_id: Option<&str>,
    agent_id: Option<&str>,
    kind: &str,
    role: Option<&str>,
    content: &str,
    metadata: Value,
) -> Result<String> {
    let id = new_id();
    conn.execute(
        "INSERT INTO messages (id, task_id, run_id, seat_id, agent_id, kind, role, content, metadata_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![id, task_id, run_id, seat_id, agent_id, kind, role, content, metadata.to_string(), now_ms()],
    )?;
    Ok(id)
}

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_meta (
  version INTEGER NOT NULL,
  applied_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'mixed',
  description TEXT, settings_json TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS agents (
  id TEXT PRIMARY KEY, name TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'cli',
  adapter TEXT NOT NULL,
  command TEXT, args_json TEXT NOT NULL DEFAULT '[]',
  env_json TEXT NOT NULL DEFAULT '{}',
  model TEXT, provider_id TEXT, capabilities_json TEXT NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'idle',
  version TEXT, last_seen_at INTEGER,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS teams (
  id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL, template_key TEXT, description TEXT,
  config_json TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS seats (
  id TEXT PRIMARY KEY, team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  role_key TEXT NOT NULL, role_name TEXT NOT NULL, responsibility TEXT,
  agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
  order_index INTEGER NOT NULL DEFAULT 0,
  gate INTEGER NOT NULL DEFAULT 0,
  UNIQUE (team_id, role_key)
);

CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  team_id TEXT REFERENCES teams(id) ON DELETE SET NULL,
  parent_id TEXT,
  title TEXT NOT NULL, description TEXT, acceptance TEXT,
  status TEXT NOT NULL DEFAULT 'inbox',
  priority INTEGER NOT NULL DEFAULT 2,
  assignee_seat_id TEXT REFERENCES seats(id) ON DELETE SET NULL,
  stage_index INTEGER NOT NULL DEFAULT 0, stage_key TEXT,
  planned_tokens INTEGER,
  actual_tokens INTEGER NOT NULL DEFAULT 0,
  actual_cost_micros INTEGER NOT NULL DEFAULT 0,
  mandate TEXT NOT NULL DEFAULT 'ask',
  work_dir TEXT,
  review_pending INTEGER NOT NULL DEFAULT 0,
  acceptance_json TEXT NOT NULL DEFAULT '[]',
  created_at INTEGER NOT NULL, started_at INTEGER, finished_at INTEGER,
  result_json TEXT
);

CREATE TABLE IF NOT EXISTS runs (
  id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  seat_id TEXT REFERENCES seats(id) ON DELETE SET NULL,
  agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
  external_session_id TEXT, stage_key TEXT,
  status TEXT NOT NULL DEFAULT 'starting',
  isolation TEXT NOT NULL DEFAULT 'process',
  isolation_reason TEXT,
  exit_code INTEGER, error TEXT,
  input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
  cost_micros INTEGER NOT NULL DEFAULT 0,
  transcript_path TEXT, transcript_bytes INTEGER NOT NULL DEFAULT 0,
  prompt_text TEXT,
  started_at INTEGER NOT NULL, ended_at INTEGER
);

CREATE TABLE IF NOT EXISTS messages (
  id TEXT PRIMARY KEY, task_id TEXT, run_id TEXT,
  seat_id TEXT, agent_id TEXT,
  kind TEXT NOT NULL, role TEXT, content TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS task_dependencies (
  task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  depends_on_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (task_id, depends_on_id)
);

CREATE TABLE IF NOT EXISTS attachments (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  name TEXT NOT NULL, path TEXT NOT NULL, size INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tool_calls (
  id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  tool_name TEXT NOT NULL, input_json TEXT NOT NULL,
  output_json TEXT, status TEXT NOT NULL,
  risk_level TEXT, approval_id TEXT,
  created_at INTEGER NOT NULL, finished_at INTEGER
);

CREATE TABLE IF NOT EXISTS usage_events (
  id TEXT PRIMARY KEY, run_id TEXT, task_id TEXT, project_id TEXT,
  team_id TEXT, seat_id TEXT, agent_id TEXT,
  provider TEXT, model TEXT,
  input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens INTEGER NOT NULL DEFAULT 0,
  cache_write_tokens INTEGER NOT NULL DEFAULT 0,
  cost_micros INTEGER NOT NULL DEFAULT 0,
  usage_source TEXT NOT NULL,
  dedupe_key TEXT NOT NULL UNIQUE,
  occurred_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_usage_time ON usage_events(occurred_at);
CREATE INDEX IF NOT EXISTS idx_usage_project_time ON usage_events(project_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_usage_agent_time ON usage_events(agent_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_usage_task ON usage_events(task_id);

CREATE TABLE IF NOT EXISTS budgets (
  id TEXT PRIMARY KEY,
  scope_type TEXT NOT NULL, scope_id TEXT,
  unit TEXT NOT NULL, period TEXT NOT NULL,
  limit_value INTEGER NOT NULL,
  warn_pct INTEGER NOT NULL DEFAULT 80, hard_stop INTEGER NOT NULL DEFAULT 1,
  enabled INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL,
  UNIQUE (scope_type, scope_id, period)
);

CREATE TABLE IF NOT EXISTS budget_reservations (
  id TEXT PRIMARY KEY, budget_id TEXT NOT NULL REFERENCES budgets(id) ON DELETE CASCADE,
  run_id TEXT, amount INTEGER NOT NULL, status TEXT NOT NULL DEFAULT 'active',
  created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS approvals (
  id TEXT PRIMARY KEY, task_id TEXT, run_id TEXT, type TEXT NOT NULL,
  risk_level TEXT, title TEXT NOT NULL, detail_json TEXT NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'pending',
  requested_by TEXT, decided_by TEXT, decision_note TEXT,
  created_at INTEGER NOT NULL, expires_at INTEGER, decided_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_approvals_task_status ON approvals(task_id, status);

CREATE TABLE IF NOT EXISTS approval_rules (
  id TEXT PRIMARY KEY, scope_type TEXT NOT NULL DEFAULT 'global', scope_id TEXT,
  match_json TEXT NOT NULL, action TEXT NOT NULL,
  priority INTEGER NOT NULL DEFAULT 100, enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
  id TEXT PRIMARY KEY, seq INTEGER NOT NULL,
  project_id TEXT, task_id TEXT, run_id TEXT,
  type TEXT NOT NULL, actor TEXT,
  payload_json TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_time ON events(created_at);
CREATE INDEX IF NOT EXISTS idx_events_task ON events(task_id, created_at);

CREATE TABLE IF NOT EXISTS artifacts (
  id TEXT PRIMARY KEY, task_id TEXT, run_id TEXT,
  kind TEXT NOT NULL, name TEXT NOT NULL, path TEXT, mime TEXT,
  size INTEGER, metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS prices (
  id TEXT PRIMARY KEY, provider TEXT NOT NULL, model_pattern TEXT NOT NULL,
  input_per_mtok INTEGER NOT NULL, output_per_mtok INTEGER NOT NULL,
  cache_read_per_mtok INTEGER NOT NULL DEFAULT 0,
  cache_write_per_mtok INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS providers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  protocol TEXT NOT NULL,
  base_url TEXT NOT NULL,
  model TEXT,
  models_json TEXT NOT NULL DEFAULT '[]',
  key_cipher TEXT,
  key_hint TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS extensions (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  description TEXT,
  config_json TEXT NOT NULL DEFAULT '{}',
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
"#;

fn seed_prices(conn: &Connection) -> Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM prices", [], |r| r.get(0))?;
    if count > 0 {
        return Ok(());
    }
    let rows: [(&str, &str, i64, i64, i64, i64); 14] = [
        (
            "anthropic",
            "claude-opus-4",
            15_000_000,
            75_000_000,
            1_500_000,
            18_750_000,
        ),
        (
            "anthropic",
            "claude-sonnet-4",
            3_000_000,
            15_000_000,
            300_000,
            3_750_000,
        ),
        (
            "anthropic",
            "claude-haiku-4",
            1_000_000,
            5_000_000,
            100_000,
            1_250_000,
        ),
        (
            "anthropic",
            "claude-3-5",
            3_000_000,
            15_000_000,
            300_000,
            3_750_000,
        ),
        (
            "anthropic",
            "claude",
            3_000_000,
            15_000_000,
            300_000,
            3_750_000,
        ),
        ("openai", "gpt-5", 1_250_000, 10_000_000, 125_000, 0),
        ("openai", "gpt-4.1", 2_000_000, 8_000_000, 500_000, 0),
        ("openai", "gpt-4o", 2_500_000, 10_000_000, 1_250_000, 0),
        (
            "deepseek",
            "deepseek-reasoner",
            550_000,
            2_190_000,
            140_000,
            0,
        ),
        ("deepseek", "deepseek", 280_000, 420_000, 28_000, 0),
        (
            "google",
            "gemini-2.5-pro",
            1_250_000,
            10_000_000,
            310_000,
            0,
        ),
        ("google", "gemini", 300_000, 2_500_000, 75_000, 0),
        ("moonshot", "kimi", 600_000, 2_500_000, 150_000, 0),
        ("zhipu", "glm", 600_000, 2_200_000, 110_000, 0),
    ];
    let tx = conn.unchecked_transaction()?;
    for (provider, pattern, i, o, cr, cw) in rows {
        tx.execute(
            "INSERT INTO prices (id, provider, model_pattern, input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![new_id(), provider, pattern, i, o, cr, cw, now_ms()],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn ensure_dirs(base: &Path) -> Result<()> {
    for name in ["sessions", "logs", "artifacts", "extensions"] {
        std::fs::create_dir_all(base.join(name))?;
    }
    Ok(())
}

pub fn json_or_empty(raw: Option<String>) -> Value {
    raw.and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(|| json!({}))
}

/// 测试用内存库：与真实初始化走同一套 schema 与迁移。
#[cfg(test)]
pub(crate) fn test_connection() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA_SQL).unwrap();
    migrate(&mut conn).unwrap();
    conn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_legacy_database_is_atomic_and_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE agents (id TEXT PRIMARY KEY, name TEXT, adapter TEXT, capabilities_json TEXT);
             INSERT INTO agents VALUES ('a1', 'Claude', 'claude-code', '{\"toolApproval\":false}');
             INSERT INTO agents VALUES ('a2', 'Codex', 'codex-pty', '{\"toolApproval\":true}');
             CREATE TABLE tasks (id TEXT PRIMARY KEY);
             INSERT INTO tasks VALUES ('t1');
             CREATE TABLE runs (
               id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
               seat_id TEXT, agent_id TEXT REFERENCES agents(id),
               external_session_id TEXT, stage_key TEXT,
               status TEXT NOT NULL DEFAULT 'starting', exit_code INTEGER, error TEXT,
               input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
               cost_micros INTEGER NOT NULL DEFAULT 0, transcript_path TEXT,
               transcript_bytes INTEGER NOT NULL DEFAULT 0, started_at INTEGER NOT NULL, ended_at INTEGER
             );
             INSERT INTO runs (id, task_id, agent_id, started_at) VALUES ('r1', 't1', 'a2', 1);
             CREATE TABLE events (id TEXT PRIMARY KEY, seq INTEGER NOT NULL, created_at INTEGER NOT NULL);
             INSERT INTO events VALUES ('e1', 1, 1);
             INSERT INTO events VALUES ('e2', 1, 2);
             CREATE TABLE messages (id TEXT PRIMARY KEY, task_id TEXT, created_at INTEGER NOT NULL);
             CREATE TABLE approvals (id TEXT PRIMARY KEY, status TEXT NOT NULL, run_id TEXT, created_at INTEGER NOT NULL);
             CREATE TABLE budget_reservations (id TEXT PRIMARY KEY, run_id TEXT, status TEXT NOT NULL);",
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        assert_eq!(fk_on_delete(&conn, "runs", "agent_id"), "SET NULL");
        let unique_seqs: i64 = conn
            .query_row("SELECT COUNT(DISTINCT seq) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(unique_seqs, 2);
        let caps: String = conn
            .query_row(
                "SELECT capabilities_json FROM agents WHERE id = 'a2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(caps.contains("\"toolApproval\":false"));

        // 幂等：再次迁移不应失败或改变版本。
        migrate(&mut conn).unwrap();
        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        // 删除 Agent 不再被历史运行阻断，且运行记录保留。
        conn.execute("DELETE FROM agents WHERE id = 'a2'", [])
            .unwrap();
        let runs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE id = 'r1' AND agent_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(runs, 1);
    }

    #[test]
    fn migrate_backfills_sandbox_fallback_reason() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            CREATE TABLE agents (id TEXT PRIMARY KEY, name TEXT, adapter TEXT, capabilities_json TEXT);
            CREATE TABLE tasks (id TEXT PRIMARY KEY);
            INSERT INTO tasks VALUES ('t1');
            CREATE TABLE runs (
              id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
              seat_id TEXT, agent_id TEXT, external_session_id TEXT, stage_key TEXT,
              status TEXT NOT NULL DEFAULT 'starting',
              isolation TEXT NOT NULL DEFAULT 'process',
              exit_code INTEGER, error TEXT,
              input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
              cost_micros INTEGER NOT NULL DEFAULT 0, transcript_path TEXT,
              transcript_bytes INTEGER NOT NULL DEFAULT 0, started_at INTEGER NOT NULL,
              ended_at INTEGER
            );
            INSERT INTO runs (id, task_id, isolation, started_at)
              VALUES ('r1', 't1', 'process', 1);
            CREATE TABLE events (
              id TEXT PRIMARY KEY, seq INTEGER NOT NULL, project_id TEXT, task_id TEXT,
              run_id TEXT, type TEXT NOT NULL, actor TEXT,
              payload_json TEXT NOT NULL DEFAULT '{}', created_at INTEGER NOT NULL
            );
            INSERT INTO events (id, seq, run_id, type, payload_json, created_at)
              VALUES ('e1', 1, 'r1', 'sandbox.fallback',
                      '{"reason":"OpenCode Git discovery needs NUL"}', 1);
            PRAGMA user_version = 7;
            "#,
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        let reason: Option<String> = conn
            .query_row(
                "SELECT isolation_reason FROM runs WHERE id = 'r1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reason.as_deref(), Some("opencode_git_nul"));
    }
}
