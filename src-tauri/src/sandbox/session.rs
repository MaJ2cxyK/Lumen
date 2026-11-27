//! 运行时沙箱会话：为单次 Agent 运行准备 AppContainer、ACL 与 relay 桥。
//!
//! 生命周期：
//! 1. `prepare` 确保 profile 存在，给项目目录/运行目录/沙箱 HOME 授予 ACL；
//! 2. 为每条上游端口（模型代理、审批网关）分配容器内端口并启动宿主桥；
//! 3. 运行结束后 `Drop` 停止桥线程；ACL 保留到关闭沙箱/卸载时统一回收。
//!
//! 并发约定：同一项目可能被多次运行同时使用，项目目录的 ACL 用全局引用计数管理，
//! 沙箱开启期间授权跨运行复用，关闭沙箱/卸载时统一回收；CLI 目录、HOME 与祖先目录
//! 同样跨运行复用，避免每次运行都遍历数十万文件。

use crate::sandbox::acl;
use crate::sandbox::appcontainer::{delete_profile, derive_sid, ensure_profile, AppContainerSid};
use crate::sandbox::host::HostSpec;
use crate::sandbox::relay::{self, BridgeHandle, RelayChannel};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// 运行时共享的 AppContainer profile 名（与产品 identifier 一致）。
pub const RUN_PROFILE: &str = "com.agenthub.desktop";

/// 单次运行的沙箱上下文。
pub struct SandboxSession {
    profile: String,
    sid: AppContainerSid,
    channels: Vec<RelayChannel>,
    ports: HashMap<String, u16>,
    /// 本会话持有引用的项目目录；ACL 由关闭沙箱时的清理统一回收。
    projects: Vec<PathBuf>,
    bridges: Vec<BridgeHandle>,
}

/// ACL marker 的内容：记录被授权的路径与类型，供关闭沙箱/卸载时回收。
#[derive(Serialize, Deserialize)]
struct MarkerRecord {
    path: String,
    kind: String,
}

/// 项目目录的全局引用计数，键为规范化后的项目路径。
fn project_refs() -> &'static Mutex<HashMap<PathBuf, usize>> {
    static REFS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
    REFS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取得项目目录的访问权；首个引用只在没有复用 marker 时授予整棵树。
fn acquire_project(
    path: &Path,
    sid: &AppContainerSid,
    marker_root: &Path,
    sid_text: &str,
) -> Result<(), String> {
    let key = path.to_path_buf();
    let mut refs = project_refs()
        .lock()
        .map_err(|_| "项目 ACL 引用计数锁损坏".to_string())?;
    let count = refs.entry(key).or_insert(0);
    if *count == 0 {
        // 大项目整树授权可能要数十秒，沙箱开启期间复用；关闭沙箱时由 cleanup 回收。
        if !cached(marker_root, path, "project", sid_text) {
            acl::grant_tree(path, sid, acl::DIR_FULL, "F")?;
            // 记录项目路径；应用崩溃或关闭沙箱时据此回收残留 ACE。
            mark(marker_root, path, "project", sid_text);
        }
    }
    *count += 1;
    Ok(())
}

/// 释放项目目录引用；ACL 保留到关闭沙箱时的统一清理。
fn release_project(path: &Path) {
    let Ok(mut refs) = project_refs().lock() else {
        return;
    };
    let Some(count) = refs.get_mut(path) else {
        return;
    };
    *count = count.saturating_sub(1);
    if *count == 0 {
        refs.remove(path);
    }
}

// SID 句柄只在持有线程内使用，随会话整体跨线程移动是安全的。
unsafe impl Send for SandboxSession {}

