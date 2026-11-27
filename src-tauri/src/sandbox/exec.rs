//! 隐藏诊断入口：在真实 AppContainer 中运行一次外部 CLI。
//!
//! 与 `probe` 不同，本模块使用真实的项目目录、CLI wrapper 和依赖目录，
//! 用于验证“真实 Claude Code / cmd 能否在沙箱内启动并读取项目”；
//! 不授予网络能力，也不经过模型代理或审批网关。

use crate::sandbox::mock_llm::{self, MockLlmReport};
use crate::sandbox::session::{self, SandboxSession, RUN_PROFILE};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_REPORT_OUTPUT: usize = 128 * 1024;
const SECRET_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_MODEL",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_API_BASE",
    "OPENAI_MODEL",
    "LUMEN_GATE_URL",
    "LUMEN_GATE_TOKEN",
    "LUMEN_RUN_ID",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecSpec {
    pub base_dir: String,
    pub project_path: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// 显式传给容器的环境变量；不会回显到报告。
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// 目前只识别 `claude-code`，用于写入不含密钥的首启配置。
    #[serde(default)]
    pub adapter: Option<String>,
    /// 显式启用本地假模型上游，仅用于验证 Claude prompt 与 relay 链路。
    #[serde(default)]
    pub mock_llm: bool,
    /// 假上游要求出现在 Claude 请求正文里的随机标记。
    #[serde(default)]
    pub prompt_nonce: Option<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// 结束后回收本次 profile 与 ACL marker；默认开启。
    #[serde(default = "default_true")]
    pub cleanup: bool,
}

