use crate::error::{AppError, Result};
use crate::state::AppState;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use tauri::State;

const MAX_FILES: usize = 500;
const MAX_PATCH_BYTES: usize = 240_000;
const MAX_UNTRACKED_BYTES: u64 = 1_048_576;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFile {
    pub path: String,
    pub code: String,
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    pub binary: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshot {
    pub available: bool,
    pub reason: Option<String>,
    pub work_dir: Option<String>,
    pub repo_root: Option<String>,
    pub files: Vec<ReviewFile>,
    pub additions: i64,
    pub deletions: i64,
    pub pending_commits: i64,
    pub can_apply: bool,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDiff {
    pub path: String,
    pub patch: String,
    pub truncated: bool,
    pub binary: bool,
}

struct TaskCtx {
    status: String,
    work_dir: Option<PathBuf>,
    project_path: Option<PathBuf>,
}

pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<Output> {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "core.quotepath=false", "-C"])
        .arg(dir)
        .args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.output()
        .map_err(|e| AppError::Msg(format!("无法调用 git：{e}")))
}

pub(crate) fn stderr_of(out: &Output) -> String {
    let text = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if text.is_empty() {
        "git 命令执行失败".to_string()
    } else {
        text
    }
}

pub(crate) fn is_git_repo(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--is-inside-work-tree"])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 项目目录的 Git 探测结果，供项目列表与新建项目即时提示。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitState {
    pub is_git: bool,
    pub reason: Option<String>,
}

pub fn git_state(dir: &Path) -> GitState {
    if !dir.is_dir() {
        return GitState {
            is_git: false,
            reason: Some(crate::i18n::tr("目录不存在", "Directory does not exist")),
        };
    }
    match git(dir, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(out) if out.status.success() => GitState {
            is_git: true,
            reason: None,
        },
        Ok(out) => GitState {
            is_git: false,
            reason: Some(stderr_of(&out)),
        },
        Err(err) => GitState {
            is_git: false,
            reason: Some(err.to_string()),
        },
    }
}