impl SandboxSession {
    /// 准备沙箱。`upstreams` 为 `(名称, 宿主侧端口)`，名称用于回查容器内端口。
    pub fn prepare(
        run_id: &str,
        run_dir: &Path,
        project_path: &Path,
        home_dir: &Path,
        program: &Path,
        extra_paths: &[PathBuf],
        upstreams: &[(&str, u16)],
    ) -> Result<Self, String> {
        let (sid, _) = ensure_profile(RUN_PROFILE)?;
        let sid_text = unsafe { crate::sandbox::appcontainer::sid_to_string(sid.as_ptr())? };
        // CLI 安装目录与沙箱 HOME 的授权结果跨运行复用：只授权一次，避免每次遍历
        // 数十万文件的 node_modules；项目目录同样复用，关闭沙箱时统一回收。
        let marker_root = home_dir.parent().unwrap_or(home_dir).join("sandbox-acl");
        let _ = std::fs::create_dir_all(&marker_root);
        let mut session = Self {
            profile: RUN_PROFILE.to_string(),
            sid,
            channels: Vec::new(),
            ports: HashMap::new(),
            projects: Vec::new(),
            bridges: Vec::new(),
        };
        let sid_ptr = session.sid.as_ptr();

        // 项目目录需要读写改删：既有文件通过 icacls 传播，新建文件靠继承。
        acquire_project(project_path, &session.sid, &marker_root, &sid_text)?;
        session.projects.push(project_path.to_path_buf());

        // 运行目录存放 settings.json、MCP 配置与 relay 就绪文件；目录随后会被整体删除，
        // 因此不需要逐项回收 ACL。
        acl::grant(run_dir, sid_ptr, acl::DIR_FULL, true)?;
        // 授权前已存在的文件不会继承新 ACE，需要逐个补授。
        if let Ok(entries) = std::fs::read_dir(run_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let _ = acl::grant(&path, sid_ptr, acl::FILE_READ, false);
                }
            }
        }
        // 沙箱 HOME：CLI 的配置、会话记录与缓存都落在这里。
        std::fs::create_dir_all(home_dir)
            .map_err(|e| format!("创建沙箱 HOME {} 失败：{e}", home_dir.display()))?;
        if !cached(&marker_root, home_dir, "tree", &sid_text) {
            acl::grant_tree(home_dir, &session.sid, acl::DIR_FULL, "F")?;
            mark(&marker_root, home_dir, "tree", &sid_text);
        }
        // 容器内要启动的程序（CLI 或宿主 exe）必须可读可执行；授权结果跨运行复用。
        if !cached(&marker_root, program, "exe", &sid_text) {
            acl::grant(program, sid_ptr, acl::EXE_READ_EXECUTE, false)?;
            mark(&marker_root, program, "exe", &sid_text);
        }
        for path in extra_paths {
            // 系统目录（cmd.exe、Program Files 下的运行时）通常已对 AppContainer 开放，
            // 授权失败不应阻断整个沙箱，只记录日志。
            let kind = if path.is_dir() {
                Some("tree")
            } else if path.is_file() {
                Some("file")
            } else {
                None
            };
            let Some(kind) = kind else {
                continue;
            };
            if !cached(&marker_root, path, kind, &sid_text) {
                let granted = if kind == "tree" {
                    acl::grant_tree(path, &session.sid, acl::EXE_READ_EXECUTE, "RX")
                } else {
                    acl::grant(path, sid_ptr, acl::EXE_READ_EXECUTE, false)
                };
                match granted {
                    Ok(()) => mark(&marker_root, path, kind, &sid_text),
                    Err(err) => tracing::warn!("沙箱授权 {} 失败：{err}", path.display()),
                }
            }
            // 额外路径同样需要祖先可穿越，例如 %APPDATA%\npm 的多级父目录。
            // 祖先目录可能被其他运行共用，授权后不回收，只记录供清理。
            for dir in acl::grant_ancestors(path, sid_ptr) {
                mark(&marker_root, &dir, "dir", &sid_text);
            }
        }
        // 祖先目录只需可穿越；系统目录改不了就跳过。授权保留到清理阶段。
        for path in [project_path, run_dir, home_dir, program] {
            for dir in acl::grant_ancestors(path, sid_ptr) {
                mark(&marker_root, &dir, "dir", &sid_text);
            }
        }
        let sddl = relay::pipe_sddl(&session.sid)?;
        for (index, (kind, target_port)) in upstreams.iter().enumerate() {
            let port = free_port()?;
            let up = format!(r"\\.\pipe\lumen-{run_id}-{index}-up");
            let down = format!(r"\\.\pipe\lumen-{run_id}-{index}-down");
            session.bridges.push(relay::start_bridge(
                up.clone(),
                down.clone(),
                *target_port,
                sddl.clone(),
            ));
            session.channels.push(RelayChannel {
                up_pipe: up,
                down_pipe: down,
                port,
            });
            session.ports.insert(kind.to_string(), port);
        }
        Ok(session)
    }

    /// 容器内对应的端口。
    pub fn container_port(&self, kind: &str) -> Option<u16> {
        self.ports.get(kind).copied()
    }

    /// 生成宿主进程配置。
    pub fn host_spec(
        &self,
        program: &Path,
        program_args: &[String],
        cwd: &Path,
        env: HashMap<String, String>,
        ready_file: &Path,
        status_file: &Path,
    ) -> HostSpec {
        HostSpec {
            profile: self.profile.clone(),
            capabilities: Vec::new(),
            channels: self.channels.clone(),
            ready_file: ready_file.to_string_lossy().to_string(),
            program: program.to_string_lossy().to_string(),
            program_args: program_args.to_vec(),
            cwd: cwd.to_string_lossy().to_string(),
            env,
            ready_timeout_ms: 15_000,
            status_file: Some(status_file.to_string_lossy().to_string()),
        }
    }
}

