use crate::crypto;
use crate::db::{new_id, now_ms, Db};
use crate::error::{AppError, Result};
use crate::models::{Provider, ProviderTestResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const PROTOCOLS: &[&str] = &["anthropic", "openai"];

pub fn list(conn: &Connection) -> Result<Vec<Provider>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, protocol, base_url, model, models_json, key_hint, enabled, created_at, updated_at
         FROM providers ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Provider {
            id: r.get(0)?,
            name: r.get(1)?,
            protocol: r.get(2)?,
            base_url: r.get(3)?,
            model: r.get(4)?,
            models: parse_models(r.get::<_, String>(5)?.as_str()),
            key_hint: r.get(6)?,
            enabled: r.get::<_, i64>(7)? != 0,
            created_at: r.get(8)?,
            updated_at: r.get(9)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn parse_models(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn normalize_models(models: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in models {
        let model = raw.trim();
        if !model.is_empty() && !out.iter().any(|seen| seen == model) {
            out.push(model.to_string());
        }
    }
    out
}

fn validate(protocol: &str, base_url: &str) -> Result<()> {
    if !PROTOCOLS.contains(&protocol) {
        return Err(AppError::Msg("协议必须是 anthropic 或 openai".into()));
    }
    let url = base_url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AppError::Msg(
            "Base URL 必须以 http:// 或 https:// 开头".into(),
        ));
    }
    // 明文 HTTP 只允许本机回环，避免 API Key 在网络上明文传输。
    if url.starts_with("http://") {
        let loopback = url.starts_with("http://localhost")
            || url.starts_with("http://127.0.0.1")
            || url.starts_with("http://[::1]");
        if !loopback {
            return Err(AppError::Msg(
                "远程供应商必须使用 https://（仅本机回环允许 http）".into(),
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn upsert(
    conn: &Connection,
    id: Option<String>,
    name: String,
    protocol: String,
    base_url: String,
    model: Option<String>,
    models: Option<Vec<String>>,
    api_key: Option<String>,
    enabled: bool,
) -> Result<String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Msg("供应商名称不能为空".into()));
    }
    let protocol = protocol.trim().to_lowercase();
    let base_url = base_url.trim().trim_end_matches('/').to_string();
    validate(&protocol, &base_url)?;
    let model = model.filter(|m| !m.trim().is_empty());
    // None 表示本次不修改已保存的模型列表（例如列表页直接切换启用状态）。
    let models_json = models.map(|list| {
        serde_json::to_string(&normalize_models(&list)).unwrap_or_else(|_| "[]".into())
    });
    let new_key = api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    let encrypted = match &new_key {
        Some(key) => Some(crypto::to_hex(&crypto::protect(key)?)),
        None => None,
    };
    let hint = new_key.as_deref().map(crypto::mask);

    if let Some(id) = id {
        let changed = conn.execute(
            "UPDATE providers SET name = ?1, protocol = ?2, base_url = ?3, model = ?4,
             models_json = COALESCE(?5, models_json),
             key_cipher = COALESCE(?6, key_cipher), key_hint = COALESCE(?7, key_hint),
             enabled = ?8, updated_at = ?9 WHERE id = ?10",
            params![
                name,
                protocol,
                base_url,
                model,
                models_json,
                encrypted,
                hint,
                enabled as i64,
                now_ms(),
                id
            ],
        )?;
        if changed == 0 {
            return Err(AppError::Msg("供应商不存在".into()));
        }
        Ok(id)
    } else {
        let id = new_id();
        conn.execute(
            "INSERT INTO providers (id, name, protocol, base_url, model, models_json, key_cipher, key_hint, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                id,
                name,
                protocol,
                base_url,
                model,
                models_json.unwrap_or_else(|| "[]".to_string()),
                encrypted,
                hint,
                enabled as i64,
                now_ms()
            ],
        )?;
        Ok(id)
    }
}

pub fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE agents SET provider_id = NULL WHERE provider_id = ?1",
        params![id],
    )?;
    conn.execute("DELETE FROM providers WHERE id = ?1", params![id])?;
    Ok(())
}

fn decode_key(cipher: Option<String>) -> Result<String> {
    let Some(cipher) = cipher.filter(|c| !c.is_empty()) else {
        return Ok(String::new());
    };
    let bytes = crypto::from_hex(&cipher).ok_or_else(crypto::missing_key_error)?;
    crypto::unprotect(&bytes)
}

