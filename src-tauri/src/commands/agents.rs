//! Agent configuration commands.

use super::*;

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
