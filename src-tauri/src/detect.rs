use crate::db::{new_id, now_ms};
use crate::error::Result;
use crate::models::{CliVersionInfo, DetectedCli};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
enum InstallMethod {
    Npm(&'static str),
    Pip(&'static str),
}

impl InstallMethod {
    fn hint(self) -> String {
        match self {
            InstallMethod::Npm(pkg) => format!("npm i -g {pkg}"),
            InstallMethod::Pip(pkg) => format!("pip install {pkg}"),
        }
    }
}

struct CliSpec {
    name: &'static str,
    adapter: &'static str,
    commands: &'static [&'static str],
    recommended: bool,
    install: Option<InstallMethod>,
}

const SPECS: &[CliSpec] = &[
    CliSpec {
        name: "Claude Code",
        adapter: "claude-code",
        commands: &["claude"],
        recommended: true,
        install: Some(InstallMethod::Npm("@anthropic-ai/claude-code")),
    },
    CliSpec {
        name: "OpenCode",
        adapter: "opencode",
        commands: &["opencode"],
        recommended: true,
        install: Some(InstallMethod::Npm("opencode-ai")),
    },
    CliSpec {
        name: "Codex",
        adapter: "pty",
        commands: &["codex"],
        recommended: false,
        install: Some(InstallMethod::Npm("@openai/codex")),
    },
    CliSpec {
        name: "Gemini CLI",
        adapter: "pty",
        commands: &["gemini"],
        recommended: false,
        install: Some(InstallMethod::Npm("@google/gemini-cli")),
    },
    CliSpec {
        name: "Qwen Code",
        adapter: "pty",
        commands: &["qwen"],
        recommended: false,
        install: Some(InstallMethod::Npm("@qwen-code/qwen-code")),
    },
    CliSpec {
        name: "Cursor Agent",
        adapter: "pty",
        commands: &["cursor-agent"],
        recommended: false,
        install: None,
    },
    CliSpec {
        name: "GitHub Copilot CLI",
        adapter: "pty",
        commands: &["copilot"],
        recommended: false,
        install: Some(InstallMethod::Npm("@github/copilot")),
    },
    CliSpec {
        name: "Amp",
        adapter: "pty",
        commands: &["amp"],
        recommended: false,
        install: Some(InstallMethod::Npm("@sourcegraph/amp")),
    },
    CliSpec {
        name: "Cline",
        adapter: "pty",
        commands: &["cline"],
        recommended: false,
        install: Some(InstallMethod::Npm("cline")),
    },
    CliSpec {
        name: "Roo Code",
        adapter: "pty",
        commands: &["roo"],
        recommended: false,
        install: None,
    },
    CliSpec {
        name: "Kilo Code",
        adapter: "pty",
        commands: &["kilo"],
        recommended: false,
        install: Some(InstallMethod::Npm("@kilocode/cli")),
    },
    CliSpec {
        name: "Auggie",
        adapter: "pty",
        commands: &["auggie"],
        recommended: false,
        install: Some(InstallMethod::Npm("@augmentcode/auggie")),
    },
    CliSpec {
        name: "Droid",
        adapter: "pty",
        commands: &["droid"],
        recommended: false,
        install: None,
    },
    CliSpec {
        name: "Goose",
        adapter: "pty",
        commands: &["goose"],
        recommended: false,
        install: None,
    },
    CliSpec {
        name: "Crush",
        adapter: "pty",
        commands: &["crush"],
        recommended: false,
        install: Some(InstallMethod::Npm("@charmland/crush")),
    },
    CliSpec {
        name: "OpenHands",
        adapter: "pty",
        commands: &["openhands"],
        recommended: false,
        install: Some(InstallMethod::Pip("openhands-ai")),
    },
    CliSpec {
        name: "Aider",
        adapter: "pty",
        commands: &["aider"],
        recommended: false,
        install: Some(InstallMethod::Pip("aider-chat")),
    },
    CliSpec {
        name: "Kimi CLI",
        adapter: "pty",
        commands: &["kimi"],
        recommended: false,
        install: Some(InstallMethod::Pip("kimi-cli")),
    },
    CliSpec {
        name: "iFlow CLI",
        adapter: "pty",
        commands: &["iflow"],
        recommended: false,
        install: Some(InstallMethod::Npm("@iflow-ai/iflow-cli")),
    },
    CliSpec {
        name: "Trae Agent",
        adapter: "pty",
        commands: &["trae-agent"],
        recommended: false,
        install: Some(InstallMethod::Pip("trae-agent")),
    },
    CliSpec {
        name: "Qoder CLI",
        adapter: "pty",
        commands: &["qoder"],
        recommended: false,
        install: Some(InstallMethod::Npm("@qoder-ai/qodercli")),
    },
    CliSpec {
        name: "Plandex",
        adapter: "pty",
        commands: &["plandex"],
        recommended: false,
        install: None,
    },
    CliSpec {
        name: "Continue CLI",
        adapter: "pty",
        commands: &["cn"],
        recommended: false,
        install: Some(InstallMethod::Npm("@continuedev/cli")),
    },
    CliSpec {
        name: "CodeBuddy",
        adapter: "pty",
        commands: &["codebuddy"],
        recommended: false,
        install: Some(InstallMethod::Npm("@tencent-ai/codebuddy-code")),
    },
    CliSpec {
        name: "Pi",
        adapter: "pty",
        commands: &["pi"],
        recommended: false,
        install: None,
    },
    CliSpec {
        name: "Hermes",
        adapter: "pty",
        commands: &["hermes"],
        recommended: false,
        install: None,
    },
];

pub fn path_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var("PATH")
        .unwrap_or_default()
        .split(';')
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .collect();
    if let Ok(appdata) = std::env::var("APPDATA") {
        let appdata = PathBuf::from(appdata);
        dirs.push(appdata.join("npm"));
        let python = appdata.join("Python");
        if let Ok(entries) = std::fs::read_dir(&python) {
            for entry in entries.flatten() {
                let scripts = entry.path().join("Scripts");
                if scripts.is_dir() {
                    dirs.push(scripts);
                }
            }
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".bun").join("bin"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        dirs.push(local.join("Programs"));
        dirs.push(local.join("pnpm"));
        dirs.push(local.join("hermes").join("bin"));
        dirs.push(local.join("Microsoft").join("WinGet").join("Links"));
    }
    dirs
}

pub fn resolve(name: &str) -> Option<PathBuf> {
    for dir in path_dirs() {
        for ext in [".cmd", ".exe", ".bat", ""] {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn command_for(path: &Path) -> (String, Vec<String>) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if ext == "ps1" {
        (
            "powershell".to_string(),
            vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                path.to_string_lossy().to_string(),
            ],
        )
    } else if ext == "cmd" || ext == "bat" {
        (
            "cmd".to_string(),
            vec!["/C".to_string(), path.to_string_lossy().to_string()],
        )
    } else {
        (path.to_string_lossy().to_string(), vec![])
    }
}

pub fn version_of(path: &Path) -> Option<String> {
    let (program, mut args) = command_for(path);
    args.push("--version".to_string());
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(_) => break,
            None if start.elapsed() > Duration::from_secs(6) => {
                let _ = child.kill();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(80)),
        }
    }
    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let err = String::from_utf8_lossy(&out.stderr);
    let line = text
        .lines()
        .chain(err.lines())
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    if line.is_empty() {
        None
    } else {
        Some(line.chars().take(120).collect())
    }
}

