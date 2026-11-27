use crate::db::{new_id, now_ms};
use crate::error::Result;
use crate::models::Budget;
use crate::runtime::RunRegistry;
use chrono::{Datelike, Local, TimeZone};
use rusqlite::{params, params_from_iter, Connection};
use serde_json::json;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

pub fn day_start_ms() -> i64 {
    let now = Local::now();
    Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}

pub fn month_start_ms() -> i64 {
    let now = Local::now();
    Local
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct BudgetRow {
    pub id: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub unit: String,
    pub period: String,
    pub limit_value: i64,
    pub warn_pct: i64,
    pub hard_stop: bool,
    pub enabled: bool,
    pub created_at: i64,
}

fn row_from(r: &rusqlite::Row) -> rusqlite::Result<BudgetRow> {
    Ok(BudgetRow {
        id: r.get(0)?,
        scope_type: r.get(1)?,
        scope_id: r.get(2)?,
        unit: r.get(3)?,
        period: r.get(4)?,
        limit_value: r.get(5)?,
        warn_pct: r.get(6)?,
        hard_stop: r.get::<_, i64>(7)? != 0,
        enabled: r.get::<_, i64>(8)? != 0,
        created_at: r.get(9)?,
    })
}

pub fn rows(conn: &Connection) -> Result<Vec<BudgetRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, scope_type, scope_id, unit, period, limit_value, warn_pct, hard_stop, enabled, created_at
         FROM budgets ORDER BY created_at DESC",
    )?;
    let items = stmt.query_map([], row_from)?;
    Ok(items.filter_map(|r| r.ok()).collect())
}

