use crate::db::{log_event, now_ms, Db};
use crate::error::Result;
use crate::models::{ScopedUsage, UsagePoint, UsageSummary};
use rusqlite::{params, Connection};
use serde_json::json;

#[derive(Debug, Clone, Default)]
pub struct UsageInput {
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub project_id: Option<String>,
    pub team_id: Option<String>,
    pub seat_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub source: String,
    pub dedupe_key: String,
}

pub fn provider_for(model: &str) -> String {
    let m = model.to_lowercase();
    if m.contains("claude") {
        "anthropic".into()
    } else if m.contains("gpt") || m.contains("o1") || m.contains("o3") {
        "openai".into()
    } else if m.contains("deepseek") {
        "deepseek".into()
    } else if m.contains("gemini") {
        "google".into()
    } else if m.contains("kimi") || m.contains("moonshot") {
        "moonshot".into()
    } else if m.contains("glm") {
        "zhipu".into()
    } else if m.contains("qwen") {
        "alibaba".into()
    } else {
        "unknown".into()
    }
}

pub fn price_for(conn: &Connection, provider: &str, model: &str) -> Result<(i64, i64, i64, i64)> {
    let m = model.to_lowercase();
    let mut stmt = conn.prepare(
        "SELECT provider, model_pattern, input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok
         FROM prices",
    )?;
    let mut best: Option<(usize, bool, i64, i64, i64, i64)> = None;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    for row in rows {
        let (p, pattern, i, o, cr, cw) = row?;
        let pat = pattern.to_lowercase();
        if !m.contains(&pat) {
            continue;
        }
        let provider_match = p == provider;
        let better = match &best {
            None => true,
            Some((len, pm, _, _, _, _)) => {
                pat.len() > *len || (pat.len() == *len && provider_match && !*pm)
            }
        };
        if better {
            best = Some((pat.len(), provider_match, i, o, cr, cw));
        }
    }
    if let Some((_, _, i, o, cr, cw)) = best {
        Ok((i, o, cr, cw))
    } else {
        Ok((3_000_000, 15_000_000, 300_000, 3_750_000))
    }
}

pub fn compute_cost_micros(
    conn: &Connection,
    provider: &str,
    model: &str,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
) -> Result<i64> {
    let (pi, po, pcr, pcw) = price_for(conn, provider, model)?;
    let total: i128 = input as i128 * pi as i128
        + output as i128 * po as i128
        + cache_read as i128 * pcr as i128
        + cache_write as i128 * pcw as i128;
    Ok((total / 1_000_000) as i64)
}

pub fn record(conn: &Connection, u: &UsageInput) -> Result<Option<i64>> {
    if u.input_tokens == 0
        && u.output_tokens == 0
        && u.cache_read_tokens == 0
        && u.cache_write_tokens == 0
    {
        return Ok(None);
    }
    let provider = if u.provider.is_empty() {
        provider_for(&u.model)
    } else {
        u.provider.clone()
    };
    let cost = compute_cost_micros(
        conn,
        &provider,
        &u.model,
        u.input_tokens,
        u.output_tokens,
        u.cache_read_tokens,
        u.cache_write_tokens,
    )?;
    let id = crate::db::new_id();
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO usage_events
         (id, run_id, task_id, project_id, team_id, seat_id, agent_id, provider, model,
          input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_micros,
          usage_source, dedupe_key, occurred_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            id,
            u.run_id,
            u.task_id,
            u.project_id,
            u.team_id,
            u.seat_id,
            u.agent_id,
            provider,
            u.model,
            u.input_tokens,
            u.output_tokens,
            u.cache_read_tokens,
            u.cache_write_tokens,
            cost,
            u.source,
            u.dedupe_key,
            now_ms()
        ],
    )?;
    if inserted == 0 {
        return Ok(None);
    }
    if let Some(run_id) = &u.run_id {
        conn.execute(
            "UPDATE runs SET input_tokens = input_tokens + ?1, output_tokens = output_tokens + ?2,
             cost_micros = cost_micros + ?3 WHERE id = ?4",
            params![u.input_tokens, u.output_tokens, cost, run_id],
        )?;
    }
    if let Some(task_id) = &u.task_id {
        let total = u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens;
        conn.execute(
            "UPDATE tasks SET actual_tokens = actual_tokens + ?1, actual_cost_micros = actual_cost_micros + ?2
             WHERE id = ?3",
            params![total, cost, task_id],
        )?;
    }
    log_event(
        conn,
        u.project_id.as_deref(),
        u.task_id.as_deref(),
        u.run_id.as_deref(),
        "usage.recorded",
        Some("agent"),
        json!({
            "model": u.model,
            "provider": provider,
            "inputTokens": u.input_tokens,
            "outputTokens": u.output_tokens,
            "cacheReadTokens": u.cache_read_tokens,
            "cacheWriteTokens": u.cache_write_tokens,
            "costMicros": cost,
            "source": u.source
        }),
    )?;
    Ok(Some(cost))
}