fn row_provider(conn: &Connection, id: &str) -> Result<Option<(Provider, String)>> {
    let row = conn
        .query_row(
            "SELECT id, name, protocol, base_url, model, models_json, key_hint, enabled, created_at, updated_at, key_cipher
             FROM providers WHERE id = ?1",
            params![id],
            |r| {
                Ok((
                    Provider {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        protocol: r.get(2)?,
                        base_url: r.get(3)?,
                        model: r.get(4)?,
                        models: parse_models(r.get::<_, String>(5)?.as_str()),
                        key_hint: r.get(6)?,
                        enabled: r.get::<_, i64>(7)? != 0,
                        created_at: r.get(8)?,
                        updated_at: r.get(9)?,
                    },
                    r.get::<_, Option<String>>(10)?,
                ))
            },
        )
        .optional()?;
    match row {
        Some((provider, cipher)) => {
            let key = decode_key(cipher)?;
            Ok(Some((provider, key)))
        }
        None => Ok(None),
    }
}

pub type ProviderEnv = (Vec<(String, String)>, Vec<String>);

/// 只以环境变量形式交给子进程，既不进命令行也不进日志。
pub fn env_for_agent(conn: &Connection, agent_id: &str) -> Result<ProviderEnv> {
    let provider_id: Option<String> = conn
        .query_row(
            "SELECT provider_id FROM agents WHERE id = ?1",
            params![agent_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(provider_id) = provider_id else {
        return Ok((Vec::new(), Vec::new()));
    };
    let Some((provider, key)) = row_provider(conn, &provider_id)? else {
        return Ok((Vec::new(), Vec::new()));
    };
    if !provider.enabled {
        return Ok((Vec::new(), Vec::new()));
    }

    let mut envs: Vec<(String, String)> = Vec::new();
    let mut removals: Vec<String> = Vec::new();
    match provider.protocol.as_str() {
        "anthropic" => {
            envs.push(("ANTHROPIC_BASE_URL".into(), provider.base_url.clone()));
            removals.push("ANTHROPIC_API_KEY".into());
            if key.is_empty() {
                removals.push("ANTHROPIC_AUTH_TOKEN".into());
            } else {
                envs.push(("ANTHROPIC_AUTH_TOKEN".into(), key));
            }
            if let Some(model) = provider.model.filter(|m| !m.trim().is_empty()) {
                envs.push(("ANTHROPIC_MODEL".into(), model));
            }
        }
        "openai" => {
            envs.push(("OPENAI_BASE_URL".into(), provider.base_url.clone()));
            removals.push("OPENAI_API_BASE".into());
            if key.is_empty() {
                removals.push("OPENAI_API_KEY".into());
            } else {
                envs.push(("OPENAI_API_KEY".into(), key));
            }
            if let Some(model) = provider.model.filter(|m| !m.trim().is_empty()) {
                envs.push(("OPENAI_MODEL".into(), model));
            }
        }
        _ => {}
    }
    Ok((envs, removals))
}

/// 供本地代理使用的供应商解析结果：真实 Key 只留在代理进程内存中。
pub struct ResolvedProvider {
    pub protocol: String,
    pub base_url: String,
    pub model: Option<String>,
    pub api_key: String,
}

pub fn resolved(conn: &Connection, agent_id: &str) -> Result<Option<ResolvedProvider>> {
    let provider_id: Option<String> = conn
        .query_row(
            "SELECT provider_id FROM agents WHERE id = ?1",
            params![agent_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(provider_id) = provider_id else {
        return Ok(None);
    };
    let Some((provider, key)) = row_provider(conn, &provider_id)? else {
        return Ok(None);
    };
    if !provider.enabled {
        return Ok(None);
    }
    Ok(Some(ResolvedProvider {
        protocol: provider.protocol,
        base_url: provider.base_url,
        model: provider.model.filter(|m| !m.trim().is_empty()),
        api_key: key,
    }))
}

fn endpoint(base: &str, suffix: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}{suffix}")
    } else {
        format!("{base}/v1{suffix}")
    }
}

fn clean_error(body: &str) -> String {
    let text = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error").map(|e| {
                e.get("message")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| e.to_string())
            })
        })
        .unwrap_or_else(|| body.trim().to_string());
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(240).collect()
}

pub fn test(db: &Db, id: &str) -> Result<ProviderTestResult> {
    let conn = db.open()?;
    let (provider, key) =
        row_provider(&conn, id)?.ok_or_else(|| AppError::Msg("供应商不存在".into()))?;
    let model = provider
        .model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| AppError::Msg("请先填写模型名称再测试连接".into()))?;

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build();
    let started = Instant::now();
    let result = match provider.protocol.as_str() {
        "anthropic" => {
            let url = endpoint(&provider.base_url, "/messages");
            let mut req = agent
                .post(&url)
                .set("content-type", "application/json")
                .set("anthropic-version", "2023-06-01");
            if !key.is_empty() {
                req = req.set("x-api-key", &key);
            }
            req.send_json(json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": "ping" }]
            }))
        }
        _ => {
            let url = endpoint(&provider.base_url, "/chat/completions");
            let mut req = agent.post(&url).set("content-type", "application/json");
            if !key.is_empty() {
                req = req.set("authorization", &format!("Bearer {key}"));
            }
            req.send_json(json!({
                "model": model,
                "max_tokens": 1,
                "stream": false,
                "messages": [{ "role": "user", "content": "ping" }]
            }))
        }
    };
    let latency_ms = started.elapsed().as_millis() as i64;

    let (ok, status, message) = match result {
        Ok(resp) => {
            let status = resp.status() as i64;
            (true, status, "连接成功".to_string())
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            let message = crypto::redact(&clean_error(&body), &key);
            (
                false,
                code as i64,
                if message.is_empty() {
                    format!("HTTP {code}")
                } else {
                    message
                },
            )
        }
        Err(err) => (false, 0, crypto::redact(&err.to_string(), &key)),
    };

    Ok(ProviderTestResult {
        ok,
        status,
        latency_ms,
        message,
    })
}