fn head_sha(dir: &Path) -> Option<String> {
    let out = git(dir, &["rev-parse", "HEAD"]).ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

fn task_ctx(state: &State<'_, AppState>, task_id: &str) -> Result<TaskCtx> {
    let conn = state.conn()?;
    let (status, work_dir, project_path): (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT t.status, t.work_dir, p.path
               FROM tasks t LEFT JOIN projects p ON p.id = t.project_id
              WHERE t.id = ?1",
            rusqlite::params![task_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| AppError::Msg("任务不存在".into()))?;
    Ok(TaskCtx {
        status,
        work_dir: work_dir.map(PathBuf::from),
        project_path: project_path.map(PathBuf::from),
    })
}

fn ensure_idle(ctx: &TaskCtx) -> Result<()> {
    if matches!(
        ctx.status.as_str(),
        "running" | "queued" | "waiting_approval"
    ) {
        return Err("任务正在运行，请先停止后再操作隔离工作区".into());
    }
    Ok(())
}

fn active_work_dir(ctx: &TaskCtx) -> Result<PathBuf> {
    let Some(dir) = ctx.work_dir.clone() else {
        return Err("该任务未启用隔离工作区".into());
    };
    if !dir.is_dir() {
        return Err("隔离工作区已不存在".into());
    }
    if !is_git_repo(&dir) {
        return Err("隔离工作区不是 Git 仓库".into());
    }
    Ok(dir)
}

/// 判断隔离工作区是否仍有待审查内容：未提交改动，或尚未被项目应用的提交。
pub fn has_pending_review(work_dir: Option<&Path>, project_path: Option<&Path>) -> bool {
    let Some(dir) = work_dir else {
        return false;
    };
    if !dir.is_dir() || !is_git_repo(dir) {
        return false;
    }
    if status_entries(dir)
        .map(|entries| !entries.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    let Some(project) = project_path else {
        return false;
    };
    if !is_git_repo(project) {
        return false;
    }
    let (Some(head), Some(project_head)) = (head_sha(dir), head_sha(project)) else {
        return false;
    };
    if head == project_head {
        return false;
    }
    unapplied_commits(project, &project_head, &head)
        .map(|commits| !commits.is_empty())
        .unwrap_or(false)
}

/// 重新计算并写入任务的待审查标记，供任务列表与看板展示。
pub fn refresh_review_flag(conn: &rusqlite::Connection, task_id: &str) -> Result<()> {
    let (work_dir, project_path): (Option<String>, Option<String>) = conn.query_row(
        "SELECT t.work_dir, p.path
           FROM tasks t LEFT JOIN projects p ON p.id = t.project_id
          WHERE t.id = ?1",
        rusqlite::params![task_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let pending = has_pending_review(
        work_dir.as_deref().map(Path::new),
        project_path.as_deref().map(Path::new),
    );
    conn.execute(
        "UPDATE tasks SET review_pending = ?1 WHERE id = ?2",
        rusqlite::params![pending as i64, task_id],
    )?;
    Ok(())
}

fn safe_rel_path(raw: &str) -> Result<&str> {
    let path = Path::new(raw);
    if raw.trim().is_empty() || path.is_absolute() {
        return Err("非法文件路径".into());
    }
    if path.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("非法文件路径".into());
    }
    Ok(raw)
}

fn classify(code: &str) -> &'static str {
    if code.contains('U') || code == "AA" || code == "DD" {
        return "conflicted";
    }
    if code == "??" {
        return "untracked";
    }
    if code.contains('R') || code.contains('C') {
        return "renamed";
    }
    if code.contains('A') {
        return "added";
    }
    if code.contains('D') {
        return "deleted";
    }
    "modified"
}

/// 解析 `git status --porcelain -z`，返回 (code, path) 列表。
fn status_entries(dir: &Path) -> Result<Vec<(String, String)>> {
    let out = git(
        dir,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if !out.status.success() {
        return Err(AppError::Msg(stderr_of(&out)));
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let mut tokens = raw.split('\0');
    let mut entries = Vec::new();
    while let Some(token) = tokens.next() {
        if token.len() < 4 {
            continue;
        }
        let code = token[..2].trim().to_string();
        let path = token[3..].to_string();
        if code == "!!" || path.is_empty() {
            continue;
        }
        let renamed = code.contains('R') || code.contains('C');
        if renamed {
            // -z 模式下重命名/复制记录的下一个 token 是原路径。
            let _ = tokens.next();
        }
        entries.push((code, path));
    }
    Ok(entries)
}

fn numstat(dir: &Path) -> Result<HashMap<String, (i64, i64, bool)>> {
    let out = git(dir, &["diff", "--numstat", "--no-renames", "HEAD", "--"])?;
    if !out.status.success() {
        return Ok(HashMap::new());
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let mut map = HashMap::new();
    for line in raw.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let binary = a == "-" && d == "-";
        let additions = if binary { 0 } else { a.parse().unwrap_or(0) };
        let deletions = if binary { 0 } else { d.parse().unwrap_or(0) };
        map.insert(path.to_string(), (additions, deletions, binary));
    }
    Ok(map)
}

fn untracked_stats(dir: &Path, rel: &str) -> (i64, i64, bool) {
    let full = dir.join(rel);
    let Ok(meta) = fs::metadata(&full) else {
        return (0, 0, false);
    };
    if !meta.is_file() {
        return (0, 0, false);
    }
    if meta.len() > MAX_UNTRACKED_BYTES {
        return (0, 0, true);
    }
    let Ok(bytes) = fs::read(&full) else {
        return (0, 0, false);
    };
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return (0, 0, true);
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines = if text.is_empty() {
        0
    } else {
        text.lines().count() as i64
    };
    (lines, 0, false)
}

fn truncate_text(text: String, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (
        crate::i18n::tr(
            &format!("{}\n…（差异过长，已截断）", &text[..end]),
            &format!("{}\n… (diff too long, truncated)", &text[..end]),
        ),
        true,
    )
}

fn untracked_patch(dir: &Path, rel: &str) -> Result<(String, bool, bool)> {
    let full = dir.join(rel);
    let file = fs::File::open(&full)?;
    let mut buf = Vec::new();
    file.take(MAX_PATCH_BYTES as u64).read_to_end(&mut buf)?;
    if buf.iter().take(8192).any(|b| *b == 0) {
        return Ok((
            crate::i18n::tr(
                "（二进制文件，未显示差异）",
                "(binary file, diff not shown)",
            ),
            false,
            true,
        ));
    }
    let text = String::from_utf8_lossy(&buf);
    let count = text.lines().count();
    let mut patch = format!(
        "diff --git a/{rel} b/{rel}\nnew file mode 100644\n--- /dev/null\n+++ b/{rel}\n@@ -0,0 +1,{count} @@\n"
    );
    for line in text.lines() {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    let truncated = full
        .metadata()
        .map(|m| m.len() > MAX_PATCH_BYTES as u64)
        .unwrap_or(false);
    if truncated {
        patch.push_str(&crate::i18n::tr(
            "…（文件过大，仅显示前段内容）\n",
            "… (file too large, only the beginning is shown)\n",
        ));
    }
    Ok((patch, truncated, false))
}

#[tauri::command]
pub fn task_review(state: State<'_, AppState>, task_id: String) -> Result<ReviewSnapshot> {
    let ctx = task_ctx(&state, &task_id)?;
    let empty = |reason: &str| ReviewSnapshot {
        available: false,
        reason: Some(reason.to_string()),
        work_dir: ctx
            .work_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        repo_root: ctx
            .project_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        files: Vec::new(),
        additions: 0,
        deletions: 0,
        pending_commits: 0,
        can_apply: false,
        truncated: false,
    };
    let Some(dir) = ctx.work_dir.clone() else {
        return Ok(empty(&crate::i18n::tr(
            "未启用隔离工作区，改动直接写入项目目录",
            "No isolated workspace is enabled; changes are written directly to the project directory",
        )));
    };
    if !dir.is_dir() {
        return Ok(empty(&crate::i18n::tr(
            "隔离工作区已不存在",
            "The isolated workspace no longer exists",
        )));
    }
    if !is_git_repo(&dir) {
        return Ok(empty(&crate::i18n::tr(
            "隔离工作区不是 Git 仓库",
            "The isolated workspace is not a Git repository",
        )));
    }

    let entries = status_entries(&dir)?;
    let stats = numstat(&dir)?;
    let truncated = entries.len() > MAX_FILES;
    let mut files = Vec::new();
    let mut additions = 0;
    let mut deletions = 0;
    for (code, path) in entries.into_iter().take(MAX_FILES) {
        let (add, del, binary) = if code == "??" {
            untracked_stats(&dir, &path)
        } else {
            stats.get(&path).copied().unwrap_or((0, 0, false))
        };
        additions += add;
        deletions += del;
        files.push(ReviewFile {
            status: classify(&code).to_string(),
            code,
            path,
            additions: add,
            deletions: del,
            binary,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let pending_commits = match (head_sha(&dir), ctx.project_path.as_deref()) {
        (Some(head), Some(project)) if is_git_repo(project) => match head_sha(project) {
            Some(base) => unapplied_commits(project, &base, &head)
                .map(|commits| commits.len() as i64)
                .unwrap_or(0),
            None => 0,
        },
        _ => 0,
    };

    Ok(ReviewSnapshot {
        available: true,
        reason: None,
        work_dir: Some(dir.to_string_lossy().to_string()),
        repo_root: ctx
            .project_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        files,
        additions,
        deletions,
        pending_commits,
        can_apply: pending_commits > 0,
        truncated,
    })
}

#[tauri::command]
pub fn task_review_diff(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
) -> Result<ReviewDiff> {
    let ctx = task_ctx(&state, &task_id)?;
    let dir = active_work_dir(&ctx)?;
    let rel = safe_rel_path(&path)?;

    let tracked = git(&dir, &["ls-files", "--error-unmatch", "--", rel])
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !tracked {
        let (patch, truncated, binary) = untracked_patch(&dir, rel)?;
        return Ok(ReviewDiff {
            path: rel.to_string(),
            patch,
            truncated,
            binary,
        });
    }

    let out = git(&dir, &["diff", "HEAD", "--", rel])?;
    if !out.status.success() {
        return Err(AppError::Msg(stderr_of(&out)));
    }
    let mut patch = String::from_utf8_lossy(&out.stdout).to_string();
    if patch.trim().is_empty() {
        patch = crate::i18n::tr("（无差异）", "(no diff)");
    }
    let (patch, truncated) = truncate_text(patch, MAX_PATCH_BYTES);
    Ok(ReviewDiff {
        path: rel.to_string(),
        patch,
        truncated,
        binary: false,
    })
}

fn status_code_for(dir: &Path, rel: &str) -> Option<String> {
    status_entries(dir)
        .ok()?
        .into_iter()
        .find(|(_, path)| path == rel)
        .map(|(code, _)| code)
}

#[tauri::command]
pub fn task_review_revert(state: State<'_, AppState>, task_id: String, path: String) -> Result<()> {
    let ctx = task_ctx(&state, &task_id)?;
    ensure_idle(&ctx)?;
    let dir = active_work_dir(&ctx)?;
    let rel = safe_rel_path(&path)?;
    let Some(code) = status_code_for(&dir, rel) else {
        return Ok(());
    };
    if code.contains('R') || code.contains('C') {
        return Err("重命名或复制的文件请使用「全部还原」".into());
    }
    if code == "??" || code.contains('A') {
        let _ = git(&dir, &["reset", "HEAD", "--", rel]);
        let full = dir.join(rel);
        if full.is_dir() {
            fs::remove_dir_all(&full)?;
        } else if full.exists() {
            fs::remove_file(&full)?;
        }
    } else {
        let out = git(&dir, &["checkout", "HEAD", "--", rel])?;
        if !out.status.success() {
            return Err(AppError::Msg(stderr_of(&out)));
        }
    }
    let conn = state.conn()?;
    refresh_review_flag(&conn, &task_id)?;
    Ok(())
}

#[tauri::command]
pub fn task_review_revert_all(state: State<'_, AppState>, task_id: String) -> Result<()> {
    let ctx = task_ctx(&state, &task_id)?;
    ensure_idle(&ctx)?;
    let dir = active_work_dir(&ctx)?;
    let out = git(&dir, &["reset", "--hard", "HEAD"])?;
    if !out.status.success() {
        return Err(AppError::Msg(stderr_of(&out)));
    }
    let out = git(&dir, &["clean", "-fd"])?;
    if !out.status.success() {
        return Err(AppError::Msg(stderr_of(&out)));
    }
    let conn = state.conn()?;
    refresh_review_flag(&conn, &task_id)?;
    Ok(())
}

fn has_identity(dir: &Path) -> bool {
    git(dir, &["config", "user.email"])
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// 按时间顺序列出 base..head 之间尚未应用的提交。
fn pending_commits(repo: &Path, base: &str, head: &str) -> Result<Vec<String>> {
    let out = git(repo, &["rev-list", "--reverse", &format!("{base}..{head}")])?;
    if !out.status.success() {
        return Err(AppError::Msg(stderr_of(&out)));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// 过滤出尚未被项目应用的提交：`git cherry` 会把补丁等价的提交标记为已应用，
/// 因此应用后再审查不会重复显示同一批提交。
fn unapplied_commits(repo: &Path, base: &str, head: &str) -> Result<Vec<String>> {
    let commits = pending_commits(repo, base, head)?;
    if commits.is_empty() {
        return Ok(commits);
    }
    let out = git(repo, &["cherry", base, head])?;
    if !out.status.success() {
        return Err(AppError::Msg(stderr_of(&out)));
    }
    let applied: HashSet<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix('-'))
        .map(|line| line.trim().to_string())
        .filter(|sha| !sha.is_empty())
        .collect();
    Ok(commits
        .into_iter()
        .filter(|sha| !applied.contains(sha))
        .collect())
}

#[tauri::command]
pub fn task_review_commit(
    state: State<'_, AppState>,
    task_id: String,
    message: String,
) -> Result<String> {
    let ctx = task_ctx(&state, &task_id)?;
    ensure_idle(&ctx)?;
    let dir = active_work_dir(&ctx)?;

    let out = git(&dir, &["add", "-A"])?;
    if !out.status.success() {
        return Err(AppError::Msg(stderr_of(&out)));
    }
    let staged = git(&dir, &["diff", "--cached", "--quiet"])?;
    if staged.status.success() {
        return Err("没有可提交的改动".into());
    }
    let msg = message.trim();
    let msg = if msg.is_empty() {
        "Lumen 隔离工作区改动"
    } else {
        msg
    };
    let out = if has_identity(&dir) {
        git(&dir, &["commit", "-m", msg])?
    } else {
        git(
            &dir,
            &[
                "-c",
                "user.name=Lumen",
                "-c",
                "user.email=lumen@local",
                "commit",
                "-m",
                msg,
            ],
        )?
    };
    if !out.status.success() {
        return Err(AppError::Msg(stderr_of(&out)));
    }
    let out = git(&dir, &["rev-parse", "--short", "HEAD"])?;
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let conn = state.conn()?;
    refresh_review_flag(&conn, &task_id)?;
    Ok(sha)
}

#[tauri::command]
pub fn task_review_apply(state: State<'_, AppState>, task_id: String) -> Result<String> {
    let ctx = task_ctx(&state, &task_id)?;
    ensure_idle(&ctx)?;
    let dir = active_work_dir(&ctx)?;
    let Some(project) = ctx.project_path.clone() else {
        return Err("任务未关联项目目录".into());
    };
    if !is_git_repo(&project) {
        return Err("项目目录不是 Git 仓库，无法应用改动".into());
    }
    let Some(head) = head_sha(&dir) else {
        return Err("隔离工作区没有提交".into());
    };
    let project_head = head_sha(&project).ok_or("项目仓库没有提交")?;
    if head == project_head {
        return Err("隔离工作区没有新的提交".into());
    }
    let dirty = git(&project, &["status", "--porcelain"])?;
    if !dirty.status.success() {
        return Err(AppError::Msg(stderr_of(&dirty)));
    }
    if !String::from_utf8_lossy(&dirty.stdout).trim().is_empty() {
        return Err("项目目录存在未提交改动，请先处理后再应用".into());
    }

    let commits = unapplied_commits(&project, &project_head, &head)?;
    if commits.is_empty() {
        return Err("没有待应用的提交；项目已包含隔离工作区的改动".into());
    }
    // 按顺序逐个应用未应用提交，失败时 abort 回到序列开始前，避免只应用一部分。
    let mut args: Vec<&str> = Vec::new();
    if !has_identity(&project) {
        args.extend(["-c", "user.name=Lumen", "-c", "user.email=lumen@local"]);
    }
    args.push("cherry-pick");
    args.extend(commits.iter().map(String::as_str));
    let out = git(&project, &args)?;
    if !out.status.success() {
        let _ = git(&project, &["cherry-pick", "--abort"]);
        let err = stderr_of(&out);
        return Err(AppError::Msg(format!(
            "应用 {} 个提交失败，已回滚：{err}",
            commits.len()
        )));
    }
    let out = git(&project, &["rev-parse", "--short", "HEAD"])?;
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let conn = state.conn()?;
    refresh_review_flag(&conn, &task_id)?;
    Ok(sha)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewBranchInfo {
    pub available: bool,
    pub reason: Option<String>,
    pub current: Option<String>,
    pub detached: bool,
    pub branches: Vec<String>,
}

fn branch_of(dir: &Path) -> (Option<String>, bool) {
    let Ok(out) = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]) else {
        return (None, false);
    };
    if !out.status.success() {
        return (None, false);
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() {
        (None, false)
    } else if name == "HEAD" {
        (None, true)
    } else {
        (Some(name), false)
    }
}

fn branches_of(dir: &Path) -> Vec<String> {
    let Ok(out) = git(
        dir,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    ) else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn checked_branch_name(dir: &Path, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("分支名不能为空".into());
    }
    let out = git(dir, &["check-ref-format", "--branch", name])?;
    if !out.status.success() {
        return Err(AppError::Msg(format!("非法分支名：{name}")));
    }
    Ok(name.to_string())
}

/// 分支信息：仅隔离工作区可切换/新建，避免改动用户当前检出。
#[tauri::command]
pub fn task_review_branch_info(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<ReviewBranchInfo> {
    let ctx = task_ctx(&state, &task_id)?;
    let Some(dir) = ctx.work_dir.clone() else {
        return Ok(ReviewBranchInfo {
            available: false,
            reason: Some(crate::i18n::tr(
                "分支操作仅用于隔离工作区",
                "Branch operations only apply to the isolated workspace",
            )),
            current: None,
            detached: false,
            branches: Vec::new(),
        });
    };
    if !dir.is_dir() || !is_git_repo(&dir) {
        return Ok(ReviewBranchInfo {
            available: false,
            reason: Some(crate::i18n::tr(
                "隔离工作区不是可用的 Git 仓库",
                "The isolated workspace is not a usable Git repository",
            )),
            current: None,
            detached: false,
            branches: Vec::new(),
        });
    }
    let (current, detached) = branch_of(&dir);
    Ok(ReviewBranchInfo {
        available: true,
        reason: None,
        current,
        detached,
        branches: branches_of(&dir),
    })
}

#[tauri::command]
pub fn task_review_branch_create(
    state: State<'_, AppState>,
    task_id: String,
    name: String,
) -> Result<String> {
    let ctx = task_ctx(&state, &task_id)?;
    ensure_idle(&ctx)?;
    let dir = active_work_dir(&ctx)?;
    let name = checked_branch_name(&dir, &name)?;
    let out = git(&dir, &["switch", "-c", &name])?;
    if !out.status.success() {
        return Err(AppError::Msg(stderr_of(&out)));
    }
    Ok(name)
}

#[tauri::command]
pub fn task_review_branch_switch(
    state: State<'_, AppState>,
    task_id: String,
    name: String,
) -> Result<String> {
    let ctx = task_ctx(&state, &task_id)?;
    ensure_idle(&ctx)?;
    let dir = active_work_dir(&ctx)?;
    let name = name.trim().to_string();
    if !branches_of(&dir).iter().any(|branch| branch == &name) {
        return Err(AppError::Msg(format!("分支不存在：{name}")));
    }
    let out = git(&dir, &["switch", &name])?;
    if !out.status.success() {
        return Err(AppError::Msg(format!(
            "切换分支失败（可能有未提交改动）：{}",
            stderr_of(&out)
        )));
    }
    Ok(name)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotItem {
    pub path: String,
    pub name: String,
    pub created_at: i64,
    pub size: i64,
}

/// 运行前快照列表：按任务过滤，最新在前。
#[tauri::command]
pub fn task_snapshots(state: State<'_, AppState>, task_id: String) -> Result<Vec<SnapshotItem>> {
    let dir = state.base_dir.join("snapshots");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let prefix = format!("{task_id}-");
    let mut items: Vec<SnapshotItem> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(&prefix) || !name.ends_with(".patch") {
                return None;
            }
            let meta = entry.metadata().ok()?;
            let created_at = name
                .trim_start_matches(&prefix)
                .trim_end_matches(".patch")
                .parse::<i64>()
                .ok()
                .unwrap_or(0);
            Some(SnapshotItem {
                path: path.to_string_lossy().to_string(),
                name,
                created_at,
                size: meta.len() as i64,
            })
        })
        .collect();
    items.sort_by_key(|a| std::cmp::Reverse(a.created_at));
    Ok(items)
}

/// 回滚到运行前快照：先为当前状态再留一份快照，再重置已跟踪文件并应用历史补丁。
#[tauri::command]
pub fn task_snapshot_rollback(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
) -> Result<String> {
    let ctx = task_ctx(&state, &task_id)?;
    ensure_idle(&ctx)?;
    let Some(project) = ctx.project_path.clone() else {
        return Err("任务未关联项目目录".into());
    };
    if !is_git_repo(&project) {
        return Err("项目目录不是 Git 仓库，无法回滚".into());
    }
    let snapshot_dir = state.base_dir.join("snapshots");
    let canonical_dir = snapshot_dir
        .canonicalize()
        .map_err(|_| AppError::Msg("快照目录不存在".into()))?;
    let full = PathBuf::from(&path)
        .canonicalize()
        .map_err(|_| AppError::Msg("快照文件不存在".into()))?;
    if !full.starts_with(&canonical_dir) {
        return Err("快照路径不在数据目录内，已拒绝".into());
    }
    let content = fs::read_to_string(&full)?;
    if !content.starts_with("# Lumen 运行前快照") {
        return Err("不是有效的 Lumen 快照文件".into());
    }
    let snapshot_head = content
        .lines()
        .find_map(|line| line.strip_prefix("# HEAD: "))
        .map(str::trim)
        .unwrap_or("");
    let current_head = head_sha(&project).ok_or("项目仓库没有提交")?;
    if !snapshot_head.is_empty() && snapshot_head != "unknown" && snapshot_head != current_head {
        return Err(AppError::Msg(format!(
            "项目当前提交与快照不一致（快照 {} / 当前 {}），请先确认分支后再回滚",
            &snapshot_head[..snapshot_head.len().min(8)],
            &current_head[..current_head.len().min(8)]
        )));
    }
    // 回滚前再存一份当前状态，回滚本身也可撤销。
    let safety = crate::snapshot::create_pre_run_snapshot(&project, &state.base_dir, &task_id)?;
    let patch_body: String = content
        .lines()
        .skip_while(|line| line.starts_with('#') || line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let apply_patch = !patch_body.trim().is_empty() && !patch_body.starts_with('（');
    let tmp = snapshot_dir.join(format!(".apply-{}-{}.patch", task_id, crate::db::now_ms()));
    let result = (|| -> Result<()> {
        let out = git(&project, &["reset", "--hard", "HEAD"])?;
        if !out.status.success() {
            return Err(AppError::Msg(stderr_of(&out)));
        }
        if apply_patch {
            fs::write(&tmp, patch_body.as_bytes())?;
            let tmp_arg = tmp.to_string_lossy().to_string();
            let mut out = git(&project, &["apply", "--binary", &tmp_arg])?;
            if !out.status.success() {
                out = git(&project, &["apply", "--binary", "--3way", &tmp_arg])?;
            }
            if !out.status.success() {
                return Err(AppError::Msg(format!(
                    "补丁应用失败，当前状态已保留快照：{}；错误：{}",
                    safety.clone().unwrap_or_else(|| "（未生成）".into()),
                    stderr_of(&out)
                )));
            }
        }
        Ok(())
    })();
    let _ = fs::remove_file(&tmp);
    result?;
    let conn = state.conn()?;
    refresh_review_flag(&conn, &task_id)?;
    Ok(format!(
        "已回滚到快照 {}；回滚前状态已保存{}",
        full.file_name().and_then(|v| v.to_str()).unwrap_or(""),
        safety
            .and_then(|value| PathBuf::from(value)
                .file_name()
                .map(|n| n.to_string_lossy().to_string()))
            .map(|name| format!("：{name}"))
            .unwrap_or_default()
    ))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrOutcome {
    /// created：已通过 gh 创建；manual：需手动执行给出的命令；blocked：缺少前置条件。
    pub mode: String,
    pub url: Option<String>,
    pub command: String,
    pub message: Option<String>,
}

fn display_quote(text: &str) -> String {
    text.replace('"', "\\\"")
}

/// 创建 PR：检测到 gh 且已登录时直接调用，否则生成可复制的命令与说明。
#[tauri::command]
pub fn task_review_pr(
    state: State<'_, AppState>,
    task_id: String,
    title: String,
    body: String,
) -> Result<PrOutcome> {
    let ctx = task_ctx(&state, &task_id)?;
    let repo = ctx
        .project_path
        .clone()
        .ok_or_else(|| AppError::Msg("任务未关联项目目录".into()))?;
    if !is_git_repo(&repo) {
        return Err("项目目录不是 Git 仓库".into());
    }
    let source = ctx.work_dir.clone().unwrap_or_else(|| repo.clone());
    if !is_git_repo(&source) {
        return Err("隔离工作区不是 Git 仓库".into());
    }
    let (branch, detached) = branch_of(&source);
    let title = if title.trim().is_empty() {
        crate::i18n::tr("Lumen 任务交付", "Lumen task delivery")
    } else {
        title.trim().to_string()
    };
    let body = body.trim().to_string();
    let remote = git(&repo, &["remote", "get-url", "origin"])
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|value| !value.is_empty());
    let Some(branch) = branch else {
        return Ok(PrOutcome {
            mode: "blocked".into(),
            url: None,
            command: String::new(),
            message: Some(if detached {
                crate::i18n::tr(
                    "当前处于游离 HEAD，请先在变更审查中新建分支并提交",
                    "You are on a detached HEAD. Create and commit a branch in the change review first",
                )
            } else {
                crate::i18n::tr("无法识别当前分支", "Unable to identify the current branch")
            }),
        });
    };
    let command = format!(
        "git push -u origin {branch}\ngh pr create --head {branch} --title \"{}\" --body \"{}\"",
        display_quote(&title),
        display_quote(&body)
    );
    let Some(remote) = remote else {
        return Ok(PrOutcome {
            mode: "blocked".into(),
            url: None,
            command,
            message: Some(crate::i18n::tr(
                "项目未配置 origin 远程仓库",
                "No origin remote is configured for the project",
            )),
        });
    };

    let gh = Command::new("gh")
        .args(["--version"])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    let gh_auth = gh
        && Command::new("gh")
            .args(["auth", "status"])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
    if !gh || !gh_auth {
        return Ok(PrOutcome {
            mode: "manual".into(),
            url: None,
            command,
            message: Some(if gh {
                crate::i18n::tr(
                    &format!("gh 未登录，请先执行 gh auth login。远程仓库：{remote}"),
                    &format!("gh is not logged in. Run gh auth login first. Remote: {remote}"),
                )
            } else {
                crate::i18n::tr(
                    &format!("未检测到 gh CLI，请手动执行命令。远程仓库：{remote}"),
                    &format!("gh CLI was not found. Run the command manually. Remote: {remote}"),
                )
            }),
        });
    }

    let mut cmd = Command::new("gh");
    cmd.current_dir(&repo).args([
        "pr", "create", "--head", &branch, "--title", &title, "--body", &body,
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .output()
        .map_err(|e| AppError::Msg(format!("调用 gh 失败：{e}")))?;
    if !out.status.success() {
        return Ok(PrOutcome {
            mode: "manual".into(),
            url: None,
            command,
            message: Some(crate::i18n::tr(
                &format!(
                    "gh 创建 PR 失败（分支可能尚未推送）：{}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
                &format!(
                    "gh failed to create the PR (the branch may not be pushed yet): {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            )),
        });
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let url = stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("http://") || line.starts_with("https://"))
        .map(str::to_string);
    Ok(PrOutcome {
        mode: "created".into(),
        url,
        command,
        message: None,
    })
}

fn remove_worktree(state: &AppState, dir: &Path, project: Option<&Path>) -> bool {
    let root = state.base_dir.join("worktrees");
    if !dir.starts_with(&root) {
        return false;
    }
    if let Some(project) = project {
        if is_git_repo(project) {
            let dir_str = dir.to_string_lossy().to_string();
            let out = git(project, &["worktree", "remove", "--force", &dir_str]);
            if out.map(|o| o.status.success()).unwrap_or(false) {
                return true;
            }
        }
    }
    fs::remove_dir_all(dir).is_ok()
}

/// 任务删除时的尽力清理，失败不影响删除结果。
pub fn cleanup_worktree_best_effort(
    state: &AppState,
    work_dir: Option<&str>,
    project_path: Option<&str>,
) {
    let Some(dir) = work_dir else { return };
    let dir = PathBuf::from(dir);
    if dir.exists() {
        let _ = remove_worktree(state, &dir, project_path.map(Path::new));
    }
}

#[tauri::command]
pub fn task_review_cleanup(state: State<'_, AppState>, task_id: String) -> Result<()> {
    let ctx = task_ctx(&state, &task_id)?;
    ensure_idle(&ctx)?;
    let Some(dir) = ctx.work_dir.clone() else {
        return Ok(());
    };
    if !remove_worktree(&state, &dir, ctx.project_path.as_deref()) {
        return Err("隔离工作区不在可清理目录内，已拒绝删除".into());
    }
    let conn = state.conn()?;
    conn.execute(
        "UPDATE tasks SET work_dir = NULL WHERE id = ?1",
        rusqlite::params![task_id],
    )?;
    refresh_review_flag(&conn, &task_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_repo() -> PathBuf {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "lumen-review-test-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .current_dir(&dir)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {:?} 失败：{}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@local"]);
        run(&["config", "user.name", "Test"]);
        fs::write(dir.join("a.txt"), "one\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn parses_modified_and_untracked_files() {
        let dir = temp_repo();
        fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        fs::write(dir.join("b.txt"), "x\ny\n").unwrap();
        let entries = status_entries(&dir).unwrap();
        assert!(entries
            .iter()
            .any(|(code, path)| code == "M" && path == "a.txt"));
        assert!(entries
            .iter()
            .any(|(code, path)| code == "??" && path == "b.txt"));
        let stats = numstat(&dir).unwrap();
        assert_eq!(stats.get("a.txt").copied(), Some((1, 0, false)));
        assert_eq!(untracked_stats(&dir, "b.txt"), (2, 0, false));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lists_pending_commits_in_order() {
        let dir = temp_repo();
        let base = head_sha(&dir).unwrap();
        fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        assert!(git(&dir, &["commit", "-am", "second"])
            .unwrap()
            .status
            .success());
        fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        assert!(git(&dir, &["commit", "-am", "third"])
            .unwrap()
            .status
            .success());
        let head = head_sha(&dir).unwrap();
        let commits = pending_commits(&dir, &base, &head).unwrap();
        assert_eq!(commits.len(), 2);
        let second = String::from_utf8_lossy(&git(&dir, &["rev-parse", "HEAD~1"]).unwrap().stdout)
            .trim()
            .to_string();
        assert_eq!(commits[0], second);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tracks_pending_review_before_and_after_apply() {
        let repo = temp_repo();
        let worktree = PathBuf::from(format!("{}-wt", repo.to_string_lossy()));
        let add = Command::new("git")
            .current_dir(&repo)
            .args(["worktree", "add", "--detach"])
            .arg(&worktree)
            .arg("HEAD")
            .output()
            .unwrap();
        assert!(
            add.status.success(),
            "创建 worktree 失败：{}",
            String::from_utf8_lossy(&add.stderr)
        );

        assert!(!has_pending_review(Some(&worktree), Some(&repo)));

        // 未提交改动即视为待审查。
        fs::write(worktree.join("a.txt"), "one\ntwo\n").unwrap();
        assert!(has_pending_review(Some(&worktree), Some(&repo)));

        // 提交后尚未应用到项目，仍待审查。
        assert!(git(&worktree, &["commit", "-am", "wt change"])
            .unwrap()
            .status
            .success());
        let wt_head = head_sha(&worktree).unwrap();
        let project_head = head_sha(&repo).unwrap();
        assert_eq!(
            unapplied_commits(&repo, &project_head, &wt_head)
                .unwrap()
                .len(),
            1
        );
        assert!(has_pending_review(Some(&worktree), Some(&repo)));

        // 项目以补丁等价方式应用后，不再待审查，避免重复应用。
        assert!(git(&repo, &["cherry-pick", &wt_head])
            .unwrap()
            .status
            .success());
        let applied_head = head_sha(&repo).unwrap();
        assert_eq!(
            unapplied_commits(&repo, &applied_head, &wt_head)
                .unwrap()
                .len(),
            0
        );
        assert!(!has_pending_review(Some(&worktree), Some(&repo)));

        let wt_str = worktree.to_string_lossy().to_string();
        let _ = git(&repo, &["worktree", "remove", "--force", &wt_str]);
        fs::remove_dir_all(&worktree).ok();
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn rejects_unsafe_paths() {
        assert!(safe_rel_path("../secret").is_err());
        assert!(safe_rel_path("C:\\Windows\\system32").is_err());
        assert!(safe_rel_path("src/app.ts").is_ok());
    }
}
