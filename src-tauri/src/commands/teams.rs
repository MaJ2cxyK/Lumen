use super::*;

#[tauri::command]
pub fn team_templates() -> Vec<TeamTemplate> {
    ["software-dev", "office-ops", "quant-research"]
        .iter()
        .map(|key| {
            let (k, name, description, icon) = template_meta(key);
            let roles = template_roles(key)
                .into_iter()
                .map(|r| TeamTemplateRole {
                    role_key: r.role_key,
                    role_name: r.role_name,
                    responsibility: r.responsibility,
                    capability: r.capability,
                    gate: r.gate,
                })
                .collect();
            TeamTemplate {
                key: k,
                name,
                description,
                icon,
                roles,
            }
        })
        .collect()
}

#[tauri::command]
pub fn teams_list(state: State<'_, AppState>, project_id: Option<String>) -> Result<Vec<Team>> {
    let conn = conn(&state)?;
    teams_query(&conn, project_id)
}

#[tauri::command]
pub fn team_create(
    state: State<'_, AppState>,
    project_id: String,
    name: String,
    template_key: String,
    description: Option<String>,
) -> Result<String> {
    let roles = template_roles(&template_key);
    if roles.is_empty() {
        return Err("团队模板不存在或没有角色".into());
    }
    let conn = conn(&state)?;
    let tx = conn.unchecked_transaction()?;
    let id = new_id();
    tx.execute(
        "INSERT INTO teams (id, project_id, name, template_key, description, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            project_id,
            name.trim(),
            template_key,
            description,
            now_ms()
        ],
    )?;
    for (i, role) in roles.iter().enumerate() {
        tx.execute(
            "INSERT INTO seats (id, team_id, role_key, role_name, responsibility, order_index, gate)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new_id(),
                id,
                role.role_key,
                role.role_name,
                role.responsibility,
                i as i64,
                role.gate as i64
            ],
        )?;
    }
    tx.commit()?;
    Ok(id)
}

#[tauri::command]
pub fn team_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute("DELETE FROM teams WHERE id = ?1", params![id])?;
    Ok(())
}

#[tauri::command]
pub fn seats_list(state: State<'_, AppState>, team_id: String) -> Result<Vec<Seat>> {
    let conn = conn(&state)?;
    let mut stmt = conn.prepare(
        "SELECT s.id, s.team_id, s.role_key, s.role_name, s.responsibility, s.agent_id, a.name,
                s.order_index, s.gate
         FROM seats s LEFT JOIN agents a ON a.id = s.agent_id
         WHERE s.team_id = ?1 ORDER BY s.order_index ASC",
    )?;
    let rows = stmt.query_map(params![team_id], |r| {
        Ok(Seat {
            id: r.get(0)?,
            team_id: r.get(1)?,
            role_key: r.get(2)?,
            role_name: r.get(3)?,
            responsibility: r.get(4)?,
            agent_id: r.get(5)?,
            agent_name: r.get(6)?,
            order_index: r.get(7)?,
            gate: r.get::<_, i64>(8)? != 0,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn seat_assign(
    state: State<'_, AppState>,
    seat_id: String,
    agent_id: Option<String>,
) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute(
        "UPDATE seats SET agent_id = ?1 WHERE id = ?2",
        params![agent_id, seat_id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn seat_update(
    state: State<'_, AppState>,
    seat_id: String,
    role_name: String,
    responsibility: Option<String>,
    gate: bool,
) -> Result<()> {
    let conn = conn(&state)?;
    conn.execute(
        "UPDATE seats SET role_name = ?1, responsibility = ?2, gate = ?3 WHERE id = ?4",
        params![role_name, responsibility, gate as i64, seat_id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn seat_reorder(
    state: State<'_, AppState>,
    team_id: String,
    seat_ids: Vec<String>,
) -> Result<()> {
    let mut conn = conn(&state)?;
    let tx = conn.transaction()?;
    for (index, seat_id) in seat_ids.iter().enumerate() {
        tx.execute(
            "UPDATE seats SET order_index = ?1 WHERE id = ?2 AND team_id = ?3",
            params![index as i64, seat_id, team_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[tauri::command]
pub fn seat_auto_assign(state: State<'_, AppState>, team_id: String) -> Result<i64> {
    let conn = conn(&state)?;
    let mut agents: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM agents ORDER BY created_at ASC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    if agents.is_empty() {
        return Err("还没有可用的 Agent，请先添加".into());
    }
    let seats: Vec<String> = {
        let mut stmt =
            conn.prepare("SELECT id FROM seats WHERE team_id = ?1 ORDER BY order_index ASC")?;
        let rows = stmt.query_map(params![team_id], |r| r.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let mut assigned = 0i64;
    for seat_id in seats.iter() {
        let agent = agents.remove(0);
        agents.push(agent.clone());
        let changed = conn.execute(
            "UPDATE seats SET agent_id = ?1 WHERE id = ?2 AND agent_id IS NULL",
            params![agent, seat_id],
        )?;
        assigned += changed as i64;
    }
    Ok(assigned)
}

pub(super) fn teams_query(conn: &Connection, project_id: Option<String>) -> Result<Vec<Team>> {
    let base = "SELECT t.id, t.project_id, p.name, t.name, t.template_key, t.description,
            (SELECT COUNT(*) FROM seats s WHERE s.team_id = t.id),
            (SELECT COUNT(*) FROM seats s WHERE s.team_id = t.id AND s.agent_id IS NOT NULL),
            t.created_at
         FROM teams t LEFT JOIN projects p ON p.id = t.project_id";
    let sql = if project_id.is_some() {
        format!("{base} WHERE t.project_id = ?1 ORDER BY t.created_at DESC")
    } else {
        format!("{base} ORDER BY t.created_at DESC")
    };
    let mut stmt = conn.prepare(&sql)?;
    let mapper = |r: &rusqlite::Row| -> rusqlite::Result<Team> {
        Ok(Team {
            id: r.get(0)?,
            project_id: r.get(1)?,
            project_name: r.get(2)?,
            name: r.get(3)?,
            template_key: r.get(4)?,
            description: r.get(5)?,
            seats_count: r.get(6)?,
            assigned_count: r.get(7)?,
            created_at: r.get(8)?,
        })
    };
    let items: Vec<Team> = if let Some(pid) = project_id {
        stmt.query_map(params![pid], mapper)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map([], mapper)?.filter_map(|r| r.ok()).collect()
    };
    Ok(items)
}