impl Drop for SandboxSession {
    fn drop(&mut self) {
        for bridge in &mut self.bridges {
            bridge.stop();
        }
        for path in &self.projects {
            release_project(path);
        }
    }
}

/// 关闭沙箱或应用启动时清理残留：撤销记录在案的 ACL、删除运行时 profile。
///
/// 只处理 marker 中登记过的路径；`sandbox-home` 内的 CLI 配置保留，重新开启后仍可复用。
/// 返回实际处理的项数，便于记录日志。
pub fn cleanup(base_dir: &Path, profile: &str) -> Result<usize, String> {
    let marker_root = base_dir.join("sandbox-acl");
    let mut records: Vec<MarkerRecord> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&marker_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("ok") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(record) = serde_json::from_str::<MarkerRecord>(&text) {
                    records.push(record);
                }
            }
        }
    }

    // 即使 profile 已被删除，仍可按名字派生 SID 回收 ACL。
    let sid = derive_sid(profile).ok();
    if let Some(sid) = &sid {
        for record in &records {
            let path = Path::new(&record.path);
            if record.kind == "tree" || record.kind == "project" {
                acl::revoke_tree(path, sid);
            } else {
                let _ = acl::revoke(path, sid.as_ptr());
            }
        }
    }

    // 回收完成后再删除记录，避免中途失败时丢失线索。
    let _ = std::fs::remove_dir_all(&marker_root);
    // profile 不存在时删除会失败，属于正常情况。
    let _ = delete_profile(profile);
    Ok(records.len())
}

/// 授权结果是否已存在（跨运行复用，避免重复遍历大目录）。
fn cached(marker_root: &Path, path: &Path, kind: &str, sid: &str) -> bool {
    marker_root
        .join(format!("{}.ok", marker_name(path, kind, sid)))
        .is_file()
}

/// 记录授权成功；写入失败只会导致下次重复授权，清理阶段会漏掉该路径。
fn mark(marker_root: &Path, path: &Path, kind: &str, sid: &str) {
    let record = MarkerRecord {
        path: path.to_string_lossy().to_string(),
        kind: kind.to_string(),
    };
    if let Ok(text) = serde_json::to_vec(&record) {
        let _ = std::fs::write(
            marker_root.join(format!("{}.ok", marker_name(path, kind, sid))),
            text,
        );
    }
}