/// 适配器真实能力：只有 Claude Code 解析结构化用量、桥接工具审批、支持断点续跑与预算硬停；
/// 其余 CLI 仅透传输出并按文本估算用量。UI 文案必须与此保持一致。
pub fn capabilities_for(adapter: &str) -> serde_json::Value {
    let claude = adapter == "claude-code";
    serde_json::json!({
        "streaming": true,
        "streamUsage": claude,
        "toolApproval": claude,
        "resume": claude,
        "budgetEnforcement": claude,
        "usageSource": if claude { "provider" } else { "estimate" },
    })
}

pub fn detect_all(with_version: bool) -> Vec<DetectedCli> {
    let mut out = Vec::new();
    for spec in SPECS {
        let found = spec.commands.iter().find_map(|c| resolve(c));
        match found {
            Some(path) => out.push(DetectedCli {
                name: spec.name.to_string(),
                command: path.to_string_lossy().to_string(),
                adapter: spec.adapter.to_string(),
                version: if with_version {
                    version_of(&path)
                } else {
                    None
                },
                installed: true,
                recommended: spec.recommended,
                installable: spec.install.is_some(),
                install_hint: spec.install.map(|m| m.hint()),
                capabilities: capabilities_for(spec.adapter),
            }),
            None => out.push(DetectedCli {
                name: spec.name.to_string(),
                command: String::new(),
                adapter: spec.adapter.to_string(),
                version: None,
                installed: false,
                recommended: spec.recommended,
                installable: spec.install.is_some(),
                install_hint: spec.install.map(|m| m.hint()),
                capabilities: capabilities_for(spec.adapter),
            }),
        }
    }
    out
}

const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

fn read_log_tail(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let tail: String = text
        .chars()
        .rev()
        .take(600)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    tail.trim().to_string()
}

