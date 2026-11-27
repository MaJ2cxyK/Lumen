//! 以 AppContainer 身份启动子进程（宿主模式）。
//!
//! `Lumen.exe --sandbox-host spec.json` 由主进程以普通用户身份启动；
//! 本模块负责：
//! 1. 确保 AppContainer profile 存在；
//! 2. 先在容器内拉起 relay，等待其写入就绪文件；
//! 3. 再在容器内拉起真正的 CLI，透传退出码。

use crate::sandbox::appcontainer::{capability_sid, ensure_profile, to_wide, CapabilitySid};
use crate::sandbox::relay::{RelayChannel, RelaySpec};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_void, OsString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::ptr;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::{
    PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::SystemServices::SE_GROUP_ENABLED;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

/// 宿主模式的启动配置。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSpec {
    /// AppContainer profile 名，例如 `com.agenthub.desktop.sandbox`。
    pub profile: String,
    /// 网络能力名，默认空（不授予任何联网能力）。
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 容器内 relay 通道。
    #[serde(default)]
    pub channels: Vec<RelayChannel>,
    /// relay 写出的就绪文件。
    pub ready_file: String,
    /// 真正要运行的 CLI 路径（绝对路径）。
    pub program: String,
    #[serde(default)]
    pub program_args: Vec<String>,
    /// 工作目录（必须是容器可访问的路径）。
    pub cwd: String,
    /// 环境变量覆盖（HOME/TEMP 等）。
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// 等待 relay 就绪的超时时间。
    #[serde(default = "default_ready_timeout")]
    pub ready_timeout_ms: u64,
    /// 失败时写入的状态文件，便于宿主侧自检读取原因。
    #[serde(default)]
    pub status_file: Option<String>,
}

fn default_ready_timeout() -> u64 {
    15_000
}

struct ChildProcess {
    process: HANDLE,
    /// 输出泵线程；子进程退出后 join，确保最后一段输出不被截断。
    pumps: Vec<JoinHandle<()>>,
}

impl ChildProcess {
    fn terminate(&self) {
        unsafe {
            TerminateProcess(self.process, 1);
            WaitForSingleObject(self.process, 2_000);
        }
    }

    fn drain(&mut self) {
        for pump in self.pumps.drain(..) {
            let _ = pump.join();
        }
    }
}

/// 容器 CLI 与宿主之间的标准输入输出管道。
///
/// 子进程侧句柄可继承，宿主侧句柄显式关闭继承，避免把管道泄漏给 relay。
struct Stdio {
    child_in: HANDLE,
    child_out: HANDLE,
    child_err: HANDLE,
    host_in: Option<File>,
    host_out: Option<File>,
    host_err: Option<File>,
}

impl Stdio {
    fn create() -> Result<Self, String> {
        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: 1,
        };
        let mut child_in: HANDLE = ptr::null_mut();
        let mut host_in: HANDLE = ptr::null_mut();
        let mut host_out: HANDLE = ptr::null_mut();
        let mut child_out: HANDLE = ptr::null_mut();
        let mut host_err: HANDLE = ptr::null_mut();
        let mut child_err: HANDLE = ptr::null_mut();
        unsafe {
            if CreatePipe(&mut child_in, &mut host_in, &attrs, 0) == 0
                || CreatePipe(&mut host_out, &mut child_out, &attrs, 0) == 0
                || CreatePipe(&mut host_err, &mut child_err, &attrs, 0) == 0
            {
                let code = GetLastError();
                for handle in [child_in, host_in, host_out, child_out, host_err, child_err] {
                    if !handle.is_null() {
                        CloseHandle(handle);
                    }
                }
                return Err(format!("创建标准输入输出管道失败：错误码 {code}"));
            }
            for handle in [host_in, host_out, host_err] {
                if SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) == 0 {
                    let code = GetLastError();
                    for handle in [child_in, host_in, host_out, child_out, host_err, child_err] {
                        if !handle.is_null() {
                            CloseHandle(handle);
                        }
                    }
                    return Err(format!("关闭宿主侧管道继承失败：错误码 {code}"));
                }
            }
            Ok(Self {
                child_in,
                child_out,
                child_err,
                host_in: Some(File::from_raw_handle(host_in)),
                host_out: Some(File::from_raw_handle(host_out)),
                host_err: Some(File::from_raw_handle(host_err)),
            })
        }
    }

    /// 子进程创建完成后关闭宿主持有的子进程侧句柄。
    fn close_child_ends(&mut self) {
        for handle in [&mut self.child_in, &mut self.child_out, &mut self.child_err] {
            if !handle.is_null() {
                unsafe {
                    CloseHandle(*handle);
                }
                *handle = ptr::null_mut();
            }
        }
    }

    /// 启动三条泵线程：宿主 stdin → 容器 stdin，容器 stdout/stderr → 宿主。
    ///
    /// stdin 泵不 join：它可能长期阻塞在宿主 stdin 上，进程退出时由系统回收。
    fn start_pumps(mut self) -> Vec<JoinHandle<()>> {
        if let Some(mut stdin) = self.host_in.take() {
            std::thread::spawn(move || {
                let source = std::io::stdin();
                let mut source = source.lock();
                copy_stream(&mut source, &mut stdin);
                // File 在这里释放，容器 CLI 读到 EOF。
            });
        }
        let stdout_pump = self.host_out.take().map(|mut out| {
            std::thread::spawn(move || {
                let target = std::io::stdout();
                let mut target = target.lock();
                copy_stream(&mut out, &mut target);
                let _ = target.flush();
            })
        });
        let stderr_pump = self.host_err.take().map(|mut err| {
            std::thread::spawn(move || {
                let target = std::io::stderr();
                let mut target = target.lock();
                copy_stream(&mut err, &mut target);
                let _ = target.flush();
            })
        });
        stdout_pump.into_iter().chain(stderr_pump).collect()
    }
}

