use crate::db::{new_id, now_ms};
use crate::error::{AppError, Result};
use crate::models::Extension;
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

const KINDS: &[&str] = &["mcp", "hook", "skill", "plugin"];
const HOOK_EVENTS: &[&str] = &[
    "PreToolUse",
    "PostToolUse",
    "UserPromptSubmit",
    "Stop",
    "SubagentStop",
    "Notification",
    "PreCompact",
    "SessionStart",
    "SessionEnd",
];

pub fn list(conn: &Connection) -> Result<Vec<Extension>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, name, description, config_json, enabled, created_at, updated_at
         FROM extensions ORDER BY kind ASC, created_at ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        let config: String = r.get(4)?;
        Ok(Extension {
            id: r.get(0)?,
            kind: r.get(1)?,
            name: r.get(2)?,
            description: r.get(3)?,
            config: serde_json::from_str(&config).unwrap_or(Value::Null),
            enabled: r.get::<_, i64>(5)? != 0,
            created_at: r.get(6)?,
            updated_at: r.get(7)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn enabled_of(conn: &Connection, kind: &str) -> Result<Vec<Extension>> {
    Ok(list(conn)?
        .into_iter()
        .filter(|e| e.enabled && e.kind == kind)
        .collect())
}

fn str_of<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(|v| v.as_str())
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn string_map(value: Option<&Value>) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(obj) = value.and_then(|v| v.as_object()) {
        for (key, val) in obj {
            if let Some(text) = val.as_str() {
                if !text.trim().is_empty() {
                    out.insert(key.clone(), Value::String(text.to_string()));
                }
            }
        }
    }
    out
}

fn sanitize_slug(raw: &str) -> String {
    let mut slug = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 48 {
            break;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "k-skill".to_string()
    } else {
        slug
    }
}

fn normalize(kind: &str, name: &str, config: Value) -> Result<Value> {
    let obj = config.as_object().cloned().unwrap_or_default();
    match kind {
        "mcp" => {
            let transport = str_of(&obj, "transport")
                .unwrap_or("stdio")
                .trim()
                .to_lowercase();
            if transport == "stdio" {
                let command = str_of(&obj, "command").unwrap_or("").trim();
                if command.is_empty() {
                    return Err(AppError::Msg("stdio 类型的 MCP 需要填写启动命令".into()));
                }
                Ok(json!({
                    "transport": "stdio",
                    "command": command,
                    "args": string_list(obj.get("args")),
                    "env": string_map(obj.get("env")),
                }))
            } else if transport == "http" || transport == "sse" {
                let url = str_of(&obj, "url").unwrap_or("").trim();
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err(AppError::Msg("远程 MCP 需要填写 http/https 地址".into()));
                }
                Ok(json!({
                    "transport": transport,
                    "url": url,
                    "headers": string_map(obj.get("headers")),
                }))
            } else {
                Err(AppError::Msg(
                    "MCP 传输方式必须是 stdio、http 或 sse".into(),
                ))
            }
        }
        "hook" => {
            let event = str_of(&obj, "event")
                .unwrap_or("PreToolUse")
                .trim()
                .to_string();
            if !HOOK_EVENTS.contains(&event.as_str()) {
                return Err(AppError::Msg("不支持的 Hook 事件".into()));
            }
            let command = str_of(&obj, "command").unwrap_or("").trim();
            if command.is_empty() {
                return Err(AppError::Msg("Hook 需要填写执行命令".into()));
            }
            let timeout = obj
                .get("timeout")
                .and_then(|v| v.as_i64())
                .unwrap_or(60)
                .clamp(1, 3600);
            Ok(json!({
                "event": event,
                "matcher": str_of(&obj, "matcher").unwrap_or("").trim(),
                "command": command,
                "timeout": timeout,
            }))
        }
        "skill" => {
            let content = str_of(&obj, "content").unwrap_or("").trim();
            if content.is_empty() {
                return Err(AppError::Msg("Skill 内容不能为空".into()));
            }
            let slug = sanitize_slug(
                str_of(&obj, "slug")
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(name),
            );
            let description = str_of(&obj, "description")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| name.trim().to_string());
            Ok(json!({
                "slug": slug,
                "description": description,
                "content": content,
            }))
        }
        "plugin" => {
            let path = str_of(&obj, "path").unwrap_or("").trim();
            if path.is_empty() {
                return Err(AppError::Msg("请填写插件目录或 .zip 路径".into()));
            }
            if !Path::new(path).exists() {
                return Err(AppError::Msg("插件路径不存在".into()));
            }
            Ok(json!({ "path": path }))
        }
        _ => Err(AppError::Msg(
            "扩展类型必须是 mcp、hook、skill 或 plugin".into(),
        )),
    }
}

