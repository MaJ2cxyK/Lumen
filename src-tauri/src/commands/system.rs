//! Application info, settings, CLI detection and sandbox diagnostics.

use super::*;

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> Result<AppInfo> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: state.base_dir.to_string_lossy().to_string(),
        db_path: state.db.path.to_string_lossy().to_string(),
        gate_port: state.gate.port,
        platform: std::env::consts::OS.to_string(),
    })
}

#[tauri::command]
pub fn app_settings_get(state: State<'_, AppState>) -> Result<AppSettings> {
    Ok(state.settings())
}

#[tauri::command]
pub fn app_settings_save(state: State<'_, AppState>, settings: AppSettings) -> Result<()> {
    let previous = state.settings();
    crate::settings::save(&state.base_dir, &settings)?;
    if let Ok(mut current) = state.settings.lock() {
        *current = settings.clone();
    }
    // 关闭沙箱时回收已授予的 ACL 与运行时 profile，避免留下可被容器访问的目录。
    #[cfg(windows)]
    if previous.sandbox_enabled && !settings.sandbox_enabled {
        if state.registry.active_count() > 0 {
            tracing::warn!("仍有运行进行中，跳过沙箱清理；下次启动时会再次尝试");
        } else {
            match crate::sandbox::session::cleanup(
                &state.base_dir,
                crate::sandbox::session::RUN_PROFILE,
            ) {
                Ok(count) => tracing::info!("沙箱已清理，回收 {count} 项 ACL 记录"),
                Err(err) => tracing::warn!("清理沙箱资源失败：{err}"),
            }
        }
    }
    Ok(())
}

/// 前端切换语言时同步后端，保证之后生成的通知、审批与系统消息使用同一语言。
#[tauri::command]
pub fn set_backend_lang(app: AppHandle, lang: String) {
    let lang = if lang == "en" { "en" } else { "zh" };
    crate::i18n::set_lang(lang);
    crate::i18n::save(&app, lang);
}

#[tauri::command]
pub fn open_data_dir(state: State<'_, AppState>) -> Result<()> {
    std::process::Command::new("explorer")
        .arg(state.base_dir.as_os_str())
        .spawn()
        .map_err(|e| AppError::Msg(format!("打开目录失败：{e}")))?;
    Ok(())
}

/// AppContainer 文件级隔离自检：验证工作目录可读写、外部路径被拦截、relay 回路可用。
#[tauri::command]
pub async fn sandbox_probe(state: State<'_, AppState>) -> Result<Value> {
    #[cfg(windows)]
    {
        let base = state.base_dir.clone();
        let report =
            tauri::async_runtime::spawn_blocking(move || crate::sandbox::probe::run(&base))
                .await
                .map_err(|e| AppError::Msg(format!("沙箱自检任务失败：{e}")))?
                .map_err(AppError::Msg)?;
        Ok(serde_json::to_value(report)?)
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        Err(AppError::Msg("沙箱自检仅支持 Windows".to_string()))
    }
}

#[tauri::command]
pub async fn cli_detect(with_version: bool) -> Result<Vec<DetectedCli>> {
    tauri::async_runtime::spawn_blocking(move || detect::detect_all(with_version))
        .await
        .map_err(|e| AppError::Msg(e.to_string()))
}

#[tauri::command]
pub async fn cli_install(name: String) -> Result<String> {
    tauri::async_runtime::spawn_blocking(move || detect::install(&name))
        .await
        .map_err(|e| AppError::Msg(e.to_string()))?
        .map_err(AppError::Msg)
}

#[tauri::command]
pub async fn cli_latest_versions(names: Vec<String>) -> Result<Vec<CliVersionInfo>> {
    tauri::async_runtime::spawn_blocking(move || detect::latest_versions(&names))
        .await
        .map_err(|e| AppError::Msg(e.to_string()))
}
