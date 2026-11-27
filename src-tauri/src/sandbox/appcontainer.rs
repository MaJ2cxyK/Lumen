//! AppContainer 配置（profile）与 SID 管理。
//!
//! 只负责“安全对象”的获取与释放：
//! - `ensure_profile`：确保指定 profile 存在，派生容器 SID；
//! - `capability_sid`：把 `internetClient` 等能力名翻译成能力 SID；
//! - `sid_to_string`：供命名管道 SDDL 使用。
//!
//! 文件 ACL 的授予在 `acl.rs`，进程创建在 `host.rs`。

use std::ptr;
use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSidToSidW};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{FreeSid, PSID};

/// `CreateAppContainerProfile` 在 profile 已存在时返回的 HRESULT。
const HRESULT_ALREADY_EXISTS: i32 = 0x8007_00B7u32 as i32;

/// 把 Rust 字符串转成以 NUL 结尾的 UTF-16。
pub fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 拥有所有权的 AppContainer SID，离开作用域时自动 `FreeSid`。
pub struct AppContainerSid(PSID);

impl AppContainerSid {
    pub fn as_ptr(&self) -> PSID {
        self.0
    }
}

impl Drop for AppContainerSid {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                FreeSid(self.0);
            }
        }
    }
}

/// 确保 AppContainer profile 存在。
///
/// 返回 `(SID, 是否本次新建)`；已存在时通过名字反查 SID。
pub fn ensure_profile(name: &str) -> Result<(AppContainerSid, bool), String> {
    let name_w = to_wide(name);
    let display_w = to_wide("Lumen Sandbox");
    let desc_w = to_wide("Lumen AppContainer sandbox for local CLI agents");
    let mut sid: PSID = ptr::null_mut();
    let hr = unsafe {
        CreateAppContainerProfile(
            name_w.as_ptr(),
            display_w.as_ptr(),
            desc_w.as_ptr(),
            ptr::null(),
            0,
            &mut sid,
        )
    };
    if hr >= 0 {
        return Ok((AppContainerSid(sid), true));
    }
    if hr == HRESULT_ALREADY_EXISTS {
        return Ok((derive_sid(name)?, false));
    }
    Err(format!(
        "创建 AppContainer profile「{name}」失败：HRESULT 0x{:08X}",
        hr as u32
    ))
}

/// 按 profile 名派生 SID（不创建 profile）。
pub fn derive_sid(name: &str) -> Result<AppContainerSid, String> {
    let name_w = to_wide(name);
    let mut sid: PSID = ptr::null_mut();
    let hr = unsafe { DeriveAppContainerSidFromAppContainerName(name_w.as_ptr(), &mut sid) };
    if hr < 0 {
        return Err(format!(
            "派生 AppContainer SID 失败：HRESULT 0x{:08X}",
            hr as u32
        ));
    }
    Ok(AppContainerSid(sid))
}

/// 删除 profile。仅用于自检等临时容器的清理。
pub fn delete_profile(name: &str) -> Result<(), String> {
    let name_w = to_wide(name);
    let hr = unsafe { DeleteAppContainerProfile(name_w.as_ptr()) };
    if hr < 0 {
        return Err(format!(
            "删除 AppContainer profile「{name}」失败：HRESULT 0x{:08X}",
            hr as u32
        ));
    }
    Ok(())
}

/// SID 转字符串（例如 `S-1-15-2-...`），用于拼接管道 SDDL。
///
/// # Safety
/// `sid` 必须是指向有效 SID 的指针（通常来自 `ensure_profile` / `derive_sid`）。
pub unsafe fn sid_to_string(sid: PSID) -> Result<String, String> {
    let mut raw: *mut u16 = ptr::null_mut();
    let ok = ConvertSidToStringSidW(sid, &mut raw);
    if ok == 0 {
        return Err(format!("SID 转字符串失败：{}", unsafe {
            GetLastError()
        }));
    }
    let text = unsafe {
        let len = (0..).take_while(|&i| *raw.add(i) != 0).count();
        let slice = std::slice::from_raw_parts(raw, len);
        let value = String::from_utf16_lossy(slice);
        LocalFree(raw as _);
        value
    };
    Ok(text)
}

/// 拥有所有权的“能力 SID”（如 internetClient），离开作用域时 `LocalFree`。
pub struct CapabilitySid(PSID);

impl CapabilitySid {
    pub fn as_ptr(&self) -> PSID {
        self.0
    }
}

impl Drop for CapabilitySid {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0 as _);
            }
        }
    }
}

/// 能力名或字面 SID → 能力 SID。
///
/// 默认不授予任何能力；只有显式配置时才会放开，避免默认联网外传。
pub fn capability_sid(name: &str) -> Result<CapabilitySid, String> {
    let sid_text = match name {
        "internetClient" => "S-1-15-3-1",
        "internetClientServer" => "S-1-15-3-2",
        "privateNetworkClientServer" => "S-1-15-3-3",
        other if other.starts_with("S-1-15-3-") => other,
        other => return Err(format!("未知的 AppContainer 能力：{other}")),
    };
    let sid_w = to_wide(sid_text);
    let mut sid: PSID = ptr::null_mut();
    let ok = unsafe { ConvertStringSidToSidW(sid_w.as_ptr(), &mut sid) };
    if ok == 0 {
        return Err(format!("解析能力 SID「{sid_text}」失败：{}", unsafe {
            GetLastError()
        }));
    }
    Ok(CapabilitySid(sid))
}