pub fn upsert(
    conn: &Connection,
    id: Option<String>,
    kind: String,
    name: String,
    description: Option<String>,
    config: Value,
    enabled: bool,
) -> Result<String> {
    let kind = kind.trim().to_lowercase();
    if !KINDS.contains(&kind.as_str()) {
        return Err(AppError::Msg("未知的扩展类型".into()));
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Msg("扩展名称不能为空".into()));
    }
    let config = normalize(&kind, &name, config)?;
    let description = description.filter(|d| !d.trim().is_empty());

    if let Some(id) = id {
        let changed = conn.execute(
            "UPDATE extensions SET kind = ?1, name = ?2, description = ?3, config_json = ?4,
             enabled = ?5, updated_at = ?6 WHERE id = ?7",
            params![
                kind,
                name,
                description,
                config.to_string(),
                enabled as i64,
                now_ms(),
                id
            ],
        )?;
        if changed == 0 {
            return Err(AppError::Msg("扩展不存在".into()));
        }
        Ok(id)
    } else {
        let id = new_id();
        conn.execute(
            "INSERT INTO extensions (id, kind, name, description, config_json, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![id, kind, name, description, config.to_string(), enabled as i64, now_ms()],
        )?;
        Ok(id)
    }
}

pub fn toggle(conn: &Connection, id: &str, enabled: bool) -> Result<()> {
    conn.execute(
        "UPDATE extensions SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
        params![enabled as i64, now_ms(), id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, base_dir: &Path, id: &str) -> Result<()> {
    let kind: Option<String> = conn
        .query_row(
            "SELECT kind FROM extensions WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .ok();
    conn.execute("DELETE FROM extensions WHERE id = ?1", params![id])?;
    if kind.as_deref() == Some("skill") {
        let dir = base_dir.join("extensions").join("skills").join(id);
        if dir.is_dir() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    Ok(())
}

/// 供 Claude Code 的 --settings 合并使用；只生成 Lumen 自己的配置内容。
pub fn extra_hooks(conn: &Connection) -> Result<Value> {
    let mut map = Map::new();
    for ext in enabled_of(conn, "hook")? {
        let Some(cfg) = ext.config.as_object() else {
            continue;
        };
        let event = cfg
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("PreToolUse");
        let command = cfg.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if command.is_empty() {
            continue;
        }
        let mut entry = Map::new();
        if let Some(matcher) = cfg
            .get("matcher")
            .and_then(|v| v.as_str())
            .filter(|m| !m.is_empty())
        {
            entry.insert("matcher".into(), Value::String(matcher.to_string()));
        }
        let timeout = cfg.get("timeout").and_then(|v| v.as_i64()).unwrap_or(60);
        entry.insert(
            "hooks".into(),
            json!([{ "type": "command", "command": command, "timeout": timeout }]),
        );
        let slot = map
            .entry(event.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(arr) = slot.as_array_mut() {
            arr.push(Value::Object(entry));
        }
    }
    Ok(Value::Object(map))
}

/// 生成仅本次运行有效的 MCP 配置，配合 --strict-mcp-config 使用。
pub fn mcp_config(conn: &Connection, run_dir: &Path) -> Result<Option<PathBuf>> {
    let mut servers = Map::new();
    for ext in enabled_of(conn, "mcp")? {
        let Some(cfg) = ext.config.as_object() else {
            continue;
        };
        let transport = cfg
            .get("transport")
            .and_then(|v| v.as_str())
            .unwrap_or("stdio");
        let mut server = Map::new();
        if transport == "stdio" {
            let command = cfg.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if command.is_empty() {
                continue;
            }
            server.insert("command".into(), Value::String(command.to_string()));
            let args = string_list(cfg.get("args"));
            if !args.is_empty() {
                server.insert("args".into(), json!(args));
            }
            if let Some(env) = cfg
                .get("env")
                .and_then(|v| v.as_object())
                .filter(|m| !m.is_empty())
            {
                server.insert("env".into(), Value::Object(env.clone()));
            }
        } else {
            let url = cfg.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if url.is_empty() {
                continue;
            }
            server.insert("type".into(), Value::String(transport.to_string()));
            server.insert("url".into(), Value::String(url.to_string()));
            if let Some(headers) = cfg
                .get("headers")
                .and_then(|v| v.as_object())
                .filter(|m| !m.is_empty())
            {
                server.insert("headers".into(), Value::Object(headers.clone()));
            }
        }
        servers.insert(ext.name.clone(), Value::Object(server));
    }
    if servers.is_empty() {
        return Ok(None);
    }
    let path = run_dir.join("mcp.json");
    std::fs::write(&path, json!({ "mcpServers": servers }).to_string())?;
    Ok(Some(path))
}

fn write_skill_plugin(base_dir: &Path, ext: &Extension) -> Result<Option<PathBuf>> {
    let Some(cfg) = ext.config.as_object() else {
        return Ok(None);
    };
    let slug = cfg
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or("k-skill");
    let description = cfg
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or(&ext.name);
    let content = cfg.get("content").and_then(|v| v.as_str()).unwrap_or("");
    if content.trim().is_empty() {
        return Ok(None);
    }
    let dir = base_dir.join("extensions").join("skills").join(&ext.id);
    std::fs::create_dir_all(dir.join(".claude-plugin"))?;
    std::fs::write(
        dir.join(".claude-plugin").join("plugin.json"),
        serde_json::to_string_pretty(&json!({
            "name": slug,
            "description": description,
            "version": "0.1.0"
        }))?,
    )?;
    let skill_dir = dir.join("skills").join(slug);
    std::fs::create_dir_all(&skill_dir)?;
    let body = format!(
        "---\nname: {}\ndescription: {}\n---\n\n{}\n",
        slug,
        serde_json::to_string(description)?,
        content.trim()
    );
    std::fs::write(skill_dir.join("SKILL.md"), body)?;
    Ok(Some(dir))
}

/// 返回本次运行可传给 --plugin-dir 的 Lumen 私有目录（插件 + 由 Skill 生成的插件包装）。
pub fn plugin_dirs(conn: &Connection, base_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for ext in enabled_of(conn, "plugin")? {
        if let Some(path) = ext.config.get("path").and_then(|v| v.as_str()) {
            let path = PathBuf::from(path);
            if path.exists() {
                dirs.push(path);
            }
        }
    }
    for ext in enabled_of(conn, "skill")? {
        if let Some(dir) = write_skill_plugin(base_dir, &ext)? {
            dirs.push(dir);
        }
    }
    Ok(dirs)
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionHealth {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub enabled: bool,
    /// ok：可用；warn：可达性存疑或需人工确认；error：确定不可用。
    pub status: String,
    pub message: String,
}

fn probe_url(url: &str) -> std::result::Result<u16, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(4))
        .build();
    match agent.get(url).call() {
        Ok(resp) => Ok(resp.status()),
        // 401/403/404 说明网络与服务可达，只是鉴权或路径不同。
        Err(ureq::Error::Status(code, _)) => Ok(code),
        Err(err) => Err(err.to_string()),
    }
}

/// 扩展健康检查：校验命令可解析、远程服务可达、插件目录结构与 Skill 内容完整。
pub fn health(conn: &Connection) -> Result<Vec<ExtensionHealth>> {
    let mut out = Vec::new();
    for ext in list(conn)? {
        let cfg = ext.config.as_object();
        let (status, message) = match ext.kind.as_str() {
            "mcp" => {
                let transport = cfg
                    .and_then(|obj| str_of(obj, "transport"))
                    .unwrap_or("stdio");
                if transport == "stdio" {
                    let command = cfg.and_then(|obj| str_of(obj, "command")).unwrap_or("");
                    if command.trim().is_empty() {
                        (
                            "error",
                            crate::i18n::tr("未配置启动命令", "No start command configured"),
                        )
                    } else if Path::new(command).is_file() {
                        (
                            "ok",
                            crate::i18n::tr(
                                &format!("可执行文件存在：{command}"),
                                &format!("Executable exists: {command}"),
                            ),
                        )
                    } else if let Some(path) = crate::detect::resolve(command) {
                        (
                            "ok",
                            crate::i18n::tr(
                                &format!("已解析到 {}", path.to_string_lossy()),
                                &format!("Resolved to {}", path.to_string_lossy()),
                            ),
                        )
                    } else {
                        (
                            "error",
                            crate::i18n::tr(
                                &format!("在 PATH 中找不到命令：{command}"),
                                &format!("Command not found in PATH: {command}"),
                            ),
                        )
                    }
                } else {
                    let url = cfg.and_then(|obj| str_of(obj, "url")).unwrap_or("");
                    if url.trim().is_empty() {
                        (
                            "error",
                            crate::i18n::tr("未配置服务地址", "No service URL configured"),
                        )
                    } else {
                        match probe_url(url) {
                            Ok(code) => (
                                "ok",
                                crate::i18n::tr(
                                    &format!("服务可达（HTTP {code}）"),
                                    &format!("Service reachable (HTTP {code})"),
                                ),
                            ),
                            Err(err) => (
                                "warn",
                                crate::i18n::tr(
                                    &format!("服务未响应：{err}"),
                                    &format!("Service not responding: {err}"),
                                ),
                            ),
                        }
                    }
                }
            }
            "hook" => {
                let command = cfg.and_then(|obj| str_of(obj, "command")).unwrap_or("");
                if command.trim().is_empty() {
                    (
                        "error",
                        crate::i18n::tr("未配置执行命令", "No execution command configured"),
                    )
                } else if command.chars().any(|c| matches!(c, '"' | '&' | '|' | ';')) {
                    (
                        "warn",
                        crate::i18n::tr(
                            &format!("复杂命令，请人工确认：{command}"),
                            &format!("Complex command; please verify manually: {command}"),
                        ),
                    )
                } else {
                    let first = command.split_whitespace().next().unwrap_or("");
                    if crate::detect::resolve(first).is_some() || Path::new(first).is_file() {
                        (
                            "ok",
                            crate::i18n::tr(
                                &format!("命令可用：{first}"),
                                &format!("Command available: {first}"),
                            ),
                        )
                    } else {
                        (
                            "warn",
                            crate::i18n::tr(
                                &format!("未在 PATH 中找到：{first}"),
                                &format!("Not found in PATH: {first}"),
                            ),
                        )
                    }
                }
            }
            "skill" => {
                let content = cfg.and_then(|obj| str_of(obj, "content")).unwrap_or("");
                if content.trim().is_empty() {
                    (
                        "error",
                        crate::i18n::tr("Skill 内容为空", "Skill content is empty"),
                    )
                } else {
                    (
                        "ok",
                        crate::i18n::tr(
                            &format!("内容完整，共 {} 字", content.chars().count()),
                            &format!("Content complete, {} characters", content.chars().count()),
                        ),
                    )
                }
            }
            "plugin" => {
                let path = cfg.and_then(|obj| str_of(obj, "path")).unwrap_or("");
                if path.trim().is_empty() {
                    (
                        "error",
                        crate::i18n::tr("未配置插件路径", "No plugin path configured"),
                    )
                } else {
                    let target = Path::new(path);
                    if !target.exists() {
                        (
                            "error",
                            crate::i18n::tr(
                                &format!("路径不存在：{path}"),
                                &format!("Path does not exist: {path}"),
                            ),
                        )
                    } else if target.is_dir() {
                        if target.join(".claude-plugin").join("plugin.json").is_file() {
                            (
                                "ok",
                                crate::i18n::tr(
                                    "插件目录结构有效",
                                    "Plugin directory structure is valid",
                                ),
                            )
                        } else {
                            (
                                "warn",
                                crate::i18n::tr(
                                    "缺少 .claude-plugin/plugin.json",
                                    "Missing .claude-plugin/plugin.json",
                                ),
                            )
                        }
                    } else if target.extension().and_then(|v| v.to_str()) == Some("zip") {
                        (
                            "ok",
                            crate::i18n::tr("插件压缩包存在", "Plugin archive exists"),
                        )
                    } else {
                        (
                            "warn",
                            crate::i18n::tr(
                                "不是插件目录或 .zip 文件",
                                "Not a plugin directory or .zip file",
                            ),
                        )
                    }
                }
            }
            _ => (
                "error",
                crate::i18n::tr("未知扩展类型", "Unknown extension type"),
            ),
        };
        out.push(ExtensionHealth {
            id: ext.id,
            name: ext.name,
            kind: ext.kind,
            enabled: ext.enabled,
            status: status.to_string(),
            message,
        });
    }
    Ok(out)
}
