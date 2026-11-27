use crate::error::{AppError, Result};

const ENTROPY: &[u8] = b"K::ProviderKey::v1";

pub fn protect(plain: &str) -> Result<Vec<u8>> {
    imp::protect(plain)
}

pub fn unprotect(blob: &[u8]) -> Result<String> {
    imp::unprotect(blob)
}

/// 仅展示密钥末四位，任何情况下都不返回明文。
pub fn mask(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 4 {
        "••••".to_string()
    } else {
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("••••{tail}")
    }
}

pub fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "••••")
}

pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    Some(out)
}

pub fn missing_key_error() -> AppError {
    AppError::Msg("供应商密钥无法解密，请重新填写 API Key".into())
}

#[cfg(windows)]
mod imp {
    use super::{missing_key_error, ENTROPY};
    use crate::error::{AppError, Result};
    use std::ffi::c_void;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            p_data_in: *const DataBlob,
            sz_data_descr: *const u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut c_void,
            p_prompt_struct: *mut c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
        fn CryptUnprotectData(
            p_data_in: *const DataBlob,
            ppsz_data_descr: *mut *mut u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut c_void,
            p_prompt_struct: *mut c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(h_mem: *mut c_void) -> *mut c_void;
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    pub fn protect(plain: &str) -> Result<Vec<u8>> {
        let mut input = plain.as_bytes().to_vec();
        let mut entropy = ENTROPY.to_vec();
        let in_blob = DataBlob {
            cb_data: input.len() as u32,
            pb_data: input.as_mut_ptr(),
        };
        let entropy_blob = DataBlob {
            cb_data: entropy.len() as u32,
            pb_data: entropy.as_mut_ptr(),
        };
        let mut out = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &in_blob,
                std::ptr::null(),
                &entropy_blob,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        input.fill(0);
        entropy.fill(0);
        if ok == 0 || out.pb_data.is_null() {
            return Err(AppError::Msg("DPAPI 加密失败，密钥未保存".into()));
        }
        let copied = unsafe {
            let slice = std::slice::from_raw_parts_mut(out.pb_data, out.cb_data as usize);
            let copied = slice.to_vec();
            slice.fill(0);
            copied
        };
        unsafe {
            LocalFree(out.pb_data as *mut c_void);
        }
        Ok(copied)
    }

    pub fn unprotect(blob: &[u8]) -> Result<String> {
        let mut input = blob.to_vec();
        let mut entropy = ENTROPY.to_vec();
        let in_blob = DataBlob {
            cb_data: input.len() as u32,
            pb_data: input.as_mut_ptr(),
        };
        let entropy_blob = DataBlob {
            cb_data: entropy.len() as u32,
            pb_data: entropy.as_mut_ptr(),
        };
        let mut out = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &in_blob,
                std::ptr::null_mut(),
                &entropy_blob,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        input.fill(0);
        entropy.fill(0);
        if ok == 0 || out.pb_data.is_null() {
            return Err(missing_key_error());
        }
        let mut copied = unsafe {
            let slice = std::slice::from_raw_parts_mut(out.pb_data, out.cb_data as usize);
            let copied = slice.to_vec();
            slice.fill(0);
            copied
        };
        unsafe {
            LocalFree(out.pb_data as *mut c_void);
        }
        let text = String::from_utf8(copied.clone()).map_err(|_| missing_key_error());
        copied.fill(0);
        text
    }
}

#[cfg(not(windows))]
mod imp {
    use crate::error::{AppError, Result};

    pub fn protect(_plain: &str) -> Result<Vec<u8>> {
        Err(AppError::Msg("密钥加密仅支持 Windows DPAPI".into()))
    }

    pub fn unprotect(_blob: &[u8]) -> Result<String> {
        Err(AppError::Msg("密钥解密仅支持 Windows DPAPI".into()))
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn dpapi_roundtrip() {
        let secret = "sk-test-1234567890";
        let blob = super::protect(secret).unwrap();
        assert_ne!(blob, secret.as_bytes());
        assert_eq!(super::unprotect(&blob).unwrap(), secret);
    }

    #[test]
    fn hex_roundtrip() {
        let data = b"K provider key";
        assert_eq!(super::from_hex(&super::to_hex(data)).unwrap(), data);
        assert_eq!(super::mask("sk-abcdefgh"), "••••efgh");
    }
}