/// 通过官方 npm / PyPI 包安装对应 CLI，安装输出写入临时日志文件，避免管道阻塞。
pub fn install(name: &str) -> std::result::Result<String, String> {
    let spec = SPECS
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("未知的 CLI：{name}"))?;
    let method = spec
        .install
        .ok_or_else(|| format!("{} 官方暂未提供自动安装方式", spec.name))?;
    let (program, args) = match method {
        InstallMethod::Npm(pkg) => {
            let npm = resolve("npm")
                .ok_or_else(|| "未找到 npm，请先安装 Node.js 18 或更高版本".to_string())?;
            let (program, mut args) = command_for(&npm);
            args.extend(["install".to_string(), "-g".to_string(), pkg.to_string()]);
            (program, args)
        }
        InstallMethod::Pip(pkg) => {
            let python = resolve("python")
                .or_else(|| resolve("py"))
                .ok_or_else(|| "未找到 Python，请先安装 Python 3.10 或更高版本".to_string())?;
            let (program, mut args) = command_for(&python);
            args.extend([
                "-m".to_string(),
                "pip".to_string(),
                "install".to_string(),
                "--upgrade".to_string(),
                "--disable-pip-version-check".to_string(),
                pkg.to_string(),
            ]);
            (program, args)
        }
    };

    let log_path = std::env::temp_dir().join(format!(
        "lumen-cli-install-{}-{}.log",
        now_ms(),
        std::process::id()
    ));
    let log_file =
        std::fs::File::create(&log_path).map_err(|e| format!("无法创建安装日志：{e}"))?;
    let log_err = log_file
        .try_clone()
        .map_err(|e| format!("无法创建安装日志：{e}"))?;
    let mut child = Command::new(&program)
        .args(&args)
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err))
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|e| format!("无法启动安装进程：{e}"))?;
    let pid = child.id();
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let tail = read_log_tail(&log_path);
                let _ = std::fs::remove_file(&log_path);
                if status.success() {
                    return Ok(if tail.is_empty() {
                        format!("{} 安装完成", spec.name)
                    } else {
                        tail
                    });
                }
                return Err(if tail.is_empty() {
                    format!("{} 安装失败（退出码 {:?}）", spec.name, status.code())
                } else {
                    tail
                });
            }
            Ok(None) if start.elapsed() > INSTALL_TIMEOUT => {
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .creation_flags(0x08000000)
                    .output();
                let _ = child.kill();
                let _ = std::fs::remove_file(&log_path);
                return Err(format!("{} 安装超时（10 分钟）", spec.name));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => {
                let _ = std::fs::remove_file(&log_path);
                return Err(format!("安装进程异常：{e}"));
            }
        }
    }
}

fn npm_url(pkg: &str) -> String {
    let encoded = pkg.replace('@', "%40").replace('/', "%2F");
    format!("https://registry.npmjs.org/{encoded}/latest")
}

fn latest_of(method: InstallMethod) -> Option<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(8))
        .build();
    let value: serde_json::Value = match method {
        InstallMethod::Npm(pkg) => agent.get(&npm_url(pkg)).call().ok()?.into_json().ok()?,
        InstallMethod::Pip(pkg) => agent
            .get(&format!("https://pypi.org/pypi/{pkg}/json"))
            .call()
            .ok()?
            .into_json()
            .ok()?,
    };
    match method {
        InstallMethod::Npm(_) => value.get("version")?.as_str().map(str::to_string),
        InstallMethod::Pip(_) => value
            .get("info")?
            .get("version")?
            .as_str()
            .map(str::to_string),
    }
}

/// 查询官方 npm registry / PyPI 上的最新版本，用于升级提示。
pub fn latest_versions(names: &[String]) -> Vec<CliVersionInfo> {
    names
        .iter()
        .filter_map(|name| {
            let spec = SPECS.iter().find(|s| s.name == name)?;
            Some(CliVersionInfo {
                name: spec.name.to_string(),
                latest_version: spec.install.and_then(latest_of),
            })
        })
        .collect()
}

pub fn seed_default_agents(conn: &Connection) -> Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))?;
    if count > 0 {
        return Ok(());
    }
    for spec in SPECS.iter().filter(|s| s.recommended) {
        if let Some(path) = spec.commands.iter().find_map(|c| resolve(c)) {
            let args_json = if spec.adapter == "opencode" {
                "[\"run\"]"
            } else {
                "[]"
            };
            // 能力如实标注：只有 Claude Code 具备结构化 usage 与工具审批桥接，
            // 其余适配器仅记录输出，UI 不应再显示“已启用审批/预算硬停”。
            let capabilities = capabilities_for(spec.adapter).to_string();
            conn.execute(
                "INSERT INTO agents (id, name, kind, adapter, command, args_json, status, capabilities_json, created_at)
                 VALUES (?1, ?2, 'cli', ?3, ?4, ?5, 'idle', ?6, ?7)",
                params![
                    new_id(),
                    spec.name,
                    spec.adapter,
                    path.to_string_lossy().to_string(),
                    args_json,
                    capabilities,
                    now_ms()
                ],
            )?;
        }
    }
    Ok(())
}

#[cfg(windows)]
use std::os::windows::process::CommandExt;
