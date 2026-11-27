pub mod approval;
pub mod backup;
pub mod budget;
pub mod commands;
pub mod crypto;
pub mod db;
pub mod detect;
pub mod error;
pub mod extensions;
pub mod hook_gate;
pub mod i18n;
pub mod llm_proxy;
pub mod models;
pub mod notify;
pub mod orchestrator;
pub mod providers;
pub mod review;
pub mod runtime;
pub mod sandbox;
pub mod settings;
pub mod snapshot;
pub mod state;
pub mod task_lifecycle;
pub mod usage;
pub mod worktree;

use crate::db::Db;
use crate::runtime::RunRegistry;
use crate::state::AppState;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::Manager;

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// 数据目录从 AgentHub 平滑迁移到 Lumen，保留既有项目、Agent 与供应商配置。
fn data_base() -> PathBuf {
    let root = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    let target = root.join("Lumen");
    let legacy = root.join("AgentHub");
    if !target.exists()
        && legacy.exists()
        && std::fs::rename(&legacy, &target).is_err()
        && copy_tree(&legacy, &target).is_err()
    {
        return legacy;
    }
    let base = if target.exists() || !legacy.exists() {
        target
    } else {
        legacy
    };
    for suffix in ["", "-wal", "-shm"] {
        let old = base.join(format!("agenthub.db{suffix}"));
        let new = base.join(format!("lumen.db{suffix}"));
        if old.exists() && !new.exists() {
            let _ = std::fs::rename(old, new);
        }
    }
    base
}

pub fn run() {
    let base = data_base();
    let log_dir = base.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .clear_targets()
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Folder {
                        path: log_dir,
                        file_name: Some("lumen".into()),
                    },
                ))
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            // 尽早恢复界面语言，保证启动阶段写入的系统消息与通知使用同一语言。
            i18n::load(app.handle());
            std::fs::create_dir_all(&base)?;
            db::ensure_dirs(&base)?;

            let database = Db::new(base.join("lumen.db"));
            database.init()?;
            database.reconcile_on_startup()?;
            {
                let conn = database.open()?;
                detect::seed_default_agents(&conn)?;
            }
            let app_settings = settings::load(&base);
            // 关闭沙箱时若有运行未结束，清理会被跳过；启动时补做一次。
            #[cfg(windows)]
            if !app_settings.sandbox_enabled {
                let cleanup_base = base.clone();
                std::thread::spawn(move || {
                    match crate::sandbox::session::cleanup(
                        &cleanup_base,
                        crate::sandbox::session::RUN_PROFILE,
                    ) {
                        Ok(count) if count > 0 => {
                            tracing::info!("启动时回收沙箱残留 {count} 项 ACL 记录");
                        }
                        Ok(_) => {}
                        Err(err) => tracing::warn!("启动时清理沙箱残留失败：{err}"),
                    }
                });
            }
            let _ = db::cleanup_transcripts(&base, app_settings.transcript_retention_days);
            let gate = approval::start_server(
                app.handle().clone(),
                database.clone(),
                app_settings.approval_timeout_secs,
            )?;
            let registry = Arc::new(RunRegistry::default());
            // 本地模型代理：真实 Key 只在主进程内存中，子进程只拿一次性令牌。
            let proxy =
                llm_proxy::start_server(app.handle().clone(), database.clone(), registry.clone())?;
            let rotate_base = base.clone();
            let rotate_db = database.path.clone();
            app.manage(AppState {
                db: database,
                base_dir: base,
                settings: Mutex::new(app_settings),
                gate,
                registry,
                proxy,
            });
            // 启动备份轮转：清理过期备份，必要时自动补一份当天的快照。
            std::thread::spawn(move || {
                if let Err(err) = backup::startup_rotate(&rotate_base, &rotate_db) {
                    tracing::warn!("启动备份轮转失败：{err}");
                }
            });
            tracing::info!("Lumen started");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::app_settings_get,
            commands::app_settings_save,
            commands::set_backend_lang,
            commands::open_data_dir,
            commands::sandbox_probe,
            commands::cli_detect,
            commands::cli_install,
            commands::cli_latest_versions,
            commands::agents_list,
            commands::agent_create,
            commands::agent_update,
            commands::agent_delete,
            commands::projects_list,
            commands::project_git_check,
            commands::project_create,
            commands::project_update,
            commands::project_delete,
            commands::team_templates,
            commands::teams_list,
            commands::team_create,
            commands::team_delete,
            commands::seats_list,
            commands::seat_assign,
            commands::seat_update,
            commands::seat_reorder,
            commands::seat_auto_assign,
            commands::tasks_list,
            commands::tasks_page,
            commands::task_get,
            commands::task_create,
            commands::task_update,
            commands::task_delete,
            commands::task_dispatch,
            commands::task_cancel,
            commands::task_reset,
            commands::task_add_note,
            commands::task_messages,
            commands::task_retry_stage,
            commands::task_pause,
            commands::task_resume,
            commands::task_reassign,
            commands::task_children,
            commands::task_dependencies,
            commands::task_dependency_add,
            commands::task_dependency_remove,
            commands::task_checklist_get,
            commands::task_checklist_save,
            commands::task_attachments,
            commands::task_attachment_add,
            commands::task_attachment_delete,
            commands::task_attachment_reveal,
            review::task_review,
            review::task_review_diff,
            review::task_review_revert,
            review::task_review_revert_all,
            review::task_review_commit,
            review::task_review_apply,
            review::task_review_cleanup,
            review::task_review_branch_info,
            review::task_review_branch_create,
            review::task_review_branch_switch,
            review::task_snapshots,
            review::task_snapshot_rollback,
            review::task_review_pr,
            commands::tool_calls_list,
            commands::runs_list,
            commands::run_prompt,
            commands::run_cancel,
            commands::run_transcript,
            commands::usage_summary,
            commands::usage_series,
            commands::usage_by_agent,
            commands::usage_by_project,
            commands::usage_by_task,
            commands::budgets_list,
            commands::budget_upsert,
            commands::budget_delete,
            commands::approvals_list,
            commands::approvals_pending,
            commands::approval_decide,
            commands::approval_rules_list,
            commands::approval_rule_upsert,
            commands::approval_rule_toggle,
            commands::approval_rule_delete,
            commands::events_list,
            commands::search_history,
            commands::dashboard_summary,
            commands::providers_list,
            commands::provider_upsert,
            commands::provider_delete,
            commands::provider_test,
            commands::provider_models,
            commands::extensions_list,
            commands::data_export,
            commands::data_backup,
            commands::backups_list,
            commands::data_restore,
            commands::open_backup_dir,
            commands::extension_upsert,
            commands::extension_toggle,
            commands::extension_delete,
            commands::extensions_health,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Lumen")
        .run(|app_handle, event| {
            // 应用退出时终止所有仍在运行的 Agent 进程树，避免残留进程继续改文件与计费。
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    state.registry.cancel_all();
                }
            }
        });
}