fn marker_name(path: &Path, kind: &str, sid: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().to_lowercase().as_bytes());
    hasher.update(b"|");
    hasher.update(kind.as_bytes());
    hasher.update(b"|");
    hasher.update(sid.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 让操作系统分配一个空闲的回环端口，供容器内 relay 绑定。
fn free_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("分配沙箱 relay 端口失败：{e}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| format!("读取沙箱 relay 端口失败：{e}"))
}

/// 一次启动解析结果：容器内实际要运行的进程，以及 wrapper 脚本/依赖目录。
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub extra_paths: Vec<PathBuf>,
}

/// 解析启动方式并收集沙箱必须授权的路径。
///
/// `.cmd/.bat/.ps1` 会转换为解释器调用，脚本本身和同目录依赖树都要授予容器访问。
pub fn plan_launch(program: &str, args: &[String]) -> Result<LaunchPlan, String> {
    let (resolved, resolved_args) = resolve_launch(program, args)?;
    let mut extra_paths = vec![resolved.clone()];

    let mut scripts: Vec<PathBuf> = Vec::new();
    if Path::new(program).is_file() {
        scripts.push(Path::new(program).to_path_buf());
    }
    if resolved_args
        .first()
        .map(|arg| arg == "/C")
        .unwrap_or(false)
    {
        if let Some(script) = resolved_args.get(1) {
            scripts.push(PathBuf::from(script));
        }
    }
    if let Some(pos) = resolved_args.iter().position(|arg| arg == "-File") {
        if let Some(script) = resolved_args.get(pos + 1) {
            scripts.push(PathBuf::from(script));
        }
    }
    for script in scripts {
        if !extra_paths.contains(&script) {
            extra_paths.push(script.clone());
        }
        let ext = script
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(ext.as_str(), "cmd" | "bat" | "ps1") {
            if let Some(parent) = script.parent() {
                // npm 全局目录可能有数 GB；包装脚本通常只调用自己的 node_modules 包。
                // 能定位到包根时只授权该包，避免为了一个 exe 传播整棵全局目录。
                if let Some(package) = wrapper_package_root(&script) {
                    if !extra_paths.contains(&package) {
                        extra_paths.push(package);
                    }
                } else {
                    let parent = parent.to_path_buf();
                    if !extra_paths.contains(&parent) {
                        extra_paths.push(parent);
                    }
                }
            }
        }
    }

    Ok(LaunchPlan {
        program: resolved,
        args: resolved_args,
        extra_paths,
    })
}

/// 从 `.cmd/.bat/.ps1` 包装脚本里找出 `node_modules` 包根。
///
/// 只接受脚本目录内的真实路径，避免包装脚本内容把授权范围引到目录外。
fn wrapper_package_root(script: &Path) -> Option<PathBuf> {
    let parent = script.parent()?;
    if std::fs::metadata(script)
        .map(|m| m.len())
        .unwrap_or(u64::MAX)
        > 1024 * 1024
    {
        return None;
    }
    let content = std::fs::read_to_string(script).ok()?;
    let lower = content.to_ascii_lowercase();
    let mut offset = 0usize;
    while let Some(pos) = lower[offset..].find("node_modules") {
        let start = offset + pos;
        let tail = &content[start..];
        let end = tail
            .find(|c: char| {
                c == '"'
                    || c == '\''
                    || c == '>'
                    || c == '<'
                    || c == '|'
                    || c == ';'
                    || c == ')'
                    || c == '%'
                    || c.is_whitespace()
            })
            .unwrap_or(tail.len());
        let relative = tail[..end].trim_matches(['\\', '/']);
        let target = parent.join(relative);
        if target.is_file() {
            if let Some(root) = package_root(&target, parent) {
                return Some(root);
            }
        }
        offset = start + "node_modules".len();
    }
    None
}

