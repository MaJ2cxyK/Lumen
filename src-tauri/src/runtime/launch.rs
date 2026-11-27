use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn is_cmd_program(program: &str) -> bool {
    let name = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    name.eq_ignore_ascii_case("cmd") || name.eq_ignore_ascii_case("cmd.exe")
}

/// OpenCode 只接受 `provider/model` 形式的显式模型参数；没有真实模型时宁可不传，
/// 也不要把它内置目录里的默认模型当作供应商模型。
pub(crate) fn opencode_model(provider_model: Option<&str>, request_model: &str) -> Option<String> {
    let model = provider_model.unwrap_or(request_model).trim();
    if model.is_empty()
        || model.eq_ignore_ascii_case("claude")
        || model.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, '&' | '|' | '<' | '>' | '^' | '"' | '%')
        })
    {
        return None;
    }
    Some(model.to_string())
}

/// 代理模式下为 OpenCode 使用 Lumen 临时 provider；直连模式沿用内置 provider。
pub(crate) fn opencode_model_ref(use_proxy: bool, protocol: &str, model: &str) -> Option<String> {
    let provider_id = match protocol {
        "openai" if use_proxy => "lumen",
        "openai" => "openai",
        "anthropic" => "anthropic",
        _ => return None,
    };
    Some(format!("{provider_id}/{model}"))
}

/// OpenAI 兼容端点通常要求 `/v1` 作为 base URL 的一部分。
pub(crate) fn with_v1(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1") {
        base.to_string()
    } else {
        format!("{base}/v1")
    }
}

/// 生成仅存在于本次运行环境中的 OpenCode 配置。密钥使用 `{env:...}` 占位，
/// 真实 Key 或一次性代理 token 不会写进配置文本或磁盘。
pub(crate) fn opencode_config(use_proxy: bool, protocol: &str, base: &str, model: &str) -> Value {
    let mut models = serde_json::Map::new();
    models.insert(
        model.to_string(),
        json!({
            "name": model,
            "limit": { "context": 200000, "output": 32000 }
        }),
    );
    let models = Value::Object(models);
    match (protocol, use_proxy) {
        ("openai", true) => json!({
            "provider": {
                "lumen": {
                    "npm": "@ai-sdk/openai-compatible",
                    "name": "Lumen",
                    "options": {
                        "baseURL": base,
                        "apiKey": "{env:OPENAI_API_KEY}"
                    },
                    "models": models
                }
            }
        }),
        ("openai", false) => json!({
            "provider": {
                "openai": { "models": models }
            }
        }),
        ("anthropic", true) => json!({
            "provider": {
                "anthropic": {
                    "options": {
                        "baseURL": base,
                        "apiKey": "{env:ANTHROPIC_API_KEY}"
                    },
                    "models": models
                }
            }
        }),
        ("anthropic", false) => json!({
            "provider": {
                "anthropic": { "models": models }
            }
        }),
        _ => json!({}),
    }
}

pub(crate) fn spawn_spec(command: &str, extra_args: &[String]) -> (String, Vec<String>) {
    let path = Path::new(command);
    if path.is_file() {
        let (program, mut args) = crate::detect::command_for(path);
        args.extend(extra_args.iter().cloned());
        (program, args)
    } else if command.chars().any(|c| c.is_whitespace()) {
        let mut args = vec!["/C".to_string(), command.to_string()];
        args.extend(extra_args.iter().cloned());
        ("cmd".to_string(), args)
    } else {
        (command.to_string(), extra_args.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_model_wins_and_placeholder_is_rejected() {
        assert_eq!(
            opencode_model(Some("deepseek-flash"), "claude").as_deref(),
            Some("deepseek-flash")
        );
        assert_eq!(
            opencode_model(None, "deepseek-flash").as_deref(),
            Some("deepseek-flash")
        );
        assert_eq!(opencode_model(None, "claude"), None);
        assert_eq!(opencode_model(Some("bad&model"), "claude"), None);
    }

    #[test]
    fn model_reference_matches_transport() {
        assert_eq!(
            opencode_model_ref(true, "openai", "deepseek-flash").as_deref(),
            Some("lumen/deepseek-flash")
        );
        assert_eq!(
            opencode_model_ref(false, "openai", "deepseek-flash").as_deref(),
            Some("openai/deepseek-flash")
        );
        assert_eq!(
            opencode_model_ref(true, "anthropic", "claude-sonnet").as_deref(),
            Some("anthropic/claude-sonnet")
        );
    }

    #[test]
    fn base_url_gets_v1_once() {
        assert_eq!(
            with_v1("https://api.deepseek.com"),
            "https://api.deepseek.com/v1"
        );
        assert_eq!(
            with_v1("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1"
        );
    }

    #[test]
    fn proxy_config_registers_dynamic_model_without_secret() {
        let config = opencode_config(
            true,
            "openai",
            "http://127.0.0.1:1234/proxy/token/v1",
            "deepseek-flash",
        );
        assert_eq!(
            config["provider"]["lumen"]["models"]["deepseek-flash"]["name"],
            "deepseek-flash"
        );
        assert_eq!(
            config["provider"]["lumen"]["options"]["apiKey"],
            "{env:OPENAI_API_KEY}"
        );
        assert_eq!(
            config["provider"]["lumen"]["options"]["baseURL"],
            "http://127.0.0.1:1234/proxy/token/v1"
        );
        assert!(!config.to_string().contains("sk-"));
    }

    #[test]
    fn direct_config_uses_builtin_provider() {
        let config = opencode_config(
            false,
            "openai",
            "https://api.deepseek.com/v1",
            "deepseek-flash",
        );
        assert!(config["provider"]["lumen"].is_null());
        assert_eq!(
            config["provider"]["openai"]["models"]["deepseek-flash"]["name"],
            "deepseek-flash"
        );
    }
}