pub fn scope_name(conn: &Connection, scope_type: &str, scope_id: Option<&str>) -> String {
    let id = scope_id.unwrap_or_default();
    match scope_type {
        "global" => crate::i18n::tr("全局", "Global"),
        "project" => conn
            .query_row(
                "SELECT name FROM projects WHERE id = ?1",
                params![id],
                |r| r.get::<_, String>(0),
            )
            .unwrap_or_else(|_| crate::i18n::tr("已删除项目", "Deleted project")),
        "team" => conn
            .query_row("SELECT name FROM teams WHERE id = ?1", params![id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap_or_else(|_| crate::i18n::tr("已删除团队", "Deleted team")),
        "agent" => conn
            .query_row("SELECT name FROM agents WHERE id = ?1", params![id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap_or_else(|_| crate::i18n::tr("已删除智能体", "Deleted agent")),
        "task" => conn
            .query_row("SELECT title FROM tasks WHERE id = ?1", params![id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap_or_else(|_| crate::i18n::tr("已删除任务", "Deleted task")),
        _ => scope_type.to_string(),
    }
}

fn spend_for(conn: &Connection, b: &BudgetRow) -> (i64, i64) {
    let metric = if b.unit == "usd" {
        "COALESCE(SUM(cost_micros), 0)"
    } else {
        "COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens), 0)"
    };
    let mut sql = format!("SELECT {metric} FROM usage_events WHERE 1=1");
    let mut values: Vec<rusqlite::types::Value> = Vec::new();
    match b.scope_type.as_str() {
        "project" => {
            sql.push_str(" AND project_id = ?");
            values.push(b.scope_id.clone().unwrap_or_default().into());
        }
        "team" => {
            sql.push_str(" AND team_id = ?");
            values.push(b.scope_id.clone().unwrap_or_default().into());
        }
        "agent" => {
            sql.push_str(" AND agent_id = ?");
            values.push(b.scope_id.clone().unwrap_or_default().into());
        }
        "task" => {
            sql.push_str(" AND task_id = ?");
            values.push(b.scope_id.clone().unwrap_or_default().into());
        }
        _ => {}
    }
    if b.period == "day" {
        sql.push_str(" AND occurred_at >= ?");
        values.push(day_start_ms().into());
    } else if b.period == "month" {
        sql.push_str(" AND occurred_at >= ?");
        values.push(month_start_ms().into());
    }
    let spent: i64 = conn
        .query_row(&sql, rusqlite::params_from_iter(values), |r| r.get(0))
        .unwrap_or(0);
    let reserved: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM budget_reservations
             WHERE budget_id = ?1 AND status = 'active' AND expires_at > ?2",
            params![b.id, now_ms()],
            |r| r.get(0),
        )
        .unwrap_or(0);
    (spent, reserved)
}

fn to_public(conn: &Connection, b: &BudgetRow) -> Budget {
    let (spent, reserved) = spend_for(conn, b);
    let pct = if b.limit_value > 0 {
        spent as f64 / b.limit_value as f64 * 100.0
    } else {
        0.0
    };
    Budget {
        id: b.id.clone(),
        scope_type: b.scope_type.clone(),
        scope_id: b.scope_id.clone(),
        scope_name: scope_name(conn, &b.scope_type, b.scope_id.as_deref()),
        unit: b.unit.clone(),
        period: b.period.clone(),
        limit_value: b.limit_value,
        warn_pct: b.warn_pct,
        hard_stop: b.hard_stop,
        enabled: b.enabled,
        spent,
        reserved,
        pct,
        blocked: b.enabled && b.hard_stop && spent >= b.limit_value && b.limit_value > 0,
        created_at: b.created_at,
    }
}

pub fn list(conn: &Connection) -> Result<Vec<Budget>> {
    Ok(rows(conn)?.iter().map(|b| to_public(conn, b)).collect())
}

#[allow(clippy::too_many_arguments)]
pub fn upsert(
    conn: &Connection,
    id: Option<String>,
    scope_type: &str,
    scope_id: Option<String>,
    unit: &str,
    period: &str,
    limit_value: i64,
    warn_pct: i64,
    hard_stop: bool,
    enabled: Option<bool>,
) -> Result<String> {
    if let Some(existing) = id {
        conn.execute(
            "UPDATE budgets SET scope_type = ?1, scope_id = ?2, unit = ?3, period = ?4,
             limit_value = ?5, warn_pct = ?6, hard_stop = ?7, enabled = COALESCE(?8, enabled)
             WHERE id = ?9",
            params![
                scope_type,
                scope_id,
                unit,
                period,
                limit_value,
                warn_pct,
                hard_stop as i64,
                enabled.map(|v| v as i64),
                existing
            ],
        )?;
        return Ok(existing);
    }
    let new = new_id();
    conn.execute(
        "INSERT INTO budgets (id, scope_type, scope_id, unit, period, limit_value, warn_pct, hard_stop, enabled, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            new,
            scope_type,
            scope_id,
            unit,
            period,
            limit_value,
            warn_pct,
            hard_stop as i64,
            enabled.unwrap_or(true) as i64,
            now_ms()
        ],
    )?;
    Ok(new)
}

pub fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM budgets WHERE id = ?1", params![id])?;
    Ok(())
}

fn matches_scope(
    b: &BudgetRow,
    project_id: &str,
    team_id: &str,
    agent_id: &str,
    task_id: &str,
) -> bool {
    match b.scope_type.as_str() {
        "global" => true,
        "project" => b.scope_id.as_deref() == Some(project_id),
        "team" => b.scope_id.as_deref() == Some(team_id),
        "agent" => b.scope_id.as_deref() == Some(agent_id),
        "task" => b.scope_id.as_deref() == Some(task_id),
        _ => false,
    }
}

pub fn check_dispatch(
    conn: &Connection,
    project_id: &str,
    team_id: &str,
    agent_id: &str,
    task_id: &str,
) -> Result<()> {
    for b in rows(conn)? {
        if !b.enabled || !matches_scope(&b, project_id, team_id, agent_id, task_id) {
            continue;
        }
        let (spent, reserved) = spend_for(conn, &b);
        if b.limit_value > 0 && spent + reserved >= b.limit_value {
            let name = scope_name(conn, &b.scope_type, b.scope_id.as_deref());
            let unit = if b.unit == "usd" { "美元" } else { "tokens" };
            return Err(crate::error::AppError::Msg(format!(
                "预算已用尽：{name}（{unit}上限 {}），请提额或调整预算后再派发",
                format_limit(b.limit_value, &b.unit)
            )));
        }
    }
    Ok(())
}

pub fn release(conn: &Connection, run_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE budget_reservations SET status = 'released' WHERE run_id = ?1 AND status = 'active'",
        params![run_id],
    )?;
    Ok(())
}

/// 一次运行在各维度预算上的作用域，供请求级硬停与看门狗复用。
pub struct BudgetScope<'a> {
    pub project_id: &'a str,
    pub team_id: &'a str,
    pub agent_id: &'a str,
    pub task_id: &'a str,
    pub run_id: &'a str,
}

/// 仅统计启用了硬停的预算，单位换算后的剩余额度。
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Headroom {
    pub usd_micros: Option<i64>,
    pub tokens: Option<i64>,
}

pub fn headroom(conn: &Connection, scope: &BudgetScope) -> Headroom {
    let mut out = Headroom::default();
    let Ok(items) = rows(conn) else {
        return out;
    };
    for b in items {
        if !b.enabled
            || !b.hard_stop
            || b.limit_value <= 0
            || !matches_scope(
                &b,
                scope.project_id,
                scope.team_id,
                scope.agent_id,
                scope.task_id,
            )
        {
            continue;
        }
        let (spent, reserved) = spend_for(conn, &b);
        let remaining = (b.limit_value - spent - reserved).max(0);
        let slot = if b.unit == "usd" {
            &mut out.usd_micros
        } else {
            &mut out.tokens
        };
        *slot = Some(match *slot {
            Some(current) => current.min(remaining),
            None => remaining,
        });
    }
    out
}

/// 只按“已实际发生”的用量判断硬停：在途请求的预留额度不应导致运行被提前终止。
pub fn spent_exhausted(conn: &Connection, scope: &BudgetScope) -> bool {
    let Ok(items) = rows(conn) else {
        return false;
    };
    items.into_iter().any(|b| {
        b.enabled
            && b.hard_stop
            && b.limit_value > 0
            && matches_scope(
                &b,
                scope.project_id,
                scope.team_id,
                scope.agent_id,
                scope.task_id,
            )
            && spend_for(conn, &b).0 >= b.limit_value
    })
}

/// 请求级准入：在 BEGIN IMMEDIATE 事务内重算剩余额度并写入短期预留，
/// 避免并发请求同时看到“还有余额”而一起超支。额度不足直接返回错误。
pub fn admit(
    conn: &Connection,
    scope: &BudgetScope,
    est_micros: i64,
    est_tokens: i64,
) -> Result<Vec<String>> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let outcome = admit_inner(conn, scope, est_micros, est_tokens);
    match &outcome {
        Ok(_) => {
            if let Err(err) = conn.execute_batch("COMMIT") {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(err.into());
            }
        }
        Err(_) => {
            let _ = conn.execute_batch("ROLLBACK");
        }
    }
    outcome
}

