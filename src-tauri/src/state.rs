use crate::approval::GateState;
use crate::db::Db;
use crate::error::Result;
use crate::llm_proxy::ProxyState;
use crate::models::AppSettings;
use crate::runtime::RunRegistry;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub db: Db,
    pub base_dir: PathBuf,
    pub settings: Mutex<AppSettings>,
    pub gate: Arc<GateState>,
    pub registry: Arc<RunRegistry>,
    pub proxy: Arc<ProxyState>,
}

impl AppState {
    pub fn conn(&self) -> Result<Connection> {
        self.db.open()
    }

    pub fn settings(&self) -> AppSettings {
        self.settings.lock().map(|s| s.clone()).unwrap_or_default()
    }
}
