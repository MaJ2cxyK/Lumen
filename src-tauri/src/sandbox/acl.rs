//! 文件系统 DACL 授予与回收。
//!
//! 只改 DACL，不触碰 SACL，因此普通用户权限即可完成，不需要 `SeSecurityPrivilege`。
//! 所有路径先做规范化：解析符号链接，并拒绝 UNC 路径，避免把 ACE 打到网络位置上。

use crate::sandbox::appcontainer::{to_wide, AppContainerSid};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::ptr;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_SUCCESS, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, EXPLICIT_ACCESS_W, GRANT_ACCESS, REVOKE_ACCESS,
    SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    InitializeSecurityDescriptor, MakeSelfRelativeSD, SetSecurityDescriptorDacl, ACL,
    DACL_SECURITY_INFORMATION, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID, SECURITY_DESCRIPTOR,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ALL_ACCESS, FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_EXECUTE,
    FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE,
    OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
};
use windows_sys::Win32::System::SystemServices::SECURITY_DESCRIPTOR_REVISION;

// 直接调用 ntdll 写入 DACL，绕过 Win32 层的“自动继承传播”。
//
// `SetNamedSecurityInfoW` 在目录上会把 DACL 里的可继承 ACE 重新下发给整棵子树；
// 对 `AppData\Local` 这类目录会触发全盘遍历，实际表现为进程卡死。
// 我们只添加/删除一条非继承 ACE，不需要也不应该触发传播。
#[link(name = "ntdll")]
extern "system" {
    fn NtSetSecurityObject(
        handle: HANDLE,
        security_information: u32,
        security_descriptor: PSECURITY_DESCRIPTOR,
    ) -> i32;
}

/// 目录的完全控制（自检时授予工作目录/沙箱 HOME）。
pub const DIR_FULL: u32 = FILE_ALL_ACCESS;
/// 可执行文件需要的最小权限：读取 + 执行。
pub const EXE_READ_EXECUTE: u32 = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
/// 只读文件（运行目录里已存在的配置）。
pub const FILE_READ: u32 = FILE_GENERIC_READ;

/// 规范化路径：解析符号链接、去掉 `\\?\` 前缀，并拒绝 UNC。
pub fn normalize(path: &Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| format!("规范化路径 {} 失败：{e}", path.display()))?;
    let text = canonical.to_string_lossy().to_string();
    if let Some(rest) = text.strip_prefix("\\\\?\\") {
        if rest.starts_with("UNC\\") {
            return Err(format!("不支持 UNC 路径：{text}"));
        }
        return Ok(PathBuf::from(rest));
    }
    if text.starts_with("\\\\") {
        return Err(format!("不支持 UNC 路径：{text}"));
    }
    Ok(canonical)
}

fn trustee(sid: PSID) -> TRUSTEE_W {
    TRUSTEE_W {
        pMultipleTrustee: ptr::null_mut(),
        MultipleTrusteeOperation: 0,
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_UNKNOWN,
        ptstrName: sid as *mut u16,
    }
}

/// 在路径上为目标 SID 追加一条允许 ACE（`GRANT_ACCESS` 会与既有 ACE 合并）。
///
/// `inherit` 为真时使用“容器和对象继承”，只对目录有意义。
pub fn grant(path: &Path, sid: PSID, rights: u32, inherit: bool) -> Result<(), String> {
    let path = normalize(path)?;
    let wide = to_wide(&path.to_string_lossy());
    unsafe {
        let mut old_dacl: *mut ACL = ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        let code = GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut old_dacl,
            ptr::null_mut(),
            &mut sd,
        );
        if code != ERROR_SUCCESS {
            return Err(format!(
                "读取 {} 的 DACL 失败：错误码 {code}",
                path.display()
            ));
        }

        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: rights,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: if inherit {
                SUB_CONTAINERS_AND_OBJECTS_INHERIT
            } else {
                NO_INHERITANCE
            },
            Trustee: trustee(sid),
        };
        let mut new_dacl: *mut ACL = ptr::null_mut();
        let code = SetEntriesInAclW(1, &entry, old_dacl, &mut new_dacl);
        LocalFree(sd as _);
        if code != ERROR_SUCCESS {
            return Err(format!(
                "构造 {} 的新 DACL 失败：错误码 {code}",
                path.display()
            ));
        }

        let result = apply_dacl(&path, new_dacl);
        LocalFree(new_dacl as _);
        result?;
    }
    Ok(())
}

/// 从路径 DACL 中移除目标 SID 的全部允许/拒绝 ACE。
pub fn revoke(path: &Path, sid: PSID) -> Result<(), String> {
    let path = normalize(path)?;
    let wide = to_wide(&path.to_string_lossy());
    unsafe {
        let mut old_dacl: *mut ACL = ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        let code = GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut old_dacl,
            ptr::null_mut(),
            &mut sd,
        );
        if code != ERROR_SUCCESS {
            return Err(format!(
                "读取 {} 的 DACL 失败：错误码 {code}",
                path.display()
            ));
        }

        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: 0,
            grfAccessMode: REVOKE_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: trustee(sid),
        };
        let mut new_dacl: *mut ACL = ptr::null_mut();
        let code = SetEntriesInAclW(1, &entry, old_dacl, &mut new_dacl);
        LocalFree(sd as _);
        if code != ERROR_SUCCESS {
            return Err(format!(
                "构造 {} 的新 DACL 失败：错误码 {code}",
                path.display()
            ));
        }

        let result = apply_dacl(&path, new_dacl);
        LocalFree(new_dacl as _);
        result?;
    }
    Ok(())
}

