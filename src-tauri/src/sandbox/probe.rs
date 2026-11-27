//! AppContainer 沙箱自检。
//!
//! 在宿主侧搭建 echo 服务与命名管道桥，再以 `--sandbox-host` 启动自身：
//! 宿主模式会在容器内拉起 relay 与 `--sandbox-selftest`，
//! 由容器内进程验证文件隔离、环境重定向与 relay 回路。
//!
//! 自检产生的 ACL、profile、临时目录都会在结束时清理。

use crate::sandbox::acl;
use crate::sandbox::appcontainer::{delete_profile, ensure_profile, AppContainerSid};
use crate::sandbox::host::HostSpec;
use crate::sandbox::relay::{self, RelayChannel};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PROBE_PROFILE: &str = "com.agenthub.desktop.sandbox.probe";

/// 自检结果（返回给前端）。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    pub ok: bool,
    pub profile: String,
    pub work_read: bool,
    pub work_write: bool,
    /// 授权前已存在的文件是否通过 icacls 传播获得访问权限。
    pub existing_read: bool,
    pub outside_read_blocked: bool,
    pub user_read_blocked: bool,
    pub outside_write_blocked: bool,
    pub home_redirected: bool,
    pub relay_echo: bool,
    /// 容器 CLI 的标准输出是否经宿主转发回调用方。
    pub stdio_forwarded: bool,
    pub host_exit_code: i32,
    pub detail: String,
    pub elapsed_ms: u64,
}

impl ProbeReport {
    fn all_passed(&self) -> bool {
        self.work_read
            && self.work_write
            && self.existing_read
            && self.outside_read_blocked
            && self.user_read_blocked
            && self.outside_write_blocked
            && self.home_redirected
            && self.relay_echo
            && self.stdio_forwarded
    }
}

/// 容器内 selftest 的输入配置（放在工作目录里，容器可读）。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfTestSpec {
    pub work_dir: String,
    pub input_file: String,
    pub existing_file: String,
    pub expected_nonce: String,
    pub outside_canary: String,
    pub outside_write_target: String,
    pub user_canary: Option<String>,
    pub relay_port: u16,
    pub env_home: String,
    pub result_file: String,
}

/// 容器内 selftest 的输出。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfTestResult {
    pub work_read: bool,
    pub work_write: bool,
    pub existing_read: bool,
    pub outside_read_blocked: bool,
    pub user_read_blocked: bool,
    pub outside_write_blocked: bool,
    pub home_redirected: bool,
    pub relay_echo: bool,
    /// 容器内实际看到的环境变量，便于诊断重定向失败。
    #[serde(default)]
    pub env_diag: String,
    #[serde(default)]
    pub error: String,
}

impl SelfTestResult {
    fn all_passed(&self) -> bool {
        self.work_read
            && self.work_write
            && self.existing_read
            && self.outside_read_blocked
            && self.user_read_blocked
            && self.outside_write_blocked
            && self.home_redirected
            && self.relay_echo
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostStatusFile {
    #[serde(default)]
    exit_code: Option<u32>,
    #[serde(default)]
    error: Option<String>,
}

/// 清理守卫：即使中途失败也会回收 ACL / profile / 临时目录。
struct Cleanup {
    sid: AppContainerSid,
    profile: Option<String>,
    granted: Vec<PathBuf>,
    /// 用 `grant_tree` 授权过的目录，回收时需清理整棵树的继承 ACE。
    trees: Vec<PathBuf>,
    root: PathBuf,
    user_canary: Option<PathBuf>,
    pipes: Vec<String>,
    /// 自检失败时保留目录，便于排查。
    keep: bool,
}

impl Cleanup {
    fn grant(&mut self, path: &Path, rights: u32, inherit: bool) -> Result<(), String> {
        acl::grant(path, self.sid.as_ptr(), rights, inherit)?;
        self.granted.push(path.to_path_buf());
        Ok(())
    }

    fn grant_tree(&mut self, path: &Path) -> Result<(), String> {
        acl::grant_tree(path, &self.sid, acl::DIR_FULL, "F")?;
        self.trees.push(path.to_path_buf());
        Ok(())
    }

    fn grant_ancestors(&mut self, path: &Path) {
        self.granted
            .extend(acl::grant_ancestors(path, self.sid.as_ptr()));
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for path in &self.granted {
            let _ = acl::revoke(path, self.sid.as_ptr());
        }
        for path in &self.trees {
            acl::revoke_tree(path, &self.sid);
        }
        if let Some(name) = &self.profile {
            let _ = delete_profile(name);
        }
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.root);
        }
        if let Some(path) = &self.user_canary {
            let _ = std::fs::remove_file(path);
        }
        for pipe in &self.pipes {
            relay::nudge(pipe);
        }
    }
}

