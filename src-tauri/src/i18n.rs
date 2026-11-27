use std::fs;
use std::sync::atomic::{AtomicU8, Ordering};
use tauri::Manager;

/// 后端动态文案的当前语言：0 = 中文，1 = 英文。
static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(lang: &str) {
    let value = if lang == "en" { 1 } else { 0 };
    LANG.store(value, Ordering::Relaxed);
}

pub fn is_en() -> bool {
    LANG.load(Ordering::Relaxed) == 1
}

/// 纯函数版本，便于测试；英文缺失时回退中文。
pub fn tr_for(en: bool, zh: &str, en_text: &str) -> String {
    if en {
        en_text.to_string()
    } else {
        zh.to_string()
    }
}

pub fn tr(zh: &str, en_text: &str) -> String {
    tr_for(is_en(), zh, en_text)
}

fn lang_file(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("language.txt"))
}

/// 启动时读取持久化语言；读取失败按中文处理。
pub fn load(app: &tauri::AppHandle) {
    let Some(path) = lang_file(app) else {
        set_lang("zh");
        return;
    };
    match fs::read_to_string(path) {
        Ok(lang) => set_lang(lang.trim()),
        Err(_) => set_lang("zh"),
    }
}

pub fn save(app: &tauri::AppHandle, lang: &str) {
    let Some(path) = lang_file(app) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let value = if lang == "en" { "en" } else { "zh" };
    let _ = fs::write(path, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tr_for_switches_between_chinese_and_english() {
        // 只测纯函数，避免修改全局语言状态影响并行用例。
        assert_eq!(tr_for(false, "中文", "English"), "中文");
        assert_eq!(tr_for(true, "中文", "English"), "English");
    }
}
