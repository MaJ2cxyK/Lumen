//! 命名管道桥与容器内 TCP ↔ 管道 relay。
//!
//! 拓扑（方案 R）：
//! ```text
//! 容器内 CLI ──TCP 127.0.0.1:P──▶ relay ──命名管道──▶ 宿主桥 ──TCP──▶ 主进程代理
//! ```
//! relay 与 CLI 运行在同一个 AppContainer 中，因此容器内的回环流量不会被隔离；
//! 宿主桥由普通用户进程持有，负责把管道流量转给主进程的本地代理。
//!
//! 每条通道使用两条单向管道：
//! - `up`：容器 → 宿主（relay 写，宿主桥读）；
//! - `down`：宿主 → 容器（宿主桥写，relay 读）。
//!
//! 不能对同一个双向管道句柄做并发读写：Windows 会按文件对象串行化同步 I/O，
//! 一侧阻塞在读取时另一侧的写入会被卡住，形成死锁。

use crate::sandbox::appcontainer::{sid_to_string, to_wide, AppContainerSid};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    GetLastError, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, OPEN_EXISTING, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::IO::CancelSynchronousIo;

const PIPE_BUFFER: u32 = 64 * 1024;

/// 单条通道：容器内监听端口 ↔ 一进一出两条管道。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayChannel {
    /// 容器 → 宿主：relay 写入，宿主桥读取。
    pub up_pipe: String,
    /// 宿主 → 容器：宿主桥写入，relay 读取。
    pub down_pipe: String,
    pub port: u16,
}

/// relay 进程的启动配置。
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelaySpec {
    pub channels: Vec<RelayChannel>,
    pub ready_file: String,
}

/// 生成命名管道的安全描述符字符串（SDDL）。
///
/// `OW`（Owner Rights）必须保留：受限令牌会同时做“常规 + 受限”两次检查，
/// 只给 AppContainer SID 时打开管道会返回 `ERROR_ACCESS_DENIED`。
pub fn pipe_sddl(sid: &AppContainerSid) -> Result<String, String> {
    let sid_text = unsafe { sid_to_string(sid.as_ptr()) }?;
    Ok(format!("D:(A;;GA;;;OW)(A;;GA;;;SY)(A;;GA;;;{sid_text})"))
}

fn create_pipe_instance(pipe_name: &str, sddl: &str, open_mode: u32) -> Result<File, String> {
    let sddl_w = to_wide(sddl);
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_w.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(format!(
            "解析管道安全描述符失败：错误码 {}",
            unsafe { GetLastError() }
        ));
    }
    let attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    let name_w = to_wide(pipe_name);
    let handle = unsafe {
        CreateNamedPipeW(
            name_w.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            &attrs,
        )
    };
    unsafe {
        LocalFree(sd as _);
    }
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "创建命名管道「{pipe_name}」失败：错误码 {}",
            unsafe { GetLastError() }
        ));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn connect_pipe_client(pipe_name: &str, access: u32) -> Result<File, String> {
    let name_w = to_wide(pipe_name);
    for _ in 0..100 {
        let handle = unsafe {
            CreateFileW(
                name_w.as_ptr(),
                access,
                0,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        };
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            return Ok(unsafe { File::from_raw_handle(handle) });
        }
        let code = unsafe { GetLastError() };
        if code == ERROR_PIPE_BUSY {
            unsafe {
                WaitNamedPipeW(name_w.as_ptr(), 100);
            }
            continue;
        }
        if code == ERROR_FILE_NOT_FOUND {
            // 管道实例可能尚未创建，短暂等待后重试。
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        return Err(format!("连接命名管道「{pipe_name}」失败：错误码 {code}"));
    }
    Err(format!("连接命名管道「{pipe_name}」超时"))
}

fn connect_server(pipe: &File) -> Result<(), String> {
    let ok = unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        if code != ERROR_PIPE_CONNECTED {
            return Err(format!("等待管道连接失败：错误码 {code}"));
        }
    }
    Ok(())
}

/// 双向转发：`tcp` 的入站流量写入 `sink`，`source` 的出站流量写回 `tcp`。
///
/// 两个管道句柄各自只在一个方向上使用，不能是同一个双向管道的副本。
fn duplex(tcp: TcpStream, sink: File, source: File) -> Result<(u64, u64), String> {
    let mut tcp_read = tcp
        .try_clone()
        .map_err(|e| format!("复制 TCP 句柄失败：{e}"))?;
    let mut tcp_write = tcp;
    let mut sink = sink;
    let mut source = source;

    let up = thread::spawn(move || pump(&mut tcp_read, &mut sink));
    let down = thread::spawn(move || {
        let copied = pump(&mut source, &mut tcp_write);
        let _ = tcp_write.shutdown(Shutdown::Write);
        copied
    });
    let up_bytes = up.join().unwrap_or(0);
    let down_bytes = down.join().unwrap_or(0);
    Ok((up_bytes, down_bytes))
}

fn pump<R: Read, W: Write>(reader: &mut R, writer: &mut W) -> u64 {
    let mut buffer = [0u8; 8192];
    let mut total = 0u64;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                total += n as u64;
                if writer.write_all(&buffer[..n]).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    total
}

