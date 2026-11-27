use super::RunRequest;
use crate::db::{insert_message, log_event, now_ms, Db};
use crate::error::Result;
use crate::usage::{record, UsageInput};
use rusqlite::params;
use serde_json::{json, Value};

/// Persistence owned by one run attempt.
///
/// The store deliberately exposes intent-level methods instead of raw
/// connections. Every method keeps the exact SQL semantics the runtime had
/// before the split, and swallows late-stage write failures the same way the
/// original code did so cleanup paths cannot mask the run outcome.
#[derive(Clone)]
pub(crate) struct RunStore {
    db: Db,
}

impl RunStore {
    pub(crate) fn new(db: Db) -> Self {
        Self { db }
    }

    pub(crate) fn record_start(&self, req: &RunRequest) -> Result<()> {
        let conn = self.db.open()?;
        conn.execute(
            "INSERT OR REPLACE INTO runs (id, task_id, seat_id, agent_id, external_session_id, stage_key, status, prompt_text, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'starting', ?7, ?8)",
            params![
                req.run_id,
                req.task_id,
                req.seat_id,
                req.agent_id,
                req.resume_session,
                req.stage_key,
                req.prompt,
                now_ms()
            ],
        )?;
        conn.execute(
            "UPDATE agents SET status = 'running', last_seen_at = ?1 WHERE id = ?2",
            params![now_ms(), req.agent_id],
        )?;
        log_event(
            &conn,
            Some(&req.project_id),
            Some(&req.task_id),
            Some(&req.run_id),
            "run.started",
            Some(&req.agent_name),
            json!({ "stage": req.stage_key, "adapter": req.adapter }),
        )?;
        Ok(())
    }

    pub(crate) fn set_isolation(&self, run_id: &str, isolation: &str) {
        if let Ok(conn) = self.db.open() {
            let _ = conn.execute(
                "UPDATE runs SET isolation = ?1 WHERE id = ?2",
                params![isolation, run_id],
            );
        }
    }

    /// Record an explicit sandbox fallback and its reason in one place.
    pub(crate) fn fallback_to_process(&self, req: &RunRequest, reason: &str, warning: &str) {
        if let Ok(conn) = self.db.open() {
            let _ = conn.execute(
                "UPDATE runs SET isolation = 'process', isolation_reason = ?1 WHERE id = ?2",
                params![reason, req.run_id],
            );
            let _ = log_event(
                &conn,
                Some(&req.project_id),
                Some(&req.task_id),
                Some(&req.run_id),
                "sandbox.fallback",
                Some("host"),
                json!({ "reason": reason, "message": warning, "isolation": "process" }),
            );
        }
    }

    pub(crate) fn set_session(&self, run_id: &str, session_id: &str) {
        if let Ok(conn) = self.db.open() {
            let _ = conn.execute(
                "UPDATE runs SET external_session_id = ?1 WHERE id = ?2",
                params![session_id, run_id],
            );
        }
    }

    pub(crate) fn insert_message(&self, message: MessageRecord<'_>) -> Result<Option<String>> {
        let conn = self.db.open()?;
        Ok(insert_message(
            &conn,
            Some(message.task_id),
            Some(message.run_id),
            message.seat_id,
            Some(message.agent_id),
            message.kind,
            message.role,
            message.content,
            message.meta,
        )
        .ok())
    }

    pub(crate) fn record_event(
        &self,
        req: &RunRequest,
        event: &str,
        actor: Option<&str>,
        payload: Value,
    ) {
        if let Ok(conn) = self.db.open() {
            let _ = log_event(
                &conn,
                Some(&req.project_id),
                Some(&req.task_id),
                Some(&req.run_id),
                event,
                actor,
                payload,
            );
        }
    }