fn admit_inner(
    conn: &Connection,
    scope: &BudgetScope,
    est_micros: i64,
    est_tokens: i64,
) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    for b in rows(conn)? {
        if !b.enabled
            || !b.hard_stop
            || b.limit_value <= 0
            || !matches_scope(
                &b,
                scope.project_id,
                scope.team_id,
                scope.agent_id,
                scope.task_id,
            )
        {
            continue;
        }
        let needed = (if b.unit == "usd" {
            est_micros
        } else {
            est_tokens
        })
        .max(0);
        if needed == 0 {
            continue;
        }
        let (spent, reserved) = spend_for(conn, &b);
        let remaining = (b.limit_value - spent - reserved).max(0);
        if needed > remaining {
            let name = scope_name(conn, &b.scope_type, b.scope_id.as_deref());
            return Err(crate::error::AppError::Msg(format!(
                "预算不足：{name} 剩余 {}，本次请求约需 {}",
                format_limit(remaining, &b.unit),
                format_limit(needed, &b.unit)
            )));
        }
        let id = new_id();
        conn.execute(
            "INSERT INTO budget_reservations (id, budget_id, run_id, amount, status, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6)",
            params![id, b.id, scope.run_id, needed, now_ms(), now_ms() + 5 * 60 * 1000],
        )?;
        ids.push(id);
    }
    Ok(ids)
}

/// 结算请求级预留：无论成功失败都要及时释放，避免占住额度到过期。
pub fn settle(conn: &Connection, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql =
        format!("UPDATE budget_reservations SET status = 'settled' WHERE id IN ({placeholders})");
    let _ = conn.execute(&sql, params_from_iter(ids.iter()));
}

