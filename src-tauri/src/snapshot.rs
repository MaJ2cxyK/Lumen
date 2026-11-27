use crate::db::now_ms;
use crate::error::{AppError, Result};
use crate::review::{git, is_git_repo, stderr_of};
use std::fs;
use std::path::Path;

const MAX_PATCH_BYTES: usize = 8 * 1024 * 1024;
const KEEP_SNAPSHOTS: usize = 20;

fn git_text(dir: &Path, args: &[&str]) -> Option<String> {
    let out = git(dir, args).ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// 未跟踪文件清单（路径 + 字节数），只做记录，不复制内容。
fn untracked_files(dir: &Path) -> Vec<(String, u64)> {
    let Ok(out) = git(dir, &["ls-files", "--others", "--exclude-standard", "-z"]) else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    raw.split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| {
            let size = fs::metadata(dir.join(path)).map(|m| m.len()).unwrap_or(0);
            (path.to_string(), size)
        })
        .collect()
}

/// 非隔离运行前为 Git 项目生成只读补丁快照，不触碰用户 index 与工作区。
/// 非 Git 项目返回 `None`，由前端提示无法自动回滚。
pub fn create_pre_run_snapshot(
    project: &Path,
    base_dir: &Path,
    task_id: &str,
) -> Result<Option<String>> {
    if !project.is_dir() || !is_git_repo(project) {
        return Ok(None);
    }
    let dir = base_dir.join("snapshots");
    fs::create_dir_all(&dir)?;

    let out = git(project, &["diff", "--binary", "HEAD"])?;
    if !out.status.success() {
        return Err(AppError::Msg(stderr_of(&out)));
    }
    let head = git_text(project, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let branch = git_text(project, &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|| "unknown".into());
    let untracked = untracked_files(project);

    let mut body = String::new();
    body.push_str("# Lumen 运行前快照\n");
    body.push_str(&format!("# 任务: {task_id}\n"));
    body.push_str(&format!("# 仓库: {}\n", project.display()));
    body.push_str(&format!("# HEAD: {head}\n"));
    body.push_str(&format!("# 分支: {branch}\n"));
    body.push_str(&format!("# 生成时间: {}\n", now_ms()));
    if untracked.is_empty() {
        body.push_str("# 未跟踪文件: 无\n");
    } else {
        body.push_str(&format!(
            "# 未跟踪文件（{} 个，仅列清单）:\n",
            untracked.len()
        ));
        for (path, size) in &untracked {
            body.push_str(&format!("#   {path} ({size} bytes)\n"));
        }
    }
    body.push_str(
        "# 说明: 以下为 `git diff --binary HEAD`，可用 `git apply` 还原已跟踪文件的改动。\n\n",
    );

    let mut diff = String::from_utf8_lossy(&out.stdout).to_string();
    if diff.trim().is_empty() {
        diff = "（工作区没有已跟踪文件的未提交改动）\n".to_string();
    }
    if diff.len() > MAX_PATCH_BYTES {
        let mut end = MAX_PATCH_BYTES;
        while end > 0 && !diff.is_char_boundary(end) {
            end -= 1;
        }
        diff.truncate(end);
        diff.push_str("\n…（补丁超过 8 MB，已截断）\n");
    }
    body.push_str(&diff);

    let path = dir.join(format!("{task_id}-{}.patch", now_ms()));
    fs::write(&path, body)?;
    prune(&dir, KEEP_SNAPSHOTS);
    Ok(Some(path.to_string_lossy().to_string()))
}

/// 按修改时间保留最近的快照，避免数据目录无界增长。
fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("patch"))
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((modified, e.path()))
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
