use crate::error::{AppError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// 为任务创建独立的 git worktree（detached HEAD）。
///
/// 仅在项目本身是 git 仓库时生效；创建失败返回具体原因，由调用方中止任务创建。
/// v1 不自动删除 worktree，方便用户审查改动或 cherry-pick。
pub fn create_detached_worktree(repo: &Path, base_dir: &Path, task_id: &str) -> Result<String> {
    if !repo.is_dir() {
        return Err(AppError::Msg(
            "项目目录不存在，无法创建隔离工作区".to_string(),
        ));
    }
    let probe = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|e| AppError::Msg(format!("无法调用 git：{e}")))?;
    if !probe.status.success() || String::from_utf8_lossy(&probe.stdout).trim() != "true" {
        return Err(AppError::Msg(
            "项目不是 Git 仓库，无法创建隔离工作区；请先初始化仓库或取消勾选隔离工作区".to_string(),
        ));
    }

    let worktrees = base_dir.join("worktrees");
    std::fs::create_dir_all(&worktrees)
        .map_err(|e| AppError::Msg(format!("无法创建 worktrees 目录：{e}")))?;
    let target: PathBuf = worktrees.join(task_id);
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "add", "--detach"])
        .arg(&target)
        .arg("HEAD")
        .output()
        .map_err(|e| AppError::Msg(format!("无法调用 git：{e}")))?;
    if !output.status.success() {
        // 清理失败时 git 可能留下了半个目录。
        let _ = std::fs::remove_dir_all(&target);
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AppError::Msg(if detail.is_empty() {
            "创建隔离工作区失败".to_string()
        } else {
            format!("创建隔离工作区失败：{detail}")
        }));
    }
    target
        .to_str()
        .map(|value| value.to_string())
        .ok_or_else(|| AppError::Msg("隔离工作区路径包含非法字符".to_string()))
}