/// 通过 ntdll 写入 DACL，避免 Win32 层把可继承 ACE 重新传播到整棵子树。
fn apply_dacl(path: &Path, new_dacl: *mut ACL) -> Result<(), String> {
    let wide = to_wide(&path.to_string_lossy());
    unsafe {
        // NtSetSecurityObject 需要自相对格式的安全描述符。
        let mut absolute: SECURITY_DESCRIPTOR = std::mem::zeroed();
        if InitializeSecurityDescriptor(
            &mut absolute as *mut _ as PSECURITY_DESCRIPTOR,
            SECURITY_DESCRIPTOR_REVISION,
        ) == 0
        {
            return Err(format!(
                "初始化 {} 的安全描述符失败：错误码 {}",
                path.display(),
                GetLastError()
            ));
        }
        if SetSecurityDescriptorDacl(
            &mut absolute as *mut _ as PSECURITY_DESCRIPTOR,
            1,
            new_dacl,
            0,
        ) == 0
        {
            return Err(format!(
                "构造 {} 的安全描述符失败：错误码 {}",
                path.display(),
                GetLastError()
            ));
        }
        let mut size = 0u32;
        MakeSelfRelativeSD(
            &mut absolute as *mut _ as PSECURITY_DESCRIPTOR,
            ptr::null_mut(),
            &mut size,
        );
        if size == 0 {
            return Err(format!(
                "计算 {} 的安全描述符大小失败：错误码 {}",
                path.display(),
                GetLastError()
            ));
        }
        let mut buffer = vec![0u8; size as usize];
        if MakeSelfRelativeSD(
            &mut absolute as *mut _ as PSECURITY_DESCRIPTOR,
            buffer.as_mut_ptr() as PSECURITY_DESCRIPTOR,
            &mut size,
        ) == 0
        {
            return Err(format!(
                "转换 {} 的安全描述符失败：错误码 {}",
                path.display(),
                GetLastError()
            ));
        }

        let handle = CreateFileW(
            wide.as_ptr(),
            READ_CONTROL | WRITE_DAC,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        );
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(format!(
                "打开 {} 以写入 DACL 失败：错误码 {}",
                path.display(),
                GetLastError()
            ));
        }
        let status = NtSetSecurityObject(
            handle,
            DACL_SECURITY_INFORMATION,
            buffer.as_ptr() as PSECURITY_DESCRIPTOR,
        );
        CloseHandle(handle);
        if status < 0 {
            return Err(format!(
                "写入 {} 的 DACL 失败：NTSTATUS 0x{:08X}",
                path.display(),
                status as u32
            ));
        }
    }
    Ok(())
}

/// 给目录树授予权限：根目录写入可继承 ACE，再向“已存在”的子对象传播。
///
/// Windows 的继承只在对象创建时生效，已存在的文件不会自动获得新 ACE；
/// 这里用系统 `icacls /T` 补授，避免逐个文件调用 API。
/// `icacls_rights` 为 icacls 的权限代号，例如 `F`、`RX`。
pub fn grant_tree(
    path: &Path,
    sid: &AppContainerSid,
    rights: u32,
    icacls_rights: &str,
) -> Result<(), String> {
    let root = normalize(path)?;
    grant(&root, sid.as_ptr(), rights, true)?;
    let sid_text = unsafe { crate::sandbox::appcontainer::sid_to_string(sid.as_ptr())? };
    // 已存在对象按对象本身授权即可（/T 会遍历全部子项），无需再带继承标志。
    run_icacls(&[
        root.to_string_lossy().to_string(),
        "/grant".to_string(),
        format!("*{sid_text}:{icacls_rights}"),
        "/T".to_string(),
        "/C".to_string(),
        "/Q".to_string(),
    ]);
    Ok(())
}

/// 撤销目录树上的容器 SID：先遍历删除，再移除根目录 ACE。
pub fn revoke_tree(path: &Path, sid: &AppContainerSid) {
    let Ok(root) = normalize(path) else {
        return;
    };
    let sid_text = unsafe { crate::sandbox::appcontainer::sid_to_string(sid.as_ptr()).ok() };
    if let Some(sid_text) = sid_text {
        run_icacls(&[
            root.to_string_lossy().to_string(),
            "/remove:g".to_string(),
            format!("*{sid_text}"),
            "/T".to_string(),
            "/C".to_string(),
            "/Q".to_string(),
        ]);
    }
    let _ = revoke(&root, sid.as_ptr());
}

/// 运行 icacls；失败只记录日志，不阻断运行（权限回收属于尽力而为）。
fn run_icacls(args: &[String]) {
    let mut command = std::process::Command::new("icacls");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let Ok(mut child) = command.spawn() else {
        tracing::warn!("启动 icacls 失败，继承授权可能不完整");
        return;
    };
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    tracing::warn!(
                        "icacls {:?} 退出码 {}",
                        args.first(),
                        status.code().unwrap_or(-1)
                    );
                }
                return;
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!("等待 icacls 失败：{err}");
                return;
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            tracing::warn!("icacls 超过 5 分钟，已终止");
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 给目标路径的每一级祖先目录追加 `FILE_TRAVERSE`，返回成功授予的路径。
///
/// 某些系统目录（如 `C:\`）由 TrustedInstaller 控制，普通用户改不了 DACL；
/// 这些目录默认通常已允许 `ALL APPLICATION PACKAGES` 通过，因此失败只跳过。
pub fn grant_ancestors(path: &Path, sid: PSID) -> Vec<PathBuf> {
    let Ok(path) = normalize(path) else {
        return Vec::new();
    };
    let mut granted = Vec::new();
    let mut current = path.parent();
    while let Some(dir) = current {
        if grant(dir, sid, FILE_TRAVERSE, false).is_ok() {
            granted.push(dir.to_path_buf());
        }
        current = dir.parent();
    }
    granted
}