pub fn summary(conn: &Connection) -> Result<UsageSummary> {
    let day_start = crate::budget::day_start_ms();
    let total_tokens: i64 = conn.query_row(
        "SELECT COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens), 0) FROM usage_events",
        [],
        |r| r.get(0),
    )?;
    let today_tokens: i64 = conn.query_row(
        "SELECT COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens), 0)
         FROM usage_events WHERE occurred_at >= ?1",
        params![day_start],
        |r| r.get(0),
    )?;
    let total_cost: i64 = conn.query_row(
        "SELECT COALESCE(SUM(cost_micros), 0) FROM usage_events",
        [],
        |r| r.get(0),
    )?;
    let today_cost: i64 = conn.query_row(
        "SELECT COALESCE(SUM(cost_micros), 0) FROM usage_events WHERE occurred_at >= ?1",
        params![day_start],
        |r| r.get(0),
    )?;
    let cache_read: i64 = conn.query_row(
        "SELECT COALESCE(SUM(cache_read_tokens), 0) FROM usage_events",
        [],
        |r| r.get(0),
    )?;
    let input_total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(input_tokens + cache_read_tokens + cache_write_tokens), 0) FROM usage_events",
        [], |r| r.get(0))?;
    let cache_hit_pct = if input_total > 0 {
        (cache_read as f64 / input_total as f64) * 100.0
    } else {
        0.0
    };
    let rounds: i64 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE role = 'assistant' AND kind = 'chat'",
        [],
        |r| r.get(0),
    )?;
    let today_rounds: i64 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE role = 'assistant' AND kind = 'chat' AND created_at >= ?1",
        params![day_start],
        |r| r.get(0),
    )?;
    Ok(UsageSummary {
        today_tokens,
        total_tokens,
        today_cost_micros: today_cost,
        total_cost_micros: total_cost,
        cache_hit_pct,
        rounds,
        today_rounds,
    })
}

pub fn series(conn: &Connection, days: i64) -> Result<Vec<UsagePoint>> {
    let start = crate::budget::day_start_ms() - (days - 1) * 86_400_000;
    let mut stmt = conn.prepare(
        "SELECT date(occurred_at / 1000, 'unixepoch', 'localtime') AS d,
                SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens),
                SUM(cost_micros)
         FROM usage_events WHERE occurred_at >= ?1 GROUP BY d ORDER BY d",
    )?;
    let rows = stmt.query_map(params![start], |r| {
        Ok(UsagePoint {
            date: r.get(0)?,
            tokens: r.get(1)?,
            cost_micros: r.get(2)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn by_agent(conn: &Connection) -> Result<Vec<ScopedUsage>> {
    let day_start = crate::budget::day_start_ms();
    let mut stmt = conn.prepare(
        "SELECT a.id, a.name,
                COALESCE(SUM(u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens), 0),
                COALESCE(SUM(u.cost_micros), 0),
                COALESCE(SUM(CASE WHEN u.occurred_at >= ?1 THEN 1 ELSE 0 END), 0)
         FROM agents a LEFT JOIN usage_events u ON u.agent_id = a.id
         GROUP BY a.id, a.name ORDER BY 3 DESC",
    )?;
    let rows = stmt.query_map(params![day_start], |r| {
        Ok(ScopedUsage {
            key: r.get(0)?,
            name: r.get(1)?,
            tokens: r.get(2)?,
            cost_micros: r.get(3)?,
            rounds: r.get(4)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn by_project(conn: &Connection) -> Result<Vec<ScopedUsage>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name,
                COALESCE(SUM(u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens), 0),
                COALESCE(SUM(u.cost_micros), 0),
                COALESCE(COUNT(u.id), 0)
         FROM projects p LEFT JOIN usage_events u ON u.project_id = p.id
         GROUP BY p.id, p.name ORDER BY 3 DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ScopedUsage {
            key: r.get(0)?,
            name: r.get(1)?,
            tokens: r.get(2)?,
            cost_micros: r.get(3)?,
            rounds: r.get(4)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn agent_stats(conn: &Connection) -> Result<Vec<crate::models::AgentStat>> {
    let day_start = crate::budget::day_start_ms();
    let mut stmt = conn.prepare(
        "SELECT a.id, a.name, a.adapter, a.status,
                COALESCE(SUM(u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens), 0) AS tokens,
                COALESCE(SUM(CASE WHEN u.occurred_at >= ?1 THEN u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens ELSE 0 END), 0) AS today_tokens,
                COALESCE(SUM(u.cost_micros), 0),
                (SELECT COUNT(*) FROM messages m WHERE m.agent_id = a.id AND m.role = 'assistant' AND m.kind = 'chat')
         FROM agents a LEFT JOIN usage_events u ON u.agent_id = a.id
         GROUP BY a.id ORDER BY tokens DESC",
    )?;
    let rows = stmt.query_map(params![day_start], |r| {
        Ok(crate::models::AgentStat {
            id: r.get(0)?,
            name: r.get(1)?,
            adapter: r.get(2)?,
            status: r.get(3)?,
            tokens: r.get(4)?,
            today_tokens: r.get(5)?,
            cost_micros: r.get(6)?,
            rounds: r.get(7)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn price_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM prices", [], |r| r.get(0))?)
}

pub fn db_path_label(db: &Db) -> String {
    db.path.to_string_lossy().to_string()
}
