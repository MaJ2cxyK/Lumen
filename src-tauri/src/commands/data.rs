//! Data export, backup and restore commands.

use super::*;

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