    pub(crate) fn insert_tool_call(
        &self,
        call_id: &str,
        run_id: &str,
        tool_name: &str,
        input: &Value,
    ) {
        if let Ok(conn) = self.db.open() {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO tool_calls (id, run_id, tool_name, input_json, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'running', ?5)",
                params![call_id, run_id, tool_name, input.to_string(), now_ms()],
            );
        }
    }

    pub(crate) fn complete_tool_call(&self, call_id: &str, output: &Value) {
        if let Ok(conn) = self.db.open() {
            let _ = conn.execute(
                "UPDATE tool_calls SET status = 'completed', output_json = ?1, finished_at = ?2 WHERE id = ?3",
                params![output.to_string(), now_ms(), call_id],
            );
        }
    }

    pub(crate) fn record_usage_on(
        &self,
        conn: &rusqlite::Connection,
        input: &UsageInput,
    ) -> Option<i64> {
        record(conn, input).ok().flatten()
    }

    pub(crate) fn finish(&self, finish: FinishRecord<'_>) -> bool {
        let Ok(conn) = self.db.open() else {
            return false;
        };
        let tx = conn.unchecked_transaction().ok();
        let run_sql = "UPDATE runs SET status = ?1, exit_code = ?2, error = ?3, ended_at = ?4, external_session_id = COALESCE(?5, external_session_id)
             WHERE id = ?6";
        let run_params = params![
            finish.status,
            finish.exit_code,
            finish.error,
            now_ms(),
            finish.session_id,
            finish.run_id
        ];
        let agent_sql = "UPDATE agents SET status = 'idle', last_seen_at = ?1 WHERE id = ?2";
        let agent_params = params![now_ms(), finish.agent_id];
        let approval_sql = "UPDATE approvals SET status = 'expired', decided_at = ?1, decision_note = '运行已结束，工具授权已失效'
             WHERE run_id = ?2 AND type = 'tool_permission' AND status = 'pending'";
        let approval_params = params![now_ms(), finish.run_id];
        match &tx {
            Some(tx) => {
                let _ = tx.execute(run_sql, run_params);
                let _ = tx.execute(agent_sql, agent_params);
                let _ = tx.execute(approval_sql, approval_params);
            }
            None => {
                let _ = conn.execute(run_sql, run_params);
                let _ = conn.execute(agent_sql, agent_params);
                let _ = conn.execute(approval_sql, approval_params);
            }
        }
        crate::budget::release(&conn, finish.run_id).ok();
        let _ = log_event(
            &conn,
            Some(finish.project_id),
            Some(finish.task_id),
            Some(finish.run_id),
            "run.finished",
            Some(finish.agent_name),
            json!({
                "status": finish.status,
                "exitCode": finish.exit_code,
                "error": finish.error,
                "isolation": finish.isolation,
                "isolationReason": finish.isolation_reason
            }),
        );
        if let Some(tx) = tx {
            let _ = tx.commit();
        }
        true
    }

    /// Best-effort cleanup for a run that never reached `finish`.
    pub(crate) fn mark_interrupted(&self, run_id: &str, agent_id: &str) {
        if let Ok(conn) = self.db.open() {
            let _ = conn.execute(
                "UPDATE runs SET status = 'failed', error = COALESCE(error, '运行异常中断'), ended_at = ?1
                 WHERE id = ?2 AND status IN ('starting', 'running')",
                params![now_ms(), run_id],
            );
            let _ = conn.execute(
                "UPDATE agents SET status = 'idle', last_seen_at = ?1 WHERE id = ?2",
                params![now_ms(), agent_id],
            );
            let _ = conn.execute(
                "UPDATE approvals SET status = 'expired', decided_at = ?1, decision_note = '运行异常中断，工具授权已失效'
                 WHERE run_id = ?2 AND type = 'tool_permission' AND status = 'pending'",
                params![now_ms(), run_id],
            );
            let _ = crate::budget::release(&conn, run_id);
        }
    }
}

pub(crate) struct MessageRecord<'a> {
    pub(crate) task_id: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) seat_id: Option<&'a str>,
    pub(crate) agent_id: &'a str,
    pub(crate) kind: &'a str,
    pub(crate) role: Option<&'a str>,
    pub(crate) content: &'a str,
    pub(crate) meta: Value,
}