fn default_timeout_ms() -> u64 {
    30_000
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecReport {
    pub ok: bool,
    pub profile: String,
    pub project_path: String,
    pub program: String,
    pub resolved_program: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub status_error: Option<String>,
    pub cleanup_records: Option<usize>,
    pub cleanup_error: Option<String>,
    pub prepare_ms: u64,
    pub run_ms: u64,
    pub cleanup_ms: u64,
    pub elapsed_ms: u64,
    pub mock_llm: Option<MockLlmReport>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostStatus {
    #[serde(default)]
    exit_code: Option<u32>,
    #[serde(default)]
    error: Option<String>,
}

pub fn run(spec_path: &Path) -> Result<ExecReport, String> {
    let text = std::fs::read_to_string(spec_path)
        .map_err(|e| format!("读取沙箱诊断配置 {} 失败：{e}", spec_path.display()))?;
    let spec: ExecSpec =
        serde_json::from_str(&text).map_err(|e| format!("解析沙箱诊断配置失败：{e}"))?;
    run_spec(&spec)
}

fn run_spec(spec: &ExecSpec) -> Result<ExecReport, String> {
    let started = Instant::now();
    if spec.mock_llm && spec.adapter.as_deref() != Some("claude-code") {
        return Err("mockLlm 目前只支持 adapter=claude-code".to_string());
    }
    if spec.mock_llm && spec.prompt_nonce.as_deref().unwrap_or("").is_empty() {
        return Err("启用 mockLlm 时必须提供非空 promptNonce".to_string());
    }
    let base = PathBuf::from(&spec.base_dir);
    let project = PathBuf::from(&spec.project_path);
    if !project.is_dir() {
        return Err(format!("项目目录不存在：{}", project.display()));
    }
    std::fs::create_dir_all(&base)
        .map_err(|e| format!("创建诊断数据目录 {} 失败：{e}", base.display()))?;

    let id = format!("sandbox-exec-{}", uuid::Uuid::new_v4().simple());
    let run_dir = base.join("sessions").join(&id);
    std::fs::create_dir_all(&run_dir)
        .map_err(|e| format!("创建诊断运行目录 {} 失败：{e}", run_dir.display()))?;
    let run_guard = RunDirGuard(run_dir.clone());

    let home = base.join("sandbox-home");
    std::fs::create_dir_all(&home)
        .map_err(|e| format!("创建沙箱 HOME {} 失败：{e}", home.display()))?;
    let sandbox_temp = home.join("tmp");
    std::fs::create_dir_all(&sandbox_temp)
        .map_err(|e| format!("创建沙箱 TEMP {} 失败：{e}", sandbox_temp.display()))?;
    if spec.adapter.as_deref() == Some("claude-code") {
        session::bootstrap_claude_home(&home)?;
    }

    let plan = session::plan_launch(&spec.program, &spec.args)?;
    let exe = std::env::current_exe().map_err(|e| format!("无法定位主程序：{e}"))?;
    let mock = if spec.mock_llm {
        Some(mock_llm::start(
            spec.prompt_nonce.clone().unwrap_or_default(),
        )?)
    } else {
        None
    };
    let upstreams: Vec<(&str, u16)> = mock
        .as_ref()
        .map(|mock| vec![("proxy", mock.port)])
        .unwrap_or_default();
    let prepare_started = Instant::now();
    let session = SandboxSession::prepare(
        &id,
        &run_dir,
        &project,
        &home,
        &exe,
        &plan.extra_paths,
        &upstreams,
    )?;
    let prepare_ms = prepare_started.elapsed().as_millis() as u64;

    let mut env = spec.env.clone();
    if let Some(mock) = &mock {
        let port = session
            .container_port("proxy")
            .ok_or_else(|| "沙箱未建立假模型上游 relay 端口".to_string())?;
        env.insert(
            "ANTHROPIC_BASE_URL".into(),
            format!("http://127.0.0.1:{port}/proxy/{}", mock.token),
        );
        env.insert("ANTHROPIC_AUTH_TOKEN".into(), mock.token.clone());
        env.insert(
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
            "1".into(),
        );
    }
    env.insert("USERPROFILE".into(), home.to_string_lossy().to_string());
    env.insert("HOME".into(), home.to_string_lossy().to_string());
    env.insert("TEMP".into(), sandbox_temp.to_string_lossy().to_string());
    env.insert("TMP".into(), sandbox_temp.to_string_lossy().to_string());

    let ready_file = run_dir.join("relay-ready.json");
    let status_file = run_dir.join("host-status.json");
    let host_spec = session.host_spec(
        &plan.program,
        &plan.args,
        &project,
        env,
        &ready_file,
        &status_file,
    );
    let host_spec_path = run_dir.join("host.json");
    std::fs::write(
        &host_spec_path,
        serde_json::to_vec(&host_spec).map_err(|e| format!("序列化沙箱宿主配置失败：{e}"))?,
    )
    .map_err(|e| format!("写入沙箱宿主配置失败：{e}"))?;

    let stdout_path = run_dir.join("stdout.log");
    let stderr_path = run_dir.join("stderr.log");
    let stdout = File::create(&stdout_path).map_err(|e| format!("创建标准输出日志失败：{e}"))?;
    let stderr = File::create(&stderr_path).map_err(|e| format!("创建标准错误日志失败：{e}"))?;
    let mut command = Command::new(&exe);
    command
        .arg("--sandbox-host")
        .arg(&host_spec_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    for name in SECRET_ENV {
        command.env_remove(name);
    }
    let run_started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|e| format!("启动沙箱宿主失败：{e}"))?;
    // 超时时由 Job Object 终止宿主及其容器子进程。
    let job = crate::sandbox::create_job();
    if let Some(job) = &job {
        let pid = child.id();
        if pid != 0 {
            let _ = job.assign(pid);
        }
    }

    let deadline = Instant::now() + Duration::from_millis(spec.timeout_ms.max(1_000));
    let mut timed_out = false;
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {}
            Err(e) => return Err(format!("等待沙箱宿主失败：{e}")),
        }
        if Instant::now() >= deadline {
            timed_out = true;
            if let Some(job) = &job {
                job.terminate();
            } else {
                let _ = child.kill();
            }
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let run_ms = run_started.elapsed().as_millis() as u64;

    let stdout = read_capped(&stdout_path);
    let stderr = read_capped(&stderr_path);
    let status: Option<HostStatus> = std::fs::read(&status_file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let status_error = status.as_ref().and_then(|s| s.error.clone());
    // 宿主退出码已经转发 CLI 退出码；状态文件用于补充宿主内部错误。
    let exit_code = exit_code.or_else(|| {
        status
            .as_ref()
            .and_then(|s| s.exit_code)
            .map(|code| code as i32)
    });

    drop(session);
    let mock_report = mock.as_ref().map(|mock| mock.report());
    let cleanup_started = Instant::now();
    let (cleanup_records, cleanup_error) = if spec.cleanup {
        match session::cleanup(&base, RUN_PROFILE) {
            Ok(count) => (Some(count), None),
            Err(err) => (None, Some(err)),
        }
    } else {
        (None, None)
    };
    let cleanup_ms = cleanup_started.elapsed().as_millis() as u64;
    drop(run_guard);

    let mock_ok = mock_report.as_ref().is_none_or(|report| {
        report.requests > 0
            && report.accepted > 0
            && report.prompt_nonce_seen
            && stdout.contains(&report.answer)
    });
    let ok = !timed_out
        && status_error.is_none()
        && cleanup_error.is_none()
        && exit_code == Some(0)
        && mock_ok;
    Ok(ExecReport {
        ok,
        profile: RUN_PROFILE.to_string(),
        project_path: spec.project_path.clone(),
        program: spec.program.clone(),
        resolved_program: plan.program.to_string_lossy().to_string(),
        exit_code,
        timed_out,
        stdout,
        stderr,
        status_error,
        cleanup_records,
        cleanup_error,
        prepare_ms,
        run_ms,
        cleanup_ms,
        elapsed_ms: started.elapsed().as_millis() as u64,
        mock_llm: mock_report,
    })
}

fn read_capped(path: &Path) -> String {
    let Ok(file) = File::open(path) else {
        return String::new();
    };
    let mut text = String::new();
    let _ = file
        .take(MAX_REPORT_OUTPUT as u64)
        .read_to_string(&mut text);
    text
}

struct RunDirGuard(PathBuf);

impl Drop for RunDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