/// 通过 Base URL 与 Key 拉取供应商可用的模型列表。
///
/// 编辑已有供应商时，`api_key` 留空会复用数据库中的密钥，避免用户重复粘贴。
pub fn fetch_models(
    db: &Db,
    id: Option<String>,
    protocol: String,
    base_url: String,
    api_key: Option<String>,
) -> Result<Vec<String>> {
    let protocol = protocol.trim().to_lowercase();
    let base_url = base_url.trim().trim_end_matches('/').to_string();
    validate(&protocol, &base_url)?;
    let typed = api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    let key = match (typed, id) {
        (Some(key), _) => key,
        (None, Some(id)) => {
            let conn = db.open()?;
            row_provider(&conn, &id)?
                .ok_or_else(|| AppError::Msg("供应商不存在".into()))?
                .1
        }
        (None, None) => String::new(),
    };

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build();
    let url = endpoint(&base_url, "/models");
    let result = match protocol.as_str() {
        "anthropic" => {
            let mut req = agent.get(&url).set("anthropic-version", "2023-06-01");
            if !key.is_empty() {
                req = req.set("x-api-key", &key);
            }
            req.call()
        }
        _ => {
            let mut req = agent.get(&url);
            if !key.is_empty() {
                req = req.set("authorization", &format!("Bearer {key}"));
            }
            req.call()
        }
    };

    let body = match result {
        Ok(resp) => resp.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            let message = crypto::redact(&clean_error(&body), &key);
            return Err(AppError::Msg(if message.is_empty() {
                format!("获取模型失败（HTTP {code}）")
            } else {
                format!("获取模型失败（HTTP {code}）：{message}")
            }));
        }
        Err(err) => {
            return Err(AppError::Msg(format!(
                "获取模型失败：{}",
                crypto::redact(&err.to_string(), &key)
            )));
        }
    };
    let value: Value =
        serde_json::from_str(&body).map_err(|_| AppError::Msg("模型列表不是有效的 JSON".into()))?;
    let models = extract_models(&value);
    if models.is_empty() {
        return Err(AppError::Msg("该供应商未返回任何模型".into()));
    }
    Ok(models)
}

/// 兼容常见模型列表结构：`{data:[{id}]}`、`{models:[{name}]}` 或直接数组。
fn extract_models(value: &Value) -> Vec<String> {
    let list = value
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| value.get("models").and_then(Value::as_array))
        .or_else(|| value.as_array());
    let Some(list) = list else {
        return Vec::new();
    };
    let mut models: Vec<String> = Vec::new();
    for item in list {
        let id = item
            .as_str()
            .map(str::to_string)
            .or_else(|| item.get("id").and_then(Value::as_str).map(str::to_string))
            .or_else(|| item.get("name").and_then(Value::as_str).map(str::to_string));
        if let Some(id) = id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            if !models.contains(&id) {
                models.push(id);
            }
        }
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_models_supports_common_shapes() {
        let openai = json!({ "data": [{ "id": "gpt-a" }, { "id": "gpt-b" }, { "id": "gpt-a" }] });
        assert_eq!(extract_models(&openai), vec!["gpt-a", "gpt-b"]);

        let anthropic = json!({ "data": [{ "id": "claude-a" }, { "id": "claude-b" }] });
        assert_eq!(extract_models(&anthropic), vec!["claude-a", "claude-b"]);

        let named = json!({ "models": [{ "name": "local-a" }, "local-b"] });
        assert_eq!(extract_models(&named), vec!["local-a", "local-b"]);

        let bare = json!(["x", "y", "x"]);
        assert_eq!(extract_models(&bare), vec!["x", "y"]);

        assert!(extract_models(&json!({ "error": "nope" })).is_empty());
    }

    #[test]
    fn normalize_models_trims_and_dedupes() {
        let input = vec![
            "  a ".to_string(),
            String::new(),
            "b".to_string(),
            "a".to_string(),
        ];
        assert_eq!(normalize_models(&input), vec!["a", "b"]);
    }
}