pub(crate) struct FinishRecord<'a> {
    pub(crate) project_id: &'a str,
    pub(crate) task_id: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) agent_id: &'a str,
    pub(crate) agent_name: &'a str,
    pub(crate) status: &'a str,
    pub(crate) exit_code: i32,
    pub(crate) error: Option<&'a str>,
    pub(crate) session_id: Option<&'a str>,
    pub(crate) isolation: &'a str,
    pub(crate) isolation_reason: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::error::Result;
    use serde_json::Value;
    use std::path::PathBuf;

    struct TestDb {
        dir: PathBuf,
        db: Db,
    }

    impl TestDb {
        fn new() -> Result<Self> {
            let dir =
                std::env::temp_dir().join(format!("lumen-run-store-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir)?;
            let db = Db::new(dir.join("lumen.db"));
            db.init()?;
            Ok(Self { dir, db })
        }
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn seed(conn: &rusqlite::Connection) {
        conn.execute(
            "INSERT INTO projects (id, name, path, created_at, updated_at) VALUES ('p1', 'Project', 'C:/tmp', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO teams (id, name, project_id, created_at) VALUES ('t1', 'Team', 'p1', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO seats (id, team_id, role_key, role_name, order_index, gate)
             VALUES ('s1', 't1', 'stage', 'Seat', 0, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agents (id, name, adapter, command, args_json, model, status, last_seen_at, created_at)
             VALUES ('a1', 'Agent', 'generic', 'cmd', '[]', 'claude', 'idle', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, project_id, team_id, title, description, status, model, created_at)
             VALUES ('task1', 'p1', 't1', 'Task', '', 'running', 'claude', 1)",
            [],
        )
        .unwrap();
    }

    fn request(store: &RunStore) -> RunRequest {
        let _ = store;
        RunRequest {
            run_id: "r1".into(),
            task_id: "task1".into(),
            project_id: "p1".into(),
            team_id: "t1".into(),
            seat_id: Some("s1".into()),
            seat_name: "Seat".into(),
            agent_id: "a1".into(),
            agent_name: "Agent".into(),
            adapter: "generic".into(),
            command: "cmd".into(),
            args: vec![],
            model: "claude".into(),
            project_path: PathBuf::from("C:/tmp"),
            stage_key: Some("stage".into()),
            prompt: "hello".into(),
            resume_session: None,
            budget_usd: None,
        }
    }

    #[test]
    fn record_start_marks_run_and_agent() {
        let test = TestDb::new().unwrap();
        seed(&test.db.open().unwrap());
        let store = RunStore::new(test.db.clone());
        store.record_start(&request(&store)).unwrap();

        let conn = test.db.open().unwrap();
        let (status, agent_status): (String, String) = conn
            .query_row(
                "SELECT r.status, a.status FROM runs r JOIN agents a ON a.id = r.agent_id WHERE r.id = 'r1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "starting");
        assert_eq!(agent_status, "running");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'run.started'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn finish_expires_approvals_and_writes_event() {
        let test = TestDb::new().unwrap();
        seed(&test.db.open().unwrap());
        let store = RunStore::new(test.db.clone());
        store.record_start(&request(&store)).unwrap();
        let conn = test.db.open().unwrap();
        conn.execute(
            "INSERT INTO approvals (id, task_id, run_id, type, title, status, created_at)
             VALUES ('ap1', 'task1', 'r1', 'tool_permission', 'Approval', 'pending', 1)",
            [],
        )
        .unwrap();
        drop(conn);

        assert!(store.finish(FinishRecord {
            project_id: "p1",
            task_id: "task1",
            run_id: "r1",
            agent_id: "a1",
            agent_name: "Agent",
            status: "completed",
            exit_code: 0,
            error: None,
            session_id: Some("sess-1"),
            isolation: "process",
            isolation_reason: None,
        }));

        let conn = test.db.open().unwrap();
        let (run_status, agent_status, approval_status): (String, String, String) = conn
            .query_row(
                "SELECT r.status, a.status, ap.status FROM runs r
                 JOIN agents a ON a.id = 'a1' JOIN approvals ap ON ap.id = 'ap1' WHERE r.id = 'r1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(run_status, "completed");
        assert_eq!(agent_status, "idle");
        assert_eq!(approval_status, "expired");
        let payload: String = conn
            .query_row(
                "SELECT payload_json FROM events WHERE type = 'run.finished'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let payload: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["status"], "completed");
    }

    #[test]
    fn mark_interrupted_only_touches_active_runs() {
        let test = TestDb::new().unwrap();
        seed(&test.db.open().unwrap());
        let store = RunStore::new(test.db.clone());
        store.record_start(&request(&store)).unwrap();
        store.mark_interrupted("r1", "a1");

        let conn = test.db.open().unwrap();
        let (status, agent_status): (String, String) = conn
            .query_row(
                "SELECT r.status, a.status FROM runs r JOIN agents a ON a.id = r.agent_id WHERE r.id = 'r1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "failed");
        assert_eq!(agent_status, "idle");
    }
}