/// 取 `...\node_modules\@scope\pkg` 或 `...\node_modules\pkg` 作为授权树根。
fn package_root(target: &Path, script_parent: &Path) -> Option<PathBuf> {
    let target = strip_verbatim(std::fs::canonicalize(target).ok()?);
    let script_parent = strip_verbatim(std::fs::canonicalize(script_parent).ok()?);
    let mut components = target.components();
    let mut root = PathBuf::new();
    let mut found = false;
    while let Some(component) = components.next() {
        root.push(component.as_os_str());
        let name = component.as_os_str().to_string_lossy();
        if !found && name.eq_ignore_ascii_case("node_modules") {
            found = true;
            let next = components.next()?;
            root.push(next.as_os_str());
            if next.as_os_str().to_string_lossy().starts_with('@') {
                let child = components.next()?;
                root.push(child.as_os_str());
            }
            break;
        }
    }
    if !found || !root.starts_with(&script_parent) || !root.is_dir() {
        return None;
    }
    Some(root)
}

/// 为沙箱 HOME 写入最小 Claude 配置：只跳过首启向导并关闭非必要联网，
/// 绝不复制宿主 `.claude.json` / `settings.json` 中的登录态或密钥。
pub fn bootstrap_claude_home(home: &Path) -> Result<(), String> {
    let config_path = home.join(".claude.json");
    if !config_path.exists() {
        let config = serde_json::json!({
            "hasCompletedOnboarding": true,
            "numStartups": 1
        });
        write_new_file(
            &config_path,
            serde_json::to_vec_pretty(&config)
                .map_err(|e| format!("序列化沙箱 Claude 配置失败：{e}"))?,
        )?;
    }

    let settings_dir = home.join(".claude");
    std::fs::create_dir_all(&settings_dir)
        .map_err(|e| format!("创建沙箱 Claude 设置目录失败：{e}"))?;
    let settings_path = settings_dir.join("settings.json");
    if !settings_path.exists() {
        let settings = serde_json::json!({
            "env": {
                "DISABLE_AUTOUPDATER": "1",
                "DISABLE_ERROR_REPORTING": "1",
                "DISABLE_TELEMETRY": "1",
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1"
            }
        });
        write_new_file(
            &settings_path,
            serde_json::to_vec_pretty(&settings)
                .map_err(|e| format!("序列化沙箱 Claude 设置失败：{e}"))?,
        )?;
    }
    Ok(())
}

/// 并发运行可能同时首次初始化沙箱 HOME：`create_new` 保证只有一个写入者。
fn write_new_file(path: &Path, content: Vec<u8>) -> Result<(), String> {
    use std::io::Write;

    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(err) => return Err(format!("创建沙箱配置文件 {} 失败：{err}", path.display())),
    };
    if let Err(err) = file.write_all(&content).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(format!("写入沙箱配置文件 {} 失败：{err}", path.display()));
    }
    Ok(())
}

/// 解析出可被 `CreateProcessW` 直接启动的绝对程序路径与参数。
///
/// 宿主模式下没有 shell 的 PATH 搜索，因此这里补上 PATH/PATHEXT 解析；
/// `.cmd/.bat/.ps1` 会被转换为对应解释器调用。
pub fn resolve_launch(program: &str, args: &[String]) -> Result<(PathBuf, Vec<String>), String> {
    let direct = Path::new(program);
    let resolved = if direct.is_file() {
        direct.to_path_buf()
    } else if program.contains('\\') || program.contains('/') {
        return Err(format!("找不到可执行文件：{program}"));
    } else {
        find_on_path(program).ok_or_else(|| format!("在 PATH 中找不到 {program}"))?
    };
    let resolved = strip_verbatim(std::fs::canonicalize(&resolved).unwrap_or(resolved));
    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "cmd" | "bat" => {
            let comspec = std::env::var_os("ComSpec")
                .map(PathBuf::from)
                .filter(|path| path.is_file())
                .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"));
            let mut full = vec!["/C".to_string(), resolved.to_string_lossy().to_string()];
            full.extend(args.iter().cloned());
            Ok((comspec, full))
        }
        "ps1" => {
            let shell = find_on_path("powershell").unwrap_or_else(|| {
                PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
            });
            let mut full = vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                resolved.to_string_lossy().to_string(),
            ];
            full.extend(args.iter().cloned());
            Ok((shell, full))
        }
        _ => Ok((resolved, args.to_vec())),
    }
}

