use crate::error::Result;
use crate::models::AppSettings;
use std::path::Path;

pub fn load(base: &Path) -> AppSettings {
    let path = base.join("config.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(base: &Path, settings: &AppSettings) -> Result<()> {
    let path = base.join("config.json");
    let tmp = base.join("config.json.tmp");
    let text = serde_json::to_string_pretty(settings)?;
    // 先写临时文件再 rename，崩溃时不会留下被截断的配置。
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_defaults_and_round_trips() {
        let dir = std::env::temp_dir().join("lumen-settings-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut settings = AppSettings::default();
        assert!(settings.sandbox_enabled, "沙箱默认应开启");
        settings.sandbox_enabled = false;
        save(&dir, &settings).unwrap();
        assert!(!load(&dir).sandbox_enabled, "沙箱开关未持久化");

        // 旧版 config.json 没有 sandboxEnabled 字段，升级后应继承新的安全默认值。
        std::fs::write(
            dir.join("config.json"),
            r#"{"maxConcurrentRuns":3,"approvalTimeoutSecs":600,"transcriptRetentionDays":90,"notify":true}"#,
        )
        .unwrap();
        assert!(load(&dir).sandbox_enabled, "旧配置未继承沙箱默认值");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
