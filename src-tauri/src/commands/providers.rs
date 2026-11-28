//! Provider configuration commands.

use super::*;

#[tauri::command]
pub fn providers_list(state: State<'_, AppState>) -> Result<Vec<Provider>> {
    let conn = conn(&state)?;
    crate::providers::list(&conn)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn provider_upsert(
    state: State<'_, AppState>,
    id: Option<String>,
    name: String,
    protocol: String,
    base_url: String,
    model: Option<String>,
    models: Option<Vec<String>>,
    api_key: Option<String>,
    enabled: bool,
) -> Result<String> {
    let conn = conn(&state)?;
    crate::providers::upsert(
        &conn, id, name, protocol, base_url, model, models, api_key, enabled,
    )
}

#[tauri::command]
pub async fn provider_models(
    state: State<'_, AppState>,
    id: Option<String>,
    protocol: String,
    base_url: String,
    api_key: Option<String>,
) -> Result<Vec<String>> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::providers::fetch_models(&db, id, protocol, base_url, api_key)
    })
    .await
    .map_err(|e| AppError::Msg(e.to_string()))?
}

#[tauri::command]
pub fn provider_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    crate::providers::delete(&conn, &id)
}

#[tauri::command]
pub async fn provider_test(state: State<'_, AppState>, id: String) -> Result<ProviderTestResult> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || crate::providers::test(&db, &id))
        .await
        .map_err(|e| AppError::Msg(e.to_string()))?
}