struct EchoServer {
    port: u16,
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn start_echo(log_path: PathBuf) -> Result<EchoServer, String> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("启动回环 echo 服务失败：{e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取 echo 端口失败：{e}"))?
        .port();
    thread::spawn(move || {
        let _ = std::fs::write(&log_path, format!("{} echo: listening\n", now_ms()));
        let log = |message: &str| {
            if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(&log_path) {
                let _ = file.write_all(format!("{} {message}\n", now_ms()).as_bytes());
            }
        };
        if let Ok((mut stream, _)) = listener.accept() {
            log("echo: accepted");
            let mut buffer = [0u8; 8192];
            let mut total = 0usize;
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        if stream.write_all(&buffer[..n]).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            log(&format!("echo: closed after {total} bytes"));
        }
    });
    Ok(EchoServer { port })
}

/// 宿主侧自检入口（会阻塞到自检结束）。
pub fn run(base_dir: &Path) -> Result<ProbeReport, String> {
    let started = Instant::now();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let root = base_dir.join("sandbox-probe").join(&id);
    let work = root.join("work");
    let home = root.join("home");
    std::fs::create_dir_all(&work).map_err(|e| format!("创建自检目录失败：{e}"))?;
    std::fs::create_dir_all(&home).map_err(|e| format!("创建沙箱 HOME 失败：{e}"))?;

    let nonce = format!("lumen-probe-{}", uuid::Uuid::new_v4().simple());
    let input_path = work.join("input.txt");
    let existing_path = work.join("existing.txt");
    let outside_canary = root.join("outside-canary.txt");
    // 这个文件在授权前创建，用来验证 icacls 传播能否覆盖已有文件。
    std::fs::write(&existing_path, &nonce).map_err(|e| format!("写入既有样本失败：{e}"))?;

    // 自检结束后无论 Profile 是否本次新建都删除，避免留下长期残留。
    let (sid, _) = ensure_profile(PROBE_PROFILE)?;
    let up_pipe = format!(r"\\.\pipe\lumen-sandbox-probe-{id}-up");
    let down_pipe = format!(r"\\.\pipe\lumen-sandbox-probe-{id}-down");
    let mut cleanup = Cleanup {
        sid,
        profile: Some(PROBE_PROFILE.to_string()),
        granted: Vec::new(),
        trees: Vec::new(),
        root: root.clone(),
        user_canary: None,
        pipes: vec![up_pipe.clone(), down_pipe.clone()],
        keep: false,
    };

    // 工作目录需要整棵树授权（含授权前已存在的文件）；沙箱 HOME 只需根目录继承。
    cleanup.grant_tree(&work)?;
    cleanup.grant(&home, acl::DIR_FULL, true)?;

    let exe = std::env::current_exe().map_err(|e| format!("获取当前程序路径失败：{e}"))?;
    cleanup.grant(&exe, acl::EXE_READ_EXECUTE, false)?;
    for path in [&work, &home] {
        cleanup.grant_ancestors(path);
    }
    cleanup.grant_ancestors(&exe);

    // 样本必须在授予可继承 ACE 之后创建，否则文件不会继承容器访问权限。
    std::fs::write(&input_path, &nonce).map_err(|e| format!("写入工作目录样本失败：{e}"))?;
    std::fs::write(&outside_canary, &nonce).map_err(|e| format!("写入隔离样本失败：{e}"))?;

    // 用户目录 canary：验证容器无法读取真实 HOME 下的文件。
    let user_canary =
        dirs::home_dir().map(|dir| dir.join(format!(".lumen-sandbox-probe-{id}.txt")));
    if let Some(path) = &user_canary {
        if std::fs::write(path, &nonce).is_ok() {
            cleanup.user_canary = Some(path.clone());
        }
    }

    let echo = start_echo(work.join("echo.log"))?;
    let probe_listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("分配 relay 端口失败：{e}"))?;
    let relay_port = probe_listener
        .local_addr()
        .map_err(|e| format!("读取 relay 端口失败：{e}"))?
        .port();
    drop(probe_listener);

    let sddl = relay::pipe_sddl(&cleanup.sid)?;
    let bridge_up = up_pipe.clone();
    let bridge_down = down_pipe.clone();
    let bridge_sddl = sddl.clone();
    let bridge_log = work.join("bridge.log");
    let echo_port = echo.port;
    let _ = std::fs::write(&bridge_log, format!("{} bridge: starting\n", now_ms()));
    let bridge_log_path = bridge_log.clone();
    thread::spawn(move || {
        let message =
            match relay::serve_bridge_once(&bridge_up, &bridge_down, echo_port, &bridge_sddl) {
                Ok(()) => "bridge: ok\n".to_string(),
                Err(err) => format!("bridge error: {err}\n"),
            };
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .append(true)
            .open(&bridge_log_path)
        {
            let _ = file.write_all(format!("{} {message}", now_ms()).as_bytes());
        }
    });

    let relay_ready = work.join("relay-ready.json");
    let result_file = work.join("selftest-result.json");
    let status_file = work.join("host-status.json");
    let selftest_spec = SelfTestSpec {
        work_dir: work.to_string_lossy().to_string(),
        input_file: input_path.to_string_lossy().to_string(),
        existing_file: existing_path.to_string_lossy().to_string(),
        expected_nonce: nonce.clone(),
        outside_canary: outside_canary.to_string_lossy().to_string(),
        outside_write_target: root.join("outside-write.txt").to_string_lossy().to_string(),
        user_canary: cleanup
            .user_canary
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        relay_port,
        env_home: home.to_string_lossy().to_string(),
        result_file: result_file.to_string_lossy().to_string(),
    };
    let selftest_path = work.join("selftest.json");
    std::fs::write(
        &selftest_path,
        serde_json::to_vec(&selftest_spec).map_err(|e| format!("序列化自检配置失败：{e}"))?,
    )
    .map_err(|e| format!("写入自检配置失败：{e}"))?;

    let mut env = std::collections::HashMap::new();
    env.insert("USERPROFILE".to_string(), selftest_spec.env_home.clone());
    env.insert("HOME".to_string(), selftest_spec.env_home.clone());
    let host_spec = HostSpec {
        profile: PROBE_PROFILE.to_string(),
        capabilities: Vec::new(),
        channels: vec![RelayChannel {
            up_pipe: up_pipe.clone(),
            down_pipe: down_pipe.clone(),
            port: relay_port,
        }],
        ready_file: relay_ready.to_string_lossy().to_string(),
        program: exe.to_string_lossy().to_string(),
        program_args: vec![
            "--sandbox-selftest".to_string(),
            selftest_path.to_string_lossy().to_string(),
        ],
        cwd: work.to_string_lossy().to_string(),
        env,
        ready_timeout_ms: 15_000,
        status_file: Some(status_file.to_string_lossy().to_string()),
    };
    let host_spec_path = work.join("host.json");
    std::fs::write(
        &host_spec_path,
        serde_json::to_vec(&host_spec).map_err(|e| format!("序列化宿主配置失败：{e}"))?,
    )
    .map_err(|e| format!("写入宿主配置失败：{e}"))?;

    let host_log = work.join("host.log");
    let host_stderr =
        std::fs::File::create(&host_log).map_err(|e| format!("创建宿主日志失败：{e}"))?;
    let host_stdout = host_stderr
        .try_clone()
        .map_err(|e| format!("复制宿主日志句柄失败：{e}"))?;
    let mut child = Command::new(&exe)
        .arg("--sandbox-host")
        .arg(&host_spec_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(host_stdout))
        .stderr(Stdio::from(host_stderr))
        .spawn()
        .map_err(|e| format!("启动沙箱宿主失败：{e}"))?;

    let deadline = Instant::now() + Duration::from_secs(60);
    let host_exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) => {}
            Err(e) => return Err(format!("等待沙箱宿主失败：{e}")),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err("沙箱自检超时（60 秒）".to_string());
        }
        thread::sleep(Duration::from_millis(50));
    };

    let result: Option<SelfTestResult> = std::fs::read(&result_file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let status: Option<HostStatusFile> = std::fs::read(&status_file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let result = result.unwrap_or_default();
    // 容器内进程把标记写到 stdout，宿主必须原样转发到日志文件。
    let stdio_forwarded = std::fs::read_to_string(&host_log)
        .map(|text| text.contains(&format!("LUMEN-STDIO-{nonce}")))
        .unwrap_or(false);

    let mut detail = format!(
        "工作目录读写={}/{}, 既有文件读={}, 外部读拦截={}, 用户目录读拦截={}, 外部写拦截={}, HOME/TEMP 重定向={}, relay 回路={}, 标准输出转发={}",
        yes_no(result.work_read),
        yes_no(result.work_write),
        yes_no(result.existing_read),
        yes_no(result.outside_read_blocked),
        yes_no(result.user_read_blocked),
        yes_no(result.outside_write_blocked),
        yes_no(result.home_redirected),
        yes_no(result.relay_echo),
        yes_no(stdio_forwarded)
    );
    if !result.error.is_empty() {
        detail.push_str(&format!("；容器内错误：{}", result.error));
    }
    if !result.env_diag.is_empty() && !result.home_redirected {
        detail.push_str(&format!("；容器内环境：{}", result.env_diag));
    }
    if let Some(status) = &status {
        if let Some(error) = &status.error {
            detail.push_str(&format!("；宿主错误：{error}"));
        }
        if let Some(code) = status.exit_code {
            detail.push_str(&format!("；宿主退出码：{code}"));
        }
    }

    let report = ProbeReport {
        ok: false,
        profile: PROBE_PROFILE.to_string(),
        work_read: result.work_read,
        work_write: result.work_write,
        existing_read: result.existing_read,
        outside_read_blocked: result.outside_read_blocked,
        user_read_blocked: result.user_read_blocked,
        outside_write_blocked: result.outside_write_blocked,
        home_redirected: result.home_redirected,
        relay_echo: result.relay_echo,
        stdio_forwarded,
        host_exit_code: host_exit,
        detail,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    let report = ProbeReport {
        ok: report.all_passed() && host_exit == 0,
        ..report
    };
    cleanup.keep = !report.ok;
    Ok(report)
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "通过"
    } else {
        "失败"
    }
}

/// 容器内 selftest 入口；返回进程退出码。
pub fn run_selftest(spec_path: &Path) -> i32 {
    match selftest(spec_path) {
        Ok(true) => 0,
        Ok(false) => 3,
        Err(err) => {
            eprintln!("sandbox selftest: {err}");
            3
        }
    }
}

fn selftest(spec_path: &Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(spec_path)
        .map_err(|e| format!("读取自检配置 {} 失败：{e}", spec_path.display()))?;
    let spec: SelfTestSpec =
        serde_json::from_str(&text).map_err(|e| format!("解析自检配置失败：{e}"))?;

    let mut result = SelfTestResult::default();
    result.work_read = std::fs::read_to_string(&spec.input_file)
        .map(|text| text.trim() == spec.expected_nonce)
        .unwrap_or(false);
    result.work_write =
        std::fs::write(Path::new(&spec.work_dir).join("written.txt"), b"ok").is_ok();
    result.existing_read = std::fs::read_to_string(&spec.existing_file)
        .map(|text| text.trim() == spec.expected_nonce)
        .unwrap_or(false);
    result.outside_read_blocked = std::fs::read(&spec.outside_canary).is_err();
    result.outside_write_blocked = std::fs::write(&spec.outside_write_target, b"x").is_err();
    result.user_read_blocked = spec
        .user_canary
        .as_ref()
        .map(|path| std::fs::read(path).is_err())
        .unwrap_or(true);
    // USERPROFILE/HOME 由我们注入；TEMP/TMP 会被系统强制指向容器的 AC\Temp。
    result.home_redirected =
        env_eq("USERPROFILE", &spec.env_home) && env_eq("HOME", &spec.env_home) && temp_ok();
    result.env_diag = format!(
        "USERPROFILE={};TEMP={};HOME={};TMP={}",
        env_value("USERPROFILE"),
        env_value("TEMP"),
        env_value("HOME"),
        env_value("TMP")
    );
    result.relay_echo = match relay_echo(spec.relay_port, &spec.expected_nonce) {
        Ok(value) => value,
        Err(err) => {
            result.error = err;
            false
        }
    };

    if let Ok(bytes) = serde_json::to_vec(&result) {
        let _ = std::fs::write(&spec.result_file, bytes);
    }
    // 供宿主侧验证标准输出转发。
    println!("LUMEN-STDIO-{}", spec.expected_nonce);
    let _ = std::io::stdout().flush();
    Ok(result.all_passed())
}

fn env_eq(name: &str, expected: &str) -> bool {
    std::env::var(name)
        .map(|value| value.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn env_value(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| "<未设置>".to_string())
}

/// 容器的 TEMP 必须落在 AppContainer 的 `AC\Temp` 下，并且可写。
fn temp_ok() -> bool {
    let Ok(temp) = std::env::var("TEMP") else {
        return false;
    };
    let lowered = temp.to_ascii_lowercase();
    if !lowered.contains("\\appdata\\local\\packages\\") || !lowered.ends_with("\\ac\\temp") {
        return false;
    }
    let probe = Path::new(&temp).join("lumen-selftest-temp.txt");
    std::fs::write(&probe, b"ok").is_ok() && std::fs::remove_file(&probe).is_ok()
}

fn relay_echo(port: u16, nonce: &str) -> Result<bool, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .map_err(|e| format!("连接 relay 127.0.0.1:{port} 失败：{e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| format!("设置读取超时失败：{e}"))?;
    let payload = format!("PING:{nonce}").into_bytes();
    stream
        .write_all(&payload)
        .map_err(|e| format!("写入 relay 失败：{e}"))?;
    let mut buffer = vec![0u8; payload.len()];
    stream
        .read_exact(&mut buffer)
        .map_err(|e| format!("读取 relay 回包失败：{e}"))?;
    Ok(buffer == payload)
}