/// 预算越限处理：发事件、建审批、通知并终止运行。返回是否触发硬停。
pub fn enforce(
    app: &AppHandle,
    conn: &Connection,
    scope: &BudgetScope,
    registry: &Arc<RunRegistry>,
    notify: bool,
) -> bool {
    let alerts = alerts(
        conn,
        scope.project_id,
        scope.team_id,
        scope.agent_id,
        scope.task_id,
    );
    let mut critical = false;
    for alert in alerts {
        let _ = app.emit("budget.alert", json!(alert));
        if alert.level == "critical" {
            critical = true;
            let title = crate::i18n::tr(
                &format!("预算越限：{}", alert.name),
                &format!("Budget exceeded: {}", alert.name),
            );
            let pending: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM approvals WHERE type = 'budget_exceeded' AND status = 'pending' AND title LIKE ?1",
                    params![format!("%{}%", alert.name)],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if pending == 0 {
                let id = new_id();
                let _ = conn.execute(
                    "INSERT INTO approvals (id, task_id, run_id, type, risk_level, title, detail_json, status, requested_by, created_at, expires_at)
                     VALUES (?1, ?2, ?3, 'budget_exceeded', 'high', ?4, ?5, 'pending', 'budget-engine', ?6, ?7)",
                    params![
                        id,
                        scope.task_id,
                        scope.run_id,
                        title.clone(),
                        json!({ "budget": alert, "runId": scope.run_id }).to_string(),
                        now_ms(),
                        now_ms() + 7 * 24 * 3600 * 1000
                    ],
                );
                let _ = app.emit(
                    "approval.requested",
                    json!({ "approvalId": id, "type": "budget_exceeded", "title": title }),
                );
                if notify {
                    crate::notify::send_task(
                        app,
                        &crate::i18n::tr("Lumen 预算告警", "Lumen budget alert"),
                        &crate::i18n::tr(
                            &format!("{} 已超过预算上限", alert.name),
                            &format!("{} has exceeded its budget limit", alert.name),
                        ),
                        Some(scope.task_id),
                        "budget",
                    );
                }
            }
            registry.cancel(scope.run_id);
        } else if alert.level == "warning" && notify {
            crate::notify::send_task(
                app,
                &crate::i18n::tr("Lumen 预算提醒", "Lumen budget warning"),
                &crate::i18n::tr(
                    &format!("{} 已使用 {:.0}%", alert.name, alert.pct),
                    &format!("{} has used {:.0}%", alert.name, alert.pct),
                ),
                Some(scope.task_id),
                "budget",
            );
        }
    }
    critical
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetAlert {
    pub budget_id: String,
    pub name: String,
    pub pct: f64,
    pub level: String,
    pub unit: String,
    pub limit_value: i64,
    pub spent: i64,
}

pub fn alerts(
    conn: &Connection,
    project_id: &str,
    team_id: &str,
    agent_id: &str,
    task_id: &str,
) -> Vec<BudgetAlert> {
    let mut out = Vec::new();
    if let Ok(items) = rows(conn) {
        for b in items {
            if !b.enabled || !matches_scope(&b, project_id, team_id, agent_id, task_id) {
                continue;
            }
            // 预警与硬停只按已结算的实际用量判断；在途预留用于请求准入，不参与越限判定。
            let (spent, _reserved) = spend_for(conn, &b);
            let pct = if b.limit_value > 0 {
                spent as f64 / b.limit_value as f64 * 100.0
            } else {
                0.0
            };
            if pct >= 100.0 && b.hard_stop {
                out.push(BudgetAlert {
                    budget_id: b.id.clone(),
                    name: scope_name(conn, &b.scope_type, b.scope_id.as_deref()),
                    pct,
                    level: "critical".into(),
                    unit: b.unit.clone(),
                    limit_value: b.limit_value,
                    spent,
                });
            } else if pct >= b.warn_pct as f64 {
                out.push(BudgetAlert {
                    budget_id: b.id.clone(),
                    name: scope_name(conn, &b.scope_type, b.scope_id.as_deref()),
                    pct,
                    level: "warning".into(),
                    unit: b.unit.clone(),
                    limit_value: b.limit_value,
                    spent,
                });
            }
        }
    }
    out
}

pub fn usd_headroom(
    conn: &Connection,
    project_id: &str,
    team_id: &str,
    agent_id: &str,
    task_id: &str,
) -> Option<f64> {
    let mut headroom: Option<f64> = None;
    if let Ok(items) = rows(conn) {
        for b in items {
            if !b.enabled
                || b.unit != "usd"
                || !matches_scope(&b, project_id, team_id, agent_id, task_id)
            {
                continue;
            }
            let (spent, reserved) = spend_for(conn, &b);
            let remaining = (b.limit_value - spent - reserved).max(0);
            let usd = remaining as f64 / 1_000_000.0;
            headroom = Some(match headroom {
                Some(current) => current.min(usd),
                None => usd,
            });
        }
    }
    headroom
}

pub fn format_limit(value: i64, unit: &str) -> String {
    if unit == "usd" {
        format!("${:.2}", value as f64 / 1_000_000.0)
    } else {
        format_tokens(value)
    }
}

pub fn format_tokens(n: i64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.3}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.3}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_budget(conn: &Connection, id: &str, limit: i64, hard_stop: bool) {
        conn.execute(
            "INSERT INTO budgets (id, scope_type, scope_id, unit, period, limit_value, warn_pct, hard_stop, enabled, created_at)
             VALUES (?1, 'global', NULL, 'usd', 'day', ?2, 80, ?3, 1, ?4)",
            params![id, limit, hard_stop as i64, now_ms()],
        )
        .unwrap();
    }

    fn add_token_budget(conn: &Connection, id: &str, limit: i64, hard_stop: bool) {
        conn.execute(
            "INSERT INTO budgets (id, scope_type, scope_id, unit, period, limit_value, warn_pct, hard_stop, enabled, created_at)
             VALUES (?1, 'global', NULL, 'tokens', 'day', ?2, 80, ?3, 1, ?4)",
            params![id, limit, hard_stop as i64, now_ms()],
        )
        .unwrap();
    }

    fn scope<'a>(run_id: &'a str) -> BudgetScope<'a> {
        BudgetScope {
            project_id: "p1",
            team_id: "t1",
            agent_id: "a1",
            task_id: "k1",
            run_id,
        }
    }

    #[test]
    fn admit_blocks_over_budget_and_settle_releases() {
        let conn = crate::db::test_connection();
        add_budget(&conn, "b1", 1_000_000, true);
        let s = scope("r1");
        assert_eq!(headroom(&conn, &s).usd_micros, Some(1_000_000));
        let ids = admit(&conn, &s, 600_000, 6_000).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(headroom(&conn, &s).usd_micros, Some(400_000));
        assert!(admit(&conn, &s, 500_000, 5_000).is_err());
        settle(&conn, &ids);
        assert_eq!(headroom(&conn, &s).usd_micros, Some(1_000_000));
    }

    #[test]
    fn soft_budget_never_blocks_requests() {
        let conn = crate::db::test_connection();
        add_budget(&conn, "b2", 100, false);
        let s = scope("r2");
        assert!(headroom(&conn, &s).usd_micros.is_none());
        assert!(admit(&conn, &s, 10_000, 100).unwrap().is_empty());
    }

    #[test]
    fn spent_exhausted_ignores_inflight_reservations() {
        let conn = crate::db::test_connection();
        add_budget(&conn, "b3", 1_000_000, true);
        let s = scope("r3");
        let _ids = admit(&conn, &s, 900_000, 9_000).unwrap();
        // 预留不算实际消耗，不应触发硬停。
        assert!(!spent_exhausted(&conn, &s));
        conn.execute(
            "INSERT INTO usage_events (id, project_id, team_id, agent_id, task_id, cost_micros, usage_source, dedupe_key, occurred_at)
             VALUES ('u1', 'p1', 't1', 'a1', 'k1', 1_000_000, 'proxy', 'k1', ?1)",
            params![now_ms()],
        )
        .unwrap();
        assert!(spent_exhausted(&conn, &s));
    }

    #[test]
    fn alerts_and_public_pct_use_settled_spend() {
        let conn = crate::db::test_connection();
        add_token_budget(&conn, "b4", 200_000, true);
        conn.execute(
            "INSERT INTO usage_events (id, project_id, team_id, agent_id, task_id, input_tokens, cost_micros, usage_source, dedupe_key, occurred_at)
             VALUES ('u1', 'p1', 't1', 'a1', 'k1', 181_970, 0, 'proxy', 'k1', ?1)",
            params![now_ms()],
        )
        .unwrap();
        // 模拟一次在途请求预留：它把 spent+reserved 推过上限，但实际消耗未超限。
        conn.execute(
            "INSERT INTO budget_reservations (id, budget_id, run_id, amount, status, created_at, expires_at)
             VALUES ('r1', 'b4', 'r4', 20_000, 'active', ?1, ?2)",
            params![now_ms(), now_ms() + 5 * 60 * 1000],
        )
        .unwrap();

        let items = alerts(&conn, "p1", "t1", "a1", "k1");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].level, "warning");
        assert_eq!(items[0].spent, 181_970);
        assert!((items[0].pct - 90.985).abs() < 0.01);

        let public = list(&conn).unwrap();
        assert_eq!(public[0].reserved, 20_000);
        assert!((public[0].pct - 90.985).abs() < 0.01);
        assert!(!public[0].blocked);

        conn.execute(
            "INSERT INTO usage_events (id, project_id, team_id, agent_id, task_id, input_tokens, cost_micros, usage_source, dedupe_key, occurred_at)
             VALUES ('u2', 'p1', 't1', 'a1', 'k1', 18_030, 0, 'proxy', 'k2', ?1)",
            params![now_ms()],
        )
        .unwrap();
        let items = alerts(&conn, "p1", "t1", "a1", "k1");
        assert_eq!(items[0].level, "critical");
        assert_eq!(items[0].spent, 200_000);
        assert!(list(&conn).unwrap()[0].blocked);
    }
}
