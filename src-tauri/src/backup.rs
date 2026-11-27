use crate::db::now_ms;
use crate::error::{AppError, Result};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const KEEP_BACKUPS: usize = 7;
const BACKUP_PREFIX: &str = "lumen-";
const SAFETY_PREFIX: &str = "pre-restore-";
const BACKUP_EXT: &str = "db";
const AUTO_BACKUP_INTERVAL: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub created_at: i64,
}

fn is_backup_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let is_db = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case(BACKUP_EXT))
        .unwrap_or(false);
    is_db && name.starts_with(BACKUP_PREFIX)
}

fn is_config_backup(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case(BACKUP_EXT))
        .unwrap_or(false)
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with(BACKUP_PREFIX) || n.starts_with(SAFETY_PREFIX))
            .unwrap_or(false)
}

fn modified_ms(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 生成一致性快照（WAL 下也安全），并按保留策略轮转。
pub fn create(base_dir: &Path, conn: &Connection) -> Result<String> {
    let dir = base_dir.join("backups");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{BACKUP_PREFIX}{}.db", now_ms()));
    // VACUUM INTO 生成独立副本，不中断当前连接。
    conn.execute("VACUUM INTO ?1", params![path.to_string_lossy()])?;
    prune(&dir, KEEP_BACKUPS);
    Ok(path.to_string_lossy().to_string())
}

/// 只保留最近 `keep` 份手动备份；恢复前的安全副本单独计数，不挤占名额。
pub fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|entry| is_backup_file(&entry.path()))
        .filter_map(|entry| {
            let modified = entry.metadata().and_then(|m| m.modified()).ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, path) in files.into_iter().skip(keep) {
        let _ = fs::remove_file(path);
    }
}

/// 列出可恢复的备份，按时间倒序。
pub fn list(base_dir: &Path) -> Result<Vec<BackupFile>> {
    let dir = base_dir.join("backups");
    let mut items = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(items);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_config_backup(&path) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        items.push(BackupFile {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            size: meta.len(),
            created_at: meta.modified().map(modified_ms).unwrap_or(0),
        });
    }
    items.sort_by_key(|a| std::cmp::Reverse(a.created_at));
    Ok(items)
}

/// 启动时轮转：清理过期备份；若 24 小时内没有备份则自动生成一份。
pub fn startup_rotate(base_dir: &Path, db_path: &Path) -> Result<()> {
    let dir = base_dir.join("backups");
    prune(&dir, KEEP_BACKUPS);
    if !db_path.is_file() {
        return Ok(());
    }
    let recent = list(base_dir)?
        .into_iter()
        .filter(|item| item.name.starts_with(BACKUP_PREFIX))
        .any(|item| {
            let age = now_ms().saturating_sub(item.created_at);
            age >= 0 && (age as u64) < AUTO_BACKUP_INTERVAL.as_millis() as u64
        });
    if recent {
        return Ok(());
    }
    let conn = Connection::open(db_path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    create(base_dir, &conn)?;
    Ok(())
}

fn safe_backup_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || Path::new(name).components().count() != 1
        || !name.ends_with(&format!(".{BACKUP_EXT}"))
        || !is_config_backup(Path::new(name))
    {
        return Err(AppError::Msg("备份文件名不合法".into()));
    }
    Ok(())
}

/// 校验备份可用后覆盖当前数据库；恢复前自动生成一份安全副本。
pub fn restore(base_dir: &Path, db_path: &Path, name: &str) -> Result<()> {
    safe_backup_name(name)?;
    let backup_path = base_dir.join("backups").join(name);
    if !backup_path.is_file() {
        return Err(AppError::Msg("备份文件不存在".into()));
    }

    {
        // 只读校验：完整性与关键表存在性，避免恢复出无法启动的库。
        let conn = Connection::open_with_flags(&backup_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let integrity: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(AppError::Msg(format!("备份完整性校验失败：{integrity}")));
        }
        let tables: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('projects', 'schema_meta')",
            [],
            |r| r.get(0),
        )?;
        if tables < 2 {
            return Err(AppError::Msg("备份不是有效的 Lumen 数据库".into()));
        }
    }

    if db_path.is_file() {
        let dir = base_dir.join("backups");
        fs::create_dir_all(&dir)?;
        let safety = dir.join(format!("{SAFETY_PREFIX}{}.db", now_ms()));
        let conn = Connection::open(db_path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute("VACUUM INTO ?1", params![safety.to_string_lossy()])?;
    }

    for sidecar in ["-wal", "-shm"] {
        let mut path = db_path.as_os_str().to_os_string();
        path.push(sidecar);
        let _ = fs::remove_file(PathBuf::from(path));
    }
    fs::copy(&backup_path, db_path)?;
    prune(&base_dir.join("backups"), KEEP_BACKUPS);
    Ok(())
}

fn sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("api_key")
        || lower.contains("apikey")
        || lower == "token"
        || lower.ends_with("_token")
        || lower.contains("secret")
        || lower.contains("password")
        || lower == "authorization"
}

/// 递归脱敏：导出 JSON 时不带出密钥、令牌与密码。
pub(crate) fn redact_value(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, item)| {
                    if sensitive_key(&key) {
                        (key, Value::String("<redacted>".into()))
                    } else {
                        (key, redact_value(item))
                    }
                })
                .collect::<Map<String, Value>>(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(redact_value).collect()),
        other => other,
    }
}

/// 对存成字符串的 JSON 配置做脱敏；无法解析时原样返回。
pub(crate) fn redact_json_text(text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(value) => redact_value(value),
        Err(_) => Value::String(text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumen-test-{tag}-{}", now_ms()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn prune_keeps_newest_backups() {
        let dir = temp_dir("prune");
        for index in 0..9 {
            let path = dir.join(format!("lumen-{index}.db"));
            fs::write(&path, b"x").unwrap();
            let file = File::options().write(true).open(&path).unwrap();
            file.set_modified(SystemTime::now() - Duration::from_secs((9 - index) as u64 * 60))
                .unwrap();
        }
        prune(&dir, 7);
        let kept: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(kept.len(), 7);
        assert!(!kept.contains(&"lumen-0.db".to_string()));
        assert!(!kept.contains(&"lumen-1.db".to_string()));
        assert!(kept.contains(&"lumen-8.db".to_string()));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn redacts_sensitive_fields() {
        let value = serde_json::json!({
            "api_key": "sk-live",
            "nested": { "access_token": "tok", "max_tokens": 4096 },
            "headers": { "Authorization": "Bearer x" },
            "safe": "value"
        });
        let redacted = redact_value(value);
        assert_eq!(redacted["api_key"], "<redacted>");
        assert_eq!(redacted["nested"]["access_token"], "<redacted>");
        assert_eq!(redacted["nested"]["max_tokens"], 4096);
        assert_eq!(redacted["headers"]["Authorization"], "<redacted>");
        assert_eq!(redacted["safe"], "value");
    }
}
