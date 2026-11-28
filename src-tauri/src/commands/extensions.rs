//! Extension management commands.

use super::*;

#[tauri::command]
pub fn extensions_health(
    state: State<'_, AppState>,
) -> Result<Vec<crate::extensions::ExtensionHealth>> {
    let conn = conn(&state)?;
    crate::extensions::health(&conn)
}

#[tauri::command]
pub fn extensions_list(state: State<'_, AppState>) -> Result<Vec<Extension>> {
    let conn = conn(&state)?;
    crate::extensions::list(&conn)
}

#[tauri::command]
pub fn extension_upsert(
    state: State<'_, AppState>,
    id: Option<String>,
    kind: String,
    name: String,
    description: Option<String>,
    config: Value,
    enabled: bool,
) -> Result<String> {
    let conn = conn(&state)?;
    crate::extensions::upsert(&conn, id, kind, name, description, config, enabled)
}

#[tauri::command]
pub fn extension_toggle(state: State<'_, AppState>, id: String, enabled: bool) -> Result<()> {
    let conn = conn(&state)?;
    crate::extensions::toggle(&conn, &id, enabled)
}

#[tauri::command]
pub fn extension_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    crate::extensions::delete(&conn, &state.base_dir, &id)
}
