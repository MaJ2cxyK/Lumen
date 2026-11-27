use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// 读取实时通知开关；状态尚未挂载时默认开启，避免启动早期静默丢通知。
pub fn enabled(app: &AppHandle) -> bool {
    app.try_state::<crate::state::AppState>()
        .map(|state| state.settings().notify)
        .unwrap_or(true)
}

pub fn send(app: &AppHandle, title: &str, body: &str) {
    send_task(app, title, body, None, "info");
}

/// 带任务定位的通知：应用内事件用于 toast 与跳转，系统通知携带 extra 供点击回传。
pub fn send_task(app: &AppHandle, title: &str, body: &str, task_id: Option<&str>, kind: &str) {
    let task_id = task_id.filter(|id| !id.is_empty());
    let _ = app.emit(
        "notify",
        json!({
            "title": title,
            "body": body,
            "taskId": task_id,
            "kind": kind,
            "at": crate::db::now_ms(),
        }),
    );
    if !enabled(app) {
        return;
    }
    let mut builder = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .extra("kind", kind);
    if let Some(id) = task_id {
        builder = builder.extra("taskId", id);
    }
    if let Err(err) = builder.show() {
        tracing::warn!("system notification failed: {err}");
    }
}