impl Drop for Stdio {
    fn drop(&mut self) {
        self.close_child_ends();
    }
}

/// 循环拷贝，直到任一端关闭。
fn copy_stream<R: Read, W: Write>(source: &mut R, target: &mut W) {
    let mut buffer = [0u8; 8192];
    loop {
        match source.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                if target.write_all(&buffer[..n]).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        if !self.process.is_null() {
            unsafe {
                CloseHandle(self.process);
            }
        }
    }
}

/// 宿主入口：返回 CLI 的退出码（失败时返回 3 并写状态文件）。
pub fn run(spec_path: &Path) -> i32 {
    let spec = match load_spec(spec_path) {
        Ok(spec) => spec,
        Err(err) => {
            eprintln!("sandbox host: {err}");
            return 3;
        }
    };
    match run_with_spec(spec_path, &spec) {
        Ok(code) => code as i32,
        Err(err) => {
            write_status(&spec, false, None, Some(&err));
            eprintln!("sandbox host: {err}");
            3
        }
    }
}

fn load_spec(path: &Path) -> Result<HostSpec, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("读取宿主配置 {} 失败：{e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("解析宿主配置失败：{e}"))
}

fn write_status(spec: &HostSpec, ok: bool, exit_code: Option<u32>, error: Option<&str>) {
    let Some(path) = &spec.status_file else {
        return;
    };
    let payload = serde_json::json!({
        "ok": ok,
        "exitCode": exit_code,
        "error": error,
    });
    let _ = std::fs::write(path, payload.to_string());
}