/// 去掉 `\\?\` 前缀：cmd 与部分 CLI 不认 verbatim 路径。
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest.to_string())
    } else {
        path
    }
}

/// 按 PATH 与 PATHEXT 查找可执行文件。
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let has_ext = Path::new(name).extension().is_some();
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_string())
        .collect();
    for dir in std::env::split_paths(&path) {
        if has_ext {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
            continue;
        }
        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_revokes_grants_and_deletes_profile() {
        let profile = "com.agenthub.desktop.sandbox.cleanup-test";
        let root = std::env::temp_dir().join("lumen-sandbox-cleanup-test");
        let project = root.join("project");
        let base = root.join("base");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(base.join("sandbox-acl")).unwrap();
        std::fs::write(project.join("sample.txt"), b"x").unwrap();

        let (sid, _) = ensure_profile(profile).unwrap();
        acl::grant_tree(&project, &sid, acl::DIR_FULL, "F").unwrap();
        let sid_text =
            unsafe { crate::sandbox::appcontainer::sid_to_string(sid.as_ptr()).unwrap() };
        mark(&base.join("sandbox-acl"), &project, "project", &sid_text);

        let count = cleanup(&base, profile).unwrap();
        assert_eq!(count, 1);
        assert!(!base.join("sandbox-acl").exists(), "marker 目录未清理");
        // profile 已被删除：重新确保时应属于“本次新建”。
        let (_, created) = ensure_profile(profile).unwrap();
        assert!(created, "profile 未删除");
        let _ = delete_profile(profile);
        // 项目目录的容器 ACE 已移除。
        let output = std::process::Command::new("icacls")
            .arg(&project)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(!text.contains(&sid_text), "icacls 仍包含容器 SID：{text}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn wrapper_grants_only_the_npm_package() {
        let root = std::env::temp_dir().join("lumen-sandbox-wrapper-test");
        let npm = root.join("npm");
        let package = npm.join("node_modules").join("@scope").join("pkg");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("bin").join("tool.exe"), b"x").unwrap();
        let script = npm.join("tool.cmd");
        std::fs::write(
            &script,
            "@ECHO off\r\n\"%dp0%\\node_modules\\@scope\\pkg\\bin\\tool.exe\" %*\r\n",
        )
        .unwrap();

        let plan = plan_launch(&script.to_string_lossy(), &["--version".to_string()]).unwrap();
        assert!(
            plan.extra_paths.iter().any(|path| path.ends_with("pkg")),
            "未授权 npm 包目录：{:?}",
            plan.extra_paths
        );
        assert!(
            !plan.extra_paths.contains(&npm),
            "不应授权整个 npm 目录：{:?}",
            plan.extra_paths
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn project_acl_marker_is_path_and_sid_specific() {
        let root = std::env::temp_dir().join("lumen-sandbox-marker-test");
        let marker_root = root.join("sandbox-acl");
        let project = root.join("project");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&marker_root).unwrap();
        std::fs::create_dir_all(&project).unwrap();

        mark(&marker_root, &project, "project", "test-sid");
        assert!(cached(&marker_root, &project, "project", "test-sid"));
        assert!(!cached(&marker_root, &project, "project", "other-sid"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
