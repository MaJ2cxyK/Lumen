//! Windows 命名管道回归测试：两条单向管道并发双向转发。
//!
//! 不能用同一个双向管道句柄（或其副本）并发读写：Windows 会按文件对象串行化
//! 同步 I/O，一侧阻塞在读取时另一侧的写入会被卡住。

use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::ptr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, OPEN_EXISTING, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn create_pipe(name: &str, open_mode: u32) -> File {
    let name_w = to_wide(name);
    let handle = unsafe {
        CreateNamedPipeW(
            name_w.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            ptr::null(),
        )
    };
    assert!(handle != INVALID_HANDLE_VALUE && !handle.is_null());
    unsafe { File::from_raw_handle(handle) }
}

fn open_pipe(name: &str, access: u32) -> File {
    let name_w = to_wide(name);
    for _ in 0..200 {
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
        if handle != INVALID_HANDLE_VALUE && !handle.is_null() {
            return unsafe { File::from_raw_handle(handle) };
        }
        // 服务端可能还没创建管道实例。
        thread::sleep(Duration::from_millis(10));
    }
    panic!("打开命名管道「{name}」超时");
}

fn connect(pipe: &File) {
    // 客户端可能已经连上，此时返回 FALSE 且错误码为 ERROR_PIPE_CONNECTED。
    unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) };
}

#[test]
fn duplex_through_two_unidirectional_pipes() {
    let id = std::process::id();
    let up = format!(r"\\.\pipe\lumen-test-up-{id}");
    let down = format!(r"\\.\pipe\lumen-test-down-{id}");

    let up_server_name = up.clone();
    let down_server_name = down.clone();
    thread::spawn(move || {
        let mut up_server = create_pipe(&up_server_name, PIPE_ACCESS_INBOUND);
        let mut down_server = create_pipe(&down_server_name, PIPE_ACCESS_OUTBOUND);
        connect(&up_server);
        connect(&down_server);
        let mut buffer = [0u8; 4];
        if let Ok(n) = up_server.read(&mut buffer) {
            let _ = down_server.write_all(&buffer[..n]);
            let _ = down_server.flush();
        }
        // 保持句柄存活，等客户端读完回包。
        thread::sleep(Duration::from_millis(200));
    });

    let mut up_client = open_pipe(&up, GENERIC_WRITE);
    let mut down_client = open_pipe(&down, GENERIC_READ);
    let writer = thread::spawn(move || {
        up_client.write_all(b"PING").unwrap();
        up_client.flush().unwrap();
    });
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0u8; 4];
        let n = down_client.read(&mut buffer).unwrap_or(0);
        let _ = tx.send((n, buffer));
    });

    let (n, buffer) = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("管道回环超时");
    writer.join().unwrap();
    assert_eq!(n, 4);
    assert_eq!(&buffer[..4], b"PING");
}