fn run_with_spec(spec_path: &Path, spec: &HostSpec) -> Result<u32, String> {
    let (sid, _) = ensure_profile(&spec.profile)?;
    let capabilities: Vec<CapabilitySid> = spec
        .capabilities
        .iter()
        .map(|name| capability_sid(name))
        .collect::<Result<_, _>>()?;
    let mut sid_attrs: Vec<SID_AND_ATTRIBUTES> = capabilities
        .iter()
        .map(|cap| SID_AND_ATTRIBUTES {
            Sid: cap.as_ptr(),
            Attributes: SE_GROUP_ENABLED as u32,
        })
        .collect();

    // relay 配置由宿主生成，避免调用方重复拼路径。
    let relay_spec_path = spec_path.with_extension("relay.json");
    let relay_spec = RelaySpec {
        channels: spec.channels.clone(),
        ready_file: spec.ready_file.clone(),
    };
    std::fs::write(
        &relay_spec_path,
        serde_json::to_vec(&relay_spec).map_err(|e| format!("序列化 relay 配置失败：{e}"))?,
    )
    .map_err(|e| format!("写入 relay 配置失败：{e}"))?;

    let exe = std::env::current_exe().map_err(|e| format!("获取当前程序路径失败：{e}"))?;
    let relay_args = vec![
        "--sandbox-relay".to_string(),
        relay_spec_path.to_string_lossy().to_string(),
    ];
    let relay = launch(
        &exe,
        &relay_args,
        &spec.cwd,
        &spec.env,
        sid.as_ptr(),
        &mut sid_attrs,
        None,
    )?;

    let ready = PathBuf::from(&spec.ready_file);
    let deadline = Instant::now() + Duration::from_millis(spec.ready_timeout_ms.max(1_000));
    while !ready.exists() {
        if unsafe { WaitForSingleObject(relay.process, 0) } == WAIT_OBJECT_0 {
            let mut code = 0u32;
            unsafe {
                GetExitCodeProcess(relay.process, &mut code);
            }
            return Err(format!("relay 提前退出（退出码 {code}）"));
        }
        if Instant::now() >= deadline {
            relay.terminate();
            return Err("等待 relay 就绪超时".to_string());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut cli = launch(
        Path::new(&spec.program),
        &spec.program_args,
        &spec.cwd,
        &spec.env,
        sid.as_ptr(),
        &mut sid_attrs,
        Some(Stdio::create()?),
    )?;
    unsafe {
        WaitForSingleObject(cli.process, INFINITE);
    }
    let mut exit_code = 0u32;
    let ok = unsafe { GetExitCodeProcess(cli.process, &mut exit_code) };
    if ok == 0 {
        relay.terminate();
        return Err(format!("读取子进程退出码失败：{}", unsafe {
            GetLastError()
        }));
    }
    // 先排空输出，确保最后一段流式内容完整送达宿主。
    cli.drain();
    relay.terminate();
    write_status(spec, exit_code == 0, Some(exit_code), None);
    Ok(exit_code)
}

/// 以指定 AppContainer SID 启动进程。
fn launch(
    program: &Path,
    args: &[String],
    cwd: &str,
    env: &HashMap<String, String>,
    sid: PSID,
    caps: &mut [SID_AND_ATTRIBUTES],
    stdio: Option<Stdio>,
) -> Result<ChildProcess, String> {
    let program_text = program.to_string_lossy().to_string();
    if program_text.is_empty() {
        return Err("要启动的程序路径为空".to_string());
    }
    let program_w = to_wide(&program_text);
    let mut command_line = quote_arg(&program_text);
    for arg in args {
        command_line.push(' ');
        command_line.push_str(&quote_arg(arg));
    }
    let mut command_w = to_wide(&command_line);
    let cwd_w = to_wide(cwd);
    let env_block = build_env_block(env);

    unsafe {
        let mut attr_size = 0usize;
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut attr_size);
        if attr_size == 0 {
            return Err(format!("初始化进程属性列表失败：错误码 {}", GetLastError()));
        }
        let words = attr_size.div_ceil(std::mem::size_of::<usize>());
        let mut attr_buffer = vec![0usize; words];
        let attr_list = attr_buffer.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        if InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_size) == 0 {
            return Err(format!("初始化进程属性列表失败：错误码 {}", GetLastError()));
        }

        let security_caps = SECURITY_CAPABILITIES {
            AppContainerSid: sid,
            Capabilities: if caps.is_empty() {
                ptr::null_mut()
            } else {
                caps.as_mut_ptr()
            },
            CapabilityCount: caps.len() as u32,
            Reserved: 0,
        };
        let updated = UpdateProcThreadAttribute(
            attr_list,
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            &security_caps as *const _ as *const c_void,
            std::mem::size_of::<SECURITY_CAPABILITIES>(),
            ptr::null_mut(),
            ptr::null(),
        );
        if updated == 0 {
            let code = GetLastError();
            DeleteProcThreadAttributeList(attr_list);
            return Err(format!("设置 AppContainer 属性失败：错误码 {code}"));
        }

        let mut startup: STARTUPINFOEXW = std::mem::zeroed();
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.lpAttributeList = attr_list;
        let mut stdio = stdio;
        let mut inherit_handles = 0i32;
        if let Some(stdio) = &stdio {
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = stdio.child_in;
            startup.StartupInfo.hStdOutput = stdio.child_out;
            startup.StartupInfo.hStdError = stdio.child_err;
            inherit_handles = 1;
        }
        let mut info: PROCESS_INFORMATION = std::mem::zeroed();
        let created = CreateProcessW(
            program_w.as_ptr(),
            command_w.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            inherit_handles,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            env_block.as_ptr() as *const c_void,
            cwd_w.as_ptr(),
            &startup.StartupInfo,
            &mut info,
        );
        DeleteProcThreadAttributeList(attr_list);
        if created == 0 {
            return Err(format!(
                "以 AppContainer 身份启动 {} 失败：错误码 {}",
                program.display(),
                GetLastError()
            ));
        }
        CloseHandle(info.hThread);
        let pumps = match stdio.take() {
            Some(mut stdio) => {
                stdio.close_child_ends();
                stdio.start_pumps()
            }
            None => Vec::new(),
        };
        Ok(ChildProcess {
            process: info.hProcess,
            pumps,
        })
    }
}

/// 构造以 NUL 结尾的 Unicode 环境块（覆盖父进程值）。
fn build_env_block(overrides: &HashMap<String, String>) -> Vec<u16> {
    let mut vars: BTreeMap<String, (OsString, OsString)> = BTreeMap::new();
    for (key, value) in std::env::vars_os() {
        let sort_key = key.to_string_lossy().to_uppercase();
        vars.insert(sort_key, (key, value));
    }
    for (key, value) in overrides {
        vars.insert(
            key.to_uppercase(),
            (OsString::from(key), OsString::from(value)),
        );
    }
    let mut block = Vec::new();
    for (_, (key, value)) in vars {
        block.extend(key.encode_wide());
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

/// Windows 命令行参数转义（空格、引号、反斜杠）。
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(ch);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}