fn accept_loop(listener: TcpListener, up_pipe: String, down_pipe: String) {
    for incoming in listener.incoming() {
        let Ok(stream) = incoming else {
            break;
        };
        let up = up_pipe.clone();
        let down = down_pipe.clone();
        thread::spawn(move || {
            let sink = match connect_pipe_client(&up, GENERIC_WRITE) {
                Ok(file) => file,
                Err(_) => return,
            };
            let source = match connect_pipe_client(&down, GENERIC_READ) {
                Ok(file) => file,
                Err(_) => return,
            };
            let _ = duplex(stream, sink, source);
        });
    }
}

/// relay 进程入口：绑定容器内端口，把流量转进宿主命名管道。
pub fn run_relay(spec_path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(spec_path)
        .map_err(|e| format!("读取 relay 配置 {} 失败：{e}", spec_path.display()))?;
    let spec: RelaySpec =
        serde_json::from_str(&text).map_err(|e| format!("解析 relay 配置失败：{e}"))?;

    for channel in &spec.channels {
        let listener = TcpListener::bind(("127.0.0.1", channel.port))
            .map_err(|e| format!("relay 绑定 127.0.0.1:{} 失败：{e}", channel.port))?;
        let up = channel.up_pipe.clone();
        let down = channel.down_pipe.clone();
        thread::spawn(move || accept_loop(listener, up, down));
    }

    if let Some(parent) = Path::new(&spec.ready_file).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&spec.ready_file, "{\"ready\":true}")
        .map_err(|e| format!("写入 relay 就绪文件失败：{e}"))?;

    // relay 由宿主在 CLI 退出后终止；这里保持进程存活。
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

/// 宿主侧桥接：接受一对管道连接，与 `target_port` 做双向转发，然后返回。
pub fn serve_bridge_once(
    up_pipe: &str,
    down_pipe: &str,
    target_port: u16,
    sddl: &str,
) -> Result<(), String> {
    let up = create_pipe_instance(up_pipe, sddl, PIPE_ACCESS_INBOUND)?;
    let down = create_pipe_instance(down_pipe, sddl, PIPE_ACCESS_OUTBOUND)?;
    connect_server(&up)?;
    connect_server(&down)?;
    let tcp = TcpStream::connect(("127.0.0.1", target_port))
        .map_err(|e| format!("桥接连接 127.0.0.1:{target_port} 失败：{e}"))?;
    duplex(tcp, down, up).map(|_| ())
}

/// 宿主侧桥接：持续为每对连接创建管道实例。
pub fn serve_bridge_forever(
    up_pipe: &str,
    down_pipe: &str,
    target_port: u16,
    sddl: &str,
) -> Result<(), String> {
    loop {
        let up = create_pipe_instance(up_pipe, sddl, PIPE_ACCESS_INBOUND)?;
        let down = create_pipe_instance(down_pipe, sddl, PIPE_ACCESS_OUTBOUND)?;
        connect_server(&up)?;
        connect_server(&down)?;
        let _ = thread::spawn(move || {
            if let Ok(tcp) = TcpStream::connect(("127.0.0.1", target_port)) {
                let _ = duplex(tcp, down, up);
            }
        });
    }
}

/// 可停止的宿主桥句柄；运行结束时调用 `stop` 回收线程与管道实例。
pub struct BridgeHandle {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl BridgeHandle {
    /// 停止桥线程：先置停止标志，再取消线程上阻塞的同步 I/O。
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let handle = thread.as_raw_handle() as HANDLE;
            unsafe {
                CancelSynchronousIo(handle);
            }
            let _ = thread.join();
        }
    }
}

impl Drop for BridgeHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 启动宿主侧桥线程：持续为每条容器连接创建管道实例并转发到 `target_port`。
pub fn start_bridge(
    up_pipe: String,
    down_pipe: String,
    target_port: u16,
    sddl: String,
) -> BridgeHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = thread::spawn(move || loop {
        if flag.load(Ordering::SeqCst) {
            break;
        }
        let up = match create_pipe_instance(&up_pipe, &sddl, PIPE_ACCESS_INBOUND) {
            Ok(file) => file,
            Err(err) => {
                tracing::warn!("{err}");
                break;
            }
        };
        let down = match create_pipe_instance(&down_pipe, &sddl, PIPE_ACCESS_OUTBOUND) {
            Ok(file) => file,
            Err(err) => {
                tracing::warn!("{err}");
                break;
            }
        };
        if connect_server(&up).is_err() || flag.load(Ordering::SeqCst) {
            break;
        }
        if connect_server(&down).is_err() || flag.load(Ordering::SeqCst) {
            break;
        }
        thread::spawn(move || {
            if let Ok(tcp) = TcpStream::connect(("127.0.0.1", target_port)) {
                let _ = duplex(tcp, down, up);
            }
        });
    });
    BridgeHandle {
        stop,
        thread: Some(thread),
    }
}

/// 尝试打开一次管道客户端后立即关闭，用于唤醒阻塞在 `ConnectNamedPipe` 的桥线程。
///
/// 只尝试一次，避免清理阶段在管道不存在时反复等待。
pub fn nudge(pipe_name: &str) {
    let name_w = to_wide(pipe_name);
    let handle = unsafe {
        CreateFileW(
            name_w.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
        drop(unsafe { File::from_raw_handle(handle) });
    }
}
