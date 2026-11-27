use crate::error::{AppError, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Read;
use std::time::Duration;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HookSpecificOutput {
    hook_event_name: String,
    permission_decision: String,
    permission_decision_reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HookOutput {
    hook_specific_output: HookSpecificOutput,
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn emit(decision: &str, reason: &str) {
    let output = HookOutput {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: "PreToolUse".to_string(),
            permission_decision: decision.to_string(),
            permission_decision_reason: reason.to_string(),
        },
    };
    if let Ok(text) = serde_json::to_string(&output) {
        use std::io::Write;
        println!("{text}");
        let _ = std::io::stdout().flush();
    }
}

/// Entry point for `Lumen.exe --hook-gate --run-id ...`.
/// 网关地址与一次性 token 从环境变量读取（由 runtime 注入），不再出现在命令行或配置文件中。
pub fn run(args: &[String]) -> Result<()> {
    let run_id = std::env::var("LUMEN_RUN_ID")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| arg_value(args, "--run-id"))
        .ok_or_else(|| AppError::Msg("missing LUMEN_RUN_ID".into()))?;
    let base_url = std::env::var("LUMEN_GATE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| arg_value(args, "--url"))
        .ok_or_else(|| AppError::Msg("missing LUMEN_GATE_URL".into()))?;
    let token = std::env::var("LUMEN_GATE_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| AppError::Msg("missing LUMEN_GATE_TOKEN".into()))?;

    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let payload: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let tool_name = payload
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if tool_name.is_empty() {
        emit("deny", "无法识别的工具调用");
        return Ok(());
    }
    let tool_input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
    let cwd = payload
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let body = json!({
        "runId": run_id,
        "toolName": tool_name,
        "toolInput": tool_input,
        "cwd": cwd,
    });

    let url = format!("{}/gate/tool", base_url.trim_end_matches('/'));
    let response = ureq::post(&url)
        .set("x-agenthub-token", &token)
        .set("content-type", "application/json")
        .timeout(Duration::from_secs(660))
        .send_json(body);

    match response {
        Ok(resp) => {
            let text = resp.into_string().unwrap_or_default();
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let decision = parsed
                .get("decision")
                .and_then(|v| v.as_str())
                .unwrap_or("deny")
                .to_string();
            let reason = parsed
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("审批服务未返回原因")
                .to_string();
            emit(&decision, &reason);
        }
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            emit(
                "deny",
                &format!(
                    "审批服务返回 {code}：{}",
                    text.chars().take(200).collect::<String>()
                ),
            );
        }
        Err(err) => {
            emit("deny", &format!("审批服务不可用：{err}"));
        }
    }
    Ok(())
}
